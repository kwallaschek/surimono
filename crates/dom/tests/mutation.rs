//! Tree mutation against the DOM Standard (§4.2.3 mutation algorithms,
//! §4.2.6 ParentNode, §4.2.8 ChildNode, §4.4 Node, §4.11 CharacterData) and
//! WPT dom/nodes (the file named per test holds the same case).

mod common;

use common::limits;
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

const PAGE: &str = r#"<!doctype html><html><head></head><body><div id="a"><i id="i">1</i><b id="b">2</b></div><p id="p"></p></body></html>"#;

fn runtime() -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    rt
}

/// Runs `script`; returns its string and the document's `<body>` markup.
fn run(script: &str) -> (String, String) {
    let mut rt = runtime();
    let r: String = rt.eval(script).unwrap();
    let html = rt.with_document(Document::to_html).unwrap();
    let body = html
        .split("<body>")
        .nth(1)
        .unwrap()
        .split("</body>")
        .next()
        .unwrap()
        .to_owned();
    (r, body)
}

const VARS: &str = "const a = document.getElementById('a'), i = document.getElementById('i'), b = document.getElementById('b'), p = document.getElementById('p');\n";

fn with_vars(script: &str) -> (String, String) {
    run(&format!("{VARS}{script}"))
}

#[test]
fn append_child_moves_a_node_within_and_between_parents() {
    // WPT Node-appendChild.html; §4.2.3 "adopt" removes from the old parent.
    let (r, body) = with_vars(
        "a.appendChild(i); p.appendChild(b); String(i.parentNode === a && b.parentNode === p)",
    );
    assert_eq!(r, "true");
    assert_eq!(
        body,
        r#"<div id="a"><i id="i">1</i></div><p id="p"><b id="b">2</b></p>"#
    );
}

#[test]
fn insert_before() {
    // WPT Node-insertBefore.html.
    let (r, body) = with_vars(
        r#"const n = document.createElement('u');
        const out = [a.insertBefore(n, b) === n, n.nextSibling === b, n.previousSibling === i];
        a.insertBefore(b, i);
        // Before itself: stays where it is (referenceChild becomes next).
        a.insertBefore(n, n);
        a.insertBefore(document.createTextNode('t'), null);
        out.join()"#,
    );
    assert_eq!(r, "true,true,true");
    assert_eq!(
        body,
        r#"<div id="a"><b id="b">2</b><i id="i">1</i><u></u>t</div><p id="p"></p>"#
    );
}

#[test]
fn insertion_errors() {
    // §4.2.3 "ensure pre-insert validity"; WPT Node-insertBefore.html,
    // Node-appendChild.html, Node-replaceChild.html, Node-removeChild.html.
    let (r, _) = with_vars(
        r#"const out = [];
        const t = (f) => { try { f(); out.push('none'); } catch (e) { out.push(e.name); } };
        t(() => a.insertBefore(p, i.firstChild));           // child's parent isn't a
        t(() => a.appendChild(a));                           // inclusive ancestor
        t(() => i.appendChild(document.body));               // ancestor
        t(() => i.firstChild.appendChild(p));                // parent is text
        t(() => a.appendChild(document));                    // a document
        t(() => document.appendChild(document.createTextNode('x')));
        t(() => document.appendChild(document.createElement('html')));
        t(() => a.replaceChild(p, i.firstChild));
        t(() => a.removeChild(p));
        t(() => document.appendChild(document.createComment('ok')));
        out.join()"#,
    );
    assert_eq!(
        r,
        "NotFoundError,HierarchyRequestError,HierarchyRequestError,HierarchyRequestError,\
         HierarchyRequestError,HierarchyRequestError,HierarchyRequestError,NotFoundError,\
         NotFoundError,none"
    );
}

