//! The watchdog: one thread per process that raises a running call's stop
//! flag when its deadline passes.
//!
//! QuickJS asks the interrupt handler once every 10,000 polls. The patched
//! engine (`vendor/rquickjs-sys`, PATCHES.md) also reads a per-runtime
//! flag on every poll, one relaxed load, so a deadline takes effect at the
//! next poll however much work lies between polls; this thread sets that
//! flag at the deadline.
//!
//! Each armed call puts one timer in a map ordered by deadline and takes it
//! out again when it ends, so the map holds only the calls running right
//! now and nothing grows per render. The thread sleeps until the earliest
//! deadline. Its clock is monotonic and only decides when to stop a script,
//! never what a script sees.
//!
//! A forked child inherits no threads, and possibly a lock some thread
//! held at the fork. So the registry lives in a lock-free
//! [`ForkSafeCell`]: a process whose fork generation differs from the
//! registry's starts a fresh registry and thread, and never touches its
//! parent's registry or that registry's lock. No lock is shared between a call's thread and the
//! watchdog except the registry's own map lock, which belongs to one
//! process (see [`LimitState`] for how a call and the watchdog agree
//! without one).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Instant;

use crate::SandboxError;
use crate::engine::LimitState;
use crate::fork_safe::ForkSafeCell;

/// Identifies one armed call's timer: its deadline, then the call's
/// sequence number, which makes keys unique and tells a stale timer from
/// the current one.
pub(crate) type TimerKey = (Instant, u64);

/// The timers of one process and the thread that fires them.
#[derive(Debug)]
pub(crate) struct Registry {
    /// Fork generation of the process this registry's thread runs in.
    generation: u64,
    /// The next call's sequence number; never 0, and stays below 2^63.
    next_seq: AtomicU64,
    inner: Mutex<BTreeMap<TimerKey, Weak<LimitState>>>,
    wake: Condvar,
}

/// This process's registry, once its thread is running.
static CURRENT: ForkSafeCell<Arc<Registry>> = ForkSafeCell::per_process();

/// Locks `m`, ignoring poisoning: every critical section here leaves its
/// data consistent at every step, so a panic in one can't corrupt it.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// This process's registry, starting the watchdog thread on first use (and
/// again in a forked child). Lock-free once started.
pub(crate) fn registry() -> Result<&'static Registry, SandboxError> {
    CURRENT
        .get_or_try_init(start, |why| SandboxError::Engine {
            message: format!("could not start the sandbox's watchdog thread: {why}"),
        })
        .map(|r| &**r)
}

/// This process's registry if its thread is running, without starting one.
pub(crate) fn current() -> Option<&'static Registry> {
    CURRENT.get().map(|r| &**r)
}

fn start() -> Result<Arc<Registry>, SandboxError> {
    let r = Arc::new(Registry {
        generation: crate::engine::fork_generation()?,
        next_seq: AtomicU64::new(1),
        inner: Mutex::new(BTreeMap::new()),
        wake: Condvar::new(),
    });
    let thread_r = r.clone();
    std::thread::Builder::new()
        .name("surimono-watchdog".into())
        .stack_size(256 * 1024)
        .spawn(move || thread_r.run())
        .map_err(|e| SandboxError::Engine {
            message: format!("could not start the sandbox's watchdog thread: {e}"),
        })?;
    Ok(r)
}

impl Registry {
    /// The fork generation of the process this registry (and its thread)
    /// belongs to.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// A sequence number for a new call, unique in this process.
    pub(crate) fn next_seq(&self) -> u64 {
        self.next_seq.fetch_add(1, Ordering::Relaxed)
    }

    /// Adds a timer that tells `state` at `key.0` that call `key.1` is due.
    pub(crate) fn insert(&self, key: TimerKey, state: Weak<LimitState>) {
        let mut timers = lock(&self.inner);
        let earliest = timers.first_key_value().is_none_or(|(k, _)| key < *k);
        timers.insert(key, state);
        drop(timers);
        if earliest {
            self.wake.notify_one();
        }
    }

    /// Removes a timer if it hasn't fired yet.
    pub(crate) fn remove(&self, key: TimerKey) {
        lock(&self.inner).remove(&key);
    }

    /// Number of timers waiting, for tests.
    #[cfg(test)]
    pub(crate) fn pending(&self) -> usize {
        lock(&self.inner).len()
    }

    /// The watchdog thread: fires due timers, then sleeps until the next.
    fn run(&self) {
        let mut due = Vec::new();
        let mut inner = lock(&self.inner);
        loop {
            let now = Instant::now();
            while let Some(entry) = inner.first_entry() {
                if entry.key().0 > now {
                    break;
                }
                due.push(entry.remove_entry());
            }
            if !due.is_empty() {
                // Fire outside the lock, so calls starting and ending don't
                // wait on it.
                drop(inner);
                for ((_, seq), state) in due.drain(..) {
                    if let Some(state) = state.upgrade() {
                        state.expire(seq);
                    }
                }
                inner = lock(&self.inner);
                continue;
            }
            inner = match inner.first_key_value() {
                Some((&(at, _), _)) => {
                    let wait = at.saturating_duration_since(now);
                    self.wake
                        .wait_timeout(inner, wait)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
                None => self
                    .wake
                    .wait(inner)
                    .unwrap_or_else(PoisonError::into_inner),
            };
        }
    }
}
