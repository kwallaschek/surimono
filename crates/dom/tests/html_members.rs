//! Tables, forms, selects, links, focus and `innerText` against the HTML Standard
//! (§4.9, §4.10, §4.6.3, §6.6, §3.2.7). Expected values are the standard's, or what
//! WPT's html/semantics/tabular-data and url tests assert.

mod common;

use common::limits;
use surimono_dom::{Document, DomRuntime};

fn runtime(page: &str) -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    let mut doc = Document::parse(page).unwrap();
    doc.set_url("http://example.test/a/b.html?q=1");
    rt.load(doc);
    rt
}

fn run(page: &str, script: &str) -> String {
    runtime(page).eval(script).unwrap()
}

/// `f()`'s exception's name, or `none`.
const THROWN: &str =
    "const thrown = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };";

const TABLE: &str = "<!doctype html><table id=t><caption>c</caption><colgroup></colgroup>
  <tfoot><tr id=f><td>f</td></tr></tfoot>
  <tr id=x><td id=c1>1</td><th id=c2>2</th><td id=c3>3</td></tr>
  <tbody id=b1><tr id=r1><td>a</td></tr><tr id=r2><td>b</td></tr></tbody>
  <thead><tr id=h><td>h</td></tr></thead>
  <tbody id=b2></tbody></table>";

#[test]
fn table_lists_follow_the_standard() {
    assert_eq!(
        run(
            TABLE,
            r#"const el = (i) => document.getElementById(i);
            const t = el('t');
            [[...t.rows].map((r) => r.id).join(), [...t.tBodies].map((b) => b.id).join(),
             t.rows === t.rows, t.caption.textContent, t.tHead.tagName, t.tFoot.tagName,
             el('r2').rowIndex, el('r2').sectionRowIndex, el('f').rowIndex, el('x').rowIndex,
             el('x').sectionRowIndex, [...el('x').cells].map((c) => c.id).join(), el('c3').cellIndex,
             [...el('b1').rows].map((r) => r.id).join()].join(' ')"#
        ),
        "h,x,r1,r2,f ,b1,b2 true c THEAD TFOOT 3 1 4 1 0 c1,c2,c3 2 r1,r2"
    );
}

#[test]
fn rows_and_cells_are_inserted_as_the_standard_says() {
    assert_eq!(
        run(
            "<!doctype html><table id=t></table>",
            r#"const t = document.getElementById('t');
            const a = t.insertRow();
            const first = t.innerHTML;
            const c = a.insertCell();
            c.textContent = 'x';
            a.insertCell(0).textContent = 'y';
            const r = t.insertRow(0);
            r.id = 'r';
            const s = t.insertRow(-1);
            s.id = 's';
            const body = t.tBodies[0];
            [first, a.outerHTML, [...t.rows].map((x) => x.id).join(), body.rows.length,
             c.cellIndex].join(' ')"#
        ),
        "<tbody><tr></tr></tbody> <tr><td>y</td><td>x</td></tr> r,,s 3 1"
    );
    assert_eq!(
        run(
            "<!doctype html><table id=t><tbody id=b><tr id=a></tr></tbody></table>",
            &format!(
                r#"{THROWN}const t = document.getElementById('t');
            const b = document.getElementById('b'), a = document.getElementById('a');
            [thrown(() => t.insertRow(5)), thrown(() => t.insertRow(-2)), thrown(() => t.deleteRow(5)),
             thrown(() => t.deleteRow(-2)), thrown(() => a.insertCell(1)), thrown(() => a.deleteCell(0)),
             thrown(() => b.insertRow(9)), thrown(() => b.deleteRow(9)),
             thrown(() => {{ t.tHead = document.createElement('tbody'); }}),
             thrown(() => {{ t.tHead = document.createElement('div'); }})].join()"#
            )
        ),
        "IndexSizeError,IndexSizeError,IndexSizeError,IndexSizeError,IndexSizeError,IndexSizeError,\
         IndexSizeError,IndexSizeError,HierarchyRequestError,TypeError"
    );
    assert_eq!(
        run(
            "<!doctype html><table id=t><tr id=a></tr><tr id=b></tr></table>",
            r#"const t = document.getElementById('t');
            t.deleteRow(-1);
            t.deleteRow(0);
            t.deleteRow(-1);
            String(t.rows.length)"#
        ),
        "0"
    );
}

