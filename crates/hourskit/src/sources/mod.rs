//! Data-source modules.
//!
//! - [`bundled`] — synchronous reader from local `data/` parquet (no network).
//! - [`parquet_io`] — parquet writer / reader for the unified `sessions.parquet` table.

pub mod bundled;
pub mod parquet_io;
