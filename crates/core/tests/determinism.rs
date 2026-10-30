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
// Strings in zones QuickJS knows by name, and local times in a DST gap
// and overlap of the host's zone (EST5EDT: 2026-03-08 and 2026-11-01).
out.push(Date.parse('Jan 1 2026 10:00 PST'), Date.parse('Jul 4 2026 10:00 EDT'),
         Date.parse('Jan 1 2026 10:00 CET'), Date.parse('Thu Jan 01 2026 09:00:00 GMT+0900'),
         Date.parse('Jan 1 2026 10:00'), Date.parse('1/2/2026 3:04:05 PM'),
         Date.parse('2026-03-08T02:30:00'), Date.parse('2026-11-01T01:30:00'),
         Date.parse('Mar 8 2026 02:30'), Date.parse('Nov 1 2026 01:30'),
         new Date(2026, 2, 8, 2, 30).getTime(), new Date(2026, 10, 1, 1, 30).getTime());
// new Date(value) where value converts to a date string.
out.push(new Date({ toString() { return '2026-01-01T10:00:00' } }).getTime(),
         new Date(new String('2026-01-01T10:00:00')).getTime(),
         new Date(['2026-03-08T02:30:00']).getTime(),
         new Date({ [Symbol.toPrimitive]() { return 'Nov 1 2026 01:30' } }).getTime());
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
    // The host offset is the zone's on 1970-01-01 (summer in the south).
    for (tz, host_offset) in [
        ("UTC0", "0"),
        ("JST-9", "-540"),
        ("EST5EDT,M3.2.0,M11.1.0", "300"),
        ("CET-1CEST,M3.5.0,M10.5.0/3", "-60"),
        ("AEST-10AEDT,M10.1.0,M4.1.0/3", "-660"),
        ("IST-5:30", "-330"),
    ] {
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

/// Runs `source` on a worker thread and fails instead of hanging if it takes
/// longer than `within`. (A hung engine thread is left behind; the test
/// process exits anyway once the harness is done.)
fn eval_within(
    options: SandboxOptions,
    source: &'static str,
    within: Duration,
) -> Result<String, SandboxError> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let mut sb = Sandbox::with_options(options).unwrap();
            let _ = tx.send(sb.eval::<String>(source));
        })
        .unwrap();
    rx.recv_timeout(within)
        .unwrap_or_else(|_| panic!("still running after {within:?}: {source}"))
}

/// Review of #8, B1: quickjs-ng's V8-style `Error.prepareStackTrace` gave
/// scripts `CallSite.getFunction()` for native frames, including the real
/// `Date` behind the shim (the host's clock and zone).
#[test]
fn stack_trace_api_is_off() {
    let mut sb = default_sandbox();
    let v: String = sb
        .eval(
            r#"
            const found = [];
            Error.prepareStackTrace = (e, sites) => {
              for (const site of sites) {
                const f = site.getFunction();
                if (typeof f === 'function') found.push(f);
              }
              return 'custom';
            };
            let stacks = '';
            try { new Date({ valueOf() { throw new Error('x') } }) } catch (e) { stacks += e.stack }
            try { Reflect.construct(Date, [0], new Proxy(function () {}, { get() { throw new Error('y') } })) } catch (e) { stacks += e.stack }
            try { [1].map(() => { throw new Error('z') }) } catch (e) { stacks += e.stack }
            const o = {};
            Error.captureStackTrace(o);
            stacks += o.stack;
            const d = Object.getOwnPropertyDescriptor(Error, 'prepareStackTrace');
            [typeof stacks, found.length, stacks.includes('custom'), 'get' in d,
             typeof Error.prepareStackTrace].join(' ')
            "#,
        )
        .unwrap();
    // Assignments still read back, so libraries that save and restore the
    // hook keep working; it is just never called.
    assert_eq!(v, "string 0 false false function");
}

/// Review of #5, vector 8: a `prepareStackTrace` that loops swallowed the
/// time-limit interrupt on every error and kept the outer loop alive.
#[test]
fn looping_prepare_stack_trace_hits_the_time_limit() {
    let options = SandboxOptions::default()
        .with_limits(Limits::default().with_time(Duration::from_millis(300)));
    let err = eval_within(
        options,
        "Error.prepareStackTrace = () => { for (;;) {} }; for (;;) { try { null.x } catch (e) {} }",
        Duration::from_secs(10),
    )
    .unwrap_err();
    assert_eq!(err, SandboxError::TimeLimit);
}

