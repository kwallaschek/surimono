//! MutationObserver (DOM §4.3), `<template>` (HTML §4.12.3) and reflected
//! attributes. WPT dom/nodes/MutationObserver-*.html and
//! html/semantics/scripting-1/the-template-element hold the same cases.

mod common;

use common::limits;
use surimono_core::SettleOptions;
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

const PAGE: &str =
    r#"<!doctype html><html><body><div id="a" class="k"><p id="b">x</p></div></body></html>"#;
const VARS: &str =
    "const a = document.getElementById('a'), b = document.getElementById('b');\nconst log = [];\n";

fn runtime() -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    rt
}

/// Runs `script`, then the event loop, and returns `log` joined.
fn observed(script: &str) -> String {
    let mut rt = runtime();
    let _: () = rt.eval(&format!("{VARS}{script}")).unwrap();
    rt.run_until_settled(&SettleOptions::default()).unwrap();
    rt.eval::<String>("log.join('|')").unwrap()
}

const SHOW: &str = r#"const show = (rs) => { for (const r of rs) log.push([r.type, r.target.id || r.target.nodeName,
  r.addedNodes.length, r.removedNodes.length, r.attributeName, r.attributeNamespace, r.oldValue,
  r.previousSibling && r.previousSibling.nodeName, r.nextSibling && r.nextSibling.nodeName].join()); };
"#;

#[test]
fn child_list() {
    // WPT MutationObserver-childList.html.
    let r = observed(&format!(
        r#"{SHOW}
        const o = new MutationObserver((rs, me) => {{ show(rs); log.push(me === o); }});
        o.observe(a, {{ childList: true }});
        const n = document.createElement('i');
        a.appendChild(n);
        a.insertBefore(document.createElement('u'), n);
        n.remove();
        b.appendChild(document.createElement('s')); // below a: not observed
        log.push('sync');"#
    ));
    assert_eq!(
        r,
        "sync|childList,a,1,0,,,,P,|childList,a,1,0,,,,P,I|childList,a,0,1,,,,U,|true"
    );
}

#[test]
fn normalize_queues_its_records_in_tree_order() {
    // DOM §4.4 normalize: the joined text ("replace data"), then one removal
    // per merged node; an empty node just goes. Linear normalize must not
    // lose these (merge of #96 and #105).
    let r = observed(&format!(
        r#"{SHOW}
        const t = (s) => document.createTextNode(s);
        const i = document.createElement('i');
        a.textContent = '';
        a.append(t(''), t('x'), t(''), t('y'), i, t(''), t('z'));
        i.append(t('p'), t('q'));
        const o = new MutationObserver(show);
        o.observe(a, {{ childList: true, characterData: true, characterDataOldValue: true, subtree: true }});
        a.normalize();
        log.push(a.textContent, a.childNodes.length, i.childNodes.length);"#
    ));
    assert_eq!(
        r,
        "xypqz|3|1\
         |childList,a,0,1,,,,,#text\
         |characterData,#text,0,0,,,x,,\
         |childList,a,0,1,,,,#text,#text\
         |childList,a,0,1,,,,#text,I\
         |characterData,#text,0,0,,,p,,\
         |childList,I,0,1,,,,#text,\
         |childList,a,0,1,,,,I,#text"
    );
}

#[test]
fn delivered_as_one_microtask_after_the_script() {
    // §4.3.2: one microtask, records in order; observers in creation order.
    let r = observed(
        r#"const o1 = new MutationObserver((rs) => log.push('one ' + rs.length));
        const o2 = new MutationObserver((rs) => log.push('two ' + rs.length));
        o2.observe(a, { attributes: true }); o1.observe(a, { attributes: true });
        a.id = 'x'; a.title = 'y';
        Promise.resolve().then(() => log.push('promise'));
        log.push('sync');"#,
    );
    assert_eq!(r, "sync|one 2|two 2|promise");
}

