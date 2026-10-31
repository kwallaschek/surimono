//! The vendored webref IDL: complete, parseable, printable, and robust
//! against truncated input.

use surimono_webidl::ast::Definition;
use surimono_webidl::vendored::{FILES, WEBREF_COMMIT};
use surimono_webidl::{IdlSet, parse};

fn idl_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("idl")
}

#[test]
fn file_list_matches_the_idl_directory() {
    let mut on_disk: Vec<String> = std::fs::read_dir(idl_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".idl"))
        .collect();
    on_disk.sort();
    let listed: Vec<&str> = FILES.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        listed, on_disk,
        "vendored::FILES must list every idl/*.idl, sorted"
    );
}

#[test]
fn sources_md_names_the_pinned_commit_and_every_file() {
    let sources = std::fs::read_to_string(idl_dir().join("SOURCES.md")).unwrap();
    assert!(sources.contains(WEBREF_COMMIT));
    for (name, _) in FILES {
        assert!(
            sources.contains(&format!("`{name}`")),
            "{name} missing from SOURCES.md"
        );
    }
}

#[test]
fn every_vendored_file_parses() {
    for (name, src) in FILES {
        let defs = parse(src).unwrap_or_else(|e| panic!("{name}:{e}"));
        assert!(!defs.is_empty(), "{name}");
    }
    let set = surimono_webidl::vendored::idl_set().unwrap();
    for name in [
        "EventTarget",
        "Node",
        "Element",
        "Document",
        "DOMTokenList",
        "HTMLElement",
        "Window",
    ] {
        assert!(set.interface(name).is_some(), "{name}");
    }
}

fn without_lines(defs: Vec<Definition>) -> Vec<Definition> {
    defs.into_iter()
        .map(|mut d| {
            d.line = 0;
            d
        })
        .collect()
}

#[test]
fn printing_and_reparsing_gives_the_same_tree() {
    for (name, src) in FILES {
        let a = parse(src).unwrap();
        let printed: String = a.iter().map(|d| format!("{d}\n")).collect();
        let b = parse(&printed).unwrap_or_else(|e| panic!("{name} reprinted:{e}"));
        assert!(
            without_lines(a) == without_lines(b),
            "{name}: round trip differs"
        );
    }
}

#[test]
fn no_name_is_defined_twice() {
    let set = surimono_webidl::vendored::idl_set().unwrap();
    assert_eq!(set.duplicate_names(), Vec::<(String, Vec<String>)>::new());
}

#[test]
fn truncated_and_garbled_input_is_an_error_not_a_panic() {
    let (_, dom) = FILES.iter().find(|(n, _)| *n == "dom.idl").unwrap();
    let cuts: Vec<usize> = dom.char_indices().map(|(i, _)| i).step_by(37).collect();
    let mut errors = 0;
    for &cut in &cuts {
        if parse(&dom[..cut]).is_err() {
            errors += 1;
        }
    }
    assert!(errors > cuts.len() / 2, "{errors} of {}", cuts.len());
    // Dropping single characters from a known-good file.
    for &cut in cuts.iter().step_by(5) {
        let mut s = dom[..cut].to_owned();
        s.push_str(dom[cut..].chars().skip(1).collect::<String>().as_str());
        let _ = parse(&s);
    }
    for junk in [
        "[",
        "]",
        "interface",
        "interface A : {",
        "\u{0}",
        "const",
        "(((",
        "<<>>",
        "callback X = ;",
    ] {
        assert!(parse(junk).is_err(), "{junk:?}");
    }
}

#[test]
fn parse_order_does_not_matter() {
    let fwd = IdlSet::parse_sources(FILES.iter().copied()).unwrap();
    let rev = IdlSet::parse_sources(FILES.iter().rev().copied()).unwrap();
    for name in ["Node", "Element", "Document"] {
        assert_eq!(fwd.interface(name), rev.interface(name), "{name}");
    }
}
