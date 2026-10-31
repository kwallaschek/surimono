//! The island runner: bundles compiled once, rendered on worker threads.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use surimono_core::rquickjs::function::{Opt, This};
use surimono_core::rquickjs::{self, Coerced, Ctx, Exception, Function, Object, Value};
use surimono_core::{Bytecode, Deadline, Limits, Sandbox, SandboxOptions};

use crate::measure::{Measure, MeasureError};
use crate::{IslandError, TextMeasurer, builtins};

/// How an [`IslandRunner`] is set up.
///
/// Build it with `..RunnerOptions::default()` so new fields don't break you.
#[derive(Debug, Clone)]
pub struct RunnerOptions {
    /// Memory, time and stack limits for each render.
    pub limits: Limits,
    /// Most worker threads, each with its own JavaScript runtime. The pool
    /// starts with one and adds a worker only when a request finds every
    /// worker busy, so a caller rendering one island at a time never uses
    /// more than one. 0 picks the default: the number of CPUs, at most 4.
    pub pool_size: usize,
    /// How long a worker may sit idle before it frees its JavaScript
    /// runtime (and the bundle preloaded in it). The next request on that
    /// worker starts a runtime and loads its bundle itself (about 0.5 ms
    /// plus the bundle's load time). `None` keeps runtimes for as long as
    /// the runner lives. Default: 2 s.
    pub release_idle_after: Option<Duration>,
    /// Seed for `Math.random`; every render starts the same sequence.
    pub seed: u64,
    /// Answers `ctx.measureText` for renderers: `TextMeasurer::new(fonts)`
    /// with the fonts the PDF engine prints with. Without it (or with no
    /// fonts in it), a render that measures text fails with
    /// [`IslandError::NoFonts`]; the built-in renderers always measure.
    pub text_measurer: Option<TextMeasurer>,
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
    /// Default for [`release_idle_after`](Self::release_idle_after).
    pub const DEFAULT_RELEASE_IDLE_AFTER: Duration = Duration::from_secs(2);

