//! End-to-end integration test for the async client.
//!
//! Spins up a wiremock origin that serves the bundled parquet bytes and
//! verifies that:
//! 1. The first call hits the network and writes the cache.
//! 2. The second call is served from cache.

#![cfg(feature = "parquet-loader")]

use std::path::PathBuf;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use hourskit::Hourskit;

fn repo_data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data")
}

async fn mount_empty_manifest(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"{}"))
        .mount(server)
        .await;
}

#[tokio::test]
async fn end_to_end_session_lookup_revalidates_with_etag() {
    use wiremock::matchers::header_exists;

    let server = MockServer::start().await;
    mount_empty_manifest(&server).await;

    let parquet_bytes = std::fs::read(repo_data_dir().join("sessions.parquet"))
        .expect("seed sessions.parquet must exist");

    Mock::given(method("GET"))
        .and(path("/sessions.parquet"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .set_body_bytes(parquet_bytes.clone()),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/sessions.parquet"))
        .and(header_exists("If-None-Match"))
        .respond_with(ResponseTemplate::new(304))
        .mount(&server)
        .await;

    let cache_dir = tempfile::TempDir::new().expect("tmp dir");
    let client = Hourskit::new()
        .with_base_url(server.uri())
        .with_cache_dir(cache_dir.path().to_path_buf())
        .with_mirror_url(None);

    let first = client.session("SPX").await.expect("first call");
    assert!(first.is_some());

    let second = client.session("AAPL").await.expect("304 path");
    assert!(second.is_some());

    let cached =
        std::fs::read(cache_dir.path().join("sessions.parquet")).expect("read cached file");
    assert_eq!(cached, parquet_bytes);
}

#[tokio::test]
async fn missing_origin_falls_back_to_stale_cache() {
    let server = MockServer::start().await;
    mount_empty_manifest(&server).await;
    let parquet_bytes = std::fs::read(repo_data_dir().join("sessions.parquet"))
        .expect("seed sessions.parquet must exist");

    let cache_dir = tempfile::TempDir::new().expect("tmp dir");

    Mock::given(method("GET"))
        .and(path("/sessions.parquet"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(parquet_bytes))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/sessions.parquet"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let client = Hourskit::new()
        .with_base_url(server.uri())
        .with_cache_dir(cache_dir.path().to_path_buf())
        .with_mirror_url(None);

    // Prime cache.
    let _ = client.session("SPX").await.expect("prime");

    // Force a fresh client (so the in-flight cell is empty) but reuse cache.
    let client2 = Hourskit::new()
        .with_base_url(server.uri())
        .with_cache_dir(cache_dir.path().to_path_buf())
        .with_mirror_url(None);

    let info = client2.session("SPX").await.expect("stale cache");
    assert!(info.is_some());
}
