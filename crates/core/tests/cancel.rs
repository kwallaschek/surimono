//! A cancel flag stops a running call from outside, whatever the script does.
//!
//! Limits here are far longer than the bounds, so only the flag can end a
//! call in time.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use common::scaled;
use surimono_core::rquickjs::Function;
use surimono_core::{Limits, Sandbox, SandboxError};

/// How long after the flag goes up a call may end. The stop itself takes
/// milliseconds; the rest is headroom for a loaded CI machine and debug builds.
const BOUND: Duration = Duration::from_secs(2);

fn sandbox() -> (Sandbox, Arc<AtomicBool>) {
    let limits = Limits::default()
        .with_time(scaled(Duration::from_secs(60)))
        .with_memory_bytes(64 << 20);
    let mut sb = Sandbox::new(limits).unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    sb.set_cancel_flag(Some(flag.clone()));
    (sb, flag)
}

/// Raises `flag` after `after` and returns when it did.
fn raise_after(flag: &Arc<AtomicBool>, after: Duration) -> thread::JoinHandle<Instant> {
    let flag = flag.clone();
    thread::spawn(move || {
        thread::sleep(after);
        flag.store(true, Ordering::Release);
        Instant::now()
    })
}

/// Runs `script`, raises the flag while it runs, and checks the call stops.
fn cancels(script: &str) {
    let (mut sb, flag) = sandbox();
    let raiser = raise_after(&flag, Duration::from_millis(100));
    let result = sb.eval::<()>(script);
    let ended = Instant::now();
    let raised = raiser.join().unwrap();
    assert_eq!(result, Err(SandboxError::Cancelled), "{script}");
    let took = ended.saturating_duration_since(raised);
    assert!(took < scaled(BOUND), "{script}: took {took:?} to stop");
    // The sandbox works again as soon as the flag is down.
    flag.store(false, Ordering::Release);
    assert_eq!(sb.eval::<i32>("6 * 7"), Ok(42));
}

#[test]
fn stops_a_plain_loop() {
    cancels("for (;;) {}");
}

#[test]
fn stops_a_loop_that_catches_everything() {
    cancels("for (;;) { try { for (;;) {} } catch (e) {} finally { continue } }");
}

#[test]
fn stops_a_loop_with_an_expensive_body() {
    // The deadline's bug class: a poll every few thousand slow iterations.
    cancels(
        "const o = []; for (let i = 0; i < 20000; i++) o.push({ i, s: 'x' + i }); \
         for (;;) JSON.stringify(o)",
    );
}

#[test]
fn stops_a_microtask_flood() {
    cancels("(function f() { Promise.resolve().then(f); Promise.resolve().then(f); })()");
}

#[test]
fn stops_a_loop_inside_a_promise_constructor() {
    // The constructor turns exceptions into rejections; the stop stays sticky.
    cancels("new Promise(() => { for (;;) {} }); for (;;) {}");
}

#[test]
fn stops_a_native_loop() {
    cancels("const s = 'a'.repeat(1 << 20); for (;;) s.indexOf('a'.repeat(1 << 12) + 'b')");
}

#[test]
fn cancelled_wins_over_a_passed_deadline() {
    let limits = Limits::default().with_time(scaled(Duration::from_millis(50)));
    let mut sb = Sandbox::new(limits).unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    sb.set_cancel_flag(Some(flag.clone()));
    let deadline = sb.deadline();
    let raised = flag.clone();
    // A host function is native code: the script can't stop until it returns,
    // so both stop reasons are in by then.
    let result: Result<(), _> = sb.run(move |ctx| {
        let host = Function::new(ctx.clone(), move || {
            while !deadline.passed() {
                thread::sleep(Duration::from_millis(1));
            }
            raised.store(true, Ordering::Release);
            thread::sleep(Duration::from_millis(100));
        })?;
        host.call::<_, ()>(())?;
        ctx.eval::<(), _>("for (;;) {}")
    });
    assert_eq!(result, Err(SandboxError::Cancelled));
}

#[test]
fn a_cancelled_call_discards_the_script_state() {
    let (mut sb, flag) = sandbox();
    sb.eval::<()>("globalThis.kept = 1").unwrap();
    let raiser = raise_after(&flag, Duration::from_millis(50));
    assert_eq!(sb.eval::<()>("for (;;) {}"), Err(SandboxError::Cancelled));
    raiser.join().unwrap();
    flag.store(false, Ordering::Release);
    assert_eq!(sb.eval::<String>("typeof kept").unwrap(), "undefined");
}

