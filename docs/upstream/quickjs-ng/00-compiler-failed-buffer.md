Title: Compiler keeps going after the bytecode buffer or constant pool fails to grow: out-of-bounds read in resolve_variables, assert in js_create_function

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** The parser ignores out-of-memory errors while emitting code. `dbuf` remembers the failure and drops the writes, `cpool_add` returns -1, and parsing carries on to the end. `js_create_function` then runs on what it has:

- `resolve_variables` reads the truncated buffer past its end (`get_u32`, `get_u16`, a `dbuf_put` of an operand): ASan reports heap-buffer-overflow reads.
- With the constant pool refused, `assert(cpool_idx >= 0)` in `js_create_function` aborts the process in a build with assertions on. With `NDEBUG` (CMake's Release type) the check is gone and the next line writes `fd->cpool[-1]`.

**Reproduce.** `quickjs-ng-repro.tar.gz`, attached, reproduces this on master at e22ce4de (2 Nov 2026): run its `repro.sh`, or see `harness/fail-nth.c` and the case named here. The harness gives a runtime an allocator through `JS_NewRuntime2` that returns NULL for the Nth allocation of a script run and then carries on, in an ASan build with `__SANITIZE_ADDRESS__` defined, which turns the small-block arena off so every allocation reaches it. gcc predefines that macro under `-fsanitize=address` and clang 18 doesn't, so `repro.sh` passes it on the command line and stops if the arena is still on. `cases/oom/compiler.js` is two lines of arrow functions:

```
REPRODUCED 00 compiler  refused allocation 4: heap-buffer-overflow (resolve_variables)
REPRODUCED 00 compiler  refused allocation 15: Assertion `cpool_idx >= 0' failed
```

**Reachability.** An allocator that fails, which is what `JS_SetMemoryLimit` and any custom `JSMallocFunctions` can do. Under a plain memory limit a refused growth tends to keep being refused, and we could not reach these sites from JavaScript alone with a fresh heap. So this is a failure-path bug, not a one-line exploit.

**Suggested fix.** At the top of `js_create_function`:

```c
if (dbuf_error(&fd->byte_code)) {
    JS_ThrowOutOfMemory(ctx);
    goto fail;
}
```

and replace the assertion with the same error, freeing `func_obj`. surimono carries this fix in its patched copy of QuickJS, and we can send it as a pull request.
