//! Offline-first reader for the bundled `sessions.parquet`.
//!
//! Reads from disk only, never touches the network. Used by:
//! - the CLI (synchronous, no async runtime needed),
//! - the seeded test suite,
//! - downstream callers that want deterministic ship-with-binary lookups.
//!
//! # Data directory resolution
//!
//! 1. `$HOURSKIT_DATA_DIR` environment variable.
//! 2. `<crate_source_root>/../../data/` — works for `cargo test` inside the
//!    workspace, git deps (cargo fetches the full repo), and
//!    `cargo install --path .`.
//!
//! ```text
//! hourskit/
//!   crates/hourskit/   <- CARGO_MANIFEST_DIR points here
//!   data/              <- ../../data from manifest dir
//! ```

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::error::{Error, Result};
use crate::session::{SessionInfo, TradingClass};
use crate::sources::parquet_io::{read_sessions, FILE_SESSIONS};

// ---------------------------------------------------------------------------
// Data directory resolution
// ---------------------------------------------------------------------------

/// Resolve the `data/` directory (may not exist yet on a fresh clone).
#[must_use]
pub fn data_dir() -> PathBuf {
    if let Ok(env_dir) = std::env::var("HOURSKIT_DATA_DIR") {
        return PathBuf::from(env_dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data")
}

fn require_file(path: &Path, hint: &str) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "{hint} not found at {} (run: cargo run --example seed_data)",
            path.display()
        )))
    }
}

// ---------------------------------------------------------------------------
// Cache (process-wide, invalidates per HOURSKIT_DATA_DIR change)
// ---------------------------------------------------------------------------

struct CacheEntry {
    dir: PathBuf,
    rows: Vec<SessionInfo>,
}

fn cache_slot() -> &'static std::sync::Mutex<Option<CacheEntry>> {
    static CACHE: OnceLock<std::sync::Mutex<Option<CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

fn load_or_cached() -> Result<Vec<SessionInfo>> {
    let dir = data_dir();
    let slot = cache_slot();

    // Cheap path: lock, peek, clone, drop guard before any I/O.
    {
        let guard = match slot.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(entry) = guard.as_ref() {
            if entry.dir == dir {
                return Ok(entry.rows.clone());
            }
        }
    }

    // Slow path: read parquet without holding the cache lock.
    let path = dir.join(FILE_SESSIONS);
    require_file(&path, FILE_SESSIONS)?;
    let rows = read_sessions(&path)?;

    // Take the lock briefly to publish the cache entry.
    let mut guard = match slot.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = Some(CacheEntry {
        dir,
        rows: rows.clone(),
    });
    drop(guard);
    Ok(rows)
}

/// Drop the in-process cache so the next call re-reads the parquet file.
///
/// Used by the CLI after a refresh so subsequent reads pick up the new bytes
/// without restarting the process. Tests that mutate `HOURSKIT_DATA_DIR`
/// likewise want to drop the cache.
pub fn invalidate_cache() {
    let slot = cache_slot();
    if let Ok(mut guard) = slot.lock() {
        *guard = None;
    }
}

// ---------------------------------------------------------------------------
// Effective-dating resolution
// ---------------------------------------------------------------------------

/// Sentinel query date that selects the LATEST effective row.
///
/// The undated lookups ([`session`], [`crate::Hourskit::session`], and the
/// free-function shortcuts) resolve "the currently-effective row" as the
/// greatest `valid_from_yyyymmdd <= MAX`. Pinning the sentinel to the
/// maximum `i32` makes every staged future row eligible, so an existing
/// caller always observes the newest active session for a symbol.
pub(crate) const VALID_FROM_LATEST: i32 = i32::MAX;

/// True when `row` is effective on the query date `on`.
///
/// A `None` baseline row is always eligible (treated as `-inf`); a
/// `Some(valid_from)` row is eligible only when `valid_from <= on`.
#[inline]
const fn row_effective_on(row: &SessionInfo, on: i32) -> bool {
    match row.valid_from_yyyymmdd {
        None => true,
        Some(valid_from) => valid_from <= on,
    }
}

/// Effective-date ordering key: a `None` baseline sorts below every dated
/// row (negative infinity), and dated rows sort by their `YYYYMMDD`.
#[inline]
fn valid_from_key(row: &SessionInfo) -> i32 {
    row.valid_from_yyyymmdd.unwrap_or(i32::MIN)
}