/// Review of #8, B2: `new Date(value)` with an object that converts to a
/// date string went to the native parser in the host's zone. ECMA-262
/// §21.4.2.1 step 4: a Date's time value is copied; anything else goes
/// through ToPrimitive (§7.1.1, no hint) and a string is parsed.
#[test]
fn date_from_an_object_uses_the_sandbox_zone() {
    for (options, want) in [
        (SandboxOptions::default(), "2026-01-01T10:00:00.000Z"),
        (tokyo(), "2026-01-01T01:00:00.000Z"),
    ] {
        let mut sb = sandbox(options);
        for src in [
            "new Date({ toString() { return '2026-01-01T10:00:00' } })",
            "new Date(new String('2026-01-01T10:00:00'))",
            "new Date(['2026-01-01T10:00:00'])",
            "new Date({ [Symbol.toPrimitive](hint) { return hint === 'default' ? '2026-01-01T10:00:00' : 'wrong' } })",
            "new Date({ valueOf() { return '2026-01-01T10:00:00' }, toString() { return 'wrong' } })",
            "new Date({ valueOf() { return {} }, toString() { return '2026-01-01T10:00:00' } })",
            "new Date({ valueOf: 1, toString() { return '2026-01-01T10:00:00' } })",
        ] {
            assert_eq!(
                sb.eval::<String>(&format!("({src}).toISOString()"))
                    .unwrap(),
                want,
                "{src}"
            );
        }
    }
    let mut sb = sandbox(tokyo());
    let checks = [
        ("'' + new Date({ valueOf() { return 5 } }).getTime()", "5"),
        ("'' + new Date(new Number(7)).getTime()", "7"),
        ("'' + new Date(new Date(5)).getTime()", "5"),
        (
            "class D extends Date {}; '' + new Date(new D(9)).getTime()",
            "9",
        ),
        // A Date's time value is copied without calling its methods.
        (
            "const d = new Date(11); d.valueOf = d.toString = d[Symbol.toPrimitive] = () => 1; '' + new Date(d).getTime()",
            "11",
        ),
        ("'' + new Date(true).getTime()", "1"),
        ("'' + new Date(null).getTime()", "0"),
        ("'' + new Date(undefined).getTime()", "NaN"),
        (
            "let n = 0; new Date({ valueOf() { n++; return 1 } }); '' + n",
            "1",
        ),
        (
            "try { new Date(new Proxy(new Date(5), {})); 'no' } catch (e) { e.constructor.name }",
            "TypeError",
        ),
        (
            "try { new Date(Symbol()); 'no' } catch (e) { e.constructor.name }",
            "TypeError",
        ),
        (
            "try { new Date({ [Symbol.toPrimitive]: 1 }); 'no' } catch (e) { e.constructor.name }",
            "TypeError",
        ),
        (
            "try { new Date({ [Symbol.toPrimitive]() { return {} } }); 'no' } catch (e) { e.constructor.name }",
            "TypeError",
        ),
        (
            "try { new Date(Object.create(null)); 'no' } catch (e) { e.constructor.name }",
            "TypeError",
        ),
    ];
    for (src, want) in checks {
        assert_eq!(sb.eval::<String>(src).unwrap(), want, "{src}");
    }
}

