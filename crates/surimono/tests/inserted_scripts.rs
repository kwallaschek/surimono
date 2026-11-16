//! A script loader through the facade (P2-DOM-19): a `<script src>` that a
//! script inserts loads through the render's loader, allowlist, caps and
//! report, in the order the page asked, and runs.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use surimono::{Limits, Options, Renderer, ResourceFailureReason, ResourceKind, ResourceLoader};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn options() -> Options {
    let font = repo().join("fixtures/fonts/NotoSans-Regular.ttf");
    let bytes = std::fs::read(&font).unwrap_or_else(|e| panic!("{}: {e}", font.display()));
    let scale = std::env::var("SURIMONO_TEST_TIME_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    Options::new().font(bytes).pool_size(1).limits(Limits {
        time: Duration::from_secs(20) * scale,
        ..Limits::default()
    })
}

/// A Rails-shaped page: a layout that renders a report, and a classic loader
/// in the asset pipeline's manner, which appends `<script src>` for two parts
/// (in order, `async = false`) and runs a callback on their `load`.
const REPORT: &str = r#"<!doctype html>
<html><head><title>Invoice 42</title>
<script>
  window.Loader = {
    load(urls, done) {
      let left = urls.length;
      for (const url of urls) {
        const s = document.createElement('script');
        s.src = url;
        s.async = false;
        s.onload = () => { if (--left === 0) done(); };
        s.onerror = () => { document.getElementById('status').textContent = 'failed: ' + url; };
        document.head.appendChild(s);
      }
    }
  };
</script></head>
<body><h1 id=status>loading</h1><table id=lines></table>
<script>
  Loader.load(['/assets/money-3f2a.js', '/assets/invoice-9c1b.js'], () => {
    Invoice.render(document.getElementById('lines'), [['Design', 1200], ['Hosting', 80]]);
    document.getElementById('status').textContent = 'Total ' + Money.format(1280);
  });
</script>
</body></html>"#;

fn assets() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (
            "/assets/money-3f2a.js",
            b"window.Money = { format: (n) => n.toFixed(2) + ' EUR' };".to_vec(),
        ),
        (
            "/assets/invoice-9c1b.js",
            b"window.Invoice = { render(table, rows) { for (const [what, n] of rows) {
                const tr = table.insertRow(); tr.insertCell().textContent = what;
                tr.insertCell().textContent = Money.format(n); } } };"
                .to_vec(),
        ),
    ]
}

#[test]
fn a_classic_script_loader_runs_through_render_page() {
    let r = Renderer::new(options().resource_loader(ResourceLoader::from_map(assets())))
        .unwrap_or_else(|e| panic!("{e}"));
    let page = r.render_page(REPORT).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        page.html
            .contains("<h1 id=\"status\">Total 1280.00 EUR</h1>"),
        "{}",
        page.html
    );
    assert!(
        page.html.contains("<td>Hosting</td><td>80.00 EUR</td>"),
        "{}",
        page.html
    );
    assert_eq!(page.report.uncaught, 0, "{:?}", page.report.errors);
    assert_eq!(page.report.failed_count, 0, "{:?}", page.report.failed);
    // Same input, same output.
    let again = r.render_page(REPORT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(page.html, again.html);
}

/// A part the allowlist refuses: `error` at the element, the report entry
/// the parser's scripts get, and the page goes on.
#[test]
fn a_refused_part_is_an_error_event_and_a_report_entry() {
    let r = Renderer::new(
        options().resource_loader(ResourceLoader::new(["/assets/money-3f2a.js"], |_| {
            Ok(b"window.Money = {};".to_vec())
        })),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let page = r.render_page(REPORT).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        page.html.contains("failed: /assets/invoice-9c1b.js"),
        "{}",
        page.html
    );
    assert_eq!(page.report.failed_count, 1);
    assert_eq!(page.report.failed[0].kind, ResourceKind::Script);
    assert_eq!(page.report.failed[0].url, "/assets/invoice-9c1b.js");
    assert_eq!(
        page.report.failed[0].reason,
        ResourceFailureReason::NotAllowed
    );
}

/// A loader on the caller's thread (as the gem's) is asked one URL at a
/// time, in the order the page asked, the parser's script first.
#[test]
fn the_callers_loader_is_asked_in_the_order_the_page_asked() {
    let r = Renderer::new(options()).unwrap_or_else(|e| panic!("{e}"));
    let html = REPORT.replace(
        "<script>\n  Loader.load",
        "<script src=\"/assets/first.js\"></script><script>\n  Loader.load",
    );
    let asked = Arc::new(Mutex::new(Vec::new()));
    let files = assets();
    let mut load = |url: &str| {
        asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(url.to_owned());
        files
            .iter()
            .find(|(u, _)| *u == url)
            .map(|(_, b)| b.clone())
            .or_else(|| (url == "/assets/first.js").then(Vec::new))
            .ok_or_else(|| "missing".to_owned())
    };
    let cancel = AtomicBool::new(false);
    let page = r
        .render_page_loading(&html, &["/assets/".to_owned()], &mut load, &cancel)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(page.html.contains("Total 1280.00 EUR"), "{}", page.html);
    assert_eq!(
        *asked.lock().unwrap_or_else(|e| e.into_inner()),
        [
            "/assets/first.js",
            "/assets/money-3f2a.js",
            "/assets/invoice-9c1b.js"
        ]
    );
}
