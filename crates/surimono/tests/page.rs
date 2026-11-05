//! Page scripts through the facade: what comes out is safe to print, limits
//! are typed, and a render doesn't depend on the one before it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use surimono::{
    Error, ErrorPolicy, LayoutQueries, Limits, Options, PageOutput, Renderer,
    ResourceFailureReason, ResourceKind, ResourceLoader, SettleReason,
};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// An island renderer that writes its spec's text into the SVG.
const ECHO: &str = r#"function entry(specJson, ctx) {
  const spec = JSON.parse(specJson);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${ctx.width}" height="${ctx.height}"><desc>${spec.text}</desc></svg>`;
}"#;

fn options() -> Options {
    let font = repo().join("fixtures/fonts/NotoSans-Regular.ttf");
    let bytes = std::fs::read(&font).unwrap_or_else(|e| panic!("{}: {e}", font.display()));
    Options::new()
        .font(bytes)
        .bundle("echo", ECHO, "entry")
        .pool_size(2)
        .limits(Limits {
            time: Duration::from_secs(20),
            ..Limits::default()
        })
}

fn renderer(options: Options) -> Renderer {
    Renderer::new(options).unwrap_or_else(|e| panic!("{e}"))
}

fn page(r: &Renderer, html: &str) -> PageOutput {
    r.render_page(html).unwrap_or_else(|e| panic!("{e}"))
}

const PAGE: &str = "<!doctype html><html><head><title>x</title></head><body><p id=a>?</p>\
    <script>document.getElementById('a').textContent = 'ran';</script></body></html>";

#[test]
fn scripts_run_only_when_asked() {
    let r = renderer(options());
    assert_eq!(r.render_islands(PAGE).ok().as_deref(), Some(PAGE));
    assert_eq!(
        page(&r, PAGE).html,
        "<!doctype html><html><head><title>x</title></head><body><p id=\"a\">ran</p></body></html>"
    );
}

#[test]
fn a_page_settles_on_the_virtual_clock() {
    let r = renderer(options());
    let started = Instant::now();
    let out = page(
        &r,
        "<body><script>
           addEventListener('load', () => setTimeout(() => {
             document.body.setAttribute('t', Date.now() - t0);
           }, 9000));
           const t0 = Date.now();
         </script>",
    );
    assert!(out.html.contains("t=\"9000\""), "{}", out.html);
    assert_eq!(out.report.settled, SettleReason::Idle);
    assert!(started.elapsed() < Duration::from_secs(8));

    let interval = page(&r, "<script>setInterval(() => {}, 100);</script>");
    assert_eq!(interval.report.settled, SettleReason::VirtualBudget);

    let busy = page(
        &r,
        "<script>for (let i = 0; i < 9; i++) {
           const f = () => { for (let k = 0; k < 500; k++) setTimeout(() => {}, 1); setTimeout(f, 1); };
           f();
         }</script>",
    );
    assert_eq!(busy.report.settled, SettleReason::TaskLimit);
}

#[test]
fn the_doctype_keeps_its_mode() {
    let r = renderer(options());
    let mode = |html: &str| {
        // What the printer will see is the same mode the page ran in.
        let out = page(
            &r,
            &format!("{html}<body><script>document.body.id = document.compatMode;</script>"),
        );
        out.html
    };
    let standards = mode("<!DOCTYPE html>");
    assert!(
        standards.starts_with("<!doctype html><html>"),
        "{standards}"
    );
    assert!(standards.contains("id=\"CSS1Compat\""), "{standards}");
    let quirks = mode("");
    assert!(quirks.starts_with("<html>"), "{quirks}");
    assert!(quirks.contains("id=\"BackCompat\""), "{quirks}");
}

#[test]
fn what_comes_out_can_be_printed() {
    let r = renderer(options());
    let html = "<!doctype html><head><noscript><link rel=stylesheet href=no.css></noscript>
        <script type=text/plain>data</script></head>
        <body><noscript><p>Turn on JavaScript</p></noscript>
        <script>document.body.append(document.createElement('noscript'));
          const s = document.createElement('script');
          s.textContent = 'document.title = 1'; document.body.append(s);</script>
        <template><script>inert()</script></template>
        <main onclick=\"go()\">kept</main>";
    let out = page(&r, html);
    assert!(!out.html.contains("<noscript"), "{}", out.html);
    assert!(!out.html.contains("Turn on"), "{}", out.html);
    assert!(!out.html.contains("createElement"), "{}", out.html);
    assert!(!out.html.contains("data"), "{}", out.html);
    assert!(out.html.contains("<main onclick=\"go()\">kept</main>"));
    // A template's contents never print, and stay as they were.
    assert!(
        out.html
            .contains("<template><script>inert()</script></template>")
    );

    let kept = renderer(options().keep_scripts(true));
    let out = page(&kept, html);
    assert!(out.html.contains("createElement"), "{}", out.html);
    assert!(out.html.contains("text/plain"), "{}", out.html);
    assert!(!out.html.contains("<noscript"), "{}", out.html);
}

