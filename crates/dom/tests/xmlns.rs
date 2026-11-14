//! `xmlns` and the other namespaced attributes of SVG and MathML elements.
//!
//! HTML §13.2.6.1 "adjust foreign attributes" gives a foreign element's
//! `xmlns` the XMLNS namespace and no prefix, `xmlns:xlink` the prefix
//! `xmlns`, `xlink:href` the prefix `xlink` and `xml:lang` the prefix `xml`;
//! DOM §4.9.2 has an attribute's prefix null or a non-empty string. html5ever
//! 0.35 gives `xmlns` the prefix `Some("")`, which made its name `:xmlns`
//! (P2-DOM-13). The values for `xmlns` are also Chromium 153's; the XML
//! serializer's are DOM Parsing §3.2.1's, and the selectors' Selectors 4's
//! (an attribute selector without a namespace matches only attributes in no
//! namespace, as WPT's `ParentNode-querySelector-namespace-tagname-attribute`
//! has it for `xml:lang`).

mod common;

use std::path::Path;

use common::{limits, scaled};
use surimono_core::SettleOptions;
use surimono_dom::{Document, DomRuntime};

const SVG: &str = "<svg id=\"s\" xmlns=\"http://www.w3.org/2000/svg\" \
    xmlns:xlink=\"http://www.w3.org/1999/xlink\" xml:lang=\"en\"><use xlink:href=\"#a\"/></svg>\
    <math id=\"m\" xmlns=\"http://www.w3.org/1998/Math/MathML\"><mi>x</mi></math>";

fn runtime(body: &str) -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    let page = format!("<!doctype html><html><head></head><body>{body}</body></html>");
    rt.load(Document::parse(&page).unwrap());
    rt
}

fn run(body: &str, script: &str) -> String {
    runtime(body).eval(script).unwrap()
}

/// For the svg and math in `root`: each attribute's name, prefix, local
/// name and namespace, then what `getAttribute`, `hasAttribute` and
/// `getAttributeNS` find for `xmlns`.
const TABLE: &str = r#"
    const XMLNS = 'http://www.w3.org/2000/xmlns/';
    const table = (root) => {
      const rows = [];
      for (const e of [root.querySelector('svg'), root.querySelector('use'), root.querySelector('math')]) {
        for (const a of e.attributes) {
          if (a.name === 'id') continue;
          rows.push([e.localName, a.name, String(a.prefix), a.localName, String(a.namespaceURI)].join(' '));
        }
        if (e.localName !== 'use') {
          rows.push([e.localName, String(e.getAttribute('xmlns')), e.hasAttribute('xmlns'),
            String(e.getAttributeNS(XMLNS, 'xmlns')), e.getAttributeNames().join(',')].join(' '));
        }
      }
      return rows.join('\n');
    };"#;

/// What a script sees of the markup's foreign attributes, by HTML's table.
const EXPECTED: &str = "\
svg xmlns null xmlns http://www.w3.org/2000/xmlns/
svg xmlns:xlink xmlns xlink http://www.w3.org/2000/xmlns/
svg xml:lang xml lang http://www.w3.org/XML/1998/namespace
svg http://www.w3.org/2000/svg true http://www.w3.org/2000/svg id,xmlns,xmlns:xlink,xml:lang
use xlink:href xlink href http://www.w3.org/1999/xlink
math xmlns null xmlns http://www.w3.org/2000/xmlns/
math http://www.w3.org/1998/Math/MathML true http://www.w3.org/1998/Math/MathML id,xmlns";

#[test]
fn foreign_attributes_from_the_markup() {
    let got = run(SVG, &format!("{TABLE} table(document.body)"));
    assert_eq!(got, EXPECTED);
}

/// Every way markup becomes nodes goes through the same parser.
#[test]
fn foreign_attributes_from_every_parse() {
    for (what, script) in [
        (
            "innerHTML",
            "const d = document.createElement('div'); d.innerHTML = SVG_; table(d)",
        ),
        (
            "insertAdjacentHTML",
            "const d = document.createElement('div'); document.body.append(d); \
             d.insertAdjacentHTML('afterbegin', SVG_); table(d)",
        ),
        (
            "outerHTML",
            "const d = document.createElement('div'), p = document.createElement('p'); \
             d.append(p); p.outerHTML = SVG_; table(d)",
        ),
        (
            "createContextualFragment",
            "const r = document.createRange(); r.selectNodeContents(document.body); \
             const d = document.createElement('div'); d.append(r.createContextualFragment(SVG_)); table(d)",
        ),
        (
            "DOMParser",
            "table(new DOMParser().parseFromString(SVG_, 'text/html').body)",
        ),
        (
            "a template's contents",
            "const t = document.createElement('template'); t.innerHTML = SVG_; table(t.content)",
        ),
        (
            "cloneNode",
            "const d = document.createElement('div'); d.innerHTML = SVG_; table(d.cloneNode(true))",
        ),
    ] {
        let svg = serde_json_string(SVG);
        let got = run("", &format!("{TABLE} const SVG_ = {svg}; {script}"));
        assert_eq!(got, EXPECTED, "{what}");
    }
}

