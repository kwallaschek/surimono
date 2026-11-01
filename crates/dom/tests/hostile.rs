//! Hostile documents and scripts against the id-handle bindings: trees
//! deeper or larger than the limits, deep trees on a small stack, handle
//! churn, and scripts that tamper with built-ins to read or forge handles
//! (review of #80).

use surimono_core::Limits;
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

const PAGE: &str = r#"<html><body><p id="p">secret</p><div id="d">x</div></body></html>"#;

/// `n` nested `<div>`s inside `<body>`; the innermost is at depth `n + 2`.
fn nested(n: usize) -> String {
    format!(
        "<html><body>{}<i id=\"deep\">deep</i>{}</body></html>",
        "<div>".repeat(n),
        "</div>".repeat(n)
    )
}

#[test]
fn parse_enforces_depth_and_node_limits() {
    // 1,000 levels used to load and then overflow the stack when dropped.
    let r = Document::parse(&nested(1_000));
    assert_eq!(r.unwrap_err(), DomError::TreeTooDeep { limit: 512 });
    // Exactly at the limit: html 1, body 2, 509 divs, <i> 512, its text 513.
    let too_deep = Document::parse(&nested(509));
    assert_eq!(too_deep.unwrap_err(), DomError::TreeTooDeep { limit: 512 });
    assert!(Document::parse(&nested(508)).is_ok());
    let limits = DomLimits {
        max_nodes: 50,
        ..DomLimits::default()
    };
    let r = Document::parse_with_limits(&"<b>x</b>".repeat(30), limits);
    assert_eq!(r.unwrap_err(), DomError::NodeLimit { limit: 50 });
}

#[test]
fn limits_are_validated() {
    for limits in [
        DomLimits {
            max_depth: DomLimits::MAX_DEPTH + 1,
            ..DomLimits::default()
        },
        DomLimits {
            max_nodes: 0,
            ..DomLimits::default()
        },
        DomLimits {
            max_handles: usize::MAX,
            ..DomLimits::default()
        },
    ] {
        assert!(
            matches!(
                Document::parse_with_limits(PAGE, limits),
                Err(DomError::InvalidLimits { .. })
            ),
            "{limits:?}"
        );
    }
}

/// The deepest tree the limits allow, on a thread with the stack a Puma or
/// Ruby thread has (1 MiB): dropping it from a script, appending a deep
/// subtree, serializing and dropping the document all work.
#[test]
fn deepest_allowed_tree_on_a_small_stack() {
    let worker = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let limits = DomLimits {
                max_depth: DomLimits::MAX_DEPTH,
                ..DomLimits::default()
            };
            // The innermost text node at exactly MAX_DEPTH.
            let doc = Document::parse_with_limits(&nested(DomLimits::MAX_DEPTH - 4), limits)
                .expect("at the limit");
            let html = doc.to_html();
            assert!(html.contains("<i id=\"deep\">deep</i>"));
            let mut rt =
                DomRuntime::new(Limits::default().with_stack_bytes(256 * 1024)).unwrap();
            rt.load(doc);
            let depth: i32 = rt
                .eval(
                    r#"
                let d = 0;
                for (let at = document.getElementById('deep'); at; at = at.parentNode) d++;
                const keep = document.getElementById('deep');
                document.body.textContent = '';
                // A detached chain as deep as the limit allows below body,
                // then joined to the document in one insertion.
                const top = document.createElement('div');
                let at = top;
                for (let i = 0; i < 1024 - 3; i++) { const c = document.createElement('div'); at.appendChild(c); at = c; }
                document.body.appendChild(top);
                d"#,
                )
                .unwrap();
            // <i> plus its ancestors up to and including the document node.
            assert_eq!(depth as usize, DomLimits::MAX_DEPTH);
            let r = rt.eval::<()>("keep.textContent");
            assert_eq!(r, Err(DomError::NodeRemoved));
            let r = rt.eval::<()>("document.body.appendChild(document.createElement('div')).appendChild(document.body.firstChild || document.createElement('b'))");
            // No firstChild binding yet: the new <b> lands at depth 4.
            assert!(r.is_ok(), "{r:?}");
            let doc = rt.take().unwrap();
            assert_eq!(doc.to_html().matches("<div>").count(), 1024 - 3 + 1 + 1);
            drop(doc);
            // One level deeper than the limit is refused, not a crash.
            let mut doc = Document::parse_with_limits(&nested(DomLimits::MAX_DEPTH - 4), limits)
                .unwrap();
            rt.load(doc);
            let r = rt.eval::<()>(
                "const b = document.createElement('b'); b.appendChild(document.createElement('b')); \
                 document.getElementById('deep').appendChild(b)",
            );
            assert_eq!(r, Err(DomError::TreeTooDeep { limit: DomLimits::MAX_DEPTH }));
            doc = rt.take().unwrap();
            drop(doc);
        })
        .unwrap();
    worker.join().expect("no stack overflow");
}