#[test]
fn the_reference_child_is_checked_before_the_node() {
    // WPT pre-insertion-validation-notfound.js: step 3 of "ensure pre-insert
    // validity" (and of "replace") comes after the parent and ancestor
    // checks and before every check of `node`. `p` is a child of neither
    // `free` nor `document`. Parents and nodes the bindings can't make yet
    // (doctypes, a second document) are left out; `free` is detached so
    // `document`, the one document, isn't its ancestor (step 2).
    let (r, _) = with_vars(
        r#"const out = [];
        const t = (f) => { try { f(); out.push('none'); } catch (e) { out.push(e.name); } };
        const two = document.createDocumentFragment();
        two.append(document.createElement('b'), document.createElement('b'));
        const free = document.createElement('div');
        t(() => free.insertBefore(document, p));             // step 3 before 4
        t(() => free.replaceChild(document, p));
        t(() => document.insertBefore(document.createTextNode('x'), p)); // before 5
        t(() => document.replaceChild(document.createTextNode('x'), p));
        t(() => document.insertBefore(document.createElement('b'), p));  // before 6
        t(() => document.replaceChild(document.createElement('b'), p));
        t(() => document.insertBefore(two, p));
        t(() => document.replaceChild(two, p));
        t(() => i.firstChild.insertBefore(p, b));            // step 1 before 3
        t(() => i.firstChild.replaceChild(p, b));
        t(() => i.insertBefore(a, b));                       // step 2 before 3
        t(() => i.replaceChild(a, b));
        t(() => a.insertBefore(document, p));                // the ancestor test
        t(() => a.replaceChild(document, p));                // comes first
        out.join()"#,
    );
    assert_eq!(
        r,
        "NotFoundError,NotFoundError,NotFoundError,NotFoundError,NotFoundError,NotFoundError,\
         NotFoundError,NotFoundError,HierarchyRequestError,HierarchyRequestError,\
         HierarchyRequestError,HierarchyRequestError,HierarchyRequestError,\
         HierarchyRequestError"
    );
}

#[test]
fn a_failed_insert_leaves_the_tree_alone() {
    let (r, body) = with_vars(
        r#"let name = '';
        try { i.appendChild(a); } catch (e) { name = e.name; }
        name + (a.parentNode === document.body)"#,
    );
    assert_eq!(r, "HierarchyRequestErrortrue");
    assert_eq!(
        body,
        r#"<div id="a"><i id="i">1</i><b id="b">2</b></div><p id="p"></p>"#
    );
}

#[test]
fn replace_and_remove_child() {
    // WPT Node-replaceChild.html, Node-removeChild.html.
    let (r, body) = with_vars(
        r#"const n = document.createElement('u');
        const out = [a.replaceChild(n, i) === i, i.parentNode, n.parentNode === a];
        // Replacing a node with its next sibling.
        a.replaceChild(b, n);
        out.push(a.removeChild(b) === b, b.parentNode, b.textContent);
        p.appendChild(b);
        // A removed subtree is intact and insertable.
        i.remove(); p.appendChild(i);
        out.join()"#,
    );
    assert_eq!(r, "true,,true,true,,2");
    assert_eq!(
        body,
        r#"<div id="a"></div><p id="p"><b id="b">2</b><i id="i">1</i></p>"#
    );
}

#[test]
fn document_fragments_insert_their_children() {
    // §4.2.3 "insert"; WPT DocumentFragment-constructor.html.
    let (r, body) = with_vars(
        r#"const f = document.createDocumentFragment();
        const g = new DocumentFragment();
        f.appendChild(i); f.append('t', b);
        g.appendChild(document.createElement('s'));
        const out = [f.nodeType, f.nodeName, f.childElementCount, f.firstElementChild === i,
                     g instanceof DocumentFragment, f.getElementById('b') === b, f.parentNode,
                     a.hasChildNodes()];
        p.appendChild(f);
        out.push(f.firstChild, i.parentNode === p, f.ownerDocument === document);
        out.join()"#,
    );
    assert_eq!(
        r,
        "11,#document-fragment,2,true,true,true,,false,,true,true"
    );
    assert_eq!(
        body,
        r#"<div id="a"></div><p id="p"><i id="i">1</i>t<b id="b">2</b></p>"#
    );
}

