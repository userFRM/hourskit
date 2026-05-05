//! Generate the bundled `data/sessions.parquet`.
//!
//! Run from the hourskit repo root:
//!
//! ```bash
//! cargo run --example seed_data
//! ```
//!
//! ## Source citations
//!
//! The bundled session data is cross-referenced against five authoritative
//! public sources. The rosters DO NOT agree perfectly — broker, exchange,
//! and operational data each draw a slightly different line. `hourskit`
//! ships the ThetaData operational superset because it carries the
//! broadest live-data coverage; the broker / exchange subsets are retained
//! as cross-validation references in the comment block on
//! [`EXTENDED_TRADING_ROSTER`] below.
//!
//! 1. **Cboe C1 Rule Book — Rule 5.1(b)(1) and 5.1(b)(2)(C)**: primary
//!    source for the regular-session 09:30-16:15 ET cutoff on cash-
//!    settled index options, the 16:15-17:00 ET Curb session, and the
//!    last-trading-day early close at 16:00 ET. Verbatim from Rule
//!    5.1(b)(2)(C):
//!
//!    > On their last trading day, Regular Trading Hours for the
//!    > following options are from 9:30 a.m. to 4:00 p.m.:
//!    > — Cboe S&P 500 AM/PM Basis options
//!    > — Index Options with Nonstandard Expirations (i.e., Weeklys
//!    >   and EOMs), Monthly Options Series, Quarterly Options
//!    >   Series, and Quarterly Expirations (i.e., QIXs)
//!    > — SPX options (p.m.-settled)
//!    > — XSP options (p.m.-settled)
//!    > — SPEQF options (p.m.-settled)
//!    > — SPEQX options (p.m.-settled)
//!    > — MRUT options (p.m.-settled)
//!
//!    The rule has TWO layers: (a) an explicitly-named roster (SPXPM,
//!    XSPPM via "SPX (PM Expiration)" / "XSP (PM Expiration)", SPEQF,
//!    SPEQX, MRUT) and (b) an open-ended catch-all for "Index Options
//!    with Nonstandard Expirations" covering every Cboe C1 Weekly /
//!    EOM / Monthly / Quarterly / QIX series. The kit encodes (a)
//!    per-root in [`LAST_TRADING_DAY_EARLY_CLOSE_ROOTS`] and (b) at the
//!    class level via [`hourskit::TradingClass::class_level_last_trading_day_close_us`].
//!    <https://cdn.cboe.com/resources/regulation/rule_book/C1_Exchange_Rule_Book.pdf>
//! 2. **Firstrade Help Center — "Options that trade until 4:15 PM Eastern
//!    Time (UTC-5)"** (broker-curated, 79 symbols, last updated
//!    2026-02-10). Also the source for the verbatim last-trading-day
//!    quote: "On the last trading day, expiring NDXP, RUTW, MRUT, SPXW,
//!    XSP, OEX & XEO options will trade until 4:00 pm ET, and
//!    non-expiring options will continue to trade until 4:15 pm ET."
//!    <https://help.firstrade.info/en/articles/9264922-options-that-trade-until-4-15-pm-eastern-time-utc-5>
//! 3. **NASDAQ Trader Options Market Hours** (NASDAQ NOM exchange-published,
//!    64 symbols):
//!    <https://www.nasdaqtrader.com/Trader.aspx?id=optionshours>
//! 4. **OPRA "Revised OPRA GTH Hours of Operation"** (effective trade date
//!    2024-08-26): primary source for the Sunday-night Global Trading
//!    Hours session 20:15 ET prior trading day to 09:25 ET current trading
//!    day (the close was extended from 09:15 to 09:25):
//!    <https://cdn.opraplan.com/documents/notices/Revised_OPRA_GTH_Hours_of_Operation_Eff_082624.pdf>
//! 5. **ThetaData operational documentation**: cross-validation of the
//!    live-data roster used for tick filtering, including legacy / niche
//!    listings (e.g. AUM, AUX, BACD, JPMD, MSTD) that pre-date some
//!    broker-side curation:
//!    <https://docs.thetadata.us/>
//!
//! Nasdaq Stock Market 21:00-04:00 ET Extended Session (2026 Global
//! Trading Hours program) is sourced from Nasdaq's own announcement:
//! <https://www.nasdaq.com/docs/nasdaq-global-trading-hours-faqs>
//!
//! ## Roster mapping into `TradingClass`
//!
//! - The cash-settled index options that get the additional Curb session
//!   (16:15-17:00 ET) and OPRA GTH (20:15 prior to 09:25 current) map to
//!   [`TradingClass::OptionsCboeC1`].
//! - Every other entry on the roster maps to
//!   [`TradingClass::OptionsCboeBzxC2Edgx`] with a 16:15 ET regular close
//!   and no Curb / GTH.
//!
//! ## NOT in scope of this seed
//!
//! Cboe-listed options outside the roster: still close at 16:15 ET in
//! practice, but the kit defers to the explicit roster as the source of
//! truth for "extended-hours eligible" until a maintainer adds them.

