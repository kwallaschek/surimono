//! The island runner: load a JS bundle once, call its entry with JSON, get SVG back.
//!
//! Part of surimono, a library that runs a document's JavaScript without a
//! browser. Most users want the `surimono` facade crate instead.
//!
//! ```
//! use surimono_islands::{IslandRequest, IslandRunner, RunnerOptions};
//!
//! let runner = IslandRunner::new(RunnerOptions { pool_size: 1, ..RunnerOptions::default() })?;
//! runner.register_bundle(
//!     "hello",
//!     "function entry(specJson, ctx) {
//!        const spec = JSON.parse(specJson);
//!        return `<svg width=\"${ctx.width}\" height=\"${ctx.height}\"><text>${spec.text}</text></svg>`;
//!      }",
//!     "entry",
//! )?;
//! let svg = runner.render(&IslandRequest {
//!     renderer: "hello",
//!     spec_json: r#"{"text":"hi"}"#,
//!     width: 100,
//!     height: 50,
//! })?;
//! assert_eq!(svg, r#"<svg width="100" height="50"><text>hi</text></svg>"#);
//! # Ok::<(), surimono_islands::IslandError>(())
//! ```

// A library never writes to stdout or stderr; tests may.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

mod error;
mod measure;
mod runner;

pub use error::IslandError;
pub use measure::{TextMeasurer, TextMetrics};
pub use runner::{IslandRequest, IslandRunner, RunnerOptions, RunnerStats};
pub use surimono_core::{Limits, SandboxError};

/// Version of this crate, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert_eq!(super::VERSION.split('.').count(), 3);
    }
}
