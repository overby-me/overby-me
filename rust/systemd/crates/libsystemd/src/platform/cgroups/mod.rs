//! This module provides methods to manage processes with cgroups. Not resource management but reliable tracking of services.
//! It dynamically decides whether cgroups v1 or v2 should be used.
//!
//! The cgroup paths created by `get_own_freezer` return a path that is inside the cgroup that contains rust-systemd itself. With the naming scheme of the freezer
//! cgroups we should mostly comply to the guidelines here <https://www.freedesktop.org/wiki/Software/systemd/PaxControlGroups>/

use std::fs;
use std::io::Read;

use log::trace;

pub(crate) mod bpf_devices;
mod cgroup1;
pub(crate) mod cgroup2;

#[derive(Debug)]
pub enum CgroupError {
    IOErr(std::io::Error, String),
    NixErr(nix::Error),
    NotMounted,
}

impl std::fmt::Display for CgroupError {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let msg = match self {
            Self::IOErr(e, f) => format!("io error: {e}, file: {f}"),
            Self::NixErr(e) => format!("nix error: {e}"),
            Self::NotMounted => "The freezer cgroup was not mounted".into(),
        };
        fmt.write_str(&msg)
    }
}

fn use_v2(cgroup_path: &std::path::Path) -> bool {
    let freeze_file = cgroup_path.join("cgroup.freeze");
    let exists = freeze_file.exists();
    trace!("{freeze_file:?} exists: {exists}");
    exists
}

const OWN_CGROUP_NAME: &str = "systemd_rs_self";

/// The cgroup name used for PID 1 (like systemd's init.scope).
const INIT_SCOPE_NAME: &str = "init.scope";

/// Detect the cgroup v2 root mount point.
/// On pure v2 systems this is `base_path` directly (e.g. `/sys/fs/cgroup/`).
/// On hybrid systems the v2 tree is at `base_path/unified/`.
fn detect_v2_root(base_path: &std::path::Path) -> Option<std::path::PathBuf> {
    // Pure v2: base_path itself has cgroup.procs
    if base_path.join("cgroup.procs").exists() {
        return Some(base_path.to_path_buf());
    }
    // Hybrid: base_path/unified/ is the v2 mount
    let unified = base_path.join("unified");
    if unified.join("cgroup.procs").exists() {
        return Some(unified);
    }
    None
}

/// moves rust-systemd into own cgroup if v2 is used
///
/// This is necessary because cgroupv2 discourages processes in cgroups that are not leafes.
/// PID 1 is placed in `init.scope` under the cgroup root, matching real systemd's layout.
pub fn move_to_own_cgroup(base_path: &std::path::Path) -> Result<(), CgroupError> {
    trace!("Move rust-systemd to own manager cgroup");
    if let Some(v2_root) = detect_v2_root(base_path) {
        let init_scope = v2_root.join(INIT_SCOPE_NAME);
        trace!("Manager path (init.scope): {init_scope:?}");
        if !init_scope.exists() {
            std::fs::create_dir_all(&init_scope)
                .map_err(|e| CgroupError::IOErr(e, format!("{init_scope:?}")))?;
        }
        move_self_to_cgroup(&init_scope)?;
    } else {
        // Fallback for legacy cgroup v1 systems
        let proc_content = std::fs::read_to_string("/proc/self/cgroup").unwrap();
        let proc_content_lines = proc_content.split('\n').collect::<Vec<_>>();
        let v2path = get_own_cgroup_v2(&proc_content_lines);
        if let Some(v2path) = v2path {
            let absolute_v2path = base_path.join("unified").join(v2path);
            let systemd_rs_subgroup =
                absolute_v2path.join(format!("systemd_rs_{}", nix::unistd::getpid()));
            let manager_cgroup = systemd_rs_subgroup.join(OWN_CGROUP_NAME);
            trace!("Manager path (legacy): {manager_cgroup:?}");
            if !manager_cgroup.exists() {
                std::fs::create_dir_all(&manager_cgroup)
                    .map_err(|e| CgroupError::IOErr(e, format!("{manager_cgroup:?}")))?;
            }
            move_self_to_cgroup(&manager_cgroup)?;
        }
    }
    Ok(())
}

