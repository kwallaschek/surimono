//! Precompiles the built-in bundles to QuickJS bytecode (P1-RT-17).
//!
//! For each built-in enabled by a Cargo feature, writes
//! `$OUT_DIR/<file>.qbc`: a blob from `surimono_core::Bytecode::precompile`,
//! which `src/precompiled.rs` embeds and loads, so a fresh process skips
//! parsing the bundle. The blob records the engine and target it was made
//! with; the runtime loads it only if they and the source match, and
//! compiles the source otherwise.
//!
//! An empty file (nothing precompiled) is written when the feature is off,
//! when cross-compiling (the build script's engine is the host's), when
//! `SURIMONO_PRECOMPILE=0`, or if compiling fails (with a warning). The
//! build never fails because of this step.

use std::path::{Path, PathBuf};

struct Bundle {
    feature: &'static str,
    path: &'static str,
    file: &'static str,
}

const BUNDLES: &[Bundle] = &[
    Bundle {
        feature: "CARGO_FEATURE_ECHARTS",
        path: "bundles/echarts/echarts.min.js",
        file: "echarts.js",
    },
    Bundle {
        feature: "CARGO_FEATURE_VEGA",
        path: "bundles/vega/vega.min.js",
        file: "vega.js",
    },
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SURIMONO_PRECOMPILE");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let native = std::env::var("HOST").ok() == std::env::var("TARGET").ok();
    let enabled = std::env::var("SURIMONO_PRECOMPILE").as_deref() != Ok("0");
    for b in BUNDLES {
        println!("cargo:rerun-if-changed={}", b.path);
        let blob = if native && enabled && std::env::var_os(b.feature).is_some() {
            precompile(b)
        } else {
            Vec::new()
        };
        write(&out.join(format!("{}.qbc", b.file)), &blob);
    }
}

fn precompile(b: &Bundle) -> Vec<u8> {
    let source = match std::fs::read_to_string(b.path) {
        Ok(s) => s,
        Err(e) => {
            println!("cargo:warning=surimono-islands: reading {}: {e}", b.path);
            return Vec::new();
        }
    };
    match surimono_core::Bytecode::precompile(&source, b.file) {
        Ok(blob) => blob,
        Err(e) => {
            println!(
                "cargo:warning=surimono-islands: precompiling {} failed ({e}); it will be compiled at run time",
                b.path
            );
            Vec::new()
        }
    }
}

fn write(path: &Path, data: &[u8]) {
    // Only rewrite on change, so dependants don't rebuild needlessly.
    if std::fs::read(path).ok().as_deref() == Some(data) {
        return;
    }
    if let Err(e) = std::fs::write(path, data) {
        panic!("writing {}: {e}", path.display());
    }
}
