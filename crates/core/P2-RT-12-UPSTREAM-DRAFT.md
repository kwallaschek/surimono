# quickjs-ng: a failed module graph leaves dangling module pointers; module walks recurse without a stack check

Draft for a public quickjs-ng issue (their policy treats AI-found bugs as public). Found by
surimono's P2-RT-12 spike on quickjs-ng 0.16.2 (as bundled by rquickjs-sys 0.14.0); the code is
unchanged on quickjs-ng master `947ffe4a` (13 Nov 2026). Kay files it.

## 1. Use after free after a failed module graph (cycle plus a failing import)

`__JS_EvalInternal` registers a module in `ctx->loaded_modules`, compiles it and resolves its
requests, calling the host's loader, which compiles the requested modules in turn (nested
`__JS_EvalInternal`). If resolution fails, the `fail1` path frees that one module
(`js_free_module_def`; the comment there reads "XXX: should free all the unresolved
dependencies"). A module compiled earlier in the same walk that imports the freed module back (a
cycle) has already stored a pointer to it in `req_module_entries[i].module`, is marked `resolved`,
and stays in `loaded_modules`. The next graph that reaches that module finds it already resolved
and links or creates functions through the freed pointer.

Minimal reproduction (any loader that compiles with `JS_Eval(..., JS_EVAL_TYPE_MODULE |
JS_EVAL_FLAG_COMPILE_ONLY)` and returns NULL when a file is missing, e.g. quickjs-libc's):

```js
// c.js
import './d.js'; export const c = 'c';
// d.js
import './e.js'; import './missing.js'; export const d = 'd';
// e.js
import { c } from './c.js'; export const e = 'e';
```

Evaluate `import './c.js';` (fails: missing.js), then `import { e } from './e.js';`. The second
evaluation walks into the freed `c` (`js_create_module_function` and `js_inner_module_linking`
follow `req_module_entries[i].module`); in our build it ends in SIGSEGV. We didn't run it under
ASan. The same happens through `import()`. WPT
`html/semantics/scripting-1/the-script-element/module/nested-imports.html` has this shape.

Fix used by surimono (patch in `vendor/rquickjs-sys`, spike only): give each outermost module
compile a generation (`JSContext.module_gen`, bumped when a module compile starts at nesting depth
0; `JSModuleDef.gen` set in `js_new_module_def`). On failure, a nested compile leaves its module in
place; the outermost one frees every module of its generation that is still unlinked and has no
function. Older modules can't point at them, since they were resolved before the generation began.

## 2. `js_resolve_module` and `js_create_module_function` recurse without `js_check_stack_overflow`

Both walk the graph recursively, once per level, with no stack check (`js_inner_module_linking`
and `js_inner_module_evaluation` have one). Through a loader the parser's own check usually stops a
deep chain first, but modules declared ahead of time (`JS_ReadObject` of module bytecode, or a host
that compiles a chain leaf first) are resolved one level at a time, and the first evaluation then
recurses through the whole chain in `js_create_module_function`. A chain of 20,000 such modules
overflowed an 832 KiB thread stack (process abort); with the check added it throws
`RangeError: Maximum call stack size exceeded` as expected. Fix: the same three lines as in
`js_inner_module_linking` at the top of both functions.
