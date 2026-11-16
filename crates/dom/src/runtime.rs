//! A sandbox with the DOM bindings installed, and the native half of them.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use surimono_core::rquickjs::context::EvalOptions;
use surimono_core::rquickjs::function::{Rest, This};
use surimono_core::rquickjs::{
    self, ArrayBuffer, Ctx, Exception, FromJs, Function, IntoJs, Object, Value,
};
use surimono_core::{
    Bytecode, Deadline, EventLoopHandle, ExternalWait, Limits, LoopBridge, Sandbox, SandboxError,
    SandboxOptions, SettleOptions, SettleOutcome, SettleReport, limit_error,
};
use surimono_webidl::runtime::{BindingError, ObjectId};

use crate::bindings::Args;
use crate::dispatch::dispatch;
use crate::document::Handle;
use crate::script::Prepared;
use crate::{
    Document, DomError, LayoutPolicy, LayoutRefusals, LayoutSetup, css_supports, media, sheet, web,
};

/// The JavaScript half of the bindings; see `glue_head.js`.
const GLUE: &str = concat!(
    include_str!("glue_head.js"),
    include_str!("glue_events.js"),
    include_str!("glue_abort.js"),
    include_str!("glue_location.js"),
    include_str!("glue_activation.js"),
    include_str!("generated/bindings.js"),
    include_str!("glue_platform.js"),
    include_str!("glue_html.js"),
    include_str!("glue_traversal.js"),
    include_str!("glue_sheet.js"),
    include_str!("glue_layout.js"),
    include_str!("glue_window.js"),
    include_str!("glue_tail.js"),
);

/// The name the glue's function has on the global object between
/// [`Sandbox::eval_bytecode`] and [`install`], with no page script between them.
const GLUE_GLOBAL: &str = "__surimonoGlue";

/// The glue, compiled once per process: it is about 150 KB of source, and every
/// page gets a new context.
static GLUE_BYTECODE: OnceLock<Bytecode> = OnceLock::new();

/// Puts the glue's function on the global object for [`install`].
fn load_glue(sandbox: &mut Sandbox) -> Result<(), SandboxError> {
    let bytecode = match GLUE_BYTECODE.get() {
        Some(b) => b,
        None => {
            let source = format!("globalThis.{GLUE_GLOBAL} = {GLUE}");
            let compiled = sandbox.compile(&source, "glue.js")?;
            GLUE_BYTECODE.get_or_init(|| compiled)
        }
    };
    sandbox.eval_bytecode(bytecode)
}

/// State the native functions share with [`DomRuntime`]. The functions
/// hold an `Rc` to this slot, not to a document: the document is moved in
/// by [`DomRuntime::load`] and out by [`DomRuntime::take`], so JavaScript
/// never keeps it (or any node) alive.
#[derive(Default)]
struct Host {
    doc: Option<Document>,
    /// The first fatal error of the current call (see [`DomError`]).
    fault: Option<DomError>,
    /// What layout queries do; survives resets and documents.
    layout: LayoutPolicy,
    /// What [`LayoutPolicy::Real`] lays out against; survives resets and
    /// documents.
    layout_setup: Option<Rc<LayoutSetup>>,
    /// The sandbox's cancel flag, raised by `limit`: a limit reported by the
    /// glue must stop the script like one the engine stops it for. Natives
    /// refuse to work while it is up, because the sandbox's watchdog sees it
    /// a few milliseconds later. Raised by this file, the fault says why, so
    /// a caller sees that limit; raised through
    /// [`DomRuntime::cancel_flag`], there is no fault and the call is
    /// `Cancelled`.
    stop: Arc<AtomicBool>,
    /// The current call's deadline; host functions refuse to work after it.
    deadline: Option<Deadline>,
    /// The page's scripts, for [`DomRuntime::run_page`].
    scripts: Vec<PageScript>,
    /// What scripts a script inserted are waiting on; see [`ScriptLoads`].
    script_loads: ScriptLoads,
    /// The sandbox's event loop, for [`ScriptLoads::wait`]; set for each call.
    event_loop: Option<EventLoopHandle>,
    /// Exceptions reported since the last report, and the first few.
    uncaught: u64,
    errors: Vec<String>,
    /// Promises a [`DomRuntime::checkpoint`] counted, since the last report,
    /// and the first few reasons. The queued tasks that decide those run in
    /// the event loop, so with the hook in place this stays empty until
    /// something runs them.
    unhandled_rejections: u64,
    rejections: Vec<String>,
    /// The sandbox's `Math.random` seed, which `crypto.getRandomValues` draws
    /// from too (`glue_window.js`).
    random_seed: u64,
}

/// Most reported exception messages kept; the rest are only counted.
const MAX_KEPT_ERRORS: usize = 8;

/// Longest message kept, as for the sandbox's own errors: they live outside
/// the JavaScript heap.
const MAX_MESSAGE_BYTES: usize = 4 * 1024;

/// `message` cut to [`MAX_MESSAGE_BYTES`] on a character boundary.
fn capped(mut message: String) -> String {
    if message.len() > MAX_MESSAGE_BYTES {
        let mut cut = MAX_MESSAGE_BYTES;
        while !message.is_char_boundary(cut) {
            cut -= 1;
        }
        message.truncate(cut);
        message.push('…');
    }
    message
}

/// The text of an error in a report.
fn message_of(error: &SandboxError) -> String {
    match error {
        SandboxError::Exception { message, .. } => capped(message.clone()),
        other => other.to_string(),
    }
}

/// Adds `message` to a list of kept messages, unless it is full.
fn keep(kept: &mut Vec<String>, message: String) {
    if kept.len() < MAX_KEPT_ERRORS {
        kept.push(message);
    }
}

/// `run_until_settled` with these options is a microtask checkpoint and no
/// more: the task limit stops every timer before it runs, and an animation
/// frame, always due after time 0, is past the budget.
const CHECKPOINT: SettleOptions = SettleOptions {
    virtual_budget_ms: 0,
    max_tasks: 0,
};

/// Most times a page's frames are loaded in turn: a frame that a load's
/// handler adds takes the next round. Each round is a full settle of the
/// event loop, so this bounds the work with the time limit, which covers them
/// all.
const MAX_FRAME_ROUNDS: usize = 8;

/// Most fetches the scripts a script inserted may ask for in one render;
/// the rest fail (an `error` event, and `missing_scripts`) without reaching
/// the resolver. The facade's own cap of 1,000 loads comes first for those it
/// allows; this one bounds what is held here for those it refuses.
const MAX_SCRIPT_FETCHES: usize = 10_000;

/// Most scripts a script inserted that may run one inside another: an
/// inline script that inserts an inline script runs it before its own
/// `appendChild` returns. One more fails the call with
/// [`SandboxError::StackOverflow`], at this depth in release builds with the
/// default stack, where it comes before the stack limit with about 1.5 times
/// the room; a debug build's frames reach the stack limit first, at about 25,
/// with the same error. Chromium 153 reports a `RangeError` after 44.
const MAX_SCRIPT_NESTING: usize = 32;

/// Most bytes of `src` those fetches may hold, together, until they are asked
/// for; past it they fail the same way.
const MAX_SCRIPT_FETCH_BYTES: usize = 16 * 1024 * 1024;

