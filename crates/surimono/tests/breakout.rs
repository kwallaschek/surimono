//! Island output can't reach outside its own box. Each case renders a page
//! with one island between page content, parses the facade's output with
//! html5ever (through blitz-html, the parser fulgur prints with), and checks
//! the DOM a browser or PDF engine builds from it:
//!
//! - the page keeps its shape: content after the island stays outside it;
//! - the page's `<html>` and `<body>` attributes are unchanged;
//! - no `<style>` element is left anywhere;
//! - the island's wrapper holds one SVG-namespace `<svg>` with only SVG
//!   elements below it, and no box-moving style on the root (or it is the
//!   error placeholder, which holds nothing).
//!
//! Cases: the reproducers from the #25 review (an ECharts emphasis colour
//! carrying page CSS with the default options and built-in bundle, and
//! custom bundles returning SVG that ends its wrapper or merges attributes
//! onto the page), plus near variants.

use blitz_dom::{BaseDocument, DocumentConfig, Node};
use blitz_html::HtmlDocument;
use surimono::{ErrorPolicy, Options, Renderer};

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const MARKER_TYPE: &str = "application/vnd.surimono+json";

fn font() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/fonts/NotoSans-Regular.ttf"
    );
    std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// A bundle that returns the island's `raw` SVG unchanged: whatever a
/// custom bundle (or a compromised one) could return.
const RAW_BUNDLE: &str = "function entry(specJson) { return JSON.parse(specJson).raw; }";

fn renderer(policy: ErrorPolicy) -> Renderer {
    Renderer::new(
        Options::new()
            .font(font())
            .bundle("raw", RAW_BUNDLE, "entry")
            .error_policy(policy),
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

/// A marker whose JSON writes `<` as `<`, as Rails' `to_json` does.
fn marker(renderer: &str, spec: &serde_json::Value) -> String {
    let json = spec.to_string().replace('<', "\\u003c");
    format!(
        "<script type=\"{MARKER_TYPE}\" data-renderer=\"{renderer}\" data-width=\"200\" \
         data-height=\"100\">{json}</script>"
    )
}

/// The page around one island marker.
fn page(island: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><title>p</title></head><body class=\"page\">\
         <p id=\"before\">a</p>{island}<p id=\"after\">b</p><p id=\"total\">c</p></body></html>"
    )
}

fn elements<'a>(doc: &'a BaseDocument, node: &Node) -> Vec<&'a Node> {
    node.children
        .iter()
        .filter_map(|&id| doc.get_node(id))
        .filter(|n| n.is_element())
        .collect()
}

fn name(node: &Node) -> (String, String) {
    node.element_data()
        .map(|e| (e.name.ns.to_string(), e.name.local.to_string()))
        .unwrap_or_default()
}

