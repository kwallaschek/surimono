//! E3-R1 stage C: what one layout costs, by engine, on generated pages and
//! on the Chromium probe pages. A measurement, printed as the table in
//! `RAIKIRI.md` section 10; nothing here is an expectation. Run on a release
//! build, alone on the machine:
//!
//! `cargo test --release -p surimono-dom --features raikiri --test layout_cost -- --ignored --nocapture --test-threads 1`
//!
//! Per page and engine: the first layout query's time, a query's time after
//! one style change (the layout is stale, so it is a whole layout again),
//! the peak of the live heap (a counting allocator) during the first layout
//! above what it was before, sampled by a thread of its own in a round that
//! isn't timed, the live heap the laid-out page holds, and the live heap
//! left once the document is dropped. Times are the best of the timed
//! rounds, each in a new runtime.
#![cfg(feature = "raikiri")]

mod common;

use std::alloc::System;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, Instant};

use blitz_dom::FontContext;
use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};
use surimono_dom::{Document, DomRuntime, LayoutEngine, LayoutPolicy, LayoutSetup, RaikiriSetup};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

/// The page thread's stack in the facade.
const STACK: usize = 8 * 1024 * 1024 + 512 * 1024;

fn font_file(name: &str) -> Vec<u8> {
    let path = format!("{}/../../fixtures/fonts/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Blitz,
    Raikiri,
}

fn setup(which: Which) -> LayoutSetup {
    let engine = match which {
        Which::Blitz => {
            let mut ctx = FontContext {
                collection: Collection::new(CollectionOptions {
                    shared: false,
                    system_fonts: false,
                }),
                source_cache: SourceCache::new(Default::default()),
            };
            for name in ["NotoSans-Regular.ttf", "NotoSans-Bold.ttf"] {
                let blob: Blob<u8> = font_file(name).into();
                ctx.collection.register_fonts(blob, None);
            }
            LayoutEngine::Blitz(Box::new(ctx))
        }
        Which::Raikiri => LayoutEngine::Raikiri(
            RaikiriSetup::new([
                ("Noto Sans", font_file("NotoSans-Regular.ttf")),
                ("Noto Sans", font_file("NotoSans-Bold.ttf")),
            ])
            .expect("the fixture fonts make a font set"),
        ),
    };
    LayoutSetup {
        width: 800,
        height: 600,
        engine,
    }
}

const HEAD: &str = "<!doctype html><html><head><style>body { margin: 0; font: 12px/1.4 'Noto Sans' }\
    .b { padding: 2px; border-bottom: 1px solid #ccc }\
    td { padding: 2px 4px } p { margin: 0 0 6px; width: 180px }\
    .grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px }\
    .card { border: 1px solid #ddd; padding: 6px } .card h3 { margin: 0; font-size: 14px }\
    .kv { display: flex; justify-content: space-between }</style></head><body>";

/// A page of the shape `shape` with about `nodes` elements and text nodes.
fn page(shape: &str, nodes: usize) -> String {
    let mut html = String::from(HEAD);
    match shape {
        // Two nodes a block.
        "blocks" => {
            for i in 0..nodes / 2 {
                html.push_str(&format!("<div class=b>Block {i}</div>"));
            }
        }
        // Seven nodes a row.
        "table" => {
            html.push_str("<table>");
            for i in 0..nodes / 7 {
                html.push_str(&format!(
                    "<tr><td>{i}</td><td>Name {}</td><td>{}.{:02}</td></tr>",
                    i % 89,
                    i % 997,
                    i % 100
                ));
            }
            html.push_str("</table>");
        }
        // Five nodes a paragraph, wrapped over two or three lines.
        "article" => {
            for i in 0..nodes / 5 {
                html.push_str(&format!(
                    "<p>Paragraph {i} runs <em>on and on</em> and wraps.</p>"
                ));
            }
        }
        // Eight nodes a card: a grid of cards, each with a flex row.
        "dashboard" => {
            html.push_str("<div class=grid>");
            for i in 0..nodes / 8 {
                html.push_str(&format!(
                    "<div class=card><h3>Metric {i}</h3><div class=kv><span>value</span><b>{}</b></div></div>",
                    i * 7 % 1000
                ));
            }
            html.push_str("</div>");
        }
        other => panic!("{other}"),
    }
    html.push_str("<div id=end></div></body></html>");
    html
}

fn live_bytes() -> i64 {
    let s = GLOBAL.stats();
    s.bytes_allocated as i64 - s.bytes_deallocated as i64
}

/// `f`'s result and the highest live heap while it ran, sampled.
fn with_peak<R>(f: impl FnOnce() -> R) -> (R, i64) {
    let stop = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(AtomicI64::new(live_bytes()));
    let sampler = {
        let (stop, peak) = (Arc::clone(&stop), Arc::clone(&peak));
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                peak.fetch_max(live_bytes(), Ordering::Relaxed);
                std::thread::yield_now();
            }
        })
    };
    let r = f();
    stop.store(true, Ordering::Relaxed);
    sampler.join().unwrap();
    peak.fetch_max(live_bytes(), Ordering::Relaxed);
    (r, peak.load(Ordering::Relaxed))
}

