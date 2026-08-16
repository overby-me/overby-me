use log::error;
use log::info;
use log::trace;
use log::warn;

use crate::lock_ext::RwLockExt;
use crate::runtime_info::{ArcMutRuntimeInfo, RuntimeInfo, UnitTable};
use crate::units::{Specific, StatusStopped, UnitId, UnitStatus};

/// The final system action to perform after all units have been stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownAction {
    Poweroff,
    Reboot,
    Halt,
    Kexec,
}

impl ShutdownAction {
    pub fn from_verb(verb: &str) -> Option<Self> {
        match verb {
            "poweroff" => Some(ShutdownAction::Poweroff),
            "reboot" => Some(ShutdownAction::Reboot),
            "halt" => Some(ShutdownAction::Halt),
            "kexec" => Some(ShutdownAction::Kexec),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ShutdownAction::Poweroff => "poweroff",
            ShutdownAction::Reboot => "reboot",
            ShutdownAction::Halt => "halt",
            ShutdownAction::Kexec => "kexec",
        }
    }
}

/// Find the `systemd-shutdown` binary relative to our own executable,
/// falling back to well-known system paths.
fn find_shutdown_binary() -> Option<std::path::PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        // Check sibling directory (same dir as PID 1)
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("systemd-shutdown");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        // Walk up to find bin/systemd-shutdown or lib/systemd/systemd-shutdown
        let mut dir = exe.parent();
        for _ in 0..5 {
            let Some(d) = dir else { break };
            for subpath in &["bin/systemd-shutdown", "lib/systemd/systemd-shutdown"] {
                let candidate = d.join(subpath);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            dir = d.parent();
        }
    }

    // Fallback to system paths
    for path in &[
        "/usr/lib/systemd/systemd-shutdown",
        "/lib/systemd/systemd-shutdown",
        "/usr/bin/systemd-shutdown",
    ] {
        let p = std::path::Path::new(path);
        if p.is_file() {
            return Some(p.to_path_buf());
        }
    }

    None
}

fn get_next_service_to_shutdown(unit_table: &UnitTable) -> Option<UnitId> {
    for unit in unit_table.values() {
        let status = &unit.common.status;
        {
            let status_locked = status.read_poisoned();
            if !(*status_locked).is_started() {
                continue;
            }
        }

        // Skip root and API/virtual filesystem mounts during the unit
        // shutdown phase.  These are still needed while we stop services
        // and will be handled later by systemd-shutdown.
        if let Specific::Mount(specific) = &unit.specific {
            let mp = specific.conf.where_.as_str();
            if mp == "/"
                || mp == "/proc"
                || mp == "/sys"
                || mp == "/dev"
                || mp == "/run"
                || mp.starts_with("/sys/")
                || mp.starts_with("/dev/")
                || mp.starts_with("/proc/")
                || mp.starts_with("/nix/store")
            {
                continue;
            }
        }

        let kill_before = unit
            .common
            .dependencies
            .before
            .iter()
            .filter(|&next_id| {
                // A `before` dependency that is no longer in the unit table has
                // already been removed (stopped), so it cannot hold up this
                // unit's shutdown. Unwrapping here panicked the shutdown thread,
                // which is what stalled the reboot (09-reboot): PID1 died before
                // reaching the systemd-shutdown handoff.
                let Some(dep) = unit_table.get(next_id) else {
                    return false;
                };
                dep.common.status.read_poisoned().is_started()
            })
            .cloned()
            .collect::<Vec<_>>();
        if kill_before.is_empty() {
            trace!("Chose unit: {}", unit.id.name);
            return Some(unit.id.clone());
        }
        trace!(
            "Dont kill service {} yet. These Units depend on it: {:?}",
            unit.id.name, kill_before
        );
    }
    None
}

