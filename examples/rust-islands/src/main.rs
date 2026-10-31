//! Renders the chart islands in a report with surimono and writes the HTML.
//!
//! From the repository root:
//!
//! ```sh
//! cargo run -p surimono-example-rust-islands -- [INPUT.html [OUTPUT.html]]
//! ```
//!
//! Without arguments it renders `examples/shared/report.html` to stdout. The
//! report has an ECharts island and an island drawn by a bundle of our own,
//! `examples/shared/sparkline.js`. Hand the result to a PDF engine such as
//! fulgur, with the same font files (see the README).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use surimono::{Error, Options, Renderer};

/// The fonts, in fallback order: Latin first, then Japanese. In your app, use
/// the files your PDF engine prints with.
const FONTS: [&str; 4] = [
    "NotoSans-Regular.ttf",
    "NotoSans-Bold.ttf",
    "NotoSansJP-Regular.otf",
    "NotoSansJP-Bold.otf",
];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("examples/shared/report.html"));
    let output = args.next().map(PathBuf::from);

    // Build the renderer once and reuse it for every document.
    let renderer = build_renderer()?;

    let html = std::fs::read_to_string(&input)?;
    let rendered = renderer.render_islands(&html).inspect_err(explain)?;

    match output {
        Some(path) => std::fs::write(path, rendered)?,
        None => print!("{rendered}"),
    }
    Ok(())
}

/// The renderer: our fonts, plus the sparkline bundle as renderer `sparkline`.
fn build_renderer() -> Result<Renderer, Box<dyn std::error::Error>> {
    let root = repo_root();
    let mut options = Options::new();
    for name in FONTS {
        options = options.font(std::fs::read(root.join("fixtures/fonts").join(name))?);
    }
    let sparkline = std::fs::read_to_string(root.join("examples/shared/sparkline.js"))?;
    options = options.bundle("sparkline", sparkline, "render");
    Ok(Renderer::new(options)?)
}

/// Says which island failed, and prints the JavaScript stack if there is one.
fn explain(error: &Error) {
    if let Some(island) = error.island() {
        eprintln!(
            "failed island: index {} (from 0), id {:?}, renderer {:?}, kind {}",
            island.index,
            island.id,
            island.renderer,
            error.kind()
        );
    }
    if let Error::JavaScript {
        stack: Some(stack), ..
    } = error
    {
        eprintln!("JavaScript stack:\n{stack}");
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
