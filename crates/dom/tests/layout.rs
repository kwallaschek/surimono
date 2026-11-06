//! The layout-query policy: expected values are CSSOM View's and CSSOM's
//! answers for an element with no box, and Geometry's for `DOMRect` (§3).

mod common;

use common::limits;
use surimono_core::SettleOptions;
use surimono_dom::{Document, DomError, DomLimits, DomRuntime, LayoutPolicy};

fn runtime(policy: Option<LayoutPolicy>) -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    if let Some(p) = policy {
        rt.set_layout_policy(p);
    }
    rt.load(Document::parse("<!doctype html><body><p id=a>x</p><svg id=s></svg>").unwrap());
    rt
}

const A: &str = "const a = document.getElementById('a');";

/// Every query in scope, as source.
const QUERIES: &[(&str, &str)] = &[
    ("getBoundingClientRect", "a.getBoundingClientRect()"),
    ("getClientRects", "a.getClientRects()"),
    ("clientTop", "a.clientTop"),
    ("clientLeft", "a.clientLeft"),
    ("clientWidth", "a.clientWidth"),
    ("clientHeight", "a.clientHeight"),
    ("scrollTop", "a.scrollTop"),
    ("scrollTo", "a.scrollTo(0, 1)"),
    ("scrollBy", "a.scrollBy(0, 1)"),
    ("scroll", "a.scroll(0, 1)"),
    ("scrollIntoView", "a.scrollIntoView()"),
    ("elementsFromPoint", "document.elementsFromPoint(1, 2)"),
    ("getBBox", "document.getElementById('s').getBBox()"),
    ("scrollTo", "window.scrollTo(0, 1)"),
    ("scrollLeft", "a.scrollLeft"),
    ("scrollTop", "a.scrollTop = 5"),
    ("scrollLeft", "a.scrollLeft = 5"),
    ("scrollWidth", "a.scrollWidth"),
    ("scrollHeight", "a.scrollHeight"),
    ("offsetParent", "a.offsetParent"),
    ("offsetTop", "a.offsetTop"),
    ("offsetLeft", "a.offsetLeft"),
    ("offsetWidth", "a.offsetWidth"),
    ("offsetHeight", "a.offsetHeight"),
    ("elementFromPoint", "document.elementFromPoint(1, 2)"),
    ("getComputedStyle", "getComputedStyle(a)"),
    ("innerWidth", "innerWidth"),
    ("innerHeight", "innerHeight"),
    ("outerWidth", "outerWidth"),
    ("outerHeight", "outerHeight"),
    ("scrollX", "scrollX"),
    ("scrollY", "scrollY"),
    ("pageXOffset", "pageXOffset"),
    ("pageYOffset", "pageYOffset"),
];

#[test]
fn zeros_is_the_default_and_answers_like_an_element_with_no_box() {
    let mut rt = runtime(None);
    let got: String = rt
        .eval(&format!(
            r#"{A}
            const r = a.getBoundingClientRect();
            const l = a.getClientRects();
            const c = getComputedStyle(a);
            JSON.stringify([
              r instanceof DOMRect, r instanceof DOMRectReadOnly, r.x, r.y, r.width, r.height,
              r.top, r.right, r.bottom, r.left,
              l instanceof DOMRectList, l.length, l.item(0), [...l].length,
              a.clientWidth, a.clientHeight, a.clientTop, a.clientLeft,
              a.scrollWidth, a.scrollHeight, a.scrollTop, a.scrollLeft,
              a.offsetWidth, a.offsetHeight, a.offsetTop, a.offsetLeft, a.offsetParent,
              document.elementFromPoint(0, 0),
              innerWidth, innerHeight, outerWidth, outerHeight,
              scrollX, scrollY, pageXOffset, pageYOffset,
              c instanceof CSSStyleDeclaration, c.length, c.width, c.getPropertyValue('color'),
              c.cssText, c.item(0),
            ])"#
        ))
        .unwrap();
    assert_eq!(
        got,
        "[true,true,0,0,0,0,0,0,0,0,true,0,null,0,0,0,0,0,0,0,0,0,0,0,0,0,null,null,\
         0,0,0,0,0,0,0,0,true,0,\"\",\"\",\"\",\"\"]"
    );
}

