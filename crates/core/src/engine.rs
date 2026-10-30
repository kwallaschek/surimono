//! Engine-binding module: the only place in surimono-core with `unsafe` code.
//!
//! It holds the pieces rquickjs does not expose safely:
//!
//! - [`LimitAllocator`], the heap allocator that enforces the memory limit and
//!   remembers that an allocation was refused, so an out-of-memory error stays
//!   an out-of-memory error even when the script catches it;
//! - script bytecode: compile a classic script once ([`compile_script`]) and
//!   evaluate it in any context of any runtime ([`eval_bytecode`]);
//! - [`run_pending_job`], which runs one microtask and leaves its exception on
//!   the context instead of swallowing it;
//! - [`update_stack_top`], so the stack limit is measured from where we enter
//!   the engine rather than from where the runtime was created.
//!
//! Every `unsafe` block carries a `SAFETY:` comment and is covered by the tests
//! at the bottom of this file and in `sandbox.rs`. The module is opted in to
//! `unsafe` with `#[allow(unsafe_code)]` on its declaration in `lib.rs`.

use std::ffi::CString;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rquickjs::allocator::{Allocator, RustAllocator};
use rquickjs::{Ctx, Value, qjs};

/// Shared state between the sandbox, its allocator and its interrupt handler.
///
/// Atomics keep this `Send + Sync`, which rquickjs requires of interrupt
/// handlers when its `parallel` feature is on; without it they cost nothing.
#[derive(Debug)]
pub(crate) struct LimitState {
    /// Bytes currently allocated by the engine (usable sizes).
    used: AtomicUsize,
    /// Maximum bytes the engine may hold.
    limit: AtomicUsize,
    /// Set when an allocation was refused; cleared at the start of each call.
    oom: AtomicBool,
    /// Set when the deadline passed; cleared at the start of each call.
    timed_out: AtomicBool,
    /// Deadline in nanoseconds since `origin`; `u64::MAX` means disarmed.
    deadline: AtomicU64,
    /// Monotonic origin for the deadline. Only decides whether to abort; it
    /// never reaches script-visible output.
    origin: Instant,
}

const DISARMED: u64 = u64::MAX;

impl LimitState {
    pub(crate) fn new(memory_limit: usize) -> Arc<Self> {
        Arc::new(Self {
            used: AtomicUsize::new(0),
            limit: AtomicUsize::new(memory_limit),
            oom: AtomicBool::new(false),
            timed_out: AtomicBool::new(false),
            deadline: AtomicU64::new(DISARMED),
            origin: Instant::now(),
        })
    }

    /// Starts a call: clears the flags and sets the deadline `budget` from now.
    pub(crate) fn arm(&self, budget: Duration) {
        self.oom.store(false, Ordering::Relaxed);
        self.timed_out.store(false, Ordering::Relaxed);
        let now = self.elapsed_nanos();
        let budget = u64::try_from(budget.as_nanos()).unwrap_or(u64::MAX);
        // Saturate one below DISARMED so a huge budget still counts as armed.
        let deadline = now.saturating_add(budget).min(DISARMED - 1);
        self.deadline.store(deadline, Ordering::Relaxed);
    }

    /// Ends a call: no deadline until the next `arm`.
    pub(crate) fn disarm(&self) {
        self.deadline.store(DISARMED, Ordering::Relaxed);
    }

    /// Whether an allocation was refused since the last `arm`.
    pub(crate) fn oom(&self) -> bool {
        self.oom.load(Ordering::Relaxed)
    }

    /// Whether the deadline passed since the last `arm`.
    pub(crate) fn timed_out(&self) -> bool {
        self.timed_out.load(Ordering::Relaxed)
    }

    /// Bytes currently held by the engine.
    pub(crate) fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    /// Checks the deadline and records a timeout. Called by the interrupt
    /// handler and between microtasks.
    pub(crate) fn deadline_passed(&self) -> bool {
        let deadline = self.deadline.load(Ordering::Relaxed);
        if deadline == DISARMED {
            return false;
        }
        if self.elapsed_nanos() >= deadline {
            self.timed_out.store(true, Ordering::Relaxed);
            return true;
        }
        false
    }

