# Proposed workflow files

The fleet can't push files under `.github/workflows/`, so workflow YAML that needs
adding or changing waits here until Kay copies it into `.github/workflows/` and
pushes. Keep these files thin: all CI logic belongs in `scripts/ci.sh` and
`scripts/ci/`.

## Current proposal (copy all four files at once)

Copy after the pull request that adds `scripts/ci/job-status.sh` and the
`--from` mode of `scripts/ci/publish-results.sh` has merged: the new publish
job runs the default branch's copy of that script.

- **Token isolation.** Every workflow is `contents: read`. Build jobs write
  `ci-out/job.json` (`job-status.sh`) and upload `ci-out/` as an artifact
  named `ci-out-<job>[-<matrix>]`. A separate `publish` job (`needs:` all of
  them, `if: always()`) is the only one with `contents: write`: it checks out
  the default branch without persisted credentials, downloads the artifacts
  and runs `publish-results.sh --from results`. No code under test runs in a
  job that holds the write token. The script also runs git with hooks and
  fsmonitor off and passes the token in `GIT_CONFIG_*` variables.
- **macOS:** `macos-15` replaces `macos-14` in full.yml (GitHub retires the
  macos-14 image on 2 November 2026).
- **bench.yml:** installs bench/rust's toolchain, then caches both
  workspaces (`. -> target` and `bench/rust -> ../../target/bench-rust`; the
  target path is relative to the workspace), so fulgur's dependencies stop
  rebuilding on every run.
- New action pin: `actions/download-artifact` v5.0.0
  (634f93cb2916e3fdff6788551b99b062d0335ce0, from `git ls-remote`).
- ci.yml's `publish` skips fork pull requests (their token is read-only).
- Lint: `actionlint -shellcheck= docs/ci/*.yml` (1.7.12) is clean.

One pull request after the copy, the old in-job mode
(`publish-results.sh <status>`, which runs where code under test ran with
the token) is removed from the script.

- **ci-check branches.** full.yml and bench.yml also run on pushes to
  `ci-check/**`. Once this YAML is live, the fleet pushes `ci-check/<task>`
  to run them before a merge, instead of `release/ci-check-<task>`.
