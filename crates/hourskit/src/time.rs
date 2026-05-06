//! Day-length and intraday-window constants in canonical integer units.
//!
//! Every analytics consumer that pairs ms-of-day arithmetic with
//! calendar-day arithmetic ends up re-typing the same handful of
//! day-length constants. This module exports them once so the
//! downstream code reads as `hourskit::time::MS_PER_DAY` instead of
//! a private `const MS_PER_DAY: i32 = 86_400_000;` per crate.
//!
//! # Unit choice
//!
//! `i32` is the canonical analytics type for ms-of-day on the FPSS
//! wire and on every shipped tick struct. `i32::MAX` (`2_147_483_647`)
//! sits comfortably above `MS_PER_DAY` (`86_400_000`), so an `i32`
//! ms-of-day reading admits the full daily span without wrap.
//!
//! # Example
//!
//! ```
//! use hourskit::time::{MIN_PER_DAY, MS_PER_DAY, SECONDS_PER_DAY};
//!
//! // FPSS ms-of-day reading — clamp on the trust boundary.
//! let ms_of_day: i32 = 57_600_000; // 16:00 ET
//! assert!((0..=MS_PER_DAY).contains(&ms_of_day));
//!
//! // Composing minute-precision `T` arithmetic.
//! assert_eq!(MS_PER_DAY, SECONDS_PER_DAY * 1_000);
//! assert_eq!(SECONDS_PER_DAY, MIN_PER_DAY * 60);
//! ```
//!
//! # Stability
//!
//! These constants are protocol facts; their values are part of the
//! public ABI of this module and will not change.

/// Milliseconds in a UTC calendar day.
///
/// `24 × 60 × 60 × 1_000 = 86_400_000`. Use as the upper bound on
/// FPSS ms-of-day stamps (the wire admits a session-midnight roll
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
}
