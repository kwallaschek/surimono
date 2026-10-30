//! The HTML pre-pass: finds island markers and replaces each with its SVG.
//!
//! An island marker is a script element whose `type` is
//! `application/vnd.surimono+json`:
//!
//! ```html
//! <script type="application/vnd.surimono+json" id="sales" class="chart"
//!         data-renderer="echarts" data-width="600" data-height="400">
//!   { "xAxis": { "data": ["Q1", "Q2"] }, "series": [{ "type": "bar", "data": [3, 5] }] }
//! </script>
//! ```
//!
//! It becomes a `div` that keeps the marker's `id` and `class` and holds the SVG:
//!
//! ```html
//! <div class="surimono-island chart" id="sales" data-renderer="echarts"><svg …>…</svg></div>
//! ```
//!
//! Everything outside the markers stays byte for byte as it was. The document is
//! tokenized with lol_html, a streaming rewriter built on the HTML tokenizer, so
//! comments, `<textarea>`, foreign content and the like are read the way a browser
//! reads them. Markers are cut out during the rewrite and their places marked with
//! a sentinel that can't occur in the input; the islands are rendered afterwards,
//! in document order, and spliced in.

use std::cell::RefCell;
use std::rc::Rc;

use lol_html::errors::RewritingError;
use lol_html::html_content::{ContentType, Element, TextChunk};
use lol_html::{RewriteStrSettings, element, rewrite_str, text};

use crate::error::{Error, IslandRef};

/// The `type` of an island marker's script element.
pub(crate) const ISLAND_TYPE: &str = "application/vnd.surimono+json";
/// Width in CSS pixels when `data-width` is missing.
pub(crate) const DEFAULT_WIDTH: u32 = 600;
/// Height in CSS pixels when `data-height` is missing.
pub(crate) const DEFAULT_HEIGHT: u32 = 400;
/// Largest accepted `data-width` or `data-height`, in CSS pixels.
pub(crate) const MAX_SIZE: u32 = 16_384;

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";

/// What happens when an island can't be rendered: its marker is malformed, its
/// renderer is unknown, or its script fails or hits a limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ErrorPolicy {
    /// Fail the whole render with the island's [`Error`]. The default.
    #[default]
    Fail,
    /// Render the rest of the document and leave an empty placeholder where the
    /// island was: `<div class="surimono-island surimono-island-error …"
    /// data-error="time-limit"><!-- surimono: … --></div>`, keeping the marker's
    /// `id` and `class`. The comment holds the error message; style
    /// `.surimono-island-error` to make failures visible on the page.
    Placeholder,
}

/// An island marker, read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Island {
    /// Which island this is, for errors.
    pub(crate) at: IslandRef,
    /// The `data-renderer` attribute.
    pub(crate) renderer: String,
    /// `data-width` in CSS pixels, or [`DEFAULT_WIDTH`].
    pub(crate) width: u32,
    /// `data-height` in CSS pixels, or [`DEFAULT_HEIGHT`].
    pub(crate) height: u32,
    /// The script's text, checked to be JSON.
    pub(crate) spec_json: String,
}

/// A marker as cut out of the document, before it is checked.
#[derive(Debug, Default)]
struct RawMarker {
    renderer: Option<String>,
    width: Option<String>,
    height: Option<String>,
    id: Option<String>,
    class: Option<String>,
    text: String,
    closed: bool,
}

#[derive(Debug, Default)]
struct Scan {
    markers: Vec<RawMarker>,
    /// The marker whose script text is being read, if any.
    current: Option<usize>,
}

