#![allow(dead_code)]

use super::CgroupError;
use std::fs;
use std::io::Read;
use std::io::Write;
use std::path::Path;

/// move a process into the cgroup. In rust-systemd the child process will call `move_self` for convenience
pub fn move_pid_to_cgroup(
    cgroup_path: &std::path::Path,
    pid: nix::unistd::Pid,
) -> Result<(), CgroupError> {
    let cgroup_procs = cgroup_path.join("cgroup.procs");

    let mut f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&cgroup_procs)
        .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_procs:?}")))?;

    let pid_str = pid.as_raw().to_string();
    f.write(pid_str.as_bytes())
        .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_procs:?}")))?;
    Ok(())
}

/// move this process into the cgroup. Used by rust-systemd after forking
pub fn move_self_to_cgroup(cgroup_path: &std::path::Path) -> Result<(), CgroupError> {
    let pid = nix::unistd::getpid();
    move_pid_to_cgroup(cgroup_path, pid)
}

/// retrieve all controllers that are currently in this cgroup
#[allow(dead_code)]
pub fn get_available_controllers(
    cgroup_path: &std::path::Path,
) -> Result<Vec<String>, CgroupError> {
    let cgroup_ctrls = cgroup_path.join("cgroup.controllers");
    let mut f = fs::File::open(&cgroup_ctrls)
        .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_ctrls:?}")))?;
    let mut buf = String::new();
    f.read_to_string(&mut buf)
        .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_ctrls:?}")))?;

    Ok(buf
        .split('\n')
        .map(std::string::ToString::to_string)
        .collect())
}

/// enable controllers for child-cgroups
#[allow(dead_code)]
pub fn enable_controllers(
    cgroup_path: &std::path::Path,
    controllers: &[String],
) -> Result<(), CgroupError> {
    let cgroup_subtreectl = cgroup_path.join("cgroup.subtree_control");
    let mut f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&cgroup_subtreectl)
        .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_subtreectl:?}")))?;

    let mut buf = String::new();
    for ctl in controllers {
        buf.push_str(" +");
        buf.push_str(ctl);
    }
    f.write_all(buf.as_bytes())
        .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_subtreectl:?}")))?;
    Ok(())
}

/// disable controllers for child-cgroups
#[allow(dead_code)]
pub fn disable_controllers(
    cgroup_path: &std::path::Path,
    controllers: &[String],
) -> Result<(), CgroupError> {
    let cgroup_subtreectl = cgroup_path.join("cgroup.subtree_control");
    let mut f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&cgroup_subtreectl)
        .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_subtreectl:?}")))?;

    let mut buf = String::new();
    for ctl in controllers {
        buf.push_str(" -");
        buf.push_str(ctl);
    }
    f.write_all(buf.as_bytes())
        .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_subtreectl:?}")))?;
    Ok(())
}

fn write_freeze_state(
    cgroup_path: &std::path::Path,
    desired_state: &str,
) -> Result<(), CgroupError> {
    let cgroup_freeze = cgroup_path.join("cgroup.freeze");
    if !cgroup_freeze.exists() {
        return Err(CgroupError::IOErr(
            std::io::Error::from(std::io::ErrorKind::NotFound),
            format!("{cgroup_freeze:?}"),
        ));
    }

    let mut f = fs::OpenOptions::new()
        .read(false)
        .write(true)
        .open(&cgroup_freeze)
        .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_freeze:?}")))?;

    f.write_all(desired_state.as_bytes())
        .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_freeze:?}")))?;
    Ok(())
}

