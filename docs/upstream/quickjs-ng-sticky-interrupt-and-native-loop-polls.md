# quickjs-ng: interrupts can be swallowed, native loops never poll, and the compiler crashes when out of memory

Draft for Kay to file at https://github.com/quickjs-ng/quickjs (issue first;
the patch is small enough for one PR). Against quickjs-ng 0.16.2 as bundled in
rquickjs-sys 0.14.0. surimono carries the patch in `vendor/rquickjs-sys`
(see its `PATCHES.md` for the exact diff) until upstream has an equivalent.

## Problem

An embedder that runs untrusted code sets an interrupt handler
(`JS_SetInterruptHandler`) and expects that once the handler returns true, the
script stops. Three gaps break that with one-line scripts. In each case the
handler already returns true (the deadline has passed), and the script runs
on forever.

**A. Native loops over array-likes don't poll.** `Array.prototype.join`,
`fill`, `reverse`, `copyWithin`, `sort`, `splice`, `shift`, `unshift`,
`concat`, `flat`, `String.raw` and the `JSON.stringify` array replacer walk
`0..length` in C, reading and writing through `JS_TryGetPropertyInt64`,
`JS_GetPropertyInt64`, `JS_SetPropertyInt64` and `JS_DeletePropertyInt64`.
None of those polls, so an array-like with a huge `length` never returns:

```js
Array.prototype.join.call({ length: 2 ** 40 }, '')
// No allocation either: writes land on a typed array in the prototype chain.
const o = Object.create(new Uint8Array(0));
Object.defineProperty(o, 'length', { value: 2 ** 32 });
Array.prototype.fill.call(o, 0)
```

