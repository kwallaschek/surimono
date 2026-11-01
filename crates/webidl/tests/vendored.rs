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

/// SHA-256 (FIPS 180-4), so the checksum test needs no dependency.
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (chunk, word) in out.chunks_exact_mut(4).zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn sha256_matches_known_vectors() {
    // FIPS 180-4 examples, plus a two-block message.
    assert_eq!(
        hex(&sha256(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hex(&sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex(&sha256(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

/// The `sha256 (first 16)` column of the file table in `idl/SOURCES.md`.
fn recorded_checksums() -> Vec<(String, String)> {
    let sources = std::fs::read_to_string(idl_dir().join("SOURCES.md")).unwrap();
    sources
        .lines()
        .filter(|l| l.starts_with("| `") && l.contains(".idl`"))
        .map(|l| {
            let cells: Vec<&str> = l.trim_matches('|').split('|').map(str::trim).collect();
            let name = cells[0].trim_matches('`').to_owned();
            let sum = (*cells.last().unwrap()).to_owned();
            (name, sum)
        })
        .collect()
}

#[test]
fn vendored_files_match_the_checksums_in_sources_md() {
    let recorded = recorded_checksums();
    let mut names: Vec<&str> = recorded.iter().map(|(n, _)| n.as_str()).collect();
    names.sort();
    let listed: Vec<&str> = FILES.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        names, listed,
        "SOURCES.md must have exactly one checksum row per vendored file"
    );
    for (name, sum) in &recorded {
        assert!(
            sum.len() == 16 && sum.bytes().all(|b| b.is_ascii_hexdigit()),
            "{name}: `{sum}` is not 16 hex digits"
        );
        // Hash the bytes on disk, not `include_str!`, so a checkout that
        // rewrote line endings fails too.
        let bytes = std::fs::read(idl_dir().join(name)).unwrap();
        let actual = hex(&sha256(&bytes));
        assert_eq!(
            &actual[..16],
            sum.to_ascii_lowercase(),
            "idl/{name} differs from the webref copy recorded in SOURCES.md \
             (vendored IDL must stay unmodified; refresh with fetch-webref.sh)"
        );
    }
}