#[test]
fn table_sections_go_where_the_standard_puts_them() {
    assert_eq!(
        run(
            "<!doctype html><table id=t><caption></caption><colgroup></colgroup><tr></tr></table>",
            r#"const t = document.getElementById('t');
            const h = t.createTHead();
            const same = t.createTHead() === h;
            const f = t.createTFoot();
            const b = t.createTBody();
            const c = t.createCaption();
            const order = () => [...t.children].map((e) => e.localName).join();
            const one = order();
            t.deleteTHead(); t.deleteTFoot(); t.deleteCaption();
            const two = order();
            const cap = document.createElement('caption');
            t.caption = cap;
            [same, c === cap, one, two, order(), t.caption === cap].join(' ')"#
        ),
        "true false caption,colgroup,thead,tbody,tbody,tfoot colgroup,tbody,tbody caption,colgroup,tbody,tbody true"
    );
}

#[test]
fn cell_attributes_reflect_as_the_standard_says() {
    assert_eq!(
        run(
            "<!doctype html><table><tr><td id=a colspan=0 rowspan=0 scope=ROW></td><td id=b colspan=2000 rowspan=x></td><td id=c></td></table>",
            r#"const [a, b, c] = ['a', 'b', 'c'].map((i) => document.getElementById(i));
            c.colSpan = 3; c.rowSpan = 4;
            [a.colSpan, a.rowSpan, a.scope, b.colSpan, b.rowSpan, b.scope, c.getAttribute('colspan'), c.rowSpan].join()"#
        ),
        "1,0,row,1000,1,,3,4"
    );
}

const SELECT: &str = "<!doctype html><form id=f name=n><select id=s name=q>
    <option id=o1 value=1>One</option><optgroup><option id=o2>  Two   words </option><option id=o3 disabled>3</option></optgroup>
    <option id=o4 label=L value=4></option></select></form>";

#[test]
fn a_select_reads_as_the_standard_says() {
    assert_eq!(
        run(
            SELECT,
            r#"const s = document.getElementById('s');
            const o = s.options;
            [s.type, s.length, o.length, o === s.options, o[1].id, o.item(3).label, o.namedItem('o2').text,
             o.o3.id, s.selectedIndex, s.value, s.selectedOptions.length, o[0].selected, o[1].selected,
             o[1].value, o[1].index, o[3].index, o[1].form.id, s.form.id, s.item(2).id, s.namedItem('o4').id,
             o instanceof HTMLOptionsCollection, o instanceof HTMLCollection, o[0] instanceof HTMLOptionElement].join('|')"#
        ),
        "select-one|4|4|true|o2|L|Two words|o3|0|1|1|true|false|Two words|1|3|f|f|o3|o4|true|true|true"
    );
}

/// Our decision, not outside truth: `selected` is the attribute, so what a script
/// chooses reaches the printed page (docs/DECISIONS.md). Chromium 153 leaves the
/// attribute alone, so the `hasAttribute` and `outerHTML` values here are ours.
#[test]
fn a_select_is_written_as_the_standard_says() {
    assert_eq!(
        run(
            SELECT,
            r#"const s = document.getElementById('s');
            s.selectedIndex = 1;
            const a = [s.value, s.options[1].hasAttribute('selected'), s.outerHTML.split('selected').length - 1];
            s.value = '4';
            a.push(s.selectedIndex, s.options[1].selected, s.options[3].selected);
            s.options[0].selected = true;
            a.push(s.selectedIndex, s.options[3].hasAttribute('selected'));
            s.multiple = true;
            s.options[3].selected = true;
            a.push(s.type, s.selectedOptions.length, s.selectedIndex);
            a.join()"#
        ),
        "Two words,true,1,3,false,true,0,false,select-multiple,2,0"
    );
    assert_eq!(
        run(
            SELECT,
            r#"const s = document.getElementById('s');
            s.add(new Option('Six', '6', true, true));
            const a = [s.length, s.value, s.options[4].text, s.options[4] instanceof HTMLOptionElement];
            s.add(new Option('Zero', '0'), 0);
            s.add(new Option('Mid'), s.options[2]);
            a.push(s.options[0].text, s.options[2].text);
            s.remove(0);
            s.options.remove(0);
            a.push(s.length, s.options[0].text);
            s.options.length = 2;
            a.push(s.length);
            s.options.length = 4;
            a.push(s.length, s.options[3].value);
            s.options[5] = new Option('Far');
            a.push(s.length, s.options[5].text);
            s.options[0] = new Option('New');
            a.push(s.length, s.options[0].text);
            s.options[0] = null;
            a.push(s.length);
            s.options.length = 0;
            a.push(s.length);
            s.remove();
            a.push(s.isConnected);
            a.join()"#
        ),
        "5,6,Six,true,Zero,Mid,5,Mid,2,4,,6,Far,6,New,5,0,false"
    );
}

