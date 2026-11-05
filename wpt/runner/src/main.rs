//! Runs the vendored WPT slices through the real `testharness.js` on
//! `surimono-dom`, and writes the results `compat.py --wpt` scores.
//! `wpt/README.md`, "Runner", is the design. Each test runs in its own child
//! process, so a crash is a result for that test and nothing else.
//!
//! ```text
//! surimono-wpt-runner --root wpt --out wpt-results.json [--jobs 2] [--filter STR]
//!                     [--commit SHA] [--time-limit-ms N] [--wall-limit-ms N]
//! surimono-wpt-runner --root wpt --one URL FILE   # child: one test, JSON on stdout
//! surimono-wpt-runner --probe FILE                # wpt/probes: one page, JSON on stdout
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use surimono_core::{Limits, SettleOptions};
use surimono_dom::{Document, DomRuntime};

/// Longer than Limits' default 2 s: dev-profile bindings run some tests, such
/// as the selector tables, far slower than a release build. It makes the rate
/// depend less on the profile and the machine's load, not not at all: this is
/// real time, and a test near it can end either way. Everything else stays at
/// the defaults.
const TIME_LIMIT: Duration = Duration::from_secs(20);
/// wptrunner's limit for a `timeout=long` test, on the virtual clock.
const VIRTUAL_BUDGET_MS: u64 = 60_000;
/// A child still alive after this is killed; the sandbox should have ended it.
const WALL_LIMIT: Duration = Duration::from_secs(120);
/// Set by `tests/runner.rs` only, to `HOW URL`, comma-separated: the child for
/// that URL dies or hangs, which no test page can make it do.
const FAULT_ENV: &str = "SURIMONO_WPT_FAULT";
/// Longest failure message kept per subtest.
const MESSAGE_CHARS: usize = 200;

const HOOK_URL: &str = "/resources/testharnessreport.js";
/// WPT's embedder hook. It stores the report in a global, read back by the
/// runner, so nothing is added to the runtime for the sake of a test.
const HOOK: &str = r#"
setup({ output: false, explicit_timeout: true });
add_completion_callback((tests, status) => {
  self.__surimonoWptReport = JSON.stringify({
    status: status.status,
    message: status.message,
    tests: tests.map((t) => ({ name: t.name, status: t.status, message: t.message })),
  });
});
"#;

