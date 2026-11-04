# surimono's changes to rquickjs-sys 0.14.0

This directory is rquickjs-sys 0.14.0 from crates.io (rquickjs git `d7ef5ee`,
path `sys/`), which bundles quickjs-ng 0.16.2. It is wired in through
`[patch.crates-io]` in the workspace `Cargo.toml`. Every change to upstream
code is listed here and marked `surimono:` in the source. Drafts for
upstream are in `docs/upstream/quickjs-ng-sticky-interrupt-and-native-loop-polls.md`
and `docs/upstream/quickjs-ng-interrupt-granularity.md`; the regexp
per-match polls (patches 19 and 20) are in
`docs/upstream/quickjs-ng-regexp-replace-polls.md`; the rest are in
`docs/upstream/quickjs-ng/`.

## Removed (not needed to build)

- `vendor/wasi-libc/`, `wasm-shim/`: only for `wasm32-unknown-unknown`, which
  surimono doesn't target. The build script fails with a clear message there
  (see below).
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
- `build.rs` (p1-rt-14, review of #32 F3): for `wasm32-unknown-unknown` it
  panics up front with a message naming the removed files, instead of
  adding `vendor/wasi-libc` include and library paths and compiling
  `wasm-shim/shim.c`, which are gone. Those three upstream blocks are
  removed; `wasm32-wasi` is untouched.

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
5. **`surimono_quickjs_patch_level()`**, returning 3: 1 with patches 1
   to 5, 2 on the branches that added the allocation-failure patches
   (6 to 13) or the interrupt flag (14 to 18) alone, 3 with both.
   surimono-core calls it and refuses to run below 3, and it fails to link
   against an unpatched rquickjs-sys rather than ship a sandbox that can
   hang or crash.

Tests: `crates/core/tests/hostile.rs` (47 scripts then, 64 since p1-rt-14,
in child processes with a watchdog) and `out_of_memory_never_aborts` in `crates/core/src/sandbox.rs`.

### Diff (patches 1 to 5)

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
    the arenas) instead of aborting the process. Large blocks those objects
    own stay allocated until the process exits. The sweep checks that each
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

## Time-accurate interrupts (P1-RT-14)

Review of #32, R4 and R5: the handler was asked only once per 10,000
polls, so a loop whose body makes one expensive native call (a
`JSON.stringify` of 20k objects, `indexOf` on a 1M-item array) saw its
deadline only every few thousand iterations, minutes late; and some native
calls with superlinear cost never polled at all. With patches 6 to 13,
patch level 3.

