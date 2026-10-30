//! The built-in `echarts` renderer end to end.
//!
//! The options live in `tests/echarts/*.json`. Set `SURIMONO_DUMP_DIR` to
//! write each rendered SVG there (`<name>.svg`), e.g. for
//! `js/chrome-check.mjs`, which renders the same options in Chromium.
#![cfg(feature = "echarts")]

use std::path::{Path, PathBuf};
use std::time::Instant;

use surimono_islands::{
    IslandError, IslandRequest, IslandRunner, RunnerOptions, SandboxError, builtin_renderers,
};

/// Width and height every test chart is rendered at.
const WIDTH: u32 = 600;
const HEIGHT: u32 = 400;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/echarts")
}

fn option(name: &str) -> String {
    let path = data_dir().join(format!("{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn runner() -> IslandRunner {
    IslandRunner::new(RunnerOptions {
        pool_size: 1,
        ..RunnerOptions::default()
    })
    .unwrap()
}

fn render(runner: &IslandRunner, spec_json: &str) -> Result<String, IslandError> {
    runner.render(&IslandRequest {
        renderer: "echarts",
        spec_json,
        width: WIDTH,
        height: HEIGHT,
    })
}

fn dump(name: &str, svg: &str) {
    if let Some(dir) = std::env::var_os("SURIMONO_DUMP_DIR") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}.svg")), svg).unwrap();
    }
}

/// Parses `svg` as XML and returns the texts of its `<text>` elements.
fn check_svg(svg: &str) -> Vec<String> {
    let doc = roxmltree::Document::parse(svg).unwrap_or_else(|e| panic!("not XML: {e}\n{svg}"));
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "svg");
    assert_eq!(root.tag_name().namespace(), Some("http://www.w3.org/2000/svg"));
    assert_eq!(root.attribute("width"), Some(WIDTH.to_string().as_str()));
    assert_eq!(root.attribute("height"), Some(HEIGHT.to_string().as_str()));
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

#[test]
fn echarts_is_a_builtin() {
    assert!(builtin_renderers().contains(&"echarts"));
    let r = runner();
    assert!(r.renderers().is_empty(), "built-ins compile on first use");
    render(&r, &option("bar")).unwrap();
    assert_eq!(r.renderers(), ["echarts"]);
}

#[test]
fn bar_chart_is_valid_svg() {
    let svg = render(&runner(), &option("bar")).unwrap();
    dump("bar", &svg);
    let texts = check_svg(&svg);
    for label in ["Weekly sales", "Sales", "Mon", "Sun", "120", "200"] {
        assert!(texts.iter().any(|t| t == label), "no {label:?} in {texts:?}");
    }
    // Seven bars (paths), with no animation styles.
    assert!(count(&svg, "path") >= 7);
    assert!(!svg.contains("@keyframes"), "animation must be off");
}

#[test]
fn japanese_labels_survive() {
    let svg = render(&runner(), &option("japanese")).unwrap();
    dump("japanese", &svg);
    let texts = check_svg(&svg);
    for label in ["月別売上高（百万円）", "東京・大阪・名古屋", "1月", "6月", "名古屋", "売上"] {
        assert!(texts.iter().any(|t| t == label), "no {label:?} in {texts:?}");
    }
}

#[test]
fn every_option_renders_and_is_deterministic() {
    let r = runner();
    for name in ["bar", "japanese", "pie", "dense"] {
        let spec = option(name);
        let first = render(&r, &spec).unwrap();
        check_svg(&first);
        dump(name, &first);
        let second = render(&r, &spec).unwrap();
        assert!(first == second, "{name}: two renders differ");
        // A different runner (fresh workers) gives the same bytes too.
        assert!(render(&runner(), &spec).unwrap() == first, "{name}: runners differ");
    }
}

#[test]
fn text_is_measured_by_the_host() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use surimono_islands::{TextMeasurer, TextMetrics};

    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    // Twice the stand-in's widths: wider labels must move the layout.
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
    let spec = option("bar");
    let wide_svg = render(&r, &spec).unwrap();
    assert!(calls.load(Ordering::Relaxed) > 0, "measureText was never called");
    let normal_svg = render(&runner(), &spec).unwrap();
    assert_ne!(wide_svg, normal_svg);
}

#[test]
fn bad_specs_are_typed_errors() {
    let r = runner();
    // Not an object.
    let err = render(&r, "[1, 2]").unwrap_err();
    assert!(
        matches!(&err, IslandError::Sandbox(SandboxError::Exception { message, .. })
            if message.contains("ECharts option object")),
        "{err:?}"
    );
    // A zero size.
    let err = r
        .render(&IslandRequest {
            renderer: "echarts",
            spec_json: "{}",
            width: 0,
            height: 100,
        })
        .unwrap_err();
    assert!(
        matches!(&err, IslandError::Sandbox(SandboxError::Exception { message, .. })
            if message.contains("positive width")),
        "{err:?}"
    );
    // An empty option is an empty chart, not an error.
    check_svg(&render(&r, "{}").unwrap());
}

#[test]
fn series_cannot_turn_animation_back_on() {
    let spec = r#"{"animation":true,"xAxis":{"type":"category","data":["a","b"]},"yAxis":{},
        "series":[{"type":"bar","animation":true,"animationDuration":5000,"data":[3,4]}]}"#;
    let svg = render(&runner(), spec).unwrap();
    check_svg(&svg);
    assert!(!svg.contains("@keyframes"), "animation must be off: {svg}");
}

/// Render times in this build, for the PR and the benchmark notes. Run with
/// `cargo test --release -p surimono-islands --test echarts -- --ignored --nocapture timing`.
#[test]
#[ignore = "timing report; run explicitly in release mode"]
fn timing_echarts() {
    let r = runner();
    let t = Instant::now();
    render(&r, &option("bar")).unwrap();
    eprintln!("echarts: first render incl. compile {:?}", t.elapsed());
    for name in ["bar", "japanese", "pie", "dense"] {
        let spec = option(name);
        let runs = 20;
        let mut times = Vec::with_capacity(runs);
        let mut size = 0;
        for _ in 0..runs {
            let t = Instant::now();
            size = render(&r, &spec).unwrap().len();
            times.push(t.elapsed());
        }
        times.sort();
        eprintln!(
            "echarts {name:>8}: median {:?}, min {:?}, max {:?}, {size} bytes of SVG",
            times[runs / 2],
            times[0],
            times[runs - 1]
        );
    }
}
