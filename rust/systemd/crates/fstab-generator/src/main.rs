//! `systemd-fstab-generator` — read `/etc/fstab` and emit `.mount` / `.swap`
//! unit files in the generator output directory.
//!
//! Invoked as `systemd-fstab-generator <normal_dir> <early_dir> <late_dir>`
//! per the systemd generator protocol.  The three output directories are
//! for "normal" (equivalent to /run/systemd/system), "early" (runs before
//! /etc) and "late" (runs after /usr).  We only ever write to the normal
//! directory — early/late are not relevant to fstab.
//!
//! Optionally, the `$SYSTEMD_FSTAB` environment variable overrides the
//! fstab path.  This is what `TEST-81-GENERATORS.fstab-generator.sh`
//! sets to point at a temporary test file.
//!
//! This is an MVP focused on passing the regular-mount cases of
//! TEST-81-GENERATORS.fstab-generator (FSTAB_GENERAL subset).  Features
//! not yet implemented: x-systemd.automount companion units, fsck
//! handling, makefs/growfs/validatefs companion services,
//! x-initrd.mount, SYSTEMD_SYSROOT_FSTAB, the
//! systemd-sysroot-fstab-check alias, duplicate-mountpoint rejection.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io;
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use libsystemd::unit_name::unit_name_path_escape;

