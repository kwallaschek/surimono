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

// ------ DOMParser, text/html (HTML §8.5.3) ------

#[test]
fn dom_parser_gives_a_new_html_document() {
    let got = eval(
        r#"const d = new DOMParser().parseFromString('<!doctype html><title>t</title><p id=q>x', 'text/html');
         [d instanceof Document, d === document, d.contentType, d.compatMode, d.doctype.name, d.title,
          d.getElementById('q').textContent, d.body.ownerDocument === d, d.URL === document.URL,
          d.readyState, d.documentElement.isConnected, document.getElementById('q'),
          d.defaultView, d.location].join()"#,
    );
    assert_eq!(
        got,
        "true,false,text/html,CSS1Compat,html,t,x,true,true,complete,true,,,"
    );
}

/// HTML §8.5.3: no browsing context, so scripts don't run and the scripting
/// flag is off.
#[test]
fn dom_parser_runs_no_script_and_parses_noscript_as_markup() {
    let got = eval(
        r#"globalThis.ran = 0;
         const d = new DOMParser().parseFromString(
           '<script>ran = 1</script><body onload="ran = 2"><noscript><b>x</b></noscript><img src=x onerror="ran = 3">',
           'text/html');
         [ran, d.querySelector('noscript').firstChild.nodeName,
          d.scripts === undefined ? d.getElementsByTagName('script').length : 0].join()"#,
    );
    assert_eq!(got, "0,B,1");
}

#[test]
fn dom_parser_quirks_and_templates() {
    let got = eval(
        r#"const d = new DOMParser().parseFromString('<template><b>x</b></template>', 'text/html');
         const t = d.querySelector('template');
         [d.compatMode, t.content.firstChild.tagName, t.content.firstChild.ownerDocument === d,
          d.doctype].join()"#,
    );
    assert_eq!(got, "BackCompat,B,true,");
}

#[test]
fn dom_parser_checks_its_type_first() {
    let got = eval(
        "const out = [];
         for (const t of ['text/plain', 'TEXT/HTML', '', undefined]) {
           try { new DOMParser().parseFromString('', t); out.push('no'); }
           catch (e) { out.push(e.name); }
         }
         try { DOMParser.prototype.parseFromString.call({}, '', 'text/html'); }
         catch (e) { out.push(e.name); }
         out.push(typeof new DOMParser());
         out.join()",
    );
    assert_eq!(
        got,
        "TypeError,TypeError,TypeError,TypeError,TypeError,object"
    );
}

/// All of a render's parses share the limits, and a refused one leaves the
/// document empty and the counts right.
#[test]
fn a_parser_bomb_hits_the_limits() {
    let nodes = DomLimits {
        max_nodes: 5_000,
        ..DomLimits::default()
    };
    assert_eq!(
        limited(
            nodes,
            "const p = new DOMParser(); for (;;) p.parseFromString('<p>x', 'text/html');"
        ),
        DomError::NodeLimit { limit: 5_000 }
    );
    assert_eq!(
        limited(
            nodes,
            "new DOMParser().parseFromString('<p>'.repeat(10000), 'text/html');"
        ),
        DomError::NodeLimit { limit: 5_000 }
    );
    let deep = DomLimits {
        max_depth: 64,
        ..DomLimits::default()
    };
    assert_eq!(
        limited(
            deep,
            "new DOMParser().parseFromString('<div>'.repeat(4000), 'text/html');"
        ),
        DomError::TreeTooDeep { limit: 64 }
    );
    const CAP: usize = 1 << 20;
    let text = DomLimits {
        max_text_bytes: CAP,
        ..DomLimits::default()
    };
    assert_eq!(
        limited(
            text,
            "const s = '<p title=' + 'x'.repeat(1 << 18) + '>'; const p = new DOMParser();
             for (;;) p.parseFromString(s, 'text/html');"
        ),
        DomError::TextLimit { limit: CAP }
    );
}

#[test]
fn a_refused_parse_leaves_consistent_counts() {
    let l = DomLimits {
        max_nodes: 5_000,
        ..DomLimits::default()
    };
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse_with_limits(PAGE, l).unwrap());
    let got: String = rt
        .eval(
            "try { new DOMParser().parseFromString('<p>'.repeat(10000), 'text/html'); } catch (e) {}
             'after'",
        )
        .unwrap_or_else(|e| e.to_string());
    assert!(got.contains("5000"), "{got}");
    rt.with_document(|d| d.check_consistency().unwrap());
}

