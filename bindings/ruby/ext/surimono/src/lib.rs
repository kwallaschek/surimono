//! Ruby native extension for surimono, built with magnus and rb-sys.
//!
//! The Ruby side (lib/surimono.rb) checks and normalises the options and calls
//! `Surimono::Renderer._native_new` with plain values. Rendering copies the HTML
//! into Rust and releases the GVL, so other Ruby threads keep running.

use std::sync::Arc;
use std::time::Duration;

use magnus::{
    Error, ExceptionClass, KwArgs, RArray, RModule, RString, Ruby, function, method, prelude::*,
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
    // Building compiles bundles and starts runtimes: release the GVL for it too.
    let built = ffi::without_gvl(move || surimono::Renderer::new(options));
    built
        .map(|r| Renderer(Arc::new(r)))
        .map_err(|e| to_ruby_error(ruby, &e))
}

impl Renderer {
    fn render(ruby: &Ruby, rb_self: &Self, html: RString) -> Result<RString, Error> {
        // Copy everything the render needs while holding the GVL; the closure
        // owns plain Rust data only.
        let html = html.to_string()?;
        let renderer = Arc::clone(&rb_self.0);
        let result = ffi::without_gvl(move || renderer.render_islands(&html));
        match result {
            Ok(out) => Ok(ruby.str_new(&out)),
            Err(e) => Err(to_ruby_error(ruby, &e)),
        }
    }

    fn renderers(&self) -> Vec<String> {
        self.0.renderers().to_vec()
    }
}

/// The Ruby exception for a surimono error: the class from lib/surimono.rb that
/// matches the kind, with the island's details as attributes.
fn to_ruby_error(ruby: &Ruby, e: &surimono::Error) -> Error {
    use surimono::Error as E;
    let class_name = match e {
        E::MemoryLimit { .. } => "MemoryLimitError",
        E::TimeLimit { .. } => "TimeLimitError",
        E::StackOverflow { .. } => "StackOverflowError",
        E::JavaScript { .. } => "JavaScriptError",
        E::InvalidIsland { .. } | E::UnknownRenderer { .. } | E::Render { .. } | E::Html(_) => {
            "RenderError"
        }
        E::Font { .. } | E::Bundle { .. } | E::InvalidOptions(_) => "ConfigurationError",
        _ => "Error",
    };
    let message = e.to_string();
    let build = || -> Result<Error, Error> {
        let module: RModule = ruby.class_object().const_get("Surimono")?;
        let class: ExceptionClass = module.const_get(class_name)?;
        let details = ruby.hash_new();
        details.aset(ruby.to_symbol("kind"), e.kind())?;
        if let Some(island) = e.island() {
            details.aset(ruby.to_symbol("island_index"), island.index)?;
            details.aset(ruby.to_symbol("island_id"), island.id.clone())?;
            details.aset(ruby.to_symbol("renderer"), island.renderer.clone())?;
        }
        if let E::JavaScript { stack, .. } = e {
            details.aset(ruby.to_symbol("js_stack"), stack.clone())?;
        }
        let exception = class.new_instance((message.as_str(), KwArgs(details)))?;
        Ok(Error::from(exception))
    };
    // If lib/surimono.rb's classes are missing, still raise something useful.
    build().unwrap_or_else(|_| Error::new(ruby.exception_runtime_error(), message.clone()))
}

fn native_version() -> String {
    surimono::VERSION.to_string()
}

// The Cargo package is `surimono-ruby`; Ruby loads `surimono/surimono.so` and
// looks for `Init_surimono`.
#[magnus::init(name = "surimono")]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("Surimono")?;
    module.define_singleton_method("native_version", function!(native_version, 0))?;
    let class = module.define_class("Renderer", ruby.class_object())?;
    class.define_singleton_method("_native_new", function!(renderer_new, 9))?;
    class.define_method("_render", method!(Renderer::render, 1))?;
    class.define_method("renderers", method!(Renderer::renderers, 0))?;
    Ok(())
}
