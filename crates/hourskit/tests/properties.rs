//! Property-based invariants for the hourskit type and parquet layers.
//!
//! Each test asserts a relationship that must hold for ALL valid inputs in
//! the (small, integer-bounded) domain. proptest's default 256 cases per
//! invariant is sufficient given the scale.

#![cfg(feature = "parquet-loader")]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::missing_panics_doc)]

use hourskit::session::{SessionInfo, TimeUnit, TimeWindow, TradingClass};
use proptest::prelude::*;

const US_PER_DAY: i64 = 86_400 * 1_000_000;

// ── TimeWindow: hour/minute decomposition is stable ─────────────────────────

proptest! {
    /// `from_clock_et(h, m, h, m+1)` always produces a non-empty 1-minute window.
    #[test]
    fn time_window_clock_constructor_round_trips(
        h in 0i64..23,
        m in 0i64..59,
    ) {
        let w = TimeWindow::from_clock_et(h, m, h, m + 1);
        prop_assert_eq!(w.duration_secs(), 60);
        prop_assert_eq!(w.duration_ms(), 60_000);
        prop_assert_eq!(w.duration_us(), 60_000_000);
    }
}

proptest! {
    /// open / close accessors preserve the exact storage value.
    #[test]
    fn time_window_accessors_are_pure_division(
        open in 0i64..US_PER_DAY,
        close in 0i64..US_PER_DAY,
    ) {
        let w = TimeWindow::new(open, close);
        prop_assert_eq!(w.open_us(), open);
        prop_assert_eq!(w.close_us(), close);
        prop_assert_eq!(w.open_ms(), open / 1_000);
        prop_assert_eq!(w.close_ms(), close / 1_000);
        prop_assert_eq!(w.open_secs(), open / 1_000_000);
        prop_assert_eq!(w.close_secs(), close / 1_000_000);
    }
}

proptest! {
    /// `contains_us` is half-open at both endpoints when used as a same-day window.
    #[test]
    fn time_window_contains_is_half_open(
        open in 0i64..US_PER_DAY,
        delta in 1i64..(US_PER_DAY / 4),
    ) {
        let close = (open + delta).min(US_PER_DAY - 1);
        let w = TimeWindow::new(open, close);
        if open < close {
            prop_assert!(w.contains_us(open));
            prop_assert!(w.contains_us(close - 1));
            prop_assert!(!w.contains_us(close));
        }
    }
}

proptest! {
    /// `duration_in(unit)` agrees with the unit-specific `duration_*` accessors.
    #[test]
    fn time_window_duration_in_matches_explicit_accessors(
        open in 0i64..US_PER_DAY,
        delta in 0i64..(US_PER_DAY / 2),
    ) {
        let close = (open + delta).min(US_PER_DAY - 1);
        let w = TimeWindow::new(open, close);
        prop_assert_eq!(w.duration_in(TimeUnit::Microseconds), w.duration_us());
        prop_assert_eq!(w.duration_in(TimeUnit::Milliseconds), w.duration_ms());
        prop_assert_eq!(w.duration_in(TimeUnit::Seconds),      w.duration_secs());
    }
}

// ── TradingClass: wire-string round-trip ─────────────────────────────────────

proptest! {
    /// Every supported variant survives wire serialisation.
    #[test]
    fn trading_class_wire_round_trip(idx in 0usize..10) {
        let class = match idx {
            0 => TradingClass::OptionsCboeC1,
            1 => TradingClass::OptionsCboeBzxC2Edgx,
            2 => TradingClass::OptionsNyseArca,
            3 => TradingClass::OptionsIse,
            4 => TradingClass::OptionsBox,
            5 => TradingClass::OptionsAmex,
            6 => TradingClass::EquityNasdaq,
            7 => TradingClass::EquityNyseArca,
            8 => TradingClass::EquityCboeBzxEdgx,
            _ => TradingClass::EquityCboeByxEdga,
        };
        let s = class.as_wire();
        let back = TradingClass::from_wire(&s);
        prop_assert_eq!(class, back);
    }
}

// ── SessionInfo: parquet round-trip ──────────────────────────────────────────

proptest! {
    /// Sessions written with arbitrary windows round-trip exactly through
    /// the parquet writer/reader.
    #[test]
    fn sessions_parquet_round_trip(
        symbols in proptest::collection::vec("[A-Z]{1,5}", 1..15),
        regular_open in 0i64..(8 * 3_600 * 1_000_000),
        delta in 1i64..(8 * 3_600 * 1_000_000),
    ) {
        use hourskit::sources::parquet_io::{read_sessions, write_sessions, FILE_SESSIONS};

        let dir = tempfile::tempdir().expect("tempdir");
        let regular_close = regular_open + delta;
        let mut rows: Vec<SessionInfo> = symbols
            .iter()
            .map(|r| SessionInfo {
                symbol: r.clone(),
                trading_class: TradingClass::EquityNasdaq,
                regular: TimeWindow::new(regular_open, regular_close),
                pre_market: None,
                post_market: None,
                curb: None,
                gth: None,
                gth_overnight: false,
                last_trading_day_close_us: None,
                settlement: hourskit::Settlement::Pm,
            })
            .collect();
        rows.sort_by(|a, b| a.symbol.cmp(&b.symbol));
        rows.dedup_by(|a, b| a.symbol == b.symbol);

        write_sessions(dir.path(), &rows).expect("write");
        let back = read_sessions(&dir.path().join(FILE_SESSIONS)).expect("read");
        prop_assert_eq!(rows.len(), back.len());
        for (a, b) in rows.iter().zip(back.iter()) {
            prop_assert_eq!(&a.symbol, &b.symbol);
            prop_assert_eq!(a.regular.open_us(),  b.regular.open_us());
            prop_assert_eq!(a.regular.close_us(), b.regular.close_us());
            prop_assert_eq!(&a.trading_class, &b.trading_class);
        }
    }
}

// ── TimeWindow: overnight membership wrapper ─────────────────────────────────

proptest! {
    /// `contains_us_overnight` matches the union of "after open" or "before close"
    /// across the full microsecond-of-day domain.
    #[test]
    fn overnight_membership_is_union_of_two_half_lines(
        open in 0i64..US_PER_DAY,
        close in 0i64..US_PER_DAY,
        probe in 0i64..US_PER_DAY,
    ) {
        let w = TimeWindow::new(open, close);
        prop_assert_eq!(
            w.contains_us_overnight(probe),
            probe >= open || probe < close,
        );
    }
}
