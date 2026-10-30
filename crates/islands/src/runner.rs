//! The island runner: bundles compiled once, rendered on worker threads.

use std::collections::HashMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError, RwLock, TryLockError};
use std::thread::{self, JoinHandle};

use surimono_core::rquickjs::function::Opt;
use surimono_core::rquickjs::{self, Coerced, Ctx, Exception, Function, Object, Value};
use surimono_core::{Bytecode, Limits, Sandbox, SandboxOptions};

use crate::{IslandError, TextMeasurer, builtins};

/// How an [`IslandRunner`] is set up.
///
/// Build it with `..RunnerOptions::default()` so new fields don't break you.
#[derive(Debug, Clone)]
pub struct RunnerOptions {
    /// Memory, time and stack limits for each render.
    pub limits: Limits,
    /// Number of worker threads, each with its own JavaScript runtime.
    /// 0 picks the default: the number of CPUs, at most 4.
    pub pool_size: usize,
    /// Seed for `Math.random`; every render starts the same sequence.
    pub seed: u64,
    /// Answers `ctx.measureText` for renderers.
    pub text_measurer: TextMeasurer,
    /// What `Date.now()` returns in renderers, in ms since the Unix epoch.
    pub now_ms: i64,
    /// Local time's offset from UTC in minutes, for date formatting.
    pub utc_offset_minutes: i32,
}

impl RunnerOptions {
    /// Most worker threads a runner may have.
    pub const MAX_POOL_SIZE: usize = 64;
    /// Default ceiling on worker threads when `pool_size` is 0.
    pub const DEFAULT_MAX_POOL_SIZE: usize = 4;

    fn sandbox_options(&self) -> SandboxOptions {
        SandboxOptions {
            limits: self.limits,
            random_seed: self.seed,
            now_ms: self.now_ms,
            utc_offset_minutes: self.utc_offset_minutes,
        }
    }

    /// The pool size this runner will use.
    pub fn effective_pool_size(&self) -> usize {
        if self.pool_size > 0 {
            return self.pool_size;
        }
        thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(Self::DEFAULT_MAX_POOL_SIZE)
    }
}

impl Default for RunnerOptions {
    fn default() -> Self {
        let sandbox = SandboxOptions::default();
        Self {
            limits: sandbox.limits,
            pool_size: 0,
            seed: sandbox.random_seed,
            text_measurer: TextMeasurer::default(),
            now_ms: sandbox.now_ms,
            utc_offset_minutes: sandbox.utc_offset_minutes,
        }
    }
}

/// One island to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IslandRequest<'a> {
    /// The registered renderer (bundle) name, e.g. `"echarts"`.
    pub renderer: &'a str,
    /// The island's spec as JSON text, passed to the entry unparsed.
    pub spec_json: &'a str,
    /// Width in CSS px, given to the renderer as `ctx.width`.
    pub width: u32,
    /// Height in CSS px, given to the renderer as `ctx.height`.
    pub height: u32,
}

/// A compiled bundle and the global function to call.
struct Bundle {
    name: String,
    entry: String,
    bytecode: Bytecode,
    /// Cleared once the bundle's top level is seen to queue a microtask: a
    /// render must then run that microtask in its own call, after the
    /// bundle and before the entry, so it is never preloaded.
    preloadable: AtomicBool,
}

struct RenderJob {
    bundle: Arc<Bundle>,
    spec_json: String,
    width: u32,
    height: u32,
    reply: SyncSender<Result<String, IslandError>>,
}

struct RegisterJob {
    name: String,
    source: String,
    entry: String,
    reply: SyncSender<Result<Bundle, IslandError>>,
}

enum Job {
    Render(RenderJob),
    Register(RegisterJob),
}

/// What every worker needs.
struct Shared {
    sandbox_options: SandboxOptions,
    measurer: TextMeasurer,
    renders: AtomicU64,
    failed_renders: AtomicU64,
    preloaded_renders: AtomicU64,
    idle_workers: AtomicUsize,
}

/// Counters for an [`IslandRunner`], for monitoring and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct RunnerStats {
    /// Renders finished, including failed ones.
    pub renders: u64,
    /// Renders that returned an error.
    pub failed_renders: u64,
    /// Renders whose bundle a worker had already loaded into its fresh
    /// runtime while it was idle, so the request skipped that step.
    pub preloaded_renders: u64,
    /// Workers waiting for a job right now, with their runtime reset and
    /// preloaded. Equal to the pool size when the runner is quiet.
    pub idle_workers: usize,
}

