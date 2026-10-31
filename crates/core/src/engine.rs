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
//!   the engine rather than from where the runtime was created, and
//!   [`stack_available`], which asks the OS how much of this thread's stack
//!   is left;
//! - [`patch_level`], which links against a symbol only the patched QuickJS
//!   in `vendor/rquickjs-sys` defines, so a build against an unpatched
//!   engine fails to link instead of shipping a sandbox that can hang;
//! - [`install_stop_flag`], which points the patched engine at the
//!   [`LimitState`]'s stop flag, read on every interrupt poll.
//!
//! Every `unsafe` block carries a `SAFETY:` comment and is covered by the tests
//! at the bottom of this file and in `sandbox.rs`. The module is opted in to
//! `unsafe` with `#[allow(unsafe_code)]` on its declaration in `lib.rs`.

use std::ffi::CString;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rquickjs::allocator::{Allocator, RustAllocator};
use rquickjs::{Ctx, Value, qjs};

use crate::SandboxError;
use crate::watchdog::{self, Registry, TimerKey};

/// Shared state between the sandbox, its allocator, its interrupt handler
/// and the watchdog thread.
///
/// Atomics keep this `Send + Sync`, which the watchdog needs, as does
/// rquickjs of interrupt handlers when its `parallel` feature is on.
#[derive(Debug)]
pub(crate) struct LimitState {
    /// Bytes currently allocated by the engine (usable sizes).
    used: AtomicUsize,
    /// Maximum bytes the engine may hold.
    limit: AtomicUsize,
    /// Why the running call must stop: [`STOP_TIME`] and [`STOP_MEMORY`]
    /// bits, zero while it may go on. The patched engine reads it on every
    /// interrupt poll and asks the handler as soon as it is nonzero (see
    /// [`install_stop_flag`]). Cleared when a call is armed and disarmed.
    stop: AtomicU32,
    /// Deadline in nanoseconds since `origin`; `u64::MAX` means disarmed.
    deadline: AtomicU64,
    /// Set when the sandbox replaces or drops this state's runtime; a
    /// [`Deadline`](crate::Deadline) still holding it then reports passed.
    retired: AtomicBool,
    /// Monotonic origin for the deadline. Only decides whether to abort; it
    /// never reaches script-visible output.
    origin: Instant,
    /// The running call's watchdog timer. Its lock also orders the changes
    /// to `deadline` and the `STOP_TIME` bit between this sandbox's thread
    /// and the watchdog, so a late timer can't stop the next call.
    timer: Mutex<Option<(Arc<Registry>, TimerKey)>>,
}

const DISARMED: u64 = u64::MAX;
/// [`LimitState::stop`] bit: the deadline passed.
const STOP_TIME: u32 = 1;
/// [`LimitState::stop`] bit: an allocation was refused.
const STOP_MEMORY: u32 = 2;

impl LimitState {
    pub(crate) fn new(memory_limit: usize) -> Arc<Self> {
        Arc::new(Self {
            used: AtomicUsize::new(0),
            limit: AtomicUsize::new(memory_limit),
            stop: AtomicU32::new(0),
            deadline: AtomicU64::new(DISARMED),
            retired: AtomicBool::new(false),
            origin: Instant::now(),
            timer: Mutex::new(None),
        })
    }

    /// Starts a call: clears the stop flag, sets the deadline `budget` from
    /// now and asks the watchdog to raise the flag then.
    ///
    /// Fails only if the watchdog thread can't be started (in a forked
    /// child, say); the call must not run then.
    pub(crate) fn arm(self: &Arc<Self>, budget: Duration) -> Result<(), SandboxError> {
        let registry = watchdog::registry()?;
        let mut timer = watchdog::lock(&self.timer);
        if let Some((old, key)) = timer.take() {
            old.remove(key);
        }
        let now = Instant::now();
        let since_origin = now.saturating_duration_since(self.origin);
        let deadline = since_origin.saturating_add(budget).as_nanos();
        // Saturate one below DISARMED so a huge budget still counts as armed.
        let deadline = u64::try_from(deadline)
            .unwrap_or(u64::MAX)
            .min(DISARMED - 1);
        self.stop.store(0, Ordering::Relaxed);
        self.deadline.store(deadline, Ordering::Relaxed);
        // A budget too large for an Instant never fires; the handler's own
        // clock check covers it, as it does a watchdog running late.
        if let Some(at) = now.checked_add(budget) {
            let key = registry.insert(at, Arc::downgrade(self));
            *timer = Some((registry, key));
        }
        Ok(())
    }

