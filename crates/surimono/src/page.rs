//! Page scripts: a document run as a page on worker threads of its own, each
//! with big stacks and a DOM runtime, like the island runner's.
//!
//! The runner's queue belongs to `surimono-islands` and knows only islands, so
//! this is a second pool, started on the first page: a caller of
//! [`Renderer::render_islands`](crate::Renderer::render_islands) never starts
//! a thread here. Each render parses a new `Document` and drops it. Pages run
//! under the same [`Limits`](crate::Limits), one deadline for the page.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use surimono_core::{SandboxError, SandboxOptions, SettleOptions, SettleOutcome};
use surimono_dom::{Document, DomError, DomRuntime};

use crate::backend;
use crate::error::Error;
use crate::options::{LayoutQueries, Options};
use crate::resources::ResourceFailureReason;
use crate::resources::{Failures, Fetcher, ResourceFailure, ResourceKind, ResourceLoader};

/// How often a waiting caller looks at its cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(5);
/// How long a worker keeps its runtime with nothing to do, as the island
/// runner's workers do by default.
const RELEASE_IDLE_AFTER: Duration = Duration::from_secs(2);

/// Why a page stopped running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SettleReason {
    /// Nothing was left to run.
    Idle,
    /// What was left is due after the 10 seconds of virtual time a page gets,
    /// as with a `setInterval` nobody clears.
    VirtualBudget,
    /// The page ran 100,000 tasks with more due.
    TaskLimit,
}

/// What happened while a page ran. Nothing here failed the render: a page with
/// an uncaught exception goes on, as it does in a browser.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PageReport {
    /// Why it stopped.
    pub settled: SettleReason,
    /// Exceptions no script caught and no `error` listener cancelled.
    pub uncaught: u64,
    /// The first few of those, as messages.
    pub errors: Vec<String>,
    /// Promises rejected with no handler.
    pub unhandled_rejections: u64,
    /// The first few reasons, as messages.
    pub rejections: Vec<String>,
    /// How many external scripts and images the page didn't get.
    pub failed_count: u64,
    /// The first few of those, in document order, scripts first.
    pub failed: Vec<ResourceFailure>,
}

/// A rendered page: the document after its scripts ran, and how that went.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PageOutput {
    /// The document, serialized, with island markers replaced by SVG.
    pub html: String,
    /// What happened while it ran.
    pub report: PageReport,
}

/// What a worker needs for every page.
struct Setup {
    sandbox: SandboxOptions,
    memory_bytes: usize,
    /// A script that puts the layout shim first in the tree.
    add_shim: String,
    /// A script that clears the settled page out for printing.
    cleanup: String,
    loader: Option<ResourceLoader>,
}

/// The classic script text of the layout shim, as a statement adding it.
fn add_shim(queries: LayoutQueries) -> String {
    let mode = match queries {
        LayoutQueries::Zeros => "zeros",
        LayoutQueries::Throw => "throw",
    };
    let shim = format!(
        "(function (mode) {{ 'use strict';\n{}\n}})('{mode}');",
        include_str!("layout_shim.js"),
    );
    let text = serde_json::Value::from(shim);
    format!(
        "(function () {{
           const s = document.createElement('script');
           s.textContent = {text};
           const root = document.documentElement;
           root.insertBefore(s, root.firstChild);
         }})();"
    )
}

fn cleanup(keep_scripts: bool) -> String {
    format!(
        "(function (keepScripts) {{ 'use strict';\n{}\n}})({keep_scripts});",
        include_str!("cleanup.js"),
    )
}

/// The page's serialized HTML and report, islands still to do.
pub(crate) type PageResult = Result<(String, PageReport), Error>;

struct Job {
    html: String,
    stop: Arc<AtomicBool>,
    reply: SyncSender<PageResult>,
}

#[derive(Default)]
struct State {
    jobs: VecDeque<Job>,
    /// Workers waiting for a job.
    idle: usize,
    closed: bool,
    workers: Vec<JoinHandle<()>>,
}

