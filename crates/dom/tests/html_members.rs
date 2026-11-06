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
        run(SELECT, r#"const s = document.getElementById('s');
            s.selectedIndex = -1;
            const a = [s.selectedIndex, JSON.stringify(s.value)];
            s.value = 'nothing';
            a.push(s.selectedIndex);
            s.value = '4';
            a.push(s.selectedIndex, s.value);
            a.join()"#),
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

/// Each list is rebuilt per change of the tree, not per read.
#[test]
fn a_read_loop_over_a_big_table_is_linear() {
    let mut rows = String::new();
    for i in 0..2000 {
        rows.push_str(&format!("<tr><td>{i}</td></tr>"));
    }
    let page = format!("<!doctype html><table id=t>{rows}</table>");
    let began = std::time::Instant::now();
    assert_eq!(
        run(
            &page,
            r#"const t = document.getElementById('t');
            let n = 0;
            for (let i = 0; i < t.rows.length; i++) n += t.rows[i].cells.length;
            String(n)"#
        ),
        "2000"
    );
    assert!(began.elapsed() < common::scaled(std::time::Duration::from_secs(30)));
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
            for (const p of [Node.prototype, Element.prototype, HTMLCollection.prototype, Document.prototype]) {
              for (const k of Object.getOwnPropertyNames(p)) {
                if (k !== 'constructor') { try { delete p[k]; } catch (e) {} }
              }
            }
            const r = HTMLTableElement.prototype.insertRow.call(t, 0);
            HTMLTableRowElement.prototype.insertCell.call(r);
            String(HTMLTableRowElement.prototype.__lookupGetter__('cells').call(r).constructor.name)"#
        ),
        "HTMLCollection"
    );
}