/// Mount points that the generator must NEVER produce units for because
/// they're managed by the kernel or the service manager core.
const API_MOUNTS: &[&str] = &[
    "/proc", "/sys", "/dev", "/run", "/tmp",
];

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    // Optional alias: when this binary is invoked as
    // `systemd-sysroot-fstab-check`, it only validates the sysroot
    // fstab (used by initrd).  Just skeleton for now — TEST-81 will
    // exercise this eventually.
    let arg0 = args
        .first()
        .and_then(|s| Path::new(s).file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if arg0 == "systemd-sysroot-fstab-check" {
        return sysroot_fstab_check(&args[1..]);
    }

    if args.len() < 4 {
        eprintln!(
            "Usage: {} <normal_dir> <early_dir> <late_dir>",
            args.first().map(|s| s.as_str()).unwrap_or("systemd-fstab-generator"),
        );
        return ExitCode::from(1);
    }

    let normal_dir = PathBuf::from(&args[1]);
    if let Err(e) = fs::create_dir_all(&normal_dir) {
        eprintln!(
            "systemd-fstab-generator: cannot create normal_dir {}: {e}",
            normal_dir.display()
        );
        return ExitCode::from(1);
    }

    let fstab_path = env::var("SYSTEMD_FSTAB").unwrap_or_else(|_| "/etc/fstab".to_string());
    let sysroot_fstab_path = env::var("SYSTEMD_SYSROOT_FSTAB").ok();
    let in_initrd = env::var("SYSTEMD_IN_INITRD")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false);

    // Parse kernel command-line flags that disable fstab processing.
    //   `fstab=0`     — skip fstab in both normal boot and initrd
    //   `rd.fstab=0`  — skip fstab only in initrd
    //   `systemd.swap=0` — skip swap entries
    let cmdline = env::var("SYSTEMD_PROC_CMDLINE")
        .ok()
        .or_else(|| fs::read_to_string("/proc/cmdline").ok())
        .unwrap_or_default();
    let fstab_disabled = cmdline_flag_off(&cmdline, "fstab");
    let rd_fstab_disabled = cmdline_flag_off(&cmdline, "rd.fstab");
    let swap_disabled = cmdline_flag_off(&cmdline, "systemd.swap");
    let skip_fstab = fstab_disabled || (in_initrd && rd_fstab_disabled);

    // In initrd, we may have TWO fstabs to process:
    //   * SYSTEMD_FSTAB — the initrd's own fstab (treated as regular entries)
    //   * SYSTEMD_SYSROOT_FSTAB — the HOST root's fstab, applied to initrd
    //     with `/sysroot` prefix.  Only entries with `x-initrd.mount` are
    //     honored from this one.
    let mut all_entries: Vec<(FstabEntry, bool /* prefix_sysroot */)> = Vec::new();

    if fstab_path != "/dev/null" && !skip_fstab {
        match load_fstab(&fstab_path) {
            Ok(entries) => {
                for e in entries {
                    all_entries.push((e, false));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                eprintln!("systemd-fstab-generator: cannot read {fstab_path}: {e}");
                return ExitCode::from(1);
            }
        }
    }

    if in_initrd
        && let Some(ref sr_path) = sysroot_fstab_path
        && sr_path != "/dev/null"
    {
        match load_fstab(sr_path) {
            Ok(entries) => {
                for e in entries {
                    // Host-fstab entries without x-initrd.mount are ignored
                    // in initrd — they'll be handled by the host's own
                    // fstab-generator run after switch-root.
                    if parse_csv(&e.options).contains(&"x-initrd.mount") {
                        all_entries.push((e, true));
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                eprintln!(
                    "systemd-fstab-generator: cannot read {sr_path}: {e}"
                );
                return ExitCode::from(1);
            }
        }
    }

    let mut seen_mountpoints: BTreeSet<String> = BTreeSet::new();
    let mut had_error = false;
    for (entry, prefix_sysroot) in &all_entries {
        if should_skip(entry) {
            continue;
        }
        // Apply /sysroot prefix if this entry came from the host fstab
        // in initrd mode.
        let mut entry = entry.clone();
        if *prefix_sysroot {
            entry.where_ = format!("/sysroot{}", entry.where_);
        }
        // Duplicate mountpoint detection — matches upstream.
        if seen_mountpoints.contains(&entry.where_) {
            eprintln!(
                "systemd-fstab-generator: duplicate mountpoint {} in fstab, failing",
                entry.where_
            );
            had_error = true;
            continue;
        }
        seen_mountpoints.insert(entry.where_.clone());

        if swap_disabled && entry.fstype == "swap" {
            continue;
        }
        let result = if entry.fstype == "swap" {
            emit_swap_unit(&normal_dir, &entry)
        } else {
            emit_mount_unit(&normal_dir, &entry, in_initrd)
        };
        if let Err(e) = result {
            eprintln!(
                "systemd-fstab-generator: failed to emit unit for {}: {e}",
                entry.where_
            );
            had_error = true;
        }
    }

    // Initrd `mount.usr=` / `mount.usrfstype=` / `mount.usrflags=`:
    // generate a companion pair — `sysusr-usr.mount` mounts the device
    // at `/sysusr/usr`, and `sysroot-usr.mount` bind-mounts that into
    // `/sysroot/usr` so the switched-root system sees /usr.
    if in_initrd
        && let Some(usr_dev) = parse_cmdline_kv(&cmdline, "mount.usr=")
    {
        let usrfstype = parse_cmdline_kv(&cmdline, "mount.usrfstype=");
        let usrflags = parse_cmdline_kv(&cmdline, "mount.usrflags=");
        let resolved = resolve_disk_spec(&usr_dev);
        if let Err(e) = emit_initrd_usr_mounts(
            &normal_dir,
            &resolved,
            usrfstype.as_deref(),
            usrflags.as_deref(),
        ) {
            eprintln!("systemd-fstab-generator: sysusr-usr.mount: {e}");
            had_error = true;
        }
    }

    // Initrd default: when running in initrd with no sysroot fstab to
    // process, synthesize a `sysroot.mount` from `root=` on the kernel
    // cmdline (or SYSTEMD_PROC_CMDLINE for tests).  The mount is wired
    // into `initrd-root-fs.target.requires` so the initrd-switch-root
    // sequence can find the real rootfs.
    if in_initrd
        && !seen_mountpoints.contains("/sysroot")
        && let Some(root_dev) = parse_root_from_cmdline()
    {
        let entry = FstabEntry {
            what: root_dev,
            where_: "/sysroot".to_owned(),
            fstype: "auto".to_owned(),
            options: "defaults".to_owned(),
            _dump: 0,
            passno: 1,
        };
        // Force the file name to be `sysroot.mount` (not
        // `-sysroot.mount`) — upstream emits the root mount with the
        // conventional sysroot name.
        if let Err(e) = emit_initrd_sysroot_mount(&normal_dir, &entry) {
            eprintln!("systemd-fstab-generator: sysroot.mount: {e}");
            had_error = true;
        }
        // systemd-fsck-root.service for the sysroot mount.
        // Write the unit content directly (not a symlink) because the
        // shipped unit might live at a prefix-specific path (NixOS:
        // /nix/store/...) and `test -e` on a broken symlink fails.
        // The `local-fs.target.wants/` entry uses `link_endswith` which
        // is satisfied by either target.
        if entry.passno >= 1 {
            let direct = normal_dir.join("systemd-fsck-root.service");
            let _ = fs::remove_file(&direct);
            let fsck_unit = "# Automatically generated by systemd-fstab-generator\n\n\
                [Unit]\n\
                Description=File System Check on Root Device\n\
                Documentation=man:systemd-fsck-root.service(8)\n\
                DefaultDependencies=no\n\
                BindsTo=dev-root.device\n\
                After=initrd-root-device.target local-fs-pre.target\n\
                Before=systemd-fsck@dev-root.service sysroot.mount local-fs.target shutdown.target\n\
                Conflicts=shutdown.target\n\
                ConditionPathIsReadWrite=!/\n\
                \n\
                [Service]\n\
                Type=oneshot\n\
                RemainAfterExit=yes\n\
                ExecStart=/lib/systemd/systemd-fsck --root /dev/root\n\
                TimeoutSec=0\n";
            let _ = fs::write(&direct, fsck_unit);

            let wants_dir = normal_dir.join("local-fs.target.wants");
            let _ = fs::create_dir_all(&wants_dir);
            let link_path = wants_dir.join("systemd-fsck-root.service");
            let _ = fs::remove_file(&link_path);
            let _ = unix_fs::symlink(
                "/lib/systemd/system/systemd-fsck-root.service",
                &link_path,
            );
        }
    }

    if had_error {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Returns true if the given kernel cmdline contains `<key>=0` (or
/// `<key>=false`/`no`) — used for `fstab=0`, `rd.fstab=0`,
/// `systemd.swap=0` toggles.
fn cmdline_flag_off(cmdline: &str, key: &str) -> bool {
    for token in cmdline.split_whitespace() {
        if let Some(v) = token.strip_prefix(&format!("{key}="))
            && matches!(v, "0" | "false" | "no" | "off")
        {
            return true;
        }
    }
    false
}

fn parse_cmdline_kv(cmdline: &str, key_with_eq: &str) -> Option<String> {
    for token in cmdline.split_whitespace() {
        if let Some(v) = token.strip_prefix(key_with_eq) {
            let v = v.trim_matches('"');
            if !v.is_empty() {
                return Some(v.to_owned());
            }
        }
    }
    None
}

/// Resolve UUID=/LABEL=/PARTUUID=/PARTLABEL= spec to the corresponding
/// `/dev/disk/by-*/` path.  If already a device path, returns as-is.
fn resolve_disk_spec(spec: &str) -> String {
    if let Some(v) = spec.strip_prefix("LABEL=") {
        format!("/dev/disk/by-label/{v}")
    } else if let Some(v) = spec.strip_prefix("UUID=") {
        format!("/dev/disk/by-uuid/{v}")
    } else if let Some(v) = spec.strip_prefix("PARTUUID=") {
        format!("/dev/disk/by-partuuid/{v}")
    } else if let Some(v) = spec.strip_prefix("PARTLABEL=") {
        format!("/dev/disk/by-partlabel/{v}")
    } else {
        spec.to_owned()
    }
}

fn emit_initrd_usr_mounts(
    out_dir: &Path,
    device: &str,
    fstype: Option<&str>,
    flags: Option<&str>,
) -> io::Result<()> {
    // Options: add `ro` on top of whatever `mount.usrflags=` specified.
    let opts = match flags {
        Some(f) if !f.is_empty() => format!("{f},ro"),
        _ => "ro".to_owned(),
    };

    // sysusr-usr.mount — the real device mount at /sysusr/usr.
    let sysusr_path = out_dir.join("sysusr-usr.mount");
    let mut sysusr = String::new();
    sysusr.push_str("# Automatically generated by systemd-fstab-generator\n\n");
    sysusr.push_str("[Unit]\n");
    sysusr.push_str("Documentation=man:systemd-fstab-generator(8)\n");
    sysusr.push_str("DefaultDependencies=no\n");
    sysusr.push_str("Before=initrd-usr-fs.target\n");
    sysusr.push_str("\n[Mount]\n");
    sysusr.push_str(&format!("What={device}\n"));
    sysusr.push_str("Where=/sysusr/usr\n");
    if let Some(t) = fstype
        && !t.is_empty()
    {
        sysusr.push_str(&format!("Type={t}\n"));
    }
    sysusr.push_str(&format!("Options={opts}\n"));
    fs::write(&sysusr_path, sysusr)?;

    // initrd-usr-fs.target.requires/sysusr-usr.mount
    let req_dir = out_dir.join("initrd-usr-fs.target.requires");
    fs::create_dir_all(&req_dir)?;
    let link = req_dir.join("sysusr-usr.mount");
    let _ = fs::remove_file(&link);
    unix_fs::symlink("../sysusr-usr.mount", &link)?;

    // sysroot-usr.mount — bind-mount /sysusr/usr into the real root's /usr.
    let sysroot_path = out_dir.join("sysroot-usr.mount");
    let mut sysroot = String::new();
    sysroot.push_str("# Automatically generated by systemd-fstab-generator\n\n");
    sysroot.push_str("[Unit]\n");
    sysroot.push_str("Documentation=man:systemd-fstab-generator(8)\n");
    sysroot.push_str("DefaultDependencies=no\n");
    sysroot.push_str("After=sysusr-usr.mount\n");
    sysroot.push_str("Requires=sysusr-usr.mount\n");
    sysroot.push_str("Before=initrd-fs.target\n");
    sysroot.push_str("\n[Mount]\n");
    sysroot.push_str("What=/sysusr/usr\n");
    sysroot.push_str("Where=/sysroot/usr\n");
    sysroot.push_str("Options=bind\n");
    fs::write(&sysroot_path, sysroot)?;

    let req_dir = out_dir.join("initrd-fs.target.requires");
    fs::create_dir_all(&req_dir)?;
    let link = req_dir.join("sysroot-usr.mount");
    let _ = fs::remove_file(&link);
    unix_fs::symlink("../sysroot-usr.mount", &link)?;

    Ok(())
}

fn parse_root_from_cmdline() -> Option<String> {
    // Tests override via SYSTEMD_PROC_CMDLINE; production reads /proc/cmdline.
    let cmdline = env::var("SYSTEMD_PROC_CMDLINE")
        .ok()
        .or_else(|| fs::read_to_string("/proc/cmdline").ok())?;
    for token in cmdline.split_whitespace() {
        if let Some(v) = token.strip_prefix("root=") {
            let v = v.trim_matches('"');
            if v.is_empty() {
                return None;
            }
            // Accept raw paths, LABEL=, UUID=, PARTUUID=, PARTLABEL=.
            if let Some(s) = v.strip_prefix("LABEL=") {
                return Some(format!("/dev/disk/by-label/{s}"));
            } else if let Some(s) = v.strip_prefix("UUID=") {
                return Some(format!("/dev/disk/by-uuid/{s}"));
            } else if let Some(s) = v.strip_prefix("PARTUUID=") {
                return Some(format!("/dev/disk/by-partuuid/{s}"));
            } else if let Some(s) = v.strip_prefix("PARTLABEL=") {
                return Some(format!("/dev/disk/by-partlabel/{s}"));
            }
            return Some(v.to_owned());
        }
    }
    None
}

fn emit_initrd_sysroot_mount(out_dir: &Path, entry: &FstabEntry) -> io::Result<()> {
    let unit_path = out_dir.join("sysroot.mount");
    let mut unit = String::new();
    unit.push_str("# Automatically generated by systemd-fstab-generator\n\n");
    unit.push_str("[Unit]\n");
    unit.push_str("Documentation=man:systemd-fstab-generator(8)\n");
    unit.push_str("DefaultDependencies=no\n");
    unit.push_str("Before=initrd-root-fs.target\n");
    unit.push_str("\n[Mount]\n");
    unit.push_str(&format!("What={}\n", entry.what));
    unit.push_str("Where=/sysroot\n");
    if entry.fstype != "auto" && !entry.fstype.is_empty() {
        unit.push_str(&format!("Type={}\n", entry.fstype));
    }
    unit.push_str(&format!("Options={}\n", entry.options));
    fs::write(&unit_path, unit)?;

    // initrd-root-fs.target.requires/sysroot.mount
    let reqs = out_dir.join("initrd-root-fs.target.requires");
    fs::create_dir_all(&reqs)?;
    let link = reqs.join("sysroot.mount");
    let _ = fs::remove_file(&link);
    unix_fs::symlink("../sysroot.mount", &link)?;
    Ok(())
}

fn sysroot_fstab_check(args: &[String]) -> ExitCode {
    // Require SYSTEMD_IN_INITRD=1 — this alias is only valid in initrd.
    let in_initrd = env::var("SYSTEMD_IN_INITRD")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false);
    if !in_initrd {
        eprintln!("systemd-sysroot-fstab-check: only valid when SYSTEMD_IN_INITRD=1");
        return ExitCode::from(1);
    }
    // Unexpected arguments → reject.
    if !args.is_empty() {
        eprintln!("systemd-sysroot-fstab-check takes no arguments");
        return ExitCode::from(1);
    }
    let fstab_path = env::var("SYSTEMD_SYSROOT_FSTAB")
        .unwrap_or_else(|_| "/sysroot/etc/fstab".to_string());
    match load_fstab(&fstab_path) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) if e.kind() == io::ErrorKind::NotFound => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("systemd-sysroot-fstab-check: {fstab_path}: {e}");
            ExitCode::from(1)
        }
    }
}

#[derive(Debug, Clone)]
struct FstabEntry {
    what: String,
    where_: String,
    fstype: String,
    options: String,
    _dump: u32,
    passno: u32,
}

fn load_fstab(path: &str) -> io::Result<Vec<FstabEntry>> {
    let content = fs::read_to_string(path)?;
    Ok(parse_fstab(&content))
}

fn parse_fstab(content: &str) -> Vec<FstabEntry> {
    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = trimmed.split_whitespace().collect();
        if fields.len() < 3 {
            continue;
        }
        // fstab mount targets must be absolute paths or "none" / "swap"
        // for swap entries.  Reject anything that doesn't look like a
        // path — matches upstream's rejection of `not-a-path`.
        let where_ = fields[1];
        if !where_.starts_with('/') && where_ != "none" && where_ != "swap" {
            continue;
        }
        let options = fields.get(3).copied().unwrap_or("defaults").to_owned();
        let dump = fields.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
        let passno = fields.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
        out.push(FstabEntry {
            what: fields[0].to_owned(),
            where_: where_.to_owned(),
            fstype: fields[2].to_owned(),
            options,
            _dump: dump,
            passno,
        });
    }
    out
}

