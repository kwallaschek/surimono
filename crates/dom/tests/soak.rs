//! Leak test for id handles: many renders in one process, each parsing a
//! document, creating and dropping thousands of wrappers and nodes, and
//! memory must stay flat after warm-up.
//!
//! Same method as `crates/islands/tests/soak.rs`: the pass/fail signal is
//! live heap bytes, counted by a global allocator wrapper (`stats_alloc`);
//! QuickJS allocates through `std::alloc` too, so a leak in the bindings,
//! blitz-dom or the JavaScript heap shows up byte for byte. Resident memory
//! is printed only.
//!
//! The 2,000-render soak is `#[ignore]`d and runs in release with the other
//! soaks (`scripts/ci/soak.sh`: `--ignored soak`); a 300-render version
//! runs with the ordinary tests.

use std::alloc::System;

use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};
use surimono_core::Limits;
use surimono_dom::{Document, DomError, DomRuntime};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

/// Largest growth of live heap bytes allowed from the end of warm-up to the
/// end, as in the islands soak.
const MAX_LIVE_GROWTH: i64 = 64 * 1024;

/// A page per render, different each time so blitz-dom's atoms and id map
/// see new strings.
fn page(i: usize) -> String {
    let mut rows = String::new();
    for r in 0..40 {
        rows.push_str(&format!(
            r#"<tr id="r{r}"><td class="k">k{i}-{r}</td><td>{}</td></tr>"#,
            (i * 31 + r * 7) % 1000
        ));
    }
    format!(
        r#"<!doctype html><html><head><title>p{i}</title></head><body>
<h1 id="title">Report {i}</h1><table id="t">{rows}</table><div id="out"></div></body></html>"#
    )
}

/// Wraps every row repeatedly, reads through wrappers, builds a list of new
/// nodes and drops it again, and replaces the table's content (dropping
/// nodes that wrappers still reference). Every 50th render also uses a
/// stale wrapper on purpose, so that error path is soaked too.
const SCRIPT: &str = r#"
(function (i) {
  let sum = 0;
  for (let pass = 0; pass < 5; pass++) {
    for (let r = 0; r < 40; r++) {
      const row = document.getElementById('r' + r);
      sum += row.textContent.length + row.parentNode.nodeName.length;
    }
  }
  const out = document.getElementById('out');
  for (let round = 0; round < 3; round++) {
    const ul = document.createElement('ul');
    for (let k = 0; k < 100; k++) {
      const li = document.createElement('li');
      li.appendChild(document.createTextNode('item ' + k + ' of ' + i));
      ul.appendChild(li);
    }
    out.appendChild(ul);
    sum += out.textContent.length;
    out.textContent = '';
  }
  const kept = document.getElementById('r3');
  document.getElementById('t').textContent = 'cleared ' + i;
  if (i % 50 === 49) { try { kept.textContent; } catch (e) {} }
  document.getElementById('title').textContent = 'sum ' + sum;
  return document.body.textContent.length;
})
"#;

fn live_bytes() -> i64 {
    let s = GLOBAL.stats();
    s.bytes_allocated as i64 - s.bytes_deallocated as i64
}

fn rss_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
}

/// One render as production will run it: a fresh document, the sandbox
/// reset (fresh JS state), the script, the document taken back and dropped.
fn render(rt: &mut DomRuntime, i: usize) {
    rt.reset().unwrap();
    rt.load(Document::parse(&page(i)).unwrap());
    let r = rt.eval::<i32>(&format!("({SCRIPT})({i})"));
    let doc = rt.take().unwrap();
    if i % 50 == 49 {
        assert_eq!(r, Err(DomError::NodeRemoved), "render {i}");
    } else {
        let len = r.unwrap_or_else(|e| panic!("render {i}: {e:?}"));
        assert!(len > 0, "render {i}");
        let html = doc.to_html();
        assert!(html.contains(&format!("cleared {i}")), "render {i}");
        assert!(!html.contains("<li>"), "render {i}");
    }
    drop(doc);
}

fn run_soak(name: &str, renders: usize, sample_every: usize, warm_up: usize) {
    let mut rt = DomRuntime::new(Limits::default()).unwrap();
    let start_rss = rss_kib();
    let started = std::time::Instant::now();
    let mut samples = Vec::new();
    for i in 0..renders {
        render(&mut rt, i);
        if (i + 1) % sample_every == 0 {
            samples.push((i + 1, live_bytes(), rss_kib().unwrap_or(0)));
        }
    }
    let elapsed = started.elapsed();
    eprintln!(
        "{name}: {renders} renders in {elapsed:?} ({:?} per render)",
        elapsed / renders as u32
    );
    eprintln!("{name}: resident memory before start {start_rss:?} KiB");
    for (n, live, rss) in &samples {
        eprintln!(
            "{name}: after {n:>5} renders  live heap {live:>10} bytes  resident {rss:>8} KiB"
        );
    }
    let at = |n: usize| samples.iter().find(|s| s.0 == n).unwrap().1;
    let warm = at(warm_up);
    let end = at(renders);
    let highest = samples
        .iter()
        .filter(|s| s.0 >= warm_up)
        .map(|s| s.1)
        .max()
        .unwrap();
    eprintln!(
        "{name}: live heap from render {warm_up} to {renders}: {warm} -> {end} bytes ({:+} bytes, highest {highest}), limit +{MAX_LIVE_GROWTH}",
        end - warm
    );
    assert!(
        end - warm < MAX_LIVE_GROWTH && highest - warm < MAX_LIVE_GROWTH,
        "{name}: live heap grew from {warm} to {end} bytes (highest {highest}) after warm-up"
    );
}

/// The two tests share the process-wide byte counter; run them one at a time.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn handles_300_renders_flat_memory() {
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    run_soak("dom-handles-short", 300, 50, 100);
}

#[test]
#[ignore = "soak: 2,000 DOM renders; run in release with `--ignored soak`"]
fn soak_dom_handles_2000_renders_flat_memory() {
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    run_soak("soak-dom-handles", 2_000, 250, 500);
}