/// Replaces every island marker in `html` with the SVG `render` returns for it.
///
/// `render` is called once per well-formed marker, in document order. Errors
/// (malformed markers and whatever `render` returns) follow `policy`.
pub(crate) fn rewrite_islands<F>(
    html: &str,
    policy: ErrorPolicy,
    mut render: F,
) -> Result<String, Error>
where
    F: FnMut(&Island) -> Result<String, Error>,
{
    let sentinel = Sentinel::for_input(html);
    let scan = Rc::new(RefCell::new(Scan::default()));

    let rewritten = {
        let on_script = {
            let scan = Rc::clone(&scan);
            let sentinel = &sentinel;
            move |el: &mut Element<'_, '_>| {
                let mut s = scan.borrow_mut();
                if !is_island(el) {
                    s.current = None;
                    return Ok(());
                }
                let index = s.markers.len();
                s.markers.push(RawMarker {
                    renderer: el.get_attribute("data-renderer"),
                    width: el.get_attribute("data-width"),
                    height: el.get_attribute("data-height"),
                    id: el.get_attribute("id"),
                    class: el.get_attribute("class"),
                    ..RawMarker::default()
                });
                s.current = Some(index);
                el.before(&sentinel.mark(index), ContentType::Html);
                el.remove_and_keep_content();
                let closed = Rc::clone(&scan);
                // A script element always has an end tag (implied at end of input
                // otherwise), so lol_html gives us the handler list.
                if let Some(handlers) = el.end_tag_handlers() {
                    handlers.push(Box::new(move |_end| {
                        let mut s = closed.borrow_mut();
                        if let Some(m) = s.markers.get_mut(index) {
                            m.closed = true;
                        }
                        s.current = None;
                        Ok(())
                    }));
                }
                Ok(())
            }
        };
        let on_text = {
            let scan = Rc::clone(&scan);
            move |t: &mut TextChunk<'_>| {
                let mut s = scan.borrow_mut();
                if let Some(i) = s.current {
                    if let Some(m) = s.markers.get_mut(i) {
                        m.text.push_str(t.as_str());
                    }
                    t.remove();
                }
                Ok(())
            }
        };
        let settings = RewriteStrSettings {
            element_content_handlers: vec![element!("script", on_script), text!("script", on_text)],
            // Plain HTML: `<esi:…>` tags are ordinary unknown elements.
            enable_esi_tags: false,
            ..RewriteStrSettings::new()
        };
        rewrite_str(html, settings).map_err(|e| match e {
            RewritingError::ParsingAmbiguity(e) => Error::Html(e.to_string()),
            other => Error::Html(other.to_string()),
        })?
    };

    let markers = std::mem::take(&mut scan.borrow_mut().markers);
    if markers.is_empty() {
        return Ok(rewritten);
    }
    let mut replacements = Vec::with_capacity(markers.len());
    for (index, raw) in markers.into_iter().enumerate() {
        let outcome = check(index, &raw).and_then(|island| render(&island));
        match (outcome, policy) {
            (Ok(svg), _) => replacements.push(wrapper(&raw, &svg)),
            (Err(e), ErrorPolicy::Placeholder) => replacements.push(placeholder(&raw, &e)),
            (Err(e), ErrorPolicy::Fail) => return Err(e),
        }
    }
    Ok(sentinel.splice(&rewritten, &replacements))
}

fn is_island(el: &Element<'_, '_>) -> bool {
    el.namespace_uri() == HTML_NAMESPACE
        && el.get_attribute("type").is_some_and(|t| {
            let essence = t.split(';').next().unwrap_or_default().trim();
            essence.eq_ignore_ascii_case(ISLAND_TYPE)
        })
}

/// Reads a cut-out marker into an [`Island`], or says what is wrong with it.
fn check(index: usize, raw: &RawMarker) -> Result<Island, Error> {
    let renderer = raw.renderer.as_deref().map(str::trim).unwrap_or_default();
    let at = IslandRef::new(
        index,
        raw.id.clone(),
        (!renderer.is_empty()).then(|| renderer.to_owned()),
    );
    let invalid = |reason: String| Error::InvalidIsland {
        island: at.clone(),
        reason,
    };
    if !raw.closed {
        return Err(invalid(
            "the script element has no closing </script>".into(),
        ));
    }
    if renderer.is_empty() {
        return Err(invalid("data-renderer is missing or empty".into()));
    }
    let width = size("data-width", raw.width.as_deref(), DEFAULT_WIDTH).map_err(invalid)?;
    let height = size("data-height", raw.height.as_deref(), DEFAULT_HEIGHT).map_err(invalid)?;
    let json = raw.text.trim();
    if let Err(e) = serde_json::from_str::<serde::de::IgnoredAny>(json) {
        return Err(invalid(format!("the script's text is not valid JSON: {e}")));
    }
    Ok(Island {
        at: at.clone(),
        renderer: renderer.to_owned(),
        width,
        height,
        spec_json: json.to_owned(),
    })
}

fn size(attr: &str, value: Option<&str>, default: u32) -> Result<u32, String> {
    let Some(value) = value else {
        return Ok(default);
    };
    let v = value.trim();
    let parsed = if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) {
        v.parse::<u32>().ok()
    } else {
        None
    };
    match parsed {
        Some(n) if (1..=MAX_SIZE).contains(&n) => Ok(n),
        _ => Err(format!(
            "{attr} must be a whole number of CSS pixels from 1 to {MAX_SIZE}, got {value:?}"
        )),
    }
}

