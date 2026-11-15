//! E3-R1: the Raikiri engine against Chromium's layout of the probe pages
//! in `tests/data/chrome-layout/`. Each page's `<page>.chrome.json` is
//! Chromium 153.0.8010.12's measurement of that very file in an 800 by 600
//! window, written by `measure.py` there (the first five pages are the probes
//! of crates/dom/RAIKIRI.md section 6, measured again from this folder): the
//! expectations, never our output. Run with `--features raikiri`.
//!
//! The root element and every element from `<body>` down are compared, in
//! document order: `getBoundingClientRect` within a twentieth of a pixel
//! (both lay out in fractions of a pixel, Chrome in 1/64 px, and an `f32`
//! sum's last bits are noise; a difference that adds up down a page, such as
//! a line height, is a gap and fails), and for HTML elements
//! `offsetLeft`, `offsetTop`, `offsetWidth` and `offsetHeight` exactly. What doesn't match is a gap, listed by page with its reason; a
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

/// Compares the root element and every element from `<body>` down with
/// Chrome's row for it: one line per number off, `<id or tag><index> <what>
/// ours <values> chrome <values>` (the root element's is `html`).
const COMPARE: &str = r#"
var compare = (chrome, root) => {
    const els = [document.body, ...document.body.querySelectorAll('*')];
    const out = [];
    if (els.length !== chrome.length) out.push('count ' + els.length + ' chrome ' + chrome.length);
    const row = (e, c, name) => {
        if (e.localName !== c[0]) { out.push(name + ' tag ' + e.localName); return; }
        const r = e.getBoundingClientRect();
        const box = [r.x, r.y, r.width, r.height];
        if (box.some((v, k) => !(Math.abs(v - c[2 + k]) <= 0.05)))
            out.push(name + ' box ours ' + box.join(',') + ' chrome ' + c.slice(2, 6).join(','));
        if (c.length > 6) {
            const off = [e.offsetLeft, e.offsetTop, e.offsetWidth, e.offsetHeight];
            if (off.some((v, k) => v !== c[6 + k]))
                out.push(name + ' offset ours ' + off.join(',') + ' chrome ' + c.slice(6, 10).join(','));
        }
    };
    if (root) row(document.documentElement, root, 'html');
    els.forEach((e, i) => { if (chrome[i]) row(e, chrome[i], (chrome[i][1] || chrome[i][0]) + i); });
    return out.join('\n');
};
"#;

/// The gap behind most of the lists below: **the half pixel of an inline
/// box**. Chromium puts every line's baseline on a whole pixel: of a line's
/// leading it gives the upper half rounded down. Raikiri halves it exactly,
/// so where the leading is odd (12px Noto Sans is 17 px high; in a 16 px or a
/// 20 px line the leading is -1 or 3) its text and its inline boxes sit half
/// a pixel lower than Chromium's, and `offsetTop`, a whole number, comes out
/// one more. The lists pin such an element's lines with our numbers, the box
/// to a twentieth of a pixel like every other.
///
/// What of page `name` differs from Chrome, one line per number off.
fn differences(name: &'static str) -> Vec<String> {
    on_layout_thread(move || {
        let chrome = probe(&format!("{name}.chrome.json"));
        let mut rt = runtime(&probe(&format!("{name}.html")));
        let out: String = rt
            .eval(&format!(
                "{COMPARE} const chrome = ({chrome}); compare(chrome.elements, chrome.root)"
            ))
            .unwrap();
        out.lines().map(str::to_owned).collect()
    })
}

/// Asserts that page `name` differs from Chrome in exactly the lines `gaps`
/// start (each `"<id or tag><index> <box or offset> ours <our values>"`, so
/// that a gap that moves fails too).
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
            "{name}: `{gap}` isn't how it differs from Chrome now: {got:#?}"
        );
    }
}

/// [`check`] for a page whose inline boxes sit low by the half pixel
/// (above): a line of an element whose name begins with one of `low`'s
/// prefixes is that gap if its box is lower than Chrome's by exactly that
/// prefix's amount and otherwise within `across` across (a twentieth of a
/// pixel, or a sixteenth where Raikiri's narrower text advances add up
/// before the element) and a twentieth in height, and its offsets are the
/// same but for `offsetTop` one more or the same and `offsetWidth` one less
/// or the same (the narrower text advance). There must be `lines` such
/// lines; every other line is one of `gaps`.
fn check_low(across: f64, name: &'static str, low: &[(&str, f64)], lines: usize, gaps: &[&str]) {
    let got = differences(name);
    let numbers = |s: &str| -> Vec<f64> { s.split(',').filter_map(|v| v.parse().ok()).collect() };
    let is_low = |line: &String| {
        let mut words = line.split(' ');
        let (Some(who), Some(what), Some(_), Some(ours), Some(_), Some(chrome)) = (
            words.next(),
            words.next(),
            words.next(),
            words.next(),
            words.next(),
            words.next(),
        ) else {
            return false;
        };
        let (ours, chrome) = (numbers(ours), numbers(chrome));
        if ours.len() != 4 || chrome.len() != 4 {
            return false;
        }
        let d: Vec<f64> = (0..4).map(|k| ours[k] - chrome[k]).collect();
        low.iter().any(|&(prefix, by)| {
            who.starts_with(prefix)
                && if what == "box" {
                    (d[1] - by).abs() < 1e-9
                        && d[0].abs() <= across
                        && d[2].abs() <= across
                        && d[3].abs() <= 0.05
                } else {
                    d[0] == 0.0
                        && (d[1] == 0.0 || d[1] == 1.0)
                        && (d[2] == 0.0 || d[2] == -1.0)
                        && d[3] == 0.0
                }
        })
    };
    let (lowered, rest): (Vec<String>, Vec<String>) = got.iter().cloned().partition(is_low);
    assert_eq!(lowered.len(), lines, "{name}: {lowered:#?}");
    let unexpected: Vec<&String> = rest
        .iter()
        .filter(|line| !gaps.iter().any(|g| line.starts_with(g)))
        .collect();
    assert!(unexpected.is_empty(), "{name}: {unexpected:#?}");
    for gap in gaps {
        assert!(
            rest.iter().any(|line| line.starts_with(gap)),
            "{name}: `{gap}` isn't how it differs from Chrome now: {got:#?}"
        );
    }
}

