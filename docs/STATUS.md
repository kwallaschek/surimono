# Status

Updated after every task. Times are JST.

> **Scheduled sessions: stop.** Since 16:35 on Sat 31 Oct a live orchestrator session
> (cse_011c8hfp9aeYhQp5RpWyywaN, with Kay) runs the fleet. If you are a scheduled
> resume and `scripts/fleet/owner-check.sh` prints STOP, or the heartbeat is under
> 3 hours old: change nothing more, stop your subagents, end the session.

## Now (Sat 31 Oct 2026, 17:05)

**Phase 1: islands.** On main: fonts, CI, pre-pass, facade (stand-in backend),
fixture pipeline, sandbox with determinism fixes, island runner, pool, soak, ECharts,
Vega, gfx text measurement, hostile suite, the 11 fixtures, and since 16:45 the gfx
measurer in islands (#30), the COMPAT generator (#28), hostile follow-ups (#42), the
font weight/style check (#43), gem staging of the vendored engine (#33), islands follow-ups (#39), the Vega bundle hash (#46) and CI hardening with the final YAML proposal for Kay (#44).

The fleet stopped three times on the account's 5-hour usage limit (03:20 to 06:17,
about 09:40 to 11:10, about 14:00 to 16:35). At 16:16 a scheduled resume started a
second orchestrator while this one was paused; both ran reviewers. One session at a
time is now enforced by `scripts/fleet/owner-check.sh`.

### Open PRs (merge order)
| PR | Task | State |
|---|---|---|
| #32 → #37, #45 | P1-RT-09 engine patch, P1-RT-11 hostile harness, P1-RT-15 OOM fixes | #32 and #37 approved; P1-RT-15b adds the bind sweep test and nits (#45 blockers); then a delta review and normal merges |
| #47 | P1-RT-14 time-accurate deadlines (R4, R5) | opened, not reviewed; patch level becomes 3 if #45 lands first |
| #12 → #27 → #16 | gfx sanitizer, outlines, hand-off | #12 BLOCK (relative URLs pass as `data:image/`), #27 BLOCK (carries #12); being fixed |
| #25 → #26 → #34 | facade on the real backend, CLI, licences | BLOCKs from 16:25 being folded in; critical path for the checkpoint run |
| #35 | P1-CF-09 checkpoint-2 run script | approved at 1beb0b9; main merged in (66cba58), delta in review; merges after #27 |
| #50, #51 | P1-CF-11 review follow-ups (checks, compat.py, offline WebRTC) | in review; #50 can merge any time, #51 after #35 and #50 |
| #18 → #19 → #23 | README, guides, examples | waiting for #25, then review |
| #38 | v0.1 release PR | last; only with Kay's OK at checkpoint 2 |

### In progress
- Runtime: P1-RT-15b (neutral engine notes, bind sweep test); P1-RT-14 (#47); perf P1-RT-16/17 after the #37 fixes.
- Graphics: #12/#27 URL classification fix, forged data attributes, then #16 and P1-GFX-06.
- Packaging: #25 breakout tests and review items, then #26, #34, #44.

### Blocked / waiting on Kay
- Nothing. (Kay deleted the two PR comments and stopped the duplicate session at 17:07. quickjs-ng wants AI-found bugs as public issues, so P1-RT-20 prepares issue drafts with reproducers on master for Kay to verify and file.)

### Next
1. When P1-RT-15b reports: one fresh reviewer for #32's delta plus #45; then merge #32, #37 (mechanical main merge if needed), then #45; add the QuickJS THIRD_PARTY row (in PENDING_DECISIONS).
2. Review #47 when RT says it is ready.
3. When #25 merges: Docs and DX batch 2, then review #18/#19/#23. Then P1-CF-09's real run from main.
4. Checkpoint 2 package (Sun ~18:00): fixtures side by side with Chrome, Grover table (PSS, from CI), hostile results, the v0.1 release PR with a passing dry-run, and "Open for checkpoint 2" in docs/fleet/PENDING_DECISIONS.md.

### Disk
When free space drops under ~5 GB, run `scripts/fleet/prune-target.sh` (freed 12.4 GB at 16:40).

### Branch rule
Never rebase or force-push a pushed branch; fix with new commits; downstream branches merge their parent in. PRs merge with merge commits. The orchestrator may merge main into an approved branch when the result is exactly the mechanical merge.
