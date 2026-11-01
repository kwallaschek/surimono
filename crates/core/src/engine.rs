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
//!   [`stack_available`], which asks the OS where this thread's stack ends
//!   and reads the stack pointer register to see how much is left;
//! - [`patch_level`], which links against a symbol only the patched QuickJS
//!   in `vendor/rquickjs-sys` defines, so a build against an unpatched
//!   engine fails to link instead of shipping a sandbox that can hang;
//! - [`install_stop_flag`], which points the patched engine at the
//!   [`LimitState`]'s stop flag, read on every interrupt poll, and at its
//!   leak count, which `JS_FreeRuntime` fills in (see [`teardown_leaks`]);
//! - [`fork_generation`], which registers a `pthread_atfork` child handler
//!   so a forked process can tell its own state from its ancestors'.
//!
//! Every `unsafe` block carries a `SAFETY:` comment and is covered by the tests
//! at the bottom of this file and in `sandbox.rs`. The module is opted in to
//! `unsafe` with `#[allow(unsafe_code)]` on its declaration in `lib.rs`.

use std::ffi::CString;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rquickjs::allocator::{Allocator, RustAllocator};
use rquickjs::{Ctx, Value, qjs};

use crate::SandboxError;
use crate::watchdog::{self, TimerKey};

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
    /// The running call's sequence number (0 when none runs), with the
    /// [`EXPIRING`] bit set while a thread that found the deadline passed
    /// raises `STOP_TIME` for that call. Ending a call waits for that bit to
    /// clear, so a late timer can't stop the next call; no lock is shared
    /// with the watchdog, so a fork can't leave one held.
    call: AtomicU64,
    /// Fork generation ([`fork_generation`]) of the process whose watchdog
    /// holds the running call's timer. In a forked child a call never waits
    /// on an [`EXPIRING`] bit that its parent's watchdog set: that thread
    /// doesn't exist here.
    armed_gen: AtomicU64,
    /// Objects still alive when the runtime was freed, written by the
    /// patched `JS_FreeRuntime` (see [`install_stop_flag`]) and read when
    /// the allocator is dropped, after it returned.
    leaked_objects: AtomicU32,
}

