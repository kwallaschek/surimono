//! Several documents in one render (DOM §4.5, §4.5.1). Expected values come
//! from the DOM Standard's algorithms; hostile cases expect typed limits.

mod common;

use std::time::{Duration, Instant};

use common::limits;
use surimono_core::{Limits, SandboxError};
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

const PAGE: &str = "<!doctype html><html><head></head><body><p id=a class=c>x</p></body></html>";

fn runtime() -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    rt
}

fn eval(script: &str) -> String {
    let mut rt = runtime();
    let got = rt.eval::<String>(script).unwrap();
    rt.with_document(|d| d.check_consistency().unwrap());
    got
}

fn limited(limits: DomLimits, script: &str) -> DomError {
    let mut rt = DomRuntime::new(common::limits()).unwrap();
    rt.load(Document::parse_with_limits(PAGE, limits).unwrap());
    rt.eval::<()>(script).unwrap_err()
}

#[test]
fn the_page_has_its_doctype() {
    let got = eval(
        "const d = document.doctype;
         [d.nodeType, d.nodeName, d.name, d.publicId === '', d.systemId === '', d.parentNode === document,
          document.firstChild === d, d.nextSibling === document.documentElement,
          document.childNodes.length, d instanceof DocumentType, d.ownerDocument === document,
          d.textContent, d.nodeValue].join()",
    );
    assert_eq!(got, "10,html,html,true,true,true,true,true,2,true,true,,");
}

#[test]
fn a_public_and_system_id_are_kept() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(
        Document::parse(r#"<!DOCTYPE html PUBLIC "-//W3C//DTD HTML 4.01//EN" "http://x/y"><p>"#)
            .unwrap(),
    );
    let got: String = rt
        .eval("const d = document.doctype; [d.publicId, d.systemId].join()")
        .unwrap();
    assert_eq!(got, "-//W3C//DTD HTML 4.01//EN,http://x/y");
}

#[test]
fn a_page_with_no_doctype_is_in_quirks_mode_and_has_none() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse("<p>").unwrap());
    let got: String = rt
        .eval("[document.doctype, document.compatMode].join()")
        .unwrap();
    assert_eq!(got, ",BackCompat");
}

#[test]
fn new_document_is_an_empty_xml_document() {
    let got = eval(
        "const d = new Document();
         [d instanceof Document, d instanceof XMLDocument, d.constructor === Document, d.nodeType,
          d.nodeName, d.URL, d.documentURI, d.contentType, d.characterSet, d.charset,
          d.inputEncoding, d.compatMode, d.readyState, d.doctype, d.documentElement, d.body,
          d.head, d.firstChild, d.ownerDocument, d.isConnected, d.title === ''].join()",
    );
    assert_eq!(
        got,
        "true,false,true,9,#document,about:blank,about:blank,application/xml,UTF-8,UTF-8,UTF-8,\
         CSS1Compat,complete,,,,,,,true,true"
    );
}

/// Each Document has its own identity, and a subclass works (WebIDL §3.7.1).
#[test]
fn new_document_subclasses_and_identity() {
    let got = eval(
        "class D extends Document { hi() { return 'hi'; } }
         const d = new D(), e = new Document();
         [d.hi(), d instanceof D, d === e, d.implementation === d.implementation,
          d.implementation === document.implementation].join()",
    );
    assert_eq!(got, "hi,true,false,true,false");
}

#[test]
fn elements_in_an_xml_document_keep_their_case_and_have_no_namespace() {
    let got = eval(
        "const d = new Document();
         const e = d.createElement('FooBar');
         e.setAttribute('AbC', '1');
         d.appendChild(e);
         [e.ownerDocument === d, e.tagName, e.localName, e.namespaceURI, e.getAttribute('AbC'),
          e.getAttribute('abc'), e.getAttributeNames(), d.getElementsByTagName('FooBar').length,
          d.getElementsByTagName('foobar').length, d.querySelector('FooBar') === e,
          d.querySelector('foobar'), d.documentElement === e, e.matches(':root')].join()",
    );
    assert_eq!(got, "true,FooBar,FooBar,,1,,AbC,1,0,true,,true,true");
}

#[test]
fn html_elements_in_an_xml_document_are_not_folded() {
    let got = eval(
        "const d = new Document();
         const e = d.createElementNS('http://www.w3.org/1999/xhtml', 'DIV');
         [e.tagName, e.localName].join()",
    );
    assert_eq!(got, "DIV,DIV");
}

