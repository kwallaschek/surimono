# frozen_string_literal: true

require "minitest/autorun"
require "pathname"
require "timeout"
require "tmpdir"
require "surimono"

# The tests render with a small bring-your-own bundle, "t": an empty SVG of
# the island's size, or, per its spec, a thrown error, a script over a limit,
# or the clock it sees.
class SurimonoTest < Minitest::Test
  TYPE = "application/vnd.surimono+json"
  REPO_FONT = File.expand_path("../../../fixtures/fonts/NotoSans-Regular.ttf", __dir__)
  BUNDLE = <<~JS
    function entry(specJson, ctx) {
      const spec = JSON.parse(specJson);
      if (spec.throw) throw new Error(spec.throw);
      if (spec.limit === "time") { for (;;) {} }
      if (spec.limit === "memory") { const a = []; for (;;) a.push(new Array(1e5).fill(1)); }
      if (spec.limit === "stack") { const f = (n) => f(n + 1) + 1; f(0); }
      if (spec.label) return `<svg xmlns="http://www.w3.org/2000/svg" width="80" height="20"><text x="0" y="15" font-family="sans-serif" font-size="12">${spec.label}</text></svg>`;
      if (spec.clock) return `<svg xmlns="http://www.w3.org/2000/svg"><title>t=${Date.now()} h=${new Date().getHours()}</title></svg>`;
      if (spec.measure) {
        let w = 0;
        for (let i = 0; i < spec.measure; i++) w += ctx.measureText("ab" + (i % 7), "12px Noto Sans").width;
        return `<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><title>w=${w.toFixed(1)}</title></svg>`;
      }
      return `<svg xmlns="http://www.w3.org/2000/svg" width="${ctx.width}" height="${ctx.height}"></svg>`;
    }
  JS
  BUNDLES = { "t" => { source: BUNDLE, entry: "entry" } }.freeze

  def island(spec = "{}", renderer: "t", attrs: "")
    %(<script type="#{TYPE}" data-renderer="#{renderer}" data-width="30" data-height="20"#{attrs}>#{spec}</script>)
  end

  def stub_svg
    '<div class="surimono-island" data-renderer="t" style="width:30px;height:20px;overflow:hidden">' \
      '<svg xmlns="http://www.w3.org/2000/svg" width="30" height="20"></svg></div>'
  end

  def font_bytes
    File.binread(REPO_FONT)
  end

  FONTS = [REPO_FONT].freeze

  # Surimono::Renderer.new with the fixture font unless fonts: is given.
  def new_renderer(**opts)
    Surimono::Renderer.new(fonts: FONTS, **opts)
  end

  def renderer(**opts)
    @renderer ||= {}
    @renderer[opts] ||= new_renderer(bundles: BUNDLES, pool_size: 2, **opts)
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
    assert_match(/\A<div class="surimono-island chart wide" id="q3" data-renderer="t" style="width:30px;height:20px;overflow:hidden">/, out)
  end

  def test_utf8_and_other_encodings
    html = "<p>売上 €</p>#{island}"
    assert_equal "<p>売上 €</p>#{stub_svg}", renderer.render(html)
    latin1 = "<p>café</p>".encode(Encoding::ISO_8859_1)
    assert_equal "<p>café</p>", renderer.render(latin1)
    assert_equal "<p>x</p>", renderer.render("<p>x</p>".b)
    e = assert_raises(ArgumentError) { renderer.render("<p>\xff</p>".b) }
    assert_match(/not valid UTF-8/, e.message)
  end

  def test_time_limit
    r = renderer(limits: { time_ms: 50 })
    e = assert_raises(Surimono::TimeLimitError) do
      r.render("<p>a</p>#{island('{"limit": "time"}', attrs: ' id="slow"')}")
    end
    assert_kind_of Surimono::Error, e
    assert_equal "time-limit", e.kind
    assert_equal 0, e.island_index
    assert_equal "slow", e.island_id
    assert_equal "t", e.renderer
    assert_match(/island #1 \(id "slow", renderer "t"\): the script exceeded the time limit/, e.message)
  end

  def test_memory_and_stack_limits
    r = renderer(limits: { memory_mb: 8 })
    e = assert_raises(Surimono::MemoryLimitError) { r.render(island('{"limit": "memory"}')) }
    assert_equal "memory-limit", e.kind
    e = assert_raises(Surimono::StackOverflowError) { r.render(island('{"limit": "stack"}')) }
    assert_equal "stack-overflow", e.kind
    assert_equal stub_svg, r.render(island), "the renderer works after a limit"
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
    assert_match(/\A<div class="surimono-island surimono-island-error" data-renderer="t" data-error="javascript" style="width:30px;height:20px"><!-- surimono: /, out)
    assert out.end_with?(stub_svg), out
  end

  def test_fonts_from_paths_and_data
    data = font_bytes
    Dir.mktmpdir do |dir|
      path = File.join(dir, "font.ttf")
      File.binwrite(path, data)
      r = Surimono::Renderer.new(fonts: [path, Pathname.new(path), data], bundles: BUNDLES)
      assert_equal stub_svg, r.render(island)
    end
    assert_raises(Errno::ENOENT) { Surimono::Renderer.new(fonts: ["/no/such/font.ttf"]) }
    e = assert_raises(Surimono::ConfigurationError) { Surimono::Renderer.new(fonts: [data, "not a font".b]) }
    assert_equal "font", e.kind
    assert_match(/font #2/, e.message)
    assert_raises(ArgumentError) { Surimono::Renderer.new(fonts: [42]) }
    e = assert_raises(ArgumentError) { Surimono::Renderer.new(fonts: [data.dup.force_encoding(Encoding::UTF_8)]) }
    assert_match(/File\.binread/, e.message)
  end

  def test_fonts_are_required
    e = assert_raises(Surimono::ConfigurationError) { Surimono::Renderer.new(bundles: BUNDLES) }
    assert_equal "invalid-options", e.kind
    assert_match(/no fonts/, e.message)
  end

  def test_option_checks
    [
      { limits: { time: 5 } }, { limits: { time_ms: 0 } }, { limits: { memory_mb: 1.5 } },
      { limits: { memory_mb: 2**50 } }, { limits: { stack_kb: 32 } }, { limits: "fast" },
      { seed: -1 }, { error_policy: :ignore }, { pool_size: 0 }, { pool_size: 2**70 }, { colour: :red },
      { renderers: "echarts" }, { renderers: [42] }, { bundles: { "mine" => 42 } },
      { bundles: { "mine" => { entry: "f" } } }, { now: "today" }, { utc_offset_minutes: 900 },
      { utc_offset_minutes: 1.5 }, { max_islands: 0 }, { max_islands: "many" }
    ].each do |opts|
      assert_raises(ArgumentError, opts.inspect) { new_renderer(**opts) }
    end
    assert_raises(ArgumentError) { renderer.render(nil) }
    assert_raises(ArgumentError) { renderer.render(42) }
    e = assert_raises(Surimono::ConfigurationError) { new_renderer(renderers: ["nope"]) }
    assert_equal "invalid-options", e.kind
    e = assert_raises(Surimono::ConfigurationError) do
      new_renderer(bundles: { "mine" => { source: "function f(){}", entry: "not a name" } })
    end
    assert_equal "bundle", e.kind
    e = assert_raises(Surimono::ConfigurationError) { new_renderer(bundles: { "mine" => "var x;" }) }
    assert_match(/globalThis\.entry/, e.message)
  end

  def test_renderers_and_bundles
    assert_equal "t", renderer.renderers.last
    assert_includes renderer.renderers, "echarts"
    only = new_renderer(bundles: { mine: "function entry(){}" }, renderers: [:mine])
    assert_equal ["mine"], only.renderers
    assert_raises(Surimono::RenderError) { only.render(island) }
  end

  def test_text_outlines_by_default_or_kept
    font = font_bytes
    outlined = Surimono::Renderer.new(fonts: [font], bundles: BUNDLES).render(island('{"label": "Sales"}'))
    refute_includes outlined, "<text"
    assert_includes outlined, "<path"
    assert_includes outlined, "Sales" # kept as the label of the outlines
    kept = Surimono::Renderer.new(fonts: [font], bundles: BUNDLES, text: :keep).render(island('{"label": "Sales"}'))
    assert_includes kept, "<text"
    assert_raises(ArgumentError) { new_renderer(text: :pixels) }
  end

  def test_clock_options
    r = new_renderer(bundles: BUNDLES, now: Time.utc(2023, 11, 14, 22, 13, 20), utc_offset_minutes: 540)
    out = r.render(island('{"clock": true}'))
    assert_includes out, "t=1700000000000 h=7"
    default = renderer.render(island('{"clock": true}'))
    assert_includes default, "t=1767225600000 h=0"
  end

  def test_other_threads_run_while_rendering
    r = renderer(limits: { time_ms: 600 }, error_policy: :placeholder)
    ticks = 0
    ticker = Thread.new do
      loop do
        ticks += 1
        sleep 0.005
      end
    end
    sleep 0.05 # let the ticker start
    before = ticks
    out = r.render(island('{"limit": "time"}'))
    during = ticks - before
    ticker.kill
    assert_includes out, 'data-error="time-limit"'
    # With the GVL held, the ticker couldn't run at all during the 600 ms render.
    assert_operator during, :>=, 20, "ticker advanced only #{during} times during the render"
  end

  def test_many_threads_share_one_renderer
    r = renderer
    html = "<main>#{island}</main>"
    outs = Array.new(8) { Thread.new { r.render(html) } }.map(&:value)
    assert_equal ["<main>#{stub_svg}</main>"], outs.uniq
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
    # Each render takes the 60 ms time limit and returns the document with a
    # placeholder: a 4 MB String that an interrupt used to leak.
    r = renderer(limits: { time_ms: 60 }, error_policy: :placeholder)
    html = "<p>#{"x" * (4 * 1024 * 1024)}</p>#{island('{"limit": "time"}')}"
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
    r = renderer(limits: { time_ms: 200 }, error_policy: :placeholder)
    t = Thread.new { r.render(island('{"limit": "time"}')) }
    sleep 0.05
    t.kill
    t.join
    refute t.alive?
    assert_equal stub_svg, r.render(island)
  end

  # Puma's preload_app! and Unicorn build objects in the parent and fork
  # workers. The runner's threads don't survive fork; the child must start
  # its own and render, never hang.
  def test_renderer_built_before_fork_renders_in_the_child
    skip "no fork on this platform" unless Process.respond_to?(:fork)

    r = renderer
    assert_equal stub_svg, r.render(island) # the parent's runner is running
    Surimono.render(island, fonts: FONTS, bundles: BUNDLES) # and a cached one
    reader, writer = IO.pipe
    pid = fork do
      reader.close
      out = [r.render(island), Surimono.render(island, fonts: FONTS, bundles: BUNDLES)]
      writer.write(out.join("\n"))
      writer.close
      exit!(0)
    rescue Exception => e # rubocop:disable Lint/RescueException
      writer.write("#{e.class}: #{e.message}")
      exit!(1)
    end
    writer.close
    status = nil
    200.times do
      _, status = Process.wait2(pid, Process::WNOHANG)
      break if status

      sleep 0.05
    end
    unless status
      Process.kill(:KILL, pid)
      Process.wait(pid)
      flunk "the forked child hung rendering"
    end
    got = reader.read
    assert status.success?, got
    assert_equal "#{stub_svg}\n#{stub_svg}", got
    assert_equal stub_svg, r.render(island), "the parent still renders"
  end

  # Runs the block in a forked child and returns what it wrote, failing the
  # test (and killing the child) if it doesn't finish within the deadline.
  def in_child(deadline: 30)
    reader, writer = IO.pipe
    pid = fork do
      reader.close
      writer.write(yield)
      writer.close
      exit!(0)
    rescue Exception => e # rubocop:disable Lint/RescueException
      writer.write("#{e.class}: #{e.message}")
      exit!(1)
    end
    writer.close
    status = nil
    give_up = Process.clock_gettime(Process::CLOCK_MONOTONIC) + deadline
    loop do
      _, status = Process.wait2(pid, Process::WNOHANG)
      break if status || Process.clock_gettime(Process::CLOCK_MONOTONIC) > give_up

      sleep 0.01
    end
    unless status
      Process.kill(:KILL, pid)
      Process.wait(pid)
      return nil
    end
    got = reader.read
    raise "child failed: #{got}" unless status.success?

    got
  ensure
    reader&.close
  end

  # Puma's fork_worker and Resque fork from a process that is busy rendering.
  # A forked child must never wait on a lock that one of the parent's other
  # threads held at that moment: before the fix, 18 of 200 children hung on
  # the text measurer's lock when the parent was measuring text.
  def test_fork_while_other_threads_measure_text
    skip "no fork on this platform" unless Process.respond_to?(:fork)

    r = new_renderer(bundles: BUNDLES, pool_size: 2, limits: { time_ms: 60_000 })
    small = island('{"measure": 3}')
    expected = r.render(small)
    stop = false
    busy = Array.new(3) do
      Thread.new { r.render(island('{"measure": 200000}')) until stop }
    end
    sleep 0.3
    hung = 0
    30.times do
      got = in_child(deadline: 20) { r.render(small) }
      if got.nil?
        hung += 1
      else
        assert_equal expected, got
      end
      sleep(rand * 0.05)
    end
    stop = true
    busy.each(&:join)
    assert_equal 0, hung, "#{hung} of 30 forked children hung"
    assert_equal expected, r.render(small), "the parent still renders"
  end

  # Puma's fork_worker forks workers from worker 0, itself a fork of the
  # master: a renderer follows forks of forks.
  def test_renderer_survives_nested_forks
    skip "no fork on this platform" unless Process.respond_to?(:fork)

    r = renderer
    assert_equal stub_svg, r.render(island)
    got = in_child do
      child = r.render(island)
      grandchild = in_child { r.render(island) } || "grandchild hung"
      "#{child}\n#{grandchild}"
    end
    refute_nil got, "the child hung"
    assert_equal "#{stub_svg}\n#{stub_svg}", got
  end

  def test_max_islands
    r = new_renderer(bundles: BUNDLES, max_islands: 2)
    assert_equal stub_svg * 2, r.render(island * 2)
    e = assert_raises(Surimono::RenderError) { r.render(island * 3) }
    assert_equal "too-many-islands", e.kind
    assert_nil e.island_index
    e = assert_raises(Surimono::RenderError) { new_renderer(bundles: BUNDLES).render(island * 1001) }
    assert_equal "too-many-islands", e.kind
  end

  # Twenty islands that each run into a 200 ms time limit: 4 s for the
  # document. An interrupt stops it before the next island.
  def slow_document
    island('{"limit": "time"}') * 20
  end

  def slow_renderer
    renderer(limits: { time_ms: 200 }, error_policy: :placeholder)
  end

  def elapsed
    started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    yield
    Process.clock_gettime(Process::CLOCK_MONOTONIC) - started
  end

  def test_timeout_cancels_the_rest_of_the_document
    r = slow_renderer
    took = elapsed do
      assert_raises(Timeout::Error) { Timeout.timeout(0.3) { r.render(slow_document) } }
    end
    assert_operator took, :<, 1.5, "Timeout waited #{took.round(2)} s for a 4 s document"
    assert_equal stub_svg, r.render(island), "the renderer still works"
  end

  def test_signal_cancels_the_rest_of_the_document
    skip "no SIGUSR2 on this platform" unless Signal.list.key?("USR2")

    r = slow_renderer
    previous = trap("USR2") { raise Stop, "signal" }
    begin
      sender = Thread.new do
        sleep 0.3
        Process.kill(:USR2, Process.pid)
      end
      took = elapsed { assert_raises(Stop) { r.render(slow_document) } }
      sender.join
      assert_operator took, :<, 1.5, "the signal waited #{took.round(2)} s for a 4 s document"
    ensure
      trap("USR2", previous)
    end
  end

  # An unblock without an exception to raise (Thread#wakeup) renders the
  # document again instead of failing it.
  def test_wakeup_renders_again
    r = renderer(limits: { time_ms: 100 }, error_policy: :placeholder)
    doc = island('{"limit": "time"}') * 3
    expected = r.render(doc)
    t = Thread.new { r.render(doc) }
    sleep 0.15
    t.wakeup if t.alive?
    assert_equal expected, t.value
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
        new_renderer(seed: 3)
      end
    end
    ready.pop
    t.raise(Stop, "stop")
    go = true
    e = assert_raises(Stop) { t.join }
    assert_equal "stop", e.message
  end

  # Widens the window in which the native side builds an island error (Ruby
  # code runs in Error#initialize) for threads that ask for it.
  module SlowErrorInit
    def initialize(*args, **details)
      if Thread.current[:surimono_slow_error]
        Thread.current[:surimono_in_error] = true
        sleep 0.5
      end
      super
    end
  end
  Surimono::TimeLimitError.prepend(SlowErrorInit)

  def thread_interrupted_while_building_the_error
    r = renderer(limits: { time_ms: 20 })
    t = Thread.new do
      Thread.current.report_on_exception = false
      Thread.current[:surimono_slow_error] = true
      begin
        r.render(island('{"limit": "time"}'))
      rescue StandardError => e
        Thread.current[:rescued] = e.class
      end
      Thread.current[:after] = true
    end
    give_up = Process.clock_gettime(Process::CLOCK_MONOTONIC) + 10
    Thread.pass until t[:surimono_in_error] || Process.clock_gettime(Process::CLOCK_MONOTONIC) > give_up
    assert t[:surimono_in_error], "the render never reached the error"
    yield t
    t
  end

  # An interrupt that arrives while the native side builds the exception
  # comes out as itself: Thread#raise's exception, not a RuntimeError, and
  # Thread#kill ends the thread instead of being swallowed.
  def test_interrupts_while_an_error_is_built_pass_through
    t = thread_interrupted_while_building_the_error { |th| th.raise(Stop, "stop") }
    t.join
    assert_equal Stop, t[:rescued]

    t = thread_interrupted_while_building_the_error(&:kill)
    assert t.join(10), "the killed thread kept running"
    assert_nil t[:rescued]
    assert_nil t[:after], "the thread went on after Thread#kill"
  end

  def test_panic_becomes_internal_error
    e = assert_raises(Surimono::InternalError) { Surimono.send(:_internal_panic_check) }
    assert_kind_of Surimono::Error, e
    assert_equal "internal", e.kind
    assert_match(/deliberate panic/, e.message)
    assert_raises(NoMethodError) { Surimono._internal_panic_check }
  end

  def test_module_render_reuses_a_renderer_per_option_set
    assert_equal stub_svg, Surimono.render(island, fonts: FONTS, bundles: BUNDLES)
    a = Surimono.send(:renderer_for, { fonts: FONTS, seed: 1 })
    b = Surimono.send(:renderer_for, { fonts: FONTS, seed: 1 })
    c = Surimono.send(:renderer_for, { fonts: FONTS, seed: 2 })
    assert_same a, b
    refute_same a, c
    20.times { |i| Surimono.send(:renderer_for, { fonts: FONTS, seed: 100 + i }) }
    assert_operator Surimono.instance_variable_get(:@renderers).size, :<=, 4
    # Least recently used goes first: seed 1, used again after seed 200 was
    # added, outlives it.
    a = Surimono.send(:renderer_for, { fonts: FONTS, seed: 1 })
    Surimono.send(:renderer_for, { fonts: FONTS, seed: 200 })
    assert_same a, Surimono.send(:renderer_for, { fonts: FONTS, seed: 1 })
    3.times { |i| Surimono.send(:renderer_for, { fonts: FONTS, seed: 201 + i }) }
    assert_same a, Surimono.send(:renderer_for, { fonts: FONTS, seed: 1 })
    e = assert_raises(Surimono::TimeLimitError) do
      Surimono.render(island('{"limit": "time"}'), fonts: FONTS, bundles: BUNDLES, limits: { time_ms: 10 })
    end
    assert_equal "time-limit", e.kind
  end
end