14. **Interrupt flag** (`surimono_set_interrupt_flag`, `js_poll_interrupts`).
   The host can give a runtime a `uint32_t` flag. Every poll still counts
   down `interrupt_counter`, and also loads the flag (a relaxed C11 atomic
   load; `volatile` where the compiler has no atomics, and so no threads);
   while it is nonzero the poll calls the handler at once. surimono-core's
   watchdog thread sets it at the deadline, and its allocator on a refused
   allocation, so the script stops at its next loop back-edge, call or
   native-loop poll. `JSRuntime` and `JSContext` get an `interrupt_flag`
   pointer (the context's is a copy, one load closer to the poll), which
   points at a static zero flag until the host sets one, so the poll needs
   no NULL check. The flag must outlive the runtime, and the handler must
   answer true while it is set.
15. **Polls in string searches.** `indexOf`, `lastIndexOf`, `includes`
   (and `startsWith`/`endsWith`, which share its loop), and
   `string_indexof`, used by `split`, `replace` and `replaceAll` with a
   string pattern, are naive searches of up to n * m character comparisons
   inside one native call. They count comparisons and poll every 65,536
   (`JS_SEARCH_POLL_UNITS`). `string_indexof` takes `ctx` and returns -2
   after an interrupt; both callers check it. `replace`/`replaceAll` and
   `split` also poll once per match: `replaceAll('a', '')` on a 64 MB
   string made 64M passes without calling script code. Results are
   unchanged.
16. **Polls in BigInt arithmetic.** `js_mp_mul_basecase` (multiply) polls
   once per row and `js_mp_divnorm` (divide and remainder) once per
   quotient limb; both take `ctx` and return -1 after an interrupt, and
   `js_bigint_mul`/`js_bigint_divrem` free their buffers and fail. Decimal
   `toString` polls once per limb it divides out, and decimal parsing once
   per limb it multiplies in. Near the 1 Mbit size cap these took 0.4 s,
   1.1 s, 11 s and 0.3 s per operation at -O2, and straight-line code never
   polls between operations. The size cap is unchanged; power goes through
   multiply, and `js_bigint_pow` now frees its running product when a
   multiply fails (review of #47, finding 1: it leaked that block, up to
   about 60 KB per stopped `**`, on every interrupt; upstream leaks it the
   same way on out of memory). Results are unchanged.
17. **Typed-array sort checks the flag, not the counter** (amends patch 3;
   review of #32, F2). Patch 3's wrapper polled on every comparison through
   one generic wrapper that called the comparator through a pointer; that
   cost 15-26% of a large default sort. Now there is one wrapper per
   element type (`JS_TA_CMP_POLL`, chosen by `js_TA_cmp_poll_for`), so the
   comparator is a direct, inlinable call, and the wrapper only loads the
   interrupt flag of patch 14 (loads only; a countdown or
   `interrupt_counter` is a load-store chain through memory on every
   comparison). When the flag is set it polls, and after an interrupt it
   answers 0 as before. Release build, 4M elements, sort only: Int32Array
   567 ms unpolled, 680 ms with patch 3, 631 ms now; Float64Array 661-685,
   757-764, 730-736; Uint8Array 652-665, 817-825, 708-719. Without a host
   flag, a default typed-array sort is no longer interrupted; surimono
   always sets one.
18. **`surimono_quickjs_patch_level()` returns 3** (see patch 5).
    surimono-core requires 3 and also links against
    `surimono_set_interrupt_flag`, so it can't be built against an engine
    without these patches.

Tests: groups D and E in `crates/core/tests/hostile.rs`;
`watchdog_raises_the_flag_at_the_deadline`,
`one_watchdog_serves_every_sandbox` and `a_refused_allocation_raises_the_flag`
in `crates/core/src/engine.rs`; `interrupted_natives_leak_nothing` in
`crates/core/src/sandbox.rs` (every group E vector, stopped by the limit,
leaves its old runtime at zero bytes). Known limit: a native call without polls
of its own still runs to its end once started; the ones left are linear
in data the memory limit bounds (a `JSON.stringify` or
`TypedArray#includes` of the whole heap).

### Diff of patches 14 to 18 (on top of patches 1 to 13)

```diff
--- a/quickjs/quickjs.c (with patches 1 to 13)
+++ b/quickjs/quickjs.c
@@ -77,6 +77,21 @@
 #define CONFIG_ATOMICS
 #endif
 
+/* surimono: the host's interrupt flag (see surimono_set_interrupt_flag).
+   Another thread sets it; the poll reads it with a relaxed load, which is
+   a plain load on every mainstream target. Without C11 atomics there are
+   no threads to set it from, so a volatile read is enough. */
+#if defined(CONFIG_ATOMICS) && !defined(GCC_BUILTIN_ATOMICS)
+typedef _Atomic uint32_t js_interrupt_flag_t;
+#define js_interrupt_flag_get(p) atomic_load_explicit((p), memory_order_relaxed)
+#elif defined(CONFIG_ATOMICS)
+typedef uint32_t js_interrupt_flag_t;
+#define js_interrupt_flag_get(p) __atomic_load_n((p), __ATOMIC_RELAXED)
+#else
+typedef volatile uint32_t js_interrupt_flag_t;
+#define js_interrupt_flag_get(p) (*(p))
+#endif
+
 #ifndef __GNUC__
 #define __extension__
 #endif
@@ -384,6 +399,9 @@ struct JSRuntime {
 
     JSInterruptHandler *interrupt_handler;
     void *interrupt_opaque;
+    /* surimono: never NULL; points at js_interrupt_flag_clear unless the
+       host set its own flag. Copied to every context. */
+    js_interrupt_flag_t *interrupt_flag;
 
     JSPromiseHook *promise_hook;
     void *promise_hook_opaque;
@@ -609,6 +627,9 @@ struct JSContext {
 
     /* when the counter reaches zero, JSRutime.interrupt_handler is called */
     int interrupt_counter;
+    /* surimono: copy of JSRuntime.interrupt_flag, one load closer to the
+       poll; when the flag is nonzero the handler is called at once. */
+    js_interrupt_flag_t *interrupt_flag;
 
     struct list_head loaded_modules; /* list of JSModuleDef.link */
 
@@ -2316,6 +2337,9 @@ static inline bool js_check_stack_overflow(JSRuntime *rt, size_t alloca_size)
     return unlikely(sp < rt->stack_limit);
 }
 
+/* surimono: the flag of runtimes whose host set none; never written. */
+static js_interrupt_flag_t js_interrupt_flag_clear;
+
 JSRuntime *JS_NewRuntime2(const JSMallocFunctions *mf, void *opaque)
 {
     JSRuntime *rt;
@@ -2339,6 +2363,7 @@ JSRuntime *JS_NewRuntime2(const JSMallocFunctions *mf, void *opaque)
     rt->malloc_state = ms;
     js_arena_init(rt);
     rt->malloc_gc_threshold = 256 * 1024;
+    rt->interrupt_flag = &js_interrupt_flag_clear; /* surimono */
 
     init_list_head(&rt->context_list);
     init_list_head(&rt->gc_obj_list);
@@ -2484,6 +2509,28 @@ void JS_SetInterruptHandler(JSRuntime *rt, JSInterruptHandler *cb, void *opaque)
     rt->interrupt_opaque = opaque;
 }
 
+/* surimono: an interrupt flag for time-accurate limits. The handler is
+   asked every JS_INTERRUPT_COUNTER_INIT polls; with a flag, every poll
+   also reads `*flag`, and while it is nonzero calls the handler at once,
+   so the time between a deadline and the stop no longer depends on how
+   much work lies between polls. The host
+   sets it from any thread (a watchdog timer, say) and clears it itself;
+   it must outlive the runtime, and the handler must answer true while it
+   is set, or every poll asks again. NULL removes it. Takes effect in every
+   context of `rt`, existing and future. */
+void surimono_set_interrupt_flag(JSRuntime *rt, uint32_t *flag)
+{
+    struct list_head *el;
+    js_interrupt_flag_t *f;
+
+    f = flag ? (js_interrupt_flag_t *)flag : &js_interrupt_flag_clear;
+    rt->interrupt_flag = f;
+    list_for_each(el, &rt->context_list) {
+        JSContext *ctx = list_entry(el, JSContext, link);
+        ctx->interrupt_flag = f;
+    }
+}
+
 void JS_SetCanBlock(JSRuntime *rt, bool can_block)
 {
     rt->can_block = can_block;
@@ -2880,6 +2927,7 @@ JSContext *JS_NewContextRaw(JSRuntime *rt)
         return NULL;
     }
     ctx->rt = rt;
+    ctx->interrupt_flag = rt->interrupt_flag; /* surimono */
     list_add_tail(&ctx->link, &rt->context_list);
     for(i = 0; i < rt->class_count; i++)
         ctx->class_proto[i] = JS_NULL;
@@ -8686,10 +8734,12 @@ static no_inline __exception int __js_poll_interrupts(JSContext *ctx)
     return 0;
 }
 
-/* surimono: identifies this patched build; see vendor/rquickjs-sys/PATCHES.md. */
+/* surimono: identifies this patched build; see vendor/rquickjs-sys/PATCHES.md.
+   3: both the allocation-failure hardening (p1-rt-15) and the interrupt
+   flag (p1-rt-14), each of which was level 2 on its own branch. */
 int surimono_quickjs_patch_level(void)
 {
-    return 2;
+    return 3;
 }
 
 /* surimono: with enabled = 0, every later allocation of this runtime goes
@@ -8704,7 +8754,11 @@ void surimono_quickjs_set_arena(JSRuntime *rt, int enabled)
 
 static inline __exception int js_poll_interrupts(JSContext *ctx)
 {
-    if (unlikely(--ctx->interrupt_counter <= 0)) {
+    /* surimono: also ask the handler whenever the host's interrupt flag is
+       set, so a deadline stops the script at its next poll (see
+       surimono_set_interrupt_flag). */
+    if (unlikely(--ctx->interrupt_counter <= 0 ||
+                 js_interrupt_flag_get(ctx->interrupt_flag))) {
         return __js_poll_interrupts(ctx);
     } else {
         return 0;
@@ -12518,7 +12572,9 @@ static js_limb_t js_mp_add_mul1(js_limb_t *tabr, const js_limb_t *taba, js_limb_
 }
 
 /* size of the result : op1_size + op2_size. */
-static void js_mp_mul_basecase(js_limb_t *result,
+/* surimono: takes `ctx` and polls once per row, returning -1 after an
+   interrupt; see js_bigint_mul. */
+static __exception int js_mp_mul_basecase(JSContext *ctx, js_limb_t *result,
                             const js_limb_t *op1, js_limb_t op1_size,
                             const js_limb_t *op2, js_limb_t op2_size)
 {
@@ -12527,9 +12583,12 @@ static void js_mp_mul_basecase(js_limb_t *result,
 
     result[op1_size] = js_mp_mul1(result, op1, op1_size, op2[0], 0);
     for(i=1;i<op2_size;i++) {
+        if (js_poll_interrupts(ctx))
+            return -1;
         r = js_mp_add_mul1(result + i, op1, op1_size, op2[i]);
         result[i + op1_size] = r;
     }
+    return 0;
 }
 
 /* tabr[] -= taba[] * b. Return the value to substract to the high
@@ -12609,7 +12668,10 @@ static js_limb_t js_mp_div1norm(js_limb_t *tabr, const js_limb_t *taba, js_limb_
    - 1] must be >= 1 << (JS_LIMB_BITS - 1). na - nb must be >= 0. 'taba'
    is modified and contains the remainder (nb limbs). tabq[0..na-nb]
    contains the quotient with tabq[na - nb] <= 1. */
-static void js_mp_divnorm(js_limb_t *tabq, js_limb_t *taba, js_limb_t na,
+/* surimono: takes `ctx` and polls once per quotient limb, returning -1
+   after an interrupt; see js_bigint_mul. */
+static __exception int js_mp_divnorm(JSContext *ctx, js_limb_t *tabq,
+                       js_limb_t *taba, js_limb_t na,
                        const js_limb_t *tabb, js_limb_t nb)
 {
     js_limb_t r, a, c, q, v, b1, b1_inv, n, dummy_r;
@@ -12618,7 +12680,7 @@ static void js_mp_divnorm(js_limb_t *tabq, js_limb_t *taba, js_limb_t na,
     b1 = tabb[nb - 1];
     if (nb == 1) {
         taba[0] = js_mp_div1norm(tabq, taba, na, b1, 0);
-        return;
+        return 0;
     }
     n = na - nb;
 
@@ -12642,6 +12704,8 @@ static void js_mp_divnorm(js_limb_t *tabq, js_limb_t *taba, js_limb_t na,
     }
 
     for(i = n - 1; i >= 0; i--) {
+        if (js_poll_interrupts(ctx))
+            return -1;
         if (unlikely(taba[i + nb] >= b1)) {
             q = -1;
         } else if (b1_inv) {
@@ -12674,6 +12738,7 @@ static void js_mp_divnorm(js_limb_t *tabq, js_limb_t *taba, js_limb_t na,
         }
         tabq[i] = q;
     }
+    return 0;
 }
 
 /* 1 <= shift <= JS_LIMB_BITS - 1 */
@@ -12964,7 +13029,13 @@ static JSBigInt *js_bigint_mul(JSContext *ctx, const JSBigInt *a,
     r = js_bigint_new(ctx, a->len + b->len);
     if (!r)
         return NULL;
-    js_mp_mul_basecase(r->tab, a->tab, a->len, b->tab, b->len);
+    /* surimono: the schoolbook kernels are quadratic in the size, so they
+       poll as they go: multiply per row, divide per quotient limb, decimal
+       toString and parsing per limb. */
+    if (js_mp_mul_basecase(ctx, r->tab, a->tab, a->len, b->tab, b->len)) {
+        js_free(ctx, r);
+        return NULL;
+    }
     /* correct the result if negative operands (no overflow is
        possible) */
     if (js_bigint_sign(a))
@@ -13054,7 +13125,13 @@ static JSBigInt *js_bigint_divrem(JSContext *ctx, const JSBigInt *a,
 
     //    js_bigint_dump1(ctx, "a", r->tab, na);
     //    js_bigint_dump1(ctx, "b", tabb, nb);
-    js_mp_divnorm(q->tab, r->tab, na, tabb, nb);
+    /* surimono: polls; see js_bigint_mul. */
+    if (js_mp_divnorm(ctx, q->tab, r->tab, na, tabb, nb)) {
+        js_free(ctx, q);
+        js_free(ctx, r);
+        js_free(ctx, tabb);
+        return NULL;
+    }
     js_free(ctx, tabb);
 
     if (is_rem) {
@@ -13266,13 +13343,17 @@ static JSBigInt *js_bigint_pow(JSContext *ctx, const JSBigInt *a, JSBigInt *b)
     memcpy(r->tab, a->tab, a->len * sizeof(a->tab[0]));
     for(i = n_bits - 2; i >= 0; i--) {
         r1 = js_bigint_mul(ctx, r, r);
-        if (!r1)
+        if (!r1) {
+            js_free(ctx, r);
             return NULL;
+        }
         js_free(ctx, r);
         r = r1;
         if ((e >> i) & 1) {
             r1 = js_bigint_mul(ctx, r, a);
-            if (!r1)
+            if (!r1) {
+                js_free(ctx, r);
                 return NULL;
+            }
             js_free(ctx, r);
             r = r1;
@@ -13567,6 +13648,12 @@ static JSBigInt *js_bigint_from_string(JSContext *ctx,
         len = 1;
         r->tab[0] = 0;
         for(;;) {
+            /* surimono: quadratic in the length; poll per limb (see
+               js_bigint_mul). */
+            if (js_poll_interrupts(ctx)) {
+                js_free(ctx, r);
+                return NULL;
+            }
             /* XXX: slow */
             v = 0;
             for(i = 0; i < digits_per_limb; i++) {
@@ -13748,6 +13835,13 @@ static JSValue js_bigint_to_string1(JSContext *ctx, JSValueConst val, int radix)
             radix_base = js_radix_base_table[radix - 2];
             len = r->len;
             for(;;) {
+                /* surimono: quadratic in the length; poll per limb (see
+                   js_bigint_mul). */
+                if (js_poll_interrupts(ctx)) {
+                    js_free(ctx, tmp);
+                    js_free(ctx, buf);
+                    return JS_EXCEPTION;
+                }
                 /* remove leading zero limbs */
                 while (len > 1 && r->tab[len - 1] == 0)
                     len--;
@@ -47410,10 +47504,29 @@ static int string_indexof_char(JSString *p, int c, int from)
     return -1;
 }
 
-static int string_indexof(JSString *p1, JSString *p2, int from)
+/* surimono: the string searches here are naive, up to len1 * len2
+   character comparisons in one native call, so they count their
+   comparisons and poll every JS_SEARCH_POLL_UNITS of them (tens of
+   microseconds of work). */
+#define JS_SEARCH_POLL_UNITS (1 << 16)
+
+static inline __exception int js_search_poll(JSContext *ctx, int64_t *work,
+                                             int64_t cost)
+{
+    *work += cost;
+    if (likely(*work < JS_SEARCH_POLL_UNITS))
+        return 0;
+    *work = 0;
+    return js_poll_interrupts(ctx);
+}
+
+/* surimono: takes `ctx` to poll (see js_search_poll); returns -2 after an
+   interrupt, with the exception pending. */
+static int string_indexof(JSContext *ctx, JSString *p1, JSString *p2, int from)
 {
     /* assuming 0 <= from <= p1->len */
     int c, i, j, len1 = p1->len, len2 = p2->len;
+    int64_t work = 0;
     if (len2 == 0)
         return from;
     for (i = from, c = string_get(p2, 0); i + len2 <= len1; i = j + 1) {
@@ -47422,6 +47535,8 @@ static int string_indexof(JSString *p1, JSString *p2, int from)
             break;
         if (!string_cmp(p1, p2, j + 1, 1, len2 - 1))
             return j;
+        if (js_search_poll(ctx, &work, (int64_t)j - i + len2))
+            return -2;
     }
     return -1;
 }
@@ -47559,6 +47674,7 @@ static JSValue js_string_indexOf(JSContext *ctx, JSValueConst this_val,
     }
     ret = -1;
     if (len >= v_len && inc * (stop - start) >= 0) {
+        int64_t work = 0; /* surimono: see js_search_poll */
         for (i = start;; i += inc) {
             if (!string_cmp(p, p1, i, 0, v_len)) {
                 ret = i;
@@ -47566,6 +47682,8 @@ static JSValue js_string_indexOf(JSContext *ctx, JSValueConst this_val,
             }
             if (i == stop)
                 break;
+            if (js_search_poll(ctx, &work, v_len))
+                goto fail;
         }
     }
     JS_FreeValue(ctx, str);
@@ -47625,6 +47743,7 @@ static JSValue js_string_includes(JSContext *ctx, JSValueConst this_val,
         start = stop = pos;
     }
     if (start >= 0 && start <= stop) {
+        int64_t work = 0; /* surimono: see js_search_poll */
         for (i = start;; i++) {
             if (!string_cmp(p, p1, i, 0, v_len)) {
                 ret = 1;
@@ -47632,6 +47751,8 @@ static JSValue js_string_includes(JSContext *ctx, JSValueConst this_val,
             }
             if (i == stop)
                 break;
+            if (js_search_poll(ctx, &work, v_len))
+                goto fail;
         }
     }
  done:
@@ -47887,6 +48008,10 @@ static JSValue js_string_replace(JSContext *ctx, JSValueConst this_val,
     endOfLastMatch = 0;
     is_first = true;
     for(;;) {
+        /* surimono: poll once per match; with a non-function replacement
+           this loop calls no script code that would. */
+        if (js_poll_interrupts(ctx))
+            goto exception;
         if (unlikely(searchp->len == 0)) {
             if (is_first)
                 pos = 0;
@@ -47895,7 +48020,9 @@ static JSValue js_string_replace(JSContext *ctx, JSValueConst this_val,
             else
                 pos = endOfLastMatch + 1;
         } else {
-            pos = string_indexof(sp, searchp, endOfLastMatch);
+            pos = string_indexof(ctx, sp, searchp, endOfLastMatch);
+            if (pos < -1)
+                goto exception;
         }
         if (pos < 0) {
             if (is_first) {
@@ -48006,7 +48133,12 @@ static JSValue js_string_split(JSContext *ctx, JSValueConst this_val,
     }
     q = p;
     for (q = p; (q += !r) <= s - r - !r; q = p = e + r) {
-        e = string_indexof(sp, rp, q);
+        /* surimono: poll once per piece too; see string_indexof. */
+        if (js_poll_interrupts(ctx))
+            goto exception;
+        e = string_indexof(ctx, sp, rp, q);
+        if (e < -1)
+            goto exception;
         if (e < 0)
             break;
         T = js_sub_string(ctx, sp, p, e);
@@ -61550,27 +61682,89 @@ static int js_TA_cmp_generic(const void *a, const void *b, void *opaque) {
     return cmp;
 }
 
-/* surimono: the typed comparators are plain C and never poll, so sorting
-   a large typed array ran far past the deadline. This wrapper polls on
-   every comparison (js_poll_interrupts asks the handler every 10000) and,
-   once interrupted, answers 0 so rqsort finishes in one linear pass. */
+/* surimono: the typed comparators are plain C with no polls of their own.
+   These wrappers check the
+   host's interrupt flag (see surimono_set_interrupt_flag) on each
+   comparison and, once interrupted, answer 0 so rqsort finishes in one
+   linear pass. The check is loads only: a countdown or the context's
+   interrupt counter is a load-store chain through memory on every
+   comparison, and a poll per comparison through one generic wrapper cost
+   20-26% of a large sort. There is one wrapper per element type, so the
+   comparator is a direct call the compiler can inline. Without a host
+   flag (the static zero flag) a default typed-array sort is not
+   interrupted; surimono always sets one. */
+typedef int js_TA_cmp_fn(const void *a, const void *b, void *opaque);
+
 struct TA_poll_sort_context {
     JSContext *ctx;
-    int (*cmp)(const void *a, const void *b, void *opaque);
+    js_TA_cmp_fn *cmp; /* used by js_TA_cmp_poll only */
+    js_interrupt_flag_t *flag;
     int exception;
 };
 
-static int js_TA_cmp_poll(const void *a, const void *b, void *opaque) {
-    struct TA_poll_sort_context *psc = opaque;
-    if (unlikely(psc->exception))
-        return 0;
-    if (unlikely(js_poll_interrupts(psc->ctx))) {
+/* Called while the interrupt flag is set: true once interrupted, and from
+   then on for every comparison. */
+static no_inline bool js_TA_sort_poll(struct TA_poll_sort_context *psc)
+{
+    if (psc->exception)
+        return true;
+    if (js_poll_interrupts(psc->ctx)) {
         psc->exception = 1;
-        return 0;
+        return true;
     }
+    return false;
+}
+
+#define JS_TA_CMP_POLL(T)                                               \
+static int js_TA_cmp_poll_##T(const void *a, const void *b, void *opaque) \
+{                                                                       \
+    struct TA_poll_sort_context *psc = opaque;                          \
+    if (unlikely(js_interrupt_flag_get(psc->flag)) && js_TA_sort_poll(psc)) \
+        return 0;                                                       \
+    return js_TA_cmp_##T(a, b, NULL);                                   \
+}
+
+JS_TA_CMP_POLL(int8)
+JS_TA_CMP_POLL(uint8)
+JS_TA_CMP_POLL(int16)
+JS_TA_CMP_POLL(uint16)
+JS_TA_CMP_POLL(int32)
+JS_TA_CMP_POLL(uint32)
+JS_TA_CMP_POLL(int64)
+JS_TA_CMP_POLL(uint64)
+JS_TA_CMP_POLL(float16)
+JS_TA_CMP_POLL(float32)
+JS_TA_CMP_POLL(float64)
+
+/* For a comparator without its own wrapper (none today). */
+static int js_TA_cmp_poll(const void *a, const void *b, void *opaque)
+{
+    struct TA_poll_sort_context *psc = opaque;
+    if (unlikely(js_interrupt_flag_get(psc->flag)) && js_TA_sort_poll(psc))
+        return 0;
     return psc->cmp(a, b, NULL);
 }
 
+/* The polling wrapper for typed comparator `cmp`. */
+static js_TA_cmp_fn *js_TA_cmp_poll_for(js_TA_cmp_fn *cmp)
+{
+#define JS_TA_CMP_POLL_CASE(T) \
+    if (cmp == js_TA_cmp_##T) return js_TA_cmp_poll_##T;
+    JS_TA_CMP_POLL_CASE(int8)
+    JS_TA_CMP_POLL_CASE(uint8)
+    JS_TA_CMP_POLL_CASE(int16)
+    JS_TA_CMP_POLL_CASE(uint16)
+    JS_TA_CMP_POLL_CASE(int32)
+    JS_TA_CMP_POLL_CASE(uint32)
+    JS_TA_CMP_POLL_CASE(int64)
+    JS_TA_CMP_POLL_CASE(uint64)
+    JS_TA_CMP_POLL_CASE(float16)
+    JS_TA_CMP_POLL_CASE(float32)
+    JS_TA_CMP_POLL_CASE(float64)
+#undef JS_TA_CMP_POLL_CASE
+    return js_TA_cmp_poll;
+}
+
 static JSValue js_typed_array_sort(JSContext *ctx, JSValueConst this_val,
                                    int argc, JSValueConst *argv)
 {
@@ -61713,9 +61907,12 @@ static JSValue js_typed_array_sort(JSContext *ctx, JSValueConst this_val,
         done:
             js_free(ctx, array_idx);
         } else {
-            /* surimono: poll while sorting; see js_TA_cmp_poll. */
-            struct TA_poll_sort_context psc = { ctx, cmpfun, 0 };
-            rqsort(p->u.array.u.ptr, len, elt_size, js_TA_cmp_poll, &psc);
+            /* surimono: poll while sorting; see js_TA_cmp_poll_for. */
+            struct TA_poll_sort_context psc = {
+                ctx, cmpfun, ctx->interrupt_flag, 0
+            };
+            rqsort(p->u.array.u.ptr, len, elt_size,
+                   js_TA_cmp_poll_for(cmpfun), &psc);
             if (psc.exception)
                 return JS_EXCEPTION;
         }
```

## Regexp per-match polls (P1-RT-23)

Review of #47: `'a'.repeat(1 << 25).replace(/(?:)/g, '')` stopped at
1,174 ms against a 250 ms limit. `lre_exec` polls every 10,000 steps of
one match attempt and restarts that count at every call, so a pattern that
matches in a few steps never polls, and a C loop over its matches must.
The loops audited: `js_regexp_Symbol_match` (global), `Symbol.split` and
the first loop of `Symbol.replace` call `exec` through `JS_RegExpExec`,
and `matchAll`'s iterator is called from script or a native iteration;
all of them go through `JS_CallInternal`, which polls on entry, so they
already polled per match. Two loops did not. With patches 1 to 18,
patch level 4.

19. **`JS_RegExpDelete` polls per match.** It is the fast path of
    `replace`/`replaceAll` with `''` on a standard global or sticky regexp
    and calls `lre_exec` directly, so nothing in it polled. It now calls
    `js_poll_interrupts` before each match and fails after an interrupt,
    freeing its buffers on the existing path. Debug build, 250 ms limit:
    the empty pattern on 32M chars took 997 ms unpatched and stops at
    250 ms; an 8-way alternation on 64M chars 4,587 ms and 253 ms.
20. **`js_regexp_Symbol_replace` polls per match in its second loop**, the
    one that builds the result from the collected matches. With a string
    replacement it calls no script code, so a match list collected just
    inside the limit was processed in one more unpolled pass of about the
    same length. Results are unchanged.
21. **`surimono_quickjs_patch_level()` returns 4.** surimono-core requires
    4 (`engine::REQUIRED_PATCH_LEVEL`). The level is part of the bytecode
    cache's engine fingerprint, so old cached bytecode is refused, as with
    any engine change.

Tests: group F in `crates/core/tests/hostile.rs` (ten vectors: the replace
fast path, `replaceAll`, string and function replacements, `split`,
`match`, `matchAll`); `interrupted_natives_leak_nothing` (three group F
vectors) and `regexp_replace_results_are_unchanged` in
`crates/core/src/sandbox.rs`.

### Diff of patches 19 to 21 (on top of patches 1 to 18)

```diff
--- a/quickjs/quickjs.c (with patches 1 to 18)
+++ b/quickjs/quickjs.c
@@ -8736,10 +8736,11 @@ static no_inline __exception int __js_poll_interrupts(JSContext *ctx)
 
 /* surimono: identifies this patched build; see vendor/rquickjs-sys/PATCHES.md.
    3: both the allocation-failure hardening (p1-rt-15) and the interrupt
-   flag (p1-rt-14), each of which was level 2 on its own branch. */
+   flag (p1-rt-14), each of which was level 2 on its own branch.
+   4: also the per-match polls in the regexp replace loops (p1-rt-23). */
 int surimono_quickjs_patch_level(void)
 {
-    return 3;
+    return 4;
 }
 
 /* surimono: with enabled = 0, every later allocation of this runtime goes
@@ -50244,6 +50245,13 @@ static JSValue JS_RegExpDelete(JSContext *ctx, JSValueConst this_val, JSValue ar
         if (last_index > str->len)
             break;
 
+        /* surimono: poll once per match. Each lre_exec starts its own
+           step counter, so a pattern that matches in a few steps never
+           reaches the regexp engine's poll, and this loop calls no script
+           code: 'a'.repeat(1 << 25).replace(/(?:)/g, '') made 32M
+           matches in one native call. */
+        if (js_poll_interrupts(ctx))
+            goto fail;
         ret = lre_exec(capture, re_bytecode,
                        str_buf, last_index, str->len, shift, ctx);
         if (ret != 1) {
@@ -50747,6 +50755,13 @@ static JSValue js_regexp_Symbol_replace(JSContext *ctx, JSValueConst this_val,
     nextSourcePosition = 0;
     for(j = 0; j < results->len; j++) {
         JSValue result;
+        /* surimono: poll once per match. The loop above polls through
+           each exec call, but this one may call no script code (a string
+           replacement), so a match list collected just inside the limit
+           was replaced in one more unpolled pass of about the same
+           length. */
+        if (js_poll_interrupts(ctx))
+            goto exception;
         result = results->arr[j];
         if (js_get_length32(ctx, &nCaptures, result) < 0)
             goto exception;
```

## Leak count at teardown (P1-RT-18)

Review of #45: patch 11 lets `JS_FreeRuntime` leave objects that an error
path leaked instead of aborting. Their small blocks go back to the
allocator with the arenas, so no byte count saw such a leak in production
(arena on); only the sweep's arena-off runs did. With patches 1 to 21,
patch level 5.

22. **`surimono_set_leak_count(rt, count)`**: `JS_FreeRuntime` stores in
    `*count` how many objects were still on `gc_obj_list` (after its final
    GC) before it drops them from the list. A new `JSRuntime.leak_count`
    field holds the pointer (NULL, the default, counts nothing).
    surimono-core points it into the runtime's `LimitState` next to the
    interrupt flag, and when rquickjs drops the runtime's allocator (after
    `JS_FreeRuntime` returned) adds it, and any bytes the runtime never gave
    back, to process-wide counters (`surimono_core::teardown_leaks`, hidden
    from the docs). The soak tests require them to stay at zero; the
    out-of-memory sweeps and `interrupted_natives_leak_nothing` require
    each runtime's count to be zero.
23. **`surimono_quickjs_patch_level()` returns 5.** surimono-core requires
    5 (`engine::REQUIRED_PATCH_LEVEL`).

Tests: `teardown_counts_leaked_objects` and `a_clean_teardown_leaks_nothing`
in `crates/core/src/engine.rs`; the sweeps in `crates/core/src/sandbox.rs`;
the soak tests in `crates/islands/tests/soak.rs`.

### Diff of patches 22 and 23 (on top of patches 1 to 21)

```diff
--- a/quickjs/quickjs.c (with patches 1 to 21)
+++ b/quickjs/quickjs.c
@@ -402,6 +402,9 @@
     /* surimono: never NULL; points at js_interrupt_flag_clear unless the
        host set its own flag. Copied to every context. */
     js_interrupt_flag_t *interrupt_flag;
+    /* surimono: where JS_FreeRuntime reports how many objects leaked
+       (see surimono_set_leak_count); NULL when nobody asked. */
+    uint32_t *leak_count;
 
     JSPromiseHook *promise_hook;
     void *promise_hook_opaque;
@@ -2531,6 +2534,17 @@
     }
 }
 
+/* surimono: JS_FreeRuntime stores in *count how many objects were still
+   alive when the runtime was freed: objects some error path leaked a
+   reference to (see the note there), which it leaves to the arenas
+   instead of aborting. The arena blocks go back to the allocator with the
+   arenas, so no byte count shows these leaks; this count does. The
+   pointer must stay valid until JS_FreeRuntime returns. NULL removes it. */
+void surimono_set_leak_count(JSRuntime *rt, uint32_t *count)
+{
+    rt->leak_count = count;
+}
+
 void JS_SetCanBlock(JSRuntime *rt, bool can_block)
 {
     rt->can_block = can_block;
@@ -2759,9 +2773,19 @@
        rather than abort the process. Large blocks they own (from the
        backing allocator) stay allocated until the process exits.
        surimono's tests check that a runtime gives back every byte it
-       allocated. */
-    if (!list_empty(&rt->gc_obj_list))
+       allocated, and the host can have the leaked objects counted
+       (surimono_set_leak_count). */
+    if (!list_empty(&rt->gc_obj_list)) {
+        if (rt->leak_count) {
+            uint32_t n = 0;
+            list_for_each(el, &rt->gc_obj_list) {
+                if (n < UINT32_MAX)
+                    n++;
+            }
+            *rt->leak_count = n;
+        }
         init_list_head(&rt->gc_obj_list);
+    }
 
     /* free the classes */
     for(i = 0; i < rt->class_count; i++) {
@@ -8737,10 +8761,11 @@
 /* surimono: identifies this patched build; see vendor/rquickjs-sys/PATCHES.md.
    3: both the allocation-failure hardening (p1-rt-15) and the interrupt
    flag (p1-rt-14), each of which was level 2 on its own branch.
-   4: also the per-match polls in the regexp replace loops (p1-rt-23). */
+   4: also the per-match polls in the regexp replace loops (p1-rt-23).
+   5: also the leak count at teardown (p1-rt-18). */
 int surimono_quickjs_patch_level(void)
 {
-    return 4;
+    return 5;
 }
 
 /* surimono: with enabled = 0, every later allocation of this runtime goes
```

## Two more failure paths (P1-RT-36)

Found while writing the quickjs-ng issue drafts (P1-RT-20): a harness that
refuses each allocation of a script, run on the vendored engine, without
this sandbox's stop flag around it. Drafts:
`docs/upstream/quickjs-ng/07-add-arg-unchecked.md` and
`08-array-add-property.md`. With patches 1 to 23, patch level 6.

24. **`JS_NewObjectFromShape` checks the array's `length` property.** A
    comment said `add_property` there "cannot fail" and only ran for the first
    array. Any `new.target` reaches it: when the prototype's empty shape is
    shared, `add_property` clones it, and a refused clone left `pr` NULL. A
    one-line script crashed the process through the public API:
    `class A extends Array {}` in a loop with `Object.create(A.prototype)` and
    `new A()` under an 8 MiB limit, 5 of 380 pad sizes (47 of 380 on
    quickjs-ng master). The object isn't on the GC list yet, so the check
    frees it by hand.
25. **Unnamed destructured parameters check `add_arg`.** -1 went into the
    bytecode as `OP_get_arg 0xffff`, a read of the C stack about 1 MiB past the
    frame when the function ran. `add_arg` fails two ways. A refused
    allocation raises the sandbox's stop flag, which ends the script at the
    call's own poll. Going past `JS_MAX_LOCAL_VARS` raises no flag, so a
    function with that many destructured parameters reached the read. Run on
    unpatched quickjs-ng e22ce4de, a generator or async function with 65,536
    `{} = {}` parameters called with no arguments crashes (SIGSEGV;
    heap-use-after-free READ in `js_dup` from `OP_get_arg` under ASan). It is
    now the `SyntaxError` the code always meant to throw.
26. **`surimono_quickjs_patch_level()` returns 6**, and surimono-core
    requires 6. The level is part of the bytecode fingerprint, so bytecode
    cached by an older engine is refused.

Tests: `SYNTAX_SCRIPT` in `crates/core/src/sandbox.rs` now builds such an
array, so `out_of_memory_never_aborts` refuses that allocation;
`too_many_destructured_parameters_is_a_syntax_error` covers patch 25.

### Diff of patches 24 and 25 (on top of patches 1 to 23)

```diff
--- a/quickjs/quickjs.c (with patches 1 to 23)
+++ b/quickjs/quickjs.c
@@ JS_NewObjectFromShape
-                    /* only used for the first array */
-                    /* cannot fail */
+                    /* surimono: not only the first array: any Array subclass
+                       or other new.target lands here. When the prototype's
+                       empty shape is shared, add_property clones it, which
+                       can fail: `new A()` crashed on a NULL `pr` under a
+                       memory limit. p isn't on the GC list yet. */
                     pr = add_property(ctx, p, JS_ATOM_length,
                                       JS_PROP_WRITABLE | JS_PROP_LENGTH);
+                    if (unlikely(!pr)) {
+                        js_free_shape(ctx->rt, p->shape);
+                        js_free(ctx, p->prop);
+                        js_free(ctx, p);
+                        return JS_EXCEPTION;
+                    }
                 }
@@ js_parse_function_decl2
                     idx = add_arg(ctx, fd, JS_ATOM_NULL);
+                    /* surimono: -1 would become OP_get_arg 0xffff, a read
+                       about 1 MiB past the frame when the function runs. */
+                    if (idx < 0)
+                        goto fail;
                     emit_op(s, OP_get_arg);
```

## Polls while compiling (P2-RT-03)

Nothing in a compile ran bytecode, so nothing in it polled, and `eval`,
`new Function`, `new RegExp` and `JSON.parse` ran to their end whatever the
limit. Some of that is quadratic in a small input: 60,000 `let`s in a function
(1 MiB) took 24 s, and `'x|'.repeat(1 << 22)` ran for minutes. A source-size cap
wouldn't have bounded those, so the fix is polls. Draft:
`docs/upstream/quickjs-ng/14-parser-never-polls.md`. With patches 1 to 26, patch
level 7. Precompiled bytecode goes through `JS_ReadObject`, which none of this
touches, so a load gets no slower.

27. **The tokenizer polls.** `next_token` polls at `redo:`, once per token and
    per skipped white space character. That is a count, not a source position,
    because `js_parse_skip_parens_token` scans the same tokens again. The loops
    that scan a single long token poll per byte: strings, templates, regexp
    literals and flags, identifiers, block and line comments. `js_atof` polls in
    its digit loops (also `Number(s)`, `parseFloat`); an interrupt there
    returns `JS_EXCEPTION` like its out-of-memory path. All of these fail
    through existing labels.
    - `js_parse_template` ignored a failure of `js_parse_string` with
      `do_throw` false (an invalid escape throws nothing). It now returns -1 if
      an exception is pending.
    - `simple_next_token` (`peek_token`) and `skip_shebang` take the context and
      poll. They can't report failure, so they stop early and leave the
      exception pending; the next `next_token` fails. Patch 1 makes that safe:
      the poll answers again at once. So is the discarded `next_token`
      failure in `js_parse_skip_parens_token`: the seek after it polls again.
    - If a parse error replaces the interrupt, the sandbox still reports
      `TimeLimit` (`classify` asks the limit state first), and a script that
      catches the error is stopped at its next poll.
    - The parameter name checks poll once per parameter:
      `js_parse_function_check_names` and `js_parse_check_duplicate_parameter`.
      They run between two tokens, and 65,000 strict parameters took 803 ms.
28. **`JSON.parse` polls.** `json_next_token` at `redo:`, `json_parse_ident` and
    the digit loops of `json_parse_number`. `json_parse_string` polls per batch
    and caps its ASCII fast path at 64 KiB a batch.
29. **The compiler's passes poll.** `resolve_variables` (its first pass over
    the global variables too), `resolve_labels` and `compute_stack_size` poll
    once per instruction, at the point of an existing failure path. They cost
    about as much as parsing, and `resolve_scope_var` walks every variable in
    scope for each name. The cleanup after a stop here is proportional to the
    function: about half the time spent so far at worst.
    - The "more jump optimizations" loop of `resolve_labels` polls once per
      jump. It runs after that pass, and each jump it shrinks moves the rest of
      the bytecode and walks every label, later jump and source location, so a
      small input is quadratic there and only there. Review of #125:
      `if(a){...}` blocks, 1.76 MiB, took 23.7 s against a 2 s limit.
    - `add_eval_variables` returns an error and polls once per variable, and
      `js_create_function` checks it. For a direct eval, every variable of every
      enclosing function goes through `get_closure_var`, which walks the
      closure variables of each function in between, all before the first pass
      over the code.