#[test]
fn attributes_and_old_values() {
    // WPT MutationObserver-attributes.html, -characterData.html.
    let r = observed(&format!(
        r#"{SHOW}
        const o = new MutationObserver(show);
        o.observe(a, {{ attributeOldValue: true, attributeFilter: ['class', 'title'] }});
        a.className = 'k2';
        a.setAttribute('lang', 'x');
        a.setAttribute('title', 't');
        a.removeAttribute('title');
        const t = b.firstChild;
        const c = new MutationObserver(show);
        c.observe(t, {{ characterDataOldValue: true }});
        t.data = 'y';
        t.appendData('z');"#
    ));
    assert_eq!(
        r,
        "attributes,a,0,0,class,,k,,|attributes,a,0,0,title,,,,|attributes,a,0,0,title,,t,,|\
         characterData,#text,0,0,,,x,,|characterData,#text,0,0,,,y,,"
    );
}

#[test]
fn subtree_take_records_and_disconnect() {
    // WPT MutationObserver-takeRecords.html, -disconnect.html.
    let r = observed(
        r#"const o = new MutationObserver((rs) => log.push('cb ' + rs.length));
        o.observe(document.body, { subtree: true, attributes: true });
        b.id = 'p';
        log.push('took ' + o.takeRecords().length, 'again ' + o.takeRecords().length);
        b.title = 'x';
        o.disconnect();
        b.title = 'y';
        // Observing again on the same node replaces the options.
        o.observe(b, { attributes: true }); o.observe(b, { childList: true });
        b.title = 'z';"#,
    );
    assert_eq!(r, "took 1|again 0");
}

#[test]
fn observe_validates_its_options() {
    // §4.3.1.
    let mut rt = runtime();
    let r: String = rt
        .eval(
            r#"const o = new MutationObserver(() => {}), out = [];
            const n = document.body;
            for (const f of [() => o.observe(n, {}), () => o.observe(n),
                             () => o.observe(n, { attributes: false, attributeOldValue: true }),
                             () => o.observe(n, { attributes: false, attributeFilter: ['a'] }),
                             () => o.observe(n, { characterData: false, characterDataOldValue: true }),
                             () => o.observe({}, { childList: true }), () => new MutationObserver(1)]) {
              try { f(); out.push('none'); } catch (e) { out.push(e.name); }
            }
            out.join()"#,
        )
        .unwrap();
    assert_eq!(r, ["TypeError"; 7].join(","));
}

#[test]
fn a_throwing_callback_is_reported_and_others_run() {
    let mut rt = runtime();
    let _: () = rt
        .eval(&format!(
            r#"{VARS}
            const bad = new MutationObserver(() => {{ throw new Error('boom'); }});
            const good = new MutationObserver(() => log.push('good'));
            bad.observe(a, {{ attributes: true }}); good.observe(a, {{ attributes: true }});
            addEventListener('error', (e) => {{ e.preventDefault(); log.push(e.message); }});
            a.id = 'x';"#
        ))
        .unwrap();
    rt.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(
        rt.eval::<String>("log.join('|')").unwrap(),
        "Error: boom|good"
    );
}

