//! IDL attributes that reflect content attributes (HTML §2.6.1 "Reflecting
//! content attributes in IDL attributes"), the ones whose rules the HTML
//! Standard writes in prose, `hidden` (§6.1) and `document.body` (§3.1.7).
//!
//! The expectations are the HTML Standard's: which attributes reflect, under
//! which content attribute name and with which rule, is read from the
//! vendored IDL (`[Reflect]`, `[ReflectURL]` and the rule's markers), and
//! each rule's getter and setter steps are §2.6.1's; the others cite their
//! section. WPT's html/dom/reflection is outside the vendored slices.
//!
//! `the_audit` also prints, for every writable attribute of the element
//! interfaces, `SVGElement` and `Document`, what a script's write does here:
//! run it with `--nocapture`.

mod common;

use std::collections::BTreeMap;

use common::limits;
use surimono_dom::{Document, DomRuntime};
use surimono_webidl::ast::{ExtAttr, ExtAttrValue, MemberKind, Type, TypeKind};

const TAGS: &[&str] = &[
    "a",
    "abbr",
    "area",
    "audio",
    "base",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "col",
    "data",
    "datalist",
    "del",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "embed",
    "fieldset",
    "font",
    "form",
    "frame",
    "frameset",
    "h1",
    "head",
    "hr",
    "html",
    "iframe",
    "img",
    "input",
    "label",
    "legend",
    "li",
    "link",
    "map",
    "marquee",
    "menu",
    "meta",
    "meter",
    "object",
    "ol",
    "optgroup",
    "option",
    "output",
    "p",
    "param",
    "picture",
    "pre",
    "progress",
    "q",
    "script",
    "select",
    "slot",
    "source",
    "span",
    "style",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "time",
    "title",
    "tr",
    "track",
    "ul",
    "video",
];

const PAGE: &str = "<!doctype html><html><head></head><body><p id=a>x</p></body></html>";

fn runtime() -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    rt
}

fn run(script: &str) -> String {
    runtime().eval(script).unwrap()
}

/// What `script` leaves for the printer: the document as HTML.
fn printed(script: &str) -> String {
    let mut rt = runtime();
    let () = rt.eval(&format!("{script}; undefined")).unwrap();
    rt.take().unwrap().to_html()
}

/// The rule an attribute reflects by, as the IDL marks it.
fn rule(ty: &Type, ext: &[ExtAttr]) -> (&'static str, String) {
    let find = |n: &str| ext.iter().find(|a| a.name == n);
    let marker = find("Reflect")
        .or(find("ReflectURL"))
        .or(find("ReflectNonNegative"))
        .or(find("ReflectPositive"))
        .or(find("ReflectPositiveWithFallback"));
    let Some(r) = marker else {
        let kind = if find("ReflectSetter").is_some() {
            "setter"
        } else if find("PutForwards").is_some() {
            "forwards"
        } else {
            "none"
        };
        return (kind, String::new());
    };
    let content = match &r.value {
        ExtAttrValue::String(s) => s.clone(),
        _ => String::new(),
    };
    let TypeKind::Named(n) = &ty.kind else {
        return ("other", content);
    };
    let string = matches!(n.as_str(), "DOMString" | "USVString");
    let kind = match n.as_str() {
        "boolean" if !ty.nullable => "bool",
        _ if string && find("ReflectURL").is_some() => "url",
        _ if string && ty.nullable => "nullable",
        _ if string => "str",
        "long" if !ty.nullable => "long",
        "unsigned long" if !ty.nullable => "ulong",
        "double" if !ty.nullable => "double",
        _ => "other",
    };
    (kind, content)
}

/// Each element interface a tag of [`TAGS`] has, with a tag for it, plus
/// `HTMLElement` and `Element` on a `div`, `SVGElement` on an SVG `g` and
/// `Document` on the document.
fn interfaces() -> BTreeMap<String, String> {
    let tags = TAGS
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(",");
    let found = run(&format!(
        "[{tags}].map((t) => t + ' ' + Object.getPrototypeOf(document.createElement(t)).constructor.name).join()"
    ));
    let mut out = BTreeMap::new();
    for pair in found.split(',') {
        let (tag, interface) = pair.split_once(' ').unwrap();
        out.entry(interface.to_owned())
            .or_insert_with(|| tag.to_owned());
    }
    out.insert("HTMLElement".to_owned(), "div".to_owned());
    out.insert("Element".to_owned(), "div".to_owned());
    out.insert("SVGElement".to_owned(), "svg:g".to_owned());
    out.insert("Document".to_owned(), "#document".to_owned());
    out
}

