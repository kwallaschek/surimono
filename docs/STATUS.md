# Status

Updated after every task. Times are JST.

> **Scheduled sessions:** if `scripts/fleet/owner-check.sh` prints STOP, or the heartbeat
> is under 3 hours old and names another session: change nothing more, end the session.

## Now (Sun 1 Nov 2026, 05:20)

**Phase 1: islands. Checkpoint 2 is today about 18:00.** The live session with Kay ended
(heartbeat 231 min old at 04:16); a scheduled resume (cse_011c8hfq5ufjN5m2pZpZysXx) holds
the fleet now. Since 04:20: merged #55 (CF-12), #25 (facade on the real backend), #26 (CLI),
#34 (licences), #56 (precompiled bytecode), #57 (ASan stack probe), each on green ci/linux and
a fresh reviewer's APPROVE. Agents of the previous session were lost with its machine; their
unpushed work on #47 was redone (head 2268e80).

### Open PRs (merge order)
| PR | Task | State |
|---|---|---|
| #47 | P1-RT-14 + P1-RT-22 deadlines, BigInt pow leak fix, fork-safe registry | fixes pushed 2268e80; delta review next |
| #18 → #19 → #23 | README, guides, examples | DX batch 2 (update to the merged facade/CLI, CHANGELOG 0.1.0), then review |
| #38 | v0.1 release PR | update to main, dry-run; merge only with Kay's OK at checkpoint 2 |

### Blocked / waiting on Kay
- Nothing.

### Next (in order; a resumed session starts here)
1. #47 delta review of 2268e80; merge on approval; move its PENDING_DECISIONS lines; mark P1-RT-14/22 done.
2. P1-CF-09 real run from main (`checkpoint.mjs`, release CLI); commit report; regenerate COMPAT.md; bench (Grover table, PSS) from main's full.yml/bench run.
3. DX batch 2 on #18/#19/#23, then review and merge.
4. #38 to main; dry-run; webidl/dom stay publish=false.
5. Checkpoint 2 message (~18:00 JST): fixtures side by side with Chrome, Grover table, hostile results, release PR with dry-run, "Open for checkpoint 2" items in docs/fleet/PENDING_DECISIONS.md, YAML copy for Kay (docs/ci/, macos-15 before 2 Nov). Weekly digest ~20:00.
6. Follow-ups queued: P1-PKG-11, P1-RT-28, P1-RT-29, P1-RT-25/26/27, P1-RT-20/21/23/18.

### Disk
When free space drops under ~5 GB, run `scripts/fleet/prune-target.sh`.

### Branch rule
Never rebase or force-push a pushed branch; fix with new commits; downstream branches merge their parent in. PRs merge with merge commits. The orchestrator may merge main into an approved branch when the result is exactly the mechanical merge.
