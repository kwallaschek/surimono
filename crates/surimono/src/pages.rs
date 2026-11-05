//! The pool that runs pages: worker threads, each with a DOM runtime.
//!
//! A [`DomRuntime`] holds `Rc`s, so it stays on one thread, and scripts need a
//! stack of known size whatever thread the caller is on (Puma's are 1 MiB), so
//! a page runs on a worker, as an island does. Workers start when a page finds
//! every one busy, up to the pool size, and end after
//! [`IDLE_AFTER`] without work, which frees their runtimes: a process that
//! stops printing goes back to the memory it had.
//!
//! A pool belongs to one process, like [`Local`](crate::renderer): it is
//! reached through the renderer's per-process slot, so a forked child never
//! sees a parent's pool in use. Starting one starts no thread.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use surimono_core::{SandboxOptions, SettleOptions};
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

use crate::error::Error;
use crate::page::{PageReport, doctype};

/// How long a worker waits for a page before it ends and frees its runtime.
const IDLE_AFTER: Duration = Duration::from_secs(2);

/// How often a waiting caller looks at its cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(5);

/// Stack beyond the script's own: Rust frames around it (blitz-dom recurses
/// per tree level), as for the island workers.
const STACK_HEADROOM: usize = 8 * 1024 * 1024;

/// How every page in a pool runs.
#[derive(Debug, Clone)]
pub(crate) struct PagePoolOptions {
    pub(crate) sandbox: SandboxOptions,
    pub(crate) settle: SettleOptions,
    pub(crate) dom: DomLimits,
    pub(crate) pool_size: usize,
}

/// A worker's answer: the page as serialized and its report.
pub(crate) struct Ran {
    pub(crate) html: String,
    pub(crate) report: PageReport,
}

struct Job {
    html: String,
    /// Raised to drop this job while it waits, and to stop its script.
    cancel: Arc<AtomicBool>,
    reply: Sender<Result<Ran, Error>>,
}

#[derive(Default)]
struct State {
    jobs: VecDeque<Job>,
    /// Workers waiting for a job.
    idle: usize,
    /// Workers started and not yet ended.
    live: usize,
    closed: bool,
    handles: Vec<JoinHandle<()>>,
}

struct Shared {
    options: PagePoolOptions,
    state: Mutex<State>,
    ready: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Nothing panics under this lock, and the state is whole whenever
        // it is released.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The next job, or `None` when the pool closed or none came in time, in
    /// which case this worker no longer counts.
    fn next(&self, idle_after: Duration) -> Option<Job> {
        let mut state = self.lock();
        let deadline = Instant::now() + idle_after;
        state.idle += 1;
        let job = loop {
            if let Some(job) = state.jobs.pop_front() {
                break Some(job);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if state.closed || left.is_zero() {
                break None;
            }
            state = self
                .ready
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        };
        state.idle -= 1;
        if job.is_none() {
            state.live -= 1;
        }
        job
    }
}

/// See the module docs.
pub(crate) struct PagePool {
    shared: Arc<Shared>,
    idle_after: Duration,
}

impl PagePool {
    pub(crate) fn new(options: PagePoolOptions) -> Self {
        Self::with_idle_after(options, IDLE_AFTER)
    }

    fn with_idle_after(options: PagePoolOptions, idle_after: Duration) -> Self {
        Self {
            shared: Arc::new(Shared {
                options,
                state: Mutex::new(State::default()),
                ready: Condvar::new(),
            }),
            idle_after,
        }
    }

