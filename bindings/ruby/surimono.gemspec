# frozen_string_literal: true

require_relative "lib/surimono/version"

Gem::Specification.new do |spec|
  spec.name = "surimono"
  spec.version = Surimono::VERSION
  spec.summary = "Run a document's JavaScript without a browser, so browserless HTML-to-PDF engines can print it."
  spec.description = "surimono runs chart libraries and page scripts in a sandboxed QuickJS runtime and hands " \
                     "the result to browserless PDF engines such as fulgur."
  spec.licenses = ["MIT", "Apache-2.0"]
  spec.required_ruby_version = ">= 3.2", "< 4.1"
  spec.files = Dir["lib/**/*.rb", "ext/**/*.{rs,toml,rb}", "LICENSE-*", "README.md"]
  spec.require_paths = ["lib"]
  spec.extensions = ["ext/surimono/extconf.rb"]
  spec.add_dependency "rb_sys", "~> 0.9.130"
  spec.metadata["rubygems_mfa_required"] = "true"
end
