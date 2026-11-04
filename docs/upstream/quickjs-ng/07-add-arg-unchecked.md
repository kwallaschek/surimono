Title: Unchecked add_arg for destructured parameters: -1 becomes OP_get_arg 0xffff, out-of-bounds stack read at run time

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** In `js_parse_function_decl2`:

```c
/* unnamed arg for destructuring */
idx = add_arg(ctx, fd, JS_ATOM_NULL);
emit_op(s, OP_get_arg);
emit_u16(s, idx);
```

`add_arg` returns -1 when the argument table can't grow, or past `JS_MAX_LOCAL_VARS` parameters, and nothing checks it (the named-parameter path a few lines down does). `(uint16_t)-1` goes into the bytecode. The function compiles, and when it runs, `OP_get_arg` reads `arg_buf[0xffff]`, about 1 MiB past the frame, and dups whatever it finds there as a JSValue.

**Reproduce.** `quickjs-ng-repro.tar.gz`, attached, reproduces this on master at e22ce4de (2 Nov 2026): run its `repro.sh`, or see `harness/fail-nth.c` and the case named here. The harness gives a runtime an allocator through `JS_NewRuntime2` that returns NULL for the Nth allocation of a script run and then carries on, in an ASan build, which turns the small-block arena off so every allocation reaches it. `cases/oom/parameter-destructuring.js`:

```js
function f({ a }) { return a; }
f({ a: 1 });
```

```
REPRODUCED 07 parameter-destructuring  refused allocation 9: SEGV (JS_CallInternal, OP_get_arg)
```

**Reachability.** Two ways in. The reproducer above needs an allocator that fails while the argument table grows; the script then goes on to run the miscompiled function. The other needs no failing allocator: past `JS_MAX_LOCAL_VARS` (65,535) `add_arg` fails with a `SyntaxError` that this path ignores, so a function with that many destructured parameters should get the same bytecode. That second way comes from reading the code. We have not run it on an unpatched engine, so we don't know that it reaches the read. With the fix below, we did run it: such a function is a `SyntaxError` at parse time.

**Suggested fix.**

```c
idx = add_arg(ctx, fd, JS_ATOM_NULL);
if (idx < 0)
    goto fail;
```

surimono carries this fix in its patched copy of QuickJS, and we can send it as a pull request.

`add_var` has unchecked callers too (`add_arguments_var` and `add_func_var` from `add_eval_variables`, `_ret_` in the statement parser), worth the same look. We haven't tried to reproduce those.
