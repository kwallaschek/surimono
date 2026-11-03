Title: RegExp replace with '' never polls between matches

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, running hostile scripts under a time limit), then reproduced on current master.

**What happens.** `replace`/`replaceAll` with an empty replacement on a standard global or sticky RegExp takes the fast path `JS_RegExpDelete`, which loops over the matches calling `lre_exec` directly. Nothing in the loop polls. `lre_exec` polls every 10,000 matcher steps but restarts that count on every call, so a pattern that matches in a few steps never reaches it. The cost is bounded by the string's length, not by the time limit. The second loop of `js_regexp_Symbol_replace` has the same gap with a string replacement: one more unpolled pass over the collected matches.

**Reproduce.**

```js
'a'.repeat(1 << 27).replace(/(?:x|y|z|w|v|u|t|a)/g, '')
```

The `repro.sh` in `quickjs-ng-repro.tar.gz`, attached, runs these with `harness/time-limit.c`, whose handler compares a clock with a deadline, in a release build, on master at e22ce4de (2 Nov 2026). `regexp-delete` finished 9,423 ms after a 250 ms deadline.

**Suggested fix.** One poll per match in both loops, through the existing cleanup paths:

```c
/* JS_RegExpDelete, before lre_exec */
if (js_poll_interrupts(ctx))
    goto fail;

/* js_regexp_Symbol_replace, top of the for (j = 0; j < results->len; j++) loop */
if (js_poll_interrupts(ctx))
    goto exception;
```

The other per-match loops call `exec` through `JS_Call`, which polls on entry. surimono carries this fix in its patched copy of QuickJS, and we can send it as a pull request.
