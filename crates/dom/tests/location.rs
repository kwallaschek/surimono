//! `Location` and `History` against the HTML Standard (§7.2.4, §7.2.5): the
//! document's URL as an object, and session history that never leaves the
//! document.

mod common;

use common::limits;
use surimono_core::SettleOptions;
use surimono_dom::{Document, DomError, DomRuntime};

const URL: &str = "http://example.test:8080/a/b.html?q=1#top";

fn runtime(url: Option<&str>) -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    let mut doc = Document::parse("<!doctype html><html><head></head><body><a id=l href='#s'>x</a><iframe id=f></iframe></body></html>").unwrap();
    if let Some(u) = url {
        doc.set_url(u);
    }
    rt.load(doc);
    rt
}

fn run(script: &str) -> String {
    runtime(Some(URL)).eval(script).unwrap()
}

/// `script`, then the rest of its tasks.
fn settled(script: &str) -> String {
    let mut rt = runtime(Some(URL));
    let () = rt
        .eval(&format!("globalThis.log = []; {script}; undefined"))
        .unwrap();
    rt.run_until_settled(&SettleOptions::default()).unwrap();
    rt.eval("log.join('|')").unwrap()
}

#[test]
fn location_reads_the_document_url() {
    assert_eq!(
        run(
            "[location.href, location.origin, location.protocol, location.host, location.hostname,
              location.port, location.pathname, location.search, location.hash,
              String(location), document.location === location, window.location === location,
              Object.prototype.toString.call(location)].join(' ')"
        ),
        "http://example.test:8080/a/b.html?q=1#top http://example.test:8080 http: example.test:8080 \
         example.test 8080 /a/b.html ?q=1 #top http://example.test:8080/a/b.html?q=1#top true true \
         [object Location]"
    );
}

#[test]
fn a_page_with_no_url_is_about_blank() {
    let mut rt = runtime(None);
    let r: String = rt
        .eval("[location.href, location.origin, location.protocol, location.host, location.pathname, location.hash].join(' ')")
        .unwrap();
    assert_eq!(r, "about:blank null about:  blank ");
}

#[test]
fn only_the_pages_document_has_a_location() {
    assert_eq!(
        run("[new DOMParser().parseFromString('', 'text/html').location,
              document.implementation.createHTMLDocument('').location].join()"),
        ","
    );
}

