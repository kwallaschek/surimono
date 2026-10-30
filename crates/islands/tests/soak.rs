//! The soak test: 10,000 island renders in one process, and memory must stay
//! flat after warm-up.
//!
//! Ignored by default; CI runs it in release mode with
//! `cargo test --release -- --ignored soak --nocapture` (scripts/ci/soak.sh).
//!
//! The pass/fail signal is **live heap bytes**, counted by a global
//! allocator wrapper (`stats_alloc`). Every JavaScript heap is allocated
//! through it too (rquickjs's allocator calls `std::alloc`), so a leak in
//! the runner, the sandbox or QuickJS shows up byte for byte. Resident
//! memory (RSS) is printed, with only a loose bound: it depends on the
//! system allocator's arenas and fragmentation, not just on our code.

use std::alloc::System;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};
use surimono_islands::{
    IslandError, IslandRequest, IslandRunner, Limits, RunnerOptions, SandboxError,
};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const RENDERS: usize = 10_000;
/// Renders per round; memory is sampled between rounds, with no render in
/// flight.
const SAMPLE_EVERY: usize = 500;
/// Live heap bytes are compared from this render on.
const WARM_UP: usize = 2_000;
/// Largest growth of live heap bytes allowed from `WARM_UP` to the end.
const MAX_LIVE_GROWTH: i64 = 64 * 1024;
/// Threads calling `render` at the same time.
const CALLERS: usize = 3;
/// Worker threads in the runner.
const POOL_SIZE: usize = 2;
/// Heap limit per render.
const MEMORY_BYTES: usize = 32 * 1024 * 1024;

/// A chart-like renderer: parses data, measures labels, builds an SVG of a
/// few hundred elements, and uses Math.random and Date. Some renders fail on
/// purpose so the error paths are soaked too.
const BUNDLE: &str = r#"
function entry(specJson, ctx) {
  const spec = JSON.parse(specJson);
  if (spec.fail === 'throw') throw new Error('planned failure ' + spec.i);
  if (spec.fail === 'oom') { const a = []; for (;;) a.push(new Array(1e4).fill(spec.i)); }
  if (spec.fail === 'loop') { for (;;) {} }
  const points = spec.values.map((v, i) => ({
    x: i * 4, y: 100 - v, label: 'p' + i + '-' + Math.random().toString(36).slice(2, 6),
  }));
  const parts = ['<svg xmlns="http://www.w3.org/2000/svg" width="' + ctx.width + '" height="' + ctx.height + '">'];
  for (const p of points) {
    const m = ctx.measureText(p.label, '10px sans-serif');
    parts.push('<circle cx="' + p.x + '" cy="' + p.y + '" r="2"/><text x="' + (p.x - m.width / 2) +
      '" y="' + (p.y - 4) + '">' + p.label + '</text>');
  }
  parts.push('<desc>' + new Date().toISOString() + ' ' + spec.i + '</desc></svg>');
  return parts.join('');
}
"#;

/// What render `i` is planned to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
    Ok,
    Throw,
    OutOfMemory,
    Timeout,
}

fn plan(i: usize) -> Plan {
    if i % 100 == 99 {
        Plan::OutOfMemory
    } else if i % 200 == 123 {
        Plan::Timeout
    } else if i % 50 == 49 {
        Plan::Throw
    } else {
        Plan::Ok
    }
}