#[test]
fn parent_node_mixin() {
    // §4.2.6; WPT ParentNode-append.html, ParentNode-prepend.html,
    // ParentNode-replaceChildren.html.
    let (_, body) = with_vars(
        r#"p.append('x', document.createElement('u'), 7);
        p.prepend('first');
        a.replaceChildren('only', i);
        p.append();
        ''"#,
    );
    assert_eq!(
        body,
        r#"<div id="a">only<i id="i">1</i></div><p id="p">firstx<u></u>7</p>"#
    );
}

#[test]
fn child_node_mixin() {
    // §4.2.8; WPT ChildNode-before.html, ChildNode-after.html,
    // ChildNode-replaceWith.html, ChildNode-remove.html.
    let (_, body) = with_vars(
        r#"b.before('<', document.createElement('u'));
        i.after('>');
        // A node passed to its own sibling's before(): viable sibling rule.
        i.before(i, 'y');
        p.replaceWith('gone', document.createElement('s'));
        p.remove();
        p.before('ignored'); p.after('ignored'); p.replaceWith('ignored');
        ''"#,
    );
    assert_eq!(
        body,
        r#"<div id="a"><i id="i">1</i>y&gt;&lt;<u></u><b id="b">2</b></div>gone<s></s>"#
    );
}

#[test]
fn clone_node() {
    // §4.4 "clone a node"; WPT Node-cloneNode.html.
    let (r, _) = with_vars(
        r#"const shallow = a.cloneNode();
        const deep = a.cloneNode(true);
        const c = document.createComment('note').cloneNode();
        [true, shallow.hasChildNodes(), shallow.id, deep.textContent,
         deep.parentNode, deep !== a, deep.firstChild !== i, deep.firstChild.id,
         a.isEqualNode(deep), a.isEqualNode(shallow), deep.isSameNode(deep), deep.isSameNode(a),
         c.data, new DocumentFragment().cloneNode(true).nodeType,
         a.isEqualNode(null)].join()"#,
    );
    assert_eq!(
        r,
        "true,false,a,12,,true,true,i,true,false,true,false,note,11,false"
    );
}

/// DOM §4.4: a clone of a document is a new document, with its own copies.
#[test]
fn cloning_a_document_gives_a_document() {
    let mut rt = runtime();
    let got: String = rt
        .eval(
            "const c = document.cloneNode(true), s = document.cloneNode();
             [c.nodeType, c === document, c.ownerDocument, c.body.tagName,
              c.body.ownerDocument === c, s.firstChild, document.body.ownerDocument === document].join()",
        )
        .unwrap();
    assert_eq!(got, "9,false,,BODY,true,,true");
}

#[test]
fn equal_nodes_compare_an_attributes_namespace_not_its_prefix() {
    // §4.4 "equals"; WPT Node-isEqualNode.html, "elements should be compared
    // on attribute namespace, local name, and value".
    let (r, _) = with_vars(
        r#"const el = (ns, name, v) => {
            const e = document.createElement('element');
            e.setAttributeNS(ns, name, v);
            return e;
        };
        const e1 = el('namespace', 'prefix:localName', 'value');
        const compare = [
            el('namespace', 'prefix:localName', 'value'),
            el('namespace2', 'prefix:localName', 'value'),
            el('namespace', 'prefix2:localName', 'value'),
            el('namespace', 'prefix:localName2', 'value'),
            el('namespace', 'prefix:localName', 'value2'),
        ].map((e) => e1.isEqualNode(e));
        // An element's own prefix does count.
        const x = document.createElementNS('namespace', 'prefix:localName');
        const y = document.createElementNS('namespace', 'prefix2:localName');
        compare.push(x.isEqualNode(y));
        compare.join()"#,
    );
    assert_eq!(r, "true,false,true,false,false,false");
}