/// The scripts a script inserted, between "prepare the script element" and
/// their run: all of it goes at the end of a render, and when a document is
/// loaded or the context reset, so a pooled runtime keeps nothing per script.
#[derive(Default)]
struct ScriptLoads {
    /// Sources to run, by element: (source, name for its errors). An inline
    /// script's for the moment between prepare and run; a fetched one's
    /// until its task runs.
    sources: HashMap<Handle, (String, String)>,
    /// Fetches asked for and not yet given to the resolver, in order: the
    /// element and its `src`, or `None` for one past a cap, which fails in its
    /// place in that order without reaching the resolver (and holds no URL).
    asked: Vec<(Handle, Option<String>)>,
    /// Fetches asked for in this render, and the bytes of their URLs.
    count: usize,
    bytes: usize,
    /// Fetched, in the order asked: the element, and whether it has a source.
    fetched: VecDeque<(Handle, bool)>,
    /// The external wait that holds the event loop's clock while `asked`
    /// isn't empty (see `surimono_core`'s event loop).
    wait: Option<ExternalWait>,
    /// `src` values of fetches that failed: the resolver had no source, or
    /// a cap above was reached.
    missing: Vec<String>,
    /// Scripts a script inserted that are running now, one inside another.
    depth: usize,
}

/// One `<script>` of the page.
struct PageScript {
    node: Handle,
    /// `None` for a script whose source couldn't be had.
    source: Option<String>,
    name: String,
    external: bool,
    deferred: bool,
}

/// What [`DomRuntime::run_page`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageReport {
    /// The event loop's report.
    pub settle: SettleReport,
    /// Exceptions no script caught and no `error` listener cancelled, in
    /// scripts, event listeners and callbacks of timers, frames and
    /// microtasks.
    pub uncaught: u64,
    /// The first few, as messages, each cut to a few KiB.
    pub errors: Vec<String>,
    /// Promises rejected with no handler at the end of a microtask
    /// checkpoint: an exception in an async function or a promise reaction
    /// that no script handled. They are not in `uncaught`: no `error` event
    /// fires for them, so no listener can cancel them. A limit nearly never
    /// is, since it fails the call: the exception is one reached while Rust
    /// reads the reason (see "Errors" in `DESIGN.md`).
    pub unhandled_rejections: u64,
    /// The first few reasons, as messages, each cut to a few KiB.
    pub rejections: Vec<String>,
    /// `src` values of external scripts the resolver had no source for: the
    /// parser's, then those scripts inserted, in the order asked.
    pub missing_scripts: Vec<String>,
    /// Layouts the page's scripts asked for that were refused, by reason
    /// (internal, with real layout).
    #[doc(hidden)]
    pub layout_refusals: LayoutRefusals,
}

type Shared = Rc<RefCell<Host>>;

/// A [`Sandbox`] with `document` and the bindings generated from WebIDL,
/// resolving every wrapper through the loaded [`Document`] on each call.
///
/// Internal phase-2 scaffolding; not part of the facade.
pub struct DomRuntime {
    sandbox: Sandbox,
    host: Shared,
    /// Whether the sandbox's current context has the bindings. Cleared when
    /// the sandbox rebuilds itself (after a limit error) or is reset.
    installed: bool,
}

impl std::fmt::Debug for DomRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DomRuntime")
            .field("sandbox", &self.sandbox)
            .field("installed", &self.installed)
            .finish_non_exhaustive()
    }
}

impl DomRuntime {
    /// A runtime whose scripts run under `limits`, with no document loaded.
    pub fn new(limits: Limits) -> Result<Self, DomError> {
        Self::with_options(SandboxOptions::default().with_limits(limits))
    }

    /// A runtime from full sandbox options.
    pub fn with_options(options: SandboxOptions) -> Result<Self, DomError> {
        let host = Shared::default();
        host.borrow_mut().random_seed = options.random_seed;
        let mut sandbox = Sandbox::with_options(options.with_event_loop(true))?;
        sandbox.set_cancel_flag(Some(host.borrow().stop.clone()));
        Ok(Self {
            sandbox,
            host,
            installed: false,
        })
    }

    /// Makes `doc` the document scripts see, returning the one it replaces.
    /// Wrappers for the old document now fail with
    /// [`DomError::StaleDocument`].
    pub fn load(&mut self, doc: Document) -> Option<Document> {
        self.host.borrow_mut().doc.replace(doc)
    }