#[test]
fn templates_have_content() {
    // HTML §4.12.3; WPT the-template-element/.
    let mut rt = DomRuntime::new(limits()).unwrap();
    let page = r#"<!doctype html><body><template id="t"><p id="in">x</p><td>y</td></template><div id="d"></div>"#;
    rt.load(Document::parse(page).unwrap());
    let r: String = rt
        .eval(
            r#"const t = document.getElementById('t'), d = document.getElementById('d');
            const c = t.content;
            const out = [t instanceof HTMLTemplateElement, c === t.content, c instanceof DocumentFragment, c.nodeName,
                         t.childNodes.length, c.childNodes.length, c.firstChild.id, document.getElementById('in'),
                         document.querySelector('#in'), t.innerHTML, t.outerHTML];
            d.appendChild(c.cloneNode(true));
            d.appendChild(document.importNode(c, true));
            out.push(d.querySelectorAll('p').length, document.importNode(c, false).childNodes.length, c.childNodes.length);
            const clone = t.cloneNode(true);
            out.push(clone.content.childNodes.length, clone.content !== c, t.cloneNode().content.childNodes.length);
            t.innerHTML = '<b>n</b>';
            out.push(t.childNodes.length, c.firstChild.tagName);
            const made = document.createElement('template');
            out.push(made.content.childNodes.length);
            made.innerHTML = '<i>m</i>'; out.push(made.content.firstChild.tagName, made.innerHTML);
            out.join('|')"#,
        )
        .unwrap();
    assert_eq!(
        r,
        "true|true|true|#document-fragment|0|2|in|||<p id=\"in\">x</p>y|\
         <template id=\"t\"><p id=\"in\">x</p>y</template>|2|0|2|2|true|0|0|B|0|I|<i>m</i>"
    );
    let doc = rt.take().unwrap();
    doc.check_consistency().unwrap();
    assert!(
        doc.to_html()
            .contains(r#"<template id="t"><b>n</b></template>"#)
    );
}

#[test]
fn template_contents_stay_inside_the_limits() {
    let deep = format!("<!doctype html><template>{}", "<div>".repeat(2000));
    assert!(Document::parse(&deep).is_err());
    let doc = Document::parse(
        "<!doctype html><template><style>@import 'http://[';</style><b title=x>y</b></template>",
    )
    .unwrap();
    doc.check_consistency().unwrap();
    assert!(doc.to_html().contains("@import"));
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(doc);
    let _: () = rt
        .eval("document.querySelector('template').remove(); document.body.textContent = ''")
        .unwrap();
    rt.take().unwrap().check_consistency().unwrap();
}

#[test]
fn reflected_attributes() {
    // HTML §2.6.1 reflecting content attributes; §4.6, §4.8.3, §4.10.5.
    let mut rt = DomRuntime::new(limits()).unwrap();
    let page = r#"<!doctype html><a id="a" href="rel/x" rel="nofollow">l</a><a id="u" href="HTTP://Example.com">m</a>
    <img id="i" src="x.png" alt="pic"><input id="n" type="CHECKBOX" name="q" value="1" checked><input id="o" type="nope">
    <link id="l" href="a.css" rel="stylesheet"><script id="s" src="b.js" type="text/x"></script>"#;
    rt.load(Document::parse(page).unwrap());
    let r: String = rt
        .eval(
            r#"const $ = (i) => document.getElementById(i);
            const a = $('a'), i = $('i'), n = $('n'), l = $('l'), s = $('s');
            const out = [a.href, a.rel, $('u').href, i.src, i.alt, n.value, n.type, n.name, n.checked,
                         $('o').type, l.href, l.rel, s.src, s.type, document.createElement('a').href,
                         a instanceof HTMLAnchorElement, i instanceof HTMLImageElement, n instanceof HTMLInputElement];
            a.href = 'y'; i.src = 'z'; n.value = 'v'; n.checked = false;
            out.push(a.getAttribute('href'), i.getAttribute('src'), n.value, n.hasAttribute('checked'));
            n.checked = true; out.push(n.checked);
            out.join('|')"#,
        )
        .unwrap();
    assert_eq!(
        r,
        "rel/x|nofollow|http://example.com/|x.png|pic|1|checkbox|q|true|text|a.css|stylesheet|b.js|text/x||\
         true|true|true|y|z|v|false|true"
    );
}

#[test]
fn records_are_bounded_when_nothing_yields() {
    // Records live in Rust, outside the JS heap limit, and a script that
    // never yields never empties the queue.
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(
        Document::parse_with_limits(
            PAGE,
            DomLimits {
                max_handles: 2_000,
                ..DomLimits::default()
            },
        )
        .unwrap(),
    );
    let r = rt.eval::<()>(
        r#"const a = document.getElementById('a');
        new MutationObserver(() => {}).observe(a, { attributes: true });
        try { for (;;) a.id = 'x'; } catch (e) {}"#,
    );
    assert_eq!(r, Err(DomError::HandleLimit { limit: 2_000 }));
}

/// A runtime on `PAGE` with `max_text_bytes` of `cap`.
fn capped(cap: usize) -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(
        Document::parse_with_limits(
            PAGE,
            DomLimits {
                max_text_bytes: cap,
                ..DomLimits::default()
            },
        )
        .unwrap(),
    );
    rt
}

const CAP: usize = 1 << 20;
const TEXT_LIMIT: DomError = DomError::TextLimit { limit: CAP };
/// Bigger than `CAP` over 64 rounds, far smaller alone.
const BIG: &str = "const big = 'x'.repeat(64 * 1024); const a = document.getElementById('a');\n";

