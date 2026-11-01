//! The island runner: bundles compiled once (built-ins at build time), rendered on worker threads.

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
use crate::{IslandError, TextMeasurer, builtins, embedded};

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
    /// Most worker threads.
    pool_size: usize,
    /// Stack size of each worker thread.
    stack_size: usize,
    /// Handles of the worker threads started and not yet seen finished, to
    /// join when the runner is dropped. Taken before `queue`'s lock when
    /// both are held. How many are alive is `QueueState::live`.
    workers: Mutex<Vec<JoinHandle<()>>>,
    /// Worker threads started so far, for their names.
    spawned: AtomicUsize,
    /// Test-only: makes workers die outside their guard (see [`Fault`]).
    #[cfg(test)]
    fault: std::sync::atomic::AtomicU8,
}

/// Test-only ways for a worker thread to die outside [`Worker::guard`], as
/// a bug there would make it.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Fault {
    None = 0,
    /// The next worker to finish a job dies after replying (once).
    AfterReply = 1,
    /// The next worker to take a job dies holding it (once).
    HoldingJob = 2,
    /// Every worker dies as it starts, before taking a job.
    AtStart = 3,
}

impl Shared {
    fn workers_lock(&self) -> MutexGuard<'_, Vec<JoinHandle<()>>> {
        // Nothing panics while holding the lock.
        self.workers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts a worker thread, counting it live in `q`. The caller holds
    /// `workers` (passed in) and the queue lock (`q`), so the new thread's
    /// handle is listed before the thread can look for it, and the live
    /// count is right whenever the queue lock is free.
    fn spawn_worker(
        self: &Arc<Self>,
        workers: &mut Vec<JoinHandle<()>>,
        q: &mut QueueState,
    ) -> Result<(), IslandError> {
        // Handles of finished threads are dropped (detached) here, so a
        // pool whose workers die and are replaced keeps a bounded list.
        workers.retain(|h| !h.is_finished());
        let index = self.spawned.fetch_add(1, Ordering::Relaxed);
        let shared = self.clone();
        let handle = thread::Builder::new()
            .name(format!("surimono-island-{index}"))
            .stack_size(self.stack_size)
            .spawn(move || {
                // First, so that whatever ends this thread, it is counted
                // out and its job and queue are seen to.
                let life = Life::new(shared);
                Worker::new(life).run()
            })
            .map_err(|e| IslandError::Worker {
                message: format!("could not start a worker thread: {e}"),
            })?;
        workers.push(handle);
        q.live += 1;
        Ok(())
    }

    #[cfg(test)]
    fn fault(&self, fault: Fault, once: bool) -> bool {
        let now = fault as u8;
        if once {
            self.fault
                .compare_exchange(now, Fault::None as u8, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        } else {
            self.fault.load(Ordering::Acquire) == now
        }
    }
}

/// A worker thread's life, from its first line to its end however it ends
/// (returning, or a panic that escaped [`Worker::guard`]). When it ends
/// before the runner closes the queue, the worker died: [`Drop`] answers
/// the job it held, starts a replacement if jobs are waiting and the dead
/// worker had taken one (a worker that dies before taking any could die
/// the same way again, so it is never replaced in a loop), and otherwise,
/// if no worker is left, fails every waiting job with
/// [`IslandError::Worker`]. The next request starts a new worker. Callers
/// never wait on a queue nobody serves.
struct Life {
    shared: Arc<Shared>,
    /// Holding a job that is not yet answered (counted in `in_flight`).
    in_job: bool,
    /// Jobs answered.
    served: u64,
}

impl Life {
    fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            in_job: false,
            served: 0,
        }
    }
}

