//! Resource limits for scripts.

use std::time::Duration;

use crate::SandboxError;

/// Resource limits for a [`Sandbox`](crate::Sandbox).
///
/// Every script runs under all three. Hitting one gives a typed
/// [`SandboxError`](crate::SandboxError).
///
/// Build one from [`Limits::default`] or [`Limits::new`] and adjust it with
/// the `with_*` setters; more limits may be added in later versions:
///
/// ```
/// use std::time::Duration;
/// use surimono_core::Limits;
///
/// let limits = Limits::default().with_time(Duration::from_millis(500));
/// assert_eq!(limits.memory_bytes, Limits::DEFAULT_MEMORY_BYTES);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
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
    /// Largest heap limit accepted: half the address space (`isize::MAX / 2`).
    /// More than any machine has, and small enough that no allocation the
    /// limit admits can overflow the allocator's size arithmetic.
    pub const MAX_MEMORY_BYTES: usize = (isize::MAX / 2) as usize;

    /// Limits from explicit values. Check them with [`validate`](Self::validate)
    /// (the sandbox does so when it is created).
    pub const fn new(memory_bytes: usize, time: Duration, stack_bytes: usize) -> Self {
        Self {
            memory_bytes,
            time,
            stack_bytes,
        }
    }

    /// These limits with the heap limit set to `memory_bytes`.
    #[must_use]
    pub const fn with_memory_bytes(mut self, memory_bytes: usize) -> Self {
        self.memory_bytes = memory_bytes;
        self
    }

    /// These limits with the time budget set to `time`.
    #[must_use]
    pub const fn with_time(mut self, time: Duration) -> Self {
        self.time = time;
        self
    }

    /// These limits with the stack limit set to `stack_bytes`.
    #[must_use]
    pub const fn with_stack_bytes(mut self, stack_bytes: usize) -> Self {
        self.stack_bytes = stack_bytes;
        self
    }

    /// Checks the limits are in the ranges the engine can enforce.
    pub fn validate(&self) -> Result<(), SandboxError> {
        if !(Self::MIN_MEMORY_BYTES..=Self::MAX_MEMORY_BYTES).contains(&self.memory_bytes) {
            return Err(SandboxError::InvalidOptions {
                message: format!(
                    "memory_bytes is {}, it must be between {} and {}",
                    self.memory_bytes,
                    Self::MIN_MEMORY_BYTES,
                    Self::MAX_MEMORY_BYTES
                ),
            });
        }
        if self.time.is_zero() {
            return Err(SandboxError::InvalidOptions {
                message: "time is zero; every call would fail at once".into(),
            });
        }
        if !(Self::MIN_STACK_BYTES..=Self::MAX_STACK_BYTES).contains(&self.stack_bytes) {
            return Err(SandboxError::InvalidOptions {
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

    #[test]
    fn setters_and_new_build_the_same_limits() {
        let a = Limits::new(2 << 20, Duration::from_millis(5), 128 * 1024);
        let b = Limits::default()
            .with_memory_bytes(2 << 20)
            .with_time(Duration::from_millis(5))
            .with_stack_bytes(128 * 1024);
        assert_eq!(a, b);
        assert_eq!(a.validate(), Ok(()));
    }

    #[test]
    fn out_of_range_limits_are_rejected() {
        let ok = Limits::default();
        for bad in [
            ok.with_memory_bytes(Limits::MIN_MEMORY_BYTES - 1),
            ok.with_memory_bytes(Limits::MAX_MEMORY_BYTES + 1),
            ok.with_memory_bytes(usize::MAX),
            ok.with_time(Duration::ZERO),
            ok.with_stack_bytes(Limits::MIN_STACK_BYTES - 1),
            ok.with_stack_bytes(Limits::MAX_STACK_BYTES + 1),
        ] {
            assert!(
                matches!(bad.validate(), Err(SandboxError::InvalidOptions { .. })),
                "{bad:?}"
            );
        }
        for good in [
            ok.with_memory_bytes(Limits::MIN_MEMORY_BYTES),
            ok.with_memory_bytes(Limits::MAX_MEMORY_BYTES),
            ok.with_time(Duration::from_nanos(1)),
            ok.with_time(Duration::MAX),
        ] {
            assert_eq!(good.validate(), Ok(()), "{good:?}");
        }
    }
}
