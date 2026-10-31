//! The QuickJS sandbox: one runtime and one context under resource limits.

use std::fmt;
use std::sync::{Arc, OnceLock};

use rquickjs::context::EvalOptions;
use rquickjs::context::intrinsic;
use rquickjs::function::{IntoArgs, Rest};
use rquickjs::promise::PromiseState;
use rquickjs::{Context, Ctx, FromJs, Function, Object, Runtime, Value};

use crate::engine::{self, LimitAllocator, LimitState};
use crate::{Limits, SandboxError};

/// How a [`Sandbox`] is set up.
///
/// Besides the limits, these pin everything a script could otherwise read
/// from the host, so the same input gives the same output whatever the
/// host's clock and time zone: `Math.random` is seeded, `Date.now()` and
/// `new Date()` return a fixed instant, local time (including how
/// `Date.parse` reads strings without a zone) is a fixed offset from UTC,
/// never the host's zone, and `performance.now()` is a virtual clock.
///
/// Floating-point maths is the exception: `Math.sin`, `Math.exp`,
/// `Math.pow` and their kin come from the platform's C library, which can
/// differ in the last bit between glibc, musl and macOS. Output is
/// byte-identical across runs on one platform; across platforms, numbers
/// computed with those functions can differ in their last digits.
///
/// Locale-sensitive methods (`toLocaleString` and friends) ignore their
/// `locales` and `options` arguments and always format like Chrome's
/// `en-US`.
///
/// Start from [`SandboxOptions::default`] and adjust it with the `with_*`
/// setters; more options will be added in later versions:
///
/// ```
/// use surimono_core::{Limits, SandboxOptions};
///
/// let tokyo = SandboxOptions::default()
///     .with_utc_offset_minutes(9 * 60)
///     .with_limits(Limits::default().with_memory_bytes(32 << 20));
/// assert_eq!(tokyo.validate(), Ok(()));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SandboxOptions {
    /// Memory, time and stack limits for every call.
    pub limits: Limits,
    /// Seed for `Math.random`. Every fresh context starts the same sequence.
    pub random_seed: u64,
    /// What `Date.now()` and `new Date()` return, in milliseconds since the
    /// Unix epoch. Also `performance.timeOrigin`.
    pub now_ms: i64,
    /// Local time's offset from UTC in minutes, e.g. 540 for Japan. Affects
    /// `getHours()`, `toString()`, `new Date(2026, 0, 1)` and the like.
    pub utc_offset_minutes: i32,
}

impl SandboxOptions {
    /// Default `Math.random` seed: "surimono" in ASCII.
    pub const DEFAULT_RANDOM_SEED: u64 = 0x7375_7269_6d6f_6e6f;
    /// Default clock: 2026-01-01T00:00:00Z.
    pub const DEFAULT_NOW_MS: i64 = 1_767_225_600_000;
    /// Largest `|now_ms|` a JavaScript Date can hold.
    pub const MAX_ABS_NOW_MS: i64 = 8_640_000_000_000_000;
    /// Earliest real-world UTC offset, in minutes (UTC-12:00).
    pub const MIN_UTC_OFFSET_MINUTES: i32 = -12 * 60;
    /// Latest real-world UTC offset, in minutes (UTC+14:00).
    pub const MAX_UTC_OFFSET_MINUTES: i32 = 14 * 60;

    /// These options with `limits` as the limits.
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// These options with `Math.random` seeded with `seed`.
    #[must_use]
    pub fn with_random_seed(mut self, seed: u64) -> Self {
        self.random_seed = seed;
        self
    }

    /// These options with the clock fixed at `now_ms` (ms since the epoch).
    #[must_use]
    pub fn with_now_ms(mut self, now_ms: i64) -> Self {
        self.now_ms = now_ms;
        self
    }

    /// These options with local time `minutes` ahead of UTC.
    #[must_use]
    pub fn with_utc_offset_minutes(mut self, minutes: i32) -> Self {
        self.utc_offset_minutes = minutes;
        self
    }

    /// Checks every option is in range.
    pub fn validate(&self) -> Result<(), SandboxError> {
        self.limits.validate()?;
        if !(-Self::MAX_ABS_NOW_MS..=Self::MAX_ABS_NOW_MS).contains(&self.now_ms) {
            return Err(SandboxError::InvalidOptions {
                message: format!(
                    "now_ms is {}, a Date holds at most ±{}",
                    self.now_ms,
                    Self::MAX_ABS_NOW_MS
                ),
            });
        }
        if !(Self::MIN_UTC_OFFSET_MINUTES..=Self::MAX_UTC_OFFSET_MINUTES)
            .contains(&self.utc_offset_minutes)
        {
            return Err(SandboxError::InvalidOptions {
                message: format!(
                    "utc_offset_minutes is {}, it must be between {} and {}",
                    self.utc_offset_minutes,
                    Self::MIN_UTC_OFFSET_MINUTES,
                    Self::MAX_UTC_OFFSET_MINUTES
                ),
            });
        }
        Ok(())
    }
}

impl Default for SandboxOptions {
    /// Default limits, the default seed, 2026-01-01T00:00:00Z, and UTC.
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            random_seed: Self::DEFAULT_RANDOM_SEED,
            now_ms: Self::DEFAULT_NOW_MS,
            utc_offset_minutes: 0,
        }
    }
}

/// See [`Sandbox::deadline`].
#[derive(Clone)]
pub struct Deadline {
    state: Arc<LimitState>,
}

impl Deadline {
    /// Whether the running call has passed its time limit. Once this is
    /// true the call is over: it fails with [`SandboxError::TimeLimit`] as
    /// soon as control returns to the engine. False when no call is running.
    ///
    /// It reads the same stop flag the engine polls, which the sandbox's
    /// watchdog raises at the deadline, so it is about as cheap as an
    /// atomic load. Call it from host functions while the call runs.
    pub fn passed(&self) -> bool {
        self.state.deadline_passed()
    }
}

impl fmt::Debug for Deadline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Deadline")
            .field("passed", &self.passed())
            .finish()
    }
}

/// Script bytecode compiled once by [`Sandbox::compile`] and runnable in any
/// sandbox of this process.
///
/// Cheap to clone and `Send + Sync`, so one compiled bundle can serve a whole
/// pool of sandboxes. The bytes can only come from [`Sandbox::compile`], which
/// is what makes loading them sound.
#[derive(Clone)]
pub struct Bytecode {
    name: Arc<str>,
    bytes: Arc<[u8]>,
}

impl Bytecode {
    /// The file name given at compile time; it appears in stack traces.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Size of the bytecode in bytes.
    pub fn size_bytes(&self) -> usize {
        self.bytes.len()
    }

    /// Runs the compiled script in `ctx` and returns its completion value.
    ///
    /// For use inside [`Sandbox::run`]; outside it no limits apply.
    pub fn eval<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        engine::eval_bytecode(ctx, &self.bytes)
    }
}

impl fmt::Debug for Bytecode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bytecode")
            .field("name", &self.name)
            .field("size_bytes", &self.bytes.len())
            .finish()
    }
}

/// The built-ins a sandbox context gets: the ECMAScript language and nothing
/// else. No `performance` (it reads the real clock), no modules, no
/// filesystem, network, timers or `std`/`os` helpers.
type Intrinsics = (
    intrinsic::Date,
    intrinsic::Eval,
    intrinsic::RegExpCompiler,
    intrinsic::RegExp,
    intrinsic::Json,
    intrinsic::Proxy,
    intrinsic::MapSet,
    intrinsic::TypedArrays,
    intrinsic::Promise,
    intrinsic::WeakRef,
    intrinsic::DOMException,
);