#![allow(clippy::print_stdout)]

use std::path::PathBuf;

use hourskit::session::{SessionInfo, Settlement, TimeWindow, TradingClass};
use hourskit::sources::parquet_io::write_sessions;

fn main() -> hourskit::Result<()> {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    std::fs::create_dir_all(&data_dir)?;

    let mut rows: Vec<SessionInfo> = Vec::new();
    rows.extend(extended_trading_options());
    rows.extend(equity_nasdaq());
    rows.extend(equity_nyse_arca());
    rows.extend(equity_cboe_bzx_edgx());
    rows.extend(equity_cboe_byx_edga());

    write_sessions(&data_dir, &rows)?;

    println!(
        "seed parquet written: {} rows -> {}",
        rows.len(),
        data_dir.join("sessions.parquet").display()
    );
    Ok(())
}

// ── Extended-trading roster ──────────────────────────────────────────────────

/// Cboe-listed options eligible to trade until 16:15 ET — 82 entries.
///
/// # Cross-reference against the five public sources
///
/// The five sources cited in the module doc do NOT agree perfectly; this
/// const ships the ThetaData operational superset and tracks each subset:
///
/// - **79 entries**: Firstrade ∩ ThetaData (broker-curated 2026-02-10 list,
///   intersected with the live-data roster ThetaData maintains).
/// - **64 entries**: NASDAQ Trader ∩ ThetaData (NASDAQ NOM
///   exchange-published Options Market Hours page, intersected with the
///   live-data roster).
/// - **~30 entries are ThetaData-only** legacy / niche listings that
///   pre-date some broker-side curation:
///   `AUM`, `AUX`, `BACD`, `BPX`, `BRB`, `BSZ`, `BVZ`, `CDD`, `CITD`,
///   `GAZ`, `GBP`, `GSSD`, `JJC`, `JPMD`, `MLPN`, `MNX`, `MSTD`, `NDO`,
///   `NZD`, `OIL`, `PZO`, `RVX`, `SFC`, `SKA`, `SLX`, `VIIX`, `VXEEM`,
///   `VXST`, `YUK`.
///
/// `SPX (PM Expiration)` and `XSP (AM Expiration)` are kept verbatim as
/// distinct root strings so the parquet remains directly queryable for
/// downstream analytics that need to discriminate the AM/PM-settled
/// variant explicitly.
const EXTENDED_TRADING_ROSTER: &[&str] = &[
    "AUM",
    "AUX",
    "BACD",
    "BPX",
    "BRB",
    "BSZ",
    "BVZ",
    "CDD",
    "CITD",
    "DBA",
    "DBB",
    "DBC",
    "DBO",
    "DBS",
    "DIA",
    "DJX",
    "EEM",
    "EFA",
    "EUI",
    "EUU",
    "GAZ",
    "GBP",
    "GSSD",
    "IWM",
    "IWN",
    "IWO",
    "IWV",
    "JJC",
    "JPMD",
    "KBE",
    "KRE",
    "MDY",
    "MLPN",
    "MNX",
    "MOO",
    "MRUT",
    "MSTD",
    "NDO",
    "NDX",
    "NZD",
    "OEF",
    "OEX",
    "OIL",
    "PZO",
    "QQQ",
    "RUT",
    "RVX",
    "SFC",
    "SKA",
    "SLX",
    "SPX",
    "SPX (PM Expiration)",
    "SPY",
    "SVXY",
    "UNG",
    "UUP",
    "UVIX",
    "UVXY",
    "VIIX",
    "VIX",
    "VIXM",
    "VIXY",
    "VXEEM",
    "VXST",
    "VXX",
    "VXZ",
    "XEO",
    "XHB",
    "XLB",
    "XLE",
    "XLF",
    "XLI",
    "XLK",
    "XLP",
    "XLU",
    "XLV",
    "XLY",
    "XME",
    "XRT",
    "XSP",
    "XSP (AM Expiration)",
    "YUK",
];

