//! The built-in `vega` and `vega-lite` renderers end to end.
//!
//! The specs live in `tests/vega/*.json` and `tests/vega-lite/*.json`. Set
//! `SURIMONO_DUMP_DIR` to write each rendered SVG to
//! `<dir>/<renderer>/<name>.svg`, e.g. for `js/chrome-check.js`.
#![cfg(feature = "vega")]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use surimono_islands::{
    IslandError, IslandRequest, IslandRunner, RunnerOptions, SandboxError, TextMeasurer,
    TextMetrics, builtin_renderers,
};

/// The island box. Vega ignores it: a spec sets its own size.
const WIDTH: u32 = 600;
const HEIGHT: u32 = 400;

fn data_dir(renderer: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(renderer)
}

fn spec(renderer: &str, name: &str) -> String {
    let path = data_dir(renderer).join(format!("{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn spec_names(renderer: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(data_dir(renderer))
        .unwrap()
        .filter_map(|e| {
            let name = e.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".json").map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

fn runner() -> IslandRunner {
    IslandRunner::new(RunnerOptions {
        pool_size: 1,
        ..RunnerOptions::default()
    })
    .unwrap()
}

fn render(runner: &IslandRunner, renderer: &str, spec_json: &str) -> Result<String, IslandError> {
    runner.render(&IslandRequest {
        renderer,
        spec_json,
        width: WIDTH,
        height: HEIGHT,
    })
}

fn dump(renderer: &str, name: &str, svg: &str) {
    if let Some(dir) = std::env::var_os("SURIMONO_DUMP_DIR") {
        let dir = PathBuf::from(dir).join(renderer);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}.svg")), svg).unwrap();
    }
}

/// Parses `svg` as XML, checks its root and returns its `<text>` contents.
fn check_svg(svg: &str) -> Vec<String> {
    let doc = roxmltree::Document::parse(svg).unwrap_or_else(|e| panic!("not XML: {e}\n{svg}"));
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "svg");
    assert_eq!(root.tag_name().namespace(), Some("http://www.w3.org/2000/svg"));
    for size in ["width", "height"] {
        let v: f64 = root.attribute(size).unwrap().parse().unwrap();
        assert!(v > 0.0, "{size} {v}");
    }
    root.descendants()
        .filter(|n| n.has_tag_name("text"))
        .map(|n| {
            n.descendants()
                .filter(|t| t.is_text())
                .filter_map(|t| t.text())
                .collect()
        })
        .collect()
}

fn count(svg: &str, tag: &str) -> usize {
    let doc = roxmltree::Document::parse(svg).unwrap();
    doc.descendants().filter(|n| n.has_tag_name(tag)).count()
}

fn sandbox_message(err: &IslandError) -> &str {
    match err {
        IslandError::Sandbox(SandboxError::Exception { message, .. }) => message,
        other => panic!("not an exception: {other:?}"),
    }
}

#[test]
fn vega_and_vega_lite_are_builtins() {
    let names = builtin_renderers();
    assert!(names.contains(&"vega") && names.contains(&"vega-lite"));
}

#[test]
fn vega_lite_bar_chart() {
    let svg = render(&runner(), "vega-lite", &spec("vega-lite", "bar")).unwrap();
    dump("vega-lite", "bar", &svg);
    let texts = check_svg(&svg);
    for label in ["Weekly sales", "Mon", "Sun", "day", "sales"] {
        assert!(texts.iter().any(|t| t == label), "no {label:?} in {texts:?}");
    }
    // Seven bars plus axis and frame paths.
    assert!(count(&svg, "path") >= 7, "{svg}");
}

#[test]
fn vega_bar_chart() {
    let svg = render(&runner(), "vega", &spec("vega", "bar")).unwrap();
    dump("vega", "bar", &svg);
    let texts = check_svg(&svg);
    for label in ["A", "H", "91", "19"] {
        assert!(texts.iter().any(|t| t == label), "no {label:?} in {texts:?}");
    }
}

#[test]
fn japanese_labels_survive() {
    let svg = render(&runner(), "vega-lite", &spec("vega-lite", "japanese")).unwrap();
    let texts = check_svg(&svg);
    for label in ["月別売上高（百万円）", "東京・大阪・名古屋", "1月", "4月", "名古屋", "売上"] {
        assert!(texts.iter().any(|t| t == label), "no {label:?} in {texts:?}");
    }
}

#[test]
fn every_spec_renders_and_is_deterministic() {
    let r = runner();
    for renderer in ["vega", "vega-lite"] {
        let names = spec_names(renderer);
        assert!(!names.is_empty());
        for name in &names {
            let spec = spec(renderer, name);
            let first =
                render(&r, renderer, &spec).unwrap_or_else(|e| panic!("{renderer}/{name}: {e}"));
            check_svg(&first);
            dump(renderer, name, &first);
            assert!(render(&r, renderer, &spec).unwrap() == first, "{renderer}/{name}");
            assert!(
                render(&runner(), renderer, &spec).unwrap() == first,
                "{renderer}/{name}: runners differ"
            );
        }
    }
}

#[test]
fn dense_scatter_has_every_point() {
    let svg = render(&runner(), "vega-lite", &spec("vega-lite", "scatter-2000")).unwrap();
    check_svg(&svg);
    // Vega draws each circle as a <path>; axes and legend add a few dozen.
    let paths = count(&svg, "path");
    assert!((2000..2100).contains(&paths), "{paths} paths");
}

#[test]
fn text_is_measured_by_the_host() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let wide = TextMeasurer::new(move |text, font| {
        seen.fetch_add(1, Ordering::Relaxed);
        let m = TextMeasurer::approximate().measure(text, font)?;
        Ok(TextMetrics {
            width: m.width * 2.0,
            ..m
        })
    });
    let r = IslandRunner::new(RunnerOptions {
        pool_size: 1,
        text_measurer: wide,
        ..RunnerOptions::default()
    })
    .unwrap();
    // Truncation with `limit` depends on measured widths.
    let spec = spec("vega", "truncated-labels");
    let wide_svg = render(&r, "vega", &spec).unwrap();
    assert!(calls.load(Ordering::Relaxed) > 0, "measureText was never called");
    let normal_svg = render(&runner(), "vega", &spec).unwrap();
    assert_ne!(wide_svg, normal_svg);
    let wide_texts = check_svg(&wide_svg);
    let normal_texts = check_svg(&normal_svg);
    let long = |texts: &[String]| texts.iter().find(|t| t.starts_with("A r")).cloned();
    let (w, n) = (long(&wide_texts).unwrap(), long(&normal_texts).unwrap());
    assert!(w.ends_with('…') && n.ends_with('…'), "{w:?} {n:?}");
    assert!(w.chars().count() < n.chars().count(), "{w:?} {n:?}");
}

#[test]
fn data_urls_are_refused() {
    let r = runner();
    let spec = r#"{"data":{"url":"https://example.com/data.csv"},"mark":"bar",
        "encoding":{"x":{"field":"a","type":"nominal"},"y":{"field":"b","type":"quantitative"}}}"#;
    let err = render(&r, "vega-lite", spec).unwrap_err();
    let message = sandbox_message(&err);
    assert!(
        message.contains("must be inline") && message.contains("https://example.com/data.csv"),
        "{message}"
    );
    let spec = r#"{"data":[{"name":"t","url":"data/cars.json"}]}"#;
    let err = render(&r, "vega", spec).unwrap_err();
    assert!(
        sandbox_message(&err).contains("data/cars.json"),
        "{err:?}"
    );
}

