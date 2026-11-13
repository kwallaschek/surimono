//! E3-R1: the Raikiri engine against Chromium's layout of the probe pages
//! in `tests/data/chrome-layout/`. Each page's `<page>.chrome.json` is
//! Chromium 153.0.8010.12's measurement of that very file in an 800 by 600
//! window, written by `measure.py` there (the first five pages are the probes
//! of crates/dom/RAIKIRI.md section 6, measured again from this folder): the
//! expectations, never our output. Run with `--features raikiri`.
//!
//! Every element from `<body>` down is compared, in document order: its
//! `getBoundingClientRect` within half a pixel (Chrome snaps text baselines to
//! whole pixels and lays out in 1/64 px; RAIKIRI.md section 6), and for HTML
//! elements `offsetLeft`, `offsetTop`, `offsetWidth` and `offsetHeight`
//! exactly. What doesn't match is a gap, listed by page with its reason; a
//! gap that closes fails the test until it is taken off the list.
#![cfg(feature = "raikiri")]

mod common;

use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::thread;

use common::limits;
use surimono_dom::{Document, DomRuntime, LayoutEngine, LayoutPolicy, LayoutSetup, RaikiriSetup};

type Job = Box<dyn FnOnce() + Send>;

/// The page thread's stack in the facade, as in `layout_real.rs`.
const STACK: usize = 8 * 1024 * 1024 + 512 * 1024;

/// Runs `f` on one long-lived layout thread, as `layout_real.rs` does.
fn on_layout_thread<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    static WORKER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("layout".into())
            .stack_size(STACK)
            .spawn(move || {
                for job in rx {
                    job();
                }
            })
            .expect("spawn the layout thread");
        Mutex::new(tx)
    });
    let (done_tx, done_rx) = mpsc::channel();
    let job: Job = Box::new(move || {
        let _ = done_tx.send(panic::catch_unwind(AssertUnwindSafe(f)));
    });
    worker
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .send(job)
        .expect("the layout thread is alive");
    match done_rx.recv().expect("the layout thread answers") {
        Ok(r) => r,
        Err(p) => panic::resume_unwind(p),
    }
}

fn repo_file(path: &str) -> Vec<u8> {
    let path = format!("{}/../../{path}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn probe(name: &str) -> String {
    String::from_utf8(repo_file(&format!(
        "crates/dom/tests/data/chrome-layout/{name}"
    )))
    .unwrap()
}

/// The fixture fonts, `Noto Sans` first so that it answers `sans-serif`,
/// with the family names the probes' `@font-face` rules give them.
fn raikiri() -> RaikiriSetup {
    RaikiriSetup::new([
        (
            "Noto Sans",
            repo_file("fixtures/fonts/NotoSans-Regular.ttf"),
        ),
        ("Noto Sans", repo_file("fixtures/fonts/NotoSans-Bold.ttf")),
        (
            "Noto Sans JP",
            repo_file("fixtures/fonts/NotoSansJP-Regular.otf"),
        ),
        (
            "Noto Sans JP",
            repo_file("fixtures/fonts/NotoSansJP-Bold.otf"),
        ),
    ])
    .expect("the fixture fonts make a font set")
}

/// A runtime laying out with Raikiri on an 800 by 600 page with no margins:
/// Chrome's window in `measure.py`.
fn runtime(html: &str) -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.set_layout_policy(LayoutPolicy::Real);
    rt.set_layout_setup(LayoutSetup {
        width: 800,
        height: 600,
        engine: LayoutEngine::Raikiri(raikiri()),
    });
    rt.load(Document::parse(html).unwrap());
    rt
}

