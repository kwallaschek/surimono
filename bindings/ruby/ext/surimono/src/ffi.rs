//! The extension's only unsafe code: releasing the GVL around a render (with
//! an unblock function that sets a cancel flag), and copying a Ruby string's
//! bytes.
//!
//! Covered by bindings/ruby/test: `test_other_threads_run_while_rendering`
//! (the GVL really is released), `test_interrupted_renders_raise_and_free_memory`
//! and `test_interrupt_around_renderer_new` (an interrupt never jumps over Rust
//! frames), `test_timeout_cancels_the_rest_of_the_document`,
//! `test_signal_cancels_the_rest_of_the_document` and
//! `test_wakeup_renders_again` (the unblock function), `test_fonts_from_paths_and_data`
//! (byte copies), and the error tests (errors come back through the GVL
//! boundary). `test_panic_becomes_internal_error` covers the panic path.

use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use magnus::{Error, RString, Ruby};

/// Runs `work` with the GVL released, so other Ruby threads keep running.
///
/// `work` must not touch any Ruby object or call into the Ruby VM: everything it
/// needs is moved in as plain Rust data.
///
/// Returns `Err` instead of unwinding in two cases:
/// - Ruby has an interrupt for this thread (`Thread#raise`, `Thread#kill`, a
///   signal such as Ctrl-C, or `Timeout.timeout`). `rb_thread_call_without_gvl`
///   handles pending interrupts before it calls `work` and again after it takes
///   the GVL back, and handling one raises with `longjmp`. `rb_protect` (through
///   `magnus::rb_sys::protect`) stops that jump right here, so it never crosses a
///   Rust frame; the error is returned, every Rust value is dropped normally, and
///   magnus raises it again once the method returns. If the interrupt came before
///   `work` ran, `work` is dropped without running.
/// - `work` panicked. The panic is caught before it reaches the C frame and
///   becomes a `Surimono::InternalError` (kind `"internal"`), so one bad render
///   can't end the process the way magnus' `fatal` for panics would.
///
/// No unblock function is passed, so an interrupt that arrives while `work` runs
/// waits until it returns; the sandbox's time limit bounds that wait. Use
/// [`without_gvl_cancellable`] for work that can stop early.
pub(crate) fn without_gvl<T, F>(ruby: &Ruby, work: F) -> Result<T, Error>
where
    F: FnOnce() -> T,
{
    nogvl(ruby, work, None)
}

/// [`without_gvl`], plus an unblock function that sets `cancel` when Ruby
/// interrupts this thread while `work` runs (`Thread#raise`, `Timeout`,
/// `Thread#kill`, a signal, also `Thread#wakeup`). `work` should check the
/// flag and return early. Afterwards Ruby raises a pending interrupt as
/// usual (returned as `Err` here); if there is none, `work`'s result comes
/// back, so the caller decides what an early return without an exception
/// means.
pub(crate) fn without_gvl_cancellable<T, F>(
    ruby: &Ruby,
    cancel: &AtomicBool,
    work: F,
) -> Result<T, Error>
where
    F: FnOnce() -> T,
{
    nogvl(ruby, work, Some(cancel))
}

/// The unblock function: raises the cancel flag `data` points to. It runs on
/// another thread, so it does one atomic store and nothing else: no
/// allocation, no Ruby call, no panic.
///
/// `RB_NOGVL_UBF_ASYNC_SAFE` is deliberately not used: with it, Ruby's signal
/// handler calls the main thread's unblock function without the interrupt
/// lock, on whichever thread took the signal (possibly one of the runner's
/// workers), which could race with the call returning and the flag going
/// away. Without it Ruby only calls it from `rb_threadptr_interrupt`, under
/// that lock (signals reach it through the timer thread, or on Ruby 3.2 a
/// helper thread for a single-threaded main thread).
unsafe extern "C" fn unblock(data: *mut c_void) {
    // SAFETY: `data` is the `&AtomicBool` given to `nogvl`, which outlives
    // the rb_thread_call_without_gvl call. Ruby calls the unblock function
    // only under this thread's interrupt lock while the thread is in the
    // blocking region, and blocking_region_end clears the function under the
    // same lock before rb_thread_call_without_gvl returns.
    let flag = unsafe { &*data.cast::<AtomicBool>() };
    flag.store(true, Ordering::Release);
}

