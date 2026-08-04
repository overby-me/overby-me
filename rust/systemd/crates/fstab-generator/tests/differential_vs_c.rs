//! Differential test (task #21): rust `systemd-fstab-generator` vs the C one,
//! focused on device-node canonicalization.
//!
//! Both translate an `/etc/fstab` into `.mount` / `.swap` units. C's
//! `fstab_node_to_udev_node` resolves `UUID=`/`LABEL=`/`PARTUUID=`/`PARTLABEL=`
//! specs to the matching `/dev/disk/by-*/` node in the `What=` field (and, for
//! swap, in the unit name). We invoke both with the generator convention
//! (`<dest> <dest> <dest>`), point `SYSTEMD_FSTAB` at a controlled fstab and
//! blank the cmdline, then compare each generated unit's `What=` value.
//!
//! The compared fields are the environment-independent ones, as a per-unit
//! sorted set: `What=`, `After=blockdev@<node>.target` (device ordering),
//! `SourcePath=`, `Documentation=`, `Where=`, `Type=`. Header field *order* and
//! section layout still differ and are ignored by the set comparison; `Options=`
//! (diverges on the `defaults` no-op) and the fsck deps (their presence is gated
//! on C's environment-dependent sysfs_check()) are excluded. Gated on env
//! `SYSTEMD_FSTAB_GENERATOR` (path to the C binary); skips otherwise. Run via
//! `just differential`.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// Run `bin <dest> <dest> <dest>` with the fstab pinned via `SYSTEMD_FSTAB` and
/// an empty cmdline, returning (exit-success, {unit filename -> sorted device
/// fields}), where the device fields are the `What=` and `After=blockdev@` lines.
fn generate_device_fields(
    bin: &str,
    dest: &Path,
    fstab: &Path,
) -> (bool, BTreeMap<String, Vec<String>>) {
    let status = Command::new(bin)
        .arg(dest)
        .arg(dest)
        .arg(dest)
        .env("SYSTEMD_FSTAB", fstab)
        .env("SYSTEMD_PROC_CMDLINE", "")
        .env_remove("SYSTEMD_IN_INITRD")
        .status()
        .unwrap_or_else(|e| panic!("failed to run {bin}: {e}"));

    let mut map = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(dest) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !(name.ends_with(".mount") || name.ends_with(".swap")) {
                continue;
            }
            let content = std::fs::read_to_string(e.path()).unwrap_or_default();
            let mut fields: Vec<String> = content
                .lines()
                .filter(|l| {
                    l.starts_with("What=")
                        || l.starts_with("After=blockdev@")
                        || l.starts_with("SourcePath=")
                        || l.starts_with("Documentation=")
                        || l.starts_with("Where=")
                        || l.starts_with("Type=")
                })
                .map(|l| l.to_string())
                .collect();
            fields.sort();
            map.insert(name, fields);
        }
    }
    (status.success(), map)
}

#[test]
fn fstab_generator_device_nodes_match_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_FSTAB_GENERATOR") else {
        eprintln!("skip differential: SYSTEMD_FSTAB_GENERATOR unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-fstab-generator");
    let tmp = std::env::temp_dir().join(format!("fstabgen-diff-{}", std::process::id()));

    // Each case is a full fstab. Distinct mountpoints only (rust currently
    // rejects duplicate `none` swap mountpoints, tracked as a follow-up).
    let cases: &[&str] = &[
        // Every device-spec form in one fstab, plus a tmpfs (no blockdev@).
        "UUID=abc-123    /a  ext4  defaults          0 2\n\
         LABEL=my-root   /b  ext4  defaults          0 2\n\
         PARTUUID=DEAD99 /c  ext4  defaults          0 2\n\
         PARTLABEL=ESP   /d  vfat  defaults          0 2\n\
         /dev/sda5       /e  ext4  defaults          0 2\n\
         tmpfs           /t  tmpfs defaults          0 0\n",
        // Root + a plain device, mixed with a UUID mount.
        "/dev/sda1       /     ext4  defaults        0 1\n\
         UUID=1234-5678  /boot vfat  umask=0077      0 2\n",
        // A single UUID swap (name + What= both canonicalized).
        "UUID=swap-uuid-1 none swap sw 0 0\n",
        // A single LABEL swap.
        "LABEL=myswap none swap defaults 0 0\n",
        // A PARTUUID swap.
        "PARTUUID=aa11 none swap sw 0 0\n",
        // Several swaps sharing the pseudo `none`/`swap` target: C keys swaps
        // by device, never by mountpoint, so all are emitted (rust previously
        // rejected these as a duplicate `none` mountpoint).
        "UUID=swap-a  none swap sw       0 0\n\
         /dev/sdc2    none swap defaults 0 0\n\
         LABEL=swp3   swap swap pri=10   0 0\n",
    ];

    let mut div = Vec::new();
    for (i, fstab) in cases.iter().enumerate() {
        let fstab_path = tmp.join(format!("fstab{i}"));
        let cr = tmp.join(format!("c{i}"));
        let rr = tmp.join(format!("r{i}"));
        let _ = std::fs::remove_dir_all(&cr);
        let _ = std::fs::remove_dir_all(&rr);
        std::fs::create_dir_all(&cr).unwrap();
        std::fs::create_dir_all(&rr).unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(&fstab_path, fstab).unwrap();

        let (c_ok, c) = generate_device_fields(&c_bin, &cr, &fstab_path);
        let (r_ok, r) = generate_device_fields(rust_bin, &rr, &fstab_path);
        if c_ok != r_ok || c != r {
            div.push(format!(
                "fstab={fstab:?}\n  C(ok={c_ok})={c:?}\n  R(ok={r_ok})={r:?}"
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(
        div.is_empty(),
        "rust vs C systemd-fstab-generator device-node drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}
