//! Ruby native extension for surimono, built with magnus and rb-sys.
//!
//! Rendering releases the GVL so other Ruby threads keep running.

use magnus::{Error, Ruby, function, prelude::*};

fn native_version() -> String {
    surimono::VERSION.to_string()
}

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("Surimono")?;
    module.define_singleton_method("native_version", function!(native_version, 0))?;
    Ok(())
}
