# Status

Updated after every task. Times are JST.

> **Scheduled sessions:** if `scripts/fleet/owner-check.sh` prints STOP, or the heartbeat
> is under 3 hours old and names another session: change nothing more, end the session.

## Now (Sun 1 Nov 2026, 09:10)

**Phase 1: islands. Checkpoint 2 is today about 18:00 JST.** The live session with Kay ended
(heartbeat 231 min old at 04:16). Scheduled resume cse_011c8hfq5ufjN5m2pZpZysXx ran the fleet
04:16–09:10 and merged, each on green ci/linux plus a fresh reviewer's APPROVE:
#55, #25 (facade on the real backend), #26 (CLI), #34 (licences), #56 (precompiled bytecode),
#57 (ASan stack probe), #47 (time-accurate deadlines, BigInt leak fix, fork-safe registry),
#58, #59 + #65 (checkpoint-2 report, re-run on 99c363cd), #60/#61 (licence files in crates),
#62 (fork generation; runner fork-safe), #18/#19/#23 (README, guides, examples), #63 (examples in
CI, facade slots by generation), #64, #66, #67 (precompiled_builtins API + gem gate), #68,
#69 (builtin_renderers const, libc workspace pin), #70 (WebIDL vendoring + stub generator, phase-2
scaffolding, internal only), #71 (dead worker → typed error, never a hang).

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
| #38 | v0.1 release PR | merge main in, re-run dry-run; merge only with Kay's OK |

### Blocked / waiting on Kay
- Nothing until checkpoint 2.

### Next (in order; a resumed session starts here)
1. Check main's full.yml on the latest main commit (macos-14 and asan jobs were "cancelled" on recent
   runs — find out whether that is timeout or concurrency; asan is known red until P1-RT-27).
2. PKG: merge main into release/v0.1.0 (#38), re-run scripts/ci/release-dry-run.sh, post evidence.
3. ~16:00–18:00 JST: checkpoint 2 message to Kay (SendUserMessage + PushNotification): the evidence
   above with links to fixtures/checkpoint-2/index.md, the bench summary, COMPAT.md, #38; the Kay
   decisions; exact steps and time needed. Then wait for his OK; ask whether to publish v0.1.
4. ~20:00 JST: Sunday digest (five lines) if checkpoint 2 hasn't covered it.
5. Meanwhile (no checkpoint can change these): P1-RT-27 (ASan time scale), P1-RT-25 (gem RSS after
   burst), P1-RT-21 (cancel inside an island), P1-RT-23/29 (regex polls), P1-RT-34, P1-DOM-03,
   P1-DOM-02 (id handles spike), P1-PKG-01d, P1-RT-20 (quickjs-ng public issue drafts).

### Disk
When free space drops under ~5 GB, run `scripts/fleet/prune-target.sh`.

### Branch rule
Never rebase or force-push a pushed branch; fix with new commits; downstream branches merge their parent in. PRs merge with merge commits. The orchestrator may merge main into an approved branch when the result is exactly the mechanical merge.