#[test]
fn a_computed_style_ignores_the_inline_style_and_cant_be_written() {
    let mut rt = runtime(None);
    let got: String = rt
        .eval(&format!(
            r#"{A}
            a.style.color = 'red';
            const c = getComputedStyle(a, '::before');
            const out = [c.color, c === getComputedStyle(a), c instanceof CSSStyleDeclaration];
            for (const f of [() => {{ c.color = 'blue'; }}, () => c.setProperty('color', 'blue'),
                             () => c.removeProperty('color'), () => {{ c.cssText = 'top: 0'; }}]) {{
              try {{ f(); out.push('no'); }} catch (e) {{ out.push(e.name); }}
            }}
            out.push(a.getAttribute('style'));
            out.join()"#
        ))
        .unwrap();
    assert_eq!(
        got,
        ",false,true,NoModificationAllowedError,NoModificationAllowedError,\
         NoModificationAllowedError,NoModificationAllowedError,color: red;"
    );
}

#[test]
fn throw_mode_ends_the_call_for_every_query_even_if_caught() {
    for (member, source) in QUERIES {
        let mut rt = runtime(Some(LayoutPolicy::Throw));
        let e = rt
            .eval::<()>(&format!("{A} try {{ {source}; }} catch (e) {{}}"))
            .unwrap_err();
        assert_eq!(
            e,
            DomError::LayoutQuery {
                member: (*member).to_owned()
            },
            "{source}"
        );
    }
}

#[test]
fn every_query_runs_under_zeros() {
    let mut rt = runtime(Some(LayoutPolicy::Zeros));
    for (_, source) in QUERIES {
        rt.eval::<()>(&format!("{{ {A} {source}; }}")).unwrap();
    }
}

#[test]
fn throw_mode_fails_a_page_and_outlasts_a_reset() {
    let mut rt = runtime(Some(LayoutPolicy::Throw));
    rt.reset().unwrap();
    rt.load(
        Document::parse("<!doctype html><script>addEventListener('load', () => document.body.offsetWidth)</script><body>")
            .unwrap(),
    );
    let e = rt
        .run_page(&SettleOptions::default(), |_| None)
        .unwrap_err();
    assert_eq!(
        e,
        DomError::LayoutQuery {
            member: "offsetWidth".to_owned()
        }
    );
}

#[test]
fn throw_mode_leaves_the_dom_alone() {
    let mut rt = runtime(Some(LayoutPolicy::Throw));
    let got: String = rt
        .eval("const r = new DOMRect(1, 2, 3, 4); document.body.tagName + r.bottom")
        .unwrap();
    assert_eq!(got, "BODY6");
}

/// Geometry §3.
#[test]
fn dom_rects() {
    let mut rt = runtime(None);
    let got: String = rt
        .eval(
            r#"const r = new DOMRect(10, 20, -3, -4);
            const ro = new DOMRectReadOnly(1, 2);
            r.width = 3;
            class Sub extends DOMRect {}
            JSON.stringify([r.left, r.right, r.top, r.bottom, r, ro.height,
              DOMRect.fromRect({ x: 5, height: 7 }), DOMRectReadOnly.fromRect() instanceof DOMRect,
              new Sub() instanceof Sub, Object.prototype.toString.call(r),
              Object.keys(DOMRect.prototype).length, DOMRect.prototype.__proto__ === DOMRectReadOnly.prototype])"#,
        )
        .unwrap();
    assert_eq!(
        got,
        "[10,13,16,20,{\"x\":10,\"y\":20,\"width\":3,\"height\":-4,\"top\":16,\"right\":13,\
         \"bottom\":20,\"left\":10},0,{\"x\":5,\"y\":0,\"width\":0,\"height\":7,\"top\":0,\
         \"right\":5,\"bottom\":7,\"left\":5},false,true,\"[object DOMRect]\",4,true]"
    );
}