(`every`, `reduce`, `includes`, `indexOf`, `lastIndexOf` and `find` already
poll; the rest don't.)

**B. The interrupt can be swallowed.** The interrupt is an uncatchable
`InternalError`, but several built-ins catch every exception with
`JS_GetException` and turn it into a promise rejection or a new error: the
`Promise` constructor and `Promise.try` (executor), async functions and async
generators, `Promise.all`/`race` (iterator errors), thenable resolution,
`DisposableStack` (`SuppressedError`), and `Error.prepareStackTrace`. After
the handler fires, `__js_poll_interrupts` resets `interrupt_counter` to
`JS_INTERRUPT_COUNTER_INIT` (10000), so the next interrupt can only fire 10000
polls later, inside the next inner loop, where it is swallowed again:

```js
for (;;) { new Promise(() => { for (;;) {} }) }
for (;;) { (async function* () { for (;;) {} })().next() }
for (;;) { try { const s = new DisposableStack(); s.defer(() => { for (;;) {} }); s.defer(() => { for (;;) {} }); s.dispose() } catch (e) {} }
```

**C. Native sorts don't poll.** `%TypedArray%.prototype.sort` without a
comparator, and `Array.prototype.sort`'s default string comparison, run
entirely in C. Sorting 12 million random `Int32Array` elements overshot a
100 ms deadline by 2.4 s (debug build).

## Patch

1. **Sticky interrupt.** In `__js_poll_interrupts`, when the handler returns
   true, leave `interrupt_counter` at 0 instead of
   `JS_INTERRUPT_COUNTER_INIT`, so the next poll asks the handler again at
   once. Code that swallowed the interrupt is stopped at its very next poll
   (a loop back-edge or a call). The counter returns to normal at the first
   poll where the handler returns false, so an embedder that clears its
   condition resumes normally. This fixes all of group B by construction,
   without touching each swallowing site.

   ```diff
            if (rt->interrupt_handler(rt, rt->interrupt_opaque)) {
   +            ctx->interrupt_counter = 0;
                JS_ThrowInterrupted(ctx);
                return -1;
            }
   ```

2. **Polls on the generic element paths.** Call `js_poll_interrupts` on the
   slow path of `JS_TryGetPropertyInt64` and `JS_GetPropertyInt64` (after the
   fast-array path fails), and at the top of `JS_SetPropertyInt64` and
   `JS_DeletePropertyInt64`. Every native array-like loop goes through these,
   and all of them can already fail (getters, setters, proxies), so callers
   already handle the exception. One caller ignored `JS_SetPropertyInt64`'s
   result (the `JSON.stringify` replacer's property list) and now checks it.
   The fast paths stay untouched: they are bounded by allocated elements.

3. **Polls in sort comparisons.** `js_array_cmp_generic` polls on each
   comparison; the default typed-array sort goes through a small wrapper
   comparator that does the same. After an interrupt both return 0 for every
   further comparison, and `rqsort`'s three-way partition then finishes in one
   linear pass instead of running the full sort.

4. **Compiler out-of-memory errors.** The parser carries on when growing the
   bytecode buffer or the constant pool fails. `js_create_function` then
   reads a truncated or NULL buffer (SIGSEGV in `resolve_scope_var`) or hits
   `assert(cpool_idx >= 0)` (abort). A script can reach this by filling the
   heap close to the memory limit and then calling `eval`. The fix checks
   `dbuf_error(&fd->byte_code)` on entry to `js_create_function` and turns the
   assertion into `JS_ThrowOutOfMemory`. Refusing each of the 918 allocations
   of a compile-heavy `eval` in turn crashed at 15 of them before the fix and
   at none after. This could be a separate issue.

`js_poll_interrupts` only decrements a counter on the common path, so the
cost is one decrement and branch per element on paths that already do a
property lookup.

## Repro test

With quickjs-ng's own API (fits `api-test.c`):

```c
static int interrupt_after_deadline(JSRuntime *rt, void *opaque)
{
    return js__hrtime_ms() > *(int64_t *)opaque;
}

static void interrupts_stop_native_loops_and_swallowing_builtins(void)
{
    static const char *scripts[] = {
        "Array.prototype.join.call({ length: 2 ** 40 }, '')",
        "const o = Object.create(new Uint8Array(0));"
        "Object.defineProperty(o, 'length', { value: 2 ** 32 });"
        "Array.prototype.fill.call(o, 0)",
        "for (;;) { new Promise(() => { for (;;) {} }) }",
        "for (;;) { (async function* () { for (;;) {} })().next() }",
        "const t = new Int32Array(12e6);"
        "for (let i = 0; i < t.length; i += 4096) t[i] = -i;"
        "t.sort()",
    };
    for (size_t i = 0; i < countof(scripts); i++) {
        JSRuntime *rt = JS_NewRuntime();
        JSContext *ctx = JS_NewContext(rt);
        int64_t deadline = js__hrtime_ms() + 100;
        JS_SetInterruptHandler(rt, interrupt_after_deadline, &deadline);
        JSValue ret = JS_Eval(ctx, scripts[i], strlen(scripts[i]), "<input>",
                              JS_EVAL_TYPE_GLOBAL);
        // Without the patch, all but the last never return; the last
        // returns seconds late.
        assert(JS_IsException(ret));
        assert(js__hrtime_ms() < deadline + 1000);
        JSValue exc = JS_GetException(ctx);
        assert(JS_IsUncatchableError(exc));
        JS_FreeValue(ctx, exc);
        JS_FreeContext(ctx);
        JS_FreeRuntime(rt);
    }
}
```

surimono's version runs 47 such scripts in child processes with a watchdog:
`crates/core/tests/hostile.rs`.

## Results in surimono

Time limit 250 ms, debug build, each script in a child process killed after
10 s (`crates/core/tests/hostile.rs`):

| Group | Unpatched | Patched |
|---|---|---|
| A, 32 array-like loops | 25 hung (killed at 10 s); `fill` on a plain object hit the memory limit after 2.8 s | all end, 250–281 ms (TimeLimit), or at once for the 5 that aren't hostile without the old guards |
| B, 11 swallowing built-ins | 7 hung (Promise ctor, Promise.try, async generator, DisposableStack, Promise.all/race iterators, thenable getter) | all TimeLimit, 250–277 ms |
| C, 4 sorts | 12M `Int32Array` 9.0 s, 6M `Float64Array` 6.0 s, 1M default `Array` sort 5.4 s | 275–433 ms |

Out of memory: refusing each single allocation of a compile-heavy `eval`
(918 allocations) crashed at 15 points before patch 4 and at none after.