/// A page can't get its scripts past the clean-up, whatever it replaces: that
/// runs in a context of its own.
#[test]
fn a_page_cannot_get_in_the_way_of_the_cleanup() {
    let r = renderer(options());
    let out = page(
        &r,
        "<!doctype html><noscript><img src=evil.png></noscript><script>
           Node.prototype.removeChild = () => {};
           Document.prototype.querySelectorAll = () => [];
           Object.defineProperty(document, 'compatMode', { value: 'x' });
           JSON.stringify = () => '{}';
           String.prototype.trim = () => 'application/vnd.surimono+json';
         </script>",
    );
    assert_eq!(
        out.html,
        "<!doctype html><html><head></head><body></body></html>"
    );
}

/// A page can empty its own tree; there is still an answer.
#[test]
fn a_page_can_remove_everything() {
    let r = renderer(options());
    let out = page(
        &r,
        "<!doctype html><script>document.documentElement.remove();</script>",
    );
    assert_eq!(out.html, "<!doctype html>");
    assert_eq!(out.report.uncaught, 0);
}

#[test]
fn islands_render_after_the_scripts() {
    let r = renderer(options());
    let marker = |text: &str| {
        format!(
            "<script type=\"application/vnd.surimono+json\" data-renderer=\"echo\" \
             data-width=\"10\" data-height=\"10\">{{\"text\":\"{text}\"}}</script>"
        )
    };
    let html = format!(
        "<!doctype html><body>{}<div id=d></div><script>
           const m = document.createElement('script');
           m.type = 'APPLICATION/vnd.surimono+json; x=1';
           m.setAttribute('data-renderer', 'echo');
           m.textContent = JSON.stringify({{ text: 'from a script' }});
           document.getElementById('d').append(m);
         </script>",
        marker("in the page")
    );
    let out = page(&r, &html);
    assert_eq!(out.html.matches("<svg").count(), 2, "{}", out.html);
    assert!(
        out.html.contains("<desc>in the page</desc>"),
        "{}",
        out.html
    );
    assert!(
        out.html.contains("<desc>from a script</desc>"),
        "{}",
        out.html
    );
    assert!(!out.html.contains("vnd.surimono"), "{}", out.html);

    // A bad island follows the error policy; the page's report stays.
    let bad = "<script>console_is_missing();</script>\
        <script type=\"application/vnd.surimono+json\" data-renderer=\"none\">{}</script>";
    assert!(matches!(
        r.render_page(bad),
        Err(Error::UnknownRenderer { .. })
    ));
    let soft = renderer(options().error_policy(ErrorPolicy::Placeholder));
    let out = page(&soft, bad);
    assert!(out.html.contains("surimono-island-error"), "{}", out.html);
    assert_eq!(out.report.uncaught, 1);
}

#[test]
fn the_report_says_what_went_wrong() {
    let r = renderer(options());
    let out = page(
        &r,
        "<img src=a.png><img src='data:image/png;base64,AA=='>
         <script>boom();</script>
         <script>Promise.reject(new Error('later'));</script>
         <script src=a.js></script>
         <script>document.body.id = 'after';</script>",
    );
    let report = &out.report;
    assert_eq!(report.uncaught, 1);
    assert!(report.errors[0].contains("boom"), "{report:?}");
    assert_eq!(report.unhandled_rejections, 1);
    assert!(report.rejections[0].contains("later"), "{report:?}");
    assert_eq!(report.failed_count, 2);
    assert_eq!(report.failed[0].kind, ResourceKind::Script);
    assert_eq!(report.failed[0].url, "a.js");
    assert_eq!(report.failed[0].reason, ResourceFailureReason::NoLoader);
    assert_eq!(report.failed[1].kind, ResourceKind::Image);
    assert_eq!(report.failed[1].url, "a.png");
    assert_eq!(report.failed[1].reason, ResourceFailureReason::NotLoaded);
    assert!(out.html.contains("id=\"after\""), "{}", out.html);
}

