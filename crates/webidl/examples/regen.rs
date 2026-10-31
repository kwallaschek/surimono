//! Regenerates the checked-in generated files from the vendored IDL:
//! `src/stubs.rs` and `tests/golden/mini.rs`. Run from anywhere with
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
        Ok(())
    })();
    match result {
        Ok(()) => {
            println!("wrote src/stubs.rs and tests/golden/mini.rs");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("regen failed: {e}");
            ExitCode::FAILURE
        }
    }
}
