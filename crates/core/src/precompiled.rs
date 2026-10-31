//! Bytecode precompiled at build time, framed with what it was built for.
//!
//! QuickJS loads bytecode without checking it: bytes it didn't write itself,
//! or wrote with another engine build, can crash the process. So a blob made
//! by [`Bytecode::precompile`] records the engine (version, surimono patch
//! level, target), a SHA-256 of the source it was compiled from and one of
//! the bytecode itself, and [`Bytecode::from_precompiled`] loads it only if
//! all of them match the running build and the source the caller has. On any
//! mismatch the caller compiles the source instead, so a stale blob costs
//! time, never correctness.
//!
//! These checks catch stale or damaged blobs, not forged ones: anyone who can
//! hand bytes to `from_precompiled` can also write a matching header. That is
//! why it is an `unsafe fn` whose contract is "only bytes this build's own
//! build script wrote".

use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::{Bytecode, Limits, Sandbox, SandboxError, engine};

/// The first bytes of every blob.
const MAGIC: &[u8; 12] = b"SURIMONO-QBC";
/// Version of the framing below.
const FORMAT: u32 = 1;

/// What a blob must have been built with to be loaded here: engine version,
/// patch level and the target's architecture, OS, ABI, byte order and
/// pointer width.
pub(crate) fn engine_stamp() -> String {
    format!(
        "quickjs-ng {}; surimono patch level {}; rquickjs-sys 0.14.0; {} {} {} {} {}-bit",
        engine::engine_version(),
        engine::patch_level(),
        std::env::consts::ARCH,
        std::env::consts::OS,
        if cfg!(target_env = "musl") {
            "musl"
        } else if cfg!(target_env = "gnu") {
            "gnu"
        } else if cfg!(target_env = "msvc") {
            "msvc"
        } else {
            "other"
        },
        if cfg!(target_endian = "little") {
            "le"
        } else {
            "be"
        },
        usize::BITS,
    )
}

/// Why a blob was not loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mismatch {
    /// Not a blob, a truncated one, or another framing version.
    Malformed,
    /// Built by or for another engine or target.
    Engine { built: String },
    /// Compiled from other source than the caller's.
    Source,
    /// The blob doesn't match its recorded hash.
    Corrupt,
}

/// A parsed, checked blob.
struct Parsed<'a> {
    name: &'a str,
    body: &'a [u8],
}

fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

fn frame(stamp: &str, name: &str, source: &str, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 256);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT.to_le_bytes());
    for field in [stamp.as_bytes(), name.as_bytes()] {
        // Both are short (a few dozen bytes); u32 lengths are plenty.
        out.extend_from_slice(&(field.len() as u32).to_le_bytes());
        out.extend_from_slice(field);
    }
    out.extend_from_slice(&sha256(source.as_bytes()));
    out.extend_from_slice(&(body.len() as u64).to_le_bytes());
    out.extend_from_slice(body);
    let digest = sha256(&out);
    out.extend_from_slice(&digest);
    out
}

/// Reads `n` bytes from the front of `rest`.
fn take<'a>(rest: &mut &'a [u8], n: usize) -> Result<&'a [u8], Mismatch> {
    if rest.len() < n {
        return Err(Mismatch::Malformed);
    }
    let (head, tail) = rest.split_at(n);
    *rest = tail;
    Ok(head)
}

fn take_u32(rest: &mut &[u8]) -> Result<u32, Mismatch> {
    let b = take(rest, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn take_str<'a>(rest: &mut &'a [u8]) -> Result<&'a str, Mismatch> {
    let n = take_u32(rest)? as usize;
    std::str::from_utf8(take(rest, n)?).map_err(|_| Mismatch::Malformed)
}

/// Parses `blob` and checks it against `stamp` and `source`.
fn check<'a>(blob: &'a [u8], stamp: &str, source: &str) -> Result<Parsed<'a>, Mismatch> {
    if blob.len() < MAGIC.len() + 32 || &blob[..MAGIC.len()] != MAGIC {
        return Err(Mismatch::Malformed);
    }
    let (framed, digest) = blob.split_at(blob.len() - 32);
    let mut rest = &framed[MAGIC.len()..];
    if take_u32(&mut rest)? != FORMAT {
        return Err(Mismatch::Malformed);
    }
    let built = take_str(&mut rest)?;
    let name = take_str(&mut rest)?;
    let source_hash = take(&mut rest, 32)?;
    let len = take(&mut rest, 8)?;
    let len = u64::from_le_bytes(len.try_into().map_err(|_| Mismatch::Malformed)?);
    if u64::try_from(rest.len()).ok() != Some(len) || rest.is_empty() {
        return Err(Mismatch::Malformed);
    }
    if built != stamp {
        return Err(Mismatch::Engine {
            built: built.to_owned(),
        });
    }
    if source_hash != sha256(source.as_bytes()) {
        return Err(Mismatch::Source);
    }
    if digest != sha256(framed) {
        return Err(Mismatch::Corrupt);
    }
    Ok(Parsed { name, body: rest })
}

impl Bytecode {
    /// Compiles `source` to a self-describing bytecode blob for
    /// [`Bytecode::from_precompiled`], for a build script to embed.
    ///
    /// The blob records the engine and target of the build running this
    /// function (for a build script: the host), so it loads only into a
    /// program built for the same target with the same engine. Compiling
    /// doesn't run the script. Syntax errors are
    /// [`SandboxError::Exception`].
    pub fn precompile(source: &str, filename: &str) -> Result<Vec<u8>, SandboxError> {
        // A build script may run an unoptimised engine: generous limits.
        let limits = Limits::default()
            .with_memory_bytes(1 << 30)
            .with_time(Duration::from_secs(600))
            .with_stack_bytes(Limits::MAX_STACK_BYTES.min(8 << 20));
        let bytecode = std::thread::Builder::new()
            .name("surimono-precompile".into())
            .stack_size(32 << 20)
            .spawn({
                let source = source.to_owned();
                let filename = filename.to_owned();
                move || Sandbox::new(limits)?.compile(&source, &filename)
            })
            .map_err(|e| SandboxError::Engine {
                message: format!("could not start the compile thread: {e}"),
            })?
            .join()
            .map_err(|_| SandboxError::Engine {
                message: "the compile thread panicked".into(),
            })??;
        Ok(frame(&engine_stamp(), filename, source, bytecode.bytes()))
    }

