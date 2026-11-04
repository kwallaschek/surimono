# quickjs-ng issue drafts

Public issue texts for Kay to file at https://github.com/quickjs-ng/quickjs/issues (P1-RT-20). The project's `SECURITY.md` takes no private reports and issues no CVEs: "Please file bug reports in the issue tracker". The fleet never files them.

**To file:**

1. Run `./repro.sh` here (10 to 15 minutes; needs git, cmake and a C compiler with AddressSanitizer). It clones current master into a temporary directory, builds it and prints one line per reproducer, with what failed if a build step fails. Every line said REPRODUCED on e22ce4de (2 Nov 2026), apart from the `-` baseline, a plain loop that must stop on time. Reword or skip any finding that says "not reproduced" by then. `cases/time/string-raw.js` peaks at about 1.1 GB of memory, and `time-limit.c` sets no memory limit.
2. Make the archive once: `tar czf quickjs-ng-repro.tar.gz repro.sh harness cases`.
3. For each finding, use the file's first line as the title and the rest as the body, and attach `quickjs-ng-repro.tar.gz` to the issue. The bodies refer to that archive by name, and nothing in them points into this repository. GitHub accepts `.gz` attachments.

The patch numbers below are for this repository only (`vendor/rquickjs-sys/PATCHES.md`).

| # | Finding | Kind | surimono patch |
|---|---|---|---|
| 00 | [Compiler continues after a failed buffer](00-compiler-failed-buffer.md) | out-of-bounds read, assert | 4 |
| 01 | [`bind` finalizer](01-bound-function-finalizer.md) | SIGSEGV from JavaScript alone | 6 |
| 02 | [`resolve_labels` after a failed buffer](02-resolve-labels-failed-buffer.md) | out-of-bounds write | 8 |
| 03 | [Parser rewrites at saved offsets](03-parser-saved-offsets.md) | out-of-bounds write | 9 |
| 04 | [`js_free_function_def` frees garbage atoms](04-free-function-def-atoms.md) | SIGSEGV | 7 |
| 05 | [`Promise.allSettled`/`any` leaks](05-promise-combinator-leaks.md) | assert in `JS_FreeRuntime` | 10, 11 |
| 06 | [libregexp group names](06-regexp-group-names.md) | out-of-bounds read | 12 |
| 07 | [Unchecked `add_arg`](07-add-arg-unchecked.md) | out-of-bounds stack read at run time | 25 |
| 08 | [`add_property` for an array's length](08-array-add-property.md) | SIGSEGV from JavaScript alone | 24 |
| 09 | [Interrupts can be swallowed](09-interrupt-is-swallowed.md) | hang | 1 |
| 10 | [Array-like loops never poll](10-array-like-loops-never-poll.md) | hang | 2 |
| 11 | [Interrupt granularity](11-interrupt-granularity.md) | minutes past a deadline | 14 |
| 12 | [Superlinear natives never poll](12-superlinear-natives-no-poll.md) | seconds to hours past a deadline | 3, 15-17 |
| 13 | [RegExp replace with `''`](13-regexp-replace-no-poll.md) | seconds past a deadline | 19, 20 |

Dropped: the `JSON.stringify` element leak (part of patch 10) is fixed on master.

07 and 08 are new, found while writing these reproducers. The older drafts one level up (`quickjs-ng-sticky-interrupt-and-native-loop-polls.md`, `quickjs-ng-interrupt-granularity.md`, `quickjs-ng-regexp-replace-polls.md`) hold the longer patch discussion; these files replace them for filing.

`harness/` and `cases/` are ours, under the repository's licence.
