//! `Renderer::render_page`: a page's own scripts, then its islands. Expected
//! values are what the HTML and DOM standards say a page computes; the visual
//! truth lives in fixtures/.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use surimono::{
    Error, ErrorPolicy, Limits, Loader, Options, Page, PageSettle, Renderer, ResourceKind,
};

fn font() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/fonts/NotoSans-Regular.ttf"
    );
    std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

const BUNDLE: &str = r#"
function entry(specJson, ctx) {
  const spec = JSON.parse(specJson);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${ctx.width}" height="${ctx.height}"><title>${spec.t}</title></svg>`;
}"#;

fn options() -> Options {
    Options::new()
        .font(font())
        .bundle("t", BUNDLE, "entry")
        .pool_size(2)
}

fn renderer(options: Options) -> Renderer {
    Renderer::new(options).unwrap_or_else(|e| panic!("{e}"))
}

fn page(r: &Renderer, html: &str) -> Page {
    r.render_page(html).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn a_page_runs_and_prints_what_its_scripts_made() {
    let r = renderer(options());
    let out = page(
        &r,
        "<!doctype html><title>x</title><body><div id=a></div><script>
           document.getElementById('a').textContent = 'from ' + 'script';
           document.addEventListener('DOMContentLoaded', () => {
             document.getElementById('a').append(document.createElement('b'));
           });
           setTimeout(() => { document.title = 'later'; }, 3000);
         </script>",
    );
    assert!(
        out.html.starts_with("<!doctype html><html>"),
        "{}",
        out.html
    );
    assert!(out.html.contains("<title>later</title>"), "{}", out.html);
    assert!(
        out.html.contains("<div id=\"a\">from script<b></b></div>"),
        "{}",
        out.html
    );
    assert_eq!(out.report.settle, PageSettle::Settled);
    assert_eq!(out.report.virtual_ms, 3000);
    assert!(out.report.is_clean(), "{:?}", out.report);
}

#[test]
fn without_a_doctype_the_page_stays_in_quirks_mode() {
    let r = renderer(options());
    let out = page(&r, "<p>x</p>");
    assert!(out.html.starts_with("<html>"), "{}", out.html);
    let out = page(&r, "<!-- c --><!DOCTYPE html><p>x</p>");
    assert!(
        out.html.starts_with("<!DOCTYPE html><html>"),
        "{}",
        out.html
    );
}

#[test]
fn islands_written_by_scripts_render() {
    let r = renderer(options());
    let out = page(
        &r,
        r#"<!doctype html><body>
        <script type="application/vnd.surimono+json" id="one" data-renderer="t" data-width="30" data-height="20">{"t":"a"}</script>
        <script>
          const s = document.createElement('script');
          s.type = 'application/vnd.surimono+json';
          s.dataset.renderer = 't';
          s.textContent = JSON.stringify({t: document.getElementById('one').textContent.length});
          document.body.append(s);
        </script>"#,
    );
    assert!(out.html.contains("<title>a</title>"), "{}", out.html);
    assert!(out.html.contains("<title>9</title>"), "{}", out.html);
    assert!(!out.html.contains("<script"), "{}", out.html);
}

#[test]
fn noscript_never_reaches_the_printer() {
    let r = renderer(options());
    let out = page(
        &r,
        "<!doctype html><head><noscript><style>p{display:none}</style></noscript></head><body>
         <noscript>Please enable JavaScript</noscript>
         <noscript><!-- </noscript><p id=a>kept</p>
         <script>document.getElementById('a').textContent += '!'</script>",
    );
    assert!(!out.html.contains("enable"), "{}", out.html);
    assert!(!out.html.contains("display:none"), "{}", out.html);
    assert!(out.html.contains("<p id=\"a\">kept!</p>"), "{}", out.html);
}

#[test]
fn an_exception_is_reported_and_the_page_goes_on() {
    let r = renderer(options());
    let out = page(
        &r,
        "<!doctype html><body><script>missing();</script>
         <script>document.body.dataset.ran = 'yes'; Promise.reject(new Error('later'));</script>
         <script src='gone.js'></script>",
    );
    assert!(out.html.contains("data-ran=\"yes\""), "{}", out.html);
    assert_eq!(out.report.uncaught, 1);
    assert!(out.report.errors[0].contains("missing"), "{:?}", out.report);
    assert_eq!(out.report.unhandled_rejections, 1);
    assert_eq!(out.report.missing_scripts, ["gone.js"]);
    assert!(!out.report.is_clean());
}

#[test]
fn same_input_same_output() {
    const PAGE: &str = "<!doctype html><body><script>
        const a = [Math.random(), new Date().toISOString(), performance.now()];
        setTimeout(() => { a.push(Math.random(), Date.now(), performance.now());
          document.body.textContent = a.join(); }, 250);
        </script>";
    let r = renderer(options().seed(7).now_ms(1_700_000_000_000));
    let first = page(&r, PAGE);
    assert_eq!(page(&r, PAGE), first);
    // A new renderer, and so a new pool, gives the same bytes.
    assert_eq!(
        page(&renderer(options().seed(7).now_ms(1_700_000_000_000)), PAGE),
        first
    );
    // The seed and the clock reach the page.
    assert_ne!(page(&renderer(options().seed(8)), PAGE).html, first.html);
    assert!(first.html.contains("2023-11-14T22:13:20.000Z"), "{first:?}");
}

#[test]
fn the_budget_and_the_task_limit_end_a_page() {
    let r = renderer(options().page_budget_ms(1000));
    const TICK: &str = "<!doctype html><body><script>
        let n = 0; setInterval(() => { document.body.dataset.n = ++n; }, 300);
        </script>";
    let out = page(&r, TICK);
    assert_eq!(out.report.settle, PageSettle::BudgetReached);
    assert!(out.html.contains("data-n=\"3\""), "{}", out.html);
    assert!(out.report.is_clean());

    let r = renderer(options().page_max_tasks(10));
    let out = page(&r, TICK);
    assert_eq!(out.report.settle, PageSettle::TaskLimit);
    assert!(!out.report.is_clean());
}

#[test]
fn limits_are_typed_errors_and_the_renderer_carries_on() {
    let quick = renderer(options().limits(Limits {
        time: Duration::from_millis(100),
        ..Limits::default()
    }));
    let small = renderer(options().limits(Limits {
        memory_bytes: 8 * 1024 * 1024,
        ..Limits::default()
    }));
    // Long enough for a debug build to parse 100,000 nodes on a busy machine.
    let long = renderer(options().limits(Limits {
        time: Duration::from_secs(60),
        ..Limits::default()
    }));
    for (r, script, kind) in [
        (&quick, "for (;;) {}", "time-limit"),
        (
            &quick,
            "setTimeout(function f() { setTimeout(f, 0); for (;;) {} }, 0)",
            "time-limit",
        ),
        (
            &small,
            "const a = []; for (;;) a.push(new Array(1e5).fill(1));",
            "memory-limit",
        ),
        (&small, "(function f() { f(); })()", "stack-overflow"),
        (
            &long,
            "document.body.innerHTML = '<i></i>'.repeat(200000);",
            "document-limit",
        ),
        (&small, "document.cloneNode()", "unsupported"),
    ] {
        match r.render_page(&format!("<!doctype html><body><script>{script}</script>")) {
            Err(e) => {
                assert_eq!(e.kind(), kind, "{script}: {e}");
                assert!(e.island().is_none());
            }
            Ok(out) => panic!("{script}: expected {kind}, got {out:?}"),
        }
        assert!(page(r, "<p>x</p>").report.is_clean());
    }
}

#[test]
fn a_document_too_large_to_hold_is_a_typed_error() {
    let r = renderer(options());
    let deep = "<div>".repeat(5_000);
    let e = r.render_page(&deep).map(|_| ()).unwrap_err();
    assert_eq!(e.kind(), "document-limit", "{e}");
}

#[test]
fn an_island_in_a_page_follows_the_error_policy() {
    const PAGE: &str = r#"<!doctype html><script type="application/vnd.surimono+json" data-renderer="nope">{}</script>"#;
    assert!(matches!(
        renderer(options()).render_page(PAGE),
        Err(Error::UnknownRenderer { .. })
    ));
    let r = renderer(options().error_policy(ErrorPolicy::Placeholder));
    assert!(page(&r, PAGE).html.contains("surimono-island-error"));
}

#[test]
fn a_cancel_returns_at_once() {
    let r = renderer(options().limits(Limits {
        time: Duration::from_secs(60),
        ..Limits::default()
    }));
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        r.render_page_cancellable("<p>x</p>", &cancel),
        Err(Error::Cancelled)
    ));
    let cancel = Arc::new(AtomicBool::new(false));
    let setter = {
        let cancel = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            cancel.store(true, Ordering::Release);
        })
    };
    let started = Instant::now();
    let out = r.render_page_cancellable("<script>for (;;) {}</script>", &cancel);
    let took = started.elapsed();
    let _ = setter.join();
    assert!(matches!(out, Err(Error::Cancelled)), "{out:?}");
    assert!(took < Duration::from_secs(20), "took {took:?}");
}