#[test]
fn a_parse_of_a_big_page_ends_at_the_time_limit() {
    let mut rt = DomRuntime::new(Limits::default().with_time(Duration::from_millis(50))).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let e = rt
        .eval::<()>(
            "const s = '<p>'.repeat(90000); const p = new DOMParser();
             for (;;) p.parseFromString(s, 'text/html');",
        )
        .unwrap_err();
    assert!(
        matches!(
            e,
            DomError::Sandbox(SandboxError::TimeLimit) | DomError::NodeLimit { .. }
        ),
        "{e:?}"
    );
}

/// A hostile `style` attribute or `<style>` fares no better here.
#[test]
fn a_parsed_document_keeps_the_page_rules_for_style() {
    let got = eval(
        r#"const d = new DOMParser().parseFromString(
             '<style>@import "http://[";</style><p style="color: red">', 'text/html');
         [d.querySelector('style').textContent, d.querySelector('p').style.color].join()"#,
    );
    assert_eq!(got, r#"@import "http://[";,red"#);
}

// ------ DOMParser for XML, and the XML parser ------

/// `parse(source, type)`'s document, by the script `then`.
fn xml(source: &str, then: &str) -> String {
    eval(&format!(
        "const d = new DOMParser().parseFromString({source:?}, 'application/xml'); {then}"
    ))
}

#[test]
fn xml_keeps_case_namespaces_and_node_kinds() {
    let got = xml(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- top --><?pi data?>
<!DOCTYPE Root PUBLIC "pub" "sys" [ <!ENTITY x "y"> <!-- ] --> ]>
<Root xmlns="urn:d" xmlns:p="urn:p" p:a="1" b="2">
  text &lt;&#65;&#x42;&amp;<p:child/><![CDATA[<raw>]]><!--c--><?t?>
</Root>
<!-- end -->"#,
        "const r = d.documentElement, c = r.children[0];
         [d instanceof XMLDocument, d.contentType, d.doctype.name, d.doctype.publicId,
          d.doctype.systemId, d.childNodes.length, r.tagName, r.namespaceURI, r.localName,
          r.getAttributeNS('urn:p', 'a'), r.getAttribute('b'), r.attributes[1].namespaceURI,
          c.tagName, c.prefix, c.namespaceURI,
          [...r.childNodes].map(n => n.nodeType).join(''), r.firstChild.data.trim(),
          r.childNodes[2].data, r.childNodes[4].target, r.childNodes[4].data === '',
          d.firstChild.data, d.childNodes[1].target, d.lastChild.data].join('|')",
    );
    assert_eq!(
        got,
        "true|application/xml|Root|pub|sys|5|Root|urn:d|Root|1|2|\
         http://www.w3.org/2000/xmlns/|p:child|p|urn:p|314873|text <AB&|<raw>|t|true| top |pi| end "
    );
}