/// Longest exception message kept, in bytes.
const MAX_MESSAGE_BYTES: usize = 4 * 1024;
/// Longest stack trace kept, in bytes.
const MAX_STACK_BYTES: usize = 16 * 1024;

/// A QuickJS runtime and context that run untrusted JavaScript under memory,
/// time and stack limits.
///
/// - **Memory**: the runtime's allocator refuses to go past
///   [`Limits::memory_bytes`]. Any refused allocation makes the call fail with
///   [`SandboxError::MemoryLimit`], even if the script catches the error; the
///   script is stopped at its next interrupt check.
/// - **Time**: each top-level call ([`eval`](Self::eval), [`call`](Self::call),
///   [`run`](Self::run), …) gets [`Limits::time`], covering the call and the
///   microtasks it queues. A watchdog thread (one per process, shared by
///   every sandbox) raises a flag at the deadline that the patched engine
///   reads at every interrupt check: every loop back-edge and call, and
///   inside the native loops that can run long (`join`, `fill`, `sort`…
///   over a huge array-like, string searches, BigInt arithmetic). The
///   interrupt is uncatchable, so `try`/`catch`/`finally` can't hold it
///   off, and it is sticky: built-ins that turn exceptions into rejections
///   or new errors (the `Promise` constructor, async generators,
///   `DisposableStack`) can't swallow it either. See
///   `vendor/rquickjs-sys/PATCHES.md`. `tests/hostile.rs` runs each hang
///   vector found so far and checks it ends with a limit error in about
///   twice the time limit; a single native call that never polls can still
///   overrun by its own length (known: none longer than the memory limit
///   lets a linear pass run). Host functions check [`Sandbox::deadline`].
/// - **Stack**: recursion deeper than [`Limits::stack_bytes`] fails with
///   [`SandboxError::StackOverflow`] unless the script catches the `RangeError`.
///
/// After a limit error the sandbox throws away its runtime and starts a fresh
/// one, so it is ready for the next call but every global the script set is
/// gone. After an ordinary exception the script's state is kept.
///
/// Scripts get the ECMAScript built-ins only: no modules or loader, no
/// filesystem, network, timers or host helpers. The sandbox is tied to the
/// thread that created it.
///
/// That thread needs [`Limits::stack_bytes`] of stack free below the point
/// where the sandbox is called, plus [`Sandbox::STACK_HEADROOM_BYTES`] for
/// the engine's own frames. Where the platform can say how much stack is
/// left (Linux and macOS), every call checks and fails with
/// [`SandboxError::InvalidOptions`] instead of risking a crash; elsewhere
/// it is the caller's job. Puma and Ruby threads have about 1 MiB of stack,
/// so run sandboxes on threads you create with
/// [`std::thread::Builder::stack_size`] set from `stack_bytes`.
pub struct Sandbox {
    options: SandboxOptions,
    /// The context, which also keeps its runtime alive. Always `Some` except
    /// if a rebuild failed, in which case every call reports an engine error.
    ctx: Option<Context>,
    /// Declared after `ctx` so it is dropped after the runtime, whose
    /// interrupt polls read its stop flag (the runtime's allocator holds a
    /// reference too; see `engine::install_stop_flag`).
    state: Arc<LimitState>,
}

impl fmt::Debug for Sandbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sandbox")
            .field("options", &self.options)
            .field("heap_bytes", &self.heap_bytes())
            .finish_non_exhaustive()
    }
}

impl Sandbox {
    /// Stack the engine may use beyond [`Limits::stack_bytes`] before its own
    /// check catches it: C frames of built-ins between checks, the
    /// allocator, the interrupt handler and the Rust glue. 256 KiB.
    pub const STACK_HEADROOM_BYTES: usize = 256 * 1024;

    /// Creates a sandbox with the given limits and default options.
    pub fn new(limits: Limits) -> Result<Self, SandboxError> {
        Self::with_options(SandboxOptions {
            limits,
            ..SandboxOptions::default()
        })
    }

    /// Creates a sandbox from full options.
    pub fn with_options(options: SandboxOptions) -> Result<Self, SandboxError> {
        options.validate()?;
        if engine::patch_level() < 2 {
            return Err(SandboxError::Engine {
                message: "QuickJS lacks surimono's sandbox patches".into(),
            });
        }
        check_stack(&options.limits)?;
        // Fail here, not at the first call, if the watchdog can't start.
        crate::watchdog::registry()?;
        let state = LimitState::new(options.limits.memory_bytes);
        let rt = Runtime::new_with_alloc(LimitAllocator::new(state.clone()))
            .map_err(|e| setup_error(&state, e))?;
        rt.set_max_stack_size(options.limits.stack_bytes);
        let handler_state = state.clone();
        rt.set_interrupt_handler(Some(Box::new(move || handler_state.should_interrupt())));
        let ctx = new_context(&rt, &state, &options)?;
        Ok(Self {
            options,
            ctx: Some(ctx),
            state,
        })
    }

    /// The options this sandbox was created with.
    pub fn options(&self) -> &SandboxOptions {
        &self.options
    }

    /// The limits every call runs under.
    pub fn limits(&self) -> &Limits {
        &self.options.limits
    }

    /// Bytes the JavaScript heap holds right now.
    pub fn heap_bytes(&self) -> usize {
        self.state.used()
    }

    /// A handle that host functions (Rust functions the script calls) use
    /// to check the running call's time limit.
    ///
    /// The engine stops a script at its next interrupt check once the
    /// deadline passes, but a host function is native code the engine
    /// can't interrupt, so a script that calls an expensive one in a loop
    /// would overrun by up to one call each time it catches the error. A
    /// host function should check [`Deadline::passed`] before expensive
    /// work and throw if it has: the call then fails with
    /// [`SandboxError::TimeLimit`], however the script handles the
    /// exception. The handle belongs to this sandbox's current runtime;
    /// take a new one after a [`reset`](Self::reset).
    pub fn deadline(&self) -> Deadline {
        Deadline {
            state: self.state.clone(),
        }
    }

    /// Discards all script state: the next call runs in a fresh runtime and
    /// context, with all of the old heap freed.
    ///
    /// The QuickJS runtime is replaced, not reused. rquickjs 0.14 caches the
    /// prototypes of Rust-backed classes (including every `Function::new`)
    /// per runtime, taken from the first context that needed them; reusing
    /// the runtime would keep that context alive and give later contexts
    /// functions inheriting from its `Function.prototype`. A fresh runtime
    /// costs about the same as a fresh context (≈0.45 ms in release builds).
    pub fn reset(&mut self) -> Result<(), SandboxError> {
        self.rebuild()
    }

    /// Evaluates a classic script and converts its completion value to `T`.
    ///
    /// Use `()` to ignore the value, or `String`, `f64`, `bool`, `Option<_>`,
    /// `Vec<_>` and other [`FromJs`] types. Scripts run in sloppy mode, like a
    /// browser's classic `<script>`.
    pub fn eval<T>(&mut self, source: &str) -> Result<T, SandboxError>
    where
        T: for<'js> FromJs<'js>,
    {
        self.run(|ctx| ctx.eval_with_options(source, eval_options("<eval>")))
    }

    /// Calls the global function `function` with `args` (a tuple of Rust values)
    /// and converts the result to `T`.
    pub fn call<A, T>(&mut self, function: &str, args: A) -> Result<T, SandboxError>
    where
        A: for<'js> IntoArgs<'js>,
        T: for<'js> FromJs<'js>,
    {
        self.run(move |ctx| global_function(ctx, function)?.call(args))
    }

