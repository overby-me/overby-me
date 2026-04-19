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
use std::io::{self, Write};
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
    let in_initrd = env::var("SYSTEMD_IN_INITRD")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false);

    let entries = match load_fstab(&fstab_path) {
        Ok(e) => e,
        Err(e) => {
            // Missing fstab is not an error: just write nothing.
            if e.kind() == io::ErrorKind::NotFound {
                return ExitCode::SUCCESS;
            }
            eprintln!("systemd-fstab-generator: cannot read {fstab_path}: {e}");
            return ExitCode::from(1);
        }
    };

    let mut seen_mountpoints: BTreeSet<String> = BTreeSet::new();
    let mut had_error = false;
    for entry in &entries {
        if should_skip(entry) {
            continue;
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

        let result = if entry.fstype == "swap" {
            emit_swap_unit(&normal_dir, entry)
        } else {
            emit_mount_unit(&normal_dir, entry, in_initrd)
        };
        if let Err(e) = result {
            eprintln!(
                "systemd-fstab-generator: failed to emit unit for {}: {e}",
                entry.where_
            );
            had_error = true;
        }
    }

    if had_error {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
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
    opts.iter().any(|o| *o == name)
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
    unit.push_str(&format!(
        "SourcePath=/etc/fstab\nDocumentation=man:fstab(5) man:systemd-fstab-generator(8)\n"
    ));
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
    let target_fs = if is_network_fs(&entry.fstype) || has_opt(&systemd_opts, "_netdev") {
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
    // Pass through some x-options the kernel/userland understands.
    for opt in &systemd_opts {
        if *opt == "_netdev" {
            opt_parts.push("_netdev".to_owned());
        }
    }
    if !opt_parts.is_empty() && opt_parts.iter().any(|p| p != "defaults") {
        unit.push_str(&format!("Options={}\n", opt_parts.join(",")));
    }
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

    // Wire up the .target.{wants,requires} symlink so the mount is
    // auto-started.  Skip if noauto.
    if !is_rootfs && !has_opt(&systemd_opts, "noauto") && !has_opt(&systemd_opts, "x-systemd.automount") {
        let link_dir = if has_opt(&systemd_opts, "nofail") {
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
    // Filter to only "real" swap options (pri=, discard).
    let swap_opts: Vec<&str> = parse_csv(&entry.options)
        .into_iter()
        .filter(|o| !o.starts_with("x-systemd.") && !matches!(*o, "defaults" | "noauto" | "nofail" | "sw"))
        .collect();
    if !swap_opts.is_empty() {
        unit.push_str(&format!("Options={}\n", swap_opts.join(",")));
    }

    fs::write(&unit_path, unit)?;

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
        let input = "\

# a comment
    # indented comment
\t
/dev/sda1 / ext4 defaults 0 0
";
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
