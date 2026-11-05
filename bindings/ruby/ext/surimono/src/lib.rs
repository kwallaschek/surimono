//! Ruby native extension for surimono, built with magnus and rb-sys.
//!
//! The Ruby side (lib/surimono.rb) checks and normalises the options and calls
//! `Surimono::Renderer._native_new` with plain values. Rendering copies the HTML
//! into Rust and releases the GVL, so other Ruby threads keep running; an
//! interrupt (`Timeout`, `Thread#raise`, `Thread#kill`, a signal) stops the
//! render, its running island included. A page's scripts go on until their time
//! limit, unobserved (see `surimono::Renderer::render_page_cancellable`).

// A library never writes to stdout or stderr; tests may.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use magnus::{
    Error, ExceptionClass, KwArgs, RArray, RHash, RModule, RString, Ruby, block::Proc, function,
    method, prelude::*,
};

#[allow(unsafe_code)]
mod ffi;

/// `Surimono::Renderer`: a `surimono::Renderer` shared by every thread that uses it.
#[magnus::wrap(class = "Surimono::Renderer", free_immediately, size)]
struct Renderer(Arc<surimono::Renderer>);

/// Options as lib/surimono.rb normalises them.
#[allow(clippy::too_many_arguments)]
fn renderer_new(
    ruby: &Ruby,
    fonts: RArray,
    memory_bytes: usize,
    time_ms: u64,
    stack_bytes: usize,
    seed: u64,
    error_policy: String,
    renderers: Option<Vec<String>>,
    bundles: Vec<(String, String, String)>,
    pool_size: Option<usize>,
    now_ms: Option<i64>,
    utc_offset_minutes: i32,
    text: String,
    max_islands: Option<usize>,
    page: (bool, String, RArray),
) -> Result<Renderer, Error> {
    let mut options = surimono::Options::new()
        .limits(surimono::Limits {
            memory_bytes,
            time: Duration::from_millis(time_ms),
            stack_bytes,
        })
        .seed(seed)
        .error_policy(match error_policy.as_str() {
            "fail" => surimono::ErrorPolicy::Fail,
            "placeholder" => surimono::ErrorPolicy::Placeholder,
            other => {
                return Err(Error::new(
                    ruby.exception_arg_error(),
                    format!("error_policy must be :fail or :placeholder, got :{other}"),
                ));
            }
        });
    for font in fonts.into_iter() {
        let data = RString::try_convert(font)?;
        options = options.font(ffi::string_bytes(data));
    }
    if let Some(names) = renderers {
        options = options.renderers(names);
    }
    for (name, source, entry) in bundles {
        options = options.bundle(name, source, entry);
    }
    if let Some(size) = pool_size {
        options = options.pool_size(size);
    }
    if let Some(ms) = now_ms {
        options = options.now_ms(ms);
    }
    options = options.utc_offset_minutes(utc_offset_minutes);
    options = options.text_mode(match text.as_str() {
        "keep" => surimono::TextMode::Keep,
        _ => surimono::TextMode::Outlines,
    });
    if let Some(max) = max_islands {
        options = options.max_islands(max);
    }
    let (keep_scripts, layout_queries, resources) = page;
    options = options.keep_scripts(keep_scripts);
    options = options.layout_queries(match layout_queries.as_str() {
        "throw" => surimono::LayoutQueries::Throw,
        _ => surimono::LayoutQueries::Zeros,
    });
    if !resources.is_empty() {
        let mut files = Vec::with_capacity(resources.len());
        for pair in resources.into_iter() {
            let (url, data) = <(String, RString)>::try_convert(pair)?;
            files.push((url, ffi::string_bytes(data)));
        }
        options = options.resource_loader(surimono::ResourceLoader::from_map(files));
    }
    // Building compiles bundles and starts runtimes: release the GVL for it too.
    let built = ffi::without_gvl(ruby, move || surimono::Renderer::new(options))?;
    built
        .map(|r| Renderer(Arc::new(r)))
        .map_err(|e| to_ruby_error(ruby, &e))
}

