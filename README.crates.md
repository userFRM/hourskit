<!--
Thin crates.io / docs.rs front page. The full README.md (sources, methodology,
recipes) stays in the repo and is NOT shipped to the registry.
-->
# hourskit

US exchange trading-hours reference for Rust — Cboe options and US equities, microsecond-native.

```toml
[dependencies]
hourskit = "0.6.1"
```

```rust,no_run
#[tokio::main]
async fn main() -> hourskit::Result<()> {
    let info = hourskit::session("SPX").await?.expect("SPX in seed");
    println!("SPX regular: {}", info.regular);
    Ok(())
}
```

Full documentation: <https://github.com/userFRM/hourskit>

Licensed under MIT OR Apache-2.0.
