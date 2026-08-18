//! Differential test (task #21): rust `systemctl` help enumerations vs the C
//! `systemctl`.
//!
//! `--type=help`, `--signal=help`, and `--state=help` print compiled-in string
//! tables and exit without contacting the manager, so they are host-independent
//! and comparable byte-for-byte. C dumps each via `DUMP_STRING_TABLE` in enum
//! order; a divergence is real upstream drift in a unit-type / signal / unit
//! state table (or in the realtime-signal rendering). Gated on the env var
//! `SYSTEMD_SYSTEMCTL` (path to the C binary); skips silently otherwise, so
//! plain `cargo test` / CI without the C tools is unaffected. Run it via `just
//! differential`, which resolves the C binary from nixpkgs#systemd.

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
fn help_enumerations_match_c_systemd() {
    let Ok(c_bin) = std::env::var("SYSTEMD_SYSTEMCTL") else {
        eprintln!("skip differential: SYSTEMD_SYSTEMCTL unset (run `just differential`)");
        return;
    };
    // Cargo builds the crate's binary and exposes its path to integration tests.
    let rust_bin = env!("CARGO_BIN_EXE_systemctl");

    // Each verb dumps a compiled-in table and exits; no manager/D-Bus needed.
    // --signal=help includes the realtime range, which both binaries render from
    // the same runtime SIGRTMIN/SIGRTMAX on this host, so it stays comparable.
    for verb in ["--type=help", "--signal=help", "--state=help"] {
        let (c_out, c_ok) = run(&c_bin, &[verb]);
        let (r_out, r_ok) = run(rust_bin, &[verb]);
        assert_eq!(
            c_ok, r_ok,
            "{verb}: exit status differs (C ok={c_ok}, rust ok={r_ok})"
        );
        assert_eq!(
            c_out, r_out,
            "{verb}: output drifted from C systemd\n--- C ---\n{c_out}--- rust ---\n{r_out}"
        );
    }
}
