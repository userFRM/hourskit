//! Compile-time and runtime checks that the public surface follows
//! Rust-API-Guidelines idioms:
//!
//! - Every value type implements `Debug + Clone` (and `Copy` where appropriate).
//! - Every error variant has a non-empty `Display` impl.
//! - `Hourskit` implements `Default`.
//! - The error enum is `Send + Sync + 'static` so it can flow across tasks.

#![cfg(feature = "parquet-loader")]

use std::fmt::Debug;

use hourskit::{
    Error, Hourskit, HourskitError, ParseTimeUnitError, SessionInfo, TimeUnit, TimeWindow,
    TradingClass,
};

// ---------------------------------------------------------------------------
// Compile-time trait bounds — these compile only if the bounds hold.
// ---------------------------------------------------------------------------

#[allow(clippy::missing_const_for_fn)]
fn _bounds_value_types() {
    const fn require_debug_clone<T: Debug + Clone>() {}
    const fn require_copy<T: Copy>() {}
    const fn require_send_sync_static<T: Send + Sync + 'static>() {}

    require_debug_clone::<SessionInfo>();
    require_debug_clone::<TimeWindow>();
    require_debug_clone::<TradingClass>();
    require_debug_clone::<TimeUnit>();

    // `TimeWindow` is small enough to be Copy; same for TimeUnit.
    require_copy::<TimeWindow>();
    require_copy::<TimeUnit>();

    // Errors must flow across async tasks.
    require_send_sync_static::<Error>();
    require_send_sync_static::<HourskitError>();
    require_send_sync_static::<ParseTimeUnitError>();
}

// ---------------------------------------------------------------------------
// Runtime checks
// ---------------------------------------------------------------------------

#[test]
fn hourskit_default_constructs() {
    let _client = Hourskit::default();
}

#[test]
fn every_value_type_displays_or_debugs() {
    // Display is implemented on the time-shaped types.
    let w = TimeWindow::from_clock_et(9, 30, 16, 0);
    assert!(!format!("{w}").is_empty());
    let class = TradingClass::OptionsCboeC1;
    assert!(!format!("{class}").is_empty());
    let unit = TimeUnit::Microseconds;
    assert!(!format!("{unit}").is_empty());
}

#[test]
fn error_variants_all_have_non_empty_display() {
    let cases: &[Error] = &[
        Error::UnknownSymbol("X".into()),
        Error::Source("y".into()),
        Error::Parquet("z".into()),
        Error::SchemaMismatch {
            file: "foo.parquet".into(),
            expected: "(a, b)".into(),
            found: "(c)".into(),
        },
        Error::ChecksumMismatch {
            file: "x.parquet".into(),
            expected: "abc".into(),
            computed: "def".into(),
        },
        Error::Other("msg".into()),
    ];
    for e in cases {
        let s: String = format!("{e}");
        assert!(!s.is_empty(), "{e:?} should have a non-empty Display");
    }
}

#[test]
fn error_implements_std_error() {
    fn require_std_error<E: std::error::Error>() {}
    require_std_error::<Error>();
}

/// `Hourskit::new()` and the builder methods accept `&str` and `String`
/// uniformly via `impl Into<String>` / `impl AsRef<str>` — proved by simply
/// compiling.
#[test]
fn builder_accepts_both_str_and_string() {
    let _c1 = Hourskit::new().with_base_url("https://example.com");
    let _c2 = Hourskit::new().with_base_url(String::from("https://example.com"));
}
