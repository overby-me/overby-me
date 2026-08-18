//! Differential test (task #21/#22): rust `systemd-creds` vs the C `systemd-creds`.
//!
//! Focuses on the `list` verb's EXIT STATUS, which must match C's verb_list
//! contract (src/creds/creds.c): exit failure (ENXIO) when no credentials
//! resolve at all, but exit success for a directory that is set but empty. This
//! is the contract the rust cmd_list fix restored; the oracle turns any future
//! regression into a failing test without a VM. Gated on the env var
//! `SYSTEMD_CREDS` (path to the C binary); skips silently otherwise, so plain
//! `cargo test` / CI without the C tools is unaffected. Run it via
//! `just differential`, which resolves the C binary from nixpkgs.
//!
//! Only the success/failure of each invocation is compared, not stdout: the
//! human-readable table formatting is not part of the contract and differs
//! harmlessly. `--system` reads /run/credentials/@system (host state, not
//! `CREDENTIALS_DIRECTORY`), but rust and C see the same host, so they still
//! agree.

use std::path::Path;
use std::process::Command;

fn run(bin: &str, args: &[&str], creds_dir: Option<&Path>) -> bool {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    cmd.env_remove("CREDENTIALS_DIRECTORY");
    if let Some(d) = creds_dir {
        cmd.env("CREDENTIALS_DIRECTORY", d);
    }
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to run {bin} {args:?}: {e}"))
        .status
        .success()
}

#[test]
fn creds_list_exit_matches_c_systemd() {
    let Ok(c_bin) = std::env::var("SYSTEMD_CREDS") else {
        eprintln!("skip differential: SYSTEMD_CREDS unset (run `just differential`)");
        return;
    };
    // Cargo builds the crate's binary and exposes its path to integration tests.
    let rust_bin = env!("CARGO_BIN_EXE_systemd-creds");

    // A set-but-empty and a populated credentials directory under the test tmp.
    let base = std::env::temp_dir().join("rust-systemd-creds-difftest");
    let _ = std::fs::remove_dir_all(&base);
    let empty = base.join("empty");
    let populated = base.join("populated");
    std::fs::create_dir_all(&empty).expect("create empty creds dir");
    std::fs::create_dir_all(&populated).expect("create populated creds dir");
    std::fs::write(populated.join("mycred"), b"secret").expect("write cred");

    // (args, CREDENTIALS_DIRECTORY): the list-verb exit-status contract.
    let cases: &[(&[&str], Option<&Path>)] = &[
        (&["list"], None),             // no dir set -> ENXIO (fail)
        (&["--system", "list"], None), // no system creds -> ENXIO (fail)
        (&["--system"], None),         // bare --system defaults to the list verb
        (&["list"], Some(&empty)),     // set-but-empty dir -> success
        (&["list"], Some(&populated)), // populated dir -> success
    ];

    let mut divergences = Vec::new();
    for (args, dir) in cases {
        let rust_ok = run(rust_bin, args, *dir);
        let c_ok = run(&c_bin, args, *dir);
        if rust_ok != c_ok {
            divergences.push(format!(
                "args={args:?} dir={dir:?}: rust ok={rust_ok}, c ok={c_ok}"
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&base);

    assert!(
        divergences.is_empty(),
        "rust vs C systemd-creds list exit-status drift ({} case(s)):\n{}",
        divergences.len(),
        divergences.join("\n")
    );
}
