//! Pseudo-classes against HTML §4.16.3 and Selectors 4, attribute selector
//! flags, and small members WPT domparsing and timers read (`rootElement`,
//! `XMLSerializer` on an `Attr`, frames reloading and losing their timers).

mod common;

use common::limits;
use surimono_core::SettleOptions;
use surimono_dom::{Document, DomRuntime};

fn runtime(page: &str, url: Option<&str>) -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    let mut doc = Document::parse(page).unwrap();
    if let Some(u) = url {
        doc.set_url(u);
    }
    rt.load(doc);
    rt
}

/// The ids matching `selector` on `page`, in order.
fn ids(page: &str, selector: &str) -> String {
    let mut rt = runtime(page, Some("http://e.test/p#t"));
    let script = format!("[...document.querySelectorAll({selector:?})].map((e) => e.id).join()");
    rt.eval(&script).unwrap()
}

const FORM: &str = "<!doctype html><body>
<input id=a><input id=b disabled><button id=c></button><div id=d disabled></div>
<fieldset id=f disabled><legend><input id=g></legend><legend><input id=h></legend><input id=i>
<fieldset id=j><input id=k></fieldset></fieldset>
<select id=s><optgroup id=o disabled><option id=p></optgroup><option id=q disabled></select>";

#[test]
fn disabled_follows_fieldsets_and_their_first_legend() {
    assert_eq!(ids(FORM, ":disabled"), "b,f,h,i,j,k,o,p,q");
    assert_eq!(ids(FORM, ":enabled"), "a,c,g,s");
    assert_eq!(ids(FORM, "input:not(:enabled)"), "b,h,i,k");
}

#[test]
fn checked_is_the_attribute_and_a_selects_first_option() {
    let page = "<!doctype html><input id=a type=checkbox checked><input id=b type=radio checked>
        <input id=c type=checkbox><input id=d checked>
        <select id=e><option id=f disabled><option id=g><option id=h></select>
        <select id=i><option id=j><option id=k selected></select>
        <select id=l multiple><option id=m></select>
        <select id=n size=2><option id=o></select>
        <select id=p><optgroup><option id=q></optgroup></select>";
    assert_eq!(ids(page, ":checked"), "a,b,g,k,q");
}

#[test]
fn required_and_optional_go_by_the_types_it_applies_to() {
    let page = "<!doctype html><input id=a required><input id=b><input id=c type=submit required>
        <input id=d type=hidden><select id=e required></select><textarea id=f></textarea><div id=g required></div>";
    assert_eq!(ids(page, ":required"), "a,e");
    assert_eq!(ids(page, ":optional"), "b,f");
}

#[test]
fn read_write_is_what_can_be_edited() {
    let page = "<!doctype html><body id=z><input id=a><input id=b readonly><input id=c disabled>
        <input id=d type=checkbox><textarea id=e></textarea><textarea id=f readonly></textarea>
        <div id=g contenteditable><span id=h></span><span id=i contenteditable=false></span></div>
        <svg id=j></svg>";
    assert_eq!(ids(page, ":read-write"), "a,e,g,h");
    assert_eq!(ids(page, "body :read-only"), "b,c,d,f,i");
}

/// Selectors 4 §7.2: extended filtering, ASCII case-insensitive.
#[test]
fn lang_is_inherited_and_filters_by_subtag() {
    let page = r#"<!doctype html><html lang="en-GB"><body id=b>
        <p id=a><span id=c lang="de-Latn-DE"></span><span id=d lang=""></span><span id=e lang="FR"></span></p>
        <svg id=f xml:lang="ja"></svg></body></html>"#;
    assert_eq!(ids(page, "body :lang(en)"), "a");
    assert_eq!(ids(page, "svg:lang(ja)"), "f");
    assert_eq!(ids(page, "p :lang(de)"), "c");
    assert_eq!(ids(page, "p :lang(de-DE)"), "c");
    assert_eq!(ids(page, "p :lang(\\*-Latn)"), "c");
    assert_eq!(ids(page, "p :lang(de-Cyrl)"), "");
    assert_eq!(ids(page, "p :lang(fr)"), "e");
    assert_eq!(ids(page, "body :lang(\"\")"), "d");
}

#[test]
fn defined_excludes_custom_names() {
    assert_eq!(
        ids(
            "<!doctype html><body><p id=a><my-el id=b></my-el><font-face id=c></font-face><x id=d></x>",
            "body :not(:defined)"
        ),
        "b"
    );
}

#[test]
fn target_is_the_urls_fragment() {
    let page = "<!doctype html><p id=x><p id=t><a id=y name=t></a><p id=caf%C3%A9>";
    assert_eq!(ids(page, ":target"), "t");
    let mut rt = runtime(page, Some("http://e.test/#y"));
    let r: String = rt.eval("document.querySelector(':target').id").unwrap();
    assert_eq!(r, "y");
    let mut rt = runtime(
        "<!doctype html><a id=y name=u></a><p id=caf\u{e9}>",
        Some("http://e.test/#caf%C3%A9"),
    );
    let r: String = rt.eval("document.querySelector(':target').id").unwrap();
    assert_eq!(r, "caf\u{e9}");
    let mut rt = runtime(
        "<!doctype html><a id=y name=u></a>",
        Some("http://e.test/#u"),
    );
    let r: String = rt.eval("document.querySelector(':target').id").unwrap();
    assert_eq!(r, "y");
    let mut rt = runtime(page, None);
    let r: u32 = rt
        .eval("document.querySelectorAll(':target').length")
        .unwrap();
    assert_eq!(r, 0);
}