/// Roster members that are cash-settled index / volatility options on Cboe
/// C1, eligible for the 16:15-17:00 Curb session AND the OPRA 20:15 prior
/// to 09:25 current GTH window.
///
/// All of these MUST appear in [`EXTENDED_TRADING_ROSTER`] above. Roots
/// that are C1 but NOT on the 82-symbol roster (e.g. SPXW, NDXP, SPXPM,
/// SPEQF, SPEQX, "XSP (PM Expiration)") are seeded by the bottom of
/// [`extended_trading_options`] with `TradingClass::OptionsCboeC1`
/// hard-coded.
const C1_ROOTS: &[&str] = &[
    "SPX",
    "SPX (PM Expiration)",
    "XSP",
    "XSP (AM Expiration)",
    "RUT",
    "MRUT",
    "VIX",
    "NDX",
    "OEX",
    "XEO",
    "DJX",
    "MNX",
];

/// Roots that close at 16:00 ET on contract expiry day per CBOE Rule
/// 5.1(b)(2)(C) — the explicitly-named subset.
///
/// # Source — verbatim
///
/// CBOE Rule 5.1(b)(2)(C):
///
/// > On their last trading day, Regular Trading Hours for the following
/// > options are from 9:30 a.m. to 4:00 p.m.:
/// > — Cboe S&P 500 AM/PM Basis options
/// > — Index Options with Nonstandard Expirations (i.e., Weeklys and
/// >   EOMs), Monthly Options Series, Quarterly Options Series, and
/// >   Quarterly Expirations (i.e., QIXs)
/// > — SPX options (p.m.-settled)
/// > — XSP options (p.m.-settled)
/// > — SPEQF options (p.m.-settled)
/// > — SPEQX options (p.m.-settled)
/// > — MRUT options (p.m.-settled)
///
/// Firstrade extended-trading-hours notice (broker-side restatement):
///
/// > On the last trading day, expiring NDXP, RUTW, MRUT, SPXW, XSP, OEX
/// > & XEO options will trade until 4:00 pm ET, and non-expiring options
/// > will continue to trade until 4:15 pm ET.
///
/// # Per-root vs. class-level encoding
///
/// `hourskit` encodes the rule on TWO layers:
///
/// 1. **Per-root** (this constant) — explicitly enumerates every named
///    root from the rule + the broker quote, so seed-data lookups
///    surface a typed `Some(57_600_000_000)` directly on the
///    [`SessionInfo`] row.
/// 2. **Class-level catch-all** —
///    [`hourskit::TradingClass::class_level_last_trading_day_close_us`]
///    returns the override for the entire `OptionsCboeC1` family. This
///    captures the open-ended "Index Options with Nonstandard
///    Expirations" bullet so future Cboe additions get the rule's
///    16:00 ET cutoff with no parquet refresh.
///
/// `NDXP`, `RUTW`, `SPXW`, `SPXPM`, `SPEQF`, `SPEQX`, `XSP (PM Expiration)`
/// are NOT on `EXTENDED_TRADING_ROSTER` above (they sit on the broader
/// Cboe C1 ladder); we still seed them here so callers querying these
/// roots find a row.
const LAST_TRADING_DAY_EARLY_CLOSE_ROOTS: &[&str] = &[
    // Firstrade-quoted roster (cash-settled named explicitly):
    "NDXP",
    "RUTW",
    "MRUT",
    "SPXW",
    "XSP",
    "OEX",
    "XEO",
    // Rule 5.1(b)(2)(C) p.m.-settled roster (explicitly named):
    "SPXPM",
    "SPEQF",
    "SPEQX",
    // Literal-root string-encoded variants on the 82-symbol
    // EXTENDED_TRADING_ROSTER (Firstrade encodes SPX p.m.-settled as a
    // distinct root string):
    "SPX (PM Expiration)",
    // Rule-derived addition: XSP p.m.-settled mirrors SPX (PM Expiration)
    // as a literal-root variant. Not in the user-supplied
    // EXTENDED_TRADING_ROSTER, but the rule names it explicitly so we
    // seed an additional row.
    "XSP (PM Expiration)",
];

/// 16:00 ET expressed in microseconds-of-day.
const LAST_TRADING_DAY_CLOSE_US: i64 = 57_600_000_000;

/// 09:30 ET expressed in microseconds-of-day — the AM SET print time
/// for SPX standard third-Friday expirations.
const AM_SET_PRINT_US: i64 = 9 * 3_600 * 1_000_000 + 30 * 60 * 1_000_000;