/// Compares every element with Chrome's row for it: one line per number off,
/// `<id or tag><index> <what> ours <values> chrome <values>`.
const COMPARE: &str = r#"
var compare = (chrome) => {
    const els = [document.body, ...document.body.querySelectorAll('*')];
    const out = [];
    if (els.length !== chrome.length) out.push('count ' + els.length + ' chrome ' + chrome.length);
    els.forEach((e, i) => {
        const c = chrome[i];
        if (!c) return;
        const name = (c[1] || c[0]) + i;
        if (e.localName !== c[0]) { out.push(name + ' tag ' + e.localName); return; }
        const r = e.getBoundingClientRect();
        const box = [r.x, r.y, r.width, r.height];
        if (box.some((v, k) => !(Math.abs(v - c[2 + k]) <= 0.5)))
            out.push(name + ' box ours ' + box.join(',') + ' chrome ' + c.slice(2, 6).join(','));
        if (c.length > 6) {
            const off = [e.offsetLeft, e.offsetTop, e.offsetWidth, e.offsetHeight];
            if (off.some((v, k) => v !== c[6 + k]))
                out.push(name + ' offset ours ' + off.join(',') + ' chrome ' + c.slice(6, 10).join(','));
        }
    });
    return out.join('\n');
};
"#;

/// What of page `name` differs from Chrome, one line per number off.
fn differences(name: &'static str) -> Vec<String> {
    on_layout_thread(move || {
        let chrome = probe(&format!("{name}.chrome.json"));
        let mut rt = runtime(&probe(&format!("{name}.html")));
        let out: String = rt
            .eval(&format!("{COMPARE} compare(({chrome}).elements)"))
            .unwrap();
        out.lines().map(str::to_owned).collect()
    })
}

/// Asserts that page `name` differs from Chrome in exactly the elements
/// `gaps` starts lines with (each `"<id or tag><index> "`).
fn check(name: &'static str, gaps: &[&str]) {
    let got = differences(name);
    let unexpected: Vec<&String> = got
        .iter()
        .filter(|line| !gaps.iter().any(|g| line.starts_with(g)))
        .collect();
    assert!(unexpected.is_empty(), "{name}: {unexpected:#?}");
    for gap in gaps {
        assert!(
            got.iter().any(|line| line.starts_with(gap)),
            "{name}: {gap} matches Chrome now; take it off the list"
        );
    }
}

/// e3-00's boxes (fixed sizes, `display: none`, percentages), `vw` and `vh`,
/// a positioned box, a table and its row group and row, an inline canvas
/// with and without attributes, an inline `<svg>`, floats, flex and grid.
/// (The block canvas sized by its attributes is a gap of its own, in
/// `layout_real.rs`; `boxes.html` has it.)
#[test]
fn boxes_match_chrome() {
    check("boxes-nocv", &[]);
}

/// The same page with a `display: block` canvas of `width=50 height=20`:
/// Raikiri makes it 800 by 320 (Chrome 50 by 20), and everything after it
/// moves down by the difference.
#[test]
fn a_block_canvas_is_the_one_gap_in_the_boxes() {
    let got = differences("boxes");
    assert!(
        got.iter()
            .any(|l| l.starts_with("cv13 box ours 0,282,800,320")),
        "{got:#?}"
    );
}

/// An inline-block of text, wrapped paragraphs, a span in running text,
/// `line-height: normal`, bold, Japanese with and without
/// `text-autospace`, an inline-block in a line, shrink-to-fit labels.
#[test]
fn text_matches_chrome_but_for_japanese_autospace() {
    // R1 of fixtures/printers/NOTES.md: with the initial `text-autospace`
    // Raikiri spaces Japanese against Latin letters and digits where
    // Chromium 153 does not, so the span after them starts further right
    // and the paragraph wraps elsewhere; with `no-autospace` (#jns) it
    // matches.
    check("text", &["jas"]);
}

/// The root's margin, an absolute box in a paragraph, a block `<svg>`,
/// `<img width height>`, shrink-to-fit, a CSS table, a list, two columns
/// (the paragraph split across them is its whole width, from the union of
/// its fragments), `pre`, `box-sizing`, `min-width` over `max-width`.
#[test]
fn gaps_of_blitz_match_chrome() {
    check("gaps", &[]);
}

