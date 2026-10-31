//! A lazily set global that a `fork` can never leave locked.
//!
//! `fork` copies only the calling thread. Any lock another thread held at
//! that moment stays held forever in the child, and so does a `OnceLock`
//! that another thread was initialising: the child's first `get_or_init`
//! then waits for a thread that doesn't exist (review of #47, finding 2: 21
//! of 29,300 forks hung on the watchdog registry's lock).
//!
//! [`ForkSafeCell`] never waits on such a thread. Reading a set value is a
//! load and an atomic check, with no lock. To set one, the threads of a
//! process elect one of them with a compare-and-swap on a word that holds
//! the electing process's fork generation
//! ([`fork_generation`](crate::fork_generation), bumped in every forked
//! child, so unlike a pid it never matches an ancestor's); the others yield
//! until it is done. A child sees an ancestor's generation in that word, so
//! it holds its own election instead of waiting on a parent's thread. Each value goes into a slot nobody has
//! touched yet, never into one that a parent's thread might have been
//! filling at the fork.
//!
//! A per-process cell ([`ForkSafeCell::per_process`]) also ignores values
//! set in an ancestor process, for state that a fork doesn't carry over
//! (a running thread). A shared cell ([`ForkSafeCell::shared`]) keeps them
//! (plain data, like compiled bytecode).

use std::fmt;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::engine::fork_generation;

/// Values a cell can hold over its lifetime: one per process, along a
/// chain of nested forks, that sets it (a per-process cell sets it again
/// in every process that uses it; a shared one only once). A failed or
/// panicking `init` takes no slot. 32 nested forks that each set the cell
/// is far beyond any server's process tree; past that, setting the cell
/// fails without running `init`.
const SLOTS: usize = 32;

/// Why a cell can't be set in this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unavailable {
    /// Every slot was used along this process's chain of forks.
    Full,
    /// The fork handler that tells processes apart couldn't be registered.
    NoForkHandler,
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Full => "too many nested forks",
            Self::NoForkHandler => "the fork handler could not be registered",
        })
    }
}

/// See the module docs.
pub(crate) struct ForkSafeCell<T> {
    /// Whether a value set in an ancestor process is ignored.
    per_process: bool,
    /// Values with the fork generation of the process that set them. Each
    /// is set at most once, by the elected thread, and never changes after.
    slots: [OnceLock<(u64, T)>; SLOTS],
    /// The slot that holds the newest value.
    current: AtomicUsize,
    /// The first slot never handed out (in this process or its ancestors).
    next: AtomicUsize,
    /// Fork generation of the process whose elected thread is setting the
    /// cell or has set it; 0 if none. Another process's generation is stale
    /// in a child.
    owner: AtomicU64,
}

impl<T> ForkSafeCell<T> {
    /// A cell whose value a forked child sets again for itself.
    pub(crate) const fn per_process() -> Self {
        Self::new(true)
    }

    /// A cell whose value a forked child inherits.
    pub(crate) const fn shared() -> Self {
        Self::new(false)
    }

    const fn new(per_process: bool) -> Self {
        Self {
            per_process,
            slots: [const { OnceLock::new() }; SLOTS],
            current: AtomicUsize::new(0),
            next: AtomicUsize::new(0),
            owner: AtomicU64::new(0),
        }
    }

    /// The value, if this process (or, for a shared cell, an ancestor) set
    /// it. Lock-free: one atomic load, `OnceLock::get`, and for a
    /// per-process cell two more for the fork generation.
    pub(crate) fn get(&self) -> Option<&T> {
        let (generation, value) = self
            .slots
            .get(self.current.load(Ordering::Acquire))?
            .get()?;
        (!self.per_process || fork_generation().is_ok_and(|g| g == *generation)).then_some(value)
    }

    /// The value, set by `init` if [`get`](Self::get) has none. Only one
    /// thread of a process runs `init` at a time; the others yield until it
    /// returns. If `init` fails (or panics) the error goes to its caller,
    /// and the next caller tries again. `unavailable` makes the error for a
    /// cell that can't be set in this process; `init` doesn't run then.
    pub(crate) fn get_or_try_init<E>(
        &self,
        init: impl FnOnce() -> Result<T, E>,
        unavailable: impl FnOnce(Unavailable) -> E,
    ) -> Result<&T, E> {
        if let Some(value) = self.get() {
            return Ok(value);
        }
        let Ok(generation) = fork_generation() else {
            return Err(unavailable(Unavailable::NoForkHandler));
        };
        // Elect one thread of this process.
        loop {
            if let Some(value) = self.get() {
                return Ok(value);
            }
            let owner = self.owner.load(Ordering::Acquire);
            if owner == generation {
                // Another thread of this process is setting it. It is alive
                // (a thread of an ancestor would have left an older
                // generation here), so this ends.
                std::thread::yield_now();
                continue;
            }
            if self
                .owner
                .compare_exchange(owner, generation, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break;
            }
        }
        // Elected. Until we return, no other thread of this process sets a
        // slot. On failure, hand the election back.
        let mut election = Election {
            owner: &self.owner,
            keep: false,
        };
        if let Some(value) = self.get() {
            // Set between our last look and the election.
            election.keep = true;
            return Ok(value);
        }
        // Only the elected thread of this process moves `next` (an
        // ancestor's moves happened before the fork), so a slot free now is
        // still free after `init`. Checking first means a full cell never
        // runs `init` (which may start a thread) only to throw it away.
        if self.next.load(Ordering::Acquire) >= SLOTS {
            return Err(unavailable(Unavailable::Full));
        }
        let value = init()?;
        // A slot no thread has touched: one a parent's thread was filling at
        // a fork was already counted in `next` before the fork.
        let i = self.next.fetch_add(1, Ordering::AcqRel);
        let Some(slot) = self.slots.get(i) else {
            return Err(unavailable(Unavailable::Full));
        };
        // An untouched slot, so this doesn't wait.
        let (_, value) = slot.get_or_init(|| (generation, value));
        self.current.store(i, Ordering::Release);
        // The process keeps the election: the cell is set for it.
        election.keep = true;
        Ok(value)
    }
}

