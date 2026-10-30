//! What a [`Renderer`](crate::Renderer) runs islands with.
//!
//! STAND-IN until the island runner (P1-RT-03, `surimono_islands::IslandRunner`)
//! and text metrics (P1-GFX-01, `surimono_gfx::{FontSet, TextMeasurer}`) are
//! merged. The types and signatures here mirror those contracts, so swapping in
//! the real ones touches this file only; everything public stays as it is.
//!
//! Until then the runner knows a single renderer, `surimono-stub`, which runs no
//! JavaScript: it returns an empty SVG of the island's size. Its spec may hold
//! `"busy_ms": n` (works for n milliseconds, or fails with the time limit when n is
//! over it), `"throw": "message"` (fails like a script exception) and
//! `"limit": "memory"` or `"stack"` (fails like a script over that limit), so the
//! facade, the gem and the CLI can be tested end to end. It disappears with the
//! stand-in. Bundles are accepted and checked but can't run yet.

use std::time::Duration;

use crate::error::{Error, IslandRef};
use crate::options::Limits;

/// The temporary renderer the stand-in runner provides.
pub(crate) const STUB_RENDERER: &str = "surimono-stub";

const MIN_MEMORY_BYTES: usize = 1024 * 1024;
const MIN_STACK_BYTES: usize = 64 * 1024;
const MAX_STACK_BYTES: usize = 16 * 1024 * 1024;

/// Font files for text measurement (mirrors `surimono_gfx::FontSet`).
#[derive(Debug, Default)]
pub(crate) struct FontSet {
    files: Vec<Vec<u8>>,
}

impl FontSet {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Adds a font file; returns how many faces it holds.
    pub(crate) fn add_font_bytes(&mut self, bytes: Vec<u8>) -> Result<usize, String> {
        let magic = bytes.get(..4).unwrap_or_default();
        let known: [&[u8]; 4] = [b"\x00\x01\x00\x00", b"OTTO", b"true", b"ttcf"];
        if !known.contains(&magic) {
            return Err("not a TrueType or OpenType font, nor a font collection".into());
        }
        self.files.push(bytes);
        Ok(1)
    }
}

/// Answers `measureText` from the fonts (mirrors `surimono_gfx::TextMeasurer`).
#[derive(Debug)]
pub(crate) struct TextMeasurer {
    #[expect(dead_code, reason = "read by the real island runner")]
    fonts: FontSet,
}

impl TextMeasurer {
    pub(crate) fn new(fonts: FontSet) -> Self {
        Self { fonts }
    }
}

/// Mirrors `surimono_islands::RunnerOptions`.
#[derive(Debug)]
pub(crate) struct RunnerOptions {
    pub(crate) limits: Limits,
    pub(crate) pool_size: usize,
    pub(crate) seed: u64,
    pub(crate) measurer: TextMeasurer,
}

/// Mirrors `surimono_islands::IslandRequest`.
#[derive(Debug)]
pub(crate) struct IslandRequest<'a> {
    pub(crate) renderer: &'a str,
    pub(crate) spec_json: &'a str,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// Mirrors `surimono_islands::IslandError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IslandError {
    UnknownRenderer,
    MemoryLimit,
    TimeLimit,
    StackOverflow,
    Exception {
        message: String,
        stack: Option<String>,
    },
    /// The script returned something that isn't an SVG string.
    BadOutput(String),
    InvalidBundle(String),
    InvalidOptions(String),
}

impl std::fmt::Display for IslandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownRenderer => f.write_str("unknown renderer"),
            Self::MemoryLimit => f.write_str("the script exceeded the memory limit"),
            Self::TimeLimit => f.write_str("the script exceeded the time limit"),
            Self::StackOverflow => f.write_str("the script exceeded the stack limit"),
            Self::Exception { message, .. } => write!(f, "JavaScript error: {message}"),
            Self::BadOutput(m) | Self::InvalidBundle(m) | Self::InvalidOptions(m) => f.write_str(m),
        }
    }
}

#[derive(Debug)]
struct Registered {
    name: String,
}

/// Mirrors `surimono_islands::IslandRunner` (Send + Sync).
#[derive(Debug)]
pub(crate) struct IslandRunner {
    limits: Limits,
    bundles: Vec<Registered>,
    _measurer: TextMeasurer,
}

impl IslandRunner {
    pub(crate) fn new(options: RunnerOptions) -> Result<Self, IslandError> {
        let l = options.limits;
        if l.memory_bytes < MIN_MEMORY_BYTES {
            return Err(IslandError::InvalidOptions(format!(
                "limits.memory_bytes is {}, the minimum is {MIN_MEMORY_BYTES}",
                l.memory_bytes
            )));
        }
        if !(MIN_STACK_BYTES..=MAX_STACK_BYTES).contains(&l.stack_bytes) {
            return Err(IslandError::InvalidOptions(format!(
                "limits.stack_bytes is {}, it must be between {MIN_STACK_BYTES} and {MAX_STACK_BYTES}",
                l.stack_bytes
            )));
        }
        if options.pool_size == 0 {
            return Err(IslandError::InvalidOptions(
                "pool_size must be at least 1".into(),
            ));
        }
        let _ = options.seed;
        Ok(Self {
            limits: l,
            bundles: Vec::new(),
            _measurer: options.measurer,
        })
    }

