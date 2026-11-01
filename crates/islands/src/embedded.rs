//! Engine-binding module: loads the built-in bundles' bytecode that
//! `build.rs` compiled, the only `unsafe` in this crate.
//!
//! Loading bytecode skips parsing and compiling, which is most of the first
//! render's cost in a new process. QuickJS trusts the bytecode it reads, so
//! only bytes this crate's build script wrote are ever loaded, and only when
//! the running engine's fingerprint equals the one recorded with them
//! (`surimono_core::Bytecode::from_embedded`); otherwise the runner compiles
//! the bundle's source, as for any registered bundle. It also compiles the
//! source if the bytecode wasn't compiled from exactly the source built into
//! this binary (its recorded SHA-256 differs), or if the environment
//! variable `SURIMONO_PRECOMPILE` is `0` (a debugging escape hatch; the
//! output is the same either way, only the first render is slower).

use std::ffi::OsStr;
use std::sync::atomic::{AtomicU8, Ordering};

use sha2::{Digest, Sha256};
use surimono_core::Bytecode;

use crate::builtins::{ALL, Builtin};

/// A yes-or-no answer worked out once per process: [`UNKNOWN`] until a
/// thread has it. Atomics, not a `OnceLock` or `Mutex`: a process forked
/// while another thread was working one out would wait on that thread
/// forever. Threads that race just work it out twice.
const UNKNOWN: u8 = 0;
const NO: u8 = 1;
const YES: u8 = 2;

/// The cached answer in `cell`, or `compute`'s, stored for next time.
fn cached(cell: &AtomicU8, compute: impl FnOnce() -> bool) -> bool {
    match cell.load(Ordering::Acquire) {
        NO => false,
        YES => true,
        _ => {
            let answer = compute();
            cell.store(if answer { YES } else { NO }, Ordering::Release);
            answer
        }
    }
}

/// The built-in's precompiled bytecode if it may and can be used, `None` to
/// compile its source instead. `fingerprint` is the writing engine's:
/// always [`ENGINE_FINGERPRINT`](crate::builtins::ENGINE_FINGERPRINT) except
/// in tests of the fallback.
pub(crate) fn bytecode(builtin: &Builtin, fingerprint: &[u8]) -> Option<Bytecode> {
    if disabled_at_run_time() {
        return None;
    }
    load(builtin, fingerprint)
}

/// Whether `SURIMONO_PRECOMPILE` turns precompiled bytecode off: read once
/// per process.
fn disabled_at_run_time() -> bool {
    static DISABLED: AtomicU8 = AtomicU8::new(UNKNOWN);
    cached(&DISABLED, || {
        switched_off(std::env::var_os("SURIMONO_PRECOMPILE").as_deref())
    })
}

/// `SURIMONO_PRECOMPILE=0` means off; unset or anything else means on.
fn switched_off(value: Option<&OsStr>) -> bool {
    value.is_some_and(|v| v == "0")
}

/// Whether `builtin.source` is the source `build.rs` compiled its bytecode
/// from. Each built-in in [`ALL`] is hashed once per process (another
/// `Builtin`, a test's altered copy, every time).
fn source_matches(builtin: &Builtin) -> bool {
    static CHECKED: [AtomicU8; ALL.len()] = [const { AtomicU8::new(UNKNOWN) }; ALL.len()];
    let hash = || Sha256::digest(builtin.source.as_bytes()).as_slice() == builtin.source_sha256;
    let known = ALL.iter().position(|b| {
        std::ptr::eq(b.source, builtin.source) && b.source_sha256 == builtin.source_sha256
    });
    match known.and_then(|i| CHECKED.get(i)) {
        Some(cell) => cached(cell, hash),
        None => hash(),
    }
}

