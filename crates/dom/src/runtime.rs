//! A sandbox with the DOM bindings installed, and the native half of them.

use std::cell::RefCell;
use std::rc::Rc;

use surimono_core::rquickjs::context::EvalOptions;
use surimono_core::rquickjs::{self, Ctx, Exception, FromJs, Function, IntoJs, Object, Value};
use surimono_core::{Deadline, Limits, Sandbox, SandboxOptions};

use crate::document::Handle;
use crate::{Document, DomError};

/// The JavaScript half of the bindings; see `glue.js`.
const GLUE: &str = include_str!("glue.js");

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

/// A [`Sandbox`] with `document` and a minimal `Node` / `Element` /
/// `Document` binding, resolving every wrapper through the loaded
/// [`Document`] on each call.
///
/// Internal phase-2 scaffolding (P1-DOM-02 spike); not part of the facade.
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
/// them to `glue.js`, which keeps them in a closure.
fn install<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<()> {
    let n = Object::new(ctx.clone())?;
    n.set("current", native_current(ctx, host)?)?;
    n.set(
        "nodeType",
        node_fn(ctx, host, "nodeType", |ctx, doc, id| {
            // `kind` is None only for layout boxes, which no tree link
            // reaches before layout; report them as plain nodes.
            let t = doc.kind(id).map_or(0, |k| k as u32);
            t.into_js(ctx)
        })?,
    )?;
    n.set(
        "nodeName",
        node_fn(ctx, host, "nodeName", |ctx, doc, id| {
            doc.node_name(id).into_js(ctx)
        })?,
    )?;
    n.set(
        "parentNode",
        node_fn(ctx, host, "parentNode", |ctx, doc, id| {
            let h = doc.parent(id).map_or(0, |p| doc.handle_for(p));
            h.into_js(ctx)
        })?,
    )?;
    n.set(
        "documentElement",
        node_fn(ctx, host, "documentElement", |ctx, doc, _| {
            let h = doc.document_element().map_or(0, |e| doc.handle_for(e));
            h.into_js(ctx)
        })?,
    )?;
    n.set(
        "body",
        node_fn(ctx, host, "body", |ctx, doc, _| {
            let h = doc.body().map_or(0, |e| doc.handle_for(e));
            h.into_js(ctx)
        })?,
    )?;
    n.set(
        "getTextContent",
        node_fn(ctx, host, "textContent", |ctx, doc, id| {
            match doc.text_content(id) {
                Some(s) => s.into_js(ctx),
                None => Ok(Value::new_null(ctx.clone())),
            }
        })?,
    )?;
    n.set(
        "setTextContent",
        node_arg_fn(ctx, host, "textContent", |ctx, doc, id, text| {
            doc.set_text_content(id, &text)?;
            Ok(Ok(Value::new_undefined(ctx.clone())))
        })?,
    )?;
    n.set(
        "getAttribute",
        node_arg_fn(ctx, host, "getAttribute", |ctx, doc, id, name| {
            Ok(match doc.attribute(id, &name) {
                Some(v) => v.into_js(ctx),
                None => Ok(Value::new_null(ctx.clone())),
            })
        })?,
    )?;
    n.set(
        "getElementById",
        node_arg_fn(ctx, host, "getElementById", |ctx, doc, _, id| {
            let h = doc.element_by_id(&id).map_or(0, |e| doc.handle_for(e));
            Ok(h.into_js(ctx))
        })?,
    )?;
    n.set(
        "createElement",
        node_arg_fn(ctx, host, "createElement", |ctx, doc, _, name| {
            Ok(match doc.create_element(&name)? {
                Some(e) => doc.handle_for(e).into_js(ctx),
                None => Err(Exception::throw_dom(
                    ctx,
                    "InvalidCharacterError",
                    "createElement: the name is not a valid element name",
                )),
            })
        })?,
    )?;
    n.set(
        "createTextNode",
        node_arg_fn(ctx, host, "createTextNode", |ctx, doc, _, data| {
            let t = doc.create_text(&data)?;
            Ok(doc.handle_for(t).into_js(ctx))
        })?,
    )?;
    n.set("appendChild", native_append_child(ctx, host)?)?;

    let glue: Function<'js> = ctx.eval(GLUE)?;
    glue.call::<_, ()>((n,))
}

