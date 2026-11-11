//! The E3-00 prototype: `getBoundingClientRect`, `offsetWidth` and
//! `offsetHeight` answered from Blitz's layout under the internal
//! `LayoutPolicy::Real`.
//!
//! Expected numbers come from outside this crate: CSS 2.1's box model
//! arithmetic for boxes with fixed sizes (§8.1 box dimensions, §10.3.3 and
//! §10.6.3 for block-level boxes in normal flow, §9.4.1 for their stacking),
//! CSSOM View §6.1 and §7 for which box a member reports, and Chromium
//! 153's measurement of the fixture fonts in
//! `crates/gfx/tests/data/chrome_measure_text.json` for text.
//!
//! Every layout here runs on one thread that lives as long as the process
//! (`on_layout_thread`), as `crates/gfx/tests/fulgur_layout.rs` does and as
//! the facade's page thread is: the style engine keeps a little per-thread
//! state for as long as the thread lives.

mod common;

use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::thread;

use blitz_dom::FontContext;
use common::limits;
use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
use surimono_dom::{Document, DomLimits, DomRuntime, LayoutPolicy, LayoutSetup};

type Job = Box<dyn FnOnce() + Send>;

/// The page thread's stack in the facade: `Limits::stack_bytes` and 8 MiB
/// (`crates/surimono/src/page.rs`).
const STACK: usize = 8 * 1024 * 1024 + 512 * 1024;

/// Runs `f` on the one layout thread and returns its result; a panic in `f`
/// is raised again here.
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