    fn sandbox_options(&self) -> SandboxOptions {
        SandboxOptions::default()
            .with_limits(self.limits)
            .with_random_seed(self.seed)
            .with_now_ms(self.now_ms)
            .with_utc_offset_minutes(self.utc_offset_minutes)
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
            release_idle_after: Some(Self::DEFAULT_RELEASE_IDLE_AFTER),
            seed: sandbox.random_seed,
            text_measurer: None,
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
    /// For a built-in: the address of its static source, so built-ins
    /// sharing one bundle share its bytecode.
    source_id: Option<usize>,
}

impl Bundle {
    /// Whether evaluating `self` and `other` leaves the same state: they
    /// are one bundle, or built-ins compiled from one source (vega and
    /// vega-lite), which differ only in the entry a render calls.
    fn loads_like(&self, other: &Bundle) -> bool {
        std::ptr::eq(self, other) || (self.source_id.is_some() && self.source_id == other.source_id)
    }
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
    code: Code,
    entry: String,
    reply: SyncSender<Result<Bundle, IslandError>>,
}

/// What a registration compiles, or reuses.
enum Code {
    Source(String),
    /// Bytecode another bundle already compiled from the same source.
    Compiled(Bytecode),
}

enum Job {
    Render(RenderJob),
    Register(RegisterJob),
}

/// What every worker needs.
struct Shared {
    sandbox_options: SandboxOptions,
    measure: Measure,
    /// See [`RunnerOptions::release_idle_after`].
    release_idle_after: Option<Duration>,
    queue: Queue,
    renders: AtomicU64,
    failed_renders: AtomicU64,
    preloaded_renders: AtomicU64,
    /// Workers holding a JavaScript runtime right now.
    runtimes: AtomicUsize,
}

/// The jobs waiting for a worker, shared by the runner and its workers.
struct Queue {
    state: Mutex<QueueState>,
    /// Signalled when a job arrives or the queue closes.
    ready: Condvar,
}

#[derive(Default)]
struct QueueState {
    jobs: VecDeque<Job>,
    /// Set when the runner is dropped: workers finish and exit.
    closed: bool,
    /// Jobs sent and not yet answered (waiting or running).
    in_flight: usize,
    /// Workers waiting for a job.
    idle: usize,
}

/// What a worker got from the queue.
enum Next {
    Job(Job),
    /// Nothing arrived in time.
    Empty,
    /// The runner is gone.
    Closed,
}

impl Queue {
    fn new() -> Self {
        Self {
            state: Mutex::new(QueueState::default()),
            ready: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, QueueState> {
        // Nothing panics while holding the lock, and the state is
        // consistent whenever it is released.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds a job; returns the number of jobs now in flight, or the job
    /// back if the queue is closed.
    fn push(&self, job: Job) -> Result<usize, Job> {
        let mut q = self.lock();
        if q.closed {
            return Err(job);
        }
        q.jobs.push_back(job);
        q.in_flight += 1;
        let in_flight = q.in_flight;
        drop(q);
        self.ready.notify_one();
        Ok(in_flight)
    }

    /// A waiting job, without waiting.
    fn try_pop(&self) -> Next {
        let mut q = self.lock();
        match q.jobs.pop_front() {
            Some(job) => Next::Job(job),
            None if q.closed => Next::Closed,
            None => Next::Empty,
        }
    }

    /// Waits for a job, at most `timeout` if given, counted as idle while
    /// waiting.
    fn pop(&self, timeout: Option<Duration>) -> Next {
        let mut q = self.lock();
        let deadline = timeout.and_then(|t| std::time::Instant::now().checked_add(t));
        q.idle += 1;
        let next = loop {
            if let Some(job) = q.jobs.pop_front() {
                break Next::Job(job);
            }
            if q.closed {
                break Next::Closed;
            }
            match deadline {
                None => q = self.ready.wait(q).unwrap_or_else(PoisonError::into_inner),
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(std::time::Instant::now());
                    if left.is_zero() {
                        break Next::Empty;
                    }
                    q = self
                        .ready
                        .wait_timeout(q, left)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
            }
        };
        q.idle -= 1;
        next
    }

    /// A job is answered (called just before the reply is sent, so a
    /// caller that sends its next job right away sees it as done).
    fn done(&self) {
        let mut q = self.lock();
        q.in_flight = q.in_flight.saturating_sub(1);
    }

    fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_all();
    }
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
    /// preloaded (or released). Equal to `workers` when the runner is quiet.
    pub idle_workers: usize,
    /// Worker threads started so far: one at first, more only when
    /// requests overlap, up to the pool size.
    pub workers: usize,
    /// Workers holding a JavaScript runtime right now. A worker frees its
    /// runtime after [`RunnerOptions::release_idle_after`] without work.
    pub runtimes: usize,
}

/// Renders islands: JavaScript bundles that turn a JSON spec into an SVG
/// string.
///
/// A bundle is a classic script that defines a global function
/// `entry(specJson, ctx)` returning an SVG string (or a Promise of one).
/// `specJson` is the island's spec as JSON text; `ctx` has `width`, `height`
/// and `measureText(text, cssFont)`, which returns canvas-style metrics
/// (`width`, `actualBoundingBoxAscent`, …) from the runner's
/// [`TextMeasurer`]. If the runner has no fonts, `measureText` throws a
/// `TypeError` and the render fails with [`IslandError::NoFonts`].
///
/// Bundles are compiled to bytecode once, at registration, and loaded from
/// bytecode for each render (no re-parsing). Each render runs on a worker
/// thread, in a fresh JavaScript runtime under the runner's limits, so
/// nothing leaks from one render to the next, the whole heap is freed after
/// every render, and the same request always gives the same SVG.
///
/// The pool starts with one worker and adds one (up to
/// [`RunnerOptions::pool_size`]) only when a request finds every worker
/// busy, so memory follows how many islands are rendered at once, not the
/// number of CPUs. Each worker prepares the runtime for its next render
/// right after sending a reply, off the caller's clock: it starts a fresh
/// runtime and, if no request is waiting, evaluates the bundle it rendered
/// last, so a repeated renderer skips that step. (A bundle whose top level
/// queues microtasks is never preloaded.) A request that arrives while its
/// worker is preloading waits for the preload to finish. A worker idle for
/// [`RunnerOptions::release_idle_after`] frees its runtime. The limits apply to each
/// island as a whole: one time budget covers loading the bundle, if it
/// wasn't preloaded, the entry call and the microtasks it queues. The
/// result is the same either way, but a cold render spends part of its
/// budget loading the bundle (about 20 ms for ECharts, 70 ms for Vega in a
/// release build), so set the time limit for the cold case: with a limit
/// close to a chart's cost, a cold render can hit it where a preloaded one
/// passes. Charts with thousands of marks (a 2,000-point Vega-Lite scatter
/// takes about 0.5 to 1.2 s) may need more than the default 2 s.
///
/// Text measurement runs inside that budget, on the worker thread, with
/// the worker's own measuring state (the fonts are shared). The engine
/// checks the time limit only every few thousand operations and can't
/// interrupt Rust code, so `measureText` is bounded twice over: it refuses
/// to start once the limit has passed (the render then fails with the time
/// limit error), and [`TextMeasurer`] refuses text over
/// [`surimono_gfx::MAX_TEXT_BYTES`], which bounds a single call.
///
/// `IslandRunner` is `Send + Sync`: share it between threads (e.g. in an
/// `Arc`); [`render`](Self::render) blocks the calling thread until a worker
/// is done.
pub struct IslandRunner {
    shared: Arc<Shared>,
    bundles: RwLock<HashMap<String, Arc<Bundle>>>,
    /// Held while a built-in renderer is compiled on first use.
    builtin_lock: Mutex<()>,
    /// Most worker threads.
    pool_size: usize,
    /// Stack size of each worker thread.
    stack_size: usize,
    /// The workers started so far (at least one).
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl fmt::Debug for IslandRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IslandRunner")
            .field("pool_size", &self.pool_size)
            .field("workers", &self.worker_count())
            .field("renderers", &self.renderers())
            .finish_non_exhaustive()
    }
}

impl IslandRunner {
    /// Starts the worker pool. No bundles are registered yet.
    pub fn new(mut opts: RunnerOptions) -> Result<Self, IslandError> {
        let measure = Measure::new(opts.text_measurer.take());
        Self::start(opts, measure)
    }

