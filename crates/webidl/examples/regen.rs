//! Regenerates the checked-in generated files from the vendored IDL:
//! `src/stubs.rs`, `tests/golden/mini.rs` and `crates/dom/src/generated/`. Run from anywhere with
//! `cargo run -p surimono-webidl --example regen`; review the diff.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let stubs = surimono_webidl::generate_vendored_stubs()?;
        std::fs::write(root.join("src/stubs.rs"), stubs)?;
        let mini = surimono_webidl::generate_mini_golden()?;
        std::fs::write(root.join("tests/golden/mini.rs"), mini)?;
        let bindings = surimono_webidl::generate_vendored_bindings()?;
        let dom = root.join("../dom/src/generated");
        std::fs::create_dir_all(&dom)?;
        std::fs::write(dom.join("bindings.js"), bindings.js)?;
        std::fs::write(dom.join("dispatch.rs"), bindings.rust)?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            println!("wrote src/stubs.rs, tests/golden/mini.rs and ../dom/src/generated/");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("regen failed: {e}");
            ExitCode::FAILURE
        }
    }
}
