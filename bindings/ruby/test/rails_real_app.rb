# frozen_string_literal: true

# Run by rails_test.rb's test_against_real_rails, in a process of its own:
# boots a small real Rails app on Propshaft or Sprockets (RAILS_ENV development
# or production, precompiled in production), writes tags with its own helpers,
# and checks that Surimono::Rails.assets loads what the app serves for them.
#
#   [ASSET_HOST=cdn.example.com] [RELATIVE_URL_ROOT=/app] \
#     ruby -I lib test/rails_real_app.rb propshaft|sprockets <empty dir> [precompile]
require "fileutils"

pipeline, dir = ARGV
css = "@media print { .a { color: red } }\n@media screen { .a { color: blue } }\n.b { background: url('logo.png') }\n"
files = {
  "app/assets/stylesheets/application.css" => css,
  "app/assets/javascripts/application.js" => "document.title = 'ran';\n",
  "app/assets/images/logo.png" => "PNG",
  "public/stylesheets/print.css" => "@media print { .p { color: black } }\n",
}
files["app/assets/stylesheets/theme.scss"] = "$c: red;\n@media print { .t { color: $c; } }\n" if pipeline == "sprockets"
files["app/assets/config/manifest.js"] = "//= link_tree ../stylesheets .css\n//= link_tree ../javascripts .js\n//= link_tree ../images\n" if pipeline == "sprockets"
files.each do |rel, body|
  FileUtils.mkdir_p(File.dirname(File.join(dir, rel)))
  File.write(File.join(dir, rel), body)
end

require "rails"
require "action_controller/railtie"
if pipeline == "propshaft"
  require "propshaft"
else
  require "sprockets/railtie"
  require "sassc"
end
require "surimono/rails"

class RealApp < Rails::Application
  config.load_defaults 8.1
  config.root = ARGV[1]
  config.eager_load = false
  config.secret_key_base = "x" * 64
  config.logger = Logger.new(nil)
  config.assets.compile = false if ENV["RAILS_ENV"] == "production" && ARGV[0] == "sprockets"
  config.asset_host = ENV["ASSET_HOST"] if ENV["ASSET_HOST"]
  config.relative_url_root = ENV["RELATIVE_URL_ROOT"] if ENV["RELATIVE_URL_ROOT"]
  config.assets.debug = true if ENV["ASSETS_DEBUG"]
end
RealApp.initialize!
app = Rails.application

# As a deploy does it: assets:precompile in a process of its own, then the app.
if ARGV[2] == "precompile"
  if pipeline == "propshaft"
    app.assets.processor.process
  else
    Sprockets::Railtie.build_environment(app).then do |e|
      Sprockets::Manifest.new(e, File.join(dir, "public/assets")).compile(%w[application.css application.js theme.css logo.png])
    end
  end
  exit 0
end

def check(cond, msg)
  return if cond

  warn "FAIL: #{msg}"
  exit 1
end

helpers = ActionController::Base.helpers
renderer = Surimono::Renderer.new(fonts: [File.expand_path("../../../fixtures/fonts/NotoSans-Regular.ttf", __dir__)])

host = ENV.fetch("ASSET_HOST", nil)
root = ENV.fetch("RELATIVE_URL_ROOT", nil)
begin
  tags = [helpers.stylesheet_link_tag("application"), helpers.javascript_include_tag("application"),
          helpers.stylesheet_link_tag("/stylesheets/print.css")]
  tags << helpers.stylesheet_link_tag("theme") if pipeline == "sprockets"
  tags << ActionController::Base.renderer.new(https: true).render(inline: "<%= stylesheet_link_tag 'application' %>") if host
  urls = tags.join("\n").scan(/(?:href|src)="([^"]+)"/).flatten
  puts "#{pipeline} #{ENV["RAILS_ENV"]} host=#{host.inspect} root=#{root.inspect}: #{urls.inspect}"
  check(urls.size == tags.size, "one URL per tag: #{tags}")
  assets = Surimono::Rails.assets
  urls.each do |url|
    bytes = assets[:loader].call(url)
    check(bytes, "no bytes for #{url}")
    check(Surimono::Rails.send(:allows?, assets[:allow], url), "#{url} not in #{assets[:allow]}")
    if url.include?("application") && url.end_with?(".css")
      # url('logo.png') rewritten to the digested logo, as the app serves it.
      check(bytes.include?("@media print { .a { color: red } }") && bytes.match?(%r{/assets/logo-\h+\.png}), "compiled CSS for #{url}: #{bytes}")
    end
    check(bytes.include?("color: red") && !bytes.include?("$c"), "Sass compiled for #{url}: #{bytes}") if url.include?("theme")
  end
  check(assets[:loader].call(urls[0].sub(/-\h+\.css/, "-0000000000.css")).nil?, "another digest gives nil")

  html = %(<!doctype html><html><head><title>t</title>#{tags[0]}#{tags[1]}</head><body><p class=a>x</p></body></html>)
  page = renderer.render_page(html, **Surimono::Rails.assets)
  check(page.report.failed_count.zero?, "page report: #{page.report.failed.inspect}")
  check(!page.html.include?("<link") && page.html.include?("@media all { .a { color: red } }") &&
        page.html.include?("<title>ran</title>"), "print rules: #{page.html}")
end
puts "ok #{pipeline}"
