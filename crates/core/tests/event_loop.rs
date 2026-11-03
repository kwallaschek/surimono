//! The event loop on a virtual clock and its settle rule (E2-03).
//!
//! Expectations come from outside: each test names the WPT file under
//! `html/webappapis/` it restates, or the section of the HTML standard it
//! checks. WPT here means the behaviour of those tests, written out against
//! a `log` array, because `testharness.js` needs the bindings this crate
//! doesn't have. Timings the standard leaves open (the virtual budget, the
//! task limit, the frame interval) are ours and say so.

use std::time::Duration;

use surimono_core::{Limits, Sandbox, SandboxError, SandboxOptions, SettleOptions, SettleOutcome};

mod common;
use common::scaled;

fn limits() -> Limits {
    Limits::default()
        .with_memory_bytes(16 << 20)
        .with_time(scaled(Duration::from_secs(5)))
}

fn sandbox_with(limits: Limits) -> Sandbox {
    let options = SandboxOptions::default()
        .with_limits(limits)
        .with_event_loop(true);
    let mut sb = Sandbox::with_options(options).unwrap();
    sb.eval::<()>("var log = []").unwrap();
    sb
}

fn sandbox() -> Sandbox {
    sandbox_with(limits())
}

/// Runs `source`, settles with the defaults, and returns the joined log.
fn logged(source: &str) -> String {
    let mut sb = sandbox();
    sb.eval::<()>(source).unwrap();
    sb.run_until_settled(&SettleOptions::default()).unwrap();
    sb.eval("log.join(' ')").unwrap()
}

// --- WPT html/webappapis/timers ------------------------------------------

/// `evil-spec-example.any.js`: WebIDL converts the handler before the timer
/// is set, so the timer its `toString` sets comes first.
#[test]
fn evil_spec_example() {
    let got = logged(
        "function logger(s) { log.push(s) }
         setTimeout({ toString: function () {
           setTimeout(\"logger('ONE')\", 100);
           return \"logger('TWO')\";
         } }, 100);",
    );
    assert_eq!(got, "ONE TWO");
}

/// `negative-settimeout.any.js` and `type-long-settimeout.any.js`: -100 is
/// 0, and 2^32 is 0 as a WebIDL `long`.
#[test]
fn negative_and_wrapping_timeouts_run_first() {
    for timeout in ["-100", "Math.pow(2, 32)"] {
        let got = logged(&format!(
            "setTimeout(() => log.push('done'), {timeout});
             setTimeout(() => log.push('unreached'), 10);"
        ));
        assert_eq!(got, "done unreached", "{timeout}");
    }
}

/// `negative-setinterval.any.js` and `type-long-setinterval.any.js`.
#[test]
fn negative_and_wrapping_intervals_run_at_once() {
    let got = logged(
        "var i = 0;
         var interval = setInterval(function () {
           if (++i === 20) { clearInterval(interval); log.push('done'); }
         }, -100);
         setTimeout(() => log.push('late', i), 1000);",
    );
    assert_eq!(got, "done late 20");
    let got = logged(
        "var interval = setInterval(function () {
           clearInterval(interval); log.push('done');
         }, Math.pow(2, 32));
         setTimeout(() => log.push('late'), 100);",
    );
    assert_eq!(got, "done late");
}

/// `cleartimeout-clearinterval.any.js`: both kinds share one list of ids.
#[test]
fn clear_timeout_and_clear_interval_are_interchangeable() {
    let got = logged(
        "clearInterval(setTimeout(() => log.push('timeout'), 0));
         clearTimeout(setInterval(() => log.push('interval'), 0));
         setTimeout(() => log.push('done'), 100);",
    );
    assert_eq!(got, "done");
}

/// `missing-timeout-setinterval.any.js`.
#[test]
fn a_missing_interval_is_zero() {
    for args in ["", ", undefined"] {
        let got = logged(&format!(
            "var ctr = 0;
             var h = setInterval(function () {{
               if (++ctr === 2) {{ clearInterval(h); log.push(performance.now()); }}
             }}{args});"
        ));
        assert_eq!(got, "0", "{args}");
    }
}

/// `clearinterval-from-callback.any.js`.
#[test]
fn clearing_an_interval_from_its_callback_stops_it() {
    let got = logged(
        "const handle = setInterval(function () {
           log.push('tick');
           clearInterval(handle);
         }, 500);",
    );
    assert_eq!(got, "tick");
}

