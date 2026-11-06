//! Threads a forked child can always start.
//!
//! `std::thread` keeps one process-wide lock for its stack-overflow handler
//! (`std::sys::pal::unix::stack_overflow::thread_info`, seen in Rust 1.89 and
//! 1.101). Every thread it starts takes that lock on its way out, and in a
//! program with a Rust `main` also on its first line, before any of our code.
//! If a thread of the parent holds it at a `fork`, it stays held in the child:
//! a new `std::thread` there never starts, or never ends, so whoever waits for
//! it waits for good. No caller can reach that lock or reset it.
//!
//! So the threads the sandbox and the island runner start in order to work
//! come from here: a plain `pthread_create`, which runs none of that. What
//! they give up is std's "has overflowed its stack" message before the
//! `SIGSEGV`; the stack limits keep scripts away from that case anyway. And
//! they never take the lock, so a process with no other `std::thread`, as a
//! Ruby server with the gem is, can't have it held at a fork.
//! `std::thread::current` still works in them, and thread-locals run their
//! destructors. A panic is caught at the bottom of the thread, as std does,
//! and ends only that thread.
//!
//! Off Unix there is no `fork`, and this is `std::thread`.

use std::ffi::CStr;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A running thread from [`spawn_host_thread`]. Dropping it detaches the
/// thread, as dropping a `JoinHandle` does.
#[derive(Debug)]
pub struct HostThread {
    #[cfg(unix)]
    thread: Option<libc::pthread_t>,
    #[cfg(not(unix))]
    thread: std::thread::JoinHandle<()>,
    finished: Arc<AtomicBool>,
}

#[cfg(unix)]
// SAFETY: a `pthread_t` is an identifier of a thread, valid from any thread
// (on some platforms it is a pointer, which is why this isn't automatic), and
// `HostThread` hands it to `pthread_join` or `pthread_detach` exactly once.
unsafe impl Send for HostThread {}

#[cfg(unix)]
// SAFETY: `&self` methods only read the atomic; the `pthread_t` is used only
// through `self` by value or in `drop`.
unsafe impl Sync for HostThread {}

impl HostThread {
    /// Whether the thread's function has returned or panicked.
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// Waits for the thread to end.
    #[cfg_attr(not(unix), allow(unused_mut))]
    pub fn join(mut self) {
        #[cfg(unix)]
        if let Some(thread) = self.thread.take() {
            // SAFETY: `thread` came from `pthread_create`, isn't detached
            // (that happens only in `drop`, and `take` keeps `drop` from it)
            // and is joined only here. The null asks for no return value.
            unsafe { libc::pthread_join(thread, std::ptr::null_mut()) };
        }
        #[cfg(not(unix))]
        let _ = self.thread.join();
    }
}

#[cfg(unix)]
impl Drop for HostThread {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            // SAFETY: as in `join`: a thread not yet joined or detached.
            unsafe { libc::pthread_detach(thread) };
        }
    }
}

/// Starts a thread running `f`, with at least `stack_size` bytes of stack.
/// See the module docs for why this isn't `std::thread`. `name`, of at most
/// 15 bytes (Linux's limit), is what the system calls the thread;
/// `std::thread::current().name()` is `None` there.
#[doc(hidden)]
pub fn spawn_host_thread(
    name: &'static CStr,
    stack_size: usize,
    f: impl FnOnce() + Send + 'static,
) -> io::Result<HostThread> {
    let finished = Arc::new(AtomicBool::new(false));
    let body = ThreadBody {
        name,
        f: Box::new(f),
        finished: Finished(finished.clone()),
    };
    #[cfg(unix)]
    {
        let thread = pthread_spawn(stack_size, body)?;
        Ok(HostThread {
            thread: Some(thread),
            finished,
        })
    }
    #[cfg(not(unix))]
    {
        let thread = std::thread::Builder::new()
            .stack_size(stack_size)
            .spawn(move || body.run())?;
        Ok(HostThread { thread, finished })
    }
}

/// What a thread runs.
struct ThreadBody {
    name: &'static CStr,
    f: Box<dyn FnOnce() + Send>,
    finished: Finished,
}

impl ThreadBody {
    fn run(self) {
        let Self { name, f, finished } = self;
        set_name(name);
        // A panic has printed its message by now, and the thread's own guards
        // have done their work on the way here. It stops at this frame:
        // unwinding out of `start` would abort the process.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        drop(finished);
    }
}

/// Names the calling thread, where the system has names. A name it refuses
/// changes nothing that matters.
fn set_name(name: &CStr) {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    // SAFETY: `pthread_self` is this thread, and `name` is a NUL-terminated
    // string that the call only reads.
    unsafe {
        libc::pthread_setname_np(libc::pthread_self(), name.as_ptr());
    }
    #[cfg(target_os = "macos")]
    // SAFETY: `name` is a NUL-terminated string that the call only reads.
    unsafe {
        libc::pthread_setname_np(name.as_ptr());
    }
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    let _ = name;
}

/// Marks the thread finished, however its function ended.
struct Finished(Arc<AtomicBool>);