/// HTML §4.10.7: a script can leave a single select with nothing chosen.
#[test]
fn a_select_can_have_nothing_selected() {
    assert_eq!(
        run(
            SELECT,
            r#"const s = document.getElementById('s');
            s.selectedIndex = -1;
            const a = [s.selectedIndex, JSON.stringify(s.value)];
            s.value = 'nothing';
            a.push(s.selectedIndex);
            s.value = '4';
            a.push(s.selectedIndex, s.value);
            a.join()"#
        ),
        "-1,\"\",-1,3,4"
    );
}

#[test]
fn form_controls_are_the_forms() {
    const PAGE: &str = "<!doctype html><form id=f action=go method=POST enctype=TEXT/plain>
        <input id=i name=a><input type=image id=m><button id=b></button><fieldset id=s><textarea id=t name=x></textarea></fieldset>
        <select id=e></select><output id=o></output><object id=j></object><label id=l></label><img id=g></form>
        <input id=outside form=f><input id=none>";
    assert_eq!(
        run(
            PAGE,
            r#"const f = document.getElementById('f');
            const e = f.elements;
            [[...e].map((x) => x.id).join(), f.length, e === f.elements, e.a.id, e.namedItem('x').id,
             e instanceof HTMLFormControlsCollection, e instanceof HTMLCollection, f.action, f.method,
             f.enctype, f.encoding, document.forms.length, document.forms[0] === f].join(' ')"#
        ),
        "i,b,s,t,e,o,j,outside 8 true i t true true http://example.test/a/go post text/plain text/plain 1 true"
    );
    assert_eq!(
        run(
            "<!doctype html><form id=f></form><form id=g action=''></form><form id=h method=nope enctype=x></form>",
            r#"const [f, g, h] = ['f', 'g', 'h'].map((i) => document.getElementById(i));
            [f.action, g.action, h.method, h.enctype].join(' ')"#
        ),
        "http://example.test/a/b.html?q=1 http://example.test/a/b.html?q=1 get application/x-www-form-urlencoded"
    );
}

#[test]
fn a_textareas_value_reaches_the_page() {
    assert_eq!(
        run(
            "<!doctype html><textarea id=t>a</textarea>",
            r#"const t = document.getElementById('t');
            const was = t.value;
            t.value = 'b<c';
            [was, t.value, t.defaultValue, t.outerHTML].join(' ')"#
        ),
        "a b<c b<c <textarea id=\"t\">b&lt;c</textarea>"
    );
}

const LINKS: &str = "<!doctype html><a id=a href='/x/y?z=1#h'>a</a><a id=u href='https://u:p@x.test:8/p'>u</a>
    <a id=n name=anchor>n</a><a id=bad href='http://['>b</a><map><area id=m href='#m'></map><img id=i><embed id=e>";

#[test]
fn links_decompose_their_urls() {
    assert_eq!(
        run(
            LINKS,
            r#"const el = (i) => document.getElementById(i);
            const a = el('a'), u = el('u'), n = el('n'), bad = el('bad'), m = el('m');
            [a.href, a.protocol, a.host, a.hostname, a.port, a.pathname, a.search, a.hash, a.origin,
             u.username, u.password, u.port, u.origin, n.href, n.protocol, n.pathname, bad.protocol, bad.pathname,
             bad.href, m.hash, m.pathname, a.text, String(a)].join(' ')"#
        ),
        "http://example.test/x/y?z=1#h http: example.test example.test  /x/y ?z=1 #h http://example.test \
         u p 8 https://x.test:8  :  :  http://[ #m /a/b.html a http://example.test/x/y?z=1#h"
    );
    assert_eq!(
        run(
            LINKS,
            r#"const a = document.getElementById('a'), n = document.getElementById('n');
            a.pathname = '/q r'; a.search = 's=t'; a.hash = ''; a.host = 'y.test:9'; a.protocol = 'https';
            n.pathname = '/nope';
            [a.getAttribute('href'), n.hasAttribute('href')].join(' ')"#
        ),
        "https://y.test:9/q%20r?s=t false"
    );
}

#[test]
fn the_documents_lists_are_live() {
    assert_eq!(
        run(
            LINKS,
            r#"const a = [document.links.length, document.images.length, document.anchors.length,
              document.embeds.length, document.plugins === undefined, document.images === document.images];
            document.body.append(document.createElement('img'), Object.assign(document.createElement('a'), { href: 'x' }));
            document.getElementById('i').id = 'renamed';
            a.push(document.links.length, document.images.length, document.images.renamed.tagName);
            a.join()"#
        ),
        "4,1,1,1,false,true,5,2,IMG"
    );
}

