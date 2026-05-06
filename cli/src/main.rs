//! `hourskit-cli` — session inspection and bundled-data manifest tooling.
//!
//! # Commands
//!
//! ```text
//! hourskit-cli session  --symbol SPX                  # text output, default unit (ms)
//! hourskit-cli session  --symbol SPX --unit us        # microseconds
//! hourskit-cli session  --symbol QQQ --format json    # machine-readable
//! hourskit-cli manifest                                # SHA-256 of every parquet file in `data/`
//! hourskit-cli refresh                                 # force fresh fetch from upstream
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(unused_must_use)]
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use sha2::{Digest, Sha256};
use tracing_subscriber::{fmt, EnvFilter};

use hourskit::{Hourskit, SessionInfo, TimeUnit, TimeWindow};

// ── CLI definition ─────────────────────────────────────────────────────────────

/// Top-level CLI parser.
#[derive(Parser, Debug)]
#[command(
    name = "hourskit-cli",
    about = "Inspect hourskit bundled session data — Cboe / NASDAQ / NYSE Arca trading hours",
    version,
    propagate_version = true
)]
struct Cli {
    /// Path to the data directory (default: the repository's `data/` folder).
    /// Override with $HOURSKIT_DATA_DIR or this flag.
    #[arg(long, env = "HOURSKIT_DATA_DIR", global = true)]
    data_dir: Option<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, global = true, default_value_t = OutputFormat::Text)]
    format: OutputFormat,

    /// Time unit used to render every microsecond field.
    #[arg(long, value_enum, global = true, default_value_t = UnitFlag::Ms)]
    unit: UnitFlag,

    #[command(subcommand)]
    cmd: Command,
}

/// Output format selector.
#[derive(Copy, Clone, Debug, ValueEnum)]
enum OutputFormat {
    /// Human-readable text output (default).
    Text,
    /// Machine-readable JSON output.
    Json,
}

/// Time-unit selector mapped to [`hourskit::TimeUnit`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum UnitFlag {
    /// Microseconds (1 µs precision).
    Us,
    /// Milliseconds (default).
    Ms,
    /// Seconds.
    S,
}

impl From<UnitFlag> for TimeUnit {
    fn from(value: UnitFlag) -> Self {
        match value {
            UnitFlag::Us => Self::Microseconds,
            UnitFlag::Ms => Self::Milliseconds,
            UnitFlag::S => Self::Seconds,
        }
    }
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
enum Command {
    /// Print the [`SessionInfo`] for `--symbol` from bundled parquet.
    Session {
        /// Symbol (e.g. SPX, QQQ, AAPL).
        #[arg(long)]
        symbol: String,
    },

    /// Generate (or regenerate) `data/manifest.json` with SHA-256 digests for
    /// every parquet file in the data directory.
    Manifest,

    /// Force a fresh fetch from the upstream origin (bypasses ETag short-circuit).
    Refresh,
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> ExitCode {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    let data_dir = cli.data_dir.unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("data")
    });
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("creating data dir {}", data_dir.display()))?;

    // Make the bundled reader use this directory regardless of where the
    // CLI was launched from. Single-threaded startup before any tokio worker
    // tasks are spawned.
    std::env::set_var("HOURSKIT_DATA_DIR", &data_dir);
    hourskit::sources::bundled::invalidate_cache();

    match cli.cmd {
        Command::Session { symbol } => cmd_session(&symbol, cli.format, cli.unit.into()),
        Command::Manifest => cmd_manifest(&data_dir),
        Command::Refresh => cmd_refresh().await,
    }
}

// ── session ───────────────────────────────────────────────────────────────────

fn cmd_session(symbol: &str, format: OutputFormat, unit: TimeUnit) -> Result<()> {
    let info = hourskit::sources::bundled::session(symbol)
        .with_context(|| format!("loading session for {symbol}"))?;
    match info {
        Some(info) => print_session(&info, format, unit),
        None => print_kv(
            format,
            &[("symbol", symbol.into()), ("found", JsonValue::Bool(false))],
            &format!("{symbol}: no session row in bundled data"),
        ),
    }
    Ok(())
}

