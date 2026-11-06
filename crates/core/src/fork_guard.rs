//! A `fork` waits for a page.
//!
//! Page work takes locks in libraries that know nothing of `fork`, and a lock
//! held by another thread at a `fork` stays held for good in the child, where
//! that thread doesn't exist. The child's first page would then never end.
//! No caller can reach those locks, so instead no thread holds one at a `fork`:
//! work that may take them marks itself [`busy`], and a `pthread_atfork`
//! handler holds new work back and waits for what is running, for up to
//! [`MOST_WAIT`]. Only `fork` waits: `posix_spawn` and Ruby's `system` don't run
//! these handlers. Past that the `fork` goes ahead, so a page with a time limit
//! longer than that can still leave a child stuck at such a lock.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

/// Longest a `fork` waits for running pages.
#[cfg(unix)]
const MOST_WAIT: Duration = Duration::from_secs(5);
/// How often a waiting thread looks again.
const POLL: Duration = Duration::from_micros(200);

/// Threads busy now.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
/// A `fork` is under way.
static FORKING: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// This thread is counted in [`ACTIVE`].
    static HELD: Cell<bool> = const { Cell::new(false) };
}

fn enter() {
    loop {
        while FORKING.load(Ordering::SeqCst) {
            std::thread::sleep(POLL);
        }
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        if !FORKING.load(Ordering::SeqCst) {
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
pub(crate) struct Busy(());

impl Drop for Busy {
    fn drop(&mut self) {
        leave();
    }
}

/// Marks this thread as doing work a `fork` has to wait for.
pub(crate) fn busy() -> Busy {
    install();
    enter();
    Busy(())
}

/// Runs `f`, which holds no lock of a library and may wait on the caller,
/// without keeping a `fork` waiting. A Ruby `fork` holds the VM lock, which a
/// load function may need.
pub(crate) fn paused<T>(f: impl FnOnce() -> T) -> T {
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
fn install() {
    static INSTALLED: AtomicBool = AtomicBool::new(false);

    extern "C" fn prepare() {
        FORKING.store(true, Ordering::SeqCst);
        let started = std::time::Instant::now();
        while ACTIVE.load(Ordering::SeqCst) > 0 && started.elapsed() < MOST_WAIT {
            std::thread::sleep(POLL);
        }
    }

    extern "C" fn parent() {
        FORKING.store(false, Ordering::SeqCst);
    }

    extern "C" fn child() {
        ACTIVE.store(0, Ordering::SeqCst);
        FORKING.store(false, Ordering::SeqCst);
    }

    if !INSTALLED.swap(true, Ordering::SeqCst) {
        // SAFETY: pthread_atfork only stores the three function pointers,
        // which are plain `extern "C"` functions of this module and live as
        // long as the process. They touch only atomics, a clock and a sleep,
        // and don't unwind. A failure (out of memory) leaves `fork` as it was.
        unsafe { libc::pthread_atfork(Some(prepare), Some(parent), Some(child)) };
    }
}

#[cfg(not(unix))]
fn install() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_work_keeps_nothing_waiting() {
        assert_eq!(paused(|| 1), 1);
        assert!(!HELD.get());
        let busy = busy();
        assert!(HELD.get());
        let seen = paused(|| HELD.get());
        assert!(!seen);
        assert!(HELD.get());
        drop(busy);
        assert!(!HELD.get());
    }
}
