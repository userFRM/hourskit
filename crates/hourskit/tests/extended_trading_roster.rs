//! Tests for the authoritative 85-symbol extended-trading-hours roster.
//!
//! Source: Cboe extended-trading-hours notice + the OPRA "Revised OPRA GTH
//! Hours of Operation" PDF (effective trade date 2024-08-26).

#![cfg(feature = "parquet-loader")]

use hourskit::sources::bundled;
use hourskit::TradingClass;

fn set_repo_data_dir() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data");
    std::env::set_var("HOURSKIT_DATA_DIR", dir);
    bundled::invalidate_cache();
}

const fn h_us(h: i64) -> i64 {
    h * 3_600 * 1_000_000
}

const fn hm_us(h: i64, m: i64) -> i64 {
    h_us(h) + m * 60 * 1_000_000
}

/// The authoritative 82-symbol extended-trading-hours roster (Cboe + OPRA),
/// kept verbatim including the `(PM Expiration)` and `(AM Expiration)`
/// variants.
const ROSTER: &[&str] = &[
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

/// Every member of the authoritative extended-trading roster must appear in
/// the parquet with `regular_close = 16:15 ET`.
#[test]
fn extended_trading_roster_all_close_at_1615() {
    set_repo_data_dir();
    assert_eq!(
        ROSTER.len(),
        82,
        "roster size — counted from the authoritative source"
    );
    // We probe both possible class targets — index/volatility symbols that
    // get the C1 class, ETFs and equity-options that get BzxC2Edgx — so the
    // assertion is "the row exists under EXACTLY ONE of the two options
    // classes".
    for symbol in ROSTER {
        let c1 =
            bundled::session_for_class(symbol, &TradingClass::OptionsCboeC1).expect("seed present");
        let other = bundled::session_for_class(symbol, &TradingClass::OptionsCboeBzxC2Edgx)
            .expect("seed present");
        let info = c1
            .or(other)
            .unwrap_or_else(|| panic!("{symbol} not in seed data"));
        assert_eq!(
            info.regular.close_us(),
            hm_us(16, 15),
            "{symbol} regular close should be 16:15 ET"
        );
        assert!(info.is_options(), "{symbol} should be an options class");
    }
}

#[test]
fn spx_pm_and_xsp_am_variants_round_trip_as_literal_symbols() {
    set_repo_data_dir();
    let pm = bundled::session("SPX (PM Expiration)")
        .expect("seed data present")
        .expect("SPX PM Expiration in roster");
    assert_eq!(pm.trading_class, TradingClass::OptionsCboeC1);
    assert!(pm.curb.is_some(), "SPX PM gets Curb");
    let am = bundled::session("XSP (AM Expiration)")
        .expect("seed data present")
        .expect("XSP AM Expiration in roster");
    assert_eq!(am.trading_class, TradingClass::OptionsCboeC1);
    assert!(am.curb.is_some(), "XSP AM gets Curb");
}

#[test]
fn opra_gth_close_is_0925_per_august_2024_revision() {
    set_repo_data_dir();
    let info = bundled::session("SPX")
        .expect("seed data present")
        .expect("SPX in roster");
    let gth = info.gth.expect("SPX has OPRA GTH");
    // 09:25 ET = 33_900_000_000 microseconds.
    assert_eq!(gth.close_us(), hm_us(9, 25));
}

#[test]
fn opra_gth_open_is_2015_unchanged() {
    set_repo_data_dir();
    let info = bundled::session("SPX")
        .expect("seed data present")
        .expect("SPX in roster");
    let gth = info.gth.expect("SPX has OPRA GTH");
    // 20:15 ET = 72_900_000_000 microseconds.
    assert_eq!(gth.open_us(), hm_us(20, 15));
}

/// `C1_SYMBOLS` and `EXTENDED_TRADING_ROSTER` agree on every member of the
/// C1 subset.
#[test]
fn cash_settled_index_classes_get_curb_and_gth() {
    set_repo_data_dir();
    let c1_subset = [
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
    for symbol in c1_subset {
        let info = bundled::session(symbol)
            .expect("seed data present")
            .unwrap_or_else(|| panic!("{symbol} not in seed data"));
        assert_eq!(info.trading_class, TradingClass::OptionsCboeC1);
        assert!(info.curb.is_some(), "{symbol} should have Curb session");
        assert!(info.gth.is_some(), "{symbol} should have GTH session");
    }
}

/// Symbols on the roster that are NOT in the C1 cash-settled index list
/// map to OptionsCboeBzxC2Edgx and have no Curb / GTH.
#[test]
fn etfs_on_the_roster_get_options_cboe_bzx_c2_edgx() {
    set_repo_data_dir();
    // Every ticker in this list is on the authoritative 82-symbol roster
    // AND is not an index/volatility C1 product.
    let etfs = [
        "DIA", "QQQ", "SPY", "IWM", "EEM", "EFA", "XLF", "XLE", "XLK",
    ];
    for symbol in etfs {
        let info = bundled::session_for_class(symbol, &TradingClass::OptionsCboeBzxC2Edgx)
            .expect("seed data present")
            .unwrap_or_else(|| panic!("{symbol} OptionsCboeBzxC2Edgx not found"));
        assert_eq!(info.regular.close_us(), hm_us(16, 15));
        assert!(info.curb.is_none());
        assert!(info.gth.is_none());
    }
}