/// [`LimitState::call`] bit: the stop flag is being raised for this call.
const EXPIRING: u64 = 1 << 63;

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
            call: AtomicU64::new(0),
            armed_gen: AtomicU64::new(0),
            leaked_objects: AtomicU32::new(0),
        })
    }

    /// Starts a call: clears the stop flag, sets the deadline `budget` from
    /// now and asks the watchdog to raise the flag then.
    ///
    /// Fails only if the watchdog thread can't be started (in a forked
    /// child, say); the call must not run then.
    pub(crate) fn arm(self: &Arc<Self>, budget: Duration) -> Result<(), SandboxError> {
        self.disarm();
        let registry = watchdog::registry()?;
        let seq = registry.next_seq();
        let since_origin = self.origin.elapsed();
        let deadline = since_origin.saturating_add(budget).as_nanos();
        // Saturate one below DISARMED so a huge budget still counts as armed.
        let deadline = u64::try_from(deadline)
            .unwrap_or(u64::MAX)
            .min(DISARMED - 1);
        self.stop.store(0, Ordering::Relaxed);
        self.deadline.store(deadline, Ordering::Relaxed);
        self.armed_gen
            .store(registry.generation(), Ordering::Relaxed);
        // Publishes the deadline with the call: whoever sees `seq` sees it.
        self.call.store(seq, Ordering::Release);
        // A budget too large for an Instant never fires; the handler's own
        // clock check covers it, as it does a watchdog running late.
        if let Some(key) = self.timer_key(deadline, seq) {
            registry.insert(key, Arc::downgrade(self));
        }
        Ok(())
    }

    /// The watchdog key of call `seq` with deadline `deadline`.
    fn timer_key(&self, deadline: u64, seq: u64) -> Option<TimerKey> {
        let at = self.origin.checked_add(Duration::from_nanos(deadline))?;
        Some((at, seq))
    }

    /// Ends a call: no deadline and a clear stop flag until the next `arm`.
    pub(crate) fn disarm(&self) {
        let deadline = self.deadline.load(Ordering::Relaxed);
        let ended = loop {
            let call = self.call.load(Ordering::Acquire);
            if call == 0 {
                break 0;
            }
            if call & EXPIRING != 0
                && self.armed_gen.load(Ordering::Relaxed) == fork_generation_now()
            {
                // A live thread is raising the flag for this call; it takes
                // two atomic operations.
                std::thread::yield_now();
                continue;
            }
            if self
                .call
                .compare_exchange_weak(call, 0, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break call & !EXPIRING;
            }
        };
        if ended != 0 {
            let armed_in = self.armed_gen.load(Ordering::Relaxed);
            let registry = watchdog::current().filter(|r| r.generation() == armed_in);
            if let (Some(registry), Some(key)) = (registry, self.timer_key(deadline, ended)) {
                registry.remove(key);
            }
        }
        self.deadline.store(DISARMED, Ordering::Relaxed);
        self.stop.store(0, Ordering::Relaxed);
    }

    /// Raises the stop flag if call `seq` is still running. Called by the
    /// watchdog when that call's timer is due, and by the clock backstop.
    pub(crate) fn expire(&self, seq: u64) {
        if self
            .call
            .compare_exchange(seq, seq | EXPIRING, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            self.stop.fetch_or(STOP_TIME, Ordering::Release);
            // Only this thread changes `call` while EXPIRING is set.
            self.call.store(seq, Ordering::Release);
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

    /// Objects the runtime's `JS_FreeRuntime` found still alive; zero
    /// until it has run.
    #[cfg(test)]
    pub(crate) fn leaked_objects(&self) -> u32 {
        self.leaked_objects.load(Ordering::Relaxed)
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
        let call = self.call.load(Ordering::Acquire) & !EXPIRING;
        let deadline = self.deadline.load(Ordering::Relaxed);
        if call == 0 || deadline == DISARMED {
            return false;
        }
        if self.timed_out() {
            return true;
        }
        if self.elapsed_nanos() < deadline {
            return false;
        }
        // Raised only if the call that was checked is still running (and
        // not if the watchdog is raising it right now, which it will).
        self.expire(call);
        true
    }

    /// The interrupt handler's answer: stop on a passed deadline, and stop at
    /// once after an out-of-memory error so a script that catches it cannot
    /// keep going.
    pub(crate) fn should_interrupt(&self) -> bool {
        let stop = self.oom() || self.deadline_passed();
        #[cfg(any(test, feature = "test-hooks"))]
        if stop {
            crate::sandbox::test_hooks::note_stop_answer();
        }
        stop
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

/// Runtimes freed with something left behind, the objects left and the
/// bytes left; see [`teardown_leaks`].
static LEAKY_RUNTIMES: AtomicU64 = AtomicU64::new(0);
static LEAKED_OBJECTS: AtomicU64 = AtomicU64::new(0);
static LEAKED_BYTES: AtomicU64 = AtomicU64::new(0);

impl Drop for LimitAllocator {
    /// rquickjs drops the allocator after `JS_FreeRuntime` has returned, so
    /// the runtime's teardown is complete here: whatever it still holds, or
    /// reported as leaked, it will never give back.
    fn drop(&mut self) {
        let objects = self.state.leaked_objects.load(Ordering::Relaxed);
        let bytes = self.state.used();
        if objects != 0 || bytes != 0 {
            LEAKY_RUNTIMES.fetch_add(1, Ordering::Relaxed);
            LEAKED_OBJECTS.fetch_add(u64::from(objects), Ordering::Relaxed);
            LEAKED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
        }
    }
}

/// What sandbox runtimes left behind when they were freed, summed over the
/// process. All zero unless an engine error path leaked a reference.
///
/// Not public API (hidden from the docs; it may change in any release): a
/// debug counter for surimono's soak and sweep tests. Leaked objects are
/// otherwise invisible: the engine frees its small-block arenas whole, so
/// the bytes come back, but the leak was real (review of #45).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TeardownLeaks {
    /// Runtimes freed with leaked objects or bytes.
    pub runtimes: u64,
    /// Objects still alive at `JS_FreeRuntime`.
    pub objects: u64,
    /// Bytes the engine never gave back (large blocks such objects owned).
    pub bytes: u64,
}

/// Reads the process-wide [`TeardownLeaks`] counters.
#[doc(hidden)]
pub fn teardown_leaks() -> TeardownLeaks {
    TeardownLeaks {
        runtimes: LEAKY_RUNTIMES.load(Ordering::Relaxed),
        objects: LEAKED_OBJECTS.load(Ordering::Relaxed),
        bytes: LEAKED_BYTES.load(Ordering::Relaxed),
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
    /// Defined only by the patched QuickJS (patch 14, level 3): every interrupt
    /// poll of `rt`'s contexts also loads `*flag` (atomically, relaxed) and
    /// calls the interrupt handler while it is nonzero.
    fn surimono_set_interrupt_flag(rt: *mut qjs::JSRuntime, flag: *mut u32);
    /// Defined only by the patched QuickJS (patch 22, level 5):
    /// `JS_FreeRuntime` stores in `*count` how many objects were still alive.
    fn surimono_set_leak_count(rt: *mut qjs::JSRuntime, count: *mut u32);
}

/// Makes every interrupt poll of `ctx`'s runtime read `state`'s stop flag,
/// so a passed deadline or a refused allocation stops the script at its
/// next poll instead of up to 10,000 polls later, and has `JS_FreeRuntime`
/// report leaked objects to `state`.
pub(crate) fn install_stop_flag(ctx: &Ctx<'_>, state: &Arc<LimitState>) {
    let flag = state.stop.as_ptr();
    let leaks = state.leaked_objects.as_ptr();
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
        // Same lifetime argument: `leaks` points into `state`, which
        // outlives `JS_FreeRuntime`, the only writer. It writes once, on the
        // thread freeing the runtime, and the allocator's `drop` reads it
        // on that thread after `JS_FreeRuntime` returned.
        surimono_set_leak_count(rt, leaks);
    }
}

/// The lowest engine patch level the sandbox runs on: 5, the leak count
/// at teardown (P1-RT-18) on top of level 4, the per-match polls in the
/// regexp replace loops (P1-RT-23) (see `vendor/rquickjs-sys/PATCHES.md`).
pub(crate) const REQUIRED_PATCH_LEVEL: i32 = 5;

/// The level of surimono's QuickJS patches in the linked engine (see
/// `vendor/rquickjs-sys/PATCHES.md`). Calling it makes the library refuse
/// to link against an unpatched rquickjs-sys.
pub(crate) fn patch_level() -> i32 {
    // SAFETY: a C function with no arguments that returns a constant.
    unsafe { surimono_quickjs_patch_level() }
}

/// This process's fork generation; see [`fork_generation`]. Starts at 1, so
/// 0 can mean "no process".
static FORK_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Set once a thread of this process (or of an ancestor, whose handlers a
/// fork keeps) has registered [`bump_fork_generation`].
static FORK_HANDLER: AtomicBool = AtomicBool::new(false);

/// `pthread_atfork` child handler: runs in the child, on the forking
/// thread, right after the fork and before `fork` returns there. It only
/// increments an atomic, which is async-signal-safe.
#[cfg(unix)]
extern "C" fn bump_fork_generation() {
    FORK_GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// Identifies this process among the ones that share its memory image: a
/// process forked from this one after this returned gets a strictly
/// greater value, and so do its own children. State tagged with a
/// generation is this process's own when the tags are equal; otherwise it
/// was inherited from an ancestor whose threads don't exist here. A pid
/// can't tell those apart: a child can be given the pid of a long-gone
/// ancestor whose state is still in its memory.
///
/// The first call registers a `pthread_atfork` child handler that bumps
/// the counter. Threads racing on the first call may each register one;
/// a fork then bumps it more than once, which keeps every comparison right.
/// Only processes made with libc's `fork` are counted: it runs the
/// handlers. glibc's `_Fork`, `syscall(SYS_fork)` and a raw `clone` system
/// call skip them, so a child made that way keeps its parent's generation
/// and must not use the sandbox (`vfork` and `posix_spawn` children exec at
/// once and never do).
///
/// Fails only if the handler can't be registered (out of memory). Off Unix
/// there is no `fork`, and it is always 1.
pub fn fork_generation() -> Result<u64, SandboxError> {
    #[cfg(unix)]
    if !FORK_HANDLER.load(Ordering::Acquire) {
        // SAFETY: pthread_atfork only stores the three function pointers
        // (two null, which it allows) in libc's handler list, under libc's
        // own lock, which libc also holds across `fork`, so a fork never
        // sees a half-registered handler. The handler is a plain
        // `extern "C"` function with no arguments that lives as long as
        // this library (glibc ties it to the calling object and drops it if
        // that is unloaded), and it is async-signal-safe.
        let err = unsafe { libc::pthread_atfork(None, None, Some(bump_fork_generation)) };
        if err != 0 {
            return Err(SandboxError::Engine {
                message: format!("could not register the sandbox's fork handler (error {err})"),
            });
        }
        // Every value read after this point is bumped in a child forked
        // after it.
        FORK_HANDLER.store(true, Ordering::Release);
    }
    Ok(FORK_GENERATION.load(Ordering::Acquire))
}

/// [`fork_generation`] for a caller that knows it was called before in
/// this process or an ancestor (so the handler is registered): one load.
pub(crate) fn fork_generation_now() -> u64 {
    FORK_GENERATION.load(Ordering::Acquire)
}

/// Bytes of stack left on this thread below the current frame, if the
/// platform can tell (Linux and macOS); `None` elsewhere.
pub(crate) fn stack_available() -> Option<usize> {
    let sp = stack_pointer();
    STACK_LOW.with(|low| low.map(|low| sp.saturating_sub(low)))
}

// The current stack pointer, read from the register where we can. The
// address of a local is not enough: AddressSanitizer's use-after-return mode
// moves address-taken locals to a "fake stack" on the heap, which can lie
// below the thread's stack, and `stack_available` then reported 0. Without a
// sanitizer the two agree to within a few bytes.

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn stack_pointer() -> usize {
    let sp: usize;
    // SAFETY: copies the stack pointer register into a general-purpose
    // register. It reads and writes no memory, doesn't push or pop, and
    // leaves the flags alone, as the options declare.
    unsafe {
        std::arch::asm!("mov {}, rsp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    sp
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn stack_pointer() -> usize {
    let sp: usize;
    // SAFETY: as on x86_64: copies `sp` into a general-purpose register and
    // touches nothing else.
    unsafe {
        std::arch::asm!("mov {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    sp
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
#[inline(always)]
fn stack_pointer() -> usize {
    // The address of a local: right except under AddressSanitizer.
    let here = std::hint::black_box(0u8);
    std::ptr::addr_of!(here) as usize
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
        assert_eq!(state.call.load(Ordering::Acquire), 0);
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
        assert!(std::ptr::eq(registry, watchdog::registry().unwrap()));
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

    /// A late timer can't stop the next call: the watchdog (or the clock
    /// backstop) raises the flag only for the call whose sequence number it
    /// holds, and ending a call waits out a raise in progress.
    #[test]
    fn a_stale_expiry_never_stops_the_next_call() {
        let state = LimitState::new(usize::MAX);
        state.arm(Duration::from_secs(60)).unwrap();
        let first = state.call.load(Ordering::Acquire);
        state.disarm();
        state.arm(Duration::from_secs(60)).unwrap();
        state.expire(first);
        assert!(!state.timed_out());
        state.expire(state.call.load(Ordering::Acquire));
        assert!(state.timed_out());
        state.disarm();
        assert_eq!(state.stop.load(Ordering::Acquire), 0);
        // An EXPIRING bit inherited from another process's watchdog (a
        // fork mid-raise) doesn't hold up the end of the call.
        state.arm(Duration::from_secs(60)).unwrap();
        let seq = state.call.load(Ordering::Acquire);
        state.call.store(seq | EXPIRING, Ordering::Release);
        state
            .armed_gen
            .store(fork_generation_now() - 1, Ordering::Relaxed);
        state.disarm();
        assert_eq!(state.call.load(Ordering::Acquire), 0);
    }

    /// Review of #47: the watchdog's `expire` racing a call's `arm` and
    /// `disarm` on another thread. While it only expires calls that have
    /// ended, no call is ever stopped; while it also expires the running
    /// call, every call still ends with a clear flag, and ending one never
    /// hangs on a raise in progress.
    #[test]
    fn expire_races_arm_and_disarm() {
        const ROUNDS: usize = 20_000;
        // In the second phase, every WAIT_EVERY-th call waits to be expired.
        const WAIT_EVERY: usize = 1_000;
        let state = LimitState::new(usize::MAX);
        // 0: expire the last ended call; 1: also the running one; 2: stop.
        let mode = AtomicU32::new(0);
        let last_ended = AtomicU64::new(0);
        let hits = AtomicUsize::new(0);
        /// Stops the expirer when the test thread leaves the scope, by
        /// returning or by a failing assert: otherwise `thread::scope` would
        /// wait for it forever and the test would hang instead of failing.
        struct StopExpirer<'a>(&'a AtomicU32);
        impl Drop for StopExpirer<'_> {
            fn drop(&mut self) {
                self.0.store(2, Ordering::Release);
            }
        }
        // Far longer than the 60 s wait below, so only the expirer thread,
        // never the real watchdog, can stop a call during the test.
        const BUDGET: Duration = Duration::from_secs(3_600);
        std::thread::scope(|s| {
            s.spawn(|| {
                let mut i = 0u64;
                loop {
                    let m = mode.load(Ordering::Acquire);
                    if m == 2 {
                        break;
                    }
                    i += 1;
                    let seq = if m == 1 && i % 2 == 0 {
                        state.call.load(Ordering::Acquire) & !EXPIRING
                    } else {
                        last_ended.load(Ordering::Acquire)
                    };
                    if seq != 0 {
                        state.expire(seq);
                    }
                }
            });
            let _stop = StopExpirer(&mode);
            for phase in 0..2u32 {
                mode.store(phase, Ordering::Release);
                for round in 0..ROUNDS {
                    state.arm(BUDGET).unwrap();
                    let seq = state.call.load(Ordering::Acquire);
                    for _ in 0..8 {
                        std::hint::spin_loop();
                    }
                    if phase == 1 && round % WAIT_EVERY == 0 {
                        // Hold this call open until the expirer stops it, so
                        // the phase stops running calls however the threads
                        // are scheduled (on a busy or single core the
                        // expirer may otherwise never run while one is open).
                        let until = Instant::now() + Duration::from_secs(60);
                        while !state.timed_out() && Instant::now() < until {
                            std::thread::yield_now();
                        }
                        assert!(state.timed_out(), "running call {seq} never expired");
                    }
                    if phase == 0 {
                        assert!(!state.timed_out(), "a stale expiry stopped call {seq}");
                    } else if state.timed_out() {
                        hits.fetch_add(1, Ordering::Relaxed);
                    }
                    state.disarm();
                    assert_eq!(state.call.load(Ordering::Acquire), 0);
                    assert_eq!(state.stop.load(Ordering::Acquire), 0);
                    last_ended.store(seq, Ordering::Release);
                }
            }
        });
        // Every call that waited was stopped while it ran, and then ended
        // cleanly (the asserts after `disarm`); the others may have been.
        let hits = hits.into_inner();
        println!("expired {hits} of {ROUNDS} running calls");
        assert!(hits >= ROUNDS / WAIT_EVERY, "{hits}");
    }

    /// The `pthread_atfork` handler gives a forked child, and its own child,
    /// a newer generation than every ancestor; the parent's stays.
    #[cfg(unix)]
    #[test]
    fn a_forked_child_has_a_newer_generation() {
        fn wait_ok(pid: libc::pid_t) -> bool {
            let mut status: libc::c_int = 0;
            // SAFETY: `pid` is our own unreaped child; `status` is a valid
            // place to write its status.
            let r = unsafe { libc::waitpid(pid, &mut status, 0) };
            r == pid && libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0
        }
        let parent = fork_generation().unwrap();
        // SAFETY: fork has no preconditions. The child only reads atomics,
        // forks and waits once more, and leaves through `_exit`, so no
        // destructor or test-harness code of the parent runs twice and no
        // lock another thread might hold is touched.
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            let child = fork_generation_now();
            let mut code = if child > parent && fork_generation().ok() == Some(child) {
                0
            } else {
                1
            };
            if code == 0 {
                // SAFETY: as above.
                let grandchild = unsafe { libc::fork() };
                if grandchild == 0 {
                    let ok = fork_generation_now() > child;
                    // SAFETY: ends the grandchild at once; see above.
                    unsafe { libc::_exit(if ok { 0 } else { 1 }) };
                }
                if grandchild < 0 || !wait_ok(grandchild) || fork_generation_now() != child {
                    code = 2;
                }
            }
            // SAFETY: ends the child at once; see above.
            unsafe { libc::_exit(code) };
        }
        assert!(pid > 0, "fork failed");
        assert!(wait_ok(pid), "the child saw an old generation");
        assert_eq!(fork_generation_now(), parent);
    }

    /// Child side of `forks_never_hang`: one thread makes calls and new
    /// sandboxes as fast as it can while this thread forks. Each child makes
    /// a sandbox and runs a call; every 25th also runs a loop in a sandbox
    /// made before the fork, which only its own fresh watchdog can stop.
    /// SURIMONO_FORKS sets the number of forks (default 3,000: at 0.12% the
    /// old registry lock hangs about 4 of them).
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "helper: run in a child process by forks_never_hang"]
    fn fork_stress_child() {
        use crate::{Limits, Sandbox, SandboxError};
        use std::sync::atomic::AtomicBool;

        let forks: usize = std::env::var("SURIMONO_FORKS")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(3000);
        // The loop below needs a short limit; the other calls only need one
        // that a loaded machine never reaches (P1-RT-32: at load 3 on two
        // cores the busy thread's `1` once took over 200 ms), since this
        // test is about hangs, not timing.
        let limits = Limits::default().with_time(Duration::from_millis(200));
        let generous = Limits::default().with_time(Duration::from_secs(120));
        let mut pre_fork = Sandbox::new(limits).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let busy = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut calls = 0u64;
                let mut sb = Sandbox::new(generous).unwrap();
                while !stop.load(Ordering::Relaxed) {
                    assert_eq!(sb.eval::<i32>("1"), Ok(1));
                    calls += 1;
                    if calls % 16 == 0 {
                        sb = Sandbox::new(generous).unwrap();
                    }
                }
                calls
            })
        };
        let child = |pre_fork: &mut Sandbox, i: usize| -> i32 {
            let Ok(mut sb) = Sandbox::new(generous) else {
                return 3;
            };
            if sb.eval::<i32>("6 * 7") != Ok(42) {
                return 4;
            }
            if i % 25 == 0 {
                let start = Instant::now();
                if pre_fork.eval::<()>("for (;;) {}") != Err(SandboxError::TimeLimit) {
                    return 5;
                }
                if start.elapsed() > Duration::from_secs(3) {
                    return 6;
                }
                if pre_fork.eval::<i32>("6 * 7") != Ok(42) {
                    return 7;
                }
            }
            0
        };
        const HANG: Duration = Duration::from_secs(20);
        const AT_ONCE: usize = 4;
        let (mut ok, mut hung, mut failed) = (0usize, 0usize, Vec::new());
        let mut running: Vec<(libc::pid_t, Instant)> = Vec::new();
        let mut reap = |running: &mut Vec<(libc::pid_t, Instant)>, keep: usize| {
            while running.len() > keep {
                let mut i = 0;
                while i < running.len() {
                    let (pid, started) = running[i];
                    let mut status: libc::c_int = 0;
                    // SAFETY: `pid` is our own child that we haven't reaped,
                    // and `status` is a valid place to write its status.
                    let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
                    if r == pid {
                        if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 {
                            ok += 1;
                        } else {
                            failed.push(status);
                        }
                        running.swap_remove(i);
                    } else if started.elapsed() > HANG {
                        // SAFETY: as above; it is still ours until reaped.
                        unsafe {
                            libc::kill(pid, libc::SIGKILL);
                            libc::waitpid(pid, &mut status, 0);
                        }
                        hung += 1;
                        running.swap_remove(i);
                    } else {
                        i += 1;
                    }
                }
                if running.len() > keep {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        };
        for i in 0..forks {
            // SAFETY: fork has no preconditions. The child runs only this
            // process's own code on its one thread (the allocator is the
            // system's, which is fork-safe) and leaves through `_exit`, so
            // no destructor or test-harness code of the parent runs twice.
            let pid = unsafe { libc::fork() };
            if pid == 0 {
                let code = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    child(&mut pre_fork, i)
                }))
                .unwrap_or(2);
                // SAFETY: ends the child at once; see above.
                unsafe { libc::_exit(code) };
            }
            assert!(pid > 0, "fork failed");
            running.push((pid, Instant::now()));
            reap(&mut running, AT_ONCE - 1);
        }
        reap(&mut running, 0);
        stop.store(true, Ordering::Relaxed);
        let calls = busy.join().unwrap();
        println!(
            "FORK-STRESS forks={forks} ok={ok} hung={hung} failed={failed:?} busy_calls={calls}"
        );
    }

    /// Review of #47, finding 2 (P1-RT-22): a fork while another thread is
    /// inside the sandbox (here about every call it makes) must never leave
    /// the child hanging on a lock that thread held. Runs
    /// `fork_stress_child` in its own process, so a hang fails the test.
    #[cfg(target_os = "linux")]
    #[test]
    fn forks_never_hang() {
        let exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(exe)
            .args([
                "engine::tests::fork_stress_child",
                "--exact",
                "--ignored",
                "--nocapture",
            ])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if started.elapsed() > Duration::from_secs(600) {
                let _ = child.kill();
                panic!("the fork stress run itself hung");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut out = String::new();
        std::io::Read::read_to_string(&mut child.stdout.take().unwrap(), &mut out).unwrap();
        assert!(status.success(), "{status:?}: {out}");
        let line = out
            .lines()
            .find(|l| l.starts_with("FORK-STRESS "))
            .unwrap_or_else(|| panic!("no result: {out}"));
        println!("{line}");
        assert!(
            line.contains(" hung=0 ") && line.contains(" failed=[] "),
            "{line}"
        );
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

    /// Review of #45: an object leaked on some error path used to vanish
    /// with the arenas at teardown, uncounted. A reference leaked on
    /// purpose (a forgotten `Value`) now shows in the counters, and a large
    /// block it owned in the bytes.
    #[test]
    fn teardown_counts_leaked_objects() {
        let before = teardown_leaks();
        let state = LimitState::new(usize::MAX);
        {
            let rt = Runtime::new_with_alloc(LimitAllocator::new(state.clone())).unwrap();
            let ctx = Context::full(&rt).unwrap();
            ctx.with(|ctx| {
                install_stop_flag(&ctx, &state);
                let v: Value<'_> = ctx.eval("({ big: new ArrayBuffer(1 << 20) })").unwrap();
                // The leak under test: a reference never released.
                #[allow(clippy::mem_forget)]
                std::mem::forget(v);
            });
        }
        // The object and its buffer, at least (the forgotten handle also
        // pins the context).
        assert!(state.leaked_objects() >= 2, "{}", state.leaked_objects());
        assert!(state.used() >= 1 << 20, "{}", state.used());
        let after = teardown_leaks();
        assert!(after.runtimes > before.runtimes, "{before:?} {after:?}");
        assert!(after.objects >= before.objects + 2, "{before:?} {after:?}");
        assert!(
            after.bytes >= before.bytes + (1 << 20),
            "{before:?} {after:?}"
        );
    }

    #[test]
    fn a_clean_teardown_leaks_nothing() {
        let state = LimitState::new(usize::MAX);
        {
            let rt = Runtime::new_with_alloc(LimitAllocator::new(state.clone())).unwrap();
            let ctx = Context::full(&rt).unwrap();
            ctx.with(|ctx| {
                install_stop_flag(&ctx, &state);
                let _: Value<'_> = ctx
                    .eval("globalThis.c = { self: null, b: new ArrayBuffer(1 << 20) }; c.self = c")
                    .unwrap();
            });
        }
        assert_eq!((state.leaked_objects(), state.used()), (0, 0));
    }

    #[test]
    fn the_engine_is_patched() {
        assert!(patch_level() >= REQUIRED_PATCH_LEVEL);
    }

    /// Also the regression test for `stack_pointer`: under AddressSanitizer
    /// (`scripts/ci/asan.sh`) the address of a local can lie on a fake stack
    /// outside the thread's stack, and this reported 0.
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
        let here = stack_available().unwrap();
        if cfg!(surimono_sanitized) {
            // Under AddressSanitizer the 4 KiB pads live on its fake stack,
            // so only the frames themselves use this thread's stack.
            let deep = deeper(16);
            assert!(deep < here, "{deep} vs {here}");
        } else {
            assert!(deeper(16) + 16 * 4096 <= here);
        }
        // Outside sanitizers the register and a local's address agree: the
        // local is in this frame, a little above the stack pointer (review
        // of #57). Under AddressSanitizer it may be on a fake stack.
        if !cfg!(surimono_sanitized) {
            let here = std::hint::black_box(0u8);
            let local = std::ptr::addr_of!(here) as usize;
            let sp = stack_pointer();
            assert!(
                sp <= local && local - sp < 64 * 1024,
                "sp {sp:#x}, local {local:#x}"
            );
        }
    }

    #[test]
    fn update_stack_top_is_harmless() {
        with_ctx(|ctx| {
            update_stack_top(&ctx);
            assert_eq!(ctx.eval::<i32, _>("1 + 1").unwrap(), 2);
        });
    }
}