/// Review of #8, B3: zone names QuickJS knows were read as local time in
/// the host's zone. The RFC 2822 §4.3 names have fixed offsets; Chrome
/// honours them too.
#[test]
fn date_parse_honours_named_zones() {
    for options in [SandboxOptions::default(), tokyo()] {
        let mut sb = sandbox(options);
        let checks = [
            "Date.parse('Jan 1 2026 10:00 PST') - Date.UTC(2026, 0, 1, 18)",
            "Date.parse('Jan 1 2026 10:00 PDT') - Date.UTC(2026, 0, 1, 17)",
            "Date.parse('Jan 1 2026 10:00 MST') - Date.UTC(2026, 0, 1, 17)",
            "Date.parse('Jan 1 2026 10:00 MDT') - Date.UTC(2026, 0, 1, 16)",
            "Date.parse('Jan 1 2026 10:00 CST') - Date.UTC(2026, 0, 1, 16)",
            "Date.parse('Jan 1 2026 10:00 CDT') - Date.UTC(2026, 0, 1, 15)",
            "Date.parse('Jan 1 2026 10:00 EST') - Date.UTC(2026, 0, 1, 15)",
            "Date.parse('Jan 1 2026 10:00 EDT') - Date.UTC(2026, 0, 1, 14)",
            "Date.parse('Thu, 01 Jan 2026 10:00:00 GMT') - Date.UTC(2026, 0, 1, 10)",
            "Date.parse('Thu, 01 Jan 2026 10:00:00 +0900') - Date.UTC(2026, 0, 1, 1)",
            "new Date('Jan 1 2026 10:00 PST').getTime() - Date.UTC(2026, 0, 1, 18)",
        ];
        for src in checks {
            assert_eq!(sb.eval::<f64>(src).unwrap(), 0.0, "{src}");
        }
    }
    // Zone-less strings are local time at the sandbox's offset; date-only
    // ISO strings are UTC (ECMA-262 §21.4.3.2).
    for (options, hour) in [(SandboxOptions::default(), 10), (tokyo(), 1)] {
        let mut sb = sandbox(options);
        for src in [
            "Date.parse('Jan 1 2026 10:00')",
            "Date.parse('2026-01-01T10:00:00')",
            "Date.parse('2026-01-01T10:00')",
            "Date.parse('1/1/2026 10:00:00 AM')",
            "Date.parse('Thu Jan 01 2026 10:00:00')",
        ] {
            let got: f64 = sb.eval(src).unwrap();
            let want: f64 = sb.eval(&format!("Date.UTC(2026, 0, 1, {hour})")).unwrap();
            assert_eq!(got, want, "{src}");
        }
        assert_eq!(
            sb.eval::<f64>("Date.parse('2026-01-01') - Date.UTC(2026, 0, 1)")
                .unwrap(),
            0.0
        );
    }
}

/// Strings for the parser comparison: hand-picked forms, then seeded fuzz
/// from fragments that reach every branch of QuickJS's parser.
const PARSE_CORPUS: &str = r#"(() => {
  const out = [
    '2026', '2026-03', '2026-03-04', '+002026-03-04', '-000001-01-01', '-000000-01-01',
    '2026-03-04T05:06', '2026-03-04T05:06:07', '2026-03-04T05:06:07.8', '2026-03-04T05:06:07.123456789',
    '2026-03-04T05:06:07.1234567891', '2026-03-04T05:06:07,5', '2026-03-04T05:06:07Z',
    '2026-03-04T05:06:07+09:00', '2026-03-04T05:06:07-0530', '2026-03-04T05:06:07+9',
    '2026-03-04T24:00', '2026-03-04T24:00:01', '2026-03-04T5:06', '2026-13-01', '2026-00-10',
    '2026-02-31', '2026-03-04T', '2026-03-04t05:06', '0050-01-01T00:00:00', '0099-12-31',
    'Thu Jan 01 2026 09:00:00 GMT+0900 (Japan Standard Time)', 'Thu, 01 Jan 2026 00:00:00 GMT',
    'Jan 1 2026', '1 Jan 2026', 'January 1, 2026 10:00 PM', '1/2/2026', '1/2/26', '12/31/99 23:59:59',
    '2026/1/2 3:04:05.6', 'Jan 1 2026 10:00 PST', 'Jan 1 2026 10:00 pst', 'Jan 1 2026 10:00 CEST',
    'Jan 1 2026 10:00 WEST', 'Jan 1 2026 10:00 UT', 'Jan 1 2026 10:00 Z', 'Jan 1 2026 10:00 zz',
    'Jan 1 2026 10:00 ESTX', 'Jan 1 2026 10:00 -05:00', 'Jan 1 2026 10:00 +1', 'Jan 1 2026 10:00 +123',
    'Jan 1 2026 12:00 AM', 'Jan 1 2026 00:00 PM', 'Jan 1 2026 13:00 AM', 'Jan 1 2026 13:00 PM',
    'Jan 1 2026 10:00:00.5 11:00', 'Jan 1 2026 (comment (nested)) 10:00', 'Jan 1 2026 (open',
    'Jan 1 2026 )', 'Wednesday Jan 1 2026', 'Jan 1 2026 Wednesday', '1 2 3', '1 2 3 4', '32 1 1',
    'Jan -1 2026', 'Jan 1 -2026', 'Jan 1 -0', 'Jan 1 +2026 10:00', 'Mar 8 2026 02:30', 'Nov 1 2026 01:30',
    '2026-03-04T05:06:07\u2212' + '05:00', 'Jan 1 2026 10:00 \u2212' + '0500', 'Jan\u00e9 1 2026',
    'Jan 1 2026\u0000 garbage', '\u0000', '', ' ', 'x', 'Jan', '24:00', 'Jan 1 2026 24:00',
    '2026-03-04T05:06:07' + ' '.repeat(120) + 'Z', 'Jan 1 2026' + ' '.repeat(115) + '10:00',
    'Jan 1 2026' + ' '.repeat(110) + '10:00 PST', '999999999', '1000000000 Jan 1', '275760-09-13',
    'Sep 13 275760', 'Sep 14 275760', 'Apr 20 -271821', 'Apr 19 -271821 23:59',
  ];
  const parts = [
    '2026', '0', '1', '01', '12', '13', '31', '32', '99', '100', '123456789', '-', '+', ':', '.',
    ',', '/', ' ', '  ', 'T', 't', 'Z', 'z', '00', '59', '60', '24', '(', ')', '(x)', 'GMT', 'UTC',
    'UT', 'PST', 'pdt', 'CEST', 'WEST', 'EEST', 'Jan', 'feb', 'MAY', 'Sept', 'December', 'AM',
    'PM', 'pm', '+09:00', '-0500', '+9', '-1234', '+123456', '-000000', 'x', 'Thu', 'Wednesday',
    '\u2212', '\u00e9', '\u3042', '\u0000', '2026-03-04', 'T05:06', ':07', '.5',
  ];
  let seed = 20261031;
  const pick = (n) => {
    seed = (Math.imul(seed, 1103515245) + 12345) >>> 0;
    return (seed >>> 8) % n;
  };
  for (let i = 0; i < 30000; i++) {
    let n = 1 + pick(12);
    let s = '';
    while (n-- > 0) s += parts[pick(parts.length)];
    out.push(s);
  }
  return out;
})()"#;