#[test]
fn many_threads_share_one_renderer() {
    let r = Arc::new(renderer(options()));
    let html = "<!doctype html><body><script>
        document.body.textContent = Math.random();</script>";
    let expected = page(&r, html);
    let threads: Vec<_> = (0..6)
        .map(|_| {
            let r = Arc::clone(&r);
            std::thread::spawn(move || r.render_page(html).map_err(|e| e.to_string()))
        })
        .collect();
    for t in threads {
        match t.join() {
            Ok(Ok(out)) => assert_eq!(out, expected),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn a_loader_gives_a_page_its_scripts_and_stylesheets() {
    let r = renderer(options());
    let mut asked = Vec::new();
    let page = {
        let mut loader = Loader::new(|url, kind| {
            asked.push((url.to_owned(), kind));
            match url {
                "http://localhost/js/a.js" => Some(b"window.order = ['a'];".to_vec()),
                "http://localhost/js/b.js" => Some(b"order.push('b');".to_vec()),
                "http://localhost/css/p.css" => Some(b"p { color: red }".to_vec()),
                _ => None,
            }
        })
        .allow("/js/")
        .allow("/css/");
        r.render_page_with(
            r#"<!doctype html><link rel="stylesheet" href="/css/p.css"><body>
            <script src="js/b.js" defer></script>
            <script src="js/a.js"></script>
            <script src="js/none.js"></script>
            <script src="https://elsewhere.example/x.js"></script>
            <script>
              addEventListener('load', () => {
                document.body.dataset.order = order.join();
                const l = document.createElement('link');
                l.rel = 'stylesheet'; l.href = '/css/gone.css';
                document.head.append(l);
              });
            </script>"#,
            &mut loader,
        )
        .unwrap_or_else(|e| panic!("{e}"))
    };
    assert!(page.html.contains("data-order=\"a,b\""), "{}", page.html);
    assert!(
        page.html.contains("<style>p { color: red }</style>"),
        "{}",
        page.html
    );
    assert!(!page.html.contains("gone.css"), "{}", page.html);
    assert_eq!(
        page.report.missing_scripts,
        ["js/none.js", "https://elsewhere.example/x.js"]
    );
    assert_eq!(page.report.missing_stylesheets, ["/css/gone.css"]);
    // In tree order, one at a time, and never for what isn't listed.
    assert_eq!(
        asked,
        [
            ("http://localhost/js/b.js".to_owned(), ResourceKind::Script),
            ("http://localhost/js/a.js".to_owned(), ResourceKind::Script),
            (
                "http://localhost/js/none.js".to_owned(),
                ResourceKind::Script
            ),
            (
                "http://localhost/css/p.css".to_owned(),
                ResourceKind::Stylesheet
            ),
            (
                "http://localhost/css/gone.css".to_owned(),
                ResourceKind::Stylesheet
            ),
        ]
    );
}

#[test]
fn without_a_loader_links_stay_for_the_printer() {
    let r = renderer(options());
    let out = page(
        &r,
        "<link rel=stylesheet href=a.css><script src=a.js></script>",
    );
    assert!(
        out.html
            .contains("<link rel=\"stylesheet\" href=\"a.css\">"),
        "{}",
        out.html
    );
    assert_eq!(out.report.missing_scripts, ["a.js"]);
    assert!(out.report.missing_stylesheets.is_empty());
}

#[test]
fn slow_loads_dont_count_against_the_scripts_time_limit() {
    let r = renderer(options().limits(Limits {
        time: Duration::from_millis(200),
        ..Limits::default()
    }));
    let mut loader = Loader::new(|_, _| {
        std::thread::sleep(Duration::from_millis(100));
        Some(b"document.title = 't'".to_vec())
    })
    .allow("/");
    let html = "<title>x</title>".to_owned() + &"<script src=/a.js></script>".repeat(5);
    let out = r.render_page_with(&html, &mut loader);
    assert!(out.is_ok(), "{out:?}");
}

#[test]
fn a_bad_loader_is_invalid_options() {
    let r = renderer(options());
    let mut loader = Loader::new(|_, _| None).base_url("nope");
    assert!(matches!(
        r.render_page_with("<p>x</p>", &mut loader),
        Err(Error::InvalidOptions(_))
    ));
}

#[test]
fn a_cancel_ends_a_page_at_its_next_load() {
    let r = renderer(options());
    let cancel = AtomicBool::new(false);
    let calls = std::cell::Cell::new(0);
    let mut loader = Loader::new(|_, _| {
        calls.set(calls.get() + 1);
        cancel.store(true, Ordering::Release);
        Some(Vec::new())
    })
    .allow("/");
    let html = "<script src=/a.js></script><script src=/b.js></script>";
    let out = r.render_page_with_cancellable(html, &mut loader, &cancel);
    assert!(matches!(out, Err(Error::Cancelled)), "{out:?}");
    drop(loader);
    assert_eq!(calls.get(), 1);
}
