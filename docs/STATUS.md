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
font weight/style check (#43) and gem staging of the vendored engine (#33).

The fleet stopped three times on the account's 5-hour usage limit (03:20 to 06:17,
about 09:40 to 11:10, about 14:00 to 16:35). At 16:16 a scheduled resume started a
second orchestrator while this one was paused; both ran reviewers. One session at a
time is now enforced by `scripts/fleet/owner-check.sh`.

### Open PRs (merge order)
| PR | Task | State |
|---|---|---|
| #39 → #46 | P1-RT-12 islands follow-ups, Vega hash | approved; main merged in mechanically (b94217c, c3f6fba); merge when CI is green |
| #32 → #37, #45 | P1-RT-09 engine patch, P1-RT-11 hostile harness, P1-RT-15 OOM fixes | #32 and #37 approved; P1-RT-15b is rewording engine notes neutrally and adding the bind sweep test (#45 blockers); then a delta review, then #32 and #45 squash-merge |
| #47 | P1-RT-14 time-accurate deadlines (R4, R5) | opened, not reviewed; patch level becomes 3 if #45 lands first |
| #12 → #27 → #16 | gfx sanitizer, outlines, hand-off | #12 BLOCK (relative URLs pass as `data:image/`), #27 BLOCK (carries #12); being fixed |
| #25 → #26 → #34 | facade on the real backend, CLI, licences | BLOCKs from 16:25 being folded in; critical path for the checkpoint run |
| #35 | P1-CF-09 checkpoint-2 run script | approved; after #27; needs main merged (conflicts with #43) |
| #44 | P1-PKG-01c CI hardening + YAML proposal | BLOCK (results-free compat check in the gate; symlinks); after #26 |
| #18 → #19 → #23 | README, guides, examples | waiting for #25, then review |
| #38 | v0.1 release PR | last; only with Kay's OK at checkpoint 2 |

### In progress
- Runtime: P1-RT-15b (neutral engine notes, bind sweep test); P1-RT-14 (#47); perf P1-RT-16/17 after the #37 fixes.
- Graphics: #12/#27 URL classification fix, forged data attributes, then #16 and P1-GFX-06.
- Packaging: #25 breakout tests and review items, then #26, #34, #44.

### Blocked / waiting on Kay
- Send the private quickjs-ng report now (Claude project doc `claude/private-quickjs-ng-security-report.md`); the fixes are readable on public PR branches.
- Delete two public PR comments with engine details: #32 comment 6535086647 and #45 comment 6540599018 (the fleet's GitHub access can't edit or delete them).

### Next
1. Merge #39 then #46 when CI is green on b94217c / c3f6fba; move #39's DECISIONS lines.
2. When P1-RT-15b reports: one fresh reviewer for #32's delta plus #45; then `MERGE_METHOD=squash scripts/fleet/merge-pr.sh 32 <sha>`, #37 (mechanical main merge if needed), then #45 squash; add the QuickJS THIRD_PARTY row (in PENDING_DECISIONS).
3. Review #47 when RT says it is ready.
4. When #25 merges: Docs and DX batch 2, then review #18/#19/#23. Then P1-CF-09's real run from main.
5. Checkpoint 2 package (Sun ~18:00): fixtures side by side with Chrome, Grover table (PSS, from CI), hostile results, the v0.1 release PR with a passing dry-run, and "Open for checkpoint 2" in docs/fleet/PENDING_DECISIONS.md.

### Disk
When free space drops under ~5 GB, run `scripts/fleet/prune-target.sh` (freed 12.4 GB at 16:40).

### Branch rule
Never rebase or force-push a pushed branch; fix with new commits; downstream branches merge their parent in. PRs merge with merge commits, except PRs whose commits describe unreported engine bugs, which squash (see DECISIONS). The orchestrator may merge main into an approved branch when the result is exactly the mechanical merge.