/// Roster of roots whose **standard third-Friday-of-the-month**
/// expirations are AM-settled per CBOE methodology. The settlement
/// rule is encoded on the `SessionInfo` row as
/// [`Settlement::AmOpen { open_us_of_day: AM_SET_PRINT_US }`]; the
/// per-expiration classifier in
/// [`hourskit::SessionInfo::settlement_cutoff_us`] then returns the
/// AM print time only on third-Friday expirations and falls back to
/// the PM cutoff on every other expiration of the same root
/// (weeklies, EOM, mid-week, etc.).
///
/// Today the roster contains SPX only — the original CBOE VIX
/// constituent. XSP, NDX, RUT and other AM-settled cash-settled index
/// roots are tracked under a separate scope (see hourskit issue #8
/// follow-ups) so this PR ships the bit-exact rule the published VIX
/// methodology calls for and nothing more.
const AM_SET_THIRD_FRIDAY_ROOTS: &[&str] = &["SPX"];

fn extended_trading_options() -> Vec<SessionInfo> {
    let regular = TimeWindow::from_clock_et(9, 30, 16, 15);
    let curb = TimeWindow::from_clock_et(16, 15, 17, 0);
    let gth = TimeWindow::from_clock_et(20, 15, 9, 25); // prior-day open, current-day close

    let mut rows: Vec<SessionInfo> = EXTENDED_TRADING_ROSTER
        .iter()
        .map(|r| {
            let class_is_c1 = C1_ROOTS.contains(r);
            let last_trading_day_close_us = if LAST_TRADING_DAY_EARLY_CLOSE_ROOTS.contains(r) {
                Some(LAST_TRADING_DAY_CLOSE_US)
            } else {
                None
            };
            let settlement = if AM_SET_THIRD_FRIDAY_ROOTS.contains(r) {
                Settlement::AmOpen {
                    open_us_of_day: AM_SET_PRINT_US,
                }
            } else {
                Settlement::Pm
            };
            SessionInfo {
                root: (*r).to_string(),
                trading_class: if class_is_c1 {
                    TradingClass::OptionsCboeC1
                } else {
                    TradingClass::OptionsCboeBzxC2Edgx
                },
                regular,
                pre_market: None,
                post_market: None,
                curb: if class_is_c1 { Some(curb) } else { None },
                gth: if class_is_c1 { Some(gth) } else { None },
                gth_overnight: class_is_c1,
                last_trading_day_close_us,
                settlement,
            }
        })
        .collect();

    // The Rule 5.1(b)(2)(C) explicit roster includes Cboe C1 series that
    // aren't on the EXTENDED_TRADING_ROSTER list above (NDXP, RUTW, SPXW,
    // SPXPM, SPEQF, SPEQX, XSP (PM Expiration)). Seed them as
    // OptionsCboeC1 with the same Curb + GTH treatment their underlier
    // carries, so callers querying any of the named early-close roots get
    // a row directly. Every name in this carry-over set is PM-settled
    // (SPXW weeklies, SPXPM, SPEQF, SPEQX, "XSP (PM Expiration)" all
    // explicitly PM by rule); none qualify for AM SET treatment.
    for &root in LAST_TRADING_DAY_EARLY_CLOSE_ROOTS {
        if EXTENDED_TRADING_ROSTER.contains(&root) {
            continue;
        }
        rows.push(SessionInfo {
            root: root.to_string(),
            trading_class: TradingClass::OptionsCboeC1,
            regular,
            pre_market: None,
            post_market: None,
            curb: Some(curb),
            gth: Some(gth),
            gth_overnight: true,
            last_trading_day_close_us: Some(LAST_TRADING_DAY_CLOSE_US),
            settlement: Settlement::Pm,
        });
    }
    rows
}

// ── EquityNasdaq ──────────────────────────────────────────────────────────────