/// Renders islands: JavaScript bundles that turn a JSON spec into an SVG
/// string.
///
/// A bundle is a classic script that defines a global function
/// `entry(specJson, ctx)` returning an SVG string (or a Promise of one).
/// `specJson` is the island's spec as JSON text; `ctx` has `width`, `height`
/// and `measureText(text, cssFont)`, which returns canvas-style metrics
/// (`width`, `actualBoundingBoxAscent`, …) from the runner's
/// [`TextMeasurer`].
///
/// Bundles are compiled to bytecode once, at registration, and loaded from
/// bytecode for each render (no re-parsing). Each render runs on one of a
/// fixed pool of worker threads, in a fresh JavaScript runtime under the
/// runner's limits, so nothing leaks from one render to the next, the whole
/// heap is freed after every render, and the same request always gives the
/// same SVG. Each worker prepares the runtime for its next render right
/// after sending a reply, off the caller's clock: it starts a fresh runtime
/// and, if no request is waiting, evaluates the bundle it rendered last, so
/// a repeated renderer skips that step. (A bundle whose top level queues
/// microtasks is never preloaded.) The limits apply to each island as a
/// whole: one time budget covers loading the bundle, if it wasn't
/// preloaded, the entry call and the microtasks it queues. The result is
/// the same either way.
///
/// A custom [`TextMeasurer`] runs inside that budget, on the worker
/// thread, and must return promptly: the time limit can only interrupt
/// JavaScript, not a measurer that blocks.
///
/// `IslandRunner` is `Send + Sync`: share it between threads (e.g. in an
/// `Arc`); [`render`](Self::render) blocks the calling thread until a worker
/// is done.
pub struct IslandRunner {
    shared: Arc<Shared>,
    bundles: RwLock<HashMap<String, Arc<Bundle>>>,
    /// Held while a built-in renderer is compiled on first use.
    builtin_lock: Mutex<()>,
    jobs: Option<Sender<Job>>,
    workers: Vec<JoinHandle<()>>,
}

impl fmt::Debug for IslandRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IslandRunner")
            .field("pool_size", &self.workers.len())
            .field("renderers", &self.renderers())
            .finish_non_exhaustive()
    }
}

impl IslandRunner {
    /// Starts the worker pool. No bundles are registered yet.
    pub fn new(opts: RunnerOptions) -> Result<Self, IslandError> {
        let sandbox_options = opts.sandbox_options();
        sandbox_options
            .validate()
            .map_err(|e| IslandError::InvalidOptions {
                message: e.to_string(),
            })?;
        let pool_size = opts.effective_pool_size();
        if pool_size > RunnerOptions::MAX_POOL_SIZE {
            return Err(IslandError::InvalidOptions {
                message: format!(
                    "pool_size is {pool_size}, the maximum is {}",
                    RunnerOptions::MAX_POOL_SIZE
                ),
            });
        }
        // Room for the script's stack limit plus the Rust frames around it
        // (text measurement, shaping). Untouched stack costs no memory.
        let stack_size = sandbox_options.limits.stack_bytes + 8 * 1024 * 1024;
        let shared = Arc::new(Shared {
            sandbox_options,
            measurer: opts.text_measurer,
            renders: AtomicU64::new(0),
            failed_renders: AtomicU64::new(0),
            preloaded_renders: AtomicU64::new(0),
            idle_workers: AtomicUsize::new(0),
        });
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let mut workers = Vec::with_capacity(pool_size);
        for i in 0..pool_size {
            let shared = shared.clone();
            let rx = rx.clone();
            let handle = thread::Builder::new()
                .name(format!("surimono-island-{i}"))
                .stack_size(stack_size)
                .spawn(move || Worker::new(shared).run(&rx))
                .map_err(|e| IslandError::Worker {
                    message: format!("could not start a worker thread: {e}"),
                })?;
            workers.push(handle);
        }
        Ok(Self {
            shared,
            bundles: RwLock::new(HashMap::new()),
            builtin_lock: Mutex::new(()),
            jobs: Some(tx),
            workers,
        })
    }

