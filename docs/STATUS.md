# Status

Updated after every task. Times are JST.

> **Scheduled sessions: stop.** A live orchestrator session (cse_011c8hfp9aeYhQp5RpWyywaN,
> with Kay) runs the fleet. If you are a scheduled resume and `scripts/fleet/owner-check.sh`
> prints STOP, or the heartbeat is under 3 hours old: change nothing more, stop your
> subagents, end the session.

## Now (Sun 1 Nov 2026, 00:40)

**Phase 1: islands. Checkpoint 2 is today about 18:00.** On main: fonts, CI, pre-pass,
facade (stand-in backend), fixture pipeline, sandbox with determinism fixes, the
patched QuickJS engine (#32 via #39) with the allocation-failure fixes (#45), island
runner with the on-demand pool (#54), ECharts, Vega, gfx text measurement, hostile
suite and harness (#37), the 11 fixtures, COMPAT generator (#28), font weight/style
check (#43), CI hardening with the final YAML proposal for Kay (#44), macOS and ASan
test fixes (#52, #53), and since 01:20 the gfx sanitizer with the island-box policy (#12), outlined text with stamps (#27), fulgur hand-off notes (#16), workspace buffer reset (#48), the checkpoint run script (#35) and the CF review follow-ups (#50, #51).

Usage-limit pauses on Sat 31 Oct: 03:20–06:17, ~09:40–11:10, ~14:00–16:35, ~18:00–23:03.

### Open PRs (merge order)
| PR | Task | State |
|---|---|---|
| #25 → #26 → #34 | facade on the real backend, CLI, licences | PKG merging the new p1-gfx-05 in, plus island clipping; then one fresh review for all three. **Critical path** |
| #55 | P1-CF-12 stamp-aware text/font checks | fix pushed (f411fe2); delta review running; needed before the real run |
| #47 | P1-RT-14 time-accurate deadlines (R4, R5) | BLOCK (BigInt pow leak); RT fixing, P1-RT-22 fork-safe registry folded in; merges main (patch level 3) |
| #56 | P1-RT-17b precompiled bytecode (cold start 2.0 → 1.5 s, PSS 41 → 27–29 MB) | in review; merge before the benchmark run |
| #57 | P1-RT-24 ASan stack probe (read SP, not a local's address) | in review; ASan stays red until P1-RT-27's time scale |
| #18 → #19 → #23 | README, guides, examples | waiting for #25, then DX batch 2 and review |
| #38 | v0.1 release PR | last; only with Kay's OK at checkpoint 2 |

### Blocked / waiting on Kay
- Nothing.

### Next (in order; a resumed session starts here)
1. Merge #55 on approval. Move each PR's DECISIONS lines from docs/fleet/PENDING_DECISIONS.md.
2. When PKG reports #25/#26/#34 with the new gfx: one fresh reviewer for all three; merge on approval.
3. When RT-17b's PR is up: fresh reviewer; merge before the benchmark.
4. After #25 (+ #26) merge: P1-CF-09 real run from main (`checkpoint.mjs`, release CLI), commit the report, regenerate COMPAT.md; trigger bench (Grover table, PSS) via a `release/ci-check-*` push or main merge.
5. DX batch 2 (CHANGELOG 0.1.0, Rails guide update for fork-safety/max_islands/pool), then review #18/#19/#23.
6. #47 delta review when RT pushes the fixes; merge.
7. Update #38 (release PR) to main; dry-run; webidl/dom stay publish=false.
8. Checkpoint 2 message (~18:00): fixtures side by side with Chrome, Grover table, hostile results, release PR with dry-run, "Open for checkpoint 2" items, YAML copy for Kay (docs/ci/, macos-15 before 2 Nov). Weekly digest ~20:00.

### Disk
When free space drops under ~5 GB, run `scripts/fleet/prune-target.sh`.

### Branch rule
Never rebase or force-push a pushed branch; fix with new commits; downstream branches merge their parent in. PRs merge with merge commits. The orchestrator may merge main into an approved branch when the result is exactly the mechanical merge.
