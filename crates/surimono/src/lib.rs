//! surimono runs a document's JavaScript without a browser, so browserless
//! HTML-to-PDF engines such as fulgur can print it.
//!
//! In this release it renders *islands*: chart specs embedded in the HTML as
//! `<script type="application/vnd.surimono+json">` markers. A [`Renderer`] runs
//! each island's renderer (ECharts, Vega, or a bundle of your own) in a
//! sandboxed QuickJS runtime with memory, time and stack [`Limits`], measures
//! text with the fonts you pass in, and swaps the marker for the SVG. Everything
//! else in the document stays byte for byte as it was.
//!
//! ```
//! use surimono::{Error, Options, Renderer};
//!
//! // Build once, reuse for every document: a Renderer is Send + Sync.
//! let renderer = Renderer::new(Options::new().seed(42))?;
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

mod backend;
mod error;
mod options;
mod prepass;
mod renderer;

pub use error::{Error, IslandRef};
pub use options::{Limits, Options};
pub use prepass::ErrorPolicy;
pub use renderer::Renderer;

/// Version of this crate, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert_eq!(super::VERSION.split('.').count(), 3);
    }
}
