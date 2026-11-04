//! Hostile documents and scripts against the id-handle bindings: trees
//! deeper or larger than the limits, deep trees on a small stack, handle
//! churn, and scripts that tamper with built-ins to read or forge handles
//! (review of #80).

mod common;

use std::time::Duration;

use common::{limits, scaled};
use surimono_core::Sandbox;
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

// With a doctype: blitz-html prints every parse error on stdout, and a
// missing doctype is one ("ERROR: Unexpected token").
const PAGE: &str =
    r#"<!doctype html><html><body><p id="p">secret</p><div id="d">x</div></body></html>"#;

/// `n` nested `<div>`s inside `<body>`; the innermost is at depth `n + 2`.
fn nested(n: usize) -> String {
    format!(
        "<!doctype html><html><body>{}<i id=\"deep\">deep</i>{}</body></html>",
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
    let r =
        Document::parse_with_limits(&format!("<!doctype html>{}", "<b>x</b>".repeat(30)), limits);
    assert_eq!(r.unwrap_err(), DomError::NodeLimit { limit: 50 });
}

/// blitz-dom panics when it can't resolve an `<img src>` or `<link href>`
/// as the element joins the document; its default base can't resolve any
/// relative reference. Parsed, set by a script, moved and removed.
#[test]
fn urls_blitz_cannot_resolve_are_harmless() {
    for url in ["x.png", "/a/b.css", "http://[", "http://a b/", "//["] {
        let page = format!(
            r#"<!doctype html><html><head><link rel="stylesheet" href="{url}"></head>
            <body><img id="i" src="{url}"></body></html>"#
        );
        let doc = Document::parse(&page).unwrap();
        let html = doc.to_html();
        assert!(html.contains(&format!(r#"src="{url}""#)), "{html}");
        assert!(html.contains(&format!(r#"href="{url}""#)), "{html}");
        let mut rt = DomRuntime::new(limits()).unwrap();
        rt.load(doc);
        let script = format!(
            r#"const i = document.getElementById('i');
            const n = document.createElement('img');
            n.setAttribute('src', '{url}');
            document.body.appendChild(n);
            i.setAttribute('src', '{url}');
            const l = document.createElement('link');
            l.setAttribute('rel', 'stylesheet');
            document.head.appendChild(l);
            l.setAttribute('href', '{url}');
            n.getAttribute('src') + i.getAttribute('src') + l.getAttribute('href')"#
        );
        let got: String = rt.eval(&script).unwrap();
        assert_eq!(got, url.repeat(3));
    }
}

/// blitz-dom's `clear_attribute` hits `unreachable!` for a `<link>`'s
/// `href` unless a stylesheet was loaded (review of #93).
#[test]
fn removing_a_links_href() {
    let page = r#"<!doctype html><head><link id="l" rel="stylesheet" href="a.css"></head>"#;
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(page).unwrap());
    let r: String = rt
        .eval(
            r#"const made = document.createElement('link');
            made.setAttribute('href', 'x');
            made.removeAttribute('href');
            const l = document.getElementById('l');
            const gone = l.toggleAttribute('href');
            l.setAttribute('href', 'y');
            l.removeAttribute('href');
            [made.hasAttribute('href'), gone, l.hasAttribute('href')].join()"#,
        )
        .unwrap();
    assert_eq!(r, "false,false,false");
}

/// A CSS `@import` with an unresolvable URL panics in blitz-dom (net.rs)
/// when it parses a `<style>`'s text, attached or not, from HTML or a
/// script, whole or split over text nodes (review of #93).
#[test]
fn style_text_never_reaches_blitz() {
    const IMPORTS: [&str; 3] = [
        r#"@import "http://[";"#,
        r#"@import url("http://[") print;"#,
        r#"@\69mport "http://[";"#,
    ];
    for css in IMPORTS {
        let doc = Document::parse(&format!(
            "<!doctype html><head><style id=s>{css}</style></head><body>\
             <svg><style id=t>{css}<g>{css}</g></style></svg><p id=p>x</p>"
        ))
        .unwrap();
        // The text is all still there for scripts and for the output.
        let html = doc.to_html();
        assert!(
            html.contains(&format!("<style id=\"s\">{css}</style>")),
            "{html}"
        );
        assert_eq!(html.matches(css).count(), 3, "{html}");
        let mut rt = DomRuntime::new(limits()).unwrap();
        rt.load(doc);
        let js = serde_free_quote(css);
        let script = format!(
            r#"const css = {js};
            const s = document.getElementById('s');
            const out = [s.textContent === css, s.firstChild.data === css];
            // Detached; by textContent; by data; by nodeValue.
            const d = document.createElement('style');
            d.textContent = css;
            d.firstChild.data = css + css;
            d.firstChild.nodeValue = css;
            // Split over two text nodes, and into a nested element.
            const e = document.createElement('style');
            e.appendChild(document.createTextNode(css.slice(0, 5)));
            e.appendChild(document.createTextNode(css.slice(5)));
            const b = e.appendChild(document.createElement('b'));
            b.textContent = css;
            b.firstChild.data = css;
            // A text node written before it joins, and one that leaves.
            const t = document.createTextNode(css);
            d.appendChild(t);
            document.body.appendChild(t);
            out.push(t.data === css, e.textContent === css + css, d.textContent === css);
            t.data = 'x'; out.push(t.data);
            document.head.appendChild(e);
            document.body.textContent = '';
            out.join()"#
        );
        assert_eq!(
            rt.eval::<String>(&script).unwrap(),
            "true,true,true,true,true,x"
        );
        rt.take().unwrap().check_consistency().unwrap();
    }
}

/// A JS string literal for `s`.
fn serde_free_quote(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        if c == '\'' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// blitz-dom's `append_children` removes the child from its old parent by
/// id after adding it, so a move within one parent lost the node and left a
/// parent link to it (review of #93).
#[test]
fn appending_a_child_to_its_own_parent() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse("<!doctype html><body><p id=p><b>1</b><i>2</i></p>").unwrap());
    // DOM §4.2.3: appendChild moves it to the end.
    let r: String = rt
        .eval(
            r#"const p = document.getElementById('p');
            const b = p.firstChild;
            p.appendChild(b);
            [p.firstChild.tagName, p.lastChild === b, b.parentNode === p, b.previousSibling.tagName].join()"#,
        )
        .unwrap();
    assert_eq!(r, "I,true,true,I");
    // The dangling parent link: panicked with "invalid key", or named an
    // unrelated node once the slab slot was reused.
    let r: bool = rt
        .eval(
            r#"const q = document.createElement('div'), o = document.createElement('div');
            const c = document.createElement('b');
            o.appendChild(c); o.appendChild(c); q.appendChild(o);
            q.textContent = '';
            document.body.appendChild(document.createElement('u'));
            false"#,
        )
        .unwrap();
    assert!(!r);
    assert_eq!(rt.eval::<()>("c.isConnected"), Err(DomError::NodeRemoved));
    rt.take().unwrap().check_consistency().unwrap();
}

/// blitz-dom gives `<input type=button value=x>` a text child each time it
/// joins or leaves the document, outside `max_nodes` (review of #93). An
/// `<input>` has no children.
#[test]
fn inputs_grow_no_children() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    let page = r#"<!doctype html><body><input id=i type=button value=parsed><div id=d></div>"#;
    rt.load(
        Document::parse_with_limits(
            page,
            DomLimits {
                max_nodes: 50,
                ..DomLimits::default()
            },
        )
        .unwrap(),
    );
    let r: String = rt
        .eval(
            r#"const parsed = document.getElementById('i');
            const i = document.createElement('input');
            i.setAttribute('type', 'button');
            i.setAttribute('value', 'v');
            const d = document.getElementById('d');
            for (let k = 0; k < 1000; k++) { document.body.appendChild(i); d.appendChild(i); }
            [parsed.hasChildNodes(), parsed.textContent, i.hasChildNodes(), i.textContent].join()"#,
        )
        .unwrap();
    assert_eq!(r, "false,,false,");
    let doc = rt.take().unwrap();
    assert!(doc.node_count() < 50, "{}", doc.node_count());
    doc.check_consistency().unwrap();
}

/// Strings live in Rust, outside the JS heap limit: each write path is
/// charged (review of #93).
#[test]
fn string_bytes_are_limited() {
    const LIMIT: usize = 8 << 20;
    let vars = "const s = 'x'.repeat(1 << 20); const b = document.body; const n = () => document.createElement('i');";
    for script in [
        "for (;;) document.createTextNode(s);",
        "for (;;) new Text(s);",
        "for (;;) document.createComment(s);",
        "for (;;) new Comment(s);",
        "for (let k = 0;; k++) document.createElement(s + k);",
        "for (let k = 0;; k++) b.setAttribute('a' + k, s);",
        "for (let k = 0;; k++) b.setAttribute(s + k, '');",
        "for (;;) n().setAttribute('a', s);",
        "for (;;) n().id = s;",
        "for (;;) n().className = s;",
        "for (;;) n().title = s;",
        "for (;;) n().textContent = s;",
        "for (;;) document.createTextNode('').data = s;",
        "for (;;) document.createComment('').data = s;",
        "for (;;) document.createTextNode('').nodeValue = s;",
        "for (;;) document.createElement('style').textContent = s;",
    ] {
        // About bytes, not time: a 1 MiB name goes through debug-build checks
        // and lower-casing, and takes seconds on a loaded machine.
        let mut rt = DomRuntime::new(limits().with_time(scaled(Duration::from_secs(120)))).unwrap();
        rt.load(
            Document::parse_with_limits(
                PAGE,
                DomLimits {
                    max_text_bytes: LIMIT,
                    ..DomLimits::default()
                },
            )
            .unwrap(),
        );
        // Fatal even if the script catches it, like the other DOM limits.
        let r = rt.eval::<()>(&format!("{vars} try {{ {script} }} catch (e) {{}}"));
        assert_eq!(r, Err(DomError::TextLimit { limit: LIMIT }), "{script}");
        rt.take().unwrap().check_consistency().unwrap();
    }
    // From the parser too.
    let big = format!("<!doctype html><p title=\"{}\">", "x".repeat(1 << 20));
    let small = DomLimits {
        max_text_bytes: 1 << 20,
        ..DomLimits::default()
    };
    assert_eq!(
        Document::parse_with_limits(&big, small).unwrap_err(),
        DomError::TextLimit { limit: 1 << 20 }
    );
}

/// Overwrites and drops give the bytes back: a page rewriting a label stays
/// well inside the budget however often it does.
#[test]
fn string_bytes_are_credited() {
    const LIMIT: usize = 8 << 20;
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(
        Document::parse_with_limits(
            PAGE,
            DomLimits {
                max_text_bytes: LIMIT,
                ..DomLimits::default()
            },
        )
        .unwrap(),
    );
    let _: () = rt
        .eval(
            r#"const s = 'x'.repeat(1 << 20);
            const d = document.getElementById('d');
            const t = document.createTextNode('');
            const c = document.createComment('');
            for (let k = 0; k < 40; k++) {
              d.textContent = s;
              d.textContent = '';
              d.setAttribute('a', s);
              d.setAttribute('a', s + k);
              d.removeAttribute('a');
              t.data = s;
              c.data = s;
              const e = d.appendChild(document.createElement('b'));
              e.setAttribute('b', s);
              e.textContent = s;
              d.textContent = '';
            }"#,
        )
        .unwrap();
    rt.take().unwrap().check_consistency().unwrap();
}

/// WebIDL §3.7.1: `new.target`'s prototype, for subclasses (review of #93).
#[test]
fn constructors_honour_new_target() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let r: String = rt
        .eval(
            r#"class T extends Text { hi() { return 'hi ' + this.data; } }
            const t = new T('x');
            [t instanceof T, t instanceof Text, t.hi(), t.nodeType, new Text('y') instanceof T].join()"#,
        )
        .unwrap();
    assert_eq!(r, "true,true,hi x,3,false");
}

/// A setter that isn't implemented isn't generated (review of #93).
#[test]
fn document_body_has_no_unimplemented_setter() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let r: String = rt
        .eval(
            r#"const d = Object.getOwnPropertyDescriptor(Document.prototype, 'body');
            String(d.set)"#,
        )
        .unwrap();
    assert_eq!(r, "undefined");
}

/// §4.2.3 allows a comment under a document (review of #93).
#[test]
fn comments_go_under_the_document() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let r: bool = rt
        .eval("const c = document.createComment('x'); document.appendChild(c).parentNode === document")
        .unwrap();
    assert!(r);
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
        DomLimits {
            max_text_bytes: 0,
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
                DomRuntime::new(common::limits().with_stack_bytes(256 * 1024)).unwrap();
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
            let r = rt.eval::<()>("document.body.appendChild(document.createElement('div')).appendChild(document.createElement('b'))");
            // The new <b> lands at depth 4.
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

/// Time to drop `n` siblings with one text child each, best of 3. Each
/// trial builds in its own `eval`, outside the timer: the build loop is nearly
/// all of the work, and would hide a slow drop.
fn wide_drop_time(n: usize) -> Duration {
    (0..3)
        .map(|_| {
            // The limit only has to let a slow build finish building.
            let mut rt =
                DomRuntime::new(limits().with_time(scaled(Duration::from_secs(60)))).unwrap();
            rt.load(Document::parse(PAGE).unwrap());
            let build = format!(
                "globalThis.d = document.getElementById('d');
                for (let i = 0; i < {n}; i++) d.appendChild(document.createElement('b')).appendChild(document.createTextNode('t'));"
            );
            let _: () = rt.eval(&build).unwrap();
            let started = std::time::Instant::now();
            let _: () = rt.eval("d.textContent = ''").unwrap();
            let took = started.elapsed();
            let doc = rt.take().unwrap();
            assert!(doc.node_count() < 20, "{}", doc.node_count());
            took
        })
        .min()
        .unwrap()
}

/// One drop of a wide tree, not one list scan per child. The drop is compared
/// with itself, so machine speed and the time scale don't matter: four times
/// the children must cost less than eight times the time. Measured in a debug
/// build: linear about 4.5, quadratic (a scan per child) about 12.7.
#[test]
fn wide_tree_drops_in_linear_time() {
    let small = wide_drop_time(6_250);
    let large = wide_drop_time(25_000);
    assert!(large < small * 8, "6,250: {small:?}; 25,000: {large:?}");
}

#[test]
fn handle_churn_hits_the_handle_limit() {
    let mut rt = DomRuntime::new(limits()).unwrap();
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
    let mut rt = DomRuntime::new(limits()).unwrap();
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
    let mut rt = DomRuntime::new(limits()).unwrap();
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

/// Runs `script` on a thread whose stack is only just large enough for the
/// sandbox (`stack_bytes` plus [`Sandbox::STACK_HEADROOM_BYTES`], plus
/// `SLACK` for this test's own frames), against a body-only page whose
/// limits allow [`DomLimits::MAX_DEPTH`].
/// On success, checks the `levels`-deep chain is in the serialization.
fn on_a_tight_stack(script: &'static str, levels: usize) -> Result<i32, DomError> {
    // The frames between the thread's start and the sandbox's stack check,
    // and the part of the requested size that isn't usable stack (32 KiB
    // was too little on Linux, debug build).
    const SLACK: usize = 64 * 1024;
    let limits = limits();
    let size = limits.stack_bytes + Sandbox::STACK_HEADROOM_BYTES + SLACK;
    std::thread::Builder::new()
        .stack_size(size)
        .spawn(move || {
            let mut rt = DomRuntime::new(limits).unwrap();
            let dom_limits = DomLimits {
                max_depth: DomLimits::MAX_DEPTH,
                ..DomLimits::default()
            };
            rt.load(Document::parse_with_limits(PAGE, dom_limits).unwrap());
            let r = rt.eval::<i32>(script);
            if r.is_ok() {
                // The chain really joined the document, and serializing it
                // still works on this thread.
                let html = rt.with_document(Document::to_html).unwrap();
                assert_eq!(html.matches("<div>").count(), levels);
            }
            r
        })
        .unwrap()
        .join()
        .expect("no stack overflow")
}

/// The script that squeezes the stack: build a detached `<div>` chain of
/// `levels`, recurse in JavaScript until the engine's stack limit throws,
/// then append the chain to `<body>` from the deepest frame that can still
/// make the call. Joining the document makes blitz-dom recurse once per
/// level in Rust, in whatever stack the JavaScript left. Returns the
/// JavaScript depth the append ran at.
macro_rules! squeeze {
    ($levels:expr) => {
        concat!(
            "const top = document.createElement('div'); let at = top;\n",
            "for (let i = 1; i < ",
            $levels,
            "; i++) { const c = document.createElement('div'); at.appendChild(c); at = c; }\n",
            r#"let done = -1;
            function f(n) {
              try { f(n + 1); } catch (e) {
                if (!(e instanceof RangeError)) throw e;
                // Too little stack left to make the call throws a
                // RangeError again, and the next frame up retries.
                if (done < 0) { document.body.appendChild(top); done = n; }
              }
            }
            f(0);
            done"#
        )
    };
}

#[test]
fn append_at_max_depth_after_recursing_to_the_stack_limit() {
    // <body> is at depth 2, so a chain of MAX_DEPTH - 2 divs (1,022) ends
    // exactly at MAX_DEPTH: the deepest insertion the limits allow
    // (review of #80).
    let depth = on_a_tight_stack(squeeze!("1022"), 1_022).expect("the append succeeds");
    assert!(depth > 100, "the append ran deep in the recursion: {depth}");
}

#[test]
fn too_deep_append_after_recursing_to_the_stack_limit_is_a_typed_error() {
    // One level more than the limit: refused before blitz-dom walks it.
    let r = on_a_tight_stack(squeeze!("1023"), 1_023);
    assert_eq!(
        r,
        Err(DomError::TreeTooDeep {
            limit: DomLimits::MAX_DEPTH
        })
    );
}