    /// Compiles `js_source` and registers it as renderer `name`, replacing
    /// any bundle of that name. `entry` is the global function to call.
    ///
    /// The bundle runs once here, under the runner's limits, to check that it
    /// defines `entry`.
    pub fn register_bundle(
        &self,
        name: &str,
        js_source: &str,
        entry: &str,
    ) -> Result<(), IslandError> {
        if name.is_empty() || entry.is_empty() {
            return Err(IslandError::InvalidOptions {
                message: "bundle name and entry must not be empty".into(),
            });
        }
        let bundle = self.dispatch(|reply| {
            Job::Register(RegisterJob {
                name: name.to_owned(),
                source: js_source.to_owned(),
                entry: entry.to_owned(),
                reply,
            })
        })?;
        self.bundles
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), Arc::new(bundle));
        Ok(())
    }

    /// Renders one island and returns the renderer's SVG string.
    ///
    /// A built-in renderer (see [`builtin_renderers`](crate::builtin_renderers))
    /// is compiled on its first use, unless a bundle was registered under
    /// its name.
    pub fn render(&self, req: &IslandRequest<'_>) -> Result<String, IslandError> {
        let bundle = self.bundle(req.renderer)?;
        serde_json::from_str::<serde::de::IgnoredAny>(req.spec_json).map_err(|e| {
            IslandError::BadJson {
                message: e.to_string(),
            }
        })?;
        self.dispatch(|reply| {
            Job::Render(RenderJob {
                bundle,
                spec_json: req.spec_json.to_owned(),
                width: req.width,
                height: req.height,
                reply,
            })
        })
    }

    /// The bundle for `name`: a registered one, or a built-in compiled now.
    fn bundle(&self, name: &str) -> Result<Arc<Bundle>, IslandError> {
        let registered = |bundles: &RwLock<HashMap<String, Arc<Bundle>>>| {
            bundles
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .get(name)
                .cloned()
        };
        if let Some(bundle) = registered(&self.bundles) {
            return Ok(bundle);
        }
        let Some(builtin) = builtins::find(name) else {
            return Err(IslandError::UnknownRenderer {
                name: name.to_owned(),
            });
        };
        // One compile per built-in, even if many threads ask at once.
        let _compiling = self
            .builtin_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(bundle) = registered(&self.bundles) {
            return Ok(bundle);
        }
        self.register_bundle(builtin.name, builtin.source, builtin.entry)?;
        registered(&self.bundles).ok_or_else(|| IslandError::UnknownRenderer {
            name: name.to_owned(),
        })
    }

    /// Names of the registered renderers, sorted. Built-in renderers are
    /// listed once they have been used.
    pub fn renderers(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .bundles
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        names.sort();
        names
    }

    /// Number of worker threads.
    pub fn pool_size(&self) -> usize {
        self.workers.len()
    }

    /// Counters since the runner started.
    pub fn stats(&self) -> RunnerStats {
        RunnerStats {
            renders: self.shared.renders.load(Ordering::Relaxed),
            failed_renders: self.shared.failed_renders.load(Ordering::Relaxed),
            preloaded_renders: self.shared.preloaded_renders.load(Ordering::Relaxed),
            idle_workers: self.shared.idle_workers.load(Ordering::SeqCst),
        }
    }

    /// Sends a job to the pool and waits for its answer.
    fn dispatch<T>(
        &self,
        make: impl FnOnce(SyncSender<Result<T, IslandError>>) -> Job,
    ) -> Result<T, IslandError> {
        let (reply, answer) = mpsc::sync_channel(1);
        let gone = || IslandError::Worker {
            message: "the worker pool has stopped".into(),
        };
        let jobs = self.jobs.as_ref().ok_or_else(gone)?;
        jobs.send(make(reply)).map_err(|_| gone())?;
        answer.recv().map_err(|_| gone())?
    }
}

impl Drop for IslandRunner {
    fn drop(&mut self) {
        // Closing the queue ends the workers once they finish their job.
        self.jobs = None;
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

/// One worker thread's state.
struct Worker {
    shared: Arc<Shared>,
    /// The worker's sandbox, always fresh when a job starts: it is reset
    /// after every job. `None` until first use or after a failure.
    sandbox: Option<Sandbox>,
    /// The bundle evaluated in the sandbox since its last reset, and nothing
    /// else: no script has run in it since. `None` whenever the sandbox is
    /// replaced or used.
    preloaded: Option<Arc<Bundle>>,
    /// The bundle of the last render, the one to preload.
    last_bundle: Option<Arc<Bundle>>,
}

impl Worker {
    fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            sandbox: None,
            preloaded: None,
            last_bundle: None,
        }
    }