/// One audited attribute: interface, attribute, rule, and what a write did.
struct Row {
    interface: String,
    name: String,
    kind: String,
    status: String,
}

/// What a script's write does to each writable attribute (and each
/// `[PutForwards]` one) of [`interfaces`], event handlers aside. The element
/// is in the document, with an `href` for a hyperlink and options for a
/// select, so a setter that depends on them has something to change.
fn audit() -> Vec<Row> {
    let set = surimono_webidl::vendored::idl_set().unwrap();
    let mut cases = Vec::new();
    let mut found = interfaces();
    // The document's last: its writes change the page the others go in.
    let document = found.remove("Document");
    let order = found
        .into_iter()
        .chain(document.map(|t| ("Document".to_owned(), t)));
    for (interface, tag) in order {
        let Some(merged) = set.interface(&interface) else {
            continue;
        };
        let mut seen = Vec::new();
        for m in &merged.members {
            let MemberKind::Attribute {
                name,
                ty,
                readonly,
                is_static: false,
                ..
            } = &m.member.kind
            else {
                continue;
            };
            let forwards = m.member.ext_attrs.iter().any(|a| a.name == "PutForwards");
            let handler = matches!(&ty.kind, TypeKind::Named(n) if n.ends_with("EventHandler"));
            if (*readonly && !forwards) || handler || seen.contains(name) {
                continue;
            }
            seen.push(name.clone());
            let (kind, content) = rule(ty, &m.member.ext_attrs);
            let content = if content.is_empty() {
                name.to_ascii_lowercase()
            } else {
                content
            };
            let idl = match &ty.kind {
                TypeKind::Named(n) => n.clone(),
                _ => "other".to_owned(),
            };
            cases.push(format!(
                "['{interface}','{tag}','{name}','{kind}','{content}','{idl}']"
            ));
        }
    }
    let script = format!(
        r#"const cases = [{}];
        const setterOf = (o, name) => {{
          for (let p = o; p !== null; p = Object.getPrototypeOf(p)) {{
            const d = Object.getOwnPropertyDescriptor(p, name);
            if (d !== undefined) return d.set === undefined ? 'getter only' : 'setter';
          }}
          return 'unbound';
        }};
        const sample = (idl, before) => idl === 'boolean' ? !before
          : idl.includes('long') || idl.includes('double') || idl.includes('short') ? 3 : 'x1';
        const make = (tag) => {{
          if (tag === '#document') return document;
          const el = tag === 'svg:g' ? document.createElementNS('http://www.w3.org/2000/svg', 'g')
            : document.createElement(tag);
          if (tag === 'a' || tag === 'area') el.setAttribute('href', 'http://a.test/p?q#h');
          if (tag === 'select') el.append(document.createElement('option'), document.createElement('option'));
          const box = document.createElement('div');
          box.append(el);
          document.body.append(box);
          return el;
        }};
        const page = () => document.documentElement === null ? '' : document.documentElement.outerHTML;
        cases.map(([iface, tag, name, kind, content, idl]) => {{
          const el = make(tag);
          let status = setterOf(el, name);
          if (status === 'setter') {{
            try {{
              const has = () => el.hasAttribute(content), attr = () => el.getAttribute(content);
              let ok;
              if (kind === 'str') {{ el[name] = 'x1'; ok = attr() === 'x1' && el[name] === 'x1'; }}
              else if (kind === 'bool') {{
                el[name] = true; ok = attr() === '' && el[name] === true;
                el[name] = false; ok = ok && !has() && el[name] === false;
              }} else if (kind === 'nullable') {{
                el[name] = 'x1'; ok = attr() === 'x1' && el[name] === 'x1';
                el[name] = null; ok = ok && !has() && el[name] === null;
              }} else if (kind === 'long' || kind === 'ulong') {{ el[name] = 3; ok = attr() === '3' && el[name] === 3; }}
              else if (kind === 'double') {{ el[name] = 2.5; ok = attr() === '2.5' && el[name] === 2.5; }}
              else if (kind === 'url') {{
                el[name] = 'http://a.test/x'; ok = attr() === 'http://a.test/x' && el[name] === 'http://a.test/x';
              }} else {{
                const before = el[name], html = page();
                el[name] = sample(idl, before);
                ok = page() !== html ? 'writes the tree' : el[name] !== before ? 'keeps the value' : 'no effect';
              }}
              status = ok === true ? 'ok' : ok === false ? 'wrong' : ok;
            }} catch (e) {{ status = 'throws ' + e.name; }}
          }}
          if (el !== document && el.isConnected) el.parentNode.remove();
          return [iface, name, kind, status].join(' ');
        }}).join('\n')"#,
        cases.join(",")
    );
    run(&script)
        .lines()
        .map(|l| {
            let mut f = l.splitn(4, ' ').map(str::to_owned);
            Row {
                interface: f.next().unwrap(),
                name: f.next().unwrap(),
                kind: f.next().unwrap(),
                status: f.next().unwrap(),
            }
        })
        .collect()
}

