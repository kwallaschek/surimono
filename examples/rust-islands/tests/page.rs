//! Runs the page example the way a user would: the page's scripts ran, what it
//! loaded is in the output, the output is the same on every run, and problems
//! land where the guides say.

use std::path::PathBuf;
use std::process::{Command, Output};

fn page(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_page"))
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("can't run the example binary: {e}"))
}

fn stdout_of(out: &Output) -> String {
    assert!(
        out.status.success(),
        "the example failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout.clone()).unwrap_or_else(|e| panic!("output is not UTF-8: {e}"))
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A page in a file of its own, removed when dropped.
struct TempPage(PathBuf);

impl TempPage {
    fn with(name: &str, html: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "surimono-example-page-{}-{name}.html",
            std::process::id()
        ));
        std::fs::write(&path, html).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
        Self(path)
    }
}

impl Drop for TempPage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn the_page_ran_and_its_files_are_inlined() {
    let out = page(&[]);
    let html = stdout_of(&out);
    assert_eq!(stderr_of(&out), "settled: Idle\n");

    assert!(html.contains("<h1 id=\"title\">Invoice 2026-0117</h1>"));
    assert!(html.contains("<td>印刷 (printing)</td><td>3</td><td>240</td><td>720</td>"));
    assert!(html.contains("<p id=\"total\">Total: 1835</p>"));
    // The marker the timer added became SVG.
    assert!(html.contains(r#"<div class="surimono-island" id="amounts" data-renderer="echarts""#));
    assert!(html.contains("<svg"));
    // The stylesheet is in the page, and nothing needs running or loading.
    assert!(html.contains("<style>"));
    assert!(html.contains("font-family: \"Noto Sans\""));
    assert!(!html.contains("<link"));
    assert!(!html.contains("<noscript"));
    assert!(!html.contains("invoice.js"));
}

#[test]
fn the_output_is_the_same_every_time() {
    assert_eq!(stdout_of(&page(&[])), stdout_of(&page(&[])));
}

#[test]
fn an_exception_goes_in_the_report_and_the_page_goes_on() {
    let file = TempPage::with(
        "throws",
        "<!doctype html><script>throw new Error('boom')</script>\
         <script>document.body.append('after')</script>",
    );
    let out = page(&[file.0.as_os_str()]);
    assert!(stdout_of(&out).contains("after"));
    assert!(stderr_of(&out).contains("uncaught: Error: boom"));
}

#[test]
fn a_file_off_the_list_goes_in_the_report() {
    let file = TempPage::with(
        "off-list",
        "<!doctype html><script src='https://example.com/a.js'></script><p>still here</p>",
    );
    let out = page(&[file.0.as_os_str()]);
    assert!(stdout_of(&out).contains("still here"));
    assert!(
        stderr_of(&out)
            .contains("not loaded: Script https://example.com/a.js: not in the loader's allowlist"),
        "{}",
        stderr_of(&out)
    );
}

#[test]
fn a_limit_fails_the_render() {
    let file = TempPage::with("loop", "<!doctype html><script>while (true) {}</script>");
    let out = page(&[file.0.as_os_str()]);
    assert!(!out.status.success());
    assert!(
        stderr_of(&out).contains("the page exceeded the time limit"),
        "{}",
        stderr_of(&out)
    );
}
