//! Runtime mount monitor.
//!
//! Watches `/proc/self/mountinfo` and synthesises an already-active `.mount`
//! unit for every mount that exists in the kernel but has no unit yet (for
//! example a filesystem mounted directly with `mount(8)` rather than by
//! systemd), and marks a `.mount` unit inactive once its mount disappears.
//!
//! This mirrors systemd's `mount_load_proc_self_mountinfo` / `mount_setup_unit`
//! and the `epoll(EPOLLPRI)` it keeps on `/proc/self/mountinfo`, so that
//! `systemctl is-active <path>.mount` reflects manual mounts. Required by
//! TEST-10-MOUNT and TEST-60-MOUNT-RATELIMIT.

use crate::lock_ext::RwLockExt;
use crate::runtime_info::ArcMutRuntimeInfo;
use crate::units::status::{StatusStarted, StatusStopped, UnitStatus};
use crate::units::{UnitId, UnitIdKind};

/// Decode the octal escapes (`\NNN`) the kernel uses in `/proc/self/mountinfo`
/// for space (`\040`), tab (`\011`), newline (`\012`) and backslash (`\134`).
fn octal_unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && let Ok(n) = u8::from_str_radix(&s[i + 1..i + 4], 8)
        {
            out.push(n);
            i += 4;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `(mountpoint, source, fstype)` for every entry in `/proc/self/mountinfo`.
///
/// Line format (mountinfo(5)):
/// `id parent maj:min root MOUNTPOINT opts [optional fields...] - FSTYPE SOURCE superopts`
fn parse_mountinfo() -> Vec<(String, String, String)> {
    let content = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let mut mounts = Vec::new();
    for line in content.lines() {
        // The variable-length optional fields are terminated by a single " - ".
        let Some((pre, post)) = line.split_once(" - ") else {
            continue;
        };
        let pre_fields: Vec<&str> = pre.split_whitespace().collect();
        let post_fields: Vec<&str> = post.split_whitespace().collect();
        if pre_fields.len() < 5 || post_fields.len() < 2 {
            continue;
        }
        let mountpoint = octal_unescape(pre_fields[4]);
        let fstype = octal_unescape(post_fields[0]);
        let source = octal_unescape(post_fields[1]);
        if mountpoint.starts_with('/') {
            mounts.push((mountpoint, source, fstype));
        }
    }
    mounts
}

/// Build a synthesised `.mount` unit describing a kernel mount. It carries no
/// default dependencies (a manual mount is not part of systemd's ordering).
fn build_synthesized_mount_unit(
    name: &str,
    what: &str,
    where_: &str,
    fstype: &str,
) -> Result<crate::units::Unit, String> {
    use crate::units::unit_parsing::{
        ParsedCommonConfig, ParsedMountConfig, ParsedMountSection, ParsedUnitSection,
    };

    // Mount default dependencies (systemd's mount_add_default_dependencies).
    // A device-backed local mount is ordered after local-fs-pre.target and its
    // backing device; a network filesystem after remote-fs-pre.target +
    // network.target. Both conflict with umount.target so they are torn down on
    // shutdown. NOTE: the `_netdev` userspace option (which also makes a mount
    // "remote") lives in /run/mount/utab, not /proc/self/mountinfo, and is not
    // read yet, so a `_netdev` mount is still classified local for now.
    let is_network = matches!(
        fstype,
        "nfs" | "nfs4"
            | "cifs"
            | "smb3"
            | "smbfs"
            | "ncpfs"
            | "glusterfs"
            | "ceph"
            | "afs"
            | "coda"
            | "ocfs2"
            | "orangefs"
            | "lustre"
            | "9p"
    );
    let mut after: Vec<String> = Vec::new();
    let mut wants: Vec<String> = Vec::new();
    let mut before: Vec<String> = vec!["umount.target".to_owned()];
    let mut requires: Vec<String> = Vec::new();
    let conflicts: Vec<String> = vec!["umount.target".to_owned()];
    if is_network {
        after.push("remote-fs-pre.target".to_owned());
        after.push("network.target".to_owned());
        before.push("remote-fs.target".to_owned());
    } else {
        after.push("local-fs-pre.target".to_owned());
        wants.push("local-fs-pre.target".to_owned());
        before.push("local-fs.target".to_owned());
    }
    if what.starts_with("/dev/") {
        let esc = crate::unit_name::unit_name_path_escape(what);
        after.push(format!("{esc}.device"));
        requires.push(format!("{esc}.device"));
        after.push(format!("blockdev@{esc}.target"));
    }

    let unit_section = ParsedUnitSection {
        description: where_.to_owned(),
        default_dependencies: false,
        after,
        before,
        wants,
        requires,
        conflicts,
        ..Default::default()
    };

    let conf = ParsedMountConfig {
        common: ParsedCommonConfig {
            name: name.to_owned(),
            unit: unit_section,
            install: Default::default(),
            fragment_path: None,
        },
        mount: ParsedMountSection {
            what: what.to_owned(),
            where_: where_.to_owned(),
            fs_type: if fstype.is_empty() || fstype == "auto" {
                None
            } else {
                Some(fstype.to_owned())
            },
            ..Default::default()
        },
    };
    crate::units::from_parsed_config::unit_from_parsed_mount(conf)
}

/// Synchronise the unit table with `/proc/self/mountinfo`: create + activate a
/// `.mount` unit for every mount without one, mark existing mount units active,
/// and mark mount units inactive once their mount is gone.
pub fn sync_mount_units(run_info: &ArcMutRuntimeInfo) {
    let mounts = parse_mountinfo();

    let mut current: std::collections::HashMap<String, (String, String, String)> =
        std::collections::HashMap::new();
    for (mountpoint, source, fstype) in mounts {
        let name = crate::units::unit_parsing::path_to_mount_unit_name(&mountpoint);
        current.insert(name, (mountpoint, source, fstype));
    }

    let mut ri = run_info.write_poisoned();

    // 1. Every current mount should have an active `.mount` unit.
    for (name, (mountpoint, source, fstype)) in &current {
        let id = UnitId {
            kind: UnitIdKind::Mount,
            name: name.clone(),
        };
        if let Some(u) = ri.unit_table.get(&id) {
            // Existing (fstab / unit-file / already-synthesised) unit: reflect
            // that the mount is present.
            let mut status = u.common.status.write_poisoned();
            if !matches!(&*status, UnitStatus::Started(_)) {
                *status = UnitStatus::Started(StatusStarted::Running);
            }
            continue;
        }
        match build_synthesized_mount_unit(name, source, mountpoint, fstype) {
            Ok(unit) => {
                crate::units::insert_new_unit_lenient(unit, &mut ri);
                if let Some(u) = ri.unit_table.get(&id) {
                    *u.common.status.write_poisoned() =
                        UnitStatus::Started(StatusStarted::Running);
                }
            }
            Err(e) => log::warn!("mount-monitor: failed to synthesise {name}: {e}"),
        }
    }

    // 2. Mount units that are active but whose mount is gone -> inactive.
    let stale: Vec<UnitId> = ri
        .unit_table
        .iter()
        .filter(|(id, u)| {
            id.kind == UnitIdKind::Mount
                && !current.contains_key(&id.name)
                && matches!(&*u.common.status.read_poisoned(), UnitStatus::Started(_))
        })
        .map(|(id, _)| id.clone())
        .collect();
    for id in stale {
        if let Some(u) = ri.unit_table.get(&id) {
            *u.common.status.write_poisoned() =
                UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
        }
    }
}

/// Start the `/proc/self/mountinfo` monitor thread. Does an initial sync, then
/// blocks in `poll(POLLPRI|POLLERR)` on the mountinfo fd (which the kernel wakes
/// on any mount-table change), re-syncing on each change. A short timeout also
/// re-syncs periodically as a safety net.
pub fn start_mount_monitor_thread(run_info: ArcMutRuntimeInfo) {
    std::thread::spawn(move || {
        use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
        use std::io::{Read, Seek, SeekFrom};
        use std::os::fd::AsRawFd;

        // Initial sync so mounts already present at start-up get their units.
        sync_mount_units(&run_info);

        let mut file = match std::fs::File::open("/proc/self/mountinfo") {
            Ok(f) => f,
            Err(e) => {
                log::warn!("mount-monitor: cannot open /proc/self/mountinfo: {e}");
                return;
            }
        };
        let fd = file.as_raw_fd();
        let mut scratch = Vec::new();
        loop {
            let mut fds = [PollFd::new(
                unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) },
                PollFlags::POLLPRI | PollFlags::POLLERR,
            )];
            // 1s safety timeout in case a change is ever missed.
            match poll(&mut fds, PollTimeout::try_from(1000).unwrap_or(PollTimeout::ZERO)) {
                Ok(_) => {}
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => {
                    log::warn!("mount-monitor: poll failed: {e}");
                    return;
                }
            }
            // Consume the readiness by re-reading the file from the start, so
            // the next poll blocks again instead of spinning on POLLPRI.
            let _ = file.seek(SeekFrom::Start(0));
            scratch.clear();
            let _ = file.read_to_end(&mut scratch);
            sync_mount_units(&run_info);
        }
    });
}