    /// The flag that stops this runtime's call: raise it from any thread and
    /// the script stops within a few milliseconds, native loops and parsers
    /// included, frames' parses too, and the call fails with
    /// `SandboxError::Cancelled`. The
    /// call consumes it: the runtime rebuilds its context and works again. Raised
    /// between calls, it stops the next one.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.host.borrow().stop.clone()
    }

    /// Chooses what scripts' layout queries do, from the next call on. The
    /// default is [`LayoutPolicy::Zeros`].
    pub fn set_layout_policy(&mut self, policy: LayoutPolicy) {
        self.host.borrow_mut().layout = policy;
    }

    /// Gives [`LayoutPolicy::Real`] its viewport and fonts, from the next
    /// call on. Internal: the E3-00 prototype, not reachable from the facade.
    #[doc(hidden)]
    pub fn set_layout_setup(&mut self, setup: LayoutSetup) {
        self.host.borrow_mut().layout_setup = Some(Rc::new(setup));
    }

    /// Layouts the loaded document has run, for tests of when layout runs.
    #[doc(hidden)]
    pub fn layouts_run(&self) -> u64 {
        self.host
            .borrow()
            .doc
            .as_ref()
            .map_or(0, Document::layouts_run)
    }

    /// Layouts of the loaded document that were refused, by reason: a
    /// refused page answers every layout query as with no box, so this is
    /// how a caller tells it from boxes that are really empty.
    #[doc(hidden)]
    pub fn layout_refusals(&self) -> LayoutRefusals {
        self.host
            .borrow()
            .doc
            .as_ref()
            .map(|d| d.layout_refusals().clone())
            .unwrap_or_default()
    }

    /// Takes the loaded document back. Wrappers still held by scripts fail
    /// with [`DomError::NoDocument`] until another is loaded, then with
    /// [`DomError::StaleDocument`].
    pub fn take(&mut self) -> Option<Document> {
        self.host.borrow_mut().doc.take()
    }

    /// Runs `f` with the loaded document, if any.
    pub fn with_document<R>(&self, f: impl FnOnce(&Document) -> R) -> Option<R> {
        self.host.borrow().doc.as_ref().map(f)
    }

    /// Discards all script state (and every wrapper), keeping the document.
    pub fn reset(&mut self) -> Result<(), DomError> {
        self.installed = false;
        self.host.borrow_mut().script_loads = ScriptLoads::default();
        self.sandbox.reset()?;
        Ok(())
    }

    /// The sandbox, e.g. for its heap size.
    pub fn sandbox(&self) -> &Sandbox {
        &self.sandbox
    }

    /// Evaluates a classic script (sloppy mode, like `<script>`) against the
    /// loaded document and converts its completion value to `T`. The script
    /// and its microtasks are a task: a limit reached in them fails the call,
    /// whether in the script, a listener or a promise nothing handled, and a
    /// promise left rejected counts in the next report.
    ///
    /// The script and that checkpoint are two sandbox calls with a deadline
    /// each, so a hostile script can use up to twice [`Limits::time`].
    pub fn eval<T>(&mut self, source: &str) -> Result<T, DomError>
    where
        T: for<'js> FromJs<'js>,
    {
        let result = self.eval_script(source);
        // Also after a script's exception: its microtasks ran all the same.
        self.checkpoint()?;
        result
    }

    /// The end of a task outside the event loop: what `Sandbox::run` leaves
    /// out. What it reports waits for the next report; an error is a limit
    /// in a rejected promise (or a sandbox that can't run at all).
    fn checkpoint(&mut self) -> Result<(), DomError> {
        self.begin_call();
        let result = self.sandbox.run_until_settled(&CHECKPOINT);
        let report = self.finish_call(result)?;
        let mut guard = self.host.borrow_mut();
        let host = &mut *guard;
        host.uncaught += report.uncaught;
        for e in &report.errors {
            keep(&mut host.errors, message_of(e));
        }
        host.unhandled_rejections += report.unhandled_rejections;
        for e in &report.rejections {
            keep(&mut host.rejections, message_of(e));
        }
        Ok(())
    }

    fn eval_script<T>(&mut self, source: &str) -> Result<T, DomError>
    where
        T: for<'js> FromJs<'js>,
    {
        if !self.installed {
            self.begin_call();
            let loaded = load_glue(&mut self.sandbox);
            self.finish_call(loaded)?;
        }
        self.begin_call();
        let Self {
            sandbox,
            host,
            installed,
        } = self;
        let bridge = sandbox.loop_bridge();
        let result = sandbox.run(|ctx| {
            if !*installed {
                install(ctx, host, bridge)?;
                *installed = true;
            }
            let mut opts = EvalOptions::default();
            opts.global = true;
            opts.strict = false;
            opts.filename = Some("<script>".to_owned());
            ctx.eval_with_options(source, opts)
        });
        self.finish_call(result)
    }

    /// Runs timers, frames and microtasks until the page settles (see
    /// `surimono_core`'s event loop), with the bindings in place. The report
    /// includes exceptions reported since the last one, wherever they came
    /// from: the bindings report them through the window's `error` event.
    /// Likewise promises left rejected, including by an [`eval`](Self::eval).
    pub fn run_until_settled(&mut self, options: &SettleOptions) -> Result<SettleReport, DomError> {
        // Installs the bindings if no script ran yet.
        self.eval_script::<()>("")?;
        let began = Instant::now();
        self.begin_call();
        let result = self.sandbox.run_until_settled(options);
        let mut report = self.finish_call(result)?;
        // With no resolver, a fetch a script inserted asks for fails.
        while report.outcome == SettleOutcome::WaitingExternal {
            self.fetch_inserted_scripts(&mut |_| None);
            if began.elapsed() >= self.sandbox.limits().time {
                return Err(SandboxError::TimeLimit.into());
            }
            self.begin_call();
            let fired = self.sandbox.call::<_, ()>("__surimonoFrameLoads", ());
            self.finish_call(fired)?;
            self.begin_call();
            let result = self.sandbox.run_until_settled(options);
            let next = self.finish_call(result)?;
            let errors = std::mem::take(&mut report.errors);
            let rejections = std::mem::take(&mut report.rejections);
            let (uncaught, rejected) = (report.uncaught, report.unhandled_rejections);
            report = next;
            report.uncaught += uncaught;
            report.unhandled_rejections += rejected;
            let mut kept: Vec<SandboxError> = errors;
            kept.append(&mut report.errors);
            kept.truncate(MAX_KEPT_ERRORS);
            report.errors = kept;
            let mut kept: Vec<SandboxError> = rejections;
            kept.append(&mut report.rejections);
            kept.truncate(MAX_KEPT_ERRORS);
            report.rejections = kept;
        }
        self.host.borrow_mut().script_loads.missing.clear();
        let mut host = self.host.borrow_mut();
        report.uncaught += std::mem::take(&mut host.uncaught);
        for message in std::mem::take(&mut host.errors) {
            if report.errors.len() < MAX_KEPT_ERRORS {
                report.errors.push(SandboxError::Exception {
                    message,
                    stack: None,
                });
            }
        }
        // These rejected before any in the report.
        report.unhandled_rejections += std::mem::take(&mut host.unhandled_rejections);
        let mut rejections: Vec<SandboxError> = std::mem::take(&mut host.rejections)
            .into_iter()
            .map(|message| SandboxError::Exception {
                message,
                stack: None,
            })
            .collect();
        rejections.append(&mut report.rejections);
        rejections.truncate(MAX_KEPT_ERRORS);
        report.rejections = rejections;
        Ok(report)
    }

    /// Runs the loaded document as a page, in a fresh context: its scripts in order, then
    /// `DOMContentLoaded` and `load`, then the event loop until it settles;
    /// all under one time limit, and each round of frames a script added
    /// under another, checked between rounds, so at most about twice it.
    /// `<iframe>`s load as documents without a browsing context of their own:
    /// see "Frames" in DESIGN.md. The order, all at virtual time 0 and before
    /// any of the page's own timers, each step a task (HTML §13.2.7):
    ///
    /// 1. `readyState` is `loading`. Scripts in tree order, except `defer`.
    ///    An external script is followed by `load` at its element, or
    ///    `error` if `resolve` had no source for it. The whole tree is there
    ///    from the start: the parser has finished before any script runs.
    /// 2. `readyState` is `interactive`, with `readystatechange`.
    /// 3. `defer` scripts in tree order.
    /// 4. `DOMContentLoaded` at the document; it bubbles to the window.
    /// 5. `readyState` is `complete`, with `readystatechange`.
    /// 6. `load` at the window, with the document as its target.
    ///
    /// A page can't cancel these steps with `clearTimeout`. They do count
    /// as tasks: with [`SettleOptions::max_tasks`] below their number the
    /// page stops part way, with [`SettleOutcome::TaskLimit`](surimono_core::SettleOutcome).
    ///
    /// An uncaught exception doesn't stop the page, wherever it is thrown
    /// (scripts, listeners, timers, frames, microtasks): it is an `error`
    /// event at the window, and is counted in the report unless a listener
    /// cancels that event. An async function or promise reaction that throws
    /// leaves a rejected promise; if no script handles it by the time of the
    /// task HTML's "notify about rejected promises" queues, that task fires
    /// `unhandledrejection` at the window, and unless a listener cancels the
    /// event the promise is counted in [`PageReport::unhandled_rejections`].
    ///
    /// A limit is different: an uncaught stack overflow fails the call with
    /// the same error everywhere, promises included, with no `error` event.
    /// Handling the promise in time is catching it, a script's own matter.
    ///
    /// A script a script inserts runs as HTML §4.12.1.1 says (see "A script a
    /// script inserts" in DESIGN.md): an inline one at once, one with `src`
    /// fetched once nothing else is due at that virtual time, `resolve`
    /// called for each in the order asked, between calls, and run in that
    /// order. Steps 5 and 6 wait for them. Each round of fetches runs under
    /// a time limit of its own, checked between rounds, as frames' do.
    ///
    /// `resolve` gives the source of an external script from its `src`
    /// value. No network and no files unless the caller's closure reads them.
    pub fn run_page(
        &mut self,
        options: &SettleOptions,
        mut resolve: impl FnMut(&str) -> Option<String>,
    ) -> Result<PageReport, DomError> {
        let mut missing_scripts = Vec::new();
        {
            let mut guard = self.host.borrow_mut();
            let host = &mut *guard;
            let doc = host.doc.as_mut().ok_or(DomError::NoDocument)?;
            doc.set_ready(0);
            let mut scripts = Vec::new();
            for (node, src, text, deferred) in doc.scripts() {
                let source = match &src {
                    Some(url) => {
                        let got = resolve(url);
                        if got.is_none() {
                            missing_scripts.push(url.clone());
                        }
                        got
                    }
                    None => Some(text),
                };
                scripts.push(PageScript {
                    node: doc.handle_for(node)?,
                    source,
                    name: src.clone().unwrap_or_else(|| "<script>".to_owned()),
                    external: src.is_some(),
                    deferred,
                });
            }
            host.scripts = scripts;
            host.script_loads = ScriptLoads::default();
            host.uncaught = 0;
            host.errors.clear();
            host.unhandled_rejections = 0;
            host.rejections.clear();
        }
        // The parser's frames are asked for here and parsed in the first step
        // of the lifecycle, before any script, and their `load` events fire
        // before the window's, as they must: a page's `load` listener reads
        // them. A script sees a frame as loaded from the start.
        self.load_frames(&mut resolve)?;
        self.reset()?;
        self.begin_call();
        let loaded = load_glue(&mut self.sandbox);
        self.finish_call(loaded)?;
        self.begin_call();
        let Self {
            sandbox,
            host,
            installed,
        } = self;
        let bridge = sandbox.loop_bridge();
        let started = sandbox.run(|ctx| {
            let start = install(ctx, host, bridge)?;
            *installed = true;
            let g = host
                .borrow()
                .doc
                .as_ref()
                .map_or(0.0, |d| d.generation() as f64);
            start.call::<_, ()>((g,))
        });
        self.finish_call(started)?;
        let began = Instant::now();
        self.begin_call();
        let result = self.sandbox.run_until_settled(options);
        let mut settle = self.finish_call(result)?;
        let mut uncaught = settle.uncaught;
        let mut errors = Vec::new();
        let mut rejected = settle.unhandled_rejections;
        let mut rejections: Vec<String> = settle.rejections.iter().map(message_of).collect();
        for e in &settle.errors {
            keep(&mut errors, message_of(e));
        }
        let mut frame_rounds = 0;
        loop {
            // A fetch a script asked for held the clock: the loop stopped with
            // nothing else due at that time, whatever the loader's speed.
            if settle.outcome == SettleOutcome::WaitingExternal {
                self.fetch_inserted_scripts(&mut resolve);
            } else if frame_rounds < MAX_FRAME_ROUNDS && self.load_frames(&mut resolve)? {
                frame_rounds += 1;
            } else {
                break;
            }
            if began.elapsed() >= self.sandbox.limits().time {
                return Err(SandboxError::TimeLimit.into());
            }
            self.begin_call();
            let fired = self.sandbox.call::<_, ()>("__surimonoFrameLoads", ());
            self.finish_call(fired)?;
            self.begin_call();
            let result = self.sandbox.run_until_settled(options);
            settle = self.finish_call(result)?;
            uncaught += settle.uncaught;
            rejected += settle.unhandled_rejections;
            for e in &settle.errors {
                keep(&mut errors, message_of(e));
            }
            for e in &settle.rejections {
                keep(&mut rejections, message_of(e));
            }
        }
        let mut host = self.host.borrow_mut();
        host.scripts.clear();
        let loads = std::mem::take(&mut host.script_loads);
        if let (Some(wait), Some(handle)) = (loads.wait, &host.event_loop) {
            handle.external_end(wait);
        }
        missing_scripts.extend(loads.missing);
        let mut all = std::mem::take(&mut host.errors);
        for e in errors {
            keep(&mut all, e);
        }
        settle.uncaught = uncaught;
        settle.unhandled_rejections = rejected;
        Ok(PageReport {
            uncaught: uncaught + std::mem::take(&mut host.uncaught),
            errors: all,
            unhandled_rejections: rejected,
            rejections,
            settle,
            missing_scripts,
            layout_refusals: host
                .doc
                .as_ref()
                .map(|d| d.layout_refusals().clone())
                .unwrap_or_default(),
        })
    }

    /// Starts the loads of `<iframe>`s nothing has loaded yet (HTML §4.8.5):
    /// `srcdoc`, the source of `src` from the page's resolver, or their
    /// `about:blank`. Whether any was started. Asks the resolver here, between
    /// calls, and parses nothing: a parse belongs in a sandbox call, the only
    /// place the time limit and a cancel can be seen, so the glue's next step
    /// does it, one native call per frame.
    fn load_frames(
        &mut self,
        resolve: &mut impl FnMut(&str) -> Option<String>,
    ) -> Result<bool, DomError> {
        let mut guard = self.host.borrow_mut();
        let Some(doc) = guard.doc.as_mut() else {
            return Ok(false);
        };
        let frames = doc.frames_to_load();
        let any = !frames.is_empty();
        for (element, srcdoc, src) in frames {
            match (srcdoc, src) {
                (Some(text), _) => doc.queue_frame(element, Some(text), "about:srcdoc".to_owned()),
                (None, Some(src)) => {
                    // The fragment is the document's to read, not the source's.
                    let plain = src.split('#').next().unwrap_or(&src);
                    let url = doc.frame_url(&src);
                    doc.queue_frame(element, resolve(plain), url);
                }
                (None, None) => doc.queue_frame(element, None, "about:blank".to_owned()),
            }
        }
        Ok(any)
    }

    fn begin_call(&mut self) {
        let mut host = self.host.borrow_mut();
        host.fault = None;
        host.deadline = Some(self.sandbox.deadline());
        host.event_loop = self.sandbox.event_loop();
    }

    /// Gives the resolver the fetches the scripts a script inserted asked
    /// for, in the order they asked, and ends the wait that held the clock.
    /// Whether there were any; their sources wait for the glue's tasks.
    fn fetch_inserted_scripts(&mut self, resolve: &mut impl FnMut(&str) -> Option<String>) -> bool {
        let (asked, wait, handle) = {
            let mut host = self.host.borrow_mut();
            let loads = &mut host.script_loads;
            (
                std::mem::take(&mut loads.asked),
                loads.wait.take(),
                host.event_loop.clone(),
            )
        };
        let ours = wait.is_some();
        if let (Some(wait), Some(handle)) = (wait, &handle) {
            handle.external_end(wait);
        }
        if !ours {
            // The loop waits on something that isn't ours: nothing else in
            // the DOM opens a wait yet, and one no one ends would hold the
            // clock for ever. A wait of ours always comes with a fetch asked
            // for, so this never gives up on another's wait in the same round.
            if let Some(handle) = &handle {
                handle.external_abandon_all();
            }
            return false;
        }
        for (element, url) in asked {
            // The loader runs here, between sandbox calls, one at a time; a
            // fetch past a cap fails in its place without it.
            let source = url.as_deref().and_then(&mut *resolve);
            let mut host = self.host.borrow_mut();
            let loads = &mut host.script_loads;
            let ok = source.is_some();
            match (source, url) {
                (Some(source), Some(url)) => {
                    loads.sources.insert(element, (source, url));
                }
                (None, Some(url)) => loads.missing.push(url),
                (_, None) => {}
            }
            loads.fetched.push_back((element, ok));
        }
        true
    }

    fn finish_call<T>(&mut self, result: Result<T, SandboxError>) -> Result<T, DomError> {
        if matches!(&result, Err(e) if e.discards_state()) {
            // The sandbox rebuilt its context; the bindings went with it.
            self.installed = false;
        }
        // Lowered only now that the call is over: the sandbox's watchdog
        // reads the flag every few milliseconds.
        let stopped = self.host.borrow().stop.swap(false, Ordering::AcqRel);
        if matches!(&result, Err(SandboxError::HostPanic { .. })) {
            // The panic stopped a native halfway through a change to the
            // tree, so the document is no more to be trusted than the
            // script's state. Its links may be wrong, but a slab drops whole.
            self.host.borrow_mut().doc = None;
        }
        let fault = {
            let mut host = self.host.borrow_mut();
            host.deadline = None;
            host.fault.take()
        };
        if stopped && !matches!(&result, Err(e) if e.discards_state()) {
            // The call ended before the watchdog looked. Same outcome: the
            // script stopped at a limit, so its state goes.
            let _ = self.reset();
        }
        match (fault, result) {
            // The limit `stop` was raised for.
            (Some(fault), Err(SandboxError::Cancelled)) if stopped => Err(fault),
            // These outrank a fault: the script was stopped anyway.
            (_, Err(e)) if e.discards_state() => Err(e.into()),
            (Some(fault), _) => Err(fault),
            // The caller's cancel, seen by a native or as the call ended by
            // itself, before the watchdog looked.
            (None, _) if stopped => Err(SandboxError::Cancelled.into()),
            (None, r) => r.map_err(DomError::from),
        }
    }
}

