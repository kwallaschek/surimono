//! The QuickJS sandbox: one runtime and one context under resource limits.

use std::fmt;
use std::sync::{Arc, OnceLock};

use rquickjs::context::EvalOptions;
use rquickjs::context::intrinsic;
use rquickjs::function::{IntoArgs, Rest};
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
///   microtasks it queues. The interrupt QuickJS raises is uncatchable, so
///   `try`/`catch`/`finally` can't hold it off.
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
pub struct Sandbox {
    options: SandboxOptions,
    state: Arc<LimitState>,
    rt: Runtime,
    /// Always `Some` outside `reset`; `None` only if a reset could not build a
    /// context, in which case every call reports an engine error.
    ctx: Option<Context>,
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
        let state = LimitState::new(options.limits.memory_bytes);
        let rt = Runtime::new_with_alloc(LimitAllocator::new(state.clone()))
            .map_err(|e| setup_error(&state, e))?;
        rt.set_max_stack_size(options.limits.stack_bytes);
        let handler_state = state.clone();
        rt.set_interrupt_handler(Some(Box::new(move || handler_state.should_interrupt())));
        let ctx = new_context(&rt, &state, &options)?;
        Ok(Self {
            options,
            state,
            rt,
            ctx: Some(ctx),
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

    /// Discards all script state and starts a fresh context.
    ///
    /// Reuses the runtime when it is clean; if the previous script left
    /// microtasks queued, the runtime is replaced too.
    pub fn reset(&mut self) -> Result<(), SandboxError> {
        if self.rt.is_job_pending() {
            return self.rebuild();
        }
        // Free the old context before building the new one, so its heap does
        // not count against the new context's limit.
        self.ctx = None;
        self.rt.run_gc();
        match new_context(&self.rt, &self.state, &self.options) {
            Ok(ctx) => {
                self.ctx = Some(ctx);
                Ok(())
            }
            Err(_) => self.rebuild(),
        }
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
        let Some(ctx) = self.ctx.clone() else {
            return Err(SandboxError::Engine {
                message: "the sandbox has no context; a reset failed".into(),
            });
        };
        let state = &*self.state;
        state.arm(self.options.limits.time);
        let result = ctx.with(|ctx| {
            engine::update_stack_top(&ctx);
            let out = f(&ctx).and_then(|v| drain_jobs(&ctx, state).map(|()| v));
            out.map_err(|e| classify(&ctx, state, e))
        });
        state.disarm();
        if let Err(e) = &result {
            if e.is_limit() {
                // A limit can stop a script anywhere, leaving its state half
                // updated and microtasks queued. Start over; if even that fails,
                // keep the old runtime, which is still sound.
                let _ = self.rebuild();
            }
        }
        result
    }

    /// Replaces the runtime and context with fresh ones.
    fn rebuild(&mut self) -> Result<(), SandboxError> {
        // Drop the old runtime first so its memory is gone before we allocate.
        self.ctx = None;
        let fresh = Self::with_options(self.options.clone())?;
        *self = fresh;
        Ok(())
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
    state: &LimitState,
    options: &SandboxOptions,
) -> Result<Context, SandboxError> {
    let ctx = Context::custom::<Intrinsics>(rt).map_err(|e| setup_error(state, e))?;
    ctx.with(|ctx| {
        engine::update_stack_top(&ctx);
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
    config.set("maxLength", max_array_like_length(&options.limits))?;
    // The seed travels as two 32-bit halves: JS numbers can't hold 64 bits.
    config.set("seedLo", (options.random_seed & 0xffff_ffff) as f64)?;
    config.set("seedHi", (options.random_seed >> 32) as f64)?;
    config.set("nowMs", options.now_ms as f64)?;
    config.set("utcOffsetMinutes", options.utc_offset_minutes)?;
    prelude.call::<_, ()>((config,))
}

/// The longest array-like the array built-ins accept. Every stored element
/// costs at least 8 bytes of heap, so nothing longer can be dense.
fn max_array_like_length(limits: &Limits) -> f64 {
    (limits.memory_bytes / 8) as f64
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

    /// Built-ins that loop over an array-like's length in native code, where
    /// the interrupt handler never runs. The prelude refuses lengths no real
    /// array could have.
    #[test]
    fn long_native_loops_are_refused() {
        let vectors = [
            "Array.prototype.join.call({ length: 2 ** 40 }, '')",
            "new Array(2 ** 32 - 1).join('')",
            "String(new Array(2 ** 32 - 1))",
            "Array.prototype.toLocaleString.call({ length: 2 ** 40 })",
            "Array.prototype.reverse.call({ length: 2 ** 40 })",
            "Array.prototype.copyWithin.call({ length: 2 ** 40 }, 0, 1)",
            "Array.prototype.sort.call({ length: 2 ** 32 - 1 })",
            "Array.prototype.splice.call({ length: 2 ** 40 }, 0, 1)",
            "Array.prototype.shift.call({ length: 2 ** 40 })",
            "Array.prototype.unshift.call({ length: 2 ** 40 }, 1)",
            "Array.prototype.slice.call({ length: 2 ** 40 }, 0, 2 ** 32 - 1)",
            "[].concat.call({ length: 2 ** 40, [Symbol.isConcatSpreadable]: true })",
            "[].concat(new Array(2 ** 32 - 2))",
            "Array.prototype.flat.call({ length: 2 ** 40 })",
            "[new Array(2 ** 32 - 1)].flat()",
            "[[new Array(2 ** 32 - 1)]].flat(2)",
            "[1].flatMap(() => new Array(2 ** 32 - 1))",
            "JSON.stringify({}, new Array(2 ** 32 - 1))",
            "String.raw({ raw: { length: 2 ** 40 } })",
        ];
        let mut sb = sandbox();
        for src in vectors {
            let start = Instant::now();
            let err = sb.eval::<()>(src).unwrap_err();
            assert!(start.elapsed() < Duration::from_secs(5), "{src}");
            match err {
                SandboxError::Exception { message, .. } => assert!(
                    message.starts_with("RangeError") && message.contains("sandbox limit"),
                    "{src}: {message}"
                ),
                e if e.is_limit() => {}
                other => panic!("{src}: {other:?}"),
            }
        }
    }

    /// The guards in the prelude must not change ordinary behaviour.
    #[test]
    fn guarded_builtins_still_work() {
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
}