impl Renderer {
    fn render(ruby: &Ruby, rb_self: &Self, html: RString) -> Result<RString, Error> {
        // Copy everything the render needs while holding the GVL; the closure
        // owns plain Rust data only.
        let html: Arc<str> = Arc::from(html.to_string()?);
        loop {
            let renderer = Arc::clone(&rb_self.0);
            let doc = Arc::clone(&html);
            // Raised by the unblock function when Ruby interrupts this thread;
            // the facade stops the running island's script and skips the rest.
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&cancel);
            let result = ffi::without_gvl_cancellable(ruby, &cancel, move || {
                renderer.render_islands_cancellable(&doc, &flag)
            })?;
            match result {
                Ok(out) => return Ok(ruby.str_new(&out)),
                // Cancelled, but Ruby had no exception to raise afterwards: a
                // Thread#wakeup, or an interrupt Thread.handle_interrupt
                // defers. Render again, as Ruby's own blocking calls retry.
                Err(surimono::Error::Cancelled) => continue,
                Err(e) => return Err(to_ruby_error(ruby, &e)),
            }
        }
    }

    /// `[html, report]`, with the report as a Hash for lib/surimono.rb.
    /// `loader` is `nil`, or `[proc, allow]`.
    fn render_page(
        ruby: &Ruby,
        rb_self: &Self,
        html: RString,
        loader: Option<(Proc, Vec<String>)>,
    ) -> Result<RArray, Error> {
        let html: Arc<str> = Arc::from(html.to_string()?);
        loop {
            let renderer = Arc::clone(&rb_self.0);
            let doc = Arc::clone(&html);
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&cancel);
            // A loader's exception, or an interrupt in it: it ends the render
            // and is raised again here.
            let raised: RefCell<Option<Error>> = RefCell::new(None);
            let (stop, store) = (&cancel, &raised);
            let loader = loader.clone();
            let result = ffi::without_gvl_cancellable(ruby, &cancel, move || match loader {
                None => renderer.render_page_cancellable(&doc, &flag),
                Some((proc, allow)) => {
                    let load = |url: &str| {
                        ffi::with_gvl(|ruby| call_loader(ruby, proc, url))
                            .unwrap_or_else(|| Err("the loader panicked".to_owned()))
                    };
                    let mut guarded = |url: &str| match load(url) {
                        Ok(LoadAnswer::Bytes(bytes)) => Ok(bytes),
                        Ok(LoadAnswer::Nothing) => Err("the loader gave nothing".to_owned()),
                        Ok(LoadAnswer::Raised(e)) => {
                            store.borrow_mut().get_or_insert(e);
                            stop.store(true, Ordering::Release);
                            Err("the loader raised".to_owned())
                        }
                        Err(message) => Err(message),
                    };
                    renderer.render_page_loading(&doc, &allow, &mut guarded, &flag)
                }
            })?;
            if let Some(e) = raised.take() {
                return Err(e);
            }
            match result {
                Ok(page) => {
                    let out = ruby.ary_new();
                    out.push(ruby.str_new(&page.html))?;
                    out.push(report_hash(ruby, &page.report)?)?;
                    return Ok(out);
                }
                // As in `render`.
                Err(surimono::Error::Cancelled) => continue,
                Err(e) => return Err(to_ruby_error(ruby, &e)),
            }
        }
    }

    fn renderers(&self) -> Vec<String> {
        self.0.renderers().to_vec()
    }
}

/// What a loader proc did with a URL.
enum LoadAnswer {
    Bytes(Vec<u8>),
    Nothing,
    Raised(Error),
}

/// Calls a loader proc: a String, or nil for nothing.
fn call_loader(_ruby: &Ruby, proc: Proc, url: &str) -> Result<LoadAnswer, String> {
    Ok(match proc.call::<_, Option<RString>>((url,)) {
        Ok(Some(text)) => LoadAnswer::Bytes(ffi::string_bytes(text)),
        Ok(None) => LoadAnswer::Nothing,
        Err(e) => LoadAnswer::Raised(e),
    })
}

/// A page's report as plain Ruby values.
fn report_hash(ruby: &Ruby, report: &surimono::PageReport) -> Result<RHash, Error> {
    let hash = ruby.hash_new();
    let settled = match report.settled {
        surimono::SettleReason::Idle => "idle",
        surimono::SettleReason::VirtualBudget => "virtual_budget",
        surimono::SettleReason::TaskLimit => "task_limit",
        _ => "other",
    };
    hash.aset(ruby.to_symbol("settled"), ruby.to_symbol(settled))?;
    hash.aset(ruby.to_symbol("uncaught"), report.uncaught)?;
    hash.aset(ruby.to_symbol("errors"), report.errors.clone())?;
    hash.aset(
        ruby.to_symbol("unhandled_rejections"),
        report.unhandled_rejections,
    )?;
    hash.aset(ruby.to_symbol("rejections"), report.rejections.clone())?;
    hash.aset(ruby.to_symbol("failed_count"), report.failed_count)?;
    let failed = ruby.ary_new();
    for f in &report.failed {
        let entry = ruby.hash_new();
        let kind = match f.kind {
            surimono::ResourceKind::Script => "script",
            surimono::ResourceKind::Image => "image",
            _ => "other",
        };
        entry.aset(ruby.to_symbol("kind"), ruby.to_symbol(kind))?;
        entry.aset(ruby.to_symbol("url"), f.url.as_str())?;
        entry.aset(ruby.to_symbol("reason"), f.reason.to_string())?;
        failed.push(entry)?;
    }
    hash.aset(ruby.to_symbol("failed"), failed)?;
    Ok(hash)
}

