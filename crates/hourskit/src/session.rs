//! Unified session model — `SessionInfo`, `TimeWindow`, `TimeUnit`, `TradingClass`.
//!
//! `hourskit` is scoped per `(symbol, trading_class)`: every public lookup
//! resolves a symbol's [`TradingClass`] and returns a single [`SessionInfo`]
//! carrying the full slate of windows that apply to it (regular session,
//! pre-market, post-market, Curb, Global Trading Hours).
//!
//! # Why microsecond-native
//!
//! Wall-clock data on US exchanges already arrives at microsecond precision
//! (OPRA, NASDAQ TotalView, NYSE Pillar). Storing milliseconds-of-day forces
//! callers to round on every tick comparison. `hourskit` stores
//! microseconds-of-day end-to-end and exposes [`TimeUnit`] so a caller picks
//! the unit at the API boundary, not the storage layer.
//!
//! # Example
//!
//! ```
//! use hourskit::{SessionInfo, Settlement, TimeUnit, TimeWindow, TradingClass};
//!
//! // Construct directly when bypassing the bundled / fetched data plane.
//! let regular = TimeWindow::from_clock_et(9, 30, 16, 0);
//! let info = SessionInfo {
//!     symbol: "AAPL".into(),
//!     trading_class: TradingClass::EquityNasdaq,
//!     regular,
//!     pre_market: Some(TimeWindow::from_clock_et(4, 0, 9, 30)),
//!     post_market: Some(TimeWindow::from_clock_et(16, 0, 20, 0)),
//!     curb: None,
//!     gth: Some(TimeWindow::from_clock_et(21, 0, 4, 0)),
//!     gth_overnight: true,
//!     last_trading_day_close_us: None,
//!     settlement: Settlement::Pm,
//!     valid_from_yyyymmdd: None,
//! };
//! // The regular session is 6h30m long; the duration accessor honours `TimeUnit`.
//! assert_eq!(info.regular.duration_secs(), 6 * 3_600 + 30 * 60);
//! assert_eq!(
//!     info.regular.duration_in(TimeUnit::Microseconds),
//!     info.regular.duration_secs() * 1_000_000,
//! );
//! ```

use serde::{Deserialize, Serialize};
use std::str::FromStr;

// ---------------------------------------------------------------------------
// TimeUnit
// ---------------------------------------------------------------------------

/// Unit of wall-clock-time-of-day measurement returned by accessor methods.
///
/// `hourskit` stores every endpoint as `i64` microseconds-of-day; the unit
/// selector lets callers (CLI, downstream SDKs) consume the same data in
/// whichever resolution suits the call site without precision loss.
///
/// # Example
///
/// ```
/// use hourskit::TimeUnit;
/// use std::str::FromStr;
///
/// assert_eq!(TimeUnit::from_str("us").expect("parse"), TimeUnit::Microseconds);
/// assert_eq!(TimeUnit::from_str("ms").expect("parse"), TimeUnit::Milliseconds);
/// assert_eq!(TimeUnit::from_str("s").expect("parse"),  TimeUnit::Seconds);
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum TimeUnit {
    /// Microseconds — native storage unit. Use for tick-precise comparisons.
    Microseconds,
    /// Milliseconds — common bus / message-queue payload unit.
    Milliseconds,
    /// Seconds — coarse wall-clock unit, ergonomic for human-facing output.
    Seconds,
}

/// Error returned when [`TimeUnit::from_str`] is given an unrecognised string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown time unit: {0:?} (expected `us`, `ms`, or `s`)")]
pub struct ParseTimeUnitError(pub String);

impl FromStr for TimeUnit {
    type Err = ParseTimeUnitError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "us" | "micros" | "microseconds" => Ok(Self::Microseconds),
            "ms" | "millis" | "milliseconds" => Ok(Self::Milliseconds),
            "s" | "sec" | "secs" | "seconds" => Ok(Self::Seconds),
            other => Err(ParseTimeUnitError(other.to_string())),
        }
    }
}

impl std::fmt::Display for TimeUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Microseconds => "us",
            Self::Milliseconds => "ms",
            Self::Seconds => "s",
        })
    }
}

// ---------------------------------------------------------------------------
// TradingClass
// ---------------------------------------------------------------------------

/// Identifies which exchange-rule family a symbol belongs to.
///
/// Each variant corresponds to a documented market-structure regime:
///
/// | Variant | Examples | Distinctive rule |
/// |---|---|---|
/// | [`OptionsCboeC1`][Self::OptionsCboeC1] | SPX, VIX, XSP, RUT (and weeklies) | 16:15 ET regular close, Curb 16:15-17:00, GTH 20:15 prior to 09:25 |
/// | [`OptionsCboeBzxC2Edgx`][Self::OptionsCboeBzxC2Edgx] | most equity options on Cboe | 16:15 ET regular close |
/// | [`OptionsNyseArca`][Self::OptionsNyseArca] | NYSE Arca options | 16:00 ET regular close |
/// | [`OptionsIse`][Self::OptionsIse] | Nasdaq ISE options | 16:00 ET regular close |
/// | [`OptionsBox`][Self::OptionsBox] | BOX options | 16:00 ET regular close |
/// | [`OptionsAmex`][Self::OptionsAmex] | NYSE American options | 16:00 ET regular close |
/// | [`EquityNasdaq`][Self::EquityNasdaq] | AAPL, QQQ, MSFT (Nasdaq-listed) | 09:30-16:00 ET, pre 04:00-09:30, post 16:00-20:00, GTH 21:00-04:00 ET (NEW 2026 Extended Session) |
/// | [`EquityNyseArca`][Self::EquityNyseArca] | NYSE-listed equities | 09:30-16:00 ET, pre 04:00-09:30, post 16:00-20:00 |
/// | [`EquityCboeBzxEdgx`][Self::EquityCboeBzxEdgx] | Cboe BZX/EDGX equities | 09:30-16:00, early-trading 02:30-04:00, pre 04:00-09:30, post 16:00-20:00 |
/// | [`EquityCboeByxEdga`][Self::EquityCboeByxEdga] | Cboe BYX/EDGA equities | 09:30-16:00, pre 07:00-09:30, post 16:00-20:00 |
///
/// # Forward compatibility
///
/// The enum is `#[non_exhaustive]` and carries an [`Other`][Self::Other]
/// catch-all so downstream callers can ferry a class identifier this kit
/// does not yet know about. Add new venues here in a SemVer-minor release.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TradingClass {
    /// Cboe C1 cash-settled index option (SPX/VIX/XSP/RUT family) — 16:15 close + Curb + GTH.
    OptionsCboeC1,
    /// Cboe BZX/C2/EDGX option family — 16:15 close.
    OptionsCboeBzxC2Edgx,
    /// NYSE Arca options — 16:00 close.
    OptionsNyseArca,
    /// Nasdaq ISE options — 16:00 close.
    OptionsIse,
    /// BOX options — 16:00 close.
    OptionsBox,
    /// NYSE American options — 16:00 close.
    OptionsAmex,
    /// Nasdaq-listed equity (Nasdaq Stock Market) — Extended Session 21:00-04:00 ET.
    EquityNasdaq,
    /// NYSE Arca-listed equity.
    EquityNyseArca,
    /// Cboe BZX/EDGX-listed equity (early-trading window from 02:30 ET).
    EquityCboeBzxEdgx,
    /// Cboe BYX/EDGA-listed equity (pre-market window from 07:00 ET).
    EquityCboeByxEdga,
    /// Forward-compat slot — emit a string when a downstream venue is unknown to this version.
    Other(String),
}

