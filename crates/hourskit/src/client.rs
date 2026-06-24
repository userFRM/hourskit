//! Stateful `Hourskit` client — single `session(symbol)` endpoint.
//!
//! Fetches `sessions.parquet` from GitHub raw (or a configurable origin)
//! with an XDG-compliant local cache + ETag revalidation. Falls back to
//! stale cache on network errors so existing workflows survive transient
//! outages, and forces a fresh body once the cache is older than the
//! configured staleness ceiling (24 hours by default).
//!
//! # Example
//!
//! ```no_run
//! # async fn run() -> hourskit::Result<()> {
//! use hourskit::Hourskit;
//!
//! let client = Hourskit::new();
//! let info = client.session("SPX").await?.expect("SPX in seed data");
//! assert!(info.curb.is_some());
//! assert!(info.gth.is_some());
//! # Ok(()) }
//! ```

use std::path::PathBuf;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::fetcher::{default_cache_dir, resolved_base_url, CachedFetcher};
use crate::session::SessionInfo;
use crate::sources::parquet_io::read_sessions;

/// User-Agent header sent on every HTTP request.
const USER_AGENT: &str = concat!(
    "hourskit/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/userFRM/hourskit)"
);

/// Default HTTP timeout, in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Logical key used by the fetcher (the parquet file is `sessions.parquet`).
const FETCH_KEY: &str = "sessions";

/// Stateful hourskit client.
///
/// Wraps an ETag-aware cached fetcher and exposes a single
/// [`session(symbol)`][Self::session] endpoint. Create once and reuse across
/// calls; the internal reqwest client is kept alive for connection pooling.
///
/// # Infallible construction
///
/// ```no_run
/// use hourskit::Hourskit;
/// let client = Hourskit::new();   // never fails
/// ```
///
/// # Builder pattern
///
/// ```no_run
/// use hourskit::Hourskit;
/// use std::path::PathBuf;
/// use std::time::Duration;
///
/// let client = Hourskit::new()
///     .with_base_url("https://my-mirror.example.com/hourskit")
///     .with_cache_dir(PathBuf::from("/tmp/hourskit-test"))
///     .with_staleness_ceiling(Duration::from_secs(60 * 60));
/// ```
pub struct Hourskit {
    fetcher: CachedFetcher,
}

