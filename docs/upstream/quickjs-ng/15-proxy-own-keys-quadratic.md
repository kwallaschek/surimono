Title: A proxy's ownKeys result is checked in quadratic time, in C, after the trap returns

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, running hostile scripts under a time limit), then reproduced on current master.

**What happens.** `js_proxy_get_own_property_names` calls `find_prop_key`, a linear scan, in two places: once per key to find duplicates, and once per key of the target that must be in the result, when the target has non-configurable keys or isn't extensible. The first is marked `XXX: inefficient` in the source. Both run after the trap has returned, in a native loop that never polls, so `JS_SetInterruptHandler` can't stop them. Every way of listing a proxy's keys goes through them: `Reflect.ownKeys`, `Object.keys`, `values`, `entries`, `getOwnPropertyDescriptors`, `assign`, `for-in`, spread and `JSON.stringify`.

**Reproduce.**

```js
const k = [];
for (let i = 0; i < 200000; i++) k.push('k' + i);
Reflect.ownKeys(new Proxy({}, { ownKeys: () => k }));
```

The `repro.sh` in `quickjs-ng-repro.tar.gz`, attached, runs this with `harness/time-limit.c`, whose handler compares a clock with a deadline, in a release build, on master at e22ce4de (2 Nov 2026). With a 250 ms limit, on a busy 2-core machine:

| Case | Input | Past the deadline |
|---|---|---|
| `proxy-own-keys` | 200,000 keys, duplicate check | 11,399 ms |
| `proxy-own-keys-target` | 100,000 keys, frozen target, both checks | 5,237 ms |

The time grows with the square of the keys: 49,000 keys take about 0.8 s, 98,000 about 2.4 s and 196,000 about 11 s. The script ends with no error at all, because nothing polls until the next bytecode instruction.

**Suggested fix.** Sort a copy of the result and search that: entries of `{ atom, index in tab }`, sorted by atom with `rqsort`. Neighbours in the sorted copy show any duplicate, and a binary search replaces the second `find_prop_key`. `tab` itself stays as it is, since its order is the result. For 196,000 keys the check drops from 11 s to about 30 ms, and a call with 5 keys costs the same within noise with the copy on the stack for up to 16 keys.

We chose a sort over the hash the `XXX` comment suggests, whether a temporary object or a table. A string like `"123"` becomes a tagged integer atom, so a script picks the atoms it wants and can make keys collide under any fixed hash function, while `rqsort` is three-way and falls back to heapsort past a depth limit.

Polls complete it: one in the loop that reads the result (a fast array doesn't poll in `JS_GetPropertyUint32`), one in the loop over the target's keys, and one in the sort's comparison. As in `js_array_cmp_generic`, the comparison answers 0 once it has failed, so `rqsort` finishes in one pass. Those matter at the top end: eight million keys still take about 5.5 s after the fix.

The error order has to stay as the spec has it: the duplicate check runs before `IsExtensible` on the target, which can be a proxy with a trap of its own. The copy needs an allocation that can fail, so both exits of the function free it.

surimono carries this fix in its patched copy of QuickJS, and we can send it as a pull request.
