//! Hostile scripts: each way found so far to hang the engine or overrun
//! the time limit, each run in a child process with a watchdog, so a
//! regression fails the test instead of hanging CI.
//!
//! Group A loops over a huge array-like inside a native built-in; group B
//! has a built-in swallow the time-limit interrupt while an outer loop runs
//! on; group C is a long native sort; group D is a loop whose body is one
//! expensive native call, which the engine used to check the deadline for
//! only every few thousand iterations; group E is a single native call
//! with superlinear cost (naive string search, BigInt arithmetic near the
//! size cap) that never polled. Before the QuickJS patches in
//! `vendor/rquickjs-sys` (see its PATCHES.md) every vector here except the
//! controls hung or overshot the limit by seconds to minutes. Each must now
//! end within about twice the time limit: with the expected limit error,
//! or for the TOCTOU vectors (whose trick only fooled the old prelude
//! guards) normally.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use surimono_core::rquickjs::ArrayBuffer;
use surimono_core::{Limits, Sandbox, SandboxError};

/// Time limit for every vector.
const LIMIT: Duration = Duration::from_millis(250);
/// A child still running after this is hung and gets killed.
const WATCHDOG: Duration = Duration::from_secs(10);

/// Allowed time for a vector's call: twice the limit, plus slack for a
/// debug build on a busy 2-core machine. The call's time includes stopping
/// the script and the rebuild after the limit error, which frees the
/// vector's heap (up to 128 MiB); process start and setup are not
/// included. AddressSanitizer (the nightly job, which sets ASAN_OPTIONS)
/// makes every allocation and free several times slower, so it gets more.
fn bound() -> Duration {
    let slack = if std::env::var_os("ASAN_OPTIONS").is_some() {
        Duration::from_millis(2000)
    } else {
        Duration::from_millis(500)
    };
    2 * LIMIT + slack
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Want {
    /// `SandboxError::TimeLimit`.
    Time,
    /// `TimeLimit` or `MemoryLimit`, whichever comes first.
    AnyLimit,
    /// Any outcome, as long as it ends in time.
    Finish,
}

#[derive(Debug, Clone, Copy)]
enum Setup {
    None,
    /// A global `buf`: an ArrayBuffer of this many pseudo-random int32s,
    /// made in Rust because a JS loop to fill it would outlast the limit.
    RandomInt32(usize),
}

struct Vector {
    name: &'static str,
    source: &'static str,
    want: Want,
    setup: Setup,
}

const fn v(name: &'static str, source: &'static str, want: Want) -> Vector {
    Vector {
        name,
        source,
        want,
        setup: Setup::None,
    }
}

/// Group A: native loops over array-likes (review of #5, vectors 1 to 4, and
/// the vectors the old prelude guards refused).
const GROUP_A: &[Vector] = &[
    v(
        "A1 join, length getter small then huge",
        "let n = 0; Array.prototype.join.call({ get length() { return n++ ? 2 ** 40 : 1 } }, '')",
        Want::Finish,
    ),
    v(
        "A1 copyWithin, length getter small then huge",
        "let n = 0; Array.prototype.copyWithin.call({ get length() { return n++ ? 2 ** 40 : 1 } }, 0, 1)",
        Want::Finish,
    ),
    v(
        "A1 join, length getter huge",
        "Array.prototype.join.call({ get length() { return 2 ** 40 } }, '')",
        Want::Time,
    ),
    v(
        "A2 join, length valueOf small then huge",
        "let n = 0; Array.prototype.join.call({ length: { valueOf() { return n++ ? 2 ** 40 : 1 } } }, '')",
        Want::Finish,
    ),
    v(
        "A2 join, length valueOf huge",
        "Array.prototype.join.call({ length: { valueOf() { return 2 ** 40 } } }, '')",
        Want::Time,
    ),
    v(
        "A3 join, typed array with an own length",
        "const t = new Uint8Array(1); Object.defineProperty(t, 'length', { value: 2 ** 40 }); Array.prototype.join.call(t, '')",
        Want::Time,
    ),
    v(
        "A3 reverse, DataView with an own length",
        "const d = new DataView(new ArrayBuffer(8)); Object.defineProperty(d, 'length', { value: 2 ** 40 }); Array.prototype.reverse.call(d)",
        Want::Time,
    ),
    v(
        "A4 fill, typed array on the prototype chain",
        "const o = Object.create(new Uint8Array(0)); Object.defineProperty(o, 'length', { value: 2 ** 32 }); Array.prototype.fill.call(o, 0)",
        Want::Time,
    ),
    v(
        "A4 fill, typed array with an own length",
        "const t = new Uint8Array(1); Object.defineProperty(t, 'length', { value: 2 ** 40 }); Array.prototype.fill.call(t, 0)",
        Want::Time,
    ),
    v(
        "A fill, plain object",
        "Array.prototype.fill.call({ length: 2 ** 40 }, 0)",
        Want::AnyLimit,
    ),
    v(
        "A join, plain object",
        "Array.prototype.join.call({ length: 2 ** 40 }, '')",
        Want::Time,
    ),
    v(
        "A join, sparse array",
        "new Array(2 ** 32 - 1).join('')",
        Want::Time,
    ),
    v(
        "A toString, sparse array",
        "String(new Array(2 ** 32 - 1))",
        Want::AnyLimit,
    ),
    v(
        "A toLocaleString",
        "Array.prototype.toLocaleString.call({ length: 2 ** 40 })",
        Want::AnyLimit,
    ),
    v(
        "A reverse",
        "Array.prototype.reverse.call({ length: 2 ** 40 })",
        Want::Time,
    ),
    v(
        "A copyWithin",
        "Array.prototype.copyWithin.call({ length: 2 ** 40 }, 0, 1)",
        Want::Time,
    ),
    v(
        "A sort",
        "Array.prototype.sort.call({ length: 2 ** 32 - 1 })",
        Want::Time,
    ),
    v(
        "A splice",
        "Array.prototype.splice.call({ length: 2 ** 40 }, 0, 1)",
        Want::Time,
    ),
    v(
        "A shift",
        "Array.prototype.shift.call({ length: 2 ** 40 })",
        Want::Time,
    ),
    v(
        "A unshift",
        "Array.prototype.unshift.call({ length: 2 ** 40 }, 1)",
        Want::Time,
    ),
    v(
        "A slice",
        "Array.prototype.slice.call({ length: 2 ** 40 }, 0, 2 ** 32 - 1)",
        Want::Finish,
    ),
    v(
        "A concat, spreadable array-like",
        "[].concat.call({ length: 2 ** 40, [Symbol.isConcatSpreadable]: true })",
        Want::Time,
    ),
    v(
        "A concat, sparse argument",
        "[].concat(new Array(2 ** 32 - 2))",
        Want::Time,
    ),
    v(
        "A flat, array-like",
        "Array.prototype.flat.call({ length: 2 ** 40 })",
        Want::Time,
    ),
    v(
        "A flat, nested",
        "[new Array(2 ** 32 - 1)].flat()",
        Want::Time,
    ),
    v(
        "A flat, deeper",
        "[[new Array(2 ** 32 - 1)]].flat(2)",
        Want::Time,
    ),
    v(
        "A flatMap",
        "[1].flatMap(() => new Array(2 ** 32 - 1))",
        Want::Time,
    ),
    v(
        "A JSON.stringify, array replacer",
        "JSON.stringify({}, new Array(2 ** 32 - 1))",
        Want::Time,
    ),
    v(
        "A String.raw",
        "String.raw({ raw: { length: 2 ** 40 } })",
        Want::Time,
    ),
    v(
        "A lastIndexOf (polled upstream; control)",
        "Array.prototype.lastIndexOf.call({ length: 2 ** 40 }, 1)",
        Want::Time,
    ),
    v(
        "A toReversed",
        "Array.prototype.toReversed.call({ length: 2 ** 32 - 1 })",
        Want::Finish,
    ),
    v(
        "A toSpliced",
        "Array.prototype.toSpliced.call({ length: 2 ** 32 - 1 }, 0, 0)",
        Want::Finish,
    ),
];

/// Group B: built-ins that swallow the interrupt (review of #5, vectors 5 to
/// 9, and the other swallowing sites it named).
const GROUP_B: &[Vector] = &[
    v(
        "B5 new Promise",
        "for (;;) { new Promise(() => { for (;;) {} }) }",
        Want::Time,
    ),
    v(
        "B6 Promise.try",
        "for (;;) { Promise.try(() => { for (;;) {} }) }",
        Want::Time,
    ),
    v(
        "B7 async generator",
        "for (;;) { (async function* () { for (;;) {} })().next() }",
        Want::Time,
    ),
    v(
        "B8 Error.prepareStackTrace",
        "Error.prepareStackTrace = () => { for (;;) {} }; for (;;) { try { null.x } catch (e) {} }",
        Want::Time,
    ),
    v(
        "B9 DisposableStack",
        "for (;;) { try { const s = new DisposableStack(); s.defer(() => { for (;;) {} }); s.defer(() => { for (;;) {} }); s.dispose() } catch (e) {} }",
        Want::Time,
    ),
    v(
        "B async function",
        "for (;;) { (async () => { for (;;) {} })() }",
        Want::Time,
    ),
    v(
        "B Promise.all, iterator next",
        "for (;;) { Promise.all({ [Symbol.iterator]() { return { next() { for (;;) {} } } } }) }",
        Want::Time,
    ),
    v(
        "B Promise.race, iterator next",
        "for (;;) { Promise.race({ [Symbol.iterator]() { return { next() { for (;;) {} } } } }) }",
        Want::Time,
    ),
    v(
        "B thenable getter",
        "for (;;) { Promise.resolve({ get then() { for (;;) {} } }) }",
        Want::Time,
    ),
    v(
        "B Array.fromAsync",
        "for (;;) { Array.fromAsync({ length: 1, 0: 1 }, () => { for (;;) {} }) }",
        Want::Time,
    ),
    v(
        "B swallowed, then straight-line code",
        "new Promise(() => { for (;;) {} }); 'finished after the swallow'",
        Want::Time,
    ),
];

/// Group C: long native sorts (review of #5: 12M int32s overshot a 100 ms
/// limit by 2.4 s).
const GROUP_C: &[Vector] = &[
    Vector {
        name: "C Int32Array sort, 12M random",
        source: "new Int32Array(buf).sort().length",
        want: Want::Time,
        setup: Setup::RandomInt32(12_000_000),
    },
    Vector {
        name: "C Float64Array sort, 6M random",
        source: "new Float64Array(buf).sort().length",
        want: Want::Time,
        setup: Setup::RandomInt32(12_000_000),
    },
    Vector {
        name: "C Int32Array sort with a comparator (control)",
        source: "new Int32Array(buf).sort((a, b) => a - b).length",
        want: Want::Time,
        setup: Setup::RandomInt32(12_000_000),
    },
    Vector {
        name: "C Array sort, 1M numbers as strings",
        source: "Array.prototype.slice.call(new Int32Array(buf)).sort().length",
        want: Want::AnyLimit,
        setup: Setup::RandomInt32(1_000_000),
    },
];

/// Group D: a loop around one expensive native call (review of #32, R4).
/// The engine asked for the deadline only every 10,000 polls, and these
/// bodies poll two or three times per iteration, so the deadline was seen
/// every few thousand iterations: still running at 15 s with a 250 ms
/// limit (187 s, 49 s and 67 s in C at -O2).
const GROUP_D: &[Vector] = &[
    v(
        "D JSON.stringify of 20k objects, in a loop",
        "const o = []; for (let i = 0; i < 20000; i++) o.push({ i, s: 'x' + i }); for (;;) JSON.stringify(o)",
        Want::Time,
    ),
    v(
        "D Array indexOf on 1M items, in a loop",
        "const a = new Array(1e6).fill(1); for (;;) a.indexOf(2)",
        Want::Time,
    ),
    v(
        "D String indexOf on 1 MB, in a loop",
        "const s = 'ab'.repeat(5e5); for (;;) s.indexOf('abc')",
        Want::Time,
    ),
    v(
        "D TypedArray includes on 16 MB, in a loop",
        "const t = new Uint8Array(1 << 24); for (;;) t.includes(1)",
        Want::Time,
    ),
    v(
        "D Array join of 100k, caught, in a loop",
        "const a = new Array(1e5).fill('ab'); for (;;) { try { a.join(',') } catch (e) {} }",
        Want::Time,
    ),
];

/// Group E: one native call with superlinear cost that never polled
/// (review of #32, R5). String searches are naive, up to n * m character
/// comparisons; BigInt multiply, divide, toString and parsing are
/// quadratic in the size, up to seconds per operation near the 1 Mbit cap,
/// and straight-line code (built here with `eval`) never polls between
/// them.
const GROUP_E: &[Vector] = &[
    v(
        "E String indexOf, 1M chars, 4K pattern",
        "'a'.repeat(1 << 20).indexOf('a'.repeat(1 << 12) + 'b')",
        Want::Time,
    ),
    v(
        "E String lastIndexOf, 1M chars, 4K pattern",
        "'a'.repeat(1 << 20).lastIndexOf('a'.repeat(1 << 12) + 'b')",
        Want::Time,
    ),
    v(
        "E String includes, 1M chars, 4K pattern",
        "'a'.repeat(1 << 20).includes('a'.repeat(1 << 12) + 'b')",
        Want::Time,
    ),
    v(
        "E String split, 1M chars, 4K separator",
        "'a'.repeat(1 << 20).split('a'.repeat(1 << 12) + 'b')",
        Want::Time,
    ),
    v(
        "E String replace, 1M chars, 4K pattern",
        "'a'.repeat(1 << 20).replace('a'.repeat(1 << 12) + 'b', '')",
        Want::Time,
    ),
    v(
        "E String replaceAll, 1M chars, 4K pattern",
        "'a'.repeat(1 << 20).replaceAll('a'.repeat(1 << 12) + 'b', '')",
        Want::Time,
    ),
    v(
        "E String replaceAll, one pass per match, 64M matches",
        "'a'.repeat(1 << 26).replaceAll('a', '')",
        Want::AnyLimit,
    ),
    v(
        "E BigInt toString near the cap, straight-line",
        "const b = (1n << 1000000n) - 1n; let s; eval('s = String(b);'.repeat(8))",
        Want::Time,
    ),
    v(
        "E BigInt multiply near the cap, straight-line",
        "const b = (1n << 500000n) - 1n; let x; eval('x = b * b;'.repeat(64))",
        Want::Time,
    ),
    v(
        "E BigInt divide near the cap, straight-line",
        "const a = (1n << 1000000n) - 1n, d = (1n << 500000n) - 3n; let q; eval('q = a / d;'.repeat(16))",
        Want::Time,
    ),
    v(
        "E BigInt parse near the cap, straight-line",
        "const t = '9'.repeat(300000); let x; eval('x = BigInt(t);'.repeat(16))",
        Want::Time,
    ),
    v("E BigInt power near the cap", "3n ** 600000n", Want::Time),
];

fn all_vectors() -> impl Iterator<Item = &'static Vector> {
    GROUP_A
        .iter()
        .chain(GROUP_B)
        .chain(GROUP_C)
        .chain(GROUP_D)
        .chain(GROUP_E)
}