#[test]
fn clone_refuses_more_nodes_than_fit() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(
        Document::parse_with_limits(
            PAGE,
            DomLimits {
                max_nodes: 14,
                ..DomLimits::default()
            },
        )
        .unwrap(),
    );
    assert_eq!(
        rt.eval::<()>("document.body.cloneNode(true)"),
        Err(DomError::NodeLimit { limit: 14 })
    );
}

#[test]
fn contains_root_and_position() {
    // §4.4; WPT Node-contains.html, Node-compareDocumentPosition.html,
    // Node-getRootNode.html.
    let (r, _) = with_vars(
        r#"const lone = document.createElement('u');
        const kid = lone.appendChild(document.createElement('s'));
        const cmp = lone.compareDocumentPosition(a);
        [a.contains(i), a.contains(a), i.contains(a), a.contains(null), a.contains(lone),
         i.getRootNode() === document, kid.getRootNode() === lone,
         a.compareDocumentPosition(a), a.compareDocumentPosition(i), i.compareDocumentPosition(a),
         i.compareDocumentPosition(b), b.compareDocumentPosition(i),
         p.compareDocumentPosition(i), cmp & 1, cmp & 32, (cmp & 6) !== 0 && (cmp & 6) !== 6,
         Node.DOCUMENT_POSITION_CONTAINED_BY].join()"#,
    );
    assert_eq!(
        r,
        "true,true,false,false,false,true,true,0,20,10,4,2,2,1,32,true,16"
    );
}

#[test]
fn normalize() {
    // §4.4; WPT Node-normalize.html.
    let (r, body) = with_vars(
        r#"p.append('a', 'b', '', document.createElement('u'), '', 'c');
        p.lastChild.previousSibling.after(document.createTextNode(''));
        const first = p.firstChild;
        p.normalize();
        [p.firstChild === first, first.data, p.lastChild.data, p.childElementCount].join()"#,
    );
    assert_eq!(r, "true,ab,c,1");
    assert_eq!(
        body,
        r#"<div id="a"><i id="i">1</i><b id="b">2</b></div><p id="p">ab<u></u>c</p>"#
    );
}

#[test]
fn normalize_drops_merged_nodes_and_keeps_held_ones() {
    // Merged nodes with no handle go at once (DESIGN.md, node lifetime), so
    // this loop stays flat.
    let mut rt = runtime();
    let script = "{ const p = document.getElementById('p');
        for (let i = 0; i < 100; i++) { p.append('a', 'b'); p.normalize(); p.textContent = ''; } }";
    let _: () = rt.eval(script).unwrap();
    let once = rt.with_document(Document::node_count).unwrap();
    let _: () = rt.eval(script).unwrap();
    assert_eq!(rt.with_document(Document::node_count).unwrap(), once);
    rt.with_document(Document::check_consistency)
        .unwrap()
        .unwrap();

    // A merged node a script holds is removed from the tree, data intact (§4.4
    // only changes the first node's data). WPT Node-normalize.html.
    let (r, body) = with_vars(
        r#"const t1 = document.createTextNode('a'), t2 = document.createTextNode('b');
        const empty = document.createTextNode('');
        p.append(empty, t1, t2);
        p.normalize();
        [t1.parentNode === p, t1.data, t2.parentNode, t2.data, empty.parentNode,
         p.firstChild === p.lastChild].join()"#,
    );
    assert_eq!(r, "true,ab,,b,,true");
    assert_eq!(
        body,
        r#"<div id="a"><i id="i">1</i><b id="b">2</b></div><p id="p">ab</p>"#
    );
}

