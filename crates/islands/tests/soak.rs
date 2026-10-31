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

mod common;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

/// Largest growth of live heap bytes allowed from the end of warm-up to the
/// end of a soak.
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
  // Reaches the heap limit in large steps, so it ends with the memory limit
  // however slow the machine is (filling the heap 80 KB at a time could hit
  // the time limit first on a busy machine). Alternates typed-array buffers
  // with ordinary arrays of 512K elements, so both allocation paths are
  // soaked.
  if (spec.fail === 'oom') {
    const a = [];
    if (spec.i % 200 === 99) { for (;;) a.push(new Uint8Array(4 * 1024 * 1024).fill(spec.i % 256)); }
    for (;;) a.push(new Array(512 * 1024).fill(spec.i));
  }
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

/// How a soak runs.
struct Soak<'a> {
    name: &'a str,
    runner: &'a IslandRunner,
    renderer: &'a str,
    /// The runner's heap limit per render.
    memory_bytes: usize,
    renders: usize,
    /// Renders per round; memory is sampled between rounds, once every
    /// worker is idle (reset and preloaded), with no render in flight.
    sample_every: usize,
    /// Live heap bytes are compared from this render on.
    warm_up: usize,
    spec: &'a (dyn Fn(usize) -> String + Sync),
    check: &'a (dyn Fn(usize, Result<String, IslandError>) + Sync),
}

