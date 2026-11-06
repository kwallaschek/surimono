//! `document.styleSheets`, `sheet`, `cssRules`, `insertRule` and `deleteRule`
//! against CSSOM §6, and that what a script inserts reaches the printed page, as it
//! reaches the print of a browser.

mod common;

use common::limits;
use surimono_core::SandboxError;
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

const PAGE: &str = "<!doctype html><html><head><style id=s>a { color: red } b{top:0}</style></head><body></body></html>";

fn runtime(page: &str) -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(page).unwrap());
    rt
}

fn run(script: &str) -> String {
    runtime(PAGE)
        .eval(&format!(
            "const s = document.getElementById('s'), sheet = s.sheet;
             const thrown = (f) => {{ try {{ f(); return 'none'; }} catch (e) {{ return e.name; }} }};
             {script}"
        ))
        .unwrap()
}

#[test]
fn a_sheet_is_read_from_its_text() {
    assert_eq!(
        run("const r = sheet.cssRules;
             [sheet instanceof CSSStyleSheet, sheet instanceof StyleSheet, sheet.type, sheet.href, sheet.ownerNode === s,
              sheet.ownerRule, sheet.parentStyleSheet, r instanceof CSSRuleList, r.length, r === sheet.cssRules,
              sheet.rules === r, r[0] instanceof CSSStyleRule, r[0] instanceof CSSRule, r[0].type, r[0].selectorText,
              r[0].cssText, r.item(1).cssText, r[0] === r[0], r[0].parentStyleSheet === sheet, r[0].parentRule,
              r[2], r.item(2), [...r].length, Object.keys(r).join()].join('|')"),
        "true|true|text/css||true|||true|2|true|true|true|true|1|a|a { color: red; }|b { top:0; }|true|true|||\
         |2|0,1"
    );
    assert_eq!(
        run("const list = document.styleSheets;
             const a = [list instanceof StyleSheetList, list === document.styleSheets, list.length, list[0] === sheet, list.item(0) === sheet, list.item(1)];
             const n = document.createElement('style');
             const out = n.sheet;
             document.head.append(n);
             a.push(out, n.sheet instanceof CSSStyleSheet, list.length, list[1] === n.sheet, n.sheet.cssRules.length);
             n.remove();
             a.push(list.length, n.sheet);
             a.join()"),
        "true,true,1,true,true,,,true,2,true,0,1,"
    );
}

#[test]
fn rules_go_in_and_out() {
    assert_eq!(
        run("const r = sheet.cssRules;
             const a = [sheet.insertRule('i { top: 1px; }'), sheet.insertRule('u{left:0}', 3), sheet.insertRule('@media print { p { x: y } }', 2)];
             a.push([...r].map((x) => x.type + ':' + x.cssText).join('/'));
             const gone = r[0];
             sheet.deleteRule(0);
             a.push(r.length, gone.parentStyleSheet, gone.cssText);
             sheet.removeRule();
             a.push(sheet.addRule('q', 'top: 0'), r[r.length - 1].cssText, sheet.addRule('o', 'top: 1', 0), r[0].selectorText);
             a.join('|')"),
        "0|3|2|1:i { top: 1px; }/1:a { color: red; }/4:@media print { p { x: y } }/1:b { top:0; }/1:u { left:0; }\
         |4||i { top: 1px; }|-1|q { top: 0; }|-1|o"
    );
    assert_eq!(
        run("const dom = thrown;
             [dom(() => sheet.insertRule('x {}', 3)), dom(() => sheet.deleteRule(2)), dom(() => sheet.insertRule('')),
              dom(() => sheet.insertRule('nothing')), dom(() => sheet.insertRule('a {} b {}')), dom(() => sheet.insertRule('@nonsense y {}')),
              dom(() => sheet.insertRule('@import \"x\";', 1)), dom(() => sheet.insertRule()), sheet.cssRules.length].join()"),
        "IndexSizeError,IndexSizeError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,HierarchyRequestError,TypeError,2"
    );
}

#[test]
fn a_selector_can_be_changed() {
    assert_eq!(
        run("const r = sheet.cssRules[0];
             r.selectorText = 'x y';
             const one = r.cssText;
             r.selectorText = '';
             [one, r.selectorText].join('|')"),
        "x y { color: red; }|x y"
    );
}

#[test]
fn a_new_text_makes_a_new_sheet() {
    assert_eq!(
        run("sheet.insertRule('i {}');
             s.textContent = 'z {}';
             [s.sheet === sheet, s.sheet.cssRules.length, s.sheet.cssRules[0].selectorText].join()"),
        "false,1,z"
    );
}

fn printed(script: &str) -> String {
    let mut rt = runtime(PAGE);
    let () = rt.eval(&format!("const s = document.getElementById('s'); {script}; undefined")).unwrap();
    rt.take().unwrap().to_html()
}

/// This is what the task is for: a rule that stays in the CSSOM isn't on the page.
#[test]
fn inserted_rules_reach_the_output() {
    let html = printed("s.sheet.insertRule('.css-1 { color: blue; }', 2)");
    assert!(
        html.contains("<style id=\"s\">a { color: red; }\nb { top:0; }\n.css-1 { color: blue; }</style>"),
        "{html}"
    );
    let html = printed("s.sheet.deleteRule(0)");
    assert!(html.contains("<style id=\"s\">b { top:0; }</style>"), "{html}");
    let html = printed(
        "const e = document.createElement('style');
         document.head.append(e);
         e.sheet.insertRule('.emotion { top: 0 }')",
    );
    assert!(html.contains("<style>.emotion { top: 0; }</style>"), "{html}");
}

#[test]
fn a_script_still_sees_the_text() {
    assert_eq!(
        run("sheet.insertRule('i {}');
             [s.textContent, s.innerHTML, s.outerHTML].join('|')"),
        "a { color: red } b{top:0}|a { color: red } b{top:0}|<style id=\"s\">a { color: red } b{top:0}</style>"
    );
}

#[test]
fn a_sheet_a_script_left_alone_prints_as_written() {
    let html = printed("s.sheet.cssRules.length");
    assert!(html.contains("<style id=\"s\">a { color: red } b{top:0}</style>"), "{html}");
}

#[test]
fn a_changed_text_beats_old_edits_in_the_output() {
    let html = printed("s.sheet.insertRule('i {}'); s.textContent = 'z {}'");
    assert!(html.contains("<style id=\"s\">z {}</style>"), "{html}");
}

#[test]
fn rules_are_charged() {
    let l = DomLimits {
        max_text_bytes: 1024 * 1024,
        ..DomLimits::default()
    };
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse_with_limits(PAGE, l).unwrap());
    let e = rt
        .eval::<()>(
            "const r = document.getElementById('s').sheet;
             const big = 'x { y: ' + 'z'.repeat(4096) + ' }';
             for (;;) r.insertRule(big);",
        )
        .unwrap_err();
    assert!(matches!(e, DomError::TextLimit { .. }), "{e:?}");
}

#[test]
fn deleting_gives_the_charge_back() {
    let mut rt = runtime(PAGE);
    let () = rt
        .eval(
            "const r = document.getElementById('s').sheet;
             for (let i = 0; i < 300; i++) { r.insertRule('x { y: 0 }'); r.deleteRule(0); }
             undefined",
        )
        .unwrap();
    rt.take().unwrap().check_consistency().unwrap();
}

/// Bulk text in one rule, deep brackets and a long selector: all linear, and a loop
/// of them ends with the limit.
#[test]
fn hostile_rules_cost_little_and_end_with_the_limit() {
    let l = common::scaled_limits(
        surimono_core::Limits::default().with_time(std::time::Duration::from_millis(400)),
    );
    for script in [
        "const t = 'a {' + '('.repeat(1 << 20); const r = s.sheet; for (;;) { try { r.insertRule(t, 0); } catch (e) {} r.deleteRule(0); }",
        "s.textContent = 'a{}'.repeat(1 << 16); for (;;) { s.textContent += ' '; s.sheet.cssRules.length; }",
        "const r = s.sheet; for (;;) r.insertRule('x {}', 0);",
    ] {
        let mut rt = DomRuntime::new(l).unwrap();
        rt.load(Document::parse(PAGE).unwrap());
        let e = rt
            .eval::<()>(&format!("const s = document.getElementById('s'); {script}"))
            .unwrap_err();
        assert!(
            matches!(
                e,
                DomError::Sandbox(SandboxError::TimeLimit | SandboxError::MemoryLimit)
            ),
            "{script}: {e:?}"
        );
    }
}

#[test]
fn replacing_built_ins_changes_nothing() {
    assert_eq!(
        run("const r = sheet.cssRules;
             String.prototype.indexOf = String.prototype.slice = () => { throw 'string'; };
             delete Node.prototype.textContent;
             delete HTMLCollection.prototype.length;
             Object.defineProperty(Object.prototype, 'n', { set() { throw 'object'; } });
             sheet.insertRule('i {}');
             delete Object.prototype.n;
             String(r.length)"),
        "3"
    );
}

/// `Object.keys` of the list is as for `NodeList`; the target can't be reached.
#[test]
fn a_rule_list_is_read_only() {
    assert_eq!(
        run("'use strict';
             const r = sheet.cssRules;
             [Reflect.set(r, 0, 1), Reflect.defineProperty(r, 0, { value: 1 }), Reflect.deleteProperty(r, 0),
              Object.getOwnPropertyDescriptor(r, 0).writable, 0 in r, 5 in r].join()"),
        "false,false,false,false,true,false"
    );
}
