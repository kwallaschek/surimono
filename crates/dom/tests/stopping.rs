//! What a caller can stop, and what is bounded when it doesn't: a cancel from
//! another thread, parses outside any script, serialization. Every outcome is
//! a typed error; the times are bounds on a stop's delay, stretched on slow
//! builds.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use common::{limits, scaled};
use surimono_core::{SandboxError, SettleOptions};
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

const PAGE: &str = "<!doctype html><body><p id=a>x</p>";

fn runtime() -> DomRuntime {
    let mut rt = DomRuntime::new(limits().with_time(scaled(Duration::from_secs(30)))).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    rt
}

fn cancel_after(flag: &Arc<AtomicBool>, wait: Duration) -> thread::JoinHandle<()> {
    let flag = flag.clone();
    thread::spawn(move || {
        thread::sleep(wait);
        flag.store(true, Ordering::Release);
    })
}

fn cancelled(e: &DomError) -> bool {
    matches!(e, DomError::Sandbox(SandboxError::Cancelled))
}

#[test]
fn a_cancel_stops_a_running_script() {
    for script in [
        "for (;;);",
        "for (;;) document.body.setAttribute('a', 'b');",
        "for (;;) { try { document.body.firstChild; } catch (e) {} }",
        "const s = '<p>x'.repeat(50000); for (;;) new DOMParser().parseFromString(s, 'text/html');",
    ] {
        let mut rt = runtime();
        let thread = cancel_after(&rt.cancel_flag(), Duration::from_millis(100));
        let started = Instant::now();
        let e = rt.eval::<()>(script).unwrap_err();
        thread.join().unwrap();
        assert!(
            cancelled(&e) || matches!(e, DomError::NodeLimit { .. }),
            "{script}: {e:?}"
        );
        assert!(
            started.elapsed() < scaled(Duration::from_secs(5)),
            "{script}"
        );
    }
}

#[test]
fn a_cancel_stops_a_page() {
    let mut rt = runtime();
    rt.load(
        Document::parse("<!doctype html><script>setInterval(() => { for (;;); }, 0)</script>")
            .unwrap(),
    );
    let thread = cancel_after(&rt.cancel_flag(), Duration::from_millis(100));
    let e = rt
        .run_page(&SettleOptions::default(), |_| None)
        .unwrap_err();
    thread.join().unwrap();
    assert!(cancelled(&e), "{e:?}");
}

#[test]
fn the_runtime_works_after_a_cancel() {
    let mut rt = runtime();
    let thread = cancel_after(&rt.cancel_flag(), Duration::from_millis(20));
    let e = rt.eval::<()>("for (;;);").unwrap_err();
    thread.join().unwrap();
    assert!(cancelled(&e), "{e:?}");
    assert!(!rt.cancel_flag().load(Ordering::Acquire));
    assert_eq!(rt.eval::<String>("document.body.tagName").unwrap(), "BODY");
}

#[test]
fn a_cancel_raised_between_calls_stops_the_next() {
    let mut rt = runtime();
    rt.cancel_flag().store(true, Ordering::Release);
    let e = rt.eval::<i32>("1").unwrap_err();
    assert!(cancelled(&e), "{e:?}");
    assert_eq!(rt.eval::<i32>("1").unwrap(), 1);
}

/// `n.limit` shares the flag: a limit stays that limit.
#[test]
fn a_limit_is_not_reported_as_a_cancel() {
    let mut rt = runtime();
    let e = rt
        .eval::<()>(
            "function f() { f(); } addEventListener('x', f); dispatchEvent(new Event('x'));",
        )
        .unwrap_err();
    assert!(!cancelled(&e), "{e:?}");
}

fn attributes(n: usize) -> String {
    let attrs: String = (0..n).map(|i| format!(" a{i}=\"\"")).collect();
    format!("<!doctype html><p{attrs}>")
}

#[test]
fn an_element_holds_the_attribute_limit() {
    let l = DomLimits {
        max_attributes: 64,
        ..DomLimits::default()
    };
    let doc = Document::parse_with_limits(&attributes(64), l).unwrap();
    doc.check_consistency().unwrap();
    assert_eq!(
        Document::parse_with_limits(&attributes(65), l).unwrap_err(),
        DomError::AttributeLimit { limit: 64 }
    );
    for script in [
        "for (let i = 0; ; i++) document.body.setAttribute('a' + i, '');",
        "for (let i = 0; ; i++) document.body.setAttributeNS('urn:x', 'a' + i, '');",
        "const a = ' a=1'; let s = '<i';
         for (let i = 0; i < 65; i++) s += ' b' + i; document.body.innerHTML = s + '>';",
        "let s = '<a'; for (let i = 0; i < 65; i++) s += ' b' + i + '=\"\"';
         new DOMParser().parseFromString(s + '/>', 'text/xml');",
        "let s = '<a'; for (let i = 0; i < 65; i++) s += ' b' + i + '=\"\"';
         new DOMParser().parseFromString(s + '>', 'text/html');",
    ] {
        let mut rt = DomRuntime::new(limits()).unwrap();
        rt.load(Document::parse_with_limits(PAGE, l).unwrap());
        assert_eq!(
            rt.eval::<()>(script).unwrap_err(),
            DomError::AttributeLimit { limit: 64 },
            "{script}"
        );
    }
}