impl Hourskit {
    /// Create a client with the default GitHub raw backend and XDG cache.
    ///
    /// Reads `HOURSKIT_BASE_URL` and `HOURSKIT_CACHE_DIR` from the
    /// environment if set, otherwise uses the GitHub raw origin and
    /// `~/.cache/hourskit/`.
    ///
    /// **This function never fails.** If the underlying HTTP client cannot
    /// be built (essentially only on exotic platforms with broken TLS), a
    /// default reqwest client is used and the error is deferred to the first
    /// fetch call. Use [`try_new`][Self::try_new] for early detection.
    #[must_use]
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|err| {
                tracing::warn!(error = %err, "reqwest build failed; using default client");
                reqwest::Client::new()
            });
        Self {
            fetcher: CachedFetcher::new(http, resolved_base_url(), default_cache_dir()),
        }
    }

    /// Create a client with early failure detection.
    ///
    /// Like [`new`][Self::new] but returns an error immediately if the HTTP
    /// client cannot be constructed.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the underlying reqwest client cannot be
    /// constructed (TLS init failure — essentially never in practice).
    pub fn try_new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()?;
        Ok(Self {
            fetcher: CachedFetcher::new(http, resolved_base_url(), default_cache_dir()),
        })
    }

    /// Override the origin URL.
    ///
    /// Default: `https://raw.githubusercontent.com/userFRM/hourskit/main/data`.
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.fetcher.set_base_url(url.into());
        self
    }

    /// Override the on-disk cache directory.
    ///
    /// Default: `~/.cache/hourskit/`.
    #[must_use]
    pub fn with_cache_dir(mut self, dir: PathBuf) -> Self {
        self.fetcher.set_cache_dir(dir);
        self
    }

    /// Override the CDN mirror URL used when the primary fetch fails.
    ///
    /// Default: jsDelivr CDN mirror of this repo. Pass `None` to disable
    /// mirror fallback entirely (useful in tests).
    #[must_use]
    pub fn with_mirror_url(mut self, url: Option<String>) -> Self {
        self.fetcher.set_mirror_url(url);
        self
    }

    /// Override the cache staleness ceiling.
    ///
    /// Default: 24 hours. After this duration the fetcher forces a fresh
    /// body even if the ETag would otherwise return 304. Pass
    /// `Duration::ZERO` to force every fetch to hit the network (useful in
    /// tests).
    #[must_use]
    pub fn with_staleness_ceiling(mut self, ceiling: Duration) -> Self {
        self.fetcher.set_staleness_ceiling(ceiling);
        self
    }

    /// Look up the [`SessionInfo`] for `symbol`.
    ///
    /// Returns `Ok(None)` when no row matches; lookup is case-insensitive
    /// on the symbol. When a symbol resolves to multiple trading classes,
    /// the row with the lowest [`crate::TradingClass::preference_rank`] is
    /// returned. Use [`session_for_class`][Self::session_for_class] to pin
    /// a specific class.
    ///
    /// When a `(symbol, trading_class)` carries multiple effective-dated
    /// rows (see [`SessionInfo::valid_from_yyyymmdd`]), the LATEST effective
    /// row — the newest active session — is returned. Use
    /// [`session_on`][Self::session_on] to resolve the row applicable on a
    /// specific query date.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] on HTTP / parquet / verification failure.
    pub async fn session(&self, symbol: impl AsRef<str>) -> Result<Option<SessionInfo>> {
        self.session_on(symbol, crate::sources::bundled::VALID_FROM_LATEST)
            .await
    }

    /// Look up the [`SessionInfo`] for `symbol` applicable on trading date
    /// `date` (`YYYYMMDD`).
    ///
    /// Returns `Ok(None)` when no row matches. Among the effective-dated
    /// rows for a `(symbol, trading_class)` the applicable row is the one
    /// with the greatest `valid_from_yyyymmdd <= date`, treating a `None`
    /// baseline as always eligible (see
    /// [`SessionInfo::valid_from_yyyymmdd`]). Symbols carrying only a
    /// baseline row resolve identically for every `date`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] on HTTP / parquet / verification failure.
    pub async fn session_on(
        &self,
        symbol: impl AsRef<str>,
        date: i32,
    ) -> Result<Option<SessionInfo>> {
        let needle = symbol.as_ref().to_ascii_uppercase();
        let rows = self.sessions_all().await?;
        Ok(crate::sources::bundled::resolve_on(rows, &needle, date))
    }

    /// Look up the [`SessionInfo`] for a specific `(symbol, trading_class)` pair.
    ///
    /// When the pair carries multiple effective-dated rows, the LATEST
    /// effective row is returned. Use
    /// [`session_for_class_on`][Self::session_for_class_on] to resolve the
    /// row applicable on a specific query date.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] on HTTP / parquet / verification failure.
    pub async fn session_for_class(
        &self,
        symbol: impl AsRef<str>,
        trading_class: &crate::TradingClass,
    ) -> Result<Option<SessionInfo>> {
        self.session_for_class_on(
            symbol,
            trading_class,
            crate::sources::bundled::VALID_FROM_LATEST,
        )
        .await
    }

    /// Look up the [`SessionInfo`] for a specific `(symbol, trading_class)`
    /// pair applicable on trading date `date` (`YYYYMMDD`).
    ///
    /// Resolves the effective-dated row within the class (greatest
    /// `valid_from_yyyymmdd <= date`, `None` baseline always eligible).
    ///
    /// # Errors
    ///
    /// Returns [`Error`] on HTTP / parquet / verification failure.
    pub async fn session_for_class_on(
        &self,
        symbol: impl AsRef<str>,
        trading_class: &crate::TradingClass,
        date: i32,
    ) -> Result<Option<SessionInfo>> {
        let needle = symbol.as_ref().to_ascii_uppercase();
        let target_wire = trading_class.as_wire();
        let rows = self.sessions_all().await?;
        Ok(crate::sources::bundled::resolve_for_class_on(
            rows,
            &needle,
            &target_wire,
            date,
        ))
    }

    /// Read every row of the bundled session table.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] on HTTP / parquet / verification failure.
    pub async fn sessions_all(&self) -> Result<Vec<SessionInfo>> {
        let bytes = self.fetcher.fetch(FETCH_KEY).await?;
        let tmp = tempfile_for_bytes(&bytes)?;
        read_sessions(tmp.path())
    }

    // ── Blocking wrappers ─────────────────────────────────────────────────────

    /// Blocking variant of [`session`][Self::session].
    ///
    /// Works from sync code and from a `#[tokio::main]` /
    /// `multi_thread`-flavored runtime. Calls from inside a tokio
    /// `current_thread` runtime return
    /// [`Error::BlockingFromCurrentThreadRuntime`] — use the async
    /// variant in that context.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] on HTTP / parquet / verification failure, or
    /// [`Error::BlockingFromCurrentThreadRuntime`] when invoked from a
    /// tokio current-thread runtime.
    pub fn session_blocking(&self, symbol: impl AsRef<str>) -> Result<Option<SessionInfo>> {
        let needle = symbol.as_ref().to_string();
        block(self.session(needle))
    }

    /// Blocking variant of [`session_on`][Self::session_on].
    ///
    /// Same runtime constraint as [`session_blocking`][Self::session_blocking].
    ///
    /// # Errors
    ///
    /// Returns [`Error`] on HTTP / parquet / verification failure, or
    /// [`Error::BlockingFromCurrentThreadRuntime`] when invoked from a
    /// tokio current-thread runtime.
    pub fn session_on_blocking(
        &self,
        symbol: impl AsRef<str>,
        date: i32,
    ) -> Result<Option<SessionInfo>> {
        let needle = symbol.as_ref().to_string();
        block(self.session_on(needle, date))
    }

    /// Blocking variant of [`sessions_all`][Self::sessions_all].
    ///
    /// Same runtime constraint as [`session_blocking`][Self::session_blocking].
    ///
    /// # Errors
    ///
    /// Returns [`Error`] on HTTP / parquet / verification failure, or
    /// [`Error::BlockingFromCurrentThreadRuntime`] when invoked from a
    /// tokio current-thread runtime.
    pub fn sessions_all_blocking(&self) -> Result<Vec<SessionInfo>> {
        block(self.sessions_all())
    }

    /// Blocking variant of [`session_for_class`][Self::session_for_class].
    ///
    /// Same runtime constraint as [`session_blocking`][Self::session_blocking].
    ///
    /// # Errors
    ///
    /// Returns [`Error`] on HTTP / parquet / verification failure, or
    /// [`Error::BlockingFromCurrentThreadRuntime`] when invoked from a
    /// tokio current-thread runtime.
    pub fn session_for_class_blocking(
        &self,
        symbol: impl AsRef<str>,
        trading_class: &crate::TradingClass,
    ) -> Result<Option<SessionInfo>> {
        let needle = symbol.as_ref().to_string();
        let class = trading_class.clone();
        block(async move { self.session_for_class(needle, &class).await })
    }

    /// Blocking variant of [`session_for_class_on`][Self::session_for_class_on].
    ///
    /// Same runtime constraint as [`session_blocking`][Self::session_blocking].
    ///
    /// # Errors
    ///
    /// Returns [`Error`] on HTTP / parquet / verification failure, or
    /// [`Error::BlockingFromCurrentThreadRuntime`] when invoked from a
    /// tokio current-thread runtime.
    pub fn session_for_class_on_blocking(
        &self,
        symbol: impl AsRef<str>,
        trading_class: &crate::TradingClass,
        date: i32,
    ) -> Result<Option<SessionInfo>> {
        let needle = symbol.as_ref().to_string();
        let class = trading_class.clone();
        block(async move { self.session_for_class_on(needle, &class, date).await })
    }
}

