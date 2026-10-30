//! Resource limits for scripts.

use std::time::Duration;

use crate::SandboxError;

/// Resource limits for a [`Sandbox`](crate::Sandbox).
///
/// Every script runs under all three. Hitting one gives a typed
/// [`SandboxError`](crate::SandboxError), never a crash or a hang.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Limits {
    /// Maximum bytes the JavaScript heap may hold at once.
    pub memory_bytes: usize,
    /// Time budget for one top-level call, including the microtasks it queues.
    pub time: Duration,
    /// Maximum native stack the engine may use below the caller, in bytes.
    ///
    /// The calling thread's stack must be larger than this plus some headroom
    /// for the Rust frames around it; the island runner sizes its worker
    /// threads accordingly.
    pub stack_bytes: usize,
}

impl Limits {
    /// Default heap limit: 64 MiB.
    pub const DEFAULT_MEMORY_BYTES: usize = 64 * 1024 * 1024;
    /// Default time budget: 2 seconds.
    pub const DEFAULT_TIME: Duration = Duration::from_secs(2);
    /// Default stack limit: 512 KiB.
    pub const DEFAULT_STACK_BYTES: usize = 512 * 1024;
    /// Smallest heap limit accepted: 1 MiB. A fresh context needs about
    /// 170 KiB, and QuickJS does not recover from failing to build one.
    pub const MIN_MEMORY_BYTES: usize = 1024 * 1024;
    /// Smallest stack limit accepted: 64 KiB.
    pub const MIN_STACK_BYTES: usize = 64 * 1024;
    /// Largest stack limit accepted: 16 MiB. QuickJS treats larger values,
    /// and zero, as "no limit", which would let recursion crash the thread.
    pub const MAX_STACK_BYTES: usize = 16 * 1024 * 1024;

    /// Checks the limits are in the ranges the engine can enforce.
    pub fn validate(&self) -> Result<(), SandboxError> {
        if self.memory_bytes < Self::MIN_MEMORY_BYTES {
            return Err(SandboxError::InvalidLimits {
                message: format!(
                    "memory_bytes is {}, the minimum is {}",
                    self.memory_bytes,
                    Self::MIN_MEMORY_BYTES
                ),
            });
        }
        if !(Self::MIN_STACK_BYTES..=Self::MAX_STACK_BYTES).contains(&self.stack_bytes) {
            return Err(SandboxError::InvalidLimits {
                message: format!(
                    "stack_bytes is {}, it must be between {} and {}",
                    self.stack_bytes,
                    Self::MIN_STACK_BYTES,
                    Self::MAX_STACK_BYTES
                ),
            });
        }
        Ok(())
    }
}

impl Default for Limits {
    /// 64 MiB of heap, 2 seconds and 512 KiB of stack.
    fn default() -> Self {
        Self {
            memory_bytes: Self::DEFAULT_MEMORY_BYTES,
            time: Self::DEFAULT_TIME,
            stack_bytes: Self::DEFAULT_STACK_BYTES,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_spec() {
        let l = Limits::default();
        assert_eq!(l.memory_bytes, 64 * 1024 * 1024);
        assert_eq!(l.time, Duration::from_secs(2));
        assert_eq!(l.stack_bytes, 512 * 1024);
        assert_eq!(l.validate(), Ok(()));
    }
}