fn assert_consistent(rt: &DomRuntime) {
    rt.with_document(Document::check_consistency)
        .unwrap()
        .unwrap();
}

#[test]
fn a_parsed_template_keeps_its_content() {
    // HTML §4.12.3, §8.5, §13.2: `innerHTML` and `insertAdjacentHTML`.
    let mut rt = runtime();
    let r: String = rt
        .eval(
            r#"const d = document.getElementById('a'), out = [];
            d.innerHTML = '<template id=t><p>in</p><template id=u><i>n</i></template></template>';
            const t = d.firstChild;
            out.push(t.childNodes.length, t.content.childNodes.length, d.innerHTML);
            const u = t.content.lastChild;
            out.push(u.content.firstChild.nodeName, t.innerHTML);
            t.innerHTML = '<template><b>x</b></template>';
            out.push(t.content.firstChild.content.childNodes.length, t.innerHTML);
            d.insertAdjacentHTML('beforeend', '<template><s></s></template>');
            out.push(d.lastChild.content.firstChild.nodeName);
            out.join('|')"#,
        )
        .unwrap();
    assert_eq!(
        r,
        "0|2|<template id=\"t\"><p>in</p><template id=\"u\"><i>n</i></template></template>|\
         I|<p>in</p><template id=\"u\"><i>n</i></template>|\
         1|<template><b>x</b></template>|S"
    );
    assert_consistent(&rt);
}

#[test]
fn template_markup_counts_against_the_text_limit() {
    for add in [
        "a.insertAdjacentHTML('beforeend', '<template>' + big + '</template>');",
        "a.lastChild.after(document.createElement('div')); a.lastChild.innerHTML = '<template>' + big + '</template>';",
    ] {
        let mut rt = capped(CAP);
        let r = rt.eval::<()>(&format!("{BIG} for (let i = 0; i < 64; i++) {{ {add} }}"));
        assert_eq!(r, Err(TEXT_LIMIT), "{add}");
    }
    // Replacing frees what it replaced: the credit reaches template content.
    let mut rt = capped(CAP);
    let _: () = rt
        .eval(&format!(
            "{BIG} for (let i = 0; i < 64; i++) a.innerHTML = '<template><template>' + big + '</template></template>';"
        ))
        .unwrap();
    assert_consistent(&rt);
}

#[test]
fn old_values_count_against_the_text_limit() {
    for (options, write) in [
        ("{ attributeOldValue: true }", "a.title = big + i;"),
        (
            "{ characterDataOldValue: true, subtree: true }",
            "b.firstChild.data = big + i;",
        ),
    ] {
        let mut rt = capped(CAP);
        let r = rt.eval::<()>(&format!(
            "{BIG} const b = document.getElementById('b');
             new MutationObserver(() => {{}}).observe(a, {options});
             try {{ for (let i = 0; i < 64; i++) {write} }} catch (e) {{}}"
        ));
        assert_eq!(r, Err(TEXT_LIMIT), "{options}");
    }
}

#[test]
fn old_values_are_only_copied_when_asked_for() {
    // Without the flag nothing is kept, so writes past the cap in total pass;
    // the second observer's flag only reaches its own records.
    let mut rt = capped(CAP);
    let r: String = rt
        .eval(&format!(
            "{BIG} const b = document.getElementById('b');
             const o = new MutationObserver(() => {{}});
             o.observe(a, {{ attributes: true, characterData: true, subtree: true }});
             const keep = new MutationObserver(() => {{}});
             keep.observe(b, {{ attributes: true, attributeOldValue: true }});
             a.title = 'old'; a.title = 'new';
             b.title = 'old'; b.title = 'new';
             for (let i = 0; i < 64; i++) {{ a.title = big + i; b.firstChild.data = big + i; }}
             const j = (rs) => rs.map((r) => r.oldValue).join();
             o.takeRecords().filter((r) => r.target === b).length + j(keep.takeRecords())"
        ))
        .unwrap();
    assert_eq!(r, "2,old");
    assert_consistent(&rt);
}

