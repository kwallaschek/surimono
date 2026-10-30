//! The island runner end to end, with small hand-written bundles.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use surimono_islands::{
    IslandError, IslandRequest, IslandRunner, Limits, RunnerOptions, SandboxError, TextMeasurer,
    TextMetrics,
};

/// A tiny "chart library": centres a measured label and stamps the output
/// with Math.random and Date so determinism is visible.
const LABEL: &str = r#"
function entry(specJson, ctx) {
  const spec = JSON.parse(specJson);
  const m = ctx.measureText(spec.label, spec.font || '12px sans-serif');
  const x = (ctx.width - m.width) / 2;
  return '<svg xmlns="http://www.w3.org/2000/svg" width="' + ctx.width + '" height="' + ctx.height + '">' +
    '<text x="' + x + '" y="' + m.actualBoundingBoxAscent + '">' + spec.label + '</text>' +
    '<desc>' + Math.random() + ' ' + new Date().toISOString() + '</desc></svg>';
}
"#;

fn options() -> RunnerOptions {
    RunnerOptions {
        pool_size: 2,
        limits: Limits {
            time: Duration::from_millis(300),
            memory_bytes: 16 * 1024 * 1024,
            ..Limits::default()
        },
        ..RunnerOptions::default()
    }
}

fn runner() -> IslandRunner {
    let r = IslandRunner::new(options()).unwrap();
    r.register_bundle("label", LABEL, "entry").unwrap();
    r
}

fn req<'a>(renderer: &'a str, spec_json: &'a str) -> IslandRequest<'a> {
    IslandRequest {
        renderer,
        spec_json,
        width: 200,
        height: 100,
    }
}

#[test]
fn runner_is_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<IslandRunner>();
    check::<RunnerOptions>();
    check::<IslandError>();
}

