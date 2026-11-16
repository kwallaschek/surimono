# frozen_string_literal: true

require_relative "../surimono"

module Surimono
  # A loader for a Rails app's own stylesheets and scripts, for
  # Renderer#render_page. Loaded only by require "surimono/rails"; it needs no
  # Rails, Propshaft or Sprockets of its own and loads none of them. It asks the
  # app object it is given, Rails.application by default.
  #
  #   require "surimono/rails"
  #   page = renderer.render_page(html, **Surimono::Rails.assets)
  #
  # A <link rel="stylesheet"> that stylesheet_link_tag wrote then goes into the
  # page as a <style> with the bytes the app's own server gives for that URL, so
  # the printer-safe output can resolve its print media, and a <script src> that
  # javascript_include_tag wrote runs. See Surimono::Rails.assets.
  module Rails
    # Most bytes a single file may have unless max_bytes: says otherwise. A
    # Tailwind or esbuild development build is a few MiB; this bounds what a
    # hostile page can make one load read, while the renderer's memory_mb
    # bounds all of a page's sources together.
    DEFAULT_MAX_BYTES = 16 * 1024 * 1024

    # Only these are read: what stylesheet_link_tag and javascript_include_tag
    # write. A page can't read an app's other public files (a frame loads
    # through the loader too) with it.
    EXTENSIONS = %w[.css .js .mjs].freeze
    # A path segment the loader reads: no dot first (so no "." or "..", and no
    # dotfile such as .manifest.json), no escape, no backslash, no NUL, no
    # colon, nothing outside ASCII. Digested asset names fit it.
    SEGMENT = /\A[A-Za-z0-9_~+@-][A-Za-z0-9_.~+@-]*\z/
    MAX_PATH = 1024
    # Rails' asset_host "assets%d.example.com" spreads assets over these four
    # (ActionView's compute_asset_host takes crc32(source) % 4).
    HOST_SPREAD = 0..3
    # Entries per loader in its cache of answers.
    CACHE_ENTRIES = 1000
    private_constant :EXTENSIONS, :SEGMENT, :MAX_PATH, :HOST_SPREAD, :CACHE_ENTRIES

    class << self
      # The loader: and allow: for Renderer#render_page (or Surimono.render_page)
      # that load the app's own stylesheets and scripts, as a Hash:
      #
      #   page = renderer.render_page(html, **Surimono::Rails.assets)
      #
      # It answers the URLs stylesheet_link_tag and javascript_include_tag write
      # under Propshaft and under Sprockets (sprockets-rails): a path under
      # config.assets.prefix ("/assets"), behind config.relative_url_root, and
      # with the app's asset_host (config.asset_host or
      # config.action_controller.asset_host) in each form Rails writes it
      # ("//host", "http://host", "https://host", or the host as configured
      # when that has a scheme; "%d" in it as 0 to 3). A query or fragment is
      # ignored. And the files in public/ that aren't assets
      # (stylesheet_link_tag "/stylesheets/print.css").
      #
      # For a URL it gives what the app's own server would: a file under
      # public/ first (a precompiled asset, as ActionDispatch::Static serves
      # it), then, where the app compiles assets live (Propshaft's
      # config.assets.server, on in development and test; Sprockets'
      # config.assets.compile), the compiled asset for that digest: CSS from
      # Sass, with url()s rewritten, as the pipeline gives it. Anything else,
      # including a digest that isn't the asset's, gives nil, which the page
      # report lists.
      #
      # It reads only .css, .js and .mjs files, only inside public/ and the
      # pipeline's own asset paths (no "..", escape, backslash, NUL or
      # absolute path; a symlink that leads out of them is refused), never from
      # the network, and only for URLs of the app's own asset host. A page
      # chooses the URLs, so this matters.
      #
      # app::       the Rails application (default Rails.application), or any
      #             object shaped like it: config, paths["public"], assets.
      # hosts::     more asset hosts, written as config.asset_host is. Needed
      #             when asset_host is a proc, which can't be read ahead.
      # max_bytes:: most bytes one file may have (default 16 MiB); a larger
      #             one gives nil.
      # also::      your own { loader:, allow: } for other URLs: a URL the app's
      #             list allows goes to the app's files first and, if they
      #             have nothing, to yours if your list allows it.
      #
      # Call it for each render: the loader keeps what it read, so one page
      # sees the same bytes for a URL, and a new one sees files changed since.
      # An @import or url() in a sheet isn't followed.
      def assets(app: nil, hosts: [], max_bytes: DEFAULT_MAX_BYTES, also: nil)
        app ||= default_app
        unless max_bytes.is_a?(Integer) && max_bytes.positive?
          raise ArgumentError, "max_bytes must be a positive Integer, got #{max_bytes.inspect}"
        end
        raise ArgumentError, "hosts must be an Array of Strings" unless hosts.is_a?(Array) && hosts.all?(String)

        other = normalize_also(also)
        loader = AssetLoader.new(app, hosts, max_bytes, other)
        { loader: loader, allow: loader.allow }
      end

      private

      def default_app
        app = defined?(::Rails) && ::Rails.respond_to?(:application) && ::Rails.application
        raise ArgumentError, "no Rails.application: pass app:" unless app

        app
      end

      def normalize_also(also)
        return nil if also.nil?
        raise ArgumentError, "also must be a Hash with :loader and :allow" unless also.is_a?(Hash)

        loader = also[:loader]
        allow = also[:allow] || []
        raise ArgumentError, "also[:loader] must respond to call" unless loader.respond_to?(:call)
        raise ArgumentError, "also[:allow] must be an Array of Strings" unless allow.is_a?(Array) && allow.all?(String)

        [loader, allow.dup.freeze]
      end
    end

    # Whether an allowlist entry list lets url through, as render_page's own
    # allowlist does, or more strictly: an entry equal to it, or one ending in
    # "/" for what is under it, unless what follows has a segment of only dots
    # and spaces, an empty segment, a ";", a backslash, a control character, a
    # ":" in its first segment or a "%".
    def self.allows?(entries, url)
      entries.any? do |entry|
        next true if url == entry
        next false unless entry.end_with?("/") && url.start_with?(entry)

        tail = url[entry.size..]
        next false if tail.match?(/[\\;%[:cntrl:]]/) || tail.split("/", 2).first.include?(":")

        segments = tail.split("/", -1)
        segments.pop if segments.size > 1 && segments.last.empty?
        segments.none? { |s| s.match?(/\A[. ]*\z/) }
      end
    end
    private_class_method :allows?

    # The loader Surimono::Rails.assets gives. Private: use assets.
    class AssetLoader
      attr_reader :allow

      def initialize(app, hosts, max_bytes, other)
        @max_bytes = max_bytes
        @other = other
        config = app.config
        assets_config = option(config, :assets)
        @prefix = normalize_root(option(assets_config, :prefix) || "/assets")
        # Longest first: "" (a path) must not take "//cdn.example.com/...".
        @bases = bases(config, hosts).sort_by { |b| [-b.size, b] }.freeze
        @public = public_dir(app)
        @pipeline = pipeline(app, assets_config)
        @cache = {}
        own = @bases.flat_map { |base| own_entries(base) }.uniq
        @own = own.freeze
        @allow = (own + (other ? other[1] : [])).uniq.freeze
      end

      # The bytes for url, or nil.
      def call(url)
        url = String(url)
        if Rails.send(:allows?, @own, url)
          found = cached(url) { load(url) }
          return found if found
        end
        return @other[0].call(url) if @other && Rails.send(:allows?, @other[1], url)

        nil
      end

      def inspect
        "#<Surimono::Rails::AssetLoader allow=#{@allow.inspect}>"
      end

      private

      def option(obj, name)
        obj.public_send(name) if obj.respond_to?(name)
      end

      # "/app/" or "app" to "/app"; "/" or "" to "".
      def normalize_root(root)
        root = String(root).sub(%r{/+\z}, "")
        root.empty? || root.start_with?("/") ? root : "/#{root}"
      end

      # What comes before the path in a URL Rails writes: "" (path only) and
      # each form of each asset host, all behind relative_url_root.
      def bases(config, extra_hosts)
        controller = option(config, :action_controller)
        root = normalize_root(option(controller, :relative_url_root) || option(config, :relative_url_root) || "")
        configured = option(controller, :asset_host) || option(config, :asset_host)
        hosts = (configured.is_a?(String) ? [configured] : []) + extra_hosts
        forms = hosts.flat_map { |h| host_forms(h) }
        (["" + root] + forms.map { |f| f + root }).uniq
      end

      def host_forms(host)
        host = host.strip.sub(%r{/+\z}, "")
        return [] if host.empty?

        spread = host.include?("%d") ? HOST_SPREAD.map { |n| host.gsub("%d", n.to_s) } : [host]
        spread.flat_map do |h|
          # ActionView's URI_REGEXP: a scheme, or protocol-relative, as written.
          if h.match?(%r{\A[-a-z]+://|\A//}i)
            [h]
          else
            ["//#{h}", "http://#{h}", "https://#{h}"]
          end
        end
      end

      def public_dir(app)
        paths = option(app, :paths)
        dir = paths && paths["public"]
        dir = dir.first if dir.respond_to?(:first) && !dir.is_a?(String)
        dir ||= (root = option(app, :root)) && File.join(root.to_s, "public")
        dir && real_dir(dir.to_s)
      end

      def real_dir(dir)
        File.realpath(dir).then { |r| File.directory?(r) ? r : nil }
      rescue SystemCallError
        nil
      end

      # [:propshaft, assembly, real load paths] or [:sprockets, env, real
      # paths] when the app compiles assets live, else nil.
      def pipeline(app, assets_config)
        assets = option(app, :assets)
        return nil if assets.nil?

        if assets.respond_to?(:load_path) && assets.respond_to?(:config)
          return nil unless option(assets_config, :server)

          if option(assets.config, :sweep_cache)
            sweeper = assets.load_path.cache_sweeper
            sweeper.execute_if_updated
          end
          [:propshaft, assets, assets.load_path.paths.filter_map { |p| real_dir(p.to_s) }]
        elsif assets.respond_to?(:find_asset) && assets.respond_to?(:paths)
          return nil if option(assets_config, :compile) == false

          [:sprockets, assets, assets.paths.filter_map { |p| real_dir(p.to_s) }]
        end
      end

      # The allowlist entries for one base: the asset prefix, and each
      # top-level directory and .css/.js file of public/ (public/assets is the
      # prefix's own).
      def own_entries(base)
        entries = ["#{base}#{@prefix}/"]
        if @public
          Dir.children(@public).sort.each do |name|
            next unless name.match?(SEGMENT)

            path = File.join(@public, name)
            if File.directory?(path)
              entries << "#{base}/#{name}/"
            elsif EXTENSIONS.include?(File.extname(name))
              entries << "#{base}/#{name}"
            end
          end
        end
        entries
      rescue SystemCallError
        entries
      end

      def cached(url)
        return @cache[url] if @cache.key?(url)

        found = yield
        @cache[url] = found if @cache.size < CACHE_ENTRIES
        found
      end

      def load(url)
        path = path_of(url)
        return nil unless path

        from_public(path) || from_pipeline(path)
      end

      # The URL's path ("/assets/a-1.css") once its base is off, or nil.
      def path_of(url)
        url = url.sub(/[?#].*\z/m, "")
        return nil if url.bytesize > MAX_PATH

        @bases.each do |base|
          next unless url.start_with?("#{base}/")

          path = url[base.size..]
          return valid_path?(path) ? path : nil
        end
        nil
      end

      def valid_path?(path)
        segments = path.split("/", -1)
        segments.shift == "" && !segments.empty? && segments.all? { |s| s.match?(SEGMENT) } &&
          EXTENSIONS.include?(File.extname(segments.last))
      end

      def from_public(path)
        @public && read_inside(File.join(@public, path), [@public])
      end

      def from_pipeline(path)
        return nil unless @pipeline && (@prefix.empty? || path.start_with?("#{@prefix}/"))

        logical = path[(@prefix.size + 1)..]
        kind, assets, roots = @pipeline
        kind == :propshaft ? from_propshaft(assets, logical, roots) : from_sprockets(assets, logical, roots)
      end

      # As Propshaft::Server serves it: the asset for the logical path, if the
      # URL's digest is its digest, compiled.
      def from_propshaft(assembly, digested, roots)
        logical, digest = ::Propshaft::Asset.extract_path_and_digest(digested)
        asset = assembly.load_path.find(logical)
        return nil unless asset&.fresh?(digest) && inside?(asset.path.to_s, roots)
        return nil if File.size(asset.path.to_s) > @max_bytes

        bounded(asset.compiled_content)
      rescue StandardError
        nil
      end

      # As Sprockets::Server#call serves it (Sprockets is MIT-licensed, by Sam
      # Stephenson and Joshua Peek): the fingerprint taken off, the asset found
      # for the rest or else for the whole name, and served only if the
      # fingerprint is its etag.
      def from_sprockets(env, full, roots)
        fingerprint = full[/-([0-9a-zA-Z]{7,128})\.[^.]+\z/, 1]
        asset = env.find_asset(fingerprint ? full.sub("-#{fingerprint}", "") : full)
        if asset.nil? && (asset = env.find_asset(full))
          fingerprint = asset.etag
        end
        return nil if asset.nil? || (fingerprint && asset.etag != fingerprint)
        return nil unless inside?(asset.filename.to_s, roots)

        bounded(asset.to_s)
      rescue StandardError
        nil
      end

      def bounded(bytes)
        bytes = String(bytes)
        bytes.bytesize > @max_bytes ? nil : bytes
      end

      # Whether file, with every symlink resolved, is inside one of roots.
      def inside?(file, roots)
        real = File.realpath(file)
        roots.any? { |root| real.start_with?("#{root}/") }
      rescue SystemCallError
        false
      end

      # A regular file's bytes, read only if it is inside roots and isn't over
      # max_bytes, or nil.
      def read_inside(file, roots)
        return nil unless inside?(file, roots)

        real = File.realpath(file)
        flags = File::RDONLY | File::NONBLOCK
        flags |= File::NOFOLLOW if defined?(File::NOFOLLOW)
        File.open(real, flags) do |f|
          # NONBLOCK so a FIFO put there can't hold the render; only a
          # regular file is read.
          return nil unless f.stat.file? && f.stat.size <= @max_bytes

          f.binmode
          bytes = f.read(@max_bytes + 1) || +""
          bytes.bytesize > @max_bytes ? nil : bytes
        end
      rescue SystemCallError, IOError
        nil
      end
    end
    private_constant :AssetLoader
  end
end
