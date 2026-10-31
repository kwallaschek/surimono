//! Render times for the islands in the visual fixtures, for the checkpoint
//! notes. Ignored by default; run in release:
//!
//! ```text
//! cargo test --release -p surimono-islands --test fixture_timing -- --ignored --nocapture timing_fixture
//! cargo test --release -p surimono-islands --test fixture_timing -- --ignored --nocapture timing_cold
//! ```
//!
//! `timing_cold_processes` starts this binary again `SURIMONO_COLD_RUNS`
//! times (default 10), one process after another; each child loads the
//! fixture fonts, starts a runner and renders every fixture island once,
//! as a new process rendering one document does. It reports the median
//! and range of the children's wall time (spawn to exit) and of the time
//! they measured themselves (runner start to last island).
//!
//! Reads every `*/input.html` under `SURIMONO_FIXTURES_DIR` (default: the
//! repository's `fixtures/islands`), takes its island markers
//! (`<script type="application/vnd.surimono+json" data-renderer=… data-width=…
//! data-height=…>`), and reports, per island, the median of a cold render
//! (the request loads the bundle) and a preloaded one (an idle worker
//! loaded it). Text is measured with the fixture fonts (Noto Sans and Noto
//! Sans JP), as in a real document.

use std::process::Command;
use std::time::{Duration, Instant};

use surimono_islands::{IslandRequest, IslandRunner, RunnerOptions};

mod common;

use common::Island;

#[test]
#[ignore = "timing report; run explicitly in release mode"]
fn timing_fixture_islands() {
    const RUNS: usize = 10;
    let islands = common::fixture_islands();

    let runner = IslandRunner::new(RunnerOptions {
        pool_size: 1,
        text_measurer: Some(common::fixture_measurer()),
        ..RunnerOptions::default()
    })
    .unwrap();
    runner
        .register_bundle("other", "function entry() { return '<svg/>' }", "entry")
        .unwrap();
    let render = |i: &Island| {
        runner.render(&IslandRequest {
            renderer: &i.renderer,
            spec_json: &i.spec,
            width: i.width,
            height: i.height,
        })
    };
    let pause = || std::thread::sleep(Duration::from_millis(200));
    let median = |mut v: Vec<Duration>| {
        v.sort();
        v[v.len() / 2]
    };
    // Compile every built-in once, outside the measurements.
    for i in &islands {
        if let Err(e) = render(i) {
            panic!("{} ({}): {e}", i.source, i.renderer);
        }
    }
    eprintln!("| fixture | renderer | size | cold | preloaded | SVG bytes |");
    eprintln!("|---|---|---|---|---|---|");
    let (mut total_cold, mut total_warm) = (Duration::ZERO, Duration::ZERO);
    for i in &islands {
        let (mut cold, mut warm) = (Vec::new(), Vec::new());
        let mut bytes = 0;
        for _ in 0..RUNS {
            runner
                .render(&IslandRequest {
                    renderer: "other",
                    spec_json: "{}",
                    width: 1,
                    height: 1,
                })
                .unwrap();
            pause();
            let t = Instant::now();
            bytes = render(i).unwrap().len();
            cold.push(t.elapsed());
            pause();
            let t = Instant::now();
            render(i).unwrap();
            warm.push(t.elapsed());
        }
        let (c, w) = (median(cold), median(warm));
        total_cold += c;
        total_warm += w;
        eprintln!(
            "| {} | {} | {}x{} | {:.1?} | {:.1?} | {bytes} |",
            i.source, i.renderer, i.width, i.height, c, w
        );
    }
    eprintln!(
        "| all {} islands | | | {:.1?} | {:.1?} | |",
        islands.len(),
        total_cold,
        total_warm
    );
}

/// Marks the child's timing line on stdout.
const COLD_PREFIX: &str = "COLD-MS ";

/// CPU time (user + system, all threads) this process has used, in ms;
/// 0 where `/proc` is missing. Less sensitive than wall time to other
/// processes on a busy machine.
fn cpu_ms() -> f64 {
    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else {
        return 0.0;
    };
    // Fields after the command name, which is in parentheses.
    let fields: Vec<&str> = stat
        .rsplit_once(')')
        .map_or("", |(_, rest)| rest)
        .split_whitespace()
        .collect();
    // utime and stime are fields 14 and 15 of the line, in clock ticks
    // (100 per second on Linux).
    let ticks = |i: usize| {
        fields
            .get(i)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    (ticks(11) + ticks(12)) * 10.0
}

/// Child side of `timing_cold_processes`: one new process, one runner,
/// every fixture island once. Does nothing unless started by it.
#[test]
#[ignore = "helper: run in a child process by timing_cold_processes"]
fn cold_child() {
    if std::env::var_os("SURIMONO_COLD_CHILD").is_none() {
        return;
    }
    let islands = common::fixture_islands();
    let measurer = common::fixture_measurer();
    let t = Instant::now();
    let runner = IslandRunner::new(RunnerOptions {
        text_measurer: Some(measurer),
        ..RunnerOptions::default()
    })
    .unwrap();
    let mut each = Vec::new();
    for i in &islands {
        let start = Instant::now();
        runner
            .render(&IslandRequest {
                renderer: &i.renderer,
                spec_json: &i.spec,
                width: i.width,
                height: i.height,
            })
            .unwrap_or_else(|e| panic!("{} ({}): {e}", i.source, i.renderer));
        each.push(format!("{:.3}", start.elapsed().as_secs_f64() * 1000.0));
    }
    println!(
        "\n{COLD_PREFIX}{:.3} {:.3} {}",
        t.elapsed().as_secs_f64() * 1000.0,
        cpu_ms(),
        each.join(" ")
    );
}

#[test]
#[ignore = "timing report; run explicitly in release mode"]
fn timing_cold_processes() {
    let runs: usize = std::env::var("SURIMONO_COLD_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let exe = std::env::current_exe().unwrap();
    let islands = common::fixture_islands().len();
    let (mut wall, mut inside, mut cpu) = (Vec::new(), Vec::new(), Vec::new());
    let mut per_island: Vec<Vec<f64>> = vec![Vec::new(); islands];
    for _ in 0..runs {
        let t = Instant::now();
        let out = Command::new(&exe)
            .args([
                "cold_child",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("SURIMONO_COLD_CHILD", "1")
            .output()
            .unwrap();
        wall.push(t.elapsed().as_secs_f64() * 1000.0);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        let numbers: Vec<f64> = stdout
            .lines()
            .find_map(|l| l.find(COLD_PREFIX).map(|i| &l[i + COLD_PREFIX.len()..]))
            .expect("the child reports its time")
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        inside.push(numbers[0]);
        cpu.push(numbers[1]);
        for (k, ms) in numbers[2..].iter().enumerate() {
            per_island[k].push(*ms);
        }
    }
    let names: Vec<String> = common::fixture_islands()
        .iter()
        .map(|i| format!("{} ({})", i.source, i.renderer))
        .collect();
    let summary = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        format!(
            "median {:.0} ms (min {:.0}, max {:.0})",
            v[v.len() / 2],
            v[0],
            v[v.len() - 1]
        )
    };
    eprintln!(
        "cold processes: {runs} runs, {islands} fixture islands each; wall {}; runner start to last island {}; process CPU time {}",
        summary(&mut wall),
        summary(&mut inside),
        summary(&mut cpu)
    );
    for (name, times) in names.iter().zip(per_island.iter_mut()) {
        eprintln!("  {name}: {}", summary(times));
    }
}
