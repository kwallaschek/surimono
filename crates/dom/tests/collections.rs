//! Lists, tokens, dataset, selectors and innerHTML against the specs. WPT
//! dom/nodes and dom/lists files that hold the same cases are named per
//! test; html/ refers to the HTML Standard.

mod common;

use common::limits;
use surimono_dom::{Document, DomRuntime};

const PAGE: &str = r#"<!doctype html><html><head></head><body><ul id="u" class="a b  a" data-x-y="1" data-z="2"><li id="l1" class="k">one</li><!--c--><li id="l2" class="k m" name="nm">two</li></ul><p id="p">text</p></body></html>"#;

fn runtime() -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    rt
}

const VARS: &str = "const u = document.getElementById('u'), l1 = document.getElementById('l1'), l2 = document.getElementById('l2'), p = document.getElementById('p');\n";

fn eval(script: &str) -> String {
    runtime()
        .eval::<String>(&format!("{VARS}{script}"))
        .unwrap()
}

/// As [`eval`], plus the serialized `<body>` content.
fn eval_body(script: &str) -> (String, String) {
    let mut rt = runtime();
    let r: String = rt.eval(&format!("{VARS}{script}")).unwrap();
    let html = rt.with_document(Document::to_html).unwrap();
    let body = html.split("<body>").nth(1).unwrap();
    (r, body.split("</body>").next().unwrap().to_owned())
}

#[test]
fn child_nodes_is_live_and_the_same_object() {
    // DOM §4.2.10.1, §4.4 childNodes [SameObject]; WPT Node-childNodes.html.
    let r = eval(
        r#"const c = u.childNodes;
        const out = [c === u.childNodes, c.length, c[0] === l1, c.item(1).nodeName, c[9], c.item(9),
                     c instanceof NodeList, Object.prototype.toString.call(c)];
        l2.remove(); out.push(c.length, c[2]);
        u.appendChild(p); out.push(c.length, c[2] === p);
        out.join()"#,
    );
    assert_eq!(r, "true,3,true,#comment,,,true,[object NodeList],2,,3,true");
}

#[test]
fn lists_behave_as_arrays_do() {
    // WebIDL §3.7.9 iterable, §3.9 legacy platform objects: index keys,
    // `in`, keys, for-of, Array.from, forEach; writes to indices fail.
    let r = eval(
        r#"const c = u.childNodes;
        const names = [];
        for (const n of c) names.push(n.nodeName);
        const out = [names.join(' '), Array.from(c).length, 0 in c, 3 in c, Object.keys(c).join('+')];
        c.forEach((n, i) => out.push(`${i}:${n.nodeType}`));
        out.push(Array.prototype.slice.call(c, 1).length, [...c.keys()].join('+'));
        c[0] = 1; out.push(c[0] === l1);
        try { (function () { 'use strict'; c[0] = 1; })(); out.push('none'); } catch (e) { out.push(e.name); }
        c.expando = 7; out.push(c.expando);
        out.push(delete c.expando, c.expando, Object.getOwnPropertyDescriptor(c, 0).value === l1);
        out.join()"#,
    );
    assert_eq!(
        r,
        "LI #comment LI,3,true,false,0+1+2,0:1,1:8,2:1,2,0+1+2,true,TypeError,7,true,,true"
    );
}

#[test]
fn html_collections() {
    // DOM §4.2.10.2; WPT HTMLCollection-*.html, Element-children.html,
    // Document-getElementsByTagName.html, Element-getElementsByClassName.html.
    let r = eval(
        r#"const c = u.children;
        const out = [c === u.children, c.length, c[1] === l2, c.item(5), c.namedItem('l1') === l1,
                     c.namedItem('nm') === l2, c.namedItem(''), c.l1 === l1, c.nm === l2,
                     c.length === 2, 'l2' in c, Object.keys(c).join('+'),
                     Object.getOwnPropertyNames(c).join('+'), c instanceof HTMLCollection];
        const lis = document.getElementsByTagName('LI');
        out.push(lis.length, lis === document.getElementsByTagName('LI'),
                 document.getElementsByTagName('*').length, u.getElementsByTagName('li').length,
                 p.getElementsByTagName('li').length);
        const ks = document.getElementsByClassName('k');
        out.push(ks.length, document.getElementsByClassName('m k').length,
                 document.getElementsByClassName('  ').length, u.getElementsByClassName('m')[0] === l2);
        l1.className = 'x'; out.push(ks.length);
        document.body.appendChild(document.createElement('li')); out.push(lis.length);
        out.join()"#,
    );
    assert_eq!(
        r,
        "true,2,true,,true,true,,true,true,true,true,0+1,0+1+l1+l2+nm,true,\
         2,true,7,2,0,2,1,0,true,1,3"
    );
}

