//! Runs fixtures through `Renderer::render_page`, the facade's whole road.
//!
//! ```text
//! surimono-page-check pages --root REPO --out DIR --run NAME [--no-printer-safe] [--keep-scripts] [fixture ...]
//! surimono-page-check hostile --root REPO --out FILE --commit SHA
//! surimono-page-check hostile-one --root REPO ID     # child: one case, JSON on stdout
//! ```
//!
//! `pages` renders each page of `fixtures/real-world` twice with one `Renderer`,
//! to `DIR/<fixture>/<NAME>-1.html` and `<NAME>-2.html`, with `<NAME>.json` saying
//! how it went. `fixtures/tools/print-pages.mjs` runs it twice, for a fresh process.
//! `--no-printer-safe` writes the page's DOM as it is (`Options::printer_safe(false)`),
//! and `--keep-scripts` keeps its `<script>` elements (`Options::keep_scripts(true)`,
//! which also leaves the output plain), so the output can be compared with
//! Chrome's own DOM.
//!
//! `hostile` runs `fixtures/hostile/manifest.toml` as pages, each case in its own
//! child, in the results format of `fixtures/hostile/RESULTS_SCHEMA.md` with
//! `as_expected` added. A case is evaluated by a page's script, so a page's
//! answer is compared with the manifest's: `fixtures/README.md`, "Hostile scripts
//! as pages", says what differs from an island.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use surimono::{Error, Limits, Options, PageOutput, Renderer, ResourceLoader};

/// Slack on `max_wall_ms` for a child's start, and a loaded machine.
const CHILD_SLACK: Duration = Duration::from_secs(20);
/// Children at once.
const PARALLEL: usize = 2;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let root = PathBuf::from(flag("--root").unwrap_or_else(|| ".".to_owned()));
    let result = match args.first().map(String::as_str) {
        Some("pages") => pages(
            &root,
            flag("--out").as_deref(),
            flag("--run").as_deref(),
            Mode {
                printer_safe: !args.iter().any(|a| a == "--no-printer-safe"),
                keep_scripts: args.iter().any(|a| a == "--keep-scripts"),
            },
            &names(&args),
        ),
        Some("hostile") => hostile(&root, flag("--out").as_deref(), flag("--commit").as_deref()),
        Some("hostile-one") => match args.last() {
            Some(id) => hostile_one(&root, id).map(|v| println!("{v}")),
            None => Err("hostile-one needs a case".to_owned()),
        },
        _ => Err("usage: surimono-page-check pages|hostile|hostile-one …".to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// The flags of `pages` that take no value.
const SWITCHES: [&str; 2] = ["--no-printer-safe", "--keep-scripts"];

/// How `pages` renders: the facade's two output options.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Mode {
    printer_safe: bool,
    keep_scripts: bool,
}

/// The arguments after the subcommand that aren't a flag or a flag's value.
fn names(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = args.iter().skip(1);
    while let Some(a) = rest.next() {
        if SWITCHES.contains(&a.as_str()) {
            continue;
        }
        if a.starts_with("--") {
            rest.next();
        } else {
            out.push(a.clone());
        }
    }
    out
}

fn options(root: &Path) -> Result<Options, String> {
    let dir = root.join("fixtures/fonts");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "ttf" || x == "otf"))
        .collect();
    files.sort();
    let mut options = Options::new();
    for file in files {
        let bytes = fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        options = options.font(bytes);
    }
    Ok(options)
}

/// The URLs a page loads: each `src` and `href` in a `<script>` or `<link>`,
/// as written.
fn sources(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tag in input.split('<').skip(1) {
        if !(tag.starts_with("script") || tag.starts_with("link")) {
            continue;
        }
        let tag = tag.split('>').next().unwrap_or(tag);
        for name in [" src=\"", " href=\""] {
            if let Some((_, rest)) = tag.split_once(name) {
                out.extend(rest.split('"').next().map(str::to_owned));
            }
        }
    }
    out
}