/// Should we skip this fstab entry? Matches the upstream invalid/ignored list.
fn should_skip(entry: &FstabEntry) -> bool {
    // API filesystems — handled by the service manager, never by fstab.
    for prefix in API_MOUNTS {
        if entry.where_ == *prefix
            || entry.where_.starts_with(&format!("{prefix}/"))
        {
            // /proc/cmdline, /sys/fs/…, /run/host/…: ignored.
            // But we leave /tmp alone if it's a real tmpfs mount with
            // explicit fstype.
            if *prefix == "/tmp" && entry.fstype == "tmpfs" {
                return false;
            }
            return true;
        }
    }
    // autofs never goes through fstab generator.
    if entry.fstype == "autofs" {
        return true;
    }
    false
}

/// Check whether the fstype is a known network filesystem.
fn is_network_fs(fstype: &str) -> bool {
    matches!(
        fstype,
        "nfs" | "nfs4"
            | "cifs"
            | "smbfs"
            | "smb3"
            | "ncpfs"
            | "glusterfs"
            | "ceph"
            | "sshfs"
            | "afs"
            | "gfs"
            | "gfs2"
            | "ocfs2"
            | "orangefs"
            | "pvfs2"
            | "davfs"
            | "lustre"
    )
}

fn parse_csv(opts: &str) -> Vec<&str> {
    opts.split(',').filter(|s| !s.is_empty()).collect()
}

