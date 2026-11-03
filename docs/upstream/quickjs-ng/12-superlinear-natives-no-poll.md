Title: Native calls with superlinear cost never poll: string search, BigInt arithmetic, typed-array sort

Found by an automated tool (an AI agent working on surimono, an embedder of quickjs-ng, refusing allocations in a test harness), then reproduced on current master.

**What happens.** These run in one native call without a poll, so a handler can't stop them:

- `indexOf`, `lastIndexOf`, `includes` and the search in `split`, `replace` and `replaceAll` with a string pattern are naive, up to n × m comparisons. `replaceAll` with a one-character pattern also loops once per match without calling script code.
- BigInt multiply, divide, `toString` and decimal parsing are quadratic. Near the 1 Mbit cap one operation takes between 0.3 s and 11 s, and straight-line code never polls between operations.
- `%TypedArray%.prototype.sort` without a comparator is all C.

**Reproduce.** `repro.sh` runs these with `harness/time-limit.c`, whose handler compares a clock with a deadline, in a release build, on master at e22ce4de (2 Nov 2026). With a 250 ms deadline, on a busy 2-core machine:

```
search-naive       s.indexOf(p), n = 2^20, m = 2^12     finished 6,560 ms after the deadline
search-replaceall  'a'.repeat(1 << 27).replaceAll(...)  finished 3,520 ms after the deadline
bigint-tostring    four toString calls in a row         still running 15 s in
sort-typed         30M Int32Array                       finished 5,250 ms after the deadline
```

**Suggested fix.** Polls where the work is: the searches count comparisons and poll every 65,536; `js_mp_mul_basecase` per row, `js_mp_divnorm` per quotient limb, decimal `toString` and parsing per limb, each failing up through its caller (`js_bigint_pow` must then free its running product); the sort through a wrapper per element type that loads the flag proposed in the granularity issue. Patches 15 to 17 in `vendor/rquickjs-sys/PATCHES.md` in surimono, with measurements: a 4M-element sort pays 8 to 11% that way, against 15 to 26% with a counter in one generic wrapper.
