//! The committed renderer bundles match `bundles/SHA256SUMS`, which
//! `js/build.sh` writes and `js/build.sh --check` reproduces from the pinned
//! npm packages. A bundle edited by hand, or rebuilt without updating the
//! sums, fails here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

fn bundles_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("bundles")
}

fn files(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
        if entry.file_type().unwrap().is_dir() {
            files(&entry.path(), &format!("{name}/"), out);
        } else if name != "SHA256SUMS" {
            out.push(name);
        }
    }
}

fn recorded() -> BTreeMap<String, String> {
    let sums = std::fs::read_to_string(bundles_dir().join("SHA256SUMS")).unwrap();
    sums.lines()
        .map(|line| {
            let (hash, file) = line.split_once("  ").expect("sha256sum format");
            (file.to_owned(), hash.to_owned())
        })
        .collect()
}

#[test]
fn bundles_match_their_recorded_sha256() {
    let recorded = recorded();
    let mut on_disk = Vec::new();
    files(&bundles_dir(), "", &mut on_disk);
    on_disk.sort();
    assert_eq!(
        on_disk,
        recorded.keys().cloned().collect::<Vec<_>>(),
        "every file under bundles/ is listed in SHA256SUMS, and nothing else"
    );
    for (file, hash) in &recorded {
        let bytes = std::fs::read(bundles_dir().join(file)).unwrap();
        let actual = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(&actual, hash, "{file} does not match SHA256SUMS");
    }
}

#[test]
fn every_bundle_ships_its_licences() {
    let recorded = recorded();
    for file in recorded.keys().filter(|f| f.ends_with(".min.js")) {
        let dir = Path::new(file).parent().unwrap().to_str().unwrap();
        assert!(
            recorded.contains_key(&format!("{dir}/LICENSE")),
            "{file} has no LICENSE next to it"
        );
    }
}
