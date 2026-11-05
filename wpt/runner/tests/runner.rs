//! The runner's own paths, on a small hostile manifest: children, crashes,
//! limits, `timeout()` and the `.any.js` wrapper. `wpt/README.md`, "Runner".
//! Whatever a page or a child does must come back as a value in the results
//! file, and must not touch the next test. Expected statuses come from
//! `wpt/README.md`'s tables and `testharness.js`, not from a run.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const PASS_AND_FAIL: &str = r#"<!doctype html>
<script src="/resources/testharness.js"></script>
<script src="/resources/testharnessreport.js"></script>
<script>
test(() => {}, "passes");
test(() => { assert_true(false); }, "fails");
</script>"#;

fn page(script: &str) -> String {
    format!(
        r#"<!doctype html>
<script src="/resources/testharness.js"></script>
<script src="/resources/testharnessreport.js"></script>
<script>{script}</script>"#
    )
}

/// `(file, what the harness sees)`. Each URL is the file's, except the variant.
fn files() -> Vec<(&'static str, String)> {
    vec![
        ("pass-and-fail.html", PASS_AND_FAIL.to_owned()),
        (
            "wrapped.any.js",
            "// META: title=Wrapped\n// META: script=helper.js\n\
             test(() => {\n\
               assert_true(self.GLOBAL.isWindow());\n\
               assert_false(self.GLOBAL.isWorker());\n\
               assert_equals(helper, 7);\n\
               assert_equals(document.title, 'Wrapped');\n\
               assert_equals(location.search, '?v=1');\n\
             }, 'wrapped');\n"
                .to_owned(),
        ),
        ("helper.js", "var helper = 7;\n".to_owned()),
        (
            "uncaught.html",
            page("test(() => {}, 'finished'); throw new Error('boom');"),
        ),
        (
            "never-done.html",
            page("test(() => {}, 'finished'); async_test('never done');"),
        ),
        (
            "no-done-call.html",
            page("setup({ explicit_done: true }); test(() => {}, 'finished');"),
        ),
        ("loop.html", page("for (;;) {}")),
        (
            "loop-in-microtasks.html",
            page("(function f() { Promise.resolve().then(f); })();"),
        ),
        (
            "loop-in-timer.html",
            page("setTimeout(() => { for (;;) {} }, 0);"),
        ),
        (
            "allocation.html",
            page("const a = []; for (;;) a.push(new Array(1e6).fill(0));"),
        ),
        (
            "deep-recursion.html",
            page("test(function f() { f(); }, 'recursion');"),
        ),
        ("no-harness.html", "<!doctype html><p>no harness".to_owned()),
        ("needs-xml.xhtml", String::new()),
        ("aborts.html", PASS_AND_FAIL.to_owned()),
        ("exits.html", PASS_AND_FAIL.to_owned()),
        ("panics.html", PASS_AND_FAIL.to_owned()),
        ("garbage.html", PASS_AND_FAIL.to_owned()),
        ("hangs.html", PASS_AND_FAIL.to_owned()),
        ("after-the-faults.html", PASS_AND_FAIL.to_owned()),
    ]
}

/// A temporary `wpt/` with the real harness and those tests, in that order.
fn tree(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("wpt-runner-{name}-{}", std::process::id()));
    let tests = root.join("tests");
    fs::create_dir_all(tests.join("resources")).unwrap();
    let harness = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/resources/testharness.js");
    fs::copy(harness, tests.join("resources/testharness.js")).unwrap();
    let mut manifest = String::from("commit = \"abc\"\n");
    for (file, text) in files() {
        fs::write(tests.join(file), text).unwrap();
        if file == "helper.js" {
            continue;
        }
        let url = match file.strip_suffix(".js") {
            Some(stem) => format!("{stem}.html?v=1"),
            None => file.to_owned(),
        };
        manifest += &format!("[[test]]\nslice = \"s\"\nfile = \"{file}\"\nurls = [\"{url}\"]\n");
    }
    manifest += "[[test]]\nslice = \"s\"\nfile = \"missing.html\"\nurls = [\"missing.html\"]\n";
    fs::write(root.join("manifest.toml"), manifest).unwrap();
    root
}

