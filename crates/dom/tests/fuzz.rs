//! Seeded random operations against the bindings: the classes of bug that
//! come from blitz-dom's document behaviour (panics on URLs and `@import`,
//! links lost on a move, nodes grown outside the limits). The operations come
//! from a fixed list, over the elements with special handling in blitz-dom,
//! and fixed seeds, so a failure reproduces. After every seed the document's
//! links and byte count are checked and its output reparsed.
//!
//! An operation may throw, and a script may hit a DOM limit; neither is a
//! finding. A panic, a stack overflow or a broken invariant is. A panic in a
//! native function reaches JavaScript as an exception, which the `try` around
//! each operation would swallow, so a panic hook counts them.

mod common;

use std::sync::Once;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::limits;
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

static PANICS: AtomicUsize = AtomicUsize::new(0);

/// Counts panics from here on, then asks the default hook to report them.
fn count_panics() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let report = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            PANICS.fetch_add(1, Ordering::Relaxed);
            report(info);
        }));
    });
}

const SEEDS: u64 = 60;
const OPS: usize = 800;

const PAGE: &str = r#"<!doctype html><html><head><title>t</title><style>b{color:red}</style>
<link rel="stylesheet" href="a.css"></head><body><form id="f"><input type="button" value="v">
<select><option>1</option></select><textarea>x</textarea></form><img src="x.png"><canvas src="1"></canvas>
<table><tr><td>c</td></tr></table><svg><style>a{}</style></svg><p>text<!--c--></p></body></html>"#;

const TAGS: &[&str] = &[
    "style", "link", "img", "input", "button", "form", "select", "textarea", "canvas", "title",
    "table", "div", "svg", "object", "output", "fieldset",
];

const ATTRS: &[&str] = &[
    "href", "href", "href", "src", "src", "rel", "type", "value", "style", "id", "class", "form",
    "checked", "name",
];

const VALUES: &[&str] = &[
    "",
    "stylesheet",
    "button",
    "submit",
    "file",
    "checkbox",
    "true",
    "f",
    "x.png",
    "http://[",
    "//[",
    "1",
    "color: red; background: url(http://[)",
    "@import 'http://[';",
    "@import url(\"http://[\") print;",
    "@\\69mport 'http://[';",
    "<b>",
    "\u{fffd}",
];

/// xorshift64*: enough to pick operations; the seed fixes the sequence.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, list: &[&'a str]) -> &'a str {
        list[self.below(list.len())]
    }
}

fn quoted(s: &str) -> String {
    format!("{s:?}")
}

/// Sets up `pool`, a growing list of nodes the operations draw from.
const PRELUDE: &str = r#"
globalThis.pool = [document.documentElement, document.head, document.body];
// Half the picks go to the elements made here, which have the special handling.
globalThis.at = (i) => pool[(i % 2 ? i : i >> 1) % (i % 2 ? pool.length : Math.min(60, pool.length))];
for (const top of [document.head, document.body])
  for (let n = top.firstChild; n; n = n.nextSibling) pool.push(n);
for (const t of TAGS) { const e = document.createElement(t); pool.push(e); pool.push(document.body.appendChild(document.createElement(t))); }
// Replacing the content of these would end most of a run in its first ops.
globalThis.small = (i) => { const n = at(i); return n === pool[0] || n === pool[1] || n === pool[2] ? pool[pool.length - 1] : n; };
globalThis.keep = (n) => { if (n && pool.length < 400) pool.push(n); return n; };
globalThis.op = (f) => { try { f(); } catch (e) {} };
"#;