30. **The regexp compiler polls.** `re_parse_poll` calls `lre_check_timeout` per
    term, class item and alternative, with no counter, since one step can cost
    milliseconds (a `memmove` of the bytecode written so far, a merge into a
    class table). It reports the reserved message `LRE_INTERRUPTED`, which
    `js_compile_regexp` turns into the uncatchable `JS_ThrowInterrupted`
    instead of a `SyntaxError`. This also covers regexp literals, which the
    parser compiles.
    - The string lists of `v` classes poll per string, since `\q{a|b|...}` is
      one class item however long it is: `parse_class_string_disjunction` (per
      alternative and per character), `re_string_list_op`,
      `re_string_list_canonicalize` and `re_emit_string_list`. The two list
      helpers report failure like an allocation that failed, so
      `REParseState.interrupted` tells `re_parse_out_of_memory` to keep the
      error.
31. **`surimono_quickjs_patch_level()` returns 7**, and surimono-core requires 7.

Left, all linear in data the memory limit bounds: the conversion in `js_atof`
after its digit loops (`js_atod` on 64 MiB of digits, about 100 to 200 ms),
`strtod` in `json_parse_number`, `JS_WriteObject` in `Sandbox::compile`, the
sort in `re_emit_string_list` (at most 350 to 450 ms past a 3 to 4 s limit on 35
MiB of `\q{}` strings, with the cleanup), and straight-line bytecode, which has
no back-edge to poll at.