impl TradingClass {
    /// Stable wire-format string for parquet / JSON / CLI output.
    ///
    /// # Example
    ///
    /// ```
    /// use hourskit::TradingClass;
    /// assert_eq!(TradingClass::OptionsCboeC1.as_wire(), "options.cboe.c1");
    /// assert_eq!(TradingClass::EquityNasdaq.as_wire(),  "equity.nasdaq");
    /// ```
    #[must_use]
    pub fn as_wire(&self) -> String {
        match self {
            Self::OptionsCboeC1 => "options.cboe.c1".to_string(),
            Self::OptionsCboeBzxC2Edgx => "options.cboe.bzx_c2_edgx".to_string(),
            Self::OptionsNyseArca => "options.nyse_arca".to_string(),
            Self::OptionsIse => "options.ise".to_string(),
            Self::OptionsBox => "options.box".to_string(),
            Self::OptionsAmex => "options.amex".to_string(),
            Self::EquityNasdaq => "equity.nasdaq".to_string(),
            Self::EquityNyseArca => "equity.nyse_arca".to_string(),
            Self::EquityCboeBzxEdgx => "equity.cboe.bzx_edgx".to_string(),
            Self::EquityCboeByxEdga => "equity.cboe.byx_edga".to_string(),
            Self::Other(s) => format!("other:{s}"),
        }
    }

    /// Parse a [`TradingClass`] from its [`as_wire`][Self::as_wire] form.
    ///
    /// Unknown strings round-trip through [`Self::Other`] so the kit never
    /// loses fidelity on unrecognised values.
    ///
    /// # Example
    ///
    /// ```
    /// use hourskit::TradingClass;
    /// assert_eq!(TradingClass::from_wire("options.cboe.c1"), TradingClass::OptionsCboeC1);
    /// assert_eq!(
    ///     TradingClass::from_wire("other:venue.x"),
    ///     TradingClass::Other("venue.x".into()),
    /// );
    /// ```
    #[must_use]
    pub fn from_wire(s: &str) -> Self {
        match s {
            "options.cboe.c1" => Self::OptionsCboeC1,
            "options.cboe.bzx_c2_edgx" => Self::OptionsCboeBzxC2Edgx,
            "options.nyse_arca" => Self::OptionsNyseArca,
            "options.ise" => Self::OptionsIse,
            "options.box" => Self::OptionsBox,
            "options.amex" => Self::OptionsAmex,
            "equity.nasdaq" => Self::EquityNasdaq,
            "equity.nyse_arca" => Self::EquityNyseArca,
            "equity.cboe.bzx_edgx" => Self::EquityCboeBzxEdgx,
            "equity.cboe.byx_edga" => Self::EquityCboeByxEdga,
            other => Self::Other(other.strip_prefix("other:").unwrap_or(other).to_string()),
        }
    }

    /// Preference rank used to disambiguate when one symbol spans multiple
    /// classes (e.g. SPY listed as both equity and options).
    ///
    /// Lower rank wins. The order is:
    ///
    /// 1. `OptionsCboeC1` — most distinctive (16:15 close + Curb + GTH).
    /// 2. `EquityNasdaq` — broadest equity coverage (regular + pre + post + Extended Session).
    /// 3. `OptionsCboeBzxC2Edgx`, then the remaining options venues by listing date.
    /// 4. The remaining equity venues.
    /// 5. `Other(_)` last.
    ///
    /// This is purely a UI/CLI default; explicit lookups via
    /// [`crate::sources::bundled::session_for_class`] always win.
    #[must_use]
    pub const fn preference_rank(&self) -> u8 {
        match self {
            Self::OptionsCboeC1 => 0,
            Self::EquityNasdaq => 1,
            Self::OptionsCboeBzxC2Edgx => 2,
            Self::OptionsNyseArca => 3,
            Self::OptionsIse => 4,
            Self::OptionsBox => 5,
            Self::OptionsAmex => 6,
            Self::EquityNyseArca => 7,
            Self::EquityCboeBzxEdgx => 8,
            Self::EquityCboeByxEdga => 9,
            Self::Other(_) => u8::MAX,
        }
    }

    /// True for any options class.
    #[must_use]
    pub const fn is_options(&self) -> bool {
        matches!(
            self,
            Self::OptionsCboeC1
                | Self::OptionsCboeBzxC2Edgx
                | Self::OptionsNyseArca
                | Self::OptionsIse
                | Self::OptionsBox
                | Self::OptionsAmex
        )
    }

    /// True for any equity class.
    #[must_use]
    pub const fn is_equity(&self) -> bool {
        matches!(
            self,
            Self::EquityNasdaq
                | Self::EquityNyseArca
                | Self::EquityCboeBzxEdgx
                | Self::EquityCboeByxEdga
        )
    }

    /// Class-level last-trading-day close-of-trading override per CBOE
    /// Rule 5.1(b)(2)(C).
    ///
    /// Returns the override microseconds-of-day when the class as a whole
    /// is governed by the rule's catch-all; `None` when the regular
    /// session close applies on the last trading day too.
    ///
    /// # Why a class-level fallback
    ///
    /// CBOE Rule 5.1(b)(2)(C), verbatim:
    ///
    /// > On their last trading day, Regular Trading Hours for the
    /// > following options are from 9:30 a.m. to 4:00 p.m.:
    /// > — Cboe S&P 500 AM/PM Basis options
    /// > — Index Options with Nonstandard Expirations (i.e., Weeklys
    /// >   and EOMs), Monthly Options Series, Quarterly Options Series,
    /// >   and Quarterly Expirations (i.e., QIXs)
    /// > — SPX options (p.m.-settled)
    /// > — XSP options (p.m.-settled)
    /// > — SPEQF options (p.m.-settled)
    /// > — SPEQX options (p.m.-settled)
    /// > — MRUT options (p.m.-settled)
    ///
    /// The "Index Options with Nonstandard Expirations" bullet is
    /// open-ended — Cboe regularly adds new Weekly / EOM / Monthly /
    /// Quarterly series to the C1 ladder. Encoding only the named
    /// symbols would silently miss every future addition. The class-level
    /// fallback ensures that any [`Self::OptionsCboeC1`] symbol the kit
    /// ships, present or future, gets the 16:00 ET last-trading-day
    /// cutoff without requiring a parquet refresh.
    ///
    /// # Resolution order
    ///
    /// [`SessionInfo::effective_close_us`] consults the per-symbol
    /// override on the row first; this class-level fallback is only
    /// considered when the per-symbol field is `None`.
    ///
    /// # Returns
    ///
    /// - `Some(57_600_000_000)` (16:00 ET in microseconds) for
    ///   [`Self::OptionsCboeC1`].
    /// - `None` for every other class.
    ///
    /// # Example
    ///
    /// ```
    /// use hourskit::TradingClass;
    /// assert_eq!(
    ///     TradingClass::OptionsCboeC1.class_level_last_trading_day_close_us(),
    ///     Some(57_600_000_000),
    /// );
    /// assert_eq!(
    ///     TradingClass::EquityNasdaq.class_level_last_trading_day_close_us(),
    ///     None,
    /// );
    /// ```
    #[must_use]
    pub const fn class_level_last_trading_day_close_us(&self) -> Option<i64> {
        match self {
            Self::OptionsCboeC1 => Some(OPTION_PM_SETTLEMENT_US),
            _ => None,
        }
    }
}

