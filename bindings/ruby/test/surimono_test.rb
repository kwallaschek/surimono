# frozen_string_literal: true

require "minitest/autorun"
require "pathname"
require "timeout"
require "tmpdir"
require "surimono"

# Until the island runner lands, the extension renders with a stand-in whose
# only renderer, "surimono-stub", runs no JavaScript but can simulate a slow
# script (busy_ms), an exception (throw) and each limit. See
# crates/surimono/src/backend.rs.
class SurimonoTest < Minitest::Test
  TYPE = "application/vnd.surimono+json"
  STUB = "surimono-stub"
  REPO_FONT = File.expand_path("../../../fixtures/fonts/NotoSans-Regular.ttf", __dir__)

  def island(spec = "{}", renderer: STUB, attrs: "")
    %(<script type="#{TYPE}" data-renderer="#{renderer}" data-width="30" data-height="20"#{attrs}>#{spec}</script>)
  end

  def stub_svg
    '<div class="surimono-island" data-renderer="surimono-stub">' \
      '<svg xmlns="http://www.w3.org/2000/svg" width="30" height="20" viewBox="0 0 30 20"></svg></div>'
  end

  # A real test font when the repository's fixtures are there, else bytes that
  # look like a TrueType file (enough for the stand-in runner).
  def font_bytes
    return File.binread(REPO_FONT) if File.exist?(REPO_FONT)

    "\x00\x01\x00\x00".b + ("\x00".b * 64)
  end

  def renderer(**opts)
    @renderer ||= {}
    @renderer[opts] ||= Surimono::Renderer.new(**opts)
  end

  def test_native_version_matches_gem_version
    assert_equal Surimono::VERSION, Surimono.native_version
  end

  def test_document_without_islands_is_unchanged
    html = "<!doctype html>\r\n<p class=x>a &amp; b</p><script>var x = 1 < 2;</script>"
    assert_equal html, renderer.render(html)
  end

  def test_renders_islands_and_keeps_the_rest
    out = renderer.render("<h1>Sales</h1>\n#{island}\n<p>end</p>")
    assert_equal "<h1>Sales</h1>\n#{stub_svg}\n<p>end</p>", out
    assert_equal Encoding::UTF_8, out.encoding
  end

  def test_keeps_id_and_class
    out = renderer.render(island("{}", attrs: ' id="q3" class="chart wide"'))
    assert_match(/\A<div class="surimono-island chart wide" id="q3" data-renderer="surimono-stub">/, out)
  end

  def test_utf8_and_other_encodings
    html = "<p>売上 €</p>#{island}"
    assert_equal "<p>売上 €</p>#{stub_svg}", renderer.render(html)
    latin1 = "<p>café</p>".encode(Encoding::ISO_8859_1)
    assert_equal "<p>café</p>", renderer.render(latin1)
    assert_equal "<p>x</p>", renderer.render("<p>x</p>".b)
    assert_raises(ArgumentError) { renderer.render("<p>\xff</p>".b) }
  end

  def test_time_limit
    r = renderer(limits: { time_ms: 50 })
    e = assert_raises(Surimono::TimeLimitError) do
      r.render("<p>a</p>#{island('{"busy_ms": 5000}', attrs: ' id="slow"')}")
    end
    assert_kind_of Surimono::Error, e
    assert_equal "time-limit", e.kind
    assert_equal 0, e.island_index
    assert_equal "slow", e.island_id
    assert_equal STUB, e.renderer
    assert_match(/island #1 \(id "slow", renderer "surimono-stub"\): the script exceeded the time limit/, e.message)
  end

  def test_memory_and_stack_limits
    e = assert_raises(Surimono::MemoryLimitError) { renderer.render(island('{"limit": "memory"}')) }
    assert_equal "memory-limit", e.kind
    e = assert_raises(Surimono::StackOverflowError) { renderer.render(island('{"limit": "stack"}')) }
    assert_equal "stack-overflow", e.kind
  end

  def test_javascript_error
    e = assert_raises(Surimono::JavaScriptError) { renderer.render(island('{"throw": "boom"}')) }
    assert_equal "javascript", e.kind
    assert_match(/JavaScript error: Error: boom/, e.message)
    refute_nil e.js_stack
    refute_kind_of ::ScriptError, e
    assert_kind_of StandardError, e
  end

  def test_render_errors
    e = assert_raises(Surimono::RenderError) { renderer.render(island("{}", renderer: "plotly")) }
    assert_equal "unknown-renderer", e.kind
    assert_equal "plotly", e.renderer
    e = assert_raises(Surimono::RenderError) { renderer.render("#{island}#{island('{oops')}") }
    assert_equal "invalid-island", e.kind
    assert_equal 1, e.island_index
  end

  def test_placeholder_policy
    r = renderer(error_policy: :placeholder)
    out = r.render("#{island('{"throw": "x"}')}#{island}")
    assert_match(/\A<div class="surimono-island surimono-island-error" data-renderer="surimono-stub" data-error="javascript"><!-- surimono: /, out)
    assert out.end_with?(stub_svg), out
  end

  def test_fonts_from_paths_and_data
    data = font_bytes
    Dir.mktmpdir do |dir|
      path = File.join(dir, "font.ttf")
      File.binwrite(path, data)
      r = Surimono::Renderer.new(fonts: [path, Pathname.new(path), data])
      assert_equal stub_svg, r.render(island)
    end
    assert_raises(Errno::ENOENT) { Surimono::Renderer.new(fonts: ["/no/such/font.ttf"]) }
    e = assert_raises(Surimono::ConfigurationError) { Surimono::Renderer.new(fonts: [data, "not a font".b]) }
    assert_equal "font", e.kind
    assert_match(/font #2/, e.message)
    assert_raises(ArgumentError) { Surimono::Renderer.new(fonts: [42]) }
  end

  def test_option_checks
    assert_raises(ArgumentError) { Surimono::Renderer.new(limits: { time: 5 }) }
    assert_raises(ArgumentError) { Surimono::Renderer.new(limits: { time_ms: 0 }) }
    assert_raises(ArgumentError) { Surimono::Renderer.new(limits: { memory_mb: 1.5 }) }
    assert_raises(ArgumentError) { Surimono::Renderer.new(seed: -1) }
    assert_raises(ArgumentError) { Surimono::Renderer.new(error_policy: :ignore) }
    assert_raises(ArgumentError) { Surimono::Renderer.new(pool_size: 0) }
    assert_raises(ArgumentError) { Surimono::Renderer.new(colour: :red) }
    assert_raises(ArgumentError) { Surimono::Renderer.new(limits: { memory_mb: 2**50 }) }
    assert_raises(ArgumentError) { Surimono::Renderer.new(limits: "fast") }
    assert_raises(ArgumentError) { Surimono::Renderer.new(pool_size: 2**70) }
    assert_raises(ArgumentError) { Surimono::Renderer.new(renderers: "echarts") }
    assert_raises(ArgumentError) { Surimono::Renderer.new(renderers: [42]) }
    assert_raises(ArgumentError) { Surimono::Renderer.new(bundles: { "mine" => 42 }) }
    assert_raises(ArgumentError) { Surimono::Renderer.new(bundles: { "mine" => { entry: "f" } }) }
    assert_raises(ArgumentError) { renderer.render(nil) }
    assert_raises(ArgumentError) { renderer.render(42) }
    e = assert_raises(Surimono::ConfigurationError) { Surimono::Renderer.new(renderers: ["nope"]) }
    assert_equal "invalid-options", e.kind
    e = assert_raises(Surimono::ConfigurationError) do
      Surimono::Renderer.new(bundles: { "mine" => { source: "function f(){}", entry: "not a name" } })
    end
    assert_equal "bundle", e.kind
  end

  def test_renderers_and_bundles
    assert_equal [STUB], renderer.renderers
    r = Surimono::Renderer.new(bundles: { "mine" => { source: "function draw(){}", entry: "draw" } })
    assert_equal [STUB, "mine"], r.renderers
    only = Surimono::Renderer.new(bundles: { mine: "function render(){}" }, renderers: [:mine])
    assert_equal ["mine"], only.renderers
    assert_raises(Surimono::RenderError) { only.render(island) }
  end

  def test_other_threads_run_while_rendering
    r = renderer
    ticks = 0
    ticker = Thread.new do
      loop do
        ticks += 1
        sleep 0.005
      end
    end
    sleep 0.05 # let the ticker start
    before = ticks
    out = r.render(island('{"busy_ms": 600}'))
    during = ticks - before
    ticker.kill
    assert_equal stub_svg, out
    # With the GVL held, the ticker couldn't run at all during the 600 ms render.
    assert_operator during, :>=, 20, "ticker advanced only #{during} times during the render"
  end

  def test_many_threads_share_one_renderer
    r = renderer
    html = "<main>#{island('{"busy_ms": 100}')}</main>"
    started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    outs = Array.new(8) { Thread.new { r.render(html) } }.map(&:value)
    elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - started
    assert_equal ["<main>#{stub_svg}</main>"], outs.uniq
    # Eight 100 ms renders one after the other take 0.8 s; overlapping, well
    # under half of that even on a busy 2-core machine.
    assert_operator elapsed, :<, 0.5 * 8 * 0.1
  end

  # Resident set size of this process in bytes (Linux), or nil.
  def rss_bytes
    status = File.read("/proc/self/status")
    status[/^VmRSS:\s+(\d+) kB/, 1].to_i * 1024
  rescue SystemCallError
    nil
  end

  # Timeout (like Rack::Timeout and Ctrl-C) interrupts with Thread#raise. Ruby
  # delivers it when the GVL-free render returns; it must come out as the
  # interrupt's exception, without jumping over Rust frames and leaking what
  # they hold (here the ~4 MB output string of every interrupted render).
  def test_interrupted_renders_raise_and_free_memory
    r = renderer
    html = "<p>#{"x" * (4 * 1024 * 1024)}</p>#{island('{"busy_ms": 60}')}"
    interrupted = lambda do
      assert_raises(Timeout::Error) { Timeout.timeout(0.02) { r.render(html) } }
    end
    10.times { interrupted.call }
    GC.start
    before = rss_bytes
    50.times { interrupted.call }
    GC.start
    after = rss_bytes
    # The renderer still works afterwards.
    assert_equal "<p>a</p>#{stub_svg}", r.render("<p>a</p>#{island}")
    skip "no /proc/self/status to measure memory" unless before && after
    # Leaking the output alone would add 50 x 4 MB.
    assert_operator after - before, :<, 64 * 1024 * 1024,
                    "RSS grew #{(after - before) / 1024 / 1024} MB over 50 interrupted renders"
  end

  def test_killed_render_thread_ends_cleanly
    r = renderer
    t = Thread.new { r.render(island('{"busy_ms": 200}')) }
    sleep 0.05
    t.kill
    t.join
    refute t.alive?
    assert_equal stub_svg, r.render(island)
  end

  class Stop < StandardError; end

  # Renderer.new also runs without the GVL. An interrupt deferred to the next
  # blocking point arrives as building finishes; the built renderer is dropped
  # and the interrupt raised.
  def test_interrupt_around_renderer_new
    ready = Queue.new
    go = false
    t = Thread.new do
      Thread.current.report_on_exception = false
      Thread.handle_interrupt(Stop => :on_blocking) do
        ready << true
        x = 0
        x += 1 until go
        Surimono::Renderer.new(seed: 3)
      end
    end
    ready.pop
    t.raise(Stop, "stop")
    go = true
    e = assert_raises(Stop) { t.join }
    assert_equal "stop", e.message
  end

  def test_panic_becomes_internal_error
    e = assert_raises(Surimono::InternalError) { Surimono.send(:_internal_panic_check) }
    assert_kind_of Surimono::Error, e
    assert_equal "internal", e.kind
    assert_match(/deliberate panic/, e.message)
    assert_raises(NoMethodError) { Surimono._internal_panic_check }
  end

  def test_module_render_reuses_a_renderer_per_option_set
    assert_equal stub_svg, Surimono.render(island)
    a = Surimono.send(:renderer_for, { seed: 1 })
    b = Surimono.send(:renderer_for, { seed: 1 })
    c = Surimono.send(:renderer_for, { seed: 2 })
    assert_same a, b
    refute_same a, c
    20.times { |i| Surimono.send(:renderer_for, { seed: 100 + i }) }
    assert_operator Surimono.instance_variable_get(:@renderers).size, :<=, 4
    # Least recently used goes first: seed 1, used again after seed 200 was
    # added, outlives it.
    a = Surimono.send(:renderer_for, { seed: 1 })
    Surimono.send(:renderer_for, { seed: 200 })
    assert_same a, Surimono.send(:renderer_for, { seed: 1 })
    3.times { |i| Surimono.send(:renderer_for, { seed: 201 + i }) }
    assert_same a, Surimono.send(:renderer_for, { seed: 1 })
    e = assert_raises(Surimono::TimeLimitError) { Surimono.render(island('{"busy_ms": 500}'), limits: { time_ms: 10 }) }
    assert_equal "time-limit", e.kind
  end
end