#[test]
fn repeated_tags_cannot_add_up_past_the_limit() {
    let l = DomLimits {
        max_attributes: 64,
        ..DomLimits::default()
    };
    let tags: String = (0..50).map(|i| format!("<body a{i} b{i} c{i}>")).collect();
    assert_eq!(
        Document::parse_with_limits(&format!("<!doctype html>{tags}"), l).unwrap_err(),
        DomError::AttributeLimit { limit: 64 }
    );
}

#[test]
fn the_default_limit_passes_a_rich_element_and_refuses_a_bomb() {
    Document::parse(&attributes(DomLimits::DEFAULT_MAX_ATTRIBUTES)).unwrap();
    assert_eq!(
        Document::parse(&attributes(DomLimits::DEFAULT_MAX_ATTRIBUTES + 1)).unwrap_err(),
        DomError::AttributeLimit {
            limit: DomLimits::DEFAULT_MAX_ATTRIBUTES
        }
    );
}

/// The case from #143: 60,000 attributes in 420 KB took 13.8 s under a 500 ms
/// limit. A tag that never ends is the worst of it.
#[test]
fn a_stop_ends_a_quadratic_tag_near_its_deadline() {
    let mut open = attributes(60_000);
    open.pop();
    for page in [attributes(60_000), open] {
        let started = Instant::now();
        let deadline = started + Duration::from_millis(500);
        let e =
            Document::parse_stoppable(&page, DomLimits::default(), || Instant::now() >= deadline)
                .unwrap_err();
        assert_eq!(e, DomError::Stopped);
        eprintln!("stopped after {:?}", started.elapsed());
        // The bound is the wait for one small feed.
        assert!(
            started.elapsed() < scaled(Duration::from_millis(1500)),
            "{:?}",
            started.elapsed()
        );
    }
}

#[test]
fn a_stop_at_once_parses_nothing() {
    assert_eq!(
        Document::parse_stoppable(PAGE, DomLimits::default(), || true).unwrap_err(),
        DomError::Stopped
    );
}

#[test]
fn a_stop_that_changes_its_mind_still_stopped() {
    let asked = std::cell::Cell::new(0);
    let e = Document::parse_stoppable(&"<p>".repeat(20_000), DomLimits::default(), || {
        asked.set(asked.get() + 1);
        asked.get() == 2
    })
    .unwrap_err();
    assert_eq!(e, DomError::Stopped);
}

#[test]
fn a_parse_that_is_never_stopped_is_the_same_parse() {
    let a = Document::parse_stoppable(PAGE, DomLimits::default(), || false).unwrap();
    assert_eq!(a.to_html(), Document::parse(PAGE).unwrap().to_html());
}

/// A long tag inside a script's parse ends at the time limit too.
#[test]
fn a_quadratic_tag_in_inner_html_ends_at_the_time_limit() {
    let mut rt = DomRuntime::new(limits().with_time(Duration::from_millis(300))).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let started = Instant::now();
    let e = rt
        .eval::<()>(
            "let s = '<p'; for (let i = 0; i < 60000; i++) s += ' a' + i + '=\"\"';
             document.body.innerHTML = s;",
        )
        .unwrap_err();
    assert!(
        matches!(e, DomError::Sandbox(SandboxError::TimeLimit)),
        "{e:?}"
    );
    assert!(started.elapsed() < scaled(Duration::from_secs(3)));
}

#[test]
fn to_html_bounded_matches_to_html_under_its_cap() {
    let doc =
        Document::parse("<!doctype html><title>a&b</title><p class='x\"y'>t<!--c--></p>").unwrap();
    let html = doc.to_html();
    assert_eq!(doc.to_html_bounded(html.len(), || false).unwrap(), html);
    assert_eq!(
        doc.to_html_bounded(html.len() - 1, || false).unwrap_err(),
        DomError::OutputLimit {
            limit: html.len() - 1
        }
    );
}