#[test]
fn registrations_count_against_the_text_limit() {
    // A filter is copied per `observe`.
    let mut rt = capped(CAP);
    let r = rt.eval::<()>(&format!(
        "{BIG} const o = new MutationObserver(() => {{}});
         try {{ for (let i = 0; i < 64; i++)
           o.observe(document.createElement('i'), {{ attributes: true, attributeFilter: [big] }});
         }} catch (e) {{}}"
    ));
    assert_eq!(r, Err(TEXT_LIMIT));
    // Plain registrations too: many observers on one node, no JS heap to show.
    const SMALL: usize = 32 * 1024;
    let mut rt = capped(SMALL);
    let r = rt.eval::<()>(&format!(
        "{BIG} try {{ for (;;) new MutationObserver(() => {{}}).observe(a, {{ attributes: true }}); }}
         catch (e) {{}}"
    ));
    assert_eq!(r, Err(DomError::TextLimit { limit: SMALL }));
}

#[test]
fn registrations_are_credited_on_replace_and_disconnect() {
    let mut rt = capped(CAP);
    let _: () = rt
        .eval(&format!(
            "{BIG} const o = new MutationObserver(() => {{}});
             const options = {{ attributes: true, attributeFilter: [big] }};
             for (let i = 0; i < 64; i++) o.observe(a, options);
             for (let i = 0; i < 64; i++) {{ o.observe(document.createElement('i'), options); o.disconnect(); }}"
        ))
        .unwrap();
    assert_consistent(&rt);
}

#[test]
fn observers_belong_to_their_document() {
    // DomRuntime::load: the old document's wrappers fail with StaleDocument.
    for call in [
        "o.observe(document.body, { attributes: true });",
        "o.disconnect();",
        "o.takeRecords();",
    ] {
        let mut rt = runtime();
        let _: () = rt
            .eval("globalThis.o = new MutationObserver(() => {});")
            .unwrap();
        rt.load(Document::parse(PAGE).unwrap());
        let script = format!("try {{ {call} }} catch (e) {{}}; document.body.id = 'q';");
        assert_eq!(
            rt.eval::<()>(&script),
            Err(DomError::StaleDocument),
            "{call}"
        );
    }
}

#[test]
fn a_new_documents_records_go_to_its_own_observer() {
    let mut rt = runtime();
    let _: () = rt
        .eval(
            "globalThis.log = [];
             globalThis.old = new MutationObserver(() => log.push('A'));
             old.observe(document.body, { attributes: true });
             document.body.id = 'p';",
        )
        .unwrap();
    rt.run_until_settled(&SettleOptions::default()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let _: () = rt
        .eval(
            "const b = new MutationObserver((rs) => log.push('B', rs.length));
             b.observe(document.body, { attributes: true });
             document.body.id = 'q';",
        )
        .unwrap();
    rt.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(rt.eval::<String>("log.join()").unwrap(), "A,B,1");
}

const REPLACE: &str = r#"const seen = (o) => o.takeRecords().map((r) =>
    [r.type, [...r.addedNodes].map((n) => n.nodeName).join('+'), [...r.removedNodes].map((n) => n.nodeName).join('+'),
     r.previousSibling && r.previousSibling.nodeName, r.nextSibling && r.nextSibling.nodeName].join()).join(' ');
  const o = new MutationObserver(() => {});
  o.observe(a, { childList: true });
  const u = document.createElement('u'), s = document.createElement('s');
  "#;

#[test]
fn replacing_queues_one_record() {
    // DOM §4.2.3 "replace" and "replace all"; WPT MutationObserver-childList,
    // -inner-outer, -textContent.
    for (write, expected) in [
        ("a.replaceChild(u, b);", "childList,U,P,,"),
        ("b.replaceWith(u);", "childList,U,P,,"),
        ("b.replaceWith(u, s);", "childList,U+S,P,,"),
        ("b.outerHTML = '<i></i><b></b>';", "childList,I+B,P,,"),
        ("b.outerHTML = '';", "childList,,P,,"),
        ("a.replaceChildren(u);", "childList,U,P,,"),
        ("a.replaceChildren();", "childList,,P,,"),
        ("a.replaceChildren(b);", "childList,P,P,,"),
        ("a.textContent = 'x';", "childList,#text,P,,"),
        ("a.textContent = '';", "childList,,P,,"),
        ("a.innerHTML = '<i></i><b></b>';", "childList,I+B,P,,"),
        ("a.innerHTML = '';", "childList,,P,,"),
        (
            "u.textContent = 'x'; a.append(u); a.removeChild(u);",
            "childList,U,,P, childList,,U,P,",
        ),
        (
            "a.append(u); a.replaceChild(a.lastChild, a.firstChild);",
            "childList,U,,P, childList,,U,P, childList,U,P,,",
        ),
        ("a.replaceChild(b, b);", "childList,,P,, childList,P,,,"),
    ] {
        let mut rt = runtime();
        let r: String = rt
            .eval(&format!(
                "{VARS}{REPLACE}{write} a.textContent === null; seen(o)"
            ))
            .unwrap();
        assert_eq!(r, expected, "{write}");
    }
}

