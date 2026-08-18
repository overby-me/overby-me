//! Differential test (task #21): rust `systemd-sysctl` config-syntax exit codes
//! vs the C `systemd-sysctl`.
//!
//! A line with no `=` that does not start with `-` is a hard syntax error in C
//! ("Line is not an assignment"): it accumulates into a non-zero exit while the
//! other lines still apply. A leading `-` with no `=` is a valid negative-match
//! option (not an error), and a comment is skipped. The message text differs
//! between the ports, so this compares only the EXIT CODE. Every case here is a
//! pure parse decision, so nothing is written to `/proc/sys`. Gated on the env
//! var `SYSTEMD_SYSCTL` (path to the C binary); skips otherwise. Run it via
//! `just differential`.

use std::io::Write;
use std::process::{Command, Stdio};

/// Feed `config` on stdin to `bin -` and return the exit code.
fn exit_code(bin: &str, config: &str) -> Option<i32> {
    let mut child = Command::new(bin)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {bin}: {e}"));
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(config.as_bytes())
        .unwrap_or_else(|e| panic!("write stdin to {bin}: {e}"));
    child
        .wait()
        .unwrap_or_else(|e| panic!("wait {bin}: {e}"))
        .code()
}

#[test]
fn syntax_error_exit_codes_match_c_systemd() {
    let Ok(c_bin) = std::env::var("SYSTEMD_SYSCTL") else {
        eprintln!("skip differential: SYSTEMD_SYSCTL unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-sysctl");

    // Fatal: no '=' and no leading '-' (exit non-zero). Not fatal: a leading '-'
    // negative-match option, and a comment (both exit 0).
    let cases: &[(&str, &str)] = &[
        ("noeqsign\n", "not an assignment (fatal)"),
        (
            "-kernel.rs_diff_nonexistent\n",
            "negative-match option (exit 0)",
        ),
        ("# a comment\n", "comment (exit 0)"),
    ];

    for (config, label) in cases {
        let c = exit_code(&c_bin, config);
        let r = exit_code(rust_bin, config);
        assert_eq!(
            c, r,
            "sysctl exit code drifted from C for {label}: C={c:?} rust={r:?}"
        );
    }
}