fn font_file(name: &str) -> Vec<u8> {
    let path = format!("{}/../../fixtures/fonts/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The fixture fonts and nothing else: no system fonts.
fn fonts() -> FontContext {
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
    ctx
}

/// An 800 by 600 viewport.
fn setup() -> LayoutSetup {
    LayoutSetup {
        width: 800,
        height: 600,
        fonts: fonts(),
    }
}

fn runtime_with(html: &str, limits_dom: DomLimits) -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.set_layout_policy(LayoutPolicy::Real);
    rt.set_layout_setup(setup());
    rt.load(Document::parse_with_limits(html, limits_dom).unwrap());
    rt
}

fn runtime(html: &str) -> DomRuntime {
    runtime_with(html, DomLimits::default())
}

/// `x,y,width,height` of the element with id `id`. A `var`, so that one
/// runtime can take it in several scripts.
const RECT: &str = "var rect = (id) => { const r = document.getElementById(id).getBoundingClientRect(); \
     return [r.x, r.y, r.width, r.height].join(); };";

const BOXES: &str = "<!doctype html><html><head><style>\
    body { margin: 0 }\
    #a { width: 200px; height: 50px; margin: 10px 0 0 20px; padding: 5px; border: 1px solid black }\
    #b { width: 50%; height: 30px }\
    #c { display: none; width: 10px; height: 10px }\
    </style></head><body><div id=a></div><div id=b></div>\
    <div id=c><p id=d>x</p></div></body></html>";

#[test]
fn block_boxes_with_fixed_sizes_follow_css_2_box_arithmetic() {
    let got: String = on_layout_thread(|| {
        let mut rt = runtime(BOXES);
        rt.eval(&format!(
            "{RECT} const a = document.getElementById('a'), b = document.getElementById('b');
             [rect('a'), rect('b'), a.offsetWidth, a.offsetHeight, b.offsetWidth, b.offsetHeight,
              a.getBoundingClientRect() instanceof DOMRect].join(' ')"
        ))
        .unwrap()
    });
    // #a: border box 200 + 2*5 + 2*1 = 212 wide and 50 + 2*5 + 2*1 = 62 high
    // (CSS 2.1 §8.1), at its margins' 20 and 10 from the viewport's corner.
    // #b: 50% of the 800px containing block (§10.3.3), below #a's border
    // box at 10 + 62 = 72 (§9.4.1; #a has no bottom margin).
    assert_eq!(got, "20,10,212,62 0,72,400,30 212 62 400 30 true");
}

#[test]
fn an_element_with_no_box_answers_zeros() {
    let got: String = on_layout_thread(|| {
        let mut rt = runtime(BOXES);
        rt.eval(&format!(
            "{RECT} const loose = document.createElement('div');
             loose.style.cssText = 'width: 30px; height: 30px';
             const c = document.getElementById('c');
             [rect('c'), rect('d'), c.offsetWidth, c.offsetHeight,
              [loose.getBoundingClientRect().width, loose.offsetWidth].join()].join(' ')"
        ))
        .unwrap()
    });
    // CSSOM View §6.1: no layout box, so a rectangle of zeros; §7: offsetWidth 0.
    // `display: none` (#c), a descendant of it (#d), and an element outside the document.
    assert_eq!(got, "0,0,0,0 0,0,0,0 0 0 0,0");
}

#[test]
fn positioned_boxes_table_cells_and_fallback_content() {
    let got: String = on_layout_thread(|| {
        let mut rt = runtime(
            "<!doctype html><html><body style=\"margin:0\">\
             <div style=\"position:relative;margin:30px;height:40px\">\
             <div id=p style=\"position:absolute;left:5px;top:6px;width:7px;height:8px\"></div></div>\
             <table style=\"border-spacing:0\"><tr><td style=\"padding:0;width:40px;height:20px\"></td>\
             <td id=c style=\"padding:0;width:60px;height:20px\"></td></tr></table>\
             <canvas id=v style=\"display:block;width:50px;height:20px\"><b id=f>no canvas</b></canvas>\
             </body></html>",
        );
        rt.eval(&format!(
            "{RECT} [rect('p'), rect('c'), rect('v').split(',').slice(2).join(), rect('f')].join(' ')"
        ))
        .unwrap()
    });
    // #p: its containing block is the padding box of the positioned div, at
    // its 30px margins; `left` and `top` are from there (CSS 2.1 §10.1, §9.3.2).
    // #c: the second cell starts where the first, 40px wide, ends, in a table
    // below the div and its margins, 30 + 40 + 30 (§17.5).
    // #v: sized by CSS here (see the ignored test below for its attributes);
    // its children are fallback content with no boxes (HTML §4.12.5).
    assert_eq!(got, "35,36,7,8 40,100,60,20 50,20 0,0,0,0");
}

/// A gap, kept with the expectation it fails: an inline element in running
/// text has no box of its own in this layout yet, so the prototype answers
/// zeros for it (DESIGN.md, "What blitz-dom 0.2.4 can't answer"). E3-01
/// reads its rectangles from the line's layout and takes this `ignore` away.
#[test]
#[ignore = "gap: an inline element's rectangles aren't read from the line layout yet"]
fn an_inline_element_has_the_rectangle_of_its_text() {
    let got: String = on_layout_thread(|| {
        let mut rt = runtime(
            "<!doctype html><html><body style=\"margin:0\">\
             <p style=\"margin:0;font:12px 'Noto Sans';line-height:20px\">a <span id=s>span</span></p>\
             </body></html>",
        );
        rt.eval(
            "const s = document.getElementById('s'), r = s.getBoundingClientRect();
             [r.x, r.y, r.width, r.height, s.getClientRects().length, s.offsetWidth, s.offsetHeight].join()",
        )
        .unwrap()
    });
    // Chromium 153.0.8010.12, this page with fixtures/fonts/NotoSans-Regular.ttf
    // as 'Noto Sans' in an 800 by 600 window (measured 11 Nov 2026 with
    // Playwright): CSSOM View §6.1 gives an inline box the union of its line
    // boxes' fragments, to the right of "a " and as high as the font's box.
    assert_eq!(got, "9.859375,1,27.28125,17,1,27,17");
}

/// A gap, kept with the expectation it fails: blitz-dom 0.2.4 doesn't size a
/// canvas from its attributes when the box is block-level (it answers 800 by
/// 0 here) and makes no box for an inline one. DESIGN.md, "What blitz-dom
/// 0.2.4 can't answer", has the way out for E3-01, which takes this
/// `ignore` away.
#[test]
#[ignore = "gap: blitz-dom 0.2.4 ignores a block canvas's width and height attributes"]
fn a_canvas_has_the_size_of_its_attributes() {
    let got: String = on_layout_thread(|| {
        let mut rt = runtime(
            "<!doctype html><html><body style=\"margin:0\">\
             <canvas id=v width=50 height=20 style=\"display:block\"></canvas>\
             <canvas id=w width=50 height=20></canvas><canvas id=d></canvas></body></html>",
        );
        rt.eval(&format!(
            "{RECT} ['v', 'w', 'd'].map((id) => rect(id).split(',').slice(2).join()).join(' ')"
        ))
        .unwrap()
    });
    // HTML §4.12.5: a canvas's natural dimensions are its bitmap's, which
    // `width` and `height` give, 300 by 150 without them.
    assert_eq!(got, "50,20 50,20 300,150");
}

#[test]
fn text_is_measured_with_the_fonts_given() {
    let got: String = on_layout_thread(|| {
        let mut rt = runtime(
            "<!doctype html><html><body style=\"margin:0\">\
             <span id=t style=\"display:inline-block;white-space:pre;\
             font:12px 'Noto Sans';line-height:20px\">Hello, world</span></body></html>",
        );
        rt.eval(&format!("{RECT} rect('t')")).unwrap()
    });
    let n: Vec<f64> = got.split(',').map(|v| v.parse().unwrap()).collect();
    // Chromium 153 measures this text in this font at 67.5725px
    // (crates/gfx/tests/data/chrome_measure_text.json, the first case). An
    // inline-block shrinks to fit its text (CSS 2.1 §10.3.9); Blitz rounds
    // that width up to a whole pixel, so the box is at most one pixel wider.
    let chrome = 67.572_494_506_835_94;
    assert!(n[2] >= chrome && n[2] < chrome + 1.0, "width {}", n[2]);
    // One line box of `line-height: 20px` (CSS 2.1 §10.8.1).
    assert_eq!((n[0], n[1], n[3]), (0.0, 0.0, 20.0), "{got}");
}

#[test]
fn text_wraps_at_the_box_width() {
    let got: String = on_layout_thread(|| {
        let mut rt = runtime(
            "<!doctype html><html><body style=\"margin:0\">\
             <p id=n style=\"margin:0;width:100px;font:12px 'Noto Sans';line-height:20px\">\
             Hello, world Hello, world</p>\
             <p id=w style=\"margin:0;width:200px;font:12px 'Noto Sans';line-height:20px\">\
             Hello, world Hello, world</p></body></html>",
        );
        rt.eval(&format!("{RECT} rect('n') + ' ' + rect('w')"))
            .unwrap()
    });
    // "Hello, world" is 67.57px in this font (Chromium 153,
    // chrome_measure_text.json), so two of them and a space are over 135px:
    // too long for one 100px line and short enough for two, and they fit one
    // 200px line. Each line box is `line-height` high (CSS 2.1 §10.8), and
    // the second paragraph starts under the first (§9.4.1).
    assert_eq!(got, "0,0,100,40 0,40,200,20");
}

#[test]
fn layout_runs_when_asked_and_again_only_after_a_change() {
    let (got, counts): (String, Vec<u64>) = on_layout_thread(|| {
        let mut rt = runtime(BOXES);
        let mut counts = vec![rt.layouts_run()];
        let mut out = Vec::new();
        let mut step = |rt: &mut DomRuntime, script: &str| {
            let s: String = rt.eval(&format!("{RECT} {script}")).unwrap();
            out.push(s);
            counts.push(rt.layouts_run());
        };
        // No query, no layout.
        step(&mut rt, "document.getElementById('a').id");
        // Two queries, one layout.
        step(&mut rt, "rect('a') + ' ' + rect('b')");
        // Reads don't make it stale.
        step(
            &mut rt,
            "document.getElementById('a').getAttribute('id') + rect('a')",
        );
        // A style attribute: 100 + 2*5 + 2*1 wide; #b moves up with #a's height.
        step(
            &mut rt,
            "document.getElementById('a').style.height = '20px';
             document.getElementById('a').style.width = '100px'; rect('a') + ' ' + rect('b')",
        );
        // A class a sheet's rule selects, the rule added through the CSSOM.
        step(
            &mut rt,
            "document.querySelector('style').sheet.insertRule('.wide { width: 300px !important }', 4);
             rect('b')",
        );
        step(
            &mut rt,
            "document.getElementById('b').className = 'wide'; rect('b')",
        );
        // The tree: a 40px block before #a pushes both down.
        step(
            &mut rt,
            "const n = document.createElement('div'); n.style.height = '40px';
             document.body.insertBefore(n, document.body.firstChild); rect('a') + ' ' + rect('b')",
        );
        // A sheet's text.
        step(
            &mut rt,
            "document.querySelector('style').textContent = 'body { margin: 0 } #b { height: 7px }';
             rect('b')",
        );
        (out.join(" | "), counts)
    });
    assert_eq!(
        got,
        "a | 20,10,212,62 0,72,400,30 | a20,10,212,62 | 20,10,112,32 0,42,400,30 | \
         0,42,400,30 | 0,42,300,30 | 20,50,112,32 0,82,300,30 | 0,60,800,7"
    );
    assert_eq!(counts, [0, 0, 1, 1, 2, 3, 4, 5, 6]);
}

#[test]
fn text_changes_make_the_layout_stale() {
    let got: String = on_layout_thread(|| {
        let mut rt = runtime(
            "<!doctype html><html><body style=\"margin:0\">\
             <span id=t style=\"display:inline-block;white-space:pre;\
             font:12px 'Noto Sans';line-height:20px\">Hello, world</span></body></html>",
        );
        rt.eval(
            "const t = document.getElementById('t');
             const before = t.getBoundingClientRect().width;
             t.firstChild.data = 'Hello, world Hello, world';
             const after = t.getBoundingClientRect().width;
             t.firstChild.data = 'Hello, world';
             [after > before * 1.9, t.getBoundingClientRect().width === before].join()",
        )
        .unwrap()
    });
    // Twice the text is about twice as wide (a space more); the same text
    // again is as wide as it was.
    assert_eq!(got, "true,true");
}

#[test]
fn the_same_page_gives_the_same_numbers_twice() {
    let page = "<!doctype html><html><head><style>body { margin: 8px; font: 14px 'Noto Sans' }\
        .row { display: flex; gap: 7px } .row > div { flex: 1; padding: 3px }\
        table { border-collapse: collapse } td { padding: 2px 5px }</style></head><body>\
        <h1 id=h>Quarterly report</h1><p id=p>The quick brown fox jumps over the lazy dog, \
        and then does it again to fill a second line of this paragraph for the test.</p>\
        <div class=row id=r><div id=r1>one</div><div id=r2>two two</div><div id=r3>three</div></div>\
        <table id=t><tr><td id=c1>Revenue</td><td id=c2>1,234</td></tr></table></body></html>";
    let run = move || {
        on_layout_thread(move || {
            let mut rt = runtime(page);
            let first: String = rt
                .eval(&format!(
                    "{RECT} var all = () => ['h','p','r','r1','r2','r3','t','c1','c2'].map(rect).join(' '); all()"
                ))
                .unwrap();
            // A change and its undoing lay out again, from nothing.
            let again: String = rt
                .eval(&format!(
                    "{RECT} var all = () => ['h','p','r','r1','r2','r3','t','c1','c2'].map(rect).join(' ');
                     document.body.className = 'x'; all(); document.body.className = ''; all()"
                ))
                .unwrap();
            assert_eq!(rt.layouts_run(), 3);
            (first, again)
        })
    };
    let (first, again) = run();
    let (second, _) = run();
    // For a look across processes: run twice with `--nocapture` and compare.
    println!("numbers: {first}");
    assert_eq!(first, again);
    assert_eq!(first, second);
    // Something was laid out: nine boxes, none of them empty.
    assert_eq!(first.split(' ').count(), 9);
    assert!(!first.contains(",0,") && !first.ends_with(",0"), "{first}");
}

#[test]
fn zeros_stays_the_default_and_real_needs_a_setup() {
    let got: (String, String, u64) = on_layout_thread(|| {
        let mut plain = DomRuntime::new(limits()).unwrap();
        plain.load(Document::parse(BOXES).unwrap());
        let zeros: String = plain
            .eval(&format!(
                "{RECT} rect('a') + ' ' + document.getElementById('a').offsetWidth"
            ))
            .unwrap();
        let mut bare = DomRuntime::new(limits()).unwrap();
        bare.set_layout_policy(LayoutPolicy::Real);
        bare.load(Document::parse(BOXES).unwrap());
        let unset: String = bare
            .eval(&format!(
                "{RECT} rect('a') + ' ' + document.getElementById('a').offsetWidth"
            ))
            .unwrap();
        (zeros, unset, plain.layouts_run() + bare.layouts_run())
    });
    assert_eq!(got, ("0,0,0,0 0".to_owned(), "0,0,0,0 0".to_owned(), 0));
}

#[test]
fn a_list_is_laid_out_whatever_its_start() {
    let got: String = on_layout_thread(|| {
        let mut rt = runtime(
            "<!doctype html><html><body style=\"margin:0\">\
             <ol start=0 style=\"margin:0;padding:0\"><li id=a style=\"width:40px;height:10px\"></li></ol>\
             <ol start=7 reversed style=\"margin:0;padding:0\">\
             <li style=\"height:10px\"></li><li id=b style=\"width:30px;height:10px\"></li></ol>\
             <ol start=-3 style=\"margin:0;padding:0\"><li id=c style=\"width:20px;height:10px\"></li></ol>\
             </body></html>",
        );
        rt.eval(&format!(
            "{RECT} [rect('a'), rect('b'), rect('c')].join(' ')"
        ))
        .unwrap()
    });
    // List items are block boxes of the sizes given, one under the other
    // (CSS 2.1 §12.5, §9.4.1); `start` changes the markers' numbers, and a
    // marker is outside its item's principal box.
    assert_eq!(got, "0,0,40,10 0,20,30,10 0,30,20,10");
}

const SMALL: &str = "<div id=a style=\"width:5px;height:5px\"></div>";

/// `x,y,width,height` of the 5 by 5 box in a page with `head` in its head and
/// `body` after the box, and how many layouts ran.
fn small_box(head: String, body: String) -> (String, u64) {
    on_layout_thread(move || {
        let mut rt = runtime(&format!(
            "<!doctype html><html><head>{head}</head><body style=\"margin:0\">{SMALL}{body}</body></html>"
        ));
        let rect: String = rt.eval(&format!("{RECT} rect('a')")).unwrap();
        (rect, rt.layouts_run())
    })
}

const NOT_LAID_OUT: (&str, u64) = ("0,0,0,0", 0);
const LAID_OUT: (&str, u64) = ("0,0,5,5", 1);

fn outcome(got: &(String, u64)) -> (&str, u64) {
    (got.0.as_str(), got.1)
}

/// A layout can't be stopped once started, so a page with more than one
/// layout may take is not laid out and answers as with no box. The bounds
/// are in `src/layout_blitz.rs` (bytes) and `src/sheet_screen.rs` (shape).
#[test]
fn a_page_over_the_layout_bounds_is_not_laid_out() {
    let style = |css: String| format!("<style>{css}</style>");
    // More than 256 KiB of text that can be rendered.
    let words = format!("<p>{}</p>", "word ".repeat((256 << 10) / 5 + 1));
    assert_eq!(outcome(&small_box(String::new(), words)), NOT_LAID_OUT);
    // More than 1 MiB of sheets.
    let rules = "b { color: red }\n".repeat((1 << 20) / 17 + 1);
    assert_eq!(
        outcome(&small_box(style(rules), String::new())),
        NOT_LAID_OUT
    );
    // More than 8 MiB of attributes.
    let attributes = format!(
        "<i {}></i>",
        (0..4_000)
            .map(|i| format!("a{i}=\"{}\" ", "x".repeat(2_000)))
            .collect::<String>()
    )
    .repeat(2);
    assert_eq!(outcome(&small_box(String::new(), attributes)), NOT_LAID_OUT);
    // A sheet nested deeper than 32 is not laid out, whatever nests: rules,
    // parentheses or selector functions.
    for (open, close) in [("b{", "}"), ("(", ")"), (":is(", ")")] {
        let nested = |n: usize| style(format!("{}b{}{{}}", open.repeat(n), close.repeat(n)));
        assert_eq!(
            outcome(&small_box(nested(32), String::new())),
            LAID_OUT,
            "{open}"
        );
        assert_eq!(
            outcome(&small_box(nested(33), String::new())),
            NOT_LAID_OUT,
            "{open}"
        );
    }
    // A block of more than 4,096 declarations.
    let block = |n: usize| style(format!("b {{ {} }}", "--x:1;".repeat(n)));
    assert_eq!(outcome(&small_box(block(4_000), String::new())), LAID_OUT);
    assert_eq!(
        outcome(&small_box(block(4_096), String::new())),
        NOT_LAID_OUT
    );
    // A selector of more than 64 combinators.
    let selector = |n: usize| style(format!("b{} {{}}", " b".repeat(n)));
    assert_eq!(outcome(&small_box(selector(64), String::new())), LAID_OUT);
    assert_eq!(
        outcome(&small_box(selector(65), String::new())),
        NOT_LAID_OUT
    );
    // Custom properties naming each other more than 64 deep, in a sheet and
    // in a style attribute.
    let chain = |n: usize| {
        (0..n)
            .map(|i| format!("--p{i}: var(--p{});", i + 1))
            .collect::<String>()
    };
    assert_eq!(
        outcome(&small_box(
            style(format!("b {{ {} }}", chain(64))),
            String::new()
        )),
        LAID_OUT
    );
    assert_eq!(
        outcome(&small_box(
            style(format!("b {{ {} }}", chain(65))),
            String::new()
        )),
        NOT_LAID_OUT
    );
    let attribute = format!("<i style=\"{}\"></i>", chain(65));
    assert_eq!(outcome(&small_box(String::new(), attribute)), NOT_LAID_OUT);
}

/// What can't be rendered doesn't count as text: a page with an inlined
/// library is the kind of page layout is for. Long attribute values are
/// copied in part, and count in part.
#[test]
fn unrendered_text_and_long_attributes_dont_stop_a_layout() {
    let source = format!("var x = '{}';", "word ".repeat((300 << 10) / 5));
    for unrendered in [
        format!("<script type=\"text/plain\">{source}</script>"),
        format!("<title>{source}</title>"),
        format!("<noscript>{source}</noscript>"),
        format!("<style>/* {source} */</style>"),
    ] {
        assert_eq!(
            outcome(&small_box(unrendered.clone(), String::new())),
            LAID_OUT,
            "{}",
            &unrendered[..12]
        );
    }
    let script = format!("<script type=\"text/plain\">{source}</script>");
    assert_eq!(outcome(&small_box(String::new(), script)), LAID_OUT);
    let data = format!(
        "<img src=\"data:image/png;base64,{}\"><i data-x=\"{}\"></i>",
        "A".repeat(300 << 10),
        "é".repeat(200 << 10)
    );
    assert_eq!(outcome(&small_box(String::new(), data)), LAID_OUT);
    // A long class list is whole: the last of its 300 classes still matches.
    let classes: String = (0..300).map(|i| format!("class-number-{i} ")).collect();
    let got: String = on_layout_thread(move || {
        let mut rt = runtime(&format!(
            "<!doctype html><html><head><style>.class-number-299 {{ width: 9px; height: 9px }}</style></head>\
             <body style=\"margin:0\"><div id=a class=\"{classes}\"></div></body></html>"
        ));
        rt.eval(&format!("{RECT} rect('a')")).unwrap()
    });
    assert_eq!(got, "0,0,9,9");
}

/// What blitz-dom's own document behaviour does with these is in DESIGN.md
/// ("Staying out of blitz-dom's document behaviour"): the layout's copy must
/// not walk into any of it.
#[test]
fn urls_and_imports_the_page_carries_are_never_followed() {
    let got: String = on_layout_thread(|| {
        let mut rt = runtime(
            "<!doctype html><html><head><link rel=stylesheet href=\"a.css\">\
             <style>@import \"http://[\"; @import url(b.css); #a { width: 40px; height: 10px; \
             background: url(c.png) }</style></head><body style=\"margin:0\">\
             <img src=\"d.png\" width=20 height=10><input type=button value=x>\
             <div id=a></div></body></html>",
        );
        rt.eval(&format!("{RECT} rect('a').split(',').slice(2).join()"))
            .unwrap()
    });
    assert_eq!(got, "40,10");
}

#[test]
fn the_deepest_tree_the_default_limits_allow_lays_out() {
    let got: String = on_layout_thread(|| {
        let depth = DomLimits::default().max_depth - 4;
        let mut html = String::from("<!doctype html><html><body style=\"margin:0\">");
        for _ in 0..depth {
            html.push_str("<div>");
        }
        html.push_str("<p id=a style=\"margin:0;width:10px;height:10px\"></p>");
        let mut rt = runtime(&html);
        rt.eval(&format!("{RECT} rect('a')")).unwrap()
    });
    assert_eq!(got, "0,0,10,10");
}

fn resident_kib() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split(' ').nth(1).and_then(|p| p.parse::<u64>().ok()))
        .map_or(0, |pages| pages * 4)
}

