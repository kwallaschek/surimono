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
mod engine;
mod error;
mod fork_safe;
mod limits;
mod sandbox;
mod watchdog;

pub use error::SandboxError;
pub use limits::Limits;
pub use sandbox::{Bytecode, Deadline, Sandbox, SandboxOptions};

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