/// Installs the bindings in `ctx`: builds the native functions and hands
/// them to the glue, which keeps them in a closure. Returns the glue's
/// function that queues a page's lifecycle.
fn install<'js>(
    ctx: &Ctx<'js>,
    host: &Shared,
    bridge: Option<LoopBridge>,
) -> rquickjs::Result<Function<'js>> {
    let n = Object::new(ctx.clone())?;
    // `DomRuntime::new` always asks for the event loop.
    let Some(bridge) = bridge else {
        return Err(Exception::throw_internal(
            ctx,
            "the sandbox has no event loop",
        ));
    };
    let bridge = Rc::new(bridge);
    n.set("checkpoint", native_checkpoint(ctx, host, &bridge)?)?;
    n.set(
        "setRejectionHook",
        native_rejection_hook(ctx, host, &bridge)?,
    )?;
    n.set("call", native_call(ctx, host)?)?;
    n.set("current", native_current(ctx, host)?)?;
    n.set("currentRoot", native_current_root(ctx, host)?)?;
    n.set("stale", native_stale(ctx, host)?)?;
    n.set("report", native_report(ctx, host)?)?;
    n.set("limit", native_limit(ctx, host)?)?;
    n.set("runSource", native_run_source(ctx, host)?)?;
    n.set("throwDom", native_throw_dom(ctx, host)?)?;
    n.set("scriptCount", native_script_count(ctx, host)?)?;
    n.set("scriptNode", script_field(ctx, host, |s| s.node)?)?;
    n.set("scriptDeferred", script_field(ctx, host, |s| s.deferred)?)?;
    n.set("scriptExternal", script_field(ctx, host, |s| s.external)?)?;
    n.set(
        "scriptFailed",
        script_field(ctx, host, |s| s.source.is_none())?,
    )?;
    n.set("runScript", native_run_script(ctx, host)?)?;
    n.set("prepareScript", native_prepare_script(ctx, host)?)?;
    n.set("runPrepared", native_run_prepared(ctx, host)?)?;
    n.set("nextFetched", native_next_fetched(ctx, host)?)?;
    install_web(ctx, host, &n)?;
    let globals = ctx.globals();
    let glue: Function<'js> = globals.get(GLUE_GLOBAL)?;
    globals.remove(GLUE_GLOBAL)?;
    glue.call((n,))
}

