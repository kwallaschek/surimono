//! The runtime pool: runtimes and bytecode are reused, script state is not.

use std::time::Duration;

use surimono_islands::{
    IslandError, IslandRequest, IslandRunner, Limits, RunnerOptions, SandboxError,
};

fn options(pool_size: usize) -> RunnerOptions {
    RunnerOptions {
        pool_size,
        limits: Limits::default()
            .with_time(Duration::from_millis(300))
            .with_memory_bytes(16 * 1024 * 1024),
        ..RunnerOptions::default()
    }
}

fn render(r: &IslandRunner, renderer: &str) -> Result<String, IslandError> {
    r.render(&IslandRequest {
        renderer,
        spec_json: "{}",
        width: 10,
        height: 10,
    })
}

/// Reports what it can see of earlier renders, then pollutes everything it
/// can reach.
const POLLUTER: &str = r#"
var renders = (typeof renders === 'number' ? renders : 0) + 1;
function entry(specJson, ctx) {
  const seen = [
    renders,
    typeof leaked,
    typeof [].polluted,
    typeof ({}).inherited,
    Math.random() !== 0.5,
    typeof Date.now,
    JSON.parse('[1]').length,
    String(globalThis.counter),
    typeof ctx.extra,
    typeof (function () {}).planted,
    typeof ctx.measureText.planted,
    Object.getPrototypeOf(ctx.measureText) === Function.prototype,
    typeof Object.getPrototypeOf(ctx.measureText).planted2,
  ].join(',');
  globalThis.leaked = 1;
  Array.prototype.polluted = 1;
  Object.prototype.inherited = 1;
  Math.random = () => 0.5;
  Date.now = undefined;
  JSON.parse = () => 'evil';
  globalThis.counter = (globalThis.counter || 0) + 1;
  ctx.extra = 1;
  // rquickjs caches the prototype of Rust functions such as measureText per
  // runtime; a reused runtime would carry these into the next render.
  Function.prototype.planted = 1;
  Object.getPrototypeOf(ctx.measureText).planted2 = 1;
  Promise.resolve().then(() => { globalThis.lateWrite = 1 });
  return seen;
}
"#;

const CLEAN: &str = "1,undefined,undefined,undefined,true,function,1,undefined,undefined,undefined,undefined,true,undefined";

#[test]
fn no_state_leaks_between_renders() {
    // One worker, so every render reuses the same runtime.
    let r = IslandRunner::with_estimated_text_metrics(options(1)).unwrap();
    r.register_bundle("polluter", POLLUTER, "entry").unwrap();
    for i in 0..5 {
        assert_eq!(render(&r, "polluter").unwrap(), CLEAN, "render {i}");
    }
}

#[test]
fn no_state_leaks_across_renderers() {
    let r = IslandRunner::with_estimated_text_metrics(options(1)).unwrap();
    r.register_bundle("polluter", POLLUTER, "entry").unwrap();
    r.register_bundle(
        "observer",
        "function entry() { return [typeof leaked, typeof lateWrite, typeof renders, typeof [].polluted].join() }",
        "entry",
    )
    .unwrap();
    render(&r, "polluter").unwrap();
    assert_eq!(
        render(&r, "observer").unwrap(),
        "undefined,undefined,undefined,undefined"
    );
    // The same with an idle gap, so the worker has preloaded the polluter.
    render(&r, "polluter").unwrap();
    idle(&r);
    assert_eq!(
        render(&r, "observer").unwrap(),
        "undefined,undefined,undefined,undefined"
    );
}

#[test]
fn no_state_leaks_after_errors() {
    let r = IslandRunner::with_estimated_text_metrics(options(1)).unwrap();
    r.register_bundle("polluter", POLLUTER, "entry").unwrap();
    for (name, src) in [
        (
            "throws",
            "function entry() { globalThis.leaked = 1; Array.prototype.polluted = 1; throw new Error('x') }",
        ),
        (
            "loops",
            "function entry() { globalThis.leaked = 1; Array.prototype.polluted = 1; for (;;) {} }",
        ),
        (
            "queues",
            "function entry() { Promise.resolve().then(() => { globalThis.leaked = 1 }); throw 1 }",
        ),
    ] {
        r.register_bundle(name, src, "entry").unwrap();
        assert!(render(&r, name).is_err(), "{name}");
        assert_eq!(render(&r, "polluter").unwrap(), CLEAN, "after {name}");
    }
}

