//! Tests for the `*_blocking` API's runtime guard.
//!
//! Calls from inside a tokio `current_thread` runtime cannot drive a
//! future to completion — `block_in_place` panics there, and re-entering
//! the same runtime would deadlock. The kit returns
//! `Error::BlockingFromCurrentThreadRuntime` instead.

#![cfg(feature = "parquet-loader")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use hourskit::{Error, Hourskit};

#[test]
fn blocking_from_current_thread_runtime_returns_typed_error() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current_thread runtime");
    let result = rt.block_on(async {
        let cache_dir = tempfile::TempDir::new().expect("tmp");
        let client = Hourskit::new()
            .with_cache_dir(cache_dir.path().to_path_buf())
            .with_mirror_url(None);
        client.session_blocking("SPX")
    });
    assert!(
        matches!(result, Err(Error::BlockingFromCurrentThreadRuntime)),
        "expected BlockingFromCurrentThreadRuntime, got {result:?}"
    );
}

#[test]
fn blocking_from_no_runtime_drives_a_fresh_runtime() {
    // Sanity check: calling `*_blocking` outside any runtime should NOT
    // produce the guard error (it spins up its own runtime). The actual
    // call may fail downstream because of network / cache state, but the
    // failure must NOT be `BlockingFromCurrentThreadRuntime`.
    let cache_dir = tempfile::TempDir::new().expect("tmp");
    let client = Hourskit::new()
        .with_base_url("http://127.0.0.1:1") // unreachable
        .with_cache_dir(cache_dir.path().to_path_buf())
        .with_mirror_url(None);
    let result = client.session_blocking("SPX");
    assert!(
        !matches!(result, Err(Error::BlockingFromCurrentThreadRuntime)),
        "blocking from no-runtime context must not trigger the runtime guard, got {result:?}"
    );
}
