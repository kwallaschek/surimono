//! Records a SHA-256 of the vendored QuickJS sources for the engine
//! fingerprint (`src/embed.rs`), as the `SURIMONO_ENGINE_SOURCES_SHA256`
//! compile-time variable.
//!
//! The workspace builds the engine from `vendor/rquickjs-sys` (a `[patch]`
//! of rquickjs-sys with surimono's QuickJS patches). Its patch level is a
//! minimum-safety marker, not bumped for every patch, so the fingerprint
//! also covers the sources themselves: every file in
//! `vendor/rquickjs-sys/quickjs` and the engine's build script (whose
//! compile flags change what the engine does). Any change to the engine
//! changes the hash and so refuses bytecode written before it. Outside
//! the workspace (no `vendor/`) the variable is empty and the fingerprint
//! relies on the version, the patch level and the canary.

use std::path::Path;
use std::{env, fs};

use sha2::{Digest, Sha256};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest = env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let vendor = Path::new(&manifest).join("../../vendor/rquickjs-sys");
    let hash = hash_engine(&vendor).unwrap_or_default();
    println!("cargo:rustc-env=SURIMONO_ENGINE_SOURCES_SHA256={hash}");
}

/// SHA-256 over each file's name, length and bytes, in name order; `None`
/// if there is no vendored engine.
fn hash_engine(vendor: &Path) -> Option<String> {
    let sources = vendor.join("quickjs");
    let mut files: Vec<_> = fs::read_dir(&sources)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    files.push(vendor.join("build.rs"));
    println!("cargo:rerun-if-changed={}", sources.display());
    let mut hasher = Sha256::new();
    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
        let bytes = fs::read(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    let digest = hasher.finalize();
    Some(digest.iter().map(|b| format!("{b:02x}")).collect())
}