#[test]
fn xml_normalizes_line_ends_and_attribute_values() {
    let got = xml(
        "<a b='1\t2\n3\r\n4&#10;5'>x\r\ny\rz</a>",
        "const a = d.documentElement; JSON.stringify([a.getAttribute('b'), a.textContent])",
    );
    assert_eq!(got, r#"["1 2 3 4\n5","x\ny\nz"]"#);
}

#[test]
fn every_document_type_parses_as_xml() {
    let got = eval(
        "const out = [];
         for (const t of ['text/xml', 'application/xml', 'application/xhtml+xml', 'image/svg+xml']) {
           const d = new DOMParser().parseFromString('<a/>', t);
           out.push(d.contentType, d instanceof XMLDocument, d.documentElement.tagName);
         }
         out.join()",
    );
    assert_eq!(
        got,
        "text/xml,true,a,application/xml,true,a,application/xhtml+xml,true,a,image/svg+xml,true,a"
    );
}

#[test]
fn malformed_xml_gives_a_parsererror_document() {
    for source in [
        "",
        "   ",
        "<a>",
        "<a></b>",
        "</a>",
        "<a/><b/>",
        "text<a/>",
        "<a/>text",
        "<a b=1/>",
        "<a b/>",
        "<a b='1' b='2'/>",
        "<a xmlns:p='u' xmlns:q='u' p:x='1' q:x='2'/>",
        "<p:a/>",
        "<a p:b='1'/>",
        "<a xmlns:p=''/>",
        "<a xmlns:xmlns='u'/>",
        "<a xmlns:xml='u'/>",
        "<a>&undefined;</a>",
        "<a>&#0;</a>",
        "<a>&#xD800;</a>",
        "<a>&#+65;</a>",
        "<a b='&undefined;'/>",
        "<a b='<'/>",
        "<a>]]></a>",
        "<a><!-- -- --></a>",
        "<a><!--",
        "<a><![CDATA[",
        "<a><?xml x?></a>",
        "<a><?pi",
        "<?xml?><a/>",
        "<?xml version='2.0'?><a/>",
        "<?xml encoding='UTF-8'?><a/>",
        " <?xml version='1.0'?><a/>",
        "<!DOCTYPE a><!DOCTYPE b><a/>",
        "<a/><!DOCTYPE a>",
        "<![CDATA[x]]><a/>",
        "<a:b:c xmlns:a='u'/>",
        "<1a/>",
        "<a>\u{1}</a>",
    ] {
        let got = xml(
            source,
            "const r = d.documentElement;
             [d.childNodes.length, r.localName, r.namespaceURI, r.children.length].join()",
        );
        assert_eq!(
            got, "1,parsererror,http://www.mozilla.org/newlayout/xml/parsererror.xml,0",
            "{source}"
        );
    }
}

#[test]
fn well_formed_corners() {
    for (source, script, expected) in [
        ("\u{feff}<a/>", "d.documentElement.tagName", "a"),
        (
            "<a><?pi  x ?></a>",
            "d.documentElement.firstChild.data",
            "x ",
        ),
        (
            "<a xmlns=''/>",
            "d.documentElement.namespaceURI === null",
            "true",
        ),
        (
            "<a xmlns='urn:x'><b xmlns=''/></a>",
            "d.documentElement.firstChild.namespaceURI === null",
            "true",
        ),
        (
            "<a xml:lang='ja'/>",
            "d.documentElement.attributes[0].namespaceURI",
            "http://www.w3.org/XML/1998/namespace",
        ),
        (
            "<a  b = '1'  ></a  >",
            "d.documentElement.getAttribute('b')",
            "1",
        ),
        (
            "<a>&#x1F600;</a>",
            "d.documentElement.textContent",
            "\u{1F600}",
        ),
        ("<!DOCTYPE a SYSTEM 's'><a/>", "d.doctype.systemId", "s"),
        ("<!-- x --><a/>", "d.childNodes.length", "2"),
        (
            "<a><b xmlns:p='u'/><c/></a>",
            "d.documentElement.children.length",
            "2",
        ),
    ] {
        assert_eq!(
            xml(source, &format!("String({script})")),
            expected,
            "{source}"
        );
    }
}

#[test]
fn a_scope_ends_with_its_element() {
    let got = xml(
        "<a><b xmlns:p='u'><p:c/></b><p:d/></a>",
        "d.documentElement.localName",
    );
    assert_eq!(got, "parsererror");
}

/// No entity expands, so there is no bomb to defuse: it is an error.
#[test]
fn an_entity_bomb_is_a_parse_error() {
    let source =
        r#"<!DOCTYPE b [ <!ENTITY a "aaaaaaaaaa"><!ENTITY c "&a;&a;&a;&a;&a;&a;"> ]><b>&c;</b>"#;
    assert_eq!(xml(source, "d.documentElement.localName"), "parsererror");
}

#[test]
fn xml_in_a_style_attribute_and_a_style_element_keeps_the_rules() {
    let got = xml(
        r#"<a xmlns="http://www.w3.org/1999/xhtml"><style>@import "http://[";</style><b style="color: red"/></a>"#,
        "const a = d.documentElement;
         [a.firstChild.textContent, a.lastChild.style.color].join()",
    );
    assert_eq!(got, r#"@import "http://[";,red"#);
}

#[test]
fn inner_html_in_an_xml_document_parses_xml() {
    let got = eval(
        r#"const d = new DOMParser().parseFromString('<a xmlns="urn:x" xmlns:p="urn:p"/>', 'application/xml');
         const a = d.documentElement, out = [];
         a.innerHTML = '<B/>text<p:c/>';
         out.push(a.childNodes.length, a.firstChild.namespaceURI, a.firstChild.tagName,
                  a.lastChild.namespaceURI, a.firstChild.ownerDocument === d);
         try { a.innerHTML = '<b>'; } catch (e) { out.push(e.name); }
         out.push(a.childNodes.length);
         out.join()"#,
    );
    assert_eq!(got, "3,urn:x,B,urn:p,true,SyntaxError,3");
}

#[test]
fn a_page_can_be_xml() {
    let mut doc = Document::parse_xml(
        r#"<!DOCTYPE html><html xmlns="http://www.w3.org/1999/xhtml"><body><P id="a">x</P></body></html>"#,
    )
    .unwrap();
    doc.set_url("https://x.test/a.xhtml");
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(doc);
    let got: String = rt
        .eval(
            "const p = document.getElementById('a');
             [document.contentType, document.constructor === Document, p.tagName, p.localName,
              document.body.tagName, document.getElementsByTagName('p').length,
              document.createElement('Div').tagName].join()",
        )
        .unwrap();
    assert_eq!(got, "application/xhtml+xml,true,P,P,body,0,Div");
}

#[test]
fn xml_hits_the_limits() {
    let deep = DomLimits {
        max_depth: 64,
        ..DomLimits::default()
    };
    assert_eq!(
        limited(
            deep,
            "new DOMParser().parseFromString('<a>'.repeat(4000), 'text/xml');"
        ),
        DomError::TreeTooDeep { limit: 64 }
    );
    let nodes = DomLimits {
        max_nodes: 5_000,
        ..DomLimits::default()
    };
    assert_eq!(
        limited(
            nodes,
            "new DOMParser().parseFromString('<a>' + '<b/>'.repeat(10000) + '</a>', 'text/xml');"
        ),
        DomError::NodeLimit { limit: 5_000 }
    );
    const CAP: usize = 1 << 20;
    let text = DomLimits {
        max_text_bytes: CAP,
        ..DomLimits::default()
    };
    assert_eq!(
        limited(
            text,
            "const s = '<a b=\"' + 'x'.repeat(1 << 19) + '\"/>'; const p = new DOMParser();
             for (;;) p.parseFromString(s, 'text/xml');"
        ),
        DomError::TextLimit { limit: CAP }
    );
    assert!(matches!(
        limited(
            DomLimits::default(),
            "new DOMParser().parseFromString('<a style=\"' + 'top:0;'.repeat(5000) + '\"/>', 'text/xml');"
        ),
        DomError::StyleLimit { .. }
    ));
}

/// Deep nesting stays off the stack, on the smallest thread a host gives us.
#[test]
fn xml_parsing_stays_off_the_stack() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let l = DomLimits {
                max_depth: DomLimits::MAX_DEPTH,
                ..DomLimits::default()
            };
            let source = format!("{}{}", "<a>".repeat(1000), "</a>".repeat(1000));
            let page = format!("<!doctype html><script>globalThis.s = {source:?}</script>");
            let mut rt = DomRuntime::new(limits()).unwrap();
            rt.load(Document::parse_with_limits(&page, l).unwrap());
            let mut rt2 = DomRuntime::new(limits()).unwrap();
            rt2.load(Document::parse_with_limits(PAGE, l).unwrap());
            let n: i32 = rt2
                .eval(&format!(
                    "const d = new DOMParser().parseFromString({source:?}, 'text/xml');
                     d.getElementsByTagName('a').length"
                ))
                .unwrap();
            assert_eq!(n, 1000);
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Quadratic shapes end at the time limit, not after it.
#[test]
fn hostile_xml_ends_at_the_time_limit() {
    for script in [
        // Many declarations in scope, each name looked up through them all.
        "let s = '<a'; for (let i = 0; i < 20000; i++) s += ' xmlns:p' + i + '=\"u\"'; s += '>';
         s += '<p0:b/>'.repeat(60000) + '</a>'; const p = new DOMParser();
         for (;;) p.parseFromString(s, 'text/xml');",
        // Long runs of names and attributes.
        "let s = '<a'; for (let i = 0; i < 40000; i++) s += ' b' + i + '=\"\"'; s += '/>';
         const p = new DOMParser(); for (;;) p.parseFromString(s, 'text/xml');",
    ] {
        let mut rt =
            DomRuntime::new(Limits::default().with_time(Duration::from_millis(300))).unwrap();
        rt.load(Document::parse(PAGE).unwrap());
        let started = Instant::now();
        let e = rt.eval::<()>(script).unwrap_err();
        assert!(
            matches!(
                e,
                DomError::Sandbox(SandboxError::TimeLimit)
                    | DomError::NodeLimit { .. }
                    | DomError::TextLimit { .. }
                    | DomError::HandleLimit { .. }
            ),
            "{e:?}"
        );
        assert!(started.elapsed() < common::scaled(Duration::from_secs(4)));
    }
}

// ------ XMLSerializer (DOM Parsing §3.2) ------

fn serialize(setup: &str) -> String {
    eval(&format!(
        "const s = (n) => new XMLSerializer().serializeToString(n); {setup}"
    ))
}

#[test]
fn xml_serialization() {
    for (setup, expected) in [
        (
            "s(document.createElement('br'))",
            r#"<br xmlns="http://www.w3.org/1999/xhtml" />"#,
        ),
        (
            "s(document.createElement('div'))",
            r#"<div xmlns="http://www.w3.org/1999/xhtml"></div>"#,
        ),
        ("s(new Document().createElement('a'))", "<a/>"),
        ("s(document.createTextNode('a<b&c>'))", "a&lt;b&amp;c&gt;"),
        ("s(document.createComment('x'))", "<!--x-->"),
        (
            "s(new Document().createProcessingInstruction('t', 'd'))",
            "<?t d?>",
        ),
        (
            "s(new Document().createCDATASection('a<'))",
            "<![CDATA[a<]]>",
        ),
        ("s(document.doctype)", "<!DOCTYPE html>"),
        (
            "s(document.implementation.createDocumentType('a', 'p', 's'))",
            r#"<!DOCTYPE a PUBLIC "p" "s">"#,
        ),
        (
            "s(document.implementation.createDocumentType('a', '', 's'))",
            r#"<!DOCTYPE a SYSTEM "s">"#,
        ),
        (
            "const d = document.implementation.createDocument('urn:x', 'p:a'); s(d)",
            r#"<p:a xmlns:p="urn:x"/>"#,
        ),
        (
            "const d = new Document(), a = d.createElement('a');
             a.setAttributeNS('urn:y', 'q:b', '1<\"\\t'); s(a)",
            r#"<a xmlns:q="urn:y" q:b="1&lt;&quot;&#9;"/>"#,
        ),
        (
            "const d = new Document(), a = d.createElementNS('urn:x', 'a');
             a.appendChild(d.createElementNS('urn:x', 'b')); s(a)",
            r#"<a xmlns="urn:x"><b/></a>"#,
        ),
        (
            "const d = new Document(), a = d.createElementNS('urn:x', 'a');
             a.appendChild(d.createElementNS(null, 'b')); s(a)",
            r#"<a xmlns="urn:x"><b xmlns=""/></a>"#,
        ),
    ] {
        assert_eq!(serialize(setup), expected, "{setup}");
    }
}

#[test]
fn serialized_xml_parses_back() {
    let got = serialize(
        "const d = document.implementation.createDocument('urn:x', 'p:a');
         const a = d.documentElement;
         a.setAttributeNS('urn:y', 'q:b', 'v');
         a.appendChild(d.createCDATASection('c'));
         a.appendChild(d.createComment('e'));
         a.appendChild(d.createProcessingInstruction('t', 'd'));
         const text = s(d);
         const back = new DOMParser().parseFromString(text, 'application/xml');
         [text === s(back), back.isEqualNode(d)].join()",
    );
    assert_eq!(got, "true,false");
}

// ------ iframes ------

fn page(html: &str, sources: &[(&str, &str)]) -> (DomRuntime, surimono_dom::PageReport) {
    let mut rt = DomRuntime::new(limits()).unwrap();
    let mut doc = Document::parse(html).unwrap();
    doc.set_url("https://x.test/dir/page.html");
    rt.load(doc);
    let report = rt
        .run_page(&surimono_core::SettleOptions::default(), |src| {
            sources
                .iter()
                .find(|(k, _)| *k == src)
                .map(|(_, v)| (*v).to_owned())
        })
        .unwrap();
    (rt, report)
}

const FRAME_PAGE: &str = r#"<!doctype html><body><script>
    globalThis.log = [];
    const f = document.createElement('iframe');
    f.onload = () => {
      const d = f.contentDocument;
      log.push(d.getElementById('in').textContent, d.URL, d.contentType,
               d.defaultView === f.contentWindow, f.contentWindow.document === d,
               d.defaultView.Node === Node, d.body.ownerDocument === d);
    };
    f.src = 'inner.html#target';
    document.body.appendChild(f);
    log.push(f.contentDocument.URL);
  </script>"#;

#[test]
fn an_iframe_loads_through_the_resolver_and_fires_load() {
    let (mut rt, report) = page(
        FRAME_PAGE,
        &[("inner.html", "<!doctype html><p id=in>hello")],
    );
    assert_eq!(report.uncaught, 0, "{:?}", report.errors);
    let got: String = rt.eval("log.join()").unwrap();
    assert_eq!(
        got,
        "about:blank,hello,https://x.test/dir/inner.html#target,text/html,true,true,true,true"
    );
}

#[test]
fn a_frame_that_cannot_be_had_still_fires_load() {
    let (mut rt, _) = page(FRAME_PAGE, &[]);
    assert_eq!(rt.eval::<String>("String(log[0])").unwrap(), "about:blank");
    // `getElementById` found nothing: the onload threw, and it did run.
    assert_eq!(rt.eval::<i32>("log.length").unwrap(), 1);
}

#[test]
fn srcdoc_and_blank_frames() {
    let (mut rt, _) = page(
        r#"<!doctype html><iframe id=a srcdoc="<p id=s>x"></iframe><iframe id=b></iframe>
           <script>globalThis.n = 0;
             for (const i of 'ab') document.getElementById(i).onload = () => n++;</script>"#,
        &[],
    );
    let got: String = rt
        .eval(
            "const d = document.getElementById('a').contentDocument;
             const e = document.getElementById('b').contentDocument;
             [n, d.getElementById('s').textContent, d.URL, e.URL, e.compatMode, e.body.tagName,
              document.getElementById('a').srcdoc].join()",
        )
        .unwrap();
    assert_eq!(
        got,
        "2,x,about:srcdoc,about:blank,BackCompat,BODY,<p id=s>x"
    );
}

#[test]
fn xml_frames_go_by_extension() {
    let (mut rt, _) = page(
        r#"<!doctype html><iframe id=a src=a.xml></iframe><iframe id=b src=b.xhtml></iframe>
           <iframe id=c src="c.svg?x"></iframe>"#,
        &[
            ("a.xml", "<a/>"),
            ("b.xhtml", "<html xmlns='http://www.w3.org/1999/xhtml'/>"),
            ("c.svg?x", "<svg xmlns='http://www.w3.org/2000/svg'/>"),
        ],
    );
    let got: String = rt
        .eval(
            "['a', 'b', 'c'].map((i) => {
               const d = document.getElementById(i).contentDocument;
               return d.contentType + ' ' + d.documentElement.tagName;
             }).join()",
        )
        .unwrap();
    assert_eq!(
        got,
        "application/xml a,application/xhtml+xml html,image/svg+xml svg"
    );
}

