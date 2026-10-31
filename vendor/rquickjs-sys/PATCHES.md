# surimono's changes to rquickjs-sys 0.14.0

This directory is rquickjs-sys 0.14.0 from crates.io (rquickjs git `d7ef5ee`,
path `sys/`), which bundles quickjs-ng 0.16.2. It is wired in through
`[patch.crates-io]` in the workspace `Cargo.toml`. Every change to upstream
code is listed here and marked `surimono:` in the source. The quickjs-ng
changes were reported to its maintainers privately.

## Removed (not needed to build)

- `vendor/wasi-libc/`, `wasm-shim/`: only for `wasm32-unknown-unknown`, which
  surimono doesn't target. The build script fails with a clear message there.
- `quickjs/`: tests, test262 configs, docs, examples, the `qjs`/`qjsc`
  programs, `quickjs-libc`, the unicode generator, meson/CMake/Makefile and
  `gen/`. `build.rs` compiles only `quickjs.c`, `libregexp.c`,
  `libunicode.c` and `dtoa.c` with the headers it copies.
- `src/bindings/`: kept for x86_64 and aarch64 Linux (gnu and musl), x86_64
  and aarch64 macOS, and Windows (x86_64 msvc and gnu, aarch64 msvc). For
  any other target, enable the `bindgen` feature or copy the file back from
  the upstream crate.

Kept: `LICENSE` (rquickjs, MIT) and `quickjs/LICENSE` (QuickJS, MIT).

## Cargo.toml and build.rs

- `Cargo.toml`: a provenance comment and `publish = false`.
- `build.rs`: `cargo:rerun-if-changed=quickjs` and `=quickjs.bind.h`. The
  script's `rerun-if-env-changed` lines switch off Cargo's default of
  rerunning on any change in the package, so without these an edit to
  `quickjs.c` would not rebuild it.

## quickjs/quickjs.c

1. **Sticky interrupt** (`__js_poll_interrupts`). When the interrupt
   handler returns true, `interrupt_counter` stays at 0 instead of going
   back to 10000, so the next poll asks the handler again. Code that runs
   after a built-in has turned the interrupt into a rejection or a new
   error is stopped at its next loop back-edge or call. The counter is
   reset normally at the first poll where the handler says no.
2. **Polls in generic element access.** `js_poll_interrupts` on the slow path
   of `JS_TryGetPropertyInt64` and `JS_GetPropertyInt64`, and at the top of
   `JS_SetPropertyInt64` and `JS_DeletePropertyInt64`. That covers the
   native loops over array-likes: join, toString, toLocaleString, fill,
   reverse, copyWithin, sort, splice, slice, shift, unshift, concat, flat,
   flatMap, toReversed, toSpliced, with, `String.raw` and the
   `JSON.stringify` replacer. Their callers already handle failures
   (getters, setters and proxies can throw). The one caller that ignored
   `JS_SetPropertyInt64`'s result (the `JSON.stringify` replacer's property
   list) now checks it.
3. **Polls in sorts.** `js_array_cmp_generic` polls per comparison, and the
   default `%TypedArray%.prototype.sort` goes through a new wrapper
   comparator, `js_TA_cmp_poll`, that does the same. After an interrupt
   both return 0, so `rqsort`'s three-way partition finishes in one linear
   pass.
4. **Compiler out-of-memory errors** (`js_create_function`). If growing
   the bytecode buffer or the constant pool failed while parsing,
   `js_create_function` reports `JS_ThrowOutOfMemory` before its later
   passes.
5. **`surimono_quickjs_patch_level()`**, returning 2 since the
   allocation-failure changes below (1 before them). surimono-core calls it
   and refuses to run below 2, so a build against an unpatched or older
   rquickjs-sys fails.

Tests: `crates/core/tests/hostile.rs` (47 scripts in child processes with a
watchdog) and `out_of_memory_never_aborts` in `crates/core/src/sandbox.rs`.

### Diff (patches 1 to 5)

