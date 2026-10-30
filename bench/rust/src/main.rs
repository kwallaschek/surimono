//! One benchmark process for the Rust path: `surimono::Renderer` replaces the
//! report's island markers with SVG, then the `fulgur` crate prints the HTML.
//! Same protocol as bench/ruby/bench.rb (JSON lines on stdout):
//!
//! ```text
//! surimono-bench <cold|warm> <input.html> <out.pdf> [iterations] [warmup]
//! ```
//!
//! Fonts come from `$BENCH_FONTS_DIR` (fixtures/fonts). Exit code 4 means
//! surimono can't render the report yet (a renderer it needs is missing).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use fulgur::asset::AssetBundle;
use fulgur::engine::Engine;
use serde_json::json;

fn emit(value: serde_json::Value) {
    println!("{value}");
}

fn fail(message: String, code: u8) -> ExitCode {
    emit(json!({ "event": "error", "message": message }));
    eprintln!("{message}");
    ExitCode::from(code)
}

fn font_files() -> Result<Vec<PathBuf>, String> {
    let dir = std::env::var("BENCH_FONTS_DIR").unwrap_or_else(|_| "fixtures/fonts".into());
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("fonts dir {dir}: {e}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| matches!(p.extension().and_then(|x| x.to_str()), Some("ttf" | "otf")))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no fonts in {dir}"));
    }
    Ok(files)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 || !matches!(args[1].as_str(), "cold" | "warm") {
        return fail("usage: surimono-bench <cold|warm> <input.html> <out.pdf> [iterations] [warmup]".into(), 2);
    }
    let (mode, input, out) = (args[1].as_str(), Path::new(&args[2]), Path::new(&args[3]));
    let iterations: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20);
    let warmup: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(2);

    let t_setup = Instant::now();
    let html = match std::fs::read_to_string(input) {
        Ok(h) => h,
        Err(e) => return fail(format!("{}: {e}", input.display()), 2),
    };
    let fonts = match font_files() {
        Ok(f) => f,
        Err(e) => return fail(e, 2),
    };
    let mut options = surimono::Options::new();
    for f in &fonts {
        match std::fs::read(f) {
            Ok(bytes) => options = options.font(bytes),
            Err(e) => return fail(format!("{}: {e}", f.display()), 2),
        }
    }
    let renderer = match surimono::Renderer::new(options) {
        Ok(r) => r,
        Err(e) => return fail(format!("surimono::Renderer::new: {e}"), 3),
    };
    let mut wanted: Vec<&str> = html
        .split("data-renderer=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    let missing: Vec<&str> = wanted.iter().copied().filter(|w| !renderer.renderers().iter().any(|r| r == w)).collect();
    if !missing.is_empty() {
        return fail(
            format!(
                "surimono {} can't render this report yet: renderers {} not available (it has: {})",
                surimono::VERSION,
                missing.join(", "),
                renderer.renderers().join(", ")
            ),
            4,
        );
    }
    let mut bundle = AssetBundle::new();
    for f in &fonts {
        if let Err(e) = bundle.add_font_file(f) {
            return fail(format!("fulgur font {}: {e}", f.display()), 3);
        }
    }
    let engine = Engine::builder().assets(bundle).build();
    let setup_ms = ms(t_setup);

    let render = |html: &str| -> Result<(Vec<u8>, f64, f64), String> {
        let t = Instant::now();
        let prepared = renderer.render_islands(html).map_err(|e| format!("surimono: {e}"))?;
        let islands_ms = ms(t);
        let t = Instant::now();
        let pdf = engine.render(&prepared).map_err(|e| format!("fulgur: {e}"))?;
        Ok((pdf, islands_ms, ms(t)))
    };

    if mode == "cold" {
        let t = Instant::now();
        let (pdf, islands_ms, pdf_ms) = match render(&html) {
            Ok(r) => r,
            Err(e) => return fail(e, 3),
        };
        let render_ms = ms(t);
        if let Err(e) = std::fs::write(out, &pdf) {
            return fail(format!("{}: {e}", out.display()), 3);
        }
        emit(json!({ "event": "cold", "setup_ms": setup_ms, "render_ms": render_ms,
            "phases": { "islands_ms": islands_ms, "pdf_ms": pdf_ms }, "bytes": pdf.len() }));
    } else {
        emit(json!({ "event": "setup", "setup_ms": setup_ms }));
        for i in 0..warmup {
            match render(&html) {
                Ok((pdf, ..)) => emit(json!({ "event": "warmup", "i": i, "bytes": pdf.len() })),
                Err(e) => return fail(e, 3),
            }
        }
        for i in 0..iterations {
            let t = Instant::now();
            let (pdf, islands_ms, pdf_ms) = match render(&html) {
                Ok(r) => r,
                Err(e) => return fail(e, 3),
            };
            let it_ms = ms(t);
            if let Err(e) = std::fs::write(out, &pdf) {
                return fail(format!("{}: {e}", out.display()), 3);
            }
            emit(json!({ "event": "iteration", "i": i, "ms": it_ms,
                "phases": { "islands_ms": islands_ms, "pdf_ms": pdf_ms }, "bytes": pdf.len() }));
        }
    }
    emit(json!({ "event": "done" }));
    ExitCode::SUCCESS
}
