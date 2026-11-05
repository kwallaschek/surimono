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
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use surimono_core::{SandboxError, SandboxOptions, SettleOptions, SettleOutcome};
use surimono_dom::{Document, DomError, DomLimits, DomRuntime, LayoutPolicy};

use crate::backend;
use crate::error::Error;
use crate::options::{LayoutQueries, Options};
use crate::print::{self, Finish, Gate};
use crate::resources::{
    Failures, Fetcher, ResourceFailure, ResourceFailureReason, ResourceKind, ResourceLoader, Source,
};

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
    /// How many external scripts, images and stylesheets the page didn't get.
    pub failed_count: u64,
    /// The first few of those: scripts, then images, then stylesheets, each in
    /// document order.
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
    /// The document's limits: the text budget follows the output limit.
    dom: DomLimits,
    /// How long everything after the scripts may take.
    finish_time: Duration,
    output_bytes: usize,
    layout: LayoutPolicy,
    /// A script that clears the settled page out for printing.
    cleanup: String,
    keep_scripts: bool,
    loader: Option<ResourceLoader>,
}

fn cleanup(keep_scripts: bool) -> String {
    format!(
        "(function (keepScripts) {{ 'use strict';\n{}\n}})({keep_scripts});",
        include_str!("cleanup.js"),
    )
}

/// The page's serialized HTML and report, islands still to do.
pub(crate) type PageResult = Result<(String, PageReport), Error>;

/// What a worker sends its caller.
enum Message {
    /// A load for a function that runs on the caller's thread.
    Need {
        url: String,
        answer: Sender<Result<Vec<u8>, String>>,
    },
    Done(PageResult),
}

/// What [`PagePool::run`] asks of a caller's own load function.
pub(crate) struct CallerLoad<'a> {
    pub(crate) allow: &'a [String],
    pub(crate) load: &'a mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
}

/// How a caller stops its job, in whatever state it's in.
#[derive(Default)]
struct Stop {
    raised: AtomicBool,
    /// The running runtime's own flag, while the job has a runtime.
    runtime: Mutex<Option<Arc<AtomicBool>>>,
}

impl Stop {
    fn runtime(&self) -> MutexGuard<'_, Option<Arc<AtomicBool>>> {
        self.runtime.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn raise(&self) {
        self.raised.store(true, Ordering::Release);
        if let Some(flag) = self.runtime().as_ref() {
            flag.store(true, Ordering::Release);
        }
    }

    /// Lets a raise reach `flag`, and fails if one already came.
    fn attach(&self, flag: Arc<AtomicBool>) -> Result<(), Error> {
        *self.runtime() = Some(flag);
        if self.raised.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        Ok(())
    }

    /// Ends that, leaving the runtime's flag down for its next page.
    fn detach(&self) {
        if let Some(flag) = self.runtime().take() {
            flag.store(false, Ordering::Release);
        }
    }
}