    /// Calls the global function `function` with each of `json_args` parsed as
    /// JSON, and returns its result, which must be a string.
    ///
    /// Invalid JSON fails with the `SyntaxError` from `JSON.parse`; a result
    /// that is not a string fails with [`SandboxError::Type`].
    pub fn call_json(
        &mut self,
        function: &str,
        json_args: &[&str],
    ) -> Result<String, SandboxError> {
        self.run(|ctx| {
            let f = global_function(ctx, function)?;
            let mut args = Vec::with_capacity(json_args.len());
            for json in json_args {
                args.push(ctx.json_parse(*json)?);
            }
            let v: Value<'_> = f.call((Rest(args),))?;
            string_result(v)
        })
    }

    /// Compiles a classic script to bytecode without running it.
    ///
    /// The result runs in any sandbox of this process through
    /// [`eval_bytecode`](Self::eval_bytecode) or [`Bytecode::eval`]. Syntax
    /// errors are reported as [`SandboxError::Exception`].
    pub fn compile(&mut self, source: &str, filename: &str) -> Result<Bytecode, SandboxError> {
        let bytes = self.run(|ctx| engine::compile_script(ctx, source, filename))?;
        Ok(Bytecode {
            name: Arc::from(filename),
            bytes: Arc::from(bytes),
        })
    }

    /// Runs compiled bytecode in this sandbox's context, ignoring its
    /// completion value.
    pub fn eval_bytecode(&mut self, bytecode: &Bytecode) -> Result<(), SandboxError> {
        self.run(|ctx| bytecode.eval(ctx).map(drop))
    }

    /// Runs `f` with the context under the sandbox's limits, then runs the
    /// microtasks it queued, all within one time budget.
    ///
    /// This is the building block for the other calls. Errors from `f` are
    /// classified like script errors: a pending exception becomes
    /// [`SandboxError::Exception`] (or a limit error), a failed conversion
    /// becomes [`SandboxError::Type`].
    pub fn run<R, F>(&mut self, f: F) -> Result<R, SandboxError>
    where
        F: for<'js> FnOnce(&Ctx<'js>) -> rquickjs::Result<R>,
    {
        self.execute(|ctx, state| {
            let v = f(ctx)?;
            drain_jobs(ctx, state)?;
            Ok(v)
        })
    }

    /// Like [`run`](Self::run), for code that may produce a Promise.
    ///
    /// `f` returns a value; after the microtasks have run, a fulfilled
    /// promise is replaced by its value and a rejected one fails like a thrown
    /// exception. `then` converts the final value. One time budget covers
    /// all of it. A promise still pending once nothing is left to run fails
    /// with [`SandboxError::Type`] (the sandbox has no timers or I/O, so it
    /// would never settle).
    pub fn run_await<R, F, G>(&mut self, f: F, then: G) -> Result<R, SandboxError>
    where
        F: for<'js> FnOnce(&Ctx<'js>) -> rquickjs::Result<Value<'js>>,
        G: for<'js> FnOnce(&Ctx<'js>, Value<'js>) -> rquickjs::Result<R>,
    {
        self.execute(|ctx, state| {
            let v = f(ctx)?;
            drain_jobs(ctx, state)?;
            let v = settle(v)?;
            let r = then(ctx, v)?;
            drain_jobs(ctx, state)?;
            Ok(r)
        })
    }

    /// Enters the context with the deadline armed, runs `body`, classifies
    /// its error, and rebuilds the sandbox after a limit error.
    fn execute<R, B>(&mut self, body: B) -> Result<R, SandboxError>
    where
        B: for<'js> FnOnce(&Ctx<'js>, &LimitState) -> rquickjs::Result<R>,
    {
        let Some(ctx) = self.ctx.clone() else {
            return Err(SandboxError::Engine {
                message: "the sandbox has no context; a reset failed".into(),
            });
        };
        check_stack(&self.options.limits)?;
        self.state.arm(self.options.limits.time)?;
        // Disarms even if `body` panics (a host function can), so no timer
        // outlives the call.
        let armed = Armed(&self.state);
        let state = &*self.state;
        let result = ctx.with(|ctx| {
            engine::update_stack_top(&ctx);
            body(&ctx, state).map_err(|e| classify(&ctx, state, e))
        });
        drop(armed);
        if let Err(e) = &result {
            if e.is_limit() {
                // A limit can stop a script anywhere, leaving its state half
                // updated and microtasks queued. Start over; if even that fails,
                // later calls report an engine error (see `rebuild`).
                let _ = self.rebuild();
            }
        }
        result
    }

    /// Replaces the runtime and context with fresh ones.
    fn rebuild(&mut self) -> Result<(), SandboxError> {
        // Drop the old context first, which frees most of the old heap before
        // the new runtime allocates. The old runtime itself (with anything its
        // job queue still holds) lives until `*self = fresh`. If building
        // fails, the sandbox is left without a context and every later call
        // reports an engine error.
        self.ctx = None;
        let fresh = Self::with_options(self.options.clone())?;
        *self = fresh;
        Ok(())
    }
}

/// Disarms the deadline when dropped.
struct Armed<'a>(&'a LimitState);

impl Drop for Armed<'_> {
    fn drop(&mut self) {
        self.0.disarm();
    }
}

fn eval_options(filename: &str) -> EvalOptions {
    let mut opts = EvalOptions::default();
    opts.global = true;
    opts.strict = false;
    opts.filename = Some(filename.to_owned());
    opts
}

/// Source of the prelude every context runs first; see `prelude.js`.
const PRELUDE_SOURCE: &str = include_str!("prelude.js");

/// The prelude's bytecode, compiled by the first context that needs it.
static PRELUDE: OnceLock<Vec<u8>> = OnceLock::new();

/// Creates a context with the sandbox's built-ins and runs the prelude.
fn new_context(
    rt: &Runtime,
    state: &Arc<LimitState>,
    options: &SandboxOptions,
) -> Result<Context, SandboxError> {
    let ctx = Context::custom::<Intrinsics>(rt).map_err(|e| setup_error(state, e))?;
    ctx.with(|ctx| {
        engine::update_stack_top(&ctx);
        engine::install_stop_flag(&ctx, state);
        install_prelude(&ctx, options).map_err(|e| match classify(&ctx, state, e) {
            limit if limit.is_limit() => limit,
            other => SandboxError::Engine {
                message: format!("the sandbox prelude failed: {other}"),
            },
        })
    })?;
    Ok(ctx)
}

/// Runs the prelude function with this sandbox's settings.
fn install_prelude(ctx: &Ctx<'_>, options: &SandboxOptions) -> rquickjs::Result<()> {
    let bytes = match PRELUDE.get() {
        Some(bytes) => bytes,
        None => {
            let bytes = engine::compile_script(ctx, PRELUDE_SOURCE, "surimono:prelude")?;
            PRELUDE.get_or_init(|| bytes)
        }
    };
    let prelude: Function<'_> =
        engine::eval_bytecode(ctx, bytes)?
            .into_function()
            .ok_or(rquickjs::Error::FromJs {
                from: "value",
                to: "function",
                message: Some("the prelude must evaluate to a function".into()),
            })?;
    let config = Object::new(ctx.clone())?;
    // The seed travels as two 32-bit halves: JS numbers can't hold 64 bits.
    config.set("seedLo", (options.random_seed & 0xffff_ffff) as f64)?;
    config.set("seedHi", (options.random_seed >> 32) as f64)?;
    config.set("nowMs", options.now_ms as f64)?;
    config.set("utcOffsetMinutes", options.utc_offset_minutes)?;
    prelude.call::<_, ()>((config,))
}

