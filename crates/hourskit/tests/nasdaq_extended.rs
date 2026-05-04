//! Tests for the Nasdaq Stock Market Extended Session — the 21:00 ET to
//! 04:00 ET overnight Global Trading Hours program announced for 2026.

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
fn equity_nasdaq_extended_session_window_matches_2026_program() {
    set_repo_data_dir();
    let info = bundled::session_for_class("AAPL", &TradingClass::EquityNasdaq)
        .expect("seed data present")
        .expect("AAPL EquityNasdaq row");
    let gth = info.gth.expect("Nasdaq Extended Session must be present");
    // 21:00 ET to 04:00 ET (next day).
    assert_eq!(gth.open_us(), hm_us(21, 0));
    assert_eq!(gth.close_us(), hm_us(4, 0));
    assert!(
        info.gth_overnight,
        "Nasdaq Extended Session crosses midnight"
    );
}

#[test]
fn equity_nasdaq_pre_market_starts_at_0400() {
    set_repo_data_dir();
    let info = bundled::session_for_class("MSFT", &TradingClass::EquityNasdaq)
        .expect("seed data present")
        .expect("MSFT EquityNasdaq row");
    let pre = info.pre_market.expect("Nasdaq pre-market");
    assert_eq!(pre.open_us(), hm_us(4, 0));
    assert_eq!(pre.close_us(), hm_us(9, 30));
}

#[test]
fn equity_nasdaq_post_market_ends_at_2000() {
    set_repo_data_dir();
    let info = bundled::session_for_class("NVDA", &TradingClass::EquityNasdaq)
        .expect("seed data present")
        .expect("NVDA EquityNasdaq row");
    let post = info.post_market.expect("Nasdaq post-market");
    assert_eq!(post.open_us(), hm_us(16, 0));
    assert_eq!(post.close_us(), hm_us(20, 0));
}

#[test]
fn extended_session_membership_in_overnight_window() {
    set_repo_data_dir();
    let info = bundled::session_for_class("AAPL", &TradingClass::EquityNasdaq)
        .expect("seed data present")
        .expect("AAPL EquityNasdaq row");
    let gth = info.gth.expect("Nasdaq GTH");
    assert!(info.gth_overnight);
    // 22:00 ET (yesterday side) is in the window.
    assert!(gth.contains_us_overnight(hm_us(22, 0)));
    // 03:00 ET (today side) is in the window.
    assert!(gth.contains_us_overnight(hm_us(3, 0)));
    // 12:00 ET is OUTSIDE.
    assert!(!gth.contains_us_overnight(hm_us(12, 0)));
}

#[test]
fn nasdaq_class_is_disjoint_from_cboe_options_class() {
    set_repo_data_dir();
    // QQQ is on both the EquityNasdaq roster and the Cboe extended-trading
    // 85-symbol options roster, so it lets us verify the kit ships disjoint
    // class entries with different regular-close times.
    let qqq_equity = bundled::session_for_class("QQQ", &TradingClass::EquityNasdaq)
        .expect("seed data present")
        .expect("QQQ equity row");
    let qqq_options = bundled::session_for_class("QQQ", &TradingClass::OptionsCboeBzxC2Edgx)
        .expect("seed data present")
        .expect("QQQ options row");
    // Equity closes at 16:00 ET, options at 16:15 ET.
    assert_eq!(qqq_equity.regular.close_us(), hm_us(16, 0));
    assert_eq!(qqq_options.regular.close_us(), hm_us(16, 15));
}
