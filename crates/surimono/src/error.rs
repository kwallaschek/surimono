//! The one error type of the public API.

use std::fmt;

/// Which island an error belongs to.
///
/// `index` counts island markers in document order from 0; `id` and `renderer`
/// are the marker's `id` and `data-renderer` attributes, when it has them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct IslandRef {
    /// Position among the document's island markers, from 0.
    pub index: usize,
    /// The marker's `id` attribute, as written in the HTML.
    pub id: Option<String>,
    /// The marker's `data-renderer` attribute.
    pub renderer: Option<String>,
}

impl IslandRef {
    pub(crate) fn new(index: usize, id: Option<String>, renderer: Option<String>) -> Self {
        Self {
            index,
            id,
            renderer,
        }
    }
}

impl fmt::Display for IslandRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "island #{}", self.index + 1)?;
        match (&self.id, &self.renderer) {
            (Some(id), Some(r)) => write!(f, " (id \"{id}\", renderer \"{r}\")"),
            (Some(id), None) => write!(f, " (id \"{id}\")"),
            (None, Some(r)) => write!(f, " (renderer \"{r}\")"),
            (None, None) => Ok(()),
        }
    }
}

/// Everything that can go wrong in surimono.
///
/// Island errors carry an [`IslandRef`] saying which island failed. New variants
/// may be added in minor releases, so match with a wildcard arm.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The island marker is malformed: no `data-renderer`, a size that isn't a
    /// positive whole number, JSON that doesn't parse, or no closing `</script>`.
    #[error("{island}: invalid island marker: {reason}")]
    InvalidIsland {
        /// The island.
        island: IslandRef,
        /// What is wrong with it.
        reason: String,
    },
    /// The marker names a renderer that isn't enabled in the options.
    #[error("{island}: unknown renderer; enabled renderers: {}", list(enabled))]
    UnknownRenderer {
        /// The island.
        island: IslandRef,
        /// The renderers that are enabled.
        enabled: Vec<String>,
    },
    /// The island's script used more memory than the memory limit allows.
    #[error("{island}: the script exceeded the memory limit")]
    MemoryLimit {
        /// The island.
        island: IslandRef,
    },
    /// The island's script ran longer than the time limit allows.
    #[error("{island}: the script exceeded the time limit")]
    TimeLimit {
        /// The island.
        island: IslandRef,
    },
    /// The island's script recursed deeper than the stack limit allows.
    #[error("{island}: the script exceeded the stack limit")]
    StackOverflow {
        /// The island.
        island: IslandRef,
    },
    /// The island's script threw an exception.
    #[error("{island}: JavaScript error: {message}")]
    JavaScript {
        /// The island.
        island: IslandRef,
        /// The exception's message.
        message: String,
        /// The JavaScript stack trace, if the engine gave one.
        stack: Option<String>,
    },
    /// The renderer ran but its result can't be used, for example because it isn't SVG.
    #[error("{island}: render failed: {message}")]
    Render {
        /// The island.
        island: IslandRef,
        /// What went wrong.
        message: String,
    },
    /// A font passed in the options couldn't be loaded.
    #[error("font #{} could not be loaded: {reason}", index + 1)]
    Font {
        /// Position of the font in the options, from 0.
        index: usize,
        /// Why it couldn't be loaded.
        reason: String,
    },
    /// A bundle passed in the options can't be used: a bad name or entry, a
    /// name that's taken, or a script that doesn't compile.
    #[error("bundle \"{name}\" can't be used: {message}")]
    Bundle {
        /// The bundle's renderer name.
        name: String,
        /// What is wrong with it.
        message: String,
    },
    /// The options are invalid, for example a limit out of range or an enabled
    /// renderer that doesn't exist.
    #[error("invalid options: {0}")]
    InvalidOptions(String),
    /// The document couldn't be rewritten, for example because its markup is
    /// ambiguous enough that the islands' context can't be known for sure.
    #[error("could not rewrite the document: {0}")]
    Html(String),
}

impl Error {
    /// The island this error belongs to, if it belongs to one.
    pub fn island(&self) -> Option<&IslandRef> {
        match self {
            Error::InvalidIsland { island, .. }
            | Error::UnknownRenderer { island, .. }
            | Error::MemoryLimit { island }
            | Error::TimeLimit { island }
            | Error::StackOverflow { island }
            | Error::JavaScript { island, .. }
            | Error::Render { island, .. } => Some(island),
            Error::Font { .. }
            | Error::Bundle { .. }
            | Error::InvalidOptions(_)
            | Error::Html(_) => None,
        }
    }

    /// A short, stable, kebab-case name for the kind of error, used in placeholders
    /// (`data-error`) and by the bindings.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::InvalidIsland { .. } => "invalid-island",
            Error::UnknownRenderer { .. } => "unknown-renderer",
            Error::MemoryLimit { .. } => "memory-limit",
            Error::TimeLimit { .. } => "time-limit",
            Error::StackOverflow { .. } => "stack-overflow",
            Error::JavaScript { .. } => "javascript",
            Error::Render { .. } => "render",
            Error::Font { .. } => "font",
            Error::Bundle { .. } => "bundle",
            Error::InvalidOptions(_) => "invalid-options",
            Error::Html(_) => "html",
        }
    }
}

fn list(names: &[String]) -> String {
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn island_errors_name_the_island() {
        let e = Error::TimeLimit {
            island: IslandRef::new(2, Some("sales".into()), Some("echarts".into())),
        };
        assert_eq!(
            e.to_string(),
            "island #3 (id \"sales\", renderer \"echarts\"): the script exceeded the time limit"
        );
        assert_eq!(e.island().map(|i| i.index), Some(2));
        assert_eq!(e.kind(), "time-limit");
    }

    #[test]
    fn unknown_renderer_lists_the_enabled_ones() {
        let island = IslandRef::new(0, None, Some("plotly".into()));
        let e = Error::UnknownRenderer {
            island: island.clone(),
            enabled: vec!["echarts".into(), "vega".into()],
        };
        assert_eq!(
            e.to_string(),
            "island #1 (renderer \"plotly\"): unknown renderer; enabled renderers: echarts, vega"
        );
        let none = Error::UnknownRenderer {
            island,
            enabled: vec![],
        };
        assert!(none.to_string().ends_with("enabled renderers: none"));
    }

    #[test]
    fn error_is_send_sync_static() {
        fn check<T: Send + Sync + 'static>() {}
        check::<Error>();
    }
}
