//! Engine-binding module: loads the built-in bundles' bytecode that
//! `build.rs` compiled, the only `unsafe` in this crate.
//!
//! Loading bytecode skips parsing and compiling, which is most of the first
//! render's cost in a new process. QuickJS trusts the bytecode it reads, so
//! only bytes this crate's build script wrote are ever loaded, and only when
//! the running engine's fingerprint equals the one recorded with them
//! (`surimono_core::Bytecode::from_embedded`); otherwise the runner compiles
//! the bundle's source, as for any registered bundle.

use surimono_core::Bytecode;

use crate::builtins::Builtin;

/// The built-in's precompiled bytecode if the running engine can read it,
/// `None` to compile its source instead. `fingerprint` is the writing
/// engine's: always [`ENGINE_FINGERPRINT`](crate::builtins::ENGINE_FINGERPRINT)
/// except in tests of the fallback.
pub(crate) fn bytecode(builtin: &Builtin, fingerprint: &[u8]) -> Option<Bytecode> {
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

    #[test]
    fn every_builtin_is_precompiled_for_this_engine() {
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
            let bc = bytecode(b, ENGINE_FINGERPRINT)
                .unwrap_or_else(|| panic!("{}: refused by the same engine", b.name));
            assert!(bc.is_embedded());
            assert_eq!(bc.size_bytes(), b.embedded.len());
        }
    }

    #[test]
    fn precompiled_bytecode_is_what_this_engine_writes() {
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
            entry: "entry",
        };
        assert!(bytecode(&none, ENGINE_FINGERPRINT).is_none());
    }
}
