# quickjs-ng: the interrupt handler is asked only every 10,000 polls, and some native calls never poll

Draft for Kay to file at https://github.com/quickjs-ng/quickjs (issue first,
then a PR if the maintainers like the shape). Against quickjs-ng 0.16.2 as
bundled in rquickjs-sys 0.14.0. surimono carries the patch in
`vendor/rquickjs-sys` (patches 6 to 10 in its `PATCHES.md`, with the exact
diff) until upstream has an equivalent. It builds on the earlier draft,
`quickjs-ng-sticky-interrupt-and-native-loop-polls.md`.

## Problem

An embedder that enforces a time limit sets an interrupt handler
(`JS_SetInterruptHandler`) that compares a clock against a deadline. Two
gaps let one-line scripts run for minutes past the deadline even though
the handler would say "stop" at once.

**A. The handler is asked only every `JS_INTERRUPT_COUNTER_INIT` (10,000)
polls.** That is fine when the code between polls is cheap, as in a plain
`for (;;) {}`. But when a loop body makes one expensive native call, the
loop polls only two or three times per iteration, so the deadline is
looked at every 3,000 to 5,000 iterations, however long each takes:

```js
const o = []; for (let i = 0; i < 20000; i++) o.push({ i, s: 'x' + i });
for (;;) JSON.stringify(o)                                   // 187 s at -O2
const a = new Array(1e6).fill(1); for (;;) a.indexOf(2)      //  49 s
const s = 'ab'.repeat(5e5); for (;;) s.indexOf('abc')        //  67 s
```

(Time from the deadline to the stop with a 250 ms limit, measured in C at
-O2 with the 10,000-poll handler.) The interval can't simply be lowered:
the handler typically reads a clock, so asking it on every poll would cost
far more than the poll itself. Making the interval configurable
(`JS_SetInterruptCounterInit`) helps a little but trades one bad number for
another: whatever the interval, some body is expensive enough.

**B. Some native calls with superlinear cost never poll.**

- `String.prototype.indexOf`, `lastIndexOf`, `includes`, and the search in
  `split`, `replace` and `replaceAll` with a string pattern are naive,
  up to n * m character comparisons, in one native call:
  `'a'.repeat(1 << 20).indexOf('a'.repeat(1 << 12) + 'b')` takes 21.5 s at
  -O2. With a 64 MiB heap, days are possible.
- `replaceAll` with a one-character pattern loops once per match without
  calling script code: `'a'.repeat(1 << 26).replaceAll('a', '')` makes 64M
  passes (9 s in a debug build).
- BigInt multiply, divide, `toString` and decimal parsing are quadratic.
  Near the 1 Mbit size cap one operation takes 0.4 s, 1.1 s, 11 s and 0.3 s
  at -O2, and straight-line code (`s = '' + b; s = '' + b; ...`) never polls
  between operations, so a few KB of source run for hours.

## Proposal

1. **An interrupt flag read on every poll.** A new API,

   ```c
   /* Every poll also loads *flag (relaxed atomic) and calls the interrupt
      handler while it is nonzero. The host sets it from any thread and
      clears it; it must outlive the runtime. NULL removes it. */
   void JS_SetInterruptFlag(JSRuntime *rt, uint32_t *flag);
   ```

   lets the embedder keep a cheap timer (one watchdog thread, a timer
   wheel, `timer_create`) that sets the flag at the deadline. The poll
   becomes

   ```c
   if (unlikely(--ctx->interrupt_counter <= 0 ||
                atomic_load_explicit(ctx->interrupt_flag, memory_order_relaxed)))
       return __js_poll_interrupts(ctx);
   ```

   `JSRuntime` and `JSContext` hold a pointer to the flag (the context's
   is a copy, one load closer to the poll). Until the host sets one it
   points at a static zero, so there's no NULL check, and embedders that
   don't use it see no change in behaviour. The counter and the handler
   stay as they are. Where the compiler has no C11 atomics (tcc, wasi,
   emscripten) there are no threads to set the flag from, and a `volatile`
   read does.

   An alternative with the same effect is `JS_RequestInterrupt(JSRuntime
   *)`, callable from any thread, with the atomic inside `JSRuntime`.
   That is simpler to document but makes the embedder synchronise the
   call with `JS_FreeRuntime`; a host-owned flag avoids that.

2. **Polls in the superlinear native loops.** With the flag, a poll is a
   decrement and a load, so it can go where the work is:
   - the string searches count comparisons and poll every 65,536;
     `string_indexof` takes `ctx` and returns -2 after an interrupt;
     `replace`/`replaceAll` and `split` also poll once per match;
   - `js_mp_mul_basecase` polls once per row and `js_mp_divnorm` once per
     quotient limb (both take `ctx` and return -1 after an interrupt;
     `js_bigint_mul` and `js_bigint_divrem` free their buffers and fail);
     decimal `toString` polls once per limb divided out, decimal parsing
     once per limb multiplied in.

   Results are unchanged. Without the flag these polls still help: the
   handler is asked every 10,000 of them, a few milliseconds of work.

