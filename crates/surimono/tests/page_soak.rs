//! The soak test for pages: many renders in one process through the facade,
//! and memory must stay flat after warm-up.
//!
//! Same method as `crates/islands/tests/soak.rs`: the pass/fail signal is live
//! heap bytes, counted by a global allocator wrapper; QuickJS allocates through
//! `std::alloc` too, so a leak in the pool, the bindings, blitz-dom or the
//! JavaScript heap shows up byte for byte. Resident memory is printed.
//!
//! The 10,000-render soak is `#[ignore]`d and runs in release with the other
//! soaks (`scripts/ci/soak.sh`); a short version runs with the ordinary tests.

use std::alloc::System;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};
use surimono::{Limits, Options, Renderer};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

/// Largest growth of live heap bytes from the end of warm-up to the end.
const MAX_LIVE_GROWTH: i64 = 64 * 1024;
/// Threads rendering at the same time, and workers in the pool.
const CALLERS: usize = 2;

const BUNDLE: &str = r#"
function entry(specJson, ctx) {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${ctx.width}" height="${ctx.height}"><desc>${specJson.length}</desc></svg>`;
}"#;

/// A report-like page, different each time so blitz-dom's atoms and id maps
/// are exercised. Some renders fail on purpose, so error paths soak too.
fn page(i: usize) -> String {
    let rows: String = (0..40)
        .map(|r| format!("<tr id=\"r{r}\"><td>{}</td><td></td></tr>", i * r))
        .collect();
    let fail = match i % 97 {
        13 => "for (;;) {}",
        31 => "const a = []; for (;;) a.push(new Array(1e5).fill(1));",
        57 => "(function f() { f(); })();",
        71 => "throw new Error('planned');",
        _ => "",
    };
    format!(
        "<!doctype html><style>td {{ color: red }}</style><body><table>{rows}</table>
         <script type=\"application/vnd.surimono+json\" data-renderer=\"t\" data-width=\"30\"
                 data-height=\"20\">{{\"i\":{i}}}</script>
         <script>
           for (const c of document.querySelectorAll('td:last-child')) {{
             c.textContent = c.previousSibling.textContent * Math.random();
             c.classList.add('n');
           }}
           new MutationObserver(() => {{}}).observe(document.body, {{ childList: true }});
           addEventListener('load', () => {{
             setTimeout(() => {{ document.title = 'p{i}'; }}, 100);
             document.body.append(document.createElement('footer'));
           }});
           {fail}
         </script>"
    )
}

/// Live heap bytes, once the workers have freed the last page they ran (they
/// do that just after answering).
fn quiet_live_bytes() -> i64 {
    std::thread::sleep(Duration::from_millis(100));
    let s = GLOBAL.stats();
    s.bytes_allocated as i64 - s.bytes_deallocated as i64
}

/// This process's resident memory in KiB (0 where unknown).
fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map_or(0, |pages| pages * 4)
}

fn soak(renders: usize, warm_up: usize) {
    let font = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/fonts/NotoSans-Regular.ttf"
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    let renderer = Arc::new(
        Renderer::new(
            Options::new()
                .font(font)
                .bundle("t", BUNDLE, "entry")
                .pool_size(CALLERS)
                .limits(Limits {
                    memory_bytes: 32 * 1024 * 1024,
                    time: Duration::from_secs(1),
                    ..Limits::default()
                }),
        )
        .unwrap_or_else(|e| panic!("{e}")),
    );
    let run = |range: std::ops::Range<usize>| {
        let next = Arc::new(AtomicUsize::new(range.start));
        let threads: Vec<_> = (0..CALLERS)
            .map(|_| {
                let (renderer, next) = (Arc::clone(&renderer), Arc::clone(&next));
                let end = range.end;
                std::thread::spawn(move || {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= end {
                            return;
                        }
                        let html = page(i);
                        match (renderer.render_page(&html), i % 97) {
                            (Ok(out), 71) => assert_eq!(out.report.uncaught, 1, "{i}"),
                            (Ok(out), _) => {
                                assert!(out.html.contains("<svg"), "{i}");
                                assert!(out.report.is_clean(), "{i}: {:?}", out.report);
                            }
                            (Err(e), 13 | 31 | 57) => {
                                let limit =
                                    e.kind().ends_with("limit") || e.kind() == "stack-overflow";
                                assert!(limit, "{i}: {e}");
                            }
                            (Err(e), _) => panic!("{i}: {e}"),
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap_or_else(|_| panic!("a caller panicked"));
        }
    };
    run(0..warm_up);
    let before = quiet_live_bytes();
    let rss_before = rss_kib();
    let step = ((renders - warm_up) / 10).max(1);
    let mut at = warm_up;
    while at < renders {
        let end = (at + step).min(renders);
        run(at..end);
        at = end;
        println!("{at}: live {} B, resident {} KiB", quiet_live_bytes(), rss_kib());
    }
    let growth = quiet_live_bytes() - before;
    let rss_after = rss_kib();
    println!(
        "page soak: {renders} renders, live heap {growth:+} B after warm-up, \
         resident {rss_before} -> {rss_after} KiB"
    );
    assert!(
        growth < MAX_LIVE_GROWTH,
        "live heap grew {growth} bytes in {} renders",
        renders - warm_up
    );
}

#[test]
fn memory_stays_flat_over_a_few_hundred_pages() {
    soak(600, 200);
}

#[test]
#[ignore = "10,000 renders; run in release by scripts/ci/soak.sh"]
fn page_soak_10000_renders() {
    soak(10_000, 2_000);
}
