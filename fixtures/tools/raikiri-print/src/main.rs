//! `fulgur render`'s command line, as far as the printer matrix uses it, on
//! fulgur's Raikiri layout backend (the `fulgur-raikiri` crate).
//!
//! ```text
//! surimono-raikiri-print --version
//! surimono-raikiri-print render [--font FILE]... [--creation-date DATE] -o OUT.pdf IN.html
//! ```
//!
//! The page prints as `fulgur render` prints it with the same arguments: the
//! library's default configuration (A4 and its default margin, which the page's
//! own `@page` overrides), the fonts given with `--font` and no others. System
//! fonts are switched off, which the fulgur CLI can't be told.

use std::path::PathBuf;
use std::process::ExitCode;

use fulgur_core::{AssetBundle, Config};
use fulgur_raikiri::{RenderOptions, render_with_options};

/// The version of the `fulgur-raikiri` crate this is built on (Cargo.toml pins it).
const RAIKIRI: &str = "0.41.0";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") {
        println!("fulgur-raikiri {RAIKIRI}");
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    const USAGE: &str = "usage: surimono-raikiri-print render [--font FILE]... [--creation-date DATE] -o OUT.pdf IN.html";
    if args.first().map(String::as_str) != Some("render") {
        return Err(USAGE.to_owned());
    }
    let mut fonts: Vec<PathBuf> = Vec::new();
    let mut date = None;
    let mut output = None;
    let mut input = None;
    let mut rest = args[1..].iter();
    while let Some(arg) = rest.next() {
        let mut value = || {
            rest.next()
                .ok_or_else(|| format!("{arg} needs a value\n{USAGE}"))
        };
        match arg.as_str() {
            "--font" | "-f" => fonts.push(PathBuf::from(value()?)),
            "--creation-date" => date = Some(value()?.clone()),
            "-o" | "--output" => output = Some(PathBuf::from(value()?)),
            other if other.starts_with('-') => {
                return Err(format!("unknown option {other}\n{USAGE}"));
            }
            other if input.is_none() => input = Some(PathBuf::from(other)),
            other => return Err(format!("a second input, {other}\n{USAGE}")),
        }
    }
    let (Some(input), Some(output)) = (input, output) else {
        return Err(USAGE.to_owned());
    };

    let mut bundle = AssetBundle::new();
    for font in &fonts {
        bundle
            .add_font_file(font)
            .map_err(|e| format!("font {}: {e}", font.display()))?;
    }
    let mut config = Config::builder();
    if let Some(date) = date {
        config = config.creation_date(date);
    }
    let options = RenderOptions {
        assets: Some(&bundle),
        // Only the fonts given: the library refuses this without one.
        system_fonts: false,
        ..RenderOptions::default()
    };
    let pdf = render_with_options(&input, &config.build(), &options).map_err(|e| e.to_string())?;
    std::fs::write(&output, pdf).map_err(|e| format!("{}: {e}", output.display()))
}
