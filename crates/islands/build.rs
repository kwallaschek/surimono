//! Compiles the built-in bundles to QuickJS bytecode when the crate is
//! built, so the first render in a new process skips parsing and compiling
//! them (P1-RT-17).
//!
//! For each enabled built-in, writes `$OUT_DIR/<name>.qjsbc` with
//! `surimono_core::compile_for_embedding` (no source text, no debug info),
//! and writes `$OUT_DIR/engine.fingerprint` with the fingerprint of the
//! engine that compiled them. At run time the runner loads the bytecode
//! only if the running engine has the same fingerprint, and otherwise
//! compiles the bundle from source (`src/embedded.rs`). The engine here is
//! surimono-core from the same Cargo build, so the two match except in
//! unusual cross builds, which the fingerprint catches.
//!
//! A bundle that fails to compile here gets empty bytecode and a warning;
//! it then compiles from source at run time, and the crate's unit tests
//! fail (`src/embedded.rs`), so a build without its bytecode doesn't go
//! unnoticed.

use std::path::PathBuf;
use std::{env, fs};

/// (Cargo feature variable, source, file name in OUT_DIR, name in stack traces)
const BUNDLES: &[(&str, &str, &str, &str)] = &[
    (
        "CARGO_FEATURE_ECHARTS",
        "bundles/echarts/echarts.min.js",
        "echarts.qjsbc",
        "echarts.js",
    ),
    (
        "CARGO_FEATURE_VEGA",
        "bundles/vega/vega.min.js",
        "vega.qjsbc",
        "vega.js",
    ),
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let fingerprint = surimono_core::engine_fingerprint();
    if fingerprint.is_none() {
        println!(
            "cargo:warning=surimono-islands: the engine fingerprint is unavailable; \
             built-in bundles will compile from source at run time"
        );
    }
    for &(feature, source, file, name) in BUNDLES {
        println!("cargo:rerun-if-changed={source}");
        let mut bytes = Vec::new();
        if env::var_os(feature).is_some() && fingerprint.is_some() {
            let js = fs::read_to_string(source).unwrap_or_else(|e| panic!("{source}: {e}"));
            match surimono_core::compile_for_embedding(&js, name) {
                Ok(compiled) => bytes = compiled,
                Err(e) => println!(
                    "cargo:warning=surimono-islands: {source} was not precompiled ({e}); \
                     it will compile from source at run time"
                ),
            }
        }
        let path = out.join(file);
        fs::write(&path, &bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
    let path = out.join("engine.fingerprint");
    fs::write(&path, fingerprint.unwrap_or_default())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}
