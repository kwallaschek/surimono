# Status

Updated after every task. Times are JST.

> **Scheduled sessions:** if `scripts/fleet/owner-check.sh` prints STOP, or the heartbeat
> is under 3 hours old and names another session: change nothing more, end the session.

## Now (Sun 1 Nov 2026, 16:00)

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

### Blocked / waiting on Kay
- Checkpoint 2 answers (above). Don't merge #38 or any phase-2 public API without them.

### Next (in order; a resumed session starts here)
1. Look for Kay's checkpoint-2 reply (this session's conversation; if a resumed session can't see it,
   check PR #38 comments). Record his answers as DECISIONS lines (sign-off of checkpoint 2) and act:
   rename nothing; if he says publish, PKG merges main into release/v0.1.0, re-runs the dry-run, and
   the orchestrator merges #38 on green + fresh review; Kay tags/publishes himself. Apply (2) to
   SPEC.md/COMPAT.md. If no reply yet, don't ask again; keep working on 2.
2. Meanwhile (no checkpoint can change these): P1-RT-29 (test-only stop-vs-cleanup hook), P1-RT-25 (gem
   RSS after burst), P1-RT-21 (cancel inside an island), P1-RT-18, P1-DOM-04 (#80 nits), P1-PKG-17
   (ASan symbolizer), P1-RT-20 (quickjs-ng drafts), P1-CF COMPAT row for blitz comment data / serializer.
3. After checkpoint-2 sign-off: phase 2 kickoff — MSRV 1.89 task first (full rebuild), then break
   phase-2 epics into tasks in TASKS.md.
4. Run reviewers/ci.sh staggered: concurrent ci.sh runs in the shared target dir give spurious
   failures (permission denied / E0460). Reviewers rely on GitHub ci/linux + targeted local tests.
5. Each push to main cancels the running full.yml (concurrency); batch docs commits.

### Disk
When free space drops under ~5 GB, run `scripts/fleet/prune-target.sh`.

### Branch rule
Never rebase or force-push a pushed branch; fix with new commits; downstream branches merge their parent in. PRs merge with merge commits. The orchestrator may merge main into an approved branch when the result is exactly the mechanical merge.