/// The bytecode, if it was compiled from this source by a compatible engine.
fn load(builtin: &Builtin, fingerprint: &[u8]) -> Option<Bytecode> {
    if !source_matches(builtin) {
        return None;
    }
    let name = format!("{}.js", builtin.name);
    // SAFETY: `builtin.embedded` is a `Builtin` constant's bytes, which are
    // only ever `include_bytes!` of a file build.rs wrote with the unmodified
    // output of `surimono_core::compile_for_embedding` (or empty, which is
    // refused). ENGINE_FINGERPRINT is `engine_fingerprint()` of the same
    // engine, written by the same build.rs run. `from_embedded` loads the
    // bytes only if `fingerprint` equals the running engine's fingerprint,
    // so the bytes were written by an engine whose fingerprint is
    // `fingerprint`; any other value passed here (a test's corrupted copy)
    // can only make it refuse.
    unsafe { Bytecode::from_embedded(&name, builtin.embedded, fingerprint) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::{ALL, ENGINE_FINGERPRINT};

    /// Whether this build or run turned precompiled bytecode off.
    fn switched_off_here() -> bool {
        option_env!("SURIMONO_PRECOMPILE") == Some("0") || disabled_at_run_time()
    }

    #[test]
    fn every_builtin_is_precompiled_for_this_engine() {
        if switched_off_here() {
            return;
        }
        assert!(
            ENGINE_FINGERPRINT.starts_with(b"surimono embedded bytecode"),
            "build.rs wrote no fingerprint"
        );
        for b in ALL {
            assert!(
                !b.embedded.is_empty(),
                "{}: build.rs wrote no bytecode",
                b.name
            );
            assert!(source_matches(b), "{}: bytecode from other source", b.name);
            let bc = bytecode(b, ENGINE_FINGERPRINT)
                .unwrap_or_else(|| panic!("{}: refused by the same engine", b.name));
            assert!(bc.is_embedded());
            assert_eq!(bc.size_bytes(), b.embedded.len());
        }
    }

    #[test]
    fn precompiled_bytecode_is_what_this_engine_writes() {
        if switched_off_here() {
            return;
        }
        // The bytecode build.rs wrote (its engine is a separate,
        // differently optimised build of the same sources) is byte for byte
        // what the engine linked into the tests writes now: compiling is
        // deterministic.
        for b in ALL {
            let now =
                surimono_core::compile_for_embedding(b.source, &format!("{}.js", b.name)).unwrap();
            assert!(
                now == b.embedded,
                "{}: {} vs {} bytes",
                b.name,
                now.len(),
                b.embedded.len()
            );
        }
    }

    #[test]
    fn bytecode_for_other_source_is_refused() {
        for b in ALL {
            // The genuine bytecode and hash, with a newer bundle's source.
            let newer: &'static str = Box::leak(format!("{}\n// newer", b.source).into_boxed_str());
            // Keep it reachable, so LeakSanitizer (scripts/ci/asan.sh)
            // doesn't report the deliberate leak.
            static KEEP: std::sync::Mutex<Vec<&str>> = std::sync::Mutex::new(Vec::new());
            KEEP.lock().unwrap_or_else(|e| e.into_inner()).push(newer);
            let stale = Builtin {
                source: newer,
                ..*b
            };
            assert!(!source_matches(&stale), "{}", b.name);
            assert!(load(&stale, ENGINE_FINGERPRINT).is_none(), "{}", b.name);
        }
    }

    #[test]
    fn the_switch_is_zero_only() {
        assert!(switched_off(Some(OsStr::new("0"))));
        for on in [None, Some("1"), Some(""), Some("false"), Some("00")] {
            assert!(!switched_off(on.map(OsStr::new)), "{on:?}");
        }
    }

    #[test]
    fn another_engines_bytecode_is_refused() {
        let mut other = ENGINE_FINGERPRINT.to_vec();
        let last = other.len() - 1;
        other[last] ^= 0x40;
        for b in ALL {
            assert!(bytecode(b, &other).is_none(), "{}", b.name);
            assert!(
                bytecode(b, &ENGINE_FINGERPRINT[..last]).is_none(),
                "{}",
                b.name
            );
            assert!(bytecode(b, b"").is_none(), "{}", b.name);
        }
    }

    #[test]
    fn empty_bytecode_is_refused() {
        let none = Builtin {
            name: "none",
            source: "",
            embedded: b"",
            source_sha256: b"",
            entry: "entry",
        };
        assert!(bytecode(&none, ENGINE_FINGERPRINT).is_none());
    }
}