impl std::fmt::Display for TradingClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_wire())
    }
}

// ---------------------------------------------------------------------------
// Microsecond constants
// ---------------------------------------------------------------------------

const US_PER_SEC: i64 = 1_000_000;
const US_PER_MIN: i64 = 60 * US_PER_SEC;
const US_PER_HOUR: i64 = 60 * US_PER_MIN;
const US_PER_DAY: i64 = 24 * US_PER_HOUR;

// ---------------------------------------------------------------------------
// PM-settlement protocol constants
// ---------------------------------------------------------------------------

/// PM-settlement option-trading cutoff for cash-settled index options
/// expiring on the same trading day, in microseconds-of-day (ET).
///
/// 16:00 ET = `57_600_000_000` μs. Per CBOE Rule 5.1(b)(2)(C), every
/// PM-settled cash-settled index option (NDXP, RUTW, MRUT, SPXW, XSP,
/// OEX, XEO, plus the rule-named SPXPM / SPEQF / SPEQX and the
/// literal-symbol SPX/XSP "(PM Expiration)" variants) closes at 16:00
/// ET on its last trading day; non-expiring contracts on the same
/// symbol continue trading until the [`SessionInfo::regular`] close
/// (typically 16:15 ET on Cboe C1).
///
/// Use this constant as a **bare protocol value** for fixtures and
/// for callers that lack a [`SessionInfo`] in hand. Callers with
/// session info should resolve through
/// [`SessionInfo::settlement_cutoff_us`] /
/// [`SessionInfo::effective_close_us`] so per-symbol overrides and
/// AM-settled SPX standard third-Friday rolls are honoured.
///
/// # Example
///
/// ```
/// use hourskit::session::OPTION_PM_SETTLEMENT_US;
/// assert_eq!(OPTION_PM_SETTLEMENT_US, 57_600_000_000);
/// // 16:00:00 ET in microseconds = 16 * 3600 * 1_000_000.
/// assert_eq!(OPTION_PM_SETTLEMENT_US, 16 * 3_600 * 1_000_000);
/// ```
pub const OPTION_PM_SETTLEMENT_US: i64 = 16 * US_PER_HOUR;

/// PM-settlement option-trading cutoff in milliseconds-of-day (ET).
///
/// `OPTION_PM_SETTLEMENT_US / 1_000 = 57_600_000` ms. Provided for
/// downstream consumers (analytics minute-precision `T` arithmetic)
/// that operate on `i32` ms-of-day stamps. The two constants stay
/// in sync via a compile-time guard inside `session::tests`.
///
/// # Example
///
/// ```
/// use hourskit::session::{OPTION_PM_SETTLEMENT_MS, OPTION_PM_SETTLEMENT_US};
/// assert_eq!(OPTION_PM_SETTLEMENT_MS, 57_600_000);
/// assert_eq!(i64::from(OPTION_PM_SETTLEMENT_MS) * 1_000, OPTION_PM_SETTLEMENT_US);
/// ```
pub const OPTION_PM_SETTLEMENT_MS: i32 = 57_600_000;

// ---------------------------------------------------------------------------
// TimeWindow
// ---------------------------------------------------------------------------

/// A half-open `[open, close)` time-of-day window in microseconds-since-midnight (ET).
///
/// Storage is `i64` microseconds throughout — every accessor delegates to a
/// pure division, so callers never round at the storage layer.
///
/// `open_us` and `close_us` are NOT compared at construction: a window can
/// span the overnight boundary (open 21:00, close 04:00 next day). The
/// surrounding [`SessionInfo::gth_overnight`] flag distinguishes the two
/// regimes for windows that may wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TimeWindow {
    /// Window open, in microseconds-of-day (ET).
    pub open_us_of_day: i64,
    /// Window close, in microseconds-of-day (ET).
    pub close_us_of_day: i64,
}

impl TimeWindow {
    /// Construct a window from raw microsecond endpoints.
    ///
    /// # Example
    ///
    /// ```
    /// use hourskit::TimeWindow;
    /// let w = TimeWindow::new(34_200_000_000, 57_600_000_000);
    /// assert_eq!(w.open_us(), 34_200_000_000);
    /// ```
    #[inline]
    #[must_use]
    pub const fn new(open_us_of_day: i64, close_us_of_day: i64) -> Self {
        Self {
            open_us_of_day,
            close_us_of_day,
        }
    }

    /// Construct a window from `(open_h, open_m, close_h, close_m)` ET wall-clock.
    ///
    /// Convenience over arithmetic; useful in seed-data and tests.
    ///
    /// # Example
    ///
    /// ```
    /// use hourskit::TimeWindow;
    /// // 09:30 ET to 16:00 ET regular session
    /// let w = TimeWindow::from_clock_et(9, 30, 16, 0);
    /// assert_eq!(w.open_secs(),  9 * 3_600 + 30 * 60);
    /// assert_eq!(w.close_secs(), 16 * 3_600);
    /// ```
    #[inline]
    #[must_use]
    pub const fn from_clock_et(open_h: i64, open_m: i64, close_h: i64, close_m: i64) -> Self {
        let open = open_h * US_PER_HOUR + open_m * US_PER_MIN;
        let close = close_h * US_PER_HOUR + close_m * US_PER_MIN;
        Self::new(open, close)
    }

    /// Open time, in microseconds-of-day (ET).
    #[inline]
    #[must_use]
    pub const fn open_us(self) -> i64 {
        self.open_us_of_day
    }

    /// Open time, in milliseconds-of-day (ET) — exact integer division.
    #[inline]
    #[must_use]
    pub const fn open_ms(self) -> i64 {
        self.open_us_of_day / 1_000
    }

    /// Open time, in seconds-of-day (ET) — exact integer division.
    #[inline]
    #[must_use]
    pub const fn open_secs(self) -> i64 {
        self.open_us_of_day / US_PER_SEC
    }

    /// Close time, in microseconds-of-day (ET).
    #[inline]
    #[must_use]
    pub const fn close_us(self) -> i64 {
        self.close_us_of_day
    }

    /// Close time, in milliseconds-of-day (ET).
    #[inline]
    #[must_use]
    pub const fn close_ms(self) -> i64 {
        self.close_us_of_day / 1_000
    }

    /// Close time, in seconds-of-day (ET).
    #[inline]
    #[must_use]
    pub const fn close_secs(self) -> i64 {
        self.close_us_of_day / US_PER_SEC
    }

    /// Window duration in microseconds.
    ///
    /// - Same-day window (`close_us >= open_us`): straightforward
    ///   `close - open`.
    /// - Overnight window (`close_us < open_us`, e.g. NASDAQ 21:00 ET to
    ///   04:00 ET next morning, or Cboe options 20:15 prior to 09:25
    ///   current): computed as `(US_PER_DAY - open) + close` so the
    ///   duration reflects the actual minutes elapsed across midnight.
    #[inline]
    #[must_use]
    pub const fn duration_us(self) -> i64 {
        if self.close_us_of_day >= self.open_us_of_day {
            self.close_us_of_day - self.open_us_of_day
        } else {
            (US_PER_DAY - self.open_us_of_day) + self.close_us_of_day
        }
    }

