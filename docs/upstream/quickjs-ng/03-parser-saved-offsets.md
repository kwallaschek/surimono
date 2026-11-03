Title: Parser rewrites bytecode at saved offsets after the buffer failed: out-of-bounds writes in js_parse_class, js_parse_for_in_of, js_parse_destructuring_element

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** Several parser functions remember an offset into `fd->byte_code` and rewrite code there later: the class constructor index and brand, destructuring defaults, the `for`/`for-in`/`for-of` loop reordering, the `switch` default. If the buffer failed to grow in between (or the label table did, since `new_label` flags the buffer), the offset is past what the buffer holds and the rewrite is an out-of-bounds write.

**Reproduce.** `repro.sh` and `harness/fail-nth.c` reproduce this on master at e22ce4de (2 Nov 2026). The harness gives a runtime an allocator through `JS_NewRuntime2` that returns NULL for the Nth allocation of a script run and then carries on, in an ASan build, which turns the small-block arena off so every allocation reaches it.

```
REPRODUCED 03 parser-offsets  js_parse_class                  cases/oom/parser-offsets.js
REPRODUCED 03 loops           js_parse_for_in_of              cases/oom/loops.js
REPRODUCED 03 syntax          js_parse_destructuring_element  cases/oom/syntax.js
```

**Reachability.** As with the other compiler findings: an allocator that fails during parsing.

**Suggested fix.** A small helper that throws the out-of-memory error when `dbuf_error(&fd->byte_code)` is set, called before each rewrite at a saved offset. This is patch 9 in `vendor/rquickjs-sys/PATCHES.md` in surimono, which also frees the value in `cpool_add` when the pool can't grow.