```diff
--- a/quickjs/quickjs.c
+++ b/quickjs/quickjs.c
@@ -8651,6 +8651,13 @@
     ctx->interrupt_counter = JS_INTERRUPT_COUNTER_INIT;
     if (rt->interrupt_handler) {
         if (rt->interrupt_handler(rt, rt->interrupt_opaque)) {
+            /* surimono: sticky interrupt. While the handler keeps saying
+               yes, ask it again at the very next poll instead of 10000
+               polls later, so code that runs after a built-in has turned
+               the interrupt into a rejection or a new error is stopped at
+               its next poll too. The counter goes back to normal at the
+               first poll where the handler says no. */
+            ctx->interrupt_counter = 0;
             JS_ThrowInterrupted(ctx);
             return -1;
         }
@@ -8658,6 +8665,12 @@
     return 0;
 }
 
+/* surimono: identifies this patched build; see vendor/rquickjs-sys/PATCHES.md. */
+int surimono_quickjs_patch_level(void)
+{
+    return 1;
+}
+
 static inline __exception int js_poll_interrupts(JSContext *ctx)
 {
     if (unlikely(--ctx->interrupt_counter <= 0)) {
@@ -10086,6 +10099,14 @@
         if (js_get_fast_array_element(ctx, p, idx, pval))
             return true;
     }
+    /* surimono: native loops over array-likes (join, reverse, sort, concat,
+       flat, splice, copyWithin...) read one index at a time through here,
+       so poll for interrupts on the slow path. The fast path is bounded by
+       the elements actually allocated. */
+    if (js_poll_interrupts(ctx)) {
+        *pval = JS_EXCEPTION;
+        return -1;
+    }
     val = JS_EXCEPTION;
     present = -1;
     prop = JS_NewAtomInt64(ctx, idx);
@@ -10116,6 +10137,9 @@
         if (js_get_fast_array_element(ctx, p, idx, &val))
             return val;
     }
+    /* surimono: see JS_TryGetPropertyInt64. */
+    if (js_poll_interrupts(ctx))
+        return JS_EXCEPTION;
     prop = JS_NewAtomInt64(ctx, idx);
     if (prop == JS_ATOM_NULL)
         return JS_EXCEPTION;
@@ -10977,6 +11001,13 @@
     JSAtom prop;
     int res;
 
+    /* surimono: native loops that write an array-like one index at a time
+       (fill, copyWithin, reverse, splice, sort...) come through here, so
+       poll for interrupts on every call. Every caller checks for -1. */
+    if (js_poll_interrupts(ctx)) {
+        JS_FreeValue(ctx, val);
+        return -1;
+    }
     if ((uint64_t)idx <= INT32_MAX) {
         /* fast path for fast arrays */
         return JS_SetPropertyValue(ctx, this_obj, js_int32(idx), val,
@@ -11940,6 +11971,10 @@
     JSAtom prop;
     int res;
 
+    /* surimono: see JS_SetPropertyInt64. */
+    if (js_poll_interrupts(ctx))
+        return -1;
+
     if ((uint64_t)idx <= JS_ATOM_MAX_INT) {
         /* fast path for fast arrays */
         return JS_DeleteProperty(ctx, obj, __JS_AtomFromUInt32(idx), flags);
@@ -36938,6 +36973,13 @@
     int function_size, byte_code_offset, cpool_offset;
     int closure_var_offset, vardefs_offset;
 
+    /* surimono: if growing the bytecode buffer failed while parsing,
+       report the out-of-memory error here, before the later passes. */
+    if (dbuf_error(&fd->byte_code)) {
+        JS_ThrowOutOfMemory(ctx);
+        goto fail;
+    }
+
     /* recompute scope linkage */
     for (scope = 0; scope < fd->scope_count; scope++) {
         fd->scopes[scope].first = -1;
@@ -36990,7 +37032,13 @@
         if (JS_IsException(func_obj))
             goto fail;
         /* save it in the constant pool */
-        assert(cpool_idx >= 0);
+        /* surimono: a negative index means cpool_add ran out of memory
+           while parsing; report the out-of-memory error. */
+        if (cpool_idx < 0) {
+            JS_FreeValue(ctx, func_obj);
+            JS_ThrowOutOfMemory(ctx);
+            goto fail;
+        }
         fd->cpool[cpool_idx] = func_obj;
     }
 
@@ -44674,6 +44722,12 @@
     if (psc->exception)
         return 0;
 
+    /* surimono: poll for interrupts on every comparison, including the
+       native default one. Once this fails, every later comparison returns
+       0 and rqsort's three-way partition finishes in one pass. */
+    if (js_poll_interrupts(ctx))
+        goto exception;
+
     if (psc->has_method) {
         argv[0] = ap->val;
         argv[1] = bp->val;
@@ -51756,7 +51810,10 @@
                     goto exception;
                 }
                 if (!JS_ToBoolFree(ctx, present)) {
-                    JS_SetPropertyInt64(ctx, jsc->property_list, j++, v);
+                    /* surimono: JS_SetPropertyInt64 can now fail on an
+                       interrupt; don't leave the exception pending. */
+                    if (JS_SetPropertyInt64(ctx, jsc->property_list, j++, v) < 0)
+                        goto exception;
                 } else {
                     JS_FreeValue(ctx, v);
                 }
@@ -61377,6 +61434,26 @@
     return cmp;
 }
 
+/* surimono: wraps a typed-array comparator. It polls for interrupts on
+   every comparison (js_poll_interrupts asks the handler every 10000) and,
+   once interrupted, answers 0 so rqsort finishes in one linear pass. */
+struct TA_poll_sort_context {
+    JSContext *ctx;
+    int (*cmp)(const void *a, const void *b, void *opaque);
+    int exception;
+};
+
+static int js_TA_cmp_poll(const void *a, const void *b, void *opaque) {
+    struct TA_poll_sort_context *psc = opaque;
+    if (unlikely(psc->exception))
+        return 0;
+    if (unlikely(js_poll_interrupts(psc->ctx))) {
+        psc->exception = 1;
+        return 0;
+    }
+    return psc->cmp(a, b, NULL);
+}
+
 static JSValue js_typed_array_sort(JSContext *ctx, JSValueConst this_val,
                                    int argc, JSValueConst *argv)
 {
@@ -61519,8 +61596,10 @@
         done:
             js_free(ctx, array_idx);
         } else {
-            rqsort(p->u.array.u.ptr, len, elt_size, cmpfun, &tsc);
-            if (tsc.exception)
+            /* surimono: poll while sorting; see js_TA_cmp_poll. */
+            struct TA_poll_sort_context psc = { ctx, cmpfun, 0 };
+            rqsort(p->u.array.u.ptr, len, elt_size, js_TA_cmp_poll, &psc);
+            if (psc.exception)
                 return JS_EXCEPTION;
         }
     }
```