struct Shared {
    setup: Setup,
    state: Mutex<State>,
    wake: Condvar,
    pool_size: usize,
    stack_size: usize,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The page workers of one process. See the module docs.
pub(crate) struct PagePool {
    shared: Arc<Shared>,
}

impl PagePool {
    /// A pool with no worker yet.
    pub(crate) fn new(options: &Options) -> Self {
        let pool_size = options
            .pool_size
            .unwrap_or_else(crate::options::default_pool_size);
        let sandbox = backend::sandbox_options(options);
        // As in the island runner: room for the script's stack limit plus the
        // Rust frames around it. Untouched stack costs no memory.
        let stack_size = options.limits.stack_bytes + 8 * 1024 * 1024;
        Self {
            shared: Arc::new(Shared {
                setup: Setup {
                    sandbox,
                    memory_bytes: options.limits.memory_bytes,
                    add_shim: add_shim(options.layout_queries),
                    cleanup: cleanup(options.keep_scripts),
                    loader: options.resource_loader.clone(),
                },
                state: Mutex::new(State::default()),
                wake: Condvar::new(),
                pool_size,
                stack_size,
            }),
        }
    }

    /// Runs `html` as a page on a worker. With `cancel`, the call returns
    /// [`Error::Cancelled`] within milliseconds of the flag going up; a page
    /// still queued never starts, and a running one is left to stop at its
    /// time limit, because [`DomRuntime`] offers no way to stop it from
    /// outside. Its worker is busy until then.
    pub(crate) fn run(&self, html: &str, cancel: Option<&AtomicBool>) -> PageResult {
        if cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
            return Err(Error::Cancelled);
        }
        let (reply, answer) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        {
            let mut state = self.shared.lock();
            let job = Job {
                html: html.to_owned(),
                stop: Arc::clone(&stop),
                reply,
            };
            if state.idle <= state.jobs.len() && state.workers.len() < self.shared.pool_size {
                // With another worker running, the job can wait for it.
                if let Err(e) = spawn(&self.shared, &mut state)
                    && state.workers.is_empty()
                {
                    return Err(e);
                }
            }
            state.jobs.push_back(job);
            self.shared.wake.notify_one();
        }
        let stopped = || Error::PageFailed {
            message: "the page's worker thread stopped".into(),
        };
        let Some(cancel) = cancel else {
            return answer.recv().unwrap_or_else(|_| Err(stopped()));
        };
        loop {
            match answer.recv_timeout(CANCEL_POLL) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout) if cancel.load(Ordering::Acquire) => {
                    stop.store(true, Ordering::Release);
                    return Err(Error::Cancelled);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(stopped()),
            }
        }
    }
}

impl Drop for PagePool {
    /// Joins the workers. [`Renderer`](crate::Renderer) never lets this run
    /// for a pool started in another process, whose threads don't exist here.
    fn drop(&mut self) {
        let workers = {
            let mut state = self.shared.lock();
            state.closed = true;
            std::mem::take(&mut state.workers)
        };
        self.shared.wake.notify_all();
        for worker in workers {
            let _ = worker.join();
        }
    }
}

fn spawn(shared: &Arc<Shared>, state: &mut State) -> Result<(), Error> {
    let worker = Arc::clone(shared);
    let handle = thread::Builder::new()
        .name(format!("surimono-page-{}", state.workers.len()))
        .stack_size(shared.stack_size)
        .spawn(move || work(&worker))
        .map_err(|e| Error::PageFailed {
            message: format!("could not start a worker thread: {e}"),
        })?;
    state.workers.push(handle);
    Ok(())
}

/// One worker's life.
fn work(shared: &Shared) {
    let mut runtime: Option<DomRuntime> = None;
    loop {
        let job = {
            let mut state = shared.lock();
            loop {
                if let Some(job) = state.jobs.pop_front() {
                    break Some(job);
                }
                if state.closed {
                    return;
                }
                state.idle += 1;
                let timed_out = if runtime.is_some() {
                    let (next, wait) = shared
                        .wake
                        .wait_timeout(state, RELEASE_IDLE_AFTER)
                        .unwrap_or_else(PoisonError::into_inner);
                    state = next;
                    wait.timed_out()
                } else {
                    state = shared
                        .wake
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                    false
                };
                state.idle -= 1;
                if timed_out && state.jobs.is_empty() {
                    break None;
                }
            }
        };
        let Some(job) = job else {
            drop(runtime.take());
            surimono_core::release_free_memory();
            continue;
        };
        if job.stop.load(Ordering::Acquire) {
            continue;
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            run_page(&shared.setup, &mut runtime, &job)
        }))
        .unwrap_or_else(|_| {
            Err(Error::PageFailed {
                message: "the page's worker panicked".into(),
            })
        });
        // A runtime that failed may hold what it failed with.
        if result.is_err() {
            runtime = None;
        }
        // Nobody may be waiting any more: a cancelled caller has gone.
        let _ = job.reply.send(result);
    }
}