#[test]
fn many_renders_on_a_small_pool_are_counted() {
    let r = IslandRunner::with_estimated_text_metrics(options(2)).unwrap();
    r.register_bundle("x", "function entry() { return 'x' }", "entry")
        .unwrap();
    for _ in 0..50 {
        render(&r, "x").unwrap();
    }
    let stats = r.stats();
    assert_eq!(stats.renders, 50);
    assert_eq!(stats.failed_renders, 0);
    assert_eq!(r.pool_size(), 2);
}

#[test]
fn heap_is_freed_after_every_render() {
    // Each render holds about 8 MiB of a 16 MiB heap. If any of it survived
    // into the next render on the same runtime, the second would fail.
    let r = IslandRunner::with_estimated_text_metrics(options(1)).unwrap();
    r.register_bundle(
        "big",
        "function entry() { globalThis.keep = new Float64Array(1024 * 1024); return 'ok' }",
        "entry",
    )
    .unwrap();
    for i in 0..100 {
        assert_eq!(render(&r, "big").unwrap(), "ok", "render {i}");
    }
}

#[test]
fn stats_count_failures() {
    // (Panics in a measurer are counted too; see the runner's unit tests.)
    let r = IslandRunner::with_estimated_text_metrics(options(1)).unwrap();
    r.register_bundle(
        "m",
        "function entry(s) { if (JSON.parse(s).t === 'fail') throw new Error('x'); return 'ok' }",
        "entry",
    )
    .unwrap();
    let go = |t: &str| {
        r.render(&IslandRequest {
            renderer: "m",
            spec_json: &format!(r#"{{"t":"{t}"}}"#),
            width: 1,
            height: 1,
        })
    };
    go("fine").unwrap();
    assert!(go("fail").is_err());
    go("fine").unwrap();
    let stats = r.stats();
    assert_eq!(stats.renders, 3);
    assert_eq!(stats.failed_renders, 1);
}

#[test]
fn limit_errors_keep_the_pool_healthy() {
    let r = IslandRunner::with_estimated_text_metrics(options(2)).unwrap();
    r.register_bundle("loop", "function entry() { for (;;) {} }", "entry")
        .unwrap();
    r.register_bundle("x", "function entry() { return 'x' }", "entry")
        .unwrap();
    for _ in 0..4 {
        assert_eq!(
            render(&r, "loop").unwrap_err(),
            IslandError::Sandbox(SandboxError::TimeLimit)
        );
        assert_eq!(render(&r, "x").unwrap(), "x");
    }
}

/// Gives an idle worker time to reset and preload.
/// Waits until every worker of `r` is idle: reset and preloaded.
fn idle(r: &IslandRunner) {
    let start = std::time::Instant::now();
    while r.stats().idle_workers < r.pool_size() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "workers never went idle: {:?}",
            r.stats()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn an_idle_worker_preloads_the_last_bundle() {
    let r = IslandRunner::with_estimated_text_metrics(options(1)).unwrap();
    r.register_bundle("x", "function entry() { return 'x' }", "entry")
        .unwrap();
    for _ in 0..5 {
        assert_eq!(render(&r, "x").unwrap(), "x");
        idle(&r);
    }
    // The first render loads the bundle itself; the worker preloads it for
    // each of the others while idle.
    let stats = r.stats();
    assert_eq!(stats.renders, 5);
    assert!(stats.preloaded_renders >= 3, "{stats:?}");
}

#[test]
fn preloaded_and_cold_renders_give_the_same_output() {
    // Top-level state and randomness: a preloaded render must see exactly
    // what a cold one sees.
    const STATEFUL: &str = r#"
var log = ['top ' + Math.random()];
function entry(specJson, ctx) {
  log.push('entry ' + Math.random());
  return log.join('|');
}
"#;
    let r = IslandRunner::with_estimated_text_metrics(options(1)).unwrap();
    r.register_bundle("s", STATEFUL, "entry").unwrap();
    r.register_bundle("x", "function entry() { return 'x' }", "entry")
        .unwrap();
    let first = render(&r, "s").unwrap();
    assert_eq!(first.matches('|').count(), 1, "{first}");
    for i in 0..6 {
        // Odd rounds switch renderer first, so the next "s" render is cold.
        if i % 2 == 1 {
            render(&r, "x").unwrap();
        }
        idle(&r);
        assert_eq!(render(&r, "s").unwrap(), first, "round {i}");
    }
    let stats = r.stats();
    assert!(stats.preloaded_renders >= 2, "{stats:?}");
    assert!(stats.preloaded_renders < stats.renders - 3, "{stats:?}");
}

#[test]
fn a_bundle_that_queues_microtasks_is_never_preloaded() {
    // The microtask must run after the entry's synchronous part, as in a
    // cold render; a preload would have run it before. This also shows that
    // a cold render loads the bundle and calls the entry in one sandbox
    // call, which is what gives the island one deadline: a separate call
    // for the bundle would have drained its microtask first.
    const QUEUES: &str = r#"
var log = ['top'];
Promise.resolve().then(() => log.push('microtask'));
function entry() { return log.join('|') }
"#;
    let r = IslandRunner::with_estimated_text_metrics(options(1)).unwrap();
    r.register_bundle("q", QUEUES, "entry").unwrap();
    for _ in 0..4 {
        assert_eq!(render(&r, "q").unwrap(), "top");
        idle(&r);
    }
    assert_eq!(r.stats().preloaded_renders, 0);
}

#[test]
fn idle_workers_are_counted() {
    let r = IslandRunner::with_estimated_text_metrics(options(2)).unwrap();
    r.register_bundle("x", "function entry() { return 'x' }", "entry")
        .unwrap();
    render(&r, "x").unwrap();
    let start = std::time::Instant::now();
    while r.stats().idle_workers != 2 {
        assert!(start.elapsed() < Duration::from_secs(10), "{:?}", r.stats());
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn replacing_a_bundle_drops_its_preloaded_copy() {
    let r = IslandRunner::with_estimated_text_metrics(options(1)).unwrap();
    r.register_bundle("v", "function entry() { return 'old' }", "entry")
        .unwrap();
    assert_eq!(render(&r, "v").unwrap(), "old");
    idle(&r);
    r.register_bundle("v", "function entry() { return 'new' }", "entry")
        .unwrap();
    idle(&r);
    assert_eq!(render(&r, "v").unwrap(), "new");
}

#[test]
fn a_preloaded_bundle_never_leaks_into_another_renderer() {
    let r = IslandRunner::with_estimated_text_metrics(options(1)).unwrap();
    r.register_bundle(
        "a",
        "var fromA = 1; function entry() { return 'a' }",
        "entry",
    )
    .unwrap();
    r.register_bundle("b", "function entry() { return typeof fromA }", "entry")
        .unwrap();
    for _ in 0..3 {
        render(&r, "a").unwrap();
        // The idle worker preloads "a"; "b" must still start clean.
        idle(&r);
        assert_eq!(render(&r, "b").unwrap(), "undefined");
        idle(&r);
    }
}

#[test]
fn registration_never_sees_a_preloaded_bundle() {
    let r = IslandRunner::with_estimated_text_metrics(options(1)).unwrap();
    r.register_bundle("a", "function entry() { return 'a' }", "entry")
        .unwrap();
    render(&r, "a").unwrap();
    // The worker now holds "a", which defines `entry`; a bundle that
    // doesn't define it must still be refused.
    idle(&r);
    assert_eq!(
        r.register_bundle("b", "var x = 1;", "entry").unwrap_err(),
        IslandError::MissingEntry {
            bundle: "b".into(),
            entry: "entry".into()
        }
    );
}

#[test]
fn an_entry_defined_in_a_microtask_is_refused_at_registration() {
    // A render looks the entry up right after the bundle runs, before any
    // microtask, so registration must refuse it too.
    let r = IslandRunner::with_estimated_text_metrics(options(1)).unwrap();
    assert_eq!(
        r.register_bundle(
            "late",
            "Promise.resolve().then(() => { globalThis.entry = () => 'x' })",
            "entry",
        )
        .unwrap_err(),
        IslandError::MissingEntry {
            bundle: "late".into(),
            entry: "entry".into()
        }
    );
}