#[test]
fn create_html_document() {
    let got = eval(
        "const d = document.implementation.createHTMLDocument('t <b>');
         [d instanceof Document, d instanceof XMLDocument, d.contentType, d.URL, d.compatMode,
          d.doctype.name, d.title, d.documentElement.outerHTML, d.head.ownerDocument === d,
          d.body.tagName, d.documentElement.namespaceURI, d.characterSet, d.readyState].join('|')",
    );
    assert_eq!(
        got,
        "true|false|text/html|about:blank|CSS1Compat|html|t <b>|\
         <html><head><title>t &lt;b&gt;</title></head><body></body></html>|true|BODY|\
         http://www.w3.org/1999/xhtml|UTF-8|complete"
    );
}

#[test]
fn create_html_document_without_a_title_has_no_title_element() {
    let got = eval(
        "const d = document.implementation.createHTMLDocument();
         [d.getElementsByTagName('title').length, d.title === '', d.head.childNodes.length,
          document.implementation.createHTMLDocument('').getElementsByTagName('title').length,
          document.implementation.hasFeature()].join()",
    );
    assert_eq!(got, "0,true,0,1,true");
}

#[test]
fn create_document() {
    let got = eval(
        "const i = document.implementation;
         const t = i.createDocumentType('a', 'b', 'c');
         const d = i.createDocument('urn:x', 'p:q', t);
         const html = i.createDocument('http://www.w3.org/1999/xhtml', 'html', null);
         const svg = i.createDocument('http://www.w3.org/2000/svg', 'svg');
         const none = i.createDocument(null, null);
         [d instanceof XMLDocument, d.contentType, d.doctype === t, t.ownerDocument === d,
          t.parentNode === d, d.documentElement.tagName, d.documentElement.namespaceURI,
          html.contentType, svg.contentType, none.documentElement, none.childNodes.length,
          none.contentType, none.constructor === XMLDocument].join('|')",
    );
    assert_eq!(
        got,
        "true|application/xml|true|true|true|p:q|urn:x|application/xhtml+xml|image/svg+xml||0|\
         application/xml|true"
    );
}

#[test]
fn create_document_rejects_bad_names_before_making_anything() {
    let got = eval(
        "const i = document.implementation, out = [];
         for (const f of [() => i.createDocument(null, '1a'), () => i.createDocument(null, 'a:b'),
                          () => i.createDocumentType('a b', '', ''),
                          () => i.createDocumentType('a>', '', '')]) {
           try { f(); out.push('no'); } catch (e) { out.push(e.name); }
         }
         out.join()",
    );
    assert_eq!(
        got,
        "InvalidCharacterError,NamespaceError,InvalidCharacterError,InvalidCharacterError"
    );
}

#[test]
fn cdata_and_processing_instructions() {
    let got = eval(
        "const d = new Document(), out = [];
         const c = d.createCDATASection('a<b');
         const p = d.createProcessingInstruction('t', 'data');
         out.push(c.nodeType, c.nodeName, c.data, c instanceof CDATASection, c instanceof Text,
                  c.ownerDocument === d, p.nodeType, p.nodeName, p.target, p.data, p.nodeValue,
                  p.textContent, p instanceof ProcessingInstruction, p instanceof CharacterData);
         p.data = 'x'; out.push(p.length);
         for (const f of [() => document.createCDATASection('a'), () => d.createCDATASection(']]>'),
                          () => d.createProcessingInstruction('1', ''),
                          () => d.createProcessingInstruction('a', '?>')]) {
           try { f(); out.push('no'); } catch (e) { out.push(e.name); }
         }
         out.push(c.cloneNode().nodeType, p.cloneNode().target, p.cloneNode().ownerDocument === d,
                  c.isEqualNode(d.createTextNode('a<b')), p.isEqualNode(p.cloneNode()));
         out.join()",
    );
    assert_eq!(
        got,
        "4,#cdata-section,a<b,true,true,true,7,t,t,data,data,data,true,true,1,\
         NotSupportedError,InvalidCharacterError,InvalidCharacterError,InvalidCharacterError,\
         4,t,true,false,true"
    );
}

/// §4.4: only exclusive Text nodes are merged.
#[test]
fn normalize_leaves_cdata_alone() {
    let got = eval(
        "const d = new Document(), e = d.createElement('e');
         e.append('a', d.createCDATASection('b'), 'c', 'd');
         e.normalize();
         [e.childNodes.length, e.lastChild.data, e.childNodes[1].nodeType].join()",
    );
    assert_eq!(got, "3,cd,4");
}

