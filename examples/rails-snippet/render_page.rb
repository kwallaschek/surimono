# frozen_string_literal: true

# What a Rails app does to print a page whose own scripts build it, in plain
# Ruby: the page's scripts run in surimono, a loader serves its files from one
# folder, the report says what happened, and fulgur prints the result.
# docs/guide/rails.md explains each part. Run from the repository root, with
# the gem built:
#
#   ruby -Ibindings/ruby/lib examples/rails-snippet/render_page.rb            # HTML to stdout
#   ruby -Ibindings/ruby/lib examples/rails-snippet/render_page.rb out.pdf    # PDF (needs the fulgur gem)
#   ruby -Ibindings/ruby/lib examples/rails-snippet/render_page.rb --input examples/shared/page.html

require "surimono"

module PageExample
  ROOT = File.expand_path("../..", __dir__)
  PAGE = File.join(ROOT, "examples/shared/page.html")
  ASSETS = File.join(ROOT, "examples/shared/assets")

  FONTS = %w[
    NotoSans-Regular.ttf
    NotoSans-Bold.ttf
    NotoSansJP-Regular.otf
    NotoSansJP-Bold.otf
  ].map { |name| File.join(ROOT, "fixtures/fonts", name) }.freeze

  LOCK = Mutex.new

  def self.renderer
    @renderer || LOCK.synchronize { @renderer ||= Surimono::Renderer.new(fonts: FONTS) }
  end

  # The url arrives as the page wrote it, with no base applied. The allowlist
  # refuses paths that climb out of /assets/.
  LOADER = lambda do |url|
    File.binread(File.join(ASSETS, url.delete_prefix("/assets/")))
  rescue SystemCallError
    nil
  end

  def self.render(html)
    renderer.render_page(html, loader: LOADER, allow: ["/assets/"])
  end

  # What happened while the page ran. None of this failed the render.
  def self.report_lines(report)
    lines = ["settled: #{report.settled}"]
    report.errors.each { |message| lines << "uncaught: #{message}" }
    report.rejections.each { |message| lines << "rejected: #{message}" }
    report.failed.each { |f| lines << "not loaded: #{f[:kind]} #{f[:url]}: #{f[:reason]}" }
    lines
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
    input = (i = args.index("--input")) ? args.slice!(i, 2).last : PAGE
    page = render(File.read(input, encoding: "UTF-8"))
    report_lines(page.report).each { |line| warn line }
    if (pdf_path = args.first)
      to_pdf(page.html).write_to_path(pdf_path)
    else
      $stdout.write(page.html)
    end
    0
  rescue Surimono::Error => e
    warn "error: #{e.message} [#{e.kind}]"
    1
  end
end

exit PageExample.main(ARGV) if $PROGRAM_NAME == __FILE__