    /// The interrupt handler's answer: stop on a passed deadline, and stop at
    /// once after an out-of-memory error so a script that catches it cannot
    /// keep going.
    pub(crate) fn should_interrupt(&self) -> bool {
        self.oom() || self.deadline_passed()
    }

    fn elapsed_nanos(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(DISARMED - 1)
    }

    /// Reserves room for `extra` more bytes, or records an out-of-memory error.
    fn admit(&self, extra: usize) -> bool {
        let used = self.used.load(Ordering::Relaxed);
        let limit = self.limit.load(Ordering::Relaxed);
        if used.saturating_add(extra) > limit {
            self.oom.store(true, Ordering::Relaxed);
            return false;
        }
        true
    }

    fn add(&self, n: usize) {
        self.used.fetch_add(n, Ordering::Relaxed);
    }

    fn sub(&self, n: usize) {
        self.used.fetch_sub(n, Ordering::Relaxed);
    }
}

/// Heap allocator for one runtime: Rust's global allocator (through rquickjs's
/// [`RustAllocator`]) plus the memory limit.
///
/// QuickJS's own `JS_SetMemoryLimit` would also enforce a limit, but it throws
/// a catchable `InternalError` and leaves no trace once caught. Enforcing the
/// limit here lets the sandbox see every refusal.
pub(crate) struct LimitAllocator {
    state: Arc<LimitState>,
}

impl LimitAllocator {
    pub(crate) fn new(state: Arc<LimitState>) -> Self {
        Self { state }
    }
}

// SAFETY: every method delegates the actual memory management to
// `RustAllocator`, which upholds the trait's contract (null or a usize-aligned
// region of at least the requested size; `usable_size` matches its own header).
// This wrapper only adds bookkeeping and may return null before delegating,
// which the contract allows. It never touches memory itself.
unsafe impl Allocator for LimitAllocator {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        if !self.state.admit(size) {
            return ptr::null_mut();
        }
        let p = RustAllocator.alloc(size);
        if !p.is_null() {
            // SAFETY: `p` was just returned non-null by `RustAllocator`.
            self.state.add(unsafe { RustAllocator::usable_size(p) });
        }
        p
    }

    fn calloc(&mut self, count: usize, size: usize) -> *mut u8 {
        // RustAllocator panics on overflow; a panic here would cross the C
        // boundary, so refuse instead.
        let Some(total) = count.checked_mul(size) else {
            return ptr::null_mut();
        };
        if !self.state.admit(total) {
            return ptr::null_mut();
        }
        let p = RustAllocator.calloc(count, size);
        if !p.is_null() {
            // SAFETY: `p` was just returned non-null by `RustAllocator`.
            self.state.add(unsafe { RustAllocator::usable_size(p) });
        }
        p
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8) {
        // SAFETY: the caller guarantees `ptr` came from this allocator, and so
        // from `RustAllocator`, which wrote the header `usable_size` reads.
        let n = unsafe { RustAllocator::usable_size(ptr) };
        self.state.sub(n);
        // SAFETY: as above; `ptr` is not used again.
        unsafe { RustAllocator.dealloc(ptr) }
    }

    unsafe fn realloc(&mut self, ptr: *mut u8, new_size: usize) -> *mut u8 {
        if ptr.is_null() {
            // RustAllocator::realloc cannot take null; behave like C realloc.
            return self.alloc(new_size);
        }
        // SAFETY: the caller guarantees `ptr` came from this allocator.
        let old = unsafe { RustAllocator::usable_size(ptr) };
        if new_size > old && !self.state.admit(new_size - old) {
            // C semantics: the old block stays valid and accounted for.
            return ptr::null_mut();
        }
        // SAFETY: the caller guarantees `ptr` came from this allocator.
        let p = unsafe { RustAllocator.realloc(ptr, new_size) };
        if p.is_null() {
            return p;
        }
        // SAFETY: `p` was just returned non-null by `RustAllocator`.
        let new = unsafe { RustAllocator::usable_size(p) };
        self.state.sub(old);
        self.state.add(new);
        p
    }

    unsafe fn usable_size(ptr: *mut u8) -> usize {
        // SAFETY: the caller guarantees `ptr` came from this allocator, and so
        // from `RustAllocator`.
        unsafe { RustAllocator::usable_size(ptr) }
    }
}

