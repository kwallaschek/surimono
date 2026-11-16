#!/usr/bin/env ruby
# frozen_string_literal: true

# Runs one saved HTML page through surimono, reports what happened and prints it
# with fulgur if the gem is there. docs/guide/checkpoint-3.md walks through it.
# Run from the repository root, with the gem built:
#
#   ruby -Ibindings/ruby/lib examples/checkpoint-3/try_template.rb page.html \
#     [--assets public] [--font font.ttf ...] [--time-ms 2000]
#
# Or, for a page from a Rails app, from the app's root, with surimono in its
# Gemfile, so the app's own stylesheets and scripts load as its server gives
# them (Surimono::Rails.assets):
#
#   bin/rails runner /path/to/surimono/examples/checkpoint-3/try_template.rb \
#     tmp/page.html --rails [--host cdn.example.com ...]
#
# Writes page.out.html, and page.pdf with the fulgur gem. Exits 0 when the page
# rendered, whatever its report says; 1 when surimono raised.

require "optparse"
require "surimono"

ROOT = File.expand_path("../..", __dir__)
DEFAULT_FONTS = %w[NotoSans-Regular.ttf NotoSans-Bold.ttf NotoSansJP-Regular.otf NotoSansJP-Bold.otf]
                .map { |name| File.join(ROOT, "fixtures/fonts", name) }.freeze

options = { fonts: [], assets: nil, rails: false, hosts: [], time_ms: 2000 }
OptionParser.new do |o|
  o.banner = "usage: try_template.rb page.html [options]"
  o.on("--assets DIR", "serve URLs starting with / from this folder") { |v| options[:assets] = File.realpath(v) }
  o.on("--rails", "load the app's own assets with Surimono::Rails.assets (under bin/rails runner)") { options[:rails] = true }
  o.on("--host HOST", "with --rails, an asset host the app's config doesn't name (an asset_host proc), repeated") do |v|
    options[:hosts] << v
  end
  o.on("--font FILE", "a font file, repeated in fallback order (default: Noto Sans and Noto Sans JP)") { |v| options[:fonts] << v }
  o.on("--time-ms N", Integer, "limits[:time_ms] (default 2000)") { |v| options[:time_ms] = v }
end.parse!
abort "usage: try_template.rb page.html [--assets DIR | --rails [--host HOST]] [--font FILE] [--time-ms N]" unless ARGV.size == 1
abort "--assets and --rails don't go together" if options[:assets] && options[:rails]
abort "--host needs --rails" if options[:hosts].any? && !options[:rails]
if options[:rails] && !(defined?(::Rails) && ::Rails.respond_to?(:application) && ::Rails.application)
  abort "--rails needs the app: run this with bin/rails runner from the app's root"
end

input = ARGV.first
html = File.read(input, encoding: "UTF-8")
fonts = options[:fonts].empty? ? DEFAULT_FONTS : options[:fonts]
base = input.delete_suffix(File.extname(input))

# Files for the page, from one folder and nowhere else. The url is as the page
# wrote it.
loader = lambda do |url|
  path = File.realpath(File.join(options[:assets], url.sub(/[?#].*\z/, "")))
  path.start_with?("#{options[:assets]}/") ? File.binread(path) : nil
rescue SystemCallError
  nil
end
# A new Surimono::Rails.assets for each render: it keeps what it read for one
# page.
load_files = lambda do
  if options[:rails]
    require "surimono/rails"
    Surimono::Rails.assets(hosts: options[:hosts])
  else
    options[:assets] ? { loader: loader, allow: ["/"] } : {}
  end
end

begin
  renderer = Surimono::Renderer.new(fonts: fonts, limits: { time_ms: options[:time_ms] })
  started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  page = renderer.render_page(html, **load_files.call)
  elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - started
rescue Surimono::Error => e
  puts "FAILED: #{e.class} [#{e.kind}]: #{e.message}"
  exit 1
end

report = page.report
puts "rendered in #{(elapsed * 1000).round} ms, #{page.html.bytesize} bytes"
puts "settled: #{report.settled}"
puts "uncaught exceptions: #{report.uncaught}"
report.errors.each { |message| puts "  #{message}" }
puts "unhandled rejections: #{report.unhandled_rejections}"
report.rejections.each { |message| puts "  #{message}" }
puts "not loaded: #{report.failed_count}"
report.failed.each { |f| puts "  #{f[:kind]} #{f[:url]}: #{f[:reason]}" }

# Zeros don't show up in a report, so ask again with layout queries that throw.
begin
  Surimono::Renderer.new(fonts: fonts, limits: { time_ms: options[:time_ms] }, layout_queries: :throw)
                    .render_page(html, **load_files.call)
  puts "layout queries: none"
rescue Surimono::LayoutQueryError => e
  puts "layout queries: yes, first #{e.member} (answered with zeros until phase 3)"
end

File.write("#{base}.out.html", page.html)
puts "wrote #{base}.out.html"

begin
  require "fulgur"
rescue LoadError
  puts "no fulgur gem, so no PDF"
  exit 0
end
assets = Fulgur::AssetBundle.new
fonts.each { |path| assets.add_font_file(path) }
Fulgur::Engine.new(assets: assets).render_html(page.html).write_to_path("#{base}.pdf")
puts "wrote #{base}.pdf"
