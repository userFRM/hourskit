//! Day-length constants and Gregorian-date arithmetic helpers in
//! canonical integer units.
//!
//! Every analytics consumer that pairs ms-of-day arithmetic with
//! calendar-day arithmetic ends up re-typing the same handful of
//! day-length constants and the same `YYYYMMDD → days-since-epoch`
//! conversion. This module exports them once so the downstream
//! code reads as `hourskit::time::MS_PER_DAY` /
//! `hourskit::time::days_between_yyyymmdd` instead of a private
//! copy per crate.
//!
//! # Unit choice
//!
//! `i32` is the canonical type for ms-of-day on the tick wire
//! and on every shipped tick struct. `i32::MAX` (`2_147_483_647`)
//! sits comfortably above `MS_PER_DAY` (`86_400_000`), so an `i32`
//! ms-of-day reading admits the full daily span without wrap. The
//! `YYYYMMDD` date helpers are also `i32` to match the wire date
//! type.
//!
//! # Example
//!
//! ```
//! use hourskit::time::{
//!     days_between_yyyymmdd, MIN_PER_DAY, MS_PER_DAY, SECONDS_PER_DAY,
//! };
//!
//! // ms-of-day reading, clamp on the trust boundary.
//! let ms_of_day: i32 = 57_600_000; // 16:00 ET
//! assert!((0..=MS_PER_DAY).contains(&ms_of_day));
//!
//! // Composing minute-precision `T` arithmetic.
//! assert_eq!(MS_PER_DAY, SECONDS_PER_DAY * 1_000);
//! assert_eq!(SECONDS_PER_DAY, MIN_PER_DAY * 60);
//!
//! // Calendar-day delta between two YYYYMMDD ints.
//! assert_eq!(days_between_yyyymmdd(20_260_515, 20_260_516), 1);
//! // Leap-year boundary: 2024 had a Feb 29 so the gap is two days.
//! assert_eq!(days_between_yyyymmdd(20_240_228, 20_240_301), 2);
//! // Non-leap year: 2025 had no Feb 29 so the gap is one day.
//! assert_eq!(days_between_yyyymmdd(20_250_228, 20_250_301), 1);
//! ```
//!
//! # RTH window
//!
//! The four `RTH_*` constants encode the SIP equity regular-trading-
//! hours window in `i32` ms-of-day and `i64` μs-of-day for callers
//! that classify a single tick without a [`crate::SessionInfo`] in
//! hand. The boundary is `[09:30 ET, 16:00 ET)` half-open in the
//! same convention as [`crate::TimeWindow`].
//!
//! ```
//! use hourskit::time::{RTH_END_MS, RTH_START_MS};
//! const NINE_THIRTY_MS: i32 = 34_200_000;
//! const FOUR_PM_MS: i32 = 57_600_000;
//! assert_eq!(RTH_START_MS, NINE_THIRTY_MS);
//! assert_eq!(RTH_END_MS, FOUR_PM_MS);
//! ```
//!
//! # Stability
//!
//! These constants are protocol facts; their values are part of the
//! public ABI of this module and will not change. The Gregorian-date
//! helpers follow the proleptic Gregorian calendar and likewise will
//! not change behaviour for any valid `YYYYMMDD` input.

/// Milliseconds in a UTC calendar day.
///
/// `24 × 60 × 60 × 1_000 = 86_400_000`. Use as the upper bound on
/// ms-of-day stamps (the wire admits a session-midnight roll
/// at `MS_PER_DAY` exactly), and as the multiplier on
/// "full days between event and settlement" inside minute-precision
/// time-to-expiration walks.
pub const MS_PER_DAY: i32 = 86_400_000;

/// Seconds in a UTC calendar day.
///
/// `24 × 60 × 60 = 86_400`. Use for second-precision wall-clock
/// arithmetic (e.g. epoch-second composition or coarse staleness
/// thresholds) where ms granularity is not required.
pub const SECONDS_PER_DAY: i32 = 86_400;