/// Points the runtime's stack limit at the current stack position.
///
/// QuickJS measures its stack limit from a "stack top" captured when the
/// runtime was created. Re-capturing it whenever we enter the engine makes the
/// limit mean "this much stack below the caller", wherever the caller is.
pub(crate) fn update_stack_top(ctx: &Ctx<'_>) {
    // SAFETY: `ctx` is a live context; we are inside `Context::with`, which
    // holds the runtime lock, so no other thread touches the runtime. Both
    // calls only read the context's runtime pointer and write two fields of it.
    unsafe {
        let rt = qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
        qjs::JS_UpdateStackTop(rt);
    }
}

/// Compiles a classic (non-module) script to QuickJS bytecode without running it.
///
/// On a syntax error, returns `Err(Exception)` with the error pending on `ctx`.
pub(crate) fn compile_script(
    ctx: &Ctx<'_>,
    source: &str,
    filename: &str,
) -> rquickjs::Result<Vec<u8>> {
    // JS_Eval requires input[len] == 0.
    let mut input = Vec::with_capacity(source.len() + 1);
    input.extend_from_slice(source.as_bytes());
    input.push(0);
    let filename = CString::new(filename.replace('\0', "?"))?;
    let raw = ctx.as_raw().as_ptr();
    let flags = (qjs::JS_EVAL_TYPE_GLOBAL | qjs::JS_EVAL_FLAG_COMPILE_ONLY) as i32;
    // SAFETY: `raw` is a live context (we hold `ctx`, which keeps it alive and
    // the runtime locked); `input` is NUL-terminated and outlives the call, and
    // the length passed excludes the terminator, as JS_Eval requires.
    let func = unsafe {
        qjs::JS_Eval(
            raw,
            input.as_ptr().cast(),
            source.len() as _,
            filename.as_ptr(),
            flags,
        )
    };
    // SAFETY: inspecting the tag of a value we own is always valid.
    if unsafe { qjs::JS_IsException(func) } {
        return Err(rquickjs::Error::Exception);
    }
    // SAFETY: `func` is an owned value from this context's runtime; `Value`
    // takes over its reference and frees it on drop.
    let func = unsafe { Value::from_raw(ctx.clone(), func) };
    let mut len: qjs::size_t = 0;
    // SAFETY: `func` is alive for the duration of the call; JS_WriteObject
    // returns a buffer allocated with the context's allocator, or null with an
    // exception pending.
    let buf = unsafe {
        qjs::JS_WriteObject(
            raw,
            &mut len,
            func.as_raw(),
            qjs::JS_WRITE_OBJ_BYTECODE as i32,
        )
    };
    if buf.is_null() {
        return Err(rquickjs::Error::Exception);
    }
    let len = usize::try_from(len).map_err(|_| rquickjs::Error::Allocation)?;
    // SAFETY: JS_WriteObject returned a non-null buffer of exactly `len`
    // initialised bytes, which we copy before freeing it with the same
    // context's allocator, as its documentation requires.
    let bytes = unsafe {
        let bytes = std::slice::from_raw_parts(buf, len).to_vec();
        qjs::js_free(raw, buf.cast());
        bytes
    };
    Ok(bytes)
}