struct Job {
    html: String,
    stop: Arc<Stop>,
    /// The allowlist of a caller's load function, if it has one.
    allow: Option<Vec<String>>,
    reply: Sender<Message>,
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
                    dom: DomLimits {
                        // A serialized page holds its text at least once.
                        max_text_bytes: options
                            .limits
                            .output_bytes
                            .min(DomLimits::DEFAULT_MAX_TEXT_BYTES),
                        ..DomLimits::default()
                    },
                    finish_time: options.limits.time,
                    output_bytes: options.limits.output_bytes,
                    layout: match options.layout_queries {
                        LayoutQueries::Zeros => LayoutPolicy::Zeros,
                        LayoutQueries::Throw => LayoutPolicy::Throw,
                    },
                    cleanup: cleanup(options.keep_scripts),
                    keep_scripts: options.keep_scripts,
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
    /// [`Error::Cancelled`] within milliseconds of the flag going up. A page
    /// still queued never starts, and a running one stops too, its worker free
    /// again within milliseconds.
    pub(crate) fn run(
        &self,
        html: &str,
        cancel: Option<&AtomicBool>,
        mut caller: Option<CallerLoad<'_>>,
    ) -> PageResult {
        let raised = || cancel.is_some_and(|c| c.load(Ordering::Acquire));
        if raised() {
            return Err(Error::Cancelled);
        }
        let (reply, messages): (_, Receiver<Message>) = mpsc::channel();
        let stop = Arc::new(Stop::default());
        {
            let mut state = self.shared.lock();
            let job = Job {
                html: html.to_owned(),
                stop: Arc::clone(&stop),
                allow: caller.as_ref().map(|c| c.allow.to_vec()),
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
        loop {
            match messages.recv_timeout(CANCEL_POLL) {
                Ok(Message::Done(result)) => return result,
                Ok(Message::Need { url, answer }) if !raised() => {
                    let bytes = match caller.as_mut() {
                        Some(c) => (c.load)(&url),
                        None => Err("no load function".to_owned()),
                    };
                    let _ = answer.send(bytes);
                }
                // The worker finds the job stopped when this load fails.
                Ok(Message::Need { answer, .. }) => {
                    let _ = answer.send(Err("cancelled".to_owned()));
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Error::PageFailed {
                        message: "the page's worker thread stopped".into(),
                    });
                }
            }
            if raised() {
                stop.raise();
                return Err(Error::Cancelled);
            }
        }
    }
}

impl Drop for PagePool {
    /// Closes the queue and doesn't join: a worker may be running a page up to
    /// the time limit, and Ruby's garbage collector drops renderers, which
    /// mustn't wait for that. A worker ends on its own after its page.
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.wake.notify_all();
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
        if job.stop.raised.load(Ordering::Acquire) {
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
        let _ = job.reply.send(Message::Done(result));
    }
}

fn run_page(setup: &Setup, runtime: &mut Option<DomRuntime>, job: &Job) -> PageResult {
    // Parsing isn't under a script's limit, and has one of its own.
    let parsing = Gate::new(&job.stop.raised, setup.finish_time);
    let stop = || parsing.check().is_err();
    let document = Document::parse_stoppable(&job.html, setup.dom, stop)
        .map_err(|e| gated_error(e, &parsing))?;
    let rt = match runtime {
        Some(rt) => rt,
        None => {
            let mut rt = DomRuntime::with_options(setup.sandbox.clone()).map_err(page_error)?;
            rt.set_layout_policy(setup.layout);
            runtime.insert(rt)
        }
    };
    rt.load(document);
    let result = job
        .stop
        .attach(rt.cancel_flag())
        .and_then(|()| run_loaded(setup, rt, job));
    job.stop.detach();
    // The document goes now, not with the next page.
    drop(rt.take());
    result
}

fn run_loaded(setup: &Setup, rt: &mut DomRuntime, job: &Job) -> PageResult {
    let ask = |url: &str| {
        let (answer, bytes) = mpsc::channel();
        let need = Message::Need {
            url: url.to_owned(),
            answer,
        };
        job.reply.send(need).map_err(|_| "cancelled".to_owned())?;
        bytes.recv().map_err(|_| "cancelled".to_owned())?
    };
    let source = match &job.allow {
        Some(allow) => Some(Source::Caller { allow, ask: &ask }),
        None => setup.loader.as_ref().map(Source::Shared),
    };
    let mut fetcher = Fetcher::new(source, setup.memory_bytes);
    let page = rt
        .run_page(&SettleOptions::default(), |url| fetcher.script(url))
        .map_err(page_error)?;
    // One deadline from here to the end, and a cancel reaches all of it.
    let gate = Gate::new(&job.stop.raised, setup.finish_time);
    gate.check()?;
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
    // A fresh context: the page's own code can't get in the way from here on.
    rt.reset().map_err(page_error)?;
    let summary: String = rt.eval(&setup.cleanup).map_err(page_error)?;
    let summary: Summary = serde_json::from_str(&summary).map_err(|e| Error::PageFailed {
        message: format!("reading the page's summary: {e}"),
    })?;
    add_images(&mut fetcher.failures, &summary);
    let stop = || gate.check().is_err();
    let mut html = rt
        .with_document(|document| document.to_html_bounded(setup.output_bytes, stop))
        .ok_or_else(|| page_error(DomError::NoDocument))?
        .map_err(|e| gated_error(e, &gate))?;
    // The parser drops the doctype; a renderer that goes by it gets the mode
    // the page ran in, standards or not. (Blitz always lays out as no-quirks.)
    if summary.standards {
        html.insert_str(0, "<!doctype html>");
    }
    // Drops the page's last wrappers, then the document.
    rt.reset().map_err(page_error)?;
    drop(rt.take());
    gate.check()?;
    let finish = Finish {
        keep_scripts: setup.keep_scripts,
        max_bytes: setup.output_bytes,
    };
    let loads = fetcher.has_loader().then_some(&mut fetcher);
    let html = print::finish_page(&html, finish, loads, &gate)?;
    let failures = fetcher.failures;
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

/// [`page_error`], for a step stopped by `gate`: by a cancel or its deadline.
fn gated_error(e: DomError, gate: &Gate<'_>) -> Error {
    match e {
        DomError::Stopped => gate.check().err().unwrap_or(Error::Cancelled),
        other => page_error(other),
    }
}

/// The public error for a page that failed.
fn page_error(e: DomError) -> Error {
    match e {
        DomError::Stopped => Error::Cancelled,
        DomError::LayoutQuery { member } => Error::PageLayoutQuery { member },
        DomError::OutputLimit { limit } => print::too_long(limit),
        DomError::Sandbox(SandboxError::Cancelled) => Error::Cancelled,
        DomError::Sandbox(SandboxError::MemoryLimit) => Error::PageMemoryLimit,
        DomError::Sandbox(SandboxError::TimeLimit) => Error::PageTimeLimit,
        DomError::Sandbox(SandboxError::StackOverflow) => Error::PageStackOverflow,
        DomError::NodeLimit { .. }
        | DomError::TreeTooDeep { .. }
        | DomError::HandleLimit { .. }
        | DomError::TextLimit { .. }
        | DomError::AttributeLimit { .. }
        | DomError::StyleLimit { .. } => Error::PageDomLimit {
            message: e.to_string(),
        },
        other => Error::PageFailed {
            message: other.to_string(),
        },
    }
}
