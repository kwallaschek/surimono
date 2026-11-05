//! The generated bindings against outside truth. Expected values come from
//! the DOM Standard (https://dom.spec.whatwg.org/, sections cited per test),
//! the HTML Standard and WebIDL.

mod common;

use common::limits;
use surimono_dom::{Document, DomRuntime};

const PAGE: &str = r#"<!doctype html><html lang="ja"><head><title>t</title></head>
<body class="a b"><!-- note --><div id="x" title="tip">one<b>two</b>three</div><p id="y"></p></body></html>"#;

fn runtime(html: &str) -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(html).unwrap());
    rt
}

fn eval(html: &str, script: &str) -> String {
    runtime(html).eval::<String>(script).unwrap()
}

#[test]
fn tree_navigation() {
    // DOM §4.4 Interface Node; WPT dom/nodes/Node-childNodes.html,
    // Node-parentNode.html, Node-nextSibling.html.
    let r = eval(
        PAGE,
        r#"
        const x = document.getElementById('x');
        const b = x.firstChild.nextSibling;
        [x.firstChild.data, x.lastChild.data, b.tagName, b.previousSibling.data,
         x.lastChild.nextSibling, x.firstChild.previousSibling,
         x.hasChildNodes(), b.firstChild.hasChildNodes(),
         x.parentElement === document.body, document.documentElement.parentElement,
         document.parentNode, document.ownerDocument, x.ownerDocument === document,
         x.isConnected, document.createElement('i').isConnected,
         document.isConnected].join('|')"#,
    );
    assert_eq!(
        r,
        "one|three|B|one|||true|false|true||||true|true|false|true"
    );
}

#[test]
fn walking_siblings_stays_correct_while_the_tree_changes() {
    // The sibling step caches a position; a stale one must not answer.
    let r = eval(
        "<!doctype html><body id=b><i id=a></i><i id=c></i>",
        r#"
        const a = document.getElementById('a'), c = document.getElementById('c');
        const first = a.nextSibling === c;
        const n = document.createElement('u');
        document.body.textContent;
        const before = c.previousSibling === a;
        document.body.appendChild(n);
        [first, before, c.nextSibling === n, n.previousSibling === c, n.nextSibling].join()"#,
    );
    assert_eq!(r, "true,true,true,true,");
}

#[test]
fn node_type_and_names() {
    // DOM §4.4 nodeType and nodeName; §4.9 tagName: HTML elements upper
    // case in an HTML document, others as written.
    let r = eval(
        PAGE,
        r#"
        const x = document.getElementById('x');
        [document.nodeType, x.nodeType, x.firstChild.nodeType,
         document.body.firstChild.nodeType, document.nodeName, x.nodeName,
         x.firstChild.nodeName, document.body.firstChild.nodeName,
         Node.ELEMENT_NODE, Node.prototype.COMMENT_NODE, x.DOCUMENT_NODE,
         x.localName, x.namespaceURI, x.prefix].join()"#,
    );
    assert_eq!(
        r,
        "9,1,3,8,#document,DIV,#text,#comment,1,8,9,div,http://www.w3.org/1999/xhtml,"
    );
    let r = eval(
        "<!doctype html><svg><linearGradient id=g></linearGradient></svg>",
        r#"const g = document.getElementById('g');
        [g.tagName, g.localName, g.namespaceURI, g instanceof Element,
         typeof HTMLElement, g instanceof HTMLElement].join()"#,
    );
    assert_eq!(
        r,
        "linearGradient,linearGradient,http://www.w3.org/2000/svg,true,function,false"
    );
}

#[test]
fn prototype_chain_follows_the_idl() {
    // WebIDL §3.7: interface objects and prototype objects; DOM §2.7 and
    // §4.4 for the inheritance.
    let r = eval(
        PAGE,
        r#"
        const x = document.getElementById('x');
        const t = x.firstChild;
        const chain = (o) => { const n = []; for (let p = Object.getPrototypeOf(o); p; p = Object.getPrototypeOf(p)) n.push(p.constructor.name); return n.join('>'); };
        [chain(x), chain(t), chain(document), Object.getPrototypeOf(Element) === Node,
         Object.prototype.toString.call(x), String(Object.getOwnPropertyDescriptor(Node, 'prototype').writable),
         Object.getOwnPropertyDescriptor(Node.prototype, 'ELEMENT_NODE').writable].join('|')"#,
    );
    assert_eq!(
        r,
        "HTMLDivElement>HTMLElement>Element>Node>EventTarget>Object|\
         Text>CharacterData>Node>EventTarget>Object|\
         Document>Node>EventTarget>Object|true|[object HTMLDivElement]|false|false"
    );
}

