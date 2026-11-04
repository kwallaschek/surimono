//! `Document::to_html` against the HTML Standard's fragment serialization
//! algorithm (https://html.spec.whatwg.org/#serialising-html-fragments,
//! §13.3). Every expected string is what that algorithm gives for the tree
//! the HTML parser builds (§13.2) with scripting disabled, which is how
//! blitz-html 0.2.0 parses.

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
fn noscript_is_not_raw_text_with_scripting_disabled() {
    // With scripting disabled the parser treats <noscript> as an ordinary
    // element (§13.2.6.4.7, "in body"), and §13.3 writes its text escaped
    // because raw text applies to noscript only when scripting is enabled.
    // Written raw, the decoded `<b>` would come back as an element.
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

/// Comment data is lost: blitz-dom 0.2.4's `NodeData::Comment` has no
/// fields, so every comment serializes as `<!---->`. Recorded as a known
/// gap; this test holds the spec's answer for when blitz keeps the data.
#[test]
#[ignore = "known gap: blitz-dom 0.2.4 keeps no comment data"]
fn comments_keep_their_data() {
    // §13.3: `<!--`, the comment's data, `-->`.
    assert_eq!(body_html("<p><!-- 売上 --></p>"), "<p><!-- 売上 --></p>");
}
