# frozen_string_literal: true

require_relative "lib/surimono/version"

Gem::Specification.new do |spec|
  spec.name = "surimono"
  spec.version = Surimono::VERSION
  spec.authors = ["Kay Wallaschek"]
  spec.homepage = "https://github.com/kwallaschek/surimono"
  spec.summary = "Run a document's JavaScript without a browser, so browserless HTML-to-PDF engines can print it."
  spec.description = "surimono runs chart libraries and page scripts in a sandboxed QuickJS runtime and hands " \
                     "the result to browserless PDF engines such as fulgur."
  spec.licenses = ["MIT", "Apache-2.0"]
  spec.required_ruby_version = ">= 3.2", "< 4.1"

  # Build gems with `rake gem:source` (and scripts/ci/native-gem.sh), never with
  # `gem build` in bindings/ruby: the extension needs workspace crates that only
  # the staged tree (tasks/gem.rake) carries. There, Cargo.toml, Cargo.lock and
  # crates/ sit next to this file; in the repository those globs match nothing.
  spec.files = Dir.chdir(__dir__) do
    Dir[
      "lib/**/*.rb",
      "ext/surimono/{extconf.rb,Cargo.toml}", "ext/surimono/src/**/*.rs",
      "Cargo.{toml,lock}", "crates/**/*",
      "LICENSE-*", "THIRD_PARTY.md", "README.md"
    ].select { |f| File.file?(f) }.sort
  end
  spec.require_paths = ["lib"]
  spec.extensions = ["ext/surimono/extconf.rb"]
  spec.add_dependency "rb_sys", "~> 0.9.130"

  spec.metadata["source_code_uri"] = "https://github.com/kwallaschek/surimono"
  spec.metadata["rubygems_mfa_required"] = "true"
end
