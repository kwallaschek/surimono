# frozen_string_literal: true

# What a Rails app does to print a report with charts, in plain Ruby: an ERB
# view writes island markers with a helper, one surimono renderer per process
# draws them, and fulgur prints the result. docs/guide/rails.md explains each
# part. Run from the repository root, with the gem built:
#
#   ruby -Ibindings/ruby/lib examples/rails-snippet/render_report.rb            # HTML to stdout
#   ruby -Ibindings/ruby/lib examples/rails-snippet/render_report.rb out.pdf    # PDF (needs the fulgur gem)
#   ruby -Ibindings/ruby/lib examples/rails-snippet/render_report.rb --input examples/shared/report.html

require "erb"
require "json"
require "surimono"

module ReportExample
  ROOT = File.expand_path("../..", __dir__)

  # The same files, in the same order, for surimono and fulgur. In a Rails app
  # this list lives in an initializer.
  FONTS = %w[
    NotoSans-Regular.ttf
    NotoSans-Bold.ttf
    NotoSansJP-Regular.otf
    NotoSansJP-Bold.otf
  ].map { |name| File.join(ROOT, "fixtures/fonts", name) }.freeze

  SPARKLINE = File.join(ROOT, "examples/shared/sparkline.js")
  TEMPLATE = File.join(__dir__, "report.html.erb")

  Report = Struct.new(:title, :quarters, :revenue, :trend, :trend_label, keyword_init: true)

  def self.sample_report
    Report.new(title: "Quarterly revenue", quarters: %w[Q1 Q2 Q3 Q4], revenue: [120, 200, 150, 80],
               trend: [120, 200, 150, 80, 140, 190], trend_label: "+12 %")
  end

  # The helper from docs/guide/rails.md, without ActionView. The JSON is not
  # HTML-escaped, because script text is read exactly as written, but <, > and &
  # become \u escapes, so "</script>" in the data can't end the marker early.
  # (In Rails, use the guide's version: ERB there escapes unless html_safe.)
  module IslandHelper
    # Characters that must not appear raw in script text: they could end the
    # element or, for U+2028/U+2029, a JavaScript string.
    UNSAFE_IN_SCRIPT = /[&<>\u2028\u2029]/

    def surimono_island(renderer, spec, width: 600, height: 400, id: nil)
      json = JSON.generate(spec).gsub(UNSAFE_IN_SCRIPT) { |c| format("\\u%04x", c.ord) }
      id_attr = id ? %( id="#{ERB::Util.html_escape(id)}") : ""
      %(<script type="application/vnd.surimono+json"#{id_attr} ) +
        %(data-renderer="#{ERB::Util.html_escape(renderer)}" ) +
        %(data-width="#{Integer(width)}" data-height="#{Integer(height)}">#{json}</script>)
    end
  end

  # A stand-in for a Rails view: instance variables plus helpers.
  class View
    include ERB::Util
    include IslandHelper

    def initialize(report)
      @report = report
    end

    def render(template)
      ERB.new(template).result(binding)
    end
  end

  def self.html_for(report)
    View.new(report).render(File.read(TEMPLATE, encoding: "UTF-8"))
  end

  LOCK = Mutex.new

  # One renderer, built on first use and shared by every thread. It survives
  # fork (Puma's preload_app!, Unicorn, Resque): a forked child starts its own
  # JavaScript runtimes on its first render.
  def self.renderer
    @renderer || LOCK.synchronize { @renderer ||= build_renderer }
  end

  def self.build_renderer
    Surimono::Renderer.new(
      fonts: FONTS,
      bundles: { "sparkline" => { source: File.read(SPARKLINE, encoding: "UTF-8"), entry: "render" } },
    )
  end

  # Prints with fulgur, using the same font files. Raises LoadError without the gem.
  def self.to_pdf(html)
    require "fulgur"
    assets = Fulgur::AssetBundle.new
    FONTS.each { |path| assets.add_font_file(path) }
    Fulgur::Engine.new(assets: assets).render_html(html)
  end

  def self.main(argv)
    args = argv.dup
    input = (i = args.index("--input")) ? args.slice!(i, 2).last : nil
    html = input ? File.read(input, encoding: "UTF-8") : html_for(sample_report)
    out = renderer.render(html)
    if (pdf_path = args.first)
      to_pdf(out).write_to_path(pdf_path)
    else
      $stdout.write(out)
    end
    0
  rescue Surimono::Error => e
    warn "error: #{e.message}"
    if e.island_index
      warn "failed island: index #{e.island_index} (from 0), id #{e.island_id.inspect}, " \
           "renderer #{e.renderer.inspect}, kind #{e.kind}"
    end
    warn "JavaScript stack:\n#{e.js_stack}" if e.is_a?(Surimono::JavaScriptError) && e.js_stack
    1
  end
end

exit ReportExample.main(ARGV) if $PROGRAM_NAME == __FILE__