// --- WPT html/webappapis/microtask-queuing -------------------------------

/// `queue-microtask.any.js`, "It throws when given non-functions".
#[test]
fn queue_microtask_throws_on_non_functions() {
    let got = logged(
        "for (const args of [[], [undefined], [null], [0], [{ handleEvent() {} }],
                             ['window.x = 5;']]) {
           try { queueMicrotask(...args); log.push('no throw'); }
           catch (e) { log.push(e instanceof TypeError); }
         }",
    );
    assert_eq!(got, "true true true true true true");
}

/// `queue-microtask.any.js`, the asynchronous and no-arguments tests.
#[test]
fn queue_microtask_runs_later_with_no_arguments() {
    let got = logged(
        "queueMicrotask(function () { log.push('args', arguments.length); }, 'x', 'y');
         log.push('sync');",
    );
    assert_eq!(got, "sync args 0");
}

/// `queue-microtask.any.js`, "It interleaves with promises as expected".
#[test]
fn queue_microtask_interleaves_with_promises() {
    let got = logged(
        "Promise.resolve().then(() => log.push('a'));
         queueMicrotask(() => log.push('b'));
         Promise.reject().catch(() => log.push('c'));
         queueMicrotask(() => log.push('done'));",
    );
    assert_eq!(got, "a b c done");
}

/// `queue-microtask-exceptions.any.js`: the exception is reported, and the
/// loop carries on.
#[test]
fn queue_microtask_reports_exceptions() {
    let mut sb = sandbox();
    sb.eval::<()>(
        "setTimeout(() => {
           queueMicrotask(() => { throw new RangeError('boo'); });
           queueMicrotask(() => log.push('next microtask'));
         });
         setTimeout(() => log.push('next task'));",
    )
    .unwrap();
    let report = sb.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(report.uncaught, 1);
    assert!(matches!(
        &report.errors[..],
        [SandboxError::Exception { message, .. }] if message == "RangeError: boo"
    ));
    assert_eq!(
        sb.eval::<String>("log.join()").unwrap(),
        "next microtask,next task"
    );
}

// --- HTML §8.7 Timers ----------------------------------------------------

/// Step "If nestingLevel is greater than 5, and timeout is less than 4":
/// six tasks run with no delay, and the timer set in the sixth (nesting
/// level 6) waits 4 ms.
#[test]
fn nesting_clamps_to_4_ms_after_5_levels() {
    let got = logged(
        "function step(n) {
           log.push(performance.now());
           if (n < 9) setTimeout(step, 0, n + 1);
         }
         setTimeout(step, 0, 1);",
    );
    assert_eq!(got, "0 0 0 0 0 0 4 8 12");
}

/// "represents nested invocations of this algorithm, not of a particular
/// method": an interval's rounds nest too.
#[test]
fn an_interval_nests_like_timeouts() {
    let got = logged(
        "var n = 0;
         var h = setInterval(() => {
           log.push(performance.now());
           if (++n === 8) clearInterval(h);
         }, 1);",
    );
    assert_eq!(got, "1 2 3 4 5 6 10 14");
}

/// The clamp raises short delays only.
#[test]
fn the_clamp_leaves_longer_delays_alone() {
    let got = logged(
        "function step(n) {
           log.push(performance.now());
           if (n < 8) setTimeout(step, 5, n + 1);
         }
         step(1);",
    );
    assert_eq!(got, "0 5 10 15 20 25 30 35");
}

/// §8.1.7.3 "perform a microtask checkpoint" makes each microtask the
/// currently running task, which no timer created: a timer set from one
/// starts at nesting level 0.
#[test]
fn a_microtask_is_not_a_timer_task() {
    let got = logged(
        "function step(n) {
           log.push(performance.now());
           if (n < 9) queueMicrotask(() => setTimeout(step, 0, n + 1));
         }
         setTimeout(step, 0, 1);",
    );
    assert_eq!(got, "0 0 0 0 0 0 0 0 0");
}

/// "run steps after a timeout": same due time, set earlier, runs first;
/// and a shorter timer overtakes a longer one set earlier.
#[test]
fn timers_run_by_time_then_by_order() {
    let got = logged(
        "setTimeout(() => log.push('a'), 10);
         setTimeout(() => log.push('b'), 5);
         setTimeout(() => log.push('c'), 10);
         setTimeout(() => {
           log.push('d');
           setTimeout(() => log.push('e'), 5);
           setTimeout(() => log.push('f'), 4);
         }, 5);",
    );
    assert_eq!(got, "b d f a c e");
}

