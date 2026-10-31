# Status

Updated after every task. Times are JST.

## Now (Sat 31 Oct 2026, 06:30)

**Phase 1: islands.** Kay authorised fleet merges at 06:17. Six reviewed PRs are
merged (#1 fonts, #4 CI, #7 pre-pass, #10 facade, #6 fixture pipeline, #5 sandbox).
All agents stopped 03:20 to 06:17 on the account's 5-hour usage limit and were
resumed at 06:20.

### Done
- 06:20 · Merged #1, #4, #7, #10, #6, #5 (merge commits, reviewed heads pinned).
- 01:19 · Checkpoint 1 approved; workflows committed by Kay; CI publishes to ci-results; scheduled resume every 4 h at :16.
- 02:10 to 02:20 · Batch 1 delivered by all four specialists (18 PRs, all green on Linux CI; #4 and #14 green on the full matrix).
- 02:30 to 02:55 · Review round 1 (fresh reviewer per area).

### Review state (merge order in brackets)
| PR | Task | Verdict |
|---|---|---|
| #1 | P1-CF-02 fonts | approved [1] |
| #2 | P1-CF-05 real-world manifest | approved |
| #6 | P1-CF-01 fixture pipeline | approved [after #1] |
| #9 | P1-CF-04 hostile suite | blocked: one Chrome expectation wrong; adding hang cases |
| #17 | P1-CF-03 10 fixtures | blocked: page heights that Chrome rounds (1/300 in) |
| #3 | P1-GFX-01/02 measureText | blocked: caches bounded by count, not bytes |
| #12 | P1-GFX-03 sanitize_svg | blocked: style escape inside integration points (security) |
| #16 | P1-GFX-04 hand-off | blocked: fulgur prints inline SVG text with system fonts |
| #5 | P1-RT-01 sandbox | approved to merge; v0.1 release blocked on the QuickJS patch (P1-RT-09) |
| #8 | P1-RT-02 determinism | blocked: three Date / prepareStackTrace host leaks |
| #11 | P1-RT-03 island runner | approved [after #5, #8] |
| #13 | P1-RT-04 pool | blocked: leftover debug print |
| #15 | P1-RT-05 soak | blocked: RSS metric unreliable; switch to live heap bytes |
| #4 | P1-PKG-01 CI | approved |
| #7 | P1-PKG-03 pre-pass | approved |
| #10 | P1-PKG-02 facade | approved [after #7] |
| #14 | P1-PKG-04 gem | blocked: Thread#raise longjmps over Rust frames |

### In progress
- Runtime (islands): fixes for #13 and #15, then ECharts (P1-RT-06) and Vega (P1-RT-07).
- Runtime (sandbox hardening): #8 fixes, then the QuickJS patch (P1-RT-09).
- Graphics: #3, #12 and #16 fixes, then island text as outlines (P1-GFX-05).
- Conformance: #9 and #17 fixes, a colour-aware diff, then the Grover benchmark (P1-CF-06) and the COMPAT generator (P1-CF-07).
- Packaging: #14 fix, pre-pass follow-ups, real backend with sanitised SVG (P1-PKG-08), CLI, licence file, then the v0.1 release PR.
- Docs and DX: README, guides, examples.

### Blocked
- Nothing. Risk: the usage limit can pause the fleet again; agents resume where they stopped.

### Next
1. Merge each fixed PR as soon as a fresh reviewer approves its head: #8, then #11, #13, #15, #21; #3, #12, #16; #9, #17; #2 (pyc removed); #14. Move its DECISIONS lines from docs/fleet/PENDING_DECISIONS.md.
2. Fresh reviewers for every fixed or new PR.
3. Start the DOM engineer (phase 2 scaffolding) once the build machine is less busy.
4. Checkpoint 2 package (Sun ~18:00): 10 side-by-side fixtures, Grover table, hostile results, v0.1 release PR with dry-run.

### Disk
The session's disk allowance is small. When free space drops under ~5 GB, run `scripts/fleet/prune-target.sh` (freed 15 GB at 08:00).

### Branch rule
Never rebase or force-push a pushed branch; fix with new commits; downstream branches merge their parent in. PRs merge with merge commits.