/// The natives of `glue_platform.js`.
fn install_web<'js>(ctx: &Ctx<'js>, host: &Shared, n: &Object<'js>) -> rquickjs::Result<()> {
    n.set("urlParse", native_url_parse(ctx, host)?)?;
    n.set("urlGet", native_url_get(ctx, host)?)?;
    n.set("urlSet", native_url_set(ctx, host)?)?;
    n.set("formEncode", text_native(ctx, host, web::form_encode)?)?;
    n.set("formDecode", text_native(ctx, host, web::form_decode)?)?;
    n.set("encodingName", text_native(ctx, host, web::encoding_name)?)?;
    n.set(
        "base64Encode",
        maybe_text_native(ctx, host, web::base64_encode)?,
    )?;
    n.set(
        "base64Decode",
        maybe_text_native(ctx, host, web::base64_decode)?,
    )?;
    n.set("cssRules", text_native(ctx, host, sheet::rules_for_script)?)?;
    n.set("cssRule", maybe_text_native(ctx, host, sheet::one_rule)?)?;
    n.set("utf8Encode", native_utf8_encode(ctx, host)?)?;
    n.set("decoderOpen", native_decoder_open(ctx, host)?)?;
    n.set("decoderFeed", native_decoder_feed(ctx, host)?)?;
    n.set("valueKind", native_value_kind(ctx, host)?)?;
    install_window(ctx, host, n)
}

/// The natives of `glue_window.js`.
fn install_window<'js>(ctx: &Ctx<'js>, host: &Shared, n: &Object<'js>) -> rquickjs::Result<()> {
    // The seed as two 32-bit halves, as the prelude takes it.
    let seed = host.borrow().random_seed;
    n.set("seedLo", (seed & 0xffff_ffff) as f64)?;
    n.set("seedHi", (seed >> 32) as f64)?;
    n.set("matchMedia", native_match_media(ctx, host)?)?;
    n.set("cssSupports", native_css_supports(ctx, host)?)?;
    n.set("secureContext", native_secure_context(ctx, host)?)?;
    Ok(())
}

/// `matchMedia(query, width, height, dpr)`: two flags, then the serialization:
/// whether it matches, whether that read the viewport's size.
fn native_match_media<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              query: String,
              width: f64,
              height: f64,
              dpr: f64|
              -> rquickjs::Result<String> {
            guarded(&host, &ctx, || {
                let env = media::Env { width, height, dpr };
                let e = media::match_media(&query, &env);
                let flag = |b: bool| if b { '1' } else { '0' };
                let mut out = String::with_capacity(e.text.len() + 2);
                out.push(flag(e.matches));
                out.push(flag(e.sized));
                out.push_str(&e.text);
                Ok(out)
            })
        },
    )
}

/// `cssSupports(a, b)`: `CSS.supports(a, b)`, or with `b` undefined
/// `CSS.supports(a)`.
fn native_css_supports<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, a: String, b: Option<String>| -> rquickjs::Result<bool> {
            guarded(&host, &ctx, || {
                Ok(match &b {
                    Some(value) => css_supports::supports_declaration(&a, value),
                    None => css_supports::supports_condition(&a),
                })
            })
        },
    )
}

/// `secureContext()`: whether the loaded page's URL is potentially
/// trustworthy; with no document, as for `about:blank`.
fn native_secure_context<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>| -> rquickjs::Result<bool> {
            guarded(&host, &ctx, || {
                let guard = host.borrow();
                Ok(guard
                    .doc
                    .as_ref()
                    .is_none_or(|d| web::potentially_trustworthy(d.url(d.root()))))
            })
        },
    )
}

/// A native from one string to another, for what needs no document.
fn text_native<'js>(
    ctx: &Ctx<'js>,
    host: &Shared,
    f: fn(&str) -> String,
) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, text: String| -> rquickjs::Result<String> {
            guarded(&host, &ctx, || Ok(f(&text)))
        },
    )
}

/// As [`text_native`], with null for no answer.
fn maybe_text_native<'js>(
    ctx: &Ctx<'js>,
    host: &Shared,
    f: fn(&str) -> Option<String>,
) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, text: String| -> rquickjs::Result<Option<String>> {
            guarded(&host, &ctx, || Ok(f(&text)))
        },
    )
}

/// `urlParse(input, base)`: the serialized URL, or null. `base` may be undefined.
fn native_url_parse<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              input: String,
              base: Option<String>|
              -> rquickjs::Result<Option<String>> {
            guarded(&host, &ctx, || Ok(web::url_parse(&input, base.as_deref())))
        },
    )
}

/// `urlGet(href, part)`.
fn native_url_get<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, href: String, part: String| -> rquickjs::Result<String> {
            guarded(&host, &ctx, || Ok(web::url_get(&href, &part)))
        },
    )
}

/// `urlSet(href, part, value)`: the new serialization, or null past the cap.
fn native_url_set<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              href: String,
              part: String,
              value: String|
              -> rquickjs::Result<Option<String>> {
            guarded(&host, &ctx, || Ok(web::url_set(&href, &part, &value)))
        },
    )
}

/// `utf8Encode(text)`: an ArrayBuffer of its bytes. A copy into the engine's own
/// memory, so it counts against the memory limit; `ArrayBuffer::new` would keep
/// the bytes outside it. The glue makes the view, since rquickjs's `TypedArray`
/// finds its constructor where a script can reach it.
fn native_utf8_encode<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, text: String| -> rquickjs::Result<ArrayBuffer<'js>> {
            guarded(&host, &ctx, || {
                ArrayBuffer::new_copy(ctx.clone(), text.as_bytes())
            })
        },
    )
}