Timings, release build, 50 ms limit, `TimeLimit` in all of the "after" column
(`Sandbox::compile`, or a call with the string already set):

| Input | Before | After |
|---|---|---|
| 60 MiB of statements in an uncalled function | 6,146 ms | 56 ms |
| The same through `eval`, direct | 6,334 ms | 55 ms |
| The same through `new Function` | 7,173 ms | 73 ms |
| 60 MiB array literal | 7,412 ms | 54 ms |
| 60,000 `let`s, 1 MiB | 24,255 ms | 50 ms |
| 131,072 `var`s, 1.4 MiB | 7,543 ms | 51 ms |
| 60 MiB regexp literal | 1,865 ms | 50 ms |
| `new RegExp`, 60 MiB of literals | 1,207 ms | 50 ms |
| `new RegExp('x\|'.repeat(8 << 20))` | stopped by hand after minutes | 50 ms |
| `if(a){...}` blocks, 1.76 MiB | 23,700 ms (2 s limit) | 2,001 ms (2 s limit), 59 ms |
| 65,000 strict parameters, 444 KiB | 803 ms | 51 ms |
| 32,768 `var`s around 8 nested arrows and a direct `eval` | 3,388 ms | 51 ms |
| 4M `\q{}` strings, 35 MiB | 2,764 ms | 54 ms |