    /// Ends a call: no deadline and a clear stop flag until the next `arm`.
    pub(crate) fn disarm(&self) {
        let mut timer = watchdog::lock(&self.timer);
        if let Some((registry, key)) = timer.take() {
            registry.remove(key);
        }
        self.deadline.store(DISARMED, Ordering::Relaxed);
        self.stop.store(0, Ordering::Relaxed);
    }

    /// Called by the watchdog when timer `key` is due: raises the stop flag
    /// if that timer still belongs to the running call.
    pub(crate) fn expire(&self, key: TimerKey) {
        let timer = watchdog::lock(&self.timer);
        if timer.as_ref().is_some_and(|(_, k)| *k == key) {
            self.stop.fetch_or(STOP_TIME, Ordering::Release);
        }
    }

    /// Whether an allocation was refused in the running call.
    pub(crate) fn oom(&self) -> bool {
        self.stop.load(Ordering::Acquire) & STOP_MEMORY != 0
    }

    /// Whether the running call passed its deadline.
    pub(crate) fn timed_out(&self) -> bool {
        self.stop.load(Ordering::Acquire) & STOP_TIME != 0
    }

    /// Bytes currently held by the engine.
    pub(crate) fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    /// Marks this state's runtime as gone (replaced or dropped).
    pub(crate) fn retire(&self) {
        self.retired.store(true, Ordering::Relaxed);
    }

    /// Whether this state's runtime is gone.
    pub(crate) fn retired(&self) -> bool {
        self.retired.load(Ordering::Relaxed)
    }

    /// Whether the running call is past its deadline, without raising the
    /// stop flag (for `Debug`).
    pub(crate) fn deadline_passed_quietly(&self) -> bool {
        let deadline = self.deadline.load(Ordering::Relaxed);
        deadline != DISARMED && (self.timed_out() || self.elapsed_nanos() >= deadline)
    }