    /// Whether this window wraps midnight (close numerically less than
    /// open). Use this to choose between [`Self::contains_us`] and
    /// [`Self::contains_us_overnight`] when the caller doesn't already
    /// have the [`SessionInfo::gth_overnight`] flag in hand.
    #[inline]
    #[must_use]
    pub const fn is_overnight(self) -> bool {
        self.close_us_of_day < self.open_us_of_day
    }

    /// Window duration in milliseconds.
    #[inline]
    #[must_use]
    pub const fn duration_ms(self) -> i64 {
        self.duration_us() / 1_000
    }

    /// Window duration in seconds.
    #[inline]
    #[must_use]
    pub const fn duration_secs(self) -> i64 {
        self.duration_us() / US_PER_SEC
    }

    /// Window duration expressed in `unit`.
    #[inline]
    #[must_use]
    pub const fn duration_in(self, unit: TimeUnit) -> i64 {
        match unit {
            TimeUnit::Microseconds => self.duration_us(),
            TimeUnit::Milliseconds => self.duration_ms(),
            TimeUnit::Seconds => self.duration_secs(),
        }
    }

    /// `[open, close)` half-open membership test, microsecond-precise.
    ///
    /// For overnight (wrap-around) windows the caller should consult
    /// [`Self::contains_us_overnight`] or compose the test against the
    /// overnight flag on [`SessionInfo`].
    #[inline]
    #[must_use]
    pub const fn contains_us(self, us_of_day: i64) -> bool {
        us_of_day >= self.open_us_of_day && us_of_day < self.close_us_of_day
    }

    /// `[open, close)` membership against a millisecond-of-day timestamp.
    #[inline]
    #[must_use]
    pub const fn contains_ms(self, ms_of_day: i64) -> bool {
        self.contains_us(ms_of_day.saturating_mul(1_000))
    }

    /// `[open, close)` membership for an overnight (wrap-around) window:
    /// matches when `us_of_day >= open` (yesterday) OR `us_of_day < close`
    /// (today). Always pair this with the [`SessionInfo::gth_overnight`]
    /// flag — a same-day window must NOT use this routine.
    #[inline]
    #[must_use]
    pub const fn contains_us_overnight(self, us_of_day: i64) -> bool {
        us_of_day >= self.open_us_of_day || us_of_day < self.close_us_of_day
    }

    /// Open time expressed in `unit`.
    #[inline]
    #[must_use]
    pub const fn open_in(self, unit: TimeUnit) -> i64 {
        match unit {
            TimeUnit::Microseconds => self.open_us(),
            TimeUnit::Milliseconds => self.open_ms(),
            TimeUnit::Seconds => self.open_secs(),
        }
    }

    /// Close time expressed in `unit`.
    #[inline]
    #[must_use]
    pub const fn close_in(self, unit: TimeUnit) -> i64 {
        match unit {
            TimeUnit::Microseconds => self.close_us(),
            TimeUnit::Milliseconds => self.close_ms(),
            TimeUnit::Seconds => self.close_secs(),
        }
    }
}

impl std::fmt::Display for TimeWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let oh = self.open_us_of_day / US_PER_HOUR;
        let om = (self.open_us_of_day / US_PER_MIN) % 60;
        let ch = self.close_us_of_day / US_PER_HOUR;
        let cm = (self.close_us_of_day / US_PER_MIN) % 60;
        write!(f, "{oh:02}:{om:02}-{ch:02}:{cm:02} ET")
    }
}

// ---------------------------------------------------------------------------
// SessionInfo
// ---------------------------------------------------------------------------

/// Settlement convention for cash-settled index option expirations.
///
/// Determines where variance-contribution accounting must terminate on
/// the expiration day. For PM-settled contracts the cutoff is the
/// option-trading session close ([`SessionInfo::effective_close_us`]
/// — typically 16:00 ET on equity options or 16:15 ET on Cboe C1
/// index options, modulated by the per-symbol last-trading-day rule on
/// CBOE Rule 5.1(b)(2)(C) names). For AM-settled contracts the cutoff
/// is the cash-equity open print time, typically 09:30 ET, embedded
/// in the [`AmOpen`][Self::AmOpen] variant.
///
/// The convention is per-(symbol, expiration). On `SessionInfo` the
/// stored value describes the contract family rule:
/// [`Pm`][Self::Pm] always returns the PM cutoff, while
/// [`AmOpen`][Self::AmOpen] returns the embedded AM open time on
/// expirations that match the SPX-standard third-Friday rule (third
/// Friday of the month) and falls back to the PM cutoff on every
/// other expiration. The classifier lives in
/// [`SessionInfo::settlement_cutoff_us`].
///
/// `#[non_exhaustive]` because future cash-settled product families
/// may carry their own conventions (e.g. weekly AM expirations with
/// a different open-print time).
///
/// # Example
///
/// ```
/// use hourskit::Settlement;
/// // 09:30 ET expressed in microseconds-of-day.
/// const NINE_THIRTY_US: i64 = 9 * 3_600 * 1_000_000 + 30 * 60 * 1_000_000;
/// let am = Settlement::AmOpen { open_us_of_day: NINE_THIRTY_US };
/// let pm = Settlement::Pm;
/// assert_ne!(am, pm);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Settlement {
    /// PM settlement — variance contribution terminates at the
    /// option-trading session close. Default for every supported
    /// product, including SPXW weeklies, SPX EOM expirations, NDX
    /// (PM), and every equity option.
    Pm,
    /// AM settlement — variance contribution terminates at
    /// `open_us_of_day` microseconds-of-day (ET) on expirations that
    /// match the SPX-standard third-Friday rule. Currently
    /// populated only for the `SPX` symbol family at 09:30 ET; other
    /// expirations on the same row fall back to PM via
    /// [`SessionInfo::settlement_cutoff_us`].
    AmOpen {
        /// AM SET print time, microseconds-of-day (ET). For SPX
        /// standard third-Friday options this is 09:30 ET
        /// (`34_200_000_000`).
        open_us_of_day: i64,
    },
}

/// `true` when `yyyymmdd` is the third Friday of its month — the
/// canonical SPX standard expiration date that triggers the
/// AM SET settlement convention.
///
/// Implemented via Sakamoto's day-of-week algorithm; works for any
/// civil date in the first two millennia.
///
/// # Example
///
/// ```
/// use hourskit::is_third_friday;
/// // 3rd Friday of May 2026 is the 15th.
/// assert!(is_third_friday(20_260_515));
/// // 2nd Friday of May 2026 — the 8th.
/// assert!(!is_third_friday(20_260_508));
/// // 4th Friday of May 2026 — the 22nd.
/// assert!(!is_third_friday(20_260_522));
/// ```
#[must_use]
pub const fn is_third_friday(yyyymmdd: i32) -> bool {
    let d = yyyymmdd % 100;
    if d < 15 || d > 21 {
        return false;
    }
    day_of_week_yyyymmdd(yyyymmdd) == 5
}