#[test]
fn inserting_a_fragment_tells_the_fragment() {
    // DOM §4.2.3 "insert": a record on the fragment with its children removed,
    // whatever suppressObservers says (it also holds in "replace").
    let mut rt = runtime();
    let r: String = rt
        .eval(&format!(
            r#"{VARS}{REPLACE}
            const f = document.createDocumentFragment();
            f.append(u, s);
            const fo = new MutationObserver(() => {{}});
            fo.observe(f, {{ childList: true }});
            a.appendChild(f);
            const first = seen(fo) + ' / ' + seen(o);
            f.append(u, s);
            seen(fo); seen(o);
            a.replaceChild(f, b);
            [first, seen(fo), seen(o)].join(' / ')"#
        ))
        .unwrap();
    assert_eq!(
        r,
        "childList,,U+S,, / childList,U+S,,P, / \
         childList,,U+S,, / childList,U+S,P,,"
    );
}

#[test]
fn template_serialization_is_content_only() {
    // HTML §13.3: a template's content, never children a script gave it.
    let mut rt = runtime();
    let r: String = rt
        .eval(
            r#"const t = document.createElement('template');
            t.appendChild(document.createElement('u'));
            const empty = [t.innerHTML, t.outerHTML];
            t.content.appendChild(document.createElement('s'));
            [...empty, t.innerHTML, t.outerHTML].join('|')"#,
        )
        .unwrap();
    assert_eq!(
        r,
        "|<template></template>|<s></s>|<template><s></s></template>"
    );
}

#[test]
fn input_value_default_on() {
    // HTML §4.10.5.1 "default/on" mode: checkbox and radio.
    let mut rt = runtime();
    let r: String = rt
        .eval(
            r#"const v = (t) => { const i = document.createElement('input'); if (t) i.type = t; return i; };
            const c = v('checkbox'); c.value = 'x';
            [v('checkbox').value, v('RADIO').value, v('text').value, v().value, v('hidden').value, c.value].join()"#,
        )
        .unwrap();
    assert_eq!(r, "on,on,,,,x");
}

#[test]
fn union_and_sequence_conversions() {
    // WebIDL §3.2.21 sequence; §3.2.24 union: null is the dictionary.
    let r = observed(
        r#"const out = [];
        const f = (x) => { try { new MutationObserver(() => {}).observe(a, { attributeFilter: x }); out.push('ok'); }
                           catch (e) { out.push(e.name); } };
        f(5); f(null); f('id'); f({}); f(['id']); f(new Set(['id']));
        const o = new MutationObserver((rs) => log.push(rs.length, rs[0].attributeName));
        o.observe(a, { attributeFilter: new Set(['title']) });
        a.id = 'n'; a.title = 't';
        const seen = [];
        const d = {};
        for (const k of ['subtree', 'characterData', 'attributes']) Object.defineProperty(d, k, { get() { seen.push(k); return true; } });
        new MutationObserver(() => {}).observe(a, d);
        out.push(seen.join('+'));
        const p = document.createElement('p'); p.innerHTML = '<i></i>';
        const count = (x) => document.importNode(p, x).childNodes.length;
        out.push(count(), count(null), count(false), count(true), count({}), count(() => {}), count({ selfOnly: true }));
        log.push(out.join());"#,
    );
    assert_eq!(
        r,
        "TypeError,TypeError,TypeError,TypeError,ok,ok,attributes+characterData+subtree,0,1,0,1,1,1,0|1|title"
    );
}