#[test]
fn child_node_methods_with_this_among_the_arguments() {
    // §4.2.8: the node leaves its place into the temporary fragment. WPT
    // ChildNode-replaceWith.html, ChildNode-after.html.
    let (r, body) = with_vars(
        r#"const x = document.createElement('x');
        a.append(x);
        // Replacing with itself and a sibling.
        i.replaceWith(x, i);
        const first = a.firstChild.nodeName + a.firstChild.nextSibling.nodeName;
        // A listed next sibling: the viable sibling is past it.
        const y = document.createElement('y'), z = document.createElement('z');
        p.append(y, z);
        const t = document.createTextNode('t');
        p.prepend(t);
        t.after(y);
        first + p.firstChild.nextSibling.nodeName + p.lastChild.nodeName"#,
    );
    assert_eq!(r, "XIYZ");
    assert_eq!(
        body,
        r#"<div id="a"><x></x><i id="i">1</i><b id="b">2</b></div><p id="p">t<y></y><z></z></p>"#
    );
}

#[test]
fn byte_count_stays_right_after_remove_attribute_ns() {
    let mut rt = runtime();
    let _: () = rt
        .eval(
            "const a = document.getElementById('a');
            a.setAttributeNS('n', 'p:k', 'value');
            a.setAttributeNS('n', 'q:k', 'other');
            a.removeAttributeNS('n', 'k');",
        )
        .unwrap();
    rt.with_document(Document::check_consistency)
        .unwrap()
        .unwrap();
    let html = rt.with_document(Document::to_html).unwrap();
    assert!(!html.contains("other"), "{html}");
}