#[test]
fn members_outside_the_table_do_not_exist() {
    // Scripts feature-detect: a member that only threw would pass the test.
    let r = eval(
        PAGE,
        "[typeof document.elementsFromPoint, 'attachShadow' in document.body, typeof Location].join()",
    );
    assert_eq!(r, "undefined,false,undefined");
}

#[test]
fn webidl_argument_conversions() {
    // WebIDL §3.2 and §3.7.5: too few arguments and a Symbol are TypeErrors;
    // null and numbers stringify; [LegacyNullToEmptyString] on data.
    let r = eval(
        PAGE,
        r#"
        const x = document.getElementById('x');
        const t = x.firstChild;
        const out = [];
        for (const f of [() => x.getAttribute(), () => x.appendChild(), () => x.getAttribute(Symbol()),
                         () => x.appendChild({}), () => x.appendChild(null)]) {
          try { f(); out.push('none'); } catch (e) { out.push(e.name); }
        }
        x.setAttribute('n', null); out.push(x.getAttribute('n'));
        x.setAttribute('v', 12); out.push(x.getAttribute('v'));
        t.data = null; out.push('[' + t.data + ']');
        out.push(x.getAttribute.length, x.appendChild.length, Text.length);
        out.push(document.createTextNode('\ud800x').data);
        out.join()"#,
    );
    assert_eq!(
        r,
        "TypeError,TypeError,TypeError,TypeError,TypeError,null,12,[],1,1,0,\u{fffd}x"
    );
}

#[test]
fn attributes() {
    // DOM §4.9: get/set/remove/toggle/has; names lower-cased on HTML
    // elements; invalid names throw InvalidCharacterError (§1.5).
    let r = eval(
        PAGE,
        r#"
        const x = document.getElementById('x');
        const out = [x.getAttribute('TITLE'), x.hasAttribute('nope'), x.getAttribute('nope')];
        x.setAttribute('DATA-K', 'v'); out.push(x.getAttribute('data-k'));
        x.removeAttribute('data-k'); out.push(x.hasAttribute('data-k'));
        x.removeAttribute('never');
        out.push(x.toggleAttribute('h'), x.hasAttribute('h'), x.toggleAttribute('h'), x.hasAttribute('h'));
        out.push(x.toggleAttribute('h', false), x.toggleAttribute('h', true), x.toggleAttribute('h', true));
        for (const bad of ['', 'a b', 'a/b', 'a=b', 'a>b']) {
          try { x.setAttribute(bad, 'v'); out.push('no error'); } catch (e) { out.push(e.name); }
        }
        out.push(document.createElement('i').hasAttributes(), x.hasAttributes());
        out.join()"#,
    );
    assert_eq!(
        r,
        "tip,false,,v,false,true,true,false,false,false,true,true,\
         InvalidCharacterError,InvalidCharacterError,InvalidCharacterError,\
         InvalidCharacterError,InvalidCharacterError,false,true"
    );
}

#[test]
fn reflection_and_ids_set_after_parse() {
    // DOM §4.9 id and className reflect; §4.2.4 getElementById sees an ID
    // however it was set, first in tree order, and an empty ID never matches.
    let r = eval(
        PAGE,
        r#"
        const x = document.getElementById('x');
        const y = document.getElementById('y');
        const out = [document.body.className, x.title, document.documentElement.lang];
        y.id = 'z'; out.push(document.getElementById('y'), document.getElementById('z') === y);
        y.setAttribute('id', 'w'); out.push(document.getElementById('z'), y.id);
        x.className = 'k'; x.title = 'new'; out.push(x.getAttribute('class'), x.getAttribute('title'));
        x.id = 'w'; out.push(document.getElementById('w') === x);
        y.removeAttribute('id'); out.push(y.id, document.getElementById('') );
        out.join()"#,
    );
    assert_eq!(r, "a b,tip,ja,,true,,w,k,new,true,,");
}

#[test]
fn comments_keep_their_data_through_the_bindings() {
    // DOM §4.11 CharacterData: data, length in UTF-16 code units.
    let r = eval(
        PAGE,
        r#"
        const c = document.body.firstChild;
        const n = document.createComment('売上😀');
        const made = new Comment('x');
        const text = new Text('y');
        c.data = c.data + '!';
        [c.data, c.nodeValue, c.textContent, n.length, n.data, made.data, text.data,
         new Text().data, c instanceof CharacterData].join('|')"#,
    );
    assert_eq!(r, " note !| note !| note !|4|売上😀|x|y||true");
}