/// What a browsing context gives every page and `testharness.js` reads, which
/// surimono-dom doesn't have yet: only names the page lacks are defined. It runs
/// as the start of `testharness.js`'s script, so the document stays as WPT wrote
/// it and the vendored file is unchanged. `wpt/README.md`, "Harness environment".
const ENVIRONMENT: &str = r#"
(function (url) {
  const define = (name, value) => {
    if (!(name in self)) {
      Object.defineProperty(self, name, { value, configurable: true, writable: true });
    }
  };
  define("parent", self);
  define("top", self);
  define("opener", null);
  const [, origin, path, search, hash] =
    /^(https?:\/\/[^/]*)([^?#]*)(\?[^#]*)?(#.*)?$/.exec(url);
  const [protocol, , host] = origin.split("/");
  const [hostname, port] = host.split(":");
  define("location", {
    href: url, origin, protocol, host, hostname, port, pathname: path,
    search: search || "", hash: hash || "", toString() { return url; },
  });
})
"#;
/// Where WPT's server would serve a test (wptserve's default host and port).
const ORIGIN: &str = "http://web-platform.test:8000/";

#[derive(Clone, Copy)]
struct TestLimits {
    /// The sandbox's time limit for one test.
    time: Duration,
    /// How long the parent waits for one child.
    wall: Duration,
}

struct Test {
    slice: String,
    url: String,
    file: String,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let root = PathBuf::from(flag("--root").unwrap_or_else(|| "wpt".to_owned()));
    let millis = |name: &str, default: Duration| {
        flag(name)
            .and_then(|v| v.parse().ok())
            .map_or(default, Duration::from_millis)
    };
    let limits = TestLimits {
        time: millis("--time-limit-ms", TIME_LIMIT),
        wall: millis("--wall-limit-ms", WALL_LIMIT),
    };
    if let Some(i) = args.iter().position(|a| a == "--one") {
        let (Some(url), Some(file)) = (args.get(i + 1), args.get(i + 2)) else {
            eprintln!("--one needs URL and FILE");
            return ExitCode::FAILURE;
        };
        inject_fault(url);
        println!("{}", run_one(&root, url, file, limits.time));
        return ExitCode::SUCCESS;
    }
    if let Some(file) = flag("--probe") {
        println!("{}", probe(Path::new(&file)));
        return ExitCode::SUCCESS;
    }
    let Some(out) = flag("--out") else {
        eprintln!("usage: surimono-wpt-runner --root wpt --out FILE [--jobs N] [--filter STR]");
        return ExitCode::FAILURE;
    };
    let jobs = flag("--jobs").and_then(|j| j.parse().ok()).unwrap_or(2);
    match run_all(
        &root,
        Path::new(&out),
        jobs,
        flag("--filter"),
        flag("--commit"),
        limits,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn load_tests(root: &Path) -> Result<(String, Vec<Test>), String> {
    let path = root.join("manifest.toml");
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let manifest: toml::Table = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let field = |v: &toml::Value, k: &str| v.get(k).and_then(|s| s.as_str()).map(str::to_owned);
    let commit = manifest
        .get("commit")
        .and_then(|c| c.as_str())
        .ok_or("manifest.toml has no commit")?
        .to_owned();
    let mut tests = Vec::new();
    for t in manifest
        .get("test")
        .and_then(|t| t.as_array())
        .ok_or("manifest.toml has no tests")?
    {
        let (Some(slice), Some(file), Some(urls)) = (
            field(t, "slice"),
            field(t, "file"),
            t.get("urls").and_then(|u| u.as_array()),
        ) else {
            return Err(format!("bad [[test]] {t:?}"));
        };
        for url in urls.iter().filter_map(|u| u.as_str()) {
            tests.push(Test {
                slice: slice.clone(),
                url: url.to_owned(),
                file: file.clone(),
            });
        }
    }
    Ok((commit, tests))
}

fn run_all(
    root: &Path,
    out: &Path,
    jobs: usize,
    filter: Option<String>,
    commit: Option<String>,
    limits: TestLimits,
) -> Result<(), String> {
    let (wpt_commit, mut tests) = load_tests(root)?;
    if let Some(f) = &filter {
        tests.retain(|t| t.url.contains(f.as_str()));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let next = AtomicUsize::new(0);
    let done: Mutex<BTreeMap<usize, Value>> = Mutex::default();
    std::thread::scope(|s| {
        for _ in 0..jobs.max(1) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(test) = tests.get(i) else { break };
                    let result = child(&exe, root, test, limits);
                    eprintln!(
                        "[{}/{}] {} {}",
                        i + 1,
                        tests.len(),
                        result["status"].as_str().unwrap_or("?"),
                        test.url
                    );
                    if let Ok(mut d) = done.lock() {
                        d.insert(i, result);
                    }
                }
            });
        }
    });
    let done = done.into_inner().map_err(|e| e.to_string())?;
    let results: Vec<Value> = done
        .into_iter()
        .map(|(i, r)| {
            let t = &tests[i];
            json!({
                "slice": t.slice, "test": t.url, "status": r["status"],
                "message": r["message"], "subtests": r["subtests"],
            })
        })
        .collect();
    let doc = json!({
        "wpt_commit": wpt_commit,
        "surimono_commit": commit.unwrap_or_else(|| "unknown".to_owned()),
        "tests": results,
    });
    let text = serde_json::to_string(&doc).map_err(|e| e.to_string())?;
    // One test per line, so a change between two runs reads as a diff.
    let text = text.replace(r#"},{"message""#, "},\n{\"message\"");
    fs::write(out, text + "\n").map_err(|e| format!("{}: {e}", out.display()))
}

/// Runs one test in a child process; whatever happens is a value.
fn child(exe: &Path, root: &Path, test: &Test, limits: TestLimits) -> Value {
    let spawned = Command::new(exe)
        .args(["--root".as_ref(), root.as_os_str()])
        .args(["--time-limit-ms", &limits.time.as_millis().to_string()])
        .args(["--one", &test.url, &test.file])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => return failure("CRASH", format!("spawn: {e}")),
    };
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(out) = stdout.as_mut() {
            let _ = out.read_to_string(&mut text);
        }
        text
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() > limits.wall => break None,
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return failure("CRASH", format!("wait: {e}")),
        }
    };
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        return failure("LIMIT", "the child outlived the wall limit".to_owned());
    };
    let text = reader.join().unwrap_or_default();
    match serde_json::from_str::<Value>(text.trim()) {
        Ok(v) if status.success() => v,
        _ => failure("CRASH", format!("the child ended with {status}")),
    }
}

fn failure(status: &str, message: String) -> Value {
    json!({ "status": status, "message": message, "subtests": [] })
}

/// What [`FAULT_ENV`] asks of the child for `url`, if anything. The stdout of a
/// child that is killed or dies is never read as a result.
fn inject_fault(url: &str) {
    let Ok(spec) = std::env::var(FAULT_ENV) else {
        return;
    };
    for entry in spec.split(',') {
        match entry.split_once(' ') {
            Some(("abort", u)) if u == url => std::process::abort(),
            Some(("exit", u)) if u == url => std::process::exit(3),
            Some(("panic", u)) if u == url => panic!("injected"),
            Some(("garbage", u)) if u == url => {
                println!("not a result");
                std::process::exit(0);
            }
            Some(("hang", u)) if u == url => loop {
                std::thread::sleep(Duration::from_secs(1));
            },
            _ => {}
        }
    }
}