    pub(crate) fn register_bundle(
        &mut self,
        name: &str,
        js_source: &str,
        entry: &str,
    ) -> Result<(), IslandError> {
        let ident = |s: &str| {
            s.chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
                && s.chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        };
        if name.trim().is_empty() || name != name.trim() {
            return Err(IslandError::InvalidBundle(format!(
                "bundle name {name:?} must be non-empty without surrounding spaces"
            )));
        }
        if self.renderer_names().iter().any(|n| n == name) {
            return Err(IslandError::InvalidBundle(format!(
                "a renderer named {name:?} already exists"
            )));
        }
        if !ident(entry) {
            return Err(IslandError::InvalidBundle(format!(
                "entry {entry:?} is not a JavaScript function name"
            )));
        }
        if js_source.trim().is_empty() {
            return Err(IslandError::InvalidBundle(format!(
                "bundle {name:?} has no source"
            )));
        }
        self.bundles.push(Registered {
            name: name.to_owned(),
        });
        Ok(())
    }

    /// Every renderer islands can name: the built-ins, then bundles in the order added.
    pub(crate) fn renderer_names(&self) -> Vec<String> {
        std::iter::once(STUB_RENDERER.to_owned())
            .chain(self.bundles.iter().map(|b| b.name.clone()))
            .collect()
    }

    pub(crate) fn render(&self, req: &IslandRequest<'_>) -> Result<String, IslandError> {
        if req.renderer != STUB_RENDERER {
            return if self.bundles.iter().any(|b| b.name == req.renderer) {
                Err(IslandError::BadOutput(
                    "JavaScript bundles can't run yet: the island runner (P1-RT-03) isn't merged"
                        .into(),
                ))
            } else {
                Err(IslandError::UnknownRenderer)
            };
        }
        let spec: serde_json::Value = serde_json::from_str(req.spec_json)
            .map_err(|e| IslandError::BadOutput(format!("spec is not JSON: {e}")))?;
        if let Some(message) = spec.get("throw").and_then(|v| v.as_str()) {
            return Err(IslandError::Exception {
                message: format!("Error: {message}"),
                stack: Some("    at render (surimono-stub)".into()),
            });
        }
        match spec.get("limit").and_then(|v| v.as_str()) {
            Some("memory") => return Err(IslandError::MemoryLimit),
            Some("stack") => return Err(IslandError::StackOverflow),
            _ => {}
        }
        if let Some(ms) = spec.get("busy_ms").and_then(serde_json::Value::as_u64) {
            let busy = Duration::from_millis(ms);
            if busy > self.limits.time {
                std::thread::sleep(self.limits.time);
                return Err(IslandError::TimeLimit);
            }
            std::thread::sleep(busy);
        }
        Ok(format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" \
             viewBox=\"0 0 {w} {h}\"></svg>",
            w = req.width,
            h = req.height
        ))
    }
}

/// The public error for an island the runner failed on.
pub(crate) fn island_error(e: IslandError, island: IslandRef, enabled: &[String]) -> Error {
    match e {
        IslandError::UnknownRenderer => Error::UnknownRenderer {
            island,
            enabled: enabled.to_vec(),
        },
        IslandError::MemoryLimit => Error::MemoryLimit { island },
        IslandError::TimeLimit => Error::TimeLimit { island },
        IslandError::StackOverflow => Error::StackOverflow { island },
        IslandError::Exception { message, stack } => Error::JavaScript {
            island,
            message,
            stack,
        },
        IslandError::BadOutput(message)
        | IslandError::InvalidBundle(message)
        | IslandError::InvalidOptions(message) => Error::Render { island, message },
    }
}

/// Default runtime pool size: the number of CPUs, from 1 to 8.
pub(crate) fn default_pool_size() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
        .clamp(1, 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<IslandRunner>();
    }

    #[test]
    fn fonts_must_look_like_fonts() {
        let mut fonts = FontSet::new();
        assert_eq!(
            fonts.add_font_bytes(b"\x00\x01\x00\x00rest".to_vec()),
            Ok(1)
        );
        assert_eq!(fonts.add_font_bytes(b"OTTOrest".to_vec()), Ok(1));
        assert!(fonts.add_font_bytes(b"<html>".to_vec()).is_err());
        assert!(fonts.add_font_bytes(Vec::new()).is_err());
    }

    #[test]
    fn every_island_error_maps_to_a_public_error() {
        let at = IslandRef::new(0, None, Some("r".into()));
        let cases = [
            (IslandError::UnknownRenderer, "unknown-renderer"),
            (IslandError::MemoryLimit, "memory-limit"),
            (IslandError::TimeLimit, "time-limit"),
            (IslandError::StackOverflow, "stack-overflow"),
            (
                IslandError::Exception {
                    message: "m".into(),
                    stack: None,
                },
                "javascript",
            ),
            (IslandError::BadOutput("b".into()), "render"),
        ];
        for (e, kind) in cases {
            assert_eq!(island_error(e, at.clone(), &[]).kind(), kind);
        }
    }

    #[test]
    fn default_pool_is_between_1_and_8() {
        assert!((1..=8).contains(&default_pool_size()));
    }
}
