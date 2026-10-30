//! The runtime pool: runtimes and bytecode are reused, script state is not.

use std::time::Duration;

use surimono_islands::{
    IslandError, IslandRequest, IslandRunner, Limits, RunnerOptions, SandboxError, TextMeasurer,
    TextMetrics,
};

fn options(pool_size: usize) -> RunnerOptions {
    RunnerOptions {
        pool_size,
        limits: Limits {
            time: Duration::from_millis(300),
            memory_bytes: 16 * 1024 * 1024,
            ..Limits::default()
        },
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
    let r = IslandRunner::new(options(1)).unwrap();
    r.register_bundle("polluter", POLLUTER, "entry").unwrap();
    for i in 0..5 {
        assert_eq!(render(&r, "polluter").unwrap(), CLEAN, "render {i}");
    }
}

#[test]
fn no_state_leaks_across_renderers() {
    let r = IslandRunner::new(options(1)).unwrap();
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
}

#[test]
fn no_state_leaks_after_errors() {
    let r = IslandRunner::new(options(1)).unwrap();
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
    let r = IslandRunner::new(options(2)).unwrap();
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
    let r = IslandRunner::new(options(1)).unwrap();
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
    let r = IslandRunner::new(RunnerOptions {
        text_measurer: TextMeasurer::new(|text, _| {
            if text == "panic" {
                panic!("boom");
            }
            Ok(TextMetrics::default())
        }),
        ..options(1)
    })
    .unwrap();
    r.register_bundle(
        "m",
        "function entry(s) { ctx = arguments[1]; ctx.measureText(JSON.parse(s).t); return 'ok' }",
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
    assert!(matches!(go("panic"), Err(IslandError::Worker { .. })));
    go("fine").unwrap();
    let stats = r.stats();
    assert_eq!(stats.renders, 3);
    assert_eq!(stats.failed_renders, 1);
}

#[test]
fn limit_errors_keep_the_pool_healthy() {
    let r = IslandRunner::new(options(2)).unwrap();
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
fn idle() {
    std::thread::sleep(Duration::from_millis(50));
}

#[test]
fn an_idle_worker_preloads_the_last_bundle() {
    let r = IslandRunner::new(options(1)).unwrap();
    r.register_bundle("x", "function entry() { return 'x' }", "entry")
        .unwrap();
    for _ in 0..5 {
        assert_eq!(render(&r, "x").unwrap(), "x");
        idle();
    }
    // The first render loads the bundle itself; the worker preloads it for
    // each of the others while idle.
    let stats = r.stats();
    assert_eq!(stats.renders, 5);
    assert!(stats.preloaded_renders >= 3, "{stats:?}");
}

#[test]
fn preloaded_and_cold_renders_give_the_same_output() {
    // Top-level state, randomness and a microtask the bundle queues: a
    // preloaded render must see exactly what a cold one sees.
    const STATEFUL: &str = r#"
var log = ['top ' + Math.random()];
Promise.resolve().then(() => log.push('microtask ' + Math.random()));
function entry(specJson, ctx) {
  log.push('entry ' + Math.random());
  return log.join('|');
}
"#;
    let r = IslandRunner::new(options(1)).unwrap();
    r.register_bundle("s", STATEFUL, "entry").unwrap();
    r.register_bundle("x", "function entry() { return 'x' }", "entry")
        .unwrap();
    let first = render(&r, "s").unwrap();
    assert_eq!(first.matches('|').count(), 2, "{first}");
    for i in 0..6 {
        // Odd rounds switch renderer first, so the next "s" render is cold.
        if i % 2 == 1 {
            render(&r, "x").unwrap();
        }
        idle();
        assert_eq!(render(&r, "s").unwrap(), first, "round {i}");
    }
    let stats = r.stats();
    assert!(stats.preloaded_renders >= 2, "{stats:?}");
    assert!(stats.preloaded_renders < stats.renders - 3, "{stats:?}");
}

#[test]
fn replacing_a_bundle_drops_its_preloaded_copy() {
    let r = IslandRunner::new(options(1)).unwrap();
    r.register_bundle("v", "function entry() { return 'old' }", "entry")
        .unwrap();
    assert_eq!(render(&r, "v").unwrap(), "old");
    idle();
    r.register_bundle("v", "function entry() { return 'new' }", "entry")
        .unwrap();
    idle();
    assert_eq!(render(&r, "v").unwrap(), "new");
}