/// Resolves `(generation, handle)` and runs `f`, turning a [`DomError`]
/// into a fatal fault and a thrown exception. `f`'s own `rquickjs` errors
/// (ordinary DOM exceptions) pass through.
fn resolve_and<'js, R>(
    ctx: &Ctx<'js>,
    host: &Shared,
    what: &str,
    generation: f64,
    handle: Handle,
    f: impl FnOnce(&mut Document, usize) -> Result<rquickjs::Result<R>, DomError>,
) -> rquickjs::Result<R> {
    let mut guard = host.borrow_mut();
    let host = &mut *guard;
    if host.deadline.as_ref().is_none_or(Deadline::passed) {
        // The call now fails with TimeLimit, whatever the script does.
        return Err(Exception::throw_internal(
            ctx,
            &format!("{what}: the time limit has passed"),
        ));
    }
    let outcome = match host.doc.as_mut() {
        None => Err(DomError::NoDocument),
        // Generations are below 2^53, so the f64 round trip is exact.
        Some(doc) => doc
            .resolve(generation as u64, handle)
            .and_then(|id| f(doc, id)),
    };
    match outcome {
        Ok(r) => r,
        Err(e) => {
            let message = format!("{what}: {}", e.script_message());
            host.fault.get_or_insert(e);
            Err(Exception::throw_internal(ctx, &message))
        }
    }
}

/// A native `(generation, handle) -> value`.
fn node_fn<'js>(
    ctx: &Ctx<'js>,
    host: &Shared,
    what: &'static str,
    f: impl Fn(&Ctx<'js>, &mut Document, usize) -> rquickjs::Result<Value<'js>> + 'js,
) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, g: f64, h: Handle| -> rquickjs::Result<Value<'js>> {
            resolve_and(&ctx, &host, what, g, h, |doc, id| Ok(f(&ctx, doc, id)))
        },
    )
}

/// A native `(generation, handle, string) -> value`.
fn node_arg_fn<'js>(
    ctx: &Ctx<'js>,
    host: &Shared,
    what: &'static str,
    f: impl Fn(
        &Ctx<'js>,
        &mut Document,
        usize,
        String,
    ) -> Result<rquickjs::Result<Value<'js>>, DomError>
    + 'js,
) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, g: f64, h: Handle, arg: String| -> rquickjs::Result<Value<'js>> {
            resolve_and(&ctx, &host, what, g, h, |doc, id| f(&ctx, doc, id, arg))
        },
    )
}

/// `current()`: `[generation, documentHandle]` of the loaded document, or
/// null.
fn native_current<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            let mut host = host.borrow_mut();
            match host.doc.as_mut() {
                None => Ok(Value::new_null(ctx)),
                Some(doc) => {
                    let root = doc.root();
                    let pair = vec![doc.generation() as f64, f64::from(doc.handle_for(root))];
                    pair.into_js(&ctx)
                }
            }
        },
    )
}

/// `appendChild(g, parent, g, child)`.
fn native_append_child<'js>(ctx: &Ctx<'js>, host: &Shared) -> rquickjs::Result<Function<'js>> {
    let host = host.clone();
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, g: f64, h: Handle, cg: f64, ch: Handle| -> rquickjs::Result<()> {
            resolve_and(&ctx, &host, "appendChild", g, h, |doc, parent| {
                let child = doc.resolve(cg as u64, ch)?;
                Ok(if doc.append_child(parent, child)? {
                    Ok(())
                } else {
                    Err(Exception::throw_dom(
                        &ctx,
                        "HierarchyRequestError",
                        "appendChild: the node can't be inserted here",
                    ))
                })
            })
        },
    )
}