/// `decoderOpen(name, ignoreBOM)`: a decoder's slot in the document.
fn native_decoder_open<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, name: String, ignore_bom: bool| -> rquickjs::Result<u32> {
            guarded(&host, &ctx, || {
                let mut guard = host.borrow_mut();
                let host = &mut *guard;
                let Some(doc) = host.doc.as_mut() else {
                    return Err(fault(&ctx, &mut host.fault, DomError::NoDocument));
                };
                match doc.decoder_open(&name, ignore_bom) {
                    Some(slot) => Ok(slot),
                    None => {
                        let e = doc.fault.take().unwrap_or(DomError::NoDocument);
                        Err(fault(&ctx, &mut host.fault, e))
                    }
                }
            })
        },
    )
}

/// `decoderFeed(slot, bytes, last, fatal)`: `bytes` holds a byte per character;
/// the text, or null for malformed input under `fatal`.
fn native_decoder_feed<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              slot: u32,
              bytes: String,
              last: bool,
              fatal: bool|
              -> rquickjs::Result<Option<String>> {
            guarded(&host, &ctx, || {
                let mut guard = host.borrow_mut();
                let host = &mut *guard;
                let Some(doc) = host.doc.as_mut() else {
                    return Err(fault(&ctx, &mut host.fault, DomError::NoDocument));
                };
                Ok(doc.decoder_feed(slot, &bytes, last, fatal))
            })
        },
    )
}

/// `valueKind(v)`, what no script-visible member tells: 1 for a Proxy, 2 for an
/// Error, 3 for a Promise, otherwise 0.
fn native_value_kind<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, v: Value<'js>| -> rquickjs::Result<u32> {
            guarded(&host, &ctx, || {
                Ok(if v.is_proxy() {
                    1
                } else if v.is_error() {
                    2
                } else if v.is_promise() {
                    3
                } else {
                    0
                })
            })
        },
    )
}

/// `checkpoint()`: the microtask checkpoint of "clean up after running
/// script", for the glue to call with no script of the page on the stack. An
/// error from it ends the call, as in `limit`.
fn native_checkpoint<'js>(
    ctx: &Ctx<'js>,
    host: &Shared,
    bridge: &Rc<LoopBridge>,
) -> rquickjs::Result<Function<'js>> {
    let (host, bridge) = (host.clone(), bridge.clone());
    Function::new(ctx.clone(), move |ctx: Ctx<'js>| -> rquickjs::Result<()> {
        guarded(&host, &ctx, || {
            let Err(error) = bridge.checkpoint(&ctx) else {
                return Ok(());
            };
            let mut host = host.borrow_mut();
            host.fault.get_or_insert(error.into());
            host.stop.store(true, Ordering::Release);
            Err(Exception::throw_internal(&ctx, "the call has ended"))
        })
    })
}

/// `setRejectionHook(f)`: see [`LoopBridge::set_rejection_hook`].
fn native_rejection_hook<'js>(
    ctx: &Ctx<'js>,
    host: &Shared,
    bridge: &Rc<LoopBridge>,
) -> rquickjs::Result<Function<'js>> {
    let (host, bridge) = (host.clone(), bridge.clone());
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, hook: Function<'js>| -> rquickjs::Result<()> {
            guarded(&host, &ctx, || {
                bridge.set_rejection_hook(&ctx, hook);
                Ok(())
            })
        },
    )
}

fn native_script_count<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(ctx.clone(), move |ctx: Ctx<'js>| -> rquickjs::Result<u32> {
        guarded(&host, &ctx, || Ok(host.borrow().scripts.len() as u32))
    })
}

/// A native reading one field of script `i`.
fn script_field<'js, R>(
    ctx: &Ctx<'js>,
    host: &Shared,
    f: impl Fn(&PageScript) -> R + 'js,
) -> rquickjs::Result<Function<'js>>
where
    R: IntoJs<'js> + Default + 'js,
{
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, i: u32| -> rquickjs::Result<R> {
            guarded(&host, &ctx, || {
                Ok(host
                    .borrow()
                    .scripts
                    .get(i as usize)
                    .map_or_else(R::default, &f))
            })
        },
    )
}

/// `runScript(i)`: evaluates script `i` as a classic global script. Its
/// exceptions reach the glue, which reports them.
fn native_run_script<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, i: u32| -> rquickjs::Result<()> {
            guarded(&host, &ctx, || {
                let (source, name) = {
                    // Not held across the eval: the script calls natives.
                    let mut guard = host.borrow_mut();
                    let Some(s) = guard.scripts.get_mut(i as usize) else {
                        return Ok(());
                    };
                    (s.source.take().unwrap_or_default(), s.name.clone())
                };
                let mut opts = EvalOptions::default();
                opts.global = true;
                opts.strict = false;
                opts.filename = Some(name);
                ctx.eval_with_options::<(), _>(source, opts)
            })
        },
    )
}

/// `prepareScript(h)`: "prepare the script element" for element `h`, which
/// an operation queued ([`Document::prepare_script`]). 0: nothing to do; 1:
/// an inline script, for `runPrepared` now; 2: an `error` event in a task; 3:
/// a fetch, asked for here, which holds the event loop's clock until
/// `run_page` gives it to the resolver.
fn native_prepare_script<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, h: Handle| -> rquickjs::Result<u32> {
            guarded(&host, &ctx, || {
                let mut guard = host.borrow_mut();
                let host = &mut *guard;
                let Some(doc) = host.doc.as_mut() else {
                    return Ok(0);
                };
                let Ok(id) = doc.node_of(ObjectId(u64::from(h))) else {
                    return Ok(0);
                };
                let loads = &mut host.script_loads;
                Ok(match doc.prepare_script(id) {
                    Prepared::Nothing => 0,
                    Prepared::Inline(source) => {
                        loads.sources.insert(h, (source, "<script>".to_owned()));
                        1
                    }
                    Prepared::Error => 2,
                    Prepared::Fetch(url) => {
                        loads.count += 1;
                        loads.bytes = loads.bytes.saturating_add(url.len());
                        if loads.count > MAX_SCRIPT_FETCHES || loads.bytes > MAX_SCRIPT_FETCH_BYTES
                        {
                            if loads.missing.len() < MAX_KEPT_ERRORS {
                                loads.missing.push(capped(url));
                            }
                            loads.asked.push((h, None));
                        } else {
                            loads.asked.push((h, Some(url)));
                        }
                        if loads.wait.is_none()
                            && let Some(handle) = &host.event_loop
                        {
                            loads.wait = Some(handle.external_begin());
                        }
                        3
                    }
                })
            })
        },
    )
}

/// `runPrepared(h)`: runs the source `prepareScript` or a fetch left for
/// element `h`, as a classic global script, once. Its exceptions reach the
/// glue, which reports them.
fn native_run_prepared<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, h: Handle| -> rquickjs::Result<()> {
            guarded(&host, &ctx, || {
                // Not held across the eval: the script calls natives.
                let (source, name) = {
                    let mut guard = host.borrow_mut();
                    let host = &mut *guard;
                    let Some(found) = host.script_loads.sources.remove(&h) else {
                        return Ok(());
                    };
                    if host.script_loads.depth >= MAX_SCRIPT_NESTING {
                        // As `limit` does: the call ends with this error,
                        // whatever the scripts below catch.
                        host.fault.get_or_insert(SandboxError::StackOverflow.into());
                        host.stop.store(true, Ordering::Release);
                        return Err(Exception::throw_internal(
                            &ctx,
                            "scripts a script inserted nest too deeply",
                        ));
                    }
                    host.script_loads.depth += 1;
                    found
                };
                let mut opts = EvalOptions::default();
                opts.global = true;
                opts.strict = false;
                opts.filename = Some(name);
                let ran = ctx.eval_with_options::<(), _>(source, opts);
                let mut guard = host.borrow_mut();
                guard.script_loads.depth = guard.script_loads.depth.saturating_sub(1);
                ran
            })
        },
    )
}

