//! SPIKE (P2-RT-12): module scripts on QuickJS-ng. Not for merging as it is;
//! see `crates/core/MODULES.md` on branch `p2-rt-12-design`.
//!
//! The engine asks the host for two things while it resolves a module graph
//! (statically at `JS_EvalFunction`, dynamically in the job `import()`
//! queues): the normalized name of a specifier against its referrer, and a
//! compiled module for a normalized name it hasn't seen in this context. Both
//! callbacks are synchronous: they return a string or a `JSModuleDef *`, with
//! no way to answer later. So every fetch happens on the sandbox's thread,
//! inside the call that needs it, one at a time, in the engine's order (a
//! depth-first walk of each module's requests in source order).
//!
//! Everything here is `#[doc(hidden)]` and named `spike_*` where it is public:
//! the public API is the design's question, not this file's.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use rquickjs::{Ctx, Promise, Value, qjs};

/// What a module graph may cost, apart from the sandbox's own limits (the
/// compiled code is on the JS heap, under the memory limit).
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct SpikeModuleLimits {
    /// Modules a context may load (inline entries don't count).
    pub max_modules: usize,
    /// Source bytes over all loaded modules.
    pub max_bytes: usize,
}

impl Default for SpikeModuleLimits {
    fn default() -> Self {
        Self {
            max_modules: 2_000,
            max_bytes: 64 << 20,
        }
    }
}

/// Resolves `specifier` against the referrer `base` (a normalized name, or a
/// classic script's file name): `Ok(url)` or `Err(message)`, a TypeError.
pub type SpikeResolve = Box<dyn FnMut(&str, &str) -> Result<String, String>>;
/// Gives a module's source for a normalized URL, or `None` (a TypeError).
pub type SpikeFetch = Box<dyn FnMut(&str) -> Option<String>>;

/// The host side of module loading for one sandbox.
#[doc(hidden)]
pub struct SpikeModuleHost {
    pub resolve: SpikeResolve,
    pub fetch: SpikeFetch,
    pub limits: SpikeModuleLimits,
    /// Normalized names in load order, for tests and the report.
    pub loaded: Vec<String>,
    pub bytes: usize,
    /// Names whose fetch failed, in order.
    pub failed: Vec<String>,
}

impl SpikeModuleHost {
    pub fn new(resolve: SpikeResolve, fetch: SpikeFetch) -> Self {
        Self {
            resolve,
            fetch,
            limits: SpikeModuleLimits::default(),
            loaded: Vec::new(),
            bytes: 0,
            failed: Vec::new(),
        }
    }

    /// Clears per-render counters (a new context starts with no modules).
    pub fn reset_counters(&mut self) {
        self.loaded.clear();
        self.bytes = 0;
        self.failed.clear();
    }
}

/// Shared between the sandbox, which keeps it alive past its runtime, and
/// the runtime's callbacks, which get a pointer to it as their opaque.
pub(crate) type SharedHost = Rc<RefCell<SpikeModuleHost>>;

/// Installs the callbacks on `ctx`'s runtime. The runtime must not outlive
/// `host`: the sandbox drops its runtime first (field order).
pub(crate) fn install(ctx: &Ctx<'_>, host: &SharedHost) {
    let raw = ctx.as_raw().as_ptr();
    // SAFETY: `raw` is a live context whose runtime we hold locked; the opaque
    // points into an `Rc` allocation that the sandbox keeps until after the
    // runtime is freed, so the callbacks never see it dangle.
    unsafe {
        let rt = qjs::JS_GetRuntime(raw);
        qjs::JS_SetModuleLoaderFunc2(
            rt,
            Some(normalize_raw),
            Some(load_raw),
            Some(check_attributes_raw),
            Rc::as_ptr(host) as *mut c_void,
        );
    }
}

/// Throws a TypeError with `message` on `ctx`.
///
/// # Safety
/// `ctx` must be a live context on this thread.
unsafe fn throw_type(ctx: *mut qjs::JSContext, message: &str) {
    let msg = CString::new(message.replace('\0', "?")).unwrap_or_default();
    // SAFETY: the format string is a literal with one %s, given one C string.
    unsafe {
        qjs::JS_ThrowTypeError(ctx, c"%s".as_ptr(), msg.as_ptr());
    }
}

