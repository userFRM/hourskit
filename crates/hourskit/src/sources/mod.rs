//! Data-source modules.
//!
//! - [`bundled`] — synchronous reader from local `data/` parquet (no network).
//! - [`parquet_io`] — parquet writer / reader for the unified `sessions.parquet` table.
//!
//! Both modules are gated behind the `parquet-loader` feature (enabled by
//! default). With the feature off the modules disappear; downstream callers
//! that want to drop the transitive `paste 1.0.15` dependency must
//! construct [`crate::SessionInfo`] values from explicit data instead of
//! reading the bundled parquet.

#[cfg(feature = "parquet-loader")]
pub mod bundled;
#[cfg(feature = "parquet-loader")]
pub mod parquet_io;
