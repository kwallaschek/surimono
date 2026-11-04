//! Hostile scripts: each way found so far to hang the engine or overrun
//! the time limit, each run in a child process with a watchdog, so a
//! regression fails the test instead of hanging CI. The known gaps are in
//! the time-limit notes on `Sandbox`.
//!
//! Group A loops over a huge array-like inside a native built-in; group B
//! has a built-in swallow the time-limit interrupt while an outer loop runs
//! on; group C is a long native sort; group D is a loop whose body is one
//! expensive native call; group E is a single native call whose cost grows
//! faster than its input (string search, BigInt arithmetic near the size
//! cap); group F is a regular expression's per-match loop inside one native
//! call (`replace`, `split`, `match`, `matchAll`); group G is a compile or
//! parse of a huge or quadratic input, which runs no bytecode (source text
//! through `compile`, `eval` and `new Function`, patterns through `RegExp`,
//! `JSON.parse`); group H is a proxy's `ownKeys` result, checked in C after
//! the trap returns. Each must stop within
//! about twice the time limit (timed apart from the cleanup after it, which
//! has a bound of its own): with the expected limit error, or for the
//! TOCTOU vectors (whose trick only fooled the old prelude guards)
//! normally. See `vendor/rquickjs-sys/PATCHES.md` for the engine patches
//! they cover.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use surimono_core::rquickjs::{self, ArrayBuffer};
use surimono_core::test_hooks::{self, CallTimings};
use surimono_core::{Limits, Sandbox, SandboxError};

mod common;
use common::scaled;

/// Bytes of source for the group G vectors that are linear in their input.
const BIG: usize = 64 << 20;

/// Time limit for every vector. Not scaled by `SURIMONO_TEST_TIME_SCALE`:
/// a longer limit would let the group C sorts finish inside it on a fast
/// machine. What the scale stretches is the time allowed to stop.
const LIMIT: Duration = Duration::from_millis(250);

/// A child still running after this is hung and gets killed (10 s, times
/// the test time scale).
fn watchdog() -> Duration {
    scaled(Duration::from_secs(10))
}

/// Allowed time for a vector's script to stop: twice the limit, plus slack
/// for a debug build on a busy 2-core machine. This is the time from arming
/// the deadline until the script has stopped, timed by the sandbox itself
/// (`surimono_core::test_hooks`, a test-only hook), so it leaves out the
/// rebuild after the limit error. The slack covers how often a slow native
/// loop polls (the group C sorts run 100 to 200 ms past the limit) and the
/// scheduler on a loaded machine. AddressSanitizer (the nightly job, which
/// sets ASAN_OPTIONS) slows every allocation, so it gets more, and every
/// bound is multiplied by the test time scale (`SURIMONO_TEST_TIME_SCALE`,
/// which that job also sets).
fn stop_bound() -> Duration {
    let slack = if asan() {
        Duration::from_millis(2000)
    } else {
        Duration::from_millis(500)
    };
    scaled(2 * LIMIT + slack)
}

/// Allowed time from the first "stop" answer of the interrupt check to the
/// call's return: unwinding the script's stack, a few ms at most. A
/// regression that keeps running after being told to stop fails here even
/// when the total is in bounds.
fn unwind_bound() -> Duration {
    scaled(Duration::from_millis(if asan() { 600 } else { 150 }))
}

/// The same for the group C sorts, which by design go on after the stop
/// answer: `rqsort` can't be aborted, so the comparison answers 0 and the
/// sort finishes in one linear pass (patches 3 and 17). That takes 65 to
/// 105 ms for the 12M-element sort in a debug build, and up to about 500 ms
/// on a heavily loaded machine; the stop bound still covers it.
fn sort_unwind_bound() -> Duration {
    stop_bound()
}

/// Allowed time for the whole call: the stop plus the rebuild after the
/// limit error, which frees the vector's heap (up to [`HEAP`]; the one
/// vector with a larger limit is stopped by time long before it fills it);
/// process start and setup are not included. On the fleet's shared machine
/// (a load of 7 to 10 on 2 cores) the 12M-element sort took 440 to 780 ms in
/// all.
fn bound() -> Duration {
    let slack = if asan() {
        Duration::from_millis(2500)
    } else {
        Duration::from_millis(750)
    };
    scaled(2 * LIMIT + slack)
}

