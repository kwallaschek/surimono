//! Cancelling inside an island: the running script stops, the pool lives on.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use common::scaled;
use surimono_islands::{
    IslandError, IslandRequest, IslandRunner, Limits, RunnerOptions, SandboxError,
};

const CANCELLED: IslandError = IslandError::Sandbox(SandboxError::Cancelled);

/// Headroom for a loaded machine; the time limit is 30 times this.
const BOUND: Duration = Duration::from_secs(2);

fn runner(pool_size: usize) -> Arc<IslandRunner> {
    let r = IslandRunner::with_estimated_text_metrics(RunnerOptions {
        pool_size,
        limits: Limits::default().with_time(scaled(BOUND * 30)),
        ..RunnerOptions::default()
    })
    .unwrap();
    r.register_bundle("spin", "function entry() { for (;;) {} }", "entry")
        .unwrap();
    r.register_bundle("echo", "function entry(s) { return s }", "entry")
        .unwrap();
    Arc::new(r)
}

fn request(renderer: &str) -> IslandRequest<'_> {
    IslandRequest {
        renderer,
        spec_json: "{}",
        width: 10,
        height: 10,
    }
}

fn wait_quiet(r: &IslandRunner) {
    let until = Instant::now() + scaled(Duration::from_secs(30));
    loop {
        let stats = r.stats();
        if stats.idle_workers == stats.workers {
            return;
        }
        assert!(Instant::now() < until, "never went quiet: {stats:?}");
        thread::sleep(Duration::from_millis(1));
    }
}

/// Runs `f` while raising the returned flag after `after`.
fn cancelled_after(
    after: Duration,
    f: impl FnOnce(&AtomicBool) -> Result<String, IslandError>,
) -> (Result<String, IslandError>, Duration) {
    let flag = Arc::new(AtomicBool::new(false));
    let raiser = {
        let flag = flag.clone();
        thread::spawn(move || {
            thread::sleep(after);
            flag.store(true, Ordering::Release);
            Instant::now()
        })
    };
    let result = f(&flag);
    let ended = Instant::now();
    let raised = raiser.join().unwrap();
    (result, ended.saturating_duration_since(raised))
}

#[test]
fn a_running_script_stops_and_the_pool_carries_on() {
    let r = runner(1);
    let (result, took) = cancelled_after(Duration::from_millis(100), |c| {
        r.render_cancellable(&request("spin"), c)
    });
    assert!(took < scaled(BOUND), "took {took:?}");
    let error = result.unwrap_err();
    assert_eq!(error, CANCELLED);
    assert!(!error.is_limit());
    // Stopped for real: the single worker serves the next render.
    assert_eq!(r.render(&request("echo")).unwrap(), "{}");
    wait_quiet(&r);
    assert_eq!(r.stats().workers, 1);
    assert_eq!(r.stats().failed_renders, 1);
}

#[test]
fn a_queued_render_is_cancelled_without_waiting_for_the_queue() {
    let r = runner(1);
    let holder_flag = Arc::new(AtomicBool::new(false));
    let holder = {
        let (r, flag) = (r.clone(), holder_flag.clone());
        thread::spawn(move || r.render_cancellable(&request("spin"), &flag))
    };
    let until = Instant::now() + scaled(Duration::from_secs(30));
    while r.stats().idle_workers != 0 {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(1));
    }
    let (queued, took) = cancelled_after(Duration::from_millis(50), |c| {
        r.render_cancellable(&request("spin"), c)
    });
    assert_eq!(queued, Err(CANCELLED));
    assert!(took < scaled(BOUND), "took {took:?}");
    // The worker is still spinning on the first render.
    assert!(!holder.is_finished());
    holder_flag.store(true, Ordering::Release);
    assert_eq!(holder.join().unwrap(), Err(CANCELLED));
    // The queued job never ran a script: it can't have spun.
    wait_quiet(&r);
    assert_eq!(r.render(&request("echo")).unwrap(), "{}");
}

#[test]
fn a_raised_flag_stops_before_anything_is_checked() {
    let r = runner(1);
    let flag = AtomicBool::new(true);
    assert_eq!(
        r.render_cancellable(&request("no such renderer"), &flag),
        Err(CANCELLED)
    );
    assert_eq!(r.stats().renders, 0);
}

#[test]
fn a_flag_that_stays_down_changes_nothing() {
    let r = runner(1);
    let flag = AtomicBool::new(false);
    assert_eq!(
        r.render_cancellable(&request("echo"), &flag),
        r.render(&request("echo")),
    );
    assert!(matches!(
        r.render_cancellable(&request("nope"), &flag),
        Err(IslandError::UnknownRenderer { .. })
    ));
    let bad = IslandRequest {
        spec_json: "{",
        ..request("echo")
    };
    assert!(matches!(
        r.render_cancellable(&bad, &flag),
        Err(IslandError::BadJson { .. })
    ));
}

#[test]
fn a_job_flag_ends_with_the_job() {
    // The caller's flag stays up; the worker must not carry it into preloads
    // or later renders.
    let r = runner(1);
    r.render(&request("echo")).unwrap();
    wait_quiet(&r);
    let (result, _) = cancelled_after(Duration::from_millis(20), |c| {
        r.render_cancellable(&request("spin"), c)
    });
    assert_eq!(result, Err(CANCELLED));
    for _ in 0..5 {
        assert_eq!(r.render(&request("echo")).unwrap(), "{}");
        wait_quiet(&r);
    }
}

#[test]
fn cancelling_again_and_again_keeps_one_runtime_per_worker() {
    let r = runner(2);
    for i in 0..100 {
        let after = Duration::from_millis(i % 7);
        let (result, _) = cancelled_after(after, |c| r.render_cancellable(&request("spin"), c));
        assert_eq!(result, Err(CANCELLED));
    }
    wait_quiet(&r);
    let stats = r.stats();
    assert!(stats.runtimes <= stats.workers, "{stats:?}");
    assert_eq!(r.render(&request("echo")).unwrap(), "{}");
}

#[test]
fn a_finished_render_beats_a_late_flag() {
    let r = runner(1);
    let flag = AtomicBool::new(false);
    let out = r.render_cancellable(&request("echo"), &flag);
    flag.store(true, Ordering::Release);
    assert_eq!(out.unwrap(), "{}");
}