#[test]
fn a_frame_runs_no_script() {
    let (mut rt, _) = page(
        r#"<!doctype html><iframe id=a srcdoc="<script>top.ran = 1</script><img src=x onerror='top.ran = 2'>"></iframe>
           <script>globalThis.ran = 0;</script>"#,
        &[],
    );
    assert_eq!(rt.eval::<i32>("ran").unwrap(), 0);
}

#[test]
fn a_removed_frame_has_no_document() {
    let got = eval(
        "const f = document.createElement('iframe');
         const before = f.contentDocument;
         document.body.appendChild(f);
         const d = f.contentDocument;
         const same = d === f.contentDocument;
         f.remove();
         [before, same, f.contentDocument, f.contentWindow, d.defaultView === null].join()",
    );
    assert_eq!(got, ",true,,,false");
}

#[test]
fn frames_added_by_a_load_take_the_next_round_and_rounds_end() {
    // Each load adds a frame: 8 rounds, then the page's end.
    let (mut rt, _) = page(
        r#"<!doctype html><body><script>
           globalThis.n = 0;
           function add() {
             const f = document.createElement('iframe');
             f.onload = () => { n++; add(); };
             document.body.appendChild(f);
           }
           add();
           </script>"#,
        &[],
    );
    assert_eq!(rt.eval::<i32>("n").unwrap(), 8);
}

