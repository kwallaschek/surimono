//! surimono runs a document's JavaScript without a browser, so browserless
//! HTML-to-PDF engines such as fulgur can print it.
//!
//! In this release it renders *islands*: chart specs embedded in the HTML as
//! `<script type="application/vnd.surimono+json">` markers. A [`Renderer`] runs
//! each island's renderer (the built-in ECharts, Vega and Vega-Lite, or a
//! bundle of your own; see [`Options::bundle`]) in a sandboxed QuickJS runtime
//! with memory, time and stack [`Limits`] per island, measures text with the
//! fonts you pass in (at least one is required), sanitises the SVG it returns
//! (text becomes outlines in those fonts by default, see [`TextMode`]), and
//! swaps the marker for it. Everything else in the document stays byte for
//! byte as it was.
//!
//! Cargo features, both on by default: `echarts` builds in the `echarts`
//! renderer (Apache ECharts), and `vega` the `vega` and `vega-lite` renderers.
//!
//! A `Renderer` built before a `fork` (Puma's `preload_app!` or `fork_worker`,
//! Unicorn, Resque) starts its runtime pool and text measurer again in the
//! child on first use.
//!
//! ```
//! use surimono::{Error, Options, Renderer};
//!
//! // Build once, reuse for every document: a Renderer is Send + Sync. Pass
//! // the font files your PDF engine prints with (here, Noto Sans).
//! # let font = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/fonts/NotoSans-Regular.ttf"))
//! #     .unwrap_or_else(|e| panic!("{e}"));
//! let renderer = Renderer::new(Options::new().font(font).seed(42))?;
//!
//! let html = r#"<h1>Sales</h1>
//! <script type="application/vnd.surimono+json" data-renderer="no-such-chart">{}</script>"#;
//! match renderer.render_islands(html) {
//!     Ok(out) => println!("{out}"),
//!     Err(Error::UnknownRenderer { island, enabled }) => {
//!         assert_eq!(island.index, 0);
//!         assert!(!enabled.is_empty());
//!     }
//!     Err(other) => return Err(other),
//! }
//! # Ok::<(), Error>(())
//! ```
//!
//! Same input, same output: no wall-clock time, real randomness, system fonts or
//! network reach a render, so the same options and HTML always give the same bytes.

#![warn(missing_docs)]
// A library never writes to stdout or stderr; tests may.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

mod backend;
mod charref;
mod error;
mod options;
mod prepass;
mod renderer;
mod svg;

pub use error::{Error, IslandRef};
pub use options::{DEFAULT_NOW_MS, Limits, Options, TextMode};
pub use prepass::ErrorPolicy;
pub use renderer::Renderer;

/// Version of this crate, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The built-in renderers compiled into this build, sorted (Cargo features
/// `echarts` and `vega`). Every [`Renderer`] offers them unless
/// [`Options::renderers`] narrows its list.
///
/// ```
/// let names = surimono::builtin_renderers();
/// assert!(names.windows(2).all(|w| w[0] < w[1]));
/// ```
pub fn builtin_renderers() -> Vec<String> {
    backend::builtin_renderers()
}

/// How many of [`builtin_renderers`] load precompiled bytecode in this
/// process, for diagnostics and packaging checks.
///
/// Equal to `builtin_renderers().len()` in a normal build. Less (usually 0)
/// when the environment variable `SURIMONO_PRECOMPILE=0` was set when this
/// was built or when the process started, or when the bytecode doesn't fit
/// the running engine (a broken cross build). Output is the same either
/// way; only each built-in's first render in a process is slower, because
/// it compiles the renderer's source. The first call takes a few
/// milliseconds; later calls are free.
///
/// ```
/// let precompiled = surimono::precompiled_builtins();
/// assert!(precompiled <= surimono::builtin_renderers().len());
/// ```
pub fn precompiled_builtins() -> usize {
    surimono_islands::precompiled_builtins()
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert_eq!(super::VERSION.split('.').count(), 3);
    }

    /// Without `SURIMONO_PRECOMPILE=0` (at build or run time) every
    /// built-in loads precompiled bytecode.
    #[test]
    fn every_builtin_is_precompiled() {
        let off = option_env!("SURIMONO_PRECOMPILE") == Some("0")
            || std::env::var_os("SURIMONO_PRECOMPILE").is_some_and(|v| v == "0");
        let want = if off {
            0
        } else {
            super::builtin_renderers().len()
        };
        assert_eq!(super::precompiled_builtins(), want);
        if cfg!(feature = "echarts") {
            assert!(super::builtin_renderers().iter().any(|n| n == "echarts"));
        }
    }
}
