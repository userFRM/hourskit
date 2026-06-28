# hourskit

US exchange trading-hours reference data for Rust, microsecond-native. Served from bundled parquet with on-demand fetch and a local cache. No API keys. Offline after the first query.

## Install

```toml
[dependencies]
hourskit = "0.6.1"
```

To track unreleased changes, depend on the repository directly:

```toml
hourskit = { git = "https://github.com/userFRM/hourskit" }
```

## Quick start

```rust,no_run
use hourskit::session;

#[tokio::main]
async fn main() -> hourskit::Result<()> {
    let info = session("SPX").await?.expect("SPX in seed");
    println!("regular {}, curb {:?}", info.regular, info.curb);
    Ok(())
}
```

## Client pattern

Create a client once and reuse it so the connection pool and parquet cache are shared across calls.

```rust,no_run
use hourskit::{Hourskit, TradingClass};

#[tokio::main]
async fn main() -> hourskit::Result<()> {
    let client = Hourskit::new();
    let qqq = client
        .session_for_class("QQQ", &TradingClass::EquityNasdaq)
        .await?
        .expect("QQQ EquityNasdaq");
    println!("QQQ open {} us-of-day", qqq.regular.open_us());
    Ok(())
}
```

Each lookup has a blocking sibling (`session_blocking`, `session_for_class_blocking`, `sessions_all_blocking`) for callers outside an async runtime.

## Coverage

| Trading class | Regular | Pre | Post | Curb | Overnight |
|---|---|---|---|---|---|
| `OptionsCboeC1` (SPX, VIX, XSP, RUT family + nonstandard-expiration C1) | 09:30-16:15 | — | — | 16:15-17:00 | 20:15 prior to 09:25 current |
| `OptionsCboeBzxC2Edgx` | 09:30-16:15 | — | — | — | — |
| `OptionsNyseArca` / `OptionsIse` / `OptionsBox` / `OptionsAmex` | 09:30-16:00 | — | — | — | — |
| `EquityNasdaq` | 09:30-16:00 | 04:00-09:30 | 16:00-20:00 | — | 21:00-04:00 (Nasdaq Extended Session, 2026 program) |
| `EquityNyseArca` | 09:30-16:00 | 04:00-09:30 | 16:00-20:00 | — | — |
| `EquityCboeBzxEdgx` | 09:30-16:00 | 02:30-09:30 | 16:00-20:00 | — | — |
| `EquityCboeByxEdga` | 09:30-16:00 | 07:00-09:30 | 16:00-20:00 | — | — |

Last-trading-day exception per CBOE Rule 5.1(b)(2)(C): the named cash-settled index symbols and the open-ended Nonstandard-Expirations bullet both close at 16:00 ET on contract expiry day, resolved through `SessionInfo::effective_close_us(event_date, exp_date)`.

Out of scope: market calendar (open / closed / half-day), index constituents.

## CLI

```bash
hourskit-cli session --symbol SPX [--unit us|ms|s] [--format text|json]
hourskit-cli manifest   # regenerate data/manifest.json
hourskit-cli refresh    # force fresh fetch from upstream
```

Install with `cargo install --path cli` from the repo root.

## Data

Sessions are scoped per `(symbol, trading class)` and stored microsecond-native: every window is `i64` microseconds since midnight US/Eastern, converted to milliseconds or seconds at the API surface on request. The crate ships `data/sessions.parquet` (one file, 14 columns, ZSTD level 3). Every copy the client reads is verified against `data/manifest.json` by SHA-256, and the data is available offline after the first query.

Run `cargo run --example seed_data && hourskit-cli manifest` to regenerate the parquet and its manifest; CI fails if they drift.

Sources:

1. CBOE C1 Rule Book, Rule 5.1(b)(1), 5.1(b)(2)(C). <https://cdn.cboe.com/resources/regulation/rule_book/C1_Exchange_Rule_Book.pdf>
2. OPRA, "Revised OPRA GTH Hours of Operation", effective 2024-08-26. <https://cdn.opraplan.com/documents/notices/Revised_OPRA_GTH_Hours_of_Operation_Eff_082624.pdf>
3. Firstrade, "Options that trade until 4:15 PM Eastern Time". <https://help.firstrade.info/en/articles/9264922-options-that-trade-until-4-15-pm-eastern-time-utc-5>
4. NASDAQ Trader, Options Market Hours. <https://www.nasdaqtrader.com/Trader.aspx?id=optionshours>
5. Nasdaq Stock Market, Global Trading Hours FAQ. <https://www.nasdaq.com/docs/nasdaq-global-trading-hours-faqs>
6. Market-data vendor operational coverage (live-data roster cross-validation).

## Cache

The stateful client caches the parquet under `~/.cache/hourskit/` (XDG) and revalidates with `If-None-Match`. Entries refresh after 24 hours; override the ceiling with `Hourskit::with_staleness_ceiling`.

Environment overrides: `HOURSKIT_BASE_URL`, `HOURSKIT_CACHE_DIR`, `HOURSKIT_MIRROR_URL`, `HOURSKIT_DATA_DIR`.

## API

Full API reference is on [docs.rs](https://docs.rs/hourskit).

## License

Dual-licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).
