Title: Parser carries on after a failed buffer or label: out-of-bounds accesses in js_parse_class, js_parse_for_in_of, js_parse_destructuring_element

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** Two related gaps, seen at three sites:

- `js_parse_class` remembers an offset into `fd->byte_code` and rewrites code there later. If the buffer failed to grow in between, the offset is past what the buffer holds, and `put_u32` writes out of bounds: ASan reports a heap-buffer-overflow WRITE of size 4, or a NULL write.
- `new_label` returns -1 when the label table can't grow, and `js_parse_for_in_of` and `js_parse_destructuring_element` index `label_slots` with it. ASan stops at a READ of size 4 just before `label_slots`. A write would follow it: `ls->pos += offset` in the first (when the value read passes its range check), `ref_count--` in the second. We saw only the READ, because ASan stops there.

**Reproduce.** `quickjs-ng-repro.tar.gz`, attached, reproduces this on master at e22ce4de (2 Nov 2026): run its `repro.sh`, or see `harness/fail-nth.c` and the case named here. The harness gives a runtime an allocator through `JS_NewRuntime2` that returns NULL for the Nth allocation of a script run and then carries on, in an ASan build, which turns the small-block arena off so every allocation reaches it.

```
REPRODUCED 03 parser-offsets  js_parse_class                  WRITE (NULL)      cases/oom/parser-offsets.js
REPRODUCED 03 loops           js_parse_for_in_of              READ of size 4    cases/oom/loops.js
REPRODUCED 03 syntax          js_parse_destructuring_element  READ of size 4    cases/oom/syntax.js
```

**Reachability.** Needs a failing allocator. Not reproduced from JavaScript alone under a memory limit: the harness refuses one allocation and then carries on, which a plain limit doesn't do.

**Suggested fix.** A small helper that throws the out-of-memory error when `dbuf_error(&fd->byte_code)` is set, called before each rewrite at a saved offset, and a check of `new_label`'s result (it already flags the buffer on failure, so the same helper covers these). surimono carries this fix in its patched copy of QuickJS, and we can send it as a pull request. That patch also frees the value in `cpool_add` when the pool can't grow.
