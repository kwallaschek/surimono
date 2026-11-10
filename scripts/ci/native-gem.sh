#!/usr/bin/env bash
# Cross-compiles a precompiled native gem with rb-sys-dock (full.yml, per platform).
# Usage: scripts/ci/native-gem.sh <platform>   e.g. x86_64-linux, arm64-darwin,
#        x86_64-linux-musl
#
# rb-sys-dock mounts only the directory it is started in. The extension depends on
# workspace crates outside bindings/ruby, so the gem is first staged into a
# self-contained tree (bindings/ruby/tmp/stage, see bindings/ruby/tasks/gem.rake)
# and rb-sys-dock runs there: the container sees exactly the source gem's files.
# The gems land in bindings/ruby/pkg/, where full.yml uploads them from.
set -euo pipefail
platform="$1"
if [ "${SURIMONO_PRECOMPILE:-}" = 0 ]; then
  echo "SURIMONO_PRECOMPILE=0: native gems must ship precompiled built-ins" >&2; exit 1
fi
rubies="${RUBY_VERSIONS:-3.2,3.3,3.4,4.0}"
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
ensure_toolchain
ruby_path
out="${CI_JOB_OUT:-$REPO_ROOT/ci-out}"
mkdir -p "$out"
gem_dir="$REPO_ROOT/bindings/ruby"
cd "$gem_dir"
bundle install --quiet

step "stage the gem"
bundle exec rake gem:stage
stage="$gem_dir/tmp/stage"

step "rb-sys-dock: $platform, Ruby $rubies"
(cd "$stage" && BUNDLE_GEMFILE="$gem_dir/Gemfile" bundle exec rb-sys-dock \
  --platform "$platform" --ruby-versions "$rubies" --build)

mkdir -p pkg
cp -v "$stage"/pkg/*.gem pkg/
ls -la pkg/ | tee "$out/gems.txt"

step "check the gem's contents"
gem_file="$(ls pkg/surimono-*-"$platform"*.gem | head -n1)"
tar -xOf "$gem_file" data.tar.gz | tar -tzf - | sort | tee "$out/contents.txt"
for v in ${rubies//,/ }; do
  grep -Eq "^lib/surimono/$v/surimono\.(so|bundle)$" "$out/contents.txt" \
    || { echo "missing the extension for Ruby $v in $gem_file" >&2; exit 1; }
done
if grep -Eq '\.rs$|Cargo\.(toml|lock)$|\.c$' "$out/contents.txt"; then
  echo "native gem carries Rust or C sources" >&2; exit 1
fi
# The statically linked crates' and bundled libraries' licences and notices.
grep -qx 'THIRD_PARTY_LICENSES.txt' "$out/contents.txt" \
  || { echo "native gem has no THIRD_PARTY_LICENSES.txt" >&2; exit 1; }

# Only a gem the runner can load is installed, tested and checked for
# precompiled bytecode: one for its own platform, here, and a musl gem for its
# CPU, in Alpine's ruby image (gem_check_mode in lib.sh). Gems for other
# platforms get the content checks above and nothing more.
# gem:verify fails unless the gem has built-in renderers and
# Surimono.precompiled_builtins equals their number: the cross-built library
# carries bytecode its own engine accepts (SURIMONO_PRECOMPILE stays unset in
# this job).
host="$(ruby -e 'print Gem::Platform.local.to_s')"
case "$(gem_check_mode "$platform" "$host")" in
  host)
    step "install $gem_file and run the tests (Ruby $(ruby -e 'print RUBY_VERSION'))"
    bundle exec rake "gem:verify[$gem_dir/$gem_file]"
    ;;
  alpine)
    if ! command -v docker >/dev/null; then
      # On GitHub a skip would turn the job green without having run the gem.
      [ -z "${GITHUB_ACTIONS:-}" ] || { echo "docker is needed to test the $platform gem" >&2; exit 1; }
      echo "no docker: $platform gem not installed or tested"
      exit 0
    fi
    # The tag names the Alpine release, so a new one can't change the libc
    # under the test unannounced. Bundler and rake come with Ruby there, so
    # gem:verify's own code is loaded without the Rakefile and the build gems
    # it needs. The repository is mounted at its own path, owned by the
    # runner's user, so nothing in it ends up owned by root.
    for v in ${rubies//,/ }; do
      image="ruby:$v-alpine${ALPINE_VERSION:-3.23}"
      step "install $gem_file and run the tests in $image"
      docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp \
        -v "$REPO_ROOT:$REPO_ROOT" -w "$gem_dir" "$image" \
        ruby -e 'require "bundler"; require "rake"; load "tasks/gem.rake"; SurimonoGem.verify_gem(ARGV[0])' \
        "$gem_dir/$gem_file"
    done
    ;;
  *) echo "host platform $host: $platform gem not installed, tested or checked for bytecode" ;;
esac
