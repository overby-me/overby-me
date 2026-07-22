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
pub(crate) fn fstype_is_network(fstype: &str) -> bool {
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

/// Whether a mount unit is a network filesystem, from its unit-file config: a
/// network fstype or an explicit `_netdev` in its `Options=`. For static / fstab
/// mount units; runtime-synthesised mounts use the utab-based check instead.
pub(crate) fn mount_is_network_static(fs_type: Option<&str>, options: Option<&str>) -> bool {
    fs_type.map(fstype_is_network).unwrap_or(false)
        || options
            .map(|o| o.split(',').any(|f| f == "_netdev"))
            .unwrap_or(false)
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
pub(crate) fn mount_default_deps(
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
pub(crate) fn dep_unit_id(name: &str) -> Option<UnitId> {
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

/// Rewrite a mount unit's default-dependencies in place to match `is_network`
/// (and the backing `what`): scrub the old mount default-deps and add the new
/// ones. Used to reclassify a synthesised mount, to make an active mount
/// override a static unit's declared ordering, and to revert a static unit.
fn apply_mount_deps_in_place(d: &mut crate::units::Dependencies, what: &str, is_network: bool) {
    d.after.retain(|x| !is_mount_default_dep(&x.name));
    d.before.retain(|x| !is_mount_default_dep(&x.name));
    d.wants.retain(|x| !is_mount_default_dep(&x.name));
    d.requires.retain(|x| !is_mount_default_dep(&x.name));
    let (na, nb, nw, nr) = mount_default_deps(what, is_network);
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

/// Mark a unit Started(Running) if not already, using a non-blocking status
/// lock (skip if contended; the next sync retries). Blocking on a per-unit lock
/// while holding the RuntimeInfo lock risks a deadlock.
fn try_mark_started(status: &std::sync::RwLock<UnitStatus>) {
    let mut st = match status.try_write() {
        Ok(st) => st,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return,
    };
    if !matches!(&*st, UnitStatus::Started(_)) {
        *st = UnitStatus::Started(StatusStarted::Running);
    }
}

/// Mark a unit Stopped, using a non-blocking status lock (see `try_mark_started`).
fn try_set_stopped(status: &std::sync::RwLock<UnitStatus>) {
    let mut st = match status.try_write() {
        Ok(st) => st,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return,
    };
    *st = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
}

/// Synchronise the unit table with `/proc/self/mountinfo` + `/run/mount/utab`.
///
/// `synthesized` tracks units this monitor created (name -> last is_network);
/// they are removed when their mount disappears. `overridden` tracks static /
/// unit-file mounts whose declared ordering the monitor has replaced with the
/// active mount's (systemd coldplugs the running mount over the unit file); when
/// their mount disappears the deps are reverted to the unit-file config.
pub fn sync_mount_units(
    run_info: &ArcMutRuntimeInfo,
    synthesized: &mut HashMap<String, bool>,
    overridden: &mut HashMap<String, bool>,
) {
    // name -> (mountpoint, source, fstype, is_network)
    let mut current: HashMap<String, (String, String, String, bool)> = HashMap::new();
    for (mountpoint, source, fstype) in parse_mountinfo() {
        let name = crate::units::unit_parsing::path_to_mount_unit_name(&mountpoint);
        let is_network = fstype_is_network(&fstype) || mount_has_netdev(&mountpoint, &source);
        current.insert(name, (mountpoint, source, fstype, is_network));
    }

    // Take the RuntimeInfo write lock (blocking) so the sync reliably lands even
    // while a test is rapidly polling `systemctl is-active`. This is safe from
    // the TEST-03-JOBS deadlock because (a) per-unit status is set non-blocking
    // (try_mark_started / try_set_stopped) so we never block on a status lock
    // while holding this one, and (b) the caller only syncs in response to an
    // actual mount-table change, so the monitor is idle (not contending) during
    // transaction stress with no mount activity.
    let mut ri = match run_info.write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };

    for (name, (mountpoint, source, fstype, is_network)) in &current {
        let id = UnitId {
            kind: UnitIdKind::Mount,
            name: name.clone(),
        };
        if ri.unit_table.contains_key(&id) {
            // Reconcile the tracking maps against the unit's actual nature. A
            // stale `synthesized` entry (left by a prior synthesised mount at
            // this path whose async removal lost the race with a daemon-reload
            // that then loaded a *static* unit of the same name) would otherwise
            // route the static unit through the synthesised branch and skip the
            // override. Key off the fragment path: Some => loaded from disk
            // (static), None => synthesised by this monitor.
            let has_fragment = ri
                .unit_table
                .get(&id)
                .map(|u| u.common.unit.fragment_path.is_some())
                .unwrap_or(false);
            if has_fragment {
                synthesized.remove(name);
            } else {
                overridden.remove(name);
            }
            if let Some(prev) = synthesized.get(name).copied() {
                // A unit this monitor synthesised: reclassify in place if its
                // local/remote classification changed (e.g. _netdev appeared in
                // /run/mount/utab after the kernel mount).
                if prev != *is_network {
                    if let Some(u) = ri.unit_table.get_mut(&id) {
                        apply_mount_deps_in_place(&mut u.common.dependencies, source, *is_network);
                    }
                    synthesized.insert(name.clone(), *is_network);
                }
            } else if overridden.get(name).copied() != Some(*is_network) {
                // A static / fstab mount unit: the active mount overrides its
                // declared local/remote classification while it is mounted.
                if let Some(u) = ri.unit_table.get_mut(&id) {
                    apply_mount_deps_in_place(&mut u.common.dependencies, source, *is_network);
                }
                overridden.insert(name.clone(), *is_network);
            }
            if let Some(u) = ri.unit_table.get(&id) {
                try_mark_started(&u.common.status);
            }
            continue;
        }

        match build_synthesized_mount_unit(name, source, mountpoint, fstype, *is_network) {
            Ok(unit) => {
                crate::units::insert_new_unit_lenient(unit, &mut ri);
                if let Some(u) = ri.unit_table.get(&id) {
                    try_mark_started(&u.common.status);
                }
                synthesized.insert(name.clone(), *is_network);
            }
            Err(e) => log::warn!("mount-monitor: failed to synthesise {name}: {e}"),
        }
    }

    // Synthesised units whose mount has gone away -> remove.
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
            try_set_stopped(&u.common.status);
        }
        // Dependency-aware removal so the unit is scrubbed from every other
        // unit's dep lists; a raw remove would leave dangling reverse-deps that
        // get re-resolved onto the next unit synthesised for the same path.
        if crate::units::remove_unit_with_dependencies(id.clone(), &mut ri).is_err() {
            ri.unit_table.remove(&id);
        }
        synthesized.remove(&name);
        overridden.remove(&name);
    }

    // Static / fstab units whose mount has gone away -> revert their deps to the
    // unit-file config (do NOT remove them; the unit file still exists).
    let reverted: Vec<String> = overridden
        .keys()
        .filter(|n| !current.contains_key(*n))
        .cloned()
        .collect();
    for name in reverted {
        let id = UnitId {
            kind: UnitIdKind::Mount,
            name: name.clone(),
        };
        if let Some(u) = ri.unit_table.get_mut(&id) {
            let (what, is_net) = if let crate::units::Specific::Mount(m) = &u.specific {
                (
                    m.conf.what.clone(),
                    mount_is_network_static(m.conf.fs_type.as_deref(), m.conf.options.as_deref()),
                )
            } else {
                (String::new(), false)
            };
            apply_mount_deps_in_place(&mut u.common.dependencies, &what, is_net);
            try_set_stopped(&u.common.status);
        }
        overridden.remove(&name);
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
        let mut overridden: HashMap<String, bool> = HashMap::new();

        // Initial sync so mounts already present at start-up get their units.
        sync_mount_units(&run_info, &mut synthesized, &mut overridden);

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
            // Block until the kernel signals a mount-table change (POLLPRI), with
            // a long safety timeout. Do NOT sync on the timeout: the monitor
            // stays idle (never contends for the RuntimeInfo lock) whenever there
            // is no mount activity, which keeps it clear of PID 1 during
            // transaction stress with no mounts (TEST-03-JOBS deadlocked when the
            // old 1s periodic sync contended there).
            let n = match poll(
                &mut fds,
                PollTimeout::try_from(30_000).unwrap_or(PollTimeout::ZERO),
            ) {
                Ok(n) => n,
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => {
                    log::warn!("mount-monitor: poll failed: {e}");
                    return;
                }
            };
            if n == 0 {
                continue; // timeout, no mount change: stay idle
            }
            // Consume the readiness by re-reading the file from the start, so
            // the next poll blocks again instead of spinning on POLLPRI.
            let _ = file.seek(SeekFrom::Start(0));
            scratch.clear();
            let _ = file.read_to_end(&mut scratch);
            // Sync now, then re-sync a few times: libmount writes /run/mount/utab
            // (where `_netdev` lives) slightly AFTER the kernel mount, so a
            // mount's network classification can appear only on a later pass. No
            // lock is held across the sleeps, and these run only right after a
            // real change, so they never contend during mount-idle stress.
            sync_mount_units(&run_info, &mut synthesized, &mut overridden);
            for _ in 0..4 {
                std::thread::sleep(std::time::Duration::from_millis(300));
                sync_mount_units(&run_info, &mut synthesized, &mut overridden);
            }
        }
    });
}