/// A report page: `rows` flex rows of a label and a figure, five nodes a row.
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

/// The numbers in DESIGN.md, "Layout queries (phase 3)". Run with
/// `--release -- --ignored --nocapture layout_cost`.
#[test]
#[ignore = "a measurement, printed; run on a release build"]
fn layout_cost() {
    on_layout_thread(|| {
        for rows in [20usize, 2_000, 9_500] {
            let html = report(rows);
            let mut rt =
                DomRuntime::new(limits().with_time(std::time::Duration::from_secs(120))).unwrap();
            rt.set_layout_policy(LayoutPolicy::Real);
            rt.set_layout_setup(setup());
            let doc = Document::parse(&html).unwrap();
            let nodes = doc.node_count();
            rt.load(doc);
            // The glue's install, kept out of the first measurement.
            let _: String = rt.eval("document.body.id").unwrap();
            let before = resident_kib();
            let mut times = Vec::new();
            for script in [
                // The first layout.
                "document.body.lastChild.getBoundingClientRect().bottom",
                // Answered from it.
                "document.body.lastChild.getBoundingClientRect().bottom",
                // One attribute, then all of it again.
                "document.body.firstChild.className = 'row x'; document.body.lastChild.getBoundingClientRect().bottom",
                "document.body.firstChild.className = 'row'; document.body.lastChild.getBoundingClientRect().bottom",
            ] {
                let start = std::time::Instant::now();
                let bottom: f64 = rt.eval(script).unwrap();
                times.push(format!(
                    "{:.2} ms (bottom {bottom})",
                    start.elapsed().as_secs_f64() * 1e3
                ));
            }
            println!(
                "{rows} rows, {nodes} nodes: {}; resident +{} KiB with the layout alive",
                times.join(", "),
                resident_kib().saturating_sub(before)
            );
        }
    });
}