#[test]
fn the_cookie_is_empty() {
    assert_eq!(
        run(
            "<!doctype html>",
            "document.cookie = 'a=b'; JSON.stringify(document.cookie)"
        ),
        "\"\""
    );
}

#[test]
fn factories_make_elements() {
    assert_eq!(
        run(
            "<!doctype html>",
            r#"const i = new Image(3, 4);
            const o = new Option();
            [i.tagName, i.getAttribute('width'), i.getAttribute('height'), i instanceof HTMLImageElement,
             Image.name, Option.prototype === HTMLOptionElement.prototype, o.outerHTML].join(' ')"#
        ),
        "IMG 3 4 true Image true <option></option>"
    );
}

const FOCUS: &str =
    "<!doctype html><body id=body><input id=i><button id=b disabled></button><div id=d></div>
    <div id=t tabindex=2></div><a id=l href=x></a><a id=no></a><input id=h type=hidden>";

#[test]
fn focus_moves_with_its_events() {
    assert_eq!(
        run(
            FOCUS,
            r#"const log = [];
            for (const type of ['focus', 'blur', 'focusin', 'focusout']) {
              document.addEventListener(type, (e) => log.push(type + ':' + e.target.id + ':' + (e.relatedTarget && e.relatedTarget.id) + ':' + e.bubbles + ':' + e.composed), true);
            }
            const el = (x) => document.getElementById(x);
            const a = [document.activeElement.id, document.hasFocus()];
            el('i').focus();
            a.push(document.activeElement.id);
            el('t').focus();
            a.push(document.activeElement.id);
            el('t').focus();
            el('d').focus(); el('b').focus(); el('no').focus(); el('h').focus();
            a.push(document.activeElement.id);
            el('d').blur();
            a.push(document.activeElement.id);
            el('t').blur();
            a.push(document.activeElement.id);
            a.join() + ' ' + log.join()"#
        ),
        "body,true,i,t,t,t,body focus:i:null:false:true,focusin:i:null:true:true,blur:i:t:false:true,\
         focusout:i:t:true:true,focus:t:i:false:true,focusin:t:i:true:true,blur:t:null:false:true,\
         focusout:t:null:true:true"
    );
    assert_eq!(
        run(
            FOCUS,
            r#"const i = document.getElementById('i');
            i.focus();
            i.remove();
            [document.activeElement.id, i.tabIndex, document.getElementById('d').tabIndex,
             document.getElementById('t').tabIndex].join()"#
        ),
        "body,0,-1,2"
    );
}

#[test]
fn inner_text_writes_lines() {
    assert_eq!(
        run(
            "<!doctype html><div id=d><b>x</b></div>",
            r#"const d = document.getElementById('d');
            d.innerText = 'a<b\nc\r\nd\re\n';
            const h = d.innerHTML;
            d.innerText = '';
            const empty = d.childNodes.length;
            d.innerText = 'one';
            [h, empty, d.innerHTML, d.childNodes.length].join(' ')"#
        ),
        "a&lt;b<br>c<br>d<br>e<br> 0 one 1"
    );
}

#[test]
fn inner_text_reads_what_is_rendered() {
    const PAGE: &str = "<!doctype html><div id=d>  a  <b> b </b>c<script>no</script><style>no</style>
        <p>para</p><p>two</p>end<br>line<div hidden>no</div><div style='display:none'>no</div>
        <pre> keep  \n this</pre><table><tr><td>1</td><td>2</td></tr><tr><td>3</td></tr></table></div>";
    assert_eq!(
        run(PAGE, "document.getElementById('d').innerText"),
        "a b c\n\npara\n\ntwo\n\nend\nline\n keep  \n this\n1\t2\n3"
    );
    assert_eq!(
        run(
            PAGE,
            r#"const d = document.createElement('div');
            d.innerHTML = '  a  <p>b</p><script>c</script>';
            d.innerText"#
        ),
        "  a  bc"
    );
}

