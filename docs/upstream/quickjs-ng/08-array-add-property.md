Title: Segfault: new on an Array subclass under a memory limit (add_property for length "cannot fail", but it can)

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** In `JS_NewObjectFromShape`, case `JS_CLASS_ARRAY`:

```c
if (likely(sh == ctx->array_shape)) {
    pr = &p->prop[0];
} else {
    /* only used for the first array */
    /* cannot fail */
    pr = add_property(ctx, p, JS_ATOM_length,
                      JS_PROP_WRITABLE | JS_PROP_LENGTH);
}
pr->u.value = js_int32(0);
```

The branch isn't only for the first array. `Array` called as a constructor with another `new.target` reaches it through `js_create_from_ctor` and `JS_NewObjectProtoClass`, and the shape found there is the empty one for that prototype, not `ctx->array_shape`. When that empty shape is shared and has no `length` successor yet, `add_property` goes through `js_clone_shape`, which allocates and can fail. Then `pr` is NULL.

**Reproduce.** `quickjs-ng-repro.tar.gz`, attached, reproduces this on master at e22ce4de (2 Nov 2026): run its `repro.sh`, or see `harness/fail-nth.c` and the case named here. The harness gives a runtime an allocator through `JS_NewRuntime2` that returns NULL for the Nth allocation of a script run and then carries on, in an ASan build with `__SANITIZE_ADDRESS__` defined, which turns the small-block arena off so every allocation reaches it. gcc predefines that macro under `-fsanitize=address` and clang 18 doesn't, so `repro.sh` passes it on the command line and stops if the arena is still on. `cases/oom/array-subclass.js`:

```js
class A extends Array {}
const keep = Object.create(A.prototype);   // shares the empty shape
new A();
```

```
REPRODUCED 08 array-subclass  refused allocation 114: SEGV (JS_NewObjectFromShape < JS_NewObjectProtoClass < js_create_from_ctor)
```

**Reproduce from JavaScript, release build, no harness:**

```sh
for k in $(seq 6000000 5003 7900000); do
  qjs --memory-limit 8M -e "const pad = new Uint8Array($k); const keep = [];
    for (;;) { class A extends Array {} keep.push(Object.create(A.prototype), new A()); }"
done
```

47 of the 380 pad sizes end in SIGSEGV on e22ce4de (2 Nov 2026) in a gcc 13 release build, all in `JS_NewObjectFromShape`; with the fix below, none do. The window depends on the build: another release build gave 40 of 380. The `repro.sh` in `quickjs-ng-repro.tar.gz` scans a shorter range and printed 21 of 160.

**Suggested fix.** Check `pr`. The object isn't on the GC list yet at this point, so free it by hand:

```c
if (unlikely(!pr)) {
    js_free_shape(ctx->rt, p->shape);
    js_free(ctx, p->prop);
    js_free(ctx, p);
    return JS_EXCEPTION;
}
```

surimono carries this fix in its patched copy of QuickJS, and we can send it as a pull request.