/// Handlers get the extra arguments, and `this` is the global; a string
/// ignores them and runs as a classic script in the global scope.
#[test]
fn handlers_get_arguments_and_the_global() {
    let got = logged(
        "setTimeout(function (a, b) {
           'use strict';
           log.push(a, b, arguments.length, this === globalThis);
         }, 0, 'x', 'y');
         setTimeout(\"var fromString = 1; log.push(typeof this.fromString)\", 0, 'ignored');
         var h = setInterval(function (a) { clearInterval(h); log.push(a); }, 0, 'kept');",
    );
    assert_eq!(got, "x y 2 true number kept");
}

/// A task's microtasks run before the next task (§8.1.7.3), and an
/// interval's next round is set after its callback's microtasks: one
/// clearing it stops it.
#[test]
fn microtasks_run_between_tasks() {
    let got = logged(
        "setTimeout(() => {
           log.push('t1');
           Promise.resolve().then(() => log.push('m1'));
         });
         setTimeout(() => log.push('t2'));
         var h = setInterval(() => {
           log.push('i');
           queueMicrotask(() => clearInterval(h));
         });",
    );
    assert_eq!(got, "t1 m1 t2 i");
}

/// WebIDL `long`: ToInt32 for every odd delay; a Symbol or BigInt throws.
#[test]
fn odd_delays_convert_like_a_webidl_long() {
    let cases = [
        ("NaN", 0),
        ("Infinity", 0),
        ("-Infinity", 0),
        ("null", 0),
        ("undefined", 0),
        ("{}", 0),
        ("'junk'", 0),
        ("-1", 0),
        ("-2147483648", 0),
        ("2147483648", 0),
        ("4294967297", 1),
        ("true", 1),
        ("'7'", 7),
        ("7.9", 7),
        ("[7]", 7),
        ("{ valueOf() { return 7; } }", 7),
    ];
    for (delay, want) in cases {
        let got = logged(&format!(
            "setTimeout(() => log.push(performance.now()), {delay});"
        ));
        assert_eq!(got, want.to_string(), "{delay}");
    }
    assert_eq!(
        logged(
            "for (const bad of [Symbol('s'), 1n]) {
               try { setTimeout(() => {}, bad); log.push('no throw'); }
               catch (e) { log.push(e.name); }
             }"
        ),
        "TypeError TypeError"
    );
}

/// Required and optional arguments, per the WebIDL of WindowOrWorkerGlobalScope.
#[test]
fn the_globals_have_their_webidl_shape() {
    let got = logged(
        "for (const [name, length] of [['setTimeout', 1], ['setInterval', 1],
                                       ['clearTimeout', 0], ['clearInterval', 0],
                                       ['queueMicrotask', 1],
                                       ['requestAnimationFrame', 1],
                                       ['cancelAnimationFrame', 1]]) {
           const d = Object.getOwnPropertyDescriptor(globalThis, name);
           log.push(d.value.name === name && d.value.length === length &&
                    d.enumerable && d.writable && d.configurable);
         }
         try { setTimeout(); } catch (e) { log.push(e.name); }
         try { setInterval(); } catch (e) { log.push(e.name); }
         log.push(setTimeout(() => {}) > 0);
         clearTimeout(); clearTimeout(0); clearTimeout('junk'); clearInterval(-1);",
    );
    assert_eq!(
        got,
        "true true true true true true true TypeError TypeError true"
    );
}