pub fn wait_frozen(cgroup_path: &std::path::Path) -> Result<(), CgroupError> {
    let cgroup_freeze = cgroup_path.join("cgroup.freeze");
    let mut f = fs::OpenOptions::new()
        .read(true)
        .write(false)
        .open(&cgroup_freeze)
        .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_freeze:?}")))?;
    loop {
        freeze(cgroup_path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)
            .map_err(|e| CgroupError::IOErr(e, format!("{cgroup_freeze:?}")))?;
        if buf[0] == b'1' {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Ok(())
}

pub fn freeze(cgroup_path: &std::path::Path) -> Result<(), CgroupError> {
    let desired_state = "1";
    write_freeze_state(cgroup_path, desired_state)
}

pub fn thaw(cgroup_path: &std::path::Path) -> Result<(), CgroupError> {
    let desired_state = "0";
    write_freeze_state(cgroup_path, desired_state)
}

// ── Resource-control enforcement ───────────────────────────────────────────
//
// These functions write resource limits to the cgroup v2 filesystem.
// Each function writes to the appropriate controller file under the unit's
// cgroup directory. Errors are returned so callers can log and continue
// (a missing controller file is not fatal — the controller may not be
// enabled on this system).
//
// Note: these functions are called from `fork_os_specific.rs` behind
// `#[cfg(feature = "cgroups")]`. When compiled without that feature they
// appear unused, so we allow dead_code for the whole block.

/// Helper: write a value to a cgroup control file, creating nothing.
/// Returns Ok(()) if the file was written, or Err if the file doesn't exist
/// or can't be written.
#[allow(dead_code)]
fn write_cgroup_file(cgroup_path: &Path, filename: &str, value: &str) -> Result<(), CgroupError> {
    let path = cgroup_path.join(filename);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .map_err(|e| CgroupError::IOErr(e, format!("{path:?}")))?;
    f.write_all(value.as_bytes())
        .map_err(|e| CgroupError::IOErr(e, format!("{path:?}")))?;
    Ok(())
}

/// Resolve a `MemoryLimit` to a string suitable for writing to a cgroup
/// memory controller file. Percentages are resolved against the system's
/// total physical memory (from `/proc/meminfo`). Returns "max" for Infinity.
#[allow(dead_code)]
fn resolve_memory_limit(limit: &crate::units::MemoryLimit) -> String {
    use crate::units::MemoryLimit;
    match limit {
        MemoryLimit::Bytes(b) => b.to_string(),
        MemoryLimit::Percent(pct) => {
            let total = read_total_memory_bytes().unwrap_or(0);
            if total == 0 {
                // Can't resolve percentage — treat as unlimited
                "max".to_owned()
            } else {
                (total * pct / 100).to_string()
            }
        }
        MemoryLimit::Infinity => "max".to_owned(),
    }
}

/// Read total physical memory in bytes from `/proc/meminfo`.
#[allow(dead_code)]
fn read_total_memory_bytes() -> Option<u64> {
    let contents = fs::read_to_string("/proc/meminfo").ok()?;
    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let rest = rest.trim();
            // Value is in kB (kibibytes)
            let kb_str = rest.strip_suffix("kB").unwrap_or(rest).trim();
            let kb: u64 = kb_str.parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

/// Set `memory.min` — minimum memory guarantee (protection from reclaim).
#[allow(dead_code)]
pub fn set_memory_min(
    cgroup_path: &Path,
    limit: &crate::units::MemoryLimit,
) -> Result<(), CgroupError> {
    let value = resolve_memory_limit(limit);
    write_cgroup_file(cgroup_path, "memory.min", &value)
}

/// Set `memory.low` — best-effort memory protection (avoid reclaim below this).
#[allow(dead_code)]
pub fn set_memory_low(
    cgroup_path: &Path,
    limit: &crate::units::MemoryLimit,
) -> Result<(), CgroupError> {
    let value = resolve_memory_limit(limit);
    write_cgroup_file(cgroup_path, "memory.low", &value)
}

/// Set `memory.high` — memory throttling boundary (processes are slowed).
#[allow(dead_code)]
pub fn set_memory_high(
    cgroup_path: &Path,
    limit: &crate::units::MemoryLimit,
) -> Result<(), CgroupError> {
    let value = resolve_memory_limit(limit);
    write_cgroup_file(cgroup_path, "memory.high", &value)
}

/// Set `memory.max` — hard memory limit (OOM killer invoked above this).
#[allow(dead_code)]
pub fn set_memory_max(
    cgroup_path: &Path,
    limit: &crate::units::MemoryLimit,
) -> Result<(), CgroupError> {
    let value = resolve_memory_limit(limit);
    write_cgroup_file(cgroup_path, "memory.max", &value)
}

/// Set `memory.swap.max` — hard swap limit.
#[allow(dead_code)]
pub fn set_memory_swap_max(
    cgroup_path: &Path,
    limit: &crate::units::MemoryLimit,
) -> Result<(), CgroupError> {
    let value = resolve_memory_limit(limit);
    write_cgroup_file(cgroup_path, "memory.swap.max", &value)
}

/// Set `cpu.weight` — CPU scheduling weight (1–10000, default 100).
#[allow(dead_code)]
pub fn set_cpu_weight(cgroup_path: &Path, weight: u64) -> Result<(), CgroupError> {
    write_cgroup_file(cgroup_path, "cpu.weight", &weight.to_string())
}

/// Set `cpu.max` — CPU bandwidth limit from a CPUQuota= percentage.
/// The format is "$MAX $PERIOD" where period defaults to 100000 (100ms)
/// and max = period * quota / 100.
#[allow(dead_code)]
pub fn set_cpu_quota(cgroup_path: &Path, quota_percent: u64) -> Result<(), CgroupError> {
    let period: u64 = 100_000; // 100ms in microseconds, matching systemd default
    let max = period * quota_percent / 100;
    let value = format!("{max} {period}");
    write_cgroup_file(cgroup_path, "cpu.max", &value)
}

/// Set `io.weight` — default I/O scheduling weight (1–10000, default 100).
/// Format: "default WEIGHT"
#[allow(dead_code)]
pub fn set_io_weight(cgroup_path: &Path, weight: u64) -> Result<(), CgroupError> {
    write_cgroup_file(cgroup_path, "io.weight", &format!("default {weight}"))
}

/// Set a per-device I/O weight in `io.weight`.
/// Format: "MAJOR:MINOR WEIGHT"
#[allow(dead_code)]
pub fn set_io_device_weight(
    cgroup_path: &Path,
    device: &str,
    weight: u64,
) -> Result<(), CgroupError> {
    if let Some(dev_id) = resolve_device_major_minor(device) {
        write_cgroup_file(cgroup_path, "io.weight", &format!("{dev_id} {weight}"))
    } else {
        Err(CgroupError::IOErr(
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("cannot resolve device: {device}"),
            ),
            device.to_owned(),
        ))
    }
}

/// Set per-device I/O bandwidth or IOPS limits in `io.max`.
/// Format: "MAJOR:MINOR rbps=VAL wbps=VAL riops=VAL wiops=VAL"
/// Each field is optional; pass 0 to skip (will be written as "max").
#[allow(dead_code)]
pub fn set_io_max(
    cgroup_path: &Path,
    device: &str,
    rbps: Option<u64>,
    wbps: Option<u64>,
    riops: Option<u64>,
    wiops: Option<u64>,
) -> Result<(), CgroupError> {
    if let Some(dev_id) = resolve_device_major_minor(device) {
        let rbps_s = rbps.map_or("max".to_owned(), |v| v.to_string());
        let wbps_s = wbps.map_or("max".to_owned(), |v| v.to_string());
        let riops_s = riops.map_or("max".to_owned(), |v| v.to_string());
        let wiops_s = wiops.map_or("max".to_owned(), |v| v.to_string());
        let value = format!("{dev_id} rbps={rbps_s} wbps={wbps_s} riops={riops_s} wiops={wiops_s}");
        write_cgroup_file(cgroup_path, "io.max", &value)
    } else {
        Err(CgroupError::IOErr(
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("cannot resolve device: {device}"),
            ),
            device.to_owned(),
        ))
    }
}

