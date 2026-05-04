//! Tests for [`hourskit::TimeUnit`] parsing, accessor consistency, and
//! microsecond-native invariants.

use std::str::FromStr;

use hourskit::{TimeUnit, TimeWindow};

#[test]
fn time_unit_parses_canonical_short_forms() {
    assert_eq!(
        TimeUnit::from_str("us").expect("us"),
        TimeUnit::Microseconds
    );
    assert_eq!(
        TimeUnit::from_str("ms").expect("ms"),
        TimeUnit::Milliseconds
    );
    assert_eq!(TimeUnit::from_str("s").expect("s"), TimeUnit::Seconds);
}

#[test]
fn time_unit_parses_long_forms() {
    assert_eq!(
        TimeUnit::from_str("microseconds").expect("microseconds"),
        TimeUnit::Microseconds,
    );
    assert_eq!(
        TimeUnit::from_str("milliseconds").expect("milliseconds"),
        TimeUnit::Milliseconds,
    );
    assert_eq!(
        TimeUnit::from_str("seconds").expect("seconds"),
        TimeUnit::Seconds,
    );
}

#[test]
fn time_unit_is_case_insensitive_and_trims_whitespace() {
    assert_eq!(
        TimeUnit::from_str("  US  ").expect("us"),
        TimeUnit::Microseconds
    );
    assert_eq!(
        TimeUnit::from_str("MS").expect("ms"),
        TimeUnit::Milliseconds
    );
    assert_eq!(TimeUnit::from_str("Sec").expect("s"), TimeUnit::Seconds);
}

#[test]
fn time_unit_rejects_unknown_input() {
    assert!(TimeUnit::from_str("ns").is_err());
    assert!(TimeUnit::from_str("hours").is_err());
    assert!(TimeUnit::from_str("").is_err());
}

#[test]
fn time_unit_display_round_trips_through_from_str() {
    for unit in [
        TimeUnit::Microseconds,
        TimeUnit::Milliseconds,
        TimeUnit::Seconds,
    ] {
        let s = unit.to_string();
        let back = TimeUnit::from_str(&s).expect("display round-trip");
        assert_eq!(unit, back);
    }
}

#[test]
fn time_window_open_in_matches_unit_specific_accessor() {
    let w = TimeWindow::from_clock_et(9, 30, 16, 0);
    assert_eq!(w.open_in(TimeUnit::Microseconds), w.open_us());
    assert_eq!(w.open_in(TimeUnit::Milliseconds), w.open_ms());
    assert_eq!(w.open_in(TimeUnit::Seconds), w.open_secs());
}

#[test]
fn time_window_close_in_matches_unit_specific_accessor() {
    let w = TimeWindow::from_clock_et(9, 30, 16, 0);
    assert_eq!(w.close_in(TimeUnit::Microseconds), w.close_us());
    assert_eq!(w.close_in(TimeUnit::Milliseconds), w.close_ms());
    assert_eq!(w.close_in(TimeUnit::Seconds), w.close_secs());
}

#[test]
fn microsecond_storage_is_lossless_for_minute_boundary() {
    // 16:15:00 ET is 58 500 000 ms = 58 500 000 000 us.
    let w = TimeWindow::from_clock_et(0, 0, 16, 15);
    assert_eq!(w.close_us(), 58_500_000_000_i64);
    assert_eq!(w.close_ms(), 58_500_000_i64);
    assert_eq!(w.close_secs(), 58_500_i64);
}

#[test]
fn time_unit_serde_round_trips() {
    let value = TimeUnit::Microseconds;
    let s = serde_json::to_string(&value).expect("serialize");
    assert_eq!(s, "\"microseconds\"");
    let back: TimeUnit = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back, value);
}
