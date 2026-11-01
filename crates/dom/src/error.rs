//! Typed errors from the DOM bindings.

use surimono_core::SandboxError;

/// Why a script running against a [`Document`](crate::Document) did not
/// complete.
///
/// The `StaleDocument`, `NodeRemoved`, `NoDocument`, `NodeLimit`,
/// `TreeTooDeep` and `HandleLimit` variants are *fatal to the
/// call*: the binding throws into the script (so it stops where it is), and
/// the call fails with this error even if the script caught the exception.
/// Each means the script used a handle that no longer names a node, or hit
/// a DOM resource limit; carrying on would give wrong data or unbounded
/// growth outside the JavaScript heap limit.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DomError {
    /// The script failed in the sandbox: an uncaught exception or a
    /// memory, time or stack limit.
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
    /// A wrapper from another document (an earlier render, or a document
    /// since replaced) was used.
    #[error("a node wrapper from another document was used")]
    StaleDocument,
    /// A wrapper whose node has since been removed and dropped was used.
    #[error("a node wrapper for a removed node was used")]
    NodeRemoved,
    /// The document reached [`DomLimits::max_nodes`](crate::DomLimits::max_nodes).
    #[error("the document reached its limit of {limit} nodes")]
    NodeLimit {
        /// The limit that was reached.
        limit: usize,
    },
    /// An insertion would nest nodes deeper than
    /// [`DomLimits::max_depth`](crate::DomLimits::max_depth).
    #[error("the document tree would be deeper than {limit} levels")]
    TreeTooDeep {
        /// The limit that was reached.
        limit: usize,
    },
    /// The document has given out
    /// [`DomLimits::max_handles`](crate::DomLimits::max_handles) handles.
    #[error("the document gave out its limit of {limit} node handles")]
    HandleLimit {
        /// The limit that was reached.
        limit: usize,
    },
    /// A [`DomLimits`](crate::DomLimits) value is out of range.
    #[error("invalid DOM limits: {message}")]
    InvalidLimits {
        /// Which limit and the allowed range.
        message: String,
    },
    /// The runtime has no document loaded.
    #[error("no document is loaded")]
    NoDocument,
}

impl DomError {
    /// The message thrown into the script for a fatal error. Not part of the
    /// contract; the Rust error is.
    pub(crate) fn script_message(&self) -> String {
        format!("surimono: {self}")
    }
}