/// Waits until every worker is idle: its runtime reset and preloaded, so
/// nothing allocates until the next render.
fn wait_until_idle(runner: &IslandRunner) {
    let start = Instant::now();
    while runner.stats().idle_workers < runner.stats().workers {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "workers never went idle: {:?}",
            runner.stats()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Soaks run one at a time: the live-byte counter is process-wide, and the
/// test harness runs tests in parallel.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs a soak with `CALLERS` threads and asserts that live heap bytes stay
/// flat after warm-up; prints the samples and resident memory.
fn run_soak(soak: &Soak<'_>) {
    let name = soak.name;
    let start_rss = rss_kib();
    let rounds = soak.renders / soak.sample_every;
    let mut samples: Vec<Sample> = Vec::with_capacity(rounds + 1);
    let first_svg_len = AtomicUsize::new(0);
    let started = Instant::now();
    for round in 0..rounds {
        let base = round * soak.sample_every;
        // CALLERS threads share the round's renders, so the workers render
        // concurrently and callers queue.
        std::thread::scope(|s| {
            for caller in 0..CALLERS {
                let first_svg_len = &first_svg_len;
                s.spawn(move || {
                    for i in (base + caller..base + soak.sample_every).step_by(CALLERS) {
                        let spec = (soak.spec)(i);
                        let result = soak.runner.render(&IslandRequest {
                            renderer: soak.renderer,
                            spec_json: &spec,
                            width: 800,
                            height: 400,
                        });
                        if i == 0 {
                            if let Ok(svg) = &result {
                                first_svg_len.store(svg.len(), Ordering::Relaxed);
                            }
                        }
                        (soak.check)(i, result);
                    }
                });
            }
        });
        wait_until_idle(soak.runner);
        samples.push(Sample {
            renders: base + soak.sample_every,
            live: live_bytes(),
            rss_kib: rss_kib().unwrap_or(0),
        });
    }
    let elapsed = started.elapsed();

    eprintln!(
        "{name}: {} renders in {elapsed:?} ({:?} per render incl. {rounds} waits for idle workers), {CALLERS} callers, {} workers, first SVG {} bytes",
        soak.renders,
        elapsed / soak.renders as u32,
        soak.runner.stats().workers,
        first_svg_len.load(Ordering::Relaxed)
    );
    eprintln!("{name}: resident memory before start {start_rss:?} KiB");
    for s in &samples {
        eprintln!(
            "{name}: after {:>5} renders  live heap {:>10} bytes  resident {:>8} KiB",
            s.renders, s.live, s.rss_kib
        );
    }
    eprintln!("{name}: {:?}", soak.runner.stats());

    // The signal: live heap bytes after warm-up.
    let at = |n: usize| samples.iter().find(|s| s.renders == n).unwrap();
    let warm = at(soak.warm_up).live;
    let end = at(soak.renders).live;
    let highest = samples
        .iter()
        .filter(|s| s.renders >= soak.warm_up)
        .map(|s| s.live)
        .max()
        .unwrap();
    eprintln!(
        "{name}: live heap from render {} to {}: {warm} -> {end} bytes ({:+} bytes, highest {highest}), limit +{MAX_LIVE_GROWTH}",
        soak.warm_up,
        soak.renders,
        end - warm
    );
    assert!(
        end - warm < MAX_LIVE_GROWTH && highest - warm < MAX_LIVE_GROWTH,
        "{name}: live heap grew from {warm} to {end} bytes (highest {highest}) after warm-up"
    );

    // A loose bound on resident memory, from the settings: every worker's
    // heap at its limit, twice for allocator slack, plus 64 MiB.
    if let Some(start) = start_rss {
        let bound =
            start + (2 * soak.runner.pool_size() * soak.memory_bytes) as u64 / 1024 + 64 * 1024;
        let peak = samples.iter().map(|s| s.rss_kib).max().unwrap();
        eprintln!("{name}: peak resident {peak} KiB, bound {bound} KiB");
        assert!(
            peak < bound,
            "{name}: peak resident memory {peak} KiB is above {bound} KiB"
        );
    }
}

#[test]
#[ignore = "soak: 10,000 renders; run in release with `--ignored soak`"]
fn soak_10000_renders_flat_memory() {
    const RENDERS: usize = 10_000;
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let runner = IslandRunner::new(RunnerOptions {
        // The measurer production uses: the fixture fonts, shaped by gfx.
        text_measurer: Some(common::fixture_measurer()),
        pool_size: POOL_SIZE,
        limits: Limits::default()
            .with_memory_bytes(MEMORY_BYTES)
            // Ordinary renders take a few ms; the margin keeps a busy
            // machine's scheduling stalls from failing them, while the
            // planned infinite loops still end quickly.
            .with_time(Duration::from_millis(1000)),
        ..RunnerOptions::default()
    })
    .unwrap();
    runner.register_bundle("chart", BUNDLE, "entry").unwrap();
    let started = Instant::now();
    run_soak(&Soak {
        name: "soak",
        runner: &runner,
        renderer: "chart",
        memory_bytes: MEMORY_BYTES,
        renders: RENDERS,
        sample_every: 500,
        warm_up: 2_000,
        spec: &spec,
        check: &check,
    });
    let stats = runner.stats();
    let planned_failures = (0..RENDERS).filter(|&i| plan(i) != Plan::Ok).count();
    assert_eq!(stats.renders, RENDERS as u64);
    assert_eq!(stats.failed_renders, planned_failures as u64);
    assert!(started.elapsed() < Duration::from_secs(600));
}

/// The product's allocation profile: real charts, rotating through the
/// ECharts test options, with an exception every 100th render.
#[cfg(feature = "echarts")]
#[test]
#[ignore = "soak: 2,000 ECharts renders; run in release with `--ignored soak`"]
fn soak_echarts_2000_renders_flat_memory() {
    const RENDERS: usize = 2_000;
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/echarts");
    let mut options: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    options.sort();
    assert!(options.len() >= 6);
    let runner = IslandRunner::new(RunnerOptions {
        // The measurer production uses: the fixture fonts, shaped by gfx.
        text_measurer: Some(common::fixture_measurer()),
        pool_size: POOL_SIZE,
        ..RunnerOptions::default()
    })
    .unwrap();
    let spec = |i: usize| {
        if i % 100 == 42 {
            // Not an option object: the renderer throws.
            "[]".to_owned()
        } else {
            options[i % options.len()].clone()
        }
    };
    let check = |i: usize, result: Result<String, IslandError>| match result {
        Err(IslandError::Sandbox(SandboxError::Exception { .. })) if i % 100 == 42 => {}
        Ok(svg) if i % 100 != 42 => assert!(svg.starts_with("<svg"), "render {i}"),
        other => panic!("render {i}: unexpected {other:?}"),
    };
    run_soak(&Soak {
        name: "soak-echarts",
        runner: &runner,
        renderer: "echarts",
        memory_bytes: Limits::DEFAULT_MEMORY_BYTES,
        renders: RENDERS,
        sample_every: 250,
        warm_up: 500,
        spec: &spec,
        check: &check,
    });
    let stats = runner.stats();
    assert_eq!(stats.renders, RENDERS as u64);
    assert_eq!(stats.failed_renders, (RENDERS / 100) as u64);
}

/// Vega-Lite (which runs Vega too), rotating through the small test specs,
/// with an exception every 100th render.
#[cfg(feature = "vega")]
#[test]
#[ignore = "soak: 600 Vega-Lite renders; run in release with `--ignored soak`"]
fn soak_vega_lite_600_renders_flat_memory() {
    const RENDERS: usize = 600;
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vega-lite");
    let specs: Vec<String> = ["bar", "japanese", "line-temporal"]
        .iter()
        .map(|n| std::fs::read_to_string(dir.join(format!("{n}.json"))).unwrap())
        .collect();
    let runner = IslandRunner::new(RunnerOptions {
        // The measurer production uses: the fixture fonts, shaped by gfx.
        text_measurer: Some(common::fixture_measurer()),
        pool_size: POOL_SIZE,
        ..RunnerOptions::default()
    })
    .unwrap();
    let spec = |i: usize| {
        if i % 100 == 42 {
            "[]".to_owned()
        } else {
            specs[i % specs.len()].clone()
        }
    };
    let check = |i: usize, result: Result<String, IslandError>| match result {
        Err(IslandError::Sandbox(SandboxError::Exception { .. })) if i % 100 == 42 => {}
        Ok(svg) if i % 100 != 42 => assert!(svg.starts_with("<svg"), "render {i}"),
        other => panic!("render {i}: unexpected {other:?}"),
    };
    run_soak(&Soak {
        name: "soak-vega-lite",
        runner: &runner,
        renderer: "vega-lite",
        memory_bytes: Limits::DEFAULT_MEMORY_BYTES,
        renders: RENDERS,
        sample_every: 150,
        warm_up: 300,
        spec: &spec,
        check: &check,
    });
    let stats = runner.stats();
    assert_eq!(stats.renders, RENDERS as u64);
    assert_eq!(stats.failed_renders, (RENDERS / 100) as u64);
}
