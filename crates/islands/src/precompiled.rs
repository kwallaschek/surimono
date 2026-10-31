//! The built-in bundles' bytecode, precompiled by `build.rs`.
//!
//! Loading it skips parsing a megabyte-sized bundle in every fresh process.
//! Only the blobs `build.rs` wrote into `OUT_DIR` are ever loaded: QuickJS
//! doesn't validate bytecode, so nothing here takes bytes from anywhere else.
//! `Bytecode::from_precompiled` also checks each blob against the running
//! engine and target and the bundle's source; if any differ (or nothing was
//! precompiled, e.g. when cross-compiling), the runner compiles the source.

use std::sync::OnceLock;

use surimono_core::Bytecode;

use crate::builtins::Builtin;

static ECHARTS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/echarts.js.qbc"));
static VEGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/vega.js.qbc"));

/// The precompiled bytecode for `builtin`, if this build has a blob for it
/// that matches the running engine and the built-in's source. Checked once
/// per process.
pub(crate) fn bytecode(builtin: &Builtin) -> Option<Bytecode> {
    static CACHE: [OnceLock<Option<Bytecode>>; 2] = [OnceLock::new(), OnceLock::new()];
    let (slot, blob) = match builtin.file {
        "echarts.js" => (&CACHE[0], ECHARTS),
        "vega.js" => (&CACHE[1], VEGA),
        _ => return None,
    };
    slot.get_or_init(|| {
        if blob.is_empty() {
            return None;
        }
        // SAFETY: `blob` is `include_bytes!` of the file this crate's own
        // build script wrote with `Bytecode::precompile` for this bundle, and
        // nothing else; `from_precompiled` rejects it unless it was built by
        // this engine for this target from exactly `builtin.source`.
        unsafe { Bytecode::from_precompiled(blob, builtin.source) }
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins;

    #[test]
    fn every_builtin_is_precompiled_in_a_native_build() {
        // Test builds are native and don't set SURIMONO_PRECOMPILE=0.
        if option_env!("SURIMONO_PRECOMPILE") == Some("0") {
            return;
        }
        for name in builtins::builtin_renderers() {
            let b = builtins::find(name).unwrap();
            let bc = bytecode(&b).unwrap_or_else(|| panic!("{name} was not precompiled"));
            assert!(bc.is_precompiled());
            assert_eq!(bc.name(), b.file);
        }
    }

    #[test]
    fn a_blob_never_loads_for_other_source() {
        if ECHARTS.is_empty() {
            return;
        }
        // SAFETY: `ECHARTS` is this build's own precompiled blob.
        assert!(unsafe { Bytecode::from_precompiled(ECHARTS, "var x = 1") }.is_none());
    }
}
