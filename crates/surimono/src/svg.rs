//! SVG on its way from a renderer into the document.
//!
//! A renderer is a JavaScript bundle, possibly the caller's own, possibly
//! fed hostile data, so its output is untrusted. [`SanitizedSvg`] is the only
//! thing the pre-pass will put into a document, and the only way to make one
//! is [`SanitizedSvg::new`], which runs `surimono_gfx::sanitize_svg`: one well
//! formed `<svg>` element, no scripts, event handlers, external references or
//! HTML that could end the wrapper, ids prefixed per island, and font families
//! mapped to the registered fonts.

use surimono_gfx::FontSet;

use crate::error::{Error, IslandRef};

/// SVG that went through `surimono_gfx::sanitize_svg` and is safe to inline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SanitizedSvg(String);

impl SanitizedSvg {
    /// Sanitises a renderer's output for the island `at`. SVG that can't be
    /// made safe (not XML, no `<svg>` root, too large or too deep) is an
    /// [`Error::Render`] for that island.
    pub(crate) fn new(svg: &str, at: &IslandRef, fonts: &FontSet) -> Result<Self, Error> {
        surimono_gfx::sanitize_svg(svg, at.index, fonts)
            .map(Self)
            .map_err(|e| Error::Render {
                island: at.clone(),
                message: format!("the renderer's SVG can't be used: {e}"),
            })
    }

    /// Wraps a string without sanitising it, for the pre-pass's own tests.
    #[cfg(test)]
    pub(crate) fn trusted_for_tests(svg: impl Into<String>) -> Self {
        Self(svg.into())
    }

    /// The SVG markup.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> IslandRef {
        IslandRef::new(2, Some("q3".into()), Some("mine".into()))
    }

    #[test]
    fn scripts_and_handlers_are_removed_and_ids_prefixed() {
        let svg = SanitizedSvg::new(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" onload=\"alert(1)\">\
             <script>alert(2)</script><rect id=\"r\" width=\"1\" height=\"1\"/>\
             <use href=\"#r\"/></svg>",
            &at(),
            &FontSet::new(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let s = svg.as_str();
        assert!(!s.contains("script") && !s.contains("onload"), "{s}");
        assert!(
            s.contains("id=\"sm2-r\"") && s.contains("href=\"#sm2-r\""),
            "{s}"
        );
    }

    #[test]
    fn markup_that_would_break_out_of_the_wrapper_is_an_error() {
        for hostile in [
            "<svg></svg></div><script>alert(1)</script><textarea>",
            "</div><svg></svg>",
            "<p>not svg</p>",
            "",
        ] {
            match SanitizedSvg::new(hostile, &at(), &FontSet::new()) {
                Err(Error::Render { island, message }) => {
                    assert_eq!(island.index, 2);
                    assert!(message.contains("can't be used"), "{message}");
                }
                other => panic!("{hostile:?}: expected Error::Render, got {other:?}"),
            }
        }
    }
}
