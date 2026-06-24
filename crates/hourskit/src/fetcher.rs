//! ETag-aware HTTP fetcher with retry, single-flight, CDN mirror fallback,
//! SHA-256 manifest verification, and a hard 24-hour staleness ceiling.
//!
//! # Cache layout
//!
//! ```text
//! $HOURSKIT_CACHE_DIR/                  (default: XDG cache / hourskit)
//! ├── sessions.parquet                   <- cached body
//! ├── sessions.parquet.etag              <- last ETag returned by origin
//! ├── manifest.json                      <- last manifest fetched + verified
//! └── manifest.json.etag                 <- last ETag returned by origin
//! ```
//!
//! # Fetch flow (per call)
//!
//! 1. Resolve the manifest first (same staleness ceiling as data files;
//!    falls back to a stale local manifest up to a configurable
//!    `manifest_stale_ceiling`, default 7 days).
//! 2. Single-flight gate: concurrent callers with the same key share one
//!    request.
//! 3. Staleness check: if the cache file's mtime is older than the
//!    staleness ceiling, do NOT send `If-None-Match` — force a fresh body.
//! 4. `304 Not Modified` -> return cached bytes; touch the cache mtime to
//!    extend the staleness window.
//! 5. `2xx` -> stream the body to a unique tmp file, hash as it goes,
//!    verify SHA-256 against the manifest BEFORE rename. On match: rename
//!    over the cache + fsync the parent directory. On mismatch: delete
//!    tmp and KEEP the prior good cache file.
//! 6. Retry-able error (5xx, 429, connect/timeout): exponential backoff up
//!    to 3 total attempts. Delays: 250 ms -> 500 ms -> 1 000 ms -> 2 000 ms (capped).
//!    429 response: respect `Retry-After` header if present.
//! 7. On primary-URL exhaustion: try jsDelivr CDN mirror once.
//! 8. All transports failed but cache exists -> warn + return stale.
//! 9. All transports failed + no cache -> return `Err`.
//!
//! # SHA-256 verification
//!
//! Verification happens on the streamed bytes BEFORE they replace the
//! cache file. A mismatch returns `Error::ChecksumMismatch`, leaves the
//! prior good cache file untouched, and removes the tmp file. The kit
//! refuses to serve unverified data: a missing or unreadable manifest
//! propagates as `Error::ManifestUnavailable`; a stale-only manifest
//! beyond `manifest_stale_ceiling` propagates as `Error::ManifestStale`.

use bytes::Bytes;
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::{Mutex, OnceCell};

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum total attempts (initial + 2 retries).
const MAX_ATTEMPTS: u32 = 3;

/// Base delay for exponential backoff.
const BACKOFF_BASE_MS: u64 = 250;

/// Cap on backoff delay.
const BACKOFF_MAX_MS: u64 = 2_000;

/// Default staleness ceiling: 24 hours.
///
/// The product requirement "If anything is cached, it will be crushed by the
/// new data within the next 24 hours" maps to this constant. Cache mtime
/// older than `STALENESS_CEILING` forces a fresh fetch even if the ETag
/// would otherwise short-circuit it.
pub(crate) const STALENESS_CEILING: Duration = Duration::from_secs(24 * 3_600);

/// Default manifest fallback ceiling: 7 days.
///
/// When the upstream is unreachable AND a local cached manifest exists,
/// the fetcher will use the stale manifest up to this age. Past this
/// ceiling, the fetcher refuses to serve unverified bytes and returns
/// [`Error::ManifestStale`].
pub(crate) const MANIFEST_STALE_CEILING: Duration = Duration::from_secs(7 * 24 * 3_600);

/// Logical manifest filename inside the cache directory and origin.
const MANIFEST_FILE: &str = "manifest.json";

// ---------------------------------------------------------------------------
// In-flight entry
// ---------------------------------------------------------------------------

/// An in-flight or completed fetch. Stored in the single-flight map while
/// a key is being fetched; on failure the typed [`Error`] is preserved
/// (wrapped in `Arc` so it can be cloned to every waiting caller).
type InflightCell = Arc<OnceCell<std::result::Result<Bytes, Arc<Error>>>>;

/// In-memory copy of a previously-fetched manifest, including the
/// observed local-cache mtime so we can age it independently from the
/// data files.
struct ManifestCache {
    digests: HashMap<String, String>,
    mtime: SystemTime,
}

// ---------------------------------------------------------------------------
// CachedFetcher
// ---------------------------------------------------------------------------

/// ETag-aware fetcher with retry, single-flight deduplication, CDN mirror
/// fallback, SHA-256 manifest verification, and a configurable staleness
/// ceiling.
pub(crate) struct CachedFetcher {
    pub(crate) http: reqwest::Client,
    /// Primary origin URL (e.g. `raw.githubusercontent.com/...//data`).
    pub(crate) base_url: String,
    /// CDN mirror base URL, consulted after primary exhausts all retries.
    pub(crate) mirror_url: Option<String>,
    pub(crate) cache_dir: PathBuf,
    /// Per-key in-flight deduplication.
    inflight: Arc<Mutex<HashMap<String, InflightCell>>>,
    /// In-memory manifest copy + the local-cache mtime that produced it.
    manifest: Arc<Mutex<Option<ManifestCache>>>,
    /// Staleness ceiling on cached data files.
    staleness_ceiling: Duration,
    /// Stale-manifest fallback ceiling.
    manifest_stale_ceiling: Duration,
}

