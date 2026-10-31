# rquickjs: class prototypes are cached per runtime, not per context

Draft for Kay to file at https://github.com/DelSkayn/rquickjs (issue, then PR
if wanted). Found while building surimono's runtime pool; surimono works
around it by giving every render a fresh runtime.

## Problem

rquickjs 0.14 caches the prototype of every Rust class, including
`RustFunction`, the class behind every `Function::new`, in the runtime's
opaque state (`runtime/opaque.rs`, `Opaque::prototypes`, keyed by the class's
`TypeId` only). The first context that creates an instance fills the cache, and
every later context of the same runtime reuses that entry. For
`RustFunction`, `prototype()` returns `Function.prototype` *of the context it
was called in*, so:

1. Rust functions created in a second context inherit from the **first**
   context's `Function.prototype`, not their own realm's. A script in context
   2 sees `Object.getPrototypeOf(rustFn) !== Function.prototype`, and anything
   a script in context 1 put on its `Function.prototype` shows up on context
   2's Rust functions. Contexts are meant to be isolated realms, so state
   leaks between them.
2. The cached `Object` holds a strong reference into context 1, so its
   `Function.prototype`, and everything reachable from it, stays alive after
   context 1 is dropped, until the runtime is freed. A pool that reuses one
   runtime for many short-lived contexts leaks one realm's worth of objects.

The same applies to any `JsClass` whose `prototype()` depends on the context
(anything built from that context's intrinsics).

## Repro test

```rust
use rquickjs::{Context, Function, Runtime};

#[test]
fn rust_functions_use_their_own_realms_function_prototype() {
    let rt = Runtime::new().unwrap();

    let first = Context::full(&rt).unwrap();
    first.with(|ctx| {
        let f = Function::new(ctx.clone(), || 1).unwrap();
        ctx.globals().set("f", f).unwrap();
        ctx.eval::<(), _>("Function.prototype.planted = 'from the first context'")
            .unwrap();
    });
    drop(first);

    let second = Context::full(&rt).unwrap();
    second.with(|ctx| {
        let g = Function::new(ctx.clone(), || 2).unwrap();
        ctx.globals().set("g", g).unwrap();
        let same: bool = ctx
            .eval("Object.getPrototypeOf(g) === Function.prototype")
            .unwrap();
        let planted: String = ctx.eval("typeof g.planted").unwrap();
        // Today: false and "string". Expected: true and "undefined".
        assert!(same);
        assert_eq!(planted, "undefined");
    });
}
```

## Minimal patch

Key the cache by context as well as class, and drop a context's entries when
the context is freed. In `runtime/opaque.rs`:

```diff
-    prototypes: UnsafeCell<HashMap<TypeId, Option<Object<'js>>>>,
+    /// Class prototypes, per context: a prototype belongs to one realm.
+    prototypes: UnsafeCell<HashMap<(TypeId, *mut qjs::JSContext), Option<Object<'js>>>>,
@@ pub fn get_or_insert_prototype<C: JsClass<'js>>(
-            let id = vtable.id();
-            match (*self.prototypes.get()).entry(id) {
+            let id = (vtable.id(), ctx.as_ptr());
+            match (*self.prototypes.get()).entry(id) {
@@
+    /// Forgets the prototypes cached for `ctx`; called when it is freed.
+    pub fn forget_context(&self, ctx: *mut qjs::JSContext) {
+        unsafe { (*self.prototypes.get()).retain(|(_, c), _| *c != ctx) }
+    }
```

and in `Context`'s `Drop` (`context/base.rs`), before `JS_FreeContext`, call
`opaque.forget_context(self.ctx.as_ptr())`. A context pointer can be reused
after the context is freed, which is why the entries must go on drop rather
than being left to go stale.

An alternative with no map changes: store the cache in each context's opaque
(`JS_SetContextOpaque`) instead of the runtime's, so it is freed with the
context.

## Workaround in surimono

`Sandbox::reset` (crates/core) builds a fresh runtime instead of a fresh
context in the old runtime; a runtime costs about the same as a context
(≈0.45 ms in release builds). Test: `reset_frees_contexts_that_used_rust_functions`
in `crates/core/src/sandbox.rs` (branch p1-rt-04).

## Also found: a NULL runtime is used when the allocator refuses

`RawRuntime::new_with_allocator` (`runtime/raw.rs`) passes the result of
`JS_NewRuntime2` straight to `JS_SetDumpFlags` and other setup calls. When the
custom allocator refuses one of the first allocations, `JS_NewRuntime2` returns
NULL and the process segfaults in `JS_SetDumpFlags(rt=0x0)`. The fix: check for
NULL and return `Error::Allocation`. Repro: a custom `Allocator` whose `alloc`
returns null on its first call, then `Runtime::new_with_alloc(it)` must return
`Err`.

surimono can't hit this through its memory limit: building a runtime needs far
less than the 1 MiB minimum. Its out-of-memory sweep (`out_of_memory_never_aborts`
in `crates/core/src/sandbox.rs`) found it while also refusing allocations made
while the runtime and context are built; the sweep now covers only what a
script can cause.
