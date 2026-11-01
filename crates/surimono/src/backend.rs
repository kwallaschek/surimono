//! What a [`Renderer`](crate::Renderer) runs islands with: the island runner
//! from `surimono-islands`, with text measured by `surimono-gfx` from the
//! caller's fonts. This module converts between the facade's types and theirs,
//! so neither crate's types appear in the public API.

use surimono_gfx::FontSet;
use surimono_islands::{IslandError, IslandRunner, RunnerOptions, SandboxError};

use crate::error::{Error, IslandRef};
use crate::options::{Limits, Options};

/// The fonts from the options, in order. A font that can't be read is
/// [`Error::Font`] with its position.
pub(crate) fn font_set(fonts: Vec<Vec<u8>>) -> Result<FontSet, Error> {
    let mut set = FontSet::new();
    for (index, bytes) in fonts.into_iter().enumerate() {
        set.add_font_bytes(bytes).map_err(|e| Error::Font {
            index,
            reason: e.to_string(),
        })?;
    }
    Ok(set)
}

/// The sandbox's limits for the facade's.
#[allow(
    clippy::field_reassign_with_default,
    reason = "field by field keeps compiling if the type becomes #[non_exhaustive]"
)]
fn core_limits(l: Limits) -> surimono_core::Limits {
    let mut core = surimono_core::Limits::default();
    core.memory_bytes = l.memory_bytes;
    core.time = l.time;
    core.stack_bytes = l.stack_bytes;
    core
}

/// Starts the runner: worker pool, limits, seed, text measurement. Bundles are
/// registered separately.
#[allow(
    clippy::field_reassign_with_default,
    reason = "field by field keeps compiling if the type becomes #[non_exhaustive]"
)]
pub(crate) fn runner(
    options: &Options,
    gfx: &surimono_gfx::TextMeasurer,
) -> Result<IslandRunner, Error> {
    let mut o = RunnerOptions::default();
    o.limits = core_limits(options.limits);
    // The facade decides the default; it rejects an explicit 0 before this.
    o.pool_size = options
        .pool_size
        .unwrap_or_else(crate::options::default_pool_size);
    o.seed = options.seed;
    o.now_ms = options.now_ms.unwrap_or(crate::options::DEFAULT_NOW_MS);
    o.utc_offset_minutes = options.utc_offset_minutes;
    // Always the caller's fonts: renderers never get an estimate.
    o.text_measurer = Some(gfx.clone());
    IslandRunner::new(o).map_err(|e| match e {
        IslandError::InvalidOptions { message } => Error::InvalidOptions(message),
        other => Error::InvalidOptions(other.to_string()),
    })
}

/// Names of the renderers compiled into this build, sorted, by the same
/// Cargo features as `surimono_islands`' own list (a test keeps the two
/// equal). A plain constant, with no lazy initialisation: a lock or `Once`
/// here could be inherited mid-initialisation by a forked child and hang
/// its first `Renderer::new`.
const NAMES: &[&str] = &[
    #[cfg(feature = "echarts")]
    "echarts",
    #[cfg(feature = "vega")]
    "vega",
    #[cfg(feature = "vega")]
    "vega-lite",
];

/// [`NAMES`]: the renderers compiled into this build, sorted.
pub(crate) fn builtin_names() -> &'static [&'static str] {
    NAMES
}

/// [`builtin_names`] as owned strings, to extend with registered bundles.
pub(crate) fn builtin_renderers() -> Vec<String> {
    builtin_names().iter().map(|&n| n.to_owned()).collect()
}

/// Checks a bundle's name and entry before it is compiled.
pub(crate) fn check_bundle(name: &str, entry: &str, taken: &[String]) -> Result<(), String> {
    let ident = |s: &str| {
        s.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
            && s.chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    };
    if name.trim().is_empty() || name != name.trim() {
        return Err(format!(
            "bundle name {name:?} must be non-empty without surrounding spaces"
        ));
    }
    if taken.iter().any(|n| n == name) {
        return Err(format!("a renderer named {name:?} already exists"));
    }
    if !ident(entry) {
        return Err(format!("entry {entry:?} is not a JavaScript function name"));
    }
    Ok(())
}