impl CachedFetcher {
    pub(crate) fn new(http: reqwest::Client, base_url: String, cache_dir: PathBuf) -> Self {
        let mirror_url = Some(
            std::env::var("HOURSKIT_MIRROR_URL").unwrap_or_else(|_| DEFAULT_MIRROR_URL.to_string()),
        );
        Self {
            http,
            base_url,
            mirror_url,
            cache_dir,
            inflight: Arc::new(Mutex::new(HashMap::new())),
            manifest: Arc::new(Mutex::new(None)),
            staleness_ceiling: STALENESS_CEILING,
            manifest_stale_ceiling: MANIFEST_STALE_CEILING,
        }
    }

    /// Override the primary origin URL.
    pub(crate) fn set_base_url(&mut self, url: String) {
        self.base_url = url;
    }

    /// Override the mirror URL. `None` disables mirror fallback entirely.
    pub(crate) fn set_mirror_url(&mut self, url: Option<String>) {
        self.mirror_url = url;
    }

    /// Override the cache directory.
    pub(crate) fn set_cache_dir(&mut self, dir: PathBuf) {
        self.cache_dir = dir;
    }

    /// Override the staleness ceiling.
    ///
    /// Default: 24 hours. Pass `Duration::ZERO` to force every fetch to
    /// hit the network (useful in tests).
    pub(crate) const fn set_staleness_ceiling(&mut self, ceiling: Duration) {
        self.staleness_ceiling = ceiling;
    }

    /// Override the stale-manifest fallback ceiling.
    ///
    /// Default: 7 days. When the upstream is unreachable and the cached
    /// manifest is older than this, the fetcher returns
    /// [`Error::ManifestStale`] rather than serving unverified data.
    #[cfg(test)]
    pub(crate) const fn set_manifest_stale_ceiling(&mut self, ceiling: Duration) {
        self.manifest_stale_ceiling = ceiling;
    }

    /// Fetch a parquet file by logical key (e.g. `"sessions"`).
    ///
    /// Single-flight: concurrent callers with the same key share one request.
    pub(crate) async fn fetch(&self, key: &str) -> Result<Bytes> {
        let cell: InflightCell = {
            let mut map = self.inflight.lock().await;
            map.entry(key.to_string())
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone()
        };

        let key_owned = key.to_string();
        let result = cell
            .get_or_init(|| async {
                match self.do_fetch(&key_owned).await {
                    Ok(b) => Ok(b),
                    Err(e) => Err(Arc::new(e)),
                }
            })
            .await;

        // Remove the cell so future fetches can run fresh.
        {
            let mut map = self.inflight.lock().await;
            map.remove(key);
        }

        match result {
            Ok(b) => Ok(b.clone()),
            Err(e) => Err(clone_error(e)),
        }
    }

    async fn do_fetch(&self, key: &str) -> Result<Bytes> {
        // Resolve the manifest first. If we cannot get one (fresh, stale,
        // or fallback) we refuse to serve unverified bytes.
        let expected_digest = self.expected_digest_for(key).await?;

        let cache_path = self.cache_dir.join(format!("{key}.parquet"));
        let etag_path = self.cache_dir.join(format!("{key}.parquet.etag"));

        let primary_outcome = self
            .fetch_with_retry(
                key,
                &self.base_url.clone(),
                &cache_path,
                &etag_path,
                expected_digest.as_deref(),
            )
            .await;
        match primary_outcome {
            Ok(bytes) => Ok(bytes),
            Err(Error::ChecksumMismatch { .. }) => {
                // Hard error — do NOT fall back to mirror or stale
                // cache. The corrupt body has already been discarded;
                // the prior good cache (if any) is intact.
                primary_outcome
            }
            Err(primary_err) => {
                if let Some(mirror) = &self.mirror_url {
                    tracing::warn!(
                        key,
                        error = %primary_err,
                        "primary fetch exhausted retries, trying CDN mirror"
                    );
                    match self
                        .fetch_single_verified(
                            key,
                            &mirror.clone(),
                            &cache_path,
                            &etag_path,
                            expected_digest.as_deref(),
                        )
                        .await
                    {
                        Ok(bytes) => return Ok(bytes),
                        Err(Error::ChecksumMismatch { .. }) => {
                            // Same hard-error semantics as the primary path.
                            return Err(Error::ChecksumMismatch {
                                file: format!("{key}.parquet"),
                                expected: expected_digest.unwrap_or_default(),
                                computed: String::new(),
                            });
                        }
                        Err(mirror_err) => {
                            tracing::warn!(
                                key,
                                mirror_error = %mirror_err,
                                "CDN mirror also failed"
                            );
                        }
                    }
                } else {
                    tracing::debug!(key, "mirror fallback disabled, returning primary error");
                }
                if cache_path.exists() {
                    tracing::warn!(key, "all transports failed, serving stale cache");
                    let bytes = tokio::fs::read(&cache_path).await?;
                    return Ok(bytes.into());
                }
                Err(primary_err)
            }
        }
    }