pub fn move_out_of_own_cgroup(base_path: &std::path::Path) -> Result<(), CgroupError> {
    if let Some(v2_root) = detect_v2_root(base_path) {
        let init_scope = v2_root.join(INIT_SCOPE_NAME);
        if init_scope.exists() {
            trace!("Move rust-systemd to cgroup root: {v2_root:?}");
            move_self_to_cgroup(&v2_root)?;
            trace!("Remove init.scope: {init_scope:?}");
            let _ = std::fs::remove_dir(&init_scope);
        }
    } else {
        let proc_content = std::fs::read_to_string("/proc/self/cgroup").unwrap();
        let proc_content_lines = proc_content.split('\n').collect::<Vec<_>>();
        if let Some(v2path) = get_own_cgroup_v2(&proc_content_lines) {
            let absolute_v2path = base_path.join(v2path);
            let mut parent_group = absolute_v2path.clone();
            parent_group.pop();
            trace!("Move rust-systemd to parent cgroup: {parent_group:?}");
            move_self_to_cgroup(&parent_group)?;

            let self_cgroup = absolute_v2path.join("systemd_rs_self");
            trace!("Remove manager cgroup: {self_cgroup:?}");
            std::fs::remove_dir(&self_cgroup)
                .map_err(|e| CgroupError::IOErr(e, format!("{self_cgroup:?}")))?;

            trace!("Remove rust-systemd managed cgroup: {absolute_v2path:?}");
            std::fs::remove_dir(&absolute_v2path)
                .map_err(|e| CgroupError::IOErr(e, format!("{absolute_v2path:?}")))?;
        }
    }
    Ok(())
}

/// Get the cgroup v2 root where service cgroups should be created.
/// This returns the root of the cgroup hierarchy (e.g. `/sys/fs/cgroup/`),
/// NOT PID 1's own cgroup. Service cgroups are placed directly under this
/// root (with slice hierarchy), matching real systemd's layout.
pub fn get_cgroup_root(base_path: &std::path::Path) -> Result<std::path::PathBuf, CgroupError> {
    if let Some(v2_root) = detect_v2_root(base_path) {
        trace!("Cgroup root: {v2_root:?}");
        Ok(v2_root)
    } else {
        // Fallback to get_own_freezer for non-v2 systems
        get_own_freezer(base_path)
    }
}

/// `base_path` should normally be /sys/fs/cgroup
///
/// Tries to get the most sensible path to create our own cgroup under.
/// Depending on whether cgroupv2 freezing is available It's either a path in
/// 1. /sys/fs/cgroup/freezer
/// 1. /sys/fs/cgroup/unified
///
/// The concrete path will be some sub-directory depending on the cgroup rust-systemd has been started in
pub fn get_own_freezer(base_path: &std::path::Path) -> Result<std::path::PathBuf, CgroupError> {
    let proc_content = std::fs::read_to_string("/proc/self/cgroup").unwrap();
    let proc_content_lines = proc_content.split('\n').collect::<Vec<_>>();

    let v1path = get_own_cgroup_v1(&proc_content_lines);
    let v1_full_path = base_path.join("freezer").join(v1path);
    trace!("v1 cgroup: {v1_full_path:?}");

    let v2path = get_own_cgroup_v2(&proc_content_lines);

    // prefer v2 path but fall back to v1 freezer
    let cgroup_path = if let Some(v2path) = v2path {
        // On hybrid systems the v2 hierarchy is mounted at base_path/unified/
        // and /proc/self/cgroup shows paths relative to that mount.
        // On pure v2 systems the hierarchy is mounted at base_path/ directly,
        // but move_to_own_cgroup creates a directory called "unified/" within
        // it, so /proc/self/cgroup shows "unified/..." as part of the path.
        // Try base_path/unified/<path> first (hybrid), then base_path/<path>
        // (pure v2) to handle both layouts.
        let hybrid_path = base_path.join("unified").join(&v2path);
        let pure_v2_path = base_path.join(&v2path);
        trace!("v2 hybrid cgroup: {hybrid_path:?}");
        trace!("v2 pure cgroup: {pure_v2_path:?}");

        if hybrid_path.join("cgroup.freeze").exists() {
            hybrid_path
        } else if pure_v2_path.join("cgroup.freeze").exists() {
            pure_v2_path
        } else {
            v1_full_path
        }
    } else {
        v1_full_path
    };

    trace!("Own cgroup: {cgroup_path:?}");

    fs::create_dir_all(&cgroup_path)
        .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_path:?}")))?;

    Ok(cgroup_path)
}

/// cgroup v2 appears in /proc/self/cgroup as `0::/path/to/cgroup`
/// but the path is relative to the mount point of cgroups (/sys/fs/cgroup/unified).
#[must_use]
pub fn get_own_cgroup_v2(proc_cgroup_content: &[&str]) -> Option<std::path::PathBuf> {
    for line in proc_cgroup_content {
        if let Some(path) = line.strip_prefix("0::") {
            // if we are already in the manager cgroup ignore that one. Return the managed cgroup
            let path = path.trim_end_matches(OWN_CGROUP_NAME);
            // ignore leading "/"
            let path = std::path::PathBuf::from(&path[1..]);
            return Some(path);
        }
    }
    None
}

