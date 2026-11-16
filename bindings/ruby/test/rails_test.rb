# frozen_string_literal: true

require "minitest/autorun"
require "minitest/mock"
require "fileutils"
require "open3"
require "pathname"
require "rbconfig"
require "timeout"
require "tmpdir"
require "surimono/rails"

# Surimono::Rails.assets without Rails: the app objects are fakes, shaped as the
# real ones were measured on 16 Nov 2026 with Ruby 3.3.6, railties,
# actionpack and actionview 8.1.4.1 (config.load_defaults 8.1), propshaft
# 1.3.2, sprockets 4.4.1 with sprockets-rails 3.5.2 and sassc-embedded 1.80.10,
# by booting a small Rails::Application of each kind and calling its helpers:
#
# - Rails.application.config: assets.prefix "/assets"; relative_url_root and
#   action_controller.relative_url_root both "/app" when it is set; asset_host
#   and action_controller.asset_host both the string set. paths["public"].first
#   is public/. All are ActiveSupport::OrderedOptions, which answer nil for an
#   unset key (FakeOptions below).
# - stylesheet_link_tag "application" writes "/assets/application-9e8b4424.css"
#   (Propshaft, 8 hex digits) or "/assets/application-<64 hex>.css"
#   (Sprockets), "/app/assets/..." behind relative_url_root, and with asset_host
#   "cdn.example.com": "//cdn.example.com/assets/..." outside a request,
#   "http://..." or "https://..." in one (the request's protocol); a host
#   written with a scheme or as "//host" stays as written, a path in it too
#   ("http://cdn.example.com:8080/sub/assets/..."); "assets%d.example.com"
#   becomes assets0 to assets3. A query in the source stays ("...css?v=1").
#   "/stylesheets/print.css" (public/, not an asset) gets the root and host the
#   same way. Sprockets with config.assets.debug writes
#   "/assets/application.debug-<64 hex>.css".
# - Propshaft: Rails.application.assets is a Propshaft::Assembly whose
#   config.server is true in development (and test) and nil otherwise, with
#   config.sweep_cache, load_path.paths (Pathnames), load_path.find(logical)
#   giving a Propshaft::Asset (path a Pathname, fresh?(digest),
#   compiled_content with url()s digested) and load_path.cache_sweeper.
#   Propshaft::Asset.extract_path_and_digest("application-9e8b4424.css") is
#   ["application.css", "9e8b4424"]. assets:precompile writes
#   public/assets/application-9e8b4424.css and .manifest.json.
# - Sprockets: Rails.application.assets is a Sprockets::Environment (nil when
#   config.assets.compile is false) with paths (Strings) and find_asset(logical)
#   giving a Sprockets::Asset with filename (the source, theme.scss for Sass),
#   etag (the 64-hex digest in the URL) and to_s (compiled CSS).
#   Precompiled files are public/assets/<digest name> and
#   .sprockets-manifest-<hex>.json.
#
# The middleware stack is ActionDispatch::Static, then Propshaft::Server (when
# config.assets.server), then ActionDispatch::Executor: a file in public/ wins
# over the live pipeline, as the loader has it. test_against_real_rails at the
# end repeats the main cases against the real gems when they are installed.
# Propshaft::Asset.extract_path_and_digest as Propshaft 1.3.2 has it (MIT,
# Copyright (c) 2021 Basecamp), which the loader calls to split a digested name.
unless defined?(Propshaft::Asset)
  module Propshaft
    class Asset
      def self.extract_path_and_digest(digested_path)
        digest = digested_path[/-([0-9a-zA-Z]{7,128})\.(?!digested)([^.]|.map)+\z/, 1]
        path   = digest ? digested_path.sub("-#{digest}", "") : digested_path

        [path, digest]
      end
    end
  end
end

