Title: Promise.allSettled and Promise.any leak objects on allocation failure (JS_FreeRuntime asserts in builds with assertions on)

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** In `js_promise_all`, when creating `reject_element` fails, `resolve_element` is not freed, and in the `Promise.any` branch a failed `JS_DefinePropertyValueUint32` leaves `next_promise` and `resolve_element` behind. They stay on the GC list. In a build with assertions on, `assert(list_empty(&rt->gc_obj_list))` in `JS_FreeRuntime` then aborts the process when the runtime is freed. With `NDEBUG` (CMake's Release type) the assertion is compiled out and they are an ordinary leak.

**Reproduce.** `quickjs-ng-repro.tar.gz`, attached, reproduces this on master at e22ce4de (2 Nov 2026): run its `repro.sh`, or see `harness/fail-nth.c` and the case named here. The harness gives a runtime an allocator through `JS_NewRuntime2` that returns NULL for the Nth allocation of a script run and then carries on, in an ASan build, which turns the small-block arena off so every allocation reaches it. `cases/oom/promise-allsettled.js` and `promise-any.js` are one line each:

```
REPRODUCED 05 promise-allsettled  refused allocation 102: Assertion `list_empty(&rt->gc_obj_list)' failed
REPRODUCED 05 promise-any         refused allocation 89:  Assertion `list_empty(&rt->gc_obj_list)' failed
```

**Reachability.** Needs a failing allocator. Not reproduced from JavaScript alone under a memory limit: the harness refuses one allocation and then carries on, which a plain limit doesn't do.

**Suggested fix.** Free those values on both paths. Separately, consider whether `JS_FreeRuntime` should assert on a leak from a failure path: an embedder running untrusted scripts under a limit with assertions on would rather lose a few bytes than the process. surimono carries this fix in its patched copy of QuickJS, and we can send it as a pull request. A similar leak in `JSON.stringify` is already fixed on master.
