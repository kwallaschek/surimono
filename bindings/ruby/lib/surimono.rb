# frozen_string_literal: true

require_relative "surimono/version"

begin
  ruby_version = RUBY_VERSION[/\d+\.\d+/]
  require_relative "surimono/#{ruby_version}/surimono"
rescue LoadError
  require "surimono/surimono"
end

# Run a document's JavaScript without a browser, so browserless
# HTML-to-PDF engines can print it.
module Surimono
  class Error < StandardError; end
end