class RailsAssetsTest < Minitest::Test
  REPO_FONT = File.expand_path("../../../fixtures/fonts/NotoSans-Regular.ttf", __dir__)

  # ActiveSupport::OrderedOptions as far as the loader uses it.
  class FakeOptions < Hash
    def method_missing(name, *args)
      key = name.to_s
      return self[key.chomp("=").to_sym] = args[0] if key.end_with?("=")

      self[name]
    end

    def respond_to_missing?(*)
      true
    end
  end

  def options(**values)
    FakeOptions.new.merge!(values)
  end

  FakePropshaftAsset = Struct.new(:path, :digest, :compiled_content) do
    def fresh?(given)
      digest == given
    end
  end

  class FakeLoadPath
    attr_reader :paths, :swept

    def initialize(paths, assets)
      @paths = paths.map { |p| Pathname.new(p) }
      @assets = assets
      @swept = 0
    end

    def find(logical)
      @assets[logical]
    end

    def cache_sweeper
      self
    end

    def execute_if_updated
      @swept += 1
    end
  end

  FakeAssembly = Struct.new(:config, :load_path)

  FakeSprocketsAsset = Struct.new(:filename, :etag, :source) do
    def to_s
      raise source if source.is_a?(Exception)

      source
    end
  end

  # Sprockets::Environment: resolve(logical) gives [uri or nil, deps] without
  # compiling, parse_asset_uri(uri) gives [filename, params], find_asset
  # compiles (4.4.1, measured).
  class FakeSprockets
    attr_reader :paths, :compiled

    def initialize(paths, assets, sources = {})
      @paths = paths
      @assets = assets
      @sources = sources
      @compiled = []
    end

    def resolve(path)
      found = @assets[path]
      return [nil, []] if found.nil?

      filename = @sources[path] || (found.is_a?(Exception) ? File.join(@paths.first, path) : found.filename)
      ["file://#{filename}?type=text/css", []]
    end

    def parse_asset_uri(uri)
      [uri[%r{\Afile://([^?]*)}, 1], {}]
    end

    def find_asset(path)
      @compiled << path
      found = @assets[path]
      raise found if found.is_a?(Exception)

      found
    end
  end

  FakeApp = Struct.new(:config, :paths, :assets)

  CSS = "@media print { .a { color: red } }\n@media screen { .a { color: blue } }\n.b { color: green }\n"
  DIGEST = "9e8b4424"
  SHA = "ad7a25c4828229ae7e9f3d126cd73bbe8de225958669027ec4df501a7c849e7b"

  def setup
    @tmp = Dir.mktmpdir("surimono-rails")
    @root = File.join(@tmp, "app")
    write("public/stylesheets/print.css", ".p { color: black }\n")
    write("public/404.html", "<p>gone</p>")
    write("public/robots.txt", "User-agent: *")
    write("app/assets/stylesheets/application.css", CSS)
    write("app/assets/javascripts/application.js", "document.title = 'ran';")
    write("config/credentials.yml.enc", "secret")
    write("config/secret.css", ".secret{}")
    File.write(File.join(@tmp, "outside.css"), ".outside{}")
  end

  def teardown
    FileUtils.rm_rf(@tmp)
  end

  def write(rel, body)
    path = File.join(@root, rel)
    FileUtils.mkdir_p(File.dirname(path))
    File.binwrite(path, body)
    path
  end

  def app_path(rel)
    File.join(@root, rel)
  end

  # A Propshaft app in development: the live pipeline serves
  # application-9e8b4424.css (compiled with a marker, so tests can tell it from
  # the file) and application-d5305eb1.js.
  def propshaft_app(server: true, asset_host: nil, relative_url_root: nil, controller: {}, assets: nil)
    stylesheets = app_path("app/assets/stylesheets")
    javascripts = app_path("app/assets/javascripts")
    assets ||= {
      "application.css" => FakePropshaftAsset.new(Pathname.new(File.join(stylesheets, "application.css")), DIGEST,
                                                  "/* compiled */\n#{CSS}"),
      "application.js" => FakePropshaftAsset.new(Pathname.new(File.join(javascripts, "application.js")), "d5305eb1",
                                                 "document.title = 'ran';"),
    }
    assembly = FakeAssembly.new(options(server: server, sweep_cache: server), FakeLoadPath.new([javascripts, stylesheets], assets))
    config = options(assets: options(prefix: "/assets", server: server), asset_host: asset_host,
                     relative_url_root: relative_url_root, action_controller: options(**controller))
    FakeApp.new(config, { "public" => [app_path("public")] }, assembly)
  end

  def sprockets_app(assets: nil, compile: true)
    stylesheets = app_path("app/assets/stylesheets")
    assets ||= {
      "application.css" => FakeSprocketsAsset.new(File.join(stylesheets, "application.css"), SHA, "/* sprockets */\n#{CSS}"),
    }
    env = compile ? FakeSprockets.new([stylesheets], assets) : nil
    config = options(assets: options(prefix: "/assets", compile: compile, digest: true), action_controller: options)
    FakeApp.new(config, { "public" => [app_path("public")] }, env)
  end

  # The lib directory this process loaded surimono from (the repository's, or
  # the installed gem's under gem:verify), for a Ruby of its own.
  def loaded_lib
    File.dirname($LOADED_FEATURES.grep(%r{/surimono\.rb\z}).first)
  end

  def loader(app, **opts)
    Surimono::Rails.assets(app: app, **opts)
  end

  def load(app, url, **opts)
    loader(app, **opts)[:loader].call(url)
  end

  # Hostile first: a page chooses the URLs.

  def test_paths_that_climb_out_are_refused
    write("public/assets/ok.css", ".ok{}")
    app = propshaft_app
    [
      "/assets/../config/secret.css", "/assets/../../outside.css", "/assets/./ok.css", "/assets/sub/../ok.css",
      "/assets/%2e%2e/config/secret.css", "/assets/..%2fconfig%2fsecret.css", "/assets/%2E%2E/x.css",
      "/assets/%252e%252e/x.css", "/assets/..\\config\\secret.css", "/assets\\..\\config\\secret.css",
      "/assets/ok.css\0.js", "/assets/ok\0.css", "/assets//ok.css", "//assets/ok.css", "/assets/ok.css/",
      "/assets/. /x.css", "/assets/.. /x.css", "/assets/ok.css ", "/assets/\u00a0.css", "/assets/ｏｋ.css",
      "/assets/．．/config/secret.css", "/assets/;/ok.css", "/assets/C:/x.css", "C:\\x.css",
      File.join(@root, "config/secret.css"), "file://#{@root}/config/secret.css", "/#{@root}/config/secret.css",
      "../config/secret.css", "config/secret.css", "assets/ok.css", "~/x.css", "/assets/~/x.css",
    ].each do |url|
      assert_nil load(app, url), url.inspect
    end
    assert_equal ".ok{}", load(app, "/assets/ok.css")
  end

  def test_symlinks_out_of_the_roots_are_refused
    FileUtils.mkdir_p(app_path("public/assets"))
    File.symlink(File.join(@tmp, "outside.css"), app_path("public/assets/link.css"))
    File.symlink(@tmp, app_path("public/up"))
    File.symlink(app_path("config"), app_path("public/assets/conf"))
    File.symlink(app_path("public/stylesheets/print.css"), app_path("public/assets/inside.css"))
    link = write("app/assets/stylesheets/linked.css", "")
    File.delete(link)
    File.symlink(File.join(@tmp, "outside.css"), link)
    assets = { "linked.css" => FakePropshaftAsset.new(Pathname.new(link), DIGEST, ".outside{}") }
    app = propshaft_app(assets: assets)
    assert_nil load(app, "/assets/link.css")
    assert_nil load(app, "/up/outside.css")
    assert_nil load(app, "/assets/conf/secret.css")
    assert_nil load(app, "/assets/linked-#{DIGEST}.css")
    # A symlink that stays inside is the file it names.
    assert_equal ".p { color: black }\n", load(app, "/assets/inside.css")
    # Sprockets likewise.
    env_assets = { "linked.css" => FakeSprocketsAsset.new(link, SHA, ".outside{}") }
    assert_nil load(sprockets_app(assets: env_assets), "/assets/linked-#{SHA}.css")
  end

  def test_other_hosts_are_refused_and_nothing_is_fetched
    write("public/assets/ok.css", ".ok{}")
    app = propshaft_app(asset_host: "https://cdn.example.com")
    [
      "https://evil.example.com/assets/ok.css", "http://cdn.example.com/assets/ok.css", "//cdn.example.com/assets/ok.css",
      "https://cdn.example.com.evil.com/assets/ok.css", "https://cdn.example.com@evil.com/assets/ok.css",
      "https://cdn.example.com:443/assets/ok.css", "HTTPS://CDN.EXAMPLE.COM/assets/ok.css", "https://cdn.example.com",
      "https://cdn.example.com/", "https:/cdn.example.com/assets/ok.css", "//evil.example.com/assets/ok.css",
      "data:text/css,.x{}", "javascript:alert(1)",
    ].each do |url|
      assert_nil load(app, url), url
    end
    assert_equal ".ok{}", load(app, "https://cdn.example.com/assets/ok.css")
    assets = Surimono::Rails.assets(app: app)
    refute(assets[:allow].any? { |e| e.include?("evil") })
    # No host configured: only paths.
    plain = propshaft_app
    assert_nil load(plain, "//cdn.example.com/assets/ok.css")
    assert_nil load(plain, "http://localhost/assets/ok.css")
  end

  def test_only_stylesheets_and_scripts_are_read
    write("public/assets/data.json", "{}")
    write("public/assets/.manifest.json", "{}")
    write("public/assets/logo.png", "PNG")
    app = propshaft_app
    %w[/404.html /robots.txt /assets/data.json /assets/.manifest.json /assets/logo.png /stylesheets
       /assets/application.css.map /stylesheets/print.CSS].each do |url|
      assert_nil load(app, url), url
    end
  end

  def test_a_fifo_or_a_device_doesnt_hold_the_render
    FileUtils.mkdir_p(app_path("public/assets"))
    File.mkfifo(app_path("public/assets/fifo.css"))
    File.symlink("/dev/zero", app_path("public/assets/zero.css"))
    Timeout.timeout(5) do
      assert_nil load(propshaft_app, "/assets/fifo.css")
      assert_nil load(propshaft_app, "/assets/zero.css")
    end
  end

  def test_one_file_is_bounded
    write("public/assets/big.css", "a" * 100)
    app = propshaft_app
    assert_nil load(app, "/assets/big.css", max_bytes: 99)
    assert_equal "a" * 100, load(app, "/assets/big.css", max_bytes: 100)
    assert_nil load(app, "/assets/application-#{DIGEST}.css", max_bytes: 10)
    assert_nil load(sprockets_app, "/assets/application-#{SHA}.css", max_bytes: 10)
    assert_raises(ArgumentError) { loader(app, max_bytes: 0) }
    assert_raises(ArgumentError) { loader(app, max_bytes: "1") }
  end

  def test_unknown_urls_give_nil_and_never_raise
    app = propshaft_app(asset_host: "assets%d.example.com", relative_url_root: "/app")
    write("app/assets/stylesheets/boom.css", "$x")
    broken = { "boom.css" => RuntimeError.new("Sass syntax error") }
    sprockets = sprockets_app(assets: broken)
    ["", "/", "/assets", "/assets/", "/assets/nope-#{DIGEST}.css", "/assets/application.css",
     "/assets/application-00000000.css", "/assets/application-#{DIGEST}.js", "x" * 5000 + ".css",
     "/assets/#{"a/" * 600}x.css", "\u{1F600}", "/assets/\u00e9.css", "?", "#", "/app", "/app/",
     "//", "///", "http://", "%", "/assets/%", "/assets/%zz.css"].each do |url|
      assert_nil load(app, url), url.inspect
      assert_nil load(sprockets, url), url.inspect
    end
    assert_nil load(sprockets, "/assets/boom-#{SHA}.css")
    assert_nil load(sprockets, "/assets/boom.css")
  end

  def cache_of(assets)
    loader = assets[:loader]
    [loader.instance_variable_get(:@cache), loader.instance_variable_get(:@held)]
  end

  # Counts File.open calls on paths under the app while the block runs.
  def counting_opens
    opens = []
    real_open = File.method(:open)
    root = File.realpath(@root)
    File.stub(:open, ->(path, *args, **kw, &blk) {
      opens << path if path.to_s.start_with?(root)
      real_open.call(path, *args, **kw, &blk)
    }) { yield opens }
  end

  def test_every_spelling_of_a_file_shares_one_answer
    write("public/assets/app.css", "a" * 1024 * 1024)
    assets = loader(propshaft_app(asset_host: "cdn.example.com"))
    counting_opens do |opens|
      1000.times do |i|
        url = ["/assets/app.css?#{i}", "//cdn.example.com/assets/app.css##{i}", "https://cdn.example.com/assets/app.css?v=#{i}#x"][i % 3]
        assert_equal 1024 * 1024, assets[:loader].call(url).bytesize
      end
      assert_equal 1, opens.size, "one read of the file"
    end
    cache, held = cache_of(assets)
    assert_equal 1, cache.size
    assert_equal 1024 * 1024, held
  end

  def test_a_page_of_query_variants_holds_one_copy
    write("public/assets/app.css", ".a{}/*#{"x" * (1024 * 1024)}*/")
    assets = loader(propshaft_app)
    links = (1..50).map { |i| %(<link rel="stylesheet" href="/assets/app.css?#{i}">) }.join
    renderer = Surimono::Renderer.new(fonts: [REPO_FONT])
    begin
      renderer.render_page("<!doctype html><title>t</title>#{links}", **assets)
    rescue Surimono::Error
      nil # 50 copies in the page may pass memory_mb; the loader's own memory is the point
    end
    cache, held = cache_of(assets)
    assert_equal 1, cache.size
    assert_operator held, :<=, 1024 * 1024 + 16
  end

  def test_the_cache_is_bounded_by_bytes_and_never_gives_two_answers
    (1..5).each { |i| write("public/assets/f#{i}.css", i.to_s * 100) }
    assets = loader(propshaft_app, max_bytes: 100)
    (1..4).each { |i| assert_equal i.to_s * 100, assets[:loader].call("/assets/f#{i}.css") }
    assert_nil assets[:loader].call("/assets/f5.css"), "past four files of max_bytes"
    assert_nil assets[:loader].call("/assets/f5.css?again"), "and again: one answer per path"
    assert_equal "1" * 100, assets[:loader].call("/assets/f1.css?kept")
    _cache, held = cache_of(assets)
    assert_equal 400, held
  end

  def test_misses_dont_grow_the_cache
    write("public/assets/late.css", ".late{}")
    assets = loader(propshaft_app)
    100_000.times { |i| assert_nil assets[:loader].call("/assets/miss-#{i}.css") }
    100.times { |i| assert_nil assets[:loader].call("/assets/../x#{i}.css") }
    cache, held = cache_of(assets)
    assert_equal 1000, cache.size
    assert_equal 0, held
    # Past 1,000 paths a new one isn't read, now or later.
    assert_nil assets[:loader].call("/assets/late.css")
    assert_nil assets[:loader].call("/assets/late.css")
    assert_equal ".late{}", load(propshaft_app, "/assets/late.css")
  end

  def test_dotfiles_and_names_outside_ascii_are_refused
    write("public/assets/.hidden.css", ".hidden{}")
    write("public/assets/..x.css", ".dots{}")
    write("public/assets/.x/y.css", ".dotdir{}")
    write("public/assets/ünï.css", ".unicode{}")
    write("public/assets/café.css", ".cafe{}")
    write("public/assets/a b.css", ".space{}")
    write("public/.well-known/x.css", ".wk{}")
    write("public/ü/x.css", ".udir{}")
    write("public/a b/x.css", ".sdir{}")
    write("public/assets/ok.css", ".ok{}")
    app = propshaft_app
    %w[/assets/.hidden.css /assets/..x.css /assets/.x/y.css /assets/ünï.css /assets/café.css /.well-known/x.css /ü/x.css].each do |url|
      assert_nil load(app, url), url
    end
    assert_nil load(app, "/assets/a b.css")
    assert_nil load(app, "/a b/x.css")
    assert_equal ".ok{}", load(app, "/assets/ok.css")
    allow = loader(app)[:allow]
    assert_equal ["/assets/", "/stylesheets/"], allow
  end

  def test_the_allowlist_rule
    allows = ->(entries, url) { Surimono::Rails.send(:allows?, entries, url) }
    assert allows.(["/r/x.css"], "/r/x.css")
    refute allows.(["/r/x.css"], "/r/x.css2")
    refute allows.(["/r"], "/r/x.css"), "only an entry ending in / is a prefix"
    assert allows.(["/r/"], "/r/a/b.css")
    assert allows.(["/r/"], "/r/a/")
    {
      "%" => ["/r/%2e%2e/x.css", "/r/a%41.css", "/r/%"],
      ":" => ["/r/c:/x.css", "/r/http:/x.css"],
      "dots" => ["/r/../x.css", "/r/./x.css", "/r/.. /x.css", "/r/a/../../x.css", "/r/..", "/r/ /x.css"],
      "empty" => ["/r//x.css", "/r/a//x.css"],
      "other" => ["/r/a\\..\\x.css", "/r/a;b.css", "/r/a\0.css", "/r/a\n.css"],
    }.each do |rule, urls|
      urls.each { |url| refute allows.(["/r/"], url), "#{rule}: #{url.inspect}" }
    end
    assert allows.(["/r/"], "/r/a/b:c.css"), "a : past the first segment is a name"
    assert allows.(["/r/"], "/r/a.b.css")
    # also: is guarded by it when the loader is called directly.
    called = []
    assets = loader(propshaft_app, also: { loader: ->(u) { called << u; "x" }, allow: ["/reports/"] })
    %w[/reports/%2e%2e/config/secret.css /reports/c:/x.css /reports/../x.css].each do |url|
      assert_nil assets[:loader].call(url), url
    end
    assert_empty called
  end

  def test_a_file_swapped_between_check_and_open_isnt_read
    target = write("public/assets/swap.css", ".checked{}")
    other = write("public/assets/other.css", ".swapped-in{}")
    real_open = File.method(:open)
    flags_seen = []
    swap = lambda do |&change|
      done = false
      ->(path, *args, **kw, &blk) {
        if !done && path.to_s.end_with?("swap.css")
          done = true
          flags_seen << args[0]
          change.call
        end
        real_open.call(path, *args, **kw, &blk)
      }
    end
    # Another regular file: not the one checked.
    File.stub(:open, swap.call { File.rename(other, target) }) do
      assert_nil load(propshaft_app, "/assets/swap.css")
    end
    # A symlink out of the roots: O_NOFOLLOW.
    write("public/assets/swap.css", ".checked{}")
    File.stub(:open, swap.call { File.delete(target); File.symlink(File.join(@tmp, "outside.css"), target) }) do
      assert_nil load(propshaft_app, "/assets/swap.css")
    end
    # A FIFO: O_NONBLOCK, so it can't hold the render.
    File.delete(target)
    write("public/assets/swap.css", ".checked{}")
    File.stub(:open, swap.call { File.delete(target); File.mkfifo(target) }) do
      Timeout.timeout(5) { assert_nil load(propshaft_app, "/assets/swap.css") }
    end
    flags_seen.each do |flags|
      assert_equal File::NONBLOCK, flags & File::NONBLOCK
      assert_equal File::NOFOLLOW, flags & File::NOFOLLOW if defined?(File::NOFOLLOW)
    end
    assert_equal 3, flags_seen.size
  end

  def test_propshaft_sizes_and_errors
    big = write("app/assets/stylesheets/big.css", "a" * 200)
    broken = write("app/assets/stylesheets/broken.css", ".x{}")
    failing = FakePropshaftAsset.new(Pathname.new(broken), DIGEST, nil)
    def failing.compiled_content = raise(ArgumentError, "a compiler failed")
    assets = {
      "big.css" => FakePropshaftAsset.new(Pathname.new(big), DIGEST, "small"),
      "broken.css" => failing,
    }
    app = propshaft_app(assets: assets)
    assert_nil load(app, "/assets/big-#{DIGEST}.css", max_bytes: 100), "the source is over max_bytes"
    assert_equal "small", load(app, "/assets/big-#{DIGEST}.css", max_bytes: 200)
    assert_nil load(app, "/assets/broken-#{DIGEST}.css")
  end

  def test_sprockets_compiles_only_inside_its_paths
    FileUtils.mkdir_p(File.join(@tmp, "out"))
    File.write(File.join(@tmp, "out/run.css.erb"), ".x{}")
    File.symlink(File.join(@tmp, "out"), app_path("app/assets/stylesheets/out"))
    assets = { "out/run.css" => FakeSprocketsAsset.new(app_path("app/assets/stylesheets/out/run.css.erb"), SHA, ".x{}") }
    app = sprockets_app(assets: assets)
    assert_nil load(app, "/assets/out/run-#{SHA}.css")
    assert_nil load(app, "/assets/out/run.css")
    assert_empty app.assets.compiled, "nothing compiled for a source outside the paths"
    big = write("app/assets/stylesheets/big.scss", "a" * 200)
    app = sprockets_app(assets: { "big.css" => FakeSprocketsAsset.new(big, SHA, "small") })
    assert_nil load(app, "/assets/big-#{SHA}.css", max_bytes: 100)
    assert_empty app.assets.compiled, "nothing compiled for a source over max_bytes"
    assert_equal "small", load(app, "/assets/big-#{SHA}.css", max_bytes: 200)
  end

  def test_compiled_bytes_over_max_bytes_are_refused
    small = write("app/assets/stylesheets/small.css", "a")
    propshaft = propshaft_app(assets: { "small.css" => FakePropshaftAsset.new(Pathname.new(small), DIGEST, "b" * 200) })
    assert_nil load(propshaft, "/assets/small-#{DIGEST}.css", max_bytes: 100)
    assert_equal "b" * 200, load(propshaft, "/assets/small-#{DIGEST}.css", max_bytes: 200)
    sprockets = sprockets_app(assets: { "small.css" => FakeSprocketsAsset.new(small, SHA, "b" * 200) })
    assert_nil load(sprockets, "/assets/small-#{SHA}.css", max_bytes: 100)
  end

  def test_sprockets_checks_the_compiled_assets_file_too
    inside = write("app/assets/stylesheets/a.css", ".a{}")
    env = FakeSprockets.new([app_path("app/assets/stylesheets")],
                            { "a.css" => FakeSprocketsAsset.new(File.join(@tmp, "outside.css"), SHA, ".outside{}") },
                            { "a.css" => inside })
    app = sprockets_app
    app.assets = env
    assert_nil load(app, "/assets/a-#{SHA}.css")
  end

  def test_a_long_path_is_refused
    long = (["d" * 200] * 6).join("/")
    write("public/assets/#{long}/x.css", ".deep{}")
    write("public/assets/#{(["d" * 200] * 4).join("/")}/x.css", ".deep4{}")
    assert_nil load(propshaft_app, "/assets/#{long}/x.css")
    assert_equal ".deep4{}", load(propshaft_app, "/assets/#{(["d" * 200] * 4).join("/")}/x.css")
  end

  def test_any_url_value_gives_nil_or_bytes_never_an_exception
    write("public/assets/ok.css", ".ok{}")
    app = propshaft_app
    assets = loader(app, also: { loader: ->(_) { "mine" }, allow: ["/r/"] })
    ["/assets/\xff.css".b, "/assets/\xff.css".dup.force_encoding(Encoding::UTF_8), "/r/\xfe".b,
     "/assets/ok.css".encode(Encoding::UTF_16LE).force_encoding(Encoding::UTF_8),
     "\xff\xfe".dup.force_encoding(Encoding::UTF_16LE), "/\xd8".dup.force_encoding(Encoding::UTF_16LE),
     "\xff".dup.force_encoding(Encoding::UTF_16BE), nil, 1, :sym, Object.new, BasicObject.new].each do |url|
      assert_nil assets[:loader].call(url), Kernel.instance_method(:inspect).bind_call(url)
    end
    assert_equal ".ok{}", assets[:loader].call("/assets/ok.css".encode(Encoding::UTF_16LE))
    assert_equal ".ok{}", assets[:loader].call("/assets/ok.css".b)
    assert_equal "mine", assets[:loader].call("/r/x.css".encode(Encoding::UTF_16LE))
  end

  def test_the_default_app_is_the_top_level_rails
    script = <<~RUBY
      require "surimono/rails"
      Dir.mktmpdir do |dir|
        Dir.mkdir(File.join(dir, "public"))
        File.write(File.join(dir, "public", "x.css"), ".top{}")
        config = Struct.new(:assets, :action_controller).new(nil, nil)
        app = Struct.new(:config, :paths, :assets).new(config, { "public" => [File.join(dir, "public")] }, nil)
        Object.const_set(:Rails, Module.new { define_singleton_method(:application) { app } })
        module Surimono
          # Code reopening Surimono sees Surimono::Rails for a bare Rails.
          SEEN = Rails
        end
        p [Surimono::Rails.assets[:loader].call("/x.css"), Surimono::SEEN == Surimono::Rails]
      end
    RUBY
    out, status = Open3.capture2e(RbConfig.ruby, "-I", loaded_lib, "-rtmpdir", "-e", script)
    assert status.success?, out
    assert_equal %([".top{}", true]), out.strip
  end

  # What Rails writes.

  def test_propshaft_in_development
    app = propshaft_app
    assert_equal "/* compiled */\n#{CSS}", load(app, "/assets/application-#{DIGEST}.css")
    assert_equal "document.title = 'ran';", load(app, "/assets/application-d5305eb1.js")
    assert_nil load(app, "/assets/application.css"), "Propshaft serves no undigested name"
    assert_nil load(app, "/assets/application-0badbeef.css"), "nor another digest"
    assert_equal "/* compiled */\n#{CSS}", load(app, "/assets/application-#{DIGEST}.css?v=1#top")
    assert_equal ".p { color: black }\n", load(app, "/stylesheets/print.css")
    swept = app.assets.load_path.swept
    loader(app)
    assert_equal swept + 1, app.assets.load_path.swept, "each loader runs the cache sweeper, as Propshaft::Server does"
  end

  def test_a_precompiled_file_wins_and_production_has_no_live_pipeline
    write("public/assets/application-#{DIGEST}.css", "/* precompiled */")
    write("public/assets/.manifest.json", "{}")
    assert_equal "/* precompiled */", load(propshaft_app, "/assets/application-#{DIGEST}.css")
    production = propshaft_app(server: nil)
    assert_equal "/* precompiled */", load(production, "/assets/application-#{DIGEST}.css")
    assert_nil load(production, "/assets/application-d5305eb1.js")
    assert_equal 0, production.assets.load_path.swept
    # Sprockets with config.assets.compile false has no environment at all.
    write("public/assets/application-#{SHA}.css", "/* sprockets precompiled */")
    off = sprockets_app(compile: false)
    assert_nil off.assets
    assert_equal "/* sprockets precompiled */", load(off, "/assets/application-#{SHA}.css")
    assert_nil load(off, "/assets/other-#{SHA}.css")
  end

  def test_sprockets_in_development
    sass = write("app/assets/stylesheets/theme.scss", "$c: red;")
    stylesheets = app_path("app/assets/stylesheets")
    assets = {
      "application.css" => FakeSprocketsAsset.new(File.join(stylesheets, "application.css"), SHA, "/* app */"),
      "application.debug.css" => FakeSprocketsAsset.new(File.join(stylesheets, "application.css"), "d" * 64, "/* debug */"),
      "theme.css" => FakeSprocketsAsset.new(sass, "e" * 64, "@media print {\n  .t {\n    color: red;\n  }\n}\n"),
      "bundle-0123456789.js" => FakeSprocketsAsset.new(File.join(stylesheets, "application.css"), "f" * 64, "/* hashed */"),
    }
    app = sprockets_app(assets: assets)
    assert_equal "/* app */", load(app, "/assets/application-#{SHA}.css")
    assert_equal "/* app */", load(app, "/assets/application.css"), "Sprockets serves an undigested name"
    assert_nil load(app, "/assets/application-#{"0" * 64}.css")
    assert_equal "/* debug */", load(app, "/assets/application.debug-#{"d" * 64}.css")
    assert_equal "@media print {\n  .t {\n    color: red;\n  }\n}\n", load(app, "/assets/theme-#{"e" * 64}.css")
    # A name a bundler already hashed is found whole, as Sprockets::Server does.
    assert_equal "/* hashed */", load(app, "/assets/bundle-0123456789.js")
  end

  def test_relative_url_root_and_asset_hosts
    url = "/assets/application-#{DIGEST}.css"
    compiled = "/* compiled */\n#{CSS}"
    rooted = propshaft_app(relative_url_root: "/app")
    assert_equal compiled, load(rooted, "/app#{url}")
    assert_equal ".p { color: black }\n", load(rooted, "/app/stylesheets/print.css")
    assert_nil load(rooted, url)
    assert_includes loader(rooted)[:allow], "/app/assets/"

    plain_host = propshaft_app(asset_host: "cdn.example.com", relative_url_root: "/app")
    ["//cdn.example.com/app", "http://cdn.example.com/app", "https://cdn.example.com/app", "/app"].each do |base|
      assert_equal compiled, load(plain_host, base + url), base
    end
    with_path = propshaft_app(asset_host: "http://cdn.example.com:8080/sub/")
    assert_equal compiled, load(with_path, "http://cdn.example.com:8080/sub#{url}")
    assert_nil load(with_path, "https://cdn.example.com:8080/sub#{url}")
    spread = propshaft_app(asset_host: "assets%d.example.com")
    (0..3).each { |n| assert_equal compiled, load(spread, "//assets#{n}.example.com#{url}") }
    assert_nil load(spread, "//assets4.example.com#{url}")
    assert_nil load(spread, "//assets%d.example.com#{url}")
    # config.action_controller wins over config, as ActionView reads it.
    controller = propshaft_app(asset_host: "old.example.com", controller: { asset_host: "//new.example.com" })
    assert_equal compiled, load(controller, "//new.example.com#{url}")
    assert_nil load(controller, "//old.example.com#{url}")
    # A proc host can't be read ahead: name the hosts.
    proc_host = propshaft_app(asset_host: ->(_source) { "cdn.example.com" })
    assert_nil load(proc_host, "//cdn.example.com#{url}")
    assert_equal compiled, load(proc_host, "//cdn.example.com#{url}", hosts: ["cdn.example.com"])
    assert_raises(ArgumentError) { loader(proc_host, hosts: "cdn.example.com") }
  end

  def test_the_allowlist_comes_from_the_app
    write("public/packs/app-1.js", "1")
    allow = loader(propshaft_app(asset_host: "https://cdn.example.com"))[:allow]
    assert_equal ["https://cdn.example.com/assets/", "https://cdn.example.com/packs/", "https://cdn.example.com/stylesheets/",
                  "/assets/", "/packs/", "/stylesheets/"], allow
    assert allow.frozen?
  end

  def test_same_bytes_for_a_url_within_one_loader
    path = write("public/assets/a.css", "one")
    assets = loader(propshaft_app)
    assert_equal "one", assets[:loader].call("/assets/a.css")
    File.write(path, "two")
    assert_equal "one", assets[:loader].call("/assets/a.css")
    assert_equal "two", load(propshaft_app, "/assets/a.css"), "a new loader reads files again"
  end

  def test_also_takes_other_urls
    calls = []
    mine = ->(url) { calls << url; "/* #{url} */" }
    assets = loader(propshaft_app, also: { loader: mine, allow: ["/reports/", "/assets/extra.css"] })
    assert_equal ["/assets/", "/stylesheets/", "/reports/", "/assets/extra.css"], assets[:allow]
    assert_equal "/* compiled */\n#{CSS}", assets[:loader].call("/assets/application-#{DIGEST}.css")
    assert_equal "/* /reports/q3.css */", assets[:loader].call("/reports/q3.css")
    # Not the app's: on to yours, which allows it.
    assert_equal "/* /assets/extra.css */", assets[:loader].call("/assets/extra.css")
    # Not the app's, and not on your list: yours isn't asked.
    assert_nil assets[:loader].call("/assets/missing.css")
    assert_nil assets[:loader].call("/reports/../config/secret.css")
    assert_nil assets[:loader].call("/other.css")
    assert_equal ["/reports/q3.css", "/assets/extra.css"], calls
    assert_raises(ArgumentError) { loader(propshaft_app, also: { loader: 1 }) }
    assert_raises(ArgumentError) { loader(propshaft_app, also: { loader: mine, allow: "/x/" }) }
    assert_raises(ArgumentError) { loader(propshaft_app, also: mine) }
  end

  def test_needs_an_app
    skip "Rails is loaded" if defined?(::Rails.application)
    assert_raises(ArgumentError) { Surimono::Rails.assets }
  end

  def test_require_surimono_loads_none_of_it
    lib = loaded_lib
    script = 'require "surimono"; p [defined?(Surimono::Rails), defined?(::Rails), ' \
             '$LOADED_FEATURES.grep(%r{surimono/rails|/(railties|propshaft|sprockets|active_support)}).size]'
    out, status = Open3.capture2e(RbConfig.ruby, "-I", lib, "-e", script)
    assert status.success?, out
    assert_equal "[nil, nil, 0]", out.strip
    script = 'require "surimono/rails"; p [defined?(::Rails), $LOADED_FEATURES.grep(%r{/(railties|propshaft|sprockets|active_support)/}).size]'
    out, status = Open3.capture2e(RbConfig.ruby, "-I", lib, "-e", script)
    assert status.success?, out
    assert_equal "[nil, 0]", out.strip
  end

  # The point of it: a page as a Rails layout writes it gets its sheet inlined
  # and its print media resolved; without the loader it is left as it was.
  def test_a_rails_page_gets_its_print_rules
    html = <<~HTML
      <!DOCTYPE html>
      <html>
        <head>
          <title>Invoice</title>
          <link rel="stylesheet" href="/assets/application-#{DIGEST}.css" data-turbo-track="reload" />
          <script src="/assets/application-d5305eb1.js"></script>
        </head>
        <body><p class="a">Total</p></body>
      </html>
    HTML
    renderer = Surimono::Renderer.new(fonts: [REPO_FONT])
    page = renderer.render_page(html, **Surimono::Rails.assets(app: propshaft_app))
    assert_equal 0, page.report.failed_count, page.report.failed.inspect
    refute_includes page.html, "<link"
    assert_includes page.html, "<title>ran</title>"
    assert_includes page.html, "/* compiled */"
    assert_includes page.html, "@media all { .a { color: red } }"
    refute_includes page.html, "@media print"
    assert_includes page.html, "@media not all { .a { color: blue } }"
    assert_includes page.html, ".b { color: green }"

    plain = renderer.render_page(html)
    assert_includes plain.html, %(<link rel="stylesheet" href="/assets/application-#{DIGEST}.css")
    refute_includes plain.html, "@media all"
    assert_equal Surimono::Renderer.new(fonts: [REPO_FONT], printer_safe: false).render_page(html).html, plain.html
  end

  # The same against the real gems, when they are installed (they aren't in CI):
  # set SURIMONO_RAILS_GEM_PATH to a GEM_PATH holding railties, propshaft,
  # sprockets-rails and sassc-embedded.
  def test_against_real_rails
    gem_path = ENV.fetch("SURIMONO_RAILS_GEM_PATH", nil)
    skip "set SURIMONO_RAILS_GEM_PATH to run against real Rails" unless gem_path

    script = File.expand_path("rails_real_app.rb", __dir__)
    lib = loaded_lib
    %w[propshaft sprockets].each do |pipeline|
      %w[development production].product([{}, { "ASSET_HOST" => "cdn.example.com", "RELATIVE_URL_ROOT" => "/app" }])
                                .each do |env, config|
        Dir.mktmpdir do |dir|
          vars = { "GEM_PATH" => gem_path, "GEM_HOME" => gem_path, "RAILS_ENV" => env,
                   "RUBYOPT" => nil, "BUNDLE_GEMFILE" => nil, "BUNDLER_SETUP" => nil, **config }
          if env == "production"
            out, status = Open3.capture2e(vars, RbConfig.ruby, "-I", lib, script, pipeline, dir, "precompile")
            assert status.success?, "#{pipeline} precompile:\n#{out}"
          end
          out, status = Open3.capture2e(vars, RbConfig.ruby, "-I", lib, script, pipeline, dir)
          assert status.success?, "#{pipeline} #{env} #{config}:\n#{out}"
          assert_includes out, "ok #{pipeline}", out
        end
      end
    end
    # Sprockets' debug mode writes application.debug-<digest>.css.
    Dir.mktmpdir do |dir|
      vars = { "GEM_PATH" => gem_path, "GEM_HOME" => gem_path, "RAILS_ENV" => "development", "ASSETS_DEBUG" => "1",
               "RUBYOPT" => nil, "BUNDLE_GEMFILE" => nil, "BUNDLER_SETUP" => nil }
      out, status = Open3.capture2e(vars, RbConfig.ruby, "-I", lib, script, "sprockets", dir)
      assert status.success? && out.include?(".debug-"), out
    end
  end
end