#[test]
fn head_body_and_document_properties() {
    // HTML §3.1.3 head and body; DOM §4.5: compatMode, characterSet,
    // contentType. A missing doctype means quirks mode (HTML §13.2.6.4.1).
    let r = eval(
        PAGE,
        r#"[document.head.tagName, document.body.tagName, document.documentElement.tagName,
            document.compatMode, document.characterSet, document.contentType,
            document.documentElement.firstElementChild === document.head,
            document.documentElement.lastElementChild === document.body,
            document.documentElement.childElementCount, document.firstElementChild === document.documentElement,
            document.getElementById('y').previousElementSibling.id,
            document.getElementById('x').nextElementSibling.id,
            document.getElementById('x').firstChild.nextElementSibling.tagName].join()"#,
    );
    assert_eq!(
        r,
        "HEAD,BODY,HTML,CSS1Compat,UTF-8,text/html,true,true,2,true,x,y,B"
    );
    assert_eq!(eval("<p>no doctype", "document.compatMode"), "BackCompat");
}

#[test]
fn the_body_element_is_the_first_body_or_frameset_child_of_html() {
    // HTML §3.1.3 "the body element".
    assert_eq!(
        eval(
            "<!doctype html><frameset><frame></frameset>",
            "document.body.tagName"
        ),
        "FRAMESET"
    );
    let r = eval(
        PAGE,
        r#"const html = document.documentElement;
        const out = [];
        const first = document.createElement('frameset');
        html.insertBefore(first, document.body);
        out.push(document.body === first);
        // An element of another namespace isn't one, whatever its name.
        html.insertBefore(document.createElementNS('urn:x', 'body'), first);
        out.push(document.body === first);
        html.remove();
        // Without an html element there is no body element either.
        const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        svg.appendChild(document.createElement('body'));
        svg.appendChild(document.createElement('head'));
        document.appendChild(svg);
        out.push(document.body, document.head);
        out.join()"#,
    );
    assert_eq!(r, "true,true,,");
}

#[test]
fn every_element_has_its_interface() {
    // HTML §3.2.2 "element interface", the element index (Elements) and §16
    // for the obsolete ones.
    let r = eval(
        PAGE,
        r#"
        const of = (n) => {
          const e = document.createElement(n);
          const p = Object.getPrototypeOf(e);
          return p.constructor.name + ':' + (e instanceof HTMLElement) + ':' +
            Object.prototype.toString.call(e);
        };
        ['div', 'body', 'html', 'head', 'h3', 'tbody', 'td', 'blockquote', 'q', 'del', 'audio',
         'xmp', 'acronym', 'section', 'x-widget', 'blink', 'applet', 'annotation-xml', 'frameset',
         'selectedcontent']
          .map(of).join('|')"#,
    );
    let want = [
        "HTMLDivElement",
        "HTMLBodyElement",
        "HTMLHtmlElement",
        "HTMLHeadElement",
        "HTMLHeadingElement",
        "HTMLTableSectionElement",
        "HTMLTableCellElement",
        "HTMLQuoteElement",
        "HTMLQuoteElement",
        "HTMLModElement",
        "HTMLAudioElement",
        "HTMLPreElement",
        "HTMLElement",
        "HTMLElement",
        "HTMLElement",
        "HTMLUnknownElement",
        "HTMLUnknownElement",
        "HTMLUnknownElement",
        "HTMLFrameSetElement",
        "HTMLSelectedContentElement",
    ];
    let want: Vec<String> = want
        .iter()
        .map(|n| format!("{n}:true:[object {n}]"))
        .collect();
    assert_eq!(r, want.join("|"));
    // WebIDL §3.7: parents through the chain, and no constructing them.
    let r = eval(
        PAGE,
        r#"
        let threw = '';
        try { new HTMLDivElement(); } catch (e) { threw = e.name; }
        [document.createElement('video') instanceof HTMLMediaElement,
         Object.getPrototypeOf(HTMLDivElement) === HTMLElement, threw,
         document.createElementNS('http://www.w3.org/1999/xhtml', 'DIV') instanceof HTMLUnknownElement,
         document.body instanceof HTMLBodyElement, HTMLDocument === Document,
         Object.getOwnPropertyDescriptor(window, 'HTMLDocument').enumerable].join('|')"#,
    );
    assert_eq!(r, "true|true|TypeError|true|true|true|false");
}

#[test]
fn brands_hold_past_32_interfaces() {
    // Interfaces numbered above 31 keep their own brand: a getter of one
    // doesn't accept a wrapper of another or of an ancestor.
    let r = eval(
        PAGE,
        r#"
        const desc = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'checked');
        const out = [];
        for (const name of ['div', 'audio', 'tr', 'input']) {
          try { desc.get.call(document.createElement(name)); out.push('ok'); }
          catch (e) { out.push(e.name); }
        }
        out.join('|')"#,
    );
    assert_eq!(r, "TypeError|TypeError|TypeError|ok");
}
