# Releasing surimono

Kay tags and publishes. The fleet never tags or publishes (CLAUDE.md, hard stops).

## No release before v0.3

Kay decided on 5 Nov 2026 that nothing is released until phase 3 is done. The `v0.1.0` tag
on `915d5cbf` stays as it is, but no gem was or will be pushed from it: that commit has a known
time-limit gap in the regexp compiler (fixed on main by #125/#126). The steps below are kept as the
template for the v0.3 release.

## v0.1.0 (template; not released)

The release PR #38 merged into main as `915d5cbf2ba2f2c4667da773380cd7c2da9ef387`. Its tree is
identical to the release-branch head `be5d49f5`, where all 15 CI checks passed (full.yml run
41783605064) and `scripts/ci/release-dry-run.sh` reported "ok: nothing was published". No Rust crate
is published in v0.1 (every package has `publish = false`).

1. **Tag** the merge commit and push the tag (no workflow runs on tags):
   ```sh
   git fetch origin && git tag -a v0.1.0 915d5cbf2ba2f2c4667da773380cd7c2da9ef387 -m "surimono v0.1.0"
   git push origin v0.1.0
   ```
2. **Native gems (4):** in GitHub Actions, open the full.yml run for `be5d49f5` (run 41783605064) or
   the main run for `915d5cbf`, and download the artifacts `gem-x86_64-linux`, `gem-aarch64-linux`,
   `gem-x86_64-darwin` and `gem-arm64-darwin`. Each holds one `surimono-0.1.0-<platform>.gem`.
   Artifacts expire after 14 days (around 18 Nov for run 41783605064).
3. **Source gem (1):** from a clean clone at the tag (needs Ruby, Rust 1.87+, a C compiler and libclang):
   ```sh
   git clone --branch v0.1.0 https://github.com/kwallaschek/surimono && cd surimono/bindings/ruby
   bundle install && bundle exec rake gem:source     # writes pkg/surimono-0.1.0.gem
   ```
4. **Push all five** (the gemspec requires MFA, so each push asks for a one-time code). Push all
   five: the README promises precompiled gems for those four platforms.
   ```sh
   gem push surimono-0.1.0.gem
   gem push surimono-0.1.0-x86_64-linux.gem
   gem push surimono-0.1.0-aarch64-linux.gem
   gem push surimono-0.1.0-x86_64-darwin.gem
   gem push surimono-0.1.0-arm64-darwin.gem
   ```
5. **Optional:** create a GitHub release for `v0.1.0` with the CHANGELOG's `[0.1.0]` section.

Afterwards the fleet pins the docs to the tag (P1-DX-07) and starts phase 2 (MSRV 1.89 first).
