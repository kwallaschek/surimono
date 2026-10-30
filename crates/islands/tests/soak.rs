//! The soak test: 10,000 island renders in one process, and resident memory
//! must stay flat after warm-up.
//!
//! Ignored by default; CI runs it in release mode with
//! `cargo test --release -- --ignored soak --nocapture` (scripts/ci/soak.sh).

use std::time::{Duration, Instant};

use surimono_islands::{
    IslandError, IslandRequest, IslandRunner, Limits, RunnerOptions, SandboxError,
};

const RENDERS: usize = 10_000;
const SAMPLE_EVERY: usize = 500;
/// The last renders, over which resident memory must stay flat.
const TAIL: usize = 2_000;
/// Largest growth allowed over the tail.
const MAX_TAIL_GROWTH: f64 = 0.02;

/// A chart-like renderer: parses data, measures labels, builds an SVG of a
/// few hundred elements, and uses Math.random and Date. Some renders fail on
/// purpose so the error paths are soaked too.
const BUNDLE: &str = r#"
function entry(specJson, ctx) {
  const spec = JSON.parse(specJson);
  if (spec.fail === 'throw') throw new Error('planned failure ' + spec.i);
  if (spec.fail === 'oom') { const a = []; for (;;) a.push(new Array(1e4).fill(spec.i)); }
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

fn spec(i: usize) -> String {
    let fail = match i {
        _ if i % 100 == 99 => r#","fail":"oom""#,
        _ if i % 50 == 49 => r#","fail":"throw""#,
        _ => "",
    };
    let values: Vec<String> = (0..200)
        .map(|k| ((i * 7 + k * 13) % 100).to_string())
        .collect();
    format!(r#"{{"i":{i},"values":[{}]{fail}}}"#, values.join(","))
}

#[test]
#[ignore = "soak: 10,000 renders; run in release with `--ignored soak`"]
fn soak_10000_renders_flat_memory() {
    let Some(start_rss) = rss_kib() else {
        eprintln!("soak: can't read resident memory on this platform; skipping");
        return;
    };
    let runner = IslandRunner::new(RunnerOptions {
        pool_size: 2,
        limits: Limits {
            memory_bytes: 32 * 1024 * 1024,
            ..Limits::default()
        },
        ..RunnerOptions::default()
    })
    .unwrap();
    runner.register_bundle("chart", BUNDLE, "entry").unwrap();

    let started = Instant::now();
    let mut samples: Vec<(usize, u64)> = Vec::new();
    let mut first_svg_len = 0;
    for i in 0..RENDERS {
        let spec = spec(i);
        let result = runner.render(&IslandRequest {
            renderer: "chart",
            spec_json: &spec,
            width: 800,
            height: 120,
        });
        match (i % 100, i % 50, result) {
            (99, _, Err(IslandError::Sandbox(SandboxError::MemoryLimit))) => {}
            (_, 49, Err(IslandError::Sandbox(SandboxError::Exception { .. }))) => {}
            (_, _, Ok(svg)) if i % 50 != 49 => {
                assert!(svg.ends_with("</desc></svg>"));
                if i == 0 {
                    first_svg_len = svg.len();
                }
            }
            (_, _, other) => panic!("render {i}: unexpected {other:?}"),
        }
        if (i + 1) % SAMPLE_EVERY == 0 {
            samples.push((i + 1, rss_kib().unwrap_or(0)));
        }
    }
    let elapsed = started.elapsed();

    eprintln!(
        "soak: {RENDERS} renders in {elapsed:?} ({:?} per render), first SVG {first_svg_len} bytes",
        elapsed / RENDERS as u32
    );
    eprintln!("soak: resident memory before start {start_rss} KiB");
    for (n, rss) in &samples {
        eprintln!("soak: after {n:>5} renders  {rss:>8} KiB");
    }
    let stats = runner.stats();
    eprintln!("soak: {stats:?}");
    assert_eq!(stats.renders, RENDERS as u64);
    assert_eq!(stats.failed_renders, (RENDERS / 50) as u64);

    let rss_at = |n: usize| {
        samples
            .iter()
            .find(|(k, _)| *k == n)
            .map(|(_, r)| *r)
            .unwrap()
    };
    let tail_start = rss_at(RENDERS - TAIL);
    let tail_end = rss_at(RENDERS);
    let growth = (tail_end as f64 - tail_start as f64) / tail_start as f64;
    eprintln!(
        "soak: last {TAIL} renders: {tail_start} -> {tail_end} KiB ({:+.2}%), limit {:+.2}%",
        growth * 100.0,
        MAX_TAIL_GROWTH * 100.0
    );
    assert!(
        growth < MAX_TAIL_GROWTH,
        "resident memory grew {:.2}% over the last {TAIL} renders",
        growth * 100.0
    );
    // A sanity bound so a slow leak during warm-up can't hide either.
    let peak = samples.iter().map(|(_, r)| *r).max().unwrap();
    assert!(
        peak < start_rss + 512 * 1024,
        "peak resident memory {peak} KiB is far above the start ({start_rss} KiB)"
    );
    assert!(elapsed < Duration::from_secs(600));
}