/// A relatively positioned block, translated and scaled blocks, and a
/// relatively positioned span: transforms and inline relative offsets,
/// which no fragment includes, added by the engine.
#[test]
fn transforms_and_relative_offsets_match_chrome() {
    check("offsets", &[]);
}

/// e3-00's report page, 20 flex rows: every row, label and figure.
#[test]
fn the_report_page_matches_chrome() {
    check("report20", &[]);
}

/// Media `screen` with the page area as the viewport: what Chrome's scripts
/// see. `@media print` rules and `<style media=print>` don't apply, screen
/// rules and width, height and orientation queries do (DECISIONS, E3-R1).
#[test]
fn the_measuring_cascade_is_screen_media_as_in_chrome() {
    check("media", &[]);
}

/// `offsetLeft` and `offsetTop` (CSSOM View §7): from the offset parent's
/// padding edge; a positioned ancestor, `body`, a table cell or table for a
/// static element; transforms ignored; a fixed element from the viewport.
#[test]
fn offsets_match_chrome() {
    check("offset-parents", &[]);
}

/// Coordinates past a page break. Raikiri paginates (here into 600 px pages)
/// and moves a row that would cross a page's end to the next page, so its
/// paged positions run lower than one continuous layout's, by the space left
/// at the foot of each page. The engine moves each page's fragments to where
/// that page's content begins in the flow, which is Chrome's continuous
/// layout: rows on later pages, the box split across two pages (its height
/// whole), and what follows it.
#[test]
fn positions_past_page_breaks_are_the_continuous_ones() {
    check("breaks", &[]);
}

/// `report(2000)` as `layout_real.rs` writes it: 2,000 rows of 22 px, over
/// 74 page breaks. Its last row ends at 44,000 px in Chrome; stacking the
/// pages one under another would put it 444 px lower (RAIKIRI.md section 2).
#[test]
fn the_last_of_two_thousand_rows_is_where_chrome_has_it() {
    let chrome = probe("report2000.chrome.json");
    let got: String = on_layout_thread(move || {
        let mut rt = runtime(&report(2000));
        rt.eval(&format!(
            "const c = ({chrome}).last_row, r = document.body.lastElementChild.getBoundingClientRect();
             [r.x, r.y, r.width, r.height].join() + ' ' + c.join()"
        ))
        .unwrap()
    });
    let (ours, chrome) = got.split_once(' ').unwrap();
    let parse = |s: &str| -> Vec<f64> { s.split(',').map(|v| v.parse().unwrap()).collect() };
    let (ours, chrome) = (parse(ours), parse(chrome));
    assert_eq!(chrome[1] + chrome[3], 44_000.0, "the expectation");
    for k in 0..4 {
        assert!(
            (ours[k] - chrome[k]).abs() <= 0.5,
            "ours {ours:?} chrome {chrome:?}"
        );
    }
}

/// `crates/dom/tests/layout_real.rs`'s `report`, and `measure.py`'s.
fn report(rows: usize) -> String {
    let mut html = String::from(
        "<!doctype html><html><head><style>body { margin: 0; font: 12px 'Noto Sans' }\
         .row { display: flex; padding: 2px 4px; border-bottom: 1px solid #ccc }\
         .l { flex: 1 } .v { width: 80px; text-align: right; font-weight: bold }\
         .row:nth-child(even) { background: #f6f6f6 }</style></head><body>",
    );
    for i in 0..rows {
        html.push_str(&format!(
            "<div class=row><span class=l>Item number {i}</span>\
             <span class=v style=\"color:#333\">{},{:03}</span></div>",
            i % 97,
            (i * 37) % 1000
        ));
    }
    html
}

