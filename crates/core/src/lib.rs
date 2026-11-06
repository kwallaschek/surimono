//! QuickJS sandbox, event loop on a virtual clock, and resource limits.
//!
//! Part of surimono, a library that runs a document's JavaScript without a
//! browser. Most users want the `surimono` facade crate instead.
//!
//! The centre of this crate is [`Sandbox`]: one QuickJS runtime and context
//! that runs untrusted scripts under [`Limits`] (memory, time, stack) and
//! reports every way a script can fail as a typed [`SandboxError`].
//!
//! ```
//! use surimono_core::{Limits, Sandbox, SandboxError};
//!
//! let mut sb = Sandbox::new(Limits::default())?;
//! assert_eq!(sb.eval::<i32>("6 * 7")?, 42);
//!
//! let mut quick = Sandbox::new(Limits::default().with_time(std::time::Duration::from_millis(50)))?;
//! assert_eq!(quick.eval::<()>("for (;;) {}"), Err(SandboxError::TimeLimit));
//! # Ok::<(), SandboxError>(())
//! ```

// A library never writes to stdout or stderr; tests may.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

#[allow(unsafe_code)]
mod embed;
#[allow(unsafe_code)]
mod engine;
mod error;
mod event_loop;
#[allow(unsafe_code)]
mod fork_guard;
mod fork_safe;
#[allow(unsafe_code)]
mod host_thread;
mod limits;
mod sandbox;
mod watchdog;

pub use embed::{compile_for_embedding, engine_fingerprint};
#[doc(hidden)]
pub use engine::{TeardownLeaks, limit_error, stack_available, teardown_leaks};
pub use engine::{fork_generation, release_free_memory};
pub use error::SandboxError;
/// Not public API: phase-2 scaffolding for `surimono-dom`.
#[doc(hidden)]
pub use event_loop::{
    EventLoopHandle, ExternalWait, LoopBridge, SettleOptions, SettleOutcome, SettleReport,
};
/// Not public API: for `surimono`.
#[doc(hidden)]
pub use fork_guard::{Busy, busy, paused};
/// Not public API: for `surimono-islands`.
#[doc(hidden)]
pub use host_thread::{HostThread, spawn_host_thread};
pub use limits::Limits;
pub use sandbox::{Bytecode, Deadline, Sandbox, SandboxOptions};

/// Not public API: timing hooks for surimono's own tests (feature
/// `test-hooks`).
#[cfg(any(test, feature = "test-hooks"))]
#[doc(hidden)]
pub use sandbox::test_hooks;

/// The rquickjs version surimono-core is built against, re-exported so
/// dependent crates use the same one for [`Sandbox::run`] closures.
pub use rquickjs;

/// Version of this crate, for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert_eq!(super::VERSION.split('.').count(), 3);
    }
}
