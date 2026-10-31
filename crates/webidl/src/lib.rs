//! WebIDL parsing and binding generation from w3c/webref.
//!
//! Part of surimono, a library that runs a document's JavaScript without a
//! browser. This crate is internal (`publish = false`) and phase-2
//! scaffolding: nothing here reaches the `surimono` facade.
//!
//! - [`parse`] turns one `.idl` source into a syntax tree ([`ast`]).
//! - [`IdlSet`] holds many parsed files and merges partial interfaces and
//!   mixins into one view per interface ([`MergedInterface`]).
//! - [`vendored`] holds the `.idl` files copied from w3c/webref at a pinned
//!   commit (no network in builds or tests).
//! - [`generate_stubs`] emits Rust binding stubs for chosen interfaces;
//!   [`stubs`] is that output for the phase-2 starting set, checked in and
//!   kept fresh by a test. See `crates/webidl/DESIGN.md`.

pub mod ast;
mod codegen;
mod lexer;
mod model;
mod parser;
pub mod runtime;
pub mod vendored;

/// Generated binding stubs for the starting set of interfaces
/// ([`STUB_INTERFACES`]). Regenerate with
/// `cargo run -p surimono-webidl --example regen`.
#[rustfmt::skip]
pub mod stubs;

pub use codegen::{GenError, STUB_INTERFACES, generate_stubs, rust_name};
pub use model::{IdlSet, MergedInterface, MergedMember, NameKind, SetError};
pub use parser::parse;

/// Version of this crate, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// An error parsing or generating from a set of IDL files.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Parsing or merging failed.
    #[error(transparent)]
    Set(#[from] SetError),
    /// Generation failed.
    #[error(transparent)]
    Gen(#[from] GenError),
}

/// The contents [`stubs`] (`src/stubs.rs`) must have: [`STUB_INTERFACES`]
/// generated from the [`vendored`] IDL.
pub fn generate_vendored_stubs() -> Result<String, Error> {
    let set = vendored::idl_set()?;
    let header = format!(
        "Binding stubs from w3c/webref commit {} (crates/webidl/idl/).",
        vendored::WEBREF_COMMIT
    );
    Ok(generate_stubs(&set, STUB_INTERFACES, &header)?)
}

/// The contents `tests/golden/mini.rs` must have: every interface in
/// `tests/golden/mini.idl`, a small hand-written IDL file that exercises
/// each member kind without depending on webref.
pub fn generate_mini_golden() -> Result<String, Error> {
    const MINI: &str = include_str!("../tests/golden/mini.idl");
    let set = IdlSet::parse_sources([("mini.idl", MINI)])?;
    Ok(generate_stubs(
        &set,
        &["Base", "Widget"],
        "Golden output for tests/golden/mini.idl.",
    )?)
}

/// A syntax error in an IDL source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{line}:{column}: {message}")]
pub struct ParseError {
    /// 1-based line.
    pub line: u32,
    /// 1-based column, in characters.
    pub column: u32,
    /// What was expected and what was found.
    pub message: String,
}

impl ParseError {
    pub(crate) fn new(line: u32, column: u32, message: impl Into<String>) -> Self {
        ParseError {
            line,
            column,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert_eq!(super::VERSION.split('.').count(), 3);
    }
}