/// Run by `date_parse_matches_quickjs_on_a_utc_host` in a child with TZ=UTC0.
#[test]
#[ignore = "helper: run in a child process with TZ=UTC0"]
fn date_parse_matches_quickjs_child() {
    use surimono_core::rquickjs::{Context, Runtime};
    let script = format!("{PARSE_CORPUS}.map((s) => Date.parse(s))");
    let rt = Runtime::new().unwrap();
    let ctx = Context::full(&rt).unwrap();
    let (host_offsets, corpus, native) = ctx.with(|ctx| {
        let offsets: Vec<i32> = ctx
            .eval("[new Date(0).getTimezoneOffset(), new Date(2026, 6, 1).getTimezoneOffset()]")
            .unwrap();
        let corpus: Vec<String> = ctx.eval(PARSE_CORPUS).unwrap();
        let native: Vec<f64> = ctx.eval(script.as_str()).unwrap();
        (offsets, corpus, native)
    });
    assert_eq!(host_offsets, [0, 0], "the child must run on a UTC host");
    let mut sb = sandbox(
        SandboxOptions::default().with_limits(Limits::default().with_time(Duration::from_secs(60))),
    );
    let ours: Vec<f64> = sb.eval(&script).unwrap();
    assert_eq!(ours.len(), corpus.len());
    let mut mismatches = Vec::new();
    for ((s, a), b) in corpus.iter().zip(&ours).zip(&native) {
        let same = (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits();
        if !same {
            mismatches.push(format!("{s:?}: sandbox {a}, QuickJS {b}"));
        }
    }
    let valid = native.iter().filter(|t| !t.is_nan()).count();
    println!("compared {} strings, {valid} valid", corpus.len());
    assert!(valid > 1000, "the corpus must reach the success paths too");
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches[..mismatches.len().min(20)].join("\n")
    );
}

/// The sandbox's Date.parse is a port of QuickJS's; with offset 0 it must
/// agree with the native parser on a UTC host, string for string.
#[test]
fn date_parse_matches_quickjs_on_a_utc_host() {
    let out = Command::new(std::env::current_exe().unwrap())
        .args([
            "date_parse_matches_quickjs_child",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("TZ", "UTC0")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "child failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("compared "), "{stdout}");
}
