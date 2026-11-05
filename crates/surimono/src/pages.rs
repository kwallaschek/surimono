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
use std::thread;
use std::time::{Duration, Instant};

use surimono_core::{SandboxOptions, SettleOptions};
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

use crate::error::Error;
use crate::loader::{ResourceKind, Session};
use crate::page::{PageReport, doctype, strip_for_print};

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

/// What a worker sends its caller.
enum Message {
    /// The source of an external script, from the caller's loader, which is
    /// the caller's: it runs on the caller's thread, and in Ruby needs the GVL.
    Need {
        src: String,
        answer: Sender<Option<String>>,
    },
    Done(Result<Ran, Error>),
}

struct Job {
    html: String,
    /// Raised to drop this job while it waits, and to stop its script.
    cancel: Arc<AtomicBool>,
    /// Whether the caller has a loader to ask.
    loads: bool,
    reply: Sender<Message>,
}

#[derive(Default)]
struct State {
    jobs: VecDeque<Job>,
    /// Pages sent and not yet answered: a worker that has answered is about to
    /// be free, so the next page doesn't need another.
    in_flight: usize,
    /// Workers started and not yet ended.
    live: usize,
    closed: bool,
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
        if job.is_none() {
            state.live -= 1;
        }
        job
    }

    /// Answers a job (just before the reply goes, so a caller that sends its
    /// next page at once sees this one as done).
    fn answer(&self, job: Job, answer: Result<Ran, Error>) {
        self.lock().in_flight -= 1;
        let _ = job.reply.send(Message::Done(answer));
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
    /// the call returns [`Error::Cancelled`] within a few milliseconds, or
    /// right after a load that is running. `session` answers on this thread.
    pub(crate) fn run(
        &self,
        html: &str,
        cancel: Option<&AtomicBool>,
        mut session: Option<&mut Session<'_, '_>>,
    ) -> Result<Ran, Error> {
        let raised = || cancel.is_some_and(|c| c.load(Ordering::Acquire));
        if raised() {
            return Err(Error::Cancelled);
        }
        let job_cancel = Arc::new(AtomicBool::new(false));
        let messages = self.submit(html, Arc::clone(&job_cancel), session.is_some())?;
        loop {
            let message = messages.recv_timeout(CANCEL_POLL);
            if raised() {
                job_cancel.store(true, Ordering::Release);
                return Err(Error::Cancelled);
            }
            match message {
                Ok(Message::Done(result)) => return result,
                Ok(Message::Need { src, answer }) => {
                    let text = session
                        .as_mut()
                        .and_then(|s| s.load(&src, ResourceKind::Script));
                    let _ = answer.send(text);
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
        loads: bool,
    ) -> Result<Receiver<Message>, Error> {
        let (reply, answer) = mpsc::channel();
        let job = Job {
            html: html.to_owned(),
            cancel,
            loads,
            reply,
        };
        let mut state = self.shared.lock();
        state.jobs.push_back(job);
        state.in_flight += 1;
        if state.in_flight > state.live && state.live < self.shared.options.pool_size {
            match self.spawn() {
                Ok(()) => state.live += 1,
                // Another worker gets to the job; with none, nobody would.
                Err(e) if state.live == 0 => {
                    state.jobs.clear();
                    state.in_flight = 0;
                    return Err(Error::Page(format!("no worker thread: {e}")));
                }
                Err(_) => {}
            }
        }
        drop(state);
        self.shared.ready.notify_one();
        Ok(answer)
    }

    fn spawn(&self) -> std::io::Result<()> {
        let shared = Arc::clone(&self.shared);
        let idle_after = self.idle_after;
        let stack = self.shared.options.sandbox.limits.stack_bytes + STACK_HEADROOM;
        thread::Builder::new()
            .name("surimono-page".to_owned())
            .stack_size(stack)
            .spawn(move || work(&shared, idle_after))
            .map(drop)
    }

    /// Workers running now.
    #[cfg(test)]
    fn workers(&self) -> usize {
        self.shared.lock().live
    }
}

/// Not joined: a worker may be running a page up to the time limit, and a
/// renderer can be dropped by Ruby's garbage collector, which mustn't wait for
/// it. A worker ends on its own after its page.
impl Drop for PagePool {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.closed = true;
        state.jobs.clear();
        drop(state);
        self.shared.ready.notify_all();
    }
}

/// A worker's life. A runtime goes after any failure, so nothing a failed page
/// did can reach the next one.
fn work(shared: &Shared, idle_after: Duration) {
    let mut runtime: Option<DomRuntime> = None;
    while let Some(job) = shared.next(idle_after) {
        if job.cancel.load(Ordering::Acquire) {
            shared.answer(job, Err(Error::Cancelled));
            continue;
        }
        let ran = catch_unwind(AssertUnwindSafe(|| {
            run_job(&shared.options, &mut runtime, &job)
        }))
        .unwrap_or_else(|_| Err(Error::Page("a page's worker panicked".to_owned())));
        if ran.is_err() {
            runtime = None;
        }
        shared.answer(job, ran);
        // Off the caller's clock: frees this page's heap now rather than at
        // the next page's start, and has a runtime ready for that page. A
        // runtime that can't reset isn't kept.
        if runtime.as_mut().is_none_or(|rt| rt.reset().is_err()) {
            runtime = DomRuntime::with_options(shared.options.sandbox.clone()).ok();
        }
    }
    drop(runtime);
    surimono_core::release_free_memory();
}

fn run_job(
    options: &PagePoolOptions,
    runtime: &mut Option<DomRuntime>,
    job: &Job,
) -> Result<Ran, Error> {
    let html = job.html.as_str();
    let document = Document::parse_with_limits(html, options.dom).map_err(page_error)?;
    let rt = match runtime {
        Some(rt) => rt,
        None => {
            runtime.insert(DomRuntime::with_options(options.sandbox.clone()).map_err(page_error)?)
        }
    };
    rt.load(document);
    // Before the page's first script, and before its time limit starts.
    let resolve = |src: &str| {
        if !job.loads || job.cancel.load(Ordering::Acquire) {
            return None;
        }
        let (answer, text) = mpsc::channel();
        let need = Message::Need {
            src: src.to_owned(),
            answer,
        };
        job.reply.send(need).ok()?;
        text.recv().ok().flatten()
    };
    let report = rt.run_page(&options.settle, resolve).map_err(page_error)?;
    let report = PageReport::new(report).map_err(Error::Page)?;
    let body = rt
        .with_document(Document::to_html)
        .ok_or_else(|| Error::Page("the page lost its document".to_owned()))?;
    drop(rt.take());
    let doctype = doctype(html).unwrap_or_default();
    Ok(Ran {
        html: format!("{doctype}{}", strip_for_print(&body)),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(pool_size: usize, idle_after: Duration) -> PagePool {
        let mut limits = surimono_core::Limits::default();
        limits.time = Duration::from_secs(30);
        PagePool::with_idle_after(
            PagePoolOptions {
                sandbox: SandboxOptions::default().with_limits(limits),
                settle: SettleOptions::default(),
                dom: DomLimits::default(),
                pool_size,
            },
            idle_after,
        )
    }

    fn wait_until(what: &str, done: impl Fn() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(20), "{what}");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn no_thread_until_the_first_page_and_none_after_it() {
        let p = pool(2, Duration::from_millis(30));
        assert_eq!(p.workers(), 0);
        assert!(p.run("<p>x</p>", None, None).is_ok());
        wait_until("the worker ends when idle", || p.workers() == 0);
        // And the next page starts one again.
        assert!(p.run("<p>y</p>", None, None).is_ok());
    }

    #[test]
    fn a_caller_at_a_time_uses_one_worker() {
        let p = pool(4, Duration::from_secs(5));
        for _ in 0..5 {
            assert!(p.run("<p>x</p>", None, None).is_ok());
            assert!(p.workers() <= 1, "{}", p.workers());
        }
    }

    #[test]
    fn workers_stop_at_the_pool_size_and_pages_wait() {
        let p = Arc::new(pool(1, Duration::from_secs(5)));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let p = Arc::clone(&p);
                thread::spawn(move || p.run("<p>x</p>", None, None).is_ok())
            })
            .collect();
        for t in threads {
            assert!(matches!(t.join(), Ok(true)));
        }
        assert_eq!(p.workers(), 1);
    }

    #[test]
    fn a_cancelled_page_never_starts() {
        let p = pool(1, Duration::from_secs(5));
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            p.run("<p>x</p>", Some(&cancel), None),
            Err(Error::Cancelled)
        ));
        assert_eq!(p.workers(), 0);
    }

    #[test]
    fn dropping_a_pool_doesnt_wait_for_a_page() {
        let p = Arc::new(pool(1, Duration::from_secs(5)));
        let running = {
            let p = Arc::clone(&p);
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&cancel);
            let t = thread::spawn(move || {
                let _ = p.run("<script>for (;;) {}</script>", Some(&flag), None);
            });
            thread::sleep(Duration::from_millis(50));
            cancel.store(true, Ordering::Release);
            t
        };
        assert!(running.join().is_ok());
        let started = Instant::now();
        drop(p);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn every_limit_of_the_dom_is_a_public_error() {
        for (e, kind) in [
            (DomError::NodeLimit { limit: 1 }, "document-limit"),
            (DomError::TreeTooDeep { limit: 1 }, "document-limit"),
            (DomError::HandleLimit { limit: 1 }, "document-limit"),
            (DomError::TextLimit { limit: 1 }, "document-limit"),
            (
                DomError::StyleLimit {
                    declarations: 1,
                    tokens: 1,
                    bytes: 1,
                },
                "document-limit",
            ),
            (DomError::Unsupported { feature: "x" }, "unsupported"),
            (DomError::StaleDocument, "page"),
            (DomError::NoDocument, "page"),
            (
                DomError::Sandbox(surimono_core::SandboxError::Engine {
                    message: "m".into(),
                }),
                "page",
            ),
            (
                DomError::Sandbox(surimono_core::SandboxError::Cancelled),
                "cancelled",
            ),
        ] {
            assert_eq!(page_error(e).kind(), kind);
        }
    }
}
