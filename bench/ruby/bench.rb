# frozen_string_literal: true

# One benchmark process. bench/tools/bench.py starts it, times it from outside
# (cold) and reads the JSON lines it prints (warm). Run from the repository root:
#
#   ruby -rbundler/setup bench/ruby/bench.rb <system> <mode> <input> <out.pdf> [iterations] [warmup]
#
# system:
#   grover            Grover 1.2.10 prints <input> (a file:// URL of the report
#                     page with the chart libraries); Grover launches Chrome.
#   grover-browser    as grover, but connected to one Chrome kept running by
#                     bench/grover/browser.cjs (Grover's browser_ws_endpoint).
#   grover-shell, grover-shell-browser
#                     as grover and grover-browser on chrome-headless-shell
#                     (BENCH_CHROME_SHELL), Puppeteer's headless: "shell".
#   surimono          Surimono::Renderer#render on <input> (the report with
#                     island markers), then the fulgur gem prints the result.
#   fulgur-prerendered the fulgur gem prints <input>: the report with Chrome's
#                     own island SVGs already inlined. A reference point for
#                     the PDF step (Chrome's SVG text isn't outlined); no
#                     JavaScript runs.
# mode:
#   cold   set up, render once, exit (the harness times the whole process).
#   warm   set up once, render `warmup` times untimed, then `iterations` times,
#          printing one JSON line per timed render.
#
# Environment: BENCH_POOL_SIZE (surimono's runtime pool; unset: the default),
# BENCH_CHROME (Chrome executable for Grover), BENCH_CHROME_SHELL
# (chrome-headless-shell, for the grover-shell systems), BENCH_FONTS_DIR
# (fixtures/fonts), SURIMONO_RUBY_LIB (load path of the built surimono gem).

require "json"

def now_ms = Process.clock_gettime(Process::CLOCK_MONOTONIC) * 1000.0

def emit(event, **fields)
  $stdout.puts(JSON.generate({ event: event, **fields }))
  $stdout.flush
end

def fail!(message, status: 3)
  emit("error", message: message)
  warn(message)
  exit(status)
end

system_name, mode, input, out_pdf, iterations, warmup = ARGV
unless system_name && %w[cold warm].include?(mode) && input && out_pdf
  warn("usage: bench.rb <grover|grover-browser|grover-shell|grover-shell-browser|surimono|fulgur-prerendered> <cold|warm> <input> <out.pdf> [iterations] [warmup]")
  exit(2)
end
iterations = Integer(iterations || 20)
warmup = Integer(warmup || 2)
ROOT = File.expand_path("../..", __dir__)

def font_files
  dir = ENV.fetch("BENCH_FONTS_DIR", File.join(ROOT, "fixtures", "fonts"))
  files = Dir[File.join(dir, "*.{ttf,otf}")].sort
  fail!("no fonts in #{dir}") if files.empty?
  files
end

# A fulgur engine with the fixture fonts registered, as a fulgur user sets it
# up. Page size and margins come from the document's @page rule.
def fulgur_engine
  require "fulgur"
  bundle = Fulgur::AssetBundle.new
  font_files.each { |f| bundle.add_font_file(f) }
  Fulgur::Engine.new(page_size: :a4, assets: bundle)
end

