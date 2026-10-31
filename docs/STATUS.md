# Status

Updated after every task. Times are JST.

## Now (Sat 31 Oct 2026, 11:20)

**Phase 1: islands.** Merged into main so far: fonts, CI, pre-pass, facade
(stand-in backend), fixture pipeline, sandbox with determinism fixes, island
runner, pool, soak, ECharts, Vega, gfx text measurement, hostile suite and the 11
fixtures. The fleet stopped twice on the account's 5-hour usage limit (03:20 to
06:17 and about 09:40 to 11:10) and resumed where it stopped.

### Open PRs (merge order)
| PR | Task | State |
|---|---|---|
| #30 | P1-RT-10 gfx measurer in islands, deadline hook | blockers fixed, delta review running |
| #32 + #33 | P1-RT-09 vendored QuickJS patch + gem staging of vendor/ | #32 approved (release blockers R4 to R8 open); #33 in review; merge together |
| #37 | P1-RT-11 hostile harness (all 59 cases) | in review; carries the #11 test fix needed once #32 lands, so merge right after #32 |
| #39 | P1-RT-12 islands follow-ups | in review |
| #12 → #27 → #16 | gfx sanitizer, outlines, hand-off | #12 and #27 blocked (sanitizer bypasses, outline bounds), being fixed; #16 approved |
| #14, #20, #22 | gem fix, CI scripts + YAML proposal, pre-pass follow-ups | in review |
| #25 → #26 → #34 | facade on the real backend, CLI, licences | in review |
| #2, #24, #28, #31 | manifest, colour checks, COMPAT generator, Grover bench | blocked, being fixed |
| #18 → #19 → #23 | README, guides, examples | waiting for the facade to settle, then review |
| #38 | v0.1 release PR | last; only with Kay's OK at checkpoint 2 |

### In progress
- Runtime p1-rt-14: deadline enforcement (watchdog flag on every poll, polls in expensive natives) — v0.1 blockers R4, R5.
- Runtime p1-rt-15: allocation-failure crash fixes in the vendored engine — v0.1 blockers R6, R7, R8 (details private).
- Graphics batch 3: CSS tokenizer for the sanitizer, SMIL dropped, outline bounds.
- Conformance batch 3: #31, #28, #2, #24 fixes, then P1-CF-09 (checkpoint-2 run of all fixtures through the real pipeline).
- Reviews: #14/#20/#22/#33, #25/#26/#34, #30/#37/#39.

### Blocked / waiting on Kay
- Delete one public PR comment with unreported QuickJS security details (asked 09:10): https://github.com/kwallaschek/surimono/pull/32#issuecomment-6535086647

### Next
1. Merge each PR when a fresh reviewer approves its exact head and CI is green; move its DECISIONS lines from docs/fleet/PENDING_DECISIONS.md.
2. When #25 merges: start Docs and DX batch 2 (resolve `confirm` markers, CHANGELOG 0.1.0, Ferrum/WickedPDF notes), then review #18/#19/#23.
3. Checkpoint 2 package (Sun ~18:00): fixtures side by side with Chrome, Grover table (PSS, from CI), hostile results, the v0.1 release PR with a passing dry-run, and the open questions in docs/fleet/PENDING_DECISIONS.md ("Open for checkpoint 2").

### Disk
When free space drops under ~5 GB, run `scripts/fleet/prune-target.sh`.

### Branch rule
Never rebase or force-push a pushed branch; fix with new commits; downstream branches merge their parent in. PRs merge with merge commits. The orchestrator may merge main into an approved branch when the result is exactly the mechanical merge (see DECISIONS).
