//! Engine-binding module: bytecode compiled when a binary is built and
//! loaded at run time without parsing.
//!
//! surimono's built-in renderers are compiled by a build script that
//! depends on this crate ([`compile_for_embedding`]) and loaded with
//! [`Bytecode::from_embedded`], so the first render in a new process skips
//! parsing and compiling the bundle. QuickJS trusts the bytecode it reads,
//! so loading is only sound for bytes written by a compatible engine:
//! embedded bytecode travels with the [`engine_fingerprint`] of the engine
//! that wrote it and is refused unless the running engine's fingerprint is
//! the same, in which case the caller compiles from source instead.
//!
//! # Why a matching fingerprint means compatible bytecode
//!
//! The build script and the binary link surimono-core from one Cargo build,
//! so the writing and the reading engine are compiled from the same QuickJS
//! sources (the same version, with the same patches); Cargo re-runs a build
//! script whenever one of its build-dependencies changes, so embedded
//! bytecode is rewritten whenever the engine is. What can still differ is
//! the build target: in a cross build the writer runs on the host. The
//! fingerprint therefore records what the bytecode format could depend on
//! there (pointer width, byte order) and the bytes the engine writes for a
//! canary script (`canary.js`) that exercises every part of the format: the
//! version byte, opcode and predefined-atom numbering, function and variable
//! flag layouts, the constant pool (numbers, multi-limb BigInts, Latin-1 and
//! UTF-16 strings, template objects), regular expression bytecode, classes,
//! generators and async functions. It also includes the engine's version
//! string and the level of surimono's patches to it
//! (`vendor/rquickjs-sys/PATCHES.md`). An engine change that alters the
//! format (a patch adding an opcode or a predefined atom, a new version)
//! changes the canary's bytes and so the fingerprint, which is what makes
//! loading sound; a new patch level changes it too, even when the format
//! stays the same, because a patch can change what bytecode means (how an
//! opcode behaves) without changing how it is written.
//!
//! Embedded bytecode is written without source text or debug information
//! (see [`compile_for_embedding`]).

use std::ffi::{CStr, CString};
use std::ops::Deref;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rquickjs::{Ctx, Value, qjs};

use crate::{Bytecode, Limits, Sandbox, SandboxError};

/// The bytes of a [`Bytecode`]: compiled in this process, or built into the
/// binary.
#[derive(Clone)]
pub(crate) enum Bytes {
    Shared(Arc<[u8]>),
    Static(&'static [u8]),
}

impl Deref for Bytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Bytes::Shared(b) => b,
            Bytes::Static(b) => b,
        }
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(bytes: Vec<u8>) -> Self {
        Bytes::Shared(Arc::from(bytes))
    }
}

/// How embedded bytecode is written: functions, without source text or
/// debug information.
const EMBED_FLAGS: u32 =
    qjs::JS_WRITE_OBJ_BYTECODE | qjs::JS_WRITE_OBJ_STRIP_SOURCE | qjs::JS_WRITE_OBJ_STRIP_DEBUG;

/// Source of the fingerprint's canary script; see the module docs.
const CANARY: &str = include_str!("canary.js");

/// Changes whenever the fingerprint's layout does.
const FINGERPRINT_HEADER: &[u8] = b"surimono embedded bytecode 2\0";

/// Stack of the thread [`compile_for_embedding`] compiles on: the stack
/// limit plus room for the Rust frames around the engine. Untouched stack
/// costs no memory.
const BUILD_THREAD_STACK: usize = 64 * 1024 * 1024;

/// Limits for compiling a trusted bundle in a build script: generous,
/// because build machines are busy and build scripts' engines unoptimised.
const BUILD_LIMITS: Limits = Limits::new(
    1024 * 1024 * 1024,
    Duration::from_secs(600),
    Limits::MAX_STACK_BYTES,
);

