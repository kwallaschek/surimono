//! Same input, same output: everything a script can read from the host is
//! pinned by `SandboxOptions`.

use std::process::Command;
use std::time::Duration;

use surimono_core::{Limits, Sandbox, SandboxError, SandboxOptions};

fn sandbox(options: SandboxOptions) -> Sandbox {
    Sandbox::with_options(options).unwrap()
}

fn default_sandbox() -> Sandbox {
    sandbox(SandboxOptions::default())
}

fn tokyo() -> SandboxOptions {
    SandboxOptions::default().with_utc_offset_minutes(9 * 60)
}

/// A script that reads every source of non-determinism we know of.
const EVERYTHING: &str = r#"
const out = [];
for (let i = 0; i < 5; i++) out.push(Math.random());
out.push(Date.now(), new Date().toISOString(), String(new Date()), Date(), `${new Date()}`);
const d = new Date(2026, 4, 17, 13, 45, 30, 250);
out.push(d.toISOString(), d.getHours(), d.getDay(), d.getTimezoneOffset(),
         d.toString(), d.toLocaleString(), d.toLocaleDateString(), d.toLocaleTimeString(),
         d.toDateString(), d.toTimeString(), d.toUTCString(), JSON.stringify(d));
d.setHours(23, 59);
out.push(d.toISOString());
d.setDate(40);
out.push(d.toISOString(), d.getMonth());
out.push(Date.parse('2026-03-04T05:06:07'), Date.parse('2026-03-04'),
         Date.parse('2026-03-04T05:06:07Z'), Date.parse('2026-03-04T05:06:07+02:00'),
         new Date('2026-03-04T05:06').getTime(), new Date(0).getHours());
out.push(performance.now(), performance.timeOrigin);
const ref = new WeakRef({ kept: true });
out.push(ref.deref().kept);
new FinalizationRegistry(() => { throw new Error('never') }).register({}, 1);
// A typical chart-library pattern: random ids and timestamps in output.
out.push('id-' + Math.random().toString(36).slice(2), new Date().getFullYear());
JSON.stringify(out)
"#;

#[test]
fn math_random_is_seeded() {
    let seq = |seed| {
        let mut sb = sandbox(SandboxOptions::default().with_random_seed(seed));
        sb.eval::<Vec<f64>>("Array.from({ length: 1000 }, Math.random)")
            .unwrap()
    };
    let a = seq(1);
    assert_eq!(a, seq(1), "same seed, same sequence");
    assert_ne!(a, seq(2), "different seed, different sequence");
    assert!(a.iter().all(|x| (0.0..1.0).contains(x)));
    // Roughly uniform: the mean of 1000 draws is near 0.5.
    let mean = a.iter().sum::<f64>() / a.len() as f64;
    assert!((0.45..0.55).contains(&mean), "mean {mean}");
    // Seed 0 still works (the generator's state is never all zero).
    assert!(seq(0).iter().any(|x| *x != 0.0));
}

#[test]
fn every_context_restarts_the_sequence() {
    let mut sb = default_sandbox();
    let first: f64 = sb.eval("Math.random()").unwrap();
    let second: f64 = sb.eval("Math.random()").unwrap();
    assert_ne!(first, second);
    sb.reset().unwrap();
    assert_eq!(sb.eval::<f64>("Math.random()").unwrap(), first);
}

#[test]
fn clock_is_fixed_and_configurable() {
    let mut sb = default_sandbox();
    let now: f64 = sb.eval("Date.now()").unwrap();
    assert_eq!(now, 1_767_225_600_000.0);
    assert_eq!(
        sb.eval::<String>("new Date().toISOString()").unwrap(),
        "2026-01-01T00:00:00.000Z"
    );
    assert_eq!(
        sb.eval::<String>("Date()").unwrap(),
        "Thu Jan 01 2026 00:00:00 GMT+0000 (Coordinated Universal Time)"
    );
    // A loop waiting for time to pass sees none.
    assert_eq!(
        sb.eval::<f64>("const t = Date.now(); for (let i = 0; i < 1e5; i++) {} Date.now() - t")
            .unwrap(),
        0.0
    );

    let mut later = sandbox(SandboxOptions::default().with_now_ms(1_000));
    assert_eq!(
        later
            .eval::<String>("new Date().toISOString() + ' ' + performance.timeOrigin")
            .unwrap(),
        "1970-01-01T00:00:01.000Z 1000"
    );
}

