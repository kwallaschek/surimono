# CLAUDE.md: how the surimono fleet works

surimono is an open-source Rust library, CLI and Ruby gem that runs a document's
JavaScript without a browser, so browserless HTML-to-PDF engines (fulgur and
others) can print it. Every agent reads this file before starting work.

## Read first

1. The project brief: https://claude.ai/code/artifact/e7d73b62-6c99-4410-b3ca-5458d9a42045
   (a Claude doc; the fleet's machine also has an untracked copy at `docs/BRIEF.md`).
   It is the source of truth for scope, phases, architecture, targets and timeline.
2. `docs/SPEC.md`: the confirmed one-page spec (name, licence, scope, targets).
3. `docs/STATUS.md`, `docs/TASKS.md`, `docs/DECISIONS.md`, `docs/COMPAT.md`.

Never redo work marked done in `docs/TASKS.md`.

## Folder ownership

Each agent changes only the folders it owns. Anything outside them goes through
the orchestrator (write the request in your final report).

| Agent | Owns |
|---|---|
| Orchestrator | `CLAUDE.md`, `Cargo.toml` (workspace root), `Cargo.lock`, `rust-toolchain.toml`, `deny.toml`, `scripts/` except `scripts/ci*`, `docs/*.md` except `COMPAT.md`, `docs/upstream/`, `THIRD_PARTY.md` |
| Conformance researcher | `fixtures/`, `wpt/`, `bench/`, `docs/COMPAT.md` |
| Runtime engineer | `crates/core/`, `crates/islands/`, `vendor/` |
| DOM engineer | `crates/webidl/`, `crates/dom/` |
| Graphics engineer | `crates/gfx/` |
| Packaging engineer | `crates/surimono/`, `crates/cli/`, `bindings/ruby/`, `.github/`, `scripts/ci.sh`, `scripts/ci/` |
| Docs and DX | `README.md`, `docs/guide/`, `examples/`, `CHANGELOG.md` |
| Reviewer | nothing; reviews every merge and can block it |

A crate's own `Cargo.toml` belongs to the crate's owner. New workspace-level
dependencies go in the root `Cargo.toml` via the orchestrator: ask in your report,
and meanwhile add the dependency directly in your crate's `Cargo.toml` with an
exact version. If another crate already uses the same dependency, use the root
`[workspace.dependencies]` pin (`<dep>.workspace = true`) instead; two different
exact pins of one crate can't resolve together.

## Fleet rules

- **Decide, don't ask.** When the brief leaves something open, choose the sensible
  default, add one line to `docs/DECISIONS.md` (date, decision, why) and continue.
  Specialists put that line in their final report; the orchestrator writes it.
- **Main always green.** One branch and one pull request per task. Merge only after
  CI passes and the reviewer approves.
- **Outside truth only.** Every expected result comes from a WPT test, a spec
  section or a Chrome reference render. When they disagree, the spec wins for DOM
  behaviour and Chrome wins for how the page looks; log the conflict. Never edit an
  expectation to match our output; record the failure in `docs/COMPAT.md` instead.
- **Same input, same output.** No wall-clock time, real randomness, system fonts or
  network in the render path. Every fixture renders byte-identically twice.
- **Memory is the product.** The soak test (10,000 renders in one process, flat
  resident memory after warm-up) stays green. JS holds ids, never pointers, across
  the Rust boundary.
- **Untrusted by default.** Every script runs with memory, time and stack limits,
  and with no filesystem or network unless the caller allows it. Hitting a limit
  gives a typed error, never a crash or a hang.
- **unsafe only in the engine-binding modules**, each module opted in with
  `#[allow(unsafe_code)]` (the workspace denies it), each block with a `// SAFETY:`
  comment and a test.
- **Update `docs/STATUS.md` after every task** with Done, In progress, Blocked and
  Next, plus times (JST). The orchestrator does this on merge.
- **Small, testable steps.** Every change ships with tests. Every public API change
  gets a line in `docs/DECISIONS.md`.
- **Dependencies must pass cargo-deny.** Bundled JS libraries keep their licence
  files and are listed in `THIRD_PARTY.md`.
- **Upstream changes** needed in Blitz, fulgur or rquickjs go in a local `[patch]`
  plus a draft in `docs/upstream/`. Kay files them.
- **Never commit secrets.** Publishing tokens stay with Kay.

## Hard stops: never do these

Publish to crates.io or RubyGems; push tags that trigger a release; make the repo
public; change the licence; accept agreements or CLAs; pay for anything; use Kay's
accounts or tokens; post publicly, email anyone, or open issues, pull requests or
comments on other projects; copy code from projects with incompatible licences or
vendor Chromium code; delete or change files outside the repo; force-push main.

## How a specialist works

1. **Worktree.** Work in your own git worktree, never in the main checkout:
   `git -C /home/claude/surimono worktree add /home/claude/wt/<task-id> -b <task-id> main`
   Branch names are the task id from `docs/TASKS.md` in lower case, e.g. `p1-rt-01`.
2. **Shared build cache.** Always `export CARGO_TARGET_DIR=/home/claude/target`,
   `export CARGO_BUILD_JOBS=2` and `export CARGO_INCREMENTAL=0` (disk is tight; never
   create another target dir) (the machine has 2 cores and 7 GB of RAM). Cargo
   serialises builds on that directory; waiting on its lock is normal. Build only
   your crates (`-p surimono-core`) while iterating. **Run
   `scripts/fleet/touch-workspace.sh` before every build or test**: worktrees share
   artifact names for workspace crates, so without it cargo can silently use another
   worktree's build of a crate (false red or false green). A worktree that changes
   `vendor/` (the engine) also sets `RUSTFLAGS="--cfg surimono_wt_<task_id>"` for every
   build and test, because touch-workspace.sh doesn't separate the engine's build
   output; delete those artifacts when done. GitHub CI uses a fresh target dir and
   is the authoritative result.
3. **Checks before you report.** Run `scripts/ci.sh` from your worktree; it runs
   exactly what CI runs on Linux (fmt, clippy with warnings as errors, tests,
   cargo-deny). Your branch must pass it.
4. **Commits.** Small commits with clear messages (`core: add memory limit to
   sandbox`). End every commit message with:
   ```
   Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
   ```
   Never commit to `main` and never merge; the orchestrator merges.
5. **Report.** Finish with a short report: branch name, what is done, test
   evidence (commands and results), DECISIONS lines to add, anything you need
   outside your folders, and what is left.

## CI on GitHub

- The fleet's GitHub access cannot push files under `.github/workflows/`. The
  workflows are thin wrappers that call `scripts/ci.sh` and `scripts/ci/*.sh`; change
  the scripts, not the YAML. A YAML change needs Kay: put the proposed file in
  `docs/ci/` and say so in your report.
- The fleet can't read the Actions API either. Every job publishes its log and
  results to the `ci-results` branch under `runs/<head sha>/<workflow>/<job>/`
  (`status.json`, `<name>/log.txt`, `<name>/exit.json`, plus anything written to
  `$CI_JOB_OUT`). Read them with
  `git fetch origin ci-results && git show origin/ci-results:runs/<sha>/...`.

## Unreported security findings

The repo is public. A bug in a third-party project (for example the QuickJS engine)
that hasn't been reported to its maintainers yet stays out of every tracked file,
commit message, PR body and PR comment. Describe what our change does, never how
the old code failed: no crash sites, signals, out-of-bounds or use-after-free
wording, and no triggering scripts. Details go only in `.private-tmp/`
(git-excluded) and your report to the orchestrator.

## Code conventions

- Rust 2024 edition, MSRV pinned in `rust-toolchain.toml`. `cargo fmt` defaults.
- Errors are typed (`thiserror`), never `panic!` on bad input or a hostile script.
  `unwrap()`/`expect()` only in tests or with a comment proving it can't fail.
- Public items carry doc comments. Keep the public API small; the facade crate
  `surimono` is the only API users should need.
- When writing files, byte-check any `\uXXXX` escapes in string literals; file tools have saved them as the literal characters.
- Tests live next to the code (`#[cfg(test)]`) or in the crate's `tests/`.
  Fixture-driven tests read from `fixtures/` and never rewrite expectations.
