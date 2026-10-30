//! Typed errors from the island runner.

use surimono_core::SandboxError;

/// Why an island did not render.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IslandError {
    /// The renderer's script failed or hit a limit.
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
    /// No bundle is registered under this renderer name.
    #[error("unknown renderer {name:?}")]
    UnknownRenderer {
        /// The name that was asked for.
        name: String,
    },
    /// The island's spec is not valid JSON.
    #[error("island spec is not valid JSON: {message} at line {line} column {column}")]
    BadJson {
        /// What the parser found wrong.
        message: String,
        /// The 1-based line of the error in the spec.
        line: usize,
        /// The 1-based column of the error in the spec.
        column: usize,
    },
    /// The renderer's entry returned something other than a string.
    #[error("renderer {renderer:?} returned {found}, not an SVG string")]
    NotAString {
        /// The renderer name.
        renderer: String,
        /// The JavaScript type it returned.
        found: String,
    },
    /// A bundle being registered does not define its entry function.
    #[error("bundle {bundle:?} does not define a global function {entry:?}")]
    MissingEntry {
        /// The bundle name.
        bundle: String,
        /// The entry function it should define.
        entry: String,
    },
    /// The runner options or the request are out of range.
    #[error("invalid island options: {message}")]
    InvalidOptions {
        /// What is wrong.
        message: String,
    },
    /// A worker thread failed; the runner replaced its sandbox.
    #[error("island worker failed: {message}")]
    Worker {
        /// What happened.
        message: String,
    },
}

impl IslandError {
    /// Whether this is a sandbox limit error (memory, time or stack).
    pub fn is_limit(&self) -> bool {
        matches!(self, Self::Sandbox(e) if e.is_limit())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_sandbox_errors() {
        let e: IslandError = SandboxError::TimeLimit.into();
        assert!(e.is_limit());
        assert_eq!(e.to_string(), "script exceeded the time limit");
        assert!(!IslandError::UnknownRenderer { name: "x".into() }.is_limit());
    }
}