/// Web IDL "invoke a callback function" with "report": an exception doesn't
/// stop later tasks, or an interval's next round (the steps after the
/// invocation still run).
#[test]
fn a_throwing_timer_is_reported_and_the_loop_goes_on() {
    let mut sb = sandbox();
    sb.eval::<()>(
        "setTimeout(() => { throw new TypeError('one'); });
         setTimeout('this is not ( javascript');
         setTimeout(() => log.push('after'));
         var n = 0;
         var h = setInterval(() => {
           if (++n === 3) clearInterval(h);
           throw 'tick ' + n;
         }, 10);",
    )
    .unwrap();
    let report = sb.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(report.outcome, SettleOutcome::Settled);
    assert_eq!(report.uncaught, 5);
    let messages: Vec<String> = report
        .errors
        .iter()
        .map(|e| match e {
            SandboxError::Exception { message, .. } => message.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(messages[0], "TypeError: one");
    assert!(messages[1].starts_with("SyntaxError"), "{}", messages[1]);
    assert_eq!(messages[2..], ["tick 1", "tick 2", "tick 3"]);
    assert_eq!(sb.eval::<String>("log.join()").unwrap(), "after");
    // Reported once.
    let again = sb.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!((again.uncaught, again.errors.len()), (0, 0));
}

// --- The virtual clock ---------------------------------------------------

/// The clock moves only between tasks, by exactly the delay, and no real
/// time passes (a day of timers fits in the test's time limit).
#[test]
fn date_and_performance_follow_the_virtual_clock() {
    let got = logged(
        "var t0 = Date.now();
         function show() {
           log.push(Date.now() - t0, performance.now(), new Date().getTime() - t0);
         }
         show();
         setTimeout(show, 1500);
         setTimeout(() => {
           const a = Date.now(), b = performance.now();
           for (let i = 0; i < 100000; i++);
           log.push(Date.now() === a, performance.now() === b);
           log.push(new Date(Date()).getTime() - t0);
         }, 2000);",
    );
    assert_eq!(got, "0 0 0 1500 1500 1500 true true 2000");
}

#[test]
fn the_clock_starts_at_the_configured_instant() {
    let options = SandboxOptions::default()
        .with_now_ms(1_000_000)
        .with_event_loop(true);
    let mut sb = Sandbox::with_options(options).unwrap();
    sb.eval::<()>("var seen; setTimeout(() => seen = Date.now(), 250)")
        .unwrap();
    sb.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(sb.eval::<f64>("seen").unwrap(), 1_000_250.0);
    assert_eq!(sb.eval::<f64>("performance.timeOrigin").unwrap(), 1e6);
}

/// Without the option nothing changes: no timers, a clock that stands still.
/// (`queueMicrotask` is QuickJS's own there, as it always was.)
#[test]
fn without_the_option_scripts_see_no_event_loop() {
    let mut sb = Sandbox::new(limits()).unwrap();
    let kinds: String = sb
        .eval(
            "[typeof setTimeout, typeof setInterval, typeof clearTimeout, typeof clearInterval,
              typeof requestAnimationFrame, typeof cancelAnimationFrame].join()",
        )
        .unwrap();
    assert_eq!(kinds, ["undefined"; 6].join(","));
    assert!(sb.event_loop().is_none());
    assert!(matches!(
        sb.run_until_settled(&SettleOptions::default()),
        Err(SandboxError::InvalidOptions { .. })
    ));
}

// --- requestAnimationFrame (HTML, animation frames) ----------------------

/// Every callback of a frame gets the same time; one requested during a
/// frame waits for the next. The 16 ms frame is our choice.
#[test]
fn frames_share_a_time_and_nest_into_the_next() {
    let got = logged(
        "requestAnimationFrame(t => {
           log.push(t);
           requestAnimationFrame(t => log.push(t, performance.now()));
         });
         requestAnimationFrame(t => log.push(t));
         setTimeout(() => requestAnimationFrame(t => log.push('late', t)), 20);",
    );
    assert_eq!(got, "16 16 32 32 late 32");
}

#[test]
fn frames_cancel_apart_from_timers() {
    let got = logged(
        "const a = requestAnimationFrame(() => log.push('a'));
         const b = requestAnimationFrame(() => log.push('b'));
         const c = requestAnimationFrame(() => log.push('c'));
         requestAnimationFrame(() => cancelAnimationFrame(c));
         cancelAnimationFrame(a);
         clearTimeout(b);
         clearInterval(b);
         cancelAnimationFrame(setTimeout(() => log.push('timeout')));
         try { requestAnimationFrame('x'); } catch (e) { log.push(e.name); }",
    );
    // c is cancelled too late: it runs before the callback that cancels it.
    assert_eq!(got, "TypeError timeout b c");
}

// --- The settle rule -----------------------------------------------------

#[test]
fn an_empty_page_settles_at_once() {
    let mut sb = sandbox();
    let r = sb.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(r.outcome, SettleOutcome::Settled);
    assert_eq!((r.virtual_now_ms, r.tasks_run, r.timers_left), (0, 0, 0));
    assert!(!r.settled_by_budget());
}

