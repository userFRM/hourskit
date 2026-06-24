//! Tests for the cash-settled-index last-trading-day exception
//! (CBOE Rule 5.1(b)(2)(C)).
//!
//! Quote (Firstrade extended-trading-hours notice): "On the last trading
//! day, expiring NDXP, RUTW, MRUT, SPXW, XSP, OEX & XEO options will
//! trade until 4:00 pm ET, and non-expiring options will continue to
//! trade until 4:15 pm ET."
//!
//! Rule 5.1(b)(2)(C) additionally covers Cboe S&P 500 AM/PM Basis
//! options, every Index Option with Nonstandard Expirations (Weeklys,
//! EOMs, Monthly Series, Quarterly Series, QIXs), and the SPX / XSP /
//! SPEQF / SPEQX / MRUT p.m.-settled named symbols. The kit encodes the
//! named subset per-symbol and the open-ended Nonstandard-Expirations
//! catch-all at the [`TradingClass`] level.

#![cfg(feature = "parquet-loader")]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::missing_panics_doc)]

use hourskit::session::{SessionInfo, TimeWindow};
use hourskit::sources::bundled;
use hourskit::TradingClass;
use proptest::prelude::*;

fn set_repo_data_dir() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data");
    std::env::set_var("HOURSKIT_DATA_DIR", dir);
    bundled::invalidate_cache();
}

const SEVEN_EARLY_CLOSE_SYMBOLS: &[&str] = &["NDXP", "RUTW", "MRUT", "SPXW", "XSP", "OEX", "XEO"];

const EARLY_CLOSE_US: i64 = 57_600_000_000; // 16:00 ET
const REGULAR_CLOSE_US: i64 = 58_500_000_000; // 16:15 ET

#[test]
fn last_trading_day_exception_applies_to_seven_symbols() {
    set_repo_data_dir();
    for symbol in SEVEN_EARLY_CLOSE_SYMBOLS {
        let info = bundled::session_for_class(symbol, &TradingClass::OptionsCboeC1)
            .expect("seed data present")
            .unwrap_or_else(|| panic!("{symbol} not found in seed data"));
        assert_eq!(
            info.last_trading_day_close_us,
            Some(EARLY_CLOSE_US),
            "{symbol} should carry the 16:00 ET last-trading-day override"
        );
    }
}

#[test]
fn per_symbol_override_field_is_none_for_unenumerated_symbols() {
    set_repo_data_dir();
    // The PER-SYMBOL override field is populated only for symbols explicitly
    // named in CBOE Rule 5.1(b)(2)(C) and the Firstrade quote. SPX / NDX
    // / RUT / VIX (cash-settled but not in the named subset), SPY / QQQ
    // (ETF options) all carry None on the per-symbol field. The
    // OptionsCboeC1 entries among them still resolve to 16:00 ET via
    // the class-level fallback — see
    // [`spx_class_level_fallback_kicks_in_when_per_symbol_override_absent`].
    for symbol in ["SPX", "NDX", "RUT", "VIX", "SPY", "QQQ"] {
        let info = bundled::session(symbol)
            .expect("seed data present")
            .unwrap_or_else(|| panic!("{symbol} not found in seed data"));
        assert_eq!(
            info.last_trading_day_close_us, None,
            "{symbol} must not carry an EXPLICIT per-symbol override"
        );
    }
}

#[test]
fn effective_close_us_returns_early_close_on_expiry_day() {
    set_repo_data_dir();
    let info = bundled::session_for_class("SPXW", &TradingClass::OptionsCboeC1)
        .expect("seed data present")
        .expect("SPXW in seed data");
    let event_date = 20_260_516;
    let exp_date = 20_260_516;
    assert_eq!(
        info.effective_close_us(event_date, exp_date),
        EARLY_CLOSE_US
    );
}

#[test]
fn effective_close_us_returns_regular_close_on_non_expiry_day() {
    set_repo_data_dir();
    let info = bundled::session_for_class("SPXW", &TradingClass::OptionsCboeC1)
        .expect("seed data present")
        .expect("SPXW in seed data");
    let event_date = 20_260_516;
    let exp_date = 20_260_517;
    assert_eq!(
        info.effective_close_us(event_date, exp_date),
        REGULAR_CLOSE_US
    );
}

#[test]
fn effective_close_us_returns_regular_close_for_unaffected_classes() {
    set_repo_data_dir();
    // SPY OptionsCboeBzxC2Edgx is NOT OptionsCboeC1, so neither the
    // per-symbol override nor the class-level Rule 5.1(b)(2)(C) fallback
    // applies. Effective close stays at the regular 16:15 ET cutoff
    // even when the contract expires today.
    let info = bundled::session_for_class("SPY", &TradingClass::OptionsCboeBzxC2Edgx)
        .expect("seed data present")
        .expect("SPY OptionsCboeBzxC2Edgx in seed data");
    let day = 20_260_516;
    assert_eq!(info.effective_close_us(day, day), REGULAR_CLOSE_US);
    assert_eq!(info.last_trading_day_close_us, None);
}

