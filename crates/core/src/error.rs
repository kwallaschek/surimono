//! Typed errors from the sandbox.

/// Why a sandboxed script did not complete.
///
/// The three limit variants are terminal for the call: the sandbox discards the
/// script's state afterwards (see [`Sandbox`](crate::Sandbox)).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SandboxError {
    /// The script tried to use more heap than [`Limits::memory_bytes`](crate::Limits::memory_bytes).
    ///
    /// Reported whenever an allocation was refused during the call, even if the
    /// script caught the error and carried on.
    #[error("script exceeded the memory limit")]
    MemoryLimit,
    /// The script ran longer than [`Limits::time`](crate::Limits::time).
    #[error("script exceeded the time limit")]
    TimeLimit,
    /// The script recursed deeper than [`Limits::stack_bytes`](crate::Limits::stack_bytes) allows.
    #[error("script exceeded the stack limit (too much recursion)")]
    StackOverflow,
    /// The call was stopped because the flag given to
    /// [`Sandbox::set_cancel_flag`](crate::Sandbox::set_cancel_flag) went up.
    ///
    /// Not a limit error, but just as terminal: the sandbox discards the
    /// script's state.
    #[error("the script was cancelled")]
    Cancelled,
    /// A Rust host function panicked inside
    /// [`Deadline::guard`](crate::Deadline::guard). A bug in the host, not the
    /// script's fault, and terminal however the script handles the exception
    /// it saw: the sandbox discards the script's state. With
    /// `panic = "abort"` the process aborts instead and this is never returned.
    #[error("a host function panicked: {message}")]
    HostPanic {
        /// The panic's message, truncated to a few KiB.
        message: String,
    },
    /// The script threw an exception it did not catch.
    #[error("script threw {message}")]
    Exception {
        /// `"Name: message"` for Error objects, otherwise the thrown value as a
        /// string. Truncated to a few KiB.
        message: String,
        /// The JavaScript stack trace, when the thrown value had one.
        stack: Option<String>,
    },
    /// A value had the wrong type, e.g. a global that should be a function.
    #[error("unexpected value: {message}")]
    Type {
        /// What was expected and what was found.
        message: String,
    },
    /// An option is outside the range the engine supports, e.g. a
    /// [`Limits`](crate::Limits) value it can't enforce.
    #[error("invalid sandbox options: {message}")]
    InvalidOptions {
        /// Which option and the allowed range.
        message: String,
    },
    /// The engine failed for a reason that is not the script's fault.
    #[error("JavaScript engine error: {message}")]
    Engine {
        /// Details from the engine.
        message: String,
    },
}

impl SandboxError {
    /// Whether this is one of the resource-limit errors (memory, time, stack).
    pub fn is_limit(&self) -> bool {
        matches!(
            self,
            Self::MemoryLimit | Self::TimeLimit | Self::StackOverflow
        )
    }

    /// Whether the sandbox threw away the script's state after this error,
    /// because the script stopped at an arbitrary point: the limit errors,
    /// [`Cancelled`](Self::Cancelled) and [`HostPanic`](Self::HostPanic).
    pub fn discards_state(&self) -> bool {
        self.is_limit() || matches!(self, Self::Cancelled | Self::HostPanic { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_errors_are_flagged() {
        assert!(SandboxError::MemoryLimit.is_limit());
        assert!(SandboxError::TimeLimit.is_limit());
        assert!(SandboxError::StackOverflow.is_limit());
        let e = SandboxError::Exception {
            message: "Error: x".into(),
            stack: None,
        };
        assert!(!e.is_limit());
        assert!(!SandboxError::Cancelled.is_limit());
        assert!(SandboxError::Cancelled.discards_state());
        assert!(SandboxError::TimeLimit.discards_state());
        assert!(!e.discards_state());
        let panic = SandboxError::HostPanic {
            message: "boom".into(),
        };
        assert!(!panic.is_limit() && panic.discards_state());
        assert_eq!(panic.to_string(), "a host function panicked: boom");
        assert_eq!(e.to_string(), "script threw Error: x");
    }
}
