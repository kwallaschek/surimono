# frozen_string_literal: true

# Resident memory of the Ruby gem after a concurrent burst: does idle release
# give it back, does CRuby's GC, does malloc_trim(3)? (P1-RT-25; the numbers are
# in docs/DECISIONS.md.) Linux only. Run by hand, on a release build of the
# extension (a debug build's numbers mean nothing):
#
#   cargo build --release --locked -p surimono-ruby
#   mkdir -p /tmp/lib/surimono
#   cp "$CARGO_TARGET_DIR/release/libsurimono.so" /tmp/lib/surimono/surimono.so
#   ruby -I/tmp/lib -Ibindings/ruby/lib crates/islands/tests/gem_memory.rb
#
# Environment: GEM_MEM_CALLERS (4), GEM_MEM_POOL (4), GEM_MEM_ROUNDS (2, burst
# rounds per caller), GEM_MEM_STAGES (idle,gc,trim, the order after the burst).

require "fiddle"
require "surimono"

ROOT = File.expand_path("../../..", __dir__)
CALLERS = Integer(ENV.fetch("GEM_MEM_CALLERS", "4"))
POOL = Integer(ENV.fetch("GEM_MEM_POOL", "4"))
ROUNDS = Integer(ENV.fetch("GEM_MEM_ROUNDS", "2"))
STAGES = ENV.fetch("GEM_MEM_STAGES", "idle,gc,trim").split(",")

ERRORS = Hash.new(0)
MALLOC_TRIM = Fiddle::Function.new(Fiddle.dlopen(nil)["malloc_trim"], [Fiddle::TYPE_SIZE_T], Fiddle::TYPE_INT)

def mem
  rollup = File.read("/proc/self/smaps_rollup")
  %w[Rss Pss Anonymous].to_h { |k| [k, rollup[/^#{k}:\s+(\d+) kB/, 1].to_i / 1024.0] }
end

def row(stage)
  m = mem
  puts format("%-28s RSS %7.1f MB  PSS %7.1f MB  anon %7.1f MB", stage, m["Rss"], m["Pss"], m["Anonymous"])
end

def render_all(renderer, documents)
  documents.each do |d|
    renderer.render(d)
  rescue Surimono::Error => e
    # A busy machine can hit a limit; the memory numbers still hold.
    ERRORS[e.kind] += 1
  end
end

fonts = Dir[File.join(ROOT, "fixtures/fonts/**/*.{ttf,otf}")].sort
documents = Dir[File.join(ROOT, "fixtures/islands/*/input.html")].sort.map { |f| File.read(f, encoding: "UTF-8") }
renderer = Surimono::Renderer.new(fonts: fonts, pool_size: POOL)

puts "#{documents.size} documents, #{CALLERS} callers, pool #{POOL}, #{ROUNDS} rounds, Ruby #{RUBY_VERSION}"
row("after Renderer.new")
render_all(renderer, documents)
row("after one serial pass")
sleep 3
row("idle 3 s")
threads = Array.new(CALLERS) { Thread.new { ROUNDS.times { render_all(renderer, documents) } } }
threads.each(&:join)
row("after the burst")
STAGES.each do |stage|
  case stage
  when "idle"
    sleep 3
    row("idle 3 s (runtimes released)")
  when "gc"
    GC.start
    row("GC.start (full)")
  when "trim"
    started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    MALLOC_TRIM.call(0)
    took = Process.clock_gettime(Process::CLOCK_MONOTONIC) - started
    row(format("malloc_trim(0), %.1f ms", took * 1000))
  end
end
puts "render errors: #{ERRORS.empty? ? 'none' : ERRORS}"
