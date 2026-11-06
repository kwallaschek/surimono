//! Runs a page's own scripts with surimono and writes the HTML.
//!
//! From the repository root:
//!
//! ```sh
//! cargo run -p surimono-example-rust-islands --bin page -- [INPUT.html [OUTPUT.html]]
//! ```
//!
//! Without arguments it renders `examples/shared/page.html` to stdout. The page
//! loads a stylesheet and a script from `/assets/`, which the resource loader
//! below serves from `examples/shared/assets`. What happened while the page ran
//! goes to stderr. A script's exception doesn't fail the render; a limit does.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use surimono::{Options, PageReport, Renderer, ResourceLoader};

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
    let root = repo_root();
    let mut args = std::env::args_os().skip(1);
    let input = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("examples/shared/page.html"));
    let output = args.next().map(PathBuf::from);

    // Build the renderer once and reuse it for every page.
    let renderer = build_renderer(&root)?;

    let page = renderer.render_page(&std::fs::read_to_string(&input)?)?;
    print_report(&page.report);

    match output {
        Some(path) => std::fs::write(path, page.html)?,
        None => print!("{}", page.html),
    }
    Ok(())
}

/// The fonts in fallback order, and a loader that serves `/assets/` from one
/// folder. The URL arrives as the page wrote it, with no base applied. The
/// allowlist refuses paths that climb out of `/assets/`.
fn build_renderer(root: &Path) -> Result<Renderer, Box<dyn std::error::Error>> {
    let mut options = Options::new();
    for name in [
        "NotoSans-Regular.ttf",
        "NotoSans-Bold.ttf",
        "NotoSansJP-Regular.otf",
        "NotoSansJP-Bold.otf",
    ] {
        options = options.font(std::fs::read(root.join("fixtures/fonts").join(name))?);
    }
    let assets = root.join("examples/shared/assets");
    let loader = ResourceLoader::new(["/assets/"], move |url| {
        let name = url.strip_prefix("/assets/").unwrap_or(url);
        std::fs::read(assets.join(name)).map_err(|e| e.to_string())
    });
    Ok(Renderer::new(options.resource_loader(loader))?)
}

/// What happened while the page ran. None of this failed the render.
fn print_report(report: &PageReport) {
    eprintln!("settled: {:?}", report.settled);
    for error in &report.errors {
        eprintln!("uncaught: {error}");
    }
    for reason in &report.rejections {
        eprintln!("rejected: {reason}");
    }
    for failure in &report.failed {
        eprintln!(
            "not loaded: {:?} {}: {}",
            failure.kind, failure.url, failure.reason
        );
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