#[test]
fn the_budget_includes_its_last_millisecond() {
    let mut sb = sandbox();
    sb.eval::<()>(
        "setTimeout(() => log.push('on time'), 1000);
         setTimeout(() => log.push('too late'), 1001);",
    )
    .unwrap();
    let options = SettleOptions {
        virtual_budget_ms: 1000,
        ..SettleOptions::default()
    };
    let r = sb.run_until_settled(&options).unwrap();
    assert_eq!(r.outcome, SettleOutcome::Settled);
    assert!(r.settled_by_budget());
    assert_eq!((r.virtual_now_ms, r.tasks_run, r.timers_left), (1000, 1, 1));
    assert_eq!(sb.eval::<String>("log.join()").unwrap(), "on time");
    // The budget counts from the start of the context: calling again with
    // the same one runs nothing, a longer one runs the rest.
    let r = sb.run_until_settled(&options).unwrap();
    assert_eq!((r.tasks_run, r.timers_left), (1, 1));
    let r = sb
        .run_until_settled(&SettleOptions {
            virtual_budget_ms: 1001,
            ..options
        })
        .unwrap();
    assert_eq!((r.tasks_run, r.timers_left), (2, 0));
}

/// 2^31 - 1 ms, the largest `long`, is 24.8 days: it never comes due, and
/// the clock doesn't move for it.
#[test]
fn a_huge_delay_settles_by_budget_without_moving_the_clock() {
    let mut sb = sandbox();
    sb.eval::<()>("setTimeout(() => log.push('never'), 2147483647)")
        .unwrap();
    let r = sb.run_until_settled(&SettleOptions::default()).unwrap();
    assert!(r.settled_by_budget());
    assert_eq!((r.virtual_now_ms, r.tasks_run), (0, 0));
    assert_eq!(sb.eval::<i32>("log.length").unwrap(), 0);
}

/// `setInterval(0)` with no clear: six rounds at 0 ms, then one every 4 ms
/// to the end of the default 10 s budget.
#[test]
fn an_interval_that_never_clears_ends_at_the_budget() {
    let mut sb = sandbox();
    sb.eval::<()>("var n = 0; setInterval(() => n++, 0)")
        .unwrap();
    let r = sb.run_until_settled(&SettleOptions::default()).unwrap();
    assert!(r.settled_by_budget());
    assert_eq!((r.virtual_now_ms, r.tasks_run), (10_000, 6 + 2500));
    assert_eq!(sb.eval::<u32>("n").unwrap(), 2506);
}

#[test]
fn the_task_limit_stops_cleanly_and_holds_across_calls() {
    let mut sb = sandbox();
    sb.eval::<()>("var n = 0; setInterval(() => n++, 1)")
        .unwrap();
    let options = SettleOptions {
        max_tasks: 10,
        ..SettleOptions::default()
    };
    let r = sb.run_until_settled(&options).unwrap();
    assert_eq!(r.outcome, SettleOutcome::TaskLimit);
    assert_eq!((r.tasks_run, r.timers_left), (10, 1));
    let r = sb.run_until_settled(&options).unwrap();
    assert_eq!((r.outcome, r.tasks_run), (SettleOutcome::TaskLimit, 10));
    assert_eq!(sb.eval::<u32>("n").unwrap(), 10);
    // Stopped between tasks, so the sandbox still works.
    assert_eq!(sb.eval::<i32>("6 * 7").unwrap(), 42);
}

#[test]
fn external_waits_hold_the_page_open() {
    let mut sb = sandbox();
    let handle = sb.event_loop().unwrap();
    handle.external_begin();
    handle.external_begin();
    sb.eval::<()>("setTimeout(() => log.push('timer'), 5)")
        .unwrap();
    // Timers still run; the page isn't settled while something is pending.
    let r = sb.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(r.outcome, SettleOutcome::WaitingExternal);
    assert_eq!((r.external_pending, r.tasks_run), (2, 1));
    assert_eq!(handle.virtual_now_ms(), 5);
    handle.external_end();
    let r = sb.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(r.outcome, SettleOutcome::WaitingExternal);
    assert_eq!(handle.external_pending(), 1);
    handle.external_end();
    handle.external_end(); // an extra end is ignored
    let r = sb.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(r.outcome, SettleOutcome::Settled);
    assert_eq!(r.external_pending, 0);
}

// --- Hostile scripts -----------------------------------------------------