/// `nextFetched()`: the next fetched script, in the order asked: its
/// element's handle, negated if the fetch failed; 0 for none.
fn native_next_fetched<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(ctx.clone(), move |ctx: Ctx<'js>| -> rquickjs::Result<f64> {
        guarded(&host, &ctx, || {
            Ok(match host.borrow_mut().script_loads.fetched.pop_front() {
                Some((h, true)) => f64::from(h),
                Some((h, false)) => -f64::from(h),
                None => 0.0,
            })
        })
    })
}

/// `runSource(source)`: a timer's string handler, as a classic global script
/// (HTML §8.7). Its exceptions reach the glue, which reports them. rquickjs
/// takes a C string, so a NUL can't reach the parser and is the only other
/// error here.
fn native_run_source<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, source: String| -> rquickjs::Result<()> {
            guarded(&host, &ctx, || {
                let mut opts = EvalOptions::default();
                opts.global = true;
                opts.strict = false;
                opts.filename = Some("<timer>".to_owned());
                match ctx.eval_with_options::<Value<'js>, _>(source, opts) {
                    Ok(_) => Ok(()),
                    Err(rquickjs::Error::Exception) => Err(rquickjs::Error::Exception),
                    Err(_) => Err(Exception::throw_syntax(&ctx, "invalid character in script")),
                }
            })
        },
    )
}

/// `limit(thrown)`: whether an uncaught exception is really a limit, which
/// only the engine knows (`surimono_core::limit_error`). One that is ends the
/// call with that error, however the script goes on; see [`Host::stop`].
fn native_limit<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, thrown: Value<'js>| -> rquickjs::Result<bool> {
            guarded(&host, &ctx, || {
                let Some(error) = limit_error(&thrown) else {
                    return Ok(false);
                };
                let mut host = host.borrow_mut();
                host.fault.get_or_insert(error.into());
                host.stop.store(true, Ordering::Release);
                Ok(true)
            })
        },
    )
}

/// `report(message)`: an exception nothing handled.
fn native_report<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, message: String| -> rquickjs::Result<()> {
            guarded(&host, &ctx, || {
                let mut host = host.borrow_mut();
                host.uncaught += 1;
                if host.errors.len() < MAX_KEPT_ERRORS {
                    host.errors.push(capped(message));
                }
                Ok(())
            })
        },
    )
}

/// `throwDom(name, message)`: throws a DOMException.
fn native_throw_dom<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, name: String, message: String| -> rquickjs::Result<()> {
            guarded(&host, &ctx, || {
                Err(Exception::throw_dom(&ctx, &name, &message))
            })
        },
    )
}

/// Runs a native's body under [`Deadline::guard`]: a panic in it ends the
/// call with `SandboxError::HostPanic`, however the script handles the
/// exception it sees. Every native's body goes through this; a test in this
/// file checks.
fn guarded<'js, R>(
    host: &Shared,
    ctx: &Ctx<'js>,
    f: impl FnOnce() -> rquickjs::Result<R>,
) -> rquickjs::Result<R> {
    let (deadline, stopping) = host.try_borrow().map_or((None, false), |h| {
        (h.deadline.clone(), h.stop.load(Ordering::Acquire))
    });
    let Some(deadline) = deadline else {
        return Err(Exception::throw_internal(ctx, "no call is running"));
    };
    if stopping {
        return Err(Exception::throw_internal(ctx, "the call has ended"));
    }
    deadline.guard(ctx, || {
        #[cfg(test)]
        tests::panic_if_due();
        f()
    })
}

/// `call(op, generation, handle, ...args)`: every generated member. The
/// glue has converted the arguments to primitives; `dispatch` reads them.
fn native_call<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              this: This<Value<'js>>,
              op: u32,
              g: f64,
              h: Handle,
              rest: Rest<Value<'js>>|
              -> rquickjs::Result<Value<'js>> {
            guarded(&host, &ctx, || {
                let (ret, queued) = call_op(&ctx, &host, op, g, h, rest)?;
                let value = ret.into_js(&ctx)?;
                // The post-connection steps of the scripts it queued, in
                // order, now that it is done: `this` is the glue's record.
                if !queued.is_empty()
                    && let Some(hooks) = this.0.as_object()
                {
                    let prepare: Function<'js> = hooks.get("p")?;
                    for element in queued {
                        prepare.call::<_, ()>((element,))?;
                    }
                }
                Ok(value)
            })
        },
    )
}

/// One operation of [`native_call`], with the host borrowed: what it returned,
/// and the script elements it queued for their post-connection steps.
fn call_op<'js>(
    ctx: &Ctx<'js>,
    host: &Shared,
    op: u32,
    g: f64,
    h: Handle,
    rest: Rest<Value<'js>>,
) -> rquickjs::Result<(crate::bindings::Ret, Vec<Handle>)> {
    let mut guard = host.borrow_mut();
    let host = &mut *guard;
    if host.deadline.as_ref().is_none_or(Deadline::passed) {
        // No fault is recorded: the sandbox's interrupt handler
        // sees the same passed deadline at the script's next
        // check and stops it, so the call fails with TimeLimit
        // even if the script catches this.
        return Err(Exception::throw_internal(
            ctx,
            "DOM call: the time limit has passed",
        ));
    }
    let Some(doc) = host.doc.as_mut() else {
        return Err(fault(ctx, &mut host.fault, DomError::NoDocument));
    };
    // Generations are below 2^53, so the f64 round trip is exact.
    if let Err(e) = doc.check(g as u64, h) {
        return Err(fault(ctx, &mut host.fault, e));
    }
    doc.deadline.clone_from(&host.deadline);
    doc.layout = host.layout;
    if host.layout == LayoutPolicy::Real {
        doc.layout_setup.clone_from(&host.layout_setup);
    }
    doc.enter(h);
    doc.clear_scripts_to_prepare();
    let outcome = dispatch(doc, op, ObjectId(u64::from(h)), &Args::new(rest.0));
    let queued = doc.take_scripts_to_prepare();
    if let Some(e) = doc.fault.take() {
        return Err(fault(ctx, &mut host.fault, e));
    }
    let ret = outcome.map_err(|e| throw(ctx, e))?;
    let mut handles = Vec::with_capacity(queued.len());
    for id in queued {
        match doc.handle_for(id) {
            Ok(h) => handles.push(h),
            Err(e) => return Err(fault(ctx, &mut host.fault, e)),
        }
    }
    Ok((ret, handles))
}

/// Turns a binding error into a JavaScript exception.
fn throw(ctx: &Ctx<'_>, e: BindingError) -> rquickjs::Error {
    match e {
        BindingError::TypeError(m) => Exception::throw_type(ctx, &m),
        BindingError::DomException { name, message } => Exception::throw_dom(ctx, name, &message),
        BindingError::NotImplemented(what) => {
            Exception::throw_type(ctx, &format!("{what} is not implemented"))
        }
    }
}

/// Records `e` as the call's fault (the first one wins) and throws.
fn fault(ctx: &Ctx<'_>, slot: &mut Option<DomError>, e: DomError) -> rquickjs::Error {
    let message = e.script_message();
    slot.get_or_insert(e);
    Exception::throw_internal(ctx, &message)
}

/// `stale()`: a wrapper of another document was passed as an argument.
fn native_stale<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(ctx.clone(), move |ctx: Ctx<'js>| -> rquickjs::Result<()> {
        guarded(&host, &ctx, || {
            Err(fault(
                &ctx,
                &mut host.borrow_mut().fault,
                DomError::StaleDocument,
            ))
        })
    })
}