impl Default for Hourskit {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Blocking helper
// ---------------------------------------------------------------------------

/// Drive a future to completion from any context (sync or async).
///
/// - **Outside any runtime** — spin up a minimal current-thread runtime
///   and drive the future on it.
/// - **Inside a `multi_thread` runtime** — call `block_in_place` +
///   `Handle::block_on` so the worker yields rather than deadlocking the
///   pool.
/// - **Inside a `current_thread` runtime** — return
///   [`Error::BlockingFromCurrentThreadRuntime`]. `block_in_place`
///   panics there, and re-entering the same runtime would deadlock.
///   Callers in this configuration must use the async variant.
fn block<F: std::future::Future<Output = Result<T>> + Send, T: Send>(fut: F) -> Result<T> {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return match handle.runtime_flavor() {
            tokio::runtime::RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(|| handle.block_on(fut))
            }
            // `CurrentThread` (and any future single-thread flavor) — we
            // cannot drive a future from inside the very runtime that
            // would have to poll it. Refuse loudly.
            _ => Err(Error::BlockingFromCurrentThreadRuntime),
        };
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(Error::Io)?;
    rt.block_on(fut)
}

/// Write bytes to a named temp file and return the [`tempfile::NamedTempFile`].
fn tempfile_for_bytes(bytes: &bytes::Bytes) -> Result<tempfile::NamedTempFile> {
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new()?;
    tmp.write_all(bytes)?;
    tmp.flush()?;
    Ok(tmp)
}