fn run_page(setup: &Setup, runtime: &mut Option<DomRuntime>, job: &Job) -> PageResult {
    let document = Document::parse(&job.html).map_err(page_error)?;
    let rt = match runtime {
        Some(rt) => rt,
        None => {
            runtime.insert(DomRuntime::with_options(setup.sandbox.clone()).map_err(page_error)?)
        }
    };
    rt.load(document);
    let result = run_loaded(setup, rt, job);
    // The document goes now, not with the next page.
    drop(rt.take());
    result
}

fn run_loaded(setup: &Setup, rt: &mut DomRuntime, job: &Job) -> PageResult {
    // Run in a context the page never sees, before any of its code exists.
    rt.eval::<()>(&setup.add_shim).map_err(page_error)?;
    let mut fetcher = Fetcher::new(setup.loader.as_ref(), setup.memory_bytes);
    let page = rt
        .run_page(&SettleOptions::default(), |url| fetcher.script(url))
        .map_err(page_error)?;
    if job.stop.load(Ordering::Acquire) {
        return Err(Error::Cancelled);
    }
    let settled = match page.settle.outcome {
        SettleOutcome::Settled if page.settle.settled_by_budget() => SettleReason::VirtualBudget,
        SettleOutcome::Settled => SettleReason::Idle,
        SettleOutcome::TaskLimit => SettleReason::TaskLimit,
        // No external wait is ever opened: scripts load before the page runs.
        SettleOutcome::WaitingExternal => {
            return Err(Error::PageFailed {
                message: "the page was left waiting on an external resource".into(),
            });
        }
    };
    let mut failures = fetcher.failures;
    // A fresh context: the page's own code can't get in the way from here on.
    rt.reset().map_err(page_error)?;
    let summary: String = rt.eval(&setup.cleanup).map_err(page_error)?;
    let summary: Summary = serde_json::from_str(&summary).map_err(|e| Error::PageFailed {
        message: format!("reading the page's summary: {e}"),
    })?;
    add_images(&mut failures, &summary);
    let body = rt
        .with_document(Document::to_html)
        .ok_or_else(|| page_error(DomError::NoDocument))?;
    // The parser drops the doctype, and without one the printer would parse
    // the page in quirks mode.
    let html = if summary.standards {
        format!("<!doctype html>{body}")
    } else {
        body
    };
    // Drops the page's last wrappers before the document goes.
    rt.reset().map_err(page_error)?;
    Ok((
        html,
        PageReport {
            settled,
            uncaught: page.uncaught,
            errors: page.errors,
            unhandled_rejections: page.unhandled_rejections,
            rejections: page.rejections,
            failed_count: failures.count,
            failed: failures.kept,
        },
    ))
}

/// What `cleanup.js` returns.
#[derive(serde::Deserialize)]
struct Summary {
    standards: bool,
    images: u64,
    urls: Vec<String>,
}

fn add_images(failures: &mut Failures, summary: &Summary) {
    for url in &summary.urls {
        failures.add(ResourceKind::Image, url, ResourceFailureReason::NotLoaded);
    }
    failures.count += summary.images.saturating_sub(summary.urls.len() as u64);
}

/// The public error for a page that failed.
fn page_error(e: DomError) -> Error {
    match e {
        DomError::Sandbox(SandboxError::Cancelled) => Error::Cancelled,
        DomError::Sandbox(SandboxError::MemoryLimit) => Error::PageMemoryLimit,
        DomError::Sandbox(SandboxError::TimeLimit) => Error::PageTimeLimit,
        DomError::Sandbox(SandboxError::StackOverflow) => Error::PageStackOverflow,
        DomError::NodeLimit { .. }
        | DomError::TreeTooDeep { .. }
        | DomError::HandleLimit { .. }
        | DomError::TextLimit { .. }
        | DomError::StyleLimit { .. } => Error::PageDomLimit {
            message: e.to_string(),
        },
        other => Error::PageFailed {
            message: other.to_string(),
        },
    }
}