/// The public error for an island the runner failed on.
pub(crate) fn island_error(e: IslandError, island: IslandRef, enabled: &[String]) -> Error {
    match e {
        IslandError::Sandbox(SandboxError::MemoryLimit) => Error::MemoryLimit { island },
        IslandError::Sandbox(SandboxError::TimeLimit) => Error::TimeLimit { island },
        IslandError::Sandbox(SandboxError::StackOverflow) => Error::StackOverflow { island },
        IslandError::Sandbox(SandboxError::Exception { message, stack }) => Error::JavaScript {
            island,
            message,
            stack,
        },
        IslandError::UnknownRenderer { .. } => Error::UnknownRenderer {
            island,
            enabled: enabled.to_vec(),
        },
        IslandError::BadJson {
            message,
            line,
            column,
        } => Error::InvalidIsland {
            island,
            reason: format!(
                "the script's text is not valid JSON: {message} at line {line} column {column}"
            ),
        },
        // A wrong return type, an engine or worker failure, and anything added
        // later: the island can't be rendered, and the message says why.
        other => Error::Render {
            island,
            message: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The facade's constant list matches the islands crate's built-ins.
    #[test]
    fn builtin_names_match_the_islands_crate() {
        assert_eq!(
            builtin_names(),
            surimono_islands::builtin_renderers().as_slice()
        );
        assert!(NAMES.windows(2).all(|w| w[0] < w[1]), "{NAMES:?}");
    }

    #[test]
    fn every_island_error_maps_to_a_public_error() {
        let at = IslandRef::new(0, None, Some("r".into()));
        let cases = [
            (SandboxError::MemoryLimit.into(), "memory-limit"),
            (SandboxError::TimeLimit.into(), "time-limit"),
            (SandboxError::StackOverflow.into(), "stack-overflow"),
            (
                SandboxError::Exception {
                    message: "m".into(),
                    stack: None,
                }
                .into(),
                "javascript",
            ),
            (
                SandboxError::Engine {
                    message: "e".into(),
                }
                .into(),
                "render",
            ),
            (
                IslandError::UnknownRenderer { name: "r".into() },
                "unknown-renderer",
            ),
            (
                IslandError::BadJson {
                    message: "b".into(),
                    line: 1,
                    column: 2,
                },
                "invalid-island",
            ),
            (
                IslandError::NoFonts {
                    renderer: "r".into(),
                },
                "render",
            ),
            (
                IslandError::NotAString {
                    renderer: "r".into(),
                    found: "number".into(),
                },
                "render",
            ),
            (
                IslandError::Worker {
                    message: "w".into(),
                },
                "render",
            ),
        ];
        for (e, kind) in cases {
            assert_eq!(island_error(e, at.clone(), &[]).kind(), kind);
        }
    }

    #[test]
    fn bundle_names_and_entries_are_checked() {
        let taken = ["echarts".to_owned()];
        assert_eq!(check_bundle("mine", "render", &taken), Ok(()));
        assert_eq!(check_bundle("mine", "$draw_2", &taken), Ok(()));
        for (name, entry) in [
            ("", "f"),
            (" x", "f"),
            ("echarts", "f"),
            ("x", "not a name"),
            ("x", "1f"),
            ("x", ""),
        ] {
            assert!(
                check_bundle(name, entry, &taken).is_err(),
                "{name:?}/{entry:?}"
            );
        }
    }

    #[test]
    fn fonts_must_be_fonts() {
        assert!(matches!(
            font_set(vec![b"not a font".to_vec()]),
            Err(Error::Font { index: 0, .. })
        ));
        assert!(font_set(Vec::new()).is_ok());
    }
}
