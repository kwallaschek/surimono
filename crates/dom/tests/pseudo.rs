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
             b.remove();
             log.push(typeof w.setTimeout(() => log.push('attached'), 0));
             gone.setTimeout(() => log.push('detached'), 0);
             w.setTimeout((x, y) => log.push(x + y), 1, 'a', 'b');
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

/// HTML §4.8.5: what does and doesn't start another load.
#[test]
fn reload_triggers() {
    let mut rt = runtime(
        "<!doctype html><body><iframe id=d srcdoc='<i>doc</i>'></iframe><iframe id=s src='one.html'></iframe>
         <script>
         globalThis.log = [];
         const d = document.getElementById('d'), s = document.getElementById('s');
         let step = 0;
         const seen = () => log.push(d.contentDocument.body.textContent + '/' + s.contentDocument.body.textContent);
         let pending = 2;
         const next = () => {
           if (--pending > 0) return;
           seen();
           step++;
           if (step === 1) {
             // Neither of these loads anything.
             d.src = 'two.html';
             s.setAttributeNS('urn:x', 'src', 'x.html');
             setTimeout(() => { log.push('quiet'); s.src = 'two.html'; pending = 1; }, 0);
           } else if (step === 2) {
             d.removeAttribute('srcdoc');
             pending = 1;
           }
         };
         d.onload = next; s.onload = next;
         </script>",
        Some("http://e.test/"),
    );
    rt.run_page(&SettleOptions::default(), |url| {
        Some(format!("<body>{}</body>", url.rsplit('/').next().unwrap()))
    })
    .unwrap();
    let r: String = rt.eval("log.join('|')").unwrap();
    assert_eq!(r, "doc/one.html|quiet|doc/two.html|two.html/two.html");
}

/// More frames than a render may load, each reloading itself: the loads stop
/// at `MAX_FRAMES` (64), which a single frame's eight rounds can't reach.
#[test]
fn reloads_count_against_the_frames_of_a_render() {
    let frames = "<iframe src='a.html'></iframe>".repeat(40);
    let mut rt = runtime(
        &format!(
            "<!doctype html><body>{frames}
             <script>
             globalThis.loads = 0;
             for (const f of document.querySelectorAll('iframe')) {{
               f.onload = () => {{ loads++; f.src = 'b' + loads + '.html'; }};
             }}
             </script>"
        ),
        Some("http://e.test/"),
    );
    rt.run_page(&SettleOptions::default(), |_| Some(String::new()))
        .unwrap();
    let n: u32 = rt.eval("loads").unwrap();
    assert_eq!(n, 64);
}

/// HTML §8.6: a frame whose parent frame has gone isn't fully active.
#[test]
fn a_nested_frames_timers_end_with_its_parent_frame() {
    let mut rt = runtime("<!doctype html><body><iframe id=f></iframe>", None);
    let () = rt
        .eval(
            "globalThis.log = [];
             const f = document.getElementById('f');
             const d = f.contentDocument;
             const inner = d.body.appendChild(d.createElement('iframe'));
             const w = inner.contentWindow;
             w.setTimeout(() => log.push('nested'), 0);
             f.remove();
             w.setTimeout(() => log.push('after'), 0);
             undefined",
        )
        .unwrap();
    rt.run_until_settled(&SettleOptions::default()).unwrap();
    let r: String = rt.eval("log.join()").unwrap();
    assert_eq!(r, "");
}

#[test]
fn target_ignores_the_case_and_escapes_of_its_name() {
    let page = "<!doctype html><p id=t data-target=1>";
    assert_eq!(ids(page, ":TARGET"), "t");
    assert_eq!(ids(page, ":t\\61rget"), "t");
    assert_eq!(ids(page, "[data-target]"), "t");
}

#[test]
fn unknown_input_types_are_text() {
    let page =
        "<!doctype html><input id=a type=foo><input id=b type=datetime><input id=c type=file>
        <input id=d type=range>";
    assert_eq!(ids(page, "input:read-write"), "a,b");
}

/// HTML §6.8.1: only these values make a host; the others inherit.
#[test]
fn contenteditable_values() {
    let page = "<!doctype html><p id=a contenteditable=inherit></p><p id=b contenteditable=bogus></p>
        <div contenteditable=TRUE id=c><p id=d contenteditable=inherit></p><p id=e contenteditable=false><i id=f contenteditable=bogus></i></p></div>
        <p id=g contenteditable=plaintext-only></p>";
    assert_eq!(ids(page, "body :read-write"), "c,d,g");
}

#[test]
fn one_select_has_one_selected_option() {
    let page =
        "<!doctype html><select><option id=a selected><option id=b selected><option id=c></select>
        <select multiple><option id=d selected><option id=e selected></select>
        <select size=3><option id=f selected><optgroup><option id=g selected></optgroup></select>
        <option id=h selected>";
    assert_eq!(ids(page, ":checked"), "b,d,e,g,h");
}

/// HTML §3.2.6.2: SVG elements too, and `xml:lang` first.
#[test]
fn lang_on_svg_and_xml_lang_priority() {
    let page = r#"<!doctype html><svg id=a lang=ja></svg><svg id=b lang=ja xml:lang=de></svg><math id=c lang=ja></math>"#;
    assert_eq!(ids(page, ":lang(ja)"), "a");
    assert_eq!(ids(page, ":lang(de)"), "b");
}
