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
        // Short form with trailing [:<mtu>[:<macaddr>]] -> [Link] MTUBytes/
        // MACAddress. rust once dropped these or misparsed the mac as full-form
        // fields (garbage Name=/DNS=).
        &["ip=eth0:dhcp:1500"],
        &["ip=eth0:auto6:9000"],
        &["ip=eth0:dhcp:1500:00:11:22:33:44:55"],
        &["ip=eth0:dhcp::00:11:22:33:44:55"],
        &["ip=10.0.0.5:dhcp:1500"],
        // Bracketed IPv6 literals in the full form. rust once shredded the
        // address on its own colons (garbage Address=[2001/5]); now they parse.
        &["ip=[2001:db8::1]::[2001:db8::ff]:64::eth0:none"],
        &["ip=[2001:db8::5]::::server:eth0:none"],
        &["ip=[fe80::1]:::::eth0:none"],
        // Full-form trailing [:<mtu>[:<mac>]] vs [:<dns>...]. A numeric field is
        // the MTU (rust once emitted DNS=1500 for it); an IP field is DNS.
        &["ip=10.0.0.2::10.0.0.1:24::eth0:none:1500"],
        &["ip=10.0.0.2::10.0.0.1:24::eth0:none:1500:00:11:22:33:44:55"],
        &["ip=10.0.0.5::10.0.0.1:255.255.255.0::eth0:off"],
        &["ip=192.168.1.10::192.168.1.1:255.255.255.0:myhost:eth1:none"],
        &["ip=dhcp", "nameserver=8.8.8.8", "nameserver=1.1.1.1"],
        &["ip=eth0:dhcp", "nameserver=8.8.8.8", "rd.peerdns=0"],
        &["ip=10.0.0.2::10.0.0.1:24::eth0:none:8.8.8.8:8.8.4.4"],
        &["ifname=eth0:00:11:22:33:44:55"], // -> 70-eth0.link
        &["ifname=lan0:aa:bb:cc:dd:ee:ff", "ip=dhcp"],
        &["net.ifname_policy=keep,kernel,path"], // -> 72-default.link
        &["net.ifname_policy=onboard,slot"],
        &["net.ifname_policy=path,mac,aa:bb:cc:dd:ee:ff"], // -> 71-<mac>.link
        &["net.ifnames=0"],                               // -> nothing (udev concern)
        &["net.ifnames=1"],
        // rd.route= merges into the interface's .network (not a separate file),
        // so a route combined with ip= for the same device is not dropped.
        &["ip=eth0:dhcp", "rd.route=10.1.0.0/16:192.168.1.1:eth0"],
        &["ip=dhcp", "rd.route=10.9.0.0/16:1.2.3.4"], // -> merged into 71-default
        &["ip=eth0:dhcp", "rd.route=10.1.0.0/16:192.168.1.1:eth0", "rd.route=10.2.0.0/16:192.168.1.1:eth0"],
        // vlan/bond/bridge: merged per-interface 70-<ifname>.netdev/.network.
        &["vlan=vlan10:eth0"],
        &["vlan=eth0.100:eth0"], // dotted device name kept in the filename
        &["bond=bond0:eth1,eth2:mode=active-backup:1500"], // options dropped like C
        &["bridge=br0:eth3,eth4"],
        &["team=team0:eth5,eth6"], // not a C option -> nothing generated
        &["bond=bond0:eth1,eth2", "ip=eth1:dhcp"], // member merges DHCP + Bond=
        &["vlan=vlan10:eth0", "ip=eth0:dhcp", "rd.route=10.0.0.0/8:1.2.3.4:eth0"], // triple merge
        // context_merge_networks: the deviceless nameserver=/rd.route=/rd.peerdns
        // bucket merges into every device network (or is emitted alone).
        &["nameserver=8.8.8.8"], // -> 71-default with DNS
        &["bond=bond0:eth1,eth2", "nameserver=8.8.8.8"], // DNS merged into members
        &["ip=eth0:dhcp", "rd.route=10.0.0.0/8:1.2.3.4"], // unbound route merges into eth0
        &["bond=bond0:eth1", "rd.peerdns=0"], // UseDNS merged into the member
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
