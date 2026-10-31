//! Runs the example binary the way a user would and checks what they would
//! check: every island became SVG, the rest of the document is untouched, the
//! output is the same on every run, and a broken island is reported clearly.

use std::path::PathBuf;
use std::process::{Command, Output};

fn example(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_surimono-example-rust-islands"))
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("can't run the example binary: {e}"))
}

fn stdout_of(out: Output) -> String {
    assert!(
        out.status.success(),
        "the example failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap_or_else(|e| panic!("output is not UTF-8: {e}"))
}

/// A file in the system temp directory, removed when dropped.
struct TempFile(PathBuf);

impl TempFile {
    fn with(name: &str, contents: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("surimono-example-{}-{name}", std::process::id()));
        std::fs::write(&path, contents).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
        Self(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn renders_every_island_and_keeps_the_rest() {
    let html = stdout_of(example(&[]));

    assert!(
        !html.contains("application/vnd.surimono+json"),
        "an island marker is left:\n{html}"
    );
    assert!(
        html.contains(
            r#"<div class="surimono-island chart" id="revenue" data-renderer="echarts" style="width:680px;height:300px;overflow:hidden"><svg"#
        ),
        "the ECharts island is missing:\n{html}"
    );
    assert!(
        html.contains(
            r#"<div class="surimono-island" id="trend" data-renderer="sparkline" style="width:200px;height:24px;overflow:hidden"><svg"#
        ),
        "the sparkline island is missing:\n{html}"
    );

    // Everything outside the islands is as it was, page script included.
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../shared/report.html"),
    )
    .unwrap_or_else(|e| panic!("read the report: {e}"));
    let head = &source[..source
        .find("<script type=\"application/vnd.surimono+json\"")
        .unwrap_or_else(|| panic!("the report has no island"))];
    assert!(
        html.starts_with(head),
        "the document before the first island changed"
    );
    assert!(html.contains("document.title = \"never runs\";"));
    assert!(html.ends_with("</html>\n"));
}

#[test]
fn same_input_same_output_across_processes() {
    let first = stdout_of(example(&[]));
    let second = stdout_of(example(&[]));
    assert!(first == second, "two runs of the same report differ");
}

#[test]
fn writes_to_a_file_when_asked() {
    let input = TempFile::with("in.html", "<p>no islands</p>");
    let output = TempFile::with("out.html", "");
    stdout_of(example(&[input.0.as_os_str(), output.0.as_os_str()]));
    let written = std::fs::read_to_string(&output.0).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(written, "<p>no islands</p>");
}

#[test]
fn a_broken_island_names_itself() {
    let input = TempFile::with(
        "broken.html",
        r#"<h1>Report</h1>
<script type="application/vnd.surimono+json" id="sales" data-renderer="no-such-chart">{}</script>"#,
    );
    let out = example(&[input.0.as_os_str()]);
    assert!(!out.status.success(), "an unknown renderer must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown renderer"),
        "the error should say what is wrong:\n{stderr}"
    );
    assert!(
        stderr.contains(r#"id Some("sales")"#) && stderr.contains("kind unknown-renderer"),
        "the error should name the island:\n{stderr}"
    );
}
