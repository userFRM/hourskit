# Changelog

All notable changes to hourskit are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-05-04

### Added

- `parquet-loader` Cargo feature (enabled by default) gates the bundled-parquet
  reader, the `Hourskit` client, the ETag-aware fetcher, the
  `sources::bundled` / `sources::parquet_io` modules, and the
  `session()` / `session_blocking()` free functions. Disable to drop
  the `parquet`, `arrow`, `bytes`, and `tempfile` dependencies — and
  the transitive `paste 1.0.15` (RUSTSEC-2024-0436). With the feature
  off, `SessionInfo`, `TimeWindow`, `TradingClass`, and `TimeUnit`
  remain available so callers can construct session windows from
  explicit data instead of reading the bundled parquet.
- `pastey 0.2.2` workspace dependency (declaration only, not yet consumed).
  Maintained drop-in for the archived `paste` macro crate; held at the
  workspace level so any first-party macro work reaches the maintained fork.
- `hourskit-cli` pins `features = ["parquet-loader"]` on its dependency on
  `hourskit` so the CLI compiles regardless of the consumer workspace's
  default-feature configuration.
- `[[example]] required-features = ["parquet-loader"]` on `seed_data` so
  the example excludes itself from `--no-default-features` builds.

### Changed

- `hourskit::Error::Arrow` and `hourskit::Error::ParquetNative` are now
  gated behind the `parquet-loader` feature. Pattern-match call-sites that
  must compile across feature configurations should pin the feature on.
- `audit.toml` and `deny.toml` rationale updated to reference the new
  feature gate.

## [0.1.0] - 2026-05-03

Initial pre-release. Not yet published to crates.io.

### Scope

`hourskit` covers US exchange trading hours scoped per `(symbol, trading
class)`:

- **Cboe options** — C1 cash-settled index options (SPX/VIX/XSP/RUT family;
  16:15 ET regular close + 16:15-17:00 Curb session + 20:15 prior to 09:25
  current GTH), Cboe BZX/C2/EDGX option family.
- **Other options** — NYSE Arca, Nasdaq ISE, BOX, NYSE American.
- **US equities** — Nasdaq Stock Market (regular session + 04:00-09:30 pre +
  16:00-20:00 post + 21:00-04:00 ET Extended Session, the Nasdaq Global
  Trading Hours program announced for 2026), NYSE Arca, Cboe BZX/EDGX
  (early-trading from 02:30 ET), Cboe BYX/EDGA.

### Added

- Unified `SessionInfo` struct returned by every lookup; carries
  `trading_class`, `regular`, `pre_market`, `post_market`, `curb`, `gth`,
  and `gth_overnight` flag.
- `TimeWindow` half-open `[open, close)` window storing `i64`
  microseconds-of-day (ET); accessor methods convert to milliseconds /
  seconds at the API boundary.
- `TimeUnit` selector (`Microseconds` / `Milliseconds` / `Seconds`) for
  caller-chosen output resolution.
- `TradingClass` enum identifying which exchange-rule family produced the
  windows; `#[non_exhaustive]` with an `Other(String)` catch-all for
  forward compatibility.
- `Hourskit` async client with infallible `new()` + builder methods
  `with_base_url`, `with_cache_dir`, `with_mirror_url`,
  `with_staleness_ceiling`. Connection pool reused across calls.
- Free functions for one-off scripts: `hourskit::session(root)`,
  `hourskit::session_blocking(root)`.
- Unified `data/sessions.parquet` schema:
  ```text
  root              : Utf8                 (non-null)
  trading_class     : Utf8                 (non-null)
  regular_open_us   : Int64                (non-null)
  regular_close_us  : Int64                (non-null)
  pre_open_us       : Int64                (nullable)
  pre_close_us      : Int64                (nullable)
  post_open_us      : Int64                (nullable)
  post_close_us     : Int64                (nullable)
  curb_open_us      : Int64                (nullable)
  curb_close_us     : Int64                (nullable)
  gth_open_us       : Int64                (nullable)
  gth_close_us      : Int64                (nullable)
  gth_overnight     : Boolean              (nullable)
  ```
- ETag-aware HTTP fetcher with retry + exponential backoff, single-flight
  per-key deduplication, jsDelivr CDN mirror fallback, and SHA-256
  manifest verification.
- **24-hour staleness ceiling** — the fetcher forces a fresh body once the
  cache mtime is older than 24 hours, even if the ETag would otherwise
  short-circuit it. Tunable via `Hourskit::with_staleness_ceiling`.
- Strict parquet schema validation on every read — divergent schemas
  return `Error::SchemaMismatch` instead of panicking.
- Atomic cache writes (tmp + fsync + rename) so a partial write or crash
  never produces a corrupt cache file.