fn pseudo_random(n: usize) -> Vec<i32> {
    let mut x: u32 = 0x9e37_79b9;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as i32
        })
        .collect()
}

/// Child side: runs the vector named in SURIMONO_HOSTILE and prints the
/// outcome, the time it took and whether the sandbox still works.
#[test]
#[ignore = "helper: run in a child process by the hostile tests"]
fn hostile_child() {
    let name = std::env::var("SURIMONO_HOSTILE").unwrap();
    let vector = all_vectors().find(|v| v.name == name).unwrap();
    let limits = Limits::default()
        .with_time(LIMIT)
        .with_memory_bytes(128 * 1024 * 1024);
    let mut sb = Sandbox::new(limits).unwrap();
    // Setup gets a call of its own, so copying the data doesn't count.
    if let Setup::RandomInt32(n) = vector.setup {
        let data = pseudo_random(n);
        sb.run(|ctx| {
            ctx.globals()
                .set("buf", ArrayBuffer::new(ctx.clone(), data)?)
        })
        .unwrap();
    }
    let start = Instant::now();
    let result: Result<(), SandboxError> = sb.run(|ctx| ctx.eval::<(), _>(vector.source));
    let took = start.elapsed();
    let after = sb.eval::<i32>("6 * 7");
    println!(
        "\nHOSTILE-RESULT {:?}|{}|{:?}",
        result,
        took.as_millis(),
        after
    );
}