fn nogvl<T, F>(ruby: &Ruby, work: F, cancel: Option<&AtomicBool>) -> Result<T, Error>
where
    F: FnOnce() -> T,
{
    struct Call<F, T> {
        work: Option<F>,
        result: Option<std::thread::Result<T>>,
    }

    unsafe extern "C" fn trampoline<F, T>(data: *mut c_void) -> *mut c_void
    where
        F: FnOnce() -> T,
    {
        // SAFETY: `data` is the `*mut Call<F, T>` made below from a local that
        // outlives this call: the calling thread is blocked inside
        // rb_thread_call_without_gvl (inside rb_protect) until this function
        // returns, and nothing else reads or writes the `Call` meanwhile.
        let call = unsafe { &mut *data.cast::<Call<F, T>>() };
        if let Some(work) = call.work.take() {
            // Unwinding out of an `extern "C"` function aborts the process, so
            // the panic is caught here and reported through `result`.
            call.result = Some(panic::catch_unwind(AssertUnwindSafe(work)));
        }
        ptr::null_mut()
    }

    let mut call = Call {
        work: Some(work),
        result: None,
    };
    let data = (&mut call as *mut Call<F, T>).cast::<c_void>();
    let (ubf, ubf_data): (rb_sys::rb_unblock_function_t, *mut c_void) = match cancel {
        Some(flag) => (
            Some(unblock),
            ptr::from_ref(flag).cast_mut().cast::<c_void>(),
        ),
        None => (None, ptr::null_mut()),
    };
    // The closure captures only raw pointers and plain values, so if Ruby
    // jumps out of it there is nothing in its frame to drop.
    let protected = magnus::rb_sys::protect(|| {
        // SAFETY: we are on a Ruby thread holding the GVL (every method Ruby
        // dispatches to us is). rb_thread_call_without_gvl calls `trampoline`
        // at most once, on this thread, with `data`, and returns only after
        // it has finished. It may also raise (longjmp) before or
        // after that call to deliver a pending interrupt; rb_protect, which
        // `protect` runs this closure under, catches the jump in a C frame
        // between here and `nogvl`, so it skips no Rust frame that has
        // anything to drop. `trampoline` runs only `work`, which by contract
        // doesn't touch the VM. `ubf_data` is null or the cancel flag, which
        // outlives this call (see `unblock`).
        unsafe {
            rb_sys::rb_thread_call_without_gvl(Some(trampoline::<F, T>), data, ubf, ubf_data);
        }
        rb_sys::Qnil as rb_sys::VALUE
    });
    // `call` is a plain local again: on every path below its contents (the
    // result, or the work that never ran) are dropped normally.
    let result = call.result.take();
    drop(call);
    protected?;
    match result {
        Some(Ok(value)) => Ok(value),
        Some(Err(payload)) => Err(internal_error(ruby, &panic_message(&*payload))),
        // rb_thread_call_without_gvl returned normally without calling us. It
        // only skips `func` by raising a pending interrupt (caught above), so
        // that doesn't happen, but if it ever did it is an error, not
        // undefined behaviour or a panic.
        None => Err(internal_error(
            ruby,
            "the render did not run (rb_thread_call_without_gvl skipped it)",
        )),
    }
}

/// The text of a panic payload.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_owned()
    }
}

/// A `Surimono::InternalError` (kind `"internal"`) for a bug in surimono.
pub(crate) fn internal_error(ruby: &Ruby, what: &str) -> Error {
    crate::surimono_error(
        ruby,
        "InternalError",
        &format!("internal error in surimono (please report it): {what}"),
        "internal",
        |_| Ok(()),
    )
}

/// Copies a Ruby string's bytes (any encoding) into Rust.
pub(crate) fn string_bytes(s: RString) -> Vec<u8> {
    // SAFETY: the slice borrows the string's buffer only until `to_vec` returns;
    // no Ruby code runs in between, so the buffer can't move or be freed, and we
    // hold the GVL (we're called from a Ruby method).
    unsafe { s.as_slice() }.to_vec()
}