#[test]
fn class_list() {
    // DOM §7.1; WPT dom/lists/DOMTokenList-*.html, dom/nodes/Element-classlist.html.
    let (r, body) = eval_body(
        r#"const c = u.classList;
        const out = [c === u.classList, c.length, c[0], c[1], c[2], c.item(0), c.item(5), c.contains('a'),
                     String(c), c.value, c instanceof DOMTokenList];
        c.add('n', 'a'); out.push(c.value);
        c.remove('b'); out.push(c.value);
        out.push(c.toggle('t'), c.toggle('t'), c.toggle('t', false), c.toggle('n', true), c.value);
        out.push(c.replace('a', 'q'), c.replace('nope', 'r'), c.value);
        for (const f of [() => c.add(''), () => c.add('a b'), () => c.remove(''), () => c.toggle(' '),
                         () => c.replace('', 'x'), () => c.supports('x')]) {
          try { f(); out.push('none'); } catch (e) { out.push(e.name); }
        }
        const fresh = document.createElement('i');
        fresh.classList.remove('x'); out.push(fresh.hasAttribute('class'));
        fresh.classList = 'via put forwards'; out.push(fresh.className, [...fresh.classList].join('+'));
        l1.classList.value = 'w'; out.push(l1.className);
        out.join()"#,
    );
    assert_eq!(
        r,
        "true,2,a,b,,a,,true,a b  a,a b  a,true,a b n,a n,true,false,false,true,a n,true,false,q n,\
         SyntaxError,InvalidCharacterError,SyntaxError,InvalidCharacterError,SyntaxError,TypeError,\
         false,via put forwards,via+put+forwards,w"
    );
    assert!(body.contains(r#"<ul id="u" class="q n""#), "{body}");
}

#[test]
fn dataset() {
    // HTML §3.2.6; WPT html/dom/elements/global-attributes/dataset-*.html.
    let (r, body) = eval_body(
        r#"const d = u.dataset;
        const out = [d === u.dataset, d.xY, d.z, d.missing, 'z' in d, 'nope' in d, Object.keys(d).join('+')];
        d.fooBar = 7; out.push(u.getAttribute('data-foo-bar'));
        delete d.z; out.push(u.hasAttribute('data-z'));
        d['x-1'] = 'h'; out.push(u.getAttribute('data-x-1'));
        try { d['a-b'] = '1'; out.push('none'); } catch (e) { out.push(e.name); }
        try { d['a b'] = '1'; out.push('none'); } catch (e) { out.push(e.name); }
        out.push(document.body.dataset.none, JSON.stringify(document.createElement('i').dataset),
                 d instanceof DOMStringMap);
        out.join()"#,
    );
    assert_eq!(
        r,
        "true,1,2,,true,false,xY+z,7,false,h,SyntaxError,InvalidCharacterError,,{},true"
    );
    assert!(body.contains(r#"data-foo-bar="7""#), "{body}");
}

#[test]
fn attributes_map() {
    // DOM §4.9.1, §4.9.2; WPT attributes.html, Element-hasAttribute.html.
    let r = eval(
        r#"const m = u.attributes;
        const a = m.getNamedItem('id');
        const out = [m === u.attributes, m.length, m[0] === a, m.item(0) === a, m.id === a,
                     m.nope, m.item(99), a.name, a.value, a.localName, a.namespaceURI, a.prefix,
                     a.ownerElement === u, a.specified, a.nodeType, a.nodeName, a.nodeValue,
                     a.textContent, a.parentNode, a instanceof Attr, a instanceof Node,
                     u.getAttributeNames().join('+'), Object.keys(m).join('+')];
        a.value = 'v'; out.push(u.id, m.getNamedItemNS(null, 'id') === a);
        const b = m.removeNamedItem('class');
        out.push(b.value, b.ownerElement, u.hasAttribute('class'));
        try { m.removeNamedItem('class'); out.push('none'); } catch (e) { out.push(e.name); }
        out.join()"#,
    );
    assert_eq!(
        r,
        "true,4,true,true,true,,,id,u,id,,,true,true,2,id,u,u,,true,true,\
         id+class+data-x-y+data-z,0+1+2+3,v,true,a b  a,,false,NotFoundError"
    );
}

#[test]
fn query_selector() {
    // DOM §4.2.6, Selectors 4; WPT ParentNode-querySelector-All.html,
    // Element-closest.html, Element-matches.html.
    let r = eval(
        r#"const all = document.querySelectorAll('li');
        const out = [all.length, all[1] === l2, all instanceof NodeList, document.querySelector('.m') === l2,
                     document.querySelector('nope'), u.querySelector('#l1') === l1,
                     p.querySelector('li'), u.querySelectorAll('ul li').length,
                     document.querySelector('[data-z="2"]') === u,
                     // The comment between them doesn't count.
                     document.querySelector('li:nth-child(2)') === l2,
                     document.querySelector('li + li') === l2,
                     document.querySelectorAll('li, p').length,
                     document.querySelectorAll('body > *').length,
                     l1.matches('li.k'), l1.matches('ul > li:first-child'), l2.matches('#l1'),
                     l1.closest('ul') === u, l1.closest('li') === l1, l1.closest('section'),
                     p.closest('body') === document.body];
        // Static: later changes don't show (DOM §4.2.6).
        l2.remove(); out.push(all.length, document.querySelectorAll('li').length);
        for (const f of [() => document.querySelector('!!'), () => document.querySelectorAll(''),
                         () => l1.matches('['), () => l1.closest('a b >')]) {
          try { f(); out.push('none'); } catch (e) { out.push(e.name); }
        }
        const lone = document.createElement('div');
        lone.innerHTML = '<a><b class="x"></b></a>';
        out.push(lone.querySelector('.x').tagName, lone.querySelectorAll('a b').length);
        const f = document.createDocumentFragment();
        f.append(lone); out.push(f.querySelector('b').tagName, f.children.length);
        // :scope is the element queried; :root only the document's element;
        // nothing is a child of a fragment; comments don't stop :empty.
        out.push(u.querySelectorAll(':scope > li').length, document.querySelectorAll(':root').length,
                 lone.matches(':root'), lone.querySelector(':root'), lone.matches('* > div'));
        p.textContent = ''; p.appendChild(document.createComment('c'));
        out.push(p.matches(':empty'));
        out.join()"#,
    );
    assert_eq!(
        r,
        "2,true,true,true,,true,,2,true,true,true,3,2,true,true,false,true,true,,true,\
         2,1,SyntaxError,SyntaxError,SyntaxError,SyntaxError,B,1,B,1,1,1,false,,false,true"
    );
}

#[test]
fn inner_html() {
    // HTML §8.5 (DOM Parsing): innerHTML get and set; WPT
    // domparsing/innerhtml-*.html.
    let (r, body) = eval_body(
        r#"const out = [u.firstChild.innerHTML, p.innerHTML, l1.outerHTML];
        const kept = p.firstChild;
        p.innerHTML = '<b id="n">x &amp; y</b><!--c-->tail<br>';
        out.push(p.childNodes.length, document.getElementById('n').textContent,
                 p.lastChild.nodeName, kept.parentNode, kept.data, p.innerHTML);
        l1.innerHTML = null; out.push(l1.childNodes.length);
        l2.innerHTML = '<td>cell</td>'; out.push(l2.innerHTML);
        // Context matters: table parts parse inside a table.
        const t = document.createElement('table');
        t.innerHTML = '<tr><td>1'; out.push(t.innerHTML);
        const s = document.createElement('script');
        s.innerHTML = '<b>'; out.push(s.childNodes.length, s.firstChild.nodeType, s.innerHTML);
        out.join('|')"#,
    );
    assert_eq!(
        r,
        "one|text|<li id=\"l1\" class=\"k\">one</li>|4|x & y|BR||text|\
         <b id=\"n\">x &amp; y</b><!--c-->tail<br>|0|cell|\
         <tbody><tr><td>1</td></tr></tbody>|1|3|<b>"
    );
    assert!(
        body.contains(r#"<li id="l2" class="k m" name="nm">cell</li>"#),
        "{body}"
    );
}

#[test]
fn outer_html_and_insert_adjacent_html() {
    // HTML §8.5; WPT domparsing/outerhtml-*.html, insert_adjacent_html.html.
    let (r, body) = eval_body(
        r#"const out = [];
        p.insertAdjacentHTML('beforebegin', '<a>1</a>');
        p.insertAdjacentHTML('afterbegin', '<a>2</a>');
        p.insertAdjacentHTML('beforeend', '<a>3</a>');
        p.insertAdjacentHTML('AfterEnd', '<a>4</a>');
        l1.outerHTML = '<li>new</li><li>more</li>';
        out.push(l1.parentNode, l1.id);
        for (const f of [() => document.documentElement.outerHTML = 'x',
                         () => p.insertAdjacentHTML('nowhere', ''),
                         () => document.documentElement.insertAdjacentHTML('afterend', 'x'),
                         () => document.createElement('i').insertAdjacentHTML('beforebegin', 'x')]) {
          try { f(); out.push('none'); } catch (e) { out.push(e.name); }
        }
        const lone = document.createElement('i');
        lone.outerHTML = 'ignored'; out.push(lone.parentNode);
        out.join()"#,
    );
    assert_eq!(
        r,
        ",l1,NoModificationAllowedError,SyntaxError,NoModificationAllowedError,\
         NoModificationAllowedError,"
    );
    assert!(body.starts_with(r#"<ul id="u" class="a b  a" data-x-y="1" data-z="2"><li>new</li><li>more</li><!--c--><li id="l2""#), "{body}");
    assert!(
        body.ends_with(r#"<a>1</a><p id="p"><a>2</a>text<a>3</a></p><a>4</a>"#),
        "{body}"
    );
}

#[test]
fn fragment_parsing_has_no_effect_until_inserted() {
    // A fragment's nodes are parsed in a detached holder: the document's
    // root element, children and IDs don't change meanwhile.
    let r = eval(
        r#"const before = document.children.length;
        p.innerHTML = '<div id="made"><style>b{color:red}</style><img src="http://["></div>';
        [before, document.children.length, document.documentElement.tagName,
         document.getElementById('made').tagName, document.querySelector('img').getAttribute('src')].join()"#,
    );
    assert_eq!(r, "1,1,HTML,DIV,http://[");
}

#[test]
fn what_the_libraries_do() {
    // marked: el.innerHTML = rendered; highlight.js: querySelectorAll, a
    // parent check, class and dataset, children, innerHTML; KaTeX
    // auto-render: walk with firstChild/nextSibling, split text into a
    // fragment, createElementNS, replaceChild.
    let (r, body) = eval_body(
        r#"const code = document.createElement('pre');
        code.innerHTML = '<code class="language-js">let a = 1 &lt; 2;</code>';
        document.body.appendChild(code);
        const out = [];
        document.querySelectorAll('pre code').forEach((el) => {
          out.push(el.parentNode.tagName, el.children.length, el.textContent);
          el.innerHTML = '<span class="hljs-keyword">let</span> a';
          el.classList.add('hljs');
          el.dataset.highlighted = 'yes';
        });
        p.textContent = 'x $a$ y';
        for (let n = p.firstChild; n; n = n.nextSibling) {
          if (n.nodeType !== 3) continue;
          const frag = document.createDocumentFragment();
          frag.appendChild(document.createTextNode('x '));
          const span = document.createElement('span');
          span.setAttribute('class', 'katex');
          const math = document.createElementNS('http://www.w3.org/1998/Math/MathML', 'math');
          math.appendChild(document.createElementNS('http://www.w3.org/1998/Math/MathML', 'mi')).appendChild(document.createTextNode('a'));
          span.appendChild(math);
          frag.appendChild(span);
          frag.appendChild(document.createTextNode(' y'));
          n.parentNode.replaceChild(frag, n);
          break;
        }
        out.join('|')"#,
    );
    assert_eq!(r, "PRE|0|let a = 1 < 2;");
    assert!(
        body.contains(r#"<p id="p">x <span class="katex"><math><mi>a</mi></math></span> y</p>"#),
        "{body}"
    );
    assert!(
        body.contains(r#"<code class="language-js hljs" data-highlighted="yes"><span class="hljs-keyword">let</span> a</code>"#),
        "{body}"
    );
}
