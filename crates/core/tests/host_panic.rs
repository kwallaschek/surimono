//! A panic in a host function ends the call with a typed error, whatever the
//! script does with the exception it sees.

use surimono_core::rquickjs::{Ctx, Function};
use surimono_core::{Limits, Sandbox, SandboxError};

/// Runs `script` with a global `boom()` whose body panics.
fn with_boom(sb: &mut Sandbox, script: &str) -> Result<(), SandboxError> {
    let deadline = sb.deadline();
    sb.run(|ctx| {
        let boom = Function::new(ctx.clone(), move |ctx: Ctx<'_>| {
            deadline.guard(&ctx, || -> surimono_core::rquickjs::Result<()> {
                panic!("host exploded")
            })
        })?;
        ctx.globals().set("boom", boom)?;
        ctx.eval::<(), _>(script)
    })
}

fn panics(script: &str) {
    let mut sb = Sandbox::new(Limits::default()).unwrap();
    sb.eval::<()>("globalThis.kept = 1").unwrap();
    match with_boom(&mut sb, script) {
        Err(SandboxError::HostPanic { message }) => assert_eq!(message, "host exploded"),
        other => panic!("{script}: {other:?}"),
    }
    assert_eq!(
        sb.eval::<String>("typeof kept").unwrap(),
        "undefined",
        "{script}: the state is discarded"
    );
    assert_eq!(sb.eval::<i32>("6 * 7"), Ok(42), "{script}");
}

#[test]
fn an_uncaught_panic_is_typed() {
    panics("boom()");
}

#[test]
fn a_caught_panic_still_fails_the_call() {
    panics("try { boom() } catch (e) {} 'carried on'");
}

#[test]
fn a_catching_loop_stops_at_the_next_poll() {
    panics("for (;;) { try { boom() } catch (e) {} }");
}

#[test]
fn a_finally_cannot_hide_it() {
    panics("(function () { try { boom() } finally { return 1 } })()");
}

#[test]
fn swallowing_built_ins_cannot_hide_it() {
    panics("new Promise(() => boom()); 1");
    panics("(async () => { boom() })().catch(() => {})");
    panics("Promise.resolve().then(() => boom()).catch(() => {})");
}

#[test]
fn a_host_function_that_does_not_panic_changes_nothing() {
    let mut sb = Sandbox::new(Limits::default()).unwrap();
    let deadline = sb.deadline();
    let out: f64 = sb
        .run(|ctx| {
            let add = Function::new(ctx.clone(), move |ctx: Ctx<'_>, a: f64, b: f64| {
                deadline.guard(&ctx, || Ok(a + b))
            })?;
            ctx.globals().set("add", add)?;
            ctx.eval("add(1, 2)")
        })
        .unwrap();
    assert_eq!(out, 3.0);
}