#[test]
fn limits_are_typed_and_the_renderer_carries_on() {
    let quick = renderer(options().limits(Limits {
        time: Duration::from_millis(200),
        ..Limits::default()
    }));
    let small = renderer(options().limits(Limits {
        memory_bytes: 8 << 20,
        ..Limits::default()
    }));
    for (r, html, kind) in [
        (&quick, "<script>for (;;) {}</script>", "time-limit"),
        (
            &quick,
            "<script>setTimeout(() => { for (;;) {} });</script>",
            "time-limit",
        ),
        (
            &small,
            "<script>const a = []; for (;;) a.push(new Array(1e5).fill(1));</script>",
            "memory-limit",
        ),
        (
            &small,
            "<script>const f = () => f() + 1; f();</script>",
            "stack-overflow",
        ),
        (
            &small,
            "<script>let n = document.body;
               for (;;) n = n.appendChild(document.createElement('p'));</script>",
            "dom-limit",
        ),
    ] {
        match r.render_page(html) {
            Err(e) => {
                assert_eq!(e.kind(), kind, "{html}: {e}");
                assert!(e.island().is_none());
            }
            Ok(out) => panic!("{html}: expected {kind}, got {out:?}"),
        }
        assert_eq!(
            page(r, "<script>document.title = 'y';</script>")
                .report
                .uncaught,
            0
        );
    }
    assert!(matches!(
        quick.render_page("<script>for (;;) {}</script>"),
        Err(Error::PageTimeLimit)
    ));
}

#[test]
fn the_markup_itself_is_limited() {
    let r = renderer(options());
    let deep = "<div>".repeat(5_000);
    match r.render_page(&deep) {
        Err(Error::PageDomLimit { message }) => assert!(message.contains("deeper"), "{message}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn layout_queries_answer_zeros_by_default() {
    let probe = "<!doctype html><div id=a></div><script>
      const a = document.getElementById('a');
      const r = a.getBoundingClientRect();
      document.body.dataset.got = JSON.stringify([
        r.width, r.right, a.offsetWidth, a.clientHeight, a.scrollTop, a.offsetParent,
        a.getClientRects().length, innerWidth,
        getComputedStyle(a).getPropertyValue('color'),
      ]);
    </script>";
    let out = page(&renderer(options()), probe);
    assert_eq!(out.report.uncaught, 0, "{:?}", out.report);
    assert!(
        out.html.contains("[0,0,0,0,0,null,0,0,&quot;&quot;]")
            || out.html.contains("[0,0,0,0,0,null,0,0,\"\"]"),
        "{}",
        out.html
    );
}

#[test]
fn layout_queries_can_throw() {
    let r = renderer(options().layout_queries(LayoutQueries::Throw));
    let out = page(
        &r,
        "<div id=a></div><script>
           const a = document.getElementById('a');
           const seen = [];
           for (const f of [() => a.getBoundingClientRect(), () => a.offsetWidth,
                            () => getComputedStyle(a), () => innerHeight]) {
             try { f(); seen.push('no'); } catch (e) { seen.push(e.name); }
           }
           document.body.dataset.got = seen.join();
         </script>
         <script>a.getClientRects();</script>",
    );
    let name = "NotSupportedError";
    let want = [name, name, name, name].join(",");
    assert!(out.html.contains(&want), "{}", out.html);
    assert_eq!(out.report.uncaught, 1);
    assert!(
        out.report.errors[0].contains("getClientRects() needs layout"),
        "{:?}",
        out.report
    );
}

/// The shim is a script, but no page can see it in the tree, and its run takes
/// no virtual time.
#[test]
fn the_shim_leaves_no_trace_in_the_tree() {
    let out = page(
        &renderer(options()),
        "<!doctype html><html lang=en><head><script>
           const root = document.documentElement;
           root.dataset.got = [
             document.querySelectorAll('script').length, root.firstChild.nodeName,
             document.currentScript.textContent.length > 0,
             Date.now() - new Date(2026, 0, 1).getTime(), performance.now(),
           ].join();
         </script>",
    );
    assert_eq!(out.report.uncaught, 0, "{:?}", out.report);
    assert!(
        out.html.contains("data-got=\"1,HEAD,true,0,0\""),
        "{}",
        out.html
    );
}

#[test]
fn same_input_same_output() {
    let html = "<!doctype html><body><script>
        document.body.id = Math.random();
        document.body.title = Date.now() + ':' + new Date().getTimezoneOffset();
        setTimeout(() => { document.body.className = Math.random(); }, 50);
      </script>";
    let opts = || {
        options()
            .seed(7)
            .now_ms(1_700_000_000_000)
            .utc_offset_minutes(540)
    };
    let r = renderer(opts());
    let first = page(&r, html);
    assert_eq!(page(&r, html), first);
    assert_eq!(page(&renderer(opts()), html), first, "a new renderer");
    let threads: Vec<_> = {
        let r = Arc::new(r);
        (0..6)
            .map(|_| {
                let r = Arc::clone(&r);
                std::thread::spawn(move || r.render_page(html).ok())
            })
            .collect()
    };
    for t in threads {
        assert_eq!(t.join().ok().flatten().as_ref(), Some(&first));
    }
    assert_ne!(
        page(&renderer(opts().seed(8)), html).html,
        first.html,
        "the seed reaches the page"
    );
}

#[test]
fn one_render_leaves_nothing_for_the_next() {
    let r = renderer(options().pool_size(1));
    let leaky = "<script>globalThis.left = 1; Object.prototype.polluted = 1;
        document.body.id = 'one';</script>";
    assert!(page(&r, leaky).html.contains("id=\"one\""));
    let out = page(
        &r,
        "<script>document.body.title = typeof left + typeof polluted;</script>",
    );
    assert!(out.html.contains("undefinedundefined"), "{}", out.html);
}

#[test]
fn a_cancel_ends_the_call_at_once() {
    let r = renderer(options().pool_size(1).limits(Limits {
        time: Duration::from_millis(600),
        ..Limits::default()
    }));
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        r.render_page_cancellable(PAGE, &cancel),
        Err(Error::Cancelled)
    ));
    cancel.store(false, Ordering::Release);
    assert_eq!(
        r.render_page_cancellable(PAGE, &cancel).ok(),
        r.render_page(PAGE).ok()
    );

    let cancel = Arc::new(AtomicBool::new(false));
    let setter = {
        let cancel = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            cancel.store(true, Ordering::Release);
        })
    };
    let started = Instant::now();
    let out = r.render_page_cancellable("<script>for (;;) {}</script>", &cancel);
    let took = started.elapsed();
    let _ = setter.join();
    assert!(matches!(out, Err(Error::Cancelled)), "{out:?}");
    assert!(took < Duration::from_millis(400), "took {took:?}");
    // The same renderer works; a page queued behind the abandoned one waits
    // for its time limit at most.
    assert_eq!(page(&r, PAGE).report.uncaught, 0);
}