/// The host from the opaque, if it isn't in use already (no callback can
/// reenter another today; refuse rather than alias if one ever does).
///
/// # Safety
/// `opaque` must be what [`install`] passed.
unsafe fn host<'a>(opaque: *mut c_void) -> &'a RefCell<SpikeModuleHost> {
    // SAFETY: see `install`.
    unsafe { &*(opaque as *const RefCell<SpikeModuleHost>) }
}

unsafe extern "C" fn normalize_raw(
    ctx: *mut qjs::JSContext,
    base: *const c_char,
    name: *const c_char,
    opaque: *mut c_void,
) -> *mut c_char {
    // SAFETY: the engine passes NUL-terminated strings that live for the call.
    let (base, name) = unsafe {
        (
            CStr::from_ptr(base).to_string_lossy().into_owned(),
            CStr::from_ptr(name).to_string_lossy().into_owned(),
        )
    };
    // SAFETY: the opaque is ours (install).
    let cell = unsafe { host(opaque) };
    let result = catch_unwind(AssertUnwindSafe(|| match cell.try_borrow_mut() {
        Ok(mut h) => (h.resolve)(&base, &name),
        Err(_) => Err("module host busy".to_owned()),
    }));
    match result {
        Ok(Ok(url)) => {
            // SAFETY: js_strndup copies `url.len()` bytes into a buffer from the
            // context's allocator, which the engine frees; null on OOM with an
            // exception pending.
            unsafe { qjs::js_strndup(ctx, url.as_ptr().cast(), url.len() as _) }
        }
        Ok(Err(message)) => {
            // SAFETY: live context.
            unsafe { throw_type(ctx, &message) };
            std::ptr::null_mut()
        }
        Err(_) => {
            // SAFETY: live context.
            unsafe { throw_type(ctx, "module resolver panicked") };
            std::ptr::null_mut()
        }
    }
}

unsafe extern "C" fn check_attributes_raw(
    ctx: *mut qjs::JSContext,
    _opaque: *mut c_void,
    attributes: qjs::JSValue,
) -> i32 {
    // Only `type: "json"` would be known to HTML today (and `css`); the
    // spike supports neither, so any `type` is refused here, before a fetch.
    // SAFETY: `attributes` is a live value for the call; we only borrow it.
    unsafe {
        if !qjs::JS_IsObject(attributes) {
            return 0;
        }
        let ty = qjs::JS_GetPropertyStr(ctx, attributes, c"type".as_ptr());
        let undefined = qjs::JS_IsUndefined(ty);
        qjs::JS_FreeValue(ctx, ty);
        if undefined {
            return 0;
        }
        throw_type(
            ctx,
            "import attributes with a type (JSON or CSS modules) are not supported",
        );
        -1
    }
}

unsafe extern "C" fn load_raw(
    ctx: *mut qjs::JSContext,
    name: *const c_char,
    opaque: *mut c_void,
    _attributes: qjs::JSValue,
) -> *mut qjs::JSModuleDef {
    // SAFETY: NUL-terminated for the call.
    let url = unsafe { CStr::from_ptr(name).to_string_lossy().into_owned() };
    // SAFETY: the opaque is ours (install).
    let cell = unsafe { host(opaque) };
    let fetched = catch_unwind(AssertUnwindSafe(|| {
        let mut h = cell
            .try_borrow_mut()
            .map_err(|_| "module host busy".to_owned())?;
        if h.loaded.len() >= h.limits.max_modules {
            return Err(format!(
                "module limit reached ({} modules): '{url}' not loaded",
                h.limits.max_modules
            ));
        }
        let Some(source) = (h.fetch)(&url) else {
            h.failed.push(url.clone());
            return Err(format!("could not load module '{url}'"));
        };
        if h.bytes.saturating_add(source.len()) > h.limits.max_bytes {
            return Err(format!(
                "module bytes limit reached ({} bytes): '{url}' not loaded",
                h.limits.max_bytes
            ));
        }
        h.bytes += source.len();
        h.loaded.push(url.clone());
        Ok(source)
    }));
    let source = match fetched {
        Ok(Ok(source)) => source,
        Ok(Err(message)) => {
            // SAFETY: live context.
            unsafe { throw_type(ctx, &message) };
            return std::ptr::null_mut();
        }
        Err(_) => {
            // SAFETY: live context.
            unsafe { throw_type(ctx, "module loader panicked") };
            return std::ptr::null_mut();
        }
    };
    // SAFETY: live context; see `compile_module`.
    unsafe { compile_module(ctx, &source, &url, &url) }
}

