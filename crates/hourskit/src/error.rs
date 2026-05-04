//! Unified error type for hourskit.
//!
//! All public methods return `hourskit::Result<T>` which is
//! `std::result::Result<T, hourskit::Error>`.

use thiserror::Error;

/// The single unified error type for hourskit operations.
///
/// Match on this enum when you need to distinguish error kinds; otherwise
/// `?` propagates it through any `Result<_, hourskit::Error>` context.
#[derive(Debug, Error)]
pub enum Error {
    /// The requested symbol root is not present in the seed data.
    #[error("unknown root: {0}")]
    UnknownRoot(String),

    /// Generic data-source error (used by upstream fetcher transports).
    #[error("source error: {0}")]
    Source(String),

    /// Parquet file read or write failed (semantic error wrapper).
    #[error("parquet I/O error: {0}")]
    Parquet(String),

    /// A parquet file's schema does not match the expected layout.
    ///
    /// Returned by every parquet reader when the column names, types,
    /// or per-column nullability diverge from the documented schema.
    #[error("parquet schema mismatch in {file}: expected {expected}, found {found}")]
    SchemaMismatch {
        /// Logical filename that mismatched (e.g. `sessions.parquet`).
        file: String,
        /// Expected schema description.
        expected: String,
        /// Actual schema observed in the parquet file.
        found: String,
    },

    /// A parquet row contains a logically inconsistent payload — for
    /// example a window with one endpoint set and the other missing.
    ///
    /// The parquet schema cannot encode pairwise constraints, so
    /// `read_sessions` enforces them at decode time. Distinct from
    /// [`Self::SchemaMismatch`], which is a structural problem with the
    /// file itself.
    #[error("data integrity violation in {file} row {row}, field {field}: {reason}")]
    DataIntegrity {
        /// Logical filename that contained the bad row.
        file: String,
        /// Root or row identifier (whichever is more useful for the operator).
        row: String,
        /// Field whose constraint was violated (e.g. `pre_market`, `gth`).
        field: String,
        /// Free-text explanation suitable for an error log.
        reason: String,
    },

    /// Underlying HTTP transport error.
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    /// I/O error (file system, tempfile, etc.).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Arrow columnar format error (from parquet reading).
    #[error("arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    /// Native parquet crate error.
    #[error("parquet error: {0}")]
    ParquetNative(#[from] parquet::errors::ParquetError),

    /// SHA-256 digest of a fetched file does not match the manifest entry.
    ///
    /// The corrupt bytes were NOT written to the on-disk cache; the
    /// prior good cache file (if any) is left intact for the next call.
    #[error("checksum mismatch for {file}: expected sha256:{expected} got sha256:{computed}")]
    ChecksumMismatch {
        /// Logical filename (e.g. `sessions.parquet`) that mismatched.
        file: String,
        /// Expected SHA-256 digest from the manifest, hex-encoded.
        expected: String,
        /// Computed SHA-256 digest of the fetched bytes, hex-encoded.
        computed: String,
    },

    /// The manifest could not be obtained and no usable manifest exists
    /// in the local cache. Fetching reference data would mean serving
    /// unverified bytes; the kit refuses.
    ///
    /// Distinct from [`Self::ManifestStale`], which fires when a
    /// previously-fetched manifest exists but is older than the
    /// configured fallback ceiling.
    #[error("manifest unavailable for {url}: {reason}")]
    ManifestUnavailable {
        /// Manifest URL that failed.
        url: String,
        /// Free-text explanation (HTTP status, parse error, ...).
        reason: String,
    },

    /// The local manifest has aged past the stale-manifest ceiling
    /// (default 7 days) and the upstream is unreachable for a fresh
    /// copy. Distinct from [`Self::ManifestUnavailable`] — a manifest
    /// did exist locally, but the fallback window has expired.
    #[error("manifest stale for {url}: cache age {age_secs}s exceeds {ceiling_secs}s")]
    ManifestStale {
        /// Manifest URL whose cache aged out.
        url: String,
        /// Observed age of the cached manifest, in seconds.
        age_secs: u64,
        /// Stale-manifest ceiling, in seconds.
        ceiling_secs: u64,
    },

    /// A `*_blocking` method was called from inside a tokio
    /// current-thread runtime. `block_in_place` is a no-op there, so
    /// driving a future to completion would deadlock; the kit refuses
    /// instead of risking a hang.
    ///
    /// Use the async variant of the same method.
    #[error(
        "blocking call from current-thread tokio runtime is unsupported; use the async variant"
    )]
    BlockingFromCurrentThreadRuntime,

    /// JSON serialization or deserialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// Time-unit string failed to parse (`us` / `ms` / `s` only).
    #[error("time-unit parse: {0}")]
    TimeUnitParse(#[from] crate::session::ParseTimeUnitError),

    /// Any other error not covered by the specific variants above.
    #[error("{0}")]
    Other(String),
}

/// `Result<T>` alias using [`enum@Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Alias for [`enum@Error`] kept for forward-compatibility with downstream
/// crates that prefer the prefixed name.
pub type HourskitError = Error;
