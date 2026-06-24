# Changelog

All notable changes to hourskit are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.1] - 2026-06-24

### Security

- Resolved RUSTSEC-2026-0185 (remote memory exhaustion in `quinn-proto` from unbounded out-of-order stream reassembly, reached transitively through `reqwest`) by updating the lockfile to `quinn-proto 0.11.15`, the first release carrying the upstream fix. No first-party code or API change was required.

### Changed

- Updated dependencies: `arrow` and `parquet` 54 to 58, `sha2` 0.10 to 0.11, `directories` 5 to 6, `tokio` to 1.52.3, and `serde_json` to 1.0.150. The bundled `data/sessions.parquet` was regenerated under `parquet 58`; the on-disk schema is unchanged and the bundled sessions round-trip identically.
- Minimum supported Rust version is now 1.86, the floor required by the refreshed `arrow` / `parquet` / `sha2` stack and their transitive dependencies.
- CI actions updated: `actions/checkout` to v6 and `actions/github-script` to v9.

## [0.6.0] - 2026-06-24

### Added

- Effective-dated sessions. `SessionInfo` gains a `valid_from_yyyymmdd: Option<i32>` field: `None` is the always-valid baseline row, `Some(YYYYMMDD)` stages a future trading-hours change that only governs query dates on or after that trading date. A `(symbol, trading_class)` may now hold multiple rows differing only by this field. The new `session_on(symbol, date)` resolver (plus `session_on_blocking`, the `Hourskit` client method, and `session_for_class_on` / `session_for_class_on_blocking`) returns the row with the greatest `valid_from_yyyymmdd <= date`, treating a `None` baseline as always eligible. The undated `session` / `session_for_class` lookups resolve the latest effective row, so existing callers always see the newest active session; any symbol that carries only a baseline row resolves identically for every date.
- Cboe extended-hours single-stock-option sessions, effective 2026-07-13, for the firmly-announced names NVDA, TSLA, AAPL, PLTR, AVGO, AMD: a pre-market session 07:30-09:25 ET and a post-market session 16:00-16:15 ET in addition to the regular 09:30-16:00 ET session. Each row is staged via `valid_from_yyyymmdd = 20260713`. The eligible-class list is updated semi-annually, so this roster is the firmly-announced subset pending the full rule-filing list.

### Changed

- **BREAKING**: `data/sessions.parquet` gains a nullable `valid_from_yyyymmdd` Int32 column. Downstream parquet consumers pinned to the prior schema must update to read the new layout.

## [0.5.0] - 2026-05-07

### Changed