#[test]
fn local_time_is_utc_by_default() {
    let mut sb = default_sandbox();
    let checks = [
        (
            "new Date(2026, 4, 17, 13, 45, 30, 250).toISOString()",
            "2026-05-17T13:45:30.250Z",
        ),
        (
            "new Date(2026, 4, 17, 13, 45).toString()",
            "Sun May 17 2026 13:45:00 GMT+0000 (Coordinated Universal Time)",
        ),
        (
            "new Date(2026, 4, 17, 13, 45).toLocaleString()",
            "5/17/2026, 1:45:00\u{202f}PM",
        ),
        (
            "new Date(2026, 0, 1).toLocaleTimeString()",
            "12:00:00\u{202f}AM",
        ),
        ("new Date(2026, 4, 17).toDateString()", "Sun May 17 2026"),
        ("'' + new Date(0).getTimezoneOffset()", "0"),
        ("'' + new Date(NaN).getHours()", "NaN"),
        ("String(new Date(NaN))", "Invalid Date"),
        ("new Date(99, 0).toISOString()", "1999-01-01T00:00:00.000Z"),
        // ECMA-262 DateString pads the year to four digits after the sign.
        (
            "new Date(Date.UTC(-1, 0)).toString().split(' ')[3]",
            "-0001",
        ),
        (
            "new Date(Date.UTC(812, 0)).toDateString().split(' ')[3]",
            "0812",
        ),
    ];
    for (src, want) in checks {
        assert_eq!(sb.eval::<String>(src).unwrap(), want, "{src}");
    }
}

#[test]
fn local_time_follows_the_configured_offset() {
    let mut sb = sandbox(tokyo());
    let checks = [
        (
            "new Date(2026, 0, 1, 9).toISOString()",
            "2026-01-01T00:00:00.000Z",
        ),
        ("new Date().toString()", "Thu Jan 01 2026 09:00:00 GMT+0900"),
        ("'' + new Date().getHours()", "9"),
        ("'' + new Date().getTimezoneOffset()", "-540"),
        (
            "const d = new Date(); d.setHours(0); d.toISOString()",
            "2025-12-31T15:00:00.000Z",
        ),
        (
            "const e = new Date(NaN); e.setFullYear(2026); e.toISOString()",
            "2025-12-31T15:00:00.000Z",
        ),
        ("'' + new Date(NaN).setHours(1)", "NaN"),
        (
            "new Date(Date.parse('2026-01-01T09:00:00')).toISOString()",
            "2026-01-01T00:00:00.000Z",
        ),
        (
            "new Date('2026-01-01').toISOString()",
            "2026-01-01T00:00:00.000Z",
        ),
        (
            "new Date('2026-01-01T09:00:00Z').toISOString()",
            "2026-01-01T09:00:00.000Z",
        ),
        ("new Date(2026, 0, 1).toLocaleDateString()", "1/1/2026"),
    ];
    for (src, want) in checks {
        assert_eq!(sb.eval::<String>(src).unwrap(), want, "{src}");
    }
    let mut west = sandbox(SandboxOptions::default().with_utc_offset_minutes(-(5 * 60 + 30)));
    assert_eq!(
        west.eval::<String>("new Date().toTimeString()").unwrap(),
        "18:30:00 GMT-0530"
    );
}

#[test]
fn date_shim_behaves_like_date() {
    let mut sb = default_sandbox();
    let checks = [
        ("typeof Date", "function"),
        ("Date.name + Date.length", "Date7"),
        ("String(new Date() instanceof Date)", "true"),
        ("String(Date.prototype.constructor === Date)", "true"),
        (
            "Object.prototype.toString.call(new Date())",
            "[object Date]",
        ),
        (
            "class D extends Date { year() { return this.getFullYear() } } '' + new D().year() + (new D() instanceof D)",
            "2026true",
        ),
        ("'' + new Date(new Date(5)).getTime()", "5"),
        ("'' + new Date(86400000).getDate()", "2"),
        ("'' + Date.UTC(2026, 0)", "1767225600000"),
        ("typeof Date.now + typeof Date.parse", "functionfunction"),
        ("'' + (new Date() - new Date(0))", "1767225600000"),
        (
            "try { Date.prototype.getHours.call({}) } catch (e) { e.constructor.name }",
            "TypeError",
        ),
        ("'' + new Date(2026, 0, 1).getYear()", "126"),
        ("Object.keys(globalThis).includes('Date') + ''", "false"),
    ];
    for (src, want) in checks {
        assert_eq!(sb.eval::<String>(src).unwrap(), want, "{src}");
    }
}

#[test]
fn performance_is_virtual() {
    let mut sb = default_sandbox();
    assert_eq!(
        sb.eval::<String>("performance.now() + ' ' + performance.timeOrigin")
            .unwrap(),
        "0 1767225600000"
    );
}