/// Fails if this thread has less stack left than the engine may use, so deep
/// recursion can't overrun the real stack and crash the process.
fn check_stack(limits: &Limits) -> Result<(), SandboxError> {
    let Some(available) = engine::stack_available() else {
        return Ok(());
    };
    let needed = limits
        .stack_bytes
        .saturating_add(Sandbox::STACK_HEADROOM_BYTES);
    if available < needed {
        return Err(SandboxError::InvalidOptions {
            message: format!(
                "this thread has {available} bytes of stack left, but the sandbox needs \
                 {needed} (stack_bytes {} plus {} of headroom); run it on a thread with a \
                 larger stack or lower stack_bytes",
                limits.stack_bytes,
                Sandbox::STACK_HEADROOM_BYTES
            ),
        });
    }
    Ok(())
}

/// Maps an error from building a runtime or context.
fn setup_error(state: &LimitState, e: rquickjs::Error) -> SandboxError {
    if state.oom() || matches!(e, rquickjs::Error::Allocation) {
        SandboxError::MemoryLimit
    } else {
        SandboxError::Engine {
            message: e.to_string(),
        }
    }
}

/// Looks up a global function by name.
fn global_function<'js>(ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Function<'js>> {
    let v: Value<'js> = ctx.globals().get(name)?;
    let found = v.type_name();
    v.into_function().ok_or_else(|| rquickjs::Error::FromJs {
        from: found,
        to: "function",
        message: Some(format!("global {name:?} is not a function")),
    })
}

/// Converts a value that must be a string.
fn string_result(v: Value<'_>) -> rquickjs::Result<String> {
    let found = v.type_name();
    match v.into_string() {
        Some(s) => s.to_string(),
        None => Err(rquickjs::Error::FromJs {
            from: found,
            to: "string",
            message: Some(format!("expected a string result, got {found}")),
        }),
    }
}

