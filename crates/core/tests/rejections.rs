//! Rejected promises nobody handled (P2-RT-05).
//!
//! Which rejections count, and when, comes from the HTML standard's
//! "unhandled promise rejections" section and from WPT's
//! `html/webappapis/scripting/processing-model-2/unhandled-promise-rejections/
//! promise-rejection-events.html`, restated here against the report because
//! the event itself belongs to the DOM. That a limit error ends the call is
//! ours: it is the sandbox rule that hitting a limit gives a typed error.

use std::time::{Duration, Instant};

use surimono_core::{
    Limits, Sandbox, SandboxError, SandboxOptions, SettleOptions, SettleReport, teardown_leaks,
};

mod common;
use common::scaled;

/// A function that overflows the stack, and one that lets it escape.
const OVERFLOW: &str = "function r() { return r() + 1 } async function f() { r() }\n";

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

fn settle(sb: &mut Sandbox) -> Result<SettleReport, SandboxError> {
    sb.run_until_settled(&SettleOptions::default())
}

/// Runs `source` and settles.
fn run(source: &str) -> Result<SettleReport, SandboxError> {
    let mut sb = sandbox();
    sb.eval::<()>(source).unwrap();
    settle(&mut sb)
}

/// The reasons kept in a report, as text.
fn reasons(report: &SettleReport) -> Vec<&str> {
    report
        .rejections
        .iter()
        .map(|e| match e {
            SandboxError::Exception { message, .. } => message.as_str(),
            other => panic!("not a script's reason: {other:?}"),
        })
        .collect()
}

fn overflows(body: &str) {
    let source = format!("{OVERFLOW}{body}");
    assert_eq!(run(&source), Err(SandboxError::StackOverflow), "{body}");
}

// --- Limit errors ---------------------------------------------------------

#[test]
fn overflow_in_an_async_function_ends_the_call() {
    overflows("f()");
}

#[test]
fn overflow_past_an_await_ends_the_call() {
    overflows("async function g() { await null; r() } g()");
}

#[test]
fn overflow_of_async_functions_calling_themselves_ends_the_call() {
    // Only the deepest call fails, and the rest return normally.
    overflows("async function g() { g() } g()");
}

#[test]
fn overflow_in_a_promise_reaction_ends_the_call() {
    overflows("Promise.resolve().then(r)");
    overflows("Promise.reject(1).catch(r)");
    overflows("Promise.resolve().finally(r)");
}

#[test]
fn overflow_in_a_promise_executor_ends_the_call() {
    overflows("new Promise(r)");
}

#[test]
fn overflow_in_an_async_generator_ends_the_call() {
    overflows("async function* g() { r() } g().next()");
    overflows("async function* g() { yield 1; r() } const i = g(); i.next(); i.next()");
    overflows(
        "async function* g() { try { yield 1 } finally { r() } }
               const i = g(); i.next().then(() => i.return())",
    );
}

#[test]
fn overflow_at_the_end_of_a_chain_ends_the_call() {
    overflows("f().then(() => {})");
    overflows("async function g() { await f() } g()");
    overflows("Promise.all([f()])");
}

#[test]
fn overflow_in_any_task_ends_the_call() {
    overflows("setTimeout(f, 10)");
    overflows("requestAnimationFrame(f)");
    overflows("queueMicrotask(f)");
    overflows("setTimeout(() => queueMicrotask(f), 10)");
}

#[test]
fn overflow_at_every_stack_depth_ends_the_call() {
    // Frame sizes walk the point of overflow across the engine's checks.
    for n in 0..40 {
        let params: Vec<String> = (0..n).map(|i| format!("a{i}")).collect();
        let params = params.join(",");
        overflows(&format!(
            "async function g({params}) {{ g() }}
             function h({params}) {{ g(); h() }}
             try {{ h() }} catch (e) {{}}"
        ));
    }
}

#[test]
fn running_out_of_memory_in_an_async_function_ends_the_call() {
    let got = run(
        "async function f() { const a = []; for (;;) a.push(new Array(1000).fill(0)) }
         setTimeout(f)",
    );
    assert_eq!(got, Err(SandboxError::MemoryLimit));
}

#[test]
fn the_time_limit_in_an_async_function_ends_the_call() {
    let mut sb = sandbox_with(limits().with_time(scaled(Duration::from_millis(100))));
    sb.eval::<()>("setTimeout(async () => { for (;;) {} })")
        .unwrap();
    assert_eq!(settle(&mut sb), Err(SandboxError::TimeLimit));
}

