//! Records a SHA-256 of the vendored QuickJS sources for the engine
//! fingerprint (`src/embed.rs`), as the `SURIMONO_ENGINE_SOURCES_SHA256`
//! compile-time variable.
//!
//! The workspace builds the engine from `vendor/rquickjs-sys` (a `[patch]`
//! of rquickjs-sys with surimono's QuickJS patches). Its patch level is a
//! minimum-safety marker, not bumped for every patch, so the fingerprint
//! also covers the sources themselves: every file under
//! `vendor/rquickjs-sys/quickjs` (subdirectories included), the engine's
//! build script (whose compile flags change what the engine does) and its
//! `Cargo.toml` (whose features and version pins do too). Any change to the engine
//! changes the hash and so refuses bytecode written before it. Outside
//! the workspace (no `vendor/`) the variable is empty and the fingerprint
//! relies on the version, the patch level and the canary.
//!
//! It also sets `cfg(surimono_sanitized)` when the crate is built with a
//! sanitizer (`-Zsanitizer=…`), for tests whose expectations differ there.

use std::path::{Path, PathBuf};
use std::{env, fs};

use sha2::{Digest, Sha256};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo::rustc-check-cfg=cfg(surimono_sanitized)");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_SANITIZE");
    if env::var_os("CARGO_CFG_SANITIZE").is_some() {
        println!("cargo:rustc-cfg=surimono_sanitized");
    }
    let manifest = env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let vendor = Path::new(&manifest).join("../../vendor/rquickjs-sys");
    let hash = hash_engine(&vendor).unwrap_or_default();
    println!("cargo:rustc-env=SURIMONO_ENGINE_SOURCES_SHA256={hash}");
}

/// SHA-256 over each file's path (relative to `vendor`, with `/`), length
/// and bytes, in path order; `None` if there is no vendored engine.
fn hash_engine(vendor: &Path) -> Option<String> {
    let sources = vendor.join("quickjs");
    if !sources.is_dir() {
        return None;
    }
    let mut files = Vec::new();
    collect(&sources, &mut files);
    files.push(vendor.join("build.rs"));
    files.push(vendor.join("Cargo.toml"));
    let mut named: Vec<(String, PathBuf)> = files
        .into_iter()
        .map(|f| (relative_name(vendor, &f), f))
        .collect();
    named.sort();
    let mut hasher = Sha256::new();
    for (name, file) in &named {
        println!("cargo:rerun-if-changed={}", file.display());
        let bytes = fs::read(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    let digest = hasher.finalize();
    Some(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// Every file under `dir`, recursively. A directory is also watched, so a
/// file added to it reruns this script. Symbolic links are followed (the
/// engine's build reads through them too).
fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .path();
        if path.is_dir() {
            collect(&path, files);
        } else {
            files.push(path);
        }
    }
}

/// `file`'s path below `base`, with `/` between components on every host.
fn relative_name(base: &Path, file: &Path) -> String {
    let rel = file.strip_prefix(base).unwrap_or(file);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}
