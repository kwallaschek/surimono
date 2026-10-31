//! Cold-start time: a fresh process's first render of each built-in
//! (P1-RT-17). Ignored by default; run in release:
//!
//! ```text
//! cargo test --release -p surimono-islands --test cold_start -- --ignored --nocapture
//! ```
//!
//! Each sample is a new child process (this test binary again) that builds a
//! runner and renders one island, timing that first render (which loads
//! the bundle) and runner creation. Build with `SURIMONO_PRECOMPILE=0` to
//! measure the same code compiling the bundle source at run time.

use std::process::Command;
use std::time::{Duration, Instant};

use surimono_islands::{IslandRequest, IslandRunner, Limits, RunnerOptions};

mod common;

const CHILD: &str = "SURIMONO_COLD_START_CHILD";

const CASES: &[(&str, &str)] = &[
    ("echarts", include_str!("echarts/bar.json")),
    ("vega", include_str!("vega/bar.json")),
    ("vega-lite", include_str!("vega-lite/bar.json")),
];

#[test]
#[ignore = "timing report; run explicitly in release mode"]
fn cold_start_report() {
    if let Ok(renderer) = std::env::var(CHILD) {
        child(&renderer);
        return;
    }
    const RUNS: usize = 7;
    let exe = std::env::current_exe().unwrap();
    eprintln!("| renderer | first render, median of {RUNS} (min-max) | runner creation |");
    eprintln!("|---|---|---|");
    for (renderer, _) in CASES {
        let mut first = Vec::new();
        let mut create = Vec::new();
        for _ in 0..RUNS {
            let out = Command::new(&exe)
                .args(["--ignored", "--exact", "cold_start_report", "--nocapture"])
                .env(CHILD, renderer)
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&out.stderr);
            let line = text
                .lines()
                .find_map(|l| l.strip_prefix("COLD "))
                .unwrap_or_else(|| panic!("child failed: {text}"));
            let mut it = line.split(' ').map(|v| v.parse::<f64>().unwrap());
            create.push(it.next().unwrap());
            first.push(it.next().unwrap());
        }
        first.sort_by(f64::total_cmp);
        create.sort_by(f64::total_cmp);
        eprintln!(
            "| {renderer} | {:.0} ms ({:.0}-{:.0}) | {:.0} ms |",
            first[RUNS / 2],
            first[0],
            first[RUNS - 1],
            create[RUNS / 2]
        );
    }
}

fn child(renderer: &str) {
    let spec = CASES.iter().find(|(r, _)| *r == renderer).unwrap().1;
    let measurer = common::fixture_measurer();
    let t0 = Instant::now();
    let runner = IslandRunner::new(RunnerOptions {
        pool_size: 1,
        limits: Limits::default().with_time(Duration::from_secs(30)),
        text_measurer: Some(measurer),
        ..RunnerOptions::default()
    })
    .unwrap();
    let t1 = Instant::now();
    let svg = runner
        .render(&IslandRequest {
            renderer,
            spec_json: spec,
            width: 600,
            height: 400,
        })
        .unwrap();
    let t2 = Instant::now();
    assert!(svg.contains("<svg"));
    eprintln!(
        "COLD {} {}",
        (t1 - t0).as_secs_f64() * 1e3,
        (t2 - t1).as_secs_f64() * 1e3
    );
}