/// Day-of-week via Zeller's congruence: 0 = Sunday, 1 = Monday, ...,
/// 6 = Saturday.
///
/// `const fn` so [`is_third_friday`] stays evaluable in const
/// contexts. Returns 7 (a sentinel outside the [0, 6] DOW range) for
/// any `yyyymmdd` whose month component is outside `1..=12`; callers
/// must validate `yyyymmdd` shape if they need a guaranteed real DOW.
///
/// Zeller's congruence (Gregorian variant) avoids the indexed lookup
/// table Sakamoto's algorithm uses, sidestepping `as usize` index
/// arithmetic in a const-context day-of-week computation.
#[must_use]
const fn day_of_week_yyyymmdd(yyyymmdd: i32) -> i32 {
    let year_in = yyyymmdd / 10_000;
    let month_in = (yyyymmdd / 100) % 100;
    let day = yyyymmdd % 100;
    if month_in < 1 || month_in > 12 {
        return 7;
    }
    // Zeller treats Jan/Feb as months 13/14 of the previous year.
    let (month, year) = if month_in < 3 {
        (month_in + 12, year_in - 1)
    } else {
        (month_in, year_in)
    };
    let year_of_century = year.rem_euclid(100);
    let century = year.div_euclid(100);
    // Zeller's congruence yields 0=Saturday, 1=Sunday, ..., 6=Friday.
    let zeller_day = (day
        + (13 * (month + 1)) / 5
        + year_of_century
        + year_of_century / 4
        + century / 4
        + 5 * century)
        .rem_euclid(7);
    // Rotate to 0=Sunday, 1=Monday, ..., 6=Saturday.
    (zeller_day + 6).rem_euclid(7)
}

/// Per-symbol slate of trading-session windows.
///
/// `SessionInfo` is the single value type returned by [`crate::Hourskit::session`]
/// (and free-function variants). It documents which [`TradingClass`] the
/// data was derived from and exposes every applicable window (regular
/// session + four optional auxiliaries).
///
/// All time values are microseconds-of-day in US/Eastern wall clock — the
/// kit performs no timezone conversion. Combine with a market calendar to
/// project the windows onto a specific date.
///
/// # Field semantics
///
/// | Field | Semantics | Typical class |
/// |---|---|---|
/// | `regular`     | core trading session                                       | every class |
/// | `pre_market`  | pre-open extended session (e.g. 04:00-09:30 ET)            | every equity class |
/// | `post_market` | post-close extended session (e.g. 16:00-20:00 ET)          | every equity class |
/// | `curb`        | post-regular Cboe C1 Curb session (e.g. 16:15-17:00 ET)    | OptionsCboeC1 |
/// | `gth`         | overnight session (Cboe options 20:15-09:25, Nasdaq 21:00-04:00) | OptionsCboeC1, EquityNasdaq |
/// | `gth_overnight` | true if `gth.open_us` is on the PRIOR trading day; false if same-day evening start | implied by class |
/// | `last_trading_day_close_us` | per-contract early close on contract expiry day (CBOE Rule 5.1(b)(2)(C)) | NDXP, RUTW, MRUT, SPXW, XSP, OEX, XEO |
/// | `settlement` | settlement convention rule for the contract family ([`Settlement::Pm`] default; [`Settlement::AmOpen`] for SPX standard third-Friday) | SPX |
/// | `valid_from_yyyymmdd` | trading date this row takes effect (`None` = always-valid baseline) | any staged future rule change |
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Symbol (uppercase) e.g. `"SPX"`, `"AAPL"`.
    pub symbol: String,
    /// Trading class — identifies which exchange-rule family produced the windows below.
    pub trading_class: TradingClass,
    /// Regular trading session window (always populated).
    pub regular: TimeWindow,
    /// Optional pre-market window.
    pub pre_market: Option<TimeWindow>,
    /// Optional post-market window.
    pub post_market: Option<TimeWindow>,
    /// Optional Cboe C1 Curb session (16:15-17:00 ET on full days).
    pub curb: Option<TimeWindow>,
    /// Optional overnight Global Trading Hours window.
    pub gth: Option<TimeWindow>,
    /// `true` when [`gth.open_us_of_day`][TimeWindow::open_us_of_day] is on the
    /// prior trading day (e.g. Cboe C1 GTH at 20:15 ET prior → 09:25 ET
    /// current); `false` when it is the same calendar evening (e.g. Nasdaq
    /// EquityNasdaq Extended Session at 21:00 ET → 04:00 ET next morning).
    pub gth_overnight: bool,
    /// Per-symbol cutoff override for cash-settled index options on their
    /// last trading day (CBOE Rule 5.1(b)(2)(C)).
    ///
    /// When `Some(close_us)`, contracts whose `exp_date == event_date`
    /// close at this value (typically `57_600_000_000` = 16:00 ET);
    /// non-expiring contracts on the same symbol continue trading until
    /// [`regular.close_us`][TimeWindow::close_us] (typically 16:15 ET).
    ///
    /// Populated for the explicitly-named symbols in CBOE Rule 5.1(b)(2)(C)
    /// — `NDXP`, `RUTW`, `MRUT`, `SPXW`, `XSP`, `OEX`, `XEO`, plus the
    /// p.m.-settled `SPXPM` / `SPEQF` / `SPEQX` and the
    /// `SPX (PM Expiration)` / `XSP (AM Expiration)` literal-symbol
    /// variants. `None` for every other symbol.
    ///
    /// `None` does NOT mean "no last-trading-day cutoff" — the rule's
    /// open-ended "Index Options with Nonstandard Expirations" bullet
    /// is encoded at the class level via
    /// [`TradingClass::class_level_last_trading_day_close_us`]. Use
    /// [`SessionInfo::effective_close_us`] to resolve the effective
    /// close for a given `(event_date, exp_date)` pair without branching
    /// on this field directly.
    pub last_trading_day_close_us: Option<i64>,
    /// Settlement convention for the contract family.
    ///
    /// Determines where variance contribution terminates on the
    /// expiration day. Default [`Settlement::Pm`] applies to every
    /// product, with the exception of the `SPX` symbol, whose standard
    /// third-Friday-of-the-month expirations follow the AM SET print
    /// at 09:30 ET. Resolve a per-expiration cutoff via
    /// [`Self::settlement_cutoff_us`] rather than branching on this
    /// field directly — the resolver carries the third-Friday
    /// classifier and falls back to PM for SPXW / EOM / weekly
    /// expirations on the SPX row.
    pub settlement: Settlement,
    /// Trading date (`YYYYMMDD`) on or after which this row takes effect.
    ///
    /// `None` is the **baseline** row: always valid, the rule that
    /// applies absent any dated override. `Some(yyyymmdd)` stages a
    /// future rule change — the row only governs query dates `>= yyyymmdd`.
    ///
    /// Effective-dating lets an announced trading-hours change be carried
    /// in the dataset ahead of its start date without disturbing current
    /// resolution. For a given `(symbol, trading_class)` the dataset may
    /// hold multiple rows differing only by this field; the applicable
    /// row for a query date `D` is the one with the greatest
    /// `valid_from_yyyymmdd <= D`, treating `None` as negative infinity
    /// (always eligible). See
    /// [`crate::sources::bundled::session_on`] for the date-aware
    /// resolver. The undated lookups
    /// ([`crate::sources::bundled::session`] and friends) resolve the
    /// LATEST effective row — i.e. the newest active session — so an
    /// existing caller always sees the most recent rule in force.
    ///
    /// Every legacy single-row symbol carries `None` here, so behaviour
    /// is unchanged for any symbol that has no staged future row.
    pub valid_from_yyyymmdd: Option<i32>,
}

impl SessionInfo {
    /// Microsecond-of-day at which the regular session closes.
    ///
    /// For OPTIONS classes that is the option late-trading cutoff; for equity
    /// classes that is the regular trading close (16:00 ET on every supported
    /// venue).
    #[inline]
    #[must_use]
    pub const fn option_close_us(&self) -> i64 {
        self.regular.close_us()
    }

