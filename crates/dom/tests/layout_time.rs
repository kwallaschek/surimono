//! E3-01e's time bound on one Raikiri layout: a layout query ends within
//! the page's time limit, with the typed time-limit error, even when the
//! layout it asked for runs longer; the layout it gave up on ends by itself
//! and leaves nothing behind; and a process that meets such pages again and
//! again, from one runtime or from several at once, never has more than
//! `MAX_LAYOUT_THREADS` (4) layout threads alive, running or given up on. A
//! test binary of its own: the count of layout threads is process-wide, and
//! a layout waits for a slot once all four are taken.
//!
//! The slow page is an ordinary one: a long text wrapped a word a line, which
//! takes Raikiri a while to lay out and the page's copy little time to make.
#![cfg(feature = "raikiri")]

mod common;

use std::alloc::System;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};
use surimono_core::SandboxError;
use surimono_dom::{
    Document, DomError, DomRuntime, LayoutEngine, LayoutPolicy, LayoutSetup, RaikiriSetup,
    layout_threads,
};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

/// The page thread's stack in the facade.
const STACK: usize = 8 * 1024 * 1024 + 512 * 1024;

/// Most layout threads, running or given up on, alive at once
/// (`MAX_LAYOUT_THREADS` in `src/layout_raikiri.rs`).
const MAX_LAYOUT_THREADS: usize = 4;

/// How much later than its time limit a call that waited on a layout may
/// end: the 1 ms the call polls at, and the sandbox's own end of the call.
fn margin() -> Duration {
    common::scaled(Duration::from_millis(100))
}

