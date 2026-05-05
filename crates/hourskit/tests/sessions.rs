//! Integration tests for the unified session endpoint.
//!
//! These tests exercise the bundled-parquet reader against the seed data
//! shipped in `data/sessions.parquet`. They never touch the network.

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

#[test]
fn spx_resolves_to_options_cboe_c1() {
    set_repo_data_dir();
    let info = bundled::session("SPX")
        .expect("data/sessions.parquet must be present")
        .expect("SPX in seed data");
    assert_eq!(info.trading_class, TradingClass::OptionsCboeC1);
    assert_eq!(info.regular.open_us(), hm_us(9, 30));
    assert_eq!(info.regular.close_us(), hm_us(16, 15));
}

#[test]
fn spx_has_curb_window_1615_to_1700() {
    set_repo_data_dir();
    let info = bundled::session("SPX")
        .expect("data/sessions.parquet must be present")
        .expect("SPX in seed data");
    let curb = info.curb.expect("SPX has Curb");
    assert_eq!(curb.open_us(), hm_us(16, 15));
    assert_eq!(curb.close_us(), hm_us(17, 0));
}

#[test]
fn spx_gth_is_overnight_2015_prior_to_0925_current() {
    set_repo_data_dir();
    let info = bundled::session("SPX")
        .expect("data/sessions.parquet must be present")
        .expect("SPX in seed data");
    let gth = info.gth.expect("SPX has GTH");
    assert!(info.gth_overnight, "Cboe C1 GTH wraps midnight");
    assert_eq!(gth.open_us(), hm_us(20, 15));
    assert_eq!(gth.close_us(), hm_us(9, 25));
}

#[test]
fn vix_xsp_rut_are_options_cboe_c1_with_curb_and_gth() {
    set_repo_data_dir();
    // VIX, XSP, RUT, MRUT, NDX, OEX, XEO, DJX, MNX are the cash-settled
    // index/volatility entries on the authoritative extended-trading roster.
    for root in [
        "VIX", "XSP", "RUT", "MRUT", "NDX", "OEX", "XEO", "DJX", "MNX",
    ] {
        let info = bundled::session(root)
            .expect("data/sessions.parquet must be present")
            .unwrap_or_else(|| panic!("{root} in seed data"));
        assert_eq!(info.trading_class, TradingClass::OptionsCboeC1);
        assert!(info.curb.is_some(), "{root} should have a Curb window");
        assert!(info.gth.is_some(), "{root} should have a GTH window");
        assert!(info.gth_overnight, "{root} GTH wraps midnight");
    }
}

#[test]
fn aapl_is_equity_nasdaq_with_pre_post_and_extended_session() {
    set_repo_data_dir();
    let info = bundled::session_for_class("AAPL", &TradingClass::EquityNasdaq)
        .expect("data/sessions.parquet must be present")
        .expect("AAPL EquityNasdaq in seed data");
    assert_eq!(info.regular.open_us(), hm_us(9, 30));
    assert_eq!(info.regular.close_us(), hm_us(16, 0));
    let pre = info.pre_market.expect("AAPL has pre-market");
    assert_eq!(pre.open_us(), hm_us(4, 0));
    assert_eq!(pre.close_us(), hm_us(9, 30));
    let post = info.post_market.expect("AAPL has post-market");
    assert_eq!(post.open_us(), hm_us(16, 0));
    assert_eq!(post.close_us(), hm_us(20, 0));
    let gth = info.gth.expect("AAPL has Nasdaq Extended Session (GTH)");
    assert_eq!(gth.open_us(), hm_us(21, 0));
    assert_eq!(gth.close_us(), hm_us(4, 0));
    assert!(info.gth_overnight, "Nasdaq Extended Session spans midnight");
}

#[test]
fn qqq_resolves_to_some_session() {
    set_repo_data_dir();
    let info = bundled::session("QQQ")
        .expect("data/sessions.parquet must be present")
        .expect("QQQ in seed data");
    // QQQ has rows for multiple classes — `session(root)` returns one of them.
    assert_eq!(info.regular.open_us(), hm_us(9, 30));
}

#[test]
fn qqq_equity_nasdaq_has_extended_session() {
    set_repo_data_dir();
    let info = bundled::session_for_class("QQQ", &TradingClass::EquityNasdaq)
        .expect("data/sessions.parquet must be present")
        .expect("QQQ EquityNasdaq in seed data");
    assert!(info.gth.is_some());
    assert!(info.gth_overnight);
}

#[test]
fn cboe_byx_edga_pre_market_starts_at_0700() {
    set_repo_data_dir();
    let info = bundled::session_for_class("SPY", &TradingClass::EquityCboeByxEdga)
        .expect("data/sessions.parquet must be present")
        .expect("SPY BYX/EDGA in seed data");
    let pre = info.pre_market.expect("BYX/EDGA has pre-market");
    assert_eq!(pre.open_us(), hm_us(7, 0));
    assert_eq!(pre.close_us(), hm_us(9, 30));
}

#[test]
fn cboe_bzx_edgx_pre_market_starts_at_0230() {
    set_repo_data_dir();
    let info = bundled::session_for_class("SPY", &TradingClass::EquityCboeBzxEdgx)
        .expect("data/sessions.parquet must be present")
        .expect("SPY BZX/EDGX in seed data");
    let pre = info.pre_market.expect("BZX/EDGX has early-trading + pre");
    assert_eq!(pre.open_us(), hm_us(2, 30));
    assert_eq!(pre.close_us(), hm_us(9, 30));
}

#[test]
fn unknown_root_returns_none() {
    set_repo_data_dir();
    let result = bundled::session("ZZNOTAREALROOT").expect("data/sessions.parquet must be present");
    assert!(result.is_none());
}

#[test]
fn lookup_is_case_insensitive() {
    set_repo_data_dir();
    let upper = bundled::session("SPX").expect("seed data present");
    let lower = bundled::session("spx").expect("seed data present");
    let mixed = bundled::session("Spx").expect("seed data present");
    assert!(upper.is_some());
    assert_eq!(upper, lower);
    assert_eq!(upper, mixed);
}