    fn run(mut self, jobs: &Mutex<Receiver<Job>>) {
        // Warm up before the first job arrives.
        let _ = self.sandbox();
        loop {
            // A waiting job goes first; otherwise preload while idle. If
            // another worker holds the queue, it is waiting for the next job
            // itself, so there is nothing for this one yet.
            let waiting = match jobs.try_lock() {
                Ok(rx) => rx.try_recv(),
                Err(TryLockError::Poisoned(rx)) => rx.into_inner().try_recv(),
                Err(TryLockError::WouldBlock) => Err(TryRecvError::Empty),
            };
            let job = match waiting {
                Ok(job) => Ok(job),
                Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => {
                    self.preload();
                    self.shared.idle_workers.fetch_add(1, Ordering::SeqCst);
                    let job = {
                        let rx = jobs.lock().unwrap_or_else(PoisonError::into_inner);
                        rx.recv()
                    };
                    self.shared.idle_workers.fetch_sub(1, Ordering::SeqCst);
                    job
                }
            };
            let Ok(job) = job else {
                return;
            };
            match job {
                Job::Render(job) => {
                    let result = self.guard(|w| w.render(&job));
                    self.shared.renders.fetch_add(1, Ordering::Relaxed);
                    if result.is_err() {
                        self.shared.failed_renders.fetch_add(1, Ordering::Relaxed);
                    }
                    let _ = job.reply.send(result);
                }
                Job::Register(job) => {
                    let result = self.guard(|w| w.register(&job));
                    let _ = job.reply.send(result);
                }
            }
            // The caller has its answer; now clear the job's state.
            self.recycle();
        }
    }

