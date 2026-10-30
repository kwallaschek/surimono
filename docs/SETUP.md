# Setup

- Repository: https://github.com/kwallaschek/surimono (public)
- Default branch: `main`; one branch and one pull request per task.
- Brief (living): https://claude.ai/code/artifact/e7d73b62-6c99-4410-b3ca-5458d9a42045
- Project instructions live in the Claude project "Surimono".

## Build machine (fleet)

- Linux x86_64, 2 cores, 7 GB RAM. Toolchain from `rust-toolchain.toml`.
- Shared target dir `/home/claude/target`, `CARGO_BUILD_JOBS=2`.
- Specialists work in worktrees under `/home/claude/wt/<task-id>`.
- `scripts/ci.sh` runs what `ci.yml` runs.

## GitHub Actions

| Workflow | When | What |
|---|---|---|
| `ci.yml` | every PR, pushes to main | Linux: fmt, clippy (warnings are errors), tests, cargo-deny |
| `full.yml` | main, nightly 02:00 JST, `release/**` | macOS, aarch64 Linux, Ruby 3.2 to 4.0, native gems, soak test |
| `reference.yml` | fixture changes on main, weekly | Chrome reference renders (pinned Playwright Chromium) |
| `bench.yml` | main, nightly 03:15 JST | Grover benchmark: time and peak memory |

Standard runners are free on public repos, so no spending limit is needed.

## Sessions

- A scheduled task resumes the orchestrator every 4 hours (at :16 JST). It stops at once if `scripts/fleet/heartbeat.sh --age` shows a heartbeat under 3 hours old, so two orchestrators never run together.
- Role cards for every agent: `docs/fleet/ROLES.md`.