fn print_session(info: &SessionInfo, format: OutputFormat, unit: TimeUnit) {
    match format {
        OutputFormat::Text => {
            println!("{:<22} {}", "symbol:", info.symbol);
            println!("{:<22} {}", "trading_class:", info.trading_class);
            println!("{:<22} {}", "regular:", info.regular);
            print_window_text("pre_market:", info.pre_market.as_ref());
            print_window_text("post_market:", info.post_market.as_ref());
            print_window_text("curb:", info.curb.as_ref());
            print_window_text("gth:", info.gth.as_ref());
            println!("{:<22} {}", "gth_overnight:", info.gth_overnight);
            match info.last_trading_day_close_us {
                Some(close_us) => println!(
                    "{:<22} {} ({})",
                    "last_trading_day:",
                    convert_us(close_us, unit),
                    unit
                ),
                None => println!("{:<22} -", "last_trading_day:"),
            }
            match info.settlement {
                hourskit::Settlement::Pm => {
                    println!("{:<22} pm", "settlement:");
                }
                hourskit::Settlement::AmOpen { open_us_of_day } => {
                    println!(
                        "{:<22} am-open @ {} ({})",
                        "settlement:",
                        convert_us(open_us_of_day, unit),
                        unit
                    );
                }
                // Forward-compat: future Settlement variants render as
                // "unknown" until the CLI is taught about them.
                _ => println!("{:<22} unknown", "settlement:"),
            }
            println!(
                "{:<22} {} ({})",
                "regular_open:",
                info.regular.open_in(unit),
                unit
            );
            println!(
                "{:<22} {} ({})",
                "regular_close:",
                info.regular.close_in(unit),
                unit
            );
        }
        OutputFormat::Json => {
            let settlement_json = match info.settlement {
                hourskit::Settlement::Pm => serde_json::json!({"kind": "pm"}),
                hourskit::Settlement::AmOpen { open_us_of_day } => serde_json::json!({
                    "kind": "am_open",
                    "open": convert_us(open_us_of_day, unit),
                }),
                // Forward-compat: future Settlement variants serialise
                // as a typed "unknown" JSON node so callers can detect
                // the gap without crashing.
                _ => serde_json::json!({"kind": "unknown"}),
            };
            let value = serde_json::json!({
                "symbol": info.symbol,
                "trading_class": info.trading_class.as_wire(),
                "unit": unit.to_string(),
                "regular": window_json(&info.regular, unit),
                "pre_market": info.pre_market.as_ref().map(|w| window_json(w, unit)),
                "post_market": info.post_market.as_ref().map(|w| window_json(w, unit)),
                "curb": info.curb.as_ref().map(|w| window_json(w, unit)),
                "gth": info.gth.as_ref().map(|w| window_json(w, unit)),
                "gth_overnight": info.gth_overnight,
                "last_trading_day_close": info
                    .last_trading_day_close_us
                    .map(|us| convert_us(us, unit)),
                "settlement": settlement_json,
            });
            match serde_json::to_string_pretty(&value) {
                Ok(s) => println!("{s}"),
                Err(e) => eprintln!("error serialising JSON: {e}"),
            }
        }
    }
}

/// Convert `us` (microseconds-of-day) into the caller's requested `unit`.
///
/// `TimeUnit` is `#[non_exhaustive]`, so we use `if let` arms with a
/// microsecond fallback for any future variant. The library itself
/// already surfaces unit-aware accessors on `TimeWindow`; this helper
/// only exists for the lone scalar field (`last_trading_day_close_us`)
/// that has no encapsulating window type.
const fn convert_us(us: i64, unit: TimeUnit) -> i64 {
    if matches!(unit, TimeUnit::Milliseconds) {
        us / 1_000
    } else if matches!(unit, TimeUnit::Seconds) {
        us / 1_000_000
    } else {
        us
    }
}

