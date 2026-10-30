# Status

Updated after every task. Times are JST.

## Now (Sat 31 Oct 2026, 01:30)

**Phase 1: islands.** Bootstrap is done; the phase 1 specialists start now.
Checkpoint 1 signed off by Kay at 01:19 (SPEC v1 as written). CI is live on GitHub; jobs publish logs to the ci-results branch.

### Done
- 01:19 · Checkpoint 1 approved. Kay committed the workflows (c6624b3). Scheduled resume every 4 h at :16 created.
- 01:10 · P0-01 to P0-03 · Workspace, toolchain pin (1.87.0), exact rquickjs and Blitz pins, deny.toml, licences, CLAUDE.md, CI workflows, docs. `scripts/ci.sh` green locally: fmt, clippy, 6 tests, cargo-deny.
- 01:00 · Root commit re-authored with Kay's GitHub noreply address (Kay's request); push access confirmed.

### In progress
- Phase 1 specialists: runtime (sandbox), graphics (text metrics), conformance (fixtures and Chrome references), packaging (CI end to end).

### Blocked
- Nothing. Known red on main: full.yml ruby and native-gem jobs (gemspec has no authors); packaging engineer is fixing it.

### Next
1. Merge the specialists' first pull requests on green CI plus reviewer approval.
2. After checkpoint 1: start Docs and DX, and the DOM engineer's phase 2 scaffolding.
3. Islands end to end for checkpoint 2 (Sun 1 Nov ~18:00).