/// Evaluates bytecode produced by [`compile_script`] and returns the script's
/// completion value.
///
/// Callers must only pass bytes produced by [`compile_script`] in this process;
/// the sandbox guarantees that by keeping bytecode in a type whose bytes can't be
/// set from outside the crate.
pub(crate) fn eval_bytecode<'js>(ctx: &Ctx<'js>, bytecode: &[u8]) -> rquickjs::Result<Value<'js>> {
    let raw = ctx.as_raw().as_ptr();
    // SAFETY: `bytecode` was written by JS_WriteObject of this same QuickJS
    // build (see the function contract), so JS_ReadObject reads valid data.
    // Without JS_READ_OBJ_ROM_DATA, QuickJS copies everything it needs, so the
    // buffer need not outlive the returned function.
    let func = unsafe {
        qjs::JS_ReadObject(
            raw,
            bytecode.as_ptr(),
            bytecode.len() as _,
            qjs::JS_READ_OBJ_BYTECODE as i32,
        )
    };
    // SAFETY: inspecting the tag of a value we own is always valid.
    if unsafe { qjs::JS_IsException(func) } {
        return Err(rquickjs::Error::Exception);
    }
    // SAFETY: JS_EvalFunction takes ownership of `func` (a function bytecode
    // object from this runtime) and returns an owned value or an exception.
    let ret = unsafe { qjs::JS_EvalFunction(raw, func) };
    // SAFETY: inspecting the tag of a value we own is always valid.
    if unsafe { qjs::JS_IsException(ret) } {
        return Err(rquickjs::Error::Exception);
    }
    // SAFETY: `ret` is an owned value from this context's runtime.
    Ok(unsafe { Value::from_raw(ctx.clone(), ret) })
}