The stop itself is prompt; the time after it is unwinding. Past 1 s, for a
16 MiB source, unwinding takes 100 to 700 ms, still under twice the limit.
Compiling the 1 MiB ECharts bundle: best of 15, three runs each, 142 to 153 ms
before, 139 to 144 ms after.

Other compile passes: 197 program shapes (jumps, `switch`, `try`, labels, scopes,
closures, direct `eval`, `with`, classes, destructuring, templates, literals,
generators, regexps, `JSON.parse`), each doubled in size until it took 6 s or
hit an engine cap, then run at that size under a 50 ms limit. About a fifth are
quadratic or worse when run to their end (a ratio of 4 per doubling, up to 8.9
for many closures with a direct `eval`), the rest linear. That found
`add_eval_variables` and the `v` class strings above; with those polled, every
shape stopped within 112 ms.

Tests: group G in `crates/core/tests/hostile.rs` (44 vectors,
`group_g_compile_interrupts_are_uncatchable` and
`group_g_parameter_checks_stop`); `interrupted_natives_leak_nothing`
(six group G vectors), `interrupted_compiles_leak_nothing`,
`interrupted_class_strings_leak_nothing`, `class_strings_are_unchanged`,
`json_strings_are_unchanged_around_a_batch` and `compile_errors_are_unchanged` in
`crates/core/src/sandbox.rs`. The out-of-memory sweeps now stop at the poll after
each refused allocation inside the parser and the passes.

