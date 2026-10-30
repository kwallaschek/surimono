//! The extension's only unsafe code: releasing the GVL around a render, and
//! copying a Ruby string's bytes.
//!
//! Covered by bindings/ruby/test: `test_other_threads_run_while_rendering`
//! (the GVL really is released), `test_fonts_from_paths_and_data` (byte copies),
//! and the error tests (panics and errors come back through the GVL boundary).

use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::ptr;

use magnus::RString;

/// Runs `work` with the GVL released, so other Ruby threads keep running.
///
/// `work` must not touch any Ruby object or call into the Ruby VM: everything it
/// needs is moved in as plain Rust data. A panic inside `work` is caught before
/// it reaches the C frame and resumed here, after the GVL is back.
///
/// No unblock function is passed, so `Thread#raise` and `Thread#kill` take
/// effect when `work` returns; every render is bounded by the sandbox's time
/// limit, so that wait is bounded too.
pub(crate) fn without_gvl<T, F>(work: F) -> T
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
        // SAFETY: `data` is the `&mut Call<F, T>` passed below. It lives on the
        // calling thread's stack, which is blocked in rb_thread_call_without_gvl
        // until this function returns, and nothing else points to it.
        let call = unsafe { &mut *data.cast::<Call<F, T>>() };
        if let Some(work) = call.work.take() {
            call.result = Some(panic::catch_unwind(AssertUnwindSafe(work)));
        }
        ptr::null_mut()
    }

    let mut call = Call {
        work: Some(work),
        result: None,
    };
    // SAFETY: rb_thread_call_without_gvl must be called from a Ruby thread holding
    // the GVL, which every method Ruby dispatches to us is. It calls `trampoline`
    // exactly once on this thread with our pointer, and returns only after it has
    // finished; with no unblock function it never skips the call. `trampoline`
    // only runs Rust code that doesn't touch the VM (the contract above).
    unsafe {
        rb_sys::rb_thread_call_without_gvl(
            Some(trampoline::<F, T>),
            (&mut call as *mut Call<F, T>).cast::<c_void>(),
            None,
            ptr::null_mut(),
        );
    }
    match call.result {
        Some(Ok(value)) => value,
        Some(Err(payload)) => panic::resume_unwind(payload),
        // Unreachable: see the SAFETY comment. Panicking here is caught by magnus
        // and raised as a Ruby exception, never undefined behaviour.
        None => panic!("rb_thread_call_without_gvl returned without running the render"),
    }
}

/// Copies a Ruby string's bytes (any encoding) into Rust.
pub(crate) fn string_bytes(s: RString) -> Vec<u8> {
    // SAFETY: the slice borrows the string's buffer only until `to_vec` returns;
    // no Ruby code runs in between, so the buffer can't move or be freed, and we
    // hold the GVL (we're called from a Ruby method).
    unsafe { s.as_slice() }.to_vec()
}