#[test]
fn spx_class_level_fallback_kicks_in_when_per_symbol_override_absent() {
    set_repo_data_dir();
    // SPX is OptionsCboeC1 but lacks a per-symbol override (Rule
    // 5.1(b)(2)(C) names "SPX p.m.-settled" specifically; AM-settled
    // SPX continues to 16:15 in practice but the rule's open-ended
    // Nonstandard-Expirations bullet sweeps it in too). Verify the
    // class-level fallback resolves SPX on its expiry day.
    let info = bundled::session_for_class("SPX", &TradingClass::OptionsCboeC1)
        .expect("seed data present")
        .expect("SPX OptionsCboeC1 in seed data");
    assert_eq!(info.last_trading_day_close_us, None);
    let day = 20_260_516;
    assert_eq!(
        info.effective_close_us(day, day),
        EARLY_CLOSE_US,
        "SPX should fall through to the class-level Rule 5.1(b)(2)(C) fallback"
    );
    // Non-expiry day still returns the regular 16:15 close.
    assert_eq!(info.effective_close_us(day, day + 1), REGULAR_CLOSE_US);
}

#[test]
fn effective_close_us_xsp_oex_xeo_round_trip() {
    set_repo_data_dir();
    // The remaining symbols on the seven-symbol list must each early-close
    // on their own expiry day. Pin the class so every lookup hits the
    // OptionsCboeC1 row carrying the override.
    for symbol in ["XSP", "OEX", "XEO", "MRUT"] {
        let info = bundled::session_for_class(symbol, &TradingClass::OptionsCboeC1)
            .expect("seed data present")
            .unwrap_or_else(|| panic!("{symbol} not found"));
        let day = 20_260_519;
        assert_eq!(
            info.effective_close_us(day, day),
            EARLY_CLOSE_US,
            "{symbol} should early-close on contract expiry day"
        );
    }
}

// ── Class-level fallback (Rule 5.1(b)(2)(C) catch-all) ──────────────────────

/// Synthesize a SessionInfo with `OptionsCboeC1` and no per-symbol
/// override. The class-level fallback should still resolve the early
/// close on the contract's expiry day. This covers the rule's
/// open-ended "Index Options with Nonstandard Expirations" bullet for
/// any future Cboe C1 symbol the kit doesn't yet enumerate per-symbol.
#[test]
fn class_level_fallback_applies_to_all_options_cboe_c1_without_explicit_override() {
    let synthetic = SessionInfo {
        symbol: "FUTUREC1WEEKLY".into(),
        trading_class: TradingClass::OptionsCboeC1,
        regular: TimeWindow::from_clock_et(9, 30, 16, 15),
        pre_market: None,
        post_market: None,
        curb: Some(TimeWindow::from_clock_et(16, 15, 17, 0)),
        gth: None,
        gth_overnight: false,
        last_trading_day_close_us: None,
        settlement: hourskit::Settlement::Pm,
        valid_from_yyyymmdd: None,
    };
    // No per-symbol override populated; class-level fallback must trigger.
    assert_eq!(
        synthetic.effective_close_us(20_260_516, 20_260_516),
        EARLY_CLOSE_US,
        "OptionsCboeC1 class-level fallback should yield 16:00 ET"
    );
    // On a non-expiry day the regular close still applies.
    assert_eq!(
        synthetic.effective_close_us(20_260_516, 20_260_517),
        synthetic.regular.close_us()
    );
}

#[test]
fn per_symbol_override_takes_precedence_over_class_level_fallback() {
    // Hypothetical 15:58:20 ET = 57_500_000_000 us — chosen to be
    // distinguishable from the canonical 16:00 ET = 57_600_000_000 us
    // class-level fallback so we can prove which branch fires first.
    const HYPOTHETICAL_PER_SYMBOL_OVERRIDE: i64 = 57_500_000_000;
    let synthetic = SessionInfo {
        symbol: "EXPLICITSYMBOL".into(),
        trading_class: TradingClass::OptionsCboeC1,
        regular: TimeWindow::from_clock_et(9, 30, 16, 15),
        pre_market: None,
        post_market: None,
        curb: None,
        gth: None,
        gth_overnight: false,
        last_trading_day_close_us: Some(HYPOTHETICAL_PER_SYMBOL_OVERRIDE),
        settlement: hourskit::Settlement::Pm,
        valid_from_yyyymmdd: None,
    };
    let day = 20_260_516;
    assert_eq!(
        synthetic.effective_close_us(day, day),
        HYPOTHETICAL_PER_SYMBOL_OVERRIDE,
        "per-symbol override must win over the class-level fallback"
    );
}