/// Plain reflections the bindings leave out on purpose: `noModule`'s presence
/// tells a page that module scripts run, and they don't here.
const LEFT_OUT: &[(&str, &str)] = &[("HTMLScriptElement", "noModule")];

/// Every attribute the IDL marks as a plain reflection reflects: a write
/// reaches the content attribute, which is what is printed, and the getter
/// reads it back by its rule. The rest are printed for the audit.
#[test]
fn the_audit() {
    let rows = audit();
    let mut wrong = Vec::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for row in &rows {
        *counts
            .entry(format!("{}: {}", row.kind, row.status))
            .or_default() += 1;
        let plain = matches!(
            row.kind.as_str(),
            "str" | "bool" | "nullable" | "long" | "ulong" | "url" | "double"
        );
        if LEFT_OUT.contains(&(row.interface.as_str(), row.name.as_str())) {
            if row.status != "unbound" {
                wrong.push(format!("{}.{} is bound", row.interface, row.name));
            }
            continue;
        }
        if plain && row.status != "ok" {
            wrong.push(format!(
                "{}.{} ({}): {}",
                row.interface, row.name, row.kind, row.status
            ));
        }
        if row.status != "ok" {
            println!(
                "AUDIT {}.{} | {} | {}",
                row.interface, row.name, row.kind, row.status
            );
        }
    }
    println!("AUDIT counts {counts:?}");
    assert!(rows.len() > 400, "{} attributes looked at", rows.len());
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// §2.6.1's rules, each on one attribute the IDL gives it to.
#[test]
fn each_rule_reads_and_writes_as_the_standard_says() {
    let got = run(r#"const out = [];
        const make = (t) => document.createElement(t);
        // DOMString: the attribute's value, or the empty string.
        const img = make('img');
        out.push([img.alt, (img.alt = 'a', img.getAttribute('alt')), (img.setAttribute('alt', 'b'), img.alt)].join());
        // [LegacyNullToEmptyString] DOMString: null is the empty string.
        const table = make('table');
        out.push([(table.bgColor = null, table.getAttribute('bgcolor')), (img.alt = null, img.alt)].join());
        // boolean: whether the attribute is there.
        const ol = make('ol');
        out.push([ol.reversed, (ol.reversed = 1, ol.getAttribute('reversed')), (ol.reversed = 0, ol.hasAttribute('reversed'))].join());
        // A renamed one: htmlFor is `for`.
        const label = make('label');
        out.push([(label.htmlFor = 'x', label.getAttribute('for')), label.hasAttribute('htmlfor')].join());
        // long, default 1: "rules for parsing integers", else the default.
        const read = (el, name, content, v) => (el.setAttribute(content, v), el[name]);
        out.push([ol.start, read(ol, 'start', 'start', ' -7x'), read(ol, 'start', 'start', 'x'),
          read(ol, 'start', 'start', '2147483648'), (ol.start = -2, ol.getAttribute('start')),
          (ol.start = 4294967297, ol.getAttribute('start'))].join());
        // long, default 0.
        const li = make('li');
        out.push([li.value, read(li, 'value', 'value', '+5'), (li.value = 9.9, li.getAttribute('value'))].join());
        // long, limited to only non-negative numbers: -1 for none, and a
        // negative write is an IndexSizeError.
        const input = make('input');
        let thrown = 'none';
        try { input.maxLength = -1; } catch (e) { thrown = e.name; }
        out.push([input.maxLength, read(input, 'maxLength', 'maxlength', '-3'), thrown].join());
        // unsigned long, clamped to [1, 1000], default 1.
        const td = make('td');
        out.push([td.colSpan, read(td, 'colSpan', 'colspan', '0'), read(td, 'colSpan', 'colspan', '5000'),
          read(td, 'colSpan', 'colspan', '-1'), read(td, 'colSpan', 'colspan', '7'),
          (td.colSpan = 3, td.getAttribute('colspan')), (td.colSpan = 2147483648, td.getAttribute('colspan'))].join());
        // unsigned long, clamped to [0, 65534], default 1.
        out.push([td.rowSpan, read(td, 'rowSpan', 'rowspan', '0'), read(td, 'rowSpan', 'rowspan', '70000')].join());
        // unsigned long, limited to only positive numbers with fallback, default 2 and 20.
        const area = make('textarea');
        out.push([area.rows, area.cols, read(area, 'rows', 'rows', '0'), read(area, 'rows', 'rows', '4'),
          (area.rows = 0, area.getAttribute('rows')), (area.cols = 5, area.getAttribute('cols'))].join());
        // double, limited to only positive numbers, default 1.0: "rules for
        // parsing floating-point number values"; a write that isn't positive
        // does nothing, and the others are the number's shortest string.
        const p = make('progress');
        out.push([p.max, read(p, 'max', 'max', ' 2.5e1x'), read(p, 'max', 'max', '.5'), read(p, 'max', 'max', '-3'),
          read(p, 'max', 'max', '1e400'), (p.max = 0, p.getAttribute('max')), (p.max = 1e21, p.getAttribute('max'))].join());
        // A URL: parsed against the document's base, or as it is.
        const base = make('base'); base.setAttribute('href', 'http://a.test/dir/'); document.head.append(base);
        out.push([img.src, read(img, 'src', 'src', 'x.png'), read(img, 'src', 'src', 'http://[bad'),
          (img.src = 'y.png', img.getAttribute('src'))].join());
        out.join(' | ')"#);
    assert_eq!(
        got,
        ",a,b | ,null | false,,false | x,false | 1,-7,1,1,-2,1 | 0,5,9 | \
         -1,-1,IndexSizeError | 1,1,1000,1,7,3,1 | 1,0,65534 | 2,20,2,4,2,5 | \
         1,25,0.5,1,1,1e400,1e+21 | ,http://a.test/dir/x.png,http://[bad,y.png"
    );
}

/// HTML §6.1 "The hidden attribute", the getter's and the setter's steps.
#[test]
fn hidden_follows_its_steps() {
    let got = run(r#"const a = document.getElementById('a'), out = [];
        const state = () => [a.hidden, a.getAttribute('hidden')].join(':');
        out.push(state());
        for (const v of [true, false, 'x', '', 1, 0, NaN, null, 'UNTIL-found', undefined, {}, -0, 'false']) {
          a.hidden = v; out.push(state());
        }
        a.setAttribute('hidden', 'Until-Found'); out.push(a.hidden);
        a.setAttribute('hidden', 'other'); out.push(a.hidden);
        a.removeAttribute('hidden'); out.push(a.hidden);
        a.hidden = true;
        out.push('hidden' in HTMLElement.prototype, Object.keys(a).length, a.outerHTML);
        out.join(' ')"#);
    assert_eq!(
        got,
        "false: true: false: true: false: true: false: false: false: until-found:until-found \
         false: true: false: true: until-found true false true 0 <p id=\"a\" hidden=\"\">x</p>"
    );
}

/// What a hidden element is for: it is in the printed page as hidden.
#[test]
fn a_hidden_element_prints_hidden() {
    let html = printed("document.getElementById('a').hidden = true");
    assert!(html.contains("<p id=\"a\" hidden=\"\">x</p>"), "{html}");
}

/// HTML §3.1.7, the `body` setter's five steps.
#[test]
fn document_body_can_be_set() {
    let got = run(r#"const out = [];
        const thrown = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
        const old = document.body;
        // 1. Not a body or frameset element. (What isn't an HTMLElement at all
        //    never reaches the steps: WebIDL refuses it for `HTMLElement?`.)
        out.push(thrown(() => { document.body = document.createElement('div'); }),
                 thrown(() => { document.body = null; }),
                 thrown(() => { document.body = document.createElementNS('http://www.w3.org/2000/svg', 'body'); }));
        // 2. The same element: nothing happens.
        document.body = old; out.push(document.body === old);
        // 3. Another body replaces it, where it was.
        const b = document.createElement('body'); b.id = 'new';
        document.body = b;
        out.push(document.body === b, old.parentNode === null, document.documentElement.lastChild === b,
                 document.querySelectorAll('body').length);
        // A frameset is a body element too.
        const f = document.createElement('frameset');
        document.body = f; out.push(document.body === f, b.parentNode === null);
        // 5. No body: appended to the document element.
        f.remove(); out.push(document.body === null);
        document.body = b; out.push(document.documentElement.lastChild === b);
        // 4. No document element.
        document.documentElement.remove();
        out.push(thrown(() => { document.body = document.createElement('body'); }));
        out.join()"#);
    assert_eq!(
        got,
        "HierarchyRequestError,HierarchyRequestError,TypeError,true,\
         true,true,true,1,true,true,true,true,HierarchyRequestError"
    );
}

#[test]
fn a_new_body_is_what_prints() {
    assert_eq!(
        printed(
            "const b = document.createElement('body'); b.textContent = 'new'; document.body = b"
        ),
        "<html><head></head><body>new</body></html>"
    );
}

/// The attributes whose reflection the HTML Standard writes in prose, each by
/// its section's steps.
#[test]
fn prose_reflections_follow_their_steps() {
    let got = run(r#"const out = [];
        const make = (t) => document.createElement(t);
        const thrown = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
        const read = (el, name, content, v) => (el.setAttribute(content, v), el[name]);
        // §3.2.6.4: `dir`, limited to only known values.
        const d = make('div');
        out.push([d.dir, read(d, 'dir', 'dir', 'RTL'), read(d, 'dir', 'dir', 'up'),
          (d.dir = 'Auto', d.getAttribute('dir')), d.dir].join());
        // The document's is the html element's.
        out.push([document.dir, (document.dir = 'rtl', document.documentElement.getAttribute('dir')),
          (document.documentElement.setAttribute('dir', 'LTR'), document.dir)].join());
        // §6.12: `popover`, nullable and limited to only known values: missing
        // is null, empty is "auto", an invalid value is "manual".
        out.push([d.popover, read(d, 'popover', 'popover', ''), read(d, 'popover', 'popover', 'AUTO'),
          read(d, 'popover', 'popover', 'Hint'), read(d, 'popover', 'popover', 'bogus'),
          (d.popover = 'x', d.getAttribute('popover')), (d.popover = undefined, d.hasAttribute('popover'))].map(String).join());
        // DOM §4.9: `slot` is the attribute.
        out.push([d.slot, (d.slot = 's1', d.getAttribute('slot'))].join());
        // §4.12.5: a canvas reflects with the defaults 300 and 150.
        const c = make('canvas');
        out.push([c.width, c.height, (c.width = 10, c.getAttribute('width')), read(c, 'width', 'width', '-5'),
          read(c, 'width', 'width', ' 7px'), (c.height = -1, c.getAttribute('height'))].join());
        // §4.8.3 and §4.8.17: an img not being rendered has its attribute's
        // dimensions, as a non-negative integer, or 0 with no image.
        const img = make('img');
        out.push([img.width, read(img, 'width', 'width', '40'), read(img, 'width', 'width', '-4'),
          (img.width = 25, img.getAttribute('width')), (img.height = -1, img.getAttribute('height')),
          (document.body.append(img), img.width)].join());
        // §4.10.5: an input's dimensions are an image button's only.
        const input = make('input'); input.setAttribute('width', '30');
        out.push([input.width, (input.type = 'IMAGE', input.width), (input.width = 12, input.getAttribute('width'))].join());
        // §4.10.13: a progress bar's value is 0 when indeterminate, else its
        // current value, at most its maximum.
        const p = make('progress');
        const pv = () => [p.value, p.max].join(':');
        out.push(pv());
        p.setAttribute('value', '0.5'); out.push(pv());
        p.setAttribute('max', '0.25'); out.push(pv());
        p.setAttribute('max', '-1'); out.push(pv());
        p.setAttribute('max', '100');
        out.push([' 1.5e1x', '+.5', '1.', '1e400', 'e5', '-3', '2E-1'].map((v) => read(p, 'value', 'value', v)).join());
        p.value = 7; out.push(p.getAttribute('value'));
        p.max = 2.5; out.push([p.getAttribute('max'), p.value].join());
        out.push(thrown(() => { p.value = NaN; }), thrown(() => { p.max = Infinity; }));
        p.value = 1e21; out.push(p.getAttribute('value'));
        p.removeAttribute('value'); out.push(p.value);
        // §4.10.14: a meter's six points, each clamped by the ones before it.
        const m = make('meter');
        const mv = () => [m.min, m.max, m.value, m.low, m.high, m.optimum].join(':');
        out.push(mv());
        m.setAttribute('min', '10'); out.push(mv());
        m.setAttribute('max', '20'); m.setAttribute('value', '25'); m.setAttribute('low', '5');
        m.setAttribute('high', '12'); m.setAttribute('optimum', 'x'); out.push(mv());
        m.setAttribute('high', '8'); out.push(mv());
        m.value = -2.5; out.push([m.getAttribute('value'), m.value].join());
        m.setAttribute('min', '-0'); out.push(Object.is(m.min, 0));
        // §4.10.12: an output's value is its text, and its default value the
        // text until `value` is set.
        const o = make('output'); o.textContent = 'a';
        out.push([o.value, o.defaultValue].join());
        o.defaultValue = 'b'; out.push([o.value, o.defaultValue, o.textContent].join());
        o.value = 'c'; out.push([o.value, o.defaultValue].join());
        o.defaultValue = 'd'; out.push([o.value, o.defaultValue, o.textContent].join());
        // §4.2.2 and §4.12.1: `text` is the child text content.
        for (const t of ['title', 'script']) {
          const e = make(t), b = make('b'); b.textContent = 'X';
          e.append('a', b, 'c');
          out.push([e.text, e.textContent, (e.text = 'new', e.childNodes.length), e.textContent].join());
        }
        // §4.2.3: a base element's href ignores every base element; here the
        // fallback base URL is about:blank, against which nothing relative parses.
        const b1 = make('base'), b2 = make('base');
        b1.setAttribute('href', 'http://other.test/'); document.head.append(b1, b2);
        out.push([b2.href, read(b2, 'href', 'href', 'sub/'), read(b2, 'href', 'href', 'http://[bad'),
          (b2.href = 'x', b2.getAttribute('href'))].join());
        // §16.3.3: the document's colours are the body's attributes.
        const body = document.body;
        out.push([document.bgColor, (document.bgColor = 'red', body.getAttribute('bgcolor')),
          (document.fgColor = null, body.getAttribute('text')), (document.linkColor = 'blue', body.getAttribute('link')),
          (document.vlinkColor = 'v', body.getAttribute('vlink')), (document.alinkColor = 'w', body.getAttribute('alink')),
          document.bgColor].join());
        // `dataset` on an SVG element, as on an HTML one.
        const g = document.createElementNS('http://www.w3.org/2000/svg', 'g');
        g.dataset.fooBar = '1';
        out.push([g.getAttribute('data-foo-bar'), g.dataset === g.dataset, g.dataset.fooBar].join());
        out.join(' | ')"#);
    assert_eq!(
        got,
        ",rtl,,Auto,auto | ,rtl,ltr | null,auto,auto,hint,manual,x,false | ,s1 | \
         300,150,10,300,7,150 | 0,40,0,25,0,25 | 0,30,12 | \
         0:1 | 0.5:1 | 0.25:0.25 | 0.5:1 | 15,0.5,1,0,0,0,0.2 | 7 | 2.5,2.5 | \
         TypeError | TypeError | 1e+21 | 0 | \
         0:1:0:0:1:0.5 | 10:10:10:10:10:10 | 10:20:20:10:12:15 | 10:20:20:10:10:15 | -2.5,10 | true | \
         a,a | b,b,b | c,b | c,d,c | ac,aXc,1,new | ac,aXc,1,new | \
         ,sub/,http://[bad,x | ,red,,blue,v,w,red | 1,true,1"
    );
}

/// With no body element, or a frameset as the body, the document's colours
/// are empty and a write does nothing (§16.3.3); with no html element,
/// `document.dir` is empty and a write does nothing (§3.2.6.4).
#[test]
fn document_reflections_without_their_element() {
    let got = run(r#"const out = [];
        const f = document.createElement('frameset');
        document.body = f;
        document.bgColor = 'red';
        out.push(document.bgColor, f.hasAttribute('bgcolor'));
        const html = document.documentElement;
        html.remove();
        document.dir = 'rtl';
        out.push(document.dir, html.hasAttribute('dir'));
        const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        document.append(svg);
        document.dir = 'rtl';
        out.push(document.dir, svg.hasAttribute('dir'));
        out.join()"#);
    assert_eq!(got, ",false,,false,,false");
}

/// A base element's href against a document with a URL (§4.2.3).
#[test]
fn a_base_href_is_parsed_against_the_document_url() {
    let mut doc = Document::parse(PAGE).unwrap();
    doc.set_url("http://a.test/dir/page.html#top");
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(doc);
    let got: String = rt
        .eval(
            r#"const b1 = document.createElement('base'), b2 = document.createElement('base');
            b1.setAttribute('href', 'http://other.test/'); document.head.append(b1, b2);
            const out = [b2.href];
            b2.setAttribute('href', 'sub/'); out.push(b2.href);
            out.join()"#,
        )
        .unwrap();
    assert_eq!(got, "http://a.test/dir/page.html,http://a.test/dir/sub/");
}

/// HTML §3.2.7, the `outerText` setter's steps: the rendered text fragment
/// takes the element's place, and the Text nodes on either side merge.
#[test]
fn outer_text_follows_its_steps() {
    let got = run(r#"const out = [];
        const box = (html) => { const d = document.createElement('div'); d.innerHTML = html; document.body.append(d); return d; };
        let d = box('a<span>b</span>c');
        out.push(d.firstElementChild.outerText);
        d.firstElementChild.outerText = 'X\r\nY\rZ\n';
        out.push(d.innerHTML, d.childNodes.length);
        d = box('a<span>b</span>c');
        d.firstElementChild.outerText = 'M';
        out.push(d.innerHTML, d.childNodes.length);
        for (const v of ['', null]) {
          d = box('a<span>b</span>c');
          d.firstElementChild.outerText = v;
          out.push(d.innerHTML, d.childNodes.length);
        }
        d = box('<i>a</i><span>b</span><i>c</i>');
        d.querySelector('span').outerText = 'M';
        out.push(d.innerHTML);
        try { document.createElement('span').outerText = 'x'; out.push('none'); } catch (e) { out.push(e.name); }
        out.join(' | ')"#);
    assert_eq!(
        got,
        "b | aX<br>Y<br>Z<br>c | 7 | aMc | 1 | ac | 1 | ac | 1 | <i>a</i>M<i>c</i> | NoModificationAllowedError"
    );
}

/// What these writes are for: each is in the page the printer gets.
#[test]
fn what_a_script_sets_is_printed() {
    let html = printed(
        r#"const a = document.getElementById('a');
        a.dir = 'rtl'; a.popover = 'manual'; a.slot = 's'; a.inert = true; a.lang = 'ja';
        document.dir = 'rtl'; document.bgColor = 'red';
        const c = document.createElement('canvas'); c.width = 640; document.body.append(c);
        const img = document.createElement('img'); img.width = 25; img.srcset = 'x.png 2x'; document.body.append(img);
        const p = document.createElement('progress'); p.value = 0.5; p.max = 2; document.body.append(p);
        const m = document.createElement('meter'); m.value = 3; m.max = 4; document.body.append(m);
        const o = document.createElement('output'); o.value = 'out'; document.body.append(o);
        const table = document.createElement('table'); document.body.append(table);
        const td = table.insertRow().insertCell(); td.colSpan = 2; td.vAlign = 'top';
        const ol = document.createElement('ol'); ol.start = 3; ol.reversed = true; document.body.append(ol);
        const g = document.createElementNS('http://www.w3.org/2000/svg', 'g'); g.dataset.k = 'v'; document.body.append(g)"#,
    );
    for piece in [
        r#"<html dir="rtl">"#,
        r#"<body bgcolor="red">"#,
        r#"<p id="a" dir="rtl" popover="manual" slot="s" inert="" lang="ja">x</p>"#,
        r#"<canvas width="640"></canvas>"#,
        r#"<img width="25" srcset="x.png 2x">"#,
        r#"<progress value="0.5" max="2"></progress>"#,
        r#"<meter value="3" max="4"></meter>"#,
        r#"<output>out</output>"#,
        r#"<td colspan="2" valign="top"></td>"#,
        r#"<ol start="3" reversed=""></ol>"#,
        r#"<g data-k="v"></g>"#,
    ] {
        assert!(html.contains(piece), "{piece} not in {html}");
    }
}
