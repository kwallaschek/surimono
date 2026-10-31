//! Where an island runner's memory goes, stage by stage. Ignored by default;
//! Linux only (it reads `/proc/self/smaps`). Run in release, alone:
//!
//! ```text
//! cargo test --release -p surimono-islands --test memory_report -- --ignored --nocapture
//! ```
//!
//! It renders a document's islands one after another, as the facade does
//! (`SURIMONO_MEM_HTML`, default: every visual fixture), twice, on a pool of
//! `SURIMONO_MEM_POOL` workers (default 4, the default on a 4-CPU machine),
//! and prints after each stage:
//!
//! - **PSS** and **RSS** of the process (`/proc/self/smaps_rollup`), and the
//!   stage's peak RSS (`VmHWM`, reset before each stage);
//! - **live heap**: bytes allocated and not freed, counted by a global
//!   allocator wrapper. Every JavaScript heap goes through it too;
//! - resident memory by mapping: the main malloc heap (`brk`), malloc's
//!   per-thread arenas, thread stacks, other anonymous mappings (large
//!   allocations malloc maps on their own) and file-backed pages (the
//!   binary, with the bundles' source and bytecode);
//! - **allocator slack**: anonymous resident memory that is neither live
//!   heap nor stack (freed memory malloc kept, fragmentation, headers).

use std::alloc::System;
use std::time::{Duration, Instant};

use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};
use surimono_islands::{IslandRequest, IslandRunner, RunnerOptions};

mod common;

use common::Island;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const KIB: u64 = 1024;
/// glibc's per-thread arena heaps are reserved in aligned 64 MiB blocks.
const ARENA_ALIGN: u64 = 64 * 1024 * KIB;

/// Resident memory of the process, in KiB, by kind.
#[derive(Debug, Default, Clone, Copy)]
struct Mem {
    rss: u64,
    pss: u64,
    anon: u64,
    brk: u64,
    arenas: u64,
    stacks: u64,
    other_anon: u64,
    file: u64,
    /// Live heap bytes / 1024.
    live: u64,
    /// Peak RSS since the last reset, KiB.
    hwm: u64,
}

struct Mapping {
    start: u64,
    end: u64,
    perms: String,
    path: String,
    rss: u64,
    anon: u64,
}

fn field_kib(line: &str, name: &str) -> Option<u64> {
    let rest = line.strip_prefix(name)?;
    rest.trim().trim_end_matches("kB").trim().parse().ok()
}

fn mappings() -> Vec<Mapping> {
    let smaps = std::fs::read_to_string("/proc/self/smaps").expect("/proc/self/smaps");
    let mut out: Vec<Mapping> = Vec::new();
    for line in smaps.lines() {
        let first = line.split_whitespace().next().unwrap_or("");
        if let Some((a, b)) = first.split_once('-') {
            if let (Ok(start), Ok(end)) = (u64::from_str_radix(a, 16), u64::from_str_radix(b, 16)) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                out.push(Mapping {
                    start,
                    end,
                    perms: parts.get(1).unwrap_or(&"").to_string(),
                    path: parts.get(5..).map(|p| p.join(" ")).unwrap_or_default(),
                    rss: 0,
                    anon: 0,
                });
                continue;
            }
        }
        if let Some(m) = out.last_mut() {
            if let Some(v) = field_kib(line, "Rss:") {
                m.rss = v;
            } else if let Some(v) = field_kib(line, "Anonymous:") {
                m.anon = v;
            }
        }
    }
    out
}

fn snapshot() -> Mem {
    let mut mem = Mem::default();
    let rollup = std::fs::read_to_string("/proc/self/smaps_rollup").unwrap_or_default();
    for line in rollup.lines() {
        if let Some(v) = field_kib(line, "Rss:") {
            mem.rss = v;
        } else if let Some(v) = field_kib(line, "Pss:") {
            mem.pss = v;
        } else if let Some(v) = field_kib(line, "Anonymous:") {
            mem.anon = v;
        }
    }
    let maps = mappings();
    for (i, m) in maps.iter().enumerate() {
        let anon_mapping = m.path.is_empty() || m.path.starts_with("[anon");
        if m.path == "[heap]" {
            mem.brk += m.rss;
        } else if m.path == "[stack]" {
            mem.stacks += m.rss;
        } else if m.path.starts_with('/') {
            mem.file += m.rss;
        } else if anon_mapping && m.perms.starts_with("rw") {
            // A thread stack sits right above its guard page; an arena heap
            // starts on a 64 MiB boundary.
            let guarded = i > 0 && {
                let prev = &maps[i - 1];
                prev.end == m.start && prev.perms.starts_with("---")
            };
            if m.start % ARENA_ALIGN == 0 {
                mem.arenas += m.rss;
            } else if guarded {
                mem.stacks += m.rss;
            } else {
                mem.other_anon += m.rss;
            }
        } else {
            mem.other_anon += m.anon;
        }
    }
    let s = GLOBAL.stats();
    mem.live = (s.bytes_allocated as i64 - s.bytes_deallocated as i64).max(0) as u64 / KIB;
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    mem.hwm = status
        .lines()
        .find_map(|l| field_kib(l, "VmHWM:"))
        .unwrap_or(0);
    mem
}