3. **Typed-array sort.** If the earlier draft's per-comparison poll in the
   default `%TypedArray%.prototype.sort` is taken, check the flag instead
   of the counter, through one wrapper per element type, so the comparator
   stays a direct, inlinable call and the check is loads only. One generic
   wrapper polling the counter on every comparison cost 15-26% of a large
   sort; this costs 8-11% (release build, 4M elements, sort only: Int32Array
   567 ms unpolled, 680 ms generic wrapper, 631 ms per-type wrapper with the
   flag). The rest is the extra call layer, since `rqsort` may be the C
   library's `qsort_r`.

## Repro test

With quickjs-ng's own API (fits `api-test.c`). A thread sets the flag at
the deadline; without the patch only the handler's own clock check, every
10,000 polls, can stop these scripts.

```c
static _Atomic uint32_t flag;
static int64_t deadline;

static int interrupt_after_deadline(JSRuntime *rt, void *opaque)
{
    return flag || js__hrtime_ms() > deadline;
}

static void *watchdog(void *arg)
{
    while (js__hrtime_ms() < deadline)
        usleep(1000);
    flag = 1;
    return NULL;
}

static void interrupts_are_time_accurate(void)
{
    static const char *scripts[] = {
        "const o = []; for (let i = 0; i < 20000; i++) o.push({ i, s: 'x' + i });"
        "for (;;) JSON.stringify(o)",
        "const a = new Array(1e6).fill(1); for (;;) a.indexOf(2)",
        "const s = 'ab'.repeat(5e5); for (;;) s.indexOf('abc')",
        "'a'.repeat(1 << 20).indexOf('a'.repeat(1 << 12) + 'b')",
        "'a'.repeat(1 << 26).replaceAll('a', '')",
        "const b = (1n << 1000000n) - 1n; let s; s = '' + b; s = '' + b; s = '' + b",
        "const b = (1n << 500000n) - 1n; let x;"
        "x = b * b; x = b * b; x = b * b; x = b * b; x = b * b; x = b * b",
    };
    for (size_t i = 0; i < countof(scripts); i++) {
        JSRuntime *rt = JS_NewRuntime();
        JSContext *ctx = JS_NewContext(rt);
        pthread_t t;
        flag = 0;
        deadline = js__hrtime_ms() + 100;
        JS_SetInterruptHandler(rt, interrupt_after_deadline, NULL);
        JS_SetInterruptFlag(rt, (uint32_t *)&flag);
        pthread_create(&t, NULL, watchdog, NULL);
        JSValue ret = JS_Eval(ctx, scripts[i], strlen(scripts[i]), "<input>",
                              JS_EVAL_TYPE_GLOBAL);
        pthread_join(t, NULL);
        // Without the patch these return seconds to hours late.
        assert(JS_IsException(ret));
        assert(js__hrtime_ms() < deadline + 100);
        JS_FreeValue(ctx, JS_GetException(ctx));
        JS_FreeContext(ctx);
        JS_FreeRuntime(rt);
    }
}
```

surimono's version runs these and more in child processes with a watchdog
(groups D and E of `crates/core/tests/hostile.rs`).

## Results in surimono

Time limit 250 ms, debug build (QuickJS at -O1), each script in a child
process killed after 10 s:

| Script | Unpatched | Patched |
|---|---|---|
| `for (;;) JSON.stringify(o)`, 20k objects | killed at 10 s | 314 ms |
| `for (;;) a.indexOf(2)`, 1M items | killed at 10 s | 265 ms |
| `for (;;) s.indexOf('abc')`, 1 MB | killed at 10 s | 252 ms |
| `for (;;) { try { a.join(',') } catch {} }`, 100k | killed at 10 s | 259 ms |
| `for (;;) t.includes(1)`, 16 MB `Uint8Array` | 1,261 ms | 257 ms |
| string `indexOf`/`lastIndexOf`/`includes`/`split`/`replace`/`replaceAll`, 1M chars, 4K pattern | killed at 10 s | 251–255 ms |
| `replaceAll('a', '')` on 64M chars | 9,198 ms | 311–1,496 ms (freeing 64 MB on a loaded machine) |
| BigInt `toString`, multiply, divide, parse near the cap, straight-line | killed at 10 s | 251–255 ms |

Cost of the flag check (release build, -O3, best of 5, CPU time):

| Benchmark | Unpatched | Patched |
|---|---|---|
| `for` loop, 3e7 iterations | 960 ms | 973–1,007 ms |
| 1e7 calls | 576–596 ms | 595–633 ms |
| `JSON.stringify` of 20k objects, 50 times | 864–871 ms | 873–908 ms |
| default `Array#sort`, 300k numbers | 253–265 ms | 251–257 ms |

The differences are within the noise of a shared 2-core machine: one
L1-resident load and a predictable branch per poll.
