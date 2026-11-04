#!/usr/bin/env bash
# Shared helpers for scripts/ci.sh and scripts/ci/*.sh. Source it after `cd` to
# the repository root:  . scripts/ci/lib.sh

REPO_ROOT="$(pwd)"

# The workflows set CARGO_TARGET_DIR=target, which Cargo resolves against the
# current directory. Make it absolute so builds started from bindings/ruby (rake,
# gem install) share the root target directory that rust-cache saves.
case "${CARGO_TARGET_DIR:-}" in
  "") ;;
  /*) ;;
  *) export CARGO_TARGET_DIR="$REPO_ROOT/$CARGO_TARGET_DIR" ;;
esac

# job.sh sets CI_JOB_OUT relative to the repository root; scripts that cd
# elsewhere need it absolute.
case "${CI_JOB_OUT:-}" in
  "") ;;
  /*) ;;
  *) export CI_JOB_OUT="$REPO_ROOT/$CI_JOB_OUT" ;;
esac

step() { printf '\n==> %s\n' "$*"; }

# The examples (examples/run.sh, from the Docs and DX branch #23). Until that
# lands, a branch without them skips the step, so either can merge first.
# Arguments go to examples/run.sh (--no-ruby: the Rust example only).
run_examples() {
  if [ -f examples/run.sh ]; then
    step "examples ${*:-(Rust and Ruby)}"
    bash examples/run.sh "$@"
  else
    step "examples: skipped, examples/run.sh is not on this branch yet"
  fi
}

# Executables of gems installed outside Bundler's bin dir (rbenv without shims,
# setup-ruby) must be on PATH for `bundle exec rake`.
ruby_path() {
  if command -v ruby >/dev/null; then
    PATH="$(ruby -e 'print Gem.bindir'):$PATH"
    export PATH
  fi
}

# rb-sys generates Ruby bindings with bindgen, which needs libclang. GitHub's
# x64 images have it; install it where it's missing (e.g. a leaner ARM image).
ensure_libclang() {
  [ "$(uname -s)" = Linux ] || return 0
  [ -n "${LIBCLANG_PATH:-}" ] && return 0
  if ldconfig -p 2>/dev/null | grep -q 'libclang'; then return 0; fi
  if ls /usr/lib/llvm-*/lib/libclang*.so* >/dev/null 2>&1; then return 0; fi
  if [ -n "${GITHUB_ACTIONS:-}" ] && command -v sudo >/dev/null; then
    step "installing libclang-dev (needed by rb-sys bindgen)"
    sudo apt-get update -qq && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq libclang-dev
  else
    echo "libclang not found; install libclang-dev (Linux) or set LIBCLANG_PATH" >&2
    return 1
  fi
}

# Installs the toolchain from rust-toolchain.toml if rustup hasn't yet.
ensure_toolchain() {
  if command -v rustup >/dev/null; then
    rustup show active-toolchain >/dev/null 2>&1 || rustup toolchain install
  fi
}

# cargo-about, pinned, for THIRD_PARTY_LICENSES.txt. It needs rustc 1.88, newer
# than the workspace's pinned toolchain, so it is built with stable; it's a
# tool, not part of what we ship.
CARGO_ABOUT_VERSION=0.9.3
ensure_cargo_about() {
  if cargo about --version 2>/dev/null | grep -qx "cargo-about $CARGO_ABOUT_VERSION"; then return 0; fi
  step "installing cargo-about $CARGO_ABOUT_VERSION"
  rustup toolchain list | grep -q '^stable' || rustup toolchain install stable --profile minimal
  RUSTUP_TOOLCHAIN=stable cargo install --locked cargo-about --version "$CARGO_ABOUT_VERSION" --features cli
}

# Prints the path of a working llvm-symbolizer, or nothing (P1-PKG-17). Without
# one, ASan and LSan print bare frames (binary+0x39e0e2). The nightly's
# llvm-tools component ships no llvm-symbolizer (checked on 1.101.0-nightly
# 2026-10-29), so this uses the distro's LLVM: ASAN_SYMBOLIZER_PATH if the
# caller set it, llvm-symbolizer on PATH, then the newest /usr/lib/llvm-N
# (Ubuntu's llvm-symbolizer-N links there; GitHub's images have LLVM but no
# unversioned llvm-symbolizer on PATH). On GitHub Actions it installs the
# distro's llvm package as a last resort.
find_llvm_symbolizer() {
  local c d
  for c in "${ASAN_SYMBOLIZER_PATH:-}" "$(command -v llvm-symbolizer 2>/dev/null || true)" \
    $(for d in /usr/lib/llvm-[0-9]*/bin/llvm-symbolizer; do
        [ -e "$d" ] && printf '%s %s\n' "${d#/usr/lib/llvm-}" "$d"
      done | sort -rn | cut -d' ' -f2); do
    if [ -n "$c" ] && [ -x "$c" ] && "$c" --version >/dev/null 2>&1; then
      echo "$c"
      return 0
    fi
  done
  if [ -n "${GITHUB_ACTIONS:-}" ] && [ -z "${SURIMONO_SYMBOLIZER_INSTALLED:-}" ] \
    && command -v sudo >/dev/null; then
    step "installing llvm (for llvm-symbolizer)" >&2
    sudo apt-get update -qq >&2 && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq llvm >&2 || return 0
    SURIMONO_SYMBOLIZER_INSTALLED=1 find_llvm_symbolizer
  fi
}

# The AddressSanitizer build and run environment for scripts/ci/asan.sh, kept
# here so a local check can source the exact same flags. Symbolized reports:
# ASAN_SYMBOLIZER_PATH names the symbolizer (the runtime also searches PATH,
# but only for the unversioned name) and symbolize=1 is set explicitly in both
# option sets, since LSan reads LSAN_OPTIONS for the report it prints at exit.
asan_env() {
  local sym
  export RUSTFLAGS="-Zsanitizer=address" RUSTDOCFLAGS="-Zsanitizer=address"
  export ASAN_OPTIONS="detect_leaks=1:symbolize=1"
  export LSAN_OPTIONS="symbolize=1"
  sym="$(find_llvm_symbolizer)"
  if [ -n "$sym" ]; then
    export ASAN_SYMBOLIZER_PATH="$sym"
    echo "ASan symbolizer: $sym ($("$sym" --version 2>/dev/null | grep -m1 -i 'version' || true))"
  else
    echo "warning: no llvm-symbolizer found; ASan/LSan frames stay unsymbolized" >&2
  fi
}