    /// Microsecond-of-day at which the LATEST window for this class closes.
    ///
    /// Computed as the maximum of `regular.close`, `curb.close` (if present),
    /// `post_market.close` (if present) and `gth.close` (if same-day; an
    /// overnight GTH closes the next morning so it is excluded — callers that
    /// want the next-morning close should use the gth field directly).
    #[inline]
    #[must_use]
    pub fn last_trading_us(&self) -> i64 {
        let mut latest = self.regular.close_us();
        if let Some(curb) = self.curb {
            latest = latest.max(curb.close_us());
        }
        if let Some(post) = self.post_market {
            latest = latest.max(post.close_us());
        }
        if let Some(gth) = self.gth {
            if !self.gth_overnight {
                latest = latest.max(gth.close_us());
            }
        }
        latest
    }

    /// Whether the class is options.
    #[inline]
    #[must_use]
    pub const fn is_options(&self) -> bool {
        self.trading_class.is_options()
    }

    /// Whether the class is equity.
    #[inline]
    #[must_use]
    pub const fn is_equity(&self) -> bool {
        self.trading_class.is_equity()
    }

    /// Resolve the effective close-of-trading microsecond-of-day for a
    /// specific contract on a specific event date.
    ///
    /// `event_date` and `exp_date` are integer-encoded YYYYMMDD; the
    /// helper compares them for equality only, so any consistent encoding
    /// (e.g. 20260516 for 2026-05-16) works.
    ///
    /// # Resolution order
    ///
    /// On the contract's last trading day (`event_date == exp_date`):
    ///
    /// 1. **Per-symbol override** — if [`Self::last_trading_day_close_us`]
    ///    is `Some`, that value is returned. This is the highest-priority
    ///    branch and lets seed data pin specific symbols (e.g. SPXW,
    ///    NDXP) at 16:00 ET regardless of the trading class.
    /// 2. **Class-level fallback** — if the per-symbol override is `None`
    ///    but [`TradingClass::class_level_last_trading_day_close_us`]
    ///    returns `Some`, that value is returned. This carries the
    ///    open-ended "Index Options with Nonstandard Expirations"
    ///    bullet of CBOE Rule 5.1(b)(2)(C) without requiring per-symbol
    ///    enumeration.
    /// 3. **Regular close** — when both branches yield `None` (or the
    ///    contract is not expiring today), [`TimeWindow::close_us`] of
    ///    [`Self::regular`] is returned (typically 16:00 / 16:15 ET).
    ///
    /// CBOE Rule 5.1(b)(2)(C) and the Firstrade extended-trading-hours
    /// notice: "On the last trading day, expiring NDXP, RUTW, MRUT, SPXW,
    /// XSP, OEX & XEO options will trade until 4:00 pm ET, and
    /// non-expiring options will continue to trade until 4:15 pm ET."
    /// The full rule additionally covers Cboe S&P 500 AM/PM Basis
    /// options, Weeklys, EOMs, Monthly / Quarterly series, and the
    /// p.m.-settled SPX / XSP / SPEQF / SPEQX / MRUT symbols — see
    /// [`TradingClass::class_level_last_trading_day_close_us`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use hourskit::session_blocking;
    /// let s = session_blocking("SPXW")?.expect("SPXW in seed data");
    /// // Same-day expiring SPXW contract on 2026-05-16:
    /// assert_eq!(s.effective_close_us(20260516, 20260516), 57_600_000_000); // 16:00 ET
    /// // Next-day expiring SPXW contract on 2026-05-16:
    /// assert!(s.effective_close_us(20260516, 20260517) > 57_600_000_000);   // 16:15 ET
    /// # Ok::<(), hourskit::Error>(())
    /// ```
    #[inline]
    #[must_use]
    pub const fn effective_close_us(&self, event_date: i32, exp_date: i32) -> i64 {
        if event_date == exp_date {
            if let Some(early) = self.last_trading_day_close_us {
                return early;
            }
            if let Some(early) = self.trading_class.class_level_last_trading_day_close_us() {
                return early;
            }
        }
        self.regular.close_us()
    }