    async fn fetch_with_retry(
        &self,
        key: &str,
        base: &str,
        cache_path: &Path,
        etag_path: &Path,
        expected_digest: Option<&str>,
    ) -> Result<Bytes> {
        let url = format!("{base}/{key}.parquet");
        let mut last_err: Option<Error> = None;
        let cache_is_fresh = cache_path.exists() && !is_stale(cache_path, self.staleness_ceiling);

        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                let delay_ms = backoff_delay_ms(attempt);
                tracing::debug!(key, attempt, delay_ms, "retry backoff");
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }

            let mut req = self.http.get(&url);
            // Only short-circuit with If-None-Match when the cached file is
            // within the staleness ceiling. Past 24h we force a fresh body.
            if cache_is_fresh {
                if let Some(etag) = read_etag(etag_path) {
                    req = req.header("If-None-Match", etag);
                }
            }

            match req.send().await {
                Ok(resp) if resp.status() == StatusCode::NOT_MODIFIED => {
                    let bytes = tokio::fs::read(cache_path).await?;
                    // Touch the cache mtime so the staleness window restarts.
                    if let Err(e) = touch(cache_path) {
                        tracing::warn!("could not touch cache mtime: {e}");
                    }
                    return Ok(bytes.into());
                }
                Ok(resp) if resp.status().is_success() => {
                    let etag = resp
                        .headers()
                        .get("etag")
                        .and_then(|v| v.to_str().ok())
                        .map(String::from);
                    let bytes = resp.bytes().await?;
                    let file_label = format!("{key}.parquet");
                    self.write_verified(&file_label, &bytes, cache_path, expected_digest)
                        .await?;
                    if let Some(e) = etag {
                        // ETag is a small text file; the same atomic-write
                        // primitive is appropriate.
                        write_atomic_with_unique_tmp(etag_path, e.as_bytes()).await?;
                    }
                    return Ok(bytes);
                }
                Ok(resp) if resp.status() == StatusCode::TOO_MANY_REQUESTS => {
                    let delay = retry_after_delay(&resp)
                        .unwrap_or_else(|| Duration::from_millis(backoff_delay_ms(attempt + 1)));
                    tracing::warn!(
                        key,
                        attempt,
                        delay_secs = delay.as_secs_f32(),
                        "429 rate-limited"
                    );
                    if attempt + 1 < MAX_ATTEMPTS {
                        tokio::time::sleep(delay).await;
                        last_err =
                            Some(Error::Source(format!("fetch {key}: 429 Too Many Requests")));
                        continue;
                    }
                    return Err(Error::Source(format!(
                        "fetch {key}: 429 Too Many Requests (final)"
                    )));
                }
                Ok(resp) if should_retry_status(resp.status()) => {
                    last_err = Some(Error::Source(format!(
                        "fetch {key}: HTTP {} {}",
                        resp.status().as_u16(),
                        resp.status().canonical_reason().unwrap_or("")
                    )));
                }
                Ok(resp) => {
                    return Err(Error::Source(format!(
                        "fetch {key}: HTTP {} {}",
                        resp.status().as_u16(),
                        resp.status().canonical_reason().unwrap_or("")
                    )));
                }
                Err(e) if is_retriable_error(&e) => {
                    tracing::warn!(key, attempt, error = %e, "transient error, will retry");
                    last_err = Some(Error::Http(e));
                }
                Err(e) => {
                    last_err = Some(Error::Http(e));
                    break;
                }
            }
        }

        Err(last_err.unwrap_or_else(|| Error::Source(format!("fetch {key}: all attempts failed"))))
    }

    /// Mirror path: same verify-before-write semantics as the primary.
    async fn fetch_single_verified(
        &self,
        key: &str,
        base: &str,
        cache_path: &Path,
        _etag_path: &Path,
        expected_digest: Option<&str>,
    ) -> Result<Bytes> {
        let url = format!("{base}/{key}.parquet");
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(Error::Source(format!(
                "mirror {key}: HTTP {} {}",
                resp.status().as_u16(),
                resp.status().canonical_reason().unwrap_or("")
            )));
        }
        let bytes = resp.bytes().await?;
        let file_label = format!("{key}.parquet");
        self.write_verified(&file_label, &bytes, cache_path, expected_digest)
            .await?;
        Ok(bytes)
    }

    /// Hash `bytes`, verify against `expected_digest` if present, and
    /// only on a match write to `cache_path` via the unique-tmp atomic
    /// primitive. On mismatch the prior good cache file (if any) is left
    /// intact.
    async fn write_verified(
        &self,
        file_label: &str,
        bytes: &Bytes,
        cache_path: &Path,
        expected_digest: Option<&str>,
    ) -> Result<()> {
        if let Some(expected) = expected_digest {
            let computed = hex_sha256(bytes);
            if computed != expected {
                tracing::warn!(
                    file = file_label,
                    expected,
                    computed,
                    "checksum mismatch — keeping prior good cache file untouched"
                );
                return Err(Error::ChecksumMismatch {
                    file: file_label.to_string(),
                    expected: expected.to_string(),
                    computed,
                });
            }
        }
        let parent = cache_path.parent().unwrap_or_else(|| Path::new("."));
        tokio::fs::create_dir_all(parent).await?;
        write_atomic_with_unique_tmp(cache_path, bytes).await?;
        Ok(())
    }

    /// Resolve the SHA-256 digest expected for `key` from the manifest.
    ///
    /// Refuses to serve an empty manifest: a missing or unparseable
    /// manifest with no local cache propagates as
    /// [`Error::ManifestUnavailable`]; a stale-only manifest beyond the
    /// configured ceiling propagates as [`Error::ManifestStale`].
    async fn expected_digest_for(&self, key: &str) -> Result<Option<String>> {
        let manifest = self.load_manifest().await?;
        Ok(manifest
            .digests
            .get(&format!("{key}.parquet"))
            .and_then(|v| v.strip_prefix("sha256:").map(str::to_string)))
    }

    /// Load the manifest into memory, fetching it (with the same
    /// 24-hour staleness rule as data files) when necessary. Falls back
    /// to a stale local copy up to `manifest_stale_ceiling`.
    async fn load_manifest(&self) -> Result<ManifestCache> {
        if let Some(fresh) = self.try_in_memory_manifest().await {
            return Ok(fresh);
        }

        let manifest_url = format!("{}/{}", self.base_url, MANIFEST_FILE);
        let manifest_path = self.cache_dir.join(MANIFEST_FILE);
        let manifest_etag = self.cache_dir.join(format!("{MANIFEST_FILE}.etag"));

        let local_is_fresh =
            manifest_path.exists() && !is_stale(&manifest_path, self.staleness_ceiling);
        let mut req = self.http.get(&manifest_url);
        if local_is_fresh {
            if let Some(etag) = read_etag(&manifest_etag) {
                req = req.header("If-None-Match", etag);
            }
        }

        let fetched = match req.send().await {
            Ok(resp) if resp.status() == StatusCode::NOT_MODIFIED => {
                if let Err(e) = touch(&manifest_path) {
                    tracing::warn!("could not touch manifest mtime: {e}");
                }
                None
            }
            Ok(resp) if resp.status().is_success() => {
                let etag = resp
                    .headers()
                    .get("etag")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);
                match resp.json::<HashMap<String, String>>().await {
                    Ok(m) => Some((m, etag)),
                    Err(e) => {
                        tracing::warn!("manifest parse failed: {e}");
                        return self.use_or_fail_local_manifest(
                            &manifest_url,
                            &manifest_path,
                            &format!("manifest parse failure: {e}"),
                        );
                    }
                }
            }
            Ok(resp) => {
                let reason = format!(
                    "HTTP {} {}",
                    resp.status().as_u16(),
                    resp.status().canonical_reason().unwrap_or("")
                );
                return self.use_or_fail_local_manifest(&manifest_url, &manifest_path, &reason);
            }
            Err(e) => {
                let reason = format!("transport: {e}");
                return self.use_or_fail_local_manifest(&manifest_url, &manifest_path, &reason);
            }
        };

        if let Some((digests, etag)) = fetched {
            self.persist_fresh_manifest(
                &manifest_url,
                &manifest_path,
                &manifest_etag,
                digests,
                etag,
            )
            .await
        } else {
            // 304 path: load from disk so the in-memory copy is up to date.
            self.reload_local_manifest(&manifest_url, &manifest_path)
                .await
        }
    }

    /// Fast-path read against the in-memory manifest cache.
    async fn try_in_memory_manifest(&self) -> Option<ManifestCache> {
        let guard = self.manifest.lock().await;
        let cached = guard.as_ref()?;
        let age = SystemTime::now().duration_since(cached.mtime).ok()?;
        if age <= self.staleness_ceiling {
            let snapshot = ManifestCache {
                digests: cached.digests.clone(),
                mtime: cached.mtime,
            };
            drop(guard);
            Some(snapshot)
        } else {
            None
        }
    }

    /// Write a freshly-fetched manifest to disk and the in-memory cache.
    async fn persist_fresh_manifest(
        &self,
        manifest_url: &str,
        manifest_path: &Path,
        manifest_etag: &Path,
        digests: HashMap<String, String>,
        etag: Option<String>,
    ) -> Result<ManifestCache> {
        let raw = serde_json::to_vec(&digests).map_err(|e| Error::ManifestUnavailable {
            url: manifest_url.to_string(),
            reason: format!("re-serialise manifest: {e}"),
        })?;
        tokio::fs::create_dir_all(&self.cache_dir).await?;
        write_atomic_with_unique_tmp(manifest_path, &raw).await?;
        if let Some(e) = etag {
            write_atomic_with_unique_tmp(manifest_etag, e.as_bytes()).await?;
        }
        let entry = ManifestCache {
            digests,
            mtime: SystemTime::now(),
        };
        let mut guard = self.manifest.lock().await;
        *guard = Some(ManifestCache {
            digests: entry.digests.clone(),
            mtime: entry.mtime,
        });
        drop(guard);
        Ok(entry)
    }

    /// Reload the on-disk manifest into the in-memory cache (used on 304).
    async fn reload_local_manifest(
        &self,
        manifest_url: &str,
        manifest_path: &Path,
    ) -> Result<ManifestCache> {
        let raw = tokio::fs::read(manifest_path)
            .await
            .map_err(|e| Error::ManifestUnavailable {
                url: manifest_url.to_string(),
                reason: format!("local manifest read: {e}"),
            })?;
        let digests: HashMap<String, String> =
            serde_json::from_slice(&raw).map_err(|e| Error::ManifestUnavailable {
                url: manifest_url.to_string(),
                reason: format!("local manifest parse: {e}"),
            })?;
        let mtime = std::fs::metadata(manifest_path)
            .and_then(|m| m.modified())
            .unwrap_or_else(|_| SystemTime::now());
        let entry = ManifestCache { digests, mtime };
        let mut guard = self.manifest.lock().await;
        *guard = Some(ManifestCache {
            digests: entry.digests.clone(),
            mtime: entry.mtime,
        });
        drop(guard);
        Ok(entry)
    }

    fn use_or_fail_local_manifest(
        &self,
        manifest_url: &str,
        manifest_path: &Path,
        upstream_reason: &str,
    ) -> Result<ManifestCache> {
        if !manifest_path.exists() {
            return Err(Error::ManifestUnavailable {
                url: manifest_url.to_string(),
                reason: upstream_reason.to_string(),
            });
        }
        let raw = std::fs::read(manifest_path).map_err(|e| Error::ManifestUnavailable {
            url: manifest_url.to_string(),
            reason: format!("local manifest read: {e}"),
        })?;
        let digests: HashMap<String, String> =
            serde_json::from_slice(&raw).map_err(|e| Error::ManifestUnavailable {
                url: manifest_url.to_string(),
                reason: format!("local manifest parse: {e}"),
            })?;
        let mtime = std::fs::metadata(manifest_path)
            .and_then(|m| m.modified())
            .map_err(|e| Error::ManifestUnavailable {
                url: manifest_url.to_string(),
                reason: format!("local manifest mtime: {e}"),
            })?;
        let age = SystemTime::now()
            .duration_since(mtime)
            .unwrap_or(Duration::ZERO);
        if age > self.manifest_stale_ceiling {
            return Err(Error::ManifestStale {
                url: manifest_url.to_string(),
                age_secs: age.as_secs(),
                ceiling_secs: self.manifest_stale_ceiling.as_secs(),
            });
        }
        tracing::warn!(
            url = manifest_url,
            age_secs = age.as_secs(),
            upstream_reason,
            "manifest upstream unreachable; using stale local copy within fallback ceiling"
        );
        Ok(ManifestCache { digests, mtime })
    }
}