/// Compiles `source` as module `name` (registering it in the context's
/// module list), sets `import.meta.url` to `meta_url`, and returns its
/// definition, or null with an exception pending.
///
/// # Safety
/// `ctx` must be a live context on this thread.
unsafe fn compile_module(
    ctx: *mut qjs::JSContext,
    source: &str,
    name: &str,
    meta_url: &str,
) -> *mut qjs::JSModuleDef {
    // JS_Eval requires input[len] == 0.
    let mut input = Vec::with_capacity(source.len() + 1);
    input.extend_from_slice(source.as_bytes());
    input.push(0);
    let Ok(cname) = CString::new(name.replace('\0', "?")) else {
        return std::ptr::null_mut();
    };
    let flags = (qjs::JS_EVAL_TYPE_MODULE | qjs::JS_EVAL_FLAG_COMPILE_ONLY) as i32;
    // SAFETY: `input` is NUL-terminated and outlives the call; the length
    // excludes the terminator.
    unsafe {
        let func = qjs::JS_Eval(
            ctx,
            input.as_ptr().cast(),
            source.len() as _,
            cname.as_ptr(),
            flags,
        );
        if qjs::JS_IsException(func) {
            return std::ptr::null_mut();
        }
        // A compile-only module is a JS_TAG_MODULE value whose pointer is the
        // definition, which stays in the context's module list after the value
        // is freed (as quickjs-libc's own loader does).
        let m = qjs::JS_VALUE_GET_PTR(func) as *mut qjs::JSModuleDef;
        let meta = qjs::JS_GetImportMeta(ctx, m);
        if qjs::JS_IsException(meta) {
            qjs::JS_FreeValue(ctx, func);
            return std::ptr::null_mut();
        }
        let url = qjs::JS_NewStringLen(ctx, meta_url.as_ptr().cast(), meta_url.len() as _);
        let ok = qjs::JS_DefinePropertyValueStr(
            ctx,
            meta,
            c"url".as_ptr(),
            url,
            qjs::JS_PROP_C_W_E as i32,
        );
        qjs::JS_FreeValue(ctx, meta);
        if ok < 0 {
            qjs::JS_FreeValue(ctx, func);
            return std::ptr::null_mut();
        }
        qjs::JS_FreeValue(ctx, func);
        m
    }
}

/// Runs an entry module (an inline `<script type="module">` or the first
/// module of a `src` one): compiles it as `name` with `import.meta.url` set to
/// `meta_url`, resolves, links and evaluates its graph. Returns the
/// evaluation promise (QuickJS-ng returns one for every module, settled at
/// once without top-level await). Fetch, parse and link errors throw here.
#[doc(hidden)]
pub fn spike_eval_module<'js>(
    ctx: &Ctx<'js>,
    name: &str,
    meta_url: &str,
    source: &str,
) -> rquickjs::Result<Promise<'js>> {
    let raw = ctx.as_raw().as_ptr();
    // SAFETY: `raw` is live while we hold `ctx`.
    let m = unsafe { compile_module(raw, source, name, meta_url) };
    if m.is_null() {
        return Err(rquickjs::Error::Exception);
    }
    // SAFETY: JS_EvalFunction takes ownership of a module value; we make one
    // reference for it (the definition itself is owned by the context's list).
    let ret = unsafe {
        let v = qjs::JS_MKPTR(qjs::JS_TAG_MODULE, m.cast());
        let v = qjs::JS_DupValue(raw, v);
        qjs::JS_EvalFunction(raw, v)
    };
    // SAFETY: inspecting a value we own.
    if unsafe { qjs::JS_IsException(ret) } {
        return Err(rquickjs::Error::Exception);
    }
    // SAFETY: an owned value of this context.
    let v = unsafe { Value::from_raw(ctx.clone(), ret) };
    v.into_promise().ok_or(rquickjs::Error::FromJs {
        from: "value",
        to: "promise",
        message: Some("module evaluation did not give a promise".into()),
    })
}

/// Compiles a module into the context's module list without resolving or
/// running it, as a bytecode cache or a prefetching loader would.
#[doc(hidden)]
pub fn spike_declare_module(ctx: &Ctx<'_>, name: &str, source: &str) -> rquickjs::Result<()> {
    // SAFETY: live context while we hold `ctx`.
    let m = unsafe { compile_module(ctx.as_raw().as_ptr(), source, name, name) };
    if m.is_null() {
        Err(rquickjs::Error::Exception)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "modules_tests.rs"]
mod tests;