/// The element an island's SVG goes into.
fn wrapper(raw: &RawMarker, svg: &str) -> String {
    format!(
        "<div class=\"{}\"{}{}>{svg}</div>",
        classes("surimono-island", raw.class.as_deref()),
        attr("id", raw.id.as_deref()),
        attr("data-renderer", raw.renderer.as_deref().map(str::trim)),
    )
}

/// What a failed island leaves behind under [`ErrorPolicy::Placeholder`].
fn placeholder(raw: &RawMarker, error: &Error) -> String {
    format!(
        "<div class=\"{}\"{}{}{}><!-- surimono: {} --></div>",
        classes(
            "surimono-island surimono-island-error",
            raw.class.as_deref()
        ),
        attr("id", raw.id.as_deref()),
        attr("data-renderer", raw.renderer.as_deref().map(str::trim)),
        attr("data-error", Some(error.kind())),
        comment_text(&error.to_string()),
    )
}

fn classes(ours: &str, theirs: Option<&str>) -> String {
    match theirs.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => format!("{ours} {}", quote_safe(c)),
        None => ours.to_owned(),
    }
}

/// ` name="value"`, or nothing. lol_html hands attribute values over as written
/// (character references intact), so only `"` needs escaping to sit inside double
/// quotes.
fn attr(name: &str, value: Option<&str>) -> String {
    match value.filter(|v| !v.is_empty()) {
        Some(v) => format!(" {name}=\"{}\"", quote_safe(v)),
        None => String::new(),
    }
}

fn quote_safe(value: &str) -> String {
    value.replace('"', "&quot;")
}

/// Text that is safe inside `<!-- … -->`: no `--`, no `<!--`.
fn comment_text(message: &str) -> String {
    let mut s = message.replace("<!--", "< !--");
    while s.contains("--") {
        s = s.replace("--", "- -");
    }
    s
}

/// Marks where each island goes in the rewritten document.
///
/// The prefix is built from Unicode noncharacters and grown until it doesn't
/// occur in the input, so every occurrence in the output is one of ours.
#[derive(Debug)]
struct Sentinel {
    prefix: String,
}

impl Sentinel {
    const END: char = ';';

    fn for_input(html: &str) -> Self {
        let mut prefix = String::from("\u{FFFF}surimono-island\u{FFFE}");
        while html.contains(prefix.as_str()) {
            prefix.insert(0, '\u{FFFF}');
        }
        Self { prefix }
    }

    fn mark(&self, index: usize) -> String {
        format!("{}{index}{}", self.prefix, Self::END)
    }