/// Clone an `Arc<Error>` back into a fresh owned `Error` for each waiter.
fn clone_error(e: &Arc<Error>) -> Error {
    match e.as_ref() {
        Error::ChecksumMismatch {
            file,
            expected,
            computed,
        } => Error::ChecksumMismatch {
            file: file.clone(),
            expected: expected.clone(),
            computed: computed.clone(),
        },
        Error::SchemaMismatch {
            file,
            expected,
            found,
        } => Error::SchemaMismatch {
            file: file.clone(),
            expected: expected.clone(),
            found: found.clone(),
        },
        Error::DataIntegrity {
            file,
            row,
            field,
            reason,
        } => Error::DataIntegrity {
            file: file.clone(),
            row: row.clone(),
            field: field.clone(),
            reason: reason.clone(),
        },
        Error::ManifestUnavailable { url, reason } => Error::ManifestUnavailable {
            url: url.clone(),
            reason: reason.clone(),
        },
        Error::ManifestStale {
            url,
            age_secs,
            ceiling_secs,
        } => Error::ManifestStale {
            url: url.clone(),
            age_secs: *age_secs,
            ceiling_secs: *ceiling_secs,
        },
        Error::BlockingFromCurrentThreadRuntime => Error::BlockingFromCurrentThreadRuntime,
        Error::UnknownSymbol(s) => Error::UnknownSymbol(s.clone()),
        Error::Source(s) => Error::Source(s.clone()),
        Error::Parquet(s) => Error::Parquet(s.clone()),
        // Variants that wrap non-clonable inner state (reqwest::Error, ...)
        // collapse into Other(message) with the discriminant preserved on the
        // Display line.
        other => Error::Other(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn backoff_delay_ms(attempt: u32) -> u64 {
    let raw = BACKOFF_BASE_MS.saturating_mul(1u64 << attempt.min(10));
    raw.min(BACKOFF_MAX_MS)
}

fn should_retry_status(status: StatusCode) -> bool {
    status.is_server_error()
}

fn is_retriable_error(e: &reqwest::Error) -> bool {
    e.is_connect() || e.is_timeout() || e.is_request()
}

fn retry_after_delay(resp: &reqwest::Response) -> Option<Duration> {
    let header = resp.headers().get("Retry-After")?;
    let val = header.to_str().ok()?;
    val.trim().parse::<u64>().ok().map(Duration::from_secs)
}

fn read_etag(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().filter(|s| !s.is_empty())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    hex_encode(&result)
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Whether `path`'s mtime is older than `ceiling` from now.
///
/// Returns `true` only when the mtime is reliably readable AND older than
/// the ceiling; on any error (missing file, no mtime support) returns
/// `false`, deferring to the rest of the fetch flow.
fn is_stale(path: &Path, ceiling: Duration) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(mtime) = meta.modified() else {
        return false;
    };
    let Ok(age) = SystemTime::now().duration_since(mtime) else {
        return false;
    };
    age > ceiling
}

/// Update `path`'s mtime to "now" by re-writing its contents (best-effort).
///
/// `set_modified` would be cheaper but is not stable across our MSRV;
/// this rewrite path uses stable `std::fs` APIs and a unique tmp
/// filename so concurrent touches cannot collide.
fn touch(path: &Path) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let bytes = std::fs::read(path)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_stem = path
        .file_name()
        .map_or_else(|| "cache".to_string(), |n| n.to_string_lossy().into_owned());
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0_u128, |d| d.as_nanos());
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = parent.join(format!("{file_stem}.touch.{pid}.{nanos}.{counter}"));
    std::fs::write(&tmp_path, &bytes)?;
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(())
}

/// Write `bytes` to `path` atomically with a unique tmp filename and
/// parent-directory fsync.
///
/// Sequence: open unique tmp -> write -> flush -> fsync(tmp) -> close ->
/// rename -> open parent dir -> fsync(parent) -> close. On any failure
/// mid-sequence the tmp file is removed (best effort) so a panic / kill
/// does not leave a permanent leak in the cache directory.
///
/// The tmp filename embeds process ID, thread ID, nanosecond timestamp,
/// and a per-call counter so concurrent writers cannot collide on the
/// same path. Parent fsync is best-effort: on filesystems that don't
/// support it (some FUSE mounts) the call is logged and ignored rather
/// than failing the whole write.
async fn write_atomic_with_unique_tmp(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::AsyncWriteExt as _;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let parent = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let file_stem = path
        .file_name()
        .map_or_else(|| "cache".to_string(), |n| n.to_string_lossy().into_owned());
    let pid = std::process::id();
    // ThreadId has no public numeric accessor on stable; hash it via the Debug
    // representation, which is stable enough for collision resistance.
    let tid = {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&std::thread::current().id(), &mut hasher);
        std::hash::Hasher::finish(&hasher)
    };
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0_u128, |d| d.as_nanos());
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!("{file_stem}.tmp.{pid}.{tid}.{nanos}.{counter}");
    let tmp_path = parent.join(tmp_name);

    let write_outcome: std::io::Result<()> = async {
        let mut f = tokio::fs::File::create(&tmp_path).await?;
        f.write_all(bytes).await?;
        f.flush().await?;
        f.sync_all().await?;
        drop(f);
        tokio::fs::rename(&tmp_path, path).await?;
        Ok(())
    }
    .await;

    if write_outcome.is_err() {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return write_outcome;
    }

    // Best-effort parent-directory fsync. Some platforms (notably
    // Windows) do not support opening a directory as a file; log and
    // continue rather than fail the whole write.
    match tokio::fs::File::open(&parent).await {
        Ok(dir) => {
            if let Err(e) = dir.sync_all().await {
                tracing::warn!(
                    parent = %parent.display(),
                    error = %e,
                    "parent-directory fsync failed (best-effort, continuing)"
                );
            }
        }
        Err(e) => {
            tracing::debug!(
                parent = %parent.display(),
                error = %e,
                "parent directory cannot be opened for fsync (best-effort, continuing)"
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Cache directory resolution
// ---------------------------------------------------------------------------

/// Resolve the cache directory.
///
/// Priority:
/// 1. `$HOURSKIT_CACHE_DIR` env var.
/// 2. XDG/platform cache dir for the `hourskit` application
///    (`directories::ProjectDirs`).
/// 3. Fallback: `~/.cache/hourskit` (or `%LOCALAPPDATA%\hourskit\cache` on Windows).
pub(crate) fn default_cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("HOURSKIT_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(proj) = directories::ProjectDirs::from("", "", "hourskit") {
        return proj.cache_dir().to_path_buf();
    }
    dirs_fallback()
}

fn dirs_fallback() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("LOCALAPPDATA")
            .map(|d| PathBuf::from(d).join("hourskit").join("cache"))
            .unwrap_or_else(|_| PathBuf::from("hourskit-cache"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME").map_or_else(
            |_| PathBuf::from(".hourskit-cache"),
            |h| PathBuf::from(h).join(".cache").join("hourskit"),
        )
    }
}

/// Default primary base URL (GitHub raw content).
pub(crate) const DEFAULT_BASE_URL: &str =
    "https://raw.githubusercontent.com/userFRM/hourskit/main/data";

/// Default CDN mirror (jsDelivr — Cloudflare-fronted mirror of the GitHub repo).
pub(crate) const DEFAULT_MIRROR_URL: &str =
    "https://cdn.jsdelivr.net/gh/userFRM/hourskit@main/data";

/// Resolve the base URL from the environment or use the default.
pub(crate) fn resolved_base_url() -> String {
    std::env::var("HOURSKIT_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_progression() {
        assert_eq!(backoff_delay_ms(0), 250);
        assert_eq!(backoff_delay_ms(1), 500);
        assert_eq!(backoff_delay_ms(2), 1000);
        assert_eq!(backoff_delay_ms(3), 2000);
        assert_eq!(backoff_delay_ms(10), 2000);
    }

    #[test]
    fn hex_sha256_known_value() {
        let digest = hex_sha256(b"");
        assert_eq!(
            digest,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hex_sha256_hello() {
        let digest = hex_sha256(b"hello world");
        assert_eq!(
            digest,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    /// Mount an empty `/manifest.json` mock on `server`. Used by tests
    /// that don't care about digest enforcement; with no entry for the
    /// key, the fetcher skips checksum verification and proceeds.
    async fn mount_empty_manifest(server: &wiremock::MockServer) {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"{}"))
            .mount(server)
            .await;
    }

    /// Disabling the mirror with `set_mirror_url(None)` causes primary 503 to
    /// propagate without falling back to a CDN.
    #[tokio::test]
    async fn with_mirror_url_none_skips_fallback() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let primary = MockServer::start().await;
        let mirror_sentinel = MockServer::start().await;
        mount_empty_manifest(&primary).await;

        Mock::given(method("GET"))
            .and(path("/sessions.parquet"))
            .respond_with(ResponseTemplate::new(503))
            .expect(3)
            .mount(&primary)
            .await;

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"irrelevant"))
            .expect(0)
            .mount(&mirror_sentinel)
            .await;

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("build reqwest client");
        let cache_dir = tempfile::TempDir::new().expect("tmpdir");
        let mut fetcher = CachedFetcher::new(http, primary.uri(), cache_dir.path().to_path_buf());
        fetcher.set_mirror_url(None);

        let result = fetcher.fetch("sessions").await;
        assert!(
            result.is_err(),
            "primary 503 + no mirror must propagate error"
        );
    }

    /// A custom mirror is consulted on primary failure and its bytes are
    /// returned.
    #[tokio::test]
    async fn with_mirror_url_custom_used() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let primary = MockServer::start().await;
        let custom_mirror = MockServer::start().await;
        mount_empty_manifest(&primary).await;

        Mock::given(method("GET"))
            .and(path("/sessions.parquet"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&primary)
            .await;

        Mock::given(method("GET"))
            .and(path("/sessions.parquet"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fake-parquet"))
            .expect(1)
            .mount(&custom_mirror)
            .await;

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("build reqwest client");
        let cache_dir = tempfile::TempDir::new().expect("tmpdir");
        let mut fetcher = CachedFetcher::new(http, primary.uri(), cache_dir.path().to_path_buf());
        fetcher.set_mirror_url(Some(custom_mirror.uri()));

        let result = fetcher.fetch("sessions").await;
        assert!(result.is_ok());
        assert_eq!(result.expect("unwrap").as_ref(), b"fake-parquet");
    }

    /// On 200 the body is written to the cache and the cache file contents
    /// match the response.
    #[tokio::test]
    async fn cache_is_populated_on_success() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let primary = MockServer::start().await;
        mount_empty_manifest(&primary).await;
        Mock::given(method("GET"))
            .and(path("/sessions.parquet"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello-cache"))
            .mount(&primary)
            .await;

        let http = reqwest::Client::builder().build().expect("build");
        let cache_dir = tempfile::TempDir::new().expect("tmpdir");
        let mut fetcher = CachedFetcher::new(http, primary.uri(), cache_dir.path().to_path_buf());
        fetcher.set_mirror_url(None);

        let bytes = fetcher.fetch("sessions").await.expect("fetch ok");
        assert_eq!(bytes.as_ref(), b"hello-cache");

        let on_disk = std::fs::read(cache_dir.path().join("sessions.parquet")).expect("read cache");
        assert_eq!(on_disk.as_slice(), b"hello-cache");
    }

    /// When the manifest cannot be obtained AND no local manifest exists,
    /// `Error::ManifestUnavailable` propagates instead of degrading to an
    /// empty digest map.
    #[tokio::test]
    async fn missing_manifest_with_no_local_cache_fails_loudly() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/sessions.parquet"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"unverified"))
            .expect(0)
            .mount(&server)
            .await;

        let http = reqwest::Client::builder().build().expect("build");
        let cache_dir = tempfile::TempDir::new().expect("tmpdir");
        let mut fetcher = CachedFetcher::new(http, server.uri(), cache_dir.path().to_path_buf());
        fetcher.set_mirror_url(None);

        let result = fetcher.fetch("sessions").await;
        assert!(
            matches!(result, Err(Error::ManifestUnavailable { .. })),
            "expected ManifestUnavailable, got {result:?}"
        );
    }

    /// When upstream is unreachable AND a local cached manifest exists
    /// within the stale-manifest ceiling, the fetcher uses the stale
    /// copy and continues.
    #[tokio::test]
    async fn stale_local_manifest_within_ceiling_is_reused() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/sessions.parquet"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fake-parquet"))
            .mount(&server)
            .await;

        let http = reqwest::Client::builder().build().expect("build");
        let cache_dir = tempfile::TempDir::new().expect("tmpdir");
        // Seed a local manifest mapping sessions.parquet to the actual
        // SHA-256 of "fake-parquet".
        let digest = hex_sha256(b"fake-parquet");
        let body = format!("{{\"sessions.parquet\":\"sha256:{digest}\"}}");
        std::fs::write(cache_dir.path().join("manifest.json"), body).expect("seed manifest");
        let mut fetcher = CachedFetcher::new(http, server.uri(), cache_dir.path().to_path_buf());
        fetcher.set_mirror_url(None);
        let bytes = fetcher
            .fetch("sessions")
            .await
            .expect("stale-manifest reuse");
        assert_eq!(bytes.as_ref(), b"fake-parquet");
    }

    /// When the local manifest is older than `manifest_stale_ceiling`,
    /// the fetcher returns `Error::ManifestStale` rather than serving
    /// unverified bytes.
    #[tokio::test]
    async fn stale_local_manifest_beyond_ceiling_fails() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/sessions.parquet"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fake-parquet"))
            .expect(0)
            .mount(&server)
            .await;

        let http = reqwest::Client::builder().build().expect("build");
        let cache_dir = tempfile::TempDir::new().expect("tmpdir");
        std::fs::write(cache_dir.path().join("manifest.json"), b"{}").expect("seed manifest");
        // Sleep so the local manifest mtime is unambiguously past the
        // tiny stale ceiling we configure below.
        tokio::time::sleep(Duration::from_millis(10)).await;

        let mut fetcher = CachedFetcher::new(http, server.uri(), cache_dir.path().to_path_buf());
        fetcher.set_mirror_url(None);
        fetcher.set_manifest_stale_ceiling(Duration::from_millis(1));

        let result = fetcher.fetch("sessions").await;
        assert!(
            matches!(result, Err(Error::ManifestStale { .. })),
            "expected ManifestStale, got {result:?}"
        );
    }

    /// Checksum mismatch must NOT clobber the prior good cache file; the
    /// kit explicitly preserves it for the next call.
    #[tokio::test]
    async fn checksum_mismatch_keeps_prior_good_cache() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // Manifest claims the digest of "good-bytes" but server serves
        // "corrupt-bytes" instead.
        let good_digest = hex_sha256(b"good-bytes");
        let manifest_body = format!("{{\"sessions.parquet\":\"sha256:{good_digest}\"}}");
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(manifest_body.into_bytes()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/sessions.parquet"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"corrupt-bytes"))
            .mount(&server)
            .await;

        let http = reqwest::Client::builder().build().expect("build");
        let cache_dir = tempfile::TempDir::new().expect("tmpdir");
        // Seed a prior good cache file.
        std::fs::write(cache_dir.path().join("sessions.parquet"), b"prior-good")
            .expect("seed prior cache");

        let mut fetcher = CachedFetcher::new(http, server.uri(), cache_dir.path().to_path_buf());
        fetcher.set_mirror_url(None);

        let result = fetcher.fetch("sessions").await;
        assert!(matches!(result, Err(Error::ChecksumMismatch { .. })));

        let on_disk = std::fs::read(cache_dir.path().join("sessions.parquet"))
            .expect("prior cache must still exist");
        assert_eq!(on_disk.as_slice(), b"prior-good");
    }

    /// is_stale returns true once the cache file's mtime is older than the
    /// ceiling.
    #[test]
    fn is_stale_flips_when_cache_exceeds_ceiling() {
        let dir = tempfile::TempDir::new().expect("tmpdir");
        let path = dir.path().join("cache.parquet");
        std::fs::write(&path, b"stale-bytes").expect("seed");

        // Fresh: ceiling is 60 s, file just written.
        assert!(!is_stale(&path, Duration::from_secs(60)));

        // With ceiling = ZERO any non-zero age is stale (after a tiny delay
        // to ensure the mtime is in the past).
        std::thread::sleep(Duration::from_millis(5));
        assert!(is_stale(&path, Duration::from_millis(1)));
    }

    /// 24h ceiling forces a fresh body even when the ETag would 304.
    /// Implementation observed: when the cache mtime is older than the
    /// ceiling, the request must NOT carry If-None-Match.
    #[tokio::test]
    async fn staleness_ceiling_forces_fresh_fetch() {
        use wiremock::matchers::{header_exists, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        mount_empty_manifest(&server).await;

        // Any request with If-None-Match should NOT happen on the second
        // call because we will mark the cache as stale. We still mount a
        // 200 success path so the fetcher can succeed.
        Mock::given(method("GET"))
            .and(path("/sessions.parquet"))
            .and(header_exists("If-None-Match"))
            .respond_with(ResponseTemplate::new(304))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/sessions.parquet"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("ETag", "\"v1\"")
                    .set_body_bytes(b"fresh-bytes"),
            )
            .mount(&server)
            .await;

        let http = reqwest::Client::builder().build().expect("build");
        let cache_dir = tempfile::TempDir::new().expect("tmpdir");
        let mut fetcher = CachedFetcher::new(http, server.uri(), cache_dir.path().to_path_buf());
        fetcher.set_mirror_url(None);
        // Hard zero ceiling: every cache lookup must be considered stale.
        fetcher.set_staleness_ceiling(Duration::from_millis(0));

        // Seed a cache file + ETag manually so the fetcher SEES a cache.
        std::fs::write(cache_dir.path().join("sessions.parquet"), b"old").expect("seed cache");
        std::fs::write(cache_dir.path().join("sessions.parquet.etag"), "\"v1\"")
            .expect("seed etag");

        // Sleep so the cache file is unambiguously older than 1 ms.
        tokio::time::sleep(Duration::from_millis(5)).await;

        let bytes = fetcher.fetch("sessions").await.expect("fresh fetch");
        assert_eq!(bytes.as_ref(), b"fresh-bytes");
    }
}
