# surimono's changes to rquickjs-sys 0.14.0

This directory is rquickjs-sys 0.14.0 from crates.io (rquickjs git `d7ef5ee`,
path `sys/`), which bundles quickjs-ng 0.16.2. It is wired in through
`[patch.crates-io]` in the workspace `Cargo.toml`. Every change to upstream
code is listed here and marked `surimono:` in the source. Drafts for
upstream are in `docs/upstream/quickjs-ng-sticky-interrupt-and-native-loop-polls.md`.

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
   back to 10000, so the next poll asks the handler again. The Promise
   constructor, `Promise.try`, async functions and generators,
   `Promise.all`/`race`, thenables, `DisposableStack` and
   `Error.prepareStackTrace` turn the interrupt into a rejection or a new
   error. The code around them is now stopped at its next loop back-edge or
   call, not 10000 polls later inside the next swallowed loop. The counter is
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
4. **Compiler out-of-memory errors** (`js_create_function`). The parser
   carries on when growing the bytecode buffer or the constant pool fails.
   `js_create_function` then read a truncated or NULL buffer
   (`resolve_scope_var`, SIGSEGV) or hit `assert(cpool_idx >= 0)` (abort).
   It now reports `JS_ThrowOutOfMemory` for both. Found by refusing each
   allocation of a compile-heavy `eval` in turn; the memory limit can
   refuse any of them.
5. **`surimono_quickjs_patch_level()`**, returning 1. surimono-core calls it,
   so it fails to link against an unpatched rquickjs-sys rather than ship a
   sandbox that can hang.

Tests: `crates/core/tests/hostile.rs` (47 scripts in child processes with a
watchdog) and `out_of_memory_never_aborts` in `crates/core/src/sandbox.rs`.

### Diff

```diff
--- a/quickjs/quickjs.c	2026-10-31 06:30:19.303433598 +0900
+++ b/quickjs/quickjs.c	2026-10-31 06:30:19.308324767 +0900
@@ -8651,6 +8651,15 @@
     ctx->interrupt_counter = JS_INTERRUPT_COUNTER_INIT;
     if (rt->interrupt_handler) {
         if (rt->interrupt_handler(rt, rt->interrupt_opaque)) {
+            /* surimono: sticky interrupt. Some built-ins (the Promise
+               constructor, Promise.try, async generators, DisposableStack,
+               Error.prepareStackTrace) turn any exception into a rejection
+               or a new, catchable error, swallowing the interrupt. Ask the
+               handler again at the very next poll instead of 10000 polls
+               later, so the code around the swallow is stopped at once.
+               The counter goes back to normal at the first poll where the
+               handler says no. */
+            ctx->interrupt_counter = 0;
             JS_ThrowInterrupted(ctx);
             return -1;
         }
@@ -8658,6 +8667,12 @@
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
@@ -10086,6 +10101,15 @@
         if (js_get_fast_array_element(ctx, p, idx, pval))
             return true;
     }
+    /* surimono: native loops over array-likes (join, reverse, sort, concat,
+       flat, splice, copyWithin...) come through here one index at a time
+       and never reach a bytecode poll, so a huge `length` would loop
+       forever. Poll on the slow path; the fast path is bounded by the
+       elements actually allocated. */
+    if (js_poll_interrupts(ctx)) {
+        *pval = JS_EXCEPTION;
+        return -1;
+    }
     val = JS_EXCEPTION;
     present = -1;
     prop = JS_NewAtomInt64(ctx, idx);
@@ -10116,6 +10140,9 @@
         if (js_get_fast_array_element(ctx, p, idx, &val))
             return val;
     }
+    /* surimono: see JS_TryGetPropertyInt64. */
+    if (js_poll_interrupts(ctx))
+        return JS_EXCEPTION;
     prop = JS_NewAtomInt64(ctx, idx);
     if (prop == JS_ATOM_NULL)
         return JS_EXCEPTION;
@@ -10977,6 +11004,15 @@
     JSAtom prop;
     int res;
 
+    /* surimono: native loops that write an array-like one index at a time
+       (fill, copyWithin, reverse, splice, sort...) never reach a bytecode
+       poll. A write that lands on a typed array in the prototype chain, or
+       past a typed array's end, allocates nothing, so the memory limit
+       can't stop them either. Every caller checks for -1. */
+    if (js_poll_interrupts(ctx)) {
+        JS_FreeValue(ctx, val);
+        return -1;
+    }
     if ((uint64_t)idx <= INT32_MAX) {
         /* fast path for fast arrays */
         return JS_SetPropertyValue(ctx, this_obj, js_int32(idx), val,
@@ -11940,6 +11976,10 @@
     JSAtom prop;
     int res;
 
+    /* surimono: see JS_SetPropertyInt64. */
+    if (js_poll_interrupts(ctx))
+        return -1;
+
     if ((uint64_t)idx <= JS_ATOM_MAX_INT) {
         /* fast path for fast arrays */
         return JS_DeleteProperty(ctx, obj, __JS_AtomFromUInt32(idx), flags);
@@ -36938,6 +36978,14 @@
     int function_size, byte_code_offset, cpool_offset;
     int closure_var_offset, vardefs_offset;
 
+    /* surimono: the parser carries on when growing the bytecode buffer
+       fails, leaving it truncated or NULL, which resolve_variables then
+       reads. Report the out-of-memory error instead. */
+    if (dbuf_error(&fd->byte_code)) {
+        JS_ThrowOutOfMemory(ctx);
+        goto fail;
+    }
+
     /* recompute scope linkage */
     for (scope = 0; scope < fd->scope_count; scope++) {
         fd->scopes[scope].first = -1;
@@ -36990,7 +37038,13 @@
         if (JS_IsException(func_obj))
             goto fail;
         /* save it in the constant pool */
-        assert(cpool_idx >= 0);
+        /* surimono: cpool_add fails only when out of memory, and the parser
+           carries on; report the error instead of asserting. */
+        if (cpool_idx < 0) {
+            JS_FreeValue(ctx, func_obj);
+            JS_ThrowOutOfMemory(ctx);
+            goto fail;
+        }
         fd->cpool[cpool_idx] = func_obj;
     }
 
@@ -44674,6 +44728,12 @@
     if (psc->exception)
         return 0;
 
+    /* surimono: the default comparison is native, so a large sort never
+       reaches a bytecode poll. Once this fails, every later comparison
+       returns 0 and rqsort's three-way partition finishes in one pass. */
+    if (js_poll_interrupts(ctx))
+        goto exception;
+
     if (psc->has_method) {
         argv[0] = ap->val;
         argv[1] = bp->val;
@@ -51756,7 +51816,10 @@
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
@@ -61377,6 +61440,27 @@
     return cmp;
 }
 
+/* surimono: the typed comparators are plain C and never poll, so sorting
+   a large typed array ran far past the deadline. This wrapper polls on
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
@@ -61519,8 +61603,10 @@
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