fn font(name: &str) -> Vec<u8> {
    let path = format!("{}/../../fixtures/fonts/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn runtime(limit: Duration) -> DomRuntime {
    let mut rt = DomRuntime::new(common::limits().with_time(limit)).unwrap();
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
    rt
}

const HEAD: &str = "<!doctype html><html><head><style>body { margin: 0; font: 12px 'Noto Sans' }</style></head><body>";

/// A text of `words` words, one a line in a 1 px column.
fn slow_page(words: usize) -> String {
    format!(
        "{HEAD}<p style=\"width:1px\">{}</p><div id=e style=\"width:5px;height:5px\"></div></body></html>",
        "a ".repeat(words)
    )
}

/// Words in the slow page: about 0.75 s of layout in a release build on the
/// fleet's machine, ten times that in a debug build. Twice as many are still
/// within the text a layout takes.
const SLOW_WORDS: usize = 60_000;

/// The time limit the slow page's calls run with, before scaling: short of
/// the slow page's layout by four times, and long enough for the glue and an
/// ordinary page with two of these test binaries on two cores at once.
const LIMIT_MS: u64 = 150;

/// Renders of the slow page in a row in the second test.
const HOSTILE_RENDERS: usize = 12;

fn ordinary_page() -> String {
    format!("{HEAD}<div id=e style=\"width:5px;height:5px\"></div></body></html>")
}

const QUERY: &str = "const r = document.getElementById('e').getBoundingClientRect(); [r.x, r.y, r.width, r.height].join()";

/// The query's answer, or its error, and how long the call took.
fn measure(rt: &mut DomRuntime, html: &str) -> (Result<String, DomError>, Duration) {
    rt.reset().unwrap();
    rt.load(Document::parse(html).unwrap());
    // The glue's install, before the clock starts.
    let _: String = rt.eval("document.body.id").unwrap();
    let start = Instant::now();
    let got = rt.eval(QUERY);
    (got, start.elapsed())
}

fn is_time_limit(got: &Result<String, DomError>) -> bool {
    matches!(got, Err(DomError::Sandbox(SandboxError::TimeLimit)))
}

/// Waits until no layout given up on still runs; how long that took.
fn drained(within: Duration) -> Duration {
    let start = Instant::now();
    while layout_threads() > 0 {
        assert!(
            start.elapsed() < within,
            "{} layout threads still run after {within:?}",
            layout_threads()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    start.elapsed()
}

fn threads() -> usize {
    std::fs::read_dir("/proc/self/task").map_or(0, Iterator::count)
}

/// Threads of the process named as the engine names its layout threads, as
/// the kernel lists them: whatever the engine counts. "surimono-layout" is 15
/// bytes, exactly the most a Linux thread name keeps (`comm`), so it isn't
/// cut. Linux only, as `/proc` is.
#[cfg(target_os = "linux")]
fn named_layout_threads() -> usize {
    std::fs::read_dir("/proc/self/task").map_or(0, |tasks| {
        tasks
            .filter_map(|t| std::fs::read_to_string(t.ok()?.path().join("comm")).ok())
            .filter(|name| name.trim_end() == "surimono-layout")
            .count()
    })
}

fn live_bytes() -> i64 {
    let s = GLOBAL.stats();
    s.bytes_allocated as i64 - s.bytes_deallocated as i64
}

fn on_big_stack<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

/// The tests share the process-wide count; one at a time.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A layout that takes longer than the page's time limit ends the script's
/// call at the limit, with the typed time limit, not when the layout ends;
/// the layout goes on alone, ends by itself, and the runtime lays the next
/// page out as if nothing had happened.
#[test]
fn a_layout_past_the_time_limit_ends_the_call_at_the_limit() {
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    on_big_stack(|| {
        // How long the slow page takes with time to spare: the layout must
        // outlast the limit below many times over for the test to mean
        // anything.
        let mut rt = runtime(common::scaled(Duration::from_secs(120)));
        let (got, full) = measure(&mut rt, &slow_page(SLOW_WORDS));
        // 12 px margins about 60,000 lines of 17 px.
        assert_eq!(
            got.as_deref(),
            Ok("0,1020024,5,5"),
            "the slow page laid out"
        );
        let limit = common::scaled(Duration::from_millis(LIMIT_MS));
        assert!(
            full > limit * 4,
            "the slow page took {full:?}, not long enough against {limit:?}"
        );

        let mut rt = runtime(limit);
        let (got, took) = measure(&mut rt, &slow_page(SLOW_WORDS));
        assert!(is_time_limit(&got), "{got:?}");
        assert!(
            took < limit + margin(),
            "the call took {took:?} with a limit of {limit:?} (the layout takes {full:?})"
        );
        // The layout runs on, and ends of itself.
        assert_eq!(layout_threads(), 1);
        let waited = drained(full * 4 + Duration::from_secs(5));
        eprintln!(
            "limit {limit:?}: the call ended after {took:?}; the layout it gave up on ({full:?} alone) ended {waited:?} later"
        );

        // The same runtime lays the next page out.
        let (got, _) = measure(&mut rt, &ordinary_page());
        assert_eq!(got.as_deref(), Ok("0,0,5,5"));
    });
}

/// A page that does this again and again holds at most `MAX_LAYOUT_THREADS`
/// layout threads and as many threads more: every call still ends within
/// its limit (a call that finds the bound reached waits for a layout to end,
/// up to its own deadline), and once they have ended the threads are gone and
/// the live heap is where it was.
#[test]
fn hostile_pages_again_and_again_hold_a_bounded_number_of_layouts() {
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    on_big_stack(|| {
        let limit = common::scaled(Duration::from_millis(LIMIT_MS));
        let mut rt = runtime(limit);
        let slow = slow_page(SLOW_WORDS);
        // Warm-up: the runtime, the glue, the fonts, one layout, and one call
        // that ends at its limit (after which the sandbox makes its runtime
        // anew, once).
        let (got, _) = measure(&mut rt, &ordinary_page());
        assert_eq!(got.as_deref(), Ok("0,0,5,5"));
        let (got, _) = measure(&mut rt, &slow);
        assert!(is_time_limit(&got), "{got:?}");
        drained(Duration::from_secs(120));
        let (got, _) = measure(&mut rt, &ordinary_page());
        assert_eq!(got.as_deref(), Ok("0,0,5,5"));
        let (threads_before, live_before) = (threads(), live_bytes());
        let mut most = 0;
        for i in 0..HOSTILE_RENDERS {
            let (got, took) = measure(&mut rt, &slow);
            assert!(is_time_limit(&got), "render {i}: {got:?}");
            assert!(took < limit + margin(), "render {i}: {took:?}");
            most = most.max(layout_threads());
            assert!(layout_threads() <= MAX_LAYOUT_THREADS, "render {i}");
            assert!(
                threads() <= threads_before + MAX_LAYOUT_THREADS,
                "render {i}: {} threads, {threads_before} before",
                threads()
            );
        }
        assert_eq!(most, MAX_LAYOUT_THREADS, "the bound was reached");
        drained(Duration::from_secs(120));
        // A thread that counted itself out may still be exiting.
        let start = Instant::now();
        while threads() > threads_before && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(threads(), threads_before);
        let (got, _) = measure(&mut rt, &ordinary_page());
        assert_eq!(got.as_deref(), Ok("0,0,5,5"));
        let grew = live_bytes() - live_before;
        eprintln!(
            "{HOSTILE_RENDERS} hostile renders: at most {most} layouts left running; live heap {grew:+} bytes after"
        );
        assert!(grew.abs() < 64 * 1024, "live heap grew by {grew} bytes");
    });
}

/// Six runtimes on six threads query the slow page at once, more callers than
/// there are slots: the process never has more than `MAX_LAYOUT_THREADS`
/// layout threads, counted by the kernel's thread names, whatever the engine
/// counts; every call ends within its limit (with the time limit, or with the
/// answer if it got a slot and its layout ended in time).
#[cfg(target_os = "linux")]
#[test]
fn several_runtimes_at_once_share_the_bounded_layout_threads() {
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    const CALLERS: usize = 6;
    let limit = common::scaled(Duration::from_millis(300));
    let start = Arc::new(Barrier::new(CALLERS));
    let stop = Arc::new(AtomicBool::new(false));
    let most = Arc::new(AtomicUsize::new(0));
    let watcher = {
        let (stop, most) = (Arc::clone(&stop), Arc::clone(&most));
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                most.fetch_max(named_layout_threads(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_micros(200));
            }
        })
    };
    let callers: Vec<_> = (0..CALLERS)
        .map(|_| {
            let start = Arc::clone(&start);
            std::thread::Builder::new()
                .stack_size(STACK)
                .spawn(move || {
                    let mut rt = runtime(limit);
                    rt.load(Document::parse(&slow_page(SLOW_WORDS)).unwrap());
                    let _: String = rt.eval("document.body.id").unwrap();
                    start.wait();
                    let begun = Instant::now();
                    let got: Result<String, DomError> = rt.eval(QUERY);
                    (got, begun.elapsed())
                })
                .unwrap()
        })
        .collect();
    let results: Vec<_> = callers.into_iter().map(|c| c.join().unwrap()).collect();
    for (i, (got, took)) in results.iter().enumerate() {
        assert!(
            is_time_limit(got) || got.as_deref() == Ok("0,1020024,5,5"),
            "caller {i}: {got:?}"
        );
        assert!(*took < limit + margin(), "caller {i}: {took:?}");
    }
    drained(Duration::from_secs(120));
    stop.store(true, Ordering::Relaxed);
    watcher.join().unwrap();
    let most = most.load(Ordering::Relaxed);
    eprintln!("{CALLERS} callers at once: at most {most} layout threads");
    assert!(most <= MAX_LAYOUT_THREADS, "{most} layout threads at once");
    assert!(most >= 2, "the callers ran at once ({most})");
}

/// While layouts given up on hold every slot, an ordinary page's layout ends
/// with the time limit, within its limit, however small the page: the cost of
/// the cap. Once they have ended, the same page lays out.
#[test]
fn an_ordinary_page_ends_with_the_time_limit_while_the_slots_are_held() {
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    on_big_stack(|| {
        let limit = common::scaled(Duration::from_millis(LIMIT_MS));
        let mut rt = runtime(limit);
        // Twice the slow page, so that the first layout given up on still
        // runs when the last slot is taken.
        let slow = slow_page(2 * SLOW_WORDS);
        for i in 0..MAX_LAYOUT_THREADS {
            let (got, _) = measure(&mut rt, &slow);
            assert!(is_time_limit(&got), "filler {i}: {got:?}");
        }
        assert_eq!(layout_threads(), MAX_LAYOUT_THREADS, "every slot held");
        let (got, took) = measure(&mut rt, &ordinary_page());
        assert!(is_time_limit(&got), "{got:?}");
        assert!(took < limit + margin(), "{took:?}");
        drained(Duration::from_secs(240));
        let mut relaxed = runtime(common::scaled(Duration::from_secs(10)));
        let (got, _) = measure(&mut relaxed, &ordinary_page());
        assert_eq!(got.as_deref(), Ok("0,0,5,5"));
    });
}