### Diff of patches 27 to 31 (on top of patches 1 to 26)

```diff
--- a/quickjs/libregexp.c
+++ b/quickjs/libregexp.c
@@ -99,4 +99,5 @@ typedef struct {
     void *opaque;
     DynBuf group_names;
+    bool interrupted; /* surimono: see re_parse_poll */
     union {
         char error_msg[TMP_BUF_SIZE];
@@ -320,5 +321,10 @@ static int re_string_add(REStringList *s, int len, const uint32_t *buf)
 
 /* a = a op b */
-static int re_string_list_op(REStringList *a, REStringList *b, int op)
+/* surimono: see re_parse_poll; a class's string list has one entry per
+   \q{} alternative, up to one per two bytes of the pattern. */
+static int re_parse_poll(REParseState *s);
+
+static int re_string_list_op(REParseState *s, REStringList *a, REStringList *b,
+                             int op)
 {
     int i, ret;
@@ -333,4 +339,7 @@ static int re_string_list_op(REStringList *a, REStringList *b, int op)
             for(i = 0; i < b->hash_size; i++) {
                 for(p = b->hash_table[i]; p != NULL; p = p->next) {
+                    /* surimono */
+                    if (re_parse_poll(s))
+                        return -1;
                     if (re_string_find2(a, p->len, p->buf, p->hash, true) < 0)
                         return -1;
@@ -347,4 +356,7 @@ static int re_string_list_op(REStringList *a, REStringList *b, int op)
                 if (p == NULL)
                     break;
+                /* surimono */
+                if (re_parse_poll(s))
+                    return -1;
                 ret = re_string_find2(b, p->len, p->buf, p->hash, false);
                 if (op == CR_OP_SUB)
@@ -393,4 +405,9 @@ static int re_string_list_canonicalize(REParseState *s1,
         for(i = 0; i < a->hash_size; i++) {
             for(p = a->hash_table[i]; p != NULL; p = p->next) {
+                /* surimono */
+                if (re_parse_poll(s1)) {
+                    re_string_list_free(a);
+                    return -1;
+                }
                 for(j = 0; j < p->len; j++) {
                     p->buf[j] = lre_canonicalize(p->buf[j], is_unicode);
@@ -699,6 +716,26 @@ static int JS_PRINTF_FORMAT_ATTR(2, 3) re_parse_error(REParseState *s, JS_PRINTF
 }
 
+/* surimono: compiling a pattern is quadratic in places (each alternative and
+   quantifier moves the bytecode written so far, each class item merges into a
+   sorted table, each forward \k scans the pattern), and nothing polled:
+   'x|'.repeat(8 << 20) ran for minutes. Called per term, class item and
+   alternative, without a counter, because one step can cost milliseconds.
+   The error is an uncatchable interrupt, not a SyntaxError; see
+   js_compile_regexp. */
+static int re_parse_poll(REParseState *s)
+{
+    if (lre_check_timeout(s->opaque)) {
+        s->interrupted = true;
+        return re_parse_error(s, "%s", LRE_INTERRUPTED);
+    }
+    return 0;
+}
+
 static int re_parse_out_of_memory(REParseState *s)
 {
+    /* surimono: re_string_list_op and re_string_list_canonicalize fail the
+       same way after an interrupt, with the error already set. */
+    if (s->interrupted)
+        return -1;
     return re_parse_error(s, "out of memory");
 }
@@ -1025,5 +1062,11 @@ static int parse_class_string_disjunction(REParseState *s, REStringList *cr,
     for(;;) {
         str.size = 0;
+        /* surimono */
+        if (re_parse_poll(s))
+            goto fail;
         while (*p != '}' && *p != '|') {
+            /* surimono */
+            if (re_parse_poll(s))
+                goto fail;
             c = get_class_atom(s, NULL, &p, true);
             if (c < 0)
@@ -1342,4 +1385,7 @@ static int re_emit_string_list(REParseState *s, const REStringList *sl)
         last_match_pos = -1;
         for(i = 0; i < n; i++) {
+            /* surimono */
+            if (re_parse_poll(s))
+                goto fail;
             p = tab[i];
             is_last = !has_empty_string && sl->cr.len == 0 && i == (n - 1);
@@ -1448,4 +1494,7 @@ static int re_parse_nested_class(REParseState *s, REStringList *cr, const uint8_
     is_first = true;
     for(;;) {
+        /* surimono */
+        if (re_parse_poll(s))
+            goto fail;
         if (*p == ']')
             break;
@@ -1506,5 +1555,5 @@ static int re_parse_nested_class(REParseState *s, REStringList *cr, const uint8_
                 if (c1 >= CLASS_RANGE_BASE) {
                 class_union:
-                    ret = re_string_list_op(cr, cr1, CR_OP_UNION);
+                    ret = re_string_list_op(s, cr, cr1, CR_OP_UNION);
                     re_string_list_free(cr1);
                     if (ret)
@@ -1522,4 +1571,7 @@ static int re_parse_nested_class(REParseState *s, REStringList *cr, const uint8_
                 /* handle '&&' */
                 for(;;) {
+                    /* surimono */
+                    if (re_parse_poll(s))
+                        goto fail;
                     if (*p == ']') {
                         break;
@@ -1531,5 +1583,5 @@ static int re_parse_nested_class(REParseState *s, REStringList *cr, const uint8_
                     if (re_parse_class_set_operand(s, cr1, &p))
                         goto fail;
-                    ret = re_string_list_op(cr, cr1, CR_OP_INTER);
+                    ret = re_string_list_op(s, cr, cr1, CR_OP_INTER);
                     re_string_list_free(cr1);
                     if (ret)
@@ -1539,4 +1591,7 @@ static int re_parse_nested_class(REParseState *s, REStringList *cr, const uint8_
                 /* handle '--' */
                 for(;;) {
+                    /* surimono */
+                    if (re_parse_poll(s))
+                        goto fail;
                     if (*p == ']') {
                         break;
@@ -1550,5 +1605,5 @@ static int re_parse_nested_class(REParseState *s, REStringList *cr, const uint8_
                     if (re_parse_class_set_operand(s, cr1, &p))
                         goto fail;
-                    ret = re_string_list_op(cr, cr1, CR_OP_SUB);
+                    ret = re_string_list_op(s, cr, cr1, CR_OP_SUB);
                     re_string_list_free(cr1);
                     if (ret)
@@ -2440,4 +2495,7 @@ static int re_parse_alternative(REParseState *s, bool is_backward_dir)
         if (*p == '|' || *p == ')')
             break;
+        /* surimono */
+        if (re_parse_poll(s))
+            return -1;
         term_start = s->byte_code.size;
         ret = re_parse_term(s, is_backward_dir);
@@ -2472,4 +2530,7 @@ static int re_parse_disjunction(REParseState *s, bool is_backward_dir)
         return -1;
     while (*s->buf_ptr == '|') {
+        /* surimono */
+        if (re_parse_poll(s))
+            return -1;
         s->buf_ptr++;
 
--- a/quickjs/libregexp.h
+++ b/quickjs/libregexp.h
@@ -51,4 +51,8 @@ extern "C" {
 #define LRE_GROUP_NAME_TRAILER_LEN 2
 
+/* surimono: the error message lre_compile gives when lre_check_timeout told
+   it to stop. No syntax error has this text. */
+#define LRE_INTERRUPTED "interrupted"
+
 uint8_t *lre_compile(int *plen, char *error_msg, int error_msg_size,
                      const char *buf, size_t buf_len, int re_flags,
--- a/quickjs/quickjs.c
+++ b/quickjs/quickjs.c
@@ -8774,8 +8774,10 @@ static no_inline __exception int __js_poll_interrupts(JSContext *ctx)
    5: also the leak count at teardown (p1-rt-18).
    6: also the failed length property of a new array and the unchecked
-   add_arg (p1-rt-36). */
+   add_arg (p1-rt-36).
+   7: also the polls in the parser, the compiler passes, JSON.parse and
+   the regexp compiler (p2-rt-03). */
 int surimono_quickjs_patch_level(void)
 {
-    return 6;
+    return 7;
 }
 
@@ -14037,4 +14039,10 @@ static JSValue js_atof(JSContext *ctx, const char *str, const char **pp,
                               p != p_start + 1 || p[-1] != '0') &&
                 js_to_digit((uint8_t)p[1]) < radix)) {
+        /* surimono: a numeric literal, or Number(s), can be as long as its
+           string. See next_token. */
+        if (js_poll_interrupts(ctx)) {
+            val = JS_EXCEPTION;
+            goto done;
+        }
         p++;
     }
@@ -14046,6 +14054,11 @@ static JSValue js_atof(JSContext *ctx, const char *str, const char **pp,
                 goto fail;
             while (js_to_digit((uint8_t)*p) < radix ||
-                   (*p == sep && js_to_digit((uint8_t)p[1]) < radix))
+                   (*p == sep && js_to_digit((uint8_t)p[1]) < radix)) {
+                if (js_poll_interrupts(ctx)) {
+                    val = JS_EXCEPTION;
+                    goto done;
+                }
                 p++;
+            }
         }
         if (p > p_start && (*p == 'e' || *p == 'E')) {
@@ -14059,6 +14072,11 @@ static JSValue js_atof(JSContext *ctx, const char *str, const char **pp,
             if (is_digit((uint8_t)*p1)) {
                 p = p1 + 1;
-                while (is_digit((uint8_t)*p) || (*p == sep && is_digit((uint8_t)p[1])))
+                while (is_digit((uint8_t)*p) || (*p == sep && is_digit((uint8_t)p[1]))) {
+                    if (js_poll_interrupts(ctx)) {
+                        val = JS_EXCEPTION;
+                        goto done;
+                    }
                     p++;
+                }
             }
         }
@@ -22830,4 +22848,7 @@ static __exception int js_parse_template_part(JSParseState *s,
         goto fail;
     for(;;) {
+        /* surimono: see next_token. */
+        if (js_poll_interrupts(s->ctx))
+            goto fail;
         if (p >= s->buf_end)
             goto unexpected_eof;
@@ -22901,4 +22922,7 @@ static __exception int js_parse_string(JSParseState *s, int sep,
         goto fail;
     for(;;) {
+        /* surimono: see next_token. */
+        if (js_poll_interrupts(s->ctx))
+            goto fail;
         if (p >= s->buf_end)
             goto invalid_char;
@@ -23048,4 +23072,7 @@ static __exception int js_parse_regexp(JSParseState *s)
         goto fail;
     for(;;) {
+        /* surimono: see next_token. */
+        if (js_poll_interrupts(s->ctx))
+            goto fail;
         if (p >= s->buf_end) {
         eof_error:
@@ -23102,4 +23129,7 @@ static __exception int js_parse_regexp(JSParseState *s)
     /* flags */
     for(;;) {
+        /* surimono: see next_token. */
+        if (js_poll_interrupts(s->ctx))
+            goto fail;
         c = utf8_decode(p, &p_next);
         /* no need to test for invalid UTF-8, 0xFFFD is not ident_next */
@@ -23225,4 +23255,7 @@ static JSAtom parse_ident(JSParseState *s, const uint8_t **pp,
         buf[ident_pos++] = '#';
     for(;;) {
+        /* surimono: see next_token. */
+        if (js_poll_interrupts(s->ctx))
+            goto done;
         if (c < 0x80) {
             buf[ident_pos++] = c;
@@ -23276,4 +23309,11 @@ static __exception int next_token(JSParseState *s)
     s->last_col_num = s->token.col_num;
  redo:
+    /* surimono: the parser runs no bytecode, so nothing polled during a
+       compile: 60 MiB of source took seconds past a 50 ms limit. Poll per
+       token and per skipped white space, and per byte in the loops that
+       scan one long token or comment. Not by source position:
+       js_parse_skip_parens_token scans the same tokens again. */
+    if (js_poll_interrupts(s->ctx))
+        goto fail;
     s->token.line_num = s->line_num;
     s->token.col_num = s->col_num;
@@ -23324,4 +23364,7 @@ static __exception int next_token(JSParseState *s)
             p += 2;
             for(;;) {
+                /* surimono: see the poll at redo. */
+                if (js_poll_interrupts(s->ctx))
+                    goto fail;
                 if (*p == '\0' && p >= s->buf_end) {
                     js_parse_error(s, "unexpected end of comment");
@@ -23359,4 +23402,7 @@ static __exception int next_token(JSParseState *s)
         skip_line_comment:
             for(;;) {
+                /* surimono: see the poll at redo. */
+                if (js_poll_interrupts(s->ctx))
+                    goto fail;
                 if (*p == '\0' && p >= s->buf_end)
                     break;
@@ -23760,7 +23806,13 @@ static int json_parse_string(JSParseState *s, const uint8_t **pp)
         }
 
+        /* surimono: see next_token. Batches are capped so this runs at
+           bounded intervals. */
+        if (js_poll_interrupts(s->ctx))
+            goto fail;
+
         // Fast path: batch consecutive ASCII characters
         const uint8_t *p_start = p;
-        while (p < s->buf_end && *p != '"' && *p != '\\' && *p >= 0x20 && *p < 0x80) {
+        const uint8_t *p_stop = s->buf_end - p > 65536 ? p + 65536 : s->buf_end;
+        while (p < p_stop && *p != '"' && *p != '\\' && *p >= 0x20 && *p < 0x80) {
             p++;
         }
@@ -23775,4 +23827,8 @@ static int json_parse_string(JSParseState *s, const uint8_t **pp)
             goto end_of_input;
 
+        /* surimono: a batch stopped by its cap, not by a special character. */
+        if (p == p_stop)
+            continue;
+
         c = *p++;
         if (c == '"')
@@ -23849,6 +23905,10 @@ static int json_parse_number(JSParseState *s, const uint8_t **pp)
         return json_parse_error(s, p, "Unexpected number");
 
-    while (is_digit(*p))
+    /* surimono: see next_token. */
+    while (is_digit(*p)) {
+        if (js_poll_interrupts(s->ctx))
+            return -1;
         p++;
+    }
 
     if (*p == '.') {
@@ -23856,6 +23916,9 @@ static int json_parse_number(JSParseState *s, const uint8_t **pp)
         if (!is_digit(*p))
             return json_parse_error(s, p, "Unterminated fractional number");
-        while (is_digit(*p))
+        while (is_digit(*p)) {
+            if (js_poll_interrupts(s->ctx))
+                return -1;
             p++;
+        }
     }
     if (*p == 'e' || *p == 'E') {
@@ -23865,6 +23928,9 @@ static int json_parse_number(JSParseState *s, const uint8_t **pp)
         if (!is_digit(*p))
             return json_parse_error(s, p, "Exponent part is missing a number");
-        while (is_digit(*p))
+        while (is_digit(*p)) {
+            if (js_poll_interrupts(s->ctx))
+                return -1;
             p++;
+        }
     }
     s->token.val = TOK_NUMBER;
@@ -23887,4 +23953,9 @@ static JSAtom json_parse_ident(JSParseState *s, const uint8_t **pp, int c)
     ident_pos = 0;
     for(;;) {
+        /* surimono: see next_token. */
+        if (js_poll_interrupts(s->ctx)) {
+            atom = JS_ATOM_NULL;
+            goto done;
+        }
         buf[ident_pos++] = c;
         c = *p;
@@ -23926,4 +23997,7 @@ static __exception int json_next_token(JSParseState *s)
     s->last_col_num = s->token.col_num;
  redo:
+    /* surimono: see next_token. */
+    if (js_poll_interrupts(s->ctx))
+        goto fail;
     s->token.line_num = s->line_num;
     s->token.col_num = s->col_num;
@@ -24050,5 +24124,10 @@ static __exception int json_next_token(JSParseState *s)
    only set if TOK_IMPORT is returned */
 /* XXX: handle all unicode cases */
-static int simple_next_token(const uint8_t **pp, bool no_line_terminator)
+/* surimono: polls like next_token, whose scan of the same bytes follows.
+   After an interrupt it returns TOK_EOF and the caller's next next_token
+   fails, or a parse error of the caller takes the interrupt's place; either
+   way the next poll stops the script, since an interrupt is sticky. */
+static int simple_next_token(JSContext *ctx, const uint8_t **pp,
+                             bool no_line_terminator)
 {
     const uint8_t *p;
@@ -24058,4 +24137,6 @@ static int simple_next_token(const uint8_t **pp, bool no_line_terminator)
     p = *pp;
     for (;;) {
+        if (js_poll_interrupts(ctx))
+            return TOK_EOF;
         switch(c = *p++) {
         case '\r':
@@ -24073,10 +24154,15 @@ static int simple_next_token(const uint8_t **pp, bool no_line_terminator)
                 if (no_line_terminator)
                     return '\n';
-                while (*p && *p != '\r' && *p != '\n')
+                while (*p && *p != '\r' && *p != '\n') {
+                    if (js_poll_interrupts(ctx))
+                        return TOK_EOF;
                     p++;
+                }
                 continue;
             }
             if (*p == '*') {
                 while (*++p) {
+                    if (js_poll_interrupts(ctx))
+                        return TOK_EOF;
                     if ((*p == '\r' || *p == '\n') && no_line_terminator)
                         return '\n';
@@ -24135,8 +24221,10 @@ static int peek_token(JSParseState *s, bool no_line_terminator)
 {
     const uint8_t *p = s->buf_ptr;
-    return simple_next_token(&p, no_line_terminator);
+    return simple_next_token(s->ctx, &p, no_line_terminator);
 }
 
-static void skip_shebang(const uint8_t **pp, const uint8_t *buf_end)
+/* surimono: stops early on an interrupt; the first next_token then fails. */
+static void skip_shebang(JSContext *ctx, const uint8_t **pp,
+                         const uint8_t *buf_end)
 {
     const uint8_t *p = *pp;
@@ -24146,4 +24234,6 @@ static void skip_shebang(const uint8_t **pp, const uint8_t *buf_end)
         p += 2;
         while (p < buf_end) {
+            if (js_poll_interrupts(ctx))
+                break;
             if (*p == '\n' || *p == '\r') {
                 break;
@@ -24995,4 +25085,8 @@ static __exception int js_parse_template(JSParseState *s, int call, int *argc)
              */
             if (js_parse_string(s, '`', false, p, &cooked, &p)) {
