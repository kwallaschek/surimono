Title: The parser, the compiler passes, JSON.parse and the RegExp compiler never poll the interrupt handler

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, running hostile scripts under a time limit), then reproduced on current master.

**What happens.** A script under a time limit can spend it all in one native call that takes a string: `eval`, `new Function`, `new RegExp` or `JSON.parse`. `JS_SetInterruptHandler` is only asked from the bytecode interpreter and a few native loops, and none of these run bytecode. Bytecode compiled from the string doesn't help either when it never runs: the time goes before the first instruction. Size is not the only problem, because three of these are quadratic in small inputs:

- `resolve_variables` looks up each name through every variable in scope: a function of 60,000 `let` declarations, about 1 MiB of source, takes about 25 s.
- `re_parse_disjunction` moves the bytecode written so far for each alternative, so `'x|'.repeat(1 << 22)` runs for minutes.
- A character class merges each item into a sorted table.

**Reproduce.**

```js
let s = 'function f(){';
for (let i = 0; i < 60000; i++) s += `let a${i}=a${i >> 1};`;
s += '}';
(0, eval)(s);

new RegExp('x|'.repeat(1 << 22));
```

The `repro.sh` in `quickjs-ng-repro.tar.gz`, attached, runs these with `harness/time-limit.c`, whose handler compares a clock with a deadline, in a release build, on master at e22ce4de (2 Nov 2026). With a 250 ms limit, on a busy 2-core machine:

| Case | Input | Past the deadline |
|---|---|---|
| `parse-source` | 67 MiB of statements in a function nothing calls | 10,426 ms |
| `parse-locals` | 60,000 `let`s, 1 MiB | 26,579 ms |
| `regexp-compile` | `'x\|'.repeat(1 << 22)`, 8 MiB | still running after 60 s |
| `json-parse` | 67 MiB of `{"a":1},` | 6,228 ms |

**Suggested fix.** Call `js_poll_interrupts` where these loops turn. On the paths below every failure label exists already, so each poll is two lines:

- `next_token` and `json_next_token`: at `redo:`, which covers each token and each skipped white space character, not by source position, because `js_parse_skip_parens_token` scans tokens again.
- The loops that scan one token: `js_parse_string`, `js_parse_template_part`, `js_parse_regexp`, `parse_ident`, the comment loops of `next_token`, `json_parse_string`, `json_parse_number`, `json_parse_ident`, and the digit loops of `js_atof`.
- `resolve_variables`, `resolve_labels` and `compute_stack_size`: once per instruction.
- libregexp: per term, class item and alternative, through `lre_check_timeout`. `lre_compile` has no way to return "interrupted" today, so it needs one, or the interrupt comes back as a catchable `SyntaxError`. We use a reserved error message that `js_compile_regexp` turns into `JS_ThrowInterrupted`. These polls have no counter, because one step can cost milliseconds.

Two details need care:

- `js_parse_template` with `do_throw` false ignores `js_parse_string`'s failure, so it has to check for a pending exception, or the interrupt is dropped.
- `js_parse_skip_parens_token` discards `next_token`'s failure and seeks back. That only stops the parser if the next poll fires again, so it depends on a handler that keeps answering true after its first true. `simple_next_token` (`peek_token`) is the same if it polls, and it can be left out, since the `next_token` after it scans the same bytes.

Compile time is unchanged within noise: compiling a 1 MiB minified bundle 15 times took 139 to 144 ms at best with these polls and 142 to 153 ms without. surimono carries this fix in its patched copy of QuickJS, and we can send it as a pull request.
