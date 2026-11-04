Title: resolve_labels writes through stale offsets after its output buffer fails to grow

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** `resolve_labels` builds the final bytecode in `bc_out`. A failed growth isn't checked before the relocations and jump patches that assume the writes landed:

- `put_u32`, `put_u16` and `put_u8` write at an offset computed from the buffer's size into a buffer that didn't grow: ASan reports heap-buffer-overflow **writes**, or SEGV on a NULL buffer;
- a `memmove` gets a negative size;
- on the way out, `assert(label_slots[i].first_reloc == NULL)` aborts a build with assertions on, because pending relocations weren't freed. With `NDEBUG` the check is gone and those `RelocEntry` blocks leak.

**Reproduce.** `quickjs-ng-repro.tar.gz`, attached, reproduces this on master at e22ce4de (2 Nov 2026): run its `repro.sh`, or see `harness/fail-nth.c` and the case named here. The harness gives a runtime an allocator through `JS_NewRuntime2` that returns NULL for the Nth allocation of a script run and then carries on, in an ASan build with `__SANITIZE_ADDRESS__` defined, which turns the small-block arena off so every allocation reaches it. gcc predefines that macro under `-fsanitize=address` and clang 18 doesn't, so `repro.sh` passes it on the command line and stops if the arena is still on. `cases/oom/resolve-labels.js` is a small function with a loop and try/finally:

```
REPRODUCED 02 resolve-labels  refused allocation 89: SEGV (put_u32 < resolve_labels)
REPRODUCED 02 parser-offsets  refused allocation 22: Assertion `label_slots[i].first_reloc == NULL' failed
```

**Reachability.** An allocator that fails during compilation. Not shown from JavaScript alone with a fresh heap; memory freed by a GC between a refused and a granted allocation would be the way in.

**Suggested fix.** Stop at the first failed write to `bc_out`, before any relocation or jump patch, and free pending relocation entries on the failure path. surimono carries this fix in its patched copy of QuickJS, and we can send it as a pull request.
