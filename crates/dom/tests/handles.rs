//! Id-based node handles end to end: parse, script through wrappers, read
//! the result back in Rust. Expected DOM behaviour is from the DOM Standard
//! (https://dom.spec.whatwg.org/), sections cited per test.

use std::time::Duration;

use surimono_core::{Limits, SandboxError};
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

const PAGE: &str = r#"<!doctype html><html><head><title>t</title></head>
<body><div id="chart"><span id="label">Sales</span> 2026</div><p id="p">x</p></body></html>"#;

fn runtime_with(html: &str) -> DomRuntime {
    let mut rt = DomRuntime::new(Limits::default()).unwrap();
    assert!(rt.load(Document::parse(html).unwrap()).is_none());
    rt
}

#[test]
fn get_element_by_id_and_text_content() {
    let mut rt = runtime_with(PAGE);
    // DOM §4.2.4 (getElementById), §4.4 (textContent: concatenated text
    // of descendants for an element, null for a document).
    let got: String = rt
        .eval(r#"document.getElementById('chart').textContent"#)
        .unwrap();
    assert_eq!(got, "Sales 2026");
    let missing: bool = rt.eval("document.getElementById('nope') === null").unwrap();
    assert!(missing);
    let doc_text: bool = rt.eval("document.textContent === null").unwrap();
    assert!(doc_text);
}

#[test]
fn parent_node_and_identity() {
    let mut rt = runtime_with(PAGE);
    // Same node, same object; parentNode up to the document (DOM §4.4).
    let r: Vec<String> = rt
        .eval(
            r#"
        const label = document.getElementById('label');
        const chart = document.getElementById('chart');
        [
          String(label.parentNode === chart),
          String(document.getElementById('label') === label),
          chart.parentNode.nodeName,
          String(document.documentElement.parentNode === document),
          String(document.parentNode),
          String(document.body.tagName),
          String(label instanceof Element && label instanceof Node),
          String(document.nodeType) + ',' + label.nodeType + ',' + label.parentNode.parentNode.nodeType,
        ]"#,
        )
        .unwrap();
    assert_eq!(
        r,
        [
            "true", "true", "BODY", "true", "null", "BODY", "true", "9,1,1"
        ]
    );
}

#[test]
fn text_content_setter_mutates_the_rust_document() {
    let mut rt = runtime_with(PAGE);
    let _: () = rt
        .eval(r#"document.getElementById('chart').textContent = 'Revenue <b>'"#)
        .unwrap();
    // The old children are gone from the Rust tree and the id map; the
    // text is one text node, not markup (DOM §4.4, "string replace all"),
    // escaped when serialized (HTML §13.3).
    let html = rt.with_document(Document::to_html).unwrap();
    assert!(
        html.contains(r#"<div id="chart">Revenue &lt;b&gt;</div>"#),
        "{html}"
    );
    assert!(!html.contains("label"), "{html}");
    let r: Vec<String> = rt
        .eval(
            r#"
        const chart = document.getElementById('chart');
        const out = [chart.textContent, String(document.getElementById('label'))];
        // A text node's data changes in place (DOM §4.4: CharacterData).
        const t = document.createTextNode('a');
        chart.appendChild(t);
        t.textContent = 'z';
        out.push(t.textContent, chart.textContent, t.nodeName);
        out"#,
        )
        .unwrap();
    assert_eq!(r, ["Revenue <b>", "null", "z", "Revenue <b>z", "#text"]);
    let html = rt.with_document(Document::to_html).unwrap();
    assert!(
        html.contains(r#"<div id="chart">Revenue &lt;b&gt;z</div>"#),
        "{html}"
    );
}

#[test]
fn removed_node_is_a_typed_error_not_wrong_data() {
    let mut rt = runtime_with(PAGE);
    let _: () = rt
        .eval(
            r#"
        globalThis.label = document.getElementById('label');
        document.getElementById('chart').textContent = '';
        // New nodes may reuse the dropped node's blitz slot.
        for (let i = 0; i < 20; i++) document.body.appendChild(document.createElement('i'));
        "#,
        )
        .unwrap();
    let r = rt.eval::<String>("label.textContent");
    assert_eq!(r, Err(DomError::NodeRemoved));
    // Fatal even when the script catches the exception.
    let r = rt.eval::<String>("try { label.parentNode; 'caught' } catch (e) { 'caught' }");
    assert_eq!(r, Err(DomError::NodeRemoved));
    // The runtime carries on; live nodes still work.
    let p: String = rt.eval("document.getElementById('p').textContent").unwrap();
    assert_eq!(p, "x");
}

#[test]
fn wrapper_from_a_previous_document_is_stale() {
    let mut rt = runtime_with(PAGE);
    let _: () = rt
        .eval("globalThis.old = document.getElementById('chart')")
        .unwrap();
    let old = rt
        .load(Document::parse(r#"<html><body><div id="chart">new</div></body></html>"#).unwrap())
        .unwrap();
    // The old document's #chart is node-for-node where the new one's is.
    let r = rt.eval::<String>("old.textContent");
    assert_eq!(r, Err(DomError::StaleDocument));
    let new: String = rt
        .eval("document.getElementById('chart').textContent")
        .unwrap();
    assert_eq!(new, "new");
    // With no document, wrappers and `document` itself report so.
    rt.take().unwrap();
    assert_eq!(rt.eval::<bool>("document === null"), Ok(true));
    assert_eq!(rt.eval::<String>("old.nodeName"), Err(DomError::NoDocument));
    // Reloading the old document makes its wrappers valid again.
    rt.load(old);
    assert_eq!(rt.eval::<String>("old.textContent").unwrap(), "Sales 2026");
}

#[test]
fn wrappers_cannot_be_forged() {
    let mut rt = runtime_with(PAGE);
    // DOM accessors on a non-node throw TypeError (WebIDL §3.7.6, "Illegal
    // invocation"); there is no id to forge.
    let r: Vec<String> = rt
        .eval(
            r#"
        const get = Object.getOwnPropertyDescriptor(Node.prototype, 'textContent').get;
        const out = [];
        for (const fake of [{}, Object.create(Element.prototype), 42]) {
          try { get.call(fake); out.push('no error'); } catch (e) { out.push(e.name); }
        }
        try { new Node(); } catch (e) { out.push(e.name); }
        out.push(String(Object.keys(document.getElementById('label')).length));
        out"#,
        )
        .unwrap();
    assert_eq!(r, ["TypeError", "TypeError", "TypeError", "TypeError", "0"]);
}

#[test]
fn create_and_append() {
    let mut rt = runtime_with(PAGE);
    let r: Vec<String> = rt
        .eval(
            r#"
        const ul = document.createElement('UL');
        for (const s of ['a', 'b']) {
          const li = document.createElement('li');
          li.appendChild(document.createTextNode(s));
          ul.appendChild(li);
        }
        document.body.appendChild(ul);
        const out = [ul.tagName, ul.textContent, String(ul.parentNode === document.body)];
        // DOM §4.2.3 pre-insert validity: HierarchyRequestError.
        try { ul.firstChildLi = ul.appendChild(document.body); } catch (e) { out.push(e.name); }
        try { document.body.appendChild(document); } catch (e) { out.push(e.name); }
        // DOM §4.5 createElement: InvalidCharacterError.
        try { document.createElement('1x'); } catch (e) { out.push(e.name); }
        out"#,
        )
        .unwrap();
    assert_eq!(
        r,
        [
            "UL",
            "ab",
            "true",
            "HierarchyRequestError",
            "HierarchyRequestError",
            "InvalidCharacterError"
        ]
    );
    let html = rt.with_document(Document::to_html).unwrap();
    assert!(
        html.contains("<ul><li>a</li><li>b</li></ul></body>"),
        "{html}"
    );
}

#[test]
fn node_and_depth_limits_are_typed_errors() {
    let mut rt = DomRuntime::new(Limits::default()).unwrap();
    rt.load(
        Document::parse_with_limits(
            PAGE,
            DomLimits {
                max_nodes: 200,
                max_depth: 50,
                ..DomLimits::default()
            },
        )
        .unwrap(),
    );
    let r = rt.eval::<()>(
        "try { for (;;) document.createElement('b'); } catch (e) {} // caught, still fatal",
    );
    assert_eq!(r, Err(DomError::NodeLimit { limit: 200 }));
    rt.load(
        Document::parse_with_limits(
            PAGE,
            DomLimits {
                max_nodes: 200,
                max_depth: 50,
                ..DomLimits::default()
            },
        )
        .unwrap(),
    );
    let r = rt.eval::<()>(
        "let at = document.body; for (let i = 0; i < 60; i++) { const d = document.createElement('div'); at.appendChild(d); at = d; }",
    );
    assert_eq!(r, Err(DomError::TreeTooDeep { limit: 50 }));
}

#[test]
fn sandbox_limits_apply_and_bindings_survive_a_rebuild() {
    let mut rt = DomRuntime::new(
        Limits::default()
            .with_memory_bytes(8 * 1024 * 1024)
            .with_time(Duration::from_millis(200)),
    )
    .unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    // A loop of host calls still ends with the time limit.
    let r = rt
        .eval::<()>("for (;;) { try { document.getElementById('p').textContent; } catch (e) {} }");
    assert_eq!(r, Err(DomError::Sandbox(SandboxError::TimeLimit)));
    let r = rt.eval::<()>(
        "const a = []; for (;;) a.push(document.body.textContent.repeat(100000) + a.length);",
    );
    assert_eq!(r, Err(DomError::Sandbox(SandboxError::MemoryLimit)));
    let r = rt.eval::<()>("function f() { return f() + 1; } f()");
    assert_eq!(r, Err(DomError::Sandbox(SandboxError::StackOverflow)));
    // The sandbox rebuilt its context after each; the bindings come back,
    // with the document as the script left it.
    let p: String = rt.eval("document.getElementById('p').textContent").unwrap();
    assert_eq!(p, "x");
    // An ordinary exception is an ordinary sandbox error.
    let r = rt.eval::<()>("null.x");
    assert!(
        matches!(r, Err(DomError::Sandbox(SandboxError::Exception { .. }))),
        "{r:?}"
    );
}

#[test]
fn no_document_loaded() {
    let mut rt = DomRuntime::new(Limits::default()).unwrap();
    assert_eq!(rt.eval::<bool>("document === null"), Ok(true));
    assert!(rt.take().is_none());
}
