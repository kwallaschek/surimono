//! E3-R1 stage C: memory over many renders that each lay out with Raikiri,
//! as `soak.rs` does for the DOM. Each render parses a report page, runs a
//! script that measures every row through `getBoundingClientRect` and
//! `offsetTop`, changes the page and measures again (two layouts), and drops
//! the document. The pass/fail signal is live heap bytes from a counting
//! allocator, after warm-up; resident memory is printed. Run with
//! `--features raikiri`; the 10,000-render soak in release with
//! `--ignored soak`.
#![cfg(feature = "raikiri")]

mod common;

use std::alloc::System;

use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};
use surimono_dom::{Document, DomRuntime, LayoutEngine, LayoutPolicy, LayoutSetup, RaikiriSetup};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

/// Largest growth of live heap bytes allowed from the end of warm-up to the
/// end, as in the DOM and islands soaks.
const MAX_LIVE_GROWTH: i64 = 64 * 1024;

/// The page thread's stack in the facade, as in `layout_real.rs`.
const STACK: usize = 8 * 1024 * 1024 + 512 * 1024;

fn font(name: &str) -> Vec<u8> {
    let path = format!("{}/../../fixtures/fonts/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// A report page of 20 flex rows (e3-00's 107-node page), different each
/// render so names and text are new.
fn page(i: usize) -> String {
    let mut html = String::from(
        "<!doctype html><html><head><style>body { margin: 0; font: 12px 'Noto Sans' }\
         .row { display: flex; padding: 2px 4px; border-bottom: 1px solid #ccc }\
         .l { flex: 1 } .v { width: 80px; text-align: right; font-weight: bold }</style></head><body>",
    );
    for r in 0..20 {
        html.push_str(&format!(
            "<div class=row id=r{r}><span class=l>Item {i}-{r}</span>\
             <span class=v style=\"color:#333\">{},{:03}</span></div>",
            (i + r) % 97,
            (i * 37 + r) % 1000
        ));
    }
    html
}

/// Every row's box, a change that makes the layout stale, every row again.
const SCRIPT: &str = "(function (i) {
  let sum = 0;
  for (let pass = 0; pass < 2; pass++) {
    for (let r = 0; r < 20; r++) {
      const row = document.getElementById('r' + r);
      sum += row.getBoundingClientRect().bottom + row.offsetTop + row.offsetWidth;
    }
    document.getElementById('r0').style.height = (30 + i % 7) + 'px';
  }
  return sum;
})";

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

fn run_soak(name: &'static str, renders: usize, sample_every: usize, warm_up: usize) {
    // On a thread of the page thread's size, as the facade lays out.
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || soak_on_this_thread(name, renders, sample_every, warm_up))
        .unwrap()
        .join()
        .unwrap();
}

fn soak_on_this_thread(name: &str, renders: usize, sample_every: usize, warm_up: usize) {
    let mut rt = DomRuntime::new(common::limits()).unwrap();
    rt.set_layout_policy(LayoutPolicy::Real);
    rt.set_layout_setup(LayoutSetup {
        width: 800,
        height: 600,
        engine: LayoutEngine::Raikiri(
            RaikiriSetup::new([
                ("Noto Sans", font("NotoSans-Regular.ttf")),
                ("Noto Sans", font("NotoSans-Bold.ttf")),
            ])
            .unwrap(),
        ),
    });
    let started = std::time::Instant::now();
    let mut samples = Vec::new();
    let mut layouts = 0;
    for i in 0..renders {
        rt.reset().unwrap();
        rt.load(Document::parse(&page(i)).unwrap());
        let sum: f64 = rt
            .eval(&format!("({SCRIPT})({i})"))
            .unwrap_or_else(|e| panic!("render {i}: {e:?}"));
        assert!(sum > 0.0, "render {i}: nothing measured");
        layouts += rt.layouts_run();
        drop(rt.take());
        if (i + 1) % sample_every == 0 {
            samples.push((i + 1, live_bytes(), rss_kib()));
        }
    }
    let elapsed = started.elapsed();
    eprintln!(
        "{name}: {renders} renders, {layouts} layouts in {elapsed:?} ({:?} per render)",
        elapsed / renders as u32
    );
    for (n, live, rss) in &samples {
        eprintln!(
            "{name}: after {n:>6} renders  live heap {live:>10} bytes  resident {rss:>8} KiB"
        );
    }
    assert_eq!(layouts, 2 * renders as u64, "two layouts a render");
    let at = |n: usize| samples.iter().find(|s| s.0 == n).unwrap().1;
    let (warm, end) = (at(warm_up), at(renders));
    let highest = samples
        .iter()
        .filter(|s| s.0 >= warm_up)
        .map(|s| s.1)
        .max()
        .unwrap();
    eprintln!(
        "{name}: live heap from render {warm_up} to {renders}: {warm} -> {end} bytes ({:+}, highest {highest}), limit +{MAX_LIVE_GROWTH}",
        end - warm
    );
    assert!(
        end - warm < MAX_LIVE_GROWTH && highest - warm < MAX_LIVE_GROWTH,
        "{name}: live heap grew from {warm} to {end} bytes (highest {highest}) after warm-up"
    );
}

/// The two tests share the process-wide byte counter; one at a time.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn raikiri_layouts_300_renders_flat_memory() {
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    run_soak("raikiri-layout-short", 300, 50, 100);
}

#[test]
#[ignore = "soak: 10,000 renders with two Raikiri layouts each; run in release with `--ignored soak`"]
fn soak_raikiri_layouts_10000_renders_flat_memory() {
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    run_soak("soak-raikiri-layout", 10_000, 1_000, 2_000);
}