/// One test, in this process. Never panics on a test's account: the parent
/// counts a panic as a crash, which is what it would be.
fn run_one(root: &Path, url: &str, file: &str, time_limit: Duration) -> Value {
    let html = match page(root, file) {
        Ok(html) => html,
        Err(e) => return failure("ERROR", e),
    };
    let doc = match Document::parse(&html) {
        Ok(doc) => doc,
        Err(e) => return failure("ERROR", format!("parse: {e}")),
    };
    let limits = Limits::default().with_time(time_limit);
    let mut rt = match DomRuntime::new(limits) {
        Ok(rt) => rt,
        Err(e) => return failure("ERROR", format!("runtime: {e}")),
    };
    rt.load(doc);
    let options = SettleOptions {
        virtual_budget_ms: VIRTUAL_BUDGET_MS,
        ..SettleOptions::default()
    };
    let base = url.split(['?', '#']).next().unwrap_or(url).to_owned();
    let page_report = rt.run_page(&options, |src| {
        let source = resolve(root, &base, src)?;
        if src.ends_with("/testharness.js") {
            let href = serde_json::to_string(&format!("{ORIGIN}{url}")).ok()?;
            return Some(format!("{ENVIRONMENT}({href});\n{source}"));
        }
        Some(source)
    });
    let page_note = match &page_report {
        Ok(r) => format!(
            "settled {:?}, {} uncaught, first {:?}, missing scripts {:?}",
            r.settle.outcome,
            r.uncaught,
            r.errors.first(),
            r.missing_scripts
        ),
        Err(e) => return failure("LIMIT", format!("run_page: {e}")),
    };
    let mut report = read_report(&mut rt);
    if report.is_none() {
        // wptrunner's model: the page is idle with no report, so time out the
        // harness, and the report comes with what finished.
        match rt.eval::<()>("if (typeof timeout === 'function') timeout();") {
            Ok(()) => {
                if let Err(e) = rt.run_until_settled(&options) {
                    return failure("LIMIT", format!("after timeout(): {e}"));
                }
            }
            Err(e) => return failure("LIMIT", format!("timeout(): {e}")),
        }
        report = read_report(&mut rt);
    }
    match report {
        Some(text) => parse_report(&text),
        None => failure("LIMIT", format!("no report from the harness; {page_note}")),
    }
}

/// One page of `wpt/probes`: its global `L`, with the page report.
fn probe(file: &Path) -> Value {
    let page = (|| {
        let html = fs::read_to_string(file).map_err(|e| e.to_string())?;
        let doc = Document::parse(&html).map_err(|e| e.to_string())?;
        DomRuntime::new(Limits::default().with_time(TIME_LIMIT))
            .map_err(|e| e.to_string())
            .map(|mut rt| {
                rt.load(doc);
                rt
            })
    })();
    let mut rt = match page {
        Ok(rt) => rt,
        Err(e) => return json!({ "failed": e }),
    };
    let report = match rt.run_page(&SettleOptions::default(), |_| None) {
        Ok(r) => json!({
            "uncaught": r.uncaught,
            "errors": r.errors,
            "unhandled_rejections": r.unhandled_rejections,
            "rejections": r.rejections,
        }),
        Err(e) => json!({ "failed": e.to_string() }),
    };
    let log = rt
        .eval::<String>("JSON.stringify(typeof L === 'undefined' ? null : L)")
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or(Value::Null);
    json!({ "log": log, "report": report })
}

fn read_report(rt: &mut DomRuntime) -> Option<String> {
    rt.eval::<String>("typeof __surimonoWptReport === 'string' ? __surimonoWptReport : ''")
        .ok()
        .filter(|s| !s.is_empty())
}

/// The hook's numeric codes as names (`wpt/tests/resources/testharness.js`,
/// `Test.statuses` and `TestsStatus.statuses`). An unknown code is a value too.
fn parse_report(text: &str) -> Value {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return failure("ERROR", "the report isn't JSON".to_owned());
    };
    let harness = match v["status"].as_u64() {
        Some(0) => "OK",
        Some(2) => "TIMEOUT",
        Some(3) => "PRECONDITION_FAILED",
        _ => "ERROR",
    };
    let subtests: Vec<Value> = v["tests"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|t| {
            let status = match t["status"].as_u64() {
                Some(0) => "PASS",
                Some(2) => "TIMEOUT",
                Some(3) => "NOTRUN",
                Some(4) => "PRECONDITION_FAILED",
                _ => "FAIL",
            };
            let message = if status == "PASS" {
                Value::Null
            } else {
                clip(&t["message"])
            };
            json!({ "name": t["name"], "status": status, "message": message })
        })
        .collect();
    json!({ "status": harness, "message": clip(&v["message"]), "subtests": subtests })
}

