Title: The interrupt handler is asked every 10,000 polls, so one expensive native call per iteration overruns a deadline by minutes

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** `js_poll_interrupts` calls the handler only when `interrupt_counter` reaches zero. That's fine for `for (;;) {}`. A loop whose body is one expensive native call polls two or three times per iteration, so the deadline is looked at every 3,000 to 5,000 iterations, however long each takes:

```js
const o = []; for (let i = 0; i < 20000; i++) o.push({ i, s: 'x' + i });
for (;;) JSON.stringify(o)                                // 187 s late
const a = new Array(1e6).fill(1); for (;;) a.indexOf(2)   //  49 s late
const s = 'ab'.repeat(5e5); for (;;) s.indexOf('abc')     //  67 s late
```

(Full runs at -O2 on 0.16.2 with a 250 ms deadline.)

**Reproduce.** `repro.sh` runs these with `harness/time-limit.c`, whose handler compares a clock with a deadline, in a release build, on master at e22ce4de (2 Nov 2026). `granular-json`, `granular-indexof` and `granular-string` were all still running 15 s in with a 250 ms deadline.

**Proposal.** The interval can't just be lowered, because a handler usually reads a clock. Add a host-owned flag instead:

```c
/* Every poll also loads *flag (relaxed atomic) and calls the interrupt handler
   while it is nonzero. The host sets it from any thread and clears it; it must
   outlive the runtime. NULL removes it. */
void JS_SetInterruptFlag(JSRuntime *rt, uint32_t *flag);
```

The poll gains one relaxed load:

```c
if (unlikely(--ctx->interrupt_counter <= 0 ||
             atomic_load_explicit(ctx->interrupt_flag, memory_order_relaxed)))
    return __js_poll_interrupts(ctx);
```

Until the host sets one, the pointer aims at a static zero, so there is no NULL check and no change for other embedders. `JS_RequestInterrupt(JSRuntime *)` would have the same effect but makes the host synchronise with `JS_FreeRuntime`. This is patch 14 in `vendor/rquickjs-sys/PATCHES.md` in surimono, used there with a watchdog thread.
