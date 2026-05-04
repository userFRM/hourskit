# hourskit

Reference data for US exchange trading hours, scoped per `(symbol,
trading class)`. Microsecond-of-day storage, parquet on disk, SHA-256
verified, no API keys.

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

Last-trading-day exception per CBOE Rule 5.1(b)(2)(C): the named cash-
settled index roots and the open-ended Nonstandard-Expirations bullet
both close at 16:00 ET on contract expiry day. Resolved through
`SessionInfo::effective_close_us(event_date, exp_date)`.

Out of scope: market calendar (open / closed / half-day), index
constituents.

## Quick start

```rust
use hourskit::{Hourskit, TradingClass};

#[tokio::main]
async fn main() -> hourskit::Result<()> {
    // Free function — process-wide client.
    let info = hourskit::session("SPX").await?.expect("SPX in seed");
    println!("regular {}, curb {:?}, gth {:?}", info.regular, info.curb, info.gth);

    // Stateful client — connection pool reused.
    let client = Hourskit::new();
    let qqq = client
        .session_for_class("QQQ", &TradingClass::EquityNasdaq)
        .await?
        .expect("QQQ EquityNasdaq");
    println!("QQQ open {} us-of-day", qqq.regular.open_us());

    // Last-trading-day cutoff for an SPXW contract expiring today.
    let spxw = hourskit::session("SPXW").await?.expect("SPXW");
    let _close = spxw.effective_close_us(20_260_516, 20_260_516); // 16:00 ET
    Ok(())
}
```

Blocking variants: `hourskit::session_blocking(root)`,
`Hourskit::session_blocking(root)`,
`Hourskit::session_for_class_blocking(root, class)`,
`Hourskit::sessions_all_blocking()`.

## API

| Symbol | Description |
|---|---|
| `Hourskit` | Stateful client. Builder methods: `with_base_url`, `with_cache_dir`, `with_mirror_url`, `with_staleness_ceiling`. |
| `Hourskit::session(root)` / `session_for_class(root, class)` / `sessions_all()` | Async lookups + their `_blocking` siblings. |
| `hourskit::session(root)` / `session_blocking(root)` | Free-function shortcuts via a process-wide client. |
| `SessionInfo` | `regular`, `pre_market`, `post_market`, `curb`, `gth`, `gth_overnight`, `last_trading_day_close_us`. |
| `SessionInfo::effective_close_us(event_date, exp_date)` | Per-contract close-of-trading microsecond-of-day. |
| `SessionInfo::option_close_us` / `last_trading_us` / `is_options` / `is_equity` | Read-only accessors. |
| `TimeWindow` | Half-open `[open, close)` window with `open_*`, `close_*`, `duration_*`, `contains_us`, `contains_us_overnight`, and `*_in(unit)` accessors. |
| `TimeUnit` | `Microseconds` / `Milliseconds` / `Seconds`. Implements `FromStr` (`"us"` / `"ms"` / `"s"`). |
| `TradingClass` | Venue / asset-class identifier with `as_wire`, `from_wire`, `is_options`, `is_equity`, `preference_rank`, `class_level_last_trading_day_close_us`. |
| `Error` / `Result<T>` | Unified error enum + alias. |
| `sources::bundled` | Synchronous offline reader for `data/sessions.parquet`: `session`, `session_for_class`, `sessions_all`, `invalidate_cache`. |
| `sources::parquet_io::{read_sessions, write_sessions, FILE_SESSIONS}` | Direct parquet I/O. |

## Reference data

The crate ships `data/sessions.parquet` (one file, 14 columns, ZSTD
level 3) and mirrors it at
`https://raw.githubusercontent.com/userFRM/hourskit/main/data/sessions.parquet`.
The stateful client fetches the GitHub copy, caches under
`~/.cache/hourskit/` (XDG), and revalidates with `If-None-Match`. After
24 hours the fetcher forces a fresh body regardless of ETag — override
with `Hourskit::with_staleness_ceiling`. Each fetch is checked against
`data/manifest.json` (SHA-256). On 5xx / network error the stale cache
is served. Concurrent callers for the same key share one in-flight
request. Cache writes are tmp + fsync + rename. Run
`cargo run --example seed_data && hourskit-cli manifest` to refresh
both files; CI fails if they drift.

Environment overrides: `HOURSKIT_BASE_URL`, `HOURSKIT_CACHE_DIR`,
`HOURSKIT_MIRROR_URL`, `HOURSKIT_DATA_DIR`.

## CLI

```
hourskit-cli session --root SPX [--unit us|ms|s] [--format text|json]
hourskit-cli manifest                                    # regenerate data/manifest.json
hourskit-cli refresh                                     # force fresh fetch from upstream
```

Install: `cargo install --path cli` from the repo root.

## Sources

1. CBOE C1 Rule Book — Rule 5.1(b)(1), 5.1(b)(2)(C). <https://www.cboe.com/us/options/membership/rule_book/c1/>
2. OPRA — "Revised OPRA GTH Hours of Operation", effective 2024-08-26. <https://cdn.opraplan.com/documents/notices/Revised_OPRA_GTH_Hours_of_Operation_Eff_082624.pdf>
3. Firstrade — "Options that trade until 4:15 PM Eastern Time". <https://help.firstrade.info/en/articles/9264922-options-that-trade-until-4-15-pm-eastern-time-utc-5>
4. NASDAQ Trader — Options Market Hours. <https://www.nasdaqtrader.com/Trader.aspx?id=optionshours>
5. Nasdaq Stock Market — Global Trading Hours FAQ. <https://www.nasdaq.com/docs/nasdaq-global-trading-hours-faqs>
6. ThetaData docs. <https://docs.thetadata.us/>

## License

Dual-licensed under MIT or Apache-2.0, at the user's option. See
`LICENSE-MIT` and `LICENSE-APACHE`.

Copyright 2026 userFRM.