/// Nasdaq-listed equity (bellwether sample roster).
///
/// - Regular session 09:30-16:00 ET
/// - Pre-market 04:00-09:30 ET
/// - Post-market 16:00-20:00 ET
/// - NEW Extended Session 21:00-04:00 ET (Nasdaq Global Trading Hours, 2026 program)
fn equity_nasdaq() -> Vec<SessionInfo> {
    let regular = TimeWindow::from_clock_et(9, 30, 16, 0);
    let pre = TimeWindow::from_clock_et(4, 0, 9, 30);
    let post = TimeWindow::from_clock_et(16, 0, 20, 0);
    // Nasdaq Extended Session: 21:00 ET to 04:00 ET next morning. We tag
    // this as "overnight" because the open and close span midnight (open
    // numerically > close).
    let gth = TimeWindow::from_clock_et(21, 0, 4, 0);
    let roots = [
        "AAPL", "MSFT", "AMZN", "NVDA", "META", "GOOGL", "GOOG", "TSLA", "AMD", "NFLX", "INTC",
        "CSCO", "ADBE", "PEP", "AVGO", "COST", "QCOM", "TXN", "TMUS", "QQQ", "SOXX", "SMH", "TQQQ",
        "SQQQ",
    ];
    roots
        .iter()
        .map(|r| SessionInfo {
            root: (*r).to_string(),
            trading_class: TradingClass::EquityNasdaq,
            regular,
            pre_market: Some(pre),
            post_market: Some(post),
            curb: None,
            gth: Some(gth),
            gth_overnight: true,
            last_trading_day_close_us: None,
            settlement: Settlement::Pm,
        })
        .collect()
}

// ── EquityNyseArca ────────────────────────────────────────────────────────────

/// NYSE Arca-listed equity (bellwether sample roster).
fn equity_nyse_arca() -> Vec<SessionInfo> {
    let regular = TimeWindow::from_clock_et(9, 30, 16, 0);
    let pre = TimeWindow::from_clock_et(4, 0, 9, 30);
    let post = TimeWindow::from_clock_et(16, 0, 20, 0);
    let roots = [
        "SPY", "DIA", "IWM", "EEM", "EFA", "IVV", "VOO", "VTI", "IJR", "IJH", "IWB", "IWD", "IWF",
        "IWO", "XLF", "XLK", "XLE", "XLV", "XLI", "XLY", "XLP", "XLU", "XLB", "XLRE", "XLC", "GDX",
        "GDXJ", "TLT", "IEF", "SHY", "HYG", "LQD", "TIP", "GLD", "SLV", "USO", "UNG", "VXX",
        "UVXY", "BRK.B",
    ];
    roots
        .iter()
        .map(|r| SessionInfo {
            root: (*r).to_string(),
            trading_class: TradingClass::EquityNyseArca,
            regular,
            pre_market: Some(pre),
            post_market: Some(post),
            curb: None,
            gth: None,
            gth_overnight: false,
            last_trading_day_close_us: None,
            settlement: Settlement::Pm,
        })
        .collect()
}

// ── EquityCboeBzxEdgx ─────────────────────────────────────────────────────────

/// Cboe BZX / EDGX equities — early-trading window from 02:30 ET.
///
/// We model the early-trading window AS the pre_market field; the standard
/// 04:00-09:30 ET window concatenates with it. Callers wanting pure 04:00
/// pre-market should consult [`TradingClass::EquityNasdaq`] or
/// [`TradingClass::EquityNyseArca`] data for the same root.
fn equity_cboe_bzx_edgx() -> Vec<SessionInfo> {
    let regular = TimeWindow::from_clock_et(9, 30, 16, 0);
    let pre = TimeWindow::from_clock_et(2, 30, 9, 30);
    let post = TimeWindow::from_clock_et(16, 0, 20, 0);
    let roots = ["SPY", "QQQ", "IWM", "DIA"];
    roots
        .iter()
        .map(|r| SessionInfo {
            root: (*r).to_string(),
            trading_class: TradingClass::EquityCboeBzxEdgx,
            regular,
            pre_market: Some(pre),
            post_market: Some(post),
            curb: None,
            gth: None,
            gth_overnight: false,
            last_trading_day_close_us: None,
            settlement: Settlement::Pm,
        })
        .collect()
}

// ── EquityCboeByxEdga ─────────────────────────────────────────────────────────

/// Cboe BYX / EDGA equities — pre-market starts at 07:00 ET.
fn equity_cboe_byx_edga() -> Vec<SessionInfo> {
    let regular = TimeWindow::from_clock_et(9, 30, 16, 0);
    let pre = TimeWindow::from_clock_et(7, 0, 9, 30);
    let post = TimeWindow::from_clock_et(16, 0, 20, 0);
    let roots = ["SPY", "QQQ", "IWM", "DIA"];
    roots
        .iter()
        .map(|r| SessionInfo {
            root: (*r).to_string(),
            trading_class: TradingClass::EquityCboeByxEdga,
            regular,
            pre_market: Some(pre),
            post_market: Some(post),
            curb: None,
            gth: None,
            gth_overnight: false,
            last_trading_day_close_us: None,
            settlement: Settlement::Pm,
        })
        .collect()
}