#[test]
fn to_html_bounded_stops_inside_a_long_text() {
    // One text, so nothing but the slices can look.
    let page = format!("<!doctype html><p>{}", "&amp;".repeat(1 << 20));
    let doc = Document::parse(&page).unwrap();
    assert_eq!(
        doc.to_html_bounded(1 << 16, || false).unwrap_err(),
        DomError::OutputLimit { limit: 1 << 16 }
    );
    let asked = std::cell::Cell::new(0u32);
    let wide = format!("<!doctype html>{}", "<p>x</p>".repeat(5_000));
    let doc = Document::parse(&wide).unwrap();
    let e = doc
        .to_html_bounded(usize::MAX, || {
            asked.set(asked.get() + 1);
            true
        })
        .unwrap_err();
    assert_eq!(e, DomError::Stopped);
    assert_eq!(asked.get(), 1);
}

/// A frame parses inside a call, so a cancel reaches it: raised at 200 ms this
/// took 18.2 s when frames parsed between calls.
#[test]
fn a_cancel_stops_a_frame_parse() {
    let mut open = attributes(10_000);
    open.pop();
    let frames: String = (0..4)
        .map(|_| format!("<iframe srcdoc='{}'></iframe>", open.replace("\"\"", "1")))
        .collect();
    let mut rt = runtime();
    rt.load(Document::parse(&format!("<!doctype html>{frames}")).unwrap());
    let thread = cancel_after(&rt.cancel_flag(), Duration::from_millis(200));
    let started = Instant::now();
    let e = rt
        .run_page(&SettleOptions::default(), |_| None)
        .unwrap_err();
    thread.join().unwrap();
    assert!(cancelled(&e), "{e:?}");
    eprintln!("cancelled frames after {:?}", started.elapsed());
    assert!(started.elapsed() < scaled(Duration::from_millis(1500)));
}

fn big_xml() -> String {
    format!("<a>{}</a>", "<b c=\"d\">e</b>".repeat(400_000))
}

#[test]
fn a_stop_ends_an_xml_parse() {
    let page = big_xml();
    let started = Instant::now();
    let deadline = started + Duration::from_millis(50);
    let e =
        Document::parse_xml_stoppable(&page, DomLimits::default(), || Instant::now() >= deadline)
            .unwrap_err();
    assert_eq!(e, DomError::Stopped);
    eprintln!("stopped XML after {:?}", started.elapsed());
    assert!(started.elapsed() < scaled(Duration::from_millis(1000)));
}

#[test]
fn a_stop_ends_xml_in_a_long_prolog_and_in_a_long_text() {
    for page in [
        format!("{}<a/>", "<!-- c -->".repeat(50_000)),
        format!("<a>{}</a>", "<!-- c -->".repeat(50_000)),
    ] {
        assert_eq!(
            Document::parse_xml_stoppable(&page, DomLimits::default(), || true).unwrap_err(),
            DomError::Stopped
        );
    }
}

#[test]
fn an_unstopped_xml_parse_is_the_same_parse() {
    let page = "<a xmlns='urn:x'><b/></a>";
    let a = Document::parse_xml_stoppable(page, DomLimits::default(), || false).unwrap();
    assert_eq!(a.to_html(), Document::parse_xml(page).unwrap().to_html());
}

/// A long element or attribute name counts as it is written.
#[test]
fn to_html_bounded_checks_names() {
    let long = "a".repeat(1 << 20);
    for page in [format!("<p {long}=1>"), format!("<{long}>")] {
        let doc = Document::parse(&page).unwrap();
        assert_eq!(
            doc.to_html_bounded(1 << 16, || false).unwrap_err(),
            DomError::OutputLimit { limit: 1 << 16 }
        );
    }
}

/// Review of #148: one long token must not outlast the stop. A parse that didn't ask
/// inside these ran to the end: 80 MB of `&amp;` took 625 ms against a 50 ms stop.
#[test]
fn a_long_xml_token_asks_the_stop() {
    let cases = [
        format!("<r>{}</r>", "&amp;".repeat(1 << 21)),
        format!("<r>{}</r>", "x".repeat(32 << 20)),
        format!("<r{}/>", " ".repeat(32 << 20)),
        format!("<r{}a='b'/>", "\n".repeat(32 << 20)),
    ];
    for page in &cases {
        let asked = std::cell::Cell::new(0_u32);
        let got = Document::parse_xml_stoppable(page, DomLimits::default(), || {
            asked.set(asked.get() + 1);
            asked.get() > 8
        });
        assert!(matches!(got, Err(DomError::Stopped)), "{}", &page[..16]);
    }
}