impl Drop for Life {
    fn drop(&mut self) {
        let shared = &self.shared;
        let mut workers = shared.workers_lock();
        let mut q = shared.queue.lock();
        q.live = q.live.saturating_sub(1);
        if self.in_job {
            // Its reply sender is already dropped: that caller has its
            // error (see `IslandRunner::dispatch`).
            q.in_flight = q.in_flight.saturating_sub(1);
        }
        if q.closed {
            return;
        }
        // A worker that took a job can be replaced: each such death uses
        // up a job, so replacements can't loop on their own.
        let took_a_job = self.in_job || self.served > 0;
        let waiting = q.in_flight > q.live;
        if waiting
            && took_a_job
            && q.live < shared.pool_size
            && shared.spawn_worker(&mut workers, &mut q).is_ok()
        {
            return;
        }
        if q.live > 0 {
            // The others take the waiting jobs.
            return;
        }
        let orphans: Vec<Job> = q.jobs.drain(..).collect();
        q.in_flight = q.in_flight.saturating_sub(orphans.len());
        drop(q);
        drop(workers);
        let error = || IslandError::Worker {
            message: "the worker thread stopped and no other worker could take this request".into(),
        };
        for job in orphans {
            match job {
                Job::Render(job) => {
                    let _ = job.reply.send(Err(error()));
                }
                Job::Register(job) => {
                    let _ = job.reply.send(Err(error()));
                }
            }
        }
    }
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
    /// Worker threads started and not yet ended (see [`Life`]).
    live: usize,
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

