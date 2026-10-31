//! Render times for the islands in the visual fixtures, for the checkpoint
//! notes. Ignored by default; run in release:
//!
//! ```text
//! cargo test --release -p surimono-islands --test fixture_timing -- --ignored --nocapture
//! ```
//!
//! Reads every `*/input.html` under `SURIMONO_FIXTURES_DIR` (default: the
//! repository's `fixtures/islands`), takes its island markers
//! (`<script type="application/vnd.surimono+json" data-renderer=… data-width=…
//! data-height=…>`), and reports, per island, the median of a cold render
//! (the request loads the bundle) and a preloaded one (an idle worker
//! loaded it). Text is measured with the fixture fonts (Noto Sans and Noto
//! Sans JP), as in a real document.

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
