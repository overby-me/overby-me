//! Differential test (task #21): rust `systemd-escape` vs the C `systemd-escape`.
//!
//! Runs a corpus of inputs and flag combinations through BOTH binaries and
//! compares stdout and success/failure. A divergence is real upstream drift in
//! the unit-name escaping rules. Gated on the env var `SYSTEMD_ESCAPE` (path to
//! the C binary); skips silently otherwise, so plain `cargo test` / CI without
//! the C tools is unaffected. Run it via `just differential-parsers`, which
//! resolves the C binary from nixpkgs#systemd.

use std::process::Command;

fn run(bin: &str, args: &[&str]) -> (String, bool) {
    let out = Command::new(bin)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {bin} {args:?}: {e}"));
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.success(),
    )
}

#[test]
fn escape_matches_c_systemd() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ESCAPE") else {
        eprintln!("skip differential: SYSTEMD_ESCAPE unset (run `just differential-parsers`)");
        return;
    };
    // Cargo builds the crate's binary and exposes its path to integration tests.
    let rust_bin = env!("CARGO_BIN_EXE_systemd-escape");

    // Inputs exercising the escaping rules: paths, special characters that must
    // be \xNN-escaped, dots/dashes, unicode, the mode flags, and round-trips.
    let cases: &[&[&str]] = &[
        &["Hello World!"],
        &["/dev/sda"],
        &["--path", "/dev/sda1"],
        &["--path", "/"],
        &["--path", "//double//slash//"],
        &["foo/bar.baz"],
        &["a.b-c:d"],
        &["-leading-dash"],
        &["..dotdot"],
        &["with spaces & $pecial <chars>"],
        &["tab\tinside"],
        &["ünïcödé-Ω-\u{1f600}"],
        &[""],
        &["--mangle", "foo/bar baz"],
        &["--mangle", "already.service"],
        &["--template", "getty@.service", "ttyS0"],
        &["--path", "--suffix", "mount", "/var/log/journal"],
        &["--suffix", "device", "sys-devices-virtual"],
        &["--unescape", "foo\\x2fbar"],
        &["--unescape", "-dev-sda"],
        &["--unescape", "--path", "dev-sda1"],
        &["--unescape", "--template", "getty@ttyS0.service"],
        &["--unescape", "--instance", "getty@ttyS0.service"],
    ];

    let mut divergences = Vec::new();
    for args in cases {
        let (rust_out, rust_ok) = run(rust_bin, args);
        let (c_out, c_ok) = run(&c_bin, args);
        if rust_out != c_out || rust_ok != c_ok {
            divergences.push(format!(
                "args={args:?}\n     rust: ok={rust_ok} out={rust_out:?}\n     c   : ok={c_ok} out={c_out:?}"
            ));
        }
    }
    assert!(
        divergences.is_empty(),
        "rust vs C systemd-escape drift ({} case(s)):\n{}",
        divergences.len(),
        divergences.join("\n")
    );
}