    /// Microsecond-of-day at which variance-contribution accounting
    /// must terminate for an option contract that expires on
    /// `expiration_date` (`YYYYMMDD`).
    ///
    /// # Resolution order
    ///
    /// 1. **AM-settled, third-Friday match** — when
    ///    [`Self::settlement`] is [`Settlement::AmOpen`] and
    ///    `expiration_date` is the third Friday of its month per
    ///    [`is_third_friday`], return the embedded `open_us_of_day`.
    ///    This is the SPX-standard AM SET print branch (09:30 ET on
    ///    SPX standard third-Friday expirations).
    /// 2. **PM fallback** — every other case routes through
    ///    [`Self::effective_close_us`] with
    ///    `event_date == exp_date == expiration_date`, returning the
    ///    per-symbol last-trading-day override or the class-level
    ///    fallback or the regular close, in that priority order.
    ///
    /// This is the function downstream volatility analytics should
    /// use to derive the time-to-expiry cutoff: it stays correct for
    /// SPXW (PM), SPX EOM (PM), every NDX / RUT / VIX expiration
    /// (PM), and SPX standard third-Friday (AM SET) without the
    /// caller branching on symbol.
    ///
    /// # Examples
    ///
    /// ```
    /// use hourskit::{Settlement, SessionInfo, TimeWindow, TradingClass};
    /// const NINE_THIRTY_US: i64 = 9 * 3_600 * 1_000_000 + 30 * 60 * 1_000_000;
    /// let mut spx = SessionInfo {
    ///     symbol: "SPX".into(),
    ///     trading_class: TradingClass::OptionsCboeC1,
    ///     regular: TimeWindow::from_clock_et(9, 30, 16, 15),
    ///     pre_market: None,
    ///     post_market: None,
    ///     curb: None,
    ///     gth: None,
    ///     gth_overnight: false,
    ///     last_trading_day_close_us: None,
    ///     settlement: Settlement::AmOpen { open_us_of_day: NINE_THIRTY_US },
    ///     valid_from_yyyymmdd: None,
    /// };
    /// // SPX 3rd-Friday May 2026 (15th) → AM SET 09:30 ET.
    /// assert_eq!(spx.settlement_cutoff_us(20_260_515), NINE_THIRTY_US);
    /// // SPX EOM May 2026 (29th) → PM cutoff (class-level 16:00 ET on last
    /// // trading day, falling out of the OptionsCboeC1 class-level rule).
    /// assert_eq!(spx.settlement_cutoff_us(20_260_529), 57_600_000_000);
    /// // Flip to PM convention — every expiration uses the PM cutoff.
    /// spx.settlement = Settlement::Pm;
    /// assert_eq!(spx.settlement_cutoff_us(20_260_515), 57_600_000_000);
    /// ```
    #[inline]
    #[must_use]
    pub const fn settlement_cutoff_us(&self, expiration_date: i32) -> i64 {
        match self.settlement {
            Settlement::AmOpen { open_us_of_day } if is_third_friday(expiration_date) => {
                open_us_of_day
            }
            _ => self.effective_close_us(expiration_date, expiration_date),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn h_us(h: i64) -> i64 {
        h * 3_600 * 1_000_000
    }

    fn hm_us(h: i64, m: i64) -> i64 {
        h_us(h) + m * 60 * 1_000_000
    }

    #[test]
    fn time_unit_round_trips_through_strings() {
        for (s, expected) in [
            ("us", TimeUnit::Microseconds),
            ("ms", TimeUnit::Milliseconds),
            ("s", TimeUnit::Seconds),
            ("microseconds", TimeUnit::Microseconds),
            ("MILLIS", TimeUnit::Milliseconds),
            ("seconds", TimeUnit::Seconds),
        ] {
            assert_eq!(TimeUnit::from_str(s).expect("parse"), expected, "{s}");
        }
    }

    #[test]
    fn time_unit_rejects_unknown() {
        assert!(TimeUnit::from_str("nanoseconds").is_err());
        assert!(TimeUnit::from_str("").is_err());
    }

    #[test]
    fn trading_class_round_trips_through_wire_string() {
        for class in [
            TradingClass::OptionsCboeC1,
            TradingClass::OptionsCboeBzxC2Edgx,
            TradingClass::OptionsNyseArca,
            TradingClass::OptionsIse,
            TradingClass::OptionsBox,
            TradingClass::OptionsAmex,
            TradingClass::EquityNasdaq,
            TradingClass::EquityNyseArca,
            TradingClass::EquityCboeBzxEdgx,
            TradingClass::EquityCboeByxEdga,
        ] {
            let s = class.as_wire();
            let back = TradingClass::from_wire(&s);
            assert_eq!(class, back, "wire round-trip for {s}");
        }
    }

    #[test]
    fn trading_class_other_round_trips() {
        let class = TradingClass::Other("venue.foo".into());
        let s = class.as_wire();
        assert_eq!(s, "other:venue.foo");
        assert_eq!(TradingClass::from_wire(&s), class);
    }

    #[test]
    fn time_window_clock_constructor_matches_explicit_microseconds() {
        let w = TimeWindow::from_clock_et(9, 30, 16, 0);
        assert_eq!(w.open_us(), hm_us(9, 30));
        assert_eq!(w.close_us(), hm_us(16, 0));
    }

    #[test]
    fn time_window_unit_accessors_are_consistent() {
        let w = TimeWindow::from_clock_et(9, 30, 16, 0);
        assert_eq!(w.open_ms(), w.open_us() / 1_000);
        assert_eq!(w.open_secs(), w.open_us() / 1_000_000);
        assert_eq!(w.close_ms(), w.close_us() / 1_000);
        assert_eq!(w.close_secs(), w.close_us() / 1_000_000);
    }

    #[test]
    fn time_window_duration_for_regular_session_is_six_thirty() {
        let w = TimeWindow::from_clock_et(9, 30, 16, 0);
        assert_eq!(w.duration_secs(), 6 * 3_600 + 30 * 60);
        assert_eq!(w.duration_ms(), w.duration_secs() * 1_000);
        assert_eq!(w.duration_us(), w.duration_secs() * 1_000_000);
    }

    #[test]
    fn time_window_contains_us_is_half_open() {
        let w = TimeWindow::from_clock_et(9, 30, 16, 0);
        assert!(w.contains_us(w.open_us()));
        assert!(!w.contains_us(w.close_us()));
        assert!(w.contains_us(w.close_us() - 1));
    }

    #[test]
    fn time_window_overnight_membership() {
        // Nasdaq Extended Session: 21:00 ET → 04:00 ET next day.
        let w = TimeWindow::from_clock_et(21, 0, 4, 0);
        // 22:00 ET is in the window (yesterday side).
        assert!(w.contains_us_overnight(hm_us(22, 0)));
        // 03:00 ET is in the window (today side).
        assert!(w.contains_us_overnight(hm_us(3, 0)));
        // 12:00 ET is OUTSIDE the window.
        assert!(!w.contains_us_overnight(hm_us(12, 0)));
    }

    #[test]
    fn nasdaq_extended_session_duration_is_seven_hours() {
        // 21:00 ET → 04:00 ET = 7 hours elapsed across midnight.
        let w = TimeWindow::from_clock_et(21, 0, 4, 0);
        assert!(w.is_overnight());
        assert_eq!(w.duration_secs(), 7 * 3_600);
        assert_eq!(w.duration_ms(), 7 * 3_600 * 1_000);
        assert_eq!(w.duration_us(), 7 * 3_600 * 1_000_000);
    }

    #[test]
    fn cboe_options_overnight_duration_is_thirteen_hours_ten_minutes() {
        // OPRA GTH: 20:15 ET prior → 09:25 ET current = 13h10m.
        let w = TimeWindow::new(hm_us(20, 15), hm_us(9, 25));
        assert!(w.is_overnight());
        assert_eq!(w.duration_secs(), 13 * 3_600 + 10 * 60);
    }

    #[test]
    fn overnight_membership_at_midnight() {
        let w = TimeWindow::from_clock_et(21, 0, 4, 0);
        // Just before midnight (23:59:59) is in the window.
        assert!(w.contains_us_overnight(hm_us(23, 59) + 59 * US_PER_SEC));
        // Midnight itself (00:00:00) is in the window.
        assert!(w.contains_us_overnight(0));
        // Just before close (03:59:59) is in the window.
        assert!(w.contains_us_overnight(hm_us(3, 59) + 59 * US_PER_SEC));
        // The close itself is NOT in the half-open window.
        assert!(!w.contains_us_overnight(hm_us(4, 0)));
    }

    #[test]
    fn same_day_window_is_not_overnight() {
        let w = TimeWindow::from_clock_et(9, 30, 16, 15);
        assert!(!w.is_overnight());
        assert_eq!(w.duration_secs(), 6 * 3_600 + 45 * 60);
    }

    #[test]
    fn session_info_last_trading_picks_curb_when_present() {
        let info = SessionInfo {
            symbol: "SPX".into(),
            trading_class: TradingClass::OptionsCboeC1,
            regular: TimeWindow::from_clock_et(9, 30, 16, 15),
            pre_market: None,
            post_market: None,
            curb: Some(TimeWindow::from_clock_et(16, 15, 17, 0)),
            gth: Some(TimeWindow::new(hm_us(20, 15), hm_us(9, 25))),
            gth_overnight: true,
            last_trading_day_close_us: None,
            settlement: Settlement::Pm,
            valid_from_yyyymmdd: None,
        };
        // last_trading_us() ignores overnight GTH (open is on prior day).
        assert_eq!(info.last_trading_us(), hm_us(17, 0));
    }

    #[test]
    fn session_info_last_trading_picks_post_market_for_equity() {
        let info = SessionInfo {
            symbol: "AAPL".into(),
            trading_class: TradingClass::EquityNasdaq,
            regular: TimeWindow::from_clock_et(9, 30, 16, 0),
            pre_market: Some(TimeWindow::from_clock_et(4, 0, 9, 30)),
            post_market: Some(TimeWindow::from_clock_et(16, 0, 20, 0)),
            curb: None,
            gth: Some(TimeWindow::from_clock_et(21, 0, 4, 0)),
            gth_overnight: true,
            last_trading_day_close_us: None,
            settlement: Settlement::Pm,
            valid_from_yyyymmdd: None,
        };
        assert_eq!(info.last_trading_us(), hm_us(20, 0));
    }

    #[test]
    fn time_window_display_format_is_clock_style() {
        assert_eq!(
            TimeWindow::from_clock_et(9, 30, 16, 0).to_string(),
            "09:30-16:00 ET"
        );
    }

    // ── Settlement convention ──────────────────────────────────────

    /// 09:30 ET expressed in microseconds-of-day.
    const AM_SET_US: i64 = 9 * 3_600 * 1_000_000 + 30 * 60 * 1_000_000;
    /// 16:00 ET — last-trading-day close for cash-settled C1 options.
    /// Pinned to [`OPTION_PM_SETTLEMENT_US`] so the test fixture and
    /// the public protocol constant stay in sync.
    const PM_LAST_TRADING_DAY_US: i64 = OPTION_PM_SETTLEMENT_US;
    /// 16:15 ET — Cboe C1 regular-session close.
    const C1_REGULAR_CLOSE_US: i64 = 16 * 3_600 * 1_000_000 + 15 * 60 * 1_000_000;

    fn spx_session_with(settlement: Settlement) -> SessionInfo {
        SessionInfo {
            symbol: "SPX".into(),
            trading_class: TradingClass::OptionsCboeC1,
            regular: TimeWindow::from_clock_et(9, 30, 16, 15),
            pre_market: None,
            post_market: None,
            curb: None,
            gth: None,
            gth_overnight: false,
            last_trading_day_close_us: None,
            settlement,
            valid_from_yyyymmdd: None,
        }
    }

    #[test]
    fn third_friday_recognises_canonical_spx_expirations() {
        // 3rd Friday of May 2026 is the 15th.
        assert!(is_third_friday(20_260_515));
        // 2nd Friday of May 2026 — the 8th.
        assert!(!is_third_friday(20_260_508));
        // 4th Friday of May 2026 — the 22nd.
        assert!(!is_third_friday(20_260_522));
        // EOM Friday of May 2026 — the 29th.
        assert!(!is_third_friday(20_260_529));
        // 3rd Friday of June 2026 = 19th (also March-cycle).
        assert!(is_third_friday(20_260_619));
        // Saturday March 21, 2026 — not Friday, even though the date
        // is in the 15-21 window.
        assert!(!is_third_friday(20_260_321));
    }

    #[test]
    fn settlement_default_is_pm_for_every_class() {
        // PM-settled SessionInfo on every supported class returns the
        // PM cutoff (regular close or class-level last-trading-day
        // override) regardless of expiration.
        let pm = spx_session_with(Settlement::Pm);
        // Standard third-Friday SPX → still PM cutoff per the row.
        assert_eq!(pm.settlement_cutoff_us(20_260_515), PM_LAST_TRADING_DAY_US);
        // Non-third-Friday SPX → same PM cutoff.
        assert_eq!(pm.settlement_cutoff_us(20_260_522), PM_LAST_TRADING_DAY_US);
    }

    #[test]
    fn settlement_am_open_fires_only_on_third_friday() {
        let spx = spx_session_with(Settlement::AmOpen {
            open_us_of_day: AM_SET_US,
        });
        // 3rd-Friday May 2026 (15th) → AM SET 09:30 ET.
        assert_eq!(spx.settlement_cutoff_us(20_260_515), AM_SET_US);
        // 4th Friday May 2026 (22nd) → PM cutoff (class-level last-
        // trading-day override on OptionsCboeC1).
        assert_eq!(spx.settlement_cutoff_us(20_260_522), PM_LAST_TRADING_DAY_US);
        // EOM Friday May 2026 (29th) → PM cutoff.
        assert_eq!(spx.settlement_cutoff_us(20_260_529), PM_LAST_TRADING_DAY_US);
        // Mid-week Wednesday May 2026 (20th) → PM cutoff.
        assert_eq!(spx.settlement_cutoff_us(20_260_520), PM_LAST_TRADING_DAY_US);
    }

    #[test]
    fn settlement_am_open_falls_back_to_per_symbol_override_when_present() {
        // When the row carries an explicit per-symbol last-trading-day
        // override, the PM branch returns that value (NOT the class
        // fallback or the regular close).
        let mut info = spx_session_with(Settlement::AmOpen {
            open_us_of_day: AM_SET_US,
        });
        // Pin a per-symbol override at 16:30 ET (60_300_000_000 μs) to
        // verify the priority ordering.
        let custom_pm_close = 16_i64 * 3_600 * 1_000_000 + 30 * 60 * 1_000_000;
        info.last_trading_day_close_us = Some(custom_pm_close);
        // Non-third-Friday → per-symbol override.
        assert_eq!(info.settlement_cutoff_us(20_260_522), custom_pm_close);
        // Third-Friday → AM SET trumps per-symbol override.
        assert_eq!(info.settlement_cutoff_us(20_260_515), AM_SET_US);
    }

    #[test]
    fn settlement_falls_back_to_regular_close_when_no_overrides() {
        // Strip the class-level override by using a non-C1 class so
        // the only PM source is `regular.close_us`.
        let info = SessionInfo {
            symbol: "SPY".into(),
            trading_class: TradingClass::EquityNyseArca,
            regular: TimeWindow::from_clock_et(9, 30, 16, 0),
            pre_market: None,
            post_market: None,
            curb: None,
            gth: None,
            gth_overnight: false,
            last_trading_day_close_us: None,
            settlement: Settlement::Pm,
            valid_from_yyyymmdd: None,
        };
        // 16:00 ET in microseconds-of-day.
        assert_eq!(info.settlement_cutoff_us(20_260_515), 57_600_000_000);
    }

    #[test]
    fn settlement_pm_uses_regular_close_for_non_expiry_lookups() {
        // The doc-tested behaviour: SessionInfo with PM settlement
        // and no per-symbol or class override returns the regular close
        // for the queried expiration day.
        let regular_close = C1_REGULAR_CLOSE_US;
        let info = SessionInfo {
            symbol: "FAKEC1".into(),
            // OptionsCboeBzxC2Edgx has no class-level last-trading-
            // day override, so only the regular close applies.
            trading_class: TradingClass::OptionsCboeBzxC2Edgx,
            regular: TimeWindow::new(0, regular_close),
            pre_market: None,
            post_market: None,
            curb: None,
            gth: None,
            gth_overnight: false,
            last_trading_day_close_us: None,
            settlement: Settlement::Pm,
            valid_from_yyyymmdd: None,
        };
        assert_eq!(info.settlement_cutoff_us(20_260_515), regular_close);
    }

    // ── PM-settlement protocol constants ───────────────────────────

    #[test]
    fn pm_settlement_constant_matches_protocol_value() {
        // 16:00 ET expressed two equivalent ways: 16 * 3600 hours
        // and the literal 57_600_000_000 microseconds. The constant
        // pins the protocol fact at one location.
        assert_eq!(OPTION_PM_SETTLEMENT_US, 57_600_000_000);
        assert_eq!(OPTION_PM_SETTLEMENT_US, 16 * 3_600 * 1_000_000);
        assert_eq!(OPTION_PM_SETTLEMENT_MS, 57_600_000);
        assert_eq!(OPTION_PM_SETTLEMENT_MS, 16 * 3_600 * 1_000);
    }

    // Compile-time guard: the μs and ms variants must agree under a
    // 1_000-factor rescale. Surfaces any future drift as a build
    // break instead of a runtime failure.
    const _: () = {
        assert!(OPTION_PM_SETTLEMENT_US == (OPTION_PM_SETTLEMENT_MS as i64) * 1_000);
    };

    #[test]
    fn pm_settlement_constant_resolves_through_class_level_override() {
        // The OptionsCboeC1 class-level override should equal the
        // public constant, confirming the constant is the single
        // source of truth for the protocol fact.
        assert_eq!(
            TradingClass::OptionsCboeC1.class_level_last_trading_day_close_us(),
            Some(OPTION_PM_SETTLEMENT_US),
        );
    }
}
