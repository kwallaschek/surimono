//! Runs the `surimono` binary as a user would.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const ISLAND: &str = "<script type=\"application/vnd.surimono+json\" data-renderer=\"echarts\" \
    data-width=\"200\" data-height=\"100\">{\"xAxis\":{\"type\":\"category\",\"data\":[\"Q1\",\"Q2\"]},\
    \"yAxis\":{\"type\":\"value\"},\"series\":[{\"type\":\"bar\",\"data\":[3,5]}]}</script>";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn font() -> String {
    repo()
        .join("fixtures/fonts/NotoSans-Regular.ttf")
        .display()
        .to_string()
}

fn run(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_surimono"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("{e}"));
    if let Some(mut input) = child.stdin.take() {
        input
            .write_all(stdin.as_bytes())
            .unwrap_or_else(|e| panic!("{e}"));
    }
    child.wait_with_output().unwrap_or_else(|e| panic!("{e}"))
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap_or(-1)
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn help_and_version() {
    let o = run(&["--help"], "");
    assert_eq!(code(&o), 0);
    assert!(text(&o.stdout).contains("Exit codes:"));
    let o = run(&["--version"], "");
    assert_eq!(
        text(&o.stdout).trim(),
        format!("surimono {}", surimono::VERSION)
    );
}

#[test]
fn stdin_to_stdout_leaves_a_document_without_islands_alone() {
    let html = "<!doctype html>\r\n<p>a &amp; b</p>";
    let o = run(&["render", "-", "--font", &font()], html);
    assert_eq!(code(&o), 0, "{}", text(&o.stderr));
    assert_eq!(text(&o.stdout), html);
}

#[test]
fn renders_an_echarts_island_from_file_to_file() {
    let dir = std::env::temp_dir().join(format!("surimono-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let input = dir.join("in.html");
    let output = dir.join("out.html");
    std::fs::write(&input, format!("<h1>Q</h1>{ISLAND}<p>end</p>"))
        .unwrap_or_else(|e| panic!("{e}"));
    let o = run(
        &[
            "render",
            &input.display().to_string(),
            "-o",
            &output.display().to_string(),
            "--font",
            &font(),
        ],
        "",
    );
    assert_eq!(code(&o), 0, "{}", text(&o.stderr));
    let out = std::fs::read_to_string(&output).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        out.starts_with("<h1>Q</h1><div class=\"surimono-island\" data-renderer=\"echarts\"><svg"),
        "{out}"
    );
    assert!(out.ends_with("</svg></div><p>end</p>"));
    assert!(!out.contains("<text"), "text is outlined by default");
    let kept = run(
        &[
            "render",
            &input.display().to_string(),
            "--font",
            &font(),
            "--text",
            "keep",
        ],
        "",
    );
    assert!(
        text(&kept.stdout).contains("<text"),
        "{}",
        text(&kept.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exit_codes_are_typed() {
    // 1: an island failed.
    let o = run(
        &["render", "-", "--font", &font()],
        "<script type=\"application/vnd.surimono+json\" data-renderer=\"plotly\">{}</script>",
    );
    assert_eq!(code(&o), 1);
    assert!(
        text(&o.stderr).contains("[unknown-renderer]"),
        "{}",
        text(&o.stderr)
    );
    // ...unless failures become placeholders.
    let o = run(
        &[
            "render",
            "-",
            "--font",
            &font(),
            "--error-policy",
            "placeholder",
        ],
        "<script type=\"application/vnd.surimono+json\" data-renderer=\"plotly\">{}</script>",
    );
    assert_eq!(code(&o), 0);
    assert!(text(&o.stdout).contains("surimono-island-error"));
    // 2: usage.
    assert_eq!(
        code(&run(
            &["render", "-", "--colour", "red", "--font", &font()],
            ""
        )),
        2
    );
    assert_eq!(code(&run(&[], "")), 2);
    // 3: a limit. Loading ECharts takes far longer than 1 ms.
    let o = run(
        &["render", "-", "--font", &font(), "--time-ms", "1"],
        ISLAND,
    );
    assert_eq!(code(&o), 3, "{}", text(&o.stderr));
    assert!(text(&o.stderr).contains("[time-limit]"));
    // 4: configuration.
    let not_a_font = repo().join("fixtures/fonts/OFL.txt").display().to_string();
    assert_eq!(code(&run(&["render", "-", "--font", &not_a_font], "")), 4);
    assert_eq!(
        code(&run(
            &["render", "-", "--stack-kb", "1", "--font", &font()],
            ""
        )),
        4
    );
    let o = run(&["render", "-"], "<p>x</p>");
    assert_eq!(code(&o), 4, "no font is a configuration error");
    assert!(text(&o.stderr).contains("no fonts"), "{}", text(&o.stderr));
    // 5: I/O.
    let o = run(&["render", "/no/such/in.html", "--font", &font()], "");
    assert_eq!(code(&o), 5);
    assert!(text(&o.stderr).contains("/no/such/in.html"));
    assert_eq!(
        code(&run(&["render", "-", "--font", "/no/such/font.ttf"], "")),
        5
    );
}

#[test]
fn a_failed_render_writes_no_output_file() {
    let out = std::env::temp_dir().join(format!("surimono-cli-none-{}.html", std::process::id()));
    let o = run(
        &[
            "render",
            "-",
            "-o",
            &out.display().to_string(),
            "--font",
            &font(),
        ],
        "<script type=\"application/vnd.surimono+json\" data-renderer=\"plotly\">{}</script>",
    );
    assert_eq!(code(&o), 1);
    assert!(!out.exists());
}

#[test]
fn same_input_same_output() {
    let a = run(&["render", "-", "--font", &font()], ISLAND);
    let b = run(&["render", "-", "--font", &font()], ISLAND);
    assert_eq!(code(&a), 0, "{}", text(&a.stderr));
    assert_eq!(a.stdout, b.stdout);
}
