# Proposed workflow files

The fleet can't push files under `.github/workflows/`, so workflow YAML that needs
adding or changing waits here until Kay copies it into `.github/workflows/` and
pushes. Keep these files thin: all CI logic belongs in `scripts/ci.sh` and
`scripts/ci/`.

## Current proposal: musl native gems (E2-11)

Copy `docs/ci/full.yml` to `.github/workflows/full.yml`. It is the only file
that changes, and only on one line: the `native-gems` matrix adds
`x86_64-linux-musl` and `aarch64-linux-musl` (rb-sys-dock images
`rbsys/<platform>:0.9.130`; Rust targets `x86_64-unknown-linux-musl` and
`aarch64-unknown-linux-musl`). The script side is already on main:
`scripts/ci/native-gem.sh` builds them like the other platforms. On the x86_64
runner it also installs the x86_64 musl gem and runs the gem's tests in Alpine's
`ruby:<version>-alpine3.23` image for every Ruby from 3.2 to 4.0
(`gem_check_mode` in `scripts/ci/lib.sh`). The aarch64 musl gem gets the same
content checks as the aarch64 glibc gem. It costs two more Linux jobs a night
and no macOS minutes. Copy it any time after the E2-11 pull request has merged.
Before that, the jobs would run a native-gem.sh that can't test a musl gem.

## Adopted

- 5 Nov 2026: token isolation (a `publish` job, the only one with
  `contents: write`), `macos-15`, the bench.yml caches and the
  `actions/download-artifact` v5.0.0 pin. All four files under `docs/ci/`
  matched `.github/workflows/` after that, until the proposal above.
