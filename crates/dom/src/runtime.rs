//! A sandbox with the DOM bindings installed, and the native half of them.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use surimono_core::rquickjs::context::EvalOptions;
use surimono_core::rquickjs::function::Rest;
use surimono_core::rquickjs::{self, Ctx, Exception, FromJs, Function, IntoJs, Object, Value};
use surimono_core::{
    Deadline, Limits, Sandbox, SandboxError, SandboxOptions, SettleOptions, SettleReport,
    limit_error,
};
use surimono_webidl::runtime::{BindingError, ObjectId};

use crate::bindings::Args;
use crate::dispatch::dispatch;
use crate::document::Handle;
use crate::{Document, DomError, LayoutPolicy};

/// The JavaScript half of the bindings; see `glue_head.js`.
const GLUE: &str = concat!(
    include_str!("glue_head.js"),
    include_str!("glue_events.js"),
    include_str!("generated/bindings.js"),
    include_str!("glue_layout.js"),
    include_str!("glue_tail.js"),
);

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
    /// The sandbox's cancel flag, raised by `limit`: a limit reported by the
    /// glue must stop the script like one the engine stops it for. Natives
    /// refuse to work while it is up, because the sandbox's watchdog sees it
    /// a few milliseconds later. Only this file raises it, and the fault
    /// says why, so a caller sees that limit and never `Cancelled`.
    stop: Arc<AtomicBool>,
    /// The current call's deadline; host functions refuse to work after it.
    deadline: Option<Deadline>,
    /// The page's scripts, for [`DomRuntime::run_page`].
    scripts: Vec<PageScript>,
    /// Exceptions reported since the last report, and the first few.
    uncaught: u64,
    errors: Vec<String>,
    /// Promises [`DomRuntime::checkpoint`] found no script handled, since the
    /// last report, and the first few reasons.
    unhandled_rejections: u64,
    rejections: Vec<String>,
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
    /// fires for them, so no listener can cancel them. A limit is never here:
    /// it fails the call.
    pub unhandled_rejections: u64,
    /// The first few reasons, as messages, each cut to a few KiB.
    pub rejections: Vec<String>,
    /// `src` values of external scripts the resolver had no source for.
    pub missing_scripts: Vec<String>,
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

    /// Chooses what scripts' layout queries do, from the next call on. The
    /// default is [`LayoutPolicy::Zeros`].
    pub fn set_layout_policy(&mut self, policy: LayoutPolicy) {
        self.host.borrow_mut().layout = policy;
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
        self.begin_call();
        let Self {
            sandbox,
            host,
            installed,
        } = self;
        let result = sandbox.run(|ctx| {
            if !*installed {
                install(ctx, host)?;
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
        self.begin_call();
        let result = self.sandbox.run_until_settled(options);
        let mut report = self.finish_call(result)?;
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
    /// all under one time limit. `<iframe>`s load as documents without a
    /// browsing context of their own: see "Frames" in DESIGN.md. The order, all at virtual time 0 and before
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
    /// leaves a rejected promise; if no script handles it by the end of the
    /// task it is counted in [`PageReport::unhandled_rejections`], and fires
    /// nothing (`unhandledrejection` isn't implemented).
    ///
    /// A limit is different: an uncaught stack overflow fails the call with
    /// the same error everywhere, promises included, with no `error` event.
    /// Handling the promise in time is catching it, a script's own matter.
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
            host.uncaught = 0;
            host.errors.clear();
            host.unhandled_rejections = 0;
            host.rejections.clear();
        }
        // The parser's frames load before any script, and their `load` events
        // fire before the window's, as they must: a page's `load` listener
        // reads them. A script sees a frame as loaded from the start.
        self.load_frames(&mut resolve)?;
        self.reset()?;
        self.begin_call();
        let Self {
            sandbox,
            host,
            installed,
        } = self;
        let started = sandbox.run(|ctx| {
            let start = install(ctx, host)?;
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
        for _ in 0..MAX_FRAME_ROUNDS {
            if !self.load_frames(&mut resolve)? {
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
        })
    }

    /// Starts the loads of `<iframe>`s nothing has loaded yet (HTML §4.8.5):
    /// `srcdoc`, the source of `src` from the page's resolver, or their
    /// `about:blank`. Whether any was started. This runs when the page has
    /// settled, so a frame's `load` comes after every timer that was ready,
    /// at a point that doesn't depend on how long a resolver took.
    fn load_frames(
        &mut self,
        resolve: &mut impl FnMut(&str) -> Option<String>,
    ) -> Result<bool, DomError> {
        let deadline = self.sandbox.deadline();
        let mut guard = self.host.borrow_mut();
        let Some(doc) = guard.doc.as_mut() else {
            return Ok(false);
        };
        doc.deadline = Some(deadline);
        let frames = doc.frames_to_load();
        let any = !frames.is_empty();
        for (element, srcdoc, src) in frames {
            match (srcdoc, src) {
                (Some(text), _) => doc.load_frame(element, Some(&text), "about:srcdoc")?,
                (None, Some(src)) => {
                    // The fragment is the document's to read, not the source's.
                    let plain = src.split('#').next().unwrap_or(&src);
                    let url = doc.frame_url(&src);
                    let text = resolve(plain);
                    doc.load_frame(element, text.as_deref(), &url)?;
                }
                (None, None) => doc.load_frame(element, None, "about:blank")?,
            }
        }
        doc.deadline = None;
        Ok(any)
    }

    fn begin_call(&mut self) {
        let mut host = self.host.borrow_mut();
        host.fault = None;
        host.deadline = Some(self.sandbox.deadline());
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
            (None, r) => r.map_err(DomError::from),
        }
    }
}

/// Installs the bindings in `ctx`: builds the native functions and hands
/// them to the glue, which keeps them in a closure. Returns the glue's
/// function that queues a page's lifecycle.
fn install<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let n = Object::new(ctx.clone())?;
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
    let glue: Function<'js> = ctx.eval(GLUE)?;
    glue.call((n,))
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
              op: u32,
              g: f64,
              h: Handle,
              rest: Rest<Value<'js>>|
              -> rquickjs::Result<Value<'js>> {
            guarded(&host, &ctx, || {
                let mut guard = host.borrow_mut();
                let host = &mut *guard;
                if host.deadline.as_ref().is_none_or(Deadline::passed) {
                    // No fault is recorded: the sandbox's interrupt handler
                    // sees the same passed deadline at the script's next
                    // check and stops it, so the call fails with TimeLimit
                    // even if the script catches this.
                    return Err(Exception::throw_internal(
                        &ctx,
                        "DOM call: the time limit has passed",
                    ));
                }
                let Some(doc) = host.doc.as_mut() else {
                    return Err(fault(&ctx, &mut host.fault, DomError::NoDocument));
                };
                // Generations are below 2^53, so the f64 round trip is exact.
                if let Err(e) = doc.check(g as u64, h) {
                    return Err(fault(&ctx, &mut host.fault, e));
                }
                doc.deadline.clone_from(&host.deadline);
                doc.layout = host.layout;
                doc.enter(h);
                let outcome = dispatch(doc, op, ObjectId(u64::from(h)), &Args::new(rest.0));
                if let Some(e) = doc.fault.take() {
                    return Err(fault(&ctx, &mut host.fault, e));
                }
                match outcome {
                    Ok(ret) => ret.into_js(&ctx),
                    Err(e) => Err(throw(&ctx, e)),
                }
            })
        },
    )
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