/// One random operation, as source.
fn operation(r: &mut Rng) -> String {
    let (a, b, c) = (r.below(1000), r.below(1000), r.below(1000));
    let tag = quoted(r.pick(TAGS));
    let attr = quoted(r.pick(ATTRS));
    let value = quoted(r.pick(VALUES));
    let body = match r.below(26) {
        0 => format!("keep(document.createElement({tag}))"),
        1 => format!("keep(document.createTextNode({value}))"),
        2 => format!("keep(document.createComment({value}))"),
        3..=5 => format!("at({a}).appendChild(at({b}))"),
        6 => format!("at({a}).setAttribute({attr}, {value})"),
        7 => format!("at({a}).removeAttribute({attr})"),
        8 => format!("at({a}).toggleAttribute({attr})"),
        9 => format!("small({a}).textContent = {value}"),
        10 => format!("at({a}).data = {value}"),
        11 => format!("at({a}).nodeValue = {value}"),
        12 => format!("keep(at({a}).firstChild); keep(at({b}).lastChild)"),
        13 => format!("keep(at({a}).nextSibling); keep(at({b}).parentNode)"),
        14 => format!("at({a}).id = {value}; at({b}).className = {value}"),
        15 => format!("at({a}).insertBefore(at({b}), at({c}))"),
        16 => format!("at({a}).insertBefore(at({b}), at({a}).firstChild)"),
        17 => format!("at({a}).replaceChild(at({b}), at({a}).firstChild)"),
        18 => format!("at({a}).removeChild(at({a}).lastChild)"),
        19 => format!("keep(at({a}).cloneNode(true))"),
        20 => format!("at({a}).append({value}, at({b}))"),
        21 => format!("at({a}).before(at({b}), {value})"),
        22 => format!("at({a}).replaceWith(at({b}), {value})"),
        23 => format!("at({a}).remove(); at({b}).normalize()"),
        24 => format!(
            "const f = keep(document.createDocumentFragment()); f.append(at({a}), {value}); at({b}).appendChild(f)"
        ),
        // Reads: every node type, whatever state it reached.
        _ => format!(
            "[at({a}).textContent, at({b}).nodeName, at({c}).isConnected, document.getElementById({value})]"
        ),
    };
    format!("op(() => {{ {body}; }});\n")
}

fn run_seed(seed: u64) {
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut rt = DomRuntime::new(limits()).unwrap();
    let dom_limits = DomLimits {
        max_nodes: 2_000,
        max_text_bytes: 1 << 20,
        ..DomLimits::default()
    };
    rt.load(Document::parse_with_limits(PAGE, dom_limits).unwrap());
    let tags = TAGS.iter().map(|t| quoted(t)).collect::<Vec<_>>().join(",");
    let _: () = rt
        .eval(&format!("const TAGS = [{tags}];\n{PRELUDE}"))
        .unwrap();
    let mut script = String::new();
    for _ in 0..OPS {
        script.push_str(&operation(&mut rng));
    }
    match rt.eval::<()>(&script) {
        Ok(()) => {}
        // A limit: fatal to the script, no more.
        Err(
            DomError::NodeLimit { .. } | DomError::TreeTooDeep { .. } | DomError::TextLimit { .. },
        ) => {}
        Err(e) => panic!("seed {seed}: {e:?}"),
    }
    assert_eq!(PANICS.load(Ordering::Relaxed), 0, "seed {seed}");
    let doc = rt.take().unwrap();
    doc.check_consistency()
        .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
    assert!(doc.node_count() <= 2_000, "seed {seed}");
    let html = doc.to_html();
    // The output parses, under limits big enough for it.
    let again = Document::parse(&html).unwrap_or_else(|e| panic!("seed {seed}: {e:?}"));
    again
        .check_consistency()
        .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
}

#[test]
fn random_operations_keep_the_document_sound() {
    count_panics();
    for seed in 1..=SEEDS {
        run_seed(seed);
    }
}

#[test]
fn hostile_markup_parses() {
    for css in VALUES {
        for page in [
            format!("<!doctype html><style>{css}</style>"),
            format!("<svg><style>{css}<b>{css}</b></style></svg>"),
            format!("<table><style>{css}</style><tr><td>x"),
            format!(
                "<link rel=stylesheet href='{css}'><img src='{css}'><input type=button value='{css}'>"
            ),
            format!(
                "<title>{css}</title><canvas src='{css}'></canvas><form id=f><input form=f></form>"
            ),
        ] {
            let doc = Document::parse(&page).unwrap_or_else(|e| panic!("{page}: {e:?}"));
            doc.check_consistency()
                .unwrap_or_else(|e| panic!("{page}: {e}"));
            doc.to_html();
        }
    }
}