#[test]
fn a_cancel_reaches_the_islands_after_a_page() {
    let r = renderer(options().bundle("spin", "function entry() { for (;;) {} }", "entry"));
    let marker =
        "<script type=\"application/vnd.surimono+json\" data-renderer=\"spin\">{}</script>";
    let html = format!("<script>document.title = 'y';</script>{marker}");
    let cancel = Arc::new(AtomicBool::new(false));
    let setter = {
        let cancel = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            cancel.store(true, Ordering::Release);
        })
    };
    let out = r.render_page_cancellable(&html, &cancel);
    let _ = setter.join();
    assert!(matches!(out, Err(Error::Cancelled)), "{out:?}");
}

#[test]
fn scripts_load_in_document_order_through_the_loader_only() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let loader = ResourceLoader::new(["js/"], move |url: &str| {
        log.lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(url.to_owned());
        match url {
            "js/a.js" => Ok(b"order.push('a');".to_vec()),
            "js/b.js" => Ok(b"order.push('b');".to_vec()),
            _ => Err("no such file".to_owned()),
        }
    });
    let r = renderer(options().resource_loader(loader));
    let out = page(
        &r,
        "<script>const order = [];</script>
         <script src=js/b.js defer></script>
         <script src=js/a.js></script>
         <script src=js/b.js></script>
         <script src=js/missing.js></script>
         <script src=js/../secret.js></script>
         <script src=https://elsewhere.test/x.js></script>
         <script>addEventListener('load', () => { document.body.id = order.join(''); });</script>",
    );
    assert!(out.html.contains("id=\"abb\""), "{}", out.html);
    assert_eq!(
        *seen.lock().unwrap_or_else(|e| e.into_inner()),
        ["js/b.js", "js/a.js", "js/missing.js"],
        "tree order, once per URL, nothing refused"
    );
    let reasons: Vec<_> = out.report.failed.iter().map(|f| f.reason.clone()).collect();
    assert_eq!(
        reasons,
        [
            ResourceFailureReason::Failed("no such file".into()),
            ResourceFailureReason::NotAllowed,
            ResourceFailureReason::NotAllowed,
        ]
    );
}