    /// Runs `html` as a page on a worker and waits for it. With `cancel` up
    /// the call returns [`Error::Cancelled`] within a few milliseconds.
    pub(crate) fn run(&self, html: &str, cancel: Option<&AtomicBool>) -> Result<Ran, Error> {
        let raised = || cancel.is_some_and(|c| c.load(Ordering::Acquire));
        if raised() {
            return Err(Error::Cancelled);
        }
        let job_cancel = Arc::new(AtomicBool::new(false));
        let answer = self.submit(html, Arc::clone(&job_cancel))?;
        loop {
            match answer.recv_timeout(CANCEL_POLL) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout) if raised() => {
                    job_cancel.store(true, Ordering::Release);
                    return Err(Error::Cancelled);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Error::Page("the worker thread stopped".to_owned()));
                }
            }
        }
    }

    fn submit(
        &self,
        html: &str,
        cancel: Arc<AtomicBool>,
    ) -> Result<Receiver<Result<Ran, Error>>, Error> {
        let (reply, answer) = mpsc::channel();
        let job = Job {
            html: html.to_owned(),
            cancel,
            reply,
        };
        let mut state = self.shared.lock();
        state.jobs.push_back(job);
        let short = state.jobs.len() > state.idle;
        if short && state.live < self.shared.options.pool_size {
            state.handles.retain(|h| !h.is_finished());
            match self.spawn() {
                Ok(handle) => {
                    state.handles.push(handle);
                    state.live += 1;
                }
                // Another worker gets to the job; with none, nobody would.
                Err(e) if state.live == 0 => {
                    state.jobs.clear();
                    return Err(Error::Page(format!("no worker thread: {e}")));
                }
                Err(_) => {}
            }
        }
        drop(state);
        self.shared.ready.notify_one();
        Ok(answer)
    }

    fn spawn(&self) -> std::io::Result<JoinHandle<()>> {
        let shared = Arc::clone(&self.shared);
        let idle_after = self.idle_after;
        let stack = self.shared.options.sandbox.limits.stack_bytes + STACK_HEADROOM;
        thread::Builder::new()
            .name("surimono-page".to_owned())
            .stack_size(stack)
            .spawn(move || work(&shared, idle_after))
    }

    /// Workers running now.
    #[cfg(test)]
    fn workers(&self) -> usize {
        self.shared.lock().live
    }
}

impl Drop for PagePool {
    fn drop(&mut self) {
        let handles = {
            let mut state = self.shared.lock();
            state.closed = true;
            state.jobs.clear();
            std::mem::take(&mut state.handles)
        };
        self.shared.ready.notify_all();
        for handle in handles {
            let _ = handle.join();
        }
    }
}

/// A worker's life. A runtime goes after any failure, so nothing a failed page
/// did can reach the next one.
fn work(shared: &Shared, idle_after: Duration) {
    let mut runtime: Option<DomRuntime> = None;
    while let Some(job) = shared.next(idle_after) {
        if job.cancel.load(Ordering::Acquire) {
            let _ = job.reply.send(Err(Error::Cancelled));
            continue;
        }
        let ran = catch_unwind(AssertUnwindSafe(|| {
            run_job(&shared.options, &mut runtime, &job.html)
        }))
        .unwrap_or_else(|_| Err(Error::Page("a page's worker panicked".to_owned())));
        if ran.is_err() {
            runtime = None;
        }
        let _ = job.reply.send(ran);
        // Off the caller's clock: frees this page's heap now rather than at
        // the next page's start, and has a runtime ready for that page. A
        // runtime that can't reset isn't kept.
        if !runtime.as_mut().is_some_and(|rt| rt.reset().is_ok()) {
            runtime = DomRuntime::with_options(shared.options.sandbox.clone()).ok();
        }
    }
    drop(runtime);
    surimono_core::release_free_memory();
}

fn run_job(
    options: &PagePoolOptions,
    runtime: &mut Option<DomRuntime>,
    html: &str,
) -> Result<Ran, Error> {
    let document = Document::parse_with_limits(html, options.dom).map_err(page_error)?;
    let rt = match runtime {
        Some(rt) => rt,
        None => {
            runtime.insert(DomRuntime::with_options(options.sandbox.clone()).map_err(page_error)?)
        }
    };
    rt.load(document);
    // Seam for E2-06: the loader answers here.
    let report = rt.run_page(&options.settle, |_| None).map_err(page_error)?;
    let report = PageReport::new(report).map_err(Error::Page)?;
    let body = rt
        .with_document(Document::to_html)
        .ok_or_else(|| Error::Page("the page lost its document".to_owned()))?;
    drop(rt.take());
    let doctype = doctype(html).unwrap_or_default();
    Ok(Ran {
        html: format!("{doctype}{body}"),
        report,
    })
}

/// The public error for a page that didn't finish.
pub(crate) fn page_error(e: DomError) -> Error {
    use surimono_core::SandboxError as S;
    match e {
        DomError::Sandbox(S::MemoryLimit) => Error::PageMemoryLimit,
        DomError::Sandbox(S::TimeLimit) => Error::PageTimeLimit,
        DomError::Sandbox(S::StackOverflow) => Error::PageStackOverflow,
        DomError::Sandbox(S::Cancelled) => Error::Cancelled,
        DomError::NodeLimit { .. }
        | DomError::TreeTooDeep { .. }
        | DomError::HandleLimit { .. }
        | DomError::TextLimit { .. }
        | DomError::StyleLimit { .. } => Error::DocumentLimit(e.to_string()),
        DomError::Unsupported { feature } => Error::Unsupported {
            feature: feature.to_owned(),
        },
        other => Error::Page(other.to_string()),
    }
}
