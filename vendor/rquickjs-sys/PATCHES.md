# surimono's changes to rquickjs-sys 0.14.0

This directory is rquickjs-sys 0.14.0 from crates.io (rquickjs git `d7ef5ee`,
path `sys/`), which bundles quickjs-ng 0.16.2. It is wired in through
`[patch.crates-io]` in the workspace `Cargo.toml`. Every change to upstream
code is listed here and marked `surimono:` in the source. Drafts for
upstream are in `docs/upstream/quickjs-ng-sticky-interrupt-and-native-loop-polls.md`
and `docs/upstream/quickjs-ng-interrupt-granularity.md`; the regexp
per-match polls (patches 19 and 20) are in
`docs/upstream/quickjs-ng-regexp-replace-polls.md`.

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