/// Runs one pending job (microtask). Returns whether a job ran.
///
/// Unlike `Ctx::execute_pending_job`, a job that throws returns
/// `Err(Exception)` with the exception left pending on `ctx`, so the caller
/// can report it.
pub(crate) fn run_pending_job(ctx: &Ctx<'_>) -> rquickjs::Result<bool> {
    let raw = ctx.as_raw().as_ptr();
    let mut job_ctx: *mut qjs::JSContext = ptr::null_mut();
    // SAFETY: we are inside `Context::with`, so the runtime is locked and
    // alive; JS_ExecutePendingJob runs at most one job and writes the job's
    // context to `job_ctx`.
    let res = unsafe {
        let rt = qjs::JS_GetRuntime(raw);
        qjs::JS_ExecutePendingJob(rt, &mut job_ctx)
    };
    if res >= 0 {
        return Ok(res > 0);
    }
    if !job_ctx.is_null() && job_ctx != raw {
        // The job belonged to another context of this runtime. The sandbox
        // never keeps two contexts alive, so this is defensive: clear the
        // exception there so it can't surface later.
        // SAFETY: `job_ctx` is the live context the job ran in (the job held a
        // reference to it); JS_GetException transfers ownership of the pending
        // exception, which JS_FreeValue releases.
        unsafe {
            let exc = qjs::JS_GetException(job_ctx);
            qjs::JS_FreeValue(job_ctx, exc);
        }
        return Err(rquickjs::Error::Unknown);
    }
    Err(rquickjs::Error::Exception)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rquickjs::{Context, Runtime};

    fn with_ctx<R>(f: impl FnOnce(Ctx<'_>) -> R) -> R {
        let state = LimitState::new(usize::MAX);
        let rt = Runtime::new_with_alloc(LimitAllocator::new(state)).unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(f)
    }

    #[test]
    fn allocator_tracks_and_limits() {
        let state = LimitState::new(usize::MAX);
        let mut a = LimitAllocator::new(state.clone());
        let p = a.alloc(100);
        assert!(!p.is_null());
        let used = state.used();
        assert!(used >= 100);
        // SAFETY: `p` came from `a`.
        let p = unsafe { a.realloc(p, 1000) };
        assert!(!p.is_null());
        assert!(state.used() >= 1000);
        // SAFETY: `p` came from `a` and is not used again.
        unsafe { a.dealloc(p) };
        assert_eq!(state.used(), 0);

        state.limit.store(64, Ordering::Relaxed);
        assert!(a.alloc(1000).is_null());
        assert!(state.oom());
        assert!(a.calloc(usize::MAX, 2).is_null(), "overflow is refused");
        let q = a.calloc(4, 8);
        assert!(!q.is_null());
        // SAFETY: `q` came from `a`; growing it past the limit fails and
        // leaves it valid, so it can still be freed.
        unsafe {
            assert!(a.realloc(q, 4096).is_null());
            a.dealloc(q);
        }
        // SAFETY: realloc(null) behaves like alloc.
        let r = unsafe { a.realloc(ptr::null_mut(), 16) };
        assert!(!r.is_null());
        // SAFETY: `r` came from `a`.
        unsafe { a.dealloc(r) };
        assert_eq!(state.used(), 0);
    }

    #[test]
    fn deadline_arms_and_disarms() {
        let state = LimitState::new(usize::MAX);
        assert!(!state.should_interrupt(), "disarmed by default");
        state.arm(Duration::ZERO);
        assert!(state.should_interrupt());
        assert!(state.timed_out());
        state.disarm();
        assert!(!state.deadline_passed());
        state.arm(Duration::MAX);
        assert!(!state.should_interrupt());
        assert!(!state.timed_out(), "arm clears the flag");
    }

    #[test]
    fn bytecode_round_trip_across_runtimes() {
        let bytes = with_ctx(|ctx| {
            compile_script(&ctx, "var answer = 40 + 2; answer", "answer.js").unwrap()
        });
        assert!(!bytes.is_empty());
        // A different runtime and context.
        let v: i32 = with_ctx(|ctx| {
            let v = eval_bytecode(&ctx, &bytes).unwrap();
            assert_eq!(ctx.globals().get::<_, i32>("answer").unwrap(), 42);
            v.as_int().unwrap()
        });
        assert_eq!(v, 42);
    }

    #[test]
    fn compile_reports_syntax_errors() {
        with_ctx(|ctx| {
            let err = compile_script(&ctx, "let = ;", "bad.js").unwrap_err();
            assert!(matches!(err, rquickjs::Error::Exception));
            let exc = ctx.catch();
            let msg: String = exc.as_object().unwrap().get("name").unwrap();
            assert_eq!(msg, "SyntaxError");
        });
    }

    #[test]
    fn compile_does_not_run_the_script() {
        with_ctx(|ctx| {
            compile_script(&ctx, "globalThis.ran = true", "x.js").unwrap();
            let ran: Option<bool> = ctx.globals().get("ran").unwrap();
            assert_eq!(ran, None);
        });
    }

    #[test]
    fn eval_bytecode_reports_exceptions() {
        with_ctx(|ctx| {
            let bytes = compile_script(&ctx, "throw new TypeError('nope')", "t.js").unwrap();
            assert!(matches!(
                eval_bytecode(&ctx, &bytes),
                Err(rquickjs::Error::Exception)
            ));
            let exc = ctx.catch();
            let msg: String = exc.as_object().unwrap().get("message").unwrap();
            assert_eq!(msg, "nope");
        });
    }

    #[test]
    fn pending_jobs_run_and_report_errors() {
        with_ctx(|ctx| {
            ctx.eval::<(), _>("globalThis.n = 0; Promise.resolve().then(() => { n += 1 })")
                .unwrap();
            assert!(run_pending_job(&ctx).unwrap());
            assert!(!run_pending_job(&ctx).unwrap());
            assert_eq!(ctx.globals().get::<_, i32>("n").unwrap(), 1);
        });
    }

    #[test]
    fn update_stack_top_is_harmless() {
        with_ctx(|ctx| {
            update_stack_top(&ctx);
            assert_eq!(ctx.eval::<i32, _>("1 + 1").unwrap(), 2);
        });
    }
}