#[test]
fn bad_specs_are_typed_errors() {
    let r = runner();
    let err = render(&r, "vega-lite", "[]").unwrap_err();
    assert!(sandbox_message(&err).contains("vega-lite spec object"), "{err:?}");
    let err = render(&r, "vega", "42").unwrap_err();
    assert!(sandbox_message(&err).contains("vega spec object"), "{err:?}");
    // An unknown mark type fails inside Vega-Lite's compiler.
    let err = render(&r, "vega-lite", r#"{"mark":"nonsense","data":{"values":[]}}"#).unwrap_err();
    sandbox_message(&err);
    // A bad expression is Vega's.
    let err = render(
        &r,
        "vega",
        r#"{"signals":[{"name":"s","update":"this is not an expression"}]}"#,
    )
    .unwrap_err();
    assert!(matches!(err, IslandError::Sandbox(SandboxError::Exception { .. })), "{err:?}");
}

#[test]
fn raster_features_fail_clearly() {
    // The label transform draws marks into a canvas bitmap.
    let spec = r#"{"width":100,"height":100,
      "data":[{"name":"t","values":[{"x":1,"y":2,"l":"a"}]}],
      "marks":[{"type":"symbol","name":"pts","from":{"data":"t"},
        "encode":{"enter":{"x":{"field":"x"},"y":{"field":"y"}}}},
        {"type":"text","from":{"data":"pts"},
         "encode":{"enter":{"text":{"field":"datum.l"}}},
         "transform":[{"type":"label","size":[100,100]}]}]}"#;
    let err = render(&runner(), "vega", spec).unwrap_err();
    assert!(sandbox_message(&err).contains("raster canvas"), "{err:?}");
}