#[test]
fn adopt_node_moves_a_subtree_and_keeps_the_wrappers() {
    let got = eval(
        "const d = new Document();
         const a = document.getElementById('a'), t = a.firstChild, attr = a.attributes[0];
         const r = d.adoptNode(a);
         [r === a, a.parentNode, a.ownerDocument === d, t.ownerDocument === d,
          attr.ownerDocument === d, t.parentNode === a, document.getElementById('a')].join()",
    );
    assert_eq!(got, "true,,true,true,true,true,");
}

#[test]
fn inserting_adopts() {
    let got = eval(
        "const d = document.implementation.createHTMLDocument();
         const a = document.getElementById('a'), f = document.createDocumentFragment();
         const b = document.createElement('b');
         f.append(b, 'x');
         d.body.appendChild(a);
         d.body.appendChild(f);
         const back = d.body.firstChild;
         document.body.appendChild(back);
         [a.ownerDocument === document, b.ownerDocument === d, b.nextSibling.ownerDocument === d,
          f.ownerDocument === d, f.childNodes.length, a.tagName].join()",
    );
    assert_eq!(got, "true,true,true,true,0,P");
}

#[test]
fn adoption_changes_how_names_are_treated() {
    let got = eval(
        "const d = new Document();
         const e = document.createElement('p');
         e.setAttribute('a', '1');
         const before = e.tagName;
         d.adoptNode(e);
         e.setAttribute('Bc', '2');
         [before, e.tagName, e.getAttribute('Bc')].join()",
    );
    assert_eq!(got, "P,p,2");
}

#[test]
fn import_node_copies_into_this_document() {
    let got = eval(
        "const d = new Document();
         const a = document.getElementById('a');
         const shallow = d.importNode(a), deep = d.importNode(a, true);
         const out = [shallow.ownerDocument === d, shallow.firstChild, deep.firstChild.data,
                      deep.firstChild.ownerDocument === d, a.ownerDocument === document];
         try { d.importNode(document); } catch (e) { out.push(e.name); }
         try { d.adoptNode(new Document()); } catch (e) { out.push(e.name); }
         out.join()",
    );
    assert_eq!(got, "true,,x,true,true,NotSupportedError,NotSupportedError");
}

#[test]
fn new_nodes_belong_to_the_document_of_this() {
    let got = eval(
        "const d = document.implementation.createHTMLDocument('');
         const e = d.createElement('div');
         e.innerHTML = '<b>x</b><template><i></i></template>';
         e.append('s');
         const b = e.firstChild, split = b.firstChild.splitText(0);
         const out = [b.ownerDocument === d, b.firstChild.ownerDocument === d,
                      e.lastChild.ownerDocument === d, split.ownerDocument === d,
                      e.children[1].content.firstChild.ownerDocument === d,
                      e.cloneNode(true).firstChild.ownerDocument === d,
                      d.createTextNode('t').ownerDocument === d,
                      d.createComment('c').ownerDocument === d,
                      d.createDocumentFragment().ownerDocument === d,
                      new Text('x').ownerDocument === document];
         d.body.textContent = 'y';
         out.push(d.body.firstChild.ownerDocument === d);
         out.join()",
    );
    assert_eq!(
        got,
        "true,true,true,true,true,true,true,true,true,true,true"
    );
}

#[test]
fn queries_are_scoped_to_this_document() {
    let got = eval(
        "const d = document.implementation.createHTMLDocument('');
         d.body.innerHTML = '<i id=a class=c></i>';
         const i = d.body.firstChild;
         [d.getElementById('a') === i, document.getElementById('a').tagName,
          d.querySelectorAll('.c').length, document.querySelectorAll('i').length,
          d.getElementsByClassName('c').length, d.getElementsByTagName('i')[0] === i,
          i.isConnected, i.matches('html > body > i'), i.getRootNode() === d,
          d.body.contains(i), document.contains(i),
          document.compareDocumentPosition(d) & 1].join()",
    );
    assert_eq!(got, "true,P,1,0,1,true,true,true,true,true,false,1");
}