/// Resets the peak RSS (`VmHWM`) to the current RSS.
fn reset_peak() {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

fn mib(kib: u64) -> String {
    format!("{:.1}", kib as f64 / 1024.0)
}

fn print_header() {
    eprintln!(
        "| stage | PSS | RSS | peak RSS | live heap | brk | arenas | stacks | other anon | file | slack |"
    );
    eprintln!("|---|---|---|---|---|---|---|---|---|---|---|");
}

fn print_row(stage: &str, m: &Mem) {
    let slack = m.anon.saturating_sub(m.live + m.stacks);
    eprintln!(
        "| {stage} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
        mib(m.pss),
        mib(m.rss),
        mib(m.hwm),
        mib(m.live),
        mib(m.brk),
        mib(m.arenas),
        mib(m.stacks),
        mib(m.other_anon),
        mib(m.file),
        mib(slack),
    );
}

/// Waits until every started worker is idle (reset and preloaded).
fn wait_until_idle(runner: &IslandRunner) {
    let start = Instant::now();
    loop {
        let s = runner.stats();
        if s.idle_workers >= workers(runner) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "workers never went idle: {s:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    // Let a worker that just went idle finish trimming.
    std::thread::sleep(Duration::from_millis(50));
}

fn workers(runner: &IslandRunner) -> usize {
    runner.stats().workers
}

fn islands() -> (String, Vec<Island>) {
    match std::env::var("SURIMONO_MEM_HTML") {
        Ok(path) => {
            let html = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let islands = common::islands_in(&path, &html);
            assert!(!islands.is_empty(), "no islands in {path}");
            (path, islands)
        }
        Err(_) => ("the visual fixtures".to_owned(), common::fixture_islands()),
    }
}

#[test]
#[ignore = "memory report; Linux, run explicitly in release mode"]
fn memory_report() {
    let pool: usize = std::env::var("SURIMONO_MEM_POOL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let (what, islands) = islands();
    eprintln!(
        "memory report: {} islands from {what}, pool_size {pool}, MALLOC_ARENA_MAX={:?}",
        islands.len(),
        std::env::var("MALLOC_ARENA_MAX").ok()
    );
    print_header();
    reset_peak();
    print_row("start", &snapshot());

    reset_peak();
    let measurer = common::fixture_measurer();
    print_row("fonts loaded", &snapshot());

    reset_peak();
    let runner = IslandRunner::new(RunnerOptions {
        pool_size: pool,
        text_measurer: Some(measurer),
        ..RunnerOptions::default()
    })
    .unwrap();
    wait_until_idle(&runner);
    print_row("pool started", &snapshot());

    let render = |i: &Island| {
        runner
            .render(&IslandRequest {
                renderer: &i.renderer,
                spec_json: &i.spec,
                width: i.width,
                height: i.height,
            })
            .unwrap_or_else(|e| panic!("{} ({}): {e}", i.source, i.renderer))
    };
    let mut first_pass = Duration::ZERO;
    for (n, i) in islands.iter().enumerate() {
        reset_peak();
        let t = Instant::now();
        render(i);
        first_pass += t.elapsed();
        let busy = snapshot();
        wait_until_idle(&runner);
        let idle = snapshot();
        print_row(
            &format!("island {} {} (busy)", n + 1, i.renderer),
            &Mem {
                hwm: busy.hwm,
                ..busy
            },
        );
        print_row(&format!("island {} {} (idle)", n + 1, i.renderer), &idle);
    }
    reset_peak();
    let t = Instant::now();
    for i in &islands {
        render(i);
    }
    let second_pass = t.elapsed();
    wait_until_idle(&runner);
    print_row("second pass (idle)", &snapshot());
    eprintln!("{:?}", runner.stats());
    eprintln!("first pass {first_pass:.1?} (excl. idle waits), second pass {second_pass:.1?}");
    drop(runner);
    print_row("runner dropped", &snapshot());
}
