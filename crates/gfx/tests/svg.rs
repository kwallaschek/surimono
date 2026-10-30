//! sanitize_svg on real chart output and on hostile SVG.
//!
//! The chart SVGs in `tests/data/svg/` are real output of ECharts 5.6.0 and
//! 6.1.0 (SSR SVG) and Vega-Lite 5.23.0 / Vega 5.33.1, made by
//! `tests/data/svg/generate.js`.

mod common;

use roxmltree::{Document, Node};
use surimono_gfx::{
    FontSet, GenericFamily, GfxError, SvgOptions, TextMeasurer, TextMode, sanitize_svg,
};

const ECHARTS: &str = include_str!("data/svg/echarts-5.6.0-bar-line.svg");
const ECHARTS_RICH: &str = include_str!("data/svg/echarts-6.1.0-rich.svg");
const VEGA: &str = include_str!("data/svg/vega-lite-5.23.0-bar.svg");

/// Sanitizes with text kept, so the sanitising itself can be compared byte
/// for byte with the input.
fn keep(svg: &str, island: usize, fonts: &FontSet) -> Result<String, GfxError> {
    let options = SvgOptions::new().with_text(TextMode::Keep);
    sanitize_svg(svg, island, &TextMeasurer::new(fonts.clone()), &options)
}
const STACK: &str = "'Noto Sans', 'Noto Sans JP'";