/// Try to find the cgroup path for the freezer controller
/// If we are in / for freezer find the longest path used in any other cgroup and use that.
///
/// cgroups v1 by convention use the same (or a subset) directory trees under each controller so using the
/// longest path gives us the most specialized categorization and is probably what others would expect rust-systemd to do?
fn get_own_cgroup_v1(proc_cgroup_content: &[&str]) -> std::path::PathBuf {
    let mut freezer_path = None;
    let mut longest_path = "/".to_owned();

    for line in proc_cgroup_content {
        let triple = line.split(':').collect::<Vec<_>>();
        if triple.len() == 3 {
            let controller = triple[1];
            let path = triple[2];

            if controller.eq("freezer") {
                // ignore leading "/"
                let path = &path[1..];
                freezer_path = Some(std::path::PathBuf::from(path));
            }

            if path.len() > longest_path.len() {
                path.clone_into(&mut longest_path);
            }
        }
    }

    if let Some(p) = freezer_path {
        p
    } else {
        // ignore leading "/"
        std::path::PathBuf::from(&longest_path[1..])
    }
}

/// move a process into the cgroup. In rust-systemd the child process will call `move_self` for convenience
pub fn move_pid_to_cgroup(
    cgroup_path: &std::path::Path,
    pid: nix::unistd::Pid,
) -> Result<(), CgroupError> {
    if use_v2(cgroup_path) {
        cgroup2::move_pid_to_cgroup(cgroup_path, pid)
    } else {
        cgroup1::move_pid_to_cgroup(cgroup_path, pid)
    }
}

/// move this process into the cgroup. Used by rust-systemd after forking
pub fn move_self_to_cgroup(cgroup_path: &std::path::Path) -> Result<(), CgroupError> {
    if use_v2(cgroup_path) {
        cgroup2::move_self_to_cgroup(cgroup_path)
    } else {
        cgroup1::move_self_to_cgroup(cgroup_path)
    }
}

/// retrieve all pids that are currently in this cgroup
pub fn get_all_procs(cgroup_path: &std::path::Path) -> Result<Vec<nix::unistd::Pid>, CgroupError> {
    let mut pids = Vec::new();
    let cgroup_procs = cgroup_path.join("cgroup.procs");
    let mut f = fs::File::open(&cgroup_procs)
        .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_procs:?}")))?;
    let mut buf = String::new();
    f.read_to_string(&mut buf)
        .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_procs:?}")))?;

    for pid_str in buf.split('\n') {
        if pid_str.is_empty() {
            break;
        }
        if let Ok(pid) = pid_str.parse::<i32>() {
            pids.push(nix::unistd::Pid::from_raw(pid));
        }
    }
    Ok(pids)
}

/// kill all processes that are currently in this cgroup.
/// This makes sure that the cgroup is first completely frozen
/// so all processes will be killed and there is no chance of any
/// remaining
pub fn freeze_kill_thaw_cgroup(
    cgroup_path: &std::path::Path,
    sig: nix::sys::signal::Signal,
) -> Result<(), CgroupError> {
    // TODO figure out how to freeze a cgroup so no new processes can be spawned while killing
    let use_v2 = use_v2(cgroup_path);
    trace!("Freeze cgroup: {cgroup_path:?}");
    if use_v2 {
        cgroup2::freeze(cgroup_path)?;
        cgroup2::wait_frozen(cgroup_path)?;
    } else {
        cgroup1::freeze(cgroup_path)?;
        cgroup1::wait_frozen(cgroup_path)?;
    }
    trace!("Kill cgroup: {cgroup_path:?}");
    kill_cgroup(cgroup_path, sig)?;
    trace!("Thaw cgroup: {cgroup_path:?}");
    if use_v2 {
        cgroup2::thaw(cgroup_path)
    } else {
        cgroup1::thaw(cgroup_path)
    }
}

pub fn remove_cgroup(cgroup_path: &std::path::Path) -> Result<(), CgroupError> {
    fs::remove_dir(cgroup_path).map_err(|e| CgroupError::IOErr(e, format!("{cgroup_path:?}")))
}

/// kill all processes that are currently in this cgroup.
/// You should use `wait_frozen` before or make in another way sure
/// there are no more processes spawned while killing
pub fn kill_cgroup(
    cgroup_path: &std::path::Path,
    sig: nix::sys::signal::Signal,
) -> Result<(), CgroupError> {
    // TODO figure out how to freeze a cgroup so no new processes can be spawned while killing
    let pids = get_all_procs(cgroup_path)?;
    for pid in &pids {
        nix::sys::signal::kill(*pid, sig).map_err(CgroupError::NixErr)?;
    }
    Ok(())
}

pub fn wait_frozen(cgroup_path: &std::path::Path) -> Result<(), CgroupError> {
    if use_v2(cgroup_path) {
        cgroup2::wait_frozen(cgroup_path)
    } else {
        cgroup1::wait_frozen(cgroup_path)
    }
}

pub fn freeze(cgroup_path: &std::path::Path) -> Result<(), CgroupError> {
    if use_v2(cgroup_path) {
        cgroup2::freeze(cgroup_path)
    } else {
        cgroup1::freeze(cgroup_path)
    }
}

pub fn thaw(cgroup_path: &std::path::Path) -> Result<(), CgroupError> {
    if use_v2(cgroup_path) {
        cgroup2::thaw(cgroup_path)
    } else {
        cgroup1::thaw(cgroup_path)
    }
}