#[test]
fn renders_svg_with_measured_text() {
    let r = runner();
    let svg = r.render(&req("label", r#"{"label":"hello"}"#)).unwrap();
    // approximate measurer: 5 chars × 0.6 × 12px = 36px wide, ascent 9.6px.
    assert!(
        svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">"#)
    );
    assert!(
        svg.contains(r#"<text x="82" y="9.6">hello</text>"#),
        "{svg}"
    );
    assert!(svg.contains("2026-01-01T00:00:00.000Z"), "{svg}");
}

#[test]
fn uses_the_configured_measurer() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let r = IslandRunner::new(RunnerOptions {
        text_measurer: TextMeasurer::new(move |text, font| {
            seen.lock()
                .unwrap()
                .push((text.to_owned(), font.to_owned()));
            Ok(TextMetrics {
                width: 100.0,
                actual_bounding_box_ascent: 7.0,
                ..TextMetrics::default()
            })
        }),
        ..options()
    })
    .unwrap();
    r.register_bundle("label", LABEL, "entry").unwrap();
    let svg = r
        .render(&req(
            "label",
            r#"{"label":"日本語","font":"bold 14px Noto Sans JP"}"#,
        ))
        .unwrap();
    assert!(svg.contains(r#"<text x="50" y="7">日本語</text>"#), "{svg}");
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[("日本語".to_owned(), "bold 14px Noto Sans JP".to_owned())]
    );
}

#[test]
fn measure_text_returns_all_canvas_fields() {
    let r = IslandRunner::new(options()).unwrap();
    r.register_bundle(
        "fields",
        "function entry(s, ctx) { const m = ctx.measureText('ab'); return Object.keys(m).join() + '|' + m.width + '|' + ctx.measureText.name }",
        "entry",
    )
    .unwrap();
    assert_eq!(
        r.render(&req("fields", "{}")).unwrap(),
        // Default font is canvas's "10px sans-serif": 2 × 0.6 × 10.
        "width,actualBoundingBoxLeft,actualBoundingBoxRight,actualBoundingBoxAscent,\
         actualBoundingBoxDescent,fontBoundingBoxAscent,fontBoundingBoxDescent|12|measureText"
    );
}

#[test]
fn measurer_errors_become_type_errors() {
    let r = IslandRunner::new(RunnerOptions {
        text_measurer: TextMeasurer::new(|_, _| Err("no such font".into())),
        ..options()
    })
    .unwrap();
    r.register_bundle("label", LABEL, "entry").unwrap();
    match r.render(&req("label", r#"{"label":"x"}"#)).unwrap_err() {
        IslandError::Sandbox(SandboxError::Exception { message, .. }) => {
            assert_eq!(message, "TypeError: measureText: no such font")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn measurer_panics_do_not_kill_the_pool() {
    let r = IslandRunner::new(RunnerOptions {
        pool_size: 1,
        text_measurer: TextMeasurer::new(|text, _| {
            if text == "boom" {
                panic!("measurer exploded");
            }
            Ok(TextMetrics::default())
        }),
        ..options()
    })
    .unwrap();
    r.register_bundle("label", LABEL, "entry").unwrap();
    match r.render(&req("label", r#"{"label":"boom"}"#)).unwrap_err() {
        IslandError::Worker { message } => {
            assert!(message.contains("measurer exploded"), "{message}")
        }
        other => panic!("{other:?}"),
    }
    assert!(r.render(&req("label", r#"{"label":"fine"}"#)).is_ok());
}

#[test]
fn same_request_same_svg() {
    let r = runner();
    let a = r.render(&req("label", r#"{"label":"x"}"#)).unwrap();
    for _ in 0..5 {
        assert_eq!(r.render(&req("label", r#"{"label":"x"}"#)).unwrap(), a);
    }
    // And from a different runner.
    assert_eq!(
        runner().render(&req("label", r#"{"label":"x"}"#)).unwrap(),
        a
    );
    // A different seed changes Math.random.
    let other = IslandRunner::new(RunnerOptions {
        seed: 7,
        ..options()
    })
    .unwrap();
    other.register_bundle("label", LABEL, "entry").unwrap();
    assert_ne!(other.render(&req("label", r#"{"label":"x"}"#)).unwrap(), a);
}

#[test]
fn unknown_renderer() {
    assert_eq!(
        runner().render(&req("nope", "{}")).unwrap_err(),
        IslandError::UnknownRenderer {
            name: "nope".into()
        }
    );
}

#[test]
fn bad_json_is_reported_with_position() {
    let err = runner()
        .render(&req("label", "{\n  \"label\": }"))
        .unwrap_err();
    match &err {
        IslandError::BadJson {
            message,
            line,
            column,
        } => {
            assert_eq!((*line, *column), (2, 12), "{message}");
            assert!(!message.contains("line"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert!(err.to_string().ends_with("at line 2 column 12"), "{err}");
}

#[test]
fn lone_surrogates_become_replacement_characters() {
    let r = runner();
    r.register_bundle(
        "surrogates",
        r#"function entry() { return '<svg>' + '\uD800' + 'a' + '\uDC00' + '\uD83D\uDE00' + '</svg>' }"#,
        "entry",
    )
    .unwrap();
    assert_eq!(
        r.render(&req("surrogates", "{}")).unwrap(),
        "<svg>\u{FFFD}a\u{FFFD}\u{1F600}</svg>"
    );
    // A bundle that breaks toWellFormed gets a typed error, not a crash.
    r.register_bundle(
        "broken",
        r#"String.prototype.toWellFormed = function () { return this };
           function entry() { return '\uD800' }"#,
        "entry",
    )
    .unwrap();
    assert!(matches!(
        r.render(&req("broken", "{}")).unwrap_err(),
        IslandError::NotAString { .. }
    ));
}

#[test]
fn non_string_output() {
    let r = runner();
    r.register_bundle("num", "function entry() { return 42 }", "entry")
        .unwrap();
    assert_eq!(
        r.render(&req("num", "{}")).unwrap_err(),
        IslandError::NotAString {
            renderer: "num".into(),
            found: "int".into()
        }
    );
    r.register_bundle("obj", "function entry() { return {} }", "entry")
        .unwrap();
    assert!(matches!(
        r.render(&req("obj", "{}")).unwrap_err(),
        IslandError::NotAString { .. }
    ));
}

#[test]
fn entry_exceptions_are_sandbox_errors() {
    let r = runner();
    r.register_bundle(
        "throws",
        "function entry(s) { JSON.parse(s).missing.field }",
        "entry",
    )
    .unwrap();
    match r.render(&req("throws", "{}")).unwrap_err() {
        IslandError::Sandbox(SandboxError::Exception { message, stack }) => {
            assert!(message.starts_with("TypeError"), "{message}");
            assert!(stack.unwrap().contains("throws.js"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn async_entries_are_awaited() {
    let r = runner();
    r.register_bundle(
        "async",
        "async function entry(s, ctx) { await null; return '<svg>' + JSON.parse(s).n + '</svg>' }",
        "entry",
    )
    .unwrap();
    assert_eq!(
        r.render(&req("async", r#"{"n":3}"#)).unwrap(),
        "<svg>3</svg>"
    );
    r.register_bundle(
        "never",
        "function entry() { return new Promise(() => {}) }",
        "entry",
    )
    .unwrap();
    assert!(matches!(
        r.render(&req("never", "{}")).unwrap_err(),
        IslandError::Sandbox(SandboxError::Type { .. })
    ));
}

#[test]
fn registration_checks_the_bundle() {
    let r = runner();
    assert_eq!(
        r.register_bundle("x", "var notEntry = 1", "entry")
            .unwrap_err(),
        IslandError::MissingEntry {
            bundle: "x".into(),
            entry: "entry".into()
        }
    );
    assert!(matches!(
        r.register_bundle("x", "function (", "entry").unwrap_err(),
        IslandError::Sandbox(SandboxError::Exception { .. })
    ));
    assert!(matches!(
        r.register_bundle("x", "for (;;) {}", "entry").unwrap_err(),
        IslandError::Sandbox(SandboxError::TimeLimit)
    ));
    assert!(matches!(
        r.register_bundle("", "function entry() {}", "entry")
            .unwrap_err(),
        IslandError::InvalidOptions { .. }
    ));
    assert_eq!(r.renderers(), vec!["label".to_owned()]);
}

#[test]
fn reregistering_replaces_the_bundle() {
    let r = runner();
    r.register_bundle("v", "function entry() { return 'one' }", "entry")
        .unwrap();
    r.register_bundle("v", "function render() { return 'two' }", "render")
        .unwrap();
    assert_eq!(r.render(&req("v", "{}")).unwrap(), "two");
    assert_eq!(r.renderers(), vec!["label".to_owned(), "v".to_owned()]);
}

#[test]
fn hostile_renderers_hit_limits_and_the_pool_recovers() {
    let r = runner();
    let hostile = [
        (
            "loop",
            "function entry() { for (;;) {} }",
            SandboxError::TimeLimit,
        ),
        (
            "alloc",
            "function entry() { const a = []; for (;;) a.push(new Array(1e4).fill(1)) }",
            SandboxError::MemoryLimit,
        ),
        (
            "recurse",
            "function entry() { return entry() + 1 }",
            SandboxError::StackOverflow,
        ),
        (
            "flood",
            "function entry() { function f() { Promise.resolve().then(f) } f(); return 'x' }",
            SandboxError::TimeLimit,
        ),
        (
            "native",
            "function entry() { return Array.prototype.join.call({ length: 2 ** 40 }, '') }",
            SandboxError::Exception {
                message: String::new(),
                stack: None,
            },
        ),
    ];
    for (name, src, want) in hostile {
        r.register_bundle(name, src, "entry").unwrap();
        let start = Instant::now();
        let err = r.render(&req(name, "{}")).unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(5), "{name}");
        match (&err, &want) {
            (
                IslandError::Sandbox(SandboxError::Exception { .. }),
                SandboxError::Exception { .. },
            ) => {}
            (IslandError::Sandbox(got), want) => assert_eq!(got, want, "{name}"),
            _ => panic!("{name}: {err:?}"),
        }
        // Every worker still renders.
        for _ in 0..4 {
            assert!(
                r.render(&req("label", r#"{"label":"ok"}"#)).is_ok(),
                "after {name}"
            );
        }
    }
}

#[test]
fn renders_concurrently_from_many_threads() {
    let r = Arc::new(runner());
    let done = Arc::new(AtomicUsize::new(0));
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let r = r.clone();
            let done = done.clone();
            std::thread::spawn(move || {
                for i in 0..10 {
                    let label = format!("t{t}i{i}");
                    let spec = format!(r#"{{"label":"{label}"}}"#);
                    let svg = r.render(&req("label", &spec)).unwrap();
                    assert!(svg.contains(&format!(">{label}</text>")));
                    done.fetch_add(1, Ordering::Relaxed);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(done.load(Ordering::Relaxed), 80);
}

#[test]
fn invalid_options_are_rejected() {
    for bad in [
        RunnerOptions {
            pool_size: 1000,
            ..options()
        },
        RunnerOptions {
            limits: Limits {
                memory_bytes: 1,
                ..Limits::default()
            },
            ..options()
        },
        RunnerOptions {
            utc_offset_minutes: 10_000,
            ..options()
        },
    ] {
        assert!(matches!(
            IslandRunner::new(bad).unwrap_err(),
            IslandError::InvalidOptions { .. }
        ));
    }
}

#[test]
fn pool_size_defaults_to_cpus_up_to_four() {
    let r = IslandRunner::new(RunnerOptions::default()).unwrap();
    let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
    assert_eq!(r.pool_size(), cpus.min(4));
    assert_eq!(
        IslandRunner::new(RunnerOptions {
            pool_size: 3,
            ..options()
        })
        .unwrap()
        .pool_size(),
        3
    );
}

#[test]
fn dropping_the_runner_stops_the_workers() {
    let r = runner();
    r.render(&req("label", r#"{"label":"x"}"#)).unwrap();
    drop(r); // joins the workers; must not hang
}
