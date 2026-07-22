//! Runtime mount monitor.
//!
//! Watches `/proc/self/mountinfo` (and `/run/mount/utab` for userspace mount
//! options) and synthesises an already-active `.mount` unit for every mount
//! that exists in the kernel but has no unit yet (for example a filesystem
//! mounted directly with `mount(8)` rather than by systemd), and marks a
//! `.mount` unit inactive once its mount disappears.
//!
//! Mirrors systemd's `mount_load_proc_self_mountinfo` / `mount_setup_unit` and
//! the `epoll(EPOLLPRI)` it keeps on `/proc/self/mountinfo`, so that
//! `systemctl is-active <path>.mount` reflects manual mounts. Required by
//! TEST-10-MOUNT and TEST-60-MOUNT-RATELIMIT.

use crate::lock_ext::RwLockExt;
use crate::runtime_info::ArcMutRuntimeInfo;
use crate::units::status::{StatusStarted, StatusStopped, UnitStatus};
use crate::units::{UnitId, UnitIdKind};
use std::collections::HashMap;

/// Decode the octal escapes (`\NNN`) the kernel and libmount use in
/// `/proc/self/mountinfo` and `/run/mount/utab` for space (`\040`), tab
/// (`\011`), newline (`\012`) and backslash (`\134`).
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

/// True for filesystem types that are inherently network filesystems, which get
/// remote-fs ordering even without an explicit `_netdev`.
fn fstype_is_network(fstype: &str) -> bool {
    matches!(
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
    )
}

/// Whether the mount of `source` at `target` carries the userspace `_netdev`
/// option, read from `/run/mount/utab`. libmount writes one line per mount as
/// space-separated `KEY=value` tokens, e.g.
/// `ID=1 SRC=/dev/x TARGET=/mnt ROOT=/ OPTS=_netdev`. The `_netdev` option only
/// lives here, never in `/proc/self/mountinfo`, and is written slightly after
/// mountinfo — the periodic re-sync picks it up.
///
/// The entry is matched on BOTH target and source: rust-systemd's
/// `deactivate_mount` uses the `umount2` syscall, which (unlike the libmount
/// `umount` command) does not remove the utab entry, so a stale entry for a
/// previous mount at the same path must not be mistaken for the current one.
fn mount_has_netdev(target: &str, source: &str) -> bool {
    let content = match std::fs::read_to_string("/run/mount/utab") {
        Ok(c) => c,
        Err(_) => return false,
    };
    for line in content.lines() {
        let mut this_target: Option<String> = None;
        let mut this_src: Option<String> = None;
        let mut opts: Option<&str> = None;
        for tok in line.split_whitespace() {
            if let Some(v) = tok.strip_prefix("TARGET=") {
                this_target = Some(octal_unescape(v));
            } else if let Some(v) = tok.strip_prefix("SRC=") {
                this_src = Some(octal_unescape(v));
            } else if let Some(v) = tok.strip_prefix("OPTS=") {
                opts = Some(v);
            }
        }
        if this_target.as_deref() == Some(target) && this_src.as_deref() == Some(source) {
            return opts
                .map(|o| o.split(',').any(|f| f == "_netdev"))
                .unwrap_or(false);
        }
    }
    false
}

/// The mount ordering/pull-in dependency names for a mount, per systemd's
/// `mount_add_default_dependencies`. Returns `(after, before, wants, requires)`.
fn mount_default_deps(
    what: &str,
    is_network: bool,
) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let mut after: Vec<String> = Vec::new();
    let mut wants: Vec<String> = Vec::new();
    let mut before: Vec<String> = vec!["umount.target".to_owned()];
    let mut requires: Vec<String> = Vec::new();
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
    (after, before, wants, requires)
}

/// Whether `name` is one of the mount default-dependency targets/devices, so it
/// can be scrubbed when a mount's local/remote classification changes.
fn is_mount_default_dep(name: &str) -> bool {
    matches!(
        name,
        "local-fs-pre.target"
            | "remote-fs-pre.target"
            | "local-fs.target"
            | "remote-fs.target"
            | "network.target"
            | "umount.target"
    ) || name.ends_with(".device")
        || name.starts_with("blockdev@")
}

/// Build a `UnitId` for a dependency name, deriving the kind from the suffix.
/// Only the kinds used by mount default-dependencies are handled.
fn dep_unit_id(name: &str) -> Option<UnitId> {
    let kind = if name.ends_with(".target") {
        UnitIdKind::Target
    } else if name.ends_with(".device") {
        UnitIdKind::Device
    } else if name.ends_with(".mount") {
        UnitIdKind::Mount
    } else {
        return None;
    };
    Some(UnitId {
        kind,
        name: name.to_owned(),
    })
}

