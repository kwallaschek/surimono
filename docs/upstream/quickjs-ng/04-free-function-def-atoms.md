Title: js_free_function_def decodes a failed bytecode buffer and frees atoms read from garbage

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** When a compile fails, `js_free_function_def` walks `fd->byte_code` with `free_bytecode_atoms` to release the atoms its opcodes hold. If the buffer failed to grow, or `resolve_labels` left it half converted, the walk decodes a truncated stream and calls `JS_FreeAtom` on whatever it reads: SEGV in `__JS_FreeAtom`, or the wrong atom's reference count dropping.

**Reproduce.** `quickjs-ng-repro.tar.gz`, attached, reproduces this on master at e22ce4de (2 Nov 2026): run its `repro.sh`, or see `harness/fail-nth.c` and the case named here. The harness gives a runtime an allocator through `JS_NewRuntime2` that returns NULL for the Nth allocation of a script run and then carries on, in an ASan build, which turns the small-block arena off so every allocation reaches it. `cases/oom/atoms.js` is one function with a few dozen property names:

```
REPRODUCED 04 atoms  SEGV (__JS_FreeAtom < free_bytecode_atoms < js_free_function_def)
```

**Reachability.** Needs a failing allocator. Not reproduced from JavaScript alone under a memory limit: the harness refuses one allocation and then carries on, which a plain limit doesn't do.

**Suggested fix.** Free a failed buffer without decoding it. The atoms it holds stay until `JS_FreeRuntime`, which frees every atom anyway. surimono carries this fix in its patched copy of QuickJS, and we can send it as a pull request.
