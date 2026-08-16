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

/// Apply `config` via `bin --create --root=<root> -`, then return the sorted
/// `getfacl -n` entry lines (tag:qualifier:perms, "#effective" suffix stripped)
/// for `<root>/<rel>`.
fn apply_and_getfacl(bin: &str, root: &Path, config: &str, rel: &str) -> Vec<String> {
    std::fs::create_dir_all(root).expect("create root");
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
    child.wait().unwrap_or_else(|e| panic!("wait {bin}: {e}"));

    let out = Command::new("getfacl")
        .arg("-n")
        .arg(root.join(rel))
        .output()
        .expect("run getfacl");
    let text = String::from_utf8_lossy(&out.stdout);
    let mut v: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split('\t').next().unwrap_or(l).trim().to_string())
        .collect();
    v.sort();
    v
}

/// C applies every ACL (`a`/`A`) line for a path in order rather than
/// deduplicating them (each `a` replaces the ACL, each `a+` merges onto it), so
/// two ACL lines for one path must both take effect. Guards the item-dedup
/// exemption. Gated on `SYSTEMD_TMPFILES` and `getfacl`.
#[test]
fn duplicate_acl_lines_match_c_systemd() {
    let Ok(c_bin) = std::env::var("SYSTEMD_TMPFILES") else {
        eprintln!("skip differential: SYSTEMD_TMPFILES unset (run `just differential`)");
        return;
    };
    if Command::new("getfacl")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("skip differential: getfacl unavailable");
        return;
    }
    let rust_bin = env!("CARGO_BIN_EXE_systemd-tmpfiles");

    let base = std::env::temp_dir().join(format!("rs-sysd-diff-acl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);

    // Each config creates /t (0644) then applies two ACL lines to it.
    let cases: &[(&str, &str)] = &[
        (
            "f /t 0644 - - -\na /t - - - - u:0:rwx\na+ /t - - - - g:0:r-x\n",
            "a then a+ accumulate",
        ),
        (
            "f /t 0644 - - -\na+ /t - - - - u:0:rwx\na+ /t - - - - g:0:r-x\n",
            "a+ then a+ accumulate",
        ),
        (
            "f /t 0644 - - -\na /t - - - - u:0:rwx\na /t - - - - g:0:rwx\n",
            "a then a (second replaces)",
        ),
    ];

    for (i, (config, label)) in cases.iter().enumerate() {
        let c = apply_and_getfacl(&c_bin, &base.join(format!("c{i}")), config, "t");
        let r = apply_and_getfacl(rust_bin, &base.join(format!("r{i}")), config, "t");
        assert_eq!(
            c, r,
            "tmpfiles ACL result drifted from C for {label}:\nC={c:?}\nrust={r:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&base);
}