/// Build a synthesised `.mount` unit describing a kernel mount, with systemd's
/// `mount_add_default_dependencies` ordering. `is_network` selects the
/// remote-fs vs local-fs targets.
fn build_synthesized_mount_unit(
    name: &str,
    what: &str,
    where_: &str,
    fstype: &str,
    is_network: bool,
) -> Result<crate::units::Unit, String> {
    use crate::units::unit_parsing::{
        ParsedCommonConfig, ParsedMountConfig, ParsedMountSection, ParsedUnitSection,
    };

    let (after, before, wants, requires) = mount_default_deps(what, is_network);
    let conflicts: Vec<String> = vec!["umount.target".to_owned()];

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

/// Synchronise the unit table with `/proc/self/mountinfo` + `/run/mount/utab`.
/// `synthesized` maps the names of units this monitor created to their last
/// `is_network` classification, so a mount whose userspace options change (e.g.
/// `_netdev` appears in utab after the kernel mount) is rebuilt with the right
/// ordering.
pub fn sync_mount_units(run_info: &ArcMutRuntimeInfo, synthesized: &mut HashMap<String, bool>) {
    // name -> (mountpoint, source, fstype, is_network)
    let mut current: HashMap<String, (String, String, String, bool)> = HashMap::new();
    for (mountpoint, source, fstype) in parse_mountinfo() {
        let name = crate::units::unit_parsing::path_to_mount_unit_name(&mountpoint);
        let is_network = fstype_is_network(&fstype) || mount_has_netdev(&mountpoint, &source);
        current.insert(name, (mountpoint, source, fstype, is_network));
    }

    let mut ri = run_info.write_poisoned();

    for (name, (mountpoint, source, fstype, is_network)) in &current {
        let id = UnitId {
            kind: UnitIdKind::Mount,
            name: name.clone(),
        };
        let prev_class = synthesized.get(name).copied();

        if ri.unit_table.contains_key(&id) {
            match prev_class {
                // Ours, and the classification is unchanged: just keep active.
                Some(c) if c == *is_network => {
                    if let Some(u) = ri.unit_table.get(&id) {
                        let mut st = u.common.status.write_poisoned();
                        if !matches!(&*st, UnitStatus::Started(_)) {
                            *st = UnitStatus::Started(StatusStarted::Running);
                        }
                    }
                    continue;
                }
                // Ours, but the classification changed (e.g. _netdev appeared in
                // /run/mount/utab after the kernel mount). Rewrite the mount
                // default-dependencies in place: remove + re-insert would
                // re-resolve the old reverse-deps (local-fs-pre.target, the old
                // device) straight back onto the new unit.
                Some(_) => {
                    let (na, nb, nw, nr) = mount_default_deps(source, *is_network);
                    if let Some(u) = ri.unit_table.get_mut(&id) {
                        {
                            let d = &mut u.common.dependencies;
                            d.after.retain(|x| !is_mount_default_dep(&x.name));
                            d.before.retain(|x| !is_mount_default_dep(&x.name));
                            d.wants.retain(|x| !is_mount_default_dep(&x.name));
                            d.requires.retain(|x| !is_mount_default_dep(&x.name));
                            for n in na {
                                if let Some(uid) = dep_unit_id(&n) {
                                    d.after.push(uid);
                                }
                            }
                            for n in nb {
                                if let Some(uid) = dep_unit_id(&n) {
                                    d.before.push(uid);
                                }
                            }
                            for n in nw {
                                if let Some(uid) = dep_unit_id(&n) {
                                    d.wants.push(uid);
                                }
                            }
                            for n in nr {
                                if let Some(uid) = dep_unit_id(&n) {
                                    d.requires.push(uid);
                                }
                            }
                            d.dedup();
                        }
                        *u.common.status.write_poisoned() =
                            UnitStatus::Started(StatusStarted::Running);
                    }
                    synthesized.insert(name.clone(), *is_network);
                    continue;
                }
                // A static (fstab / unit-file) mount unit: leave its deps alone,
                // only reflect that the mount is present.
                None => {
                    if let Some(u) = ri.unit_table.get(&id) {
                        let mut st = u.common.status.write_poisoned();
                        if !matches!(&*st, UnitStatus::Started(_)) {
                            *st = UnitStatus::Started(StatusStarted::Running);
                        }
                    }
                    continue;
                }
            }
        }

        match build_synthesized_mount_unit(name, source, mountpoint, fstype, *is_network) {
            Ok(unit) => {
                crate::units::insert_new_unit_lenient(unit, &mut ri);
                if let Some(u) = ri.unit_table.get(&id) {
                    *u.common.status.write_poisoned() = UnitStatus::Started(StatusStarted::Running);
                }
                synthesized.insert(name.clone(), *is_network);
            }
            Err(e) => log::warn!("mount-monitor: failed to synthesise {name}: {e}"),
        }
    }

    // Remove synthesised units whose mount has gone away (unmounted).
    let gone: Vec<String> = synthesized
        .keys()
        .filter(|n| !current.contains_key(*n))
        .cloned()
        .collect();
    for name in gone {
        let id = UnitId {
            kind: UnitIdKind::Mount,
            name: name.clone(),
        };
        if let Some(u) = ri.unit_table.get(&id) {
            *u.common.status.write_poisoned() =
                UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
        }
        // Dependency-aware removal so the unit is scrubbed from every other
        // unit's dep lists; a raw remove would leave dangling reverse-deps that
        // get re-resolved onto the next unit synthesised for the same path.
        if crate::units::remove_unit_with_dependencies(id.clone(), &mut ri).is_err() {
            ri.unit_table.remove(&id);
        }
        synthesized.remove(&name);
    }
}

/// Start the `/proc/self/mountinfo` monitor thread. Does an initial sync, then
/// blocks in `poll(POLLPRI|POLLERR)` on the mountinfo fd (which the kernel wakes
/// on any mount-table change), re-syncing on each change. A 1s timeout also
/// re-syncs periodically, which additionally picks up delayed `/run/mount/utab`
/// updates (e.g. userspace `_netdev`).
pub fn start_mount_monitor_thread(run_info: ArcMutRuntimeInfo) {
    std::thread::spawn(move || {
        use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
        use std::io::{Read, Seek, SeekFrom};
        use std::os::fd::AsRawFd;

        let mut synthesized: HashMap<String, bool> = HashMap::new();

        // Initial sync so mounts already present at start-up get their units.
        sync_mount_units(&run_info, &mut synthesized);

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
            sync_mount_units(&run_info, &mut synthesized);
        }
    });
}
