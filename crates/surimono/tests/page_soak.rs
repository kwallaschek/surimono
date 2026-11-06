//! Page renders through the facade, many in one process: a fresh document
//! each time, a page's script state gone with it, and memory flat after
//! warm-up. Method as `crates/islands/tests/soak.rs`: the signal is live heap
//! bytes through a counting global allocator, which sees QuickJS and blitz-dom
//! too; resident memory is printed only.
//!
//! The 10,000-render soak is `#[ignore]`d and runs in release with the other
//! soaks (`scripts/ci/soak.sh`: `--ignored soak`); a 300-render version runs
//! with the ordinary tests.

use std::alloc::System;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};
use surimono::{Error, Limits, Options, Renderer, ResourceLoader};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

/// Largest growth of live heap bytes allowed after warm-up.
const MAX_LIVE_GROWTH: i64 = 64 * 1024;
/// Threads rendering at the same time.
const CALLERS: usize = 2;

const ECHO: &str = r#"function entry(specJson, ctx) {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${ctx.width}" height="${ctx.height}"><desc>${JSON.parse(specJson).n}</desc></svg>`;
}"#;

/// A page per render, different each time, with an inline script, an external
/// one, timers, an exception, an island and work for the clean-up. Every 50th
/// ends at a limit on purpose, so that path is soaked too.
fn page(i: usize) -> String {
    let doom = if i % 50 == 49 {
        if i % 100 == 49 {
            "<script>for (;;) {}</script>"
        } else {
            "<script>const a = []; for (;;) a.push(new Array(2e5).fill(1));</script>"
        }
    } else {
        ""
    };
    let rows: String = (0..30)
        .map(|r| format!("<tr id=\"r{r}\"><td>k{i}-{r}</td></tr>"))
        .collect();
    format!(
        r#"<!doctype html><html><head><title>p{i}</title><style media="print">:root{{--a:1;--b:var(--a)}}</style><noscript><p>off</p></noscript></head>
<body><h1 id="title">Report {i}</h1><table id="t">{rows}</table><img src="a{i}.png"><div id="out"></div>
<script type="application/vnd.surimono+json" data-renderer="echo">{{"n":{i}}}</script>
<link rel="stylesheet" href="lib/app.css">
<script src="lib/app.js"></script>
<script src="lib/missing-{i}.js"></script>
<script>
  build(document.getElementById('out'), {i});
  const left = new Array(1000).fill({i});
  setTimeout(() => {{ document.getElementById('title').textContent = 'done ' + left.length; }}, 500);
  Promise.reject(new Error('kept {i}'));
  addEventListener('load', () => {{ throw new Error('uncaught {i}'); }});
  document.getElementById('r0').getBoundingClientRect();
</script>{doom}"#
    )
}

const STYLE: &str = "h1 { color: red } td { padding: 1px }";

const LIB: &str = "function build(out, i) {
  for (let k = 0; k < 100; k++) {
    const li = document.createElement('li');
    li.textContent = 'item ' + k + ' of ' + i;
    li.style.color = 'red';
    out.appendChild(li);
  }
  out.textContent = '';
}";

fn load(url: &str) -> Result<Vec<u8>, String> {
    match url {
        "lib/app.js" => Ok(LIB.as_bytes().to_vec()),
        "lib/app.css" => Ok(STYLE.as_bytes().to_vec()),
        _ => Err("no such file".to_owned()),
    }
}

fn renderer() -> Renderer {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/fonts/NotoSans-Regular.ttf");
    let font = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let loader = ResourceLoader::new(["lib/"], load);
    Renderer::new(
        Options::new()
            .font(font)
            .bundle("echo", ECHO, "entry")
            .resource_loader(loader)
            .pool_size(2)
            .limits(Limits {
                memory_bytes: 32 << 20,
                time: Duration::from_millis(if cfg!(debug_assertions) { 3000 } else { 1000 }),
                ..Limits::default()
            }),
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

/// Every third render loads through a function on the calling thread.
fn render(r: &Renderer, i: usize) {
    let result = if i % 3 == 0 {
        let allow = ["lib/".to_owned()];
        r.render_page_loading(&page(i), &allow, &mut load, &AtomicBool::new(false))
    } else {
        r.render_page(&page(i))
    };
    match result {
        Ok(out) => {
            assert_ne!(i % 50, 49, "render {i} should have hit a limit");
            assert!(out.html.contains("done 1000"), "render {i}: {}", out.html);
            assert!(out.html.contains("<svg"), "render {i}");
            assert_eq!(out.report.uncaught, 1, "render {i}: {:?}", out.report);
            assert_eq!(out.report.unhandled_rejections, 1, "render {i}");
            assert_eq!(out.report.failed_count, 2, "render {i}");
        }
        Err(Error::PageTimeLimit | Error::PageMemoryLimit) => {
            assert_eq!(i % 50, 49, "render {i}");
        }
        Err(e) => panic!("render {i}: {e}"),
    }
}

fn live_bytes() -> i64 {
    let s = GLOBAL.stats();
    s.bytes_allocated as i64 - s.bytes_deallocated as i64
}

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("VmRSS:"))
                .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        })
        .unwrap_or(0)
}

fn run_soak(name: &str, rounds: usize, sample_every: usize, warm_up: usize) {
    let r = Arc::new(renderer());
    let started = std::time::Instant::now();
    let mut samples = Vec::new();
    for round in 0..rounds {
        let callers: Vec<_> = (0..CALLERS)
            .map(|c| {
                let r = Arc::clone(&r);
                std::thread::spawn(move || render(&r, round * CALLERS + c))
            })
            .collect();
        for c in callers {
            if let Err(e) = c.join() {
                std::panic::resume_unwind(e);
            }
        }
        if (round + 1) % sample_every == 0 {
            samples.push((round + 1, live_bytes(), rss_kib()));
        }
    }
    let renders = rounds * CALLERS;
    eprintln!("{name}: {renders} renders in {:?}", started.elapsed());
    for (n, live, rss) in &samples {
        eprintln!("{name}: after round {n:>5}  live heap {live:>10} bytes  resident {rss:>8} KiB");
    }
    let warm = samples
        .iter()
        .find(|s| s.0 == warm_up)
        .map_or_else(|| panic!("no sample at {warm_up}"), |s| s.1);
    let highest = samples
        .iter()
        .filter(|s| s.0 >= warm_up)
        .map(|s| s.1)
        .max()
        .unwrap_or(warm);
    let end = samples.last().map_or(warm, |s| s.1);
    eprintln!(
        "{name}: live heap from round {warm_up}: {warm} -> {end} bytes ({:+}, highest {highest}), limit +{MAX_LIVE_GROWTH}",
        end - warm
    );
    assert!(
        highest - warm < MAX_LIVE_GROWTH,
        "{name}: live heap grew from {warm} to {highest} bytes after warm-up"
    );
}

/// The two tests share the process-wide byte counter.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn pages_300_renders_flat_memory() {
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    run_soak("pages-short", 150, 25, 50);
}

#[test]
#[ignore = "soak: 10,000 page renders; run in release with `--ignored soak`"]
fn soak_pages_10000_renders_flat_memory() {
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    run_soak("soak-pages", 5_000, 500, 1_000);
}
