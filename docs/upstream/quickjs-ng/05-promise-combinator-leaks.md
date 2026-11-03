Title: Promise.allSettled and Promise.any leak on allocation failure, then JS_FreeRuntime asserts

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** In `js_promise_all`, when creating `reject_element` fails, `resolve_element` is not freed, and in the `Promise.any` branch a failed `JS_DefinePropertyValueUint32` leaves `next_promise` and `resolve_element` behind. They stay on the GC list, and `assert(list_empty(&rt->gc_obj_list))` in `JS_FreeRuntime` aborts when the embedder tears the runtime down, which is what an embedder does after an out-of-memory error.

**Reproduce.** `repro.sh` and `harness/fail-nth.c` reproduce this on master at e22ce4de (2 Nov 2026). The harness gives a runtime an allocator through `JS_NewRuntime2` that returns NULL for the Nth allocation of a script run and then carries on, in an ASan build, which turns the small-block arena off so every allocation reaches it. `cases/oom/promise-allsettled.js` and `promise-any.js` are one line each:

```
REPRODUCED 05 promise-allsettled  refused allocation 102: Assertion `list_empty(&rt->gc_obj_list)' failed
REPRODUCED 05 promise-any         refused allocation 89:  Assertion `list_empty(&rt->gc_obj_list)' failed
```

**Suggested fix.** Free those values on both paths. Separately, consider whether `JS_FreeRuntime` should abort for a leak on a failure path: an embedder running untrusted scripts under a limit would rather lose a few bytes than the process. Patches 10 and 11 in `vendor/rquickjs-sys/PATCHES.md` in surimono. A similar leak in `JSON.stringify` is already fixed on master.
