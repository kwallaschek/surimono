Title: Native loops over array-likes never poll: Array.prototype.join.call({ length: 2 ** 40 }) ignores the interrupt handler

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, running hostile scripts under a time limit), then reproduced on current master.

**What happens.** `join`, `fill`, `reverse`, `copyWithin`, `sort`, `splice`, `shift`, `unshift`, `concat`, `flat`, `String.raw` and the `JSON.stringify` array replacer walk `0..length` in C through `JS_TryGetPropertyInt64`, `JS_GetPropertyInt64`, `JS_SetPropertyInt64` and `JS_DeletePropertyInt64`. None of those polls, so an array-like with a huge `length` never returns. Some don't allocate either, so a memory limit can't help:

```js
Array.prototype.join.call({ length: 2 ** 40 }, '')
const o = Object.create(new Uint8Array(0));
Object.defineProperty(o, 'length', { value: 2 ** 32 });
Array.prototype.fill.call(o, 0)
```

**Reproduce.** The `repro.sh` in `quickjs-ng-repro.tar.gz`, attached, runs these with `harness/time-limit.c`, whose handler compares a clock with a deadline, in a release build, on master at e22ce4de (2 Nov 2026). `array-join`, `array-fill`, `array-reverse`, `array-copywithin`, `array-concat` and `string-raw` were all still running after 15 s with a 250 ms deadline.

**Suggested fix.** `js_poll_interrupts` on the slow path of the two getters (after the fast-array path fails) and at the top of the setter and the delete. Those can already fail through getters and proxies, so callers handle it. The one that doesn't is the `JSON.stringify` property list, where `JS_SetPropertyInt64`'s result should be checked. surimono carries this fix in its patched copy of QuickJS, and we can send it as a pull request.
