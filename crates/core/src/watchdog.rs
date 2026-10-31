//! The watchdog: one thread per process that raises a running call's stop
//! flag when its deadline passes.
//!
//! QuickJS asks the interrupt handler only once every 10,000 polls, and a
//! loop whose body makes one expensive native call (`JSON.stringify` of a
//! big object, `indexOf` on a big array) polls two or three times per
//! iteration, so the deadline used to be looked at only every few thousand
//! iterations, minutes late. The patched engine (`vendor/rquickjs-sys`,
//! PATCHES.md) also reads a per-runtime flag on every poll, one relaxed
//! load; this thread sets that flag at the deadline.
//!
//! Each armed call puts one timer in a map ordered by deadline and takes it
//! out again when it ends, so the map holds only the calls running right
//! now and nothing grows per render. The thread sleeps until the earliest
//! deadline. Its clock is monotonic and only decides when to stop a script,
//! never what a script sees.
//!
//! A forked child inherits no threads, and possibly a lock some thread
//! held at the fork, so a process whose id differs from the registry's
//! starts a fresh registry and thread and never touches the old one.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Instant;

use crate::SandboxError;
use crate::engine::LimitState;

/// Identifies one armed call's timer: its deadline, then a sequence number
/// that makes keys unique and tells a stale timer from the current one.
pub(crate) type TimerKey = (Instant, u64);

/// The timers of one process and the thread that fires them.
#[derive(Debug)]
pub(crate) struct Registry {
    pid: u32,
    inner: Mutex<Inner>,
    wake: Condvar,
}

#[derive(Debug, Default)]
struct Inner {
    timers: BTreeMap<TimerKey, Weak<LimitState>>,
    next_seq: u64,
}

/// This process's registry, once its thread is running.
static CURRENT: Mutex<Option<Arc<Registry>>> = Mutex::new(None);

/// Locks `m`, ignoring poisoning: every critical section here leaves its
/// data consistent at every step, so a panic in one can't corrupt it.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// This process's registry, starting the watchdog thread on first use (and
/// again in a forked child).
pub(crate) fn registry() -> Result<Arc<Registry>, SandboxError> {
    let pid = std::process::id();
    let mut current = lock(&CURRENT);
    if let Some(r) = current.as_ref().filter(|r| r.pid == pid) {
        return Ok(r.clone());
    }
    let r = Arc::new(Registry {
        pid,
        inner: Mutex::new(Inner::default()),
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
    *current = Some(r.clone());
    Ok(r)
}

impl Registry {
    /// Adds a timer that raises `state`'s stop flag at `at`.
    pub(crate) fn insert(&self, at: Instant, state: Weak<LimitState>) -> TimerKey {
        let mut inner = lock(&self.inner);
        let key = (at, inner.next_seq);
        inner.next_seq = inner.next_seq.wrapping_add(1);
        let earliest = inner.timers.first_key_value().is_none_or(|(k, _)| key < *k);
        inner.timers.insert(key, state);
        drop(inner);
        if earliest {
            self.wake.notify_one();
        }
        key
    }

    /// Removes a timer if it hasn't fired yet.
    pub(crate) fn remove(&self, key: TimerKey) {
        // A registry inherited across fork has no thread, and its lock may
        // have been held by a thread that no longer exists.
        if self.pid != std::process::id() {
            return;
        }
        lock(&self.inner).timers.remove(&key);
    }

    /// Number of timers waiting, for tests.
    #[cfg(test)]
    pub(crate) fn pending(&self) -> usize {
        lock(&self.inner).timers.len()
    }

    /// The watchdog thread: fires due timers, then sleeps until the next.
    fn run(&self) {
        let mut due = Vec::new();
        let mut inner = lock(&self.inner);
        loop {
            let now = Instant::now();
            while let Some(entry) = inner.timers.first_entry() {
                if entry.key().0 > now {
                    break;
                }
                due.push(entry.remove_entry());
            }
            if !due.is_empty() {
                // Fire outside the lock: `expire` takes the call's own lock,
                // which `LimitState::arm` holds while it inserts here.
                drop(inner);
                for (key, state) in due.drain(..) {
                    if let Some(state) = state.upgrade() {
                        state.expire(key);
                    }
                }
                inner = lock(&self.inner);
                continue;
            }
            inner = match inner.timers.first_key_value() {
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