/// Hands an election back when the elected thread fails or panics.
struct Election<'a> {
    owner: &'a AtomicU64,
    /// Set once the cell holds this process's value.
    keep: bool,
}

impl Drop for Election<'_> {
    fn drop(&mut self) {
        if !self.keep {
            self.owner.store(0, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn many_threads_init_once() {
        static CELL: ForkSafeCell<usize> = ForkSafeCell::per_process();
        static RUNS: AtomicUsize = AtomicUsize::new(0);
        let barrier = Barrier::new(8);
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    barrier.wait();
                    let v = CELL
                        .get_or_try_init(
                            || {
                                std::thread::sleep(std::time::Duration::from_millis(20));
                                Ok::<_, ()>(RUNS.fetch_add(1, Ordering::SeqCst) + 7)
                            },
                            |_| (),
                        )
                        .copied();
                    assert_eq!(v, Ok(7));
                });
            }
        });
        assert_eq!(RUNS.load(Ordering::SeqCst), 1);
        assert_eq!(CELL.get(), Some(&7));
    }

    #[test]
    fn a_failed_init_is_retried() {
        let cell = ForkSafeCell::<u8>::shared();
        assert_eq!(cell.get_or_try_init(|| Err("no"), |_| "full"), Err("no"));
        assert_eq!(cell.get(), None);
        assert_eq!(cell.get_or_try_init(|| Ok(3), |_| "full"), Ok(&3));
        assert_eq!(cell.get_or_try_init(|| Ok(4), |_| "full"), Ok(&3));
    }

    #[test]
    fn a_panicking_init_is_retried() {
        let cell = ForkSafeCell::<u8>::shared();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cell.get_or_try_init(|| -> Result<u8, ()> { panic!("init") }, |_| ())
        }));
        assert!(r.is_err());
        assert_eq!(cell.get_or_try_init(|| Ok(5), |_| ()), Ok(&5));
    }

    /// A process that inherited a cell set (or being set) by another
    /// process, simulated by planting another id: a per-process cell sets
    /// its own value in a fresh slot, a shared one keeps the old.
    #[test]
    fn a_child_holds_its_own_election() {
        // Another process's generation (an ancestor's is lower; any other
        // nonzero value is handled the same way).
        let other = fork_generation().unwrap().wrapping_add(1000);
        for per_process in [true, false] {
            let cell = ForkSafeCell::<u8>::new(per_process);
            cell.slots[0].get_or_init(|| (other, 1));
            cell.next.store(1, Ordering::SeqCst);
            // Another process's thread was mid-election at the "fork".
            cell.owner.store(other, Ordering::SeqCst);
            // And had claimed slot 1 without filling it.
            cell.next.store(2, Ordering::SeqCst);
            let want = if per_process { 2 } else { 1 };
            assert_eq!(cell.get_or_try_init(|| Ok(2), |_| ()), Ok(&want));
            if per_process {
                assert_eq!(cell.current.load(Ordering::SeqCst), 2);
            }
        }
    }

    /// Review of #47: a cell out of slots must fail before `init` runs, or
    /// every call would start (and leak) another watchdog thread.
    #[test]
    fn out_of_slots_fails_without_running_init() {
        let cell = ForkSafeCell::<u8>::per_process();
        cell.next.store(SLOTS, Ordering::SeqCst);
        let runs = AtomicUsize::new(0);
        let init = || {
            runs.fetch_add(1, Ordering::SeqCst);
            Ok(1)
        };
        let full = |why| match why {
            Unavailable::Full => "full",
            Unavailable::NoForkHandler => "no handler",
        };
        assert_eq!(cell.get_or_try_init(init, full), Err("full"));
        // The election was handed back: the next caller fails the same way.
        assert_eq!(cell.get_or_try_init(init, full), Err("full"));
        assert_eq!(runs.load(Ordering::SeqCst), 0, "init ran");
        // The last free slot is still used.
        let cell = ForkSafeCell::<u8>::per_process();
        cell.next.store(SLOTS - 1, Ordering::SeqCst);
        assert_eq!(cell.get_or_try_init(init, full), Ok(&1));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    /// A failed `init` takes no slot: the cell can fail many more times
    /// than it has slots and still be set.
    #[test]
    fn failures_take_no_slot() {
        let cell = ForkSafeCell::<u8>::per_process();
        for _ in 0..SLOTS * 2 {
            assert_eq!(cell.get_or_try_init(|| Err("no"), |_| "full"), Err("no"));
        }
        assert_eq!(cell.next.load(Ordering::SeqCst), 0);
        assert_eq!(cell.get_or_try_init(|| Ok(9), |_| "full"), Ok(&9));
    }
}