- Bundled offline reader (`sources::bundled`) with process-wide cache;
  `invalidate_cache()` re-reads on the next call.
- CLI binary (`hourskit-cli`):
  - `hourskit-cli session --root <ROOT> [--unit us|ms|s] [--format text|json]`
  - `hourskit-cli manifest` — regenerate `data/manifest.json` with SHA-256
    digests.
  - `hourskit-cli refresh` — force a fresh fetch from the upstream origin.
- Maintainer workflow: `cargo run --example seed_data` regenerates
  `data/sessions.parquet` from in-tree Rust constants; CI verifies the
  manifest matches the regenerated digests.
- Env overrides: `HOURSKIT_BASE_URL`, `HOURSKIT_CACHE_DIR`,
  `HOURSKIT_MIRROR_URL`, `HOURSKIT_DATA_DIR`.
- Workspace lints: `forbid(unsafe_code)`, `deny(missing_docs)`,
  `deny(unwrap_used)` / `deny(expect_used)` / `deny(panic)` outside
  tests, pedantic + nursery clippy groups at warn level.
- Apache-2.0 / MIT dual license.

### Data sources

- The 82-entry **Cboe extended-trading-hours roster** seeded into
  `data/sessions.parquet` is sourced from the Cboe extended-trading-hours
  notice. `SPX (PM Expiration)` and `XSP (AM Expiration)` are kept as
  literal root strings.
- The **OPRA Global Trading Hours close** is set to 09:25 ET per the OPRA
  notice "Revised OPRA GTH Hours of Operation" (effective trade date
  2024-08-26). The Sunday-night session start at 20:15 ET is unchanged.
- **Source citations:** roster cross-referenced against CBOE C1 Rule 5.1,
  OPRA GTH 2024-08-26 PDF, Firstrade broker list, NASDAQ Trader exchange
  page, and ThetaData operational data. ThetaData superset selected for
  broadest coverage; Firstrade + NASDAQ subsets retained as
  cross-validation references. See [`examples/seed_data.rs`](crates/hourskit/examples/seed_data.rs)
  for the full citation block + per-source URLs and the ThetaData-only
  delta list.
- **Cash-settled-index last-trading-day exception** encoded per CBOE Rule
  5.1(b)(2)(C) on TWO layers for institutional-grade coverage:
  - **Per-root**: NDXP, RUTW, MRUT, SPXW, XSP, OEX, XEO (Firstrade
    quote) plus SPXPM, SPEQF, SPEQX (rule-named p.m.-settled) plus
    `SPX (PM Expiration)` / `XSP (PM Expiration)` (literal-root
    string-encoded variants) all carry an explicit
    `last_trading_day_close_us = Some(57_600_000_000)` (16:00 ET in
    microseconds) on the parquet row.
  - **Class-level fallback**:
    `TradingClass::class_level_last_trading_day_close_us` returns
    `Some(57_600_000_000)` for every `OptionsCboeC1` root, capturing
    the rule's open-ended "Index Options with Nonstandard Expirations
    (Weeklys, EOMs, Monthly, Quarterly, QIXs)" bullet without
    requiring per-root enumeration. Future Cboe additions to the C1
    ladder inherit the rule automatically.

  Resolved via `SessionInfo::effective_close_us(event_date, exp_date)`
  with priority: per-root override → class-level fallback → regular
  close. Schema gains a nullable `last_trading_day_close_us: Int64`
  column; populated for the named subset, NULL otherwise (and resolved
  through the class-level fallback at lookup time).

### Notes

- **Cboe options GTH vs. NASDAQ Extended Session.** Cboe C1 options run a
  20:15 ET prior to 09:25 ET current overnight session for SPX / VIX /
  XSP / RUT. NASDAQ has a separately-named "Global Trading Hours" program
  announced for 2026 — a 21:00 ET to 04:00 ET Extended Session for
  Nasdaq-listed equities. Both are modelled by the `gth` field; the
  `gth_overnight` flag distinguishes "open is on the prior trading day"
  (Cboe) from "open is the same calendar evening" (Nasdaq numerically
  greater than close, wraps midnight).
- **Curb / GTH suspension on half-days.** Cboe Rule 6.1A suspends Curb
  and GTH on early-close days. `hourskit` ships the rule-defined window
  only — combine with a market calendar to filter on half-days.
- **No HTML scraper.** The 0.0.x cboekit codebase shipped a `cboe-html`
  scraper as a `backfill` feature; it has been removed in favour of
  in-tree Rust constants. Maintainer regenerates parquet via
  `cargo run --example seed_data`.

[0.1.0]: https://github.com/userFRM/hourskit/releases/tag/v0.1.0