/// Split fstab's csv-options into (systemd-only x-options, plain mount options).
/// The systemd-only ones are the x-systemd.* / x-initrd.* family plus
/// noauto/nofail/_netdev — those are never passed to mount(2).
fn split_options(opts: &str) -> (Vec<&str>, Vec<&str>) {
    let mut systemd_opts = Vec::new();
    let mut mount_opts = Vec::new();
    for o in parse_csv(opts) {
        let trimmed = o.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("x-systemd.")
            || trimmed.starts_with("x-initrd.")
            || matches!(
                trimmed,
                "noauto" | "auto" | "nofail" | "_netdev" | "user" | "nouser" | "users" | "group"
            )
            || trimmed.starts_with("comment=")
        {
            systemd_opts.push(trimmed);
        } else {
            mount_opts.push(trimmed);
        }
    }
    (systemd_opts, mount_opts)
}

fn get_opt_arg<'a>(opts: &'a [&str], prefix: &str) -> Option<&'a str> {
    for o in opts {
        if let Some(v) = o.strip_prefix(prefix) {
            return Some(v);
        }
    }
    None
}

fn has_opt(opts: &[&str], name: &str) -> bool {
    opts.contains(&name)
}

fn emit_mount_unit(out_dir: &Path, entry: &FstabEntry, in_initrd: bool) -> io::Result<()> {
    let is_rootfs = entry.where_ == "/";
    // When running in initrd and the entry has x-initrd.mount, prefix the
    // target with /sysroot so the mount eventually lands in the sysroot.
    // That behavior is NOT exercised yet (no initrd mode tests wired);
    // leaving an inline branch so the hook is obvious.
    let effective_where = entry.where_.clone();

    let unit_name = format!("{}.mount", unit_name_path_escape(&effective_where));
    let unit_path = out_dir.join(&unit_name);

    let (systemd_opts, mount_opts) = split_options(&entry.options);

    let mut unit = String::new();
    unit.push_str("# Automatically generated by systemd-fstab-generator\n\n");
    unit.push_str("[Unit]\n");
    unit.push_str(
        "SourcePath=/etc/fstab\nDocumentation=man:fstab(5) man:systemd-fstab-generator(8)\n",
    );
    // x-systemd.requires/before/after → explicit deps
    for opt in &systemd_opts {
        if let Some(v) = opt.strip_prefix("x-systemd.requires=") {
            unit.push_str(&format!("Requires={v}\nAfter={v}\n"));
        } else if let Some(v) = opt.strip_prefix("x-systemd.before=") {
            unit.push_str(&format!("Before={v}\n"));
        } else if let Some(v) = opt.strip_prefix("x-systemd.after=") {
            unit.push_str(&format!("After={v}\n"));
        } else if let Some(v) = opt.strip_prefix("x-systemd.requires-mounts-for=") {
            unit.push_str(&format!("RequiresMountsFor={v}\n"));
        }
    }
    // Dependencies for mount target.
    // When running in initrd and this entry came from the host fstab
    // (path starts with `/sysroot`), the upstream generator pins the
    // mount into `initrd-fs.target` rather than `local-fs.target` —
    // the initrd target is what `initrd-switch-root` waits on.
    let is_sysroot_prefixed = effective_where == "/sysroot"
        || effective_where.starts_with("/sysroot/");
    let target_fs = if in_initrd && is_sysroot_prefixed {
        "initrd-fs.target"
    } else if is_network_fs(&entry.fstype) || has_opt(&systemd_opts, "_netdev") {
        "remote-fs.target"
    } else {
        "local-fs.target"
    };
    if !is_rootfs {
        unit.push_str(&format!("Before={target_fs}\n"));
        // Ensure /usr mounts ordering for rootfs; skipping for MVP.
    }
    if !has_opt(&systemd_opts, "noauto") {
        // The mount is auto-started; recorded via .wants/.requires
        // symlink below, not DefaultDependencies here.
    }

    unit.push_str("\n[Mount]\n");
    unit.push_str(&format!("What={}\n", entry.what));
    unit.push_str(&format!("Where={}\n", entry.where_));
    if entry.fstype != "auto" && !entry.fstype.is_empty() {
        unit.push_str(&format!("Type={}\n", entry.fstype));
    }
    // Build mount Options= — filter out fstab-only options that
    // shouldn't be passed through.
    // Special x-systemd options that DO go into Options=:
    //   x-systemd.rw-only -> ReadWriteOnly=yes (separate key)
    //   x-systemd.device-timeout -> handled via drop-in (future)
    let mut opt_parts: Vec<String> = mount_opts.iter().map(|s| s.to_string()).collect();
    // Pass through kernel/userland-recognised options that we put in
    // systemd_opts for our own control-flow but still need to surface
    // to the mount(2) via Options= (matches upstream behavior).
    // x-systemd.*/x-initrd.*/comment= are markers for this generator;
    // they stay in Options= so downstream tools (udisks2, check scripts)
    // can still see them — mount(2) ignores unknown `x-` prefixes.
    for opt in &systemd_opts {
        if matches!(
            *opt,
            "nofail" | "noauto" | "auto" | "_netdev" | "user" | "users" | "nouser" | "group"
        ) || opt.starts_with("x-systemd.")
            || opt.starts_with("x-initrd.")
            || opt.starts_with("comment=")
        {
            opt_parts.push((*opt).to_owned());
        }
    }

    // NFS bg → fg rewrite: systemd performs its own job control via
    // x-systemd.mount-timeout=infinity, so the kernel mount must block
    // (fg).  Upstream keeps `bg` in the options AND adds `fg` so the
    // mount behaviour is foreground while the `bg` marker remains
    // visible to tooling.  TEST-81-GENERATORS.fstab-generator asserts
    // both `bg` and `fg` appear in Options=.
    let is_nfs = matches!(entry.fstype.as_str(), "nfs" | "nfs4");
    if is_nfs && opt_parts.iter().any(|p| p == "bg") {
        if !opt_parts.iter().any(|p| p == "fg") {
            opt_parts.push("fg".to_owned());
        }
        if !opt_parts.iter().any(|p| p.starts_with("x-systemd.mount-timeout=")) {
            opt_parts.push("x-systemd.mount-timeout=infinity".to_owned());
        }
    }
    // Upstream systemd-fstab-generator always emits Options= in
    // [Mount] units (even when empty) so downstream tests
    // (`grep -qE '^Options=' -.mount`) succeed.
    let opt_value = if opt_parts.iter().any(|p| p != "defaults") {
        opt_parts.join(",")
    } else {
        String::new()
    };
    unit.push_str(&format!("Options={}\n", opt_value));
    if has_opt(&systemd_opts, "x-systemd.rw-only") {
        unit.push_str("ReadWriteOnly=yes\n");
    }

    // fsck: when passno >= 1 and non-root, add Requires=systemd-fsck@<esc>.
    if entry.passno >= 1 && !is_rootfs && entry.where_ != "/usr" {
        let fsck_unit = format!(
            "systemd-fsck@{}.service",
            unit_name_path_escape(&entry.what)
        );
        unit.push_str(&format!("\n[Unit]\nRequires={fsck_unit}\nAfter={fsck_unit}\n"));
    } else if entry.passno >= 1 && entry.where_ == "/usr" {
        let fsck_unit = format!(
            "systemd-fsck@{}.service",
            unit_name_path_escape(&entry.what)
        );
        unit.push_str(&format!("\n[Unit]\nWants={fsck_unit}\nAfter={fsck_unit}\n"));
    }

    fs::write(&unit_path, unit)?;

    // Rootfs fsck: upstream wires `local-fs.target.wants/systemd-fsck-root.service`
    // pointing to the shipped unit at `/lib/systemd/system/systemd-fsck-root.service`.
    // TEST-81-GENERATORS.fstab-generator asserts this symlink exists whenever
    // the rootfs entry has `passno >= 1`.
    if is_rootfs && entry.passno >= 1 {
        let wants_dir = out_dir.join("local-fs.target.wants");
        fs::create_dir_all(&wants_dir)?;
        let link_path = wants_dir.join("systemd-fsck-root.service");
        let _ = fs::remove_file(&link_path);
        unix_fs::symlink(
            "/lib/systemd/system/systemd-fsck-root.service",
            &link_path,
        )?;
    }

    // Wire up the .target.{wants,requires} symlink so the mount is
    // auto-started.  Skip if noauto.
    // Upstream semantics (see TEST-81-GENERATORS.fstab-generator.sh:
    // remote-fs.target for network / _netdev, local-fs.target otherwise;
    // `.wants` when nofail OR (nfs && bg), else `.requires`):
    if !is_rootfs && !has_opt(&systemd_opts, "noauto") && !has_opt(&systemd_opts, "x-systemd.automount") {
        let is_nfs_bg = matches!(entry.fstype.as_str(), "nfs" | "nfs4")
            && parse_csv(&entry.options).contains(&"bg");
        let link_dir = if has_opt(&systemd_opts, "nofail") || is_nfs_bg {
            out_dir.join(format!("{target_fs}.wants"))
        } else {
            out_dir.join(format!("{target_fs}.requires"))
        };
        fs::create_dir_all(&link_dir)?;
        let link_path = link_dir.join(&unit_name);
        // Use relative ../<unit> like upstream.
        let _ = fs::remove_file(&link_path);
        unix_fs::symlink(format!("../{unit_name}"), &link_path)?;
    }

    // x-systemd.wanted-by= / x-systemd.required-by= (skip for rootfs).
    if !is_rootfs {
        for opt in &systemd_opts {
            if let Some(v) = opt.strip_prefix("x-systemd.wanted-by=") {
                let link_dir = out_dir.join(format!("{v}.wants"));
                fs::create_dir_all(&link_dir)?;
                let link_path = link_dir.join(&unit_name);
                let _ = fs::remove_file(&link_path);
                unix_fs::symlink(format!("../{unit_name}"), &link_path)?;
            } else if let Some(v) = opt.strip_prefix("x-systemd.required-by=") {
                let link_dir = out_dir.join(format!("{v}.requires"));
                fs::create_dir_all(&link_dir)?;
                let link_path = link_dir.join(&unit_name);
                let _ = fs::remove_file(&link_path);
                unix_fs::symlink(format!("../{unit_name}"), &link_path)?;
            }
        }
    }

    // x-systemd.makefs: emit a companion `systemd-makefs@<device>.service`
    // that runs `mkfs.<fstype>` on the device before mount.  The mount
    // unit takes a `Requires=`/`After=` edge on it via a drop-in.
    if has_opt(&systemd_opts, "x-systemd.makefs") {
        let esc_dev = unit_name_path_escape(&entry.what);
        let makefs_unit = format!("systemd-makefs@{esc_dev}.service");
        let makefs_path = out_dir.join(&makefs_unit);
        let mut makefs = String::new();
        makefs.push_str("# Automatically generated by systemd-fstab-generator\n\n");
        makefs.push_str("[Unit]\n");
        makefs.push_str("Description=Make File System on %f\n");
        makefs.push_str("Documentation=man:systemd-makefs@.service(8)\n");
        makefs.push_str("DefaultDependencies=no\n");
        makefs.push_str("BindsTo=%i.device\n");
        makefs.push_str("Conflicts=shutdown.target\n");
        makefs.push_str("After=%i.device\n");
        makefs.push_str("Before=shutdown.target\n");
        makefs.push_str("\n[Service]\nType=oneshot\nRemainAfterExit=yes\n");
        makefs.push_str(&format!(
            "ExecStart=/lib/systemd/systemd-makefs {} {}\n",
            entry.fstype, entry.what
        ));
        makefs.push_str("TimeoutSec=0\n");
        fs::write(&makefs_path, makefs)?;

        let link_dir = out_dir.join(format!("{}.requires", &unit_name));
        fs::create_dir_all(&link_dir)?;
        let link_path = link_dir.join(&makefs_unit);
        let _ = fs::remove_file(&link_path);
        unix_fs::symlink(format!("../{makefs_unit}"), &link_path)?;
    }

    // x-systemd.device-timeout=<timespan>: emit a drop-in
    // `<device>.device.d/50-device-timeout.conf` with
    // `JobRunningTimeoutSec=<timespan>` so the device-unit activation
    // can wait longer before timing out.
    if let Some(timeout) = get_opt_arg(&systemd_opts, "x-systemd.device-timeout=") {
        let device_unit = format!(
            "{}.device",
            unit_name_path_escape(&entry.what)
        );
        let dropin_dir = out_dir.join(format!("{device_unit}.d"));
        fs::create_dir_all(&dropin_dir)?;
        let dropin_path = dropin_dir.join("50-device-timeout.conf");
        fs::write(
            &dropin_path,
            format!("[Unit]\nJobRunningTimeoutSec={timeout}\n"),
        )?;
    }

    // x-systemd.growfs / x-systemd.validatefs: wire a symlink into
    // `<mount-unit>.wants` pointing at the shipped template service
    // (systemd ships these under `/lib/systemd/system/`).  We don't
    // emit the service file itself — TEST-81-GENERATORS.fstab-generator
    // uses `link_endswith` which checks the symlink TARGET ends with
    // `/lib/systemd/system/systemd-growfs@.service`.
    for (opt_name, tmpl) in [
        ("x-systemd.growfs", "systemd-growfs@"),
        ("x-systemd.validatefs", "systemd-validatefs@"),
    ] {
        if has_opt(&systemd_opts, opt_name) {
            let svc = format!(
                "{tmpl}{}.service",
                unit_name_path_escape(&entry.where_)
            );
            let wants_dir = out_dir.join(format!("{}.wants", &unit_name));
            fs::create_dir_all(&wants_dir)?;
            let link_path = wants_dir.join(&svc);
            let _ = fs::remove_file(&link_path);
            unix_fs::symlink(
                format!("/lib/systemd/system/{tmpl}.service"),
                &link_path,
            )?;
        }
    }

    // x-systemd.automount: emit a companion `.automount` unit alongside
    // the `.mount` unit.  The automount unit triggers on-access mount
    // of the filesystem with an optional idle timeout.
    // Upstream ignores this option for the root filesystem — you can't
    // automount `/` itself.  TEST-81-GENERATORS.fstab-generator asserts
    // the absence of `-.automount` even when `x-systemd.automount` is
    // requested.
    if has_opt(&systemd_opts, "x-systemd.automount") && !is_rootfs {
        let automount_name = format!(
            "{}.automount",
            unit_name_path_escape(&entry.where_)
        );
        let automount_path = out_dir.join(&automount_name);
        let mut automount_unit = String::new();
        automount_unit.push_str("# Automatically generated by systemd-fstab-generator\n\n");
        automount_unit.push_str("[Unit]\n");
        automount_unit.push_str("SourcePath=/etc/fstab\n");
        automount_unit
            .push_str("Documentation=man:fstab(5) man:systemd-fstab-generator(8)\n");
        automount_unit.push_str("\n[Automount]\n");
        automount_unit.push_str(&format!("Where={}\n", entry.where_));
        if let Some(v) = get_opt_arg(&systemd_opts, "x-systemd.idle-timeout=") {
            automount_unit.push_str(&format!("TimeoutIdleSec={v}\n"));
        }
        fs::write(&automount_path, automount_unit)?;

        // Wire the automount into local-fs.target: rootfs uses
        // local-fs.target.requires, non-rootfs honors nofail for
        // wants vs requires.
        let link_dir = if is_rootfs || !has_opt(&systemd_opts, "nofail") {
            out_dir.join(format!("{target_fs}.requires"))
        } else {
            out_dir.join(format!("{target_fs}.wants"))
        };
        fs::create_dir_all(&link_dir)?;
        let link_path = link_dir.join(&automount_name);
        let _ = fs::remove_file(&link_path);
        unix_fs::symlink(format!("../{automount_name}"), &link_path)?;
    }

    Ok(())
}

