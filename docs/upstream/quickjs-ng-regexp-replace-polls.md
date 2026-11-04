# quickjs-ng: RegExp replace with '' never polls the interrupt handler between matches

Draft for Kay to file at https://github.com/quickjs-ng/quickjs (public: quickjs-ng's policy treats
AI-found bugs as public). Against quickjs-ng 0.16.2 as bundled in rquickjs-sys 0.14.0. surimono
carries the fix in `vendor/rquickjs-sys` (patches 19 and 20 in its `PATCHES.md`, #81) until upstream
has an equivalent.

## Problem

`String.prototype.replace` / `replaceAll` with an empty replacement on a standard global (or sticky)
RegExp takes the fast path `JS_RegExpDelete`, which loops over the matches calling `lre_exec`
directly. Nothing in that loop calls `js_poll_interrupts`. `lre_exec` polls its own way
(`lre_poll_timeout`, every 10,000 matcher steps), but it resets that counter on every call. So a
pattern that matches in a few steps never reaches it, and the whole loop runs inside one native call
without asking the interrupt handler:

    'a'.repeat(1 << 25).replace(/(?:)/g, '')                       // ~1 s, no poll
    'a'.repeat(1 << 26).replace(/(?:x|y|z|w|v|u|t|a)/g, '')        // ~4.5 s, no poll

With a 250 ms deadline in the interrupt handler, these run to their end (debug build). The cost is
bounded only by the string length, so it is bounded by the memory limit, not by the time limit.

A smaller instance of the same gap: the second loop of `js_regexp_Symbol_replace` (the one that
builds the result from the collected matches) calls no script code when the replacement is a string.
A match list collected just before the deadline is then processed in one more unpolled pass of about
the same length.

The other per-match loops (`Symbol.match` with g, `Symbol.split`, the first loop of
`Symbol.replace`, the `matchAll` iterator) call `exec` through `JS_Call`, and `JS_CallInternal` polls
on entry, so they are fine.

## Proposed fix

Call `js_poll_interrupts(ctx)` once per match in both loops and fail on interrupt through the
existing cleanup paths:

    /* JS_RegExpDelete, before lre_exec */
    if (js_poll_interrupts(ctx))
        goto fail;

    /* js_regexp_Symbol_replace, top of the for (j = 0; j < results->len; j++) loop */
    if (js_poll_interrupts(ctx))
        goto exception;

`js_poll_interrupts` is a counter decrement on the fast path, so the cost per match is negligible.
Results are unchanged. With the stock 10,000-poll counter the handler is asked every 10,000 matches,
which is enough to stop the examples above within a few milliseconds of the deadline.

## Test

With an interrupt handler that returns 1 after a deadline (say 250 ms), each example above should end
with an InternalError "interrupted" shortly after the deadline, instead of running to completion.
