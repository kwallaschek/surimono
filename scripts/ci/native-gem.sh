#!/usr/bin/env bash
# Cross-compiles a precompiled native gem with rb-sys-dock (full.yml, per platform).
# Usage: scripts/ci/native-gem.sh <platform>   e.g. x86_64-linux, arm64-darwin
#
# rb-sys-dock mounts only the directory it is started in. The extension depends on
# workspace crates outside bindings/ruby, so the gem is first staged into a
# self-contained tree (bindings/ruby/tmp/stage, see bindings/ruby/tasks/gem.rake)
# and rb-sys-dock runs there: the container sees exactly the source gem's files.
# The gems land in bindings/ruby/pkg/, where full.yml uploads them from.
set -euo pipefail
platform="$1"
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

# A gem for this runner's own platform can be installed and tested right here.
host="$(ruby -e 'print Gem::Platform.local.to_s')"
case "$host" in
  "$platform"|"$platform"-gnu)
    step "install $gem_file and run the tests (Ruby $(ruby -e 'print RUBY_VERSION'))"
    bundle exec rake "gem:verify[$gem_dir/$gem_file]"
    ;;
  *) echo "host platform $host: install test skipped for $platform" ;;
esac