/// Chromium 153's answers, from a probe of each page.
#[test]
fn inner_text_of_tables_and_controls() {
    for (body, want) in [
        (
            "<table><thead><tr><td>a<td>b</thead><tbody><tr><td>c<td>d</tbody><tfoot><tr><td>e</table>",
            "a\tb\nc\td\ne",
        ),
        (
            "<table><tr><td>a</td><script></script></tr><tr><td>b</table>",
            "a\nb",
        ),
        (
            "<table><caption>c</caption><tr><td>a</table>after",
            "c\na\nafter",
        ),
        ("<details><summary>s</summary>c</details>x", "s\nx"),
        ("<details open><summary>s</summary>c</details>x", "s\nc\nx"),
        (
            "a<select><option>o1<option>o2</select>b<textarea>z</textarea>c<button>q</button>",
            "a\no1\no2\nbcq",
        ),
        ("<div style='DISPLAY: NONE'>x</div>y", "y"),
    ] {
        let page = format!("<!doctype html><div id=d>{body}</div>");
        assert_eq!(
            run(&page, "document.getElementById('d').innerText"),
            want,
            "{body}"
        );
    }
}

/// Chromium 153's answers to the number edge cases of reflection and factories.
#[test]
fn numbers_past_their_range_take_the_default() {
    assert_eq!(
        run(
            "<!doctype html>",
            r#"const d = document.createElement('div');
            d.setAttribute('tabindex', '2147483648');
            const td = document.createElement('td');
            td.colSpan = 4294967295;
            const a = [d.tabIndex, td.getAttribute('colspan')];
            td.rowSpan = 4294967295;
            const im = new Image(3, -1);
            const sel = document.createElement('select');
            sel.size = 4294967295;
            a.push(td.getAttribute('rowspan'), im.getAttribute('width'), im.getAttribute('height'),
                   Image.length, Option.length, sel.getAttribute('size'));
            a.join()"#
        ),
        "-1,1,1,3,0,0,0,0"
    );
}

/// Chromium 153: focus belongs to a navigable, which a made document hasn't got.
#[test]
fn focus_needs_a_browsing_context() {
    assert_eq!(
        run(
            "<!doctype html>",
            r#"const doc = document.implementation.createHTMLDocument('');
            const i = doc.createElement('input');
            doc.body.append(i);
            let fired = false;
            i.addEventListener('focus', () => { fired = true; });
            i.focus();
            [doc.activeElement === i, doc.activeElement.nodeName, fired].join()"#
        ),
        "false,BODY,false"
    );
}

/// Chromium 153: rows go in tree order, so a foot before the body breaks a line.
#[test]
fn inner_text_follows_tree_order_for_a_foot_first() {
    for (body, want) in [
        (
            "<table><tfoot><tr><td>f</tfoot><tbody><tr><td>b1<tr><td>b2</tbody></table>x",
            "f\nb1\nb2\nx",
        ),
        (
            "<table><tr><td>a</table><table><tfoot><tr><td>f</tfoot><tr><td>b</table>y",
            "a\nf\nb\ny",
        ),
    ] {
        let page = format!("<!doctype html><div id=d>{body}</div>");
        assert_eq!(
            run(&page, "document.getElementById('d').innerText"),
            want,
            "{body}"
        );
    }
}

/// Chromium 153's answer: the usual placeholder option keeps its value.
#[test]
fn a_disabled_selected_option_has_its_value() {
    assert_eq!(
        run(
            "<!doctype html><select id=q><option disabled selected value=p>Pick</option><option>x</select>",
            "const q = document.getElementById('q'); q.value + q.selectedIndex"
        ),
        "p0"
    );
}

#[test]
fn a_new_body_goes_right_after_the_last() {
    assert_eq!(
        run(
            "<!doctype html><table id=t><tbody></tbody><!--c--><tfoot></tfoot></table>",
            r#"const t = document.getElementById('t');
            t.createTBody();
            [...t.childNodes].map((n) => n.nodeName).join()"#
        ),
        "TBODY,TBODY,#comment,TFOOT"
    );
}

/// Each list is rebuilt per change of the tree, not per read: four times the
/// rows cost about four times as much, where a rebuild per read would cost
/// sixteen. A ratio in one run, under a time limit no build reaches
/// (P2-DOM-17).
#[test]
fn a_read_loop_over_a_big_table_is_linear() {
    let ratio = common::cost_ratio(500, 2000, |n| {
        let mut rows = String::new();
        for i in 0..n {
            rows.push_str(&format!("<tr><td>{i}</td></tr>"));
        }
        let mut rt = DomRuntime::new(common::cost_limits()).unwrap();
        rt.load(Document::parse(&format!("<!doctype html><table id=t>{rows}</table>")).unwrap());
        let began = std::time::Instant::now();
        let cells: String = rt
            .eval(
                r#"const t = document.getElementById('t');
                let n = 0;
                for (let i = 0; i < t.rows.length; i++) n += t.rows[i].cells.length;
                String(n)"#,
            )
            .unwrap();
        let took = began.elapsed();
        assert_eq!(cells, n.to_string());
        took
    });
    assert!(ratio < 8.0, "4x the rows took {ratio:.1}x as long");
}