/// §4.2.3 step 6, and "replace".
#[test]
fn what_a_document_may_contain() {
    let got = eval(
        "const i = document.implementation, out = [];
         const name = (f) => { try { f(); return 'ok'; } catch (e) { return e.name; } };
         const fresh = () => {
           const d = i.createDocument(null, null);
           return [d, d.appendChild(i.createDocumentType('a', '', '')), d.appendChild(d.createElement('e'))];
         };
         { const [d, t, e] = fresh();
           out.push(name(() => d.appendChild(d.createElement('x'))),
                    name(() => d.appendChild(d.createTextNode('x'))),
                    name(() => d.appendChild(i.createDocumentType('b', '', ''))),
                    name(() => d.insertBefore(d.createComment('c'), t)),
                    name(() => d.replaceChild(d.createElement('x'), t)),
                    name(() => d.replaceChild(i.createDocumentType('b', '', ''), e)),
                    name(() => d.replaceChild(i.createDocumentType('b', '', ''), t))); }
         { const d = i.createDocument(null, null), e = d.appendChild(d.createElement('e'));
           out.push(name(() => d.insertBefore(i.createDocumentType('b', '', ''), e)));
           const d2 = i.createDocument(null, null), e2 = d2.appendChild(d2.createElement('e'));
           out.push(name(() => d2.appendChild(i.createDocumentType('b', '', '')))); }
         { const d = i.createDocument(null, null), c = d.appendChild(d.createComment('c'));
           d.appendChild(i.createDocumentType('b', '', ''));
           out.push(name(() => d.insertBefore(d.createElement('e'), c)));
           const f = d.createDocumentFragment(); f.append(d.createElement('e'), d.createElement('e'));
           out.push(name(() => d.appendChild(f))); }
         out.push(name(() => document.body.appendChild(i.createDocumentType('b', '', ''))),
                  name(() => document.body.appendChild(new Document())));
         out.join()",
    );
    assert_eq!(
        got,
        "HierarchyRequestError,HierarchyRequestError,HierarchyRequestError,ok,\
         HierarchyRequestError,HierarchyRequestError,ok,ok,HierarchyRequestError,\
         HierarchyRequestError,HierarchyRequestError,HierarchyRequestError,HierarchyRequestError"
    );
}

#[test]
fn a_doctype_is_a_child_node() {
    let got = eval(
        "const t = document.doctype, c = document.createComment('c');
         t.before(c);
         const first = document.firstChild === c;
         t.remove();
         [first, document.doctype, t.parentNode, t.ownerDocument === document,
          t.cloneNode().name, t.isEqualNode(t.cloneNode()),
          t.isEqualNode(document.implementation.createDocumentType('html', 'x', ''))].join()",
    );
    assert_eq!(got, "true,,,true,html,true,false");
}

#[test]
fn events_in_another_document_stay_out_of_the_window() {
    let got = eval(
        "const d = document.implementation.createHTMLDocument('');
         let seen = [];
         addEventListener('x', () => seen.push('window'));
         d.addEventListener('x', () => seen.push('document'));
         d.body.addEventListener('x', () => seen.push('body'));
         d.body.dispatchEvent(new Event('x', { bubbles: true }));
         seen.join()",
    );
    assert_eq!(got, "body,document");
}

#[test]
fn observers_see_other_documents() {
    let mut rt = runtime();
    let _: () = rt
        .eval(
            "globalThis.d = new Document(); globalThis.n = 0;
             new MutationObserver((r) => { n += r.length; }).observe(d, { childList: true });
             d.appendChild(d.createElement('e'));",
        )
        .unwrap();
    assert_eq!(rt.eval::<i32>("n").unwrap(), 1);
}

#[test]
fn the_output_is_the_page_alone() {
    let mut rt = runtime();
    let _: () = rt
        .eval(
            "const d = document.implementation.createHTMLDocument('other');
             d.body.appendChild(document.getElementById('a'));",
        )
        .unwrap();
    let html = rt.with_document(Document::to_html).unwrap();
    assert_eq!(html, "<html><head></head><body></body></html>");
}

#[test]
fn set_url_reaches_document_url() {
    let mut doc = Document::parse(PAGE).unwrap();
    doc.set_url("https://example.test/a?b");
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(doc);
    assert_eq!(
        rt.eval::<String>("document.URL + ' ' + new Document().URL")
            .unwrap(),
        "https://example.test/a?b about:blank"
    );
}

// ------ hostile ------

/// All documents share one node limit.
#[test]
fn a_document_bomb_hits_the_node_limit() {
    let l = DomLimits {
        max_nodes: 5_000,
        ..DomLimits::default()
    };
    for script in [
        "for (;;) new Document();",
        "for (;;) document.implementation.createHTMLDocument('x');",
        "for (;;) document.implementation.createDocument('urn:x', 'a');",
        "for (;;) document.cloneNode(true);",
    ] {
        assert_eq!(
            limited(l, script),
            DomError::NodeLimit { limit: 5_000 },
            "{script}"
        );
    }
}