/// Brand checks, stale wrappers and [Replaceable], as for every other member.
#[test]
fn members_check_their_receiver() {
    let mut rt = runtime(None);
    let got: String = rt
        .eval(&format!(
            r#"{A}
            const out = [];
            const name = (f) => {{ try {{ f(); return 'no'; }} catch (e) {{ return e.name; }} }};
            const d = Object.getOwnPropertyDescriptor(Element.prototype, 'clientWidth');
            out.push(name(() => d.get.call({{}})));
            out.push(name(() => Element.prototype.getBoundingClientRect.call(document)));
            out.push(name(() => getComputedStyle({{}})));
            out.push(name(() => getComputedStyle()));
            out.push(name(() => document.elementFromPoint(1)));
            out.push(name(() => document.getElementById('s').offsetWidth));
            innerWidth = 7;
            out.push(innerWidth, Object.getOwnPropertyDescriptor(window, 'innerWidth').writable);
            out.join()"#
        ))
        .unwrap();
    assert_eq!(
        got,
        "TypeError,TypeError,TypeError,TypeError,TypeError,no,7,true"
    );
}

#[test]
fn a_stale_wrapper_fails_like_any_other() {
    let mut rt = runtime(None);
    rt.eval::<()>("globalThis.old = document.getElementById('a')")
        .unwrap();
    rt.load(Document::parse("<!doctype html>").unwrap());
    for q in [
        "old.clientWidth",
        "old.getBoundingClientRect()",
        "getComputedStyle(old)",
    ] {
        let e = rt
            .eval::<()>(&format!("try {{ {q}; }} catch (e) {{}}"))
            .unwrap_err();
        assert_eq!(e, DomError::StaleDocument, "{q}");
    }
}

#[test]
fn the_window_answers_with_no_document() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    assert_eq!(rt.eval::<i32>("innerWidth").unwrap(), 0);
}

/// CSSOM §7.2 says [NewObject], and handles are never reused: each call gets a
/// new wrapper over one handle, so a loop doesn't reach `max_handles`.
#[test]
fn a_computed_style_is_a_new_object_over_one_handle() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    let l = DomLimits {
        max_handles: 200,
        ..DomLimits::default()
    };
    rt.load(Document::parse_with_limits("<!doctype html><body><p id=a>x</p>", l).unwrap());
    let distinct: bool = rt
        .eval(&format!(
            "{A} let distinct = true, last = null;
             for (let i = 0; i < 2000; i++) {{
               const c = getComputedStyle(a);
               distinct = distinct && c !== last && c.width === '';
               last = c;
             }}
             distinct"
        ))
        .unwrap();
    assert!(distinct);
}

/// WebIDL §3.2.7: `elementFromPoint` takes restricted doubles.
#[test]
fn element_from_point_refuses_what_isnt_finite() {
    let mut rt = runtime(None);
    let got: String = rt
        .eval(
            "const out = [];
             for (const a of [[NaN, 0], [0, NaN], [Infinity, 0], [0, -Infinity], ['x', 0]]) {
               try { document.elementFromPoint(...a); out.push('no'); }
               catch (e) { out.push(e.name); }
             }
             out.push(document.elementFromPoint('1', 2));
             out.join()",
        )
        .unwrap();
    assert_eq!(got, "TypeError,TypeError,TypeError,TypeError,TypeError,");
}

/// Nothing has a box: nothing moves, nothing is hit, and the box is empty.
#[test]
fn zeros_mode_for_what_moves_or_returns_lists() {
    let mut rt = runtime(None);
    let r: String = rt
        .eval(&format!(
            "{A} a.scrollTo(5, 5); a.scrollBy(1, 1); a.scrollIntoView(); scrollTo(1, 2); window.scrollBy(1, 1);
             const b = document.getElementById('s').getBBox();
             [a.scrollTop, scrollY, document.elementsFromPoint(1, 2).length, b.x, b.width, b instanceof DOMRect].join()"
        ))
        .unwrap();
    assert_eq!(r, "0,0,0,0,0,true");
}

/// CSSOM View: one argument is a dictionary, and the functions have names.
#[test]
fn scroll_methods_convert_their_arguments() {
    let mut rt = runtime(None);
    let r: String = rt
        .eval(&format!(
            "{A} const name = (f) => {{ try {{ f(); return 'none'; }} catch (e) {{ return e.name; }} }};
             [name(() => a.scrollTo(1)), name(() => scrollTo('x')), name(() => a.scrollBy({{ top: 1 }})),
              name(() => scrollBy(null)), name(() => a.scroll()), name(() => a.scrollTo(1, 2)),
              a.scrollTo.name, scrollBy.name, a.scroll.length].join()"
        ))
        .unwrap();
    assert_eq!(
        r,
        "TypeError,TypeError,none,none,none,none,scrollTo,scrollBy,0"
    );
}
