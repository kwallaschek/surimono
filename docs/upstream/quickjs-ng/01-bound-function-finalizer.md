Title: Segfault: Function.prototype.bind under a memory limit (finalizer reads an unset bound_function)

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** `js_function_bind` creates the object first and allocates the `JSBoundFunction` record afterwards:

```c
func_obj = JS_NewObjectProtoClass(ctx, prototype, JS_CLASS_BOUND_FUNCTION);
...
bf = js_malloc(ctx, sizeof(*bf) + arg_count * sizeof(JSValue));
if (!bf)
    goto exception;
```

On failure `goto exception` frees the object, and `js_bound_function_finalizer` dereferences `p->u.bound_function`, which was never set. `js_bound_function_mark` has the same problem if a GC sees the object first.

**Reproduce from JavaScript, release build, no harness:**

```sh
for k in $(seq 5900000 16384 6400000); do
  qjs --memory-limit 8M -e "const pad = new Uint8Array($k);
    const args = new Array(30000).fill(0);
    (function () {}).bind(null, ...args)"
done
```

24 of the 31 pad sizes end in SIGSEGV on e22ce4de (2 Nov 2026). The window depends on the build. `repro.sh` runs this scan and a single-allocation version (`cases/oom/bind.js`).

**Suggested fix.** Allocate the record before the object, or set `p->u.bound_function = NULL` first and return early from the finalizer and the mark function on NULL, as other finalizers do. This is patch 6 in `vendor/rquickjs-sys/PATCHES.md` in surimono.
