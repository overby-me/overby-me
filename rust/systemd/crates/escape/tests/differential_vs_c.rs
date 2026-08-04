//! Differential test (task #21): rust `systemd-escape` vs the C `systemd-escape`.
//!
//! Runs a corpus of inputs and flag combinations through BOTH binaries and
//! compares stdout and success/failure. A divergence is real upstream drift in
//! the unit-name escaping rules. Gated on the env var `SYSTEMD_ESCAPE` (path to
//! the C binary); skips silently otherwise, so plain `cargo test` / CI without
//! the C tools is unaffected. Run it via `just differential`, which resolves
//! the C binary from nixpkgs#systemd.

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
        eprintln!("skip differential: SYSTEMD_ESCAPE unset (run `just differential`)");
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
        &["--path", ""],
        &["--path", "//double//slash//"],
        &["--path", "."],
        &["--path", ".."],
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
        // The suffix must be an exact unit type name; C rejects a dotted or
        // empty suffix (rust used to strip the dot and accept ".service").
        &["--suffix", ".service", "foo"],
        &["--suffix", "", "foo"],
        &["--suffix", "bogus", "foo"],
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

/// Run `bin args...` and capture (stdout, stderr, success) verbatim.
fn run_full(bin: &str, args: &[&str]) -> (String, String, bool) {
    let out = Command::new(bin)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {bin} {args:?}: {e}"));
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

/// The option-validation errors are rust's own messages (not clap's), so they
/// must match C's escape-tool byte for byte: no "Error: " prefix, C's exact
/// text, and C's check order for multi-conflict inputs. Excludes the structural
/// clap-vs-getopt cases (a missing --template value, and `-u --mangle` which C
/// resolves by last-wins rather than erroring).
#[test]
fn escape_option_errors_match_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ESCAPE") else {
        eprintln!("skip differential: SYSTEMD_ESCAPE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-escape");

    let cases: &[&[&str]] = &[
        &[],                                                     // Not enough arguments.
        &["--template", "foo.service", "a"],                    // Template name ... is not valid.
        &["--suffix", ".service", "foo"],                       // Invalid unit suffix type ".service".
        &["--suffix", "", "foo"],                               // Invalid unit suffix type "".
        &["--template", "getty@.service", "--suffix", "service", "x"], // may not be combined
        &["--mangle", "--template", "getty@.service", "x"],     // not compatible with --mangle
        &["--mangle", "--suffix", "service", "x"],              // not compatible with --mangle
        &["--unescape", "--suffix", "service", "x"],            // --suffix is not compatible with --unescape
        &["--path", "--mangle", "x"],                           // --path may not be combined with --mangle
        &["--instance", "x"],                                   // must be used in conjunction with --unescape
        &["--instance", "--template", "getty@.service", "x"],   // may not be combined with --template
    ];

    let mut div = Vec::new();
    for args in cases {
        let (ro, re, rok) = run_full(rust_bin, args);
        let (co, ce, cok) = run_full(&c_bin, args);
        if ro != co || re != ce || rok != cok {
            div.push(format!(
                "args={args:?}\n     rust: ok={rok} out={ro:?} err={re:?}\n     c   : ok={cok} out={co:?} err={ce:?}"
            ));
        }
    }
    assert!(
        div.is_empty(),
        "rust vs C systemd-escape option-error drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}