fn asan() -> bool {
    std::env::var_os("ASAN_OPTIONS").is_some()
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
    /// A global string `s` with this text, set in a call of its own, so
    /// building it can't use up the limit and let the vector pass unrun.
    Text(fn() -> String),
    /// The script is this text, handed to [`Sandbox::compile`]; the vector's
    /// `source` isn't used.
    Compile(fn() -> String),
    /// The same for [`Sandbox::eval`].
    Eval(fn() -> String),
    /// The global `s` as for `Text`, then each of these scripts in a call of
    /// its own, so a loaded machine can't run the setup out of time.
    TextThen(fn() -> String, &'static [&'static str]),
}

/// Heap limit of every vector that doesn't set its own.
const HEAP: usize = 128 * 1024 * 1024;

struct Vector {
    name: &'static str,
    source: &'static str,
    want: Want,
    setup: Setup,
    /// Heap limit in bytes.
    heap: usize,
}

const fn v(name: &'static str, source: &'static str, want: Want) -> Vector {
    Vector {
        name,
        source,
        want,
        setup: Setup::None,
        heap: HEAP,
    }
}

impl Vector {
    /// This vector with `setup`.
    const fn with(mut self, setup: Setup) -> Self {
        self.setup = setup;
        self
    }

    /// This vector for input cheap enough per byte that a fast machine may
    /// get through it within the limit: only the time to stop counts.
    const fn any_outcome(mut self) -> Self {
        self.want = Want::Finish;
        self
    }

    /// This vector with a heap limit of `bytes` instead of [`HEAP`].
    const fn with_heap(mut self, bytes: usize) -> Self {
        self.heap = bytes;
        self
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
        // The callback only runs in a microtask, so the outer loop just piles
        // up pending promises: a fast machine filled the 128 MiB heap before
        // the 250 ms limit (macos-14: MemoryLimit, stopped at 219 ms). In a
        // debug build on the fleet's machine the pile fills 128 MiB in 0.68 s
        // and 1 GiB in 4.2 s, so 1 GiB takes macos-14 over 1.3 s: the time
        // limit must end it first (P1-RT-43). The interrupt handler also
        // checks the clock on the script's own thread, so a starved watchdog
        // can't let the pile win; a MemoryLimit here is a regression.
        Want::Time,
    )
    .with_heap(1 << 30),
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
        heap: HEAP,
    },
    Vector {
        name: "C Float64Array sort, 6M random",
        source: "new Float64Array(buf).sort().length",
        want: Want::Time,
        setup: Setup::RandomInt32(12_000_000),
        heap: HEAP,
    },
    Vector {
        name: "C Int32Array sort with a comparator (control)",
        source: "new Int32Array(buf).sort((a, b) => a - b).length",
        want: Want::Time,
        setup: Setup::RandomInt32(12_000_000),
        heap: HEAP,
    },
    Vector {
        name: "C Array sort, 1M numbers as strings",
        source: "Array.prototype.slice.call(new Int32Array(buf)).sort().length",
        want: Want::AnyLimit,
        setup: Setup::RandomInt32(1_000_000),
        heap: HEAP,
    },
];

/// Group D: a loop around one expensive native call (review of #32, R4;
/// the interrupt flag, patch 14).
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

/// Group E: native calls whose cost grows faster than their input (review
/// of #32, R5; the polls of patches 15 and 16). Straight-line code is built
/// with `eval`.
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
        "E String replaceAll, one pass per match, 16M matches",
        "'a'.repeat(1 << 24).replaceAll('a', '')",
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
    // One power took 164 ms on a GitHub runner: repeat it so it can't finish
    // inside the limit on a fast machine.
    v(
        "E BigInt power near the cap, straight-line",
        "let x; eval('x = 3n ** 600000n;'.repeat(16))",
        Want::Time,
    ),
];

