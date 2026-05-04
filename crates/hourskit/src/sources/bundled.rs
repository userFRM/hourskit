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
// Public API
// ---------------------------------------------------------------------------

/// Look up the session for `root`, returning `None` if no row matches.
///
/// Lookup is case-insensitive on the root. If multiple rows share a root
/// (e.g. SPY listed as both options and equity), the row whose
/// [`TradingClass::preference_rank`] is lowest wins. Use
/// [`session_for_class`] to pin a specific class explicitly.
///
/// # Errors
///
/// Returns [`Error`] if the bundled parquet file is missing or fails to
/// read.
pub fn session(root: &str) -> Result<Option<SessionInfo>> {
    let needle = root.to_ascii_uppercase();
    let rows = load_or_cached()?;
    Ok(rows
        .into_iter()
        .filter(|r| r.root == needle)
        .min_by_key(|r| r.trading_class.preference_rank()))
}

/// Look up the session for a specific `(root, trading_class)` pair.
///
/// # Errors
///
/// Returns [`Error`] if the bundled parquet file is missing or fails to
/// read.
pub fn session_for_class(root: &str, trading_class: &TradingClass) -> Result<Option<SessionInfo>> {
    let needle = root.to_ascii_uppercase();
    let target_wire = trading_class.as_wire();
    let rows = load_or_cached()?;
    Ok(rows
        .into_iter()
        .find(|r| r.root == needle && r.trading_class.as_wire() == target_wire))
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

    #[test]
    fn data_dir_does_not_panic() {
        let _ = data_dir();
    }
}