#[test]
fn a_huge_length_is_refused_without_work() {
    assert_eq!(
        run(
            "<!doctype html><select id=s></select>",
            r#"const s = document.getElementById('s');
            s.length = 4294967295;
            s.options.length = 200000;
            String(s.length)"#
        ),
        "0"
    );
}

/// Replaced built-ins don't reach the glue.
#[test]
fn replacing_dom_members_changes_nothing() {
    assert_eq!(
        run(
            TABLE,
            r#"const t = document.getElementById('t');
            const s = document.createElement('select');
            const o = new Option('a');
            s.add(o);
            const list = s.options;
            for (const p of [Node.prototype, Element.prototype, HTMLCollection.prototype, Document.prototype]) {
              for (const k of Object.getOwnPropertyNames(p)) {
                if (k !== 'constructor') { try { delete p[k]; } catch (e) {} }
              }
            }
            const r = HTMLTableElement.prototype.insertRow.call(t, 0);
            HTMLTableRowElement.prototype.insertCell.call(r);
            HTMLSelectElement.prototype.add.call(s, new Option('b'), o);
            list[0] = new Option('c');
            String(HTMLTableRowElement.prototype.__lookupGetter__('cells').call(r).constructor.name)"#
        ),
        "HTMLCollection"
    );
}

// ------ What a page's own script meets with no user (P2-DOM-16) ------

/// `wpt/probes/cases.mjs`'s page around `body`: `log()` fills `L`.
fn probe_page(body: &str) -> String {
    format!(
        "<!doctype html><html><head><script>var L=[];function log(){{L.push([].map.call(arguments,String).join(' '))}}</script></head><body>{body}</body></html>"
    )
}

/// `html` loaded at `url` and run to the end, as the probe runner does; then
/// what it logged, one line per `log()`.
fn page_log(html: &str, url: &str) -> String {
    let mut rt = DomRuntime::new(limits()).unwrap();
    let mut doc = Document::parse(html).unwrap();
    doc.set_url(url);
    rt.load(doc);
    rt.run_page(&surimono_core::SettleOptions::default(), |_| None)
        .unwrap();
    rt.eval("L.join('\\n')").unwrap()
}

/// CSSOM View §6 (extensions to `Document`), `scrollingElement`: the root
/// element in no-quirks mode; in quirks mode the body element unless it is
/// potentially scrollable, else null. With no box nothing is potentially
/// scrollable. Chromium 153 on `wpt/probes` `scroll`: `start 0 0 0 0 HTML`.
#[test]
fn the_scrolling_element_is_the_root_or_the_quirks_body() {
    assert_eq!(
        run(
            "<!doctype html>",
            r#"const n = (x) => x === null ? 'null' : x.nodeName;
            const parse = (s, t) => new DOMParser().parseFromString(s, t || 'text/html');
            const empty = document.implementation.createHTMLDocument('');
            empty.removeChild(empty.documentElement);
            const moved = parse('x');
            const b = moved.body;
            moved.documentElement.appendChild(moved.createElement('div')).appendChild(b);
            const nobody = parse('x');
            nobody.documentElement.removeChild(nobody.body);
            [n(document.scrollingElement), 'scrollingElement' in Document.prototype,
             n(document.implementation.createHTMLDocument('').scrollingElement),
             n(parse('<p>x').scrollingElement), n(parse('<!doctype html><p>x').scrollingElement),
             n(parse('<a/>', 'application/xml').scrollingElement),
             n(document.implementation.createDocument(null, null).scrollingElement),
             n(empty.scrollingElement), n(nobody.scrollingElement), n(moved.scrollingElement),
             n(parse('<html style="overflow:scroll"><body style="overflow:scroll">').scrollingElement),
             n(parse('<frameset></frameset>').scrollingElement)].join()"#
        ),
        // The last: HTML's "the body element" is a frameset too, so the spec
        // gives it; Chromium 153 gives null (a conflict; the spec wins).
        "HTML,true,HTML,BODY,HTML,a,null,null,null,null,BODY,FRAMESET"
    );
    // A quirks page: no doctype.
    assert_eq!(
        page_log(
            "<html><head><script>var L=[];function log(){L.push([].map.call(arguments,String).join(' '))}</script></head><body><script>log(document.compatMode, document.scrollingElement.nodeName)</script></body></html>",
            "http://probe.test/q"
        ),
        "BackCompat BODY"
    );
}