fn page_json(result: &Result<PageOutput, Error>) -> Value {
    match result {
        Ok(p) => json!({
            "settled": format!("{:?}", p.report.settled),
            "uncaught": p.report.uncaught,
            "errors": p.report.errors,
            "unhandled_rejections": p.report.unhandled_rejections,
            "failed_count": p.report.failed_count,
            "bytes": p.html.len(),
        }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

fn pages(
    root: &Path,
    out: Option<&str>,
    run: Option<&str>,
    mode: Mode,
    only: &[String],
) -> Result<(), String> {
    let (Some(out), Some(run)) = (out, run) else {
        return Err("pages needs --out and --run".to_owned());
    };
    let suite = root.join("fixtures/real-world");
    let mut list: Vec<String> = fs::read_dir(&suite)
        .map_err(|e| format!("{}: {e}", suite.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("input.html").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| only.is_empty() || only.contains(n))
        .collect();
    list.sort();
    for name in list {
        let dir = suite.join(&name);
        let input = fs::read_to_string(dir.join("input.html")).map_err(|e| e.to_string())?;
        let mut files = Vec::new();
        for url in sources(&input) {
            let path = dir.join(&url);
            let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            files.push((url, bytes));
        }
        let renderer = Renderer::new(
            options(root)?
                .resource_loader(ResourceLoader::from_map(files))
                .printer_safe(mode.printer_safe)
                .keep_scripts(mode.keep_scripts)
                .pool_size(1),
        )
        .map_err(|e| e.to_string())?;
        let target = Path::new(out).join(&name);
        fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        let mut reports = Vec::new();
        for pass in 1..=2 {
            let started = Instant::now();
            let result = renderer.render_page(&input);
            let mut report = page_json(&result);
            report["ms"] = json!(started.elapsed().as_millis() as u64);
            reports.push(report);
            if let Ok(page) = result {
                let file = target.join(format!("{run}-{pass}.html"));
                fs::write(file, page.html).map_err(|e| e.to_string())?;
            }
        }
        let text = serde_json::to_string_pretty(&reports).map_err(|e| e.to_string())?;
        fs::write(target.join(format!("{run}.json")), text).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn manifest(root: &Path) -> Result<toml::Table, String> {
    let path = root.join("fixtures/hostile/manifest.toml");
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.parse().map_err(|e| format!("{}: {e}", path.display()))
}

fn cases(manifest: &toml::Table) -> Vec<toml::Value> {
    manifest
        .get("case")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default()
}

/// A case's files with what each must give: one, or its `steps`.
fn steps(case: &toml::Value) -> Vec<toml::Value> {
    match case.get("steps").and_then(|s| s.as_array()) {
        Some(list) => list.clone(),
        None => vec![case.clone()],
    }
}

/// `text` as a string literal safe inside a `<script>`.
fn literal(text: &str) -> Result<String, String> {
    let json = serde_json::to_string(text).map_err(|e| e.to_string())?;
    Ok(json.replace('<', "\\u003c"))
}

/// A page that does what the islands' harness does with a case's script: its
/// completion value, a string or a promise of one. Printable ASCII in the output,
/// so nothing a serializer does can change the answer.
fn wrap(source: &str) -> Result<String, String> {
    Ok(format!(
        r#"<!doctype html><pre id=out></pre><script>
const out = document.getElementById("out");
const stringify = JSON.stringify, replace = String.prototype.replace;
const put = (kind, value) => {{
  const text = stringify([kind, value]);
  const safe = replace.call(text, /[^ -~]|[<>&]/g, (c) => "\\u" + ("000" + c.charCodeAt(0).toString(16)).slice(-4));
  out.textContent = safe;
}};
const settle = (v) => typeof v === "string" ? put("ok", v) : put("unusable", typeof v);
Promise.resolve((0, eval)({})).then(settle, (e) => put("rejected", String(e)));
</script>"#,
        literal(source)?
    ))
}

/// The answer `wrap`'s page left: `(kind, value)`.
fn answer(html: &str) -> Option<(String, String)> {
    let (_, rest) = html.split_once("<pre id=\"out\">")?;
    let text = rest.split("</pre>").next()?;
    let list: Vec<String> = serde_json::from_str(text).ok()?;
    let [kind, value] = <[String; 2]>::try_from(list).ok()?;
    Some((kind, value))
}

fn outcome(result: Result<PageOutput, Error>, started: Instant) -> Value {
    let wall_ms = started.elapsed().as_secs_f64() * 1e3;
    let (outcome, output, message) = match result {
        Err(Error::PageMemoryLimit) => ("MemoryLimit", None, None),
        Err(Error::PageTimeLimit) => ("TimeLimit", None, None),
        Err(Error::PageStackOverflow) => ("StackOverflow", None, None),
        Err(e) => ("Exception", None, Some(e.to_string())),
        Ok(page) => match answer(&page.html) {
            Some((kind, value)) if kind == "ok" => ("Ok", Some(value), None),
            Some((_, value)) => ("Exception", None, Some(value)),
            None if page.report.uncaught > 0 => {
                ("Exception", None, page.report.errors.first().cloned())
            }
            None => (
                "Exception",
                None,
                Some("the page gave no answer".to_owned()),
            ),
        },
    };
    json!({ "outcome": outcome, "output": output, "message": message, "wall_ms": wall_ms })
}

/// Whether a result is what the manifest's `expect`, `expect_any`, `output`
/// and `error` ask.
fn as_expected(step: &toml::Value, result: &Value) -> bool {
    let got = result["outcome"].as_str().unwrap_or("");
    let outcome_fits = match (step.get("expect"), step.get("expect_any")) {
        (Some(one), _) => one.as_str() == Some(got),
        (None, Some(any)) => any
            .as_array()
            .is_some_and(|l| l.iter().any(|o| o.as_str() == Some(got))),
        _ => false,
    };
    let output_fits = match step.get("output").and_then(|o| o.as_str()) {
        Some(want) if got == "Ok" => result["output"].as_str() == Some(want),
        _ => true,
    };
    let error_fits = match step.get("error").and_then(|e| e.as_str()) {
        Some(want) if got == "Exception" => {
            result["message"].as_str().is_some_and(|m| m.contains(want))
        }
        _ => true,
    };
    outcome_fits && output_fits && error_fits
}

fn limits(manifest: &toml::Table) -> Result<(usize, u64, usize, u64), String> {
    let table = manifest.get("limits").ok_or("no [limits]")?;
    let number = |k: &str| {
        table
            .get(k)
            .and_then(|v| v.as_integer())
            .and_then(|v| u64::try_from(v).ok())
            .ok_or_else(|| format!("no limits.{k}"))
    };
    Ok((
        number("memory_bytes")? as usize,
        number("time_ms")?,
        number("stack_bytes")? as usize,
        number("max_wall_ms")?,
    ))
}

/// One case in this process; each step is one render, on one `Renderer`.
fn hostile_one(root: &Path, id: &str) -> Result<Value, String> {
    let manifest = manifest(root)?;
    let (memory_bytes, time_ms, stack_bytes, _) = limits(&manifest)?;
    let case = cases(&manifest)
        .into_iter()
        .find(|c| c.get("id").and_then(|v| v.as_str()) == Some(id))
        .ok_or_else(|| format!("no case {id}"))?;
    let renderer = Renderer::new(options(root)?.pool_size(1).limits(Limits {
        memory_bytes,
        time: Duration::from_millis(time_ms),
        stack_bytes,
        ..Limits::default()
    }))
    .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for step in steps(&case) {
        let file = step.get("file").and_then(|f| f.as_str()).ok_or("no file")?;
        let path = root.join("fixtures/hostile").join(file);
        let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let html = wrap(&String::from_utf8_lossy(&bytes))?;
        let started = Instant::now();
        let result = outcome(renderer.render_page(&html), started);
        out.push(result);
    }
    Ok(Value::from(out))
}

/// A case in a child; a death or a hang is a result.
fn child(exe: &Path, root: &Path, id: &str, wall: Duration) -> Result<Value, String> {
    let mut child = Command::new(exe)
        .args(["hostile-one", "--root"])
        .arg(root)
        .arg(id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break Some(status),
            None if started.elapsed() > wall => break None,
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    let dead = |outcome: &str| {
        let wall_ms = started.elapsed().as_secs_f64() * 1e3;
        json!([{ "outcome": outcome, "output": null, "message": null, "wall_ms": wall_ms }])
    };
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        return Ok(dead("Hang"));
    };
    let text = reader.join().unwrap_or_default();
    match serde_json::from_str::<Value>(text.trim()) {
        Ok(v) if status.success() => Ok(v),
        _ => Ok(dead("Crash")),
    }
}

fn hostile(root: &Path, out: Option<&str>, commit: Option<&str>) -> Result<(), String> {
    let (Some(out), Some(commit)) = (out, commit) else {
        return Err("hostile needs --out and --commit".to_owned());
    };
    let manifest = manifest(root)?;
    let (.., max_wall_ms) = limits(&manifest)?;
    let wall = Duration::from_millis(max_wall_ms) + CHILD_SLACK;
    let list = cases(&manifest);
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let next = AtomicUsize::new(0);
    let done: Mutex<BTreeMap<usize, Value>> = Mutex::default();
    std::thread::scope(|s| {
        for _ in 0..PARALLEL {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(case) = list.get(i) else { break };
                    let id = case.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let result = child(&exe, root, id, wall).unwrap_or_else(
                        |e| json!([{ "outcome": "Crash", "output": null, "message": e, "wall_ms": 0 }]),
                    );
                    if let Ok(mut d) = done.lock() {
                        d.insert(i, result);
                    }
                }
            });
        }
    });
    let done = done.into_inner().map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for (i, results) in done {
        let case = &list[i];
        let parts = steps(case);
        let numbered = case.get("steps").is_some();
        for (n, part) in parts.iter().enumerate() {
            let result = results
                .get(n)
                .cloned()
                .unwrap_or(json!({ "outcome": "Crash" }));
            let mut entry = json!({
                "id": case.get("id").and_then(|v| v.as_str()),
                "step": if numbered { json!(n + 1) } else { Value::Null },
                "outcome": result["outcome"],
                "output": result["output"],
                "message": result["message"],
                "wall_ms": result["wall_ms"],
            });
            entry["as_expected"] = json!(as_expected(part, &result));
            entries.push(entry);
        }
    }
    let total = entries.len();
    let fits = entries.iter().filter(|e| e["as_expected"] == true).count();
    eprintln!("{fits} of {total} as the manifest says");
    let doc = json!({ "schema": 1, "commit": commit, "harness": "fixtures/tools/page-check", "cases": entries });
    let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    fs::write(out, text + "\n").map_err(|e| format!("{out}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_scripts_and_links_as_written() {
        let input = r#"<link rel="stylesheet" href="a.css"><script src="../b.js"></script>
            <a href="no.html"></a><script>var s = "<img src=x>";</script>"#;
        assert_eq!(sources(input), ["a.css", "../b.js"]);
    }

    #[test]
    fn switches_take_no_value_and_are_no_page() {
        let args: Vec<String> = [
            "pages",
            "--root",
            "r",
            "--no-printer-safe",
            "d3-charts",
            "--run",
            "a",
            "--keep-scripts",
            "marked-report",
        ]
        .map(str::to_owned)
        .to_vec();
        assert_eq!(names(&args), ["d3-charts", "marked-report"]);
    }

    #[test]
    fn a_case_cannot_close_the_script_it_runs_in() {
        let page = wrap("</script><p>").unwrap();
        assert!(!page.contains("</script><p>"), "{page}");
    }

    #[test]
    fn answers_read_back_and_a_page_without_one_has_none() {
        assert_eq!(
            answer(r#"<pre id="out">["ok","a<b"]</pre>"#),
            Some(("ok".to_owned(), "a<b".to_owned()))
        );
        assert_eq!(answer("<pre id=\"out\"></pre>"), None);
        assert_eq!(answer("<p>"), None);
    }

    #[test]
    fn the_manifest_decides() {
        let step: toml::Value = "expect = \"Ok\"\noutput = \"x\""
            .parse::<toml::Table>()
            .unwrap()
            .into();
        assert!(as_expected(
            &step,
            &json!({ "outcome": "Ok", "output": "x" })
        ));
        assert!(!as_expected(
            &step,
            &json!({ "outcome": "Ok", "output": "y" })
        ));
        assert!(!as_expected(&step, &json!({ "outcome": "Crash" })));
        let any: toml::Value = "expect_any = [\"TimeLimit\", \"Ok\"]"
            .parse::<toml::Table>()
            .unwrap()
            .into();
        assert!(as_expected(&any, &json!({ "outcome": "TimeLimit" })));
        assert!(!as_expected(&any, &json!({ "outcome": "Hang" })));
    }
}