    /// **Not public API**, behind the `test-estimate` feature: for
    /// surimono's own tests and tools. Like [`new`](Self::new), but
    /// measures text with a deterministic font-free estimate (0.6 em per
    /// character, 1 em for CJK) instead of fonts, ignoring
    /// `opts.text_measurer`. Chart labels laid out with it do not fit
    /// printed text.
    #[cfg(feature = "test-estimate")]
    pub fn with_estimated_text_metrics(mut opts: RunnerOptions) -> Result<Self, IslandError> {
        opts.text_measurer = None;
        Self::start(opts, Measure::Estimate)
    }

    /// Starts the worker pool with `measure` answering `ctx.measureText`.
    fn start(opts: RunnerOptions, measure: Measure) -> Result<Self, IslandError> {
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
        // (text measurement, shaping). Untouched stack costs no memory: a
        // worker's resident stack is the deepest it has been, typically
        // well under 1 MiB.
        let stack_size = sandbox_options.limits.stack_bytes + 8 * 1024 * 1024;
        let shared = Arc::new(Shared {
            sandbox_options,
            measure,
            release_idle_after: opts.release_idle_after,
            queue: Queue::new(),
            renders: AtomicU64::new(0),
            failed_renders: AtomicU64::new(0),
            preloaded_renders: AtomicU64::new(0),
            runtimes: AtomicUsize::new(0),
        });
        let runner = Self {
            shared,
            bundles: RwLock::new(HashMap::new()),
            builtin_lock: Mutex::new(()),
            pool_size,
            stack_size,
            workers: Mutex::new(Vec::with_capacity(pool_size)),
        };
        // One worker from the start; the rest only when requests overlap.
        let first = runner.spawn_worker(0)?;
        runner.workers_lock().push(first);
        Ok(runner)
    }

    fn workers_lock(&self) -> MutexGuard<'_, Vec<JoinHandle<()>>> {
        self.workers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn worker_count(&self) -> usize {
        self.workers_lock().len()
    }

    fn spawn_worker(&self, index: usize) -> Result<JoinHandle<()>, IslandError> {
        let shared = self.shared.clone();
        thread::Builder::new()
            .name(format!("surimono-island-{index}"))
            .stack_size(self.stack_size)
            .spawn(move || Worker::new(shared).run())
            .map_err(|e| IslandError::Worker {
                message: format!("could not start a worker thread: {e}"),
            })
    }

    /// Starts another worker if more jobs are in flight than there are
    /// workers and the pool isn't full. A worker that can't start is not an
    /// error: the job waits for a running one.
    fn grow(&self) {
        let mut workers = self.workers_lock();
        let in_flight = self.shared.queue.lock().in_flight;
        if in_flight > workers.len() && workers.len() < self.pool_size {
            if let Ok(handle) = self.spawn_worker(workers.len()) {
                workers.push(handle);
            }
        }
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
        let bundle = self.compile_bundle(name, Code::Source(js_source.to_owned()), entry)?;
        self.bundles
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), Arc::new(bundle));
        Ok(())
    }