/// `current()`: the loaded document's generation, or 0 (never a
/// generation) for none. `currentRoot()`: the document node's handle.
/// Two calls returning numbers, not one returning an array: rquickjs fills
/// arrays with ordinary sets, which a script's `Array.prototype` index
/// setters would intercept.
fn native_current<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(ctx.clone(), move |ctx: Ctx<'js>| -> rquickjs::Result<f64> {
        guarded(&host, &ctx, || {
            // Generations are below 2^53, so the f64 is exact.
            Ok(host
                .borrow()
                .doc
                .as_ref()
                .map_or(0.0, |d| d.generation() as f64))
        })
    })
}

fn native_current_root<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>| -> rquickjs::Result<Handle> {
            guarded(&host, &ctx, || {
                let mut guard = host.borrow_mut();
                let host = &mut *guard;
                match host.doc.as_mut() {
                    None => Ok(0),
                    Some(doc) => {
                        let root = doc.root();
                        doc.handle_for(root)
                            .map_err(|e| fault(&ctx, &mut host.fault, e))
                    }
                }
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    thread_local! {
        /// How many guarded natives may still run before one panics.
        static PANIC_AFTER: Cell<Option<u32>> = const { Cell::new(None) };
        /// How many guarded natives have started on this thread.
        static STARTED: Cell<u32> = const { Cell::new(0) };
    }

    pub(super) fn panic_if_due() {
        STARTED.set(STARTED.get() + 1);
        match PANIC_AFTER.get() {
            Some(0) => {
                PANIC_AFTER.set(None);
                panic!("native exploded");
            }
            Some(n) => PANIC_AFTER.set(Some(n - 1)),
            None => {}
        }
    }

    /// Ending its own wait never gives up on another's (a font or an image
    /// load, once those open waits): `fetch_inserted_scripts` abandons waits
    /// only in a round where it held none of its own, even with nothing asked.
    #[test]
    fn delivering_fetches_leaves_other_waits_open() {
        let mut rt = DomRuntime::new(Limits::default()).unwrap();
        rt.load(Document::parse("<!doctype html><title>x</title>").unwrap());
        let _: () = rt.eval("0").unwrap();
        let handle = rt.sandbox().event_loop().unwrap();
        let other = handle.external_begin();
        rt.begin_call();
        rt.host.borrow_mut().script_loads.wait = Some(handle.external_begin());
        assert_eq!(handle.external_pending(), 2);
        rt.fetch_inserted_scripts(&mut |_| None);
        assert_eq!(handle.external_pending(), 1, "only its own wait ends");
        assert!(handle.external_end(other));
        // A round with no wait of its own: the loop waits on someone else's,
        // and nothing else would end it.
        let stuck = handle.external_begin();
        assert!(!rt.fetch_inserted_scripts(&mut |_| None));
        assert!(!handle.external_end(stuck), "abandoned");
    }

    /// A new native without the guard would keep rquickjs's behaviour: a
    /// `try` hides the panic and the script carries on (#102).
    #[test]
    fn every_native_is_guarded() {
        // Split so this test's own text doesn't count.
        let needle = concat!("Function", "::new(");
        let source = include_str!("runtime.rs");
        let body = &source[..source.find("#[cfg(test)]\nmod tests").unwrap()];
        let mut parts = body.split(needle);
        parts.next();
        let mut seen = 0;
        for part in parts {
            seen += 1;
            let head: String = part.chars().take(420).collect();
            assert!(head.contains("guarded(&host, &ctx"), "not guarded: {head}");
        }
        assert!(seen >= 9, "{seen}");
    }

    /// The same through a page: scripts, events and timers.
    #[test]
    fn a_panic_in_a_page_ends_it() {
        const PAGE: &str = "<!doctype html><script>
            addEventListener('load', () => { document.title = 'x'; });
            setTimeout(() => { try { document.body.textContent = 'y'; } catch (e) {} }, 0);
            try { document.createElement('1x'); } catch (e) {}
            </script><script src='missing.js'></script>
            <script>try { throw new Error('e'); } finally {}</script>";
        let run = |rt: &mut DomRuntime| rt.run_page(&SettleOptions::default(), |_| None);
        let mut rt = DomRuntime::new(Limits::default()).unwrap();
        rt.load(Document::parse(PAGE).unwrap());
        run(&mut rt).unwrap();
        let mut panicked = 0;
        for n in 0..200 {
            rt.load(Document::parse(PAGE).unwrap());
            PANIC_AFTER.set(Some(n));
            match run(&mut rt) {
                Ok(_) => {
                    PANIC_AFTER.set(None);
                    break;
                }
                Err(DomError::Sandbox(SandboxError::HostPanic { .. })) => panicked += 1,
                Err(e) => panic!("after {n} calls: {e:?}"),
            }
            assert!(rt.take().is_none());
        }
        assert!(panicked > 20, "{panicked}");
    }

    const WALK: &str = "try {
        const d = document.getElementById('a');
        d.textContent = 'x'; d.setAttribute('t', 'u');
        d.appendChild(document.createElement('b'));
        try { d.appendChild(0); } catch (e) {}
        document.body.parentNode.firstChild;
      } catch (e) {}
      'carried on'";

    /// A panic in any of the natives this walk reaches fails the call
    /// though the script catches it, and leaves nothing to carry on from.
    #[test]
    fn a_panic_in_a_native_ends_the_call_and_discards_the_document() {
        let page = "<!doctype html><body><div id=a>y</div>";
        let mut rt = DomRuntime::new(Limits::default()).unwrap();
        rt.load(Document::parse(page).unwrap());
        // Installs the bindings; without a panic the walk just works.
        assert_eq!(rt.eval::<String>(WALK).unwrap(), "carried on");
        let mut panicked = 0;
        for n in 0..30 {
            rt.load(Document::parse(page).unwrap());
            let _: () = rt.eval("globalThis.kept = 1").unwrap();
            PANIC_AFTER.set(Some(n));
            match rt.eval::<String>(WALK) {
                Ok(_) => {
                    // The walk has fewer native calls than `n`.
                    PANIC_AFTER.set(None);
                    assert!(panicked > 0);
                    break;
                }
                Err(DomError::Sandbox(SandboxError::HostPanic { message })) => {
                    assert_eq!(message, "native exploded");
                    panicked += 1;
                }
                Err(e) => panic!("after {n} calls: {e:?}"),
            }
            assert!(rt.take().is_none(), "the document is discarded");
            rt.load(Document::parse(page).unwrap());
            let kept: String = rt.eval("typeof kept").unwrap();
            assert_eq!(kept, "undefined", "the script's state is discarded");
            assert_eq!(rt.eval::<String>("document.body.tagName").unwrap(), "BODY");
        }
        assert!(panicked > 5, "{panicked}");
    }

    /// [`Deadline::passed`] doesn't see a panic, so what keeps a `catch` off
    /// the half-changed document is the engine: a panic raises the stop flag,
    /// and the only way to a native is through a glue function, whose entry
    /// polls that flag.
    #[test]
    fn no_native_runs_after_a_panic() {
        let mut rt = DomRuntime::new(Limits::default()).unwrap();
        rt.load(Document::parse("<!doctype html><body>").unwrap());
        assert_eq!(rt.eval::<String>("document.body.tagName").unwrap(), "BODY");
        STARTED.set(0);
        PANIC_AFTER.set(Some(1));
        let script = "for (let k = 0; k < 20; k++) {
            try { document.body.setAttribute('a', 'b'); } catch (e) {}
          }";
        let result = rt.eval::<()>(script);
        assert!(
            matches!(
                result,
                Err(DomError::Sandbox(SandboxError::HostPanic { .. }))
            ),
            "{result:?}"
        );
        assert_eq!(STARTED.get(), 2, "one call, then the one that panicked");
    }
}
