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
