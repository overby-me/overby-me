//! Differential test (task #21): rust `systemd-id128 show` vs the C
//! `systemd-id128`.
//!
//! The well-known-ID and GPT-partition-type table is a large static map that
//! gains entries between systemd versions. This compares the full set of
//! (NAME, ID) pairs, order-independent, so an added, removed, or changed entry
//! becomes a failing test. Gated on env `SYSTEMD_ID128` (path to the C binary);
//! skips silently otherwise. Run via `just differential`.

use std::collections::BTreeSet;
use std::process::Command;

/// The `show` table as a set of (NAME, lowercased-ID) pairs.
fn show_pairs(bin: &str) -> BTreeSet<(String, String)> {
    let out = Command::new(bin)
        .arg("show")
        .output()
        .unwrap_or_else(|e| panic!("failed to run {bin} show: {e}"));
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1) // header: "NAME  ID"
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let name = it.next()?;
            let id = it.next()?;
            Some((name.to_string(), id.to_ascii_lowercase()))
        })
        .collect()
}

#[test]
fn id128_show_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ID128") else {
        eprintln!("skip differential: SYSTEMD_ID128 unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-id128");
    let c = show_pairs(&c_bin);
    let r = show_pairs(rust_bin);
    assert!(!c.is_empty(), "C systemd-id128 show produced no rows");

    let only_c: Vec<_> = c.difference(&r).collect();
    let only_r: Vec<_> = r.difference(&c).collect();
    assert!(
        only_c.is_empty() && only_r.is_empty(),
        "rust vs C systemd-id128 show drift:\n  only in C ({}): {:?}\n  only in rust ({}): {:?}",
        only_c.len(),
        only_c,
        only_r.len(),
        only_r
    );
}