fn print_window_text(label: &str, w: Option<&TimeWindow>) {
    match w {
        Some(window) if window.is_overnight() => {
            let oh = window.open_secs() / 3_600;
            let om = (window.open_secs() / 60) % 60;
            let ch = window.close_secs() / 3_600;
            let cm = (window.close_secs() / 60) % 60;
            println!("{label:<22} {oh:02}:{om:02} (prior) - {ch:02}:{cm:02} (current) ET");
        }
        Some(window) => println!("{label:<22} {window}"),
        None => println!("{label:<22} -"),
    }
}

fn window_json(w: &TimeWindow, unit: TimeUnit) -> serde_json::Value {
    serde_json::json!({
        "open": w.open_in(unit),
        "close": w.close_in(unit),
        "duration": w.duration_in(unit),
        "overnight": w.is_overnight(),
    })
}

// ── Output helpers ────────────────────────────────────────────────────────────

#[derive(Clone)]
enum JsonValue {
    String(String),
    Bool(bool),
}

impl From<String> for JsonValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for JsonValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_string())
    }
}

fn print_kv(format: OutputFormat, pairs: &[(&str, JsonValue)], text: &str) {
    match format {
        OutputFormat::Text => println!("{text}"),
        OutputFormat::Json => {
            let mut map = serde_json::Map::new();
            for (k, v) in pairs {
                let val = match v {
                    JsonValue::String(s) => serde_json::Value::String(s.clone()),
                    JsonValue::Bool(b) => serde_json::Value::Bool(*b),
                };
                map.insert((*k).to_string(), val);
            }
            match serde_json::to_string_pretty(&serde_json::Value::Object(map)) {
                Ok(s) => println!("{s}"),
                Err(e) => eprintln!("error: serialising JSON: {e}"),
            }
        }
    }
}

// ── Manifest ──────────────────────────────────────────────────────────────────

fn cmd_manifest(data_dir: &Path) -> Result<()> {
    use std::collections::BTreeMap;

    let mut entries: BTreeMap<String, String> = BTreeMap::new();
    walk_parquets(data_dir, data_dir, &mut entries)?;

    let manifest =
        serde_json::to_string_pretty(&entries).context("serializing manifest to JSON")?;
    let manifest_path = data_dir.join("manifest.json");
    std::fs::write(&manifest_path, manifest)
        .with_context(|| format!("writing {}", manifest_path.display()))?;

    println!(
        "Wrote manifest with {} entries -> {}",
        entries.len(),
        manifest_path.display()
    );
    Ok(())
}

fn walk_parquets(
    root: &Path,
    dir: &Path,
    entries: &mut std::collections::BTreeMap<String, String>,
) -> Result<()> {
    let read_dir =
        std::fs::read_dir(dir).with_context(|| format!("reading data dir {}", dir.display()))?;

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_parquets(root, &path, entries)?;
            continue;
        }
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) if n.ends_with(".parquet") => n.to_string(),
            _ => continue,
        };
        let logical = path
            .strip_prefix(root)
            .map_or(name, |p| p.to_string_lossy().replace('\\', "/"));
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        let mut hex = String::with_capacity(digest.len() * 2);
        for b in digest.as_slice() {
            use std::fmt::Write as _;
            let _ = write!(hex, "{b:02x}");
        }
        entries.insert(logical, format!("sha256:{hex}"));
    }
    Ok(())
}

// ── Refresh ───────────────────────────────────────────────────────────────────

async fn cmd_refresh() -> Result<()> {
    // Force the staleness ceiling to zero so the fetcher MUST hit the
    // network on this call.
    let client = Hourskit::new().with_staleness_ceiling(Duration::from_millis(0));
    let rows = client
        .sessions_all()
        .await
        .context("fetching sessions.parquet from upstream")?;
    println!(
        "refreshed bundled data: {} rows from upstream sessions.parquet",
        rows.len()
    );
    Ok(())
}
