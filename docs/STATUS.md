# Status

Updated after every task. Times are JST.

> **Scheduled sessions:** if `scripts/fleet/owner-check.sh` prints STOP, or the heartbeat
> is under 3 hours old and names another session: change nothing more, end the session.

## Now (Thu 5 Nov 2026, 00:50)

Scheduled session cse_011c8hfoqJ45246YxjoyFzH7, running since 20:18. v0.1.0 isn't tagged yet (Kay's step, docs/RELEASE.md). MSRV 1.89 is on main. Merged tonight: #88 #90 #92 #93 #99 #101 #103 #106 #108 #109 #115 #116 #117 #118 #119 #120 #121 #122. Open: DOM stack #94 #96 #97 #104 #105 with all first-review blockers fixed and pushed, awaiting merge-up and re-review; #123 (WPT, approved, base e2-04b); #124 (needs review). First WPT run: timers 73.5%, dom/nodes 18.8%, dom/events 10.8%, domparsing 13.5%, all below target; biggest cause `new Document()`.

## Earlier (Wed 4 Nov 2026, 14:50)

**Main is green**: full.yml passed all 14 jobs at 0e67e98c (ASan leak fixed in #111, macOS flakes in #112,
symbolized ASan in #107). Merge freeze lifted. Also merged: v0.1 README/CHANGELOG (#110, #113) and the
Ruby 3.2 signal fix (#114). **v0.1.0 release PR #38 merged** (915d5cbf; pre-release review approved, 15/15 checks). Kay tags
v0.1.0 and pushes the five gems following docs/RELEASE.md. After #38: phase 2 starts
with MSRV 1.89, then the open phase-2 PRs (#88 #90 #92–#105) and #106.

## Earlier (Wed 4 Nov 2026, 13:05)

**Checkpoint 2 signed off by Kay at 12:57** (all questions approved; see DECISIONS). The live session
(cse_011c8hfp9aeYhQp5RpWyywaN) took the fleet back at 12:58. **Merge freeze until main's full.yml is
green**: linux-extra-asan reports a 1.1 MB leak at exit of the core lib tests (P1-RT-42) and test-macos-14
has timing flakes (P1-RT-43); P1-PKG-17 symbolizes ASan traces in CI. Then: v0.1 release (#38: main merged
in, dry-run, fresh review, merge; Kay tags/publishes), then phase 2 starting with MSRV 1.89 (MSRV 1.89 waits until #38 has merged, so the release docs and dry-run keep saying 1.87).

## Earlier (Wed 4 Nov 2026, 04:40)

Session cse_011c8hfnWG3Q5HtHFaa21vge resumed the fleet after 55 h with no heartbeat. No reply from Kay
to checkpoint 2 visible here (no comments or reviews from him on #38). #82 (P1-DOM-04) and #83 (P1-RT-29)
were merged on Sun evening. main adfe71a7: 13 of 14 full.yml jobs green; asan red (P1-DOM-05, a test's
2 s limit); reference red (P1-CF-30, no browser). Phase-2 internal scaffolding starts now under the
DECISIONS 2026-11-04 rule; nothing public until sign-off. macos-14 still ran on 2 Nov.

## Earlier (Sun 1 Nov 2026, 16:00)

**Phase 1: islands. Checkpoint 2 message sent to Kay at 15:58 JST** (SendUserMessage + push), in session
cse_011c8hfoBxyki5Uo3FJx7LZe. Waiting for his 5 answers: (1) engine distribution (recommend gem + CLI
v0.1, Rust crates held), (2) memory target: a) ≥5× PSS or b) keep 10× + COMPAT gap, (3) MSRV 1.89 at
phase 2, (4) outline labels default OK, (5) publish v0.1? Also asked him to copy docs/ci/*.yml into
.github/workflows before macos-14 retires on 2 Nov. The message covers today's 20:00 digest.

This session merged (each on green ci/linux + a fresh reviewer's APPROVE): #72 (RT-35 macOS preload
flake), #73 + #77 (RT-27 ASan time scale), #74 (DOM-03), #75 + #79 (BUILTIN_RENDERERS), #76 (PKG-01d),
#78 (GFX-06 stylo thread-local leak in a test), #81 (RT-23 regexp per-match polls, engine patch level 4),
#80 (DOM-02 id-handle spike, internal; first review blocked, fixed). main abd31b70: full.yml all 14 jobs
green incl. asan and macos-14. #38 head f0d7091f has main up to abd31b70 (not #80/#81).

### Checkpoint 2 evidence (all on main)
- Fixtures: fixtures/checkpoint-2/index.md (+ NOTES.md). Gate 12/12 islands pass; 34/35 comparisons
  pass; the one fail is vegalite-line-ja printed by fulgur, shape 10.2% vs 10% at the 10 px "6月"
  label (Chrome-printed 4.8%, page 7.3%). Determinism 11/11 byte-identical across processes, TZ and
  locale. Mention: local Chromium differs from CI by 65 px / 38 px on two ECharts fixtures; refs and
  fulgur floor are CI's (NOTES.md). Results predate #63 (no render-path change).
- Hostile: 59 cases, 60 results, all as the manifest expects; no crash, no hang (COMPAT.md).
- Bench (ci-results runs/15b530e0…/bench/grover/bench/summary.md; not committed to bench/):
  Grover ÷ surimono+fulgur Ruby: 2.4× cold / 2.4× warm (2× met); PSS 5.8× / 5.6× (10× not met;
  fulgur alone with Chrome's SVGs is only 8.7×). Rust: 3.0×/2.4×, PSS 8.0×/8.2×. Against
  chrome-headless-shell: 1.8–2.3× time, 3.8–5.2× PSS.
- Release PR #38 (release/v0.1.0, head 715a08f4): dry-run passes except the engine distribution
  (crates.io users get unpatched rquickjs-sys; link fails). Licence files fixed. Source gem and the
  x86_64-linux native gem build and pass. Needs main merged in again before the message.
- Kay decisions (docs/fleet/PENDING_DECISIONS.md "Open for checkpoint 2"): engine distribution
  (recommend v0.1 = gem + CLI from source, Rust crates held; publish the fork under our own names
  later), the 10× memory target (PSS vs summed RSS; fulgur's own floor), MSRV 1.89 at phase 2,
  outline text (labels not selectable), macos-14 retirement → YAML change (docs/ci/), reference.yml
  paths should include crates/gfx/**.

### Open PRs
| PR | Task | State |
|---|---|---|
| #38 | v0.1 release PR | waits for Kay; merge main in again (gets #81) before any merge |
| #84 | P1-RT-18 | ci/linux green, in review |

### Blocked / waiting on Kay
- Checkpoint 2 answers (above). Don't merge #38 or any phase-2 public API without them.

### Next (in order; a resumed session starts here)
1. Merge up the DOM stack (e2-02c into e2-02d into e2-04a into e2-04b; rerun `cargo run -p surimono-webidl --example regen` on generated conflicts; bring main in, for wcargo.sh). Fresh reviewers limited to the fix deltas, merge bottom up, each on green ci/linux.
2. Retarget #123 to main, regenerate COMPAT (the command is in #123), merge; merge #124 after review.
3. Then P2-RT-03, P2-CF-02, E2-13 (WPT wins), E2-06/07/08 (loader, layout policy, facade output), E2-10 surimono side, E2-12. Checkpoint 3 is Sun 22 Nov 11:00.
4. Sunday 8 Nov 20:00 JST: five-line digest to Kay.
5. Each push to main cancels the running full.yml; batch docs commits; read full.yml on main (first run on 1.89: native-gem images use their own stable).

### Disk
When free space drops under ~5 GB, run `scripts/fleet/prune-target.sh`.

### Branch rule
Never rebase or force-push a pushed branch; fix with new commits; downstream branches merge their parent in. PRs merge with merge commits. The orchestrator may merge main into an approved branch when the result is exactly the mechanical merge.