    /// Adds a job; returns whether more jobs are now in flight than live
    /// workers, or the job back if the queue is closed.
    fn push(&self, job: Job) -> Result<bool, Job> {
        let mut q = self.lock();
        if q.closed {
            return Err(job);
        }
        q.jobs.push_back(job);
        q.in_flight += 1;
        let short = q.in_flight > q.live;
        drop(q);
        self.ready.notify_one();
        Ok(short)
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
    /// Worker threads running now: one at first, more only when requests
    /// overlap, up to the pool size. A worker thread that died (a bug: a
    /// panic that escaped its guard) no longer counts; it is replaced when
    /// work is waiting for it.
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
/// bytecode for each render (no re-parsing). The built-in renderers were
/// compiled when this crate was built: their bytecode is loaded in place on
/// first use, so the first render in a new process skips parsing and
/// compiling. If the running engine can't read that bytecode, or it wasn't
/// compiled from exactly the bundle source in this binary, the built-in is
/// compiled from source instead, with the same output; setting the
/// environment variable `SURIMONO_PRECOMPILE=0` when building or running
/// does the same, as a debugging escape hatch. Each render runs
/// on a worker thread, in a fresh JavaScript runtime under the runner's
/// limits, so nothing leaks from one render to the next, the whole heap is
/// freed after every render, and the same request always gives the same
/// SVG.
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
/// [`RunnerOptions::release_idle_after`] frees its runtime.
///
/// The limits apply to each island as a whole: one time budget covers
/// loading the bundle, if it wasn't preloaded, the entry call and the
/// microtasks it queues. The result is the same either way, but a cold
/// render spends part of its budget loading the bundle (about 20 to 40 ms
/// for the built-in ECharts or Vega in a release build), so set the time
/// limit for the cold case: with a limit close to a chart's cost, a cold
/// render can hit it where a preloaded one passes. Charts with thousands
/// of marks (a 2,000-point Vega-Lite scatter
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
///
/// A runner belongs to the process that started it. Its worker threads
/// don't survive `fork`, and a thread that didn't may have held one of its
/// locks, so in a forked child a runner from the parent never touches its
/// locks or workers: [`render`](Self::render) and
/// [`register_bundle`](Self::register_bundle) fail at once with
/// [`IslandError::Worker`], [`stats`](Self::stats) counts no workers,
/// [`renderers`](Self::renderers) is empty, and dropping it leaks it (its
/// memory is the parent's, copied on write) instead of joining threads that
/// don't exist. Start a new runner in the child.
pub struct IslandRunner {
    /// Fork generation of the process that started the runner (see
    /// [`surimono_core::fork_generation`]).
    generation: u64,
    shared: Arc<Shared>,
    bundles: RwLock<HashMap<String, Arc<Bundle>>>,
    /// Held while a built-in renderer is loaded or compiled on first use.
    builtin_lock: Mutex<()>,
    /// The fingerprint the built-ins' bytecode was written with (see
    /// `embedded.rs`); replaced only by tests of the fallback.
    embedded_fingerprint: &'static [u8],
}

impl fmt::Debug for IslandRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.in_this_process() {
            return f
                .debug_struct("IslandRunner")
                .field("pool_size", &self.shared.pool_size)
                .field("forked", &true)
                .finish_non_exhaustive();
        }
        f.debug_struct("IslandRunner")
            .field("pool_size", &self.shared.pool_size)
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
            pool_size,
            stack_size,
            workers: Mutex::new(Vec::with_capacity(pool_size)),
            spawned: AtomicUsize::new(0),
            #[cfg(test)]
            fault: std::sync::atomic::AtomicU8::new(Fault::None as u8),
        });
        let generation = surimono_core::fork_generation().map_err(IslandError::Sandbox)?;
        let runner = Self {
            generation,
            shared,
            bundles: RwLock::new(HashMap::new()),
            builtin_lock: Mutex::new(()),
            embedded_fingerprint: builtins::ENGINE_FINGERPRINT,
        };
        // One worker from the start; the rest only when requests overlap.
        {
            let mut workers = runner.shared.workers_lock();
            runner
                .shared
                .spawn_worker(&mut workers, &mut runner.shared.queue.lock())?;
        }
        Ok(runner)
    }

    /// Whether this process started the runner (it isn't a forked child's
    /// copy of a parent's). Two atomic loads, no lock.
    fn in_this_process(&self) -> bool {
        surimono_core::fork_generation().is_ok_and(|g| g == self.generation)
    }

    /// An error if the runner belongs to another process (see the type's
    /// docs on `fork`).
    fn check_process(&self) -> Result<(), IslandError> {
        if self.in_this_process() {
            return Ok(());
        }
        Err(IslandError::Worker {
            message: "this runner was started in another process (before a fork) and its \
                      worker threads don't exist here; start a new IslandRunner in this process"
                .into(),
        })
    }

    /// Live worker threads.
    fn worker_count(&self) -> usize {
        self.shared.queue.lock().live
    }

    /// Starts another worker if more jobs are in flight than there are live
    /// workers and the pool isn't full. A worker that can't start is not an
    /// error while another runs: the job waits for it. With no worker
    /// alive (the last one died), the waiting jobs fail instead of waiting
    /// for nobody.
    fn grow(&self) {
        let shared = &self.shared;
        let mut workers = shared.workers_lock();
        let mut q = shared.queue.lock();
        if q.closed || q.in_flight <= q.live || q.live >= shared.pool_size {
            return;
        }
        let Err(e) = shared.spawn_worker(&mut workers, &mut q) else {
            return;
        };
        if q.live > 0 {
            return;
        }
        let orphans: Vec<Job> = q.jobs.drain(..).collect();
        q.in_flight = q.in_flight.saturating_sub(orphans.len());
        drop(q);
        drop(workers);
        for job in orphans {
            match job {
                Job::Render(job) => {
                    let _ = job.reply.send(Err(e.clone()));
                }
                Job::Register(job) => {
                    let _ = job.reply.send(Err(e.clone()));
                }
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
        self.check_process()?;
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
    /// is loaded on its first use, unless a bundle was registered under its
    /// name.
    pub fn render(&self, req: &IslandRequest<'_>) -> Result<String, IslandError> {
        self.check_process()?;
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

    /// The bundle for `name`: a registered one, or a built-in loaded (or
    /// compiled) now.
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
        // One load or compile per built-in, even if many threads ask at once.
        let _compiling = self
            .builtin_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(bundle) = registered(&self.bundles) {
            return Ok(bundle);
        }
        let source_id = Some(builtin.source.as_ptr() as usize);
        let bundle = match embedded::bytecode(&builtin, self.embedded_fingerprint) {
            // Bytecode built into the binary from our own bundle: loaded in
            // place, no compile. This crate's tests check that it defines
            // its entry, so it skips the registration run.
            Some(bytecode) => Bundle {
                name: builtin.name.to_owned(),
                entry: builtin.entry.to_owned(),
                bytecode,
                preloadable: AtomicBool::new(true),
                source_id,
            },
            // This engine can't read it: compile the source, like any
            // registered bundle. Built-ins that share a source (vega and
            // vega-lite) share its bytecode: compile it once.
            None => {
                let shared = self
                    .bundles
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .values()
                    .find(|b| b.source_id == source_id)
                    .map(|b| b.bytecode.clone());
                let code = match shared {
                    Some(bytecode) => Code::Compiled(bytecode),
                    None => Code::Source(builtin.source.to_owned()),
                };
                let mut bundle = self.compile_bundle(builtin.name, code, builtin.entry)?;
                bundle.source_id = source_id;
                bundle
            }
        };
        // A bundle registered under this name while we compiled wins.
        let mut bundles = self.bundles.write().unwrap_or_else(PoisonError::into_inner);
        let bundle = bundles
            .entry(name.to_owned())
            .or_insert_with(|| Arc::new(bundle));
        Ok(bundle.clone())
    }

    /// Names of the registered renderers, sorted. Built-in renderers are
    /// listed once they have been used. Empty in a forked child (see the
    /// type's docs on `fork`).
    pub fn renderers(&self) -> Vec<String> {
        if !self.in_this_process() {
            return Vec::new();
        }
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
    /// for how many are running).
    pub fn pool_size(&self) -> usize {
        self.shared.pool_size
    }

    /// Counters since the runner started. In a forked child (see the
    /// type's docs on `fork`), the counters as they were at the fork, with
    /// no workers, idle workers or runtimes: none exist in that process.
    pub fn stats(&self) -> RunnerStats {
        if !self.in_this_process() {
            return RunnerStats {
                renders: self.shared.renders.load(Ordering::Relaxed),
                failed_renders: self.shared.failed_renders.load(Ordering::Relaxed),
                preloaded_renders: self.shared.preloaded_renders.load(Ordering::Relaxed),
                ..RunnerStats::default()
            };
        }
        let (idle_workers, workers) = {
            let q = self.shared.queue.lock();
            (q.idle, q.live)
        };
        RunnerStats {
            renders: self.shared.renders.load(Ordering::Relaxed),
            failed_renders: self.shared.failed_renders.load(Ordering::Relaxed),
            preloaded_renders: self.shared.preloaded_renders.load(Ordering::Relaxed),
            idle_workers,
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
        let short = self
            .shared
            .queue
            .push(make(reply))
            .map_err(|_| IslandError::Worker {
                message: "the worker pool has stopped".into(),
            })?;
        // Cheap check first; grow() decides under the lock.
        if short {
            self.grow();
        }
        // Every job is answered or dropped: by its worker, by a dying
        // worker's `Life` (with an error), or with the worker that died
        // holding it. The queue can't close while `self` is borrowed.
        answer.recv().map_err(|_| IslandError::Worker {
            message: "the worker thread stopped while handling this request".into(),
        })?
    }
}

impl Drop for IslandRunner {
    fn drop(&mut self) {
        if !self.in_this_process() {
            // A forked child's copy: its workers don't exist here, and a
            // thread that didn't survive the fork may have held a lock or
            // been changing the data behind one. Leak all of it, touching
            // nothing: `get_mut` takes no lock, and moving a value out
            // reads none of it. The leaked extra reference keeps `Shared`
            // (the queue, the worker handles and their locks) from ever
            // being dropped here.
            Box::leak(Box::new(Arc::clone(&self.shared)));
            let bundles = self
                .bundles
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner);
            Box::leak(Box::new(std::mem::take(bundles)));
            return;
        }
        // Closing the queue ends the workers once they finish their job.
        self.shared.queue.close();
        // Joined without holding the lock: a worker dying right now takes
        // it (see `Life`). One that started a replacement before the close
        // listed it before ending, so the next round joins that one too.
        loop {
            let batch = std::mem::take(&mut *self.shared.workers_lock());
            if batch.is_empty() {
                break;
            }
            for w in batch {
                let _ = w.join();
            }
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
    /// Last, so it is dropped after the sandbox (see [`Life`]).
    life: Life,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.drop_sandbox();
    }
}

impl Worker {
    fn new(life: Life) -> Self {
        let shared = life.shared.clone();
        Self {
            measure: shared.measure.for_worker(),
            shared,
            sandbox: None,
            preloaded: None,
            last_bundle: None,
            life,
        }
    }

    fn run(mut self) {
        let shared = self.shared.clone();
        let queue = &shared.queue;
        #[cfg(test)]
        if shared.fault(Fault::AtStart, false) {
            panic!("test fault: worker dies at start");
        }
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
            self.life.in_job = true;
            #[cfg(test)]
            if shared.fault(Fault::HoldingJob, true) {
                panic!("test fault: worker dies holding a job");
            }
            match job {
                Job::Render(job) => {
                    let result = self.guard(|w| w.render(&job));
                    self.shared.renders.fetch_add(1, Ordering::Relaxed);
                    if result.is_err() {
                        self.shared.failed_renders.fetch_add(1, Ordering::Relaxed);
                    }
                    queue.done();
                    self.life.in_job = false;
                    let _ = job.reply.send(result);
                }
                Job::Register(job) => {
                    let result = self.guard(|w| w.register(&job));
                    queue.done();
                    self.life.in_job = false;
                    let _ = job.reply.send(result);
                }
            }
            self.life.served += 1;
            #[cfg(test)]
            if shared.fault(Fault::AfterReply, true) {
                panic!("test fault: worker dies after replying");
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

    /// Runs `f` on its own thread and returns its result, failing the test
    /// (instead of hanging it) if it takes more than 60 s.
    fn bounded<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(Duration::from_secs(60))
            .expect("a caller waited 60 s: hung on a dead worker")
    }

    fn echo_runner(pool_size: usize) -> Arc<IslandRunner> {
        let r = IslandRunner::start(
            RunnerOptions {
                pool_size,
                ..RunnerOptions::default()
            },
            Measure::Estimate,
        )
        .unwrap();
        r.register_bundle("echo", "function entry(s) { return s }", "entry")
            .unwrap();
        wait_idle(&r);
        Arc::new(r)
    }

    /// Waits (bounded) until every worker is back waiting for a job, past
    /// the fault checks of its last one, so a fault set now hits the next.
    fn wait_idle(r: &IslandRunner) {
        let until = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let stats = r.stats();
            if stats.idle_workers == stats.workers {
                return;
            }
            assert!(std::time::Instant::now() < until, "{stats:?}");
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn echo(r: &Arc<IslandRunner>, n: usize) -> Result<String, IslandError> {
        let r = r.clone();
        bounded(move || {
            r.render(&IslandRequest {
                renderer: "echo",
                spec_json: &n.to_string(),
                width: 1,
                height: 1,
            })
        })
    }

    fn set_fault(r: &IslandRunner, fault: Fault) {
        r.shared.fault.store(fault as u8, Ordering::Release);
    }

    /// Waits (bounded) until the runner has `n` live workers.
    fn wait_for_workers(r: &IslandRunner, n: usize) {
        let until = std::time::Instant::now() + Duration::from_secs(60);
        while r.stats().workers != n {
            assert!(std::time::Instant::now() < until, "{:?}", r.stats());
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Waits (bounded) until no job is in flight or queued. A worker that
    /// died holding a job counts it out a moment after its caller saw the
    /// error.
    fn wait_quiet(r: &IslandRunner) {
        let until = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            {
                let q = r.shared.queue.lock();
                if q.in_flight == 0 && q.jobs.is_empty() {
                    return;
                }
                assert!(
                    std::time::Instant::now() < until,
                    "in flight {}, queued {}",
                    q.in_flight,
                    q.jobs.len()
                );
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_worker_that_dies_after_replying_is_replaced() {
        let r = echo_runner(1);
        set_fault(&r, Fault::AfterReply);
        assert_eq!(echo(&r, 1).unwrap(), "1");
        wait_for_workers(&r, 0);
        // The next request starts a new worker.
        assert_eq!(echo(&r, 2).unwrap(), "2");
        assert_eq!(r.stats().workers, 1);
        wait_quiet(&r);
    }

    #[test]
    fn a_worker_that_dies_holding_a_job_fails_only_that_job() {
        let r = echo_runner(1);
        set_fault(&r, Fault::HoldingJob);
        match echo(&r, 1).unwrap_err() {
            IslandError::Worker { message } => {
                assert!(message.contains("stopped while handling"), "{message}")
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(echo(&r, 2).unwrap(), "2");
        wait_quiet(&r);
        wait_for_workers(&r, 1);
    }

    /// Requests queued behind a worker that dies after replying are served
    /// by its replacement, not left waiting.
    #[test]
    fn waiting_callers_are_served_by_a_replacement() {
        let r = echo_runner(1);
        // Hold the only worker in a slow job so the others queue up.
        r.register_bundle(
            "slow",
            "function entry(s) { let t = 0; for (let i = 0; i < 2e6; i++) t += i; return s }",
            "entry",
        )
        .unwrap();
        wait_idle(&r);
        set_fault(&r, Fault::AfterReply);
        let results = bounded({
            let r = r.clone();
            move || {
                thread::scope(|s| {
                    let slow = s.spawn(|| {
                        r.render(&IslandRequest {
                            renderer: "slow",
                            spec_json: "0",
                            width: 1,
                            height: 1,
                        })
                    });
                    let others: Vec<_> = (1..4)
                        .map(|n| {
                            let r = &r;
                            s.spawn(move || {
                                r.render(&IslandRequest {
                                    renderer: "echo",
                                    spec_json: &n.to_string(),
                                    width: 1,
                                    height: 1,
                                })
                            })
                        })
                        .collect();
                    let mut all = vec![slow.join().unwrap()];
                    all.extend(others.into_iter().map(|h| h.join().unwrap()));
                    all
                })
            }
        });
        for (n, result) in results.into_iter().enumerate() {
            assert_eq!(result.unwrap(), n.to_string());
        }
        assert!(r.stats().workers <= 1, "{:?}", r.stats());
    }

    /// Workers that die before serving anything are not replaced in a
    /// loop: waiting callers get the typed error, and the pool works again
    /// once workers can start.
    #[test]
    fn callers_fail_when_workers_die_at_start() {
        let r = echo_runner(2);
        // Kill the worker that already runs: its next job ends it.
        set_fault(&r, Fault::HoldingJob);
        assert!(matches!(echo(&r, 0), Err(IslandError::Worker { .. })));
        set_fault(&r, Fault::AtStart);
        wait_for_workers(&r, 0);
        let results = bounded({
            let r = r.clone();
            move || {
                thread::scope(|s| {
                    let callers: Vec<_> = (0..4)
                        .map(|n| {
                            let r = &r;
                            s.spawn(move || {
                                r.render(&IslandRequest {
                                    renderer: "echo",
                                    spec_json: &n.to_string(),
                                    width: 1,
                                    height: 1,
                                })
                            })
                        })
                        .collect();
                    callers
                        .into_iter()
                        .map(|h| h.join().unwrap())
                        .collect::<Vec<_>>()
                })
            }
        });
        for result in results {
            match result.unwrap_err() {
                IslandError::Worker { message } => {
                    assert!(message.contains("no other worker"), "{message}")
                }
                other => panic!("{other:?}"),
            }
        }
        wait_for_workers(&r, 0);
        wait_quiet(&r);
        set_fault(&r, Fault::None);
        assert_eq!(echo(&r, 5).unwrap(), "5");
        assert_eq!(r.stats().workers, 1);
    }

    /// Repeated deaths leave no threads, handles or in-flight counts
    /// behind, and dropping the runner joins whatever is left.
    #[test]
    fn repeated_deaths_do_not_grow_the_pool() {
        let r = echo_runner(2);
        for n in 0..50 {
            wait_idle(&r);
            set_fault(
                &r,
                if n % 2 == 0 {
                    Fault::AfterReply
                } else {
                    Fault::HoldingJob
                },
            );
            let _ = echo(&r, n);
            assert!(r.stats().workers <= 2, "{:?}", r.stats());
            // Live workers plus, at most, two dead ones whose threads
            // haven't fully exited yet (each is pruned once finished).
            let handles = r.shared.workers_lock().len();
            assert!(handles <= r.pool_size() + 2, "{handles} handles");
        }
        set_fault(&r, Fault::None);
        assert_eq!(echo(&r, 99).unwrap(), "99");
        wait_quiet(&r);
        assert!(r.stats().workers <= 2, "{:?}", r.stats());
        let r = Arc::into_inner(r).expect("no other owner");
        bounded(move || drop(r));
    }

    /// The built-ins' test specs, by renderer.
    fn builtin_specs() -> Vec<(&'static str, String)> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let read = |p: &str| std::fs::read_to_string(dir.join(p)).unwrap();
        let mut specs = Vec::new();
        if cfg!(feature = "echarts") {
            specs.push(("echarts", read("echarts/bar.json")));
            specs.push(("echarts", read("echarts/japanese.json")));
        }
        if cfg!(feature = "vega") {
            specs.push(("vega", read("vega/bar.json")));
            specs.push(("vega-lite", read("vega-lite/line-temporal.json")));
        }
        specs
    }

    #[test]
    fn builtins_fall_back_to_source_for_another_engine() {
        let options = || RunnerOptions {
            pool_size: 1,
            limits: Limits::default().with_time(std::time::Duration::from_secs(30)),
            ..RunnerOptions::default()
        };
        let embedded = IslandRunner::start(options(), Measure::Estimate).unwrap();
        // Bytecode recorded with another engine's fingerprint: refused, so
        // this runner compiles every built-in from source.
        let mut other = IslandRunner::start(options(), Measure::Estimate).unwrap();
        let mut fingerprint = builtins::ENGINE_FINGERPRINT.to_vec();
        let last = fingerprint.len() - 1;
        fingerprint[last] ^= 1;
        other.embedded_fingerprint = Box::leak(fingerprint.into_boxed_slice());
        // Keep it reachable, so LeakSanitizer (scripts/ci/asan.sh) doesn't
        // report the deliberate leak.
        static KEEP: std::sync::Mutex<Vec<&[u8]>> = std::sync::Mutex::new(Vec::new());
        KEEP.lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(other.embedded_fingerprint);
        for (renderer, spec) in builtin_specs() {
            let req = IslandRequest {
                renderer,
                spec_json: &spec,
                width: 600,
                height: 400,
            };
            let precompiled = embedded.render(&req).unwrap();
            let from_source = other.render(&req).unwrap();
            assert!(precompiled == from_source, "{renderer}: outputs differ");
            // Unless SURIMONO_PRECOMPILE=0 turned precompiled bytecode off.
            let switched_off = option_env!("SURIMONO_PRECOMPILE") == Some("0")
                || std::env::var_os("SURIMONO_PRECOMPILE").is_some_and(|v| v == "0");
            assert_eq!(
                embedded.bundle(renderer).unwrap().bytecode.is_embedded(),
                !switched_off
            );
            assert!(!other.bundle(renderer).unwrap().bytecode.is_embedded());
        }
    }
}