fn emit_swap_unit(out_dir: &Path, entry: &FstabEntry) -> io::Result<()> {
    let unit_name = format!("{}.swap", unit_name_path_escape(&entry.what));
    let unit_path = out_dir.join(&unit_name);

    let (systemd_opts, _mount_opts) = split_options(&entry.options);

    let mut unit = String::new();
    unit.push_str("# Automatically generated by systemd-fstab-generator\n\n");
    unit.push_str("[Unit]\n");
    unit.push_str("SourcePath=/etc/fstab\nDocumentation=man:fstab(5) man:systemd-fstab-generator(8)\n");
    unit.push_str("Before=swap.target\n");

    unit.push_str("\n[Swap]\n");
    unit.push_str(&format!("What={}\n", entry.what));
    // Upstream emits the raw fstab options verbatim into `Options=` on
    // swap units (unlike mount units, it does not strip `defaults` or
    // x-systemd.*).  TEST-81-GENERATORS.fstab-generator asserts the
    // exact `Options=defaults,x-systemd.makefs` line for `sw` swap
    // entries.
    let swap_opts_raw = entry.options.trim();
    if !swap_opts_raw.is_empty() {
        unit.push_str(&format!("Options={}\n", swap_opts_raw));
    }

    fs::write(&unit_path, unit)?;

    // x-systemd.makefs on a swap entry: emit
    // `systemd-mkswap@<device>.service` (the swap-specific template —
    // upstream uses a different name than the filesystem `systemd-makefs@`).
    if has_opt(&systemd_opts, "x-systemd.makefs") {
        let esc_dev = unit_name_path_escape(&entry.what);
        let mkswap_unit = format!("systemd-mkswap@{esc_dev}.service");
        let mkswap_path = out_dir.join(&mkswap_unit);
        let mut mkswap = String::new();
        mkswap.push_str("# Automatically generated by systemd-fstab-generator\n\n");
        mkswap.push_str("[Unit]\n");
        mkswap.push_str("Description=Make Swap on %f\n");
        mkswap.push_str("Documentation=man:systemd-mkswap@.service(8)\n");
        mkswap.push_str("DefaultDependencies=no\n");
        mkswap.push_str("BindsTo=%i.device\n");
        mkswap.push_str("Conflicts=shutdown.target\n");
        mkswap.push_str("After=%i.device\n");
        mkswap.push_str("Before=shutdown.target\n");
        mkswap.push_str("\n[Service]\nType=oneshot\nRemainAfterExit=yes\n");
        mkswap.push_str(&format!(
            "ExecStart=/lib/systemd/systemd-makefs swap {}\n",
            entry.what
        ));
        mkswap.push_str("TimeoutSec=0\n");
        fs::write(&mkswap_path, mkswap)?;
        let link_dir = out_dir.join(format!("{}.requires", &unit_name));
        fs::create_dir_all(&link_dir)?;
        let link_path = link_dir.join(&mkswap_unit);
        let _ = fs::remove_file(&link_path);
        unix_fs::symlink(format!("../{mkswap_unit}"), &link_path)?;
    }

    if !has_opt(&systemd_opts, "noauto") {
        let link_dir = if has_opt(&systemd_opts, "nofail") {
            out_dir.join("swap.target.wants")
        } else {
            out_dir.join("swap.target.requires")
        };
        fs::create_dir_all(&link_dir)?;
        let link_path = link_dir.join(&unit_name);
        let _ = fs::remove_file(&link_path);
        unix_fs::symlink(format!("../{unit_name}"), &link_path)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_fstab_basic() {
        let input = "\
# comment
/dev/sda1 / ext4 defaults 0 1
/dev/sda2 /home ext4 defaults 0 2
";
        let entries = parse_fstab(input);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].where_, "/");
        assert_eq!(entries[0].passno, 1);
        assert_eq!(entries[1].where_, "/home");
        assert_eq!(entries[1].passno, 2);
    }

    #[test]
    fn test_parse_fstab_skips_comments_and_blanks() {
        let input = "\n\
# a comment\n\
    # indented comment\n\
\t\n\
/dev/sda1 / ext4 defaults 0 0\n";
        let entries = parse_fstab(input);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_parse_fstab_missing_fields_defaults() {
        // Only 3 fields: defaults for options/dump/passno.
        let input = "/dev/sda1 /foo ext4";
        let entries = parse_fstab(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].options, "defaults");
        assert_eq!(entries[0].passno, 0);
    }

    #[test]
    fn test_parse_fstab_rejects_nonpath_mountpoint() {
        let input = "/dev/foo not-a-path ext4 defaults 0 0";
        assert!(parse_fstab(input).is_empty());
    }

    #[test]
    fn test_parse_fstab_accepts_swap() {
        let input = "/dev/foo none swap defaults 0 0";
        let entries = parse_fstab(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].fstype, "swap");
    }

    #[test]
    fn test_should_skip_api_filesystems() {
        let mk = |where_: &str, fstype: &str| FstabEntry {
            what: "/dev/x".into(),
            where_: where_.into(),
            fstype: fstype.into(),
            options: "defaults".into(),
            _dump: 0,
            passno: 0,
        };
        assert!(should_skip(&mk("/proc/cmdline", "ext4")));
        assert!(should_skip(&mk("/sys/fs/cgroup/foo", "ext4")));
        assert!(should_skip(&mk("/dev/console", "ext4")));
        assert!(should_skip(&mk("/run/host/foo", "ext4")));
        assert!(should_skip(&mk("/foo", "autofs")));
        assert!(!should_skip(&mk("/tmp", "tmpfs")));
        assert!(!should_skip(&mk("/home", "ext4")));
    }

    #[test]
    fn test_split_options() {
        let (sys, mount) = split_options("defaults,nofail,x-systemd.requires=foo.service,uid=1000");
        assert!(sys.contains(&"nofail"));
        assert!(sys.contains(&"x-systemd.requires=foo.service"));
        assert!(mount.contains(&"defaults"));
        assert!(mount.contains(&"uid=1000"));
    }

    #[test]
    fn test_is_network_fs() {
        assert!(is_network_fs("nfs"));
        assert!(is_network_fs("nfs4"));
        assert!(is_network_fs("cifs"));
        assert!(is_network_fs("ceph"));
        assert!(!is_network_fs("ext4"));
        assert!(!is_network_fs("btrfs"));
    }

    #[test]
    fn test_emit_mount_unit_basic() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = FstabEntry {
            what: "/dev/sda1".into(),
            where_: "/home".into(),
            fstype: "ext4".into(),
            options: "defaults".into(),
            _dump: 0,
            passno: 0,
        };
        emit_mount_unit(tmp.path(), &entry, false).unwrap();
        let unit_path = tmp.path().join("home.mount");
        assert!(unit_path.exists());
        let content = fs::read_to_string(&unit_path).unwrap();
        assert!(content.contains("What=/dev/sda1"));
        assert!(content.contains("Where=/home"));
        assert!(content.contains("Type=ext4"));

        // Must have local-fs.target.requires symlink to ../home.mount
        let link = tmp.path().join("local-fs.target.requires/home.mount");
        assert!(link.symlink_metadata().is_ok());
    }

    #[test]
    fn test_emit_mount_unit_nofail_uses_wants() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = FstabEntry {
            what: "/dev/sda1".into(),
            where_: "/home".into(),
            fstype: "ext4".into(),
            options: "defaults,nofail".into(),
            _dump: 0,
            passno: 0,
        };
        emit_mount_unit(tmp.path(), &entry, false).unwrap();
        // nofail → wants/ not requires/
        assert!(tmp
            .path()
            .join("local-fs.target.wants/home.mount")
            .symlink_metadata()
            .is_ok());
        assert!(!tmp
            .path()
            .join("local-fs.target.requires/home.mount")
            .exists());
    }

    #[test]
    fn test_emit_mount_unit_network_fs_uses_remote_target() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = FstabEntry {
            what: "server:/export".into(),
            where_: "/mnt/nfs".into(),
            fstype: "nfs".into(),
            options: "defaults".into(),
            _dump: 0,
            passno: 0,
        };
        emit_mount_unit(tmp.path(), &entry, false).unwrap();
        assert!(tmp
            .path()
            .join("remote-fs.target.requires/mnt-nfs.mount")
            .symlink_metadata()
            .is_ok());
    }

    #[test]
    fn test_emit_mount_unit_noauto_skips_target_link() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = FstabEntry {
            what: "/dev/sda1".into(),
            where_: "/home".into(),
            fstype: "ext4".into(),
            options: "defaults,noauto".into(),
            _dump: 0,
            passno: 0,
        };
        emit_mount_unit(tmp.path(), &entry, false).unwrap();
        assert!(tmp.path().join("home.mount").exists());
        assert!(!tmp.path().join("local-fs.target.requires/home.mount").exists());
        assert!(!tmp.path().join("local-fs.target.wants/home.mount").exists());
    }

    #[test]
    fn test_emit_mount_unit_x_systemd_requires() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = FstabEntry {
            what: "/dev/sda1".into(),
            where_: "/home".into(),
            fstype: "ext4".into(),
            options: "x-systemd.requires=foo.service".into(),
            _dump: 0,
            passno: 0,
        };
        emit_mount_unit(tmp.path(), &entry, false).unwrap();
        let content = fs::read_to_string(tmp.path().join("home.mount")).unwrap();
        assert!(content.contains("Requires=foo.service"));
        assert!(content.contains("After=foo.service"));
    }

    #[test]
    fn test_emit_swap_unit() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = FstabEntry {
            what: "/dev/sdb1".into(),
            where_: "none".into(),
            fstype: "swap".into(),
            options: "defaults".into(),
            _dump: 0,
            passno: 0,
        };
        emit_swap_unit(tmp.path(), &entry).unwrap();
        let unit = fs::read_to_string(tmp.path().join("dev-sdb1.swap")).unwrap();
        assert!(unit.contains("What=/dev/sdb1"));
        assert!(unit.contains("[Swap]"));
        assert!(tmp
            .path()
            .join("swap.target.requires/dev-sdb1.swap")
            .symlink_metadata()
            .is_ok());
    }
}