/// Touch Events: the `GlobalEventHandlers` touch handlers are there only when
/// the user agent's "expose legacy touch event APIs" is true, and with it
/// false `createEvent('TouchEvent')` throws (WPT
/// dom/nodes/Document-createEvent-touchevent.window.js). surimono has no touch
/// screen. Chromium 153 on `wpt/probes` `mqhover`: `object 0 false`
/// (`typeof navigator`, `maxTouchPoints`, `'ontouchstart' in window`); on
/// `inputev` a constructed `TouchEvent` still dispatches.
#[test]
fn no_touch_screen_means_no_touch_handlers() {
    assert_eq!(
        run(
            "<!doctype html><svg><rect id=r /></svg>",
            r#"const out = [];
            const svg = document.getElementById('r'), div = document.createElement('div');
            for (const n of ['ontouchstart', 'ontouchend', 'ontouchmove', 'ontouchcancel'])
              out.push(n in window || n in document || n in div || n in svg || n in HTMLElement.prototype);
            for (const v of ['TouchEvent', 'touchevent', 'TOUCHEVENT']) {
              try { document.createEvent(v); out.push('created'); } catch (e) { out.push(e.name); }
            }
            const e = new TouchEvent('touchstart', { bubbles: true });
            let got = '';
            div.addEventListener('touchstart', (x) => { got = x.constructor.name + ':' + x.isTrusted; });
            div.dispatchEvent(e);
            out.push(typeof navigator, navigator.maxTouchPoints, got, 'onpointerdown' in window);
            out.join()"#
        ),
        "false,false,false,false,NotSupportedError,NotSupportedError,NotSupportedError,\
         object,0,TouchEvent:false,true"
    );
}

/// `wpt/probes` `focus`, the case's page as it is; the expected lines are
/// Chromium 153's in `wpt/probes/results.json`. HTML §4.16.3: `:focus` is the
/// focused area, `:focus-within` it and its ancestors (Selectors 4 §9.4), and
/// `:focus-visible` the UA's call, which Chromium answers true after a
/// script's `focus()` with no input.
#[test]
fn focus_pseudo_classes_follow_the_probe() {
    let body = r#"<input id=i><div id=d>d</div><button id=b>b</button><script>var i=document.getElementById('i'),d=document.getElementById('d'),ev=[];function ae(){var a=document.activeElement;return a?a.nodeName+(a.id?'#'+a.id:''):String(a)}function m(e,s){try{return e.matches(s)}catch(x){return x.name}}function t(n,f){try{f();return 'ok'}catch(x){return x.name}}for(const y of ['focus','blur','focusin','focusout'])document.addEventListener(y,e=>ev.push(y+':'+(e.target.id||e.target.nodeName)+':'+e.isTrusted),true);log('start',ae(),typeof document.hasFocus,document.querySelectorAll(':focus').length);log('input.focus()',t('',()=>i.focus()),ae(),m(i,':focus'),m(i,':focus-visible'),m(document.body,':focus-within'),document.querySelectorAll(':focus').length);log('div.focus()',t('',()=>d.focus()),ae());d.tabIndex=0;log('div tabindex',t('',()=>d.focus()),ae(),m(d,':focus'));log('blur()',t('',()=>d.blur()),ae(),m(d,':focus'));log('events',ev.join(' '))</script>"#;
    assert_eq!(
        page_log(&probe_page(body), "http://probe.test/focus"),
        "start BODY function 0\n\
         input.focus() ok INPUT#i true true true 1\n\
         div.focus() ok INPUT#i\n\
         div tabindex ok DIV#d true\n\
         blur() ok BODY false\n\
         events focus:i:true focusin:i:true blur:i:true focusout:i:true focus:d:true \
         focusin:d:true blur:d:true focusout:d:true"
    );
}

