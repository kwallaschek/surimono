//! Ranges against the DOM Standard (§5) and WPT dom/ranges and
//! domparsing/createContextualFragment: what mutations do to live ranges
//! (§4.2.3, §4.10, §4.11, §4.4), `Range`'s methods, and the limits.

mod common;

use std::time::{Duration, Instant};

use common::{limits, scaled};
use surimono_core::{Limits, SandboxError};
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

const PAGE: &str = r#"<!doctype html><html><head></head><body><div id="a"><i id="i">one</i><b id="b">two</b>tail</div><p id="p">para</p></body></html>"#;

const VARS: &str = "
const a = document.getElementById('a'), i = document.getElementById('i'),
  b = document.getElementById('b'), p = document.getElementById('p');
const one = i.firstChild, two = b.firstChild, tail = a.lastChild, para = p.firstChild;
const r = document.createRange();
const show = (rg) => {
  const name = (n) => n.nodeName === '#text' ? JSON.stringify(n.data) : n.nodeName;
  return name(rg.startContainer) + ':' + rg.startOffset + ' ' + name(rg.endContainer) + ':' + rg.endOffset;
};
";

fn run_with(limit: DomLimits, script: &str) -> Result<String, DomError> {
    let time = scaled(Duration::from_secs(30));
    let mut rt = DomRuntime::new(limits().with_time(time)).unwrap();
    rt.load(Document::parse_with_limits(PAGE, limit).unwrap());
    let r: Result<String, DomError> = rt.eval(&format!("{VARS}{script}"));
    if let Some(check) = rt.with_document(Document::check_consistency) {
        check.unwrap();
    }
    r
}

fn run(script: &str) -> String {
    run_with(DomLimits::default(), script).unwrap()
}

fn body_of(script: &str) -> (String, String) {
    let time = scaled(Duration::from_secs(30));
    let mut rt = DomRuntime::new(limits().with_time(time)).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let r: String = rt.eval(&format!("{VARS}{script}")).unwrap();
    rt.with_document(Document::check_consistency)
        .unwrap()
        .unwrap();
    let html = rt.with_document(Document::to_html).unwrap();
    let body = html.split("<body>").nth(1).unwrap();
    (r, body.split("</body>").next().unwrap().to_owned())
}

#[test]
fn a_new_range_is_collapsed_at_the_document() {
    assert_eq!(
        run(
            "[show(r), r.collapsed, show(new Range()), r instanceof AbstractRange, Object.prototype.toString.call(r)].join('|')"
        ),
        "#document:0 #document:0|true|#document:0 #document:0|true|[object Range]"
    );
}