#[test]
fn the_sandbox_starts_over_after_a_limit_in_a_rejection() {
    let mut sb = sandbox();
    sb.eval::<()>(&format!("globalThis.mine = 1; {OVERFLOW} f()"))
        .unwrap();
    assert_eq!(settle(&mut sb), Err(SandboxError::StackOverflow));
    assert_eq!(sb.eval::<String>("typeof mine").unwrap(), "undefined");
    let report = settle(&mut sb).unwrap();
    assert_eq!(report.unhandled_rejections, 0);
}

#[test]
fn a_caught_limit_error_that_is_rejected_again_is_still_one() {
    overflows("async function g() { try { r() } catch (e) { throw e } } g()");
    overflows(
        "let saved; try { r() } catch (e) { saved = e }
               setTimeout(() => Promise.reject(saved), 10)",
    );
}

// --- Forged and disguised errors ------------------------------------------

/// What a script can make with the engine's words: plain rejections.
#[test]
fn a_forged_rejection_is_an_ordinary_one() {
    for (make, text) in [
        (
            "new RangeError('Maximum call stack size exceeded')",
            "RangeError: Maximum call stack size exceeded",
        ),
        (
            "new InternalError('out of memory')",
            "InternalError: out of memory",
        ),
        (
            "new InternalError('interrupted')",
            "InternalError: interrupted",
        ),
        (
            "{ name: 'RangeError', message: 'Maximum call stack size exceeded' }",
            "RangeError: Maximum call stack size exceeded",
        ),
    ] {
        let mut sb = sandbox();
        sb.eval::<()>(&format!("globalThis.kept = 1; Promise.reject({make})"))
            .unwrap();
        let report = settle(&mut sb).unwrap();
        assert_eq!(report.unhandled_rejections, 1, "{make}");
        assert_eq!(reasons(&report), [text], "{make}");
        assert_eq!(sb.eval::<i32>("kept").unwrap(), 1, "{make}");
    }
}

#[test]
fn a_forged_throw_is_an_ordinary_exception() {
    let mut sb = sandbox();
    sb.eval::<()>("globalThis.kept = 1").unwrap();
    let got = sb.eval::<()>("throw new RangeError('Maximum call stack size exceeded')");
    assert!(
        matches!(got, Err(SandboxError::Exception { .. })),
        "{got:?}"
    );
    let got = sb.eval::<()>("throw new InternalError('out of memory')");
    assert!(
        matches!(got, Err(SandboxError::Exception { .. })),
        "{got:?}"
    );
    assert_eq!(sb.eval::<i32>("kept").unwrap(), 1);

    sb.eval::<()>("setTimeout(() => { throw new RangeError('Maximum call stack size exceeded') })")
        .unwrap();
    assert_eq!(settle(&mut sb).unwrap().uncaught, 1);
}

#[test]
fn objects_built_on_a_real_error_are_not_real_errors() {
    let report = run("function r() { return r() + 1 }
         let real; try { r() } catch (e) { real = e }
         Promise.reject(Object.create(real));
         Promise.reject(new Proxy(real, {}));")
    .unwrap();
    assert_eq!(report.unhandled_rejections, 2);
}

#[test]
fn a_real_error_with_its_properties_changed_still_counts() {
    overflows(
        "async function g() {
           try { r() } catch (e) {
             e.name = 'Harmless'; e.message = 'nothing to see'; delete e.stack; throw e;
           }
         }
         g()",
    );
}

#[test]
fn tampering_with_error_prototypes_hides_nothing() {
    overflows("RangeError.prototype.name = 'Harmless'; f()");
    overflows("delete RangeError.prototype.name; delete Error.prototype.name; f()");
    overflows(
        "Object.defineProperty(RangeError.prototype, 'name', {
                 get() { throw 1 },
               });
               f()",
    );
    overflows("Object.setPrototypeOf(RangeError.prototype, null); f()");
}

#[test]
fn tampering_hides_nothing_from_a_synchronous_throw_either() {
    let mut sb = sandbox();
    let got = sb.eval::<()>(
        "RangeError.prototype.name = 'Harmless';
         function r() { return r() + 1 } r()",
    );
    assert_eq!(got, Err(SandboxError::StackOverflow));
}

#[test]
fn replacing_promise_then_hides_nothing() {
    overflows("Promise.prototype.then = () => {}; f()");
    overflows("Promise.prototype.catch = () => {}; f()");
}

// --- Handled rejections ---------------------------------------------------