/// Selectors 4 §6.2.1: the `i` and `s` flags, and the attributes HTML lists
/// as case-insensitive in an HTML document.
#[test]
fn attribute_case_flags() {
    let page = r#"<!doctype html><input id=a type="TEXT" name="User"><p id=b title="ABC"><svg id=c title="abc" type="TEXT"></svg>"#;
    assert_eq!(ids(page, "[name=user i]"), "a");
    assert_eq!(ids(page, "[name=user]"), "");
    assert_eq!(ids(page, "[title=abc i]"), "b,c");
    assert_eq!(ids(page, "[title^=A s]"), "b");
    assert_eq!(ids(page, "input[type=text]"), "a");
    assert_eq!(ids(page, "input[type=text s]"), "");
}

#[test]
fn root_element_is_an_svg_documents_svg() {
    let mut rt = runtime("<!doctype html><p>", None);
    let r: String = rt
        .eval(
            "const p = new DOMParser();
             const s = p.parseFromString('<svg xmlns=\"http://www.w3.org/2000/svg\"/>', 'image/svg+xml');
             const x = p.parseFromString('<svg/>', 'text/xml');
             [document.rootElement, s.rootElement === s.documentElement, x.rootElement,
              new Document().rootElement].join()",
        )
        .unwrap();
    assert_eq!(r, ",true,,");
}

#[test]
fn xml_serializer_on_attributes_and_taken_prefixes() {
    let mut rt = runtime("<!doctype html><p>", None);
    let r: String = rt
        .eval(
            "const s = new XMLSerializer();
             const root = new DOMParser().parseFromString('<root xmlns:p=\"uri1\"><child/></root>', 'text/xml').documentElement;
             root.firstChild.setAttributeNS('uri2', 'p:foobar', 'v');
             const c = new DOMParser().parseFromString('<a xmlns:q=\"u\"><b/></a><!-- -->', 'text/xml').documentElement;
             [s.serializeToString(document.createAttribute('x')), s.serializeToString(root)].join('|')",
        )
        .unwrap();
    assert_eq!(
        r,
        r#"|<root xmlns:p="uri1"><child xmlns:ns1="uri2" ns1:foobar="v"/></root>"#
    );
}

#[test]
fn a_detached_frames_timers_never_run() {
    let mut rt = runtime(
        "<!doctype html><body><iframe id=a></iframe><iframe id=b></iframe>",
        None,
    );
    let () = rt
        .eval(
            "globalThis.log = [];
             const a = document.getElementById('a'), b = document.getElementById('b');
             const w = a.contentWindow, gone = b.contentWindow;
             const late = a.contentWindow;
             b.remove();
             log.push(typeof w.setTimeout(() => log.push('attached'), 0));
             gone.setTimeout(() => log.push('detached'), 0);
             const t = w.setTimeout((x, y) => log.push(x + y), 1, 'a', 'b');
             w.setTimeout(() => {}, 1);
             undefined",
        )
        .unwrap();
    rt.run_until_settled(&SettleOptions::default()).unwrap();
    let r: String = rt.eval("log.join()").unwrap();
    assert_eq!(r, "number,attached,ab");
}

#[test]
fn a_frame_loads_again_when_its_source_changes() {
    let mut rt = runtime(
        "<!doctype html><body><iframe id=f src='one.html'></iframe>
         <script>
         globalThis.log = [];
         const f = document.getElementById('f');
         f.onload = () => {
           log.push(f.contentDocument.URL + ' ' + f.contentDocument.body.textContent);
           if (log.length === 1) f.src = 'two.html';
         };
         </script>",
        Some("http://e.test/"),
    );
    rt.run_page(&SettleOptions::default(), |url| {
        Some(format!("<body>{}</body>", url.rsplit('/').next().unwrap()))
    })
    .unwrap();
    let r: String = rt.eval("log.join('|')").unwrap();
    assert_eq!(
        r,
        "http://e.test/one.html one.html|http://e.test/two.html two.html"
    );
}

/// A frame that reloads itself forever ends at the render's rounds and frames.
#[test]
fn a_frame_cant_reload_without_end() {
    let mut rt = runtime(
        "<!doctype html><body><iframe id=f src='a.html'></iframe>
         <script>
         globalThis.n = 0;
         const f = document.getElementById('f');
         f.onload = () => { n++; f.src = 'b' + n + '.html'; };
         </script>",
        Some("http://e.test/"),
    );
    rt.run_page(&SettleOptions::default(), |_| Some(String::new()))
        .unwrap();
    let n: u32 = rt.eval("n").unwrap();
    assert!((1..=64).contains(&n), "{n}");
}