fn attrs(node: &Node) -> Vec<(String, String)> {
    node.element_data()
        .map(|e| {
            e.attrs()
                .iter()
                .map(|a| (a.name.local.to_string(), a.value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn attr(node: &Node, key: &str) -> Option<String> {
    attrs(node)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

/// Parses the facade's output and checks that the island stayed in its box.
fn contained(out: &str) -> Result<(), String> {
    let doc = HtmlDocument::from_html(out, DocumentConfig::default());
    let html = doc.root_element();
    if attrs(html) != [("lang".to_owned(), "en".to_owned())] {
        return Err(format!("<html> attributes changed: {:?}", attrs(html)));
    }
    let body = elements(&doc, html)
        .into_iter()
        .find(|n| name(n).1 == "body")
        .ok_or("no <body>")?;
    if attrs(body) != [("class".to_owned(), "page".to_owned())] {
        return Err(format!("<body> attributes changed: {:?}", attrs(body)));
    }
    let kids = elements(&doc, body);
    let shape: Vec<(String, String)> = kids
        .iter()
        .map(|n| (name(n).1, attr(n, "id").unwrap_or_default()))
        .collect();
    let want = [("p", "before"), ("div", ""), ("p", "after"), ("p", "total")];
    if shape.len() != want.len()
        || shape
            .iter()
            .zip(want)
            .any(|((n, i), (wn, wi))| n != wn || i != wi)
    {
        return Err(format!("the page's body changed shape: {shape:?}"));
    }
    let mut stack = vec![html];
    while let Some(n) = stack.pop() {
        if name(n).1.eq_ignore_ascii_case("style") {
            return Err("a <style> element is in the document".into());
        }
        stack.extend(elements(&doc, n));
    }
    let island = kids[1];
    let class = attr(island, "class").unwrap_or_default();
    if !class.split(' ').any(|c| c == "surimono-island") {
        return Err(format!("the second body child isn't the island: {class:?}"));
    }
    let inside = elements(&doc, island);
    if class.split(' ').any(|c| c == "surimono-island-error") {
        // The placeholder: an empty div with a comment.
        return if inside.is_empty() {
            Ok(())
        } else {
            Err("the error placeholder holds elements".into())
        };
    }
    if inside.len() != 1 || name(inside[0]) != (SVG_NS.to_owned(), "svg".to_owned()) {
        return Err(format!(
            "the wrapper holds {:?}, not one <svg>",
            inside.iter().map(|n| name(n)).collect::<Vec<_>>()
        ));
    }
    let style = attr(inside[0], "style")
        .unwrap_or_default()
        .to_ascii_lowercase();
    for bad in [
        "position",
        "transform",
        "z-index",
        "float",
        "display",
        "margin",
    ] {
        if style.contains(bad) {
            return Err(format!("the root <svg> style has {bad}: {style}"));
        }
    }
    let mut stack = inside;
    while let Some(n) = stack.pop() {
        let (ns, local) = name(n);
        if ns != SVG_NS {
            return Err(format!("<{local}> in namespace {ns} inside the island"));
        }
        stack.extend(elements(&doc, n));
    }
    Ok(())
}

/// Renders the page around `island` with both error policies. Fail may
/// refuse the island (an error is safe); Placeholder always gives a page,
/// and every page given must keep the island in its box.
fn assert_contained(case: &str, island: &str) {
    let html = page(island);
    for policy in [ErrorPolicy::Fail, ErrorPolicy::Placeholder] {
        match renderer(policy).render_islands(&html) {
            Ok(out) => {
                if let Err(why) = contained(&out) {
                    panic!("{case} ({policy:?}): {why}\n{out}");
                }
            }
            Err(e) if policy == ErrorPolicy::Fail => {
                assert_eq!(e.island().map(|i| i.index), Some(0), "{case}: {e}");
            }
            Err(e) => panic!("{case} ({policy:?}): {e}"),
        }
    }
}

/// The checker itself catches the breakouts it is meant to catch: the raw
/// hostile markup spliced in without the facade fails it.
#[test]
fn the_check_catches_raw_breakouts() {
    for (case, raw) in hostile_svgs() {
        let unsafe_page = page(&format!("<div class=\"surimono-island\">{raw}</div>"));
        assert!(
            contained(&unsafe_page).is_err(),
            "{case}: the raw markup should fail the check"
        );
    }
    // Escaped markup is only text, even raw.
    let text = page(&format!(
        "<div class=\"surimono-island\">{}</div>",
        escaped_markup_in_title()
    ));
    assert_eq!(contained(&text), Ok(()));
    let fine = page(&format!(
        "<div class=\"surimono-island\"><svg xmlns=\"{SVG_NS}\"><rect/></svg></div>"
    ));
    assert_eq!(contained(&fine), Ok(()));
}

/// SVG a custom bundle could return that, inlined as it is, breaks out of
/// its wrapper or changes the page (#25 review, finding 1 b to e).
fn hostile_svgs() -> Vec<(&'static str, String)> {
    let x = "http://www.w3.org/1999/xhtml";
    vec![
        (
            "page css in <style>",
            format!(
                "<svg xmlns=\"{SVG_NS}\"><style>body{{display:none}} #total{{visibility:hidden}}</style><rect/></svg>"
            ),
        ),
        (
            "<style> with several attributes",
            format!(
                "<svg xmlns=\"{SVG_NS}\"><style type=\"text/css\" media=\"all\" id=\"s\">body{{display:none}}</style></svg>"
            ),
        ),
        (
            "<textarea/> in desc swallows the page",
            format!("<svg xmlns=\"{SVG_NS}\"><desc><textarea/></desc><rect/></svg>"),
        ),
        (
            "<select/> in title swallows the page",
            format!("<svg xmlns=\"{SVG_NS}\"><title><select/></title><rect/></svg>"),
        ),
        (
            "self-closed xhtml div in foreignObject takes the page in",
            format!(
                "<svg xmlns=\"{SVG_NS}\"><foreignObject><div xmlns=\"{x}\"/></foreignObject><rect/></svg>"
            ),
        ),
        (
            "xhtml body in foreignObject merges onto the page body",
            format!(
                "<svg xmlns=\"{SVG_NS}\"><foreignObject><body xmlns=\"{x}\" class=\"pwn\" style=\"display:none\"/></foreignObject></svg>"
            ),
        ),
        (
            "html in desc merges onto the page html",
            format!("<svg xmlns=\"{SVG_NS}\"><desc><html class=\"x\"/></desc></svg>"),
        ),
        (
            "lowercase foreignobject",
            format!(
                "<svg xmlns=\"{SVG_NS}\"><foreignobject><html class=\"y\"/></foreignobject></svg>"
            ),
        ),
        (
            "upper-case FOREIGNOBJECT",
            format!(
                "<svg xmlns=\"{SVG_NS}\"><FOREIGNOBJECT><body xmlns=\"{x}\" class=\"z\"/></FOREIGNOBJECT></svg>"
            ),
        ),
        (
            "link, form and style in foreignObject",
            format!(
                "<svg xmlns=\"{SVG_NS}\"><foreignObject><a xmlns=\"{x}\" href=\"https://example.com/\">x</a>\
                 <form xmlns=\"{x}\"/><style xmlns=\"{x}\">p{{display:none}}</style></foreignObject></svg>"
            ),
        ),
        (
            "xhtml element straight inside svg",
            format!("<svg xmlns=\"{SVG_NS}\"><p xmlns=\"{x}\">x</p><rect/></svg>"),
        ),
        (
            "prefixed root reads as html",
            "<svg:svg xmlns:svg=\"http://www.w3.org/2000/svg\"><svg:style>body{display:none}</svg:style>\
             <svg:rect/></svg:svg>"
                .to_owned(),
        ),
        (
            "fixed overlay on the root",
            format!(
                "<svg xmlns=\"{SVG_NS}\" style=\"position:fixed;inset:0;background:red\"><rect/></svg>"
            ),
        ),
    ]
}

/// Markup written as text in a title: it must stay text after sanitising.
fn escaped_markup_in_title() -> String {
    format!(
        "<svg xmlns=\"{SVG_NS}\"><title>&lt;/title&gt;&lt;/svg&gt;&lt;style&gt;body{{display:none}}&lt;/style&gt;</title><rect/></svg>"
    )
}

#[test]
fn custom_bundle_svg_stays_in_its_box() {
    for (case, raw) in hostile_svgs() {
        assert_contained(case, &marker("raw", &serde_json::json!({ "raw": raw })));
    }
    assert_contained(
        "escaped markup in title",
        &marker(
            "raw",
            &serde_json::json!({ "raw": escaped_markup_in_title() }),
        ),
    );
}

/// The built-in ECharts with default options: a data-driven emphasis colour
/// used to land in an island `<style>` that styled the whole page
/// (`body{display:none}` hid it, `#total{…}` reached the page's own element).
#[cfg(feature = "echarts")]
#[test]
fn echarts_emphasis_colour_cannot_style_the_page() {
    let spec = serde_json::json!({
        "xAxis": { "type": "category", "data": ["Q1", "Q2", "Q3"] },
        "yAxis": { "type": "value" },
        "series": [{
            "type": "bar",
            "data": [3, 5, 2],
            "emphasis": { "itemStyle": {
                "color": "red;} body{display:none} #total{visibility:hidden} .x{fill:red"
            } }
        }]
    });
    let html = page(&marker("echarts", &spec));
    let r = Renderer::new(Options::new().font(font())).unwrap_or_else(|e| panic!("{e}"));
    let out = r.render_islands(&html).unwrap_or_else(|e| panic!("{e}"));
    if let Err(why) = contained(&out) {
        panic!("{why}\n{out}");
    }
    // The chart itself is still drawn.
    assert!(out.contains("<path"), "{out}");
}
