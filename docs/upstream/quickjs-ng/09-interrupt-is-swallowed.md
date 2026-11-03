Title: Interrupts can be swallowed: a script keeps running after the handler returned true

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** The interrupt is an uncatchable `InternalError`, but several built-ins take every exception with `JS_GetException` and turn it into a rejection or a new error: the `Promise` constructor and `Promise.try`, async functions and generators, `Promise.all`/`race`, thenable resolution, `DisposableStack`, `Error.prepareStackTrace`. After the handler fires, `__js_poll_interrupts` resets `interrupt_counter` to 10000, so the next chance to stop is 10,000 polls later, inside the next inner loop, where it is swallowed again.

**Reproduce.**

```js
for (;;) { new Promise(() => { for (;;) {} }) }
for (;;) { (async function* () { for (;;) {} })().next() }
```

`repro.sh` runs these with `harness/time-limit.c`, whose handler compares a clock with a deadline, in a release build, on master at e22ce4de (2 Nov 2026). `swallow-promise`, `swallow-asyncgen` and `swallow-disposable` were all still running after 15 s with a 250 ms deadline.

**Suggested fix.** When the handler returns true, leave the counter at 0, so the next poll asks again at once:

```diff
         if (rt->interrupt_handler(rt, rt->interrupt_opaque)) {
+            ctx->interrupt_counter = 0;
             JS_ThrowInterrupted(ctx);
             return -1;
         }
```

The counter goes back to normal at the first poll where the handler says no, so an embedder that clears its condition resumes as before. This fixes every swallowing site by construction. It is patch 1 in `vendor/rquickjs-sys/PATCHES.md` in surimono.