#[test]
fn wide_tree_drops_in_linear_time() {
    // 50,000 siblings, each with a child: one drop, not 50,000 list scans.
    let mut rt = DomRuntime::new(Limits::default()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let started = std::time::Instant::now();
    let _: () = rt
        .eval(
            r#"
        const d = document.getElementById('d');
        for (let i = 0; i < 25000; i++) d.appendChild(document.createElement('b')).appendChild(document.createTextNode('t'));
        d.textContent = '';"#,
        )
        .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    let doc = rt.take().unwrap();
    assert!(doc.node_count() < 20, "{}", doc.node_count());
}

#[test]
fn handle_churn_hits_the_handle_limit() {
    let mut rt = DomRuntime::new(Limits::default()).unwrap();
    rt.load(
        Document::parse_with_limits(
            PAGE,
            DomLimits {
                max_handles: 1_000,
                ..DomLimits::default()
            },
        )
        .unwrap(),
    );
    // Live nodes stay few; handles ever given out grow.
    let r = rt.eval::<()>(
        r#"const d = document.getElementById('d');
        try { for (;;) { d.appendChild(document.createElement('b')); d.textContent = ''; } } catch (e) {}"#,
    );
    assert_eq!(r, Err(DomError::HandleLimit { limit: 1_000 }));
}

/// The script that tampers with built-ins, then tries to forge a wrapper
/// from `{}` and to read the pair of a real one. Returns what leaked.
const TAMPER: &str = r#"
// Strings, not arrays: the index setters below would intercept a push.
globalThis.seen = '';
let out = '';
const apply = Reflect.apply;
const dp = Object.defineProperty;
const realGet = Object.getOwnPropertyDescriptor(Node.prototype, 'textContent').get;
const realSet = Object.getOwnPropertyDescriptor(Node.prototype, 'textContent').set;
const realName = Object.getOwnPropertyDescriptor(Node.prototype, 'nodeName').get;
const fakes = [{}, Object.setPrototypeOf({}, Element.prototype), new Proxy(document, {})];
const calls = [[realGet, []], [realName, []], [realSet, ['pwned']]];
const spy = (what) => function (...a) { seen += what + ':' + JSON.stringify(a) + ';'; return [1, 1]; };
WeakMap.prototype.get = spy('WeakMap.get');
WeakMap.prototype.set = spy('WeakMap.set');
WeakMap.prototype.has = spy('WeakMap.has');
Map.prototype.get = spy('Map.get');
Map.prototype.set = spy('Map.set');
Map.prototype.has = spy('Map.has');
Object.create = spy('Object.create');
Object.defineProperty = spy('Object.defineProperty');
Object.setPrototypeOf = spy('Object.setPrototypeOf');
Function.prototype.call = spy('call');
Function.prototype.apply = spy('apply');
String = spy('String');
for (let i = 0; i < 16; i++) {
  dp(Array.prototype, i, { get() { seen += 'Array.get ' + i + ';'; return 1; },
    set(v) { seen += 'Array.set ' + i + ';'; }, configurable: true });
  dp(Object.prototype, i, { get() { seen += 'Object.get ' + i + ';'; return undefined; },
    set(v) { seen += 'Object.set ' + i + ';'; }, configurable: true });
}
for (let f = 0; f < 3; f++) {
  for (let c = 0; c < 3; c++) {
    try { apply(calls[c][0], fakes[f], calls[c][1]); out += 'no error,'; } catch (e) { out += e.name + ','; }
  }
}
// Real wrappers still work, and nothing above saw a pair or was called.
const p = document.getElementById('p');
out += apply(realGet, p, []) + ',' + (document.getElementById('p') === p);
out += ',' + (p.parentNode === document.body) + ',' + document.body.tagName;
out += ',' + (document.createElement('b') instanceof Element);
out + ',[' + seen + ']'
"#;

const EXPECTED: &str = "TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,\
TypeError,TypeError,TypeError,secret,true,true,BODY,true,[]";

#[test]
fn tampering_before_first_use_cannot_read_or_forge_handles() {
    let mut rt = DomRuntime::new(Limits::default()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    // The first eval installs the bindings, then runs the tampering script
    // before any wrapper exists.
    let out: String = rt.eval(TAMPER).unwrap();
    assert_eq!(out, EXPECTED);
    let html = rt.with_document(Document::to_html).unwrap();
    assert!(html.contains(r#"<p id="p">secret</p>"#), "{html}");
}

#[test]
fn tampering_after_use_cannot_read_or_forge_handles() {
    let mut rt = DomRuntime::new(Limits::default()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    // Wrappers exist and are cached before the built-ins are replaced.
    let _: String = rt
        .eval("globalThis.early = document.getElementById('p'); early.textContent")
        .unwrap();
    let out: String = rt.eval(TAMPER).unwrap();
    assert_eq!(out, EXPECTED);
    let same: bool = rt.eval("early === document.getElementById('p')").unwrap();
    assert!(same);
    // After a reset the bindings install afresh, untouched by the old
    // context's tampering.
    rt.reset().unwrap();
    let out: String = rt.eval(TAMPER).unwrap();
    assert_eq!(out, EXPECTED);
}