#[test]
fn class_level_fallback_returns_none_for_equity_classes() {
    use hourskit::TradingClass;
    assert_eq!(
        TradingClass::EquityNasdaq.class_level_last_trading_day_close_us(),
        None
    );
    assert_eq!(
        TradingClass::EquityNyseArca.class_level_last_trading_day_close_us(),
        None
    );
    assert_eq!(
        TradingClass::EquityCboeBzxEdgx.class_level_last_trading_day_close_us(),
        None
    );
    assert_eq!(
        TradingClass::EquityCboeByxEdga.class_level_last_trading_day_close_us(),
        None
    );
    // Other options classes also return None (only OptionsCboeC1 is governed).
    assert_eq!(
        TradingClass::OptionsCboeBzxC2Edgx.class_level_last_trading_day_close_us(),
        None
    );
    assert_eq!(
        TradingClass::OptionsNyseArca.class_level_last_trading_day_close_us(),
        None
    );
    assert_eq!(
        TradingClass::OptionsIse.class_level_last_trading_day_close_us(),
        None
    );
    assert_eq!(
        TradingClass::OptionsBox.class_level_last_trading_day_close_us(),
        None
    );
    assert_eq!(
        TradingClass::OptionsAmex.class_level_last_trading_day_close_us(),
        None
    );
}

#[test]
fn options_cboe_c1_class_level_fallback_is_1600_et() {
    use hourskit::TradingClass;
    assert_eq!(
        TradingClass::OptionsCboeC1.class_level_last_trading_day_close_us(),
        Some(EARLY_CLOSE_US),
    );
}

#[test]
fn spxpm_speqf_speqx_explicitly_seeded_with_override() {
    set_repo_data_dir();
    for symbol in ["SPXPM", "SPEQF", "SPEQX", "XSP (PM Expiration)"] {
        let info = bundled::session_for_class(symbol, &TradingClass::OptionsCboeC1)
            .expect("seed data present")
            .unwrap_or_else(|| panic!("{symbol} not found in seed data"));
        assert_eq!(
            info.last_trading_day_close_us,
            Some(EARLY_CLOSE_US),
            "{symbol} should carry the explicit per-symbol last-trading-day override"
        );
        // Sanity: the explicit override and the class-level fallback agree.
        let day = 20_260_516;
        assert_eq!(info.effective_close_us(day, day), EARLY_CLOSE_US);
    }
}

#[test]
fn rule_full_named_roster_is_complete() {
    // Verify every CBOE Rule 5.1(b)(2)(C) explicitly-named symbol resolves
    // to the early close via either the per-symbol override or the
    // class-level fallback. NB: "Cboe S&P 500 AM/PM Basis options" is
    // covered solely by the class-level fallback and has no dedicated
    // symbol string in the seed data.
    set_repo_data_dir();
    for symbol in [
        // Firstrade-quoted:
        "NDXP",
        "RUTW",
        "MRUT",
        "SPXW",
        "XSP",
        "OEX",
        "XEO",
        // Rule-named p.m.-settled:
        "SPXPM",
        "SPEQF",
        "SPEQX",
        // Literal-symbol variants:
        "SPX (PM Expiration)",
        "XSP (PM Expiration)",
    ] {
        let info = bundled::session_for_class(symbol, &TradingClass::OptionsCboeC1)
            .expect("seed data present")
            .unwrap_or_else(|| panic!("{symbol} not found"));
        let day = 20_260_519;
        assert_eq!(
            info.effective_close_us(day, day),
            EARLY_CLOSE_US,
            "{symbol} must early-close on contract expiry day per Rule 5.1(b)(2)(C)"
        );
    }
}

// ── Proptest invariants ─────────────────────────────────────────────────────

proptest! {
    /// For any (event_date, exp_date) pair, `effective_close_us` is
    /// monotone with respect to event_date relative to exp_date:
    ///
    /// - When event_date == exp_date and an override is configured,
    ///   the result equals the override (which is <= regular.close_us
    ///   because 16:00 ET <= 16:15 ET).
    /// - When event_date != exp_date, the result equals regular.close_us.
    /// - The result never exceeds regular.close_us.
    #[test]
    fn effective_close_is_bounded_above_by_regular_close(
        event_date in 20_200_101_i32..20_400_101,
        exp_offset in -10_000_i32..10_000,
    ) {
        set_repo_data_dir();
        let exp_date = event_date.saturating_add(exp_offset);
        for symbol in SEVEN_EARLY_CLOSE_SYMBOLS {
            let info = bundled::session_for_class(symbol, &TradingClass::OptionsCboeC1)
                .expect("seed data present")
                .unwrap_or_else(|| panic!("{symbol} not found"));
            let close = info.effective_close_us(event_date, exp_date);
            prop_assert!(close <= info.regular.close_us());
            if event_date == exp_date {
                prop_assert_eq!(close, EARLY_CLOSE_US);
            } else {
                prop_assert_eq!(close, info.regular.close_us());
            }
        }
    }
}