/// Enable cgroup controllers on the parent cgroup's `cgroup.subtree_control`
/// so the child cgroup can use them. This is idempotent — already-enabled
/// controllers are silently accepted.
#[allow(dead_code)]
pub fn enable_controllers_on_parent(
    cgroup_path: &Path,
    controllers: &[&str],
) -> Result<(), CgroupError> {
    // In cgroups v2, controllers must be enabled at every level from the root
    // down to the target cgroup.  Walk up the tree to collect all ancestor
    // cgroup.subtree_control files, then enable controllers top-down so that
    // each parent has the controller available before we enable it on its child.
    let mut ancestors = Vec::new();
    let mut current = cgroup_path.to_path_buf();
    while let Some(parent) = current.parent() {
        let subtree_ctl = parent.join("cgroup.subtree_control");
        if subtree_ctl.exists() {
            ancestors.push(subtree_ctl);
        } else {
            // Left the cgroup filesystem
            break;
        }
        current = parent.to_path_buf();
    }
    // Enable controllers top-down (root first)
    ancestors.reverse();
    for subtree_ctl in &ancestors {
        for ctl in controllers {
            let _ = fs::write(subtree_ctl, format!("+{ctl}"));
        }
    }
    Ok(())
}

/// Resolve a device node path (e.g. "/dev/sda") to "MAJOR:MINOR" string.
#[allow(dead_code)]
fn resolve_device_major_minor(device: &str) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(device).ok()?;
    let rdev = meta.rdev();
    // major = rdev >> 8, minor = rdev & 0xff (for Linux)
    let major = libc::major(rdev);
    let minor = libc::minor(rdev);
    Some(format!("{major}:{minor}"))
}