#[test]
fn weak_refs_are_strong_and_cleanup_never_runs() {
    let mut sb = default_sandbox();
    let v: String = sb
        .eval(
            r#"
            globalThis.cleaned = 0;
            const reg = new FinalizationRegistry(() => { cleaned++ });
            let refs = [];
            for (let i = 0; i < 1000; i++) {
              const o = { i };
              refs.push(new WeakRef(o));
              reg.register(o, i);
            }
            new Array(100000).fill(0).map(() => ({}));
            refs.every((r, i) => r.deref().i === i) + ' ' + Object.prototype.toString.call(refs[0])
            "#,
        )
        .unwrap();
    assert_eq!(v, "true [object WeakRef]");
    assert_eq!(sb.eval::<i32>("cleaned").unwrap(), 0);
    assert!(matches!(
        sb.eval::<()>("new WeakRef(1)").unwrap_err(),
        SandboxError::Exception { .. }
    ));
}

#[test]
fn console_exists_and_is_silent() {
    let mut sb = default_sandbox();
    sb.eval::<()>("console.log('x'); console.warn('y'); console.error(new Error('z'))")
        .unwrap();
}

#[test]
fn two_runs_are_byte_identical() {
    let a = default_sandbox().eval::<String>(EVERYTHING).unwrap();
    let b = default_sandbox().eval::<String>(EVERYTHING).unwrap();
    assert_eq!(a, b);
    // And in a reused sandbox after a reset, and after other work.
    let mut sb = default_sandbox();
    sb.eval::<()>("for (let i = 0; i < 1000; i++) Math.random(); globalThis.x = new Date(5)")
        .unwrap();
    sb.reset().unwrap();
    assert_eq!(sb.eval::<String>(EVERYTHING).unwrap(), a);
    // And after a limit error rebuilt the runtime.
    let mut limited = sandbox(
        SandboxOptions::default()
            .with_limits(Limits::default().with_time(std::time::Duration::from_millis(50))),
    );
    assert_eq!(
        limited.eval::<()>("for (;;) {}"),
        Err(SandboxError::TimeLimit)
    );
    assert_eq!(limited.eval::<String>(EVERYTHING).unwrap(), a);
}

#[test]
fn invalid_options_are_rejected() {
    for bad in [
        SandboxOptions::default().with_utc_offset_minutes(15 * 60),
        SandboxOptions::default().with_utc_offset_minutes(-13 * 60),
        SandboxOptions::default().with_now_ms(i64::MAX),
        SandboxOptions::default().with_limits(Limits::default().with_time(Duration::ZERO)),
        SandboxOptions::default().with_limits(Limits::default().with_memory_bytes(usize::MAX)),
    ] {
        assert!(matches!(
            Sandbox::with_options(bad).unwrap_err(),
            SandboxError::InvalidOptions { .. }
        ));
    }
}

/// Output of `EVERYTHING` in the default and Tokyo sandboxes, plus the host's
/// own idea of the zone (to prove the child really ran in another one).
fn tz_report() -> String {
    let host = {
        use surimono_core::rquickjs::{Context, Runtime};
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        ctx.with(|ctx| {
            ctx.eval::<i32, _>("new Date(0).getTimezoneOffset()")
                .unwrap()
        })
    };
    let utc = default_sandbox().eval::<String>(EVERYTHING).unwrap();
    let jst = sandbox(tokyo()).eval::<String>(EVERYTHING).unwrap();
    format!("host-offset={host}\n{utc}\n{jst}")
}

/// Run by `output_ignores_the_host_time_zone` in a child process.
#[test]
#[ignore = "helper: run in a child process with TZ set"]
fn tz_child() {
    println!("TZ-REPORT-BEGIN\n{}\nTZ-REPORT-END", tz_report());
}

#[test]
fn output_ignores_the_host_time_zone() {
    let ours = tz_report();
    let ours_body = ours.split_once('\n').unwrap().1.to_owned();
    // POSIX zone strings need no tzdata, so they work in minimal containers.
    for (tz, host_offset) in [("JST-9", "-540"), ("EST5EDT,M3.2.0,M11.1.0", "300")] {
        let out = Command::new(std::env::current_exe().unwrap())
            .args([
                "tz_child",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("TZ", tz)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "child failed: {stdout}");
        let report = stdout
            .split("TZ-REPORT-BEGIN\n")
            .nth(1)
            .and_then(|s| s.split("\nTZ-REPORT-END").next())
            .unwrap_or_else(|| panic!("no report in: {stdout}"));
        let (host, body) = report.split_once('\n').unwrap();
        assert_eq!(
            host,
            format!("host-offset={host_offset}"),
            "TZ={tz} took effect"
        );
        assert_eq!(body, ours_body, "TZ={tz} changed the output");
    }
}