/// The byte limit too: strings in any document count.
#[test]
fn strings_in_other_documents_count_together() {
    const CAP: usize = 1 << 20;
    let l = DomLimits {
        max_text_bytes: CAP,
        ..DomLimits::default()
    };
    for script in [
        "const s = 'x'.repeat(1 << 16); const k = [];
         for (;;) k.push(new Document().createComment(s));",
        "const s = 'x'.repeat(1 << 16);
         for (;;) document.implementation.createHTMLDocument(s);",
        "const s = 'x'.repeat(1 << 16);
         for (;;) document.implementation.createDocumentType(s, s, s);",
        "const s = 'x'.repeat(1 << 16); const d = new Document();
         for (;;) d.createProcessingInstruction('a' + s, s);",
        "const s = 'x'.repeat(1 << 16); const d = new Document();
         for (;;) d.createCDATASection(s);",
    ] {
        assert_eq!(
            limited(l, script),
            DomError::TextLimit { limit: CAP },
            "{script}"
        );
    }
}

/// A freed document string gives its charge back, and the count stays right
/// through adoption and drops.
#[test]
fn byte_counts_survive_moves_between_documents() {
    let got = eval(
        "const d = new Document();
         for (let i = 0; i < 200; i++) {
           const e = d.createElement('e');
           e.innerHTML = '<b>x</b><!--c-->';
           document.body.appendChild(e);
           e.textContent = '';
           d.adoptNode(e);
           d.createProcessingInstruction('a', 'b');
         }
         document.body.textContent = 'y';
         'ok'",
    );
    assert_eq!(got, "ok");
}

/// Handles are charged to one table whichever document asks.
#[test]
fn handles_are_shared() {
    let l = DomLimits {
        max_handles: 1_000,
        ..DomLimits::default()
    };
    assert_eq!(
        limited(l, "const k = []; for (;;) k.push(new Document());"),
        DomError::HandleLimit { limit: 1_000 }
    );
}

#[test]
fn depth_is_checked_in_other_documents() {
    let l = DomLimits {
        max_depth: 40,
        ..DomLimits::default()
    };
    assert_eq!(
        limited(
            l,
            "const d = new Document(); let at = d.appendChild(d.createElement('e'));
             for (;;) at = at.appendChild(d.createElement('e'));"
        ),
        DomError::TreeTooDeep { limit: 40 }
    );
    // A tree deep enough for a detached home doesn't fit under a document.
    assert_eq!(
        limited(
            l,
            "const d = new Document(); const top = d.createElement('e'); let at = top;
             for (let i = 0; i < 39; i++) at = at.appendChild(d.createElement('e'));
             document.body.appendChild(top);"
        ),
        DomError::TreeTooDeep { limit: 40 }
    );
}

/// Moving a big subtree back and forth takes linear work per move, and the
/// time limit ends the loop.
#[test]
fn an_adoption_loop_ends_at_the_time_limit() {
    let mut rt = DomRuntime::new(Limits::default().with_time(Duration::from_millis(200))).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let started = Instant::now();
    let e = rt
        .eval::<()>(
            "const d = new Document(), top = document.createElement('div');
             for (let i = 0; i < 20000; i++) top.appendChild(document.createElement('i'));
             for (;;) { d.adoptNode(top); document.adoptNode(top); }",
        )
        .unwrap_err();
    assert!(
        matches!(e, DomError::Sandbox(SandboxError::TimeLimit)),
        "{e:?}"
    );
    assert!(started.elapsed() < common::scaled(Duration::from_secs(3)));
}

/// Wrappers of every document go stale together.
#[test]
fn documents_go_with_the_render() {
    let mut rt = runtime();
    let _: () = rt
        .eval("globalThis.d = new Document(); globalThis.e = d.createElement('e');")
        .unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    for q in ["d.createElement('x')", "e.tagName", "d.firstChild"] {
        assert_eq!(
            rt.eval::<()>(&format!("try {{ {q}; }} catch (x) {{}}")),
            Err(DomError::StaleDocument),
            "{q}"
        );
    }
}

/// Nothing a script does to its own documents reaches another render.
#[test]
fn a_render_leaves_nothing_behind() {
    let mut rt = runtime();
    for _ in 0..50 {
        rt.reset().unwrap();
        rt.load(Document::parse(PAGE).unwrap());
        let n: i32 = rt
            .eval(
                "const d = document.implementation.createHTMLDocument('t');
                 d.body.appendChild(document.getElementById('a'));
                 document.getElementsByTagName('p').length",
            )
            .unwrap();
        assert_eq!(n, 0);
    }
}
