//! `release_free_memory` really gives pages back. A file of its own: a test
//! binary with one test has no other thread allocating while it reads RSS.

#![cfg(all(target_os = "linux", target_env = "gnu"))]

mod common;

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap()
}

#[test]
fn freed_pages_go_back_to_the_os() {
    if common::time_scale() > 1 {
        // AddressSanitizer's allocator keeps freed blocks in quarantine.
        return;
    }
    const BLOCK: usize = 4096;
    const BLOCKS: usize = 16 * 1024; // 64 MiB
    let mut blocks: Vec<Vec<u8>> = (0..BLOCKS).map(|i| vec![i as u8 | 1; BLOCK]).collect();
    // The last block stays, so the allocator can't shrink the heap by itself.
    let last = blocks.pop().unwrap();
    drop(blocks);
    let before = rss_kib();
    surimono_core::release_free_memory();
    let after = rss_kib();
    assert!(
        after + 32 * 1024 < before,
        "resident memory went from {before} to {after} KiB"
    );
    assert_eq!(last[0], (BLOCKS - 1) as u8 | 1);
}
