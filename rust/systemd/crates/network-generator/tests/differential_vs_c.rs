//! Differential test (task #21): rust `systemd-network-generator` vs the C one.
//!
//! Both translate kernel-command-line `ip=`/`nameserver=`/... items into
//! `.network` files. Invoked with C's convention (`--root=DIR` + the cmdline
//! items as positional arguments), the generated file tree is a deterministic,
//! host-independent function of the arguments, so it is compared verbatim.
//! Gated on env `SYSTEMD_NETWORK_GENERATOR` (path to the C binary); skips
//! otherwise. Run via `just differential`.

use std::collections::BTreeMap;
use std::process::Command;

/// Run `bin --root=<root> <items...>` and collect the generated files as a map
/// of relative path -> contents.
fn generate(bin: &str, root: &std::path::Path, items: &[&str]) -> BTreeMap<String, String> {
    let status = Command::new(bin)
        .arg(format!("--root={}", root.display()))
        .args(items)
        .status()
        .unwrap_or_else(|e| panic!("failed to run {bin} {items:?}: {e}"));
    assert!(status.success(), "{bin} {items:?} exited with {status}");

    let mut out = BTreeMap::new();
    let net = root.join("run/systemd/network");
    if let Ok(entries) = std::fs::read_dir(&net) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let content = std::fs::read_to_string(e.path()).unwrap_or_default();
            out.insert(name, content);
        }
    }
    out
}

#[test]
fn network_generator_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_NETWORK_GENERATOR") else {
        eprintln!("skip differential: SYSTEMD_NETWORK_GENERATOR unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-network-generator");
    let tmp = std::env::temp_dir().join(format!("netgen-diff-{}", std::process::id()));

    let cases: &[&[&str]] = &[
        &["ip=dhcp"],
        &["ip=on"],
        &["ip=eth0:dhcp"],
        &["ip=eth0:dhcp6"],
        &["ip=eth0:auto6"],
        &["ip=10.0.0.5::10.0.0.1:255.255.255.0::eth0:off"],
        &["ip=192.168.1.10::192.168.1.1:255.255.255.0:myhost:eth1:none"],
        &["ip=dhcp", "nameserver=8.8.8.8", "nameserver=1.1.1.1"],
        &["ip=eth0:dhcp", "nameserver=8.8.8.8", "rd.peerdns=0"],
        &["ip=10.0.0.2::10.0.0.1:24::eth0:none:8.8.8.8:8.8.4.4"],
    ];

    let mut div = Vec::new();
    for (i, items) in cases.iter().enumerate() {
        let cr = tmp.join(format!("c{i}"));
        let rr = tmp.join(format!("r{i}"));
        let _ = std::fs::remove_dir_all(&cr);
        let _ = std::fs::remove_dir_all(&rr);
        std::fs::create_dir_all(&cr).unwrap();
        std::fs::create_dir_all(&rr).unwrap();
        let c = generate(&c_bin, &cr, items);
        let r = generate(rust_bin, &rr, items);
        if c != r {
            div.push(format!("items={items:?}\n  C={c:?}\n  R={r:?}"));
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(
        div.is_empty(),
        "rust vs C systemd-network-generator drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}
