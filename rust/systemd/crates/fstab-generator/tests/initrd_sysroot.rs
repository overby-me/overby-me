//! Integration test: in the initrd, the fstab-generator must turn the real
//! root's `x-initrd.mount` fstab entries into `/sysroot`-prefixed mount units,
//! with the root entry `/` becoming `sysroot.mount` wired to
//! `initrd-root-fs.target`. Without this the initrd never mounts the real root
//! and can't switch-root (TEST covers the NixOS `root=fstab` initrd layout).

use std::process::Command;

#[test]
fn initrd_sysroot_mount_generated_and_wired() {
    let dir = tempfile::tempdir().unwrap();
    let fstab = dir.path().join("fstab");
    // Mirrors NixOS's systemd-initrd /etc/fstab: real-root filesystems listed
    // with their final mountpoints and the x-initrd.mount flag.
    std::fs::write(
        &fstab,
        "/dev/disk/by-label/nixos / ext4 x-initrd.mount 0 1\n\
         nix-store /nix/.ro-store 9p x-initrd.mount,trans=virtio 0 0\n",
    )
    .unwrap();

    let normal = dir.path().join("normal");
    let early = dir.path().join("early");
    let late = dir.path().join("late");
    for d in [&normal, &early, &late] {
        std::fs::create_dir_all(d).unwrap();
    }

    let status = Command::new(env!("CARGO_BIN_EXE_systemd-fstab-generator"))
        .arg(&normal)
        .arg(&early)
        .arg(&late)
        .env("SYSTEMD_IN_INITRD", "1")
        .env("SYSTEMD_PROC_CMDLINE", "root=fstab")
        .env("SYSTEMD_FSTAB", &fstab)
        .status()
        .unwrap();
    assert!(status.success(), "generator exited with failure");

    // The root entry (mountpoint /, x-initrd.mount) becomes sysroot.mount.
    let sysroot = normal.join("sysroot.mount");
    assert!(
        sysroot.exists(),
        "sysroot.mount not generated; units: {:?}",
        std::fs::read_dir(&normal)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .collect::<Vec<_>>()
    );
    let content = std::fs::read_to_string(&sysroot).unwrap();
    assert!(
        content.contains("Where=/sysroot\n"),
        "sysroot.mount has wrong Where (expected exactly /sysroot):\n{content}"
    );
    assert!(
        content.contains("What=/dev/disk/by-label/nixos"),
        "sysroot.mount has wrong What:\n{content}"
    );

    // …and is RequiredBy initrd-root-fs.target so the initrd actually mounts it.
    assert!(
        normal
            .join("initrd-root-fs.target.requires/sysroot.mount")
            .exists(),
        "sysroot.mount not wired into initrd-root-fs.target.requires"
    );

    // A non-root x-initrd.mount entry is prefixed too but hangs off
    // initrd-fs.target, not initrd-root-fs.target.
    assert!(
        normal.join("sysroot-nix-.ro\\x2dstore.mount").exists()
            || normal.join("sysroot-nix-.ro-store.mount").exists(),
        "child /nix/.ro-store mount not prefixed to /sysroot"
    );
}