/// Runs each vector in a child with a watchdog and checks the outcomes.
fn run_group(group: &[Vector]) {
    let exe = std::env::current_exe().unwrap();
    let mut report = Vec::new();
    let mut failures = Vec::new();
    for vector in group {
        let mut child = Command::new(&exe)
            .args([
                "hostile_child",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("SURIMONO_HOSTILE", vector.name)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            if started.elapsed() > WATCHDOG {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut stdout = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut stdout)
            .unwrap();
        let Some(status) = status else {
            failures.push(format!("{}: HUNG, killed after {WATCHDOG:?}", vector.name));
            continue;
        };
        let line = stdout
            .lines()
            .find_map(|l| l.split_once("HOSTILE-RESULT ").map(|(_, r)| r))
            .map(str::to_owned);
        let Some(line) = line.filter(|_| status.success()) else {
            let mut stderr = String::new();
            let _ = child.stderr.take().unwrap().read_to_string(&mut stderr);
            failures.push(format!(
                "{}: child failed ({status:?}): {stdout} {stderr}",
                vector.name
            ));
            continue;
        };
        let mut parts = line.splitn(3, '|');
        let (result, ms, after) = (
            parts.next().unwrap_or(""),
            parts
                .next()
                .unwrap_or("")
                .parse::<u64>()
                .unwrap_or(u64::MAX),
            parts.next().unwrap_or(""),
        );
        report.push(format!("{:>6} ms  {:<16} {}", ms, result, vector.name));
        let outcome_ok = match vector.want {
            Want::Time => result == "Err(TimeLimit)",
            Want::AnyLimit => result == "Err(TimeLimit)" || result == "Err(MemoryLimit)",
            Want::Finish => true,
        };
        if !outcome_ok {
            failures.push(format!(
                "{}: wanted {:?}, got {result}",
                vector.name, vector.want
            ));
        }
        if Duration::from_millis(ms) > bound() {
            failures.push(format!("{}: took {ms} ms, over {:?}", vector.name, bound()));
        }
        if after != "Ok(42)" {
            failures.push(format!(
                "{}: sandbox broken afterwards: {after}",
                vector.name
            ));
        }
    }
    println!("{}", report.join("\n"));
    assert!(
        failures.is_empty(),
        "{} of {} vectors failed:\n{}\n\nall results:\n{}",
        failures.len(),
        group.len(),
        failures.join("\n"),
        report.join("\n")
    );
}

#[test]
fn group_a_native_array_like_loops_end() {
    run_group(GROUP_A);
}

#[test]
fn group_b_swallowed_interrupts_end() {
    run_group(GROUP_B);
}

#[test]
fn group_c_long_native_sorts_end() {
    run_group(GROUP_C);
}

#[test]
fn group_d_loops_around_expensive_native_calls_end() {
    run_group(GROUP_D);
}

#[test]
fn group_e_superlinear_native_calls_end() {
    run_group(GROUP_E);
}