#[test]
fn a_handled_overflow_is_unaffected() {
    let mut sb = sandbox();
    sb.eval::<()>(&format!(
        "{OVERFLOW}
         f().catch((e) => log.push(e.name));
         (async () => {{ try {{ await f() }} catch (e) {{ log.push('await') }} }})();
         log.push('sync')"
    ))
    .unwrap();
    let report = settle(&mut sb).unwrap();
    assert_eq!(report.unhandled_rejections, 0);
    assert_eq!(
        sb.eval::<String>("log.join(' ')").unwrap(),
        "sync RangeError await"
    );
}

#[test]
fn a_fulfilled_or_handled_promise_is_not_counted() {
    let report = run("Promise.resolve(1).then(() => {});
         Promise.reject(1).catch(() => {});
         Promise.reject(1).then(null, () => {});
         Promise.allSettled([Promise.reject(1)]);
         Promise.any([Promise.reject(1), 2]);
         (async () => { try { await Promise.reject(1) } catch {} })();
         new Promise(() => {});")
    .unwrap();
    assert_eq!(report.unhandled_rejections, 0);
    assert_eq!(report.rejections, []);
}

/// `promise-rejection-events.html`: "delayed handling: ... a handler added
/// in the same task".
#[test]
fn a_handler_added_later_in_the_same_task_means_no_report() {
    let report = run("const p = Promise.reject(1);
         for (let i = 0; i < 1000; i++) log.push(i);
         p.catch(() => {});")
    .unwrap();
    assert_eq!(report.unhandled_rejections, 0);

    overflow_handled_later("const p = f(); p.catch(() => {})");
}

/// `promise-rejection-events.html`: handlers attached from microtasks, up to
/// the end of the checkpoint.
#[test]
fn a_handler_added_in_a_microtask_means_no_report() {
    let report = run("const p = Promise.reject(1);
         queueMicrotask(() => queueMicrotask(() => p.catch(() => {})));")
    .unwrap();
    assert_eq!(report.unhandled_rejections, 0);

    overflow_handled_later("const p = f(); Promise.resolve().then(() => p.catch(() => {}))");
}

fn overflow_handled_later(body: &str) {
    let report = run(&format!("{OVERFLOW}{body}")).unwrap();
    assert_eq!(report.unhandled_rejections, 0, "{body}");
}

/// `promise-rejection-events.html`: a handler from a later task is too late.
#[test]
fn a_handler_added_in_a_later_task_is_too_late() {
    let report = run("const p = Promise.reject(1);
         setTimeout(() => p.catch(() => {}));")
    .unwrap();
    assert_eq!(report.unhandled_rejections, 1);

    overflows("const p = f(); setTimeout(() => p.catch(() => {}))");
}

// --- Plain unhandled rejections -------------------------------------------

#[test]
fn plain_rejections_are_counted_and_the_loop_goes_on() {
    let mut sb = sandbox();
    sb.eval::<()>(
        "Promise.reject(new Error('one'));
         (async () => { throw new TypeError('two') })();
         Promise.resolve().then(() => { throw 3 });
         setTimeout(() => { Promise.reject('four'); log.push('ran') }, 10);
         setTimeout(() => log.push('ran'), 20);",
    )
    .unwrap();
    let report = settle(&mut sb).unwrap();
    assert_eq!(report.unhandled_rejections, 4);
    assert_eq!(report.uncaught, 0, "not uncaught exceptions");
    assert_eq!(
        reasons(&report),
        ["Error: one", "TypeError: two", "3", "four"]
    );
    assert_eq!(sb.eval::<String>("log.join(' ')").unwrap(), "ran ran");
}

#[test]
fn a_rejection_counts_once_along_a_chain() {
    let report = run("Promise.reject(1).then(() => {}).then(() => {});").unwrap();
    assert_eq!(report.unhandled_rejections, 1);
}

#[test]
fn counts_are_since_the_last_report() {
    let mut sb = sandbox();
    sb.eval::<()>("Promise.reject(1)").unwrap();
    assert_eq!(settle(&mut sb).unwrap().unhandled_rejections, 1);
    assert_eq!(settle(&mut sb).unwrap().unhandled_rejections, 0);
    sb.eval::<()>("Promise.reject(2); Promise.reject(3)")
        .unwrap();
    assert_eq!(settle(&mut sb).unwrap().unhandled_rejections, 2);
}

#[test]
fn only_the_first_few_reasons_are_kept() {
    let report = run("for (let i = 0; i < 20; i++) Promise.reject(i)").unwrap();
    assert_eq!(report.unhandled_rejections, 20);
    assert_eq!(reasons(&report), ["0", "1", "2", "3", "4", "5", "6", "7"]);
}

#[test]
fn the_same_page_reports_the_same_rejections() {
    let source = "for (let i = 0; i < 50; i++) {
           const p = Promise.reject(new Error('e' + i));
           if (i % 3 === 0) p.catch(() => {});
         }";
    let first = run(source).unwrap();
    assert_eq!(first.unhandled_rejections, 33);
    assert_eq!(first, run(source).unwrap());
}

// --- Hostile reasons ------------------------------------------------------

#[test]
fn a_reason_with_throwing_getters_is_still_counted() {
    let report = run(
        "Promise.reject({ get name() { throw 1 }, get message() { throw 2 } });
         Promise.reject(new Proxy({}, { get() { throw 3 } }));",
    )
    .unwrap();
    assert_eq!(report.unhandled_rejections, 2);
}

#[test]
fn a_getter_can_handle_a_promise_still_on_the_list() {
    let report = run("let later;
         Promise.reject({ get name() { later.catch(() => {}); return 'x' } });
         later = Promise.reject(2);")
    .unwrap();
    assert_eq!(report.unhandled_rejections, 1);
}

#[test]
fn work_a_getter_queues_is_part_of_the_checkpoint() {
    let mut sb = sandbox();
    sb.eval::<()>(
        "Promise.reject({ get name() {
           queueMicrotask(() => log.push('micro'));
           Promise.reject(2);
           return 'x';
         } });",
    )
    .unwrap();
    let report = settle(&mut sb).unwrap();
    assert_eq!(report.unhandled_rejections, 2);
    assert_eq!(sb.eval::<String>("log.join(' ')").unwrap(), "micro");
}

#[test]
fn a_getter_that_never_returns_hits_the_time_limit() {
    let mut sb = sandbox_with(limits().with_time(scaled(Duration::from_millis(100))));
    sb.eval::<()>("Promise.reject({ get name() { for (;;) {} } })")
        .unwrap();
    assert_eq!(settle(&mut sb), Err(SandboxError::TimeLimit));
}

/// A getter that rejects again can't keep the checkpoint going: each read
/// makes one more rejection, and reads stop with the last reason kept.
#[test]
fn rejections_that_keep_rejecting_end_with_the_kept_reasons() {
    let mut sb = sandbox();
    sb.eval::<()>(
        "const o = { get name() { Promise.reject(o); return 'x' } };
         Promise.reject(o);",
    )
    .unwrap();
    let report = settle(&mut sb).unwrap();
    assert_eq!(report.unhandled_rejections, 9);
    assert_eq!(report.rejections.len(), 8);
}

/// The list's entries are outside the JavaScript heap.
#[test]
fn too_many_rejections_waiting_is_a_memory_error() {
    let mut sb = sandbox_with(limits().with_memory_bytes(256 << 20));
    let got = sb.eval::<()>("for (let i = 0; i < 100001; i++) Promise.reject(i)");
    assert_eq!(got, Err(SandboxError::MemoryLimit));
}

#[test]
fn handled_rejections_do_not_fill_the_list() {
    let mut sb = sandbox_with(limits().with_memory_bytes(256 << 20));
    let report = {
        sb.eval::<()>(
            "const done = () => {};
             for (let i = 0; i < 200000; i++) Promise.reject(i).catch(done);",
        )
        .unwrap();
        settle(&mut sb).unwrap()
    };
    assert_eq!(report.unhandled_rejections, 0);
}

#[test]
fn handlers_in_rejection_order_are_quick() {
    // In reverse of what a list scanned from its end handles well.
    let mut sb = sandbox_with(limits().with_memory_bytes(256 << 20));
    sb.eval::<()>(
        "const all = [];
         for (let i = 0; i < 90000; i++) all.push(Promise.reject(i));
         for (const p of all) p.catch(() => {});",
    )
    .unwrap();
    assert_eq!(settle(&mut sb).unwrap().unhandled_rejections, 0);
}

// --- Where it applies, and what it leaves behind --------------------------

#[test]
fn a_sandbox_without_the_event_loop_tracks_nothing() {
    let mut sb = Sandbox::new(limits()).unwrap();
    sb.eval::<()>(&format!("{OVERFLOW} f()")).unwrap();
    assert_eq!(sb.eval::<i32>("1").unwrap(), 1);
}

#[test]
fn rejections_a_sandbox_is_dropped_with_are_let_go() {
    for _ in 0..50 {
        let mut sb = sandbox();
        sb.eval::<()>("globalThis.p = Promise.reject({}); Promise.reject(new Error('x'))")
            .unwrap();
        drop(sb);
    }
    assert_eq!(teardown_leaks().objects, 0);
}

#[test]
fn a_reset_lets_go_of_waiting_rejections() {
    let mut sb = sandbox();
    sb.eval::<()>("Promise.reject(1)").unwrap();
    sb.reset().unwrap();
    assert_eq!(settle(&mut sb).unwrap().unhandled_rejections, 0);
    drop(sb);
    assert_eq!(teardown_leaks().objects, 0);
}

/// What a long-lived sandbox pays: nothing that grows.
#[test]
fn the_heap_stays_flat_over_many_rejections() {
    let mut sb = sandbox();
    let round = "for (let i = 0; i < 200; i++) {
           Promise.reject(new Error('plain'));
           Promise.reject(new Error('handled')).catch(() => {});
           (async () => { throw { i } })();
         }";
    let mut warm = 0;
    for i in 0..300 {
        sb.eval::<()>(round).unwrap();
        let report = settle(&mut sb).unwrap();
        assert_eq!(report.unhandled_rejections, 400);
        if i == 50 {
            sb.run(|ctx| {
                ctx.run_gc();
                Ok(())
            })
            .unwrap();
            warm = sb.heap_bytes();
        }
    }
    sb.run(|ctx| {
        ctx.run_gc();
        Ok(())
    })
    .unwrap();
    let last = sb.heap_bytes();
    assert!(last <= warm + 16 * 1024, "{warm} grew to {last}");
}