/// Resolve the applicable row for `(symbol, trading_class)` on query date
/// `on`, honouring effective dating within the class (greatest
/// `valid_from_yyyymmdd <= on`, `None` baseline always eligible).
pub(crate) fn resolve_for_class_on(
    rows: Vec<SessionInfo>,
    needle: &str,
    target_wire: &str,
    on: i32,
) -> Option<SessionInfo> {
    rows.into_iter()
        .filter(|r| {
            r.symbol == needle
                && r.trading_class.as_wire() == target_wire
                && row_effective_on(r, on)
        })
        .max_by_key(valid_from_key)
}

/// Resolve the applicable row for `symbol` on query date `on`.
///
/// For a `(symbol, trading_class)` group that holds multiple effective-dated
/// rows, the applicable row is the one with the greatest
/// `valid_from_yyyymmdd <= on` (a `None` baseline counting as `-inf`). Across
/// trading classes the lowest [`TradingClass::preference_rank`] wins, matching
/// the undated [`session`] tie-break.
pub(crate) fn resolve_on(rows: Vec<SessionInfo>, needle: &str, on: i32) -> Option<SessionInfo> {
    rows.into_iter()
        .filter(|r| r.symbol == needle && row_effective_on(r, on))
        // Lowest preference_rank wins across classes; within a class the
        // greatest valid_from wins. `min_by` keeps the first element on
        // ties, so order by (rank asc, valid_from desc) and take the min.
        .min_by(|a, b| {
            a.trading_class
                .preference_rank()
                .cmp(&b.trading_class.preference_rank())
                .then_with(|| valid_from_key(b).cmp(&valid_from_key(a)))
        })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Look up the session for `symbol`, returning `None` if no row matches.
///
/// Lookup is case-insensitive on the symbol. If multiple rows share a symbol
/// (e.g. SPY listed as both options and equity), the row whose
/// [`TradingClass::preference_rank`] is lowest wins. Use
/// [`session_for_class`] to pin a specific class explicitly.
///
/// # Effective dating
///
/// When a `(symbol, trading_class)` carries multiple effective-dated rows
/// (see [`SessionInfo::valid_from_yyyymmdd`]), this function resolves the
/// LATEST effective row — the newest active session. Existing callers
/// therefore always see the most recent rule in force. Use [`session_on`]
/// to resolve the row applicable on a specific query date.
///
/// # Errors
///
/// Returns [`Error`] if the bundled parquet file is missing or fails to
/// read.
pub fn session(symbol: &str) -> Result<Option<SessionInfo>> {
    session_on(symbol, VALID_FROM_LATEST)
}

/// Look up the session for `symbol` applicable on trading date `date`
/// (`YYYYMMDD`), returning `None` if no row matches.
///
/// Lookup is case-insensitive on the symbol. Among the effective-dated rows
/// for a `(symbol, trading_class)` the applicable row is the one with the
/// greatest `valid_from_yyyymmdd <= date`, treating a `None` baseline row as
/// always eligible (see [`SessionInfo::valid_from_yyyymmdd`]). When the
/// symbol resolves across multiple trading classes, the lowest
/// [`TradingClass::preference_rank`] wins, matching [`session`].
///
/// A symbol that carries only a `None` baseline row resolves identically for
/// every `date`, so this is fully backward-compatible with [`session`] for
/// any symbol that has no staged future row.
///
/// # Errors
///
/// Returns [`Error`] if the bundled parquet file is missing or fails to
/// read.
pub fn session_on(symbol: &str, date: i32) -> Result<Option<SessionInfo>> {
    let needle = symbol.to_ascii_uppercase();
    let rows = load_or_cached()?;
    Ok(resolve_on(rows, &needle, date))
}

/// Look up the session for a specific `(symbol, trading_class)` pair.
///
/// When the pair carries multiple effective-dated rows, the LATEST effective
/// row is returned (matching the undated [`session`] semantics). Use
/// [`session_for_class_on`] to resolve the row applicable on a specific
/// query date.
///
/// # Errors
///
/// Returns [`Error`] if the bundled parquet file is missing or fails to
/// read.
pub fn session_for_class(
    symbol: &str,
    trading_class: &TradingClass,
) -> Result<Option<SessionInfo>> {
    session_for_class_on(symbol, trading_class, VALID_FROM_LATEST)
}

/// Look up the session for a specific `(symbol, trading_class)` pair
/// applicable on trading date `date` (`YYYYMMDD`).
///
/// Resolves the effective-dated row within the class: the greatest
/// `valid_from_yyyymmdd <= date`, treating a `None` baseline as always
/// eligible (see [`SessionInfo::valid_from_yyyymmdd`]).
///
/// # Errors
///
/// Returns [`Error`] if the bundled parquet file is missing or fails to
/// read.
pub fn session_for_class_on(
    symbol: &str,
    trading_class: &TradingClass,
    date: i32,
) -> Result<Option<SessionInfo>> {
    let needle = symbol.to_ascii_uppercase();
    let target_wire = trading_class.as_wire();
    let rows = load_or_cached()?;
    Ok(resolve_for_class_on(rows, &needle, &target_wire, date))
}

/// Read the full session table.
///
/// # Errors
///
/// Returns [`Error`] if the bundled parquet file is missing or fails to
/// read.
pub fn sessions_all() -> Result<Vec<SessionInfo>> {
    load_or_cached()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Settlement, TimeWindow};

    #[test]
    fn data_dir_does_not_panic() {
        let _ = data_dir();
    }

    fn row(symbol: &str, class: TradingClass, valid_from: Option<i32>) -> SessionInfo {
        SessionInfo {
            symbol: symbol.to_string(),
            trading_class: class,
            regular: TimeWindow::from_clock_et(9, 30, 16, 0),
            pre_market: None,
            post_market: None,
            curb: None,
            gth: None,
            gth_overnight: false,
            last_trading_day_close_us: None,
            settlement: Settlement::Pm,
            valid_from_yyyymmdd: valid_from,
        }
    }

    #[test]
    fn baseline_only_symbol_resolves_for_any_date() {
        let rows = vec![row("AAPL", TradingClass::EquityNasdaq, None)];
        // Every query date returns the baseline row; fully back-compat.
        for date in [10_000_101, 20_260_712, VALID_FROM_LATEST] {
            let got = resolve_on(rows.clone(), "AAPL", date).expect("baseline");
            assert_eq!(got.valid_from_yyyymmdd, None);
        }
    }

    #[test]
    fn staged_future_row_only_applies_on_or_after_valid_from() {
        let rows = vec![
            row("NVDA", TradingClass::OptionsCboeBzxC2Edgx, None),
            row("NVDA", TradingClass::OptionsCboeBzxC2Edgx, Some(20_260_713)),
        ];
        // Day before: baseline row.
        let before = resolve_on(rows.clone(), "NVDA", 20_260_712).expect("before");
        assert_eq!(before.valid_from_yyyymmdd, None);
        // Effective day: staged row.
        let on = resolve_on(rows.clone(), "NVDA", 20_260_713).expect("on");
        assert_eq!(on.valid_from_yyyymmdd, Some(20_260_713));
        // Latest sentinel: staged row (newest active session).
        let latest = resolve_on(rows, "NVDA", VALID_FROM_LATEST).expect("latest");
        assert_eq!(latest.valid_from_yyyymmdd, Some(20_260_713));
    }

    #[test]
    fn greatest_valid_from_at_or_below_date_wins() {
        let rows = vec![
            row("X", TradingClass::EquityNasdaq, None),
            row("X", TradingClass::EquityNasdaq, Some(20_260_101)),
            row("X", TradingClass::EquityNasdaq, Some(20_260_713)),
        ];
        // On 2026-07-12 the 2026-01-01 row is the greatest <= date.
        let mid = resolve_on(rows.clone(), "X", 20_260_712).expect("mid");
        assert_eq!(mid.valid_from_yyyymmdd, Some(20_260_101));
        // On 2026-07-13 the newest row wins.
        let new = resolve_on(rows, "X", 20_260_713).expect("new");
        assert_eq!(new.valid_from_yyyymmdd, Some(20_260_713));
    }

    #[test]
    fn lowest_preference_rank_wins_across_classes() {
        // SPY listed as both options (C1, rank 0) and equity (rank 7);
        // the options row wins regardless of effective dating.
        let rows = vec![
            row("SPY", TradingClass::EquityNyseArca, None),
            row("SPY", TradingClass::OptionsCboeC1, None),
        ];
        let got = resolve_on(rows, "SPY", VALID_FROM_LATEST).expect("spy");
        assert_eq!(got.trading_class, TradingClass::OptionsCboeC1);
    }
}