fn run(root: &Path, extra: &[&str], fault: &str) -> Value {
    let out = root.join("results.json");
    let status = Command::new(env!("CARGO_BIN_EXE_surimono-wpt-runner"))
        .arg("--root")
        .arg(root)
        .arg("--out")
        .arg(&out)
        .args(["--commit", "cafe", "--time-limit-ms", "1000"])
        .args(["--wall-limit-ms", "8000", "--jobs", "2"])
        .args(extra)
        .env("SURIMONO_WPT_FAULT", fault)
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");
    serde_json::from_str(&fs::read_to_string(out).unwrap()).unwrap()
}

fn test<'a>(results: &'a Value, url: &str) -> &'a Value {
    results["tests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["test"] == url)
        .unwrap_or_else(|| panic!("{url} isn't in the results"))
}

fn subtests(test: &Value) -> Vec<(&str, &str)> {
    test["subtests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["name"].as_str().unwrap(), s["status"].as_str().unwrap()))
        .collect()
}

#[test]
fn every_path_ends_in_a_value() {
    let root = tree("all");
    let r = run(
        &root,
        &[],
        "abort aborts.html,exit exits.html,panic panics.html,\
         garbage garbage.html,hang hangs.html",
    );
    assert_eq!(r["wpt_commit"], "abc");
    assert_eq!(r["surimono_commit"], "cafe");
    let status = |url: &str| test(&r, url)["status"].as_str().unwrap().to_owned();

    // A test that ends, and the wrapper for `.any.js`, its query and its META.
    let t = test(&r, "pass-and-fail.html");
    assert_eq!(t["status"], "OK");
    assert_eq!(subtests(t), [("passes", "PASS"), ("fails", "FAIL")], "{t}");
    let t = test(&r, "wrapped.any.html?v=1");
    assert_eq!(t["status"], "OK");
    assert_eq!(subtests(t), [("wrapped", "PASS")], "{t}");

    // The harness's own verdict on an uncaught error.
    let t = test(&r, "uncaught.html");
    assert_eq!(t["status"], "ERROR", "{t}");

    // The page settles with no report, so the runner calls `timeout()`: what
    // finished keeps its status, and the rest isn't a pass.
    let t = test(&r, "never-done.html");
    assert_eq!(t["status"], "TIMEOUT", "{t}");
    assert_eq!(
        subtests(t),
        [("finished", "PASS"), ("never done", "NOTRUN")],
        "{t}"
    );
    let t = test(&r, "no-done-call.html");
    assert_eq!(t["status"], "TIMEOUT", "{t}");
    assert_eq!(subtests(t), [("finished", "PASS")], "{t}");

    // Sandbox limits: a value, and no subtest gets credit.
    for url in [
        "loop.html",
        "loop-in-microtasks.html",
        "loop-in-timer.html",
        "allocation.html",
    ] {
        let t = test(&r, url);
        assert_eq!(t["status"], "LIMIT", "{url}: {t}");
        assert_eq!(subtests(t), [], "{url}");
    }

    // A stack overflow is catchable, as in a browser: one failed subtest.
    let t = test(&r, "deep-recursion.html");
    assert_eq!(t["status"], "OK", "{t}");
    assert_eq!(subtests(t), [("recursion", "FAIL")], "{t}");

    assert_eq!(status("no-harness.html"), "LIMIT");
    assert_eq!(status("needs-xml.xhtml"), "ERROR");
    assert_eq!(status("missing.html"), "ERROR");

    // A child that dies, or lies, is that test's result and no other's.
    for url in ["aborts.html", "exits.html", "panics.html", "garbage.html"] {
        let t = test(&r, url);
        assert_eq!(t["status"], "CRASH", "{url}: {t}");
        assert_eq!(subtests(t), [], "{url}");
    }
    let t = test(&r, "hangs.html");
    assert_eq!(t["status"], "LIMIT", "{t}");
    assert!(t["message"].as_str().unwrap().contains("wall limit"), "{t}");
    assert_eq!(status("after-the-faults.html"), "OK");

    // In manifest order, whatever order the jobs finished in.
    let urls: Vec<_> = r["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["test"].as_str().unwrap())
        .collect();
    assert_eq!(urls.len(), files().len());
    assert_eq!(urls[..2], ["pass-and-fail.html", "wrapped.any.html?v=1"]);
    assert_eq!(*urls.last().unwrap(), "missing.html");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_filter_keeps_only_its_tests() {
    let root = tree("filter");
    let r = run(&root, &["--filter", "pass-and"], "");
    assert_eq!(r["tests"].as_array().unwrap().len(), 1);
    assert_eq!(r["tests"][0]["test"], "pass-and-fail.html");
    fs::remove_dir_all(root).unwrap();
}