/// A tight loop of timers: the cap on live timers ends it with a typed
/// error long before the time limit, even if the script catches the throw.
#[test]
fn a_timer_bomb_hits_the_cap() {
    for script in [
        "for (;;) setTimeout(function () {});",
        "for (;;) { try { setInterval(function () {}, 1); } catch (e) {} }",
        "for (;;) { try { requestAnimationFrame(function () {}); } catch (e) {} }",
    ] {
        let mut sb = sandbox_with(limits().with_memory_bytes(64 << 20));
        assert_eq!(
            sb.eval::<()>(script),
            Err(SandboxError::MemoryLimit),
            "{script}"
        );
        // The rebuilt sandbox has a fresh loop.
        assert_eq!(sb.eval::<i32>("setTimeout(() => {})").unwrap(), 1);
    }
}

/// Exponential timers: the task limit ends it, in a clean state.
#[test]
fn a_timer_fork_bomb_hits_the_task_limit() {
    let mut sb = sandbox_with(limits().with_memory_bytes(64 << 20));
    sb.eval::<()>("var n = 0; function f() { n++; setTimeout(f); setTimeout(f); } f()")
        .unwrap();
    let options = SettleOptions {
        max_tasks: 5_000,
        ..SettleOptions::default()
    };
    let r = sb.run_until_settled(&options).unwrap();
    assert_eq!(r.outcome, SettleOutcome::TaskLimit);
    assert_eq!(r.tasks_run, 5_000);
    assert_eq!(sb.eval::<u32>("n").unwrap(), 5_001);
}

/// Recursive microtasks never reach the next task; only the time limit can
/// stop them, wherever they start.
#[test]
fn recursive_microtasks_hit_the_time_limit() {
    let quick = limits().with_time(scaled(Duration::from_millis(200)));
    let mut sb = sandbox_with(quick);
    sb.eval::<()>("function f() { queueMicrotask(f); } setTimeout(f);")
        .unwrap();
    assert_eq!(
        sb.run_until_settled(&SettleOptions::default()),
        Err(SandboxError::TimeLimit)
    );
    sb.eval::<()>("function g() { Promise.resolve().then(g); } setTimeout(g);")
        .unwrap();
    assert_eq!(
        sb.run_until_settled(&SettleOptions::default()),
        Err(SandboxError::TimeLimit)
    );
    assert_eq!(
        sb.eval::<()>("function h() { queueMicrotask(h); } h();"),
        Err(SandboxError::TimeLimit)
    );
}

/// A callback can't escape any limit, and the sandbox comes back fresh.
#[test]
fn limits_apply_inside_callbacks() {
    let cases = [
        ("setTimeout(() => { for (;;); })", SandboxError::TimeLimit),
        (
            "setTimeout(() => { try { for (;;); } finally { log.push('x'); } })",
            SandboxError::TimeLimit,
        ),
        ("setTimeout('for (;;);')", SandboxError::TimeLimit),
        (
            "requestAnimationFrame(() => { for (;;); })",
            SandboxError::TimeLimit,
        ),
        (
            "setTimeout(() => queueMicrotask(() => { for (;;); }))",
            SandboxError::TimeLimit,
        ),
        (
            "setTimeout(() => { var a = []; for (;;) a.push(new Array(1000).fill(0)); })",
            SandboxError::MemoryLimit,
        ),
        (
            "setTimeout(() => { try { var a = []; for (;;) a.push(new Array(1000).fill(0)); }
                                catch (e) {} })",
            SandboxError::MemoryLimit,
        ),
        (
            "setTimeout(function f() { f(); })",
            SandboxError::StackOverflow,
        ),
        (
            "queueMicrotask(function f() { f(); })",
            SandboxError::StackOverflow,
        ),
        (
            "setTimeout(() => queueMicrotask(function f() { f(); }))",
            SandboxError::StackOverflow,
        ),
    ];
    for (script, want) in cases {
        let time = if want == SandboxError::TimeLimit {
            scaled(Duration::from_millis(200))
        } else {
            limits().time
        };
        let mut sb = sandbox_with(limits().with_time(time));
        sb.eval::<()>(script).unwrap();
        assert_eq!(
            sb.run_until_settled(&SettleOptions::default()),
            Err(want),
            "{script}"
        );
        let r = sb.run_until_settled(&SettleOptions::default()).unwrap();
        assert_eq!((r.outcome, r.tasks_run), (SettleOutcome::Settled, 0));
        assert_eq!(sb.eval::<String>("typeof log").unwrap(), "undefined");
    }
}