/// Compiles a classic script for embedding in a binary, for build scripts.
///
/// Like [`Sandbox::compile`], except that the bytecode carries no source
/// text and no debug information, which makes it (and the heap of every
/// runtime that loads it) several times smaller: QuickJS otherwise keeps
/// the source of every function for `Function.prototype.toString`, so
/// nested functions repeat the bundle's text. Scripts loaded from it behave
/// the same, except that `toString()` of their functions returns
/// `function name() { [native code] }` and stack traces name functions
/// without file and line.
///
/// Runs under generous limits (1 GiB, 10 minutes, a 16 MiB stack limit), on
/// a thread of its own with room for that stack. The output depends only on
/// `source`, `filename` and the engine: compiling twice gives the same
/// bytes.
pub fn compile_for_embedding(source: &str, filename: &str) -> Result<Vec<u8>, SandboxError> {
    std::thread::scope(|scope| {
        let compiler = std::thread::Builder::new()
            .name("surimono-embed".into())
            .stack_size(BUILD_THREAD_STACK)
            .spawn_scoped(scope, || compile_stripped(source, filename, BUILD_LIMITS))
            .map_err(|e| SandboxError::Engine {
                message: format!("could not start the compiler thread: {e}"),
            })?;
        compiler.join().map_err(|_| SandboxError::Engine {
            message: "the compiler thread panicked".into(),
        })?
    })
}

fn compile_stripped(source: &str, filename: &str, limits: Limits) -> Result<Vec<u8>, SandboxError> {
    let mut sb = Sandbox::new(limits)?;
    sb.run(|ctx| write_script(ctx, source, filename, EMBED_FLAGS))
}

/// Identifies the bytecode format of this engine build; see the module docs
/// for what it covers. `None` if the canary could not be compiled (only if
/// memory ran out), in which case no embedded bytecode is loaded.
///
/// Computed once per process (compiling the canary takes about a
/// millisecond).
pub fn engine_fingerprint() -> Option<&'static [u8]> {
    static FINGERPRINT: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    FINGERPRINT
        .get_or_init(|| {
            let canary = compile_stripped(CANARY, "surimono:canary", Limits::default()).ok()?;
            let mut fp = FINGERPRINT_HEADER.to_vec();
            fp.extend_from_slice(engine_version().to_bytes_with_nul());
            fp.extend_from_slice(&crate::engine::patch_level().to_le_bytes());
            fp.push(std::mem::size_of::<usize>() as u8);
            fp.push(if cfg!(target_endian = "little") {
                b'l'
            } else {
                b'b'
            });
            fp.extend_from_slice(&canary);
            Some(fp)
        })
        .as_deref()
}

/// QuickJS's version string, e.g. `0.16.2`.
fn engine_version() -> &'static CStr {
    // SAFETY: JS_GetVersion takes no arguments and returns a pointer to a
    // static, NUL-terminated string literal in the engine; it never fails.
    unsafe { CStr::from_ptr(qjs::JS_GetVersion()) }
}

impl Bytecode {
    /// Bytecode built into the binary, loaded without copying, if it was
    /// written by a compatible engine: `fingerprint` (the writing engine's
    /// [`engine_fingerprint`]) must equal this engine's. Returns `None`
    /// otherwise, or for empty `bytes`; compile from source then.
    ///
    /// # Safety
    ///
    /// `bytes` must be exactly what [`compile_for_embedding`] returned in an
    /// engine whose [`engine_fingerprint`] returned `fingerprint`, as when
    /// one build script writes both. QuickJS trusts the bytecode it reads:
    /// any other bytes with a matching fingerprint are undefined behaviour.
    pub unsafe fn from_embedded(
        name: &str,
        bytes: &'static [u8],
        fingerprint: &[u8],
    ) -> Option<Bytecode> {
        let ours = engine_fingerprint()?;
        if bytes.is_empty() || fingerprint != ours {
            return None;
        }
        Some(Bytecode {
            name: Arc::from(name),
            bytes: Bytes::Static(bytes),
        })
    }

    /// Whether this bytecode was built into the binary (see
    /// [`from_embedded`](Self::from_embedded)) rather than compiled in this
    /// process.
    pub fn is_embedded(&self) -> bool {
        matches!(self.bytes, Bytes::Static(_))
    }
}

