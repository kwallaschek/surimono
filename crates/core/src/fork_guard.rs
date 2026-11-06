//! A `fork` waits for work that may hold a library's lock.
//!
//! Page work takes locks in libraries that know nothing of `fork`, and a lock
//! held by another thread at a `fork` stays held for good in the child, where
//! that thread doesn't exist. Whoever takes it there never returns. No caller
//! can reach those locks, so the aim is that no thread holds one at a `fork`:
//! work that may take them marks itself [`busy`], and `pthread_atfork` handlers
//! hold new work back and wait for what is running, for up to [`MOST_WAIT`].
//!
//! That is a bound, not a promise. The facade's `Busy` covers a page's parse,
//! scripts and finish, each with its own `Limits::time`, so up to three times
//! the limit: 6 s with the default 2 s, which can outlast the wait. Past it the
//! `fork` goes ahead, and a child that inherits such a lock stops at it.
//!
//! The wait is the forking thread's, and Ruby holds its VM lock across `fork`,
//! so there every Ruby thread of the process stops with it. Only `fork` runs
//! these handlers. `posix_spawn` doesn't, nor does Ruby's `system` for an
//! ordinary user, which uses `vfork`; as root Ruby calls `fork` for `system`,
//! backticks and `spawn` too, so each of those waits.

use std::cell::Cell;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use crate::error::SandboxError;

/// Longest a `fork` waits for running work.
#[cfg(unix)]
const MOST_WAIT: Duration = Duration::from_secs(5);
/// How often a waiting thread looks again.
const POLL: Duration = Duration::from_micros(200);

/// Threads busy now.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
/// `fork`s under way: glibc runs two threads' `prepare` handlers at once.
static FORKS: AtomicUsize = AtomicUsize::new(0);
/// Set once the handlers are registered here or in an ancestor, which a fork
/// keeps.
static HANDLERS: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// This thread is counted in [`ACTIVE`].
    static HELD: Cell<bool> = const { Cell::new(false) };
}

fn enter() {
    loop {
        while FORKS.load(Ordering::SeqCst) > 0 {
            std::thread::sleep(POLL);
        }
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        if FORKS.load(Ordering::SeqCst) == 0 {
            HELD.set(true);
            return;
        }
        leave();
    }
}

fn leave() {
    HELD.set(false);
    // None only in a forked child, where the count started again.
    let _ = ACTIVE.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
}

/// While it lives, a `fork` waits for this thread. See the module docs.
#[derive(Debug)]
pub struct Busy(PhantomData<*const ()>);

impl Drop for Busy {
    fn drop(&mut self) {
        leave();
    }
}

/// Marks this thread as doing work a `fork` has to wait for, once any `fork`
/// under way is done.
///
/// Fails only if the handlers can't be registered (out of memory).
pub fn busy() -> Result<Busy, SandboxError> {
    register()?;
    enter();
    Ok(Busy(PhantomData))
}

/// Runs `f`, which holds no library's lock and may wait for the caller, without
/// keeping a `fork` waiting: a Ruby `fork` holds the VM lock, which a load
/// function may need. Off a [`Busy`] thread it just runs `f`.
pub fn paused<T>(f: impl FnOnce() -> T) -> T {
    struct Resume;
    impl Drop for Resume {
        fn drop(&mut self) {
            enter();
        }
    }
    if !HELD.get() {
        return f();
    }
    leave();
    let _resume = Resume;
    f()
}

#[cfg(unix)]
extern "C" fn prepare() {
    FORKS.fetch_add(1, Ordering::SeqCst);
    let started = std::time::Instant::now();
    while ACTIVE.load(Ordering::SeqCst) > 0 && started.elapsed() < MOST_WAIT {
        std::thread::sleep(POLL);
    }
}

#[cfg(unix)]
extern "C" fn parent() {
    let _ = FORKS.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
}

#[cfg(unix)]
extern "C" fn child() {
    ACTIVE.store(0, Ordering::SeqCst);
    FORKS.store(0, Ordering::SeqCst);
}

/// Threads racing on the first call may each register the handlers, as with
/// `fork_generation`. A fork then counts itself twice and clears both.
fn register() -> Result<(), SandboxError> {
    #[cfg(unix)]
    if !HANDLERS.load(Ordering::Acquire) {
        // SAFETY: as in `fork_generation`: pthread_atfork only stores the
        // function pointers, under libc's own lock, which it also holds across
        // `fork`. They are plain `extern "C"` functions with no arguments that
        // live as long as this library. They touch atomics, a clock and a
        // sleep, none of which unwinds or takes a lock.
        let err = unsafe { libc::pthread_atfork(Some(prepare), Some(parent), Some(child)) };
        if err != 0 {
            return Err(SandboxError::Engine {
                message: format!("could not register the fork guard (error {err})"),
            });
        }
        // Only now may work start: every fork after this waits for it.
        HANDLERS.store(true, Ordering::Release);
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn paused_work_keeps_nothing_waiting() {
        assert_eq!(paused(|| 1), 1);
        assert!(!HELD.get());
        let busy = busy().unwrap_or_else(|e| panic!("{e}"));
        assert!(HELD.get());
        let inside = paused(|| HELD.get());
        assert!(!inside);
        assert!(HELD.get());
        drop(busy);
        assert!(!HELD.get());
    }

    /// Two forks at once: the first to finish doesn't let work in on the other.
    #[test]
    fn new_work_waits_for_the_last_of_two_forks() {
        prepare();
        prepare();
        parent();
        let (done, finished) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let held = busy().map(drop);
            let _ = done.send(held);
        });
        assert!(finished.recv_timeout(Duration::from_millis(100)).is_err());
        parent();
        let held = finished.recv_timeout(Duration::from_secs(5));
        assert!(matches!(held, Ok(Ok(()))), "{held:?}");
        worker
            .join()
            .unwrap_or_else(|_| panic!("the worker panicked"));
    }
}
