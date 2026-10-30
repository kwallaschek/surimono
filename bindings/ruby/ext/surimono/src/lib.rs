//! Ruby native extension for surimono, built with magnus and rb-sys.
//!
//! Rendering releases the GVL so other Ruby threads keep running.

use magnus::{Error, Ruby, function, prelude::*};

fn native_version() -> String {
    surimono::VERSION.to_string()
}

// The Cargo package is `surimono-ruby`; Ruby loads `surimono/surimono.so` and
// looks for `Init_surimono`.
#[magnus::init(name = "surimono")]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("Surimono")?;
    module.define_singleton_method("native_version", function!(native_version, 0))?;
    Ok(())
}