/// One time budget covers the whole run: many short callbacks can't add up
/// past it. (A budget per callback would let this run for an hour.)
#[test]
fn the_time_limit_covers_all_the_callbacks_together() {
    let mut sb = sandbox_with(limits().with_time(scaled(Duration::from_millis(200))));
    sb.eval::<()>("setInterval(() => { for (let i = 0; i < 20000; i++); }, 1)")
        .unwrap();
    let options = SettleOptions {
        virtual_budget_ms: u32::MAX.into(),
        max_tasks: u64::MAX,
    };
    assert_eq!(sb.run_until_settled(&options), Err(SandboxError::TimeLimit));
}

/// `clearTimeout` during dispatch: of a later timer due at the same time, of
/// the timer itself, and of an id already finished.
#[test]
fn clearing_during_dispatch() {
    let got = logged(
        "var b;
         var a = setTimeout(() => {
           log.push('a');
           clearTimeout(b);
           clearTimeout(a);
           setTimeout(() => clearTimeout(a));
         }, 1);
         b = setTimeout(() => log.push('b'), 1);
         setTimeout(() => log.push('c'), 1);
         var i1, i2;
         i1 = setInterval(() => { log.push('i1'); clearInterval(i2); clearInterval(i1); }, 2);
         i2 = setInterval(() => log.push('i2'), 2);",
    );
    assert_eq!(got, "a c i1");
}

/// A cleared timer's id is dead for good: ids aren't reused.
#[test]
fn ids_count_up_and_are_never_reused() {
    let got = logged(
        "var ids = [];
         for (var i = 0; i < 3; i++) { var id = setTimeout(() => {}); clearTimeout(id); ids.push(id); }
         log.push(ids.join());",
    );
    assert_eq!(got, "1,2,3");
}

