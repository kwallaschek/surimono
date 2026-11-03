Title: libregexp: group-name table out of step after a failed write, heap overflow read in js_regexp_exec

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** `re_parse_term` appends capture-group names to a `dbuf`. A failed write is ignored, parsing goes on, and `lre_compile` doesn't check that buffer the way it checks the bytecode buffer. The compiled regexp then holds a truncated name table, and `js_regexp_exec` runs off its end: `JS_NewAtom` calls `strlen` on a pointer past the buffer while it builds the `groups` object.

**Reproduce.** `repro.sh` and `harness/fail-nth.c` reproduce this on master at e22ce4de (2 Nov 2026). The harness gives a runtime an allocator through `JS_NewRuntime2` that returns NULL for the Nth allocation of a script run and then carries on, in an ASan build, which turns the small-block arena off so every allocation reaches it. `cases/oom/regexp-groups.js` is a named-groups regexp in one line:

```
REPRODUCED 06 regexp-groups  refused allocation 27: heap-buffer-overflow (strlen < JS_NewAtom < js_regexp_exec)
```

**Suggested fix.** Stop parsing at the first failed write to the name table, and check it in `lre_compile` together with the bytecode buffer. This is patch 12 in `vendor/rquickjs-sys/PATCHES.md` in surimono.
