//! Differential test (task #21): rust `systemd-sysusers` config-syntax exit
//! codes vs the C `systemd-sysusers`.
//!
//! In C every sysusers.d parse error is fatal: the error propagates out before
//! any account is written and the tool exits 1 (unlike tmpfiles, there is no
//! soft note-and-continue mode). The message text differs between the ports, so
//! this compares only the EXIT CODE. It deliberately exercises only the
//! parse-error and empty-config paths: a *valid* entry makes rust shell out to
//! `groupadd`/`useradd`, whose chroot into `--root` needs privileges the test
//! host may lack, so that path diverges by environment, not by contract. Gated
//! on the env var `SYSTEMD_SYSUSERS` (path to the C binary); skips otherwise.
//! Run it via `just differential`.

use std::path::Path;
use std::process::{Command, Stdio};

/// Run `bin --root=<root> --inline <line>` and return the exit code.
fn exit_code(bin: &str, root: &Path, line: &str) -> Option<i32> {
    Command::new(bin)
        .arg("--root")
        .arg(root)
        .args(["--inline", line])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("failed to run {bin}: {e}"))
        .code()
}

#[test]
fn syntax_error_exit_codes_match_c_systemd() {
    let Ok(c_bin) = std::env::var("SYSTEMD_SYSUSERS") else {
        eprintln!("skip differential: SYSTEMD_SYSUSERS unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-sysusers");

    let root = std::env::temp_dir().join(format!("rs-sysd-diff-sysusers-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("etc")).expect("create temp root");

    // Every parse error is fatal (exit 1); a reserve-range directive is valid
    // and exits 0 without creating an account (so no groupadd/useradd shell-out
    // that would diverge by environment). A comment is deliberately NOT used
    // here: via --inline C hands "#" to the parser as the type ("Unknown command
    // type '#'", exit 1) since only the file reader strips comments, whereas
    // rust's shared parser skips it -- a #31-class leniency, not this contract.
    let cases: &[(&str, &str)] = &[
        ("q foo 1234", "unknown type (fatal)"),
        ("u", "too few fields (fatal)"),
        ("u foo notanuid", "invalid ID specification (fatal)"),
        ("r - 500-600", "reserve range (valid, exit 0)"),
    ];

    for (line, label) in cases {
        let c = exit_code(&c_bin, &root, line);
        let r = exit_code(rust_bin, &root, line);
        assert_eq!(
            c, r,
            "sysusers exit code drifted from C for {label}: C={c:?} rust={r:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}
