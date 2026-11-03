//! A sandbox with the DOM bindings installed, and the native half of them.

use std::cell::RefCell;
use std::rc::Rc;

use surimono_core::rquickjs::context::EvalOptions;
use surimono_core::rquickjs::function::Rest;
use surimono_core::rquickjs::{self, Ctx, Exception, FromJs, Function, Object, Value};
use surimono_core::{Deadline, Limits, Sandbox, SandboxOptions};
use surimono_webidl::runtime::{BindingError, ObjectId};

use crate::bindings::Args;
use crate::dispatch::dispatch;
use crate::document::Handle;
use crate::{Document, DomError};

/// The JavaScript half of the bindings; see `glue_head.js`.
const GLUE: &str = concat!(
    include_str!("glue_head.js"),
    include_str!("generated/bindings.js"),
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
    /// The current call's deadline; host functions refuse to work after it.
    deadline: Option<Deadline>,
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
        Ok(Self {
            sandbox: Sandbox::with_options(options)?,
            host: Shared::default(),
            installed: false,
        })
    }

    /// Makes `doc` the document scripts see, returning the one it replaces.
    /// Wrappers for the old document now fail with
    /// [`DomError::StaleDocument`].
    pub fn load(&mut self, doc: Document) -> Option<Document> {
        self.host.borrow_mut().doc.replace(doc)
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
    /// loaded document and converts its completion value to `T`.
    pub fn eval<T>(&mut self, source: &str) -> Result<T, DomError>
    where
        T: for<'js> FromJs<'js>,
    {
        {
            let mut host = self.host.borrow_mut();
            host.fault = None;
            host.deadline = Some(self.sandbox.deadline());
        }
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
        if matches!(&result, Err(e) if e.is_limit()) {
            // The sandbox rebuilt its context; the bindings went with it.
            self.installed = false;
        }
        let fault = {
            let mut host = self.host.borrow_mut();
            host.deadline = None;
            host.fault.take()
        };
        match (fault, result) {
            // A limit error outranks a fault (the script was stopped anyway).
            (_, Err(e)) if e.is_limit() => Err(e.into()),
            (Some(fault), _) => Err(fault),
            (None, r) => r.map_err(DomError::from),
        }
    }
}

/// Installs the bindings in `ctx`: builds the native functions and hands
/// them to the glue, which keeps them in a closure.
fn install<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<()> {
    let n = Object::new(ctx.clone())?;
    n.set("call", native_call(ctx, host)?)?;
    n.set("current", native_current(ctx, host)?)?;
    n.set("currentRoot", native_current_root(ctx, host)?)?;
    n.set("stale", native_stale(ctx, host)?)?;
    let glue: Function<'js> = ctx.eval(GLUE)?;
    glue.call::<_, ()>((n,))
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
            let mut guard = host.borrow_mut();
            let host = &mut *guard;
            if host.deadline.as_ref().is_none_or(Deadline::passed) {
                // No fault is recorded: the sandbox's interrupt handler sees
                // the same passed deadline at the script's next check and
                // stops it, so the call fails with TimeLimit even if the
                // script catches this.
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
            let outcome = dispatch(doc, op, ObjectId(u64::from(h)), &Args::new(rest.0));
            if let Some(e) = doc.fault.take() {
                return Err(fault(&ctx, &mut host.fault, e));
            }
            match outcome {
                Ok(ret) => ret.into_js(&ctx),
                Err(e) => Err(throw(&ctx, e)),
            }
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
        Err(fault(
            &ctx,
            &mut host.borrow_mut().fault,
            DomError::StaleDocument,
        ))
    })
}

/// `current()`: the loaded document's generation, or 0 (never a
/// generation) for none. `currentRoot()`: the document node's handle.
/// Two calls returning numbers, not one returning an array: rquickjs fills
/// arrays with ordinary sets, which a script's `Array.prototype` index
/// setters would intercept.
fn native_current<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(ctx.clone(), move || -> f64 {
        // Generations are below 2^53, so the f64 is exact.
        host.borrow()
            .doc
            .as_ref()
            .map_or(0.0, |d| d.generation() as f64)
    })
}

fn native_current_root<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>| -> rquickjs::Result<Handle> {
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
        },
    )
}
