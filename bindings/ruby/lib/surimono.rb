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
#
#   renderer = Surimono::Renderer.new(fonts: ["app/assets/fonts/NotoSans-Regular.ttf"])
#   html = renderer.render(html)   # island markers replaced by SVG
#   html = renderer.render(html, scripts: true)   # the page's scripts run first
#
# Island markers look like this:
#
#   <script type="application/vnd.surimono+json" data-renderer="echarts"
#           data-width="600" data-height="400">{ ...chart spec... }</script>
module Surimono
  # Base class of every error surimono raises. Island errors say which island
  # failed: #island_index (from 0), #island_id and #renderer.
  class Error < StandardError
    # Stable kebab-case name of the error kind, e.g. "time-limit".
    attr_reader :kind
    # Position of the failed island among the document's islands, from 0.
    attr_reader :island_index
    # The failed island's id attribute, if it has one.
    attr_reader :island_id
    # The failed island's data-renderer attribute.
    attr_reader :renderer

    def initialize(message = nil, kind: nil, island_index: nil, island_id: nil, renderer: nil)
      super(message)
      @kind = kind
      @island_index = island_index
      @island_id = island_id
      @renderer = renderer
    end
  end

  # A script used more memory than limits[:memory_mb] allows.
  class MemoryLimitError < Error; end
  # A script ran longer than limits[:time_ms] allows.
  class TimeLimitError < Error; end
  # A script recursed deeper than limits[:stack_kb] allows.
  class StackOverflowError < Error; end
  # A page or its scripts passed a limit on the DOM: nodes, tree depth, node
  # handles, bytes of text or the size of a style attribute.
  class DomLimitError < Error; end

  # An island's script threw. Named JavaScriptError because Ruby already has
  # a top-level ::ScriptError.
  class JavaScriptError < Error
    # The JavaScript stack trace, if the engine gave one.
    attr_reader :js_stack

    def initialize(message = nil, js_stack: nil, **details)
      super(message, **details)
      @js_stack = js_stack
    end
  end

  # An island couldn't be rendered for another reason: a malformed marker, an
  # unknown renderer, unusable renderer output, more islands than max_islands
  # (kind "too-many-islands"), or markup too ambiguous to rewrite. Or a page
  # couldn't run (kind "page"), though a page's own exception isn't this: see
  # PageReport#uncaught.
  class RenderError < Error; end
  # The renderer's options are invalid: a font that can't be read, a limit out
  # of range, an unknown renderer name or a bundle that can't be used.
  class ConfigurationError < Error; end
  # A bug in surimono itself (kind "internal"), e.g. a Rust panic during a
  # render. The process keeps running; please report it with the message.
  class InternalError < Error; end

  # What happened while a page ran, from Renderer#render_page. A page with an
  # uncaught exception still renders, as it would in a browser.
  #
  # settled::              why it stopped: :idle (nothing left), :virtual_budget
  #                        (timers left after 10 s of virtual time) or
  #                        :task_limit (100,000 tasks, with more due)
  # uncaught::             exceptions nothing caught
  # errors::               the first few, as messages
  # unhandled_rejections:: promises rejected with no handler
  # rejections::           the first few reasons, as messages
  # failed_count::         external scripts and images the page didn't get
  # failed::               the first few, as { kind: :script or :image, url:,
  #                        reason: } Hashes
  PageReport = Struct.new(:settled, :uncaught, :errors, :unhandled_rejections, :rejections, :failed_count,
                          :failed, keyword_init: true)

  # A rendered page: its html, and the PageReport as report.
  Page = Struct.new(:html, :report, keyword_init: true)

  # Renders a document's islands, and its scripts when asked. Build one per configuration and reuse it: it
  # keeps JavaScript runtimes with the renderers compiled, and it is safe to
  # share between threads. Rendering releases the GVL.
  class Renderer
    LIMIT_KEYS = %i[memory_mb time_ms stack_kb].freeze
    ERROR_POLICIES = %i[fail placeholder].freeze
    LAYOUT_QUERIES = %i[zeros throw].freeze
    # Largest value accepted for a limit or pool_size, so every value fits the
    # native types; the native side checks the real ranges.
    MAX_OPTION = 2**31 - 1
    private_constant :LIMIT_KEYS, :ERROR_POLICIES, :LAYOUT_QUERIES, :MAX_OPTION

    # fonts::        required, at least one: font files for measuring and
    #                outlining text, as paths (String or Pathname) or font data
    #                as binary Strings (e.g. from File.binread). Use the fonts
    #                your PDF engine prints with. System fonts are never used.
    # limits::       limits for each island (not the whole document): memory_mb
    #                (1 or more, default 64), time_ms (default 2000; one
    #                deadline per island covering loading the renderer and
    #                drawing), stack_kb (64 to 16384, default 512). A document
    #                with N islands can take up to N times time_ms.
    # max_islands::  most islands one document may have (default 1000); a
    #                document with more raises RenderError (kind
    #                "too-many-islands") before any island runs. Together with
    #                time_ms it bounds how long one document can take.
    # seed::         seed for Math.random; same seed and input, same output.
    # now::          the Time every script sees as "now" (Date.now(), new
    #                Date()). Default: 2026-01-01 00:00:00 UTC. The host's
    #                clock is never read.
    # utc_offset_minutes:: local time's offset from UTC for scripts, e.g. 540
    #                for Japan, so time axes show local time. -720 to 840.
    #                Default: 0 (UTC). The host's time zone is never read.
    # error_policy:: :fail (default) raises on the first failed island;
    #                :placeholder leaves an empty
    #                <div class="surimono-island surimono-island-error"> instead.
    # renderers::    enable only these renderer names (default: all).
    # bundles::      your own renderers: { "name" => { source: js, entry: "entry" } }
    #                (or { "name" => js }). The script defines a global function
    #                (default name "entry"; with a bundler, assign it to
    #                globalThis.entry) called as entry(specJson, ctx) for each
    #                island: specJson is the spec as JSON text (JSON.parse it),
    #                ctx has width, height and measureText(text, cssFont). It
    #                returns an SVG string, which is sanitised before use.
    # text::         :outlines (default) turns chart text into outlines of its
    #                glyphs in your fonts, so any PDF engine prints exactly what
    #                was measured (the words stay as aria-label, but can't be
    #                selected in the PDF; needs at least one font). :keep leaves
    #                <text>, for engines that set inline SVG text in your fonts
    #                (fulgur 0.40 doesn't).
    # pool_size::    JavaScript runtimes (threads) kept for concurrent renders
    #                (default: CPU count, at most 4; at most 64).
    # layout_queries:: with page scripts, what getBoundingClientRect, offsetWidth
    #                and the like give, as nothing is laid out: :zeros (default)
    #                or :throw (a NotSupportedError in the script).
    # keep_scripts:: with page scripts, leave <script> elements in the result.
    #                Default false: all but island markers are removed.
    # resources::    with page scripts, what <script src> may load, as
    #                { "url as written in the page" => source }. Only these load:
    #                no network and no files otherwise. Limits apply to a page's
    #                scripts as one, not per script.
    #
    # A Renderer survives fork: built before Puma's preload_app! (or
    # fork_worker), Unicorn or Resque forks, it starts its runtimes and text
    # measurer and compiles your bundles again in each child on first use.
    #
    # Interrupts (Timeout.timeout, Rack::Timeout, Thread#raise, Thread#kill,
    # SIGTERM) stop a render, its running island included, and raise as usual.
    # On Ruby 3.2, a render on the main thread while other threads exist runs a
    # sleeping helper thread, "surimono-signal-pump", for its length, so that
    # signals reach it; Ruby 3.3 and later need none.
    #
    # An interrupt doesn't stop a bundle while it loads; it takes effect right
    # after, so that render can outlast your timeout. This matters on the first
    # render in a forked child, which compiles every bundle: each can take
    # twice time_ms (a compile, then a check run), and one too large to compile
    # within time_ms takes as long as its size needs before it fails. The
    # built-in renderers are precompiled and load quickly.
    #
    # Bad argument types or values raise ArgumentError; a font path that
    # can't be read raises the usual Errno error; options the native side
    # rejects (a limit out of range, an unknown renderer name, a font or
    # bundle that can't be used) raise ConfigurationError.
    def self.new(fonts: [], limits: {}, seed: 0, error_policy: :fail, renderers: nil, bundles: {}, pool_size: nil,
                 now: nil, utc_offset_minutes: 0, text: :outlines, max_islands: nil,
                 layout_queries: :zeros, keep_scripts: false, resources: {})
      lim = normalize_limits(limits)
      _native_new(
        Array(fonts).map { |f| font_data(f) },
        lim[:memory_mb] * 1024 * 1024,
        lim[:time_ms],
        lim[:stack_kb] * 1024,
        non_negative_integer(seed, "seed"),
        normalize_policy(error_policy),
        normalize_renderers(renderers),
        normalize_bundles(bundles),
        pool_size && positive_integer(pool_size, "pool_size"),
        normalize_now(now),
        normalize_offset(utc_offset_minutes),
        normalize_text(text),
        max_islands && positive_integer(max_islands, "max_islands"),
        [keep_scripts ? true : false, normalize_layout_queries(layout_queries), normalize_resources(resources)]
      )
    end

    # Renders a document and returns the new HTML (UTF-8). Every island marker is
    # replaced with its SVG and everything else is left byte for byte as it was.
    # The document's own scripts don't run unless you pass scripts: true: then
    # this is render_page(html).html. Raises a Surimono::Error subclass when an
    # island fails (with error_policy: :fail) or a limit is reached.
    #
    # html in another encoding is transcoded to UTF-8 first; a
    # <meta charset> in it is left as written, so declare UTF-8 (or drop the
    # declaration) if the result goes to a renderer that reads it.
    def render(html, scripts: false)
      return render_page(html).html if scripts

      with_html(html) { |doc| _render(doc) }
    end

    # Runs the document's own scripts as a browser would, on a virtual clock
    # until the page settles, then renders its islands, and returns a Page with
    # the html and a PageReport. The html is the document serialized: without
    # <noscript>, and without <script> unless keep_scripts: true. Each render
    # gets a new document and script state. Nothing is laid out and images
    # aren't loaded; an external script needs resources:.
    #
    # An exception in a page doesn't raise, it goes in the report. A limit does:
    # MemoryLimitError, TimeLimitError, StackOverflowError or DomLimitError.
    #
    # An interrupt raises at once, but a page already running goes on, unobserved,
    # until its time limit, taking one of pool_size's runtimes until then.
    def render_page(html)
      out, report = with_html(html) { |doc| _render_page(doc) }
      Page.new(html: out, report: PageReport.new(**report))
    end

    def with_html(html)
      raise ArgumentError, "html must be a String, got #{html.class}" unless html.respond_to?(:to_str)

      doc = Renderer.send(:utf8, html.to_str)
      return yield(doc) unless SIGNAL_PUMP && Thread.current.equal?(Thread.main) && Thread.list.size > 1

      with_signal_pump { yield(doc) }
    end

    # Ruby 3.2 passes a signal on to a render on the main thread only through
    # a Ruby thread that drains its signal pipe: a helper it starts itself when
    # the main thread is the only thread, or another thread that happens to go
    # to sleep. With other threads alive and none of them sleeping at the right
    # moment, the signal waited until the whole document was rendered. A thread
    # that sleeps in short steps for the length of the render drains it. Ruby
    # 3.3 and later have a timer thread for this. (P1-PKG-20)
    SIGNAL_PUMP = RUBY_ENGINE == "ruby" && RUBY_VERSION.start_with?("3.2.")
    SIGNAL_PUMP_INTERVAL = 0.05
    private_constant :SIGNAL_PUMP, :SIGNAL_PUMP_INTERVAL

    # Runs the block with the signal pump. The pump only sleeps; it stops on its
    # own once `done` is set, so even an interrupt that lands between
    # Thread.new and the assignment can't leave it running, and the ensure
    # wakes and joins it so it never outlives the render. It is a convenience:
    # an exception raised into it is dropped (pump.join would raise it out of
    # render, after the whole document), and with no thread to be had the
    # render goes ahead without it, so signals wait as they would on any
    # blocking C call.
    def with_signal_pump
      done = false
      pump = nil
      begin
        begin
          pump = Thread.new do
            begin
              sleep SIGNAL_PUMP_INTERVAL until done
            rescue Exception # rubocop:disable Lint/RescueException
              retry # Thread#raise from outside; Thread#kill can't be rescued
            end
          end
        rescue ThreadError
          pump = nil # a thread limit, or a frozen ThreadGroup
        end
        if pump
          # From here, not from its block: Ruby runs a new thread only once
          # this one lets it, so it never shows up unnamed.
          pump.name = "surimono-signal-pump"
          pump.report_on_exception = false
        end
        yield
      ensure
        done = true
        if pump
          begin
            pump.wakeup
          rescue ThreadError
            nil # already finished
          end
          pump.join
        end
      end
    end

    private :_render, :_render_page, :with_html, :with_signal_pump

    class << self
      private

      def font_data(font)
        return File.binread(font.to_path) if font.respond_to?(:to_path)
        raise ArgumentError, "fonts must be paths or binary Strings of font data, got #{font.class}" unless font.is_a?(String)
        return font if font.encoding == Encoding::BINARY

        if font.include?("\0") || font.bytesize > 4096
          raise ArgumentError, "a font String in #{font.encoding} looks like font data read with File.read; " \
                               "read it with File.binread (binary Strings are font data, others are paths)"
        end
        File.binread(font)
      end

      def normalize_limits(limits)
        raise ArgumentError, "limits must be a Hash, got #{limits.class}" unless limits.is_a?(Hash)

        unknown = limits.keys.map(&:to_sym) - LIMIT_KEYS
        raise ArgumentError, "unknown limits: #{unknown.join(", ")} (known: #{LIMIT_KEYS.join(", ")})" unless unknown.empty?

        given = limits.to_h { |k, v| [k.to_sym, positive_integer(v, "limits[:#{k}]")] }
        if given.key?(:stack_kb) && !(64..16_384).cover?(given[:stack_kb])
          raise ArgumentError, "limits[:stack_kb] must be from 64 to 16384, got #{given[:stack_kb]}"
        end
        { memory_mb: 64, time_ms: 2000, stack_kb: 512 }.merge(given)
      end

      def normalize_now(now)
        return nil if now.nil?
        raise ArgumentError, "now must be a Time, got #{now.class}" unless now.is_a?(Time)

        (now.to_r * 1000).floor
      end

      def normalize_text(text)
        return text.to_s if %i[outlines keep].include?(text.to_s.to_sym)

        raise ArgumentError, "text must be :outlines or :keep, got #{text.inspect}"
      end

      def normalize_layout_queries(queries)
        return queries.to_s if LAYOUT_QUERIES.include?(queries.to_s.to_sym)

        raise ArgumentError, "layout_queries must be :zeros or :throw, got #{queries.inspect}"
      end

      # [[url, source], ...]; source as bytes, whatever its encoding says.
      def normalize_resources(resources)
        raise ArgumentError, "resources must be a Hash, got #{resources.class}" unless resources.respond_to?(:to_hash)

        resources.to_hash.map do |url, source|
          raise ArgumentError, "a resource's source must be a String, got #{source.class}" unless source.is_a?(String)

          [String(url), source]
        end
      end

      def normalize_offset(minutes)
        return minutes if minutes.is_a?(Integer) && (-720..840).cover?(minutes)

        raise ArgumentError, "utc_offset_minutes must be an Integer from -720 to 840, got #{minutes.inspect}"
      end

      def normalize_policy(policy)
        sym = policy.to_s.to_sym
        unless ERROR_POLICIES.include?(sym)
          raise ArgumentError, "error_policy must be :fail or :placeholder, got #{policy.inspect}"
        end

        sym.to_s
      end

      def normalize_renderers(renderers)
        return nil if renderers.nil?
        raise ArgumentError, "renderers must be an Array of names, got #{renderers.class}" unless renderers.is_a?(Array)

        renderers.map do |r|
          raise ArgumentError, "renderer names must be Strings or Symbols, got #{r.inspect}" unless r.is_a?(String) || r.is_a?(Symbol)

          r.to_s
        end
      end

      def normalize_bundles(bundles)
        raise ArgumentError, "bundles must be a Hash, got #{bundles.class}" unless bundles.is_a?(Hash)

        bundles.map do |name, spec|
          spec = { source: spec } if spec.is_a?(String)
          raise ArgumentError, "bundle #{name.inspect} must be a Hash or a String of source, got #{spec.class}" unless spec.is_a?(Hash)

          source = spec[:source] || spec["source"]
          entry = spec[:entry] || spec["entry"] || "entry"
          raise ArgumentError, "bundle #{name.inspect} needs a :source" unless source.is_a?(String)

          [name.to_s, utf8(source), entry.to_s]
        end
      end

      def positive_integer(value, name)
        return value if value.is_a?(Integer) && value.positive? && value <= MAX_OPTION

        raise ArgumentError, "#{name} must be an Integer from 1 to #{MAX_OPTION}, got #{value.inspect}"
      end

      def non_negative_integer(value, name)
        return value if value.is_a?(Integer) && value >= 0 && value < 2**64

        raise ArgumentError, "#{name} must be an Integer from 0 to 2**64-1, got #{value.inspect}"
      end

      def utf8(str)
        return str if str.encoding == Encoding::UTF_8 && str.valid_encoding?

        if [Encoding::BINARY, Encoding::UTF_8].include?(str.encoding)
          converted = str.dup.force_encoding(Encoding::UTF_8)
          return converted if converted.valid_encoding?

          raise ArgumentError, "HTML (#{str.encoding}) is not valid UTF-8; pass it in its real encoding or as UTF-8"
        end
        begin
          str.encode(Encoding::UTF_8)
        rescue EncodingError => e
          raise ArgumentError, "HTML in #{str.encoding} can't be converted to UTF-8: #{e.message}"
        end
      end
    end
  end

  ##
  # :singleton-method: builtin_renderers
  # The built-in renderers in this native extension, sorted:
  # ["echarts", "vega", "vega-lite"]. Defined by the extension.

  ##
  # :singleton-method: precompiled_builtins
  # How many of Surimono.builtin_renderers load precompiled bytecode in this
  # process: all of them, unless SURIMONO_PRECOMPILE=0 was set when the gem
  # was built or when the process started (then 0), or the bytecode doesn't
  # fit the engine (a broken build). Output is the same either way; only
  # each built-in's first render in a process is slower. For diagnostics and
  # packaging checks. Defined by the extension.

  # Renderers Surimono.render keeps, least recently used dropped first. Each
  # one holds a pool of JavaScript runtimes with the renderers compiled, so the
  # cache is small; build Renderer objects yourself for more control.
  RENDERER_CACHE_SIZE = 4
  private_constant :RENDERER_CACHE_SIZE
  @renderers = {}
  @renderers_lock = Mutex.new

  class << self
    # Renders html's islands with a renderer for these options, built on first
    # use and reused afterwards. Takes the same options as Renderer.new.
    #
    #   Surimono.render(html, fonts: ["fonts/NotoSans-Regular.ttf"])
    #
    # Up to 4 option sets are kept, least recently used dropped first. Options
    # are compared by value: font paths as written (a font file changed on disk
    # is not read again by a cached renderer), font data by its bytes, which
    # costs a hash of those bytes per call. For many configurations or large
    # font data, build a Renderer once and keep it.
    #
    # scripts: true runs the document's own scripts first: see
    # Renderer#render_page.
    def render(html, scripts: false, **options)
      renderer_for(options).render(html, scripts: scripts)
    end

    private

    def renderer_for(options)
      key = options.hash
      @renderers_lock.synchronize do
        cached = @renderers[key]
        if cached && cached[0] == options
          # Most recently used goes last.
          @renderers.delete(key)
          @renderers[key] = cached
          return cached[1]
        end

        renderer = Renderer.new(**options)
        @renderers.delete(@renderers.keys.first) while @renderers.size >= RENDERER_CACHE_SIZE
        @renderers[key] = [options.dup.freeze, renderer]
        renderer
      end
    end
  end
  private_class_method :_internal_panic_check
end