/// The Ruby exception for a surimono error: the class from lib/surimono.rb that
/// matches the kind, with the island's details as attributes.
fn to_ruby_error(ruby: &Ruby, e: &surimono::Error) -> Error {
    use surimono::Error as E;
    let class_name = match e {
        E::MemoryLimit { .. } | E::PageMemoryLimit => "MemoryLimitError",
        E::TimeLimit { .. } | E::PageTimeLimit => "TimeLimitError",
        E::StackOverflow { .. } | E::PageStackOverflow => "StackOverflowError",
        E::PageDomLimit { .. } => "DomLimitError",
        E::JavaScript { .. } => "JavaScriptError",
        E::InvalidIsland { .. }
        | E::UnknownRenderer { .. }
        | E::Render { .. }
        | E::Html(_)
        | E::TooManyIslands { .. }
        | E::PageFailed { .. }
        | E::Cancelled => "RenderError",
        E::Font { .. } | E::Bundle { .. } | E::InvalidOptions(_) => "ConfigurationError",
        _ => "Error",
    };
    surimono_error(ruby, class_name, &e.to_string(), e.kind(), |details| {
        if let Some(island) = e.island() {
            details.aset(ruby.to_symbol("island_index"), island.index)?;
            details.aset(ruby.to_symbol("island_id"), island.id.clone())?;
            details.aset(ruby.to_symbol("renderer"), island.renderer.clone())?;
        }
        if let E::JavaScript { stack, .. } = e {
            details.aset(ruby.to_symbol("js_stack"), stack.clone())?;
        }
        Ok(())
    })
}

/// A `Surimono::<class_name>` exception with `message`, `kind` and the details
/// `add` puts in. If lib/surimono.rb's classes are missing (a `NameError`),
/// still raises something useful (a RuntimeError with the message). Anything
/// else that goes wrong while the exception is built is returned unchanged:
/// in particular an interrupt delivered meanwhile (`Thread#raise`,
/// `Thread#kill`, which isn't an exception at all) must not become a
/// RuntimeError or be swallowed.
pub(crate) fn surimono_error(
    ruby: &Ruby,
    class_name: &str,
    message: &str,
    kind: &str,
    add: impl FnOnce(RHash) -> Result<(), Error>,
) -> Error {
    let build = || -> Result<Error, Error> {
        let module: RModule = ruby.class_object().const_get("Surimono")?;
        let class: ExceptionClass = module.const_get(class_name)?;
        let details = ruby.hash_new();
        details.aset(ruby.to_symbol("kind"), kind)?;
        add(details)?;
        let exception = class.new_instance((message, KwArgs(details)))?;
        Ok(Error::from(exception))
    };
    match build() {
        Ok(e) => e,
        Err(e) if e.is_kind_of(ruby.exception_name_error()) => {
            Error::new(ruby.exception_runtime_error(), message.to_owned())
        }
        Err(e) => e,
    }
}

/// Panics inside a GVL-free section, to test that a panic becomes
/// `Surimono::InternalError` instead of ending the process. Private; used by
/// the gem's tests only.
fn internal_panic_check(ruby: &Ruby) -> Result<(), Error> {
    ffi::without_gvl::<(), _>(ruby, || {
        panic!("deliberate panic from Surimono._internal_panic_check")
    })
}

fn native_version() -> String {
    surimono::VERSION.to_string()
}

/// `Surimono.builtin_renderers`: the built-in renderers in this build, sorted.
fn builtin_renderers() -> Vec<&'static str> {
    surimono::builtin_renderers().to_vec()
}

/// `Surimono.precompiled_builtins`: how many built-ins load precompiled
/// bytecode in this process (see `surimono::precompiled_builtins`).
fn precompiled_builtins() -> usize {
    surimono::precompiled_builtins()
}

// The Cargo package is `surimono-ruby`; Ruby loads `surimono/surimono.so` and
// looks for `Init_surimono`.
#[magnus::init(name = "surimono")]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("Surimono")?;
    module.define_singleton_method("native_version", function!(native_version, 0))?;
    module.define_singleton_method("builtin_renderers", function!(builtin_renderers, 0))?;
    module.define_singleton_method("precompiled_builtins", function!(precompiled_builtins, 0))?;
    module.define_singleton_method("_internal_panic_check", function!(internal_panic_check, 0))?;
    let class = module.define_class("Renderer", ruby.class_object())?;
    class.define_singleton_method("_native_new", function!(renderer_new, 14))?;
    class.define_method("_render", method!(Renderer::render, 1))?;
    class.define_method("_render_page", method!(Renderer::render_page, 2))?;
    class.define_method("renderers", method!(Renderer::renderers, 0))?;
    Ok(())
}