+                /* surimono: an invalid escape throws nothing here; an
+                   interrupt or a failed allocation does. */
+                if (JS_HasException(ctx))
+                    return -1;
                 cooked.u.str.str = JS_UNDEFINED;
             }
@@ -26763,4 +26857,8 @@ static int js_parse_check_duplicate_parameter(JSParseState *s, JSAtom name)
     JSFunctionDef *fd = s->cur_func;
     int i;
+    /* surimono: walks every parameter so far, per parameter. See
+       js_parse_function_check_names. */
+    if (js_poll_interrupts(s->ctx))
+        return -1;
     for (i = 0; i < fd->arg_count; i++) {
         if (fd->args[i].var_name == name)
@@ -34754,5 +34852,10 @@ static bool is_var_in_arg_scope(const JSVarDef *vd)
 }
 
-static void add_eval_variables(JSContext *ctx, JSFunctionDef *s)
+/* surimono: for a direct eval, every variable of every enclosing function goes
+   through get_closure_var, which walks the closure variables so far in each
+   function on the way, so this is quadratic in the variables and linear in the
+   nesting, all between two tokens: 32,768 `var`s and eight nested arrows took
+   3.4 s. Polls per variable; returns -1 after an interrupt. */
+static __exception int add_eval_variables(JSContext *ctx, JSFunctionDef *s)
 {
     JSFunctionDef *fd;
@@ -34843,4 +34946,6 @@ static void add_eval_variables(JSContext *ctx, JSFunctionDef *s)
         scope_idx = fd->scopes[scope_level].first;
         while (scope_idx >= 0) {
+            if (js_poll_interrupts(ctx)) /* surimono */
+                return -1;
             vd = &fd->vars[scope_idx];
             capture_var(fd, vd);
@@ -34854,4 +34959,6 @@ static void add_eval_variables(JSContext *ctx, JSFunctionDef *s)
             /* XXX: propagate is_const and var_kind too ? */
             for(i = 0; i < fd->arg_count; i++) {
+                if (js_poll_interrupts(ctx)) /* surimono */
+                    return -1;
                 vd = &fd->args[i];
                 if (vd->var_name != JS_ATOM_NULL) {
@@ -34863,4 +34970,6 @@ static void add_eval_variables(JSContext *ctx, JSFunctionDef *s)
             }
             for(i = 0; i < fd->var_count; i++) {
+                if (js_poll_interrupts(ctx)) /* surimono */
+                    return -1;
                 vd = &fd->vars[i];
                 /* do not close top level last result */
@@ -34876,4 +34985,6 @@ static void add_eval_variables(JSContext *ctx, JSFunctionDef *s)
         } else {
             for(i = 0; i < fd->var_count; i++) {
+                if (js_poll_interrupts(ctx)) /* surimono */
+                    return -1;
                 vd = &fd->vars[i];
                 /* do not close top level last result */
@@ -34892,4 +35003,6 @@ static void add_eval_variables(JSContext *ctx, JSFunctionDef *s)
             for (idx = 0; idx < fd->closure_var_count; idx++) {
                 JSClosureVar *cv = &fd->closure_var[idx];
+                if (js_poll_interrupts(ctx)) /* surimono */
+                    return -1;
                 get_closure_var(ctx, s, fd,
                                 JS_CLOSURE_REF,
@@ -34899,4 +35012,5 @@ static void add_eval_variables(JSContext *ctx, JSFunctionDef *s)
         }
     }
+    return 0;
 }
 
@@ -35355,4 +35469,11 @@ static __exception int resolve_variables(JSContext *ctx, JSFunctionDef *s)
         int flags;
 
+        /* surimono: see the poll in the loop below; pos is 0, so the path
+           at fail keeps every atom of the bytecode. */
+        if (js_poll_interrupts(ctx)) {
+            pos = 0;
+            goto fail;
+        }
+
         /* check if global variable (XXX: simplify) */
         for(idx = 0; idx < s->closure_var_count; idx++) {
@@ -35391,4 +35512,9 @@ static __exception int resolve_variables(JSContext *ctx, JSFunctionDef *s)
     col_num = 0;
     for (pos = 0; pos < bc_len; pos = pos_next) {
+        /* surimono: this pass costs as much as the parser and its name
+           lookups walk every variable in scope: a function of 60,000 `let`s,
+           1 MiB of source, took 24 s here and in the next one. */
+        if (js_poll_interrupts(ctx))
+            goto fail;
         op = bc_buf[pos];
         len = opcode_info[op].size;
@@ -36075,4 +36201,7 @@ static __exception int resolve_labels(JSContext *ctx, JSFunctionDef *s)
             goto fail;
         }
+        /* surimono: see resolve_variables. */
+        if (js_poll_interrupts(ctx))
+            goto fail;
         op = bc_buf[pos];
         len = opcode_info[op].size;
@@ -36805,4 +36934,10 @@ static __exception int resolve_labels(JSContext *ctx, JSFunctionDef *s)
         int j, pos, diff, delta;
 
+        /* surimono: each shrunk jump moves the rest of the bytecode and walks
+           every label, later jump and source location, so this loop is
+           quadratic in a small input: 1.8 MiB of `if(a){...}` took 24 s. No
+           relocation is pending here, so the failure path above is safe. */
+        if (js_poll_interrupts(ctx))
+            goto fail;
         delta = 3;
         switch (op = jp->op) {
@@ -36995,4 +37130,7 @@ static __exception int compute_stack_size(JSContext *ctx,
 
     while (s->pc_stack_len > 0) {
+        /* surimono: see resolve_variables. */
+        if (js_poll_interrupts(ctx))
+            goto fail;
         pos = s->pc_stack[--s->pc_stack_len];
         stack_len = s->stack_level_tab[pos];
@@ -37245,6 +37383,6 @@ static JSValue js_create_function(JSContext *ctx, JSFunctionDef *fd)
        other variable lookup is done. */
 #ifndef QJS_DISABLE_PARSER
-    if (fd->has_eval_call)
-        add_eval_variables(ctx, fd);
+    if (fd->has_eval_call && add_eval_variables(ctx, fd))
+        goto fail;
 #endif // QJS_DISABLE_PARSER
 
@@ -37585,4 +37723,8 @@ static int js_parse_function_check_names(JSParseState *s, JSFunctionDef *fd,
     ||  fd->func_type == JS_PARSE_FUNC_METHOD) {
         for (idx = 0; idx < fd->arg_count; idx++) {
+            /* surimono: quadratic in the parameters, and it runs between two
+               tokens: 65,000 strict parameters took 0.9 s with no poll. */
+            if (js_poll_interrupts(s->ctx))
+                return -1;
             name = fd->args[idx].var_name;
             if (name != JS_ATOM_NULL) {
@@ -38424,5 +38566,5 @@ static JSValue __JS_EvalInternal(JSContext *ctx, JSValueConst this_obj,
 
     js_parse_init(ctx, s, input, input_len, filename, line);
-    skip_shebang(&s->buf_ptr, s->buf_end);
+    skip_shebang(ctx, &s->buf_ptr, s->buf_end);
 
     eval_type = flags & JS_EVAL_TYPE_MASK;
@@ -49557,4 +49699,9 @@ static JSValue js_compile_regexp(JSContext *ctx, JSValueConst pattern,
     JS_FreeCString(ctx, str);
     if (!re_bytecode_buf) {
+        /* surimono: lre_compile polls; see re_parse_poll. */
+        if (!strcmp(error_msg, LRE_INTERRUPTED)) {
+            JS_ThrowInterrupted(ctx);
+            return JS_EXCEPTION;
+        }
         JS_ThrowSyntaxError(ctx, "%s", error_msg);
         return JS_EXCEPTION;
```