/// Minutes in a UTC calendar day.
///
/// `24 × 60 = 1_440`. Use as the multiplier on
/// "full days between event and settlement" inside minute-precision
/// time-to-expiration decompositions, and as the denominator term in
/// CBOE's `T = minutes_to_expiry / (365 × MIN_PER_DAY)` formula.
pub const MIN_PER_DAY: i32 = 1_440;

// ---------------------------------------------------------------------------
// Regular-trading-hours window (US equities)
// ---------------------------------------------------------------------------

/// Regular-trading-hours session start (09:30:00 ET) in
/// milliseconds-of-day.
///
/// `9 * 3_600_000 + 30 * 60_000 = 34_200_000`. The lower bound on
/// the SIP equity RTH window — every consolidated-tape trade with a
/// ms-of-day below this value is in the pre-market session.
///
/// Use this constant for single-tick RTH classification when the
/// caller does not have a [`crate::SessionInfo`] in hand. Callers
/// with session info should resolve through
/// [`crate::TimeWindow::contains_ms`] on the row's
/// [`crate::SessionInfo::regular`] window so per-class deviations
/// (e.g. Cboe BZX/EDGX 02:30 ET early-trading) are honoured.
pub const RTH_START_MS: i32 = 34_200_000;

/// Regular-trading-hours session end (16:00:00 ET) in
/// milliseconds-of-day.
///
/// `16 * 3_600_000 = 57_600_000`. The upper bound on the SIP equity
/// RTH window — every consolidated-tape trade with a ms-of-day at
/// or above this value is in the post-market session.
///
/// Coincides numerically with
/// [`crate::session::OPTION_PM_SETTLEMENT_MS`]; the two constants
/// have different names because the meaning is different — RTH end
/// is the equity-session boundary; PM settlement is the option-
/// trading cutoff for cash-settled index options on their last
/// trading day.
pub const RTH_END_MS: i32 = 57_600_000;

/// Regular-trading-hours session start (09:30:00 ET) in
/// microseconds-of-day.
///
/// `RTH_START_MS * 1_000 = 34_200_000_000`. Provided for callers
/// that compose with `i64` μs-of-day arithmetic on the tick wire.
pub const RTH_START_US: i64 = 34_200_000_000;

/// Regular-trading-hours session end (16:00:00 ET) in
/// microseconds-of-day.
///
/// `RTH_END_MS * 1_000 = 57_600_000_000`. Provided for callers
/// that compose with `i64` μs-of-day arithmetic on the tick wire.
pub const RTH_END_US: i64 = 57_600_000_000;

// ---------------------------------------------------------------------------
// Gregorian-date arithmetic
// ---------------------------------------------------------------------------

/// Days between the Unix epoch (1970-01-01) and the day encoded by
/// `yyyymmdd`, in the proleptic Gregorian calendar.
///
/// The input is the wire date encoding: `year * 10_000 + month *
/// 100 + day` (e.g. `20_260_516` for 2026-05-16). The output is the
/// signed integer count of days since 1970-01-01 (which itself
/// returns `0`).
///
/// # Behaviour
///
/// - Returns `0` for non-positive inputs (`yyyymmdd <= 0`) — these
///   arise from default-constructed fixtures and degrade to
///   "epoch day" rather than producing garbage.
/// - Returns `0` when the month component is outside `1..=12` or the
///   day component is outside `1..=31`. Callers that need a
///   guaranteed real day must validate the YYYYMMDD shape upstream.
/// - Negative for years before 1970 — pre-epoch dates count down
///   linearly. `days_from_epoch(19_691_231)` returns `-1`.
/// - Iterates the year span, so cost is O(|year - 1970|). Acceptable
///   for analytics use sites that call it once per option chain
///   build, not per tick.
///
/// # Example
///
/// ```
/// use hourskit::time::days_from_epoch;
///
/// assert_eq!(days_from_epoch(19_700_101), 0);
/// assert_eq!(days_from_epoch(19_700_102), 1);
/// assert_eq!(days_from_epoch(19_710_101), 365);
/// // Leap years: 1972 has 366 days.
/// assert_eq!(days_from_epoch(19_730_101), 365 + 365 + 366);
/// ```
#[must_use]
pub fn days_from_epoch(yyyymmdd: i32) -> i32 {
    if yyyymmdd <= 0 {
        return 0;
    }
    let year = yyyymmdd / 10_000;
    let month = (yyyymmdd / 100) % 100;
    let day = yyyymmdd % 100;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return 0;
    }
    let mut days: i32 = 0;
    if year >= 1970 {
        for y in 1970..year {
            days += if is_leap(y) { 366 } else { 365 };
        }
    } else {
        for y in year..1970 {
            days -= if is_leap(y) { 366 } else { 365 };
        }
    }
    let month_lengths: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        let idx = usize::try_from(m - 1).unwrap_or(0);
        let len = if m == 2 && is_leap(year) {
            29
        } else {
            month_lengths[idx]
        };
        days += len;
    }
    days + day - 1
}