#[test]
fn vega_soaks_up_no_state_between_specs() {
    // A spec with signals and random data, then a plain one: the second
    // must render exactly as it does alone.
    let r = runner();
    let plain = spec("vega", "bar");
    let alone = render(&r, "vega", &plain).unwrap();
    let noisy = r#"{"signals":[{"name":"n","value":3}],
      "data":[{"name":"r","transform":[{"type":"sequence","start":0,"stop":50},
        {"type":"formula","as":"v","expr":"random()"}]}]}"#;
    render(&r, "vega", noisy).unwrap();
    assert!(render(&r, "vega", &plain).unwrap() == alone);
}

/// Render times in this build. Run with
/// `cargo test --release -p surimono-islands --test vega -- --ignored --nocapture timing`.
#[test]
#[ignore = "timing report; run explicitly in release mode"]
fn timing_vega() {
    const RUNS: usize = 20;
    let r = runner();
    r.register_bundle("other", "function entry() { return '<svg/>' }", "entry")
        .unwrap();
    for renderer in ["vega", "vega-lite"] {
        let t = Instant::now();
        render(&r, renderer, &spec(renderer, "bar")).unwrap();
        eprintln!("{renderer}: first render, including compiling the bundle: {:?}", t.elapsed());
    }
    let pause = || std::thread::sleep(Duration::from_millis(150));
    let median = |mut v: Vec<Duration>| {
        v.sort();
        v[v.len() / 2]
    };
    for renderer in ["vega", "vega-lite"] {
        for name in spec_names(renderer) {
            let spec = spec(renderer, &name);
            let (mut cold, mut warm) = (Vec::new(), Vec::new());
            let mut size = 0;
            for _ in 0..RUNS {
                r.render(&IslandRequest {
                    renderer: "other",
                    spec_json: "{}",
                    width: 1,
                    height: 1,
                })
                .unwrap();
                pause();
                let t = Instant::now();
                size = render(&r, renderer, &spec).unwrap().len();
                cold.push(t.elapsed());
                pause();
                let t = Instant::now();
                render(&r, renderer, &spec).unwrap();
                warm.push(t.elapsed());
            }
            eprintln!(
                "{renderer:>9} {name:>16}: median cold {:>9.2?}, preloaded {:>9.2?}, {size} bytes of SVG",
                median(cold),
                median(warm)
            );
        }
    }
}