/// Replaces a settled promise by its value; other values pass through.
fn settle(v: Value<'_>) -> rquickjs::Result<Value<'_>> {
    let Some(promise) = v.as_promise() else {
        return Ok(v);
    };
    match promise.state() {
        PromiseState::Resolved | PromiseState::Rejected => promise
            .result::<Value<'_>>()
            .unwrap_or(Err(rquickjs::Error::Unknown)),
        PromiseState::Pending => Err(rquickjs::Error::FromJs {
            from: "pending promise",
            to: "value",
            message: Some(
                "the promise never settled: nothing was left to run (the sandbox has no timers or I/O)"
                    .into(),
            ),
        }),
    }
}

/// Runs queued microtasks until none are left or a limit is hit.
fn drain_jobs(ctx: &Ctx<'_>, state: &LimitState) -> rquickjs::Result<()> {
    loop {
        // Checked before every job: a flood of tiny jobs might never reach the
        // engine's own interrupt check, and an out-of-memory error the script
        // swallowed still fails the call.
        if state.should_interrupt() {
            return Err(rquickjs::Error::Unknown);
        }
        if !engine::run_pending_job(ctx)? {
            return Ok(());
        }
    }
}

/// Turns an engine error into a [`SandboxError`], consuming any pending
/// exception.
fn classify(ctx: &Ctx<'_>, state: &LimitState, e: rquickjs::Error) -> SandboxError {
    let exception = matches!(e, rquickjs::Error::Exception).then(|| ctx.catch());
    if state.oom() {
        return SandboxError::MemoryLimit;
    }
    if state.timed_out() {
        return SandboxError::TimeLimit;
    }
    match (e, exception) {
        (_, Some(value)) => exception_error(value),
        (rquickjs::Error::FromJs { from, to, message }, _)
        | (rquickjs::Error::IntoJs { from, to, message }, _) => SandboxError::Type {
            message: message.unwrap_or_else(|| format!("expected {to}, got {from}")),
        },
        (rquickjs::Error::Allocation, _) => SandboxError::MemoryLimit,
        (other, _) => SandboxError::Engine {
            message: other.to_string(),
        },
    }
}

/// Describes a thrown value.
fn exception_error(value: Value<'_>) -> SandboxError {
    let (name, message, stack) = match value.as_object() {
        Some(obj) => (
            string_prop(obj.get("name").ok()),
            string_prop(obj.get("message").ok()),
            string_prop(obj.get("stack").ok()),
        ),
        None => (None, None, None),
    };
    match (name.as_deref(), message.as_deref()) {
        (Some("RangeError"), Some("Maximum call stack size exceeded")) => {
            return SandboxError::StackOverflow;
        }
        (Some("InternalError"), Some("out of memory")) => return SandboxError::MemoryLimit,
        (Some("InternalError"), Some("interrupted")) => return SandboxError::TimeLimit,
        _ => {}
    }
    let text = match (name, message) {
        (Some(n), Some(m)) if m.is_empty() => n,
        (Some(n), Some(m)) => format!("{n}: {m}"),
        (None, Some(m)) => m,
        (Some(n), None) => n,
        (None, None) => describe_primitive(&value),
    };
    SandboxError::Exception {
        message: truncate(text, MAX_MESSAGE_BYTES),
        stack: stack
            .filter(|s| !s.is_empty())
            .map(|s| truncate(s, MAX_STACK_BYTES)),
    }
}

fn string_prop(v: Option<Value<'_>>) -> Option<String> {
    v?.into_string()?.to_string().ok()
}

/// Describes a thrown non-Error value without running any script code.
fn describe_primitive(value: &Value<'_>) -> String {
    if let Some(s) = value.as_string() {
        return s.to_string().unwrap_or_default();
    }
    if let Some(n) = value.as_number() {
        return n.to_string();
    }
    if let Some(b) = value.as_bool() {
        return b.to_string();
    }
    match value.type_name() {
        "undefined" | "null" => value.type_name().to_owned(),
        other => format!("a value of type {other}"),
    }
}

fn truncate(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Small limits so the hostile tests are quick.
    fn limits() -> Limits {
        Limits {
            memory_bytes: 16 * 1024 * 1024,
            time: Duration::from_millis(300),
            stack_bytes: 256 * 1024,
        }
    }

    fn sandbox() -> Sandbox {
        Sandbox::new(limits()).unwrap()
    }

    #[test]
    fn host_functions_stop_at_the_deadline() {
        // A host function that takes 20 ms per call, called in a loop that
        // catches its errors: the engine alone would check the deadline only
        // every few thousand calls, i.e. minutes later.
        let mut sb = Sandbox::new(limits()).unwrap();
        let deadline = sb.deadline();
        assert!(!deadline.passed(), "no call is running");
        let start = Instant::now();
        let err = sb
            .run(|ctx| {
                let d = deadline.clone();
                let slow = Function::new(ctx.clone(), move |ctx: Ctx<'_>| {
                    if d.passed() {
                        return Err(rquickjs::Exception::throw_internal(
                            &ctx,
                            "past the deadline",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                    Ok(())
                })?;
                ctx.globals().set("slow", slow)?;
                ctx.eval::<(), _>("for (;;) { try { slow() } catch (e) {} }")
            })
            .unwrap_err();
        let took = start.elapsed();
        assert_eq!(err, SandboxError::TimeLimit);
        assert!(took < Duration::from_secs(2), "took {took:?}");
        // The handle tracks the call: no call, no deadline.
        assert!(!sb.deadline().passed());
    }

    /// Runs a hostile script and checks it fails with `want`, well within a
    /// bound that proves it didn't hang.
    fn expect_limit(source: &str, want: SandboxError) {
        let mut sb = sandbox();
        let start = Instant::now();
        let err = sb.eval::<()>(source).unwrap_err();
        let took = start.elapsed();
        assert_eq!(err, want, "script: {source}");
        assert!(took < Duration::from_secs(5), "took {took:?}: {source}");
        // The sandbox is usable afterwards.
        assert_eq!(sb.eval::<i32>("6 * 7").unwrap(), 42);
    }

    #[test]
    fn evaluates_scripts() {
        let mut sb = sandbox();
        assert_eq!(sb.eval::<i32>("1 + 2").unwrap(), 3);
        assert_eq!(sb.eval::<String>("'a' + 'b'").unwrap(), "ab");
        sb.eval::<()>("var kept = 5").unwrap();
        assert_eq!(sb.eval::<i32>("kept").unwrap(), 5);
        assert_eq!(
            sb.eval::<Vec<String>>("['x', 'y']").unwrap(),
            vec!["x", "y"]
        );
    }

    #[test]
    fn scripts_are_sloppy_like_classic_scripts() {
        let mut sb = sandbox();
        // Assigning an undeclared variable throws in strict mode only.
        assert_eq!(sb.eval::<i32>("undeclared = 3; undeclared").unwrap(), 3);
    }

    #[test]
    fn exceptions_carry_message_and_stack() {
        let mut sb = sandbox();
        let err = sb
            .eval::<()>("function boom() { throw new TypeError('bad') }\nboom()")
            .unwrap_err();
        let SandboxError::Exception { message, stack } = err else {
            panic!("expected an exception, got {err:?}");
        };
        assert_eq!(message, "TypeError: bad");
        assert!(stack.unwrap().contains("boom"));
    }

    #[test]
    fn thrown_primitives_are_described() {
        let mut sb = sandbox();
        for (src, want) in [
            ("throw 'plain'", "plain"),
            ("throw 42", "42"),
            ("throw null", "null"),
            ("throw undefined", "undefined"),
            ("throw Symbol('s')", "a value of type symbol"),
        ] {
            match sb.eval::<()>(src).unwrap_err() {
                SandboxError::Exception { message, stack } => {
                    assert_eq!(message, want, "{src}");
                    assert_eq!(stack, None);
                }
                other => panic!("{src}: {other:?}"),
            }
        }
    }

    #[test]
    fn syntax_errors_are_exceptions() {
        let mut sb = sandbox();
        match sb.eval::<()>("let = ;").unwrap_err() {
            SandboxError::Exception { message, .. } => {
                assert!(message.starts_with("SyntaxError"), "{message}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn huge_messages_are_truncated() {
        let mut sb = sandbox();
        match sb
            .eval::<()>("throw new Error('x'.repeat(1000000))")
            .unwrap_err()
        {
            SandboxError::Exception { message, .. } => {
                assert!(message.len() <= MAX_MESSAGE_BYTES + 3);
                assert!(message.ends_with('…'));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn type_errors_on_conversion() {
        let mut sb = sandbox();
        assert!(matches!(
            sb.eval::<i32>("'not a number'").unwrap_err(),
            SandboxError::Type { .. }
        ));
    }

    #[test]
    fn calls_global_functions() {
        let mut sb = sandbox();
        sb.eval::<()>("function add(a, b) { return a + b }")
            .unwrap();
        assert_eq!(sb.call::<_, i32>("add", (2, 3)).unwrap(), 5);
        assert_eq!(
            sb.call::<_, String>("add", ("a", "b")).unwrap(),
            "ab".to_string()
        );
        match sb.call::<_, i32>("missing", ()).unwrap_err() {
            SandboxError::Type { message } => assert!(message.contains("missing"), "{message}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn call_json_parses_arguments() {
        let mut sb = sandbox();
        sb.eval::<()>("function render(spec, opts) { return spec.title + ':' + opts.w }")
            .unwrap();
        assert_eq!(
            sb.call_json("render", &[r#"{"title":"T"}"#, r#"{"w":3}"#])
                .unwrap(),
            "T:3"
        );
        // Invalid JSON is the parser's SyntaxError.
        match sb.call_json("render", &["{oops", "{}"]).unwrap_err() {
            SandboxError::Exception { message, .. } => {
                assert!(message.starts_with("SyntaxError"), "{message}")
            }
            other => panic!("{other:?}"),
        }
        // A non-string result is a type error.
        sb.eval::<()>("function num() { return 1 }").unwrap();
        assert!(matches!(
            sb.call_json("num", &[]).unwrap_err(),
            SandboxError::Type { .. }
        ));
    }

    #[test]
    fn bytecode_compiles_once_and_runs_anywhere() {
        let mut a = sandbox();
        let bc = a
            .compile("function greet(n) { return 'hi ' + n }", "greet.js")
            .unwrap();
        assert_eq!(bc.name(), "greet.js");
        assert!(bc.size_bytes() > 0);
        // Compiling does not run it.
        assert!(a.call::<_, String>("greet", ("x",)).is_err());

        let mut b = sandbox();
        b.eval_bytecode(&bc).unwrap();
        assert_eq!(b.call::<_, String>("greet", ("b",)).unwrap(), "hi b");
        // And in a reset context of the first sandbox.
        a.reset().unwrap();
        a.eval_bytecode(&bc).unwrap();
        assert_eq!(a.call::<_, String>("greet", ("a",)).unwrap(), "hi a");
    }

    #[test]
    fn compile_reports_syntax_errors() {
        let mut sb = sandbox();
        assert!(matches!(
            sb.compile("function (", "bad.js").unwrap_err(),
            SandboxError::Exception { .. }
        ));
    }

    #[test]
    fn reset_discards_globals() {
        let mut sb = sandbox();
        sb.eval::<()>("globalThis.leak = 1; Array.prototype.evil = 1")
            .unwrap();
        sb.reset().unwrap();
        assert_eq!(
            sb.eval::<String>("typeof leak + ' ' + typeof [].evil")
                .unwrap(),
            "undefined undefined"
        );
    }

    #[test]
    fn reset_frees_contexts_that_used_rust_functions() {
        // rquickjs caches Rust-function prototypes per runtime; a reused
        // runtime would pin the first context and share its Function.prototype.
        let mut sb = sandbox();
        let base = sb.heap_bytes();
        for _ in 0..3 {
            sb.run(|ctx| {
                let f = Function::new(ctx.clone(), |x: f64| x + 1.0)?;
                ctx.globals().set("rustFn", f)?;
                ctx.eval::<(), _>(
                    "Function.prototype.planted = 1; globalThis.big = new Float64Array(1 << 20)",
                )
            })
            .unwrap();
            assert!(sb.heap_bytes() > base + 8_000_000);
            sb.reset().unwrap();
            assert!(
                sb.heap_bytes() < base + 64 * 1024,
                "heap {}",
                sb.heap_bytes()
            );
            let fresh: String = sb
                .run(|ctx| {
                    let f = Function::new(ctx.clone(), |x: f64| x + 1.0)?;
                    ctx.globals().set("rustFn", f)?;
                    ctx.eval(
                        "String(Object.getPrototypeOf(rustFn) === Function.prototype) + ' ' + typeof rustFn.planted",
                    )
                })
                .unwrap();
            assert_eq!(fresh, "true undefined");
        }
    }

    #[test]
    fn reset_with_queued_microtasks_replaces_the_runtime() {
        let mut sb = sandbox();
        // An exception leaves a microtask queued.
        let _ = sb.eval::<()>("Promise.resolve().then(() => { globalThis.ran = 1 }); throw 1");
        sb.reset().unwrap();
        assert_eq!(sb.eval::<String>("typeof ran").unwrap(), "undefined");
    }

    #[test]
    fn microtasks_run_before_the_call_returns() {
        let mut sb = sandbox();
        sb.eval::<()>("globalThis.order = []; Promise.resolve().then(() => order.push('micro')); order.push('sync')")
            .unwrap();
        assert_eq!(sb.eval::<String>("order.join(',')").unwrap(), "sync,micro");
    }

    #[test]
    fn no_host_access() {
        let mut sb = sandbox();
        let kinds: String = sb
            .eval(
                "[typeof std, typeof os, typeof require, typeof process, typeof fetch,
                  typeof XMLHttpRequest, typeof setTimeout, typeof setInterval,
                  typeof print, typeof scriptArgs].join(',')",
            )
            .unwrap();
        assert_eq!(kinds, ["undefined"; 10].join(","));
        // Static imports are a syntax error in classic scripts.
        assert!(matches!(
            sb.eval::<()>("import fs from 'fs'").unwrap_err(),
            SandboxError::Exception { .. }
        ));
        // Dynamic import has no loader and rejects.
        sb.eval::<()>(
            "globalThis.r = 'pending'; import('fs').then(() => r = 'loaded', () => r = 'rejected')",
        )
        .unwrap();
        assert_eq!(sb.eval::<String>("r").unwrap(), "rejected");
    }

    #[test]
    fn eval_and_function_constructor_stay_inside() {
        let mut sb = sandbox();
        assert_eq!(sb.eval::<i32>("eval('2 + 2')").unwrap(), 4);
        assert_eq!(
            sb.eval::<String>("Function('return typeof globalThis')()")
                .unwrap(),
            "object"
        );
    }

    // --- Hostile scripts -------------------------------------------------

    #[test]
    fn infinite_loop_hits_time_limit() {
        expect_limit("for (;;) {}", SandboxError::TimeLimit);
    }

    #[test]
    fn caught_infinite_loop_hits_time_limit() {
        expect_limit(
            "while (true) { try { while (true) {} } catch (e) {} finally { } }",
            SandboxError::TimeLimit,
        );
    }

    #[test]
    fn interrupt_skips_finally() {
        let mut sb = sandbox();
        sb.eval::<()>("globalThis.cleaned = false").unwrap();
        let err = sb
            .eval::<()>("try { for (;;) {} } finally { globalThis.cleaned = true; for (;;) {} }")
            .unwrap_err();
        assert_eq!(err, SandboxError::TimeLimit);
    }

    #[test]
    fn time_limit_is_per_call() {
        let mut sb = sandbox();
        // Several calls that each use most of the budget all succeed.
        for _ in 0..3 {
            sb.eval::<()>("var t = 0; for (let i = 0; i < 2e5; i++) { t += i }")
                .unwrap();
        }
    }

    #[test]
    fn regex_backtracking_hits_time_limit() {
        expect_limit(
            "/(a+)+$/.test('a'.repeat(40) + 'b')",
            SandboxError::TimeLimit,
        );
    }

    #[test]
    fn allocation_bomb_hits_memory_limit() {
        expect_limit(
            "const a = []; for (;;) a.push(new Array(10000).fill(1.5))",
            SandboxError::MemoryLimit,
        );
    }

    #[test]
    fn caught_out_of_memory_still_reports_memory_limit() {
        // The script swallows the error and loops; it must end quickly and as
        // MemoryLimit, not run on to the time limit.
        let mut sb = Sandbox::new(Limits {
            time: Duration::from_secs(10),
            ..limits()
        })
        .unwrap();
        let start = Instant::now();
        let err = sb
            .eval::<()>(
                "for (;;) { try { const a = []; for (;;) a.push('x'.repeat(1 << 16) + Math.random()) } catch (e) {} }",
            )
            .unwrap_err();
        assert_eq!(err, SandboxError::MemoryLimit);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn caught_out_of_memory_that_returns_normally_still_fails() {
        expect_limit(
            "try { 'x'.repeat(64 * 1024 * 1024) } catch (e) { 'recovered' }",
            SandboxError::MemoryLimit,
        );
    }

    #[test]
    fn deep_recursion_hits_stack_limit() {
        expect_limit(
            "function f(n) { return f(n + 1) + 1 } f(0)",
            SandboxError::StackOverflow,
        );
    }

    #[test]
    fn deep_recursion_in_native_code_hits_stack_limit() {
        // JSON.stringify recurses natively.
        expect_limit(
            "let o = {}; for (let i = 0; i < 5e4; i++) o = { o }; JSON.stringify(o)",
            SandboxError::StackOverflow,
        );
    }

    #[test]
    fn caught_stack_overflow_is_ordinary_js() {
        let mut sb = sandbox();
        let v: String = sb
            .eval("function f() { f() } try { f(); 'no' } catch (e) { e instanceof RangeError ? 'caught' : 'other' }")
            .unwrap();
        assert_eq!(v, "caught");
    }

    #[test]
    fn microtask_flood_hits_a_limit() {
        // One job queues the next, forever: constant memory, so time.
        expect_limit(
            "function f() { Promise.resolve().then(f) } f()",
            SandboxError::TimeLimit,
        );
    }

    #[test]
    fn exponential_microtask_flood_hits_a_limit() {
        let mut sb = sandbox();
        let start = Instant::now();
        let err = sb
            .eval::<()>("function f() { Promise.resolve().then(f); Promise.resolve().then(f) } f()")
            .unwrap_err();
        assert!(
            matches!(err, SandboxError::MemoryLimit | SandboxError::TimeLimit),
            "{err:?}"
        );
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(sb.eval::<i32>("1").unwrap(), 1);
    }

    #[test]
    fn async_infinite_loop_hits_time_limit() {
        expect_limit(
            "(async () => { for (;;) { await null } })()",
            SandboxError::TimeLimit,
        );
    }

    #[test]
    fn huge_string_hits_memory_limit() {
        expect_limit("'x'.repeat(256 * 1024 * 1024)", SandboxError::MemoryLimit);
        expect_limit(
            "let s = 'x'.repeat(1 << 20); for (;;) s = s.concat(s).toUpperCase()",
            SandboxError::MemoryLimit,
        );
    }

    #[test]
    fn doubling_string_ends_with_a_typed_error() {
        // Concatenation builds ropes lazily, so this reaches QuickJS's maximum
        // string length (2^30) before the heap fills: an ordinary exception.
        let mut sb = sandbox();
        match sb.eval::<()>("let s = 'x'; for (;;) s += s").unwrap_err() {
            SandboxError::Exception { message, .. } => {
                assert_eq!(message, "InternalError: string too long")
            }
            SandboxError::MemoryLimit => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn huge_typed_array_hits_memory_limit() {
        expect_limit("new Float64Array(1e8)", SandboxError::MemoryLimit);
    }

    /// The array built-ins the engine patch touches behave as usual. (Huge
    /// array-likes are in `tests/hostile.rs`, which runs them in a child
    /// process so a regression fails instead of hanging.)
    #[test]
    fn array_builtins_behave_normally() {
        let mut sb = sandbox();
        let checks = [
            ("[1, 2, 3].join('-')", "1-2-3"),
            ("String([1, [2, 3]])", "1,2,3"),
            ("[3, 1, 2].sort().join()", "1,2,3"),
            ("[3, 1, 2].sort((a, b) => b - a).join()", "3,2,1"),
            ("[1, 2, 3].reverse().join()", "3,2,1"),
            (
                "const a = [1, 2, 3, 4]; a.splice(1, 2, 'x'); a.join()",
                "1,x,4",
            ),
            (
                "const b = [1, 2]; b.shift(); b.unshift(0, 0); b.join()",
                "0,0,2",
            ),
            ("[1, 2, 3, 4, 5].copyWithin(0, 3).join()", "4,5,3,4,5"),
            ("[1].concat([2, 3], 4, [[5]]).length + ''", "5"),
            ("[1, [2, [3, [4]]]].flat(Infinity).join()", "1,2,3,4"),
            ("[1, [2, [3]]].flat().length + ''", "3"),
            ("[1, 2].flatMap(x => [x, x * 10]).join()", "1,10,2,20"),
            (
                "[1, 2].flatMap(function (x) { return [x + this.k] }, { k: 1 }).join()",
                "2,3",
            ),
            (
                "(function () { return Array.prototype.slice.call(arguments, 1).join() })(1, 2, 3)",
                "2,3",
            ),
            (
                "Array.prototype.join.call(new Uint8Array([1, 2]), '+')",
                "1+2",
            ),
            ("Array.prototype.join.call(new String('ab'), '.')", "a.b"),
            (
                "Array.prototype.join.call({ length: 2, 0: 'a', 1: 'b' })",
                "a,b",
            ),
            ("JSON.stringify({ a: 1, b: 2 }, ['b'])", "{\"b\":2}"),
            (
                "JSON.stringify({ a: [1] }, null, 1)",
                "{\n \"a\": [\n  1\n ]\n}",
            ),
            ("String.raw`a\\n${1}b`", "a\\n1b"),
            (
                "[].join.name + [].join.length + [].concat.length + JSON.stringify.length",
                "join113",
            ),
            ("Object.keys(Array.prototype).length + ''", "0"),
            ("[1, 2, 3].toLocaleString()", "1,2,3"),
        ];
        for (src, want) in checks {
            assert_eq!(sb.eval::<String>(src).unwrap(), want, "{src}");
        }
    }

    #[test]
    fn limit_errors_discard_state() {
        let mut sb = sandbox();
        sb.eval::<()>("globalThis.before = 1").unwrap();
        assert_eq!(sb.eval::<()>("for (;;) {}"), Err(SandboxError::TimeLimit));
        assert_eq!(sb.eval::<String>("typeof before").unwrap(), "undefined");
    }

    #[test]
    fn heap_returns_to_baseline_after_reset() {
        let mut sb = sandbox();
        let base = sb.heap_bytes();
        assert!(base > 0);
        sb.eval::<()>("globalThis.big = new Array(100000).fill('x').map((s, i) => s + i)")
            .unwrap();
        assert!(sb.heap_bytes() > base + 1_000_000);
        sb.reset().unwrap();
        let after = sb.heap_bytes();
        assert!(after < base + 64 * 1024, "base {base}, after reset {after}");
    }

    #[test]
    fn limits_out_of_range_are_rejected() {
        for bad in [
            Limits {
                memory_bytes: 1024,
                ..limits()
            },
            Limits {
                stack_bytes: 0,
                ..limits()
            },
            Limits {
                stack_bytes: 1024,
                ..limits()
            },
            Limits {
                stack_bytes: 64 * 1024 * 1024,
                ..limits()
            },
        ] {
            assert!(
                matches!(
                    Sandbox::new(bad).unwrap_err(),
                    SandboxError::InvalidOptions { .. }
                ),
                "{bad:?}"
            );
        }
        // The smallest allowed limits work.
        let mut sb = Sandbox::new(Limits {
            memory_bytes: Limits::MIN_MEMORY_BYTES,
            time: Duration::from_millis(100),
            stack_bytes: Limits::MIN_STACK_BYTES,
        })
        .unwrap();
        assert_eq!(sb.eval::<i32>("1 + 1").unwrap(), 2);
        assert_eq!(sb.eval::<()>("for (;;) {}"), Err(SandboxError::TimeLimit));
        assert_eq!(
            sb.eval::<()>("function f() { return f() + 1 } f()"),
            Err(SandboxError::StackOverflow)
        );
        assert_eq!(
            sb.eval::<()>("const a = []; for (;;) a.push({})"),
            Err(SandboxError::MemoryLimit)
        );
    }

    #[test]
    fn run_await_settles_promises() {
        let mut sb = sandbox();
        let to_string = |_: &Ctx<'_>, v: Value<'_>| string_result(v);
        // A plain value passes through.
        assert_eq!(
            sb.run_await(|ctx| ctx.eval("'plain'"), to_string).unwrap(),
            "plain"
        );
        // A promise resolved through several microtasks.
        assert_eq!(
            sb.run_await(
                |ctx| ctx.eval("(async () => { await null; await 1; return 'later' })()"),
                to_string
            )
            .unwrap(),
            "later"
        );
        // A rejection is an exception.
        match sb
            .run_await(
                |ctx| ctx.eval("Promise.reject(new RangeError('no'))"),
                to_string,
            )
            .unwrap_err()
        {
            SandboxError::Exception { message, .. } => assert_eq!(message, "RangeError: no"),
            other => panic!("{other:?}"),
        }
        // A promise nothing will ever settle.
        match sb
            .run_await(|ctx| ctx.eval("new Promise(() => {})"), to_string)
            .unwrap_err()
        {
            SandboxError::Type { message } => assert!(message.contains("never settled")),
            other => panic!("{other:?}"),
        }
        // Limits still apply, and the budget covers the awaiting.
        assert_eq!(
            sb.run_await(
                |ctx| ctx.eval("(async () => { for (;;) await null })()"),
                to_string
            ),
            Err(SandboxError::TimeLimit)
        );
    }

    #[test]
    fn run_gives_direct_context_access() {
        let mut sb = sandbox();
        let n = sb
            .run(|ctx| {
                ctx.globals().set("fromRust", 41)?;
                ctx.eval::<i32, _>("fromRust + 1")
            })
            .unwrap();
        assert_eq!(n, 42);
    }

    #[test]
    fn sandboxes_are_independent() {
        let mut a = sandbox();
        let mut b = sandbox();
        a.eval::<()>("globalThis.only_in_a = 1").unwrap();
        assert_eq!(b.eval::<String>("typeof only_in_a").unwrap(), "undefined");
    }

    #[test]
    fn works_from_a_deep_rust_stack() {
        // The stack limit is measured from where we enter the engine, so a
        // sandbox created high up and used deep down still catches recursion
        // before the thread's real stack runs out.
        fn deep(n: usize, sb: &mut Sandbox) -> SandboxError {
            let pad = [0u8; 4096];
            std::hint::black_box(&pad);
            if n == 0 {
                sb.eval::<()>("function f() { return f() + 1 } f()")
                    .unwrap_err()
            } else {
                deep(n - 1, sb)
            }
        }
        let handle = std::thread::Builder::new()
            .stack_size(4 * 1024 * 1024)
            .spawn(|| {
                let mut sb = sandbox();
                deep(200, &mut sb)
            })
            .unwrap();
        assert_eq!(handle.join().unwrap(), SandboxError::StackOverflow);
    }

    /// Review of #5, F2: `stack_bytes` of 8 MiB on a 1 MiB thread aborted the
    /// process on deep recursion. Now the sandbox refuses to run there.
    #[test]
    fn too_little_thread_stack_is_refused() {
        if engine::stack_available().is_none() {
            return;
        }
        let big = Limits::default().with_stack_bytes(8 * 1024 * 1024);
        let small_thread = std::thread::Builder::new().stack_size(1024 * 1024);
        let err = small_thread
            .spawn(move || Sandbox::new(big).unwrap_err())
            .unwrap()
            .join()
            .unwrap();
        match err {
            SandboxError::InvalidOptions { message } => {
                assert!(message.contains("stack"), "{message}")
            }
            other => panic!("{other:?}"),
        }
        // The default limits fit a 1 MiB thread, and recursion there is a
        // typed error.
        let err = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                let mut sb = Sandbox::new(Limits::default()).unwrap();
                sb.eval::<()>("function f() { return f() + 1 } f()")
                    .unwrap_err()
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(err, SandboxError::StackOverflow);
        // A sandbox made high up and called from deep down is refused at the
        // call, not run into the guard page. (glibc may hand a new thread a
        // cached stack bigger than asked for, so recurse until the stack
        // really is short rather than trusting the requested size.)
        let err = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                fn deep(sb: &mut Sandbox, short: usize) -> Result<(), SandboxError> {
                    let pad = std::hint::black_box([0u8; 4096]);
                    let r = if engine::stack_available().unwrap() < short {
                        sb.eval::<()>("function f() { return f() + 1 } f()")
                    } else {
                        deep(sb, short)
                    };
                    std::hint::black_box(&pad);
                    r
                }
                let limits = Limits::default();
                let mut sb = Sandbox::new(limits).unwrap();
                let short = limits.stack_bytes + Sandbox::STACK_HEADROOM_BYTES - 64 * 1024;
                deep(&mut sb, short).unwrap_err()
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(
            matches!(err, SandboxError::InvalidOptions { .. }),
            "{err:?}"
        );
    }

    /// A script that allocates on many paths: the compiler (through `eval`
    /// and `Function`), async functions and promise jobs, generators,
    /// strings, arrays, Date parsing, sorting, JSON and regexps.
    const ALLOCATING_SCRIPT: &str = r#"
        async function f(n) { await null; return [n, { a: n }, 'x' + n]; }
        const out = [];
        for (let i = 0; i < 8; i++) out.push(f(i).then((v) => v.join()));
        Promise.all(out).then((v) => { globalThis.r = JSON.stringify(v); });
        const g = (function* () { yield* [1, 2, 3]; })();
        [...g, ...new Set([4, 5]), ...new Map([[6, 7]]).keys()].sort().join()
          + new Date('2026-01-01T00:00:00').toISOString()
          + JSON.stringify(JSON.parse('{"a":[1,{"b":2}]}'))
          + /(a+)(b)?/.exec('aaab')[0] + String(Math.random()).length
          + [3, 1, 2].toSorted().join() + new Int32Array([3, 1, 2]).sort().join()
          + Object.entries({ x: 1, y: [2] }).flat(2).join()
          + (() => { try { null.x } catch (e) { return e.stack.length > 0 } })()
          + eval('(function (a, b) { let c = a + b; return [c, ...arguments]; })(1, 2).join()')
          + new Function('x', 'class K { #p = x; get p() { return this.#p * 2; } } return new K().p')(21)
          + `${[1, 2].map((x) => x ** 2)}`.padStart(8, '-')
    "#;

    /// Compiles a large generated program through `eval`, so allocations in
    /// the parser and bytecode emitter are refused too.
    const COMPILING_SCRIPT: &str = r#"
        const parts = [];
        for (let i = 0; i < 200; i++) {
          parts.push(`function f${i}(a, ...rest) {
            let x = a + ${i}; const o = { k${i}: x, [\`c${i}\`]: [x, ...rest] };
            for (let j = 0; j < 3; j++) { try { x += j; } finally { x--; } }
            return async () => { await null; return o.k${i} + x; };
          }`);
        }
        eval(parts.join('\n') + '; typeof f199') + new Function(parts[7] + '; return f7(1)')().constructor.name
    "#;

    /// Child side of `out_of_memory_never_aborts`. With SURIMONO_OOM_COUNT
    /// set it prints how many allocations a clean run of the script makes;
    /// otherwise, for each N in SURIMONO_OOM_NS, it refuses the script's Nth
    /// allocation.
    #[test]
    #[ignore = "helper: run in a child process by out_of_memory_never_aborts"]
    fn oom_sweep_child() {
        let limits = limits().with_time(Duration::from_secs(10));
        let script = match std::env::var("SURIMONO_OOM_SCRIPT").as_deref() {
            Ok("compiling") => COMPILING_SCRIPT,
            _ => ALLOCATING_SCRIPT,
        };
        if std::env::var_os("SURIMONO_OOM_COUNT").is_some() {
            let mut sb = Sandbox::new(limits).unwrap();
            engine::fault::refuse_nth(usize::MAX);
            sb.eval::<String>(script).unwrap();
            println!("OOM-COUNT {}", engine::fault::count());
            return;
        }
        let ns = std::env::var("SURIMONO_OOM_NS").unwrap();
        let (mut ok, mut refused) = (0, 0);
        for n in ns.split(',').map(|n| n.parse::<usize>().unwrap()) {
            let mut sb = Sandbox::new(limits).unwrap();
            engine::fault::refuse_nth(n);
            let result = sb.eval::<String>(script);
            engine::fault::refuse_nth(0);
            // Whatever happened, the sandbox works afterwards.
            assert_eq!(
                sb.eval::<i32>("6 * 7"),
                Ok(42),
                "after refusing allocation {n}"
            );
            match result {
                Ok(_) => ok += 1,
                Err(SandboxError::MemoryLimit) => refused += 1,
                Err(other) => panic!("refusing allocation {n} gave {other:?}"),
            }
        }
        println!("OOM-SWEEP ok={ok} memory_limit={refused}");
    }

    /// Review of #5, F7: refuse each single allocation of an allocation-heavy
    /// script in turn. The memory limit can refuse any allocation a script
    /// makes, so QuickJS must report every such failure, never crash or
    /// `abort()`; each run must end as success or `MemoryLimit` and leave
    /// the sandbox usable. (Building the runtime and context is not swept:
    /// it needs about 200 KiB, under the 1 MiB minimum limit, so the limit
    /// never refuses there.) Runs in child processes, so a crash fails the
    /// test instead of killing the harness.
    #[test]
    fn out_of_memory_never_aborts() {
        let exe = std::env::current_exe().unwrap();
        let child = |script: &str, env: (&str, String)| {
            let out = std::process::Command::new(&exe)
                .args([
                    "sandbox::tests::oom_sweep_child",
                    "--exact",
                    "--ignored",
                    "--nocapture",
                ])
                .env("SURIMONO_OOM_SCRIPT", script)
                .env(env.0, &env.1)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            assert!(
                out.status.success(),
                "the child died ({:?}) on the {script} script with {}={}:\n{stdout}\n{}",
                out.status,
                env.0,
                env.1,
                String::from_utf8_lossy(&out.stderr)
            );
            stdout
        };
        for script in ["allocating", "compiling"] {
            let counted = child(script, ("SURIMONO_OOM_COUNT", "1".into()));
            let total: usize = counted
                .split("OOM-COUNT ")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| panic!("no count in: {counted}"));
            // quickjs-ng carves small objects out of arena chunks, so these
            // are the chunk and buffer allocations the limit can refuse.
            assert!(total > 20, "{script}: only {total} allocations");
            let ns: Vec<usize> = (1..=total).collect();
            for chunk in ns.chunks(250) {
                let list = chunk
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let out = child(script, ("SURIMONO_OOM_NS", list));
                assert!(out.contains("OOM-SWEEP"), "{out}");
            }
        }
    }
}