/// Box positions across, compared as everywhere in this file.
const ACROSS: f64 = 0.05;
/// Across, where Raikiri's narrower text advances add up before an element.
const ADVANCES: f64 = 0.0625;

/// Asserts that page `name` is refused: every element answers as with no
/// box, all eight numbers zero, where Chrome has its numbers.
fn check_refused(name: &'static str) {
    let got = differences(name);
    assert!(!got.is_empty(), "{name}: laid out as in Chrome");
    let zeros: String = on_layout_thread(move || {
        runtime(&probe(&format!("{name}.html")))
            .eval(
                "[document.documentElement, ...document.querySelectorAll('body, body *')].map((e) => {
                    const r = e.getBoundingClientRect();
                    return [r.x, r.y, r.width, r.height, e.offsetLeft, e.offsetTop, e.offsetWidth,
                            e.offsetHeight].every((v) => v === 0);
                }).every((z) => z) ? 'zeros' : 'numbers'",
            )
            .unwrap()
    });
    assert_eq!(zeros, "zeros", "{name}: {got:#?}");
}

/// A tool: prints how the pages named in `SURIMONO_PROBES` (separated by
/// commas) differ from Chrome, the lines the lists in this file pin. Run
/// with `-- --ignored print_differences --nocapture`.
#[test]
#[ignore = "a tool, for writing a page's list of gaps"]
fn print_differences() {
    let pages = std::env::var("SURIMONO_PROBES").unwrap_or_default();
    for name in pages.split(',').filter(|name| !name.is_empty()) {
        let name: &'static str = name.to_owned().leak();
        println!("{name}:");
        for line in differences(name) {
            println!("    {line:?},");
        }
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
    // matches. And the half pixel: the spans in 20 px and 18 px lines (#s,
    // #bs), and the inline-block on a line with a 16px strut (#t, and the
    // line's box around it).
    check(
        "text",
        &[
            "div1 box ours 0,0,67.578125,22.5",
            "t2 box ours 0,2.5,67.578125,20",
            "s9 box ours 9.859375,141.5,27.28125,17",
            "s9 offset ours 10,2,27,17",
            "bs14 box ours 145.84375,259.5,44.40625,19",
            "bs14 offset ours 146,0,44,19",
            "jas17 box ours 0,321,289.671875,42",
            "jas17 offset ours 276,1,290,42",
        ],
    );
}

/// The root's margin, an absolute box in a paragraph, a block `<svg>`,
/// `<img width height>`, shrink-to-fit, a CSS table, a list, two columns
/// (the paragraph split across them is its whole width, from the union of
/// its fragments), `pre`, `box-sizing`, `min-width` over `max-width`.
#[test]
fn gaps_of_blitz_match_chrome() {
    // The half pixel of an inline-block alone on a 16 px line (#mm).
    check(
        "gaps",
        &[
            "mm16 box ours 7,268.5,120,5",
            "mm16 offset ours 0,262,120,5",
        ],
    );
}

/// A relatively positioned block, translated and scaled blocks, and a
/// relatively positioned span: transforms and inline relative offsets,
/// which no fragment includes, added by the engine.
#[test]
fn transforms_and_relative_offsets_match_chrome() {
    // The half pixel of the span (#rs).
    check(
        "offsets",
        &[
            "rs5 box ours 9.859375,53.5,14.578125,17",
            "rs5 offset ours 10,54,15,17",
        ],
    );
}

/// e3-00's report page, 20 flex rows: every row, label and figure.
#[test]
fn the_report_page_matches_chrome() {
    check("report20", &[]);
}

/// Media `screen`, the media type Chrome's scripts see, with the page area as
/// the viewport (here Chrome's window; `an_author_page_size_is_the_viewport_
/// unlike_in_chrome` has where they part). `@media print` rules and `<style
/// media=print>` don't apply; screen rules and width and height queries do
/// (DECISIONS, E3-R1).
#[test]
fn the_measuring_cascade_is_screen_media_as_in_chrome() {
    // Raikiri's media queries know widths, heights and aspect ratios, not
    // `orientation`: neither `landscape` nor `portrait` matches (#o).
    check(
        "media",
        &["o4 box ours 0,34,10,5", "o4 offset ours 0,34,10,5"],
    );
}

/// `offsetLeft` and `offsetTop` (CSSOM View §7): from the offset parent's
/// padding edge; a positioned ancestor, `body`, a table cell or table for a
/// static element; transforms ignored; a fixed element from the viewport.
#[test]
fn offsets_match_chrome() {
    // The half pixel of the spans (#sp, #b, #rs, #it).
    check(
        "offset-parents",
        &[
            "sp4 box ours 25,51.5,50.890625,17",
            "sp4 offset ours 2,22,51,17",
            "b5 box ours 49.65625,51.5,26.234375,17",
            "b5 offset ours 27,22,26,17",
            "rs17 box ours 24,104.5,25.125,17",
            "rs17 offset ours 24,105,25,17",
            "it18 box ours 41.703125,104.5,7.421875,17",
        ],
    );
}

/// Coordinates past a page break. A script sees one continuous layout: the
/// engine paginates into one page long enough for the page, so no row moves
/// to a next page and nothing is cut. 60 rows of 22 px in a 600 px viewport,
/// then a box of 700 px and what follows it: where Chrome has them. (Stacked
/// 600 px pages would put row 27 at 600 where it is at 594, and so on down.)
#[test]
fn positions_past_page_breaks_are_the_continuous_ones() {
    check("breaks", &[]);
}

/// Forced page breaks (`break-before: page`, `page-break-after: always`,
/// `break-before: always`, a named page, a break before a table row) do
/// nothing in the layout a script measures, as on Chrome's screen; a forced
/// column break still breaks the column.
#[test]
fn forced_page_breaks_do_nothing_as_on_a_screen() {
    // Raikiri makes no box for a `<br>`, which in Chrome has the place and
    // height of its line break (and no offsets).
    check("forced-breaks", &["br18 box ours 0,0,0,0"]);
}

/// The page's height is the viewport's, though nothing is cut at it:
/// `height: 100%` on the root and `body`, `vh`, `bottom` of an absolute and
/// of a fixed box, and content past the first 600 px.
#[test]
fn the_page_height_is_the_viewport_height() {
    check("viewport-height", &[]);
}

/// A span on several lines, one with a relative offset, a quarter turn and a
/// scaled box in a translated one: `getBoundingClientRect` is around every
/// line and transformed, `offsetLeft` and `offsetTop` are the first line's
/// and `offsetWidth` and `offsetHeight` the untransformed size.
#[test]
fn inline_elements_on_several_lines_and_nested_transforms_match_chrome() {
    // The half pixel of the spans in 16 px lines (#s, #b, #r).
    check(
        "inline-lines",
        &[
            "s2 box ours 20,9.5,118.3125,49",
            "s2 offset ours 93,10,118,49",
            "b3 box ours 49.25,41.5,56.21875,17",
            "b3 offset ours 49,42,56,17",
            "r5 box ours 8,57.5,115.921875,33",
            "r5 offset ours 49,0,116,33",
        ],
    );
}

/// Offsets when the offset parent is `body`: Chromium measures from the root
/// element's border edge (a margin on the root moves nothing), from `body`'s
/// border edge when `body` is positioned, and from the initial containing
/// block for an absolute box no element contains; the root element's own
/// offsets are its border edge's. A fixed box inside a transform has the
/// transformed element as its offset parent.
#[test]
fn offsets_from_body_follow_the_containing_blocks_as_in_chrome() {
    // The half pixel of the spans (#sp, #b; #sp in the second page).
    check(
        "root-offsets",
        &[
            "sp11 box ours 42.859375,50.5,56.640625,17",
            "sp11 offset ours 34,44,57,17",
            "b12 box ours 73.265625,50.5,26.234375,17",
            "b12 offset ours 64,44,26,17",
        ],
    );
    check(
        "body-positioned",
        &[
            "sp9 box ours 35.859375,41.5,27.28125,17",
            "sp9 offset ours 17,27,27,17",
        ],
    );
    check("root-positioned", &[]);
}

/// Forced page breaks on `::before` and `::after`, the page values `left`,
/// `right`, `recto` and `all`, a named page on a pseudo-element, `orphans`
/// and `widows` no paragraph can meet: none of them moves anything, as on
/// Chrome's screen.
#[test]
fn page_breaks_on_pseudo_elements_do_nothing_either() {
    check("pseudo-breaks", &[]);
}

/// What only a paged medium has (CSS GCPM 3), and Raikiri knows where no
/// browser does: `position: running()`, `float: footnote`, and `content`
/// with `string()`, `element()`, `leader()`, `target-counter()` or
/// `target-text()`. Chromium drops each declaration, so an earlier one
/// still holds (`position: absolute; position: running(x)` is absolute); the
/// measuring copy gets the text without them (`src/sheet_unpaged.rs`).
/// `string-set`, `bookmark-*`, a page float and `@page` margin boxes move
/// nothing.
#[test]
fn paged_media_declarations_are_dropped_as_a_browser_drops_them() {
    // The half pixel of the inline elements (#f, #t, #t2, #t3).
    check(
        "paged-only",
        &[
            "f4 box ours 24.65625,24.5,25.78125,17",
            "f4 offset ours 25,25,26,17",
            "t10 box ours 0,82.5,20.015625,17",
            "t10 offset ours 0,83,20,17",
            "t212 box ours 0,98.5,20.015625,17",
            "t212 offset ours 0,99,20,17",
            "t314 box ours 0,114.5,20.015625,17",
            "t314 offset ours 0,115,20,17",
        ],
    );
}

/// Boxes above the page area's top edge, where a script measures text off
/// screen: `position: absolute` at -9999px and at -99999px, a box half above
/// the edge, a fixed box, `translate`, `position: relative`, the clip
/// pattern for hidden text, a first line's span (1 px above the edge), and
/// a first block with a negative margin, which takes `body` with it.
/// Raikiri's fragments begin at their page's top, so the engine's one page
/// begins 131,072 px above the flow's start.
#[test]
fn boxes_above_the_page_top_are_where_chrome_has_them() {
    // The half pixel of the inline elements (#fs, #k, #k2, #k3, #wm, #ps).
    check(
        "above-top",
        &[
            "fs2 box ours 9.859375,-0.5,27.28125,17",
            "fs2 offset ours 10,0,27,17",
            "k6 box ours 0,-9999.5,6.9375,17",
            "k6 offset ours 0,0,7,17",
            "k28 box ours 100,-10.5,6.9375,17",
            "k28 offset ours 0,0,7,17",
            "k313 box ours 5000,9.5,6.9375,17",
            "k313 offset ours 0,0,7,17",
            "wm21 box ours 0,75.5,70.15625,17",
            "wm21 offset ours 0,76,70,17",
            "ps24 box ours 9.859375,-64.5,27.28125,17",
            "ps24 offset ours 10,0,27,17",
        ],
    );
    check("margin-above", &[]);
}

/// A kept gap: Raikiri keeps every length and position within 10,000,000 px
/// and says nothing, so a page with a box that reaches it would get wrong
/// numbers (the box after a 17,000,000 px one at 10,000,000). The engine
/// doesn't lay such a page out, and every element answers as with no box,
/// where Chrome has these numbers.
#[test]
fn a_page_longer_than_raikiri_lays_out_answers_as_with_no_box() {
    check(
        "too-long",
        &[
            "html box ours 0,0,0,0",
            "html offset ours 0,0,0,0",
            "body0 box ours 0,0,0,0",
            "body0 offset ours 0,0,0,0",
            "big1 box ours 0,0,0,0",
            "big1 offset ours 0,0,0,0",
            "a2 box ours 0,0,0,0",
            "a2 offset ours 0,0,0,0",
        ],
    );
}

/// `offsetLeft`, `offsetTop`, `offsetWidth` and `offsetHeight` of boxes at
/// and of fractions of a pixel, negative and positive: the nearest whole
/// number, and of two equally near the greater (-2.5 is -2, 2.5 is 3).
#[test]
fn offsets_and_sizes_round_as_chrome_rounds_them() {
    check("fractions", &[]);
}

/// What contains a fixed or an absolute box and so is its offset parent:
/// `will-change: transform`, `contain: paint` and `contain: layout`, as in
/// Chrome. Gaps, all Raikiri's: `translate`, `scale` and `rotate` as
/// properties of their own do nothing (#m, the fixed box in it, #ro), and
/// `perspective` contains no fixed box (#fx3).
#[test]
fn containers_of_fixed_boxes_match_chrome_but_for_what_raikiri_lacks() {
    check(
        "fixed-containers",
        &[
            "m1 box ours 0,0,40,30",
            "fx2 box ours 2,1,3,3",
            "fx36 box ours 2,1,3,3",
            "ro11 box ours 50,49,40,10",
        ],
    );
}

/// A deliberate deviation, not a gap: the viewport a script measures in is
/// the page area the page prints on (DECISIONS, E3-R1). With an author
/// `@page { size: 400px 300px; margin: 50px }` that is 300 by 200 px, where
/// Chrome's scripts keep seeing its 800 by 600 window: widths, `vw`, `vh`
/// and a fixed box's corner follow the page.
#[test]
fn an_author_page_size_is_the_viewport_unlike_in_chrome() {
    check(
        "at-page",
        &[
            "html box ours 0,0,300,110",
            "html offset ours 0,0,300,110",
            "body0 box ours 0,0,300,110",
            "body0 offset ours 0,0,300,110",
            "a1 box ours 0,0,300,10",
            "a1 offset ours 0,0,300,10",
            "v2 box ours 0,10,150,100",
            "v2 offset ours 0,10,150,100",
            "fx3 box ours 295,195,5,5",
            "fx3 offset ours 295,195,5,5",
        ],
    );
}

/// A table with a caption, a header group and a column: the table's box is
/// its wrapper's, around the caption (Raikiri's fragment is the grid's), and
/// its parts are measured from there. A gap, Raikiri's: `col` and `colgroup`
/// have no box.
#[test]
fn a_table_with_a_caption_and_a_header_matches_chrome_but_for_columns() {
    check(
        "table-parts",
        &[
            "cg3 box ours 0,0,0,0",
            "cg3 offset ours 0,0,0,0",
            "co4 box ours 0,0,0,0",
            "co4 offset ours 0,0,0,0",
        ],
    );
}

/// More of what Raikiri lays out differently, each on its own so that one
/// doesn't move the next: `zoom` does nothing (#z, its span and the box
/// around it), a span in vertical text is 4 px to the right (#vs), a
/// singular transform leaves no box where Chrome has an empty rectangle at
/// its place (#z0, #zx), and `position: sticky; bottom: 0` isn't stuck to
/// the viewport's bottom (#s2). The half pixel of empty inline elements and
/// of a right-to-left span (#es, #et, #eu, #rs). `display: contents`, a
/// list's items and `position: sticky; top: 0` are Chrome's.
#[test]
fn more_of_what_raikiri_lays_out_differently() {
    check(
        "more-gaps",
        &[
            "div1 box ours 0,1400,400,40",
            "div1 offset ours 0,1400,400,40",
            "z2 box ours 5,1405,40,30",
            "s3 box ours 5,1404.5,6.34375,17",
            "vs6 box ours 20,1500,17,41.265625",
            "vs6 offset ours 20,0,17,41",
            "z09 box ours 0,0,0,0",
            "zx10 box ours 0,0,0,0",
            "es16 box ours 6.734375,1899.5,0,17",
            "es16 offset ours 7,0,0,17",
            "et17 box ours 17.234375,1899.5,6,17",
            "et17 offset ours 17,0,6,17",
            "eu18 box ours 29,1899.5,0,17",
            "eu18 offset ours 29,0,0,17",
            "rs21 box ours 25.765625,1999.5,174.234375,33",
            "rs21 offset ours 26,0,174,33",
            "s228 box ours 0,1010,800,10",
            "s228 offset ours 0,1010,800,10",
        ],
    );
}

/// Line heights that aren't a whole 1/64 px. Chromium lays lines out in
/// 1/64 px: `line-height: 1.4` at 13px is 18.1875 px a line (rounded
/// down), `line-height: 1.3em` at 14px 18.203125 (a length: the nearest).
/// Raikiri keeps 18.2, so a page drifted a pixel from Chromium every 30 to
/// 60 lines; the measuring cascade gives it the line height Chromium uses
/// (`snapped_line_height`). What is left is the half pixel of inline boxes,
/// here of a fractional leading: Chromium rounds the upper half of the
/// leading down to a whole pixel (in `15px/1.5`, 1.25 px is 1), Raikiri
/// keeps it (#s, 0.265625 px lower; offsetTop one more on 22 of 40 spans).
/// No block, paragraph or line box differs.
#[test]
fn line_heights_in_fractions_of_a_pixel_match_chrome_but_for_the_half_pixel() {
    check_low(ADVANCES, "article", &[("s", 0.265625)], 60, &[]);
    check_low(ACROSS, "line-height-13", &[("s", 0.09375)], 63, &[]);
    check_low(ACROSS, "line-height-wrap", &[("last", 0.09375)], 1, &[]);
    // 16px/1.2, `normal` at 15px (whole pixels in both), 18.3px at 13px,
    // 1.3em at 14px, and lines of integer and fractional products.
    check_low(
        ACROSS,
        "line-heights",
        &[
            ("as", 0.59375),
            ("alast", 0.59375),
            ("cs", 0.15625),
            ("clast", 0.15625),
            ("ds", 0.609375),
            ("dlast", 0.609375),
        ],
        148,
        &[],
    );
}

/// The height of every `p` of page `name` that differs from Chrome's by
/// more than a twentieth of a pixel, as `(id, ours, Chrome's)`.
fn paragraph_heights(name: &'static str) -> Vec<(String, f64, f64)> {
    on_layout_thread(move || {
        let chrome = probe(&format!("{name}.chrome.json"));
        runtime(&probe(&format!("{name}.html")))
            .eval::<String>(&format!(
                "const chrome = ({chrome}).elements.filter((r) => r[0] === 'p');
                 [...document.querySelectorAll('p')].map((p, i) => {{
                     const h = p.getBoundingClientRect().height;
                     return Math.abs(h - chrome[i][5]) <= 0.05 ? '' : p.id + ' ' + h + ' ' + chrome[i][5];
                 }}).filter((l) => l).join('\\n')"
            ))
            .unwrap()
            .lines()
            .map(|l| {
                let mut w = l.split(' ');
                let id = w.next().unwrap_or_default().to_owned();
                let mut n = || w.next().and_then(|v| v.parse().ok()).unwrap_or(f64::NAN);
                (id, n(), n())
            })
            .collect()
    })
}

/// Asserts that the paragraphs of page `name` are Chrome's height but for
/// `pinned`, each `(id, our height)`, whose Chrome height differs.
fn check_heights(name: &'static str, pinned: &[(&str, f64)]) {
    let got = paragraph_heights(name);
    let ours: Vec<(&str, f64)> = got.iter().map(|(id, h, _)| (id.as_str(), *h)).collect();
    assert_eq!(ours, pinned, "{name}: {got:#?}");
}

/// Every span of page `name` whose top, from the top of the nearest ancestor
/// with an id (its paragraph), isn't Chrome's to a twentieth of a pixel, as
/// `(that ancestor's id and the span's place in it, our offset, Chrome's)`.
/// The paragraphs' heights (`paragraph_heights`) don't say where on its line a
/// span of another font size sits, and the spans' tops on the page add up
/// every paragraph height above them.
fn span_tops(name: &'static str) -> Vec<(String, f64, f64)> {
    on_layout_thread(move || {
        let chrome = probe(&format!("{name}.chrome.json"));
        runtime(&probe(&format!("{name}.html")))
            .eval::<String>(&format!(
                "const all = ({chrome}).elements, chrome = all.filter((r) => r[0] === 'span');
                 const top = (id) => all.find((r) => r[1] === id)[3];
                 [...document.querySelectorAll('span')].map((s, i) => {{
                     const p = s.closest('[id]'), at = [...p.querySelectorAll('span')].indexOf(s);
                     const ours = s.getBoundingClientRect().y - p.getBoundingClientRect().y;
                     const theirs = chrome[i][3] - top(p.id);
                     return Math.abs(ours - theirs) <= 0.05 ? '' : p.id + '/' + at + ' ' + ours + ' ' + theirs;
                 }}).filter((l) => l).join('\\n')"
            ))
            .unwrap()
            .lines()
            .map(|l| {
                let mut w = l.split(' ');
                let id = w.next().unwrap_or_default().to_owned();
                let mut n = || w.next().and_then(|v| v.parse().ok()).unwrap_or(f64::NAN);
                (id, n(), n())
            })
            .collect()
    })
}

/// Asserts that the spans of page `name` sit where Chrome's do on their
/// lines but for `pinned`, each `(paragraph id and span index, our offset
/// less Chrome's in 1/64 px)`.
fn check_span_tops(name: &'static str, pinned: &[(&str, i64)]) {
    let got = span_tops(name);
    let ours: Vec<(&str, i64)> = got
        .iter()
        .map(|(id, ours, theirs)| (id.as_str(), ((ours - theirs) * 64.0).round() as i64))
        .collect();
    assert_eq!(ours, pinned, "{name}: {got:#?}");
}

/// Where a span sits on its line, from its paragraph's top, against Chrome's
/// (E3-R5; `check_heights` above reads the paragraphs' heights only). The
/// same two gaps as there, seen from the spans, in 1/64 px: a constant
/// offset where half the leading isn't a whole pixel (Chromium rounds its
/// upper half down: "Inline boxes up to a pixel low" in RAIKIRI.md section
/// 9; alone in `q12`, `q14`, `q15`, `q17`, `q21`, `q25`, `q28` to `q31`,
/// `q33` and `m0` to `m2`), and an offset that grows by the line's difference
/// from Chromium's line box, 0.5 px a line, where a span of another font size
/// sits on a line whose line height is a length (the other paragraphs listed,
/// on top of a constant one in `q16` to `q27` and `m4`). Every other span is
/// Chrome's.
#[test]
fn spans_in_lines_of_mixed_sizes_are_pinned_where_they_differ() {
    check_span_tops(
        "line-height-shapes",
        &[
            ("q0/1", -32),
            ("q0/2", -64),
            ("q0/3", -96),
            ("q2/0", 32),
            ("q2/1", 64),
            ("q2/2", 96),
            ("q2/3", 128),
            ("q3/0", 32),
            ("q3/1", 64),
            ("q3/2", 96),
            ("q3/3", 128),
            ("q4/1", -32),
            ("q4/2", -64),
            ("q4/3", -96),
            ("q6/0", 32),
            ("q6/1", 64),
            ("q6/2", 96),
            ("q6/3", 128),
            ("q7/0", 32),
            ("q7/1", 64),
            ("q7/2", 96),
            ("q7/3", 128),
            ("q8/1", -32),
            ("q8/2", -64),
            ("q8/3", -96),
            ("q10/0", 32),
            ("q10/1", 64),
            ("q10/2", 96),
            ("q10/3", 128),
            ("q11/0", 32),
            ("q11/1", 64),
            ("q11/2", 96),
            ("q11/3", 128),
            ("q12/0", 48),
            ("q12/1", 48),
            ("q12/2", 48),
            ("q12/3", 48),
            ("q14/0", 32),
            ("q14/1", 32),
            ("q14/2", 32),
            ("q14/3", 32),
            ("q15/0", 32),
            ("q15/1", 32),
            ("q15/2", 32),
            ("q15/3", 32),
            ("q16/0", 48),
            ("q16/1", 80),
            ("q16/2", 112),
            ("q16/3", 144),
            ("q17/0", 48),
            ("q17/1", 48),
            ("q17/2", 48),
            ("q17/3", 48),
            ("q18/0", 16),
            ("q18/1", -16),
            ("q18/2", -48),
            ("q18/3", -80),
            ("q19/0", 16),
            ("q19/1", -16),
            ("q19/2", -48),
            ("q19/3", -80),
            ("q20/0", 7),
            ("q20/1", -25),
            ("q20/2", -57),
            ("q20/3", -89),
            ("q21/0", 7),
            ("q21/1", 7),
            ("q21/2", 7),
            ("q21/3", 7),
            ("q22/0", 39),
            ("q22/1", 71),
            ("q22/2", 103),
            ("q22/3", 135),
            ("q23/0", 39),
            ("q23/1", 71),
            ("q23/2", 103),
            ("q23/3", 135),
            ("q24/0", 7),
            ("q24/1", -25),
            ("q24/2", -57),
            ("q24/3", -89),
            ("q25/0", 7),
            ("q25/1", 7),
            ("q25/2", 7),
            ("q25/3", 7),
            ("q26/0", 39),
            ("q26/1", 71),
            ("q26/2", 103),
            ("q26/3", 135),
            ("q27/0", 39),
            ("q27/1", 71),
            ("q27/2", 103),
            ("q27/3", 135),
            ("q28/0", 6),
            ("q28/1", 6),
            ("q28/2", 6),
            ("q28/3", 6),
            ("q29/0", 13),
            ("q29/1", 13),
            ("q29/2", 13),
            ("q29/3", 13),
            ("q30/0", 32),
            ("q30/1", 32),
            ("q30/2", 32),
            ("q30/3", 32),
            ("q31/0", 19),
            ("q31/1", 19),
            ("q31/2", 19),
            ("q31/3", 19),
            ("q32/0", 32),
            ("q32/1", 64),
            ("q32/2", 96),
            ("q32/3", 128),
            ("q33/0", 32),
            ("q33/1", 32),
            ("q33/2", 32),
            ("q33/3", 32),
            ("q34/1", -32),
            ("q34/2", -64),
            ("q34/3", -96),
            ("q35/1", -32),
            ("q35/2", -64),
            ("q35/3", -96),
        ],
    );
    check_span_tops(
        "line-height-grid",
        &[
            ("m0/0", 61),
            ("m0/1", 61),
            ("m0/2", 61),
            ("m0/3", 61),
            ("m0/4", 61),
            ("m1/0", 32),
            ("m1/1", 32),
            ("m1/2", 32),
            ("m1/3", 32),
            ("m1/4", 32),
            ("m2/0", 6),
            ("m2/1", 6),
            ("m2/2", 6),
            ("m2/3", 6),
            ("m2/4", 6),
            ("m4/0", 39),
            ("m4/1", 71),
            ("m4/2", 103),
            ("m4/3", 135),
            ("m4/4", 167),
        ],
    );
}

/// Where Chromium's lines still differ from the snapped line height, each
/// paragraph's height pinned with ours, Chrome's from its file. Ten lines of
/// `x` each, or five of text with a span (`line-height-shapes` q0 to q35).
/// Gaps, Raikiri's line box against Chromium's, not modelled here:
///
/// - **A span of another font size on a line whose line height is a
///   length** (`px`, `%`, `em`): 0.5 px a line, taller or shorter (a
///   `<small>`, `<code>` or `<sup>` in text): over five lines 2 px (q0 to
///   q35; `line-height-grid` #m4, 2.5 px). With a number line height it is
///   Chrome's.
/// - **Percentages** whose lines Chromium makes shorter than the length:
///   `133.33%` and `162.5%` at 13, 14.3 and 16 px, 1/32 to 5/64 px a line
///   (q74 to q88, `line-height-grid` #c110).
///
/// Fractional font sizes (`14.3px`, `10.8px`, `0.9em`) and the 108 number
/// line heights at whole sizes are Chrome's: the snapping takes the font
/// size in 1/64 px first.
#[test]
fn mixed_font_sizes_and_percentages_are_pinned_where_lines_differ() {
    check_heights(
        "line-height-grid",
        &[("c110", 173.28125), ("m4", 103.515625)],
    );
    check_heights(
        "line-height-shapes",
        &[
            ("q0", 96.0),
            ("q2", 100.0),
            ("q3", 108.0),
            ("q4", 106.0),
            ("q6", 110.0),
            ("q7", 118.0),
            ("q8", 126.0),
            ("q10", 130.0),
            ("q11", 138.0),
            ("q16", 103.5),
            ("q18", 107.5),
            ("q19", 115.5),
            ("q20", 97.015625),
            ("q22", 101.015625),
            ("q23", 109.015625),
            ("q24", 97.015625),
            ("q26", 101.015625),
            ("q27", 109.015625),
            ("q32", 91.0),
            ("q34", 95.0),
            ("q35", 103.0),
            ("q74", 173.28125),
            ("q76", 211.25),
            ("q80", 190.625),
            ("q82", 232.34375),
            ("q86", 213.28125),
            ("q88", 260.0),
        ],
    );
}

/// Lengths of 10,000,000 px and more that Raikiri's clamp changes nothing
/// of: `max-width` and `max-height` at 99,999,999 px (the "no limit"
/// idiom), and lengths in a `display: none` subtree. These pages are laid
/// out, as Chromium lays them out.
#[test]
fn a_huge_maximum_or_a_hidden_huge_box_doesnt_refuse_the_page() {
    for name in [
        "far-max-width",
        "far-max-height",
        "far-hidden",
        "far-hidden-child",
    ] {
        check(name, &[]);
    }
}

/// What a browser does with the values only a paged medium has when they
/// come through `var()`, `attr()` or an escape. A literal one (`float:
/// f\6fotnote` included) is dropped as it is parsed, and an earlier
/// declaration holds; one that `var()` brings in, or a `content` with
/// `attr()` and a paged function, is kept and invalid once substituted, so
/// the property has its initial value: the span stays in its line, the box
/// static, the `::after` has no content (`src/sheet_unpaged.rs`).
/// `@supports` on them is false; at the top level and in `@media` a
/// statement `float: footnote; #a11 { }` is one rule Chromium drops. The
/// only differences left are the half pixel of the inline elements.
#[test]
fn paged_values_through_var_attr_and_escapes_are_what_chrome_makes_of_them() {
    check_low(
        ACROSS,
        "paged-float",
        &[
            ("fn", 0.5),
            ("sa", 0.5),
            ("sb", 0.5),
            ("sd", 0.5),
            ("se", 0.5),
            ("sf", 0.5),
            ("sg", 0.5),
        ],
        36,
        &[],
    );
    check_low(ACROSS, "paged-var", &[("f", 0.5)], 4, &[]);
    check_low(ADVANCES, "paged-content", &[("c", 0.5)], 36, &[]);
    check_low(ACROSS, "paged-attr", &[("c", 0.5)], 20, &[]);
    check("paged-supports", &[]);
    check_low(ACROSS, "paged-supports-not", &[("n", 0.5)], 2, &[]);
    check("paged-rules", &[]);
}

/// The refusal of a page with a box Raikiri can't place, upward too. A box
/// that begins at or above the top of the engine's page (131,072 px above
/// the page area's) was cut there, and a length Raikiri clamps (10,000,000
/// px, here a margin, `top` and `position: relative`) places what follows
/// from it wrong: such a page answers as with no box, where Chrome has
/// these numbers. A box wholly above that page and placed from nothing
/// else has no fragment: it is a page with a `top` past the clamp
/// (`above-far-abs`), refused for that.
#[test]
fn a_page_with_a_box_cut_at_the_top_or_clamped_answers_as_with_no_box() {
    for name in [
        "above-cut",
        "above-cut-more",
        "above-far",
        "above-far-child",
        "above-far-abs",
    ] {
        check_refused(name);
    }
}

/// Lengths in fractions of 1/64 px. Chromium cuts a length to 1/64 px
/// toward zero (`left: -0.51px` is -0.5) before it rounds an offset, and
/// so does `layout.rs`: the x side matches. The y side doesn't where the
/// fraction is finer than 1/64 px: the engine's page begins 131,072 px up,
/// where an `f32` holds 1/64 px and rounds to the nearest (-0.51 becomes
/// -0.515625, so `offsetTop` is -1 where Chromium has 0; 2.4999 becomes 2.5,
/// so 3 where Chromium has 2). Ours, for Raikiri's fragments.
#[test]
fn fractions_of_a_64th_round_as_chrome_rounds_them_across_not_down() {
    check(
        "round-trunc",
        &[
            "v02 offset ours 0,-1,5,4",
            "v13 offset ours -1,-2,5,4",
            "v24 offset ours -2,-3,5,4",
            "v35 offset ours 0,-1,5,4",
            "v810 offset ours 2,3,5,4",
        ],
    );
}

/// Two of Raikiri's gaps. An inline element that holds only an absolutely
/// positioned box (a `label` around the visually-hidden pattern) answers
/// the container's width by 0, where Chromium has 0 by 0 (`label-abs`,
/// `sr-only`). A `table-footer-group`'s `height` does nothing: the group is
/// 16 px high where Chromium has 20, and what follows is 4 px higher.
#[test]
fn an_inline_around_an_absolute_box_and_a_footer_group_height_are_raikiris_gaps() {
    check_low(
        ACROSS,
        "label-abs",
        &[("i", 0.5)],
        2,
        &[
            "l1 box ours 0,0,800,0",
            "l1 offset ours 0,0,800,0",
            "sp3 box ours 0,0,800,0",
            "sp3 offset ours 0,0,800,0",
            "an5 box ours 0,0,800,0",
            "an5 offset ours 0,0,800,0",
        ],
    );
    check(
        "sr-only",
        &[
            "label3 box ours 0,16,800,0",
            "label3 offset ours 0,16,800,0",
        ],
    );
    check(
        "table-footer",
        &[
            "html box ours 0,0,800,177",
            "html offset ours 0,0,800,177",
            "body0 box ours 0,0,800,177",
            "body0 offset ours 0,0,800,177",
            "fg4 box ours 0,16,28.296875,16",
            "fg4 offset ours 0,16,28,16",
            "p5 box ours 0,32,800,16",
            "p5 offset ours 0,32,800,16",
            "s6 box ours 28.6875,31.5,5.75,17",
            "s6 offset ours 29,32,6,17",
            "t7 box ours 0,48,13.421875,62",
            "t7 offset ours 0,48,13,62",
            "tbody8 box ours 2,70,9.421875,18",
            "tr9 box ours 2,70,9.421875,18",
            "td10 box ours 2,70,9.421875,18",
            "th11 box ours 2,50,9.421875,18",
            "tr12 box ours 2,50,9.421875,18",
            "td13 box ours 2,50,9.421875,18",
            "tf14 box ours 2,90,9.421875,18",
            "tr15 box ours 2,90,9.421875,18",
            "td16 box ours 2,90,9.421875,18",
            "t217 box ours 0,110,13.375,62",
            "t217 offset ours 0,110,13,62",
            "tf218 box ours 2,152,9.375,18",
            "tr19 box ours 2,152,9.375,18",
            "td20 box ours 2,152,9.375,18",
            "tbody21 box ours 2,112,9.375,38",
            "tr22 box ours 2,112,9.375,18",
            "b223 box ours 2,112,9.375,18",
            "tr24 box ours 2,132,9.375,18",
            "td25 box ours 2,132,9.375,18",
            "end26 box ours 0,172,800,5",
            "end26 offset ours 0,172,800,5",
        ],
    );
}

/// `report(2000)` as `layout_real.rs` writes it: 2,000 rows of 22 px, over
/// what would be 74 page breaks. Its last row ends at 44,000 px in Chrome;
/// stacking Raikiri's 600 px pages would put it 444 px lower (RAIKIRI.md
/// section 2).
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
            "root-offsets",
            "body-positioned",
            "root-positioned",
            "forced-breaks",
            "viewport-height",
            "inline-lines",
            "pseudo-breaks",
            "paged-only",
            "above-top",
            "margin-above",
            "too-long",
            "fractions",
            "fixed-containers",
            "at-page",
            "table-parts",
            "more-gaps",
            "article",
            "line-heights",
            "round-trunc",
            "paged-float",
            "paged-var",
            "paged-content",
            "paged-attr",
            "paged-rules",
            "above-cut",
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
        // The root element's row is not among those from `<body>` down.
        .filter(|line| !line.starts_with("\"root\""))
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
        "root-offsets",
        "body-positioned",
        "root-positioned",
        "forced-breaks",
        "viewport-height",
        "inline-lines",
        "pseudo-breaks",
        "paged-only",
        "above-top",
        "margin-above",
        "fractions",
        "fixed-containers",
        "at-page",
        "table-parts",
        "more-gaps",
        "article",
        "line-height-13",
        "line-heights",
        "paged-float",
        "paged-var",
        "paged-content",
        "paged-attr",
        "paged-rules",
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