## Allocation-failure paths (P1-RT-15)

The heap limit can refuse any allocation, so every failure path has to
leave the engine consistent. These changes come from refusing each single
allocation of the sweep scripts in turn with the arena off (see below),
under AddressSanitizer in a C harness and through the `Sandbox` API.
Each is marked `surimono:` in the source.

6. **Bound functions** (`js_function_bind`): the bound-function record is
   allocated before the object, and `js_bound_function_finalizer` and
   `js_bound_function_mark` return early on a NULL record, like the other
   finalizers.
7. **Function definitions with a failed bytecode buffer**
   (`js_free_function_def`): a buffer whose growth failed is freed without
   decoding it for atoms. Its atom references are left to
   `JS_FreeRuntime`, which frees every atom.
8. **`resolve_labels`** stops at the first failed write to the new
   bytecode, before any relocation or jump patch, and its failure path
   frees pending relocation entries and flags the source buffer so it is
   not decoded either (patch 7).
9. **Parser patches at saved offsets**: a new helper,
   `js_check_bytecode_oom`, throws the out-of-memory error before code at a
   saved offset is rewritten if the buffer has failed (class constructor
   index and brand, destructuring defaults, `for`/`for-in`/`for-of` loop
   reordering, `switch` default). `new_label` already flags the buffer
   when the label table can't grow, so this covers failed labels too.
   `cpool_add` frees its value when the constant pool can't grow.
10. **Reference leaks on failure**: `JSON.stringify` frees the current array
    element when converting its index fails; `Promise.allSettled` and
    `Promise.any` free the resolve-element function when the next step
    fails.
11. **`JS_FreeRuntime`** no longer asserts that the object list is empty:
    objects a failure path leaked are left alone (their arena blocks go with
    the arenas) instead of aborting the process. The sweep checks that each
    runtime gives back every byte, so a leak still fails the tests.
12. **libregexp** (`re_parse_term`, `lre_compile`): parsing stops at the
    first failed write to the group-name table, and compiling checks it
    with the bytecode buffer.
13. **Test-only arena switch**: `JSArenaState.large_blocks_only` and
    `surimono_quickjs_set_arena(rt, enabled)`. With the arena off, every
    allocation of that runtime goes to the backing allocator, so the
    sweep's allocator sees each one; with it on (the default, and always in
    production) only arena refills and large blocks get there. Blocks are
    freed by their own header, so switching is safe at any time.

Tests: `out_of_memory_never_aborts` and `out_of_memory_full_sweep` in
`crates/core/src/sandbox.rs` (allocating, syntax and compile-heavy scripts,
compiled from source and run from bytecode, arena on and off; each run
must end as success or `MemoryLimit`, leave the sandbox usable and give
back every byte). The full diff is
`diff -u <rquickjs-sys-0.14.0>/quickjs vendor/rquickjs-sys/quickjs`.