/// Every number of every probe page, as one string.
fn all_numbers() -> String {
    on_layout_thread(|| {
        let mut out = String::new();
        for name in [
            "boxes",
            "text",
            "gaps",
            "offsets",
            "report20",
            "breaks",
            "media",
            "offset-parents",
        ] {
            let mut rt = runtime(&probe(&format!("{name}.html")));
            let numbers: String = rt
                .eval(
                    "[document.body, ...document.body.querySelectorAll('*')].map((e) => {
                        const r = e.getBoundingClientRect();
                        return [r.x, r.y, r.width, r.height, e.offsetLeft, e.offsetTop].join();
                    }).join(' ')",
                )
                .unwrap();
            out.push_str(&format!("{name}: {numbers}\n"));
        }
        out
    })
}

/// Same input, same output: every number of every probe twice in one
/// process, and the same in a second process (this test binary run again
/// for [`numbers_for_another_process`]).
#[test]
fn the_same_pages_give_the_same_numbers_twice_and_in_another_process() {
    let first = all_numbers();
    assert_eq!(first, all_numbers());
    let exe = std::env::current_exe().unwrap();
    let child = std::process::Command::new(exe)
        .args([
            "--exact",
            "numbers_for_another_process",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("SURIMONO_E3R1_NUMBERS", "1")
        .output()
        .unwrap();
    assert!(child.status.success(), "{child:?}");
    let stdout = String::from_utf8(child.stdout).unwrap();
    let theirs = stdout
        .split("BEGIN NUMBERS\n")
        .nth(1)
        .and_then(|rest| rest.split("END NUMBERS").next())
        .unwrap_or_else(|| panic!("no numbers in {stdout}"));
    assert_eq!(first, theirs);
}

/// Prints [`all_numbers`] for the test above, in its own process.
#[test]
#[ignore = "run by the_same_pages_give_the_same_numbers_twice_and_in_another_process"]
fn numbers_for_another_process() {
    if std::env::var_os("SURIMONO_E3R1_NUMBERS").is_some() {
        print!("BEGIN NUMBERS\n{}END NUMBERS", all_numbers());
    }
}

/// Chrome's rows of page `name`: tag and border box, in document order.
fn chrome_boxes(name: &str) -> Vec<(String, [f64; 4])> {
    probe(&format!("{name}.chrome.json"))
        .lines()
        .filter_map(|line| {
            let row = &line[line.find("[\"")? + 1..];
            let fields: Vec<&str> = row.split(',').collect();
            let num = |k: usize| {
                fields[k]
                    .trim_matches(|c| c == ']' || c == '}')
                    .parse()
                    .unwrap()
            };
            Some((
                fields[0].trim_matches('"').to_owned(),
                [num(2), num(3), num(4), num(5)],
            ))
        })
        .collect()
}

/// Raikiri's own route to the boxes of page `name`, for comparison: its
/// parse of the page's HTML, the documented `raikiri_html::layout` (print
/// media, as fulgur-raikiri prints), and the union of each element's box and
/// replaced fragments with pages stacked, as the #185 prototype read them.
/// No transforms or relative offsets: no source has them.
fn documented_route(name: &str) -> Vec<(String, [f64; 4])> {
    use raikiri_html::{
        FragmentKind, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources,
    };
    let fonts = raikiri_html::FontCollectionBuilder::new()
        .system_fonts(false)
        .font_bytes(
            "Noto Sans",
            repo_file("fixtures/fonts/NotoSans-Regular.ttf"),
        )
        .font_bytes("Noto Sans", repo_file("fixtures/fonts/NotoSans-Bold.ttf"))
        .font_bytes(
            "Noto Sans JP",
            repo_file("fixtures/fonts/NotoSansJP-Regular.otf"),
        )
        .font_bytes(
            "Noto Sans JP",
            repo_file("fixtures/fonts/NotoSansJP-Bold.otf"),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new()
        .stylesheet("@layer { :where(html) { font-family: sans-serif } }")
        .stylesheet("@page { margin: 0px 0px 0px 0px; }")
        .border_width_device_scale(Some(1.0))
        .fonts(fonts);
    let html = probe(&format!("{name}.html"));
    let doc = raikiri_html::parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let mut page_box = raikiri_traits::PageBox::new();
    page_box.width = 800.0;
    page_box.height = 600.0;
    let LayoutStatus::Completed(laid) = raikiri_html::layout(
        &doc,
        PageDefaults::builder().page_box(page_box).build(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("aborted");
    };
    let mut rects: std::collections::BTreeMap<u64, [f64; 4]> = Default::default();
    for page in laid.pages() {
        let dy = f64::from(page.index()) * 600.0;
        for f in page.fragments() {
            if matches!(f.kind(), FragmentKind::Text) {
                continue;
            }
            let r = f.rect();
            let b = [
                f64::from(r.x),
                f64::from(r.y) + dy,
                f64::from(r.width),
                f64::from(r.height),
            ];
            rects
                .entry(f.node().0)
                .and_modify(|a| {
                    let (l, t) = (a[0].min(b[0]), a[1].min(b[1]));
                    let (rt, bo) = (
                        (a[0] + a[2]).max(b[0] + b[2]),
                        (a[1] + a[3]).max(b[1] + b[3]),
                    );
                    *a = [l, t, rt - l, bo - t];
                })
                .or_insert(b);
        }
    }
    let dom = doc.dom();
    let body = (0..dom.node_count())
        .find(|&i| dom.get_node(i).and_then(|n| n.tag_name()) == Some("body"))
        .unwrap();
    dom.preorder(body)
        .filter_map(|i| {
            let tag = dom.get_node(i)?.tag_name()?.to_owned();
            Some((tag, rects.get(&(i as u64)).copied().unwrap_or_default()))
        })
        .collect()
}

/// The engine's boxes of page `name`, in document order.
fn engine_route(name: &'static str) -> Vec<(String, [f64; 4])> {
    let got: String = on_layout_thread(move || {
        let mut rt = runtime(&probe(&format!("{name}.html")));
        rt.eval(
            "[document.body, ...document.body.querySelectorAll('*')].map((e) => {
                const r = e.getBoundingClientRect();
                return [e.localName, r.x, r.y, r.width, r.height].join();
            }).join(' ')",
        )
        .unwrap()
    });
    got.split(' ')
        .map(|row| {
            let f: Vec<&str> = row.split(',').collect();
            let n = |k: usize| f[k].parse().unwrap();
            (f[0].to_owned(), [n(1), n(2), n(3), n(4)])
        })
        .collect()
}

/// How many of `ours` are within half a pixel of Chrome's row.
fn agreeing(ours: &[(String, [f64; 4])], chrome: &[(String, [f64; 4])]) -> usize {
    assert_eq!(ours.len(), chrome.len());
    ours.iter()
        .zip(chrome)
        .filter(|((t, a), (u, b))| t == u && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 0.5))
        .count()
}

/// RAIKIRI.md section 2 left one question for E3-R1: whether the tree we
/// build into Raikiri's arena and lay out through its paged functions (path
/// B, the engine) measures as Raikiri's own parse and documented `layout()`
/// do. On every probe page the engine agrees with Chrome on at least as many
/// elements as the documented route, and on more where transforms, inline
/// relative offsets or page breaks are in play, which only the engine adds.
#[test]
fn the_engine_agrees_with_chrome_at_least_as_often_as_raikiris_own_route() {
    for name in [
        "boxes-nocv",
        "boxes",
        "text",
        "gaps",
        "offsets",
        "report20",
        "breaks",
        "offset-parents",
    ] {
        let chrome = chrome_boxes(name);
        let (a, b) = (documented_route(name), engine_route(name));
        let (agree_a, agree_b) = (agreeing(&a, &chrome), agreeing(&b, &chrome));
        println!(
            "{name}: {} elements; within 0.5 px of Chrome: documented route {agree_a}, engine {agree_b}",
            chrome.len()
        );
        assert!(
            agree_b >= agree_a,
            "{name}: engine {agree_b}, documented route {agree_a}"
        );
    }
}