- **BREAKING**: renamed the `root` identifier to `symbol` across the
  public API to align with upstream `userFRM/thetadx`'s
  `Contract.symbol` field. The on-disk `data/sessions.parquet` schema
  preserves the wire-format `root` column name; only the in-memory
  Rust type and parameter names move.

  Migration map:

  | Before | After |
  |---|---|
  | `SessionInfo.root: String` | `SessionInfo.symbol: String` |
  | `Hourskit::session(root)` | `Hourskit::session(symbol)` |
  | `Hourskit::session_for_class(root, class)` | `Hourskit::session_for_class(symbol, class)` |
  | `Hourskit::session_blocking(root)` | `Hourskit::session_blocking(symbol)` |
  | `Hourskit::session_for_class_blocking(root, class)` | `Hourskit::session_for_class_blocking(symbol, class)` |
  | `hourskit::session(root)` (free fn) | `hourskit::session(symbol)` |
  | `hourskit::session_blocking(root)` (free fn) | `hourskit::session_blocking(symbol)` |
  | `hourskit::sources::bundled::session(root)` | `hourskit::sources::bundled::session(symbol)` |
  | `hourskit::sources::bundled::session_for_class(root, class)` | `hourskit::sources::bundled::session_for_class(symbol, class)` |
  | `Error::UnknownRoot(String)` | `Error::UnknownSymbol(String)` |
  | `hourskit-cli session --root SPX` | `hourskit-cli session --symbol SPX` |

  The wire-format `"root"` parquet column name and the
  `data/sessions.parquet` byte layout are unchanged, so cached files
  and downstream parquet consumers continue to work without
  re-fetching. Only Rust callers need to update field/parameter names.
  Closes [#19](https://github.com/userFRM/hourskit/issues/19).

## [0.4.0] - 2026-05-05

### Added

- `hourskit::time` module exporting day-length constants in canonical
  integer units: `MS_PER_DAY: i32 = 86_400_000`,
  `SECONDS_PER_DAY: i32 = 86_400`, `MIN_PER_DAY: i32 = 1_440`. Used as
  the upper bound on FPSS ms-of-day stamps and as the "full day"
  multiplier inside minute-precision time-to-expiration walks. Removes
  duplicated `const MS_PER_DAY: i32 = 86_400_000;` declarations in
  downstream analytics crates.
- `hourskit::time::days_from_epoch(yyyymmdd: i32) -> i32` and
  `hourskit::time::days_between_yyyymmdd(from: i32, to: i32) -> i32`
  Gregorian-date arithmetic helpers. Linear-scan implementation
  matching the three private copies that `thetadatadx-analytics`
  carried in `_shared/time_to_expiration.rs`,
  `_shared/market_data/rate.rs`, and `greeks/emission.rs`. Output-byte
  parity verified by a 47k-row sweep across every valid YYYYMMDD in
  1970..=2099 inside `time::tests`.
- `hourskit::session::OPTION_PM_SETTLEMENT_US: i64 = 57_600_000_000`
  and `hourskit::session::OPTION_PM_SETTLEMENT_MS: i32 = 57_600_000`
  protocol constants encoding the 16:00 ET PM-settlement option-
  trading cutoff per CBOE Rule 5.1(b)(2)(C). Surfaces the value as
  a typed name so seed data, fixtures, and downstream analytics
  callers without a `SessionInfo` in hand stop hardcoding the
  literal. Internal sites in `TradingClass::class_level_last_trading_day_close_us`,
  the `examples/seed_data` per-root override roster, and the
  `parquet_io` round-trip fixture now resolve through the constant.
- `hourskit::time::RTH_START_MS: i32 = 34_200_000`,
  `hourskit::time::RTH_END_MS: i32 = 57_600_000`,
  `hourskit::time::RTH_START_US: i64 = 34_200_000_000`, and
  `hourskit::time::RTH_END_US: i64 = 57_600_000_000` constants
  encoding the SIP equity regular-trading-hours window
  `[09:30 ET, 16:00 ET)`. Compile-time guards pin the μs/ms
  variants together. Removes duplicated `RTH_START_MS` /
  `RTH_END_MS` declarations in `thetadatadx-analytics`
  (`analytics/ohlcvc/bar.rs`).

## [0.3.0] - 2026-05-05

### Added

- `Settlement` enum (`Pm`, `AmOpen { open_us_of_day: i64 }`) describing the
  per-(root, expiration) settlement convention for cash-settled index option
  contracts. `#[non_exhaustive]` so future cash-settled product families can
  add variants without breaking downstream `match _ => {}` arms.
- `SessionInfo::settlement: Settlement` field. Default `Settlement::Pm` on
  every shipped row; populated as `Settlement::AmOpen { 09:30 ET }` for the
  SPX root only — the original CBOE VIX constituent whose standard
  third-Friday-of-the-month expirations follow the AM SET print.
- `SessionInfo::settlement_cutoff_us(expiration_date) -> i64` resolves the
  microsecond-of-day where variance-contribution accounting must terminate
  for an option contract that expires on `expiration_date`. AM-settled rows
  return the embedded `open_us_of_day` on third-Friday expirations and fall
  back to `effective_close_us(expiration_date, expiration_date)` on every
  other expiration; PM-settled rows always return `effective_close_us`.
  Downstream volatility analytics should call this in place of a hardcoded
  16:00 ET cutoff so SPX standard third-Friday rolls compute time-to-expiry
  off the AM SET print rather than the option-trading session close.
- `is_third_friday(yyyymmdd: i32) -> bool` free function — the canonical
  SPX-standard-expiration classifier. Implemented via Zeller's congruence
  for `const fn` evaluability.
- `settlement_am_open_us: Int64` (nullable) column on the bundled
  `data/sessions.parquet` — the parquet-side encoding of
  `Settlement::AmOpen`. NULL maps to `Settlement::Pm`.

### Changed

- `data/sessions.parquet` schema gains the `settlement_am_open_us` column;
  consumers that pin a specific schema string will see a one-shot
  `SchemaMismatch` after upgrade until they regenerate / refetch.
  Manifest digest bumped accordingly.
- The CLI `hourskit-cli session --root <ROOT>` text + JSON output now
  surfaces `settlement: pm | am-open @ <us-of-day>` so operators can
  inspect the rule from a one-shot lookup.

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

[0.4.0]: https://github.com/userFRM/hourskit/releases/tag/v0.4.0
[0.1.0]: https://github.com/userFRM/hourskit/releases/tag/v0.1.0