struct Cost {
    nodes: usize,
    first: Duration,
    again: Duration,
    peak: i64,
    held: i64,
    left: i64,
    answer: String,
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// One page laid out by one engine: the first round's memory, with the heap
/// sampled, and the best of `rounds` more for the times.
fn cost(which: Which, html: &str, rounds: usize) -> Cost {
    let mut out: Option<Cost> = None;
    for round in 0..=rounds {
        let before_load = live_bytes();
        let mut rt = DomRuntime::new(common::limits().with_time(Duration::from_secs(600))).unwrap();
        rt.set_layout_policy(LayoutPolicy::Real);
        rt.set_layout_setup(setup(which));
        let doc = Document::parse(html).unwrap();
        let nodes = doc.node_count();
        rt.load(doc);
        // The glue's install, kept out of the measurement.
        let _: String = rt.eval("document.body.id").unwrap();
        let before = live_bytes();
        let query = "document.getElementById('end').getBoundingClientRect().y";
        let start = Instant::now();
        let (answer, peak): (Result<f64, _>, i64) = if round == 0 {
            with_peak(|| rt.eval(query))
        } else {
            (rt.eval(query), 0)
        };
        let first = start.elapsed();
        let held = live_bytes() - before;
        let mut again = Duration::MAX;
        for k in 0..3 {
            let start = Instant::now();
            let _: Result<f64, _> = rt.eval(&format!(
                "document.body.firstElementChild.style.marginTop = '{k}px';\
                 document.getElementById('end').getBoundingClientRect().y"
            ));
            again = again.min(start.elapsed());
        }
        drop(rt.take());
        drop(rt);
        let left = live_bytes() - before_load;
        let this = Cost {
            nodes,
            first,
            again,
            peak: peak - before,
            held,
            left,
            answer: format!("{answer:?}"),
        };
        out = Some(match out {
            // The sampled round's times are not the ones reported.
            None => Cost {
                first: Duration::MAX,
                again: Duration::MAX,
                ..this
            },
            Some(best) => Cost {
                first: best.first.min(this.first),
                again: best.again.min(this.again),
                ..best
            },
        });
    }
    out.expect("one round at least")
}

fn on_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
#[ignore = "a measurement, printed; run on a release build"]
fn layout_cost_generated() {
    on_big_stack(|| {
        println!(
            "| Page | Nodes | Engine | First layout | After one style change | Peak heap during it | Held while laid out | Left after drop | µs a node | KiB a node (peak) | y of the last box |"
        );
        println!("|---|---|---|---|---|---|---|---|---|---|---|");
        for shape in ["blocks", "table", "article", "dashboard"] {
            for nodes in [100, 1_000, 5_000, 19_900] {
                let html = page(shape, nodes);
                for which in [Which::Blitz, Which::Raikiri] {
                    let rounds = if nodes > 5_000 { 2 } else { 3 };
                    let c = cost(which, &html, rounds);
                    println!(
                        "| {shape} | {} | {which:?} | {:.2} ms | {:.2} ms | {} KiB | {} KiB | {} B | {:.1} | {:.1} | {} |",
                        c.nodes,
                        ms(c.first),
                        ms(c.again),
                        c.peak / 1024,
                        c.held / 1024,
                        c.left,
                        ms(c.first) * 1e3 / c.nodes as f64,
                        c.peak as f64 / 1024.0 / c.nodes as f64,
                        c.answer
                    );
                }
            }
        }
    });
}

#[test]
#[ignore = "a measurement, printed; run on a release build"]
fn layout_cost_probes() {
    on_big_stack(|| {
        let dir = format!("{}/tests/data/chrome-layout", env!("CARGO_MANIFEST_DIR"));
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".html"))
            .collect();
        names.sort();
        for which in [Which::Blitz, Which::Raikiri] {
            let mut firsts = Vec::new();
            let mut nodes = 0;
            let mut peak = 0i64;
            for name in &names {
                let html = std::fs::read_to_string(format!("{dir}/{name}")).unwrap();
                // The probes' last element may be anything; the root's box.
                let html = html.replace("</body>", "<div id=end></div></body>");
                let c = cost(which, &html, 3);
                nodes += c.nodes;
                peak = peak.max(c.peak / 1024);
                firsts.push((ms(c.first), name.clone(), c.nodes));
            }
            firsts.sort_by(|a, b| a.0.total_cmp(&b.0));
            let total: f64 = firsts.iter().map(|f| f.0).sum();
            let median = firsts[firsts.len() / 2].0;
            let worst = firsts.last().unwrap();
            println!(
                "{which:?}: {} probe pages, {nodes} nodes: first layout median {median:.2} ms, total {total:.1} ms, slowest {:.2} ms ({}, {} nodes); largest peak heap during a layout {peak} KiB",
                firsts.len(),
                worst.0,
                worst.1,
                worst.2
            );
        }
    });
}
