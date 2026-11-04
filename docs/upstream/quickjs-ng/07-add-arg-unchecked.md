Title: Segfault from JavaScript alone: unchecked add_arg for destructured parameters, -1 becomes OP_get_arg 0xffff

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** In `js_parse_function_decl2`:

```c
/* unnamed arg for destructuring */
idx = add_arg(ctx, fd, JS_ATOM_NULL);
emit_op(s, OP_get_arg);
emit_u16(s, idx);
```

`add_arg` returns -1 when the argument table can't grow, or past `JS_MAX_LOCAL_VARS` parameters, and nothing checks it (the named-parameter path a few lines down does). `(uint16_t)-1` goes into the bytecode. The function compiles, and when it runs, `OP_get_arg` reads `arg_buf[0xffff]` and dups whatever it finds there as a JSValue. With a short argument table that is about 1 MiB past the frame.

**Reproduce.** `quickjs-ng-repro.tar.gz`, attached, reproduces this on master at e22ce4de (2 Nov 2026): run its `repro.sh`, or see `harness/fail-nth.c` and the case named here. The harness gives a runtime an allocator through `JS_NewRuntime2` that returns NULL for the Nth allocation of a script run and then carries on, in an ASan build with `__SANITIZE_ADDRESS__` defined, which turns the small-block arena off so every allocation reaches it. gcc predefines that macro under `-fsanitize=address` and clang 18 doesn't, so `repro.sh` passes it on the command line and stops if the arena is still on. `cases/oom/parameter-destructuring.js`:

```js
function f({ a }) { return a; }
f({ a: 1 });
```

```
REPRODUCED 07 parameter-destructuring  refused allocation 9: SEGV (JS_CallInternal, OP_get_arg)
```

**Reproduce from JavaScript, release build, default limits, no harness:**

```js
const params = new Array(65536).fill('{} = {}').join(',');
const GeneratorFunction = (function* () {}).constructor;
new GeneratorFunction(params, 'yield 7')().next();
```

```
$ qjs generator-parameters.js
Segmentation fault                     (exit 139, 10 of 10 runs on e22ce4de, 2 Nov 2026)
```

In an ASan build that is a heap-use-after-free READ of size 4 in `js_dup`, called from `OP_get_arg` in `JS_CallInternal`, on an object `free_object` had already freed. An `async function` of the same shape does the same.

**Reachability.** Two ways in. The first reproducer needs an allocator that fails while the argument table grows; the script then goes on to run the miscompiled function. The second needs nothing but a script: past `JS_MAX_LOCAL_VARS` (65,535) `add_arg` fails with a `SyntaxError` that this path ignores, and the function compiles to bytecode with no error. The 65,536th parameter is the one that gets the bad index. What we ran on e22ce4de:

- 65,536 parameters of the form `{ a0 }`: `SyntaxError: too many variables declared`. `add_var`'s limit stops parameters that bind names first.
- 65,536 of the form `{}`, called with 65,535 objects: runs and returns, with no ASan report, and the pending `SyntaxError` surfaces afterwards.
- 65,536 of the form `{} = {}`, called with no arguments, as a generator or `async function`: the crash above, with the default stack limit, because those frames are on the heap.
- The same as a plain function: `RangeError: Maximum call stack size exceeded` with the default limit, because the frame is about 1 MiB. With `--stack-size 1100` or more it crashes the same way.

With the fix below, all the crashing forms give `SyntaxError: too many parameters in function definition` at parse time.

**Suggested fix.**

```c
idx = add_arg(ctx, fd, JS_ATOM_NULL);
if (idx < 0)
    goto fail;
```

surimono carries this fix in its patched copy of QuickJS, and we can send it as a pull request.

`add_var` has unchecked callers too (`add_arguments_var` and `add_func_var` from `add_eval_variables`, `_ret_` in the statement parser), worth the same look. We haven't tried to reproduce those.
