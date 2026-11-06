# frozen_string_literal: true

# Checks the page example the way a user would: the scripts ran, the page's
# files are in the output, the report says what went wrong, and a limit raises.
# Run from the repository root:
#
#   ruby -Ibindings/ruby/lib examples/rails-snippet/render_page_test.rb

require "minitest/autorun"
require "timeout"
require_relative "render_page"

class RenderPageTest < Minitest::Test
  def page_html
    File.read(PageExample::PAGE, encoding: "UTF-8")
  end

  def test_the_page_ran_and_its_files_are_inlined
    page = PageExample.render(page_html)
    assert_includes page.html, %(<h1 id="title">Invoice 2026-0117</h1>)
    assert_includes page.html, "<td>印刷 (printing)</td><td>3</td><td>240</td><td>720</td>"
    assert_includes page.html, %(<p id="total">Total: 1835</p>)
    assert_includes page.html, %(<div class="surimono-island" id="amounts" data-renderer="echarts")
    assert_includes page.html, "<style>"
    refute_includes page.html, "<link"
    refute_includes page.html, "<noscript"
    refute_includes page.html, "invoice.js"
    assert_equal ["settled: idle"], PageExample.report_lines(page.report)
  end

  def test_same_input_same_bytes
    assert_equal PageExample.render(page_html).html, PageExample.render(page_html).html
  end

  def test_an_exception_goes_in_the_report_and_the_page_goes_on
    page = PageExample.render("<!doctype html><script>throw new Error('boom')</script>" \
                              "<script>document.body.append('after')</script>")
    assert_includes page.html, "after"
    assert_equal 1, page.report.uncaught
    assert_equal ["Error: boom"], page.report.errors
  end

  def test_a_file_off_the_list_goes_in_the_report
    page = PageExample.render("<!doctype html><script src='https://example.com/a.js'></script><p>still here</p>")
    assert_includes page.html, "still here"
    assert_equal 1, page.report.failed_count
    assert_equal({ kind: :script, url: "https://example.com/a.js", reason: "not in the loader's allowlist" },
                 page.report.failed.first)
  end

  def test_a_missing_file_goes_in_the_report
    page = PageExample.render("<!doctype html><script src='/assets/none.js'></script>")
    assert_equal 1, page.report.failed_count
  end

  def test_a_limit_raises
    assert_raises(Surimono::TimeLimitError) do
      PageExample.render("<!doctype html><script>while (true) {}</script>")
    end
  end

  def test_a_timeout_stops_the_page_and_frees_its_runtime
    assert_raises(Timeout::Error) do
      Timeout.timeout(0.2) { PageExample.render("<!doctype html><script>while (true) {}</script>") }
    end
    assert_includes PageExample.render("<!doctype html><p>next").html, "next"
  end

  def test_prints_a_pdf_with_fulgur
    begin
      require "fulgur"
    rescue LoadError
      skip "the fulgur gem is not installed"
    end
    pdf = PageExample.to_pdf(PageExample.render(page_html).html)
    assert pdf.to_s.start_with?("%PDF-"), "not a PDF"
  end
end