#[test]
fn a_frame_bomb_stops_at_the_frame_limit() {
    let (mut rt, _) = page(
        r#"<!doctype html><body><script>
           globalThis.n = 0;
           for (let i = 0; i < 5000; i++) {
             const f = document.createElement('iframe');
             f.onload = () => { n++; };
             document.body.appendChild(f);
           }
           </script>"#,
        &[],
    );
    assert_eq!(rt.eval::<i32>("n").unwrap(), 64);
    rt.with_document(|d| d.check_consistency().unwrap());
}

/// A frame's nodes are the render's: a frame source can't get around a limit.
#[test]
fn a_frame_counts_against_the_limits() {
    let big = "<p>".repeat(9_000);
    let mut rt = DomRuntime::new(limits()).unwrap();
    let l = DomLimits {
        max_nodes: 5_000,
        ..DomLimits::default()
    };
    rt.load(Document::parse_with_limits("<!doctype html><iframe src=a></iframe>", l).unwrap());
    let e = rt
        .run_page(&surimono_core::SettleOptions::default(), |_| {
            Some(big.clone())
        })
        .unwrap_err();
    assert_eq!(e, DomError::NodeLimit { limit: 5_000 });
}

/// The same page twice gives the same output: nothing depends on when a
/// resolver answers.
#[test]
fn frames_are_deterministic() {
    let run = || {
        let (mut rt, _) = page(FRAME_PAGE, &[("inner.html", "<p id=in>hello")]);
        rt.eval::<String>("log.join() + document.documentElement.outerHTML")
            .unwrap()
    };
    assert_eq!(run(), run());
}