/// Calendar-day delta between two `YYYYMMDD` dates.
///
/// `YYYYMMDD` ints are not linear in days — `20_260_601 - 20_260_531
/// = 70`, but the true delta is `1`. This helper converts each end
/// to days-since-epoch via [`days_from_epoch`] and returns the
/// signed difference. Callers that want a non-negative delta should
/// pre-order their dates; the analytics consumers always pass
/// `(event_date, expiration_date)` with `event <= expiration` and
/// rely on the caller-side ordering rather than asking this helper
/// to clamp.
///
/// # Example
///
/// ```
/// use hourskit::time::days_between_yyyymmdd;
///
/// assert_eq!(days_between_yyyymmdd(20_260_515, 20_260_516), 1);
/// assert_eq!(days_between_yyyymmdd(20_260_516, 20_260_515), -1);
/// // Leap-year boundary: Feb 28 → Mar 1 is 2 days in 2024.
/// assert_eq!(days_between_yyyymmdd(20_240_228, 20_240_301), 2);
/// // Non-leap-year boundary: Feb 28 → Mar 1 is 1 day in 2025.
/// assert_eq!(days_between_yyyymmdd(20_250_228, 20_250_301), 1);
/// ```
#[must_use]
pub fn days_between_yyyymmdd(from: i32, to: i32) -> i32 {
    days_from_epoch(to) - days_from_epoch(from)
}

