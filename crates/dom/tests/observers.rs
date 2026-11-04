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
        for (;;) { try { a.id = 'x'; } catch (e) {} }"#,
    );
    assert_eq!(r, Err(DomError::HandleLimit { limit: 2_000 }));
}