fn clip(v: &Value) -> Value {
    match v.as_str() {
        Some(s) => Value::from(s.chars().take(MESSAGE_CHARS).collect::<String>()),
        None => Value::Null,
    }
}

/// The document for a test: its own file, or for `.any.js` and `.window.js` the
/// page WPT's server wraps around it.
fn page(root: &Path, file: &str) -> Result<String, String> {
    if [".xhtml", ".xht", ".xml", ".svg"]
        .iter()
        .any(|ext| file.ends_with(ext))
    {
        return Err("needs an XML parser, which surimono-dom doesn't have".to_owned());
    }
    let path = root.join("tests").join(file);
    let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if !(file.ends_with(".any.js") || file.ends_with(".window.js")) {
        return Ok(text);
    }
    let mut head = String::from("<!doctype html>\n<meta charset=utf-8>\n");
    let mut scripts = String::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("//") else {
            if line.trim().is_empty() {
                continue;
            }
            break;
        };
        let Some(meta) = rest.trim_start().strip_prefix("META:") else {
            continue;
        };
        match meta.trim().split_once('=') {
            Some(("title", v)) => head.push_str(&format!("<title>{v}</title>\n")),
            Some(("script", v)) => scripts.push_str(&format!("<script src=\"{v}\"></script>\n")),
            _ => {}
        }
    }
    head.push_str(
        "<script>self.GLOBAL = { isWindow() { return true; }, isWorker() { return false; }, \
         isShadowRealm() { return false; } };</script>\n",
    );
    head.push_str("<script src=\"/resources/testharness.js\"></script>\n");
    head.push_str(&format!("<script src=\"{HOOK_URL}\"></script>\n"));
    head.push_str(&scripts);
    head.push_str("<div id=log></div>\n");
    let name = file.rsplit('/').next().unwrap_or(file);
    head.push_str(&format!("<script src=\"{name}\"></script>\n"));
    Ok(head)
}

/// The source of a `<script src>`, only from `wpt/tests/`, no network.
fn resolve(root: &Path, base: &str, src: &str) -> Option<String> {
    let path = src.split(['?', '#']).next()?;
    if path.contains("://") || path.starts_with("//") {
        return None;
    }
    let joined = if let Some(abs) = path.strip_prefix('/') {
        abs.to_owned()
    } else {
        let dir = base.rsplit_once('/').map_or("", |(d, _)| d);
        format!("{dir}/{path}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    let normal = parts.join("/");
    if format!("/{normal}") == HOOK_URL {
        return Some(HOOK.to_owned());
    }
    let bytes = fs::read(root.join("tests").join(normal)).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_codes_map_as_testharness_defines_them() {
        let r = parse_report(
            r#"{"status":0,"tests":[{"name":"a","status":0},{"name":"b","status":1},
                {"name":"c","status":2},{"name":"d","status":3},{"name":"e","status":4},
                {"name":"f","status":9}]}"#,
        );
        assert_eq!(r["status"], "OK");
        let got: Vec<_> = r["subtests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["status"].as_str().unwrap())
            .collect();
        let want = [
            "PASS",
            "FAIL",
            "TIMEOUT",
            "NOTRUN",
            "PRECONDITION_FAILED",
            "FAIL",
        ];
        assert_eq!(got, want);
        for (code, want) in [
            (1, "ERROR"),
            (2, "TIMEOUT"),
            (3, "PRECONDITION_FAILED"),
            (7, "ERROR"),
        ] {
            let r = parse_report(&format!(r#"{{"status":{code},"tests":[]}}"#));
            assert_eq!(r["status"], want);
        }
        assert_eq!(parse_report("not json")["status"], "ERROR");
    }

    #[test]
    fn scripts_resolve_inside_the_tests_only() {
        let dir = std::env::temp_dir().join(format!("wpt-runner-{}", std::process::id()));
        fs::create_dir_all(dir.join("tests/a/b")).unwrap();
        fs::write(dir.join("tests/a/x.js"), "x").unwrap();
        fs::write(dir.join("secret.js"), "s").unwrap();
        assert_eq!(resolve(&dir, "a/b/t.html", "../x.js?q=1"), Some("x".into()));
        assert_eq!(resolve(&dir, "a/b/t.html", "/a/x.js"), Some("x".into()));
        assert_eq!(resolve(&dir, "a/b/t.html", "../../../secret.js"), None);
        assert_eq!(resolve(&dir, "a/b/t.html", "https://e.test/x.js"), None);
        assert!(
            resolve(&dir, "a/t.html", HOOK_URL)
                .unwrap()
                .contains("setup(")
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
