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

  # An island's script used more memory than limits[:memory_mb] allows.
  class MemoryLimitError < Error; end
  # An island's script ran longer than limits[:time_ms] allows.
  class TimeLimitError < Error; end
  # An island's script recursed deeper than limits[:stack_kb] allows.
  class StackOverflowError < Error; end

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
  # unknown renderer, unusable renderer output, or markup too ambiguous to rewrite.
  class RenderError < Error; end
  # The renderer's options are invalid: a font that can't be read, a limit out
  # of range, an unknown renderer name or a bundle that can't be used.
  class ConfigurationError < Error; end
  # A bug in surimono itself (kind "internal"), e.g. a Rust panic during a
  # render. The process keeps running; please report it with the message.
  class InternalError < Error; end

  # Renders a document's islands. Build one per configuration and reuse it: it
  # keeps JavaScript runtimes with the renderers compiled, and it is safe to
  # share between threads. Rendering releases the GVL.
  class Renderer
    LIMIT_KEYS = %i[memory_mb time_ms stack_kb].freeze
    ERROR_POLICIES = %i[fail placeholder].freeze
    # Largest value accepted for a limit or pool_size, so every value fits the
    # native types; the native side checks the real ranges.
    MAX_OPTION = 2**31 - 1
    private_constant :LIMIT_KEYS, :ERROR_POLICIES, :MAX_OPTION

    # fonts::        font files for measuring text: paths (String or Pathname),
    #                or font data as binary Strings (e.g. from File.binread).
    #                Use the fonts your PDF engine prints with. System fonts are
    #                never used.
    # limits::       limits for each island (not the whole document): memory_mb
    #                (1 or more, default 64), time_ms (default 2000; loading
    #                the renderer and drawing each get this much), stack_kb
    #                (64 to 16384, default 512). A document with N islands can
    #                take up to N times time_ms.
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
    # pool_size::    JavaScript runtimes (threads) kept for concurrent renders
    #                (default: CPU count, at most 4; at most 64).
    #
    # A Renderer survives fork: built before Puma's preload_app! or Unicorn
    # forks, it starts its runtimes again in each worker on first use.
    #
    # Bad argument types or values raise ArgumentError; a font path that
    # can't be read raises the usual Errno error; options the native side
    # rejects (a limit out of range, an unknown renderer name, a font or
    # bundle that can't be used) raise ConfigurationError.
    def self.new(fonts: [], limits: {}, seed: 0, error_policy: :fail, renderers: nil, bundles: {}, pool_size: nil,
                 now: nil, utc_offset_minutes: 0)
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
        normalize_offset(utc_offset_minutes)
      )
    end

    # Runs what this version of surimono runs on a document, as enabled by the
    # options, and returns the new HTML (UTF-8). In 0.1 that is the islands:
    # every island marker is replaced with its SVG, and everything else is
    # left byte for byte as it was. (Page scripts come in a later version, off
    # until you turn them on.) Raises a Surimono::Error subclass when an island
    # fails (with error_policy: :fail).
    #
    # html in another encoding is transcoded to UTF-8 first; a
    # <meta charset> in it is left as written, so declare UTF-8 (or drop the
    # declaration) if the result goes to a renderer that reads it.
    def render(html)
      raise ArgumentError, "html must be a String, got #{html.class}" unless html.respond_to?(:to_str)

      _render(Renderer.send(:utf8, html.to_str))
    end

    private :_render

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
    def render(html, **options)
      renderer_for(options).render(html)
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