/// HTML §7.2.5 "shared history push/replace state steps".
#[test]
fn push_state_moves_the_document_url_and_nothing_else() {
    assert_eq!(
        run("const n = history.length;
             const d0 = new DOMParser().parseFromString('', 'text/html');
             history.pushState({ a: [1] }, '', '/fake/x?y#z');
             const d = new DOMParser().parseFromString('', 'text/html');
             const a = document.createElement('a'); a.setAttribute('href', 'rel');
             [history.length - n, document.URL, document.documentURI, document.baseURI, location.pathname,
              location.search, location.hash, d.URL, d0.URL, JSON.stringify(history.state), a.href,
              document.contentType].join(' ')"),
        "1 http://example.test:8080/fake/x?y#z http://example.test:8080/fake/x?y#z \
         http://example.test:8080/fake/x?y#z /fake/x ?y #z http://example.test:8080/fake/x?y#z \
         http://example.test:8080/a/b.html?q=1#top {\"a\":[1]} http://example.test:8080/fake/rel text/html"
    );
}

#[test]
fn replace_state_keeps_the_length() {
    assert_eq!(
        run("const n = history.length;
             history.replaceState(7, 'ignored', 'c.html');
             [history.length - n, document.URL, history.state].join(' ')"),
        "0 http://example.test:8080/a/c.html 7"
    );
}

#[test]
fn null_or_empty_url_keeps_the_document_url() {
    assert_eq!(
        run("history.pushState(1, ''); history.pushState(2, '', null); history.pushState(3, '', '');
             [history.length, document.URL].join(' ')"),
        format!("4 {URL}")
    );
}

/// "can have its URL rewritten": the same origin, or a fragment for what
/// isn't HTTP.
#[test]
fn a_url_the_document_cant_rewrite_to_is_a_security_error() {
    assert_eq!(
        run(
            "const name = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
             [name(() => history.pushState(1, '', 'http://other.test/')),
              name(() => history.replaceState(1, '', 'https://example.test:8080/')),
              name(() => history.pushState(1, '', 'http://u@example.test:8080/')),
              name(() => history.pushState(1, '', 'http://[')),
              name(() => history.pushState(1, '', 'x'.repeat(3 * 1024 * 1024))),
              name(() => history.pushState(1)),
              document.URL === location.href].join()"
        ),
        "SecurityError,SecurityError,SecurityError,SecurityError,SecurityError,TypeError,true"
    );
    let r: String = runtime(None)
        .eval(
            "const name = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
             [name(() => history.pushState(1, '', 'about:blank#x')), document.URL,
              name(() => history.pushState(1, '', 'about:other')), document.URL].join()",
        )
        .unwrap();
    assert_eq!(r, "none,about:blank#x,SecurityError,about:blank#x");
}

/// HTML §2.7: serialized when pushed, and refused for what can't be.
#[test]
fn state_is_a_clone() {
    assert_eq!(
        run("const o = { list: [1, { x: 2 }], when: new Date(5), re: /a+/gi,
                         m: new Map([[1, 'a']]), s: new Set(['b']), u: undefined };
             o.self = o;
             history.pushState(o, '');
             o.list[0] = 9;
             const st = history.state;
             [st === history.state, st !== o, st.list[0], st.list[1].x, st.when.getTime(), String(st.re),
              st.m.get(1), st.s.has('b'), 'u' in st, st.self === st].join()"),
        "true,true,1,2,5,/a+/gi,a,true,true,true"
    );
    assert_eq!(
        run(
            "const name = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
             [name(() => history.pushState(() => 1, '')),
              name(() => history.pushState({ f() {} }, '')),
              name(() => history.pushState(Symbol('s'), '')),
              name(() => history.pushState(document.body, '')),
              name(() => history.pushState([location], ''))].join()"
        ),
        "DataCloneError,DataCloneError,DataCloneError,DataCloneError,DataCloneError"
    );
}

#[test]
fn a_script_cant_break_the_clone_through_builtins() {
    assert_eq!(
        run("Map.prototype.get = () => { throw new Error('no'); };
             Map.prototype.set = () => { throw new Error('no'); };
             Array.isArray = () => false;
             history.pushState({ a: [1] }, '');
             Array.isArray(history.state.a) === false && JSON.stringify(history.state)"),
        r#"{"a":[1]}"#
    );
}

#[test]
fn a_deep_state_ends_at_the_stack_limit() {
    let mut rt = runtime(Some(URL));
    let r: Result<String, DomError> = rt.eval(
        "let o = {}; for (let i = 0; i < 2000000; i++) o = { o };
         history.pushState(o, ''); 'done'",
    );
    assert!(r.is_err(), "{r:?}");
}

#[test]
fn scroll_restoration_takes_its_enum() {
    assert_eq!(
        run(
            "const a = history.scrollRestoration; history.scrollRestoration = 'manual';
             const b = history.scrollRestoration; history.scrollRestoration = 'nope';
             [a, b, history.scrollRestoration].join()"
        ),
        "auto,manual,manual"
    );
}

#[test]
fn traversal_stays_in_the_entries_and_fires_popstate() {
    assert_eq!(
        settled(
            "addEventListener('popstate', (e) => log.push(['pop', e.state, location.pathname, e.isTrusted, history.state].join()));
             history.pushState('one', '', '/one');
             history.pushState('two', '', '/two');
             history.back();
             setTimeout(() => history.go(-1), 1);
             setTimeout(() => history.forward(), 2);
             setTimeout(() => history.go(5), 3);
             setTimeout(() => history.go(0), 4);
             setTimeout(() => log.push(document.URL, history.length), 5)"
        ),
        "pop,one,/one,true,one|pop,,/a/b.html,true,|pop,one,/one,true,one|http://example.test:8080/one|3"
    );
}

#[test]
fn a_push_after_going_back_drops_the_entries_ahead() {
    assert_eq!(
        settled(
            "history.pushState(1, '', '/1'); history.pushState(2, '', '/2'); history.back();
             setTimeout(() => { history.pushState(3, '', '/3'); log.push(history.length); history.forward(); }, 1);
             setTimeout(() => log.push(location.pathname), 2)"
        ),
        "3|/3"
    );
}

#[test]
fn a_fragment_changes_the_url_in_the_document() {
    assert_eq!(
        settled(
            "addEventListener('hashchange', (e) => log.push(e.oldURL, e.newURL, location.hash, e.bubbles));
             location.hash = '#next';
             log.push(document.URL)"
        ),
        "http://example.test:8080/a/b.html?q=1#next|\
         http://example.test:8080/a/b.html?q=1#top|http://example.test:8080/a/b.html?q=1#next|#next|false"
    );
    assert_eq!(
        settled(
            "addEventListener('hashchange', () => log.push('hash'));
             const n = history.length;
             location.hash = 'top';
             log.push(history.length - n);
             location.href = '#s'; log.push(location.hash, history.length - n);
             location.replace('#t'); log.push(location.hash, history.length - n);
             location.assign('b.html?q=1#u'); log.push(location.hash, history.length - n)"
        ),
        "0|#s|1|#t|1|#u|2|hash|hash|hash"
    );
}

#[test]
fn a_link_to_a_fragment_is_the_same_navigation() {
    assert_eq!(
        settled(
            "addEventListener('hashchange', (e) => log.push(e.newURL));
             document.getElementById('l').click()"
        ),
        "http://example.test:8080/a/b.html?q=1#s"
    );
}

/// Nothing leaves the document, from any of the ways to ask.
#[test]
fn navigation_stops_in_the_page() {
    assert_eq!(
        settled(
            "addEventListener('hashchange', () => log.push('hash'));
             location.href = 'http://other.test/';
             location.assign('/elsewhere');
             location.replace('http://other.test/');
             location.reload();
             location.search = '?x';
             location.pathname = '/y';
             location.protocol = 'https:';
             location.host = 'other.test';
             location.hostname = 'other.test';
             location.port = '1';
             location = '/z';
             document.location = '/z';
             log.push(document.URL, history.length)"
        ),
        format!("{URL}|1")
    );
    assert_eq!(
        run(
            "const name = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
             name(() => location.assign('http://['))"
        ),
        "SyntaxError"
    );
}

#[test]
fn a_javascript_url_runs_in_a_task() {
    assert_eq!(
        settled("location.href = 'javascript:log.push(%22ran%22)'; log.push('first')"),
        "first|ran"
    );
}

#[test]
fn a_frames_window_has_its_own_location_history_and_parser() {
    assert_eq!(
        run("const w = document.getElementById('f').contentWindow;
             w.history.pushState(1, '', '#f');
             const fromFrame = new w.DOMParser();
             const fromPage = new DOMParser();
             const m = w.DOMParser.prototype.parseFromString;
             [w.location.href, w.location !== location, location.href === document.URL, w.history !== history,
              fromFrame.parseFromString('', 'text/html').URL,
              m.call(fromPage, '', 'text/html').URL === document.URL,
              DOMParser.prototype.parseFromString.call(fromFrame, '', 'text/html').URL,
              fromFrame instanceof DOMParser, fromFrame instanceof w.DOMParser,
              w.DOMParser === DOMParser].join()"),
        "about:blank#f,true,true,true,about:blank#f,true,about:blank#f,true,true,false"
    );
}

#[test]
fn a_url_goes_with_the_render() {
    let mut rt = runtime(Some(URL));
    let () = rt
        .eval("history.pushState(1, '', '/moved'); undefined")
        .unwrap();
    rt.load(Document::parse("<!doctype html><p>").unwrap());
    let r: String = rt.eval("location.href + ' ' + history.length").unwrap();
    assert_eq!(r, "about:blank 1");
}

/// HTML §7.2.4: the setter returns when the fragment is the one it has, a
/// missing one counting as empty.
#[test]
fn setting_the_same_hash_does_nothing() {
    let mut rt = runtime(Some("http://example.test/a"));
    let () = rt
        .eval(
            "globalThis.log = [];
             addEventListener('hashchange', () => log.push('hash'));
             addEventListener('popstate', () => log.push('pop'));
             location.hash = ''; location.hash = '#';
             log.push(location.href, history.length); undefined",
        )
        .unwrap();
    rt.run_until_settled(&SettleOptions::default()).unwrap();
    let r: String = rt.eval("log.join('|')").unwrap();
    assert_eq!(r, "http://example.test/a|1");
}

/// HTML §7.4.2: to the current URL, navigating replaces. "Navigate to a
/// fragment": the state is never carried over, so `popstate` and `history.state`
/// are null. Chrome keeps `history.state`, a conflict DESIGN.md records.
#[test]
fn navigating_to_the_current_url_adds_no_entry() {
    assert_eq!(
        settled(
            "addEventListener('hashchange', () => log.push('hash'));
             addEventListener('popstate', (e) => log.push('pop:' + e.state));
             history.replaceState(7, '');
             const n = history.length;
             location.href = location.href;
             log.push(history.length - n, history.state);
             history.replaceState(8, '');
             location.assign(location.href);
             log.push(history.state)"
        ),
        "pop:null|0||pop:null|"
    );
    let mut rt = runtime(Some("http://example.test/a#s"));
    let r: String = rt
        .eval(
            "const n = history.length, l = document.getElementById('l');
             l.click(); l.click();
             String(history.length - n)",
        )
        .unwrap();
    assert_eq!(r, "0");
}

/// The first entry is the URL from before the first navigation, on a page
/// that has never touched `history`.
#[test]
fn back_after_a_first_fragment_navigation_restores_the_url() {
    for navigate in [
        "location.hash = 'x'",
        "document.getElementById('l').click()",
    ] {
        let mut rt = runtime(Some("http://example.test/a"));
        let () = rt
            .eval(&format!(
                "globalThis.log = [];
                 addEventListener('popstate', () => log.push('pop ' + location.href));
                 addEventListener('hashchange', (e) => log.push(e.oldURL + ' to ' + e.newURL));
                 {navigate};
                 setTimeout(() => {{ log.length = 0; history.back(); }}, 1);
                 undefined"
            ))
            .unwrap();
        rt.run_until_settled(&SettleOptions::default()).unwrap();
        let r: String = rt.eval("log.join('|')").unwrap();
        let fragment = if navigate.contains("hash") { "x" } else { "s" };
        assert_eq!(
            r,
            format!(
                "pop http://example.test/a|http://example.test/a#{fragment} to http://example.test/a"
            )
        );
    }
}

/// HTML "update document for history step application".
#[test]
fn a_fragment_fires_popstate_at_once_and_hashchange_later() {
    assert_eq!(
        settled(
            "addEventListener('popstate', (e) => log.push('pop:' + e.state + ':' + location.hash));
             addEventListener('hashchange', () => log.push('hash'));
             location.hash = 'a';
             log.push('sync')"
        ),
        "pop:null:#a|sync|hash"
    );
}

/// HTML §7.2.4 "Location-object navigate".
#[test]
fn until_load_a_location_navigation_replaces() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    let mut doc = Document::parse(
        "<!doctype html><script>
         globalThis.log = [];
         location.hash = 'a'; log.push(history.length);
         location.assign('#b'); log.push(history.length);
         addEventListener('load', () => { location.hash = 'c'; log.push(history.length); });
         setTimeout(() => { location.hash = 'd'; log.push(history.length); }, 1);
         </script><a id=l href='#e'>x</a>",
    )
    .unwrap();
    doc.set_url("http://example.test/");
    rt.load(doc);
    rt.run_page(&SettleOptions::default(), |_| None).unwrap();
    let r: String = rt.eval("log.join() + ' ' + location.hash").unwrap();
    assert_eq!(r, "1,1,1,2 #d");
}

#[test]
fn a_frames_fragment_fires_nothing_at_the_page() {
    assert_eq!(
        settled(
            "addEventListener('hashchange', () => log.push('hash'));
             addEventListener('popstate', () => log.push('pop'));
             const w = document.getElementById('f').contentWindow;
             w.location.hash = 'x';
             log.push(w.location.hash, location.hash)"
        ),
        "#x|#top"
    );
}

#[test]
fn a_link_acts_on_its_own_document_and_target() {
    assert_eq!(
        settled(
            "const blank = document.createElement('a');
             blank.href = '#x'; blank.target = '_blank'; blank.click();
             const parsed = new DOMParser().parseFromString('<a href=\"#y\">p</a>', 'text/html').querySelector('a');
             parsed.click();
             const top = document.createElement('a');
             top.href = '#z'; top.target = '_TOP'; top.click();
             log.push(location.hash);
             const d = document.getElementById('f').contentDocument;
             const inner = d.body.appendChild(d.createElement('a'));
             inner.href = '#w'; inner.click();
             log.push(location.hash, d.location === null, d.URL)"
        ),
        "#z|#z|true|about:blank#w"
    );
}

#[test]
fn a_javascript_url_goes_by_its_parsed_scheme() {
    for url in [
        " javascript:log.push(1)",
        "java\nscript:log.push(1)",
        "JAVASCRIPT:log.push(1)",
    ] {
        assert_eq!(settled(&format!("location.href = {url:?}")), "1", "{url}");
    }
    assert_eq!(
        settled(
            "const a = document.createElement('a');
             a.href = ' java\\tscript:log.push(2)'; a.click()"
        ),
        "2"
    );
}

/// A refused URL changes neither the document nor the entries.
#[test]
fn a_refused_url_leaves_the_history_alone() {
    assert_eq!(
        settled(
            "history.pushState(1, '', '/one'); history.pushState(2, '', '/two'); history.back();
             setTimeout(() => {
               const n = history.length, s = 'x'.repeat(3 * 1024 * 1024);
               location.hash = s;
               try { history.pushState(3, '', '#' + s); } catch (e) { log.push(e.name); }
               try { location.href = '#' + s; } catch (e) { log.push(e.name); }
               log.push(history.length === n, location.pathname + location.hash, history.state);
               history.forward();
             }, 1);
             setTimeout(() => log.push(location.pathname), 2)"
        ),
        "SecurityError|SyntaxError|true|/one|1|/two"
    );
}

/// HTML "cannot navigate", and `download`.
#[test]
fn a_detached_area_and_a_download_link_go_nowhere() {
    assert_eq!(
        settled(
            "const area = document.createElement('area'); area.href = '#a'; area.click();
             const a = document.getElementById('l'); a.href = '#b'; a.setAttribute('download', ''); a.click();
             log.push(location.hash);
             const loose = document.createElement('a'); loose.href = '#c'; loose.click();
             log.push(location.hash)"
        ),
        "#top|#c"
    );
}
