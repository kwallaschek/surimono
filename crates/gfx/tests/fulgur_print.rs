//! The real island path: inline `<svg>` printed by fulgur, checked with
//! `pdffonts`. fulgur 0.40.0 prints inline SVG text through blitz-dom's usvg
//! with a font database of system fonts only (blitz-dom 0.2.4 `util.rs`), so:
//!
//! - with `TextMode::Outlines` (the default) the PDF holds no font for island
//!   text at all, only paths, and HTML text uses the registered fonts;
//! - with `TextMode::Keep` island text prints in a system font, or not at all.
//!   That is the known failure recorded in docs/COMPAT.md; the test asserts it
//!   still happens, so a fixed fulgur or Blitz shows up here.
//!
//! Needs the `fulgur` CLI 0.40.0 (`cargo install fulgur-cli --version 0.40.0`,
//! or `$SURIMONO_FULGUR`), `pdffonts` and `pdftotext` (poppler-utils). Without
//! them the test prints why and passes, unless `SURIMONO_REQUIRE_FULGUR=1`.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use surimono_gfx::{SvgOptions, TextMeasurer, TextMode, sanitize_svg};

/// PostScript names of the registered test fonts, as pdffonts prints them.
const REGISTERED: [&str; 4] = [
    "NotoSans-Regular",
    "NotoSans-Bold",
    "NotoSansJP-Regular",
    "NotoSansJP-Bold",
];

const ECHARTS_RICH: &str = include_str!("data/svg/echarts-6.1.0-rich.svg");

/// The reviewer's repro from #16: one HTML label and one SVG label.
const REPRO_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="60"><text x="0" y="20" font-family="'Noto Sans', 'Noto Sans JP'" font-size="14">SVG 売上高 Revenue</text></svg>"#;

/// A pdffonts name of a registered font (Krilla appends `-Identity-H`).
fn is_registered(font: &str) -> bool {
    REGISTERED.iter().any(|r| {
        font.strip_prefix(r)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
    })
}

fn tool(env: &str, name: &str) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(env) {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(name))
            .find(|p| p.is_file())
    })
}

struct Tools {
    fulgur: PathBuf,
    pdffonts: PathBuf,
    pdftotext: PathBuf,
}

fn tools() -> Option<Tools> {
    let found = (|| {
        Some(Tools {
            fulgur: tool("SURIMONO_FULGUR", "fulgur")?,
            pdffonts: tool("SURIMONO_PDFFONTS", "pdffonts")?,
            pdftotext: tool("SURIMONO_PDFTOTEXT", "pdftotext")?,
        })
    })();
    if found.is_none() {
        assert!(
            std::env::var_os("SURIMONO_REQUIRE_FULGUR").is_none(),
            "SURIMONO_REQUIRE_FULGUR is set but fulgur, pdffonts or pdftotext is missing"
        );
        eprintln!("skipped: needs the fulgur CLI 0.40.0, pdffonts and pdftotext");
    }
    found
}

/// Prints `html` with fulgur and the registered test fonts; returns the font
/// names pdffonts lists (subset tags removed) and the PDF's text.
fn print(tools: &Tools, html: &str, name: &str) -> (Vec<String>, String) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fulgur-print");
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join(format!("{name}.html"));
    let pdf = dir.join(format!("{name}.pdf"));
    std::fs::write(&input, html).unwrap();
    let mut cmd = Command::new(&tools.fulgur);
    cmd.arg("render");
    for f in common::FONT_FILES {
        cmd.arg("-f").arg(common::fonts_dir().join(f));
    }
    let out = cmd.arg("-o").arg(&pdf).arg(&input).output().unwrap();
    assert!(
        out.status.success(),
        "fulgur: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let fonts = Command::new(&tools.pdffonts).arg(&pdf).output().unwrap();
    assert!(fonts.status.success());
    let names = String::from_utf8_lossy(&fonts.stdout)
        .lines()
        .skip(2)
        .filter_map(|l| l.split_whitespace().next())
        .map(|n| n.split_once('+').map_or(n, |(_, rest)| rest).to_string())
        .collect();
    let text = Command::new(&tools.pdftotext)
        .arg(&pdf)
        .arg("-")
        .output()
        .unwrap();
    (names, String::from_utf8_lossy(&text.stdout).into_owned())
}

fn page(svg: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"></head><body>\
         <p style=\"font:14px 'Noto Sans JP'\">HTML 売上高 Revenue</p>{svg}</body></html>"
    )
}

#[test]
fn outlined_island_text_prints_without_system_fonts() {
    let Some(tools) = tools() else { return };
    let measurer = TextMeasurer::new(common::test_fonts());
    for (name, svg) in [("repro", REPRO_SVG), ("echarts", ECHARTS_RICH)] {
        let island = sanitize_svg(svg, 0, &measurer, &SvgOptions::new()).unwrap();
        let (fonts, text) = print(&tools, &page(&island), &format!("outlines-{name}"));
        println!("{name}, outlines: pdffonts lists {fonts:?}");
        assert!(!fonts.is_empty(), "the HTML label needs a font");
        for f in &fonts {
            assert!(is_registered(f), "{name}: system font {f} in the PDF");
        }
        // Island text is paths now: only the HTML label is text in the PDF.
        assert!(text.contains("HTML"), "{text}");
        assert!(
            !text.contains("SVG") && !text.contains("Monthly"),
            "{name}: {text}"
        );
    }
}

/// Expected failure, recorded in docs/COMPAT.md: with `TextMode::Keep`,
/// fulgur 0.40.0 prints island text in system fonts (or drops it where none
/// covers it). If this starts failing, fulgur fixed inline SVG fonts: update
/// COMPAT.md and HANDOFF.md, and reconsider the default.
#[test]
fn kept_island_text_prints_in_system_fonts_expected_failure() {
    let Some(tools) = tools() else { return };
    let measurer = TextMeasurer::new(common::test_fonts());
    let keep = SvgOptions::new().with_text(TextMode::Keep);
    let island = sanitize_svg(REPRO_SVG, 0, &measurer, &keep).unwrap();
    let (fonts, text) = print(&tools, &page(&island), "keep-repro");
    println!("repro, keep: pdffonts lists {fonts:?}");
    let system: Vec<&String> = fonts.iter().filter(|f| !is_registered(f)).collect();
    assert!(
        !system.is_empty() || !text.contains("SVG"),
        "SVG text printed in registered fonts only ({fonts:?}): fulgur fixed inline SVG \
         fonts; update docs/COMPAT.md and crates/gfx/HANDOFF.md"
    );
}