#[test]
fn inner_and_outer_html_of_an_xml_document_are_xml() {
    let got = eval(
        r#"const d = new DOMParser().parseFromString(
             '<a xmlns="http://www.w3.org/1999/xhtml"><br/><b>&lt;</b></a>', 'application/xml');
         const a = d.documentElement;
         [a.innerHTML, a.firstChild.outerHTML].join('|')"#,
    );
    let x = r#"xmlns="http://www.w3.org/1999/xhtml""#;
    assert_eq!(got, format!("<br {x} /><b {x}>&lt;</b>|<br {x} />"));
}

/// With no Content-Type, an XML page's type goes by its URL's extension.
#[test]
fn an_xml_page_takes_its_type_from_its_url() {
    for (url, expected, namespace) in [
        ("https://x.test/a.xml", "application/xml", ""),
        ("https://x.test/a.svg?q", "image/svg+xml", ""),
        (
            "https://x.test/a.xht#f",
            "application/xhtml+xml",
            "http://www.w3.org/1999/xhtml",
        ),
        (
            "https://x.test/a.xhtml",
            "application/xhtml+xml",
            "http://www.w3.org/1999/xhtml",
        ),
        (
            "https://x.test/a",
            "application/xhtml+xml",
            "http://www.w3.org/1999/xhtml",
        ),
    ] {
        let mut doc = Document::parse_xml("<a/>").unwrap();
        doc.set_url(url);
        let mut rt = DomRuntime::new(limits()).unwrap();
        rt.load(doc);
        let got: String = rt
            .eval("document.contentType + ' ' + (document.createElement('b').namespaceURI || '')")
            .unwrap();
        assert_eq!(got, format!("{expected} {namespace}"), "{url}");
    }
}
