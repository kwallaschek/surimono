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
//! Next to each bytecode file it writes `<name>.sha256`, the SHA-256 of the
//! exact source it compiled; the runner checks it against the source built
//! into the binary, so bytecode never runs for a different bundle.
//!
//! A bundle that fails to compile here gets empty bytecode and a warning;
//! it then compiles from source at run time, and the crate's unit tests
//! fail (`src/embedded.rs`), so a build without its bytecode doesn't go
//! unnoticed.
//!
//! `SURIMONO_PRECOMPILE=0` builds without bytecode (every built-in then
//! compiles from source at run time, with the same output): a debugging
//! escape hatch. The runner honours the same variable at run time.

use std::path::PathBuf;
use std::{env, fs};

use sha2::{Digest, Sha256};

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
    println!("cargo:rerun-if-env-changed=SURIMONO_PRECOMPILE");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let enabled = env::var_os("SURIMONO_PRECOMPILE").is_none_or(|v| v != "0");
    let fingerprint = surimono_core::engine_fingerprint();
    if !enabled {
        println!(
            "cargo:warning=surimono-islands: SURIMONO_PRECOMPILE=0, so built-in bundles \
             will compile from source at run time"
        );
    } else if fingerprint.is_none() {
        println!(
            "cargo:warning=surimono-islands: the engine fingerprint is unavailable; \
             built-in bundles will compile from source at run time"
        );
    }
    for &(feature, source, file, name) in BUNDLES {
        println!("cargo:rerun-if-changed={source}");
        let (mut bytes, mut source_sha256) = (Vec::new(), Vec::new());
        if enabled && env::var_os(feature).is_some() && fingerprint.is_some() {
            let js = fs::read_to_string(source).unwrap_or_else(|e| panic!("{source}: {e}"));
            match surimono_core::compile_for_embedding(&js, name) {
                Ok(compiled) => {
                    bytes = compiled;
                    source_sha256 = Sha256::digest(js.as_bytes()).to_vec();
                }
                Err(e) => println!(
                    "cargo:warning=surimono-islands: {source} was not precompiled ({e}); \
                     it will compile from source at run time"
                ),
            }
        }
        write(&out.join(file), &bytes);
        write(&out.join(file).with_extension("sha256"), &source_sha256);
    }
    write(
        &out.join("engine.fingerprint"),
        fingerprint.unwrap_or_default(),
    );
}

fn write(path: &std::path::Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}
