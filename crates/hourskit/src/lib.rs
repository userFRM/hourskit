//! `hourskit` — exchange trading-hours service for Rust.
//!
//! Ships a single parquet table (`sessions.parquet`) covering the full slate
//! of US exchange session windows scoped per `(symbol, trading_class)`:
//!
//! - **Cboe options** — C1 cash-settled index options (SPX/VIX/XSP/RUT family,
//!   16:15 ET close, Curb 16:15-17:00, GTH 20:15 prior to 09:25), Cboe
//!   BZX/C2/EDGX option family.
//! - **Other options** — NYSE Arca, ISE, BOX, NYSE American.
//! - **US equities** — Nasdaq Stock Market (regular session + 04:00-09:30 pre +
//!   16:00-20:00 post + NEW 21:00-04:00 ET Extended Session, the Nasdaq
//!   Global Trading Hours program announced for 2026), NYSE Arca, Cboe BZX/EDGX
//!   (early-trading from 02:30 ET), Cboe BYX/EDGA.
//!
//! Storage is microsecond-native: every endpoint stores `i64` microseconds
//! since midnight (US/Eastern); accessor methods convert to milliseconds or
//! seconds at the API surface based on the caller's [`TimeUnit`] choice.
//!
//! # Quick start
//!
//! ```no_run
//! #[tokio::main]
//! async fn main() -> hourskit::Result<()> {
//!     // Free function — no client setup needed.
//!     let info = hourskit::session("SPX").await?.expect("SPX in seed");
//!     println!("SPX regular: {}", info.regular);
//!     println!("SPX class:   {}", info.trading_class);
//!     Ok(())
//! }
//! ```
//!
//! For repeated lookups, create a [`Hourskit`] client once and reuse it so the
//! HTTP connection pool and parquet cache are shared across calls.
//!
//! # Major types
//!
//! - [`Hourskit`] — stateful client; create once, call many times.
//! - [`SessionInfo`] — per-symbol slate of session windows.
//! - [`TimeWindow`] — half-open `[open, close)` window in microseconds-of-day (ET).
//! - [`TimeUnit`] — caller-chosen unit (`Microseconds` / `Milliseconds` / `Seconds`).
//! - [`TradingClass`] — venue / asset-class identifier.
//! - [`Error`] — unified error type; match on this, never on sub-types.
//!
//! # Reference data lifecycle
//!
//! `hourskit` ships `data/sessions.parquet` inside the published crate
//! and mirrors the same file at the public GitHub raw URL
//! `https://raw.githubusercontent.com/userFRM/hourskit/main/data/sessions.parquet`.
//! The stateful client ([`Hourskit`]) prefers the GitHub copy: first
//! call fetches into `~/.cache/hourskit/`, subsequent calls revalidate
//! via ETag (`If-None-Match`) within a 24-hour window then refetch
//! unconditionally. SHA-256 verified against `data/manifest.json` on
//! every read. The bundled copy is a fallback for offline / no-network
//! deployments — read via [`sources::bundled`].
//!
//! # Environment overrides
//!
//! | Variable | Effect |
//! |---|---|
//! | `HOURSKIT_BASE_URL`   | Replace the GitHub raw origin URL |
//! | `HOURSKIT_CACHE_DIR`  | Override `~/.cache/hourskit/` |
//! | `HOURSKIT_MIRROR_URL` | CDN fallback URL (default: jsDelivr) |
//! | `HOURSKIT_DATA_DIR`   | Override the local `data/` directory used by the bundled reader |

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(unreachable_pub)]
#![deny(unused_must_use)]
#![deny(rustdoc::broken_intra_doc_links)]

#[cfg(feature = "parquet-loader")]
pub mod client;
pub mod error;
#[cfg(feature = "parquet-loader")]
pub(crate) mod fetcher;
pub mod session;
pub mod sources;
pub mod time;

// ── Top-level re-exports ──────────────────────────────────────────────────────

#[cfg(feature = "parquet-loader")]
pub use client::Hourskit;
pub use error::{Error, HourskitError, Result};
pub use session::{
    is_third_friday, ParseTimeUnitError, SessionInfo, Settlement, TimeUnit, TimeWindow,
    TradingClass,
};

// ── Free-function shortcut ────────────────────────────────────────────────────
//
// One-shot use: a process-wide `Hourskit` instance is shared so multiple calls
// reuse the HTTP client and cache.

#[cfg(feature = "parquet-loader")]
use std::sync::OnceLock;

#[cfg(feature = "parquet-loader")]
fn global_client() -> &'static Hourskit {
    static CLIENT: OnceLock<Hourskit> = OnceLock::new();
    CLIENT.get_or_init(Hourskit::new)
}

/// Look up the [`SessionInfo`] for `symbol` via the process-wide client.
///
/// Returns `Ok(None)` if no row matches. Lookup is case-insensitive.
///
/// # Errors
///
/// Propagates HTTP / parquet errors from the underlying fetcher.
#[cfg(feature = "parquet-loader")]
pub async fn session(symbol: impl AsRef<str>) -> Result<Option<SessionInfo>> {
    global_client().session(symbol).await
}

/// Look up the [`SessionInfo`] for `symbol` applicable on trading date
/// `date` (`YYYYMMDD`) via the process-wide client.
///
/// Returns `Ok(None)` if no row matches. Resolves the effective-dated row
/// per [`SessionInfo::valid_from_yyyymmdd`]: the greatest
/// `valid_from_yyyymmdd <= date` (a `None` baseline always eligible).
///
/// # Errors
///
/// Propagates HTTP / parquet errors from the underlying fetcher.
#[cfg(feature = "parquet-loader")]
pub async fn session_on(symbol: impl AsRef<str>, date: i32) -> Result<Option<SessionInfo>> {
    global_client().session_on(symbol, date).await
}

/// Blocking variant of [`session()`].
///
/// # Errors
///
/// Propagates HTTP / parquet errors from the underlying fetcher.
#[cfg(feature = "parquet-loader")]
pub fn session_blocking(symbol: impl AsRef<str>) -> Result<Option<SessionInfo>> {
    global_client().session_blocking(symbol)
}

/// Blocking variant of [`session_on()`].
///
/// # Errors
///
/// Propagates HTTP / parquet errors from the underlying fetcher.
#[cfg(feature = "parquet-loader")]
pub fn session_on_blocking(symbol: impl AsRef<str>, date: i32) -> Result<Option<SessionInfo>> {
    global_client().session_on_blocking(symbol, date)
}