    fn splice(&self, rewritten: &str, replacements: &[String]) -> String {
        let extra: usize = replacements.iter().map(String::len).sum();
        let mut out = String::with_capacity(rewritten.len() + extra);
        let mut rest = rewritten;
        while let Some(pos) = rest.find(self.prefix.as_str()) {
            out.push_str(&rest[..pos]);
            let after = &rest[pos + self.prefix.len()..];
            let replacement = after.find(Self::END).and_then(|end| {
                let index: usize = after[..end].parse().ok()?;
                Some((replacements.get(index)?, end))
            });
            match replacement {
                Some((html, end)) => {
                    out.push_str(html);
                    rest = &after[end + Self::END.len_utf8()..];
                }
                // Can't happen: the prefix doesn't occur in the input and every
                // mark we wrote has an index below replacements.len().
                None => {
                    out.push_str(&self.prefix);
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "application/vnd.surimono+json";

    /// Renders every island as a tiny SVG naming its renderer, size and spec.
    fn fake(island: &Island) -> Result<String, Error> {
        Ok(format!(
            "<svg data-r=\"{}\" width=\"{}\" height=\"{}\">{}</svg>",
            island.renderer, island.width, island.height, island.spec_json
        ))
    }

    fn run(html: &str) -> String {
        match rewrite_islands(html, ErrorPolicy::Fail, fake) {
            Ok(out) => out,
            Err(e) => panic!("rewrite failed: {e}"),
        }
    }

    fn run_err(html: &str) -> Error {
        match rewrite_islands(html, ErrorPolicy::Fail, fake) {
            Ok(out) => panic!("expected an error, got {out}"),
            Err(e) => e,
        }
    }

    #[test]
    fn document_without_islands_is_unchanged() {
        let html = "<!doctype html>\r\n<html><head><title>a &amp; b</title>\
                    <script>var x = '<script type=\"application/vnd.surimono+json\">';</script>\
                    </head><body CLASS=x data-a='1' data-b=2><p>Hi<br/>there</p>\
                    <!-- <script type=\"application/vnd.surimono+json\"> --></body></html>";
        assert_eq!(run(html), html);
    }

    #[test]
    fn replaces_one_island_and_keeps_everything_else() {
        let html = format!(
            "<p>before</p>\n<script type=\"{T}\" data-renderer=\"echarts\" data-width=\"300\" \
             data-height=\"200\">{{\"a\": 1}}</script>\n<p>after</p>"
        );
        assert_eq!(
            run(&html),
            "<p>before</p>\n<div class=\"surimono-island\" data-renderer=\"echarts\">\
             <svg data-r=\"echarts\" width=\"300\" height=\"200\">{\"a\": 1}</svg></div>\n\
             <p>after</p>"
        );
    }

    #[test]
    fn default_size_is_600_by_400() {
        let out = run(&format!(
            "<script type={T} data-renderer=vega>{{}}</script>"
        ));
        assert!(out.contains("width=\"600\" height=\"400\""), "{out}");
    }

    #[test]
    fn keeps_id_and_class() {
        let out = run(&format!(
            "<script id=\"sales\" class=\"chart wide\" type=\"{T}\" data-renderer=\"echarts\">[]</script>"
        ));
        assert_eq!(
            out,
            "<div class=\"surimono-island chart wide\" id=\"sales\" data-renderer=\"echarts\">\
             <svg data-r=\"echarts\" width=\"600\" height=\"400\">[]</svg></div>"
        );
    }

    #[test]
    fn many_islands_in_order_inside_nested_markup() {
        let mut html = String::from("<main><section><div class=\"grid\">");
        for i in 0..25 {
            html.push_str(&format!(
                "<div class=\"cell\"><h2>Chart {i}</h2><script type=\"{T}\" data-renderer=\"r{i}\">\
                 {{\"n\":{i}}}</script></div>"
            ));
        }
        html.push_str("</div></section></main>");
        let mut seen = Vec::new();
        let out = rewrite_islands(&html, ErrorPolicy::Fail, |island| {
            seen.push(island.at.index);
            fake(island)
        })
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(seen, (0..25).collect::<Vec<_>>());
        for i in 0..25 {
            let want = format!(
                "<div class=\"cell\"><h2>Chart {i}</h2><div class=\"surimono-island\" \
                 data-renderer=\"r{i}\"><svg data-r=\"r{i}\" width=\"600\" height=\"400\">\
                 {{\"n\":{i}}}</svg></div></div>"
            );
            assert!(out.contains(&want), "island {i} missing in {out}");
        }
        assert!(out.starts_with("<main><section><div class=\"grid\">"));
        assert!(out.ends_with("</div></section></main>"));
        assert!(!out.contains("<script"));
    }

    #[test]
    fn attributes_in_any_order_and_quoting() {
        for marker in [
            format!(
                "<script data-height='50' data-width=70 data-renderer=\"x\" type='{T}'>1</script>"
            ),
            format!(
                "<SCRIPT\n  TYPE=\"{}\"\tDATA-RENDERER=x\r\n data-width=\" 70 \" data-height=50 >1</SCRIPT>",
                T.to_uppercase()
            ),
            format!(
                "<script type=\"{T}; charset=utf-8\" data-renderer=x data-width=70 data-height=50>1</script>"
            ),
        ] {
            assert_eq!(
                run(&marker),
                "<div class=\"surimono-island\" data-renderer=\"x\"><svg data-r=\"x\" width=\"70\" \
                 height=\"50\">1</svg></div>",
                "marker: {marker}"
            );
        }
    }

    #[test]
    fn single_quoted_values_with_double_quotes_are_escaped() {
        let out = run(&format!(
            "<script type={T} data-renderer=x id='a\"b' class='c&amp;d'>0</script>"
        ));
        assert!(
            out.starts_with(
                "<div class=\"surimono-island c&amp;d\" id=\"a&quot;b\" data-renderer=\"x\">"
            ),
            "{out}"
        );
    }

    #[test]
    fn other_scripts_are_left_alone() {
        let html = format!(
            "<script>if (a < b && c > d) {{ document.write('</p>'); }}</script>\
             <script type=\"application/json\" data-renderer=\"echarts\">{{}}</script>\
             <script type=\"module\">import x from './x.js';</script>\
             <script type=\"{T}\" data-renderer=\"echarts\">{{}}</script>\
             <script src=\"a.js\"></script>"
        );
        let out = run(&html);
        assert_eq!(
            out,
            "<script>if (a < b && c > d) { document.write('</p>'); }</script>\
             <script type=\"application/json\" data-renderer=\"echarts\">{}</script>\
             <script type=\"module\">import x from './x.js';</script>\
             <div class=\"surimono-island\" data-renderer=\"echarts\"><svg data-r=\"echarts\" \
             width=\"600\" height=\"400\">{}</svg></div>\
             <script src=\"a.js\"></script>"
        );
    }

    #[test]
    fn script_end_tag_inside_json_string_must_be_escaped() {
        // In HTML a script ends at the first `</script`, so a JSON string holding
        // one is written `<\/script>`; JSON reads `\/` as `/`.
        let out = run(&format!(
            "<script type={T} data-renderer=x>{{\"label\":\"<\\/script><b>\"}}</script><i>after</i>"
        ));
        assert_eq!(
            out,
            "<div class=\"surimono-island\" data-renderer=\"x\"><svg data-r=\"x\" width=\"600\" \
             height=\"400\">{\"label\":\"<\\/script><b>\"}</svg></div><i>after</i>"
        );
        // Unescaped, the script ends early and the rest isn't JSON.
        let e = run_err(&format!(
            "<script type={T} data-renderer=x>{{\"label\":\"</script>\"}}</script>"
        ));
        assert!(matches!(e, Error::InvalidIsland { .. }), "{e}");
    }

    #[test]
    fn cdata_and_comment_like_text_is_just_json() {
        let json = "{\"t\":\"<![CDATA[ x ]]> <!-- y --> & &amp; <b>\"}";
        let out = run(&format!("<script type={T} data-renderer=x>{json}</script>"));
        assert!(out.contains(&format!(">{json}</svg>")), "{out}");
    }

    #[test]
    fn island_text_is_raw_not_entity_decoded() {
        let mut got = String::new();
        let html = format!("<script type={T} data-renderer=x>\"&lt;&amp;\"</script>");
        rewrite_islands(&html, ErrorPolicy::Fail, |i| {
            got = i.spec_json.clone();
            Ok(String::new())
        })
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(got, "\"&lt;&amp;\"");
    }

    #[test]
    fn crlf_and_bom_are_kept() {
        let html = format!(
            "\u{feff}<html>\r\n<body>\r\n<script type=\"{T}\" data-renderer=\"x\">\r\n{{\"a\":\r\n1}}\r\n</script>\r\n</body>\r\n</html>\r\n"
        );
        let out = run(&html);
        assert_eq!(
            out,
            "\u{feff}<html>\r\n<body>\r\n<div class=\"surimono-island\" data-renderer=\"x\">\
             <svg data-r=\"x\" width=\"600\" height=\"400\">{\"a\":\r\n1}</svg></div>\r\n\
             </body>\r\n</html>\r\n"
        );
    }

    #[test]
    fn utf8_text_around_and_inside_islands() {
        let html = format!(
            "<p>売上 €</p><script type={T} data-renderer=x>{{\"label\":\"四半期\"}}</script><p>終わり</p>"
        );
        let out = run(&html);
        assert_eq!(
            out,
            "<p>売上 €</p><div class=\"surimono-island\" data-renderer=\"x\"><svg data-r=\"x\" \
             width=\"600\" height=\"400\">{\"label\":\"四半期\"}</svg></div><p>終わり</p>"
        );
    }

    #[test]
    fn markers_in_comments_textarea_and_svg_are_not_islands() {
        let html = format!(
            "<!-- <script type=\"{T}\" data-renderer=x>{{}}</script> -->\
             <textarea><script type=\"{T}\" data-renderer=x>{{}}</script></textarea>\
             <svg><script type=\"{T}\" data-renderer=x>{{}}</script></svg>"
        );
        assert_eq!(run(&html), html);
    }

    #[test]
    fn input_containing_the_sentinel_text_is_safe() {
        let tricky = "\u{FFFF}surimono-island\u{FFFE}0;";
        let html = format!("<p>{tricky}</p><script type={T} data-renderer=x>1</script>");
        let out = run(&html);
        assert_eq!(
            out,
            format!(
                "<p>{tricky}</p><div class=\"surimono-island\" data-renderer=\"x\"><svg \
                 data-r=\"x\" width=\"600\" height=\"400\">1</svg></div>"
            )
        );
    }

    #[test]
    fn malformed_markers_are_invalid_islands() {
        let cases = [
            (format!("<script type={T}>{{}}</script>"), "data-renderer"),
            (
                format!("<script type={T} data-renderer=\"  \">{{}}</script>"),
                "data-renderer",
            ),
            (
                format!("<script type={T} data-renderer=x data-width=0>{{}}</script>"),
                "data-width",
            ),
            (
                format!("<script type={T} data-renderer=x data-width=12px>{{}}</script>"),
                "data-width",
            ),
            (
                format!("<script type={T} data-renderer=x data-height=-5>{{}}</script>"),
                "data-height",
            ),
            (
                format!("<script type={T} data-renderer=x data-height=99999>{{}}</script>"),
                "data-height",
            ),
            (
                format!("<script type={T} data-renderer=x>{{oops}}</script>"),
                "not valid JSON",
            ),
            (
                format!("<script type={T} data-renderer=x></script>"),
                "not valid JSON",
            ),
            (
                format!("<p>x</p><script type={T} data-renderer=x>{{}}"),
                "closing </script>",
            ),
        ];
        for (html, needle) in cases {
            let e = run_err(&html);
            let Error::InvalidIsland { island, reason } = &e else {
                panic!("{html}: expected InvalidIsland, got {e:?}");
            };
            assert_eq!(island.index, 0);
            assert!(reason.contains(needle), "{html}: {reason}");
        }
    }

    #[test]
    fn fail_policy_stops_at_the_first_error_and_names_the_island() {
        let html = format!(
            "<script type={T} data-renderer=ok>1</script>\
             <script type={T} id=\"second\" data-renderer=bad>2</script>\
             <script type={T} data-renderer=ok>3</script>"
        );
        let mut calls = 0;
        let e = match rewrite_islands(&html, ErrorPolicy::Fail, |island| {
            calls += 1;
            if island.renderer == "bad" {
                Err(Error::TimeLimit {
                    island: island.at.clone(),
                })
            } else {
                fake(island)
            }
        }) {
            Ok(out) => panic!("expected an error, got {out}"),
            Err(e) => e,
        };
        assert_eq!(calls, 2);
        assert_eq!(
            e.to_string(),
            "island #2 (id \"second\", renderer \"bad\"): the script exceeded the time limit"
        );
    }

    #[test]
    fn placeholder_policy_renders_the_rest() {
        let html = format!(
            "<p>a</p><script type={T} id=s class=\"chart\" data-renderer=bad>1</script>\
             <script type={T} data-renderer=ok>2</script><script type={T} data-renderer=ok>{{x</script>"
        );
        let out = rewrite_islands(&html, ErrorPolicy::Placeholder, |island| {
            if island.renderer == "bad" {
                Err(Error::JavaScript {
                    island: island.at.clone(),
                    message: "x is not defined -- see <!-- here -->".into(),
                    stack: None,
                })
            } else {
                fake(island)
            }
        })
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            out,
            "<p>a</p><div class=\"surimono-island surimono-island-error chart\" id=\"s\" \
             data-renderer=\"bad\" data-error=\"javascript\"><!-- surimono: island #1 (id \"s\", \
             renderer \"bad\"): JavaScript error: x is not defined - - see < !- - here - -> --></div>\
             <div class=\"surimono-island\" data-renderer=\"ok\"><svg data-r=\"ok\" width=\"600\" \
             height=\"400\">2</svg></div>\
             <div class=\"surimono-island surimono-island-error\" data-renderer=\"ok\" \
             data-error=\"invalid-island\"><!-- surimono: island #3 (renderer \"ok\"): invalid \
             island marker: the script's text is not valid JSON: key must be a string at line 1 \
             column 2 --></div>"
        );
    }

    #[test]
    fn comment_text_never_closes_the_comment() {
        for m in ["-->", "--->", "a--b", "<!--", "---", "--!>"] {
            let c = comment_text(m);
            assert!(!c.contains("--"), "{m:?} -> {c:?}");
        }
    }

    #[test]
    fn rendering_twice_gives_the_same_bytes() {
        let html = format!(
            "<div><script type={T} data-renderer=x data-width=10>{{\"a\":[1,2]}}</script></div>"
        );
        assert_eq!(run(&html), run(&html));
    }
}