#[inline]
const fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_length_constants_match_known_values() {
        assert_eq!(MS_PER_DAY, 86_400_000);
        assert_eq!(SECONDS_PER_DAY, 86_400);
        assert_eq!(MIN_PER_DAY, 1_440);
    }

    #[test]
    fn day_length_constants_are_internally_consistent() {
        assert_eq!(MS_PER_DAY, SECONDS_PER_DAY * 1_000);
        assert_eq!(SECONDS_PER_DAY, MIN_PER_DAY * 60);
        assert_eq!(MS_PER_DAY, MIN_PER_DAY * 60 * 1_000);
    }

    // Compile-time guard rather than a runtime test — the assertion
    // pattern would otherwise fire `clippy::assertions_on_constants`,
    // and the const block surfaces any future type-shrink as a
    // build break instead of a runtime failure.
    const _: () = {
        assert!(MS_PER_DAY < i32::MAX);
        assert!(SECONDS_PER_DAY < i32::MAX);
        assert!(MIN_PER_DAY < i32::MAX);
    };

    // ---------------------------------------------------------------
    //  RTH window
    // ---------------------------------------------------------------

    #[test]
    fn rth_window_constants_match_clock_values() {
        // 09:30:00 ET in milliseconds-of-day.
        assert_eq!(RTH_START_MS, 9 * 3_600_000 + 30 * 60_000);
        // 16:00:00 ET in milliseconds-of-day.
        assert_eq!(RTH_END_MS, 16 * 3_600_000);
        // 09:30:00 ET in microseconds-of-day.
        assert_eq!(
            RTH_START_US,
            9_i64 * 3_600 * 1_000_000 + 30_i64 * 60 * 1_000_000
        );
        // 16:00:00 ET in microseconds-of-day.
        assert_eq!(RTH_END_US, 16_i64 * 3_600 * 1_000_000);
    }

    // Compile-time guards: μs and ms variants must agree under a
    // 1_000-factor rescale.
    const _: () = {
        assert!(RTH_START_US == (RTH_START_MS as i64) * 1_000);
        assert!(RTH_END_US == (RTH_END_MS as i64) * 1_000);
        assert!(RTH_START_MS < RTH_END_MS);
        assert!(RTH_START_US < RTH_END_US);
        // RTH window fits inside MS_PER_DAY upper bound.
        assert!(RTH_END_MS < MS_PER_DAY);
    };

    #[test]
    fn rth_window_classifies_typical_session_stamps() {
        // 09:29:59 ET → just before RTH open → pre-market.
        let pre_open_ms: i32 = 9 * 3_600_000 + 29 * 60_000 + 59_000;
        assert!(pre_open_ms < RTH_START_MS);
        // 12:00:00 ET → mid-session.
        let noon_ms: i32 = 12 * 3_600_000;
        assert!((RTH_START_MS..RTH_END_MS).contains(&noon_ms));
        // 16:00:00.001 ET → first post-market.
        let post_close_ms: i32 = 16 * 3_600_000 + 1;
        assert!(post_close_ms >= RTH_END_MS);
    }

    // ---------------------------------------------------------------
    //  Gregorian-date helpers
    // ---------------------------------------------------------------

    #[test]
    fn days_from_epoch_anchors_at_1970_01_01() {
        assert_eq!(days_from_epoch(19_700_101), 0);
        assert_eq!(days_from_epoch(19_700_102), 1);
        assert_eq!(days_from_epoch(19_700_201), 31);
        assert_eq!(days_from_epoch(19_710_101), 365);
    }

    #[test]
    fn days_from_epoch_handles_leap_years() {
        // 1972 is a leap year (div 4, not div 100).
        // 1970 (365) + 1971 (365) = 730 days to 1972-01-01.
        assert_eq!(days_from_epoch(19_720_101), 730);
        // 1972-12-31 is day 365 of the year (366 days, indices 0..365).
        assert_eq!(days_from_epoch(19_721_231), 730 + 365);
        // 1973-01-01 is the day after, +1 reflects 366-day year 1972.
        assert_eq!(days_from_epoch(19_730_101), 730 + 366);
        // 2000 is a leap year (div 400).
        // 2024 is a leap year (div 4, not div 100).
        assert_eq!(
            days_between_yyyymmdd(20_240_228, 20_240_301),
            2,
            "leap-year Feb 28 → Mar 1 is 2 days"
        );
        // 1900 is NOT a leap year (div 100, not div 400).
        // 2025 is NOT a leap year.
        assert_eq!(
            days_between_yyyymmdd(20_250_228, 20_250_301),
            1,
            "non-leap-year Feb 28 → Mar 1 is 1 day"
        );
    }

    #[test]
    fn days_from_epoch_rejects_invalid_inputs() {
        assert_eq!(days_from_epoch(0), 0);
        assert_eq!(days_from_epoch(-1), 0);
        assert_eq!(days_from_epoch(i32::MIN), 0);
        // Month out of band.
        assert_eq!(days_from_epoch(20_261_301), 0);
        // Day out of band.
        assert_eq!(days_from_epoch(20_260_500), 0);
        assert_eq!(days_from_epoch(20_260_532), 0);
    }

    #[test]
    fn days_from_epoch_handles_pre_epoch_dates() {
        // 1969-12-31 is one day before the epoch.
        assert_eq!(days_from_epoch(19_691_231), -1);
        // 1969-01-01 is 365 days before 1970-01-01.
        assert_eq!(days_from_epoch(19_690_101), -365);
        // 1968 is a leap year.
        assert_eq!(days_from_epoch(19_680_101), -365 - 366);
    }

    #[test]
    fn days_between_yyyymmdd_inverts_correctly() {
        let from = 20_260_515;
        let to = 20_260_523;
        assert_eq!(days_between_yyyymmdd(from, to), 8);
        assert_eq!(days_between_yyyymmdd(to, from), -8);
        assert_eq!(days_between_yyyymmdd(from, from), 0);
    }

    #[test]
    fn days_between_yyyymmdd_crosses_year_boundary() {
        // 2025-12-31 → 2026-01-01 is one day.
        assert_eq!(days_between_yyyymmdd(20_251_231, 20_260_101), 1);
        // 2026-01-01 → 2027-01-01 is 365 days (2026 is not a leap year).
        assert_eq!(days_between_yyyymmdd(20_260_101, 20_270_101), 365);
        // 2024-01-01 → 2025-01-01 is 366 days (2024 is a leap year).
        assert_eq!(days_between_yyyymmdd(20_240_101, 20_250_101), 366);
    }

    #[test]
    fn days_between_yyyymmdd_step_walk_matches_reference() {
        // Walk a sample of dates and confirm the days-from-epoch
        // count matches a direct day-by-day step counter for every
        // valid date in 2026 (a representative non-leap year).
        let mut day_counter: i32 = days_from_epoch(20_260_101);
        let month_lengths_2026: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        for (m_idx, m_len) in month_lengths_2026.iter().enumerate() {
            for d in 1..=*m_len {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                let yyyymmdd = 2026 * 10_000 + (m_idx as i32 + 1) * 100 + d;
                assert_eq!(
                    days_from_epoch(yyyymmdd),
                    day_counter,
                    "step-walk mismatch on {yyyymmdd}"
                );
                day_counter += 1;
            }
        }
    }

    #[test]
    fn days_between_yyyymmdd_step_walk_2024_leap_year() {
        // 2024 is a leap year — 366 days.
        let start = days_from_epoch(20_240_101);
        let end = days_from_epoch(20_241_231);
        assert_eq!(
            end - start,
            365,
            "leap year walk: 366 days minus the +0 anchor"
        );
    }

    /// Independent reference implementation, kept to confirm output-byte
    /// parity across every valid YYYYMMDD in a 1970..=2099 sweep.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    fn analytics_reference_days_from_epoch(yyyymmdd: i32) -> i32 {
        let y = yyyymmdd / 10_000;
        let m = (yyyymmdd / 100) % 100;
        let d = yyyymmdd % 100;
        let mut days = 0;
        for yi in 1970..y {
            days += if is_leap(yi) { 366 } else { 365 };
        }
        let months: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        for mi in 1..m {
            let idx = usize::try_from(mi - 1).unwrap_or(0);
            let len = if mi == 2 && is_leap(y) {
                29
            } else {
                months[idx]
            };
            days += len;
        }
        days + d - 1
    }

    #[test]
    fn days_from_epoch_matches_analytics_reference_across_long_sweep() {
        // Sweep every valid date in 1970..=2099. Roughly 47k iterations.
        // Confirms drop-in parity for the downstream import.
        for year in 1970..=2099 {
            let month_lengths: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            for (m_idx, base_len) in month_lengths.iter().enumerate() {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                let m = m_idx as i32 + 1;
                let len = if m == 2 && is_leap(year) {
                    29
                } else {
                    *base_len
                };
                for d in 1..=len {
                    let yyyymmdd = year * 10_000 + m * 100 + d;
                    assert_eq!(
                        days_from_epoch(yyyymmdd),
                        analytics_reference_days_from_epoch(yyyymmdd),
                        "mismatch on {yyyymmdd}"
                    );
                }
            }
        }
    }
}