/// `setAttribute` makes an attribute in no namespace (DOM §4.9, "set an
/// attribute value"), whatever its name: KaTeX writes `xmlns` this way.
#[test]
fn set_attribute_makes_xmlns_in_no_namespace() {
    let got = run(
        "",
        r#"const out = [];
        for (const [ns, local] of [['http://www.w3.org/2000/svg', 'svg'], ['http://www.w3.org/1998/Math/MathML', 'math']]) {
          const e = document.createElementNS(ns, local);
          e.setAttribute('xmlns', ns);
          const a = e.attributes[0];
          out.push([a.name, String(a.prefix), a.localName, String(a.namespaceURI),
            e.getAttribute('xmlns'), e.hasAttribute('xmlns')].join(' '));
        }
        out.join('\n')"#,
    );
    assert_eq!(
        got,
        "xmlns null xmlns null http://www.w3.org/2000/svg true\n\
         xmlns null xmlns null http://www.w3.org/1998/Math/MathML true"
    );
}

/// DOM Parsing §3.2.1.1: a default namespace declared by the markup's `xmlns`
/// is written as `xmlns`, not taken for a prefix.
#[test]
fn the_xml_serializer_writes_xmlns_as_a_default_declaration() {
    let got = run(
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect/></svg>\
         <math xmlns=\"http://www.w3.org/1998/Math/MathML\"><mi>x</mi></math>\
         <svg id=\"x\" xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">\
         <use xlink:href=\"#a\"/></svg>",
        r#"const s = new XMLSerializer();
        [s.serializeToString(document.querySelector('svg')),
         s.serializeToString(document.querySelector('math')),
         s.serializeToString(document.getElementById('x'))].join('\n')"#,
    );
    assert_eq!(
        got,
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect/></svg>\n\
         <math xmlns=\"http://www.w3.org/1998/Math/MathML\"><mi>x</mi></math>\n\
         <svg id=\"x\" xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">\
         <use xlink:href=\"#a\"/></svg>"
    );
}

/// The HTML serialization (§13.3) writes them by namespace, with no colon
/// before `xmlns`.
#[test]
fn the_html_serialization_writes_xmlns_with_no_colon() {
    let mut rt = runtime(SVG);
    let () = rt.eval("undefined").unwrap();
    let html = rt.take().unwrap().to_html();
    assert!(
        html.contains(
            "<svg id=\"s\" xmlns=\"http://www.w3.org/2000/svg\" \
             xmlns:xlink=\"http://www.w3.org/1999/xlink\" xml:lang=\"en\"><use xlink:href=\"#a\"></use></svg>"
        ),
        "{html}"
    );
    assert!(
        html.contains("<math id=\"m\" xmlns=\"http://www.w3.org/1998/Math/MathML\">"),
        "{html}"
    );
    assert!(!html.contains(":xmlns"), "{html}");
}

/// Selectors 4: `[xmlns]` matches only an `xmlns` in no namespace (one
/// `setAttribute` made), `[*|xmlns]` any.
#[test]
fn attribute_selectors_see_the_namespace() {
    let got = run(
        SVG,
        r#"const e = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        e.id = 'set'; e.setAttribute('xmlns', 'http://www.w3.org/2000/svg');
        document.body.append(e);
        const ids = (sel) => [...document.querySelectorAll(sel)].map((n) => n.id).join(',');
        [ids('[xmlns]'), ids('[*|xmlns]'), ids('svg[*|lang]'), ids('[lang]'),
         document.getElementById('s').matches('[*|xmlns="http://www.w3.org/2000/svg"]')].join(' | ')"#,
    );
    assert_eq!(got, "set | s,m,set | s |  | true");
}

/// Alpine reads an attribute whose name starts with `:` as `x-bind`, so a
/// `:xmlns` was a binding to the expression `http://www.w3.org/2000/svg` and
/// threw. Alpine 3.17.5, as the real-world fixture vendors it.
#[test]
fn alpine_leaves_an_inline_svgs_xmlns_alone() {
    let lib = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/real-world/lib/alpinejs-3.17.5/cdn.min.js");
    if !lib.exists() {
        eprintln!("no Alpine bundle here, skipped");
        return;
    }
    let page = "<!doctype html><html><head><script defer src=\"alpine.js\"></script></head><body>\
        <div x-data=\"{ n: 3 }\"><svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10 10\">\
        <rect :width=\"n\" height=\"1\"/></svg><span x-text=\"n\"></span></div></body></html>";
    let mut rt = DomRuntime::new(
        limits()
            .with_time(scaled(std::time::Duration::from_secs(60)))
            .with_memory_bytes(512 << 20),
    )
    .unwrap();
    rt.load(Document::parse(page).unwrap());
    let report = rt
        .run_page(&SettleOptions::default(), |src| {
            (src == "alpine.js")
                .then(|| std::fs::read_to_string(&lib).ok())
                .flatten()
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(report.uncaught, 0, "{:?}", report.errors);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.missing_scripts.is_empty());
    let html = rt.with_document(Document::to_html).unwrap();
    assert!(
        html.contains("<rect :width=\"n\" height=\"1\" width=\"3\"></rect>"),
        "{html}"
    );
    assert!(html.contains("<span x-text=\"n\">3</span>"), "{html}");
}

/// `s` as a JavaScript string literal.
fn serde_json_string(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}