# Builds the renderer for a system once; returns a lambda that renders one
# document to PDF bytes and reports its phases.
def setup(system_name, input)
  case system_name
  when "grover", "grover-browser", "grover-shell", "grover-shell-browser"
    require "grover"
    shell = system_name.start_with?("grover-shell")
    Grover.configure do |c|
      c.allow_file_uris = true
      # Chrome can't use its sandbox as root or on Ubuntu 24.04 runners (AppArmor);
      # the Chrome references run without it too.
      c.node_env_vars = { "GROVER_NO_SANDBOX" => "true" }
    end
    chrome = if shell
      ENV["BENCH_CHROME_SHELL"] or fail!("BENCH_CHROME_SHELL is not set (chrome-headless-shell not installed)", status: 4)
    else
      ENV["BENCH_CHROME"] or fail!("BENCH_CHROME is not set")
    end
    options = {
      root_path: File.join(ROOT, "bench", "grover"), # where node_modules/puppeteer is
      executable_path: chrome,
      # Grover's default for URLs is networkidle2, which waits 500 ms after the
      # last request on every document. "load" plus the islands' own done flag
      # is the fastest correct setting, so Grover is measured at its best.
      wait_until: "load",
      wait_for_selector: "html[data-islands=\"done\"]",
      raise_on_js_error: true,
      raise_on_request_failure: true,
      prefer_css_page_size: true,
      print_background: true,
    }
    # Grover passes debug.headless to puppeteer.launch; "shell" is the old
    # headless mode that chrome-headless-shell implements.
    options[:debug] = { headless: "shell" } if shell
    browser = nil
    if system_name.end_with?("-browser")
      # The keeper only holds Chrome open; bench/tools/bench.py leaves its own
      # process (not its Chrome) out of the memory sum.
      browser = IO.popen(["node", File.join(ROOT, "bench", "grover", "browser.cjs"), chrome, shell ? "shell" : "new"], "r+")
      line = browser.gets or fail!("bench/grover/browser.cjs did not start Chrome")
      options[:browser_ws_endpoint] = JSON.parse(line).fetch("ws")
      at_exit do
        browser.close_write
        browser.read
        browser.close
      end
    end
    lambda do
      t = now_ms
      pdf = Grover.new(input, **options).to_pdf
      [pdf, { "grover_ms" => now_ms - t }]
    end
  when "surimono"
    lib = ENV.fetch("SURIMONO_RUBY_LIB", File.join(ROOT, "bindings", "ruby", "lib"))
    $LOAD_PATH.unshift(lib)
    begin
      require "surimono"
    rescue LoadError => e
      fail!("surimono gem not built (#{e.message}); build it with `bundle exec rake compile` in bindings/ruby", status: 4)
    end
    unless defined?(Surimono::Renderer)
      fail!("the surimono gem on this commit has no Surimono::Renderer yet (P1-PKG-04)", status: 4)
    end
    html = File.read(input, encoding: "UTF-8")
    # BENCH_POOL_SIZE sets the runtime pool; unset, the facade's default is
    # used (bench.py records which).
    pool = ENV["BENCH_POOL_SIZE"].to_s.empty? ? {} : { pool_size: Integer(ENV["BENCH_POOL_SIZE"]) }
    renderer = Surimono::Renderer.new(fonts: font_files, **pool)
    wanted = html.scan(/data-renderer="([^"]+)"/).flatten.uniq.sort
    missing = wanted - renderer.renderers
    unless missing.empty?
      fail!("surimono #{Surimono::VERSION} can't render this report yet: renderers #{missing.join(", ")} " \
            "not available (it has: #{renderer.renderers.join(", ")})", status: 4)
    end
    engine = fulgur_engine
    lambda do
      t = now_ms
      prepared = renderer.render(html)
      t1 = now_ms
      pdf = engine.render_html(prepared).to_s
      [pdf, { "islands_ms" => t1 - t, "pdf_ms" => now_ms - t1 }]
    end
  when "fulgur-prerendered"
    html = File.read(input, encoding: "UTF-8")
    engine = fulgur_engine
    lambda do
      t = now_ms
      pdf = engine.render_html(html).to_s
      [pdf, { "pdf_ms" => now_ms - t }]
    end
  else
    fail!("unknown system #{system_name}", status: 2)
  end
end

t_setup = now_ms
render = setup(system_name, input)
setup_ms = now_ms - t_setup

if mode == "cold"
  t = now_ms
  pdf, phases = render.call
  render_ms = now_ms - t
  File.binwrite(out_pdf, pdf)
  emit("cold", setup_ms: setup_ms, render_ms: render_ms, phases: phases, bytes: pdf.bytesize)
else
  emit("setup", setup_ms: setup_ms)
  warmup.times do |i|
    pdf, = render.call
    emit("warmup", i: i, bytes: pdf.bytesize)
  end
  iterations.times do |i|
    t = now_ms
    pdf, phases = render.call
    ms = now_ms - t
    File.binwrite(out_pdf, pdf)
    emit("iteration", i: i, ms: ms, phases: phases, bytes: pdf.bytesize)
  end
end
emit("done")
