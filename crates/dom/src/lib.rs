//! Web API bindings from QuickJS to blitz-dom. JS objects hold node ids; all DOM state stays in Rust.
//!
//! Part of surimono, a library that runs a document's
//! JavaScript without a browser. See `docs/BRIEF.md` for the design and
//! `crates/dom/DESIGN.md` for how wrappers address nodes.
//!
//! Phase-2 scaffolding: internal only, not re-exported by the facade.

mod document;
mod error;
mod runtime;

pub use document::{Document, DomLimits};
pub use error::DomError;
pub use runtime::DomRuntime;

/// Version of this crate, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert_eq!(super::VERSION.split('.').count(), 3);
    }
}
