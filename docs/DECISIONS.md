# Decisions

One line per decision: date (JST), decision, why. Kay reads this at every
checkpoint and can overturn anything. Newest last.

- 2026-10-31 · Pin blitz-dom =0.2.4, blitz-html =0.2.0, blitz-traits =0.2.0 (with stylo 0.8, parley 0.6, taffy 0.9) · These are the exact versions in fulgur-cli 0.40.0's Cargo.lock, the latest fulgur; Blitz 0.3 betas exist but fulgur hasn't moved, and phase 3 needs the same layout fulgur prints.
- 2026-10-31 · Pin rquickjs =0.14.0 (QuickJS-ng) · Latest release; memory limit, interrupt handler and stack limit are built in.
- 2026-10-31 · MSRV 1.87, and rust-toolchain.toml pins 1.87.0 itself · rquickjs 0.14 needs 1.87 (Blitz 0.2.4 needs 1.86); pinning the toolchain to the MSRV means every build proves it.
- 2026-10-31 · Rust edition 2024, Cargo resolver 3 · Current edition; resolver 3 respects rust-version when picking dependency versions.
- 2026-10-31 · cargo-deny allows MPL-2.0 only as named exceptions for the Servo style crates (stylo, selectors, servo_arc and friends) · Blitz cannot work without Stylo, fulgur ships the same; MPL-2.0 is file-level copyleft and fine for unmodified dependencies of an MIT OR Apache-2.0 crate, but it is not "permissive", so it is fenced to those crates.
- 2026-10-31 · `publish = false` across the workspace until the v0.1 release PR · Makes an accidental `cargo publish` impossible; the release PR flips it and the dry-run proves it.
- 2026-10-31 · Crate names: surimono (facade), surimono-core, -webidl, -dom, -gfx, -islands, -cli (binary `surimono`), surimono-ruby (native ext, never published) · One prefix, matches the brief's layout; all names are free on crates.io today.
- 2026-10-31 · Ruby gem via magnus =0.9.3 and rb-sys =0.9.130, extension crate in the workspace at bindings/ruby/ext/surimono but out of default-members · Plain `cargo build` needs no Ruby headers or libclang; CI and scripts/ci.sh build it explicitly.
- 2026-10-31 · Kay's LICENSE (MIT, "Copyright (c) 2026 Kay Wallaschek") becomes LICENSE-MIT unchanged, plus LICENSE-APACHE · The brief licenses MIT OR Apache-2.0; the file names follow the Rust convention.
- 2026-10-31 · The repo (kwallaschek/surimono) is public, so docs/BRIEF.md and docs/PROJECT_INSTRUCTIONS.md stay untracked (gitignored) · They mention Kay's employer and personal checklist; the living brief is the Claude doc linked in CLAUDE.md.
- 2026-10-31 · Keep macOS and ARM off pull requests even though Actions is free on a public repo · PR feedback stays fast; main and nightly still cover them.
- 2026-10-31 · The repo's root commit was re-created with Kay's GitHub noreply address as author and main force-pushed once, at Kay's explicit request · The original carried his work email in a public repo.
- 2026-10-31 · The bootstrap commits (workspace, then fleet scripts) go straight to main after scripts/ci.sh passes locally; every later change goes through a pull request, CI and a fresh reviewer · There is no CI to gate the commit that creates CI.
- 2026-10-31 · All agents share one cargo target dir (/home/claude/target) and 2 build jobs · The build machine has 2 cores and 7 GB; separate target dirs per worktree would rebuild Stylo several times over.
- 2026-10-31 · Workflows are thin wrappers around scripts/ci.sh and scripts/ci/*.sh, and Kay commits the YAML once · The fleet's GitHub App may not push .github/workflows/, so all CI logic lives in scripts the fleet can change.
- 2026-10-31 · CI jobs publish logs and results to a `ci-results` branch (runs/<sha>/<workflow>/<job>/) · The fleet's proxy blocks the Actions API (runs, logs, artifacts), but git works; this is how failures reach the owning agent.
- 2026-10-31 · Nightly AddressSanitizer job on Rust nightly, excluding the Ruby extension · The brief asks for ASan nightly; the Ruby ext links libruby, which ASan builds don't handle cleanly.
- 2026-10-31 · Fleet GitHub calls go through scripts/fleet/ (open PR, read check runs and ci-results, merge); reviewer verdicts are PR comments · Through the proxy the fleet can open, comment on and merge PRs on this repo and read check runs, but not write commit statuses or read the Actions API.
- 2026-10-31 · The orchestrator pushes its own docs (STATUS, TASKS, DECISIONS, SETUP, fleet role cards) and fleet scripts straight to main; code and everything specialists own goes through pull requests · These files don't affect the build, and STATUS changes after every merge.
- 2026-10-31 · A scheduled task resumes the fleet every 4 hours (at :16 JST); it exits at once if the orchestrator heartbeat (branch fleet-heartbeat) is under 3 hours old · Keeps the fleet going if this session ends, without ever running two orchestrators.