/// Replacing what a script can reach changes nothing: the loop uses its own
/// captured functions, and looks up no property through a prototype.
#[test]
fn tampering_with_built_ins_changes_nothing() {
    let mut sb = sandbox();
    sb.eval::<()>(
        "var out = '';
         const setT = setTimeout, clearT = clearTimeout, micro = queueMicrotask;
         function evil() { out += 'LEAK '; throw new Error('evil'); }
         Map.prototype.get = Map.prototype.set = Map.prototype.delete = evil;
         Array.prototype.slice = evil;
         Reflect.apply = evil;
         Promise.prototype.then = evil;
         Promise.prototype.constructor = evil;
         Promise.resolve = evil;
         globalThis.eval = evil;
         Function.prototype.call = Function.prototype.apply = evil;
         Object.defineProperty(Object.prototype, '1', { get: evil, configurable: true });
         Object.defineProperty(Array.prototype, '0', { set: evil, configurable: true });
         for (const name of ['handler', 'args', 'repeat', 'length', 'constructor']) {
           Object.defineProperty(Object.prototype, name, { set: evil, get: evil,
                                                           configurable: true });
         }
         setT(function (x) { out += 'fn ' + x + ' '; }, 0, 'arg');
         setT(\"out += 'string '\");
         clearT(setT(evil));
         micro(() => out += 'micro ');",
    )
    .unwrap();
    let r = sb.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!((r.tasks_run, r.uncaught), (2, 0));
    assert_eq!(sb.eval::<String>("out").unwrap(), "micro fn arg string ");
}

/// An exception object whose getters throw, loop, or re-enter the timers.
#[test]
fn hostile_exception_objects() {
    let mut sb = sandbox();
    sb.eval::<()>(
        "setTimeout(() => { throw { get name() { throw 1; }, get message() { throw 2; } }; });
         queueMicrotask(() => { throw { get name() { setTimeout(() => log.push('re-entered')); return 'N'; } }; });
         setTimeout(() => log.push('after'));",
    )
    .unwrap();
    let r = sb.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(r.outcome, SettleOutcome::Settled);
    assert_eq!(r.uncaught, 2);
    assert_eq!(sb.eval::<String>("log.join()").unwrap(), "after,re-entered");

    let mut sb = sandbox_with(limits().with_time(scaled(Duration::from_millis(200))));
    sb.eval::<()>("setTimeout(() => { throw { get name() { for (;;); } }; })")
        .unwrap();
    assert_eq!(
        sb.run_until_settled(&SettleOptions::default()),
        Err(SandboxError::TimeLimit)
    );
}

/// Only the first few exceptions are kept; a flood is only counted.
#[test]
fn reported_errors_are_bounded() {
    let mut sb = sandbox();
    sb.eval::<()>("for (var i = 0; i < 1000; i++) setTimeout(() => { throw new Error('x'); });")
        .unwrap();
    let r = sb.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(r.uncaught, 1000);
    assert_eq!(r.errors.len(), 8);
}

// --- Same input, same output; no growth ----------------------------------

#[test]
fn the_same_page_gives_the_same_log() {
    let page = "
      function rec(tag) {
        log.push(tag, Date.now(), performance.now(), Math.random());
      }
      for (let i = 0; i < 40; i++) setTimeout(rec, (i * 7) % 13, 't' + i);
      let n = 0;
      const h = setInterval(() => { rec('i' + n); if (++n === 30) clearInterval(h); }, 3);
      function chain(k) {
        rec('c' + k);
        Promise.resolve().then(() => rec('m' + k));
        if (k < 30) setTimeout(chain, Math.floor(Math.random() * 9), k + 1);
      }
      chain(0);
      requestAnimationFrame(function fr(t) { rec('f' + t); if (t < 200) requestAnimationFrame(fr); });
    ";
    let first = logged(page);
    assert!(first.contains("c30"));
    // Other sandboxes in between share nothing with it.
    logged("setTimeout(() => Math.random(), 3)");
    assert_eq!(logged(page), first);
}

/// Teardown with timers still queued: handlers go with the context and the
/// queue with the sandbox, so a reset frees everything a render held, and
/// settling itself leaves nothing behind in the heap.
#[test]
fn renders_leave_nothing_behind() {
    let mut sb = sandbox();
    sb.reset().unwrap();
    let baseline = sb.heap_bytes();
    for round in 0..200 {
        sb.eval::<()>(
            "globalThis.big = new Array(1000).fill('x');
             for (let i = 0; i < 100; i++) {
               setTimeout((a) => a.length, i, big);
               setInterval(function () { return big; }, 7 + i);
               setTimeout(() => {}, 1e9, big);
               requestAnimationFrame(() => big);
               queueMicrotask(() => big);
             }",
        )
        .unwrap();
        if round % 2 == 0 {
            let options = SettleOptions {
                virtual_budget_ms: 100,
                ..SettleOptions::default()
            };
            let r = sb.run_until_settled(&options).unwrap();
            assert!(r.settled_by_budget());
        }
        sb.reset().unwrap();
        assert_eq!(sb.heap_bytes(), baseline, "round {round}");
    }
}

/// The engine's own count at `JS_FreeRuntime`: arenas hand bytes back
/// whatever happened, so only this shows a leaked object. Covers a clean
/// teardown with timers queued, and one forced by a limit in a callback.
#[test]
fn teardown_leaks_nothing() {
    let before = surimono_core::teardown_leaks();
    for script in [
        "setTimeout((a) => a, 1e9, { big: new Array(100).fill(0) });
         setInterval(() => {}, 5);
         requestAnimationFrame(() => {});
         queueMicrotask(() => {});",
        "setTimeout(() => { setInterval(() => {}, 1); for (;;); });
         setTimeout(() => {}, 1e9);",
    ] {
        for _ in 0..20 {
            let mut sb = sandbox_with(limits().with_time(scaled(Duration::from_millis(50))));
            sb.eval::<()>(script).unwrap();
            let _ = sb.run_until_settled(&SettleOptions {
                virtual_budget_ms: 20,
                ..SettleOptions::default()
            });
            sb.reset().unwrap();
        }
    }
    assert_eq!(surimono_core::teardown_leaks(), before);
}

/// Finished timers free their handlers while the context lives on, so a
/// long-running page doesn't grow with the number of timers it has used.
#[test]
fn finished_and_cleared_timers_free_their_handlers() {
    let mut sb = sandbox();
    let churn = "for (let i = 0; i < 2000; i++) {
                   const big = new Array(100).fill(i);
                   setTimeout(() => big.length);
                   clearTimeout(setTimeout(() => big.length, 5));
                 }";
    sb.eval::<()>(churn).unwrap();
    sb.run_until_settled(&SettleOptions {
        max_tasks: u64::MAX,
        ..SettleOptions::default()
    })
    .unwrap();
    let settled_once = sb.heap_bytes();
    for _ in 0..5 {
        sb.eval::<()>(churn).unwrap();
        sb.run_until_settled(&SettleOptions {
            max_tasks: u64::MAX,
            ..SettleOptions::default()
        })
        .unwrap();
    }
    let growth = sb.heap_bytes().saturating_sub(settled_once);
    assert!(growth < 64 * 1024, "heap grew by {growth} bytes");
}