/// Group F: a regular expression's per-match loop inside one native call
/// (P1-RT-23; patch 19). Each match of an empty pattern takes a few steps,
/// so the regexp engine's own poll (every 10,000 steps of one `lre_exec`)
/// never comes, and the loop must poll per match. The strings are sized
/// so each call runs well over the limit on a fast machine, and so a
/// version that collects every match stays under the memory limit long
/// enough to need the time limit (or else hits the memory limit:
/// `AnyLimit`).
const GROUP_F: &[Vector] = &[
    // Review of #47: stopped at 1,174 ms against the 250 ms limit, because
    // replace with '' on a standard global regexp takes a fast path
    // (JS_RegExpDelete) that calls the matcher directly. Unpolled it runs
    // to its end in about 0.9 s here, inside the bound, so the vectors
    // after it make the same loop longer.
    v(
        "F RegExp replace with '', empty pattern, 32M chars",
        "'a'.repeat(1 << 25).replace(/(?:)/g, '')",
        Want::Time,
    ),
    // The same fast path with an 8-way alternation: a few dozen steps per
    // match, 64M matches, an empty result. Unpolled it ran 4.5 s (debug
    // build, loaded 2-core machine), well past the bound.
    v(
        "F RegExp replace with '', alternation, 64M chars",
        "'a'.repeat(1 << 26).replace(/(?:x|y|z|w|v|u|t|a)/g, '')",
        Want::Time,
    ),
    v(
        "F RegExp replace with '', sticky alternation, 64M chars",
        "'a'.repeat(1 << 26).replace(/(?:x|y|z|w|v|u|t|a)/gy, '')",
        Want::Time,
    ),
    v(
        "F String replaceAll with a regexp and '', 64M chars",
        "'a'.repeat(1 << 26).replaceAll(/(?:x|y|z|w|v|u|t|a)/g, '')",
        Want::Time,
    ),
    v(
        "F RegExp replace with a string, empty pattern, 4M chars",
        "'a'.repeat(1 << 22).replace(/(?:)/g, 'b')",
        Want::AnyLimit,
    ),
    v(
        "F RegExp replace with a function, empty pattern, 4M chars",
        "'a'.repeat(1 << 22).replace(/(?:)/g, () => '')",
        Want::AnyLimit,
    ),
    v(
        "F RegExp split, never matches, 16M chars",
        "'a'.repeat(1 << 24).split(/(?!)/)",
        Want::Time,
    ),
    v(
        "F RegExp split, empty pattern, 4M chars",
        "'a'.repeat(1 << 22).split(/(?:)/)",
        Want::AnyLimit,
    ),
    v(
        "F RegExp match, global, empty pattern, 4M chars",
        "'a'.repeat(1 << 22).match(/(?:)/g)",
        Want::AnyLimit,
    ),
    v(
        "F RegExp matchAll, spread, empty pattern, 4M chars",
        "[...'a'.repeat(1 << 22).matchAll(/(?:)/g)]",
        Want::AnyLimit,
    ),
];

fn all_vectors() -> impl Iterator<Item = &'static Vector> {
    GROUP_A
        .iter()
        .chain(GROUP_B)
        .chain(GROUP_C)
        .chain(GROUP_D)
        .chain(GROUP_E)
        .chain(GROUP_F)
        .chain(GROUP_G)
        .chain(GROUP_H)
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
        .with_memory_bytes(vector.heap);
    let mut sb = Sandbox::new(limits).unwrap();
    // Setup gets a call of its own, so copying the data doesn't count.
    let script = match vector.setup {
        Setup::RandomInt32(n) => {
            let data = pseudo_random(n);
            sb.run(|ctx| {
                ctx.globals()
                    .set("buf", ArrayBuffer::new(ctx.clone(), data)?)
            })
            .unwrap();
            None
        }
        Setup::Text(text) => {
            let text = text();
            sb.run(|ctx| {
                ctx.globals()
                    .set("s", rquickjs::String::from_str(ctx.clone(), &text)?)
            })
            .unwrap();
            None
        }
        Setup::TextThen(text, then) => {
            let text = text();
            sb.run(|ctx| {
                ctx.globals()
                    .set("s", rquickjs::String::from_str(ctx.clone(), &text)?)
            })
            .unwrap();
            for script in then {
                sb.eval::<()>(script).unwrap();
            }
            None
        }
        Setup::Compile(text) | Setup::Eval(text) => Some(text()),
        Setup::None => None,
    };
    test_hooks::take_last_call();
    let start = Instant::now();
    let result: Result<(), SandboxError> = match (vector.setup, script) {
        (Setup::Compile(_), Some(script)) => sb.compile(&script, "hostile.js").map(drop),
        (Setup::Eval(_), Some(script)) => sb.eval(&script),
        _ => sb.run(|ctx| ctx.eval::<(), _>(vector.source)),
    };
    let took = start.elapsed();
    let timings = test_hooks::take_last_call();
    let after = sb.eval::<i32>("6 * 7");
    // A missing hook record prints u128::MAX, which fails the stop bound.
    let ms = |d: Option<Duration>| d.map_or(0, |d| d.as_millis());
    let (stop_ms, unwind_ms, cleanup_ms) = match timings {
        Some(CallTimings {
            run,
            unwind,
            cleanup,
        }) => (run.as_millis(), ms(unwind), ms(cleanup)),
        None => (u128::MAX, u128::MAX, u128::MAX),
    };
    println!(
        "\nHOSTILE-RESULT {:?}|{}|{}|{}|{}|{:?}",
        result,
        took.as_millis(),
        stop_ms,
        unwind_ms,
        cleanup_ms,
        after
    );
}