#[test]
fn echarts_output_changes_only_ids_fonts_and_cdata() {
    let fonts = common::test_fonts();
    let out = keep(ECHARTS, 2, &fonts).unwrap();
    Document::parse(&out).expect("output is well-formed XML");

    assert!(out.contains(r#"id="sm2-zr0-g0""#));
    assert!(out.contains(r#"fill="url(#sm2-zr0-g0)""#));
    assert!(out.contains(r#"clip-path="url(#sm2-zr0-c0)""#));
    assert!(out.contains(&format!("font-family:{STACK};")));
    assert!(!out.contains("sans-serif"));
    assert!(!out.contains("CDATA"));

    // Undo exactly those rewrites: what is left must equal the input byte
    // for byte (ECharts's input has no XML prolog to drop).
    let undone = out.replace("sm2-", "").replace(STACK, "sans-serif");
    let expected = ECHARTS
        .trim_end()
        .replace("<![CDATA[", "")
        .replace("]]>", "");
    assert_eq!(undone, expected);
}

#[test]
fn vega_output_changes_only_fonts() {
    let fonts = common::test_fonts();
    let out = keep(VEGA, 0, &fonts).unwrap();
    Document::parse(&out).expect("output is well-formed XML");
    let attr = format!("font-family=\"{STACK}\"");
    assert_eq!(out.matches(&attr).count(), 16);
    assert_eq!(
        out.replace(&attr, "font-family=\"sans-serif\""),
        VEGA.trim_end()
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
        let out = keep(&svg, 0, &fonts).unwrap();
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
fn font_shorthand_and_style_sheets() {
    let fonts = common::test_fonts();
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><style>text{font:bold 12px/1.2 Arial !important} .t{font-family:monospace}</style><text style='font: italic 10px serif; fill: red'>a</text></svg>"#;
    let out = keep(svg, 0, &fonts).unwrap();
    assert_eq!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><style>text{font:bold 12px/1.2 "Noto Sans", "Noto Sans JP" !important} .t{font-family:"Noto Sans", "Noto Sans JP"}</style><text style='font: italic 10px "Noto Sans", "Noto Sans JP"; fill: red'>a</text></svg>"#
    );
}

#[test]
fn deterministic_and_unique_per_island() {
    let fonts = common::test_fonts();
    let a1 = keep(ECHARTS, 1, &fonts).unwrap();
    let a2 = keep(ECHARTS, 1, &fonts).unwrap();
    let b = keep(ECHARTS, 7, &fonts).unwrap();
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
];

#[test]
fn hostile_svg_is_neutralised() {
    let fonts = common::test_fonts();
    let measurer = TextMeasurer::new(fonts.clone());
    let mut rejected = 0;
    let mut cleaned = Vec::new();
    for input in HOSTILE {
        let out = match keep(input, 0, &fonts) {
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
        // Outlining rebuilds text elements from the sanitized source: still safe.
        let outlined = sanitize_svg(input, 0, &measurer, &SvgOptions::new()).unwrap();
        let odoc =
            Document::parse(&outlined).unwrap_or_else(|e| panic!("{input} -> {outlined}: {e}"));
        check_safe(odoc.root_element(), false, input, &outlined);
        assert!(
            !outlined.to_ascii_lowercase().contains("evil.example"),
            "{outlined}"
        );
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
    // Only the malformed comment is rejected; everything else is cleaned.
    assert_eq!(rejected, 1);
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
    let err = keep("<html/>", 0, &fonts).unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid SVG: the root element is <html>, not <svg>"
    );
}

/// Every id the charts emit (ECharts: `zr0-c*` clip paths, `zr0-g*`
/// gradients, `zr0-p*` decal patterns, `zr0-s*` shadow filters) gets the
/// island prefix, and every reference points at a prefixed id that exists,
/// with text kept and with text outlined.
#[test]
fn every_id_and_reference_carries_the_island_prefix() {
    let fonts = common::test_fonts();
    let measurer = TextMeasurer::new(fonts.clone());
    let refs = |s: &str| -> Vec<String> {
        let mut out = Vec::new();
        for (open, close) in [("url(#", ')'), ("href=\"#", '"')] {
            for (k, _) in s.match_indices(open) {
                let rest = &s[k + open.len()..];
                out.push(rest[..rest.find(close).unwrap()].to_string());
            }
        }
        out
    };
    for (name, svg) in [
        ("echarts 5", ECHARTS),
        ("echarts 6", ECHARTS_RICH),
        ("vega", VEGA),
    ] {
        for options in [
            SvgOptions::new().with_text(TextMode::Keep),
            SvgOptions::new(),
        ] {
            let out = sanitize_svg(svg, 5, &measurer, &options).unwrap();
            let doc = Document::parse(&out).unwrap();
            let ids: Vec<&str> = doc
                .descendants()
                .filter_map(|n| n.attribute("id"))
                .collect();
            for id in &ids {
                assert!(id.starts_with("sm5-"), "{name}: id {id}");
            }
            for r in refs(&out) {
                assert!(
                    ids.contains(&r.as_str()),
                    "{name}: reference to #{r}, ids {ids:?}"
                );
            }
            for class in doc.descendants().filter_map(|n| n.attribute("class")) {
                for c in class.split_ascii_whitespace() {
                    assert!(!c.starts_with("zr"), "{name}: class {c}");
                }
            }
            assert!(!out.contains(".zr0"), "{name}: unprefixed ECharts selector");
        }
    }
    // The rich chart really has all four kinds, all still referenced.
    let out = sanitize_svg(ECHARTS_RICH, 5, &measurer, &SvgOptions::new()).unwrap();
    for id in ["sm5-zr0-c0", "sm5-zr0-g0", "sm5-zr0-p0", "sm5-zr0-s0"] {
        assert!(out.contains(&format!("id=\"{id}\"")), "{id}");
        assert!(out.contains(&format!("url(#{id})")), "{id}");
    }
}

/// With the default options, real chart output keeps no `<text>`: every
/// label becomes outlines in the registered fonts, with its string kept as
/// the group's `aria-label`. The result is deterministic.
#[test]
fn real_charts_print_as_outlines() {
    let fonts = common::test_fonts();
    let measurer = TextMeasurer::new(fonts.clone());
    for (name, svg, labels) in [
        ("echarts 5", ECHARTS, &["売上 Sales", "Mon", "水", "40"][..]),
        (
            "echarts 6",
            ECHARTS_RICH,
            &["月別売上 Monthly", "単位: 百万円", "1月", "Apr", "金額"][..],
        ),
        ("vega", VEGA, &["月別 Revenue", "日本", "A"][..]),
    ] {
        let originals = Document::parse(svg)
            .unwrap()
            .descendants()
            .filter(|n| n.has_tag_name("text"))
            .count();
        assert!(originals > 5, "{name}");
        let out = sanitize_svg(svg, 1, &measurer, &SvgOptions::new()).unwrap();
        let doc = Document::parse(&out).expect("output is well-formed XML");
        let left: Vec<_> = doc
            .descendants()
            .filter(|n| n.is_element())
            .filter(|n| ["text", "tspan", "textPath"].contains(&n.tag_name().name()))
            .collect();
        assert!(left.is_empty(), "{name}: {} text elements left", left.len());
        // The groups that replace a <text> (Vega labels other groups too).
        let groups: Vec<&str> = doc
            .descendants()
            .filter(|n| n.attribute("role") == Some("img"))
            .filter_map(|n| n.attribute("aria-label"))
            .collect();
        assert_eq!(
            groups.len(),
            originals,
            "{name}: one labelled group per <text>"
        );
        for label in labels {
            assert!(
                groups.contains(label),
                "{name}: no group labelled {label:?} in {groups:?}"
            );
        }
        // Every group holds outlines (no label is empty in these charts).
        for g in doc
            .descendants()
            .filter(|n| n.attribute("role") == Some("img"))
        {
            assert!(
                g.descendants()
                    .any(|p| p.has_tag_name("path")
                        && p.attribute("d").is_some_and(|d| d.len() > 20)),
                "{name}: {:?} has no outline",
                g.attribute("aria-label")
            );
        }
        // Same input, same output: twice in one measurer and in a fresh one.
        assert_eq!(
            out,
            sanitize_svg(svg, 1, &measurer, &SvgOptions::new()).unwrap()
        );
        let fresh = TextMeasurer::new(common::test_fonts());
        assert_eq!(
            out,
            sanitize_svg(svg, 1, &fresh, &SvgOptions::new()).unwrap()
        );
        // For the Chrome comparison (tests/chrome/svg_outline.mjs).
        let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("outlined");
        std::fs::create_dir_all(&dir).unwrap();
        let file = name.replace(' ', "-");
        std::fs::write(dir.join(format!("{file}.svg")), &out).unwrap();
        std::fs::write(
            dir.join(format!("{file}.orig.svg")),
            keep(svg, 1, &fonts).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn outlining_text_needs_fonts() {
    let none = TextMeasurer::new(FontSet::new());
    let with_text = r#"<svg xmlns="http://www.w3.org/2000/svg"><text>a</text></svg>"#;
    assert_eq!(
        sanitize_svg(with_text, 0, &none, &SvgOptions::new()),
        Err(GfxError::NoFonts)
    );
    let options = SvgOptions::new().with_text(TextMode::Keep);
    assert!(sanitize_svg(with_text, 0, &none, &options).is_ok());
    let without = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>"#;
    assert_eq!(
        sanitize_svg(without, 0, &none, &SvgOptions::new()).unwrap(),
        without
    );
}
