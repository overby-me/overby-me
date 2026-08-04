//! Differential test (task #21): rust `systemd-tmpfiles` config-syntax exit
//! codes vs the C `systemd-tmpfiles`.
//!
//! C distinguishes fatal config syntax errors (too few fields, no type
//! character, invalid age) which make it exit `EX_DATAERR` (65) while still
//! processing the rest, from ignorable ones (unknown type) which leave the exit
//! status at 0. The message text differs between the ports, so this compares
//! only the EXIT CODE, which is the host-independent contract. Gated on the env
//! var `SYSTEMD_TMPFILES` (path to the C binary); skips silently otherwise. Run
//! it via `just differential`.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Feed `config` on stdin to `bin --create --root=<root> -` and return the exit
/// code. The `--root` sandbox keeps the run off the real filesystem; the cases
/// below create nothing regardless, since each is a pure parse decision.
fn exit_code(bin: &str, root: &Path, config: &str) -> Option<i32> {
    let mut child = Command::new(bin)
        .args(["--create", "--root"])
        .arg(root)
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
    let Ok(c_bin) = std::env::var("SYSTEMD_TMPFILES") else {
        eprintln!("skip differential: SYSTEMD_TMPFILES unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-tmpfiles");

    let root = std::env::temp_dir().join(format!("rs-sysd-diff-tmpfiles-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create temp root");

    // (config line, human label). Fatal cases must exit 65 in both; the
    // ignorable unknown-type case must leave the exit status at 0 in both.
    let cases: &[(&str, &str)] = &[
        ("baz\n", "no type character (fatal, EX_DATAERR)"),
        ("z /run/rs-diff\n", "unknown type (ignorable, exit 0)"),
    ];

    for (config, label) in cases {
        let c = exit_code(&c_bin, &root, config);
        let r = exit_code(rust_bin, &root, config);
        assert_eq!(
            c, r,
            "tmpfiles exit code drifted from C for {label}: C={c:?} rust={r:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}