/// §5.5 "set the start or end": a start past the end, or in another tree,
/// takes the end with it.
#[test]
fn setting_a_boundary_keeps_the_order() {
    assert_eq!(
        run("r.setStart(two, 1); const x = show(r);
             r.setEnd(one, 1); const y = show(r);
             const d = document.createElement('div');
             r.setEnd(two, 2); r.setStart(d, 0);
             [x, y, show(r)].join('|')"),
        r#""two":1 "two":1|"one":1 "one":1|DIV:0 DIV:0"#
    );
}

#[test]
fn boundary_errors_are_the_standards() {
    assert_eq!(
        run("const name = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
             [name(() => r.setStart(one, 4)),
              name(() => r.setStart(document.doctype, 0)),
              name(() => r.setStartBefore(document)),
              name(() => r.selectNode(document)),
              name(() => r.selectNodeContents(document.doctype)),
              name(() => r.compareBoundaryPoints(4, r)),
              name(() => r.compareBoundaryPoints(0, new DOMParser().parseFromString('', 'text/html').createRange())),
              name(() => r.comparePoint(document.createElement('i'), 0)),
              name(() => r.isPointInRange(one, 9)),
              name(() => r.setStart(a)),
              name(() => r.setStart({}, 0))].join()"),
        "IndexSizeError,InvalidNodeTypeError,InvalidNodeTypeError,InvalidNodeTypeError,\
         InvalidNodeTypeError,NotSupportedError,WrongDocumentError,WrongDocumentError,\
         IndexSizeError,TypeError,TypeError"
    );
}

#[test]
fn comparisons_read_tree_order() {
    assert_eq!(
        run("r.setStart(one, 1); r.setEnd(tail, 2);
             const s = document.createRange(); s.selectNode(b);
             [r.compareBoundaryPoints(Range.START_TO_START, s),
              r.compareBoundaryPoints(Range.START_TO_END, s),
              r.compareBoundaryPoints(Range.END_TO_END, s),
              r.compareBoundaryPoints(Range.END_TO_START, s),
              r.comparePoint(one, 0), r.comparePoint(two, 0), r.comparePoint(tail, 3),
              r.isPointInRange(two, 1), r.isPointInRange(para, 0),
              r.intersectsNode(i), r.intersectsNode(p), r.intersectsNode(document),
              r.commonAncestorContainer === a].join()"),
        "-1,1,1,-1,-1,0,1,true,false,true,false,true,true"
    );
}

/// `§5.5 stringifier`, across element boundaries, in UTF-16 units.
#[test]
fn the_stringifier_takes_the_text_in_the_range() {
    assert_eq!(
        run("r.setStart(one, 1); r.setEnd(para, 2); const x = String(r);
             two.data = 'a\\u{1F600}b'; r.setStart(two, 1); r.setEnd(two, 3);
             [x, r.toString()].join('|')"),
        "netwotailpa|\u{1F600}"
    );
}

/// §4.2.3 "insert": only offsets past the insertion point move.
#[test]
fn an_insertion_moves_later_offsets() {
    assert_eq!(
        run("r.setStart(a, 1); r.setEnd(a, 2);
             a.insertBefore(document.createElement('u'), b);
             const x = show(r);
             a.insertBefore(document.createElement('u'), a.firstChild);
             const y = show(r);
             a.appendChild(document.createElement('u'));
             [x, y, show(r)].join('|')"),
        "DIV:1 DIV:3|DIV:2 DIV:4|DIV:2 DIV:4"
    );
}

#[test]
fn inserting_a_fragment_counts_its_children_and_empties_its_ranges() {
    assert_eq!(
        run("const f = document.createDocumentFragment();
             const k = f.appendChild(document.createElement('u'));
             k.appendChild(document.createTextNode('in'));
             f.appendChild(document.createElement('u'));
             const inside = document.createRange(); inside.setStart(k.firstChild, 1); inside.setEnd(f, 2);
             r.setStart(a, 1); r.setEnd(a, 3);
             a.insertBefore(f, b);
             [show(r), show(inside)].join('|')"),
        "DIV:1 DIV:5|#document-fragment:0 #document-fragment:0"
    );
}

/// §4.2.3 "remove": a boundary in the removed subtree goes to the parent, at
/// the node's index; later offsets drop by one.
#[test]
fn a_removal_moves_boundaries_out_of_the_subtree() {
    assert_eq!(
        run("r.setStart(two, 1); r.setEnd(a, 3);
             const s = document.createRange(); s.setStart(a, 2); s.setEnd(para, 1);
             a.removeChild(b);
             [show(r), show(s)].join('|')"),
        "DIV:1 DIV:2|DIV:1 \"para\":1"
    );
}

/// "Replace all" removes the children first: one pass, the same result.
#[test]
fn replacing_all_children_collapses_ranges_inside() {
    assert_eq!(
        run("r.setStart(one, 1); r.setEnd(a, 3);
             const s = document.createRange(); s.setStart(para, 1); s.setEnd(para, 2);
             a.textContent = 'new';
             p.innerHTML = '<b>x</b>';
             [show(r), show(s)].join('|')"),
        "DIV:0 DIV:0|P:0 P:0"
    );
}

/// §4.2.3 "replace": removal and insertion in that order.
#[test]
fn replacing_a_child_keeps_ranges_at_its_place() {
    assert_eq!(
        run("r.setStart(two, 1); r.setEnd(a, 3);
             a.replaceChild(document.createElement('u'), b);
             show(r)"),
        "DIV:1 DIV:3"
    );
}

/// §4.10 "replace data".
#[test]
fn data_changes_move_offsets() {
    assert_eq!(
        run("tail.data = 'abcdefgh';
             r.setStart(tail, 2); r.setEnd(tail, 6);
             tail.deleteData(1, 2); const x = show(r);
             tail.insertData(0, 'zz'); const y = show(r);
             tail.replaceData(3, 2, 'QQQQ'); const z = show(r);
             tail.data = 'gone';
             [x, y, z, show(r)].join('|')"),
        r#""adefgh":1 "adefgh":4|"zzadefgh":3 "zzadefgh":6|"zzaQQQQfgh":3 "zzaQQQQfgh":8|"gone":0 "gone":0"#
    );
}

#[test]
fn every_way_to_set_data_is_a_replace() {
    for setter in ["nodeValue = 'x'", "textContent = 'x'", "data = 'x'"] {
        assert_eq!(
            run(&format!(
                "r.setStart(tail, 1); r.setEnd(tail, 3); tail.{setter}; show(r)"
            )),
            r#""x":0 "x":0"#,
            "{setter}"
        );
    }
    assert_eq!(
        run("const c = a.appendChild(document.createComment('abc'));
             r.setStart(c, 1); r.setEnd(c, 2); c.data = 'zzzz'; [r.startOffset, r.endOffset].join()"),
        "0,0"
    );
}

/// §4.11 "split".
#[test]
fn split_text_carries_later_offsets_to_the_new_node() {
    assert_eq!(
        run("tail.data = 'abcdef';
             r.setStart(tail, 1); r.setEnd(tail, 5);
             const s = document.createRange(); s.setStart(a, 3); s.setEnd(a, 3);
             const t = document.createRange(); t.setStart(tail, 2); t.setEnd(tail, 3);
             tail.splitText(3);
             [show(r), show(s), show(t)].join('|')"),
        r#""abc":1 "def":2|DIV:4 DIV:4|"abc":2 "abc":3"#
    );
    assert_eq!(
        run("const lone = document.createTextNode('abcdef');
             r.setStart(lone, 2); r.setEnd(lone, 5);
             lone.splitText(3);
             show(r)"),
        r#""abc":2 "abc":3"#
    );
}

/// §4.4 `normalize` step 6.
#[test]
fn normalize_carries_boundaries_into_the_merged_node() {
    assert_eq!(
        run("const t2 = p.appendChild(document.createTextNode('more'));
             const t3 = p.appendChild(document.createTextNode('last'));
             r.setStart(t2, 2); r.setEnd(p, 2);
             const s = document.createRange(); s.setStart(p, 1); s.setEnd(t3, 1);
             p.normalize();
             [show(r), show(s), p.childNodes.length].join('|')"),
        r#""paramorelast":6 "paramorelast":8|"paramorelast":4 "paramorelast":9|1"#
    );
    assert_eq!(
        run("const e = p.appendChild(document.createTextNode(''));
             r.setStart(e, 0); r.setEnd(p, 2);
             p.normalize();
             show(r)"),
        r#""para":4 P:1"#
    );
}

#[test]
fn delete_contents_across_elements() {
    let (r, body) = body_of(
        "r.setStart(one, 1); r.setEnd(para, 2);
         r.deleteContents();
         show(r)",
    );
    assert_eq!(r, "BODY:1 BODY:1");
    assert_eq!(body, r#"<div id="a"><i id="i">o</i></div><p id="p">ra</p>"#);
}

#[test]
fn extract_contents_splits_partly_contained_nodes() {
    let (r, body) = body_of(
        "r.setStart(one, 1); r.setEnd(para, 2);
         const f = r.extractContents();
         const d = document.createElement('section');
         d.appendChild(f);
         [show(r), d.innerHTML].join('|')",
    );
    assert_eq!(
        r,
        r#"BODY:1 BODY:1|<div id="a"><i id="i">ne</i><b id="b">two</b>tail</div><p id="p">pa</p>"#
    );
    assert_eq!(body, r#"<div id="a"><i id="i">o</i></div><p id="p">ra</p>"#);
}

#[test]
fn clone_contents_copies_and_changes_nothing() {
    let (r, body) = body_of(
        "r.setStart(one, 1); r.setEnd(para, 2);
         const d = document.createElement('section');
         d.appendChild(r.cloneContents());
         const c = document.createRange(); c.setStart(tail, 1); c.setEnd(tail, 3);
         const e = document.createElement('section');
         e.appendChild(c.cloneContents());
         [show(r), d.innerHTML, e.innerHTML].join('|')",
    );
    assert_eq!(
        r,
        r#""one":1 "para":2|<div id="a"><i id="i">ne</i><b id="b">two</b>tail</div><p id="p">pa</p>|ai"#
    );
    assert_eq!(
        body,
        PAGE.split("<body>")
            .nth(1)
            .unwrap()
            .split("</body>")
            .next()
            .unwrap()
    );
}

#[test]
fn a_doctype_cant_be_cloned_or_extracted_by_a_range() {
    assert_eq!(
        run("r.selectNodeContents(document);
             const name = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
             [name(() => r.cloneContents()), name(() => r.extractContents())].join()"),
        "HierarchyRequestError,HierarchyRequestError"
    );
}

/// Deep chains are walked, not recursed; these run at the document's depth.
#[test]
fn the_deepest_chain_is_cut_on_a_small_stack() {
    let script = "
      let top = document.createElement('div'), at = top;
      for (let i = 0; i < 500; i++) at = at.appendChild(document.createElement('div'));
      at.appendChild(document.createTextNode('deep'));
      const q = top.appendChild(document.createElement('div'));
      let right = q;
      for (let i = 0; i < 499; i++) right = right.appendChild(document.createElement('div'));
      right.appendChild(document.createTextNode('deep'));
      document.body.appendChild(top);
      r.setStart(at.firstChild, 2); r.setEnd(right.firstChild, 2);
      const a1 = r.cloneContents().querySelectorAll('div').length;
      const a2 = r.extractContents().querySelectorAll('div').length;
      a1 + ',' + a2 + ',' + show(r);
    ";
    let result = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || run(script))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(result, "1000,1000,DIV:1 DIV:1");
}

#[test]
fn insert_node_splits_text_and_grows_a_collapsed_range() {
    let (r, body) = body_of(
        "r.setStart(para, 2); r.collapse(true);
         r.insertNode(document.createElement('u'));
         show(r)",
    );
    assert_eq!(r, r#""pa":2 P:2"#);
    assert_eq!(
        body.rsplit("</div>").next().unwrap(),
        r#"<p id="p">pa<u></u>ra</p>"#
    );
    assert_eq!(
        run(
            "const name = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
             const c = a.appendChild(document.createComment('x'));
             r.setStart(c, 0);
             const lone = document.createRange(); lone.setStart(document.createTextNode('x'), 0);
             [name(() => r.insertNode(document.createElement('u'))),
              name(() => lone.insertNode(document.createElement('u')))].join()"
        ),
        "HierarchyRequestError,HierarchyRequestError"
    );
}

#[test]
fn surround_contents_wraps_or_refuses() {
    let (r, body) = body_of(
        "r.setStart(para, 1); r.setEnd(para, 3);
         r.surroundContents(document.createElement('u'));
         show(r)",
    );
    assert_eq!(r, "P:1 P:2");
    assert_eq!(
        body.rsplit("</div>").next().unwrap(),
        r#"<p id="p">p<u>ar</u>a</p>"#
    );
    assert_eq!(
        run(
            "const name = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
             r.setStart(one, 1); r.setEnd(two, 1);
             const s = document.createRange(); s.selectNodeContents(p);
             [name(() => r.surroundContents(document.createElement('u'))),
              name(() => s.surroundContents(document.createDocumentFragment()))].join()"
        ),
        "InvalidStateError,InvalidNodeTypeError"
    );
}

#[test]
fn clone_range_is_independent() {
    assert_eq!(
        run("r.setStart(one, 1); r.setEnd(two, 1);
             const c = r.cloneRange();
             r.collapse(); r.detach();
             [show(c), show(r), c === r].join('|')"),
        r#""one":1 "two":1|"two":1 "two":1|false"#
    );
}

/// DOM §5.4: nothing is checked but the node types, and nothing moves.
#[test]
fn static_ranges_stay_where_they_were_put() {
    assert_eq!(
        run("const s = new StaticRange({ startContainer: tail, startOffset: 99, endContainer: one, endOffset: 1 });
             tail.remove();
             const name = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
             [show(s), s.collapsed, s instanceof AbstractRange, s instanceof Range,
              name(() => new StaticRange()),
              name(() => new StaticRange({ startContainer: a, startOffset: 0, endContainer: a })),
              name(() => new StaticRange({ startContainer: document.doctype, startOffset: 0, endContainer: a, endOffset: 0 })),
              name(() => new StaticRange({ startContainer: a.attributes[0], startOffset: 0, endContainer: a, endOffset: 0 })),
              name(() => StaticRange({}))].join('|')"),
        r#""tail":99 "one":1|false|true|false|TypeError|TypeError|InvalidNodeTypeError|InvalidNodeTypeError|TypeError"#
    );
}

/// HTML §8.5: a void element, `html`, or no element at all as the context.
#[test]
fn contextual_fragments_parse_in_the_start_nodes_context() {
    assert_eq!(
        run("const html = (f) => { const d = document.createElement('section'); d.appendChild(f); return d.innerHTML; };
             r.selectNodeContents(document.documentElement);
             const x = html(r.createContextualFragment('<td>a</td><span>b</span>'));
             r.setStart(para, 1);
             const y = html(r.createContextualFragment('<p>c'));
             const tr = document.createElement('table').appendChild(document.createElement('tr'));
             r.selectNodeContents(tr);
             const z = html(r.createContextualFragment('<td>d</td>'));
             const q = document.createRange();
             q.createContextualFragment('x').nodeName + '|' + [x, y, z].join('|')"),
        "#document-fragment|a<span>b</span>|<p>c</p>|<td>d</td>"
    );
}

/// The XML parser hands back what it is given, so the wrappers come off here.
#[test]
fn contextual_fragments_in_xml_lose_html_wrappers() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(
        Document::parse_xml(
            r#"<html xmlns="http://www.w3.org/1999/xhtml"><head></head><body/></html>"#,
        )
        .unwrap(),
    );
    let r: String = rt
        .eval(
            "const x = 'xmlns=\"http://www.w3.org/1999/xhtml\"';
             const r = document.createRange(); r.setStart(document.documentElement, 0);
             const f = r.createContextualFragment(`<html ${x}><head><title>t</title></head><body><html><p>a</p></html><i>b</i></body></html>`);
             const g = r.createContextualFragment(`<html xmlns='urn:x'><body/></html>`);
             [...f.childNodes].map((n) => n.localName).join() + '|' + [...g.childNodes].map((n) => n.localName).join() + g.firstChild.childNodes.length",
        )
        .unwrap();
    assert_eq!(r, "title,p,i|html1");
}

#[test]
fn ranges_cross_documents_with_their_nodes() {
    assert_eq!(
        run("const d = document.implementation.createHTMLDocument('t');
             const q = d.createRange();
             q.selectNodeContents(d.body);
             const f = q.createContextualFragment('<b>x</b>');
             d.body.appendChild(f);
             q.selectNode(d.body.firstChild);
             const e = q.cloneContents();
             [f.ownerDocument === d, e.ownerDocument === d, q.startContainer === d.body, String(q)].join()"),
        "true,true,true,x"
    );
}

/// A boundary's node can be removed and every wrapper dropped: it still has
/// its handle, so the slab slot isn't reused under the range.
#[test]
fn a_boundary_keeps_its_node_alive() {
    assert_eq!(
        run("const box = document.createElement('div');
             box.innerHTML = '<u>a</u><u>b</u>';
             r.setStart(box.lastChild, 0);
             r.setEnd(box.lastChild, 1);
             box.textContent = '';
             a.innerHTML = '<s>c</s><s>d</s>';
             [show(r), r.startContainer === box].join('|')"),
        "DIV:0 DIV:0|true"
    );
}

#[test]
fn a_range_from_another_render_is_stale() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let () = rt
        .eval("globalThis.r = document.createRange(); r.selectNode(document.body); undefined")
        .unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let stale: Result<u32, DomError> = rt.eval("r.startOffset");
    assert_eq!(stale, Err(DomError::StaleDocument));
}

// Limits.

#[test]
fn too_many_ranges_is_a_typed_error_that_try_cant_hide() {
    let l = DomLimits {
        max_ranges: 100,
        ..DomLimits::default()
    };
    for make in ["document.createRange()", "new Range()", "r.cloneRange()"] {
        let e = run_with(
            l,
            &format!("try {{ for (let i = 0; i < 1000; i++) {make}; }} catch (e) {{}} 'done'"),
        );
        assert_eq!(e, Err(DomError::RangeLimit { limit: 100 }), "{make}");
    }
}

#[test]
fn max_ranges_is_validated() {
    let l = DomLimits {
        max_ranges: 0,
        ..DomLimits::default()
    };
    assert!(matches!(l.validate(), Err(DomError::InvalidLimits { .. })));
}

/// Out of handles in the middle of a removal: the boundary can't move to a
/// parent with no handle yet, stays on a live node, and the call fails.
#[test]
fn running_out_of_handles_in_a_removal_stays_typed() {
    let l = DomLimits {
        max_handles: 40,
        ..DomLimits::default()
    };
    let e = run_with(
        l,
        "const top = document.createElement('div');
         top.innerHTML = '<u><b>x</b></u>';
         const deep = top.firstChild.firstChild.firstChild;
         r.setStart(deep, 0); r.setEnd(deep, 1);
         try { for (;;) document.createTextNode('x'); } catch (e) {}
         deep.remove();
         show(r)",
    );
    assert_eq!(e, Err(DomError::HandleLimit { limit: 40 }));
}

/// Every range sits under the removed subtree, in a wide and deep tree: each
/// removal is the worst walk there is, and the sandbox's limit still ends it.
#[test]
fn many_ranges_in_a_deep_tree_end_at_the_time_limit() {
    let mut rt = DomRuntime::new(limits().with_time(scaled(Duration::from_millis(200)))).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let start = Instant::now();
    let r: Result<String, DomError> = rt.eval(
        "let at = document.body;
         for (let i = 0; i < 500; i++) at = at.appendChild(document.createElement('div'));
         const leaf = at.appendChild(document.createTextNode('x'));
         for (let i = 0; i < 60000; i++) { const r = document.createRange(); r.setStart(leaf, 0); r.setEnd(leaf, 1); }
         for (;;) {
           document.body.appendChild(document.body.firstChild);
           const b = document.body.lastChild; b.remove(); document.body.appendChild(b);
         }",
    );
    assert_eq!(r, Err(DomError::Sandbox(SandboxError::TimeLimit)));
    assert!(start.elapsed() < scaled(Duration::from_secs(5)));
}

/// `extractContents` on a wide list with many ranges on the same node pays
/// for both in its polls, and ends at the time limit, not after it.
#[test]
fn a_wide_extract_stops_near_the_time_limit() {
    let mut rt = DomRuntime::new(limits().with_time(scaled(Duration::from_millis(100)))).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let start = Instant::now();
    let r: Result<u32, DomError> = rt.eval(
        "const box = document.body.appendChild(document.createElement('div'));
         for (let i = 0; i < 30000; i++) box.appendChild(document.createElement('i'));
         for (let i = 0; i < 2000; i++) { const r = document.createRange(); r.selectNodeContents(box); }
         const r = document.createRange(); r.selectNodeContents(box);
         r.extractContents().childNodes.length",
    );
    assert!(
        matches!(
            r,
            Ok(30000) | Err(DomError::Sandbox(SandboxError::TimeLimit))
        ),
        "{r:?}"
    );
    assert!(start.elapsed() < scaled(Duration::from_secs(5)));
}

/// A leaked range per label must not make the page's other mutations slow:
/// nodes no range touches pay one lookup.
#[test]
fn leaked_ranges_cost_untouched_mutations_nothing() {
    let time = Limits::default().time;
    let mut rt = DomRuntime::new(limits().with_time(scaled(time * 4))).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let r: u32 = rt
        .eval(
            "const box = document.body.appendChild(document.createElement('div'));
             for (let i = 0; i < 20000; i++) {
               const t = box.appendChild(document.createElement('i')).appendChild(document.createTextNode('label'));
               const r = document.createRange(); r.selectNodeContents(t);
             }
             const out = document.body.appendChild(document.createElement('div'));
             for (let i = 0; i < 20000; i++) {
               const c = out.appendChild(document.createElement('b'));
               c.textContent = 'x'; c.remove();
             }
             out.childNodes.length",
        )
        .unwrap();
    assert_eq!(r, 0);
}