    /// Bytecode from a blob [`Bytecode::precompile`] wrote, if it was built
    /// for this engine and target and from exactly `source`; `None`
    /// otherwise, and the caller compiles `source` instead.
    ///
    /// The blob stays where it is (typically `include_bytes!` data) and is
    /// never copied.
    ///
    /// # Safety
    ///
    /// `blob` must be the unmodified output of [`Bytecode::precompile`],
    /// produced by the caller's own build (a build script writing to
    /// `OUT_DIR`, embedded with `include_bytes!`). Never pass bytes that came
    /// from a file, the network or anyone else: QuickJS doesn't validate
    /// bytecode, and the checks here detect stale or damaged blobs, not
    /// forged ones.
    #[must_use]
    pub unsafe fn from_precompiled(blob: &'static [u8], source: &str) -> Option<Bytecode> {
        let parsed = check(blob, &engine_stamp(), source).ok()?;
        Some(Bytecode::from_parts_static(parsed.name, parsed.body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "globalThis.answer = 40 + 2; answer";

    fn leak(v: Vec<u8>) -> &'static [u8] {
        Box::leak(v.into_boxed_slice())
    }

    #[test]
    fn precompiled_bytecode_loads_and_runs() {
        let blob = leak(Bytecode::precompile(SRC, "answer.js").unwrap());
        // SAFETY: `blob` is the unmodified output of `precompile` in this
        // process.
        let bc = unsafe { Bytecode::from_precompiled(blob, SRC) }.unwrap();
        assert!(bc.is_precompiled());
        assert_eq!(bc.name(), "answer.js");
        let mut sb = Sandbox::new(Limits::default()).unwrap();
        let v: i32 = sb.run(|ctx| bc.eval(ctx)?.get()).unwrap();
        assert_eq!(v, 42);
    }

    #[test]
    fn precompiled_bytecode_is_what_compile_makes() {
        let blob = Bytecode::precompile(SRC, "answer.js").unwrap();
        let parsed = check(&blob, &engine_stamp(), SRC).unwrap();
        let compiled = Sandbox::new(Limits::default())
            .unwrap()
            .compile(SRC, "answer.js")
            .unwrap();
        assert!(!compiled.is_precompiled());
        assert_eq!(parsed.body, compiled.bytes());
    }

    #[test]
    fn syntax_errors_are_reported() {
        assert!(matches!(
            Bytecode::precompile("let = ;", "bad.js"),
            Err(SandboxError::Exception { .. })
        ));
    }

    #[test]
    fn other_source_is_rejected() {
        let blob = leak(Bytecode::precompile(SRC, "answer.js").unwrap());
        assert_eq!(
            check(blob, &engine_stamp(), "globalThis.answer = 41").err(),
            Some(Mismatch::Source)
        );
        // SAFETY: `blob` is the unmodified output of `precompile`.
        assert!(unsafe { Bytecode::from_precompiled(blob, "x") }.is_none());
    }

    #[test]
    fn other_engine_or_target_is_rejected() {
        let blob = Bytecode::precompile(SRC, "answer.js").unwrap();
        let other = engine_stamp().replace("patch level", "patch level 0");
        assert!(matches!(
            check(&blob, &other, SRC),
            Err(Mismatch::Engine { built }) if built == engine_stamp()
        ));
        // A blob built for another target, read here.
        let body = check(&blob, &engine_stamp(), SRC).unwrap().body.to_vec();
        let foreign = leak(frame(&other, "answer.js", SRC, &body));
        assert!(matches!(
            check(foreign, &engine_stamp(), SRC),
            Err(Mismatch::Engine { .. })
        ));
        // SAFETY: `foreign` is framed exactly as `precompile` frames; it is
        // rejected before its bytecode is read.
        assert!(unsafe { Bytecode::from_precompiled(foreign, SRC) }.is_none());
    }

    #[test]
    fn damaged_blobs_are_rejected_without_reading_the_bytecode() {
        let blob = Bytecode::precompile(SRC, "answer.js").unwrap();
        let stamp = engine_stamp();
        // Every truncation.
        for n in 0..blob.len() {
            assert!(check(&blob[..n], &stamp, SRC).is_err(), "truncated to {n}");
        }
        // A flipped bit anywhere.
        for i in 0..blob.len() {
            let mut bad = blob.clone();
            bad[i] ^= 0x10;
            assert!(check(&bad, &stamp, SRC).is_err(), "bit flipped at {i}");
        }
        // Trailing bytes.
        let mut long = blob.clone();
        long.push(0);
        assert!(check(&long, &stamp, SRC).is_err());
        assert_eq!(check(b"", &stamp, SRC).err(), Some(Mismatch::Malformed));
    }

    #[test]
    fn the_stamp_names_engine_and_target() {
        let s = engine_stamp();
        assert!(
            s.starts_with("quickjs-ng 0.16.2; surimono patch level "),
            "{s}"
        );
        assert!(s.contains(std::env::consts::ARCH), "{s}");
        assert!(s.ends_with(&format!("{}-bit", usize::BITS)), "{s}");
    }
}