/// What a style sheet costs stylo by the byte, for `MAX_SHEET_BYTES`. Run
/// with `--release -- --ignored --nocapture sheet_parse_cost`.
#[test]
#[ignore = "a measurement, printed; run on a release build"]
fn sheet_parse_cost() {
    on_layout_thread(|| {
        let mut css = String::new();
        let mut i = 0;
        while css.len() < 1 << 20 {
            css.push_str(&format!(
                ".c{i} > .d{i}:not(.e{i}) {{ margin: {i}px auto; color: rgb({}, 2, 3); \
                 transform: translate(1px, {i}px) rotate(3deg); grid-template-columns: repeat(3, 1fr) }}\n",
                i % 255
            ));
            i += 1;
        }
        css.truncate(1 << 20);
        for with in [false, true] {
            let html = format!(
                "<!doctype html><html><head><style>{}</style></head>\
                 <body><div id=a style=\"width:5px;height:5px\"></div></body></html>",
                if with { css.as_str() } else { "" }
            );
            let mut rt = runtime(&html);
            let _: String = rt.eval("document.body.id").unwrap();
            let start = std::time::Instant::now();
            let w: f64 = rt
                .eval("document.getElementById('a').getBoundingClientRect().width")
                .unwrap();
            println!(
                "{} of sheet, {i} rules: first layout {:.2} ms (width {w})",
                if with { "1 MiB" } else { "0 bytes" },
                start.elapsed().as_secs_f64() * 1e3
            );
        }
    });
}
