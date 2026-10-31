# frozen_string_literal: true

# Checks the Rails snippet the way a user would: the view writes valid markers,
# every island becomes SVG, output is the same every time, a renderer built
# before fork works in the child, and a broken island names itself. Run from the repository root:
#
#   ruby -Ibindings/ruby/lib examples/rails-snippet/render_report_test.rb

require "minitest/autorun"
require "timeout"
require_relative "render_report"

class RenderReportTest < Minitest::Test
  MARKER = "application/vnd.surimono+json"

  def report_html(**changes)
    report = ReportExample.sample_report
    changes.each { |k, v| report[k] = v }
    ReportExample.html_for(report)
  end

  # The JSON inside each island marker, by id.
  def island_specs(html)
    html.scan(%r{<script type="#{Regexp.escape(MARKER)}" id="([^"]+)"[^>]*>(.*?)</script>}m)
        .to_h { |id, json| [id, JSON.parse(json)] }
  end

  def test_the_view_writes_one_marker_per_chart
    specs = island_specs(report_html)
    assert_equal %w[revenue trend], specs.keys
    assert_equal %w[Q1 Q2 Q3 Q4], specs["revenue"].dig("xAxis", "data")
    assert_includes report_html, %(id="revenue" data-renderer="echarts" data-width="680" data-height="300">)
  end

  def test_data_cannot_end_a_marker_early
    title = %(Q&A "1" </script><script>alert(1)</script>)
    html = report_html(title: title)
    assert_equal 2, html.scan("<script").size, "the title opened or closed a script element"
    assert_equal title, island_specs(html)["revenue"].dig("title", "text")
    assert_includes html, "<h1>Q&amp;A &quot;1&quot; &lt;/script&gt;"
  end

  def test_renders_every_island_and_keeps_the_rest
    out = ReportExample.renderer.render(report_html)
    refute_includes out, MARKER
    assert_includes out, %(<div class="surimono-island" id="revenue" data-renderer="echarts" ) +
                         %(style="width:680px;height:300px;overflow:hidden"><svg)
    assert_includes out, %(<div class="surimono-island" id="trend" data-renderer="sparkline" ) +
                         %(style="width:200px;height:24px;overflow:hidden"><svg)
    assert out.start_with?(report_html[0, report_html.index("<script")]), "the page before the first island changed"
    assert_equal Encoding::UTF_8, out.encoding
  end

  def test_same_input_same_output
    html = report_html
    assert_equal ReportExample.renderer.render(html), ReportExample.renderer.render(html)
  end

  # Puma's preload_app!, Unicorn and Resque build the renderer, then fork. The
  # parent has rendered (its runtime threads are running); the child uses the
  # same Renderer object, which starts its own runtimes there.
  def test_a_renderer_built_before_fork_works_in_the_child
    parent = ReportExample.renderer
    html = report_html
    expected = parent.render(html)
    reader, writer = IO.pipe
    pid = fork do
      reader.close
      child = ReportExample.renderer
      writer.write(child.equal?(parent) ? child.render(html) : "a different renderer")
      writer.close
      exit!(0)
    end
    writer.close
    begin
      Timeout.timeout(60) { Process.wait(pid) }
    rescue Timeout::Error
      Process.kill(:KILL, pid)
      flunk "the forked child hung while rendering"
    end
    # The pipe reads in the default external encoding (US-ASCII in a C locale).
    assert_equal expected, reader.read.force_encoding(Encoding::UTF_8)
  end

  def test_a_broken_island_names_itself
    html = %(<h1>Report</h1>\n<script type="#{MARKER}" id="sales" data-renderer="no-such-chart">{}</script>)
    error = assert_raises(Surimono::RenderError) { ReportExample.renderer.render(html) }
    assert_equal "unknown-renderer", error.kind
    assert_equal 0, error.island_index
    assert_equal "sales", error.island_id
    assert_equal "no-such-chart", error.renderer
    assert_match(/unknown renderer/, error.message)
  end

  def test_prints_a_pdf_with_fulgur
    begin
      require "fulgur"
    rescue LoadError
      skip "the fulgur gem is not installed"
    end
    pdf = ReportExample.to_pdf(ReportExample.renderer.render(report_html))
    assert pdf.to_s.start_with?("%PDF-"), "not a PDF"
  end
end