#[test]
fn renders_with_limit_rejections_leak_nothing() {
    let mut sb = sandbox();
    for _ in 0..50 {
        sb.eval::<()>(&format!("{OVERFLOW} Promise.reject(1); f()"))
            .unwrap();
        assert_eq!(settle(&mut sb), Err(SandboxError::StackOverflow));
    }
    drop(sb);
    assert_eq!(teardown_leaks().objects, 0);
}

// --- Reasons nobody keeps ---------------------------------------------------

/// `count` promises rejected with one reason whose three strings hold 4 MiB
/// each, none of them ASCII, so reading the reason copies and converts them.
fn large_reasons(count: usize) -> String {
    format!(
        "const big = 'x'.repeat(4000000) + '\\u3042';
         const o = {{ name: big, message: big, stack: big }};
         for (let i = 0; i < {count}; i++) Promise.reject(o);"
    )
}

/// Review of #127: reading every reason took about 12 ms each, between two
/// polls, so 4,000 of them ran for 48 s under a 2 s limit.
#[test]
fn many_large_reasons_stay_inside_the_time_limit() {
    let budget = scaled(Duration::from_secs(1));
    let mut sb = sandbox_with(
        Limits::default()
            .with_memory_bytes(256 << 20)
            .with_time(budget),
    );
    sb.eval::<()>(&large_reasons(4_000)).unwrap();
    let start = Instant::now();
    let result = settle(&mut sb);
    let elapsed = start.elapsed();
    assert!(elapsed < budget * 3, "{elapsed:?} under a {budget:?} limit");
    // A very slow machine may stop at the limit, which is also an answer.
    match result {
        Ok(report) => {
            assert_eq!(report.unhandled_rejections, 4_000);
            assert_eq!(report.rejections.len(), 8);
        }
        Err(e) => assert_eq!(e, SandboxError::TimeLimit),
    }
}

/// Only the reasons kept are read, so only their getters run.
#[test]
fn getters_of_reasons_nobody_keeps_do_not_run() {
    let mut sb = sandbox();
    sb.eval::<()>(
        "var reads = 0;
         for (let i = 0; i < 100; i++)
           Promise.reject({ get message() { reads++; return 'm' } });",
    )
    .unwrap();
    let report = settle(&mut sb).unwrap();
    assert_eq!(report.unhandled_rejections, 100);
    assert_eq!(report.rejections.len(), 8);
    assert_eq!(sb.eval::<i32>("reads").unwrap(), 8);
}

/// A limit error is told by its mark, which takes no read, so it ends the call
/// wherever it is in the list.
#[test]
fn a_limit_error_past_the_kept_reasons_still_ends_the_call() {
    let source = format!("{OVERFLOW} for (let i = 0; i < 20; i++) Promise.reject(i); f()");
    assert_eq!(run(&source), Err(SandboxError::StackOverflow));
}