fn shutdown_unit(shutdown_id: &UnitId, run_info: &RuntimeInfo) {
    let unit = run_info.unit_table.get(shutdown_id).unwrap();
    {
        trace!("Set unit status: {}", unit.id.name);
        let mut status_locked = unit.common.status.write_poisoned();
        *status_locked = UnitStatus::Stopping;
    }
    match &unit.specific {
        Specific::Service(specific) => {
            let mut_state = &mut *specific.state.write_poisoned();
            let kill_res =
                mut_state
                    .srvc
                    .kill(&specific.conf, unit.id.clone(), &unit.id.name, run_info);
            match kill_res {
                Ok(()) => {
                    trace!("Killed service unit: {}", unit.id.name);
                }
                // ExecStop failures are common during shutdown (missing
                // binaries, unsupported flags, etc.) — log as warnings
                // rather than errors to reduce noise.
                Err(e) => warn!("Service {} stop: {e}", unit.id.name),
            }
            if let Some(datagram) = &mut_state.srvc.notifications {
                match datagram.shutdown(std::net::Shutdown::Both) {
                    Ok(()) => {
                        trace!(
                            "Closed notification socket for service unit: {}",
                            unit.id.name
                        );
                    }
                    Err(e) => trace!(
                        "Error closing notification socket for service unit {}: {}",
                        unit.id.name, e
                    ),
                }
            }
            mut_state.srvc.notifications = None;

            if let Some(note_sock_path) = &mut_state.srvc.notifications_path
                && note_sock_path.exists()
            {
                match std::fs::remove_file(note_sock_path) {
                    Ok(()) => {
                        trace!(
                            "Removed notification socket for service unit: {}",
                            unit.id.name
                        );
                    }
                    Err(e) => trace!(
                        "Error removing notification socket for service unit {}: {}",
                        unit.id.name, e
                    ),
                }
            }
        }
        Specific::Socket(specific) => {
            let mut_state = &mut *specific.state.write_poisoned();
            trace!("Close socket unit: {}", unit.id.name);
            if let Err(e) = mut_state.sock.close_all(
                &specific.conf,
                unit.id.name.clone(),
                &mut run_info.fd_store.write_poisoned(),
            ) {
                error!("Error while closing sockets: {e}")
            }
            trace!("Closed socket unit: {}", unit.id.name);
        }
        Specific::Target(_)
        | Specific::Slice(_)
        | Specific::Timer(_)
        | Specific::Path(_)
        | Specific::Device(_) => {
            // Nothing to do
        }
        Specific::Swap(specific) => {
            // Deactivate swap during shutdown
            trace!("Deactivating swap unit: {}", unit.id.name);
            let conf = &specific.conf;
            #[cfg(target_os = "linux")]
            {
                if let Ok(cstr) = std::ffi::CString::new(conf.what.as_str()) {
                    // SAFETY: swapoff(2) is a standard Linux syscall. The path
                    // is a valid NUL-terminated C string.
                    let ret = unsafe { libc::swapoff(cstr.as_ptr()) };
                    if ret == 0 {
                        trace!("Deactivated swap unit: {}", unit.id.name);
                    } else {
                        let errno = std::io::Error::last_os_error();
                        error!(
                            "Failed to deactivate swap {} ({}): {}",
                            conf.what, unit.id.name, errno
                        );
                    }
                } else {
                    error!("Invalid swap path for unit {}: {}", unit.id.name, conf.what);
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = conf;
                trace!(
                    "Swap deactivation is a no-op on non-Linux ({})",
                    unit.id.name
                );
            }
        }
        Specific::Mount(specific) => {
            // Unmount the filesystem during shutdown
            trace!("Unmounting mount unit: {}", unit.id.name);
            let conf = &specific.conf;
            let mut umount_flags = nix::mount::MntFlags::empty();
            if conf.lazy_unmount {
                umount_flags |= nix::mount::MntFlags::MNT_DETACH;
            }
            if conf.force_unmount {
                umount_flags |= nix::mount::MntFlags::MNT_FORCE;
            }
            match nix::mount::umount2(conf.where_.as_str(), umount_flags) {
                Ok(()) => {
                    trace!("Unmounted mount unit: {}", unit.id.name);
                }
                Err(e) => {
                    // Unmount failures during shutdown are common (busy
                    // filesystems, etc.) — systemd-shutdown will retry.
                    warn!(
                        "Failed to unmount {} ({}): {} (will retry in systemd-shutdown)",
                        conf.where_, unit.id.name, e
                    );
                }
            }
        }
    }
    {
        trace!("Set unit status: {}", unit.id.name);
        let mut status_locked = unit.common.status.write_poisoned();
        *status_locked = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
    }
}

static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Returns `true` once a shutdown sequence has been initiated.
///
/// Background threads (socket activation, notification handler, etc.) should
/// check this flag and exit gracefully when it becomes `true`.
pub fn is_shutting_down() -> bool {
    SHUTTING_DOWN.load(std::sync::atomic::Ordering::SeqCst)
}

pub fn shutdown_sequence(run_info: ArcMutRuntimeInfo, action: ShutdownAction) {
    if SHUTTING_DOWN
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_err()
    {
        // is already shutting down. Exit the process.
        warn!("Got a second termination signal. Exiting potentially dirty");
        std::process::exit(0);
    }

    std::thread::spawn(move || {
        trace!("Shutting down");

        // Wake up all handler threads (socket activation, notification,
        // stdout/stderr) so they re-check the SHUTTING_DOWN flag and exit
        // instead of blocking forever in select().
        if let Ok(ri) = run_info.read() {
            ri.notify_eventfds();
        }

        // Re-acquire the RuntimeInfo read lock per iteration instead of
        // holding one guard across the whole kill loop: shutdown_unit blocks
        // on ExecStop/kill waits, and a single guard held for the entire
        // sequence starves any writer (and with it, via the writer-preferring
        // rwlock, every other reader) for the whole shutdown.
        trace!("Kill all units");
        loop {
            let ri = match run_info.read() {
                Ok(r) => r,
                Err(e) => e.into_inner(),
            };
            let Some(id) = get_next_service_to_shutdown(&ri.unit_table) else {
                break;
            };
            shutdown_unit(&id, &ri);
        }
        trace!("Killed all units");

        let run_info_lock = match run_info.read() {
            Ok(r) => r,
            Err(e) => e.into_inner(),
        };
        let run_info_locked = &*run_info_lock;

        let control_socket = run_info_locked
            .config
            .notification_sockets_dir
            .join("control.socket");
        if control_socket.exists() {
            match std::fs::remove_file(control_socket) {
                Ok(()) => {
                    trace!("Removed control socket");
                }
                Err(e) => error!("Error removing control socket: {e}"),
            }
        }

        #[cfg(feature = "cgroups")]
        {
            let _ = crate::platform::cgroups::move_out_of_own_cgroup(&std::path::PathBuf::from(
                "/sys/fs/cgroup/unified",
            ))
            .map_err(|e| error!("Error while cleaning up cgroups: {}", e));
        }

        info!(
            "Shutdown finished, performing final action: {}",
            action.as_str()
        );

        // Execute the systemd-shutdown binary which handles process killing,
        // filesystem unmounting, and the final reboot(2) syscall.
        if let Some(shutdown_bin) = find_shutdown_binary() {
            info!("Executing {} {}", shutdown_bin.display(), action.as_str());
            match std::process::Command::new(&shutdown_bin)
                .arg(action.as_str())
                .status()
            {
                Ok(status) => {
                    if !status.success() {
                        error!(
                            "systemd-shutdown exited with status: {}",
                            status.code().unwrap_or(-1)
                        );
                    }
                }
                Err(e) => {
                    error!("Failed to execute systemd-shutdown: {e}");
                }
            }
        } else {
            warn!("systemd-shutdown binary not found, calling reboot(2) directly");
        }

        // Fallback: call reboot(2) directly if systemd-shutdown failed or
        // was not found.
        #[cfg(target_os = "linux")]
        {
            info!(
                "Performing direct reboot(2) syscall for {}",
                action.as_str()
            );
            unsafe {
                libc::sync();
            }
            let cmd = match action {
                ShutdownAction::Poweroff => libc::RB_POWER_OFF,
                ShutdownAction::Reboot => libc::RB_AUTOBOOT,
                ShutdownAction::Halt => libc::RB_HALT_SYSTEM,
                ShutdownAction::Kexec => 0x45584543u32 as libc::c_int, // LINUX_REBOOT_CMD_KEXEC
            };
            unsafe {
                libc::reboot(cmd);
            }
        }

        // If we somehow get here (non-Linux or reboot failed), exit.
        error!("All shutdown methods failed, exiting");
        std::process::exit(1);
    });
}