/// Compiles a classic script and writes its bytecode with `flags`.
///
/// On a syntax error, returns `Err(Exception)` with the error pending on
/// `ctx`. (`engine::compile_script` with the flags as a parameter; kept
/// apart while that file is being reworked on other branches.)
fn write_script(
    ctx: &Ctx<'_>,
    source: &str,
    filename: &str,
    flags: u32,
) -> rquickjs::Result<Vec<u8>> {
    // JS_Eval requires input[len] == 0.
    let mut input = Vec::with_capacity(source.len() + 1);
    input.extend_from_slice(source.as_bytes());
    input.push(0);
    let filename = CString::new(filename.replace('\0', "?"))?;
    let raw = ctx.as_raw().as_ptr();
    let eval_flags = (qjs::JS_EVAL_TYPE_GLOBAL | qjs::JS_EVAL_FLAG_COMPILE_ONLY) as i32;
    // SAFETY: `raw` is a live context (we hold `ctx`, which keeps it alive and
    // the runtime locked); `input` is NUL-terminated and outlives the call, and
    // the length passed excludes the terminator, as JS_Eval requires.
    let func = unsafe {
        qjs::JS_Eval(
            raw,
            input.as_ptr().cast(),
            source.len() as _,
            filename.as_ptr(),
            eval_flags,
        )
    };
    // SAFETY: inspecting the tag of a value we own is always valid.
    if unsafe { qjs::JS_IsException(func) } {
        return Err(rquickjs::Error::Exception);
    }
    // SAFETY: `func` is an owned value from this context's runtime; `Value`
    // takes over its reference and frees it on drop.
    let func = unsafe { Value::from_raw(ctx.clone(), func) };
    let mut len: qjs::size_t = 0;
    // SAFETY: `func` is alive for the duration of the call; JS_WriteObject
    // returns a buffer allocated with the context's allocator, or null with an
    // exception pending. The flags only select what is written.
    let buf = unsafe { qjs::JS_WriteObject(raw, &mut len, func.as_raw(), flags as i32) };
    if buf.is_null() {
        return Err(rquickjs::Error::Exception);
    }
    let len = usize::try_from(len).map_err(|_| rquickjs::Error::Allocation)?;
    // SAFETY: JS_WriteObject returned a non-null buffer of exactly `len`
    // initialised bytes, which we copy before freeing it with the same
    // context's allocator, as its documentation requires.
    let bytes = unsafe {
        let bytes = std::slice::from_raw_parts(buf, len).to_vec();
        qjs::js_free(raw, buf.cast());
        bytes
    };
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r#"
var log = [];
function outer(a) {
  function inner(b) { return a * b; }
  return inner(6);
}
function fails() { throw new TypeError('embedded failure ' + outer(7)); }
log.push(outer(7));
"#;

    fn embedded(source: &str) -> &'static [u8] {
        Box::leak(
            compile_for_embedding(source, "test.js")
                .unwrap()
                .into_boxed_slice(),
        )
    }

    fn fingerprint() -> &'static [u8] {
        engine_fingerprint().expect("the canary compiles")
    }

    #[test]
    fn the_canary_compiles_and_the_fingerprint_is_stable() {
        let fp = fingerprint();
        assert!(fp.starts_with(FINGERPRINT_HEADER));
        assert!(fp.len() > 1000, "covers the canary: {} bytes", fp.len());
        assert_eq!(engine_fingerprint(), Some(fp));
        // Recomputed from scratch, it is byte for byte the same.
        let canary = compile_stripped(CANARY, "surimono:canary", Limits::default()).unwrap();
        assert!(fp.ends_with(&canary));
    }

    #[test]
    fn the_fingerprint_covers_the_format() {
        // A change to any part of the canary changes the fingerprint's
        // canary bytes; spot-check the parts the module docs name.
        let base = compile_stripped(CANARY, "c", Limits::default()).unwrap();
        for (from, to) in [
            ("0x10000000000000000n", "0x10000000000000001n"),
            ("'utf16 \\u3042", "'utf16 \\u3043"),
            ("/[\\p{L}\\p{N}]+/u", "/[\\p{L}\\p{Nd}]+/u"),
            ("1.7976931348623157e308", "1.7976931348623155e308"),
            ("static #count = 0;", "static #count = 1;"),
        ] {
            assert!(CANARY.contains(from), "{from}");
            let changed =
                compile_stripped(&CANARY.replace(from, to), "c", Limits::default()).unwrap();
            assert_ne!(base, changed, "{from} -> {to}");
        }
    }

    #[test]
    fn compiling_for_embedding_is_deterministic() {
        assert_eq!(
            compile_for_embedding(SCRIPT, "a.js").unwrap(),
            compile_for_embedding(SCRIPT, "a.js").unwrap()
        );
    }

    #[test]
    fn embedded_bytecode_is_stripped_and_smaller() {
        let mut sb = Sandbox::new(Limits::default()).unwrap();
        let full = sb.compile(SCRIPT, "a.js").unwrap();
        let stripped = compile_for_embedding(SCRIPT, "a.js").unwrap();
        assert!(
            stripped.len() < full.size_bytes(),
            "{} vs {}",
            stripped.len(),
            full.size_bytes()
        );
        assert!(
            !stripped.windows(9).any(|w| w == b"inner(b) "),
            "no source text"
        );
    }

    #[test]
    fn embedded_bytecode_runs_like_compiled_bytecode() {
        // SAFETY: the bytes and the fingerprint come from this engine.
        let bc = unsafe { Bytecode::from_embedded("test.js", embedded(SCRIPT), fingerprint()) }
            .expect("same engine, same fingerprint");
        assert!(bc.is_embedded());
        assert_eq!(bc.name(), "test.js");
        let mut sb = Sandbox::new(Limits::default()).unwrap();
        sb.eval_bytecode(&bc).unwrap();
        assert_eq!(sb.eval::<String>("log.join()").unwrap(), "42");
        assert_eq!(sb.eval::<i32>("outer(2)").unwrap(), 12);
        // Errors keep their type and message; stack traces lose the line.
        match sb.eval::<()>("fails()").unwrap_err() {
            SandboxError::Exception { message, .. } => {
                assert_eq!(message, "TypeError: embedded failure 42")
            }
            other => panic!("{other:?}"),
        }
        // Functions no longer carry their source.
        let text: String = sb.eval("outer.toString()").unwrap();
        assert!(text.contains("[native code]"), "{text}");
        assert!(!sb.compile("1", "x.js").unwrap().is_embedded());
    }

    #[test]
    fn a_different_fingerprint_is_refused() {
        let bytes = embedded(SCRIPT);
        let fp = fingerprint();
        let mut other = fp.to_vec();
        // A different engine: one byte of the canary's bytecode differs.
        let last = other.len() - 1;
        other[last] ^= 1;
        let shorter = &fp[..fp.len() - 1];
        let mut version = fp.to_vec();
        version[FINGERPRINT_HEADER.len()] ^= 1;
        for wrong in [&other[..], shorter, &version[..], b"", FINGERPRINT_HEADER] {
            // SAFETY: refused before anything is read.
            assert!(unsafe { Bytecode::from_embedded("t", bytes, wrong) }.is_none());
        }
        // SAFETY: empty bytecode is refused before anything is read.
        assert!(unsafe { Bytecode::from_embedded("t", b"", fp) }.is_none());
    }

    #[test]
    fn syntax_errors_are_reported() {
        assert!(matches!(
            compile_for_embedding("let = ;", "bad.js"),
            Err(SandboxError::Exception { .. })
        ));
    }

    #[test]
    fn the_fingerprint_includes_the_patch_level() {
        let fp = fingerprint();
        let version = engine_version().to_bytes_with_nul();
        let at = FINGERPRINT_HEADER.len() + version.len();
        assert_eq!(&fp[FINGERPRINT_HEADER.len()..at], version);
        assert_eq!(
            fp[at..at + 4],
            crate::engine::patch_level().to_le_bytes(),
            "the patch level follows the version"
        );
        // Bytecode recorded at another patch level is refused.
        let mut other = fp.to_vec();
        other[at] ^= 1;
        let bytes = embedded(SCRIPT);
        // SAFETY: refused before anything is read.
        assert!(unsafe { Bytecode::from_embedded("t", bytes, &other) }.is_none());
    }

    #[test]
    fn the_version_is_readable() {
        let v = engine_version().to_str().unwrap();
        assert_eq!(v.split('.').count(), 3, "{v}");
    }
}