/// Runs each vector in a child with a watchdog and checks the outcomes.
fn run_group(group: &[Vector], unwind_bound: Duration) {
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
            if started.elapsed() > watchdog() {
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
            failures.push(format!(
                "{}: HUNG, killed after {:?}",
                vector.name,
                watchdog()
            ));
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
        let mut parts = line.splitn(6, '|');
        let mut text = || parts.next().unwrap_or("");
        let ms = |s: &str| s.parse::<u64>().unwrap_or(u64::MAX);
        let result = text();
        let (total, stop, unwind, cleanup) = (ms(text()), ms(text()), ms(text()), ms(text()));
        let after = text();
        report.push(format!(
            "{total:>6} ms (stop {stop:>5}, unwind {unwind:>3}, cleanup {cleanup:>5})  {result:<16} {}",
            vector.name
        ));
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
        if Duration::from_millis(stop) > stop_bound() {
            failures.push(format!(
                "{}: took {stop} ms to stop, over {:?}",
                vector.name,
                stop_bound()
            ));
        }
        if Duration::from_millis(unwind) > unwind_bound {
            failures.push(format!(
                "{}: took {unwind} ms to unwind after the stop answer, over {:?}",
                vector.name, unwind_bound
            ));
        }
        if Duration::from_millis(total) > bound() {
            failures.push(format!(
                "{}: took {total} ms with the cleanup ({cleanup} ms), over {:?}",
                vector.name,
                bound()
            ));
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

/// Text for the group G setups: a plain `fn`, so vectors stay `const`.
mod text {
    use super::BIG;

    /// Statements in a function nothing calls: all the cost is the compile.
    pub fn statements() -> String {
        format!("function f(){{{}}}", "a=1;".repeat(1 << 22))
    }
    pub fn top_level_vars() -> String {
        "var a=1;".repeat(1 << 21)
    }
    pub fn array_literal() -> String {
        format!("function f(){{return[{}]}}", "1,".repeat(1 << 23))
    }
    /// 1 MiB of source, quadratic: each name walks every variable in scope
    /// when the variables are resolved, after the parser is done.
    pub fn many_lets() -> String {
        let lets: String = (0..60_000)
            .map(|i| format!("let a{i}=a{};", i / 2))
            .collect();
        format!("function f(){{{lets}}}")
    }
    /// The same at the top level: each `var` searches the ones before it.
    pub fn many_vars() -> String {
        (0..1 << 17).map(|i| format!("var a{i};")).collect()
    }
    pub fn block_comment() -> String {
        format!("/*{}*/", "x".repeat(BIG))
    }
    pub fn line_comment() -> String {
        format!("//{}", "x".repeat(BIG))
    }
    pub fn html_comment() -> String {
        format!("<!--{}", "x".repeat(BIG))
    }
    pub fn shebang() -> String {
        format!("#!{}", "x".repeat(BIG))
    }
    pub fn spaces() -> String {
        " ".repeat(BIG)
    }
    pub fn newlines() -> String {
        "\n".repeat(BIG)
    }
    pub fn string_literal() -> String {
        format!("'{}'", "x".repeat(BIG))
    }
    pub fn template() -> String {
        format!("`{}`", "x".repeat(BIG))
    }
    pub fn tagged_template() -> String {
        format!("t`{}`", "x".repeat(BIG))
    }
    pub fn regexp_literal() -> String {
        format!("/{}/", "x".repeat(BIG))
    }
    pub fn regexp_flags() -> String {
        format!("/a/{}", "x".repeat(BIG))
    }
    pub fn identifier() -> String {
        "x".repeat(BIG)
    }
    pub fn number() -> String {
        "1".repeat(BIG)
    }
    /// `async` makes the parser look past the comment for `function`.
    pub fn peek_past_comment() -> String {
        format!("async/*{}*/function f(){{}}", "x".repeat(BIG))
    }
    pub fn alternatives() -> String {
        "x|".repeat(1 << 21)
    }
    pub fn empty_alternatives() -> String {
        "|".repeat(1 << 22)
    }
    pub fn terms() -> String {
        "(?:x)".repeat(3 << 20)
    }
    pub fn quantifiers() -> String {
        "(?:x{2,3})*".repeat(1 << 20)
    }
    pub fn class_items() -> String {
        let items: String = (0..1 << 16)
            .map(|i| format!("\\u{{{:x}}}", i * 3))
            .collect();
        format!("[{items}]")
    }
    pub fn class_items_literal() -> String {
        format!("/{}/u", class_items())
    }
    pub fn backreferences() -> String {
        format!("{}(?<n>x)", "\\k<n>".repeat(1 << 20))
    }
    pub fn json_numbers() -> String {
        format!("[{}1]", "1,".repeat(1 << 23))
    }
    pub fn json_objects() -> String {
        format!("[{}1]", "{\"a\":1},".repeat(1 << 21))
    }
    pub fn json_string() -> String {
        format!("\"{}\"", "x".repeat(BIG))
    }
    pub fn json_spaces() -> String {
        format!("{}1", " ".repeat(BIG))
    }
    pub fn json_digits() -> String {
        "1".repeat(BIG)
    }
    pub fn empty_statements() -> String {
        ";".repeat(BIG)
    }
}

/// Group G (P2-RT-03): compiles and parses that run no bytecode. The heap is
/// large enough that time, not memory, must end them.
const GROUP_G_HEAP: usize = 1 << 30;

const fn compile(name: &'static str, text: fn() -> String) -> Vector {
    v(name, "", Want::Time)
        .with(Setup::Compile(text))
        .with_heap(GROUP_G_HEAP)
}

const fn compile_cheap(name: &'static str, text: fn() -> String) -> Vector {
    compile(name, text).any_outcome()
}

const fn with_text(name: &'static str, source: &'static str, text: fn() -> String) -> Vector {
    v(name, source, Want::Time)
        .with(Setup::Text(text))
        .with_heap(GROUP_G_HEAP)
}

const GROUP_G: &[Vector] = &[
    compile("G compile, statements", text::statements),
    compile("G compile, top-level vars", text::top_level_vars),
    compile("G compile, array literal", text::array_literal),
    compile("G compile, empty statements", text::empty_statements),
    compile("G compile, 60,000 lets (resolve pass)", text::many_lets),
    compile("G compile, 131,072 vars", text::many_vars),
    compile_cheap("G compile, block comment", text::block_comment),
    compile_cheap("G compile, line comment", text::line_comment),
    compile_cheap("G compile, HTML comment", text::html_comment),
    compile_cheap("G compile, shebang", text::shebang),
    compile_cheap("G compile, spaces", text::spaces),
    compile_cheap("G compile, newlines", text::newlines),
    compile("G compile, string literal", text::string_literal),
    compile("G compile, template", text::template),
    compile("G compile, tagged template", text::tagged_template),
    compile("G compile, regexp literal", text::regexp_literal),
    compile("G compile, regexp flags", text::regexp_flags),
    compile("G compile, identifier", text::identifier),
    compile_cheap("G compile, number", text::number),
    compile("G compile, peek past a comment", text::peek_past_comment),
    compile(
        "G compile, regexp literal with a huge class",
        text::class_items_literal,
    ),
    v("G eval, statements", "", Want::Time)
        .with(Setup::Eval(text::statements))
        .with_heap(GROUP_G_HEAP),
    with_text("G direct eval", "eval(s)", text::statements),
    with_text("G indirect eval", "(0, eval)(s)", text::statements),
    with_text("G new Function", "new Function(s)", text::statements),
    with_text(
        "G eval of 60,000 lets, in try",
        "try { eval(s) } catch (e) {}",
        text::many_lets,
    ),
    with_text(
        "G RegExp, alternatives",
        "new RegExp(s)",
        text::alternatives,
    ),
    with_text(
        "G RegExp, empty alternatives",
        "new RegExp(s)",
        text::empty_alternatives,
    ),
    with_text("G RegExp, terms", "new RegExp(s)", text::terms).any_outcome(),
    with_text("G RegExp, quantifiers", "new RegExp(s)", text::quantifiers).any_outcome(),
    with_text("G RegExp, class", "new RegExp(s, 'u')", text::class_items),
    with_text(
        "G RegExp, class ignoring case",
        "new RegExp(s, 'iu')",
        text::class_items,
    ),
    with_text(
        "G RegExp, forward backreferences",
        "new RegExp(s)",
        text::backreferences,
    ),
    with_text("G JSON.parse, numbers", "JSON.parse(s)", text::json_numbers),
    with_text("G JSON.parse, objects", "JSON.parse(s)", text::json_objects),
    with_text("G JSON.parse, string", "JSON.parse(s)", text::json_string).any_outcome(),
    with_text("G JSON.parse, spaces", "JSON.parse(s)", text::json_spaces).any_outcome(),
    with_text("G JSON.parse, digits", "JSON.parse(s)", text::json_digits).any_outcome(),
    with_text(
        "G JSON.parse, numbers, with a reviver",
        "JSON.parse(s, (k, v) => v)",
        text::json_numbers,
    ),
];

/// Keys of the group H setup: enough that one linear search per key takes
/// far over [`stop_bound`] (2.4 s for 98,000 in a release build).
const KEYS: usize = 100_000;

/// A JSON object of [`KEYS`] properties, for the group H setup.
fn keys_json() -> String {
    let keys: Vec<String> = (0..KEYS).map(|i| format!("\"k{i}\":1")).collect();
    format!("{{{}}}", keys.join(","))
}

/// The group H setup: `t` has [`KEYS`] properties, `k` is its keys, `n` is
/// as many numeric strings counting down and `d` as many copies of one
/// string.
const KEYS_SETUP: &[&str] = &[
    "var t = JSON.parse(s); s = ''",
    "var k = Object.keys(t)",
    "var n = k.map((x) => x.slice(1))",
    "n.reverse()",
    "var d = new Array(k.length).fill('x')",
];

/// One vector of group H: `source` in a loop, so it ends by the time limit
/// with one call far shorter than the limit, and fails if one call is long.
const fn keys(name: &'static str, source: &'static str) -> Vector {
    v(name, source, Want::Time)
        .with(Setup::TextThen(keys_json, KEYS_SETUP))
        .with_heap(GROUP_G_HEAP)
}

/// Group H (P2-RT-04): `js_proxy_get_own_property_names` looked up each key
/// in a linear scan in C, once for duplicates and once per key of the target
/// that must be present. The time was spent after the trap had returned, in
/// every path through a proxy's own keys.
const GROUP_H: &[Vector] = &[
    keys(
        "H Reflect.ownKeys",
        "var p = new Proxy({}, { ownKeys: () => k }); for (;;) Reflect.ownKeys(p)",
    ),
    keys(
        "H Object.keys",
        "var p = new Proxy({}, { ownKeys: () => k }); for (;;) Object.keys(p)",
    ),
    keys(
        "H Object.entries",
        "var p = new Proxy({}, { ownKeys: () => k }); for (;;) Object.entries(p)",
    ),
    keys(
        "H Object.getOwnPropertyDescriptors",
        "var p = new Proxy({}, { ownKeys: () => k });
         for (;;) Object.getOwnPropertyDescriptors(p)",
    ),
    keys(
        "H for-in",
        "var p = new Proxy({}, { ownKeys: () => k }); for (;;) for (var x in p) break",
    ),
    keys(
        "H spread",
        "var p = new Proxy({}, { ownKeys: () => k }); for (;;) ({ ...p })",
    ),
    keys(
        "H Object.assign",
        "var p = new Proxy({}, { ownKeys: () => k }); for (;;) Object.assign({}, p)",
    ),
    keys(
        "H JSON.stringify",
        "var p = new Proxy({}, { ownKeys: () => k }); for (;;) JSON.stringify(p)",
    ),
    keys(
        "H proxy of a proxy",
        "var p = new Proxy(new Proxy({}, { ownKeys: () => k }), {}); for (;;) Reflect.ownKeys(p)",
    ),
    keys(
        "H numeric keys",
        "var p = new Proxy({}, { ownKeys: () => n }); for (;;) Reflect.ownKeys(p)",
    ),
    keys(
        "H a duplicate at the end",
        "var a = k.concat('k0'), p = new Proxy({}, { ownKeys: () => a });
         for (;;) try { Reflect.ownKeys(p) } catch (e) {}",
    ),
    keys(
        "H one key throughout",
        "var p = new Proxy({}, { ownKeys: () => d });
         for (;;) try { Reflect.ownKeys(p) } catch (e) {}",
    ),
    keys(
        "H frozen target, every key looked up",
        "var p = new Proxy(Object.freeze(t), { ownKeys: () => k }); for (;;) Reflect.ownKeys(p)",
    ),
    keys(
        "H non-extensible target, every key marked",
        "var p = new Proxy(Object.preventExtensions(t), { ownKeys: () => k });
         for (;;) Reflect.ownKeys(p)",
    ),
    keys(
        "H plain Object.getOwnPropertyDescriptors",
        "for (;;) Object.getOwnPropertyDescriptors(t)",
    ),
    keys("H plain Object.entries", "for (;;) Object.entries(t)"),
];

/// Group G: the interrupt during a compile or parse is the time limit, not
/// an error a script can catch. `caught` is set if it can.
#[test]
fn group_g_compile_interrupts_are_uncatchable() {
    for (name, setup, script) in [
        ("parser", text::statements as fn() -> String, "eval(s)"),
        ("compiler pass", text::many_lets, "eval(s)"),
        ("regexp", text::alternatives, "new RegExp(s)"),
        ("regexp literal", text::class_items_literal, "eval(s)"),
        ("JSON", text::json_numbers, "JSON.parse(s)"),
    ] {
        let limits = Limits::default()
            .with_time(LIMIT)
            .with_memory_bytes(GROUP_G_HEAP);
        let mut sb = Sandbox::new(limits).unwrap();
        let text = setup();
        sb.run(|ctx| {
            ctx.globals()
                .set("s", rquickjs::String::from_str(ctx.clone(), &text)?)
        })
        .unwrap();
        let source =
            format!("var caught = false; try {{ {script} }} catch (e) {{ caught = true }}");
        let mut caught = None;
        let result = sb.run(|ctx| {
            let run = ctx.eval::<(), _>(source.as_str());
            // The stop is pending; this read runs no script code.
            caught = ctx.globals().get::<_, bool>("caught").ok();
            run
        });
        assert_eq!(result, Err(SandboxError::TimeLimit), "{name}");
        assert_eq!(caught, Some(false), "{name}: a script caught the interrupt");
    }
}

#[test]
fn group_a_native_array_like_loops_end() {
    run_group(GROUP_A, unwind_bound());
}

#[test]
fn group_b_swallowed_interrupts_end() {
    run_group(GROUP_B, unwind_bound());
}

#[test]
fn group_c_long_native_sorts_end() {
    run_group(GROUP_C, sort_unwind_bound());
}

#[test]
fn group_d_loops_around_expensive_native_calls_end() {
    run_group(GROUP_D, unwind_bound());
}

#[test]
fn group_e_superlinear_native_calls_end() {
    run_group(GROUP_E, unwind_bound());
}

#[test]
fn group_f_regexp_per_match_loops_end() {
    run_group(GROUP_F, unwind_bound());
}

#[test]
fn group_g_compiles_and_parses_end() {
    run_group(GROUP_G, unwind_bound());
}

#[test]
fn group_h_proxy_own_keys_end() {
    run_group(GROUP_H, unwind_bound());
}
