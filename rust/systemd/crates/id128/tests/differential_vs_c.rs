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

/// Run `bin verb args...` and return (trimmed stdout, success).
fn run(bin: &str, args: &[&str]) -> (String, bool) {
    let out = Command::new(bin)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {bin} {args:?}: {e}"));
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        out.status.success(),
    )
}

/// The app-specific derivation (`-a APP`) is `sd_id128_get_app_specific`:
/// HMAC-SHA256 keyed by the base ID over the app-ID, truncated to 16 bytes with
/// the UUID version/variant nibbles forced to v4/DCE. `machine-id`/`boot-id`
/// both read host state (`/etc/machine-id`, `/proc/.../boot_id`), so rust and C
/// derive from the *same* base: any drift in the HMAC, the truncation, the
/// version stamping, or the output formatting becomes a failing test.
#[test]
fn id128_app_specific_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ID128") else {
        eprintln!("skip differential: SYSTEMD_ID128 unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-id128");

    // machine-id needs /etc/machine-id (or /run/machine-id); skip if the host
    // has neither, as both binaries would then fail on the machine-id verb.
    let have_machine_id = std::path::Path::new("/etc/machine-id").exists()
        || std::path::Path::new("/run/machine-id").exists();

    // A spread of app UUIDs (arbitrary but fixed): different version/variant
    // nibbles in the input must not change that the output is forced to v4/DCE.
    // The all-zeros ID is included on purpose: C rejects it ("Application ID
    // cannot be all zeros.") and rust must reject it identically.
    let apps = [
        "8e124d49-89d8-4d9f-a0dc-8a9b6f6f0f0f",
        "00000000-0000-0000-0000-000000000000",
        "ffffffff-ffff-ffff-ffff-ffffffffffff",
        "c2e5688f-2b6f-4f1a-9d3c-1a2b3c4d5e6f",
    ];

    // boot-id always exists on Linux; machine-id is gated above. Test both the
    // default (compact) and `-u` (RFC 4122 dashed) output formats.
    let mut verbs: Vec<&str> = vec!["boot-id"];
    if have_machine_id {
        verbs.push("machine-id");
    }

    // Count the derivations that C accepts, so a corpus that somehow rejected
    // everything (which would make the equality checks vacuous) still fails.
    let mut c_successes = 0;
    for verb in verbs {
        for app in apps {
            for fmt in [&["-a", app][..], &["-a", app, "-u"][..]] {
                let mut args = vec![verb];
                args.extend_from_slice(fmt);
                let (c_out, c_ok) = run(&c_bin, &args);
                let (r_out, r_ok) = run(rust_bin, &args);
                // The core contract: rust matches C on both the success flag and
                // the derived ID (or lack of one), covering the reject path too.
                assert_eq!(
                    (c_ok, &c_out),
                    (r_ok, &r_out),
                    "rust vs C `systemd-id128 {}` drift: C=({c_ok}, {c_out:?}) rust=({r_ok}, {r_out:?})",
                    args.join(" "),
                );
                if c_ok {
                    c_successes += 1;
                }
            }
        }
    }
    assert!(c_successes > 0, "no app-specific derivation succeeded; corpus is vacuous");
}