impl Drop for Finished {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Stacks are a multiple of this, and at least this: every platform takes
/// that, where some refuse a size that isn't a multiple of the page.
#[cfg(unix)]
const STACK_GRANULE: usize = 64 * 1024;

#[cfg(unix)]
fn pthread_spawn(stack_size: usize, body: ThreadBody) -> io::Result<libc::pthread_t> {
    let stack_size = stack_size
        .max(1)
        .checked_next_multiple_of(STACK_GRANULE)
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    let check = |code: libc::c_int| match code {
        0 => Ok(()),
        code => Err(io::Error::from_raw_os_error(code)),
    };
    let mut attr = std::mem::MaybeUninit::<libc::pthread_attr_t>::uninit();
    // SAFETY: initialises `attr`, which is read only after this succeeds.
    check(unsafe { libc::pthread_attr_init(attr.as_mut_ptr()) })?;
    let mut thread = std::mem::MaybeUninit::<libc::pthread_t>::uninit();
    let arg = Box::into_raw(Box::new(body));
    // SAFETY: `attr` is initialised. `pthread_create` copies what it needs of
    // it, so it can be destroyed right after, exactly once, on every path.
    // `start` has the signature the call asks for, and on success it owns
    // `arg`.
    let created = unsafe {
        let sized = check(libc::pthread_attr_setstacksize(
            attr.as_mut_ptr(),
            stack_size,
        ));
        let created = sized.and_then(|()| {
            check(libc::pthread_create(
                thread.as_mut_ptr(),
                attr.as_ptr(),
                start,
                arg.cast(),
            ))
        });
        libc::pthread_attr_destroy(attr.as_mut_ptr());
        created
    };
    match created {
        // SAFETY: a successful `pthread_create` wrote `thread`.
        Ok(()) => Ok(unsafe { thread.assume_init() }),
        Err(e) => {
            // SAFETY: no thread started, so `arg` is still ours, and it came
            // from `Box::into_raw` above.
            drop(unsafe { Box::from_raw(arg) });
            Err(e)
        }
    }
}

/// The new thread's first frame.
#[cfg(unix)]
extern "C" fn start(arg: *mut libc::c_void) -> *mut libc::c_void {
    // SAFETY: `pthread_spawn` passes a pointer from `Box::into_raw` and, once
    // the thread exists, never uses it again.
    let body = unsafe { Box::from_raw(arg.cast::<ThreadBody>()) };
    body.run();
    std::ptr::null_mut()
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;

    const NAME: &CStr = c"surimono-test";

    #[test]
    fn it_runs_and_joins() {
        let (tx, rx) = mpsc::channel();
        let thread = spawn_host_thread(NAME, 256 * 1024, move || tx.send(7).unwrap()).unwrap();
        assert_eq!(rx.recv(), Ok(7));
        thread.join();
    }

    #[test]
    fn it_reports_when_it_has_finished() {
        let (tx, rx) = mpsc::channel::<()>();
        let thread = spawn_host_thread(NAME, 256 * 1024, move || {
            let _ = rx.recv();
        })
        .unwrap();
        assert!(!thread.is_finished());
        drop(tx);
        let until = Instant::now() + Duration::from_secs(30);
        while !thread.is_finished() {
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        thread.join();
    }

    #[test]
    fn a_panic_ends_only_the_thread() {
        let thread = spawn_host_thread(NAME, 256 * 1024, || panic!("on purpose")).unwrap();
        let until = Instant::now() + Duration::from_secs(30);
        while !thread.is_finished() {
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        thread.join();
    }

    #[test]
    fn the_stack_is_at_least_what_was_asked_for() {
        const SIZE: usize = 1024 * 1024 + 1;
        let (tx, rx) = mpsc::channel();
        let thread = spawn_host_thread(NAME, SIZE, move || {
            tx.send(crate::engine::stack_available()).unwrap();
        })
        .unwrap();
        if let Some(left) = rx.recv().unwrap() {
            assert!(left + 64 * 1024 >= SIZE, "{left}");
        }
        thread.join();
    }

    /// Dropping the handle detaches: the thread goes on, and nothing waits.
    #[test]
    fn dropping_the_handle_detaches() {
        let (tx, rx) = mpsc::channel();
        let (go, wait) = mpsc::channel::<()>();
        drop(
            spawn_host_thread(NAME, 256 * 1024, move || {
                let _ = wait.recv();
                tx.send(1).unwrap();
            })
            .unwrap(),
        );
        go.send(()).unwrap();
        assert_eq!(rx.recv(), Ok(1));
    }

    /// `/proc` shows it, as `one_watchdog_serves_every_sandbox` relies on.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_system_sees_its_name() {
        let (tx, rx) = mpsc::channel();
        let thread = spawn_host_thread(NAME, 256 * 1024, move || {
            let comm = std::fs::read_to_string("/proc/thread-self/comm");
            tx.send(comm.unwrap()).unwrap();
        })
        .unwrap();
        assert_eq!(rx.recv().unwrap(), "surimono-test\n");
        thread.join();
    }

    /// `usize::MAX / 2` rounds, and the kernel refuses it: `pthread_create`
    /// fails, so `pthread_spawn` takes its body back and frees it (the count
    /// shows it did, ASan that it did so once). `usize::MAX - 1024` stops
    /// earlier, at the rounding.
    #[test]
    fn a_stack_too_large_for_the_address_space_is_an_error() {
        for (size, code) in [
            (usize::MAX / 2, libc::EAGAIN),
            (usize::MAX - 1024, libc::EINVAL),
        ] {
            let ran = Arc::new(AtomicBool::new(false));
            let flag = ran.clone();
            let spawned = spawn_host_thread(NAME, size, move || {
                flag.store(true, Ordering::SeqCst);
            });
            let error = spawned.unwrap_err();
            #[cfg(target_os = "linux")]
            assert_eq!(error.raw_os_error(), Some(code), "{size}");
            #[cfg(not(target_os = "linux"))]
            let _ = (error, code);
            assert!(!ran.load(Ordering::SeqCst));
            assert_eq!(Arc::strong_count(&ran), 1, "{size}");
        }
    }
}