#[test]
fn character_data_operations() {
    // §4.11; WPT CharacterData-*.html; offsets are UTF-16 code units.
    let (r, _) = run(r#"const t = document.createTextNode('abcdef');
        const out = [t.substringData(1, 2), t.substringData(4, 100), t.substringData(6, 1)];
        t.appendData('g'); out.push(t.data);
        t.insertData(1, 'X'); out.push(t.data);
        t.deleteData(1, 1); out.push(t.data);
        t.replaceData(0, 2, 'Z'); out.push(t.data);
        t.deleteData(3, 1000); out.push(t.data);
        for (const f of [() => t.substringData(99, 1), () => t.insertData(99, 'x'),
                         () => t.deleteData(99, 1), () => t.replaceData(99, 1, 'x')]) {
          try { f(); out.push('none'); } catch (e) { out.push(e.name); }
        }
        const u = document.createTextNode('😀b');
        out.push(u.length, u.substringData(2, 1));
        const c = document.createComment('xy'); c.appendData('z'); out.push(c.data);
        out.join()"#);
    assert_eq!(
        r,
        "bc,ef,,abcdefg,aXbcdefg,abcdefg,Zcdefg,Zcd,\
         IndexSizeError,IndexSizeError,IndexSizeError,IndexSizeError,3,b,xyz"
    );
}

#[test]
fn split_text_and_whole_text() {
    // §4.12; WPT Text-splitText.html, Text-wholeText.html.
    let (r, body) = with_vars(
        r#"const t = p.appendChild(document.createTextNode('abcd'));
        p.appendChild(document.createElement('u'));
        const tail = t.splitText(1);
        const lone = document.createTextNode('xy').splitText(1);
        let err = '';
        try { t.splitText(9); } catch (e) { err = e.name; }
        [t.data, tail.data, tail.previousSibling === t, tail.nextSibling.tagName,
         t.wholeText, tail.wholeText, lone.data, lone.parentNode, err].join()"#,
    );
    assert_eq!(r, "a,bcd,true,U,abcd,abcd,y,,IndexSizeError");
    assert!(body.contains(r#"<p id="p">abcd<u></u></p>"#), "{body}");
}

#[test]
fn namespaces() {
    // §4.5 createElementNS and §1.5 "validate and extract"; WPT
    // Document-createElementNS.html, attributes.html.
    let (r, body) = with_vars(
        r#"const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        const x = document.createElementNS('urn:x', 'p:Thing');
        const out = [svg.tagName, svg.namespaceURI, svg instanceof HTMLElement, x.tagName, x.prefix, x.localName];
        svg.setAttributeNS('http://www.w3.org/1999/xlink', 'xlink:href', '#a');
        svg.setAttributeNS(null, 'viewBox', '0 0 1 1');
        out.push(svg.getAttributeNS('http://www.w3.org/1999/xlink', 'href'),
                 svg.getAttribute('xlink:href'), svg.getAttribute('viewBox'),
                 svg.hasAttributeNS(null, 'viewBox'), svg.hasAttributeNS('urn:y', 'viewBox'));
        svg.setAttributeNS('http://www.w3.org/1999/xlink', 'other:href', '#b');
        out.push(svg.getAttribute('xlink:href'));
        p.appendChild(svg);
        for (const f of [() => document.createElementNS(null, 'p:x'),
                         () => document.createElementNS('urn:x', 'xml:x'),
                         () => document.createElementNS('urn:x', 'xmlns'),
                         () => document.createElementNS('urn:x', '1x')]) {
          try { f(); out.push('none'); } catch (e) { out.push(e.name); }
        }
        out.join()"#,
    );
    assert_eq!(
        r,
        "svg,http://www.w3.org/2000/svg,false,p:Thing,p,Thing,#a,#a,0 0 1 1,true,false,#b,\
         NamespaceError,NamespaceError,NamespaceError,InvalidCharacterError"
    );
    assert!(
        body.contains(r##"<svg xlink:href="#b" viewBox="0 0 1 1"></svg>"##),
        "{body}"
    );
}

#[test]
fn insert_adjacent() {
    // §4.9 insertAdjacentElement, insertAdjacentText; WPT
    // Element-insertAdjacent*.html.
    let (r, body) = with_vars(
        r#"const u = (n) => document.createElement(n);
        const out = [a.insertAdjacentElement('beforeBegin', u('u')).tagName];
        a.insertAdjacentElement('afterbegin', u('s'));
        a.insertAdjacentText('beforeend', 'e');
        a.insertAdjacentElement('AFTEREND', u('q'));
        const lone = u('em');
        out.push(lone.insertAdjacentElement('beforebegin', u('a')), lone.parentNode);
        try { a.insertAdjacentElement('nowhere', u('a')); } catch (e) { out.push(e.name); }
        out.join()"#,
    );
    assert_eq!(r, "U,,,SyntaxError");
    assert_eq!(
        body,
        r#"<u></u><div id="a"><s></s><i id="i">1</i><b id="b">2</b>e</div><q></q><p id="p"></p>"#
    );
}

#[test]
fn adopt_node_detaches() {
    // §4.5 adoptNode.
    let (r, _) = with_vars(
        r#"let err = '';
        try { document.adoptNode(document); } catch (e) { err = e.name; }
        [document.adoptNode(i) === i, i.parentNode, err].join()"#,
    );
    assert_eq!(r, "true,,NotSupportedError");
}

#[test]
fn id_lookup_follows_the_tree() {
    // §4.2.4: only the connected tree; a removed element isn't found.
    let (r, _) = with_vars(
        r#"const out = [];
        i.remove(); out.push(document.getElementById('i'));
        p.appendChild(i); out.push(document.getElementById('i') === i);
        out.join()"#,
    );
    assert_eq!(r, ",true");
}

#[test]
fn a_refused_multi_node_insert_keeps_the_nodes() {
    // §4.2.6: the nodes go into a fragment first, so they have left their
    // parent when the insertion is refused, and they stay usable.
    let (r, body) = with_vars(
        r#"const t = i.firstChild;
        let err = '';
        try { t.append('x', b); } catch (e) { err = e.name; }
        try { document.append(i, 'x'); } catch (e) { err += ' ' + e.name; }
        p.appendChild(b); p.appendChild(i);
        [err, b.textContent, i.textContent].join()"#,
    );
    assert_eq!(r, "TypeError HierarchyRequestError,2,1");
    assert!(
        body.ends_with(r#"<p id="p"><b id="b">2</b><i id="i">1</i></p>"#),
        "{body}"
    );
}
