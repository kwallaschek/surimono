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

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use surimono_islands::{IslandRequest, IslandRunner, RunnerOptions};

mod common;

struct Island {
    fixture: String,
    renderer: String,
    width: u32,
    height: u32,
    spec: String,
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let start = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let len = tag[start..].find('"')?;
    Some(tag[start..start + len].to_owned())
}

fn islands_in(fixture: &str, html: &str) -> Vec<Island> {
    const MARKER: &str = "type=\"application/vnd.surimono+json\"";
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(open) = rest.find("<script") {
        let after = &rest[open..];
        let Some(tag_end) = after.find('>') else {
            break;
        };
        let tag = &after[..tag_end];
        let body_start = tag_end + 1;
        let Some(close) = after[body_start..].find("</script>") else {
            break;
        };
        if tag.contains(MARKER) {
            if let (Some(renderer), Some(w), Some(h)) = (
                attribute(tag, "data-renderer"),
                attribute(tag, "data-width"),
                attribute(tag, "data-height"),
            ) {
                out.push(Island {
                    fixture: fixture.to_owned(),
                    renderer,
                    width: w.parse().unwrap_or(0),
                    height: h.parse().unwrap_or(0),
                    spec: after[body_start..body_start + close].trim().to_owned(),
                });
            }
        }
        rest = &after[body_start + close..];
    }
    out
}

#[test]
#[ignore = "timing report; run explicitly in release mode"]
fn timing_fixture_islands() {
    const RUNS: usize = 10;
    let dir = std::env::var_os("SURIMONO_FIXTURES_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/islands"),
        PathBuf::from,
    );
    let mut fixtures: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("input.html").exists())
        .collect();
    fixtures.sort();
    let mut islands = Vec::new();
    for f in &fixtures {
        let html = std::fs::read_to_string(f.join("input.html")).unwrap();
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        islands.extend(islands_in(&name, &html));
    }
    assert!(!islands.is_empty(), "no islands under {}", dir.display());

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
            panic!("{} ({}): {e}", i.fixture, i.renderer);
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
            i.fixture, i.renderer, i.width, i.height, c, w
        );
    }
    eprintln!(
        "| all {} islands | | | {:.1?} | {:.1?} | |",
        islands.len(),
        total_cold,
        total_warm
    );
}