    /// Whether the running call is past its deadline; false when no call
    /// runs. Normally the watchdog has raised the stop flag and this is one
    /// load. If the watchdog runs late (a starved machine), the clock is
    /// checked too, and a passed deadline raises the flag here. Called by
    /// the interrupt handler, between microtasks and through
    /// [`crate::Deadline::passed`].
    pub(crate) fn deadline_passed(&self) -> bool {
        let deadline = self.deadline.load(Ordering::Relaxed);
        if deadline == DISARMED {
            return false;
        }
        if self.timed_out() {
            return true;
        }
        if self.elapsed_nanos() < deadline {
            return false;
        }
        // Under the timer lock, so the call that was checked is still the
        // one running when the flag goes up.
        let _timer = watchdog::lock(&self.timer);
        if self.deadline.load(Ordering::Relaxed) == deadline {
            self.stop.fetch_or(STOP_TIME, Ordering::Release);
        }
        true
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

    /// Records a refused allocation. Raising the stop flag makes the engine
    /// stop the script at its next poll, before it can catch the error.
    fn refuse(&self) -> bool {
        self.stop.fetch_or(STOP_MEMORY, Ordering::Release);
        false
    }

    /// Reserves room for `extra` more bytes, or records an out-of-memory error.
    fn admit(&self, extra: usize) -> bool {
        #[cfg(test)]
        if fault::refuse_this_allocation() {
            return self.refuse();
        }
        let used = self.used.load(Ordering::Relaxed);
        let limit = self.limit.load(Ordering::Relaxed);
        if used.saturating_add(extra) > limit {
            return self.refuse();
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

unsafe extern "C" {
    /// Defined only by the patched QuickJS in `vendor/rquickjs-sys`.
    fn surimono_quickjs_patch_level() -> std::ffi::c_int;
    /// Defined only by the patched QuickJS (patch level 2): every interrupt
    /// poll of `rt`'s contexts also loads `*flag` (atomically, relaxed) and
    /// calls the interrupt handler while it is nonzero.
    fn surimono_set_interrupt_flag(rt: *mut qjs::JSRuntime, flag: *mut u32);
}

/// Makes every interrupt poll of `ctx`'s runtime read `state`'s stop flag,
/// so a passed deadline or a refused allocation stops the script at its
/// next poll instead of up to 10,000 polls later.
pub(crate) fn install_stop_flag(ctx: &Ctx<'_>, state: &Arc<LimitState>) {
    let flag = state.stop.as_ptr();
    // SAFETY: `ctx` is a live context and we are inside `Context::with`, so
    // the runtime is locked; the call only writes the flag pointer into the
    // runtime and its contexts. The pointer stays valid as long as the
    // runtime polls: it points into `state`, an `Arc` that the runtime's
    // `LimitAllocator` also owns, and rquickjs drops the allocator only
    // after `JS_FreeRuntime` has returned. `AtomicU32` has the same layout
    // as `u32` (and C11's `_Atomic uint32_t` on our targets); the engine
    // only ever loads it atomically, and Rust writes it atomically.
    unsafe {
        let rt = qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
        surimono_set_interrupt_flag(rt, flag);
    }
}

/// The level of surimono's QuickJS patches in the linked engine (see
/// `vendor/rquickjs-sys/PATCHES.md`). Calling it makes the library refuse
/// to link against an unpatched rquickjs-sys.
pub(crate) fn patch_level() -> i32 {
    // SAFETY: a C function with no arguments that returns a constant.
    unsafe { surimono_quickjs_patch_level() }
}

/// Bytes of stack left on this thread below the current frame, if the
/// platform can tell (Linux and macOS); `None` elsewhere.
pub(crate) fn stack_available() -> Option<usize> {
    let here = std::hint::black_box(0u8);
    let sp = std::ptr::addr_of!(here) as usize;
    STACK_LOW.with(|low| low.map(|low| sp.saturating_sub(low)))
}

thread_local! {
    /// Lowest address of this thread's stack, looked up once per thread.
    static STACK_LOW: Option<usize> = stack_low();
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn stack_low() -> Option<usize> {
    let mut attr = std::mem::MaybeUninit::<libc::pthread_attr_t>::uninit();
    // SAFETY: pthread_getattr_np initialises `attr` for the calling thread
    // (glibc and musl both support it, the main thread included); we read it
    // only after it reports success and destroy it exactly once.
    unsafe {
        if libc::pthread_getattr_np(libc::pthread_self(), attr.as_mut_ptr()) != 0 {
            return None;
        }
        let mut addr: *mut libc::c_void = ptr::null_mut();
        let mut size: libc::size_t = 0;
        let ok = libc::pthread_attr_getstack(attr.as_ptr(), &mut addr, &mut size) == 0;
        libc::pthread_attr_destroy(attr.as_mut_ptr());
        if !ok || addr.is_null() {
            return None;
        }
        // `addr` is the lowest address of the stack mapping.
        let low = addr as usize;
        #[cfg(target_env = "musl")]
        let low = musl_main_thread_low(low, low.saturating_add(size)).unwrap_or(low);
        Some(low)
    }
}

/// musl's `pthread_getattr_np` reports only the part of the main thread's
/// stack that is mapped so far, not how far it can grow, so the stack check
/// would refuse sandboxes the thread has room for. The main thread's stack
/// grows down to `RLIMIT_STACK` below its top. With an unlimited rlimit
/// there is no bound to trust, so the mapped part is kept (refusing is safer
/// than overrunning). `None` off the main thread.
#[cfg(all(any(target_os = "linux", target_os = "android"), target_env = "musl"))]
fn musl_main_thread_low(mapped_low: usize, top: usize) -> Option<usize> {
    // SAFETY: getpid and the gettid system call take no arguments and
    // can't fail; the thread id of the main thread equals the process id.
    let main = unsafe { libc::syscall(libc::SYS_gettid) == libc::c_long::from(libc::getpid()) };
    if !main {
        return None;
    }
    let mut lim = std::mem::MaybeUninit::<libc::rlimit>::uninit();
    // SAFETY: getrlimit writes `lim` when it returns 0; it is read only then.
    let lim = unsafe {
        if libc::getrlimit(libc::RLIMIT_STACK, lim.as_mut_ptr()) != 0 {
            return None;
        }
        lim.assume_init()
    };
    if lim.rlim_cur == libc::RLIM_INFINITY {
        return None;
    }
    let size = usize::try_from(lim.rlim_cur).ok()?;
    Some(top.checked_sub(size)?.min(mapped_low))
}

#[cfg(target_os = "macos")]
fn stack_low() -> Option<usize> {
    // SAFETY: both functions only read the calling thread's descriptor;
    // pthread_get_stackaddr_np returns the stack's highest address.
    unsafe {
        let me = libc::pthread_self();
        let top = libc::pthread_get_stackaddr_np(me) as usize;
        let size = libc::pthread_get_stacksize_np(me);
        top.checked_sub(size)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
fn stack_low() -> Option<usize> {
    None
}

/// Test-only fault injection: refuse exactly the Nth allocation, to show no
/// out-of-memory path in QuickJS crashes or aborts the process.
#[cfg(test)]
pub(crate) mod fault {
    use std::ffi::c_int;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use rquickjs::{Ctx, qjs};

    static REFUSE_AT: AtomicUsize = AtomicUsize::new(0);
    static COUNT: AtomicUsize = AtomicUsize::new(0);

    /// Refuse the `n`th allocation from now on (1-based); 0 switches it off.
    /// Process-wide: use it only in a child process running one test.
    pub(crate) fn refuse_nth(n: usize) {
        COUNT.store(0, Ordering::SeqCst);
        REFUSE_AT.store(n, Ordering::SeqCst);
    }

    /// Allocations counted since the last `refuse_nth`.
    pub(crate) fn count() -> usize {
        COUNT.load(Ordering::SeqCst)
    }

    pub(super) fn refuse_this_allocation() -> bool {
        let n = REFUSE_AT.load(Ordering::Relaxed);
        let i = COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        n != 0 && i == n
    }

    static ARENA_OFF: AtomicBool = AtomicBool::new(false);

    /// With `off`, runtimes created from now on skip QuickJS's small-block
    /// arenas, so every allocation reaches the sandbox's allocator and
    /// `refuse_nth` can refuse each one. Normally only arena refills get
    /// there. Process-wide: use it only in a child process.
    pub(crate) fn set_arena_off(off: bool) {
        ARENA_OFF.store(off, Ordering::SeqCst);
    }

    unsafe extern "C" {
        /// Defined only by the patched QuickJS in `vendor/rquickjs-sys`.
        fn surimono_quickjs_set_arena(rt: *mut qjs::JSRuntime, enabled: c_int);
    }

    /// Applies `set_arena_off` to the runtime of `ctx`.
    pub(crate) fn apply_arena_setting(ctx: &Ctx<'_>) {
        if ARENA_OFF.load(Ordering::SeqCst) {
            // SAFETY: `ctx` is a live context inside `Context::with`, so its
            // runtime is alive and locked; the call writes one field of it.
            unsafe { surimono_quickjs_set_arena(qjs::JS_GetRuntime(ctx.as_raw().as_ptr()), 0) }
        }
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
    // The job threw, and its exception is pending on the context it was
    // queued from. QuickJS's job entries keep a raw context pointer without
    // a reference, so `job_ctx` is only compared, never dereferenced. The
    // sandbox keeps one context per runtime, so it is always `ctx`.
    debug_assert!(
        job_ctx == raw,
        "a job ran in a context the sandbox doesn't own"
    );
    if job_ctx != raw {
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
        state.arm(Duration::ZERO).unwrap();
        assert!(state.should_interrupt());
        assert!(state.timed_out());
        state.disarm();
        assert!(!state.deadline_passed());
        assert!(!state.timed_out(), "disarm clears the flag");
        state.arm(Duration::MAX).unwrap();
        assert!(!state.should_interrupt());
        assert!(!state.timed_out(), "arm clears the flag");
        state.disarm();
    }

    /// The watchdog raises the stop flag at the deadline without anyone
    /// polling the clock, and only for the call that armed it.
    #[test]
    fn watchdog_raises_the_flag_at_the_deadline() {
        let state = LimitState::new(usize::MAX);
        let start = Instant::now();
        state.arm(Duration::from_millis(50)).unwrap();
        while state.stop.load(Ordering::Acquire) == 0 {
            assert!(start.elapsed() < Duration::from_secs(5), "never raised");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(start.elapsed() >= Duration::from_millis(50));
        assert!(state.timed_out());
        state.disarm();
        assert_eq!(state.stop.load(Ordering::Acquire), 0);

        // A call that ends before its deadline leaves no timer behind, and
        // the next call is not stopped by the old deadline.
        state.arm(Duration::from_millis(30)).unwrap();
        state.disarm();
        state.arm(Duration::from_secs(60)).unwrap();
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(state.stop.load(Ordering::Acquire), 0);
        assert!(!state.deadline_passed());
        state.disarm();
        assert!(state.timer.lock().unwrap().is_none());
    }

    /// Many sandboxes' calls share the one watchdog thread, and every timer
    /// leaves the registry when its call ends.
    #[test]
    fn one_watchdog_serves_every_sandbox() {
        let states: Vec<_> = (0..64).map(|_| LimitState::new(usize::MAX)).collect();
        for (i, s) in states.iter().enumerate() {
            s.arm(Duration::from_millis(20 + i as u64)).unwrap();
        }
        let start = Instant::now();
        while !states.iter().all(|s| s.timed_out()) {
            assert!(start.elapsed() < Duration::from_secs(5), "not all raised");
            std::thread::sleep(Duration::from_millis(1));
        }
        for s in &states {
            s.disarm();
        }
        let registry = watchdog::registry().unwrap();
        assert!(Arc::ptr_eq(&registry, &watchdog::registry().unwrap()));
        // Arming and disarming leaves nothing behind. (Other tests' calls
        // may be running, so allow for theirs.)
        for _ in 0..10_000 {
            states[0].arm(Duration::from_secs(60)).unwrap();
            states[0].disarm();
        }
        assert!(registry.pending() < 100, "{} timers", registry.pending());
        // One watchdog thread in the process (Linux names it in /proc).
        if let Ok(tasks) = std::fs::read_dir("/proc/self/task") {
            let watchdogs = tasks
                .filter_map(|t| std::fs::read_to_string(t.ok()?.path().join("comm")).ok())
                .filter(|name| name.starts_with("surimono-watch"))
                .count();
            assert_eq!(watchdogs, 1);
        }
    }

    #[test]
    fn a_refused_allocation_raises_the_flag() {
        let state = LimitState::new(64);
        state.arm(Duration::from_secs(60)).unwrap();
        let mut a = LimitAllocator::new(state.clone());
        assert!(a.alloc(1000).is_null());
        assert!(state.oom());
        assert_ne!(state.stop.load(Ordering::Acquire), 0);
        assert!(state.should_interrupt());
        state.disarm();
        assert!(!state.oom());
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
    fn the_engine_is_patched() {
        assert!(patch_level() >= 2);
    }

    #[test]
    fn stack_available_is_plausible() {
        if !cfg!(any(target_os = "linux", target_os = "macos")) {
            return;
        }
        // A thread we size ourselves reports at least what we asked for.
        let avail = std::thread::Builder::new()
            .stack_size(4 * 1024 * 1024)
            .spawn(stack_available)
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
        // glibc may reuse a cached stack up to 4x the requested size.
        assert!(
            (3 * 1024 * 1024..=17 * 1024 * 1024).contains(&avail),
            "{avail}"
        );
        // Deeper frames see less.
        fn deeper(n: usize) -> usize {
            let pad = std::hint::black_box([0u8; 4096]);
            if n == 0 {
                stack_available().unwrap() + usize::from(pad[0])
            } else {
                deeper(n - 1)
            }
        }
        assert!(deeper(16) + 16 * 4096 <= stack_available().unwrap());
    }

    #[test]
    fn update_stack_top_is_harmless() {
        with_ctx(|ctx| {
            update_stack_top(&ctx);
            assert_eq!(ctx.eval::<i32, _>("1 + 1").unwrap(), 2);
        });
    }
}