#[test]
fn a_failed_script_fires_error_and_the_page_goes_on() {
    let out = page(
        &renderer(options()),
        "<script>let e = 0;</script>
         <script src=a.js onerror='e++'></script>
         <script>document.body.id = 'e' + e;</script>",
    );
    assert!(out.html.contains("id=\"e1\""), "{}", out.html);
}

#[test]
fn a_panicking_loader_is_a_report_entry() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let loader = ResourceLoader::new(["x"], move |_: &str| -> Result<Vec<u8>, String> {
        counted.fetch_add(1, Ordering::Relaxed);
        panic!("bug in the caller's loader");
    });
    let r = renderer(options().resource_loader(loader));
    let out = page(&r, "<script src=x></script>");
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(out.report.failed_count, 1);
}

/// Many siblings to clear out stay within the page's limits.
#[test]
fn a_page_of_scripts_is_cleared_in_time() {
    let r = renderer(options());
    let html = "<i></i><script></script>".repeat(40_000);
    let out = page(&r, &html);
    assert!(!out.html.contains("<script"));
    assert_eq!(out.html.matches("<i>").count(), 40_000);
}

/// Each `<script src>` of a fixture's page, as written.
fn sources(input: &str) -> Vec<String> {
    input
        .split("src=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .filter(|s| s.ends_with(".js"))
        .map(str::to_owned)
        .collect()
}

/// Renders a real-world fixture page; `None` if the fixtures aren't here.
fn real_world(name: &str, keep: bool) -> Option<(String, PageOutput)> {
    let dir = repo().join("fixtures/real-world").join(name);
    let input = std::fs::read_to_string(dir.join("input.html")).ok()?;
    let expected = std::fs::read_to_string(dir.join("reference/dom.html")).ok()?;
    let files: Vec<(String, Vec<u8>)> = sources(&input)
        .into_iter()
        .map(|url| {
            let path = dir.join(&url);
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            (url, bytes)
        })
        .collect();
    let time = if cfg!(debug_assertions) { 120 } else { 20 };
    let r = renderer(
        options()
            .resource_loader(ResourceLoader::from_map(files))
            .keep_scripts(keep)
            .limits(Limits {
                time: Duration::from_secs(time),
                memory_bytes: 512 << 20,
                ..Limits::default()
            }),
    );
    Some((expected, page(&r, &input)))
}

/// As crates/dom/tests/real_world.rs: whitespace between tags doesn't matter,
/// and `style` goes last in each tag on both sides, since Chrome adds it
/// lazily (see the note there).
fn normal(html: &str) -> String {
    let squeezed = html.split_ascii_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::with_capacity(squeezed.len());
    let mut rest = squeezed.as_str();
    while let Some(at) = rest.find(" style=\"") {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        let body = &tail[" style=\"".len()..];
        let (Some(end), Some(close)) = (body.find('"'), tail.find('>')) else {
            out.push_str(tail);
            return out;
        };
        let attr = &tail[..end + " style=\"\"".len()];
        out.push_str(&tail[attr.len()..close]);
        out.push_str(attr);
        rest = &tail[close..];
    }
    out.push_str(rest);
    out
}

/// The whole road, facade to Chrome's DOM: same tree as the reference with the
/// scripts kept, none of them left without, and twice the same.
fn matches_chrome(name: &str) {
    let Some((expected, kept)) = real_world(name, true) else {
        eprintln!("{name}: no fixture here, skipped");
        return;
    };
    assert_eq!(kept.report.uncaught, 0, "{name}: {:?}", kept.report);
    assert_eq!(kept.report.failed_count, 0, "{name}: {:?}", kept.report);
    let (ours, chrome) = (normal(&kept.html), normal(&expected));
    let at = ours
        .bytes()
        .zip(chrome.bytes())
        .position(|(a, b)| a != b)
        .unwrap_or(ours.len().min(chrome.len()));
    let around = |h: &str| {
        h.chars()
            .skip(at.saturating_sub(80))
            .take(200)
            .collect::<String>()
    };
    assert!(
        ours == chrome,
        "{name}: differs at byte {at}:\n ours: {}\n chrome: {}",
        around(&ours),
        around(&chrome)
    );
    let Some((_, printed)) = real_world(name, false) else {
        return;
    };
    assert!(!printed.html.contains("<script"), "{name}");
    assert_eq!(
        real_world(name, false).map(|r| r.1),
        Some(printed),
        "{name}"
    );
}

#[test]
fn marked_matches_chrome() {
    matches_chrome("marked-report");
}

#[test]
fn highlight_js_matches_chrome() {
    matches_chrome("highlightjs-code");
}
