# frozen_string_literal: true

require "minitest/autorun"
require "fileutils"
require "tmpdir"
require "rubygems"

# The gemspec, evaluated in a tree shaped like the staged one (tasks/gem.rake).
class GemspecTest < Minitest::Test
  GEM_DIR = File.expand_path("..", __dir__)

  def write(root, rel, body = "x")
    path = File.join(root, rel)
    FileUtils.mkdir_p(File.dirname(path))
    File.write(path, body)
  end

  # setup-ruby's bundler-cache installs gems into vendor/bundle next to the
  # gemspec; `rake native` or `rake build` there must not pack them.
  def test_only_vendored_crates_are_packed
    Dir.mktmpdir do |root|
      FileUtils.cp(File.join(GEM_DIR, "surimono.gemspec"), root)
      write(root, "lib/surimono/version.rb", File.read(File.join(GEM_DIR, "lib/surimono/version.rb")))
      write(root, "lib/surimono.rb")
      write(root, "crates/core/Cargo.toml")
      write(root, "vendor/rquickjs-sys/Cargo.toml")
      write(root, "vendor/rquickjs-sys/quickjs/quickjs.c")
      write(root, "vendor/bundle/ruby/3.3.0/gems/rake-13.3.0/lib/rake.rb")
      write(root, "vendor/cache/rake-13.3.0.gem")
      write(root, "vendor/notes.txt")
      write(root, "THIRD_PARTY_LICENSES.txt")
      files = Gem::Specification.load(File.join(root, "surimono.gemspec")).files
      assert_includes files, "vendor/rquickjs-sys/Cargo.toml"
      assert_includes files, "vendor/rquickjs-sys/quickjs/quickjs.c"
      assert_includes files, "crates/core/Cargo.toml"
      assert_empty files.grep(%r{\Avendor/(bundle|cache)/}), files.inspect
      refute_includes files, "vendor/notes.txt"
      assert_includes files, "THIRD_PARTY_LICENSES.txt"
    end
  end
end