#[test]
fn a_raised_flag_refuses_calls_without_touching_state() {
    let (mut sb, flag) = sandbox();
    sb.eval::<()>("globalThis.kept = 1").unwrap();
    flag.store(true, Ordering::Release);
    assert_eq!(
        sb.eval::<()>("globalThis.kept = 2"),
        Err(SandboxError::Cancelled)
    );
    flag.store(false, Ordering::Release);
    assert_eq!(sb.eval::<i32>("kept"), Ok(1));
}

#[test]
fn the_flag_survives_a_reset_and_a_limit_error() {
    let limits = Limits::default().with_time(scaled(Duration::from_millis(50)));
    let mut sb = Sandbox::new(limits).unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    sb.set_cancel_flag(Some(flag.clone()));

    sb.reset().unwrap();
    assert_eq!(sb.eval::<()>("for (;;) {}"), Err(SandboxError::TimeLimit));
    flag.store(true, Ordering::Release);
    assert_eq!(sb.eval::<i32>("1"), Err(SandboxError::Cancelled));
}

#[test]
fn the_flag_survives_a_cancelled_call() {
    let (mut sb, flag) = sandbox();
    for _ in 0..3 {
        let raiser = raise_after(&flag, Duration::from_millis(20));
        assert_eq!(sb.eval::<()>("for (;;) {}"), Err(SandboxError::Cancelled));
        raiser.join().unwrap();
        flag.store(false, Ordering::Release);
    }
}

#[test]
fn removing_the_flag_ends_its_effect() {
    let (mut sb, flag) = sandbox();
    flag.store(true, Ordering::Release);
    sb.set_cancel_flag(None);
    assert_eq!(sb.eval::<i32>("1 + 1"), Ok(2));
}

#[test]
fn a_flag_that_stays_down_changes_nothing() {
    let (mut sb, _flag) = sandbox();
    // Long enough for the watchdog to look at the flag many times.
    let script = "let x = 0; for (let i = 0; i < 3e6; i++) x += i; x";
    let with_flag: f64 = sb.eval(script).unwrap();
    let mut plain = Sandbox::new(Limits::default()).unwrap();
    assert_eq!(with_flag, plain.eval::<f64>(script).unwrap());
}

#[test]
fn a_flag_raised_after_the_call_ended_changes_nothing() {
    let (mut sb, flag) = sandbox();
    let v: i32 = sb.eval("1 + 1").unwrap();
    flag.store(true, Ordering::Release);
    assert_eq!(v, 2);
    flag.store(false, Ordering::Release);
    assert_eq!(sb.eval::<i32>("2 + 2"), Ok(4));
}

#[test]
fn the_time_limit_still_applies_with_a_flag_down() {
    let limits = Limits::default().with_time(scaled(Duration::from_millis(50)));
    let mut sb = Sandbox::new(limits).unwrap();
    sb.set_cancel_flag(Some(Arc::new(AtomicBool::new(false))));
    assert_eq!(sb.eval::<()>("for (;;) {}"), Err(SandboxError::TimeLimit));
}

#[test]
fn a_flag_stops_only_its_own_sandbox() {
    let bystander = thread::spawn(|| {
        let (mut sb, _flag) = sandbox();
        sb.eval::<f64>("let x = 0; for (let i = 0; i < 2e7; i++) x += 1; x")
    });
    let (mut sb, flag) = sandbox();
    let raiser = raise_after(&flag, Duration::from_millis(1));
    assert_eq!(sb.eval::<()>("for (;;) {}"), Err(SandboxError::Cancelled));
    raiser.join().unwrap();
    assert_eq!(bystander.join().unwrap(), Ok(2e7));
}

#[test]
fn host_functions_see_a_cancelled_call() {
    let (mut sb, flag) = sandbox();
    let deadline = sb.deadline();
    let raiser = raise_after(&flag, Duration::from_millis(20));
    let result: Result<(), _> = sb.run(move |ctx| {
        let host = Function::new(ctx.clone(), move || {
            // Native code the engine can't interrupt: only `passed` ends this.
            while !deadline.passed() {
                thread::sleep(Duration::from_millis(1));
            }
        })?;
        host.call::<_, ()>(())?;
        ctx.eval::<(), _>("for (;;) {}")
    });
    raiser.join().unwrap();
    assert_eq!(result, Err(SandboxError::Cancelled));
}