/// HTML §6.6.3 "focus fixup": a removed focused element (or one under a
/// removed node) leaves the document with nothing focused, so it doesn't
/// match `:focus` there or after it is put back; only the page's document has
/// focus (§6.6.4). `:focus-within` is the element and its ancestors, `html`
/// included (Selectors 4 §9.4); `:focus-visible` follows `:focus` for an SVG
/// element as for HTML ones.
#[test]
fn focus_pseudo_classes_follow_the_focused_area() {
    assert_eq!(
        run(
            "<!doctype html><div id=w><input id=i></div><svg id=s><rect id=r tabindex=0 /></svg>",
            r#"const $ = (x) => document.getElementById(x);
            const q = (s) => [...document.querySelectorAll(s)].map((e) => e.id || e.nodeName).join('+') || '-';
            const st = () => [q(':focus'), q(':focus-visible'), q(':focus-within')].join(' ');
            const out = [];
            const i = $('i'), w = $('w');
            i.focus();
            out.push(st(), i.closest(':focus-within').id, w.matches(':focus-within:not(:focus)'));
            $('r').focus();
            out.push(st());
            i.focus(); i.remove();
            out.push(document.activeElement.nodeName, st(), i.matches(':focus'));
            w.appendChild(i);
            out.push(st(), i.matches(':focus'));
            i.focus(); w.remove();
            out.push(document.activeElement.nodeName, st(), w.matches(':focus-within'));
            document.body.appendChild(w);
            i.focus(); document.body.innerHTML = '';
            out.push(document.activeElement.nodeName, st());
            const other = new DOMParser().parseFromString('<input id=z>', 'text/html');
            const z = other.getElementById('z');
            z.focus();
            out.push(z.matches(':focus'), other.querySelectorAll(':focus,:focus-within').length);
            out.join('|')"#
        ),
        "i i HTML+BODY+w+i|i|true|r r HTML+BODY+s+r|BODY|- - -|false|- - -|false|BODY|- - -|false|\
         BODY|- - -|false|0"
    );
}

/// HTML §6.6.5: an `autofocus` element is a candidate; "flush autofocus
/// candidates" (at the rendering update after `load`, where Chromium 153 does
/// it) focuses the first focusable one, with `focus` and `focusin`. Chromium
/// 153 on `wpt/probes` `autofocus`: `a 1` 20 ms after `load`.
#[test]
fn autofocus_focuses_the_first_candidate_after_load() {
    let body = r#"<input id=a autofocus><script>addEventListener('load',()=>setTimeout(()=>{var e=document.activeElement;log(e?e.id||e.nodeName:String(e),document.querySelectorAll(':focus').length)},20))</script>"#;
    assert_eq!(
        page_log(&probe_page(body), "http://probe.test/autofocus"),
        "a 1"
    );
    // Not focusable candidates are passed over (step 5.8); events as for
    // `focus()`, trusted; nothing before `load`; a `setTimeout` a `load`
    // listener sets comes after the flush, and a frame callback after it too.
    let body = r#"<div id=n autofocus>n</div><input id=x disabled autofocus><input id=a autofocus><input id=b autofocus><script>for(const y of ['focus','focusin'])document.addEventListener(y,e=>log(y,e.target.id,e.isTrusted),true);document.addEventListener('DOMContentLoaded',()=>log('dcl',document.activeElement.nodeName));addEventListener('load',()=>{log('load',document.activeElement.nodeName);setTimeout(()=>log('later',document.activeElement.id));requestAnimationFrame(()=>log('frame',document.activeElement.id))})</script>"#;
    assert_eq!(
        page_log(&probe_page(body), "http://probe.test/af"),
        "dcl BODY\nload BODY\nfocus a true\nfocusin a true\nlater a\nframe a"
    );
}

/// HTML §6.6.5 step 4: with candidates but something already focused, or a
/// target element, the flag is set and nothing is autofocused; a fragment that
/// names no element is no target. A candidate removed before the flush is
/// none, and one inserted by a script is one.
#[test]
fn autofocus_yields_to_focus_and_to_a_target() {
    let after_load = "<script>addEventListener('load',()=>setTimeout(()=>log(document.activeElement.id||document.activeElement.nodeName),20))</script>";
    for (body, url, want) in [
        (
            "<input id=a autofocus><input id=b><script>document.getElementById('b').focus()</script>",
            "http://probe.test/p",
            "b",
        ),
        ("<p id=t>t</p><input id=a autofocus>", "http://probe.test/p#t", "BODY"),
        ("<p id=t>t</p><input id=a autofocus>", "http://probe.test/p#nothere", "a"),
        (
            "<input id=r autofocus><script>document.getElementById('r').remove()</script><input id=z>",
            "http://probe.test/p",
            "BODY",
        ),
        (
            "<script>var c=document.createElement('input');c.id='c';c.autofocus=true;document.body.appendChild(c)</script>",
            "http://probe.test/p",
            "c",
        ),
    ] {
        assert_eq!(
            page_log(&probe_page(&format!("{body}{after_load}")), url),
            want,
            "{body} at {url}"
        );
    }
}
