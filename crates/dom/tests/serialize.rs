//! `Document::to_html` against the HTML Standard's fragment serialization
//! algorithm (https://html.spec.whatwg.org/#serialising-html-fragments,
//! §13.3). Every expected string is what that algorithm gives for the tree
//! the HTML parser builds (§13.2) with scripting enabled, since scripts run
//! on this tree.

use surimono_dom::Document;

/// The serialization of `<body>`'s contents for `body`.
fn body_html(body: &str) -> String {
    let html = Document::parse(&format!(
        "<!doctype html><html><head></head><body>{body}</body></html>"
    ))
    .unwrap()
    .to_html();
    let start = html.find("<body>").expect("a body") + "<body>".len();
    let end = html.rfind("</body>").expect("a body end tag");
    html[start..end].to_owned()
}

#[test]
fn attributes_in_xml_xmlns_and_xlink_namespaces_keep_their_prefix() {
    // §13.2.6.5 "adjust foreign attributes" puts these in the XLink, XML
    // and XMLNS namespaces; §13.3 "attribute's serialized name" writes
    // `xlink:`/`xml:`/`xmlns:` plus the local name, `xmlns` alone for the
    // `xmlns` attribute, and the local name for no namespace.
    let src = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 1 1">"#,
        r##"<a xlink:href="#u" xlink:title="t" xml:lang="ja"><use xlink:href="#s"></use></a>"##,
        "</svg>",
    );
    assert_eq!(body_html(src), src);
}

#[test]
fn colon_names_on_html_elements_are_plain_local_names() {
    // On HTML elements the parser doesn't adjust attributes: `xlink:href`
    // is a no-namespace attribute whose local name contains a colon.
    let src = r#"<p xlink:href="u" xml:lang="ja">x</p>"#;
    assert_eq!(body_html(src), src);
}

#[test]
fn raw_text_is_only_for_html_elements() {
    // §13.3: text in an HTML style or script element is written as is; an
    // SVG <style> or <script> is foreign content (§13.2.6.5), so its
    // character reference is decoded when parsed and escaped again here.
    assert_eq!(
        body_html("<style>a > b & c</style><script>if (a < b) x()</script>"),
        "<style>a > b & c</style><script>if (a < b) x()</script>"
    );
    assert_eq!(
        body_html("<svg><style>a &gt; b</style><script>a &lt; b</script></svg>"),
        "<svg><style>a &gt; b</style><script>a &lt; b</script></svg>"
    );
}

#[test]
fn noscript_is_raw_text_with_scripting_enabled() {
    // With scripting enabled the parser reads <noscript> as raw text
    // (§13.2.6.4.7, "in body"), and §13.3 writes that text literally.
    let src = "<noscript>1 &lt; 2 &lt;b&gt; <i>x</i></noscript>";
    assert_eq!(body_html(src), src);
    let again = body_html(&body_html(src));
    assert_eq!(again, src, "reparsing the output gives the same tree");
}

#[test]
fn void_elements_are_only_html_elements() {
    // §13.3: only HTML elements serialize as void. SVG `source` and
    // `track` (not breakout tags, §13.2.6.5) get end tags.
    assert_eq!(
        body_html("<svg><source/><track/></svg><br><source>"),
        "<svg><source></source><track></track></svg><br><source>"
    );
}

#[test]
fn comments_keep_their_data() {
    // §13.3: `<!--`, the comment's data, `-->`.
    assert_eq!(body_html("<p><!-- 売上 --></p>"), "<p><!-- 売上 --></p>");
}

/// HTML §13.3: scripting is disabled in template contents, whose owner document has
/// no browsing context, so `noscript` text there is escaped, as Chromium 153 does.
#[test]
fn noscript_in_template_contents_is_escaped() {
    let mut rt = surimono_dom::DomRuntime::new(surimono_core::Limits::default()).unwrap();
    rt.load(
        surimono_dom::Document::parse(
            "<!doctype html><body><template id=t><noscript><b>a</b></noscript></template><noscript><i>x</i></noscript>",
        )
        .unwrap(),
    );
    let () = rt
        .eval("document.getElementById('t').content.firstChild.textContent = '<b>&'; undefined")
        .unwrap();
    let html = rt.take().unwrap().to_html();
    assert!(
        html.contains("<template id=\"t\"><noscript>&lt;b&gt;&amp;</noscript></template><noscript><i>x</i></noscript>"),
        "{html}"
    );
}

/// What a printer builds from our output: blitz-html as fulgur uses it, which
/// parses with scripting off. Each element's local name with its depth, and
/// each text node's data, in tree order.
fn printer_tree(html: &str) -> Vec<(usize, String)> {
    let doc = blitz_html::HtmlDocument::from_html(html, blitz_dom::DocumentConfig::default());
    let (mut out, mut stack) = (Vec::new(), vec![(0, 0)]);
    while let Some((id, depth)) = stack.pop() {
        let node = doc.get_node(id).unwrap_or_else(|| panic!("node {id}"));
        match &node.data {
            blitz_dom::NodeData::Element(e) => out.push((depth, e.name.local.to_string())),
            blitz_dom::NodeData::Text(t) => out.push((depth, format!("#{}", t.content))),
            _ => {}
        }
        stack.extend(node.children.iter().rev().map(|&c| (c, depth + 1)));
    }
    out
}

/// HTML §13.3 with §13.2.6.4.7: escaped `noscript` text in template contents is
/// text to a printer whichever way it parses. With scripting off (blitz-html)
/// the character references decode back to the same text; with it on, the
/// element is raw text that holds no `</noscript`. Nothing a page or script put
/// there becomes a `style`, a `script` or a deep tree in the printer's parse.
#[test]
fn noscript_in_template_contents_stays_text_to_a_printer() {
    let deep = "<div>".repeat(500);
    for text in [
        "</template><style>@import 'http://[';</style>".to_owned(),
        format!("</template>{deep}"),
        "</noscript></template><script>alert(1)</script>".to_owned(),
    ] {
        // As a script wrote it, and as the page parsed it (scripting on: raw text,
        // which `</noscript` ends).
        let mut outputs = Vec::new();
        if !text.contains("</noscript") {
            let parsed =
                format!("<!doctype html><body><template><noscript>{text}</noscript></template>");
            outputs.push(Document::parse(&parsed).unwrap().to_html());
        }
        let mut rt = surimono_dom::DomRuntime::new(surimono_core::Limits::default()).unwrap();
        rt.load(Document::parse("<!doctype html><body><template id=t></template>").unwrap());
        let js = format!(
            "const n = document.createElement('noscript'); n.textContent = '{}';\
             document.getElementById('t').content.append(n); undefined",
            text.replace('\\', "\\\\").replace('\'', "\\'")
        );
        let () = rt.eval(&js).unwrap();
        outputs.push(rt.take().unwrap().to_html());
        for html in outputs {
            let tree = printer_tree(&html);
            let at = tree
                .iter()
                .position(|(_, n)| n == "noscript")
                .unwrap_or_else(|| panic!("{html}"));
            let depth = tree[at].0;
            assert_eq!(tree[at - 1], (depth - 1, "template".to_owned()), "{html}");
            assert_eq!(tree[at + 1], (depth + 1, format!("#{text}")), "{html}");
            let names: Vec<&str> = tree.iter().map(|(_, n)| n.as_str()).collect();
            assert!(
                !names
                    .iter()
                    .any(|n| matches!(*n, "style" | "script" | "div")),
                "{html}"
            );
        }
    }
}