fn spec(i: usize) -> String {
    let fail = match plan(i) {
        Plan::Ok => "",
        Plan::Throw => r#","fail":"throw""#,
        Plan::OutOfMemory => r#","fail":"oom""#,
        Plan::Timeout => r#","fail":"loop""#,
    };
    let values: Vec<String> = (0..200)
        .map(|k| ((i * 7 + k * 13) % 100).to_string())
        .collect();
    format!(r#"{{"i":{i},"values":[{}]{fail}}}"#, values.join(","))
}

/// Checks one render's result against its plan.
fn check(i: usize, result: Result<String, IslandError>) {
    match (plan(i), result) {
        (Plan::Ok, Ok(svg)) => assert!(svg.ends_with("</desc></svg>"), "render {i}"),
        (Plan::Throw, Err(IslandError::Sandbox(SandboxError::Exception { .. }))) => {}
        (Plan::OutOfMemory, Err(IslandError::Sandbox(SandboxError::MemoryLimit))) => {}
        (Plan::Timeout, Err(IslandError::Sandbox(SandboxError::TimeLimit))) => {}
        (p, other) => panic!("render {i} ({p:?}): unexpected {other:?}"),
    }
}

/// Bytes allocated through the global allocator and not yet freed.
fn live_bytes() -> i64 {
    let s = GLOBAL.stats();
    s.bytes_allocated as i64 - s.bytes_deallocated as i64
}

/// Resident set size of this process in KiB: /proc on Linux, `ps` elsewhere.
fn rss_kib() -> Option<u64> {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        return status
            .lines()
            .find_map(|l| l.strip_prefix("VmRSS:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok());
    }
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

struct Sample {
    renders: usize,
    live: i64,
    rss_kib: u64,
}

#[test]
#[ignore = "soak: 10,000 renders; run in release with `--ignored soak`"]
fn soak_10000_renders_flat_memory() {
    let runner = IslandRunner::new(RunnerOptions {
        pool_size: POOL_SIZE,
        limits: Limits {
            memory_bytes: MEMORY_BYTES,
            time: Duration::from_millis(250),
            ..Limits::default()
        },
        ..RunnerOptions::default()
    })
    .unwrap();
    runner.register_bundle("chart", BUNDLE, "entry").unwrap();

    let start_rss = rss_kib();
    let mut samples: Vec<Sample> = Vec::with_capacity(RENDERS / SAMPLE_EVERY + 1);
    let first_svg_len = AtomicUsize::new(0);
    let started = Instant::now();
    for round in 0..RENDERS / SAMPLE_EVERY {
        let base = round * SAMPLE_EVERY;
        // CALLERS threads share the round's renders, so the workers render
        // concurrently and callers queue.
        std::thread::scope(|s| {
            for caller in 0..CALLERS {
                let runner = &runner;
                let first_svg_len = &first_svg_len;
                s.spawn(move || {
                    for i in (base + caller..base + SAMPLE_EVERY).step_by(CALLERS) {
                        let spec = spec(i);
                        let result = runner.render(&IslandRequest {
                            renderer: "chart",
                            spec_json: &spec,
                            width: 800,
                            height: 120,
                        });
                        if i == 0 {
                            if let Ok(svg) = &result {
                                first_svg_len.store(svg.len(), Ordering::Relaxed);
                            }
                        }
                        check(i, result);
                    }
                });
            }
        });
        // Workers reset their runtime right after replying; let them finish.
        std::thread::sleep(Duration::from_millis(100));
        samples.push(Sample {
            renders: base + SAMPLE_EVERY,
            live: live_bytes(),
            rss_kib: rss_kib().unwrap_or(0),
        });
    }
    let elapsed = started.elapsed();

    eprintln!(
        "soak: {RENDERS} renders in {elapsed:?} ({:?} per render incl. {} pauses), {CALLERS} callers, {POOL_SIZE} workers, first SVG {} bytes",
        elapsed / RENDERS as u32,
        RENDERS / SAMPLE_EVERY,
        first_svg_len.load(Ordering::Relaxed)
    );
    eprintln!("soak: resident memory before start {start_rss:?} KiB");
    for s in &samples {
        eprintln!(
            "soak: after {:>5} renders  live heap {:>10} bytes  resident {:>8} KiB",
            s.renders, s.live, s.rss_kib
        );
    }
    let stats = runner.stats();
    eprintln!("soak: {stats:?}");
    let planned_failures = (0..RENDERS).filter(|&i| plan(i) != Plan::Ok).count();
    assert_eq!(stats.renders, RENDERS as u64);
    assert_eq!(stats.failed_renders, planned_failures as u64);

    // The signal: live heap bytes after warm-up.
    let at = |n: usize| samples.iter().find(|s| s.renders == n).unwrap();
    let warm = at(WARM_UP).live;
    let end = at(RENDERS).live;
    let max_after_warm_up = samples
        .iter()
        .filter(|s| s.renders >= WARM_UP)
        .map(|s| s.live)
        .max()
        .unwrap();
    eprintln!(
        "soak: live heap from render {WARM_UP} to {RENDERS}: {warm} -> {end} bytes ({:+} bytes, highest {max_after_warm_up}), limit +{MAX_LIVE_GROWTH}",
        end - warm
    );
    assert!(
        end - warm < MAX_LIVE_GROWTH && max_after_warm_up - warm < MAX_LIVE_GROWTH,
        "live heap grew from {warm} to {end} bytes (highest {max_after_warm_up}) after warm-up"
    );

    // A loose bound on resident memory, from the settings: every worker's
    // heap at its limit, twice for allocator slack, plus 64 MiB.
    if let Some(start) = start_rss {
        let bound = start + (2 * POOL_SIZE * MEMORY_BYTES) as u64 / 1024 + 64 * 1024;
        let peak = samples.iter().map(|s| s.rss_kib).max().unwrap();
        eprintln!("soak: peak resident {peak} KiB, bound {bound} KiB");
        assert!(
            peak < bound,
            "peak resident memory {peak} KiB is above {bound} KiB"
        );
    }
    assert!(elapsed < Duration::from_secs(600));
}