    /// Runs `f`, turning a panic (e.g. in a text measurer) into an error
    /// instead of killing the worker.
    fn guard<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, IslandError>,
    ) -> Result<T, IslandError> {
        let result = catch_unwind(AssertUnwindSafe(|| f(self)));
        result.unwrap_or_else(|panic| {
            // The runtime may be mid-call; never reuse it.
            self.sandbox = None;
            self.preloaded = None;
            let message = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a panic with no message".to_owned());
            Err(IslandError::Worker {
                message: format!("panicked: {message}"),
            })
        })
    }

    /// The worker's sandbox, with a fresh context, created on first use.
    fn sandbox(&mut self) -> Result<&mut Sandbox, IslandError> {
        let sb = match self.sandbox.take() {
            Some(sb) => sb,
            None => {
                self.preloaded = None;
                Sandbox::with_options(self.shared.sandbox_options.clone())?
            }
        };
        Ok(self.sandbox.insert(sb))
    }

    /// Gives the sandbox a fresh context for the next job, freeing this
    /// job's heap. A sandbox that can't reset is dropped and rebuilt later.
    fn recycle(&mut self) {
        self.preloaded = None;
        if let Some(sb) = self.sandbox.as_mut() {
            if sb.reset().is_err() {
                self.sandbox = None;
            }
        }
    }

    /// Evaluates the last render's bundle in the fresh sandbox, so the next
    /// render of the same renderer can skip that step. Runs under the
    /// sandbox's limits; if it fails, the sandbox is reset again and the
    /// next render evaluates the bundle itself.
    fn preload(&mut self) {
        let Some(bundle) = self.last_bundle.clone() else {
            return;
        };
        if self.preloaded.is_some() || !bundle.preloadable.load(Ordering::Relaxed) {
            return;
        }
        // Ok(true): loaded with nothing queued, so a render can start
        // straight at the entry, exactly as if it had evaluated the bundle
        // itself in the same call.
        let loaded = catch_unwind(AssertUnwindSafe(|| {
            self.sandbox().and_then(|sb| {
                Ok(sb.run(|ctx| {
                    bundle.bytecode.eval(ctx)?;
                    Ok(!ctx.execute_pending_job())
                })?)
            })
        }));
        match loaded {
            Ok(Ok(true)) => self.preloaded = Some(bundle),
            Ok(Ok(false)) => {
                bundle.preloadable.store(false, Ordering::Relaxed);
                self.recycle();
            }
            Ok(Err(_)) => self.recycle(),
            // The runtime may be mid-call; never reuse it.
            Err(_) => self.sandbox = None,
        }
    }

    /// Takes the preloaded bundle, if any, leaving a sandbox that is either
    /// pristine or holds exactly `wanted`. Returns whether `wanted` (if
    /// given) is already loaded. A sandbox holding another bundle is reset,
    /// so no job ever sees a bundle it did not ask for.
    fn take_preloaded(&mut self, wanted: Option<&Arc<Bundle>>) -> bool {
        let Some(loaded) = self.preloaded.take() else {
            return false;
        };
        if wanted.is_some_and(|w| Arc::ptr_eq(w, &loaded)) {
            return true;
        }
        self.recycle();
        false
    }

    fn register(&mut self, job: &RegisterJob) -> Result<Bundle, IslandError> {
        self.take_preloaded(None);
        let sb = self.sandbox()?;
        let bytecode = sb.compile(&job.source, &format!("{}.js", job.name))?;
        sb.eval_bytecode(&bytecode)?;
        let entry = job.entry.clone();
        let is_function = sb.run(move |ctx| {
            let v: Value<'_> = ctx.globals().get(entry.as_str())?;
            Ok(v.is_function())
        })?;
        if !is_function {
            return Err(IslandError::MissingEntry {
                bundle: job.name.clone(),
                entry: job.entry.clone(),
            });
        }
        Ok(Bundle {
            name: job.name.clone(),
            entry: job.entry.clone(),
            bytecode,
            preloadable: AtomicBool::new(true),
        })
    }

    fn render(&mut self, job: &RenderJob) -> Result<String, IslandError> {
        let measurer = self.shared.measurer.clone();
        let bundle = &job.bundle;
        // The sandbox is used from here on, so it is no longer pristine.
        let preloaded = self.take_preloaded(Some(bundle));
        self.last_bundle = Some(bundle.clone());
        if preloaded {
            self.shared
                .preloaded_renders
                .fetch_add(1, Ordering::Relaxed);
        }
        let sb = self.sandbox()?;
        // One call, so one deadline covers loading the bundle and the
        // render. A preloaded bundle was loaded off the caller's clock with
        // nothing left queued, so starting at the entry is the same script
        // sequence.
        let outcome = sb.run_await(
            |ctx| {
                if !preloaded {
                    bundle.bytecode.eval(ctx)?;
                }
                let entry = entry_function(ctx, &bundle.entry)?;
                let island = Object::new(ctx.clone())?;
                island.set("width", job.width)?;
                island.set("height", job.height)?;
                island.set("measureText", measure_function(ctx, measurer)?)?;
                entry.call((job.spec_json.as_str(), island))
            },
            |_, v| match v.as_string() {
                Some(s) => Ok(Ok(s.to_string()?)),
                None => Ok(Err(v.type_name().to_owned())),
            },
        )?;
        outcome.map_err(|found| IslandError::NotAString {
            renderer: bundle.name.clone(),
            found,
        })
    }
}

/// The bundle's entry function.
fn entry_function<'js>(ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Function<'js>> {
    let v: Value<'js> = ctx.globals().get(name)?;
    let found = v.type_name();
    v.into_function().ok_or_else(|| rquickjs::Error::FromJs {
        from: found,
        to: "function",
        message: Some(format!("the bundle's entry {name:?} is not a function")),
    })
}

/// `ctx.measureText(text, cssFont)` for renderers.
fn measure_function<'js>(
    ctx: &Ctx<'js>,
    measurer: TextMeasurer,
) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              text: Coerced<String>,
              font: Opt<Coerced<String>>|
              -> rquickjs::Result<Object<'js>> {
            let font = font.0.map_or_else(|| "10px sans-serif".to_owned(), |f| f.0);
            let m = measurer
                .measure(&text.0, &font)
                .map_err(|msg| Exception::throw_type(&ctx, &format!("measureText: {msg}")))?;
            let o = Object::new(ctx)?;
            o.set("width", m.width)?;
            o.set("actualBoundingBoxLeft", m.actual_bounding_box_left)?;
            o.set("actualBoundingBoxRight", m.actual_bounding_box_right)?;
            o.set("actualBoundingBoxAscent", m.actual_bounding_box_ascent)?;
            o.set("actualBoundingBoxDescent", m.actual_bounding_box_descent)?;
            o.set("fontBoundingBoxAscent", m.font_bounding_box_ascent)?;
            o.set("fontBoundingBoxDescent", m.font_bounding_box_descent)?;
            Ok(o)
        },
    )?
    .with_name("measureText")
}