    /// Compiles (or reuses) `code` on a worker and checks it defines `entry`.
    fn compile_bundle(&self, name: &str, code: Code, entry: &str) -> Result<Bundle, IslandError> {
        self.dispatch(|reply| {
            Job::Register(RegisterJob {
                name: name.to_owned(),
                code,
                entry: entry.to_owned(),
                reply,
            })
        })
    }

    /// Renders one island and returns the renderer's SVG string.
    ///
    /// A built-in renderer (see [`builtin_renderers`](crate::builtin_renderers))
    /// is compiled on its first use, unless a bundle was registered under
    /// its name.
    pub fn render(&self, req: &IslandRequest<'_>) -> Result<String, IslandError> {
        let bundle = self.bundle(req.renderer)?;
        serde_json::from_str::<serde::de::IgnoredAny>(req.spec_json).map_err(bad_json)?;
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
        // Built-ins that share a source (vega and vega-lite) share its
        // bytecode: compile it once.
        let shared = self
            .bundles
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .find(|b| b.source_id == Some(builtin.source.as_ptr() as usize))
            .map(|b| b.bytecode.clone());
        let code = match shared {
            Some(bytecode) => Code::Compiled(bytecode),
            None => Code::Source(builtin.source.to_owned()),
        };
        let mut bundle = self.compile_bundle(builtin.name, code, builtin.entry)?;
        bundle.source_id = Some(builtin.source.as_ptr() as usize);
        // A bundle registered under this name while we compiled wins.
        let mut bundles = self.bundles.write().unwrap_or_else(PoisonError::into_inner);
        let bundle = bundles
            .entry(name.to_owned())
            .or_insert_with(|| Arc::new(bundle));
        Ok(bundle.clone())
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

    /// Most worker threads this runner starts (see [`RunnerStats::workers`]
    /// for how many it has started).
    pub fn pool_size(&self) -> usize {
        self.pool_size
    }

    /// Counters since the runner started.
    pub fn stats(&self) -> RunnerStats {
        let workers = self.worker_count();
        RunnerStats {
            renders: self.shared.renders.load(Ordering::Relaxed),
            failed_renders: self.shared.failed_renders.load(Ordering::Relaxed),
            preloaded_renders: self.shared.preloaded_renders.load(Ordering::Relaxed),
            idle_workers: self.shared.queue.lock().idle,
            workers,
            runtimes: self.shared.runtimes.load(Ordering::Relaxed),
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
        let in_flight = self.shared.queue.push(make(reply)).map_err(|_| gone())?;
        // Cheap check first; grow() decides under the lock.
        if in_flight > 1 {
            self.grow();
        }
        answer.recv().map_err(|_| gone())?
    }
}

impl Drop for IslandRunner {
    fn drop(&mut self) {
        // Closing the queue ends the workers once they finish their job.
        self.shared.queue.close();
        for w in self.workers_lock().drain(..) {
            let _ = w.join();
        }
    }
}

/// One worker thread's state.
struct Worker {
    shared: Arc<Shared>,
    /// This worker's own measurer (see [`Measure::for_worker`]).
    measure: Measure,
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

impl Drop for Worker {
    fn drop(&mut self) {
        self.drop_sandbox();
    }
}

impl Worker {
    fn new(shared: Arc<Shared>) -> Self {
        Self {
            measure: shared.measure.for_worker(),
            shared,
            sandbox: None,
            preloaded: None,
            last_bundle: None,
        }
    }

    fn run(mut self) {
        let shared = self.shared.clone();
        let queue = &shared.queue;
        // Warm up before the first job arrives.
        let _ = self.sandbox();
        loop {
            // A waiting job goes first; otherwise preload while idle, wait,
            // and free the runtime if nothing comes for a while.
            let next = match queue.try_pop() {
                Next::Empty => {
                    self.preload();
                    match queue.pop(self.shared.release_idle_after) {
                        Next::Empty => {
                            self.release();
                            queue.pop(None)
                        }
                        next => next,
                    }
                }
                next => next,
            };
            let job = match next {
                Next::Job(job) => job,
                Next::Empty | Next::Closed => return,
            };
            match job {
                Job::Render(job) => {
                    let result = self.guard(|w| w.render(&job));
                    self.shared.renders.fetch_add(1, Ordering::Relaxed);
                    if result.is_err() {
                        self.shared.failed_renders.fetch_add(1, Ordering::Relaxed);
                    }
                    queue.done();
                    let _ = job.reply.send(result);
                }
                Job::Register(job) => {
                    let result = self.guard(|w| w.register(&job));
                    queue.done();
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
            self.drop_sandbox();
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
                let sb = Sandbox::with_options(self.shared.sandbox_options.clone())?;
                self.shared.runtimes.fetch_add(1, Ordering::Relaxed);
                sb
            }
        };
        Ok(self.sandbox.insert(sb))
    }

    /// Drops the sandbox and whatever is loaded in it.
    fn drop_sandbox(&mut self) {
        self.preloaded = None;
        if self.sandbox.take().is_some() {
            self.shared.runtimes.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Frees the runtime of a worker that has been idle for a while; the
    /// next job starts a new one.
    fn release(&mut self) {
        self.drop_sandbox();
    }

    /// Gives the sandbox a fresh context for the next job, freeing this
    /// job's heap. A sandbox that can't reset is dropped and rebuilt later.
    fn recycle(&mut self) {
        self.preloaded = None;
        if let Some(sb) = self.sandbox.as_mut() {
            if sb.reset().is_err() {
                self.drop_sandbox();
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
            Err(_) => self.drop_sandbox(),
        }
    }

    /// Takes the preloaded bundle, if any, leaving a sandbox that is either
    /// pristine or holds exactly what evaluating `wanted` would leave (see
    /// [`Bundle::loads_like`]). Returns whether `wanted` (if given) is
    /// already loaded. A sandbox holding another bundle is reset,
    /// so no job ever sees a bundle it did not ask for.
    fn take_preloaded(&mut self, wanted: Option<&Arc<Bundle>>) -> bool {
        let Some(loaded) = self.preloaded.take() else {
            return false;
        };
        if wanted.is_some_and(|w| w.loads_like(&loaded)) {
            return true;
        }
        self.recycle();
        false
    }

    fn register(&mut self, job: &RegisterJob) -> Result<Bundle, IslandError> {
        self.take_preloaded(None);
        let sb = self.sandbox()?;
        let bytecode = match &job.code {
            Code::Source(source) => sb.compile(source, &format!("{}.js", job.name))?,
            Code::Compiled(bytecode) => bytecode.clone(),
        };
        // The entry must exist right after the bundle runs, before any
        // microtask: that is when a render looks it up.
        let entry = job.entry.clone();
        let is_function = sb.run(|ctx| {
            bytecode.eval(ctx)?;
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
            source_id: None,
        })
    }

    fn render(&mut self, job: &RenderJob) -> Result<String, IslandError> {
        let measurer = self.measure.clone();
        let bundle = &job.bundle;
        // The sandbox is used from here on, so it is no longer pristine.
        let preloaded = self.take_preloaded(Some(bundle));
        self.last_bundle = Some(bundle.clone());
        if preloaded {
            self.shared
                .preloaded_renders
                .fetch_add(1, Ordering::Relaxed);
        }
        let no_fonts = Arc::new(AtomicBool::new(false));
        let measure_text = (measurer, no_fonts.clone());
        let sb = self.sandbox()?;
        let deadline = sb.deadline();
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
                let (measurer, no_fonts) = measure_text;
                island.set(
                    "measureText",
                    measure_function(ctx, measurer, no_fonts, deadline)?,
                )?;
                entry.call((job.spec_json.as_str(), island))
            },
            svg_string,
        );
        // Measured without fonts: whatever the script made of the
        // TypeError (even if it caught it), the layout is wrong.
        if no_fonts.load(Ordering::Relaxed) {
            return Err(IslandError::NoFonts {
                renderer: bundle.name.clone(),
            });
        }
        outcome?.map_err(|found| IslandError::NotAString {
            renderer: bundle.name.clone(),
            found,
        })
    }
}

/// A spec that isn't JSON, with the parser's position as numbers.
fn bad_json(e: serde_json::Error) -> IslandError {
    // serde_json's Display ends with " at line L column C"; keep only the
    // message, the position goes in its own fields.
    let text = e.to_string();
    let suffix = format!(" at line {} column {}", e.line(), e.column());
    let message = text.strip_suffix(&suffix).unwrap_or(&text).to_owned();
    IslandError::BadJson {
        message,
        line: e.line(),
        column: e.column(),
    }
}

/// The entry's result as a Rust string, or the JavaScript type it was.
///
/// A JavaScript string can hold lone surrogates, which no UTF-8 string
/// can. Each one becomes U+FFFD, as `String.prototype.toWellFormed` does
/// and as encoding the page for print would.
fn svg_string<'js>(ctx: &Ctx<'js>, v: Value<'js>) -> rquickjs::Result<Result<String, String>> {
    let Some(s) = v.as_string() else {
        return Ok(Err(v.type_name().to_owned()));
    };
    match s.to_string() {
        Err(rquickjs::Error::Utf8(_)) => {}
        other => return other.map(Ok),
    }
    let to_well_formed: Function<'js> = ctx
        .globals()
        .get::<_, Object<'js>>("String")?
        .get::<_, Object<'js>>("prototype")?
        .get("toWellFormed")?;
    let fixed: Value<'js> = to_well_formed.call((This(v),))?;
    match fixed.as_string().map(|s| s.to_string()) {
        Some(Ok(s)) => Ok(Ok(s)),
        // Only a bundle that replaced toWellFormed gets here.
        _ => Ok(Err("a string that is not valid Unicode".to_owned())),
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
///
/// Sets `no_fonts` if it was called with no fonts to measure with. Refuses
/// to measure once the render's time limit has passed: shaping is
/// expensive, and the engine checks the deadline only every few thousand
/// operations, so a loop of calls could otherwise run far past the limit.
fn measure_function<'js>(
    ctx: &Ctx<'js>,
    measurer: Measure,
    no_fonts: Arc<AtomicBool>,
    deadline: Deadline,
) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              text: Coerced<String>,
              font: Opt<Coerced<String>>|
              -> rquickjs::Result<Object<'js>> {
            if deadline.passed() {
                // The render now fails with TimeLimit, whatever the script does.
                return Err(Exception::throw_internal(
                    &ctx,
                    "measureText: the time limit has passed",
                ));
            }
            let font = font.0.map_or_else(|| "10px sans-serif".to_owned(), |f| f.0);
            let m = measurer.measure(&text.0, &font).map_err(|e| {
                let msg = match e {
                    MeasureError::NoFonts => {
                        no_fonts.store(true, Ordering::Relaxed);
                        "no fonts were given to measure text with".to_owned()
                    }
                    MeasureError::Other(msg) => msg,
                };
                Exception::throw_type(&ctx, &format!("measureText: {msg}"))
            })?;
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::TextMetrics;

    /// A runner whose measurer panics on "boom", fails on "fail" and
    /// otherwise reports width 1 per byte.
    fn runner_with_odd_measurer() -> IslandRunner {
        let measure = Measure::Custom(Arc::new(|text: &str, _: &str| match text {
            "boom" => panic!("measurer exploded"),
            "fail" => Err("no such font".to_owned()),
            _ => Ok(TextMetrics {
                width: text.len() as f64,
                ..TextMetrics::default()
            }),
        }));
        let r = IslandRunner::start(
            RunnerOptions {
                pool_size: 1,
                ..RunnerOptions::default()
            },
            measure,
        )
        .unwrap();
        r.register_bundle(
            "m",
            "function entry(s, ctx) { return String(ctx.measureText(JSON.parse(s).t).width) }",
            "entry",
        )
        .unwrap();
        r
    }

    fn measure(r: &IslandRunner, text: &str) -> Result<String, IslandError> {
        r.render(&IslandRequest {
            renderer: "m",
            spec_json: &format!(r#"{{"t":"{text}"}}"#),
            width: 1,
            height: 1,
        })
    }

    #[test]
    fn measurer_errors_become_type_errors() {
        let r = runner_with_odd_measurer();
        match measure(&r, "fail").unwrap_err() {
            IslandError::Sandbox(surimono_core::SandboxError::Exception { message, .. }) => {
                assert_eq!(message, "TypeError: measureText: no such font")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn measurer_panics_do_not_kill_the_pool() {
        let r = runner_with_odd_measurer();
        assert_eq!(measure(&r, "abc").unwrap(), "3");
        match measure(&r, "boom").unwrap_err() {
            IslandError::Worker { message } => {
                assert!(message.contains("measurer exploded"), "{message}")
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(measure(&r, "abcd").unwrap(), "4");
        let stats = r.stats();
        assert_eq!((stats.renders, stats.failed_renders), (3, 1));
    }
}
