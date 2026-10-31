# frozen_string_literal: true

# Gem packaging from inside the surimono repository.
#
# The extension depends on workspace crates by path (crates/surimono and what it
# pulls in), which live outside bindings/ruby. A gem can only carry files below
# its own directory, so `gem:stage` assembles a self-contained copy of the gem in
# tmp/stage: the gem's files, the crates the extension reaches by path, the
# vendored crates the workspace's [patch] sections point to (e.g. the patched
# QuickJS in vendor/rquickjs-sys), a workspace Cargo.toml naming only those
# crates and keeping its [patch] sections, and the repository's Cargo.lock
# trimmed to them. The source gem is built from that tree, and so are the
# precompiled native gems (scripts/ci/native-gem.sh runs rb-sys-dock in it), so
# CI compiles exactly what users install.

require "fileutils"
require "json"
require "open3"
require "rbconfig"
require "tmpdir"

module SurimonoGem
  GEM_DIR = File.expand_path("..", __dir__)
  REPO_ROOT = File.expand_path("../..", GEM_DIR)
  STAGE_DIR = File.join(GEM_DIR, "tmp", "stage")
  PKG_DIR = File.join(GEM_DIR, "pkg")
  EXT_REL = "bindings/ruby/ext/surimono"
  GEM_REL = "bindings/ruby"
  # Repository files every gem ships (licences) or the staged workspace needs.
  ROOT_FILES = %w[LICENSE-MIT LICENSE-APACHE THIRD_PARTY.md README.md].freeze
  # Gem-side paths that stay out of the staged tree.
  GEM_EXCLUDE = %r{\A(tasks|tmp|pkg|vendor|\.bundle)/}

  module_function

  def cargo
    ENV.fetch("CARGO", "cargo")
  end

  def capture!(*cmd, chdir: REPO_ROOT)
    out, err, status = Open3.capture3(*cmd, chdir: chdir)
    raise "#{cmd.join(" ")} failed (#{status.exitstatus}):\n#{err}" unless status.success?

    out
  end

  # Files of a directory that git tracks or would track (untracked but not
  # ignored), relative to the repository root.
  def repo_files(rel_dir)
    capture!("git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", rel_dir)
      .split("\0").reject(&:empty?).select { |f| File.file?(File.join(REPO_ROOT, f)) }.uniq
  end

  # Workspace-relative directories of the crates the extension reaches through
  # path dependencies (normal, build and dev), excluding the extension itself.
  def crate_dirs
    meta = JSON.parse(capture!(cargo, "metadata", "--format-version", "1", "--no-deps",
      "--manifest-path", File.join(REPO_ROOT, "Cargo.toml")))
    by_dir = meta.fetch("packages").to_h { |p| [File.dirname(p.fetch("manifest_path")), p] }
    seen = []
    queue = [File.join(REPO_ROOT, EXT_REL)]
    until queue.empty?
      dir = File.expand_path(queue.shift)
      next if seen.include?(dir)

      pkg = by_dir.fetch(dir) { raise "path dependency #{dir} is not a workspace member" }
      seen << dir
      pkg.fetch("dependencies").each { |d| queue << d["path"] if d["path"] }
    end
    (seen.map { |d| d.delete_prefix("#{REPO_ROOT}/") } - [EXT_REL]).sort
  end

  # Repository-relative directories that the workspace manifest's [patch.*]
  # sections point to with `path = "..."` (vendored, patched crates).
  def patch_dirs
    dirs = []
    section = nil
    File.foreach(File.join(REPO_ROOT, "Cargo.toml")) do |line|
      # Drop a comment (a `#` outside a string), so a commented-out
      # `path = "..."` isn't read.
      line = line.sub(/\A((?:[^"#]|"[^"]*")*)#.*/m, '\1')
      if (m = line.match(/^\s*\[([^\]]+)\]/))
        section = m[1].strip
      elsif section&.start_with?("patch") && (m = line.match(/\bpath\s*=\s*"([^"]+)"/))
        dirs << m[1]
      end
    end
    dirs.each do |d|
      raise "[patch] path #{d} is outside the repository" if d.start_with?("/") || d.split("/").include?("..")
      raise "[patch] path #{d} doesn't exist" unless File.directory?(File.join(REPO_ROOT, d))
    end
    dirs.uniq
  end

  # The repository's workspace manifest with the member list cut down to the
  # staged crates. Everything else (package defaults, workspace dependencies,
  # lints, profiles) stays as is; unused workspace dependencies are ignored by Cargo.
  def workspace_manifest(crates)
    text = File.read(File.join(REPO_ROOT, "Cargo.toml")).dup
    members = (["ext/surimono"] + crates).map { |m| "    #{m.inspect}," }.join("\n")
    text.sub!(/^members\s*=\s*\[.*?\]/m, "members = [\n#{members}\n]") ||
      raise("no members list in Cargo.toml")
    text = text.sub(/^default-members\s*=\s*\[.*?\]\n?/m, "")
    "# Generated from the surimono repository's Cargo.toml by bindings/ruby/tasks/gem.rake.\n" \
      "# It lists only the crates the Ruby extension needs.\n\n#{text}"
  end

  def copy(src_rel, dest_rel)
    dest = File.join(STAGE_DIR, dest_rel)
    FileUtils.mkdir_p(File.dirname(dest))
    FileUtils.cp(File.join(REPO_ROOT, src_rel), dest, preserve: true)
  end

  def stage
    crates = crate_dirs
    FileUtils.rm_rf(STAGE_DIR)
    FileUtils.mkdir_p(STAGE_DIR)

    repo_files(GEM_REL).each do |f|
      rel = f.delete_prefix("#{GEM_REL}/")
      copy(f, rel) unless rel.match?(GEM_EXCLUDE)
    end
    crates.each { |dir| repo_files(dir).each { |f| copy(f, f) } }
    patched = patch_dirs
    patched.each { |dir| repo_files(dir).each { |f| copy(f, f) } }
    ROOT_FILES.each { |f| copy(f, f) if File.file?(File.join(REPO_ROOT, f)) }
    # The licences and notices of everything the native library links, for
    # the gem (MIT/Apache/BSD notices; MPL-2.0 source availability).
    capture!(File.join(REPO_ROOT, "scripts/ci/third-party-licenses.sh"),
      File.join(STAGE_DIR, "THIRD_PARTY_LICENSES.txt"))
    File.write(File.join(STAGE_DIR, "Cargo.toml"), workspace_manifest(crates))
    copy("Cargo.lock", "Cargo.lock")
    # Trim the lock file to the staged crates. Versions stay as locked; Cargo only
    # drops packages nothing in the staged workspace uses.
    capture!(cargo, "metadata", "--format-version", "1", "--manifest-path",
      File.join(STAGE_DIR, "Cargo.toml"), chdir: STAGE_DIR)
    puts "staged #{crates.size} crates (#{crates.join(", ")})" \
         "#{patched.empty? ? "" : " and patched #{patched.join(", ")}"} in #{STAGE_DIR}"
    STAGE_DIR
  end

  def source_gem
    stage
    spec = Gem::Specification.load(File.join(STAGE_DIR, "surimono.gemspec"))
    missing = %w[Cargo.toml Cargo.lock ext/surimono/Cargo.toml THIRD_PARTY_LICENSES.txt] - spec.files
    raise "staged gemspec is missing #{missing.join(", ")}" unless missing.empty?

    FileUtils.mkdir_p(PKG_DIR)
    out = File.join(PKG_DIR, spec.file_name)
    Bundler.with_unbundled_env do
      Dir.chdir(STAGE_DIR) { sh_ok!("gem", "build", "surimono.gemspec", "--output", out) }
    end
    out
  end

  # Installs a gem file into an empty GEM_HOME and runs the test suite against
  # the installed copy (not lib/). For the source gem this compiles the extension
  # the way `gem install surimono` does on a machine without a precompiled gem.
  # Before the tests it checks that the installed extension is the one loaded
  # and that every built-in renderer loads precompiled bytecode
  # (Surimono.precompiled_builtins; 0 expected with SURIMONO_PRECOMPILE=0).
  def verify_gem(gem_file)
    home = File.join(GEM_DIR, "tmp", "gemhome")
    FileUtils.rm_rf(home)
    env = { "GEM_HOME" => home, "GEM_PATH" => home }
    Bundler.with_unbundled_env do
      sh_ok!(env, "gem", "install", "--no-document", "minitest", "-v", "~> 5.25")
      sh_ok!(env, "gem", "install", "--no-document", "--verbose", File.expand_path(gem_file))
      check = 'require "surimono"; f = $LOADED_FEATURES.grep(%r{/surimono\\.(so|bundle)\\z}).first; ' \
              'abort("surimono loaded from #{f.inspect}, not the installed gem") unless f&.start_with?(ENV["GEM_HOME"]); ' \
              'puts "native extension: #{f}"; ' \
              'b = Surimono.builtin_renderers; n = Surimono.precompiled_builtins; ' \
              'puts "precompiled built-ins: #{n} of #{b.size} (#{b.join(", ")})"; ' \
              'want = ENV["SURIMONO_PRECOMPILE"] == "0" ? 0 : b.size; ' \
              'abort("#{n} of #{b.size} built-ins load precompiled bytecode, expected #{want}") unless n == want'
      sh_ok!(env, RbConfig.ruby, "-e", check)
      tests = Dir[File.join(GEM_DIR, "test", "**", "*_test.rb")].sort
      sh_ok!(env, RbConfig.ruby, "-I", File.join(GEM_DIR, "test"), "-e",
        "ARGV.each { |f| require f }", *tests)
    end
  end

  def sh_ok!(*cmd)
    puts "+ #{cmd.reject { |c| c.is_a?(Hash) }.join(" ")}"
    system(*cmd) || raise("command failed: #{cmd.reject { |c| c.is_a?(Hash) }.join(" ")}")
  end
end

namespace :gem do
  desc "Assemble the self-contained gem source tree in tmp/stage"
  task :stage do
    SurimonoGem.stage
  end

  desc "Build the source gem (platform ruby) from the staged tree into pkg/"
  task :source do
    puts SurimonoGem.source_gem
  end

  desc "Build the source gem, install it into an empty GEM_HOME and run the tests against it"
  task :verify_source do
    SurimonoGem.verify_gem(SurimonoGem.source_gem)
  end

  desc "Install a built gem (e.g. a native gem) into an empty GEM_HOME and run the tests against it"
  task :verify, [:gem_file] do |_t, args|
    SurimonoGem.verify_gem(args.fetch(:gem_file))
  end
end
