//! Handle signals sent to this process from either the outside or the child processes
//!
//! ## Deadlock prevention
//!
//! The signal handler receives a cloned `ArcMutPidTable` so it can update PID
//! table entries (e.g. `Service` → `ServiceExited`) **without** acquiring the
//! `RuntimeInfo` read lock.  This breaks a 3-way deadlock that otherwise occurs
//! when:
//!
//!   1. Activation threads hold read locks on `RuntimeInfo` while polling
//!      `wait_for_service` (checking the PID table for `ServiceExited`).
//!   2. A `systemctl` command (e.g. from a udev `RUN+=` rule) tries to acquire
//!      a write lock on `RuntimeInfo` — it blocks because readers hold locks,
//!      and on glibc's writer-preferring `pthread_rwlock` all *new* readers are
//!      also blocked.
//!   3. The old exit-handler thread needed a read lock to update the PID table
//!      — but was blocked by the pending writer from (2).
//!
//! Now the signal handler updates the PID table directly (step 3 no longer
//! needs the lock), so `wait_for_service` sees `ServiceExited` promptly,
//! releases its read lock, and the writer from (2) can proceed.
//!
//! ## SIGRTMIN+ signals
//!
//! Real systemd uses real-time signals for control when D-Bus is not available.
//! We handle the following SIGRTMIN+N offsets:
//!
//! | Offset | Action                                        |
//! |--------|-----------------------------------------------|
//! |  +0    | Switch to `default.target`                    |
//! |  +1    | Switch to `rescue.target`                     |
//! |  +2    | Switch to `emergency.target`                  |
//! |  +3    | Halt the system                               |
//! |  +4    | Poweroff the system                           |
//! |  +5    | Reboot the system                             |
//! |  +6    | Kexec reboot (treated as reboot)              |
//! | +13    | Daemon re-execution                           |
//! | +14    | Log status info                               |
//! | +15    | Set log level to debug                        |
//! | +16    | Restore log level to info                     |
//! | +20    | Enable show-status messages                   |
//! | +21    | Disable show-status messages                  |
//! | +22    | Set log level to debug (alt)                  |
//! | +23    | Set log level to info (alt)                   |
//! | +24    | Immediate exit (container mode)               |

use crate::lock_ext::MutexExt;
use crate::runtime_info::{ArcMutPidTable, ArcMutRuntimeInfo, PidEntry};
use crate::services;
use log::error;
use log::info;
use log::trace;
use log::warn;
use signal_hook::iterator::Signals;

/// Returns the set of SIGRTMIN+N offsets we want to handle.
pub const SIGRTMIN_OFFSETS: &[i32] = &[0, 1, 2, 3, 4, 5, 6, 13, 14, 15, 16, 20, 21, 22, 23, 24];

/// Compute the list of real-time signal numbers to register.
///
/// `libc::SIGRTMIN()` is a function (not a constant) because glibc reserves
/// some RT signals for its threading implementation.  We call it at runtime
/// and add our desired offsets.
pub fn sigrtmin_signals() -> Vec<libc::c_int> {
    let base = unsafe { libc::__libc_current_sigrtmin() };
    let max = unsafe { libc::__libc_current_sigrtmax() };
    SIGRTMIN_OFFSETS
        .iter()
        .map(|offset| base + offset)
        .filter(|&sig| sig <= max)
        .collect()
}

pub fn handle_signals(
    mut signals: Signals,
    run_info: ArcMutRuntimeInfo,
    pid_table: ArcMutPidTable,
) {
    let sigrtmin_base = unsafe { libc::__libc_current_sigrtmin() };

    loop {
        // Pick up new signals
        for signal in signals.forever() {
            match signal as libc::c_int {
                signal_hook::consts::SIGCHLD => {
                    std::iter::from_fn(get_next_exited_child)
                        .take_while(Result::is_ok)
                        .for_each(|val| {
                            match val {
                                Ok((pid, code)) => {
                                    // Phase 1: Update the PID table immediately,
                                    // WITHOUT acquiring the RuntimeInfo read lock.
                                    // This lets `wait_for_service` (which polls the
                                    // PID table under a RuntimeInfo read lock) see
                                    // the `ServiceExited` entry and proceed.
                                    let unit_id = {
                                        let mut pt = pid_table.lock_poisoned();
                                        match pt.get(&pid) {
                                            Some(PidEntry::Helper(_id, srvc_name)) => {
                                                trace!(
                                                    "Helper process for service: {srvc_name} exited with: {code:?}"
                                                );
                                                pt.insert(pid, PidEntry::HelperExited(code));
                                                None // no further handling needed
                                            }
                                            Some(PidEntry::Service(_id, _srvctype)) => {
                                                // Remove the Service entry and replace
                                                // it with ServiceExited so that
                                                // wait_for_service can observe it.
                                                let entry = pt.remove(&pid);
                                                let id = match entry {
                                                    Some(PidEntry::Service(id, _)) => id,
                                                    _ => unreachable!(),
                                                };
                                                trace!("Save service as exited. PID: {pid}");
                                                pt.insert(pid, PidEntry::ServiceExited(code));
                                                Some(id)
                                            }
                                            Some(
                                                PidEntry::HelperExited(_)
                                                | PidEntry::ServiceExited(_),
                                            ) => {
                                                error!(
                                                    "Pid {pid} exited but was already saved as exited"
                                                );
                                                None
                                            }
                                            None => {
                                                trace!(
                                                    "All processes spawned by rust-systemd have a pid entry. \
                                                     This did not: {pid}. Probably a rerooted orphan."
                                                );
                                                None
                                            }
                                        }
                                    };

                                    // Phase 2: If the exited process was a service,
                                    // spawn a thread to handle restart/cleanup logic.
                                    // That thread *will* need the RuntimeInfo read
                                    // lock, but by now the critical PID-table update
                                    // is already visible.
                                    if let Some(id) = unit_id {
                                        let run_info_clone = run_info.clone();
                                        services::service_exit_handler_new_thread(
                                            pid, id, code, run_info_clone,
                                        );
                                    }
                                }
                                Err(e) => {
                                    error!("{e}");
                                }
                            }
                        });
                }
                signal_hook::consts::SIGTERM
                | signal_hook::consts::SIGINT
                | signal_hook::consts::SIGQUIT => {
                    info!("Received termination signal. rust-systemd checking out");
                    crate::shutdown::shutdown_sequence(
                        run_info.clone(),
                        crate::shutdown::ShutdownAction::Poweroff,
                    );
                }

                sig => {
                    // Check if this is a SIGRTMIN+N signal
                    let offset = sig - sigrtmin_base;
                    handle_rtmin_signal(offset, &run_info);
                }
            }
        }
    }
}

/// Handle a SIGRTMIN+N signal by dispatching on the offset.
fn handle_rtmin_signal(offset: i32, run_info: &ArcMutRuntimeInfo) {
    match offset {
        // ── Target switching ────────────────────────────────────────────
        0 => {
            info!("SIGRTMIN+0: Switching to default.target");
            switch_target("default.target", run_info);
        }
        1 => {
            info!("SIGRTMIN+1: Switching to rescue.target");
            switch_target("rescue.target", run_info);
        }
        2 => {
            info!("SIGRTMIN+2: Switching to emergency.target");
            switch_target("emergency.target", run_info);
        }

        // ── Shutdown / reboot ───────────────────────────────────────────
        3 => {
            info!("SIGRTMIN+3: Halting system");
            crate::shutdown::shutdown_sequence(
                run_info.clone(),
                crate::shutdown::ShutdownAction::Halt,
            );
        }
        4 => {
            info!("SIGRTMIN+4: Powering off system");
            crate::shutdown::shutdown_sequence(
                run_info.clone(),
                crate::shutdown::ShutdownAction::Poweroff,
            );
        }
        5 => {
            info!("SIGRTMIN+5: Rebooting system");
            crate::shutdown::shutdown_sequence(
                run_info.clone(),
                crate::shutdown::ShutdownAction::Reboot,
            );
        }
        6 => {
            info!("SIGRTMIN+6: Kexec reboot");
            crate::shutdown::shutdown_sequence(
                run_info.clone(),
                crate::shutdown::ShutdownAction::Kexec,
            );
        }

        // ── Daemon re-execution ─────────────────────────────────────────
        13 => {
            info!("SIGRTMIN+13: Daemon re-execution requested");
            daemon_reexec(run_info);
        }

        // ── Log level / status display ──────────────────────────────────
        14 => {
            info!("SIGRTMIN+14: Logging status information");
            log_status_info(run_info);
        }
        15 => {
            info!("SIGRTMIN+15: Setting log level to debug");
            set_log_level(log::LevelFilter::Debug);
        }
        16 => {
            info!("SIGRTMIN+16: Restoring log level to info");
            set_log_level(log::LevelFilter::Info);
        }
        20 => {
            info!("SIGRTMIN+20: Enabling show-status messages");
            // Currently a no-op placeholder; could set a global flag.
        }
        21 => {
            info!("SIGRTMIN+21: Disabling show-status messages");
            // Currently a no-op placeholder.
        }
        22 => {
            info!("SIGRTMIN+22: Setting log level to debug (alt)");
            set_log_level(log::LevelFilter::Debug);
        }
        23 => {
            info!("SIGRTMIN+23: Setting log level to info (alt)");
            set_log_level(log::LevelFilter::Info);
        }

        // ── Immediate exit (container mode) ─────────────────────────────
        24 => {
            info!("SIGRTMIN+24: Immediate exit requested (container mode)");
            std::process::exit(0);
        }

        _ => {
            warn!("Received unhandled SIGRTMIN+{offset} signal");
        }
    }
}

/// Switch the running system to a different target.
///
/// This is analogous to `systemctl isolate <target>`, activating the specified
/// target and all of its dependencies.
fn switch_target(target_name: &str, run_info: &ArcMutRuntimeInfo) {
    use crate::lock_ext::RwLockExt;
    use crate::units::UnitId;

    let target_id: Option<UnitId> = {
        let ri = run_info.read_poisoned();
        // Try to find the target by name, then by alias.
        if let Some(unit) = ri.unit_table.values().find(|u| u.id.name == target_name) {
            Some(unit.id.clone())
        } else {
            ri.unit_table
                .values()
                .find(|u| u.common.unit.aliases.iter().any(|a| a == target_name))
                .map(|unit| unit.id.clone())
        }
    };

    match target_id {
        Some(id) => {
            let run_info_clone = run_info.clone();
            std::thread::spawn(move || {
                info!("Activating target: {}", id.name);
                let errs = crate::units::activate_needed_units(id, run_info_clone);
                if !errs.is_empty() {
                    for err in &errs {
                        error!("Error activating target: {err:?}");
                    }
                }
            });
        }
        None => {
            error!("Cannot switch to {target_name}: unit not found");
        }
    }
}

/// Perform daemon re-execution.
///
/// This re-execs the service manager binary while preserving currently running
/// service processes.  Since we set ourselves as subreaper, all child processes
/// remain our children after exec.
///
/// State serialization approach:
///   1. Write a list of (unit_name, pid) pairs to a state file.
///   2. Set an environment variable `SYSTEMD_RS_REEXEC=1` so the new instance
///      knows it was re-exec'd.
///   3. Call `execve()` on our own binary.
///
/// The new instance re-loads all unit files from disk.  Running service
/// processes continue unaffected — SIGCHLD for their eventual exit will be
/// delivered to the new PID 1 instance.
pub fn daemon_reexec(run_info: &ArcMutRuntimeInfo) {
    use crate::lock_ext::RwLockExt;

    let self_path = {
        let ri = run_info.read_poisoned();
        ri.config.self_path.clone()
    };

    // Serialize running-service state.
    let state_path = std::path::Path::new("/run/systemd/rust-systemd-reexec-state");
    match serialize_reexec_state(run_info, state_path) {
        Ok(()) => {
            info!("Serialized reexec state to {}", state_path.display());
        }
        Err(e) => {
            error!("Failed to serialize reexec state: {e}");
            // Continue with reexec anyway — we'll just lose PID tracking.
        }
    }

    // Set the reexec marker so the new instance knows.
    // SAFETY: We're about to exec, so the environment modification is fine.
    unsafe { std::env::set_var("SYSTEMD_RS_REEXEC", "1") };

    info!("Re-executing service manager: {}", self_path.display());

    // Build argv: use the same executable path with no extra arguments.
    // (The new instance will pick up the reexec environment variable.)
    let c_path = match std::ffi::CString::new(self_path.to_string_lossy().as_bytes()) {
        Ok(p) => p,
        Err(e) => {
            error!("Invalid executable path for reexec: {e}");
            return;
        }
    };

    let c_argv: Vec<std::ffi::CString> = vec![c_path.clone()];

    // Collect current environment.
    let c_envp: Vec<std::ffi::CString> = std::env::vars()
        .filter_map(|(k, v)| std::ffi::CString::new(format!("{k}={v}")).ok())
        .collect();

    // execve replaces the current process image.
    // If it returns, it failed.
    let err = nix::unistd::execve(&c_path, &c_argv, &c_envp);
    error!("execve failed during daemon-reexec: {err:?}");
}

/// Serialize the current running-service state to a file.
///
/// Format: one line per running service, `unit_name\tpid\n`.
/// This allows the re-exec'd instance to adopt running processes.
fn serialize_reexec_state(
    run_info: &ArcMutRuntimeInfo,
    path: &std::path::Path,
) -> Result<(), String> {
    use crate::lock_ext::{MutexExt, RwLockExt};
    use std::io::Write;

    let ri = run_info.read_poisoned();
    let pid_table = ri.pid_table.lock_poisoned();

    // Ensure the parent directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }

    let mut file =
        std::fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;

    for (pid, entry) in pid_table.iter() {
        if let PidEntry::Service(unit_id, srvc_type) = entry {
            writeln!(file, "{}\t{}\t{:?}", unit_id.name, pid.as_raw(), srvc_type)
                .map_err(|e| format!("write: {e}"))?;
        }
    }

    Ok(())
}

/// Check if this instance was started via daemon-reexec and, if so,
/// try to restore PID tracking for running services.
///
/// Returns `true` if reexec state was restored.
pub fn check_and_restore_reexec_state(run_info: &ArcMutRuntimeInfo) -> bool {
    use crate::lock_ext::{MutexExt, RwLockExt};

    // Check the environment flag.
    let is_reexec = std::env::var("SYSTEMD_RS_REEXEC").is_ok_and(|v| v == "1");
    if !is_reexec {
        return false;
    }

    // Clear the flag so further execs don't misinterpret it.
    // SAFETY: only called during early single-threaded startup.
    unsafe { std::env::remove_var("SYSTEMD_RS_REEXEC") };

    let state_path = std::path::Path::new("/run/systemd/rust-systemd-reexec-state");
    if !state_path.exists() {
        info!("Reexec detected but no state file found — starting fresh");
        return true;
    }

    let content = match std::fs::read_to_string(state_path) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to read reexec state: {e}");
            return true;
        }
    };

    let ri = run_info.read_poisoned();
    let mut pid_table = ri.pid_table.lock_poisoned();
    let mut restored = 0u32;

    for line in content.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            continue;
        }
        let unit_name = parts[0];
        let pid_raw: i32 = match parts[1].parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        let pid = nix::unistd::Pid::from_raw(pid_raw);

        // Verify the process is still alive.
        let alive = nix::sys::signal::kill(pid, None).is_ok();
        if !alive {
            trace!("Reexec: PID {pid_raw} for {unit_name} is no longer running, skipping");
            continue;
        }

        // Find the unit in the table and get its ServiceType.
        if let Some(unit) = ri.unit_table.values().find(|u| u.id.name == unit_name) {
            if let crate::units::Specific::Service(srvc) = &unit.specific {
                pid_table.insert(pid, PidEntry::Service(unit.id.clone(), srvc.conf.srcv_type));
                restored += 1;
                info!("Reexec: restored PID tracking for {unit_name} (PID {pid_raw})");
            }
        } else {
            trace!("Reexec: unit {unit_name} not found in table, skipping PID {pid_raw}");
        }
    }

    // Clean up the state file.
    let _ = std::fs::remove_file(state_path);

    info!("Reexec: restored {restored} running service(s)");
    true
}

/// Log a summary of the current system state.
fn log_status_info(run_info: &ArcMutRuntimeInfo) {
    use crate::lock_ext::RwLockExt;
    use crate::units::UnitStatus;

    let ri = run_info.read_poisoned();
    let total = ri.unit_table.len();
    let mut active = 0u32;
    let mut failed = 0u32;
    let mut inactive = 0u32;

    for unit in ri.unit_table.values() {
        let status = unit.common.status.read_poisoned();
        match &*status {
            UnitStatus::Started(_) | UnitStatus::Starting => active += 1,
            UnitStatus::Stopped(_, errors) if !errors.is_empty() => failed += 1,
            _ => inactive += 1,
        }
    }

    info!(
        "System status: {total} units loaded, {active} active, {failed} failed, {inactive} inactive"
    );
}

/// Dynamically change the log level filter.
fn set_log_level(level: log::LevelFilter) {
    log::set_max_level(level);
    info!("Log level changed to {level}");
}

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub enum ChildTermination {
    Signal(nix::sys::signal::Signal),
    Exit(i32),
}

impl std::fmt::Display for ChildTermination {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Signal(sig) => write!(f, "signal {sig}"),
            Self::Exit(code) => write!(f, "exit code {code}"),
        }
    }
}

impl ChildTermination {
    #[must_use]
    pub const fn success(&self) -> bool {
        match self {
            Self::Signal(_) => false,
            Self::Exit(code) => *code == 0,
        }
    }
}

type ChildIterElem = Result<(nix::unistd::Pid, ChildTermination), nix::Error>;

fn get_next_exited_child() -> Option<ChildIterElem> {
    let wait_any_pid = nix::unistd::Pid::from_raw(-1);
    let wait_flags = nix::sys::wait::WaitPidFlag::WNOHANG;
    match nix::sys::wait::waitpid(wait_any_pid, Some(wait_flags)) {
        Ok(exit_status) => match exit_status {
            nix::sys::wait::WaitStatus::Exited(pid, code) => {
                Some(Ok((pid, ChildTermination::Exit(code))))
            }
            nix::sys::wait::WaitStatus::Signaled(pid, signal, _dumped_core) => {
                // signals get handed to the parent if the child got killed by it but didnt handle the
                // signal itself
                // we dont care if the service dumped it's core
                Some(Ok((pid, ChildTermination::Signal(signal))))
            }
            nix::sys::wait::WaitStatus::StillAlive => {
                trace!("No more state changes to poll");
                None
            }
            _ => {
                trace!("Ignored child signal received with code: {exit_status:?}");
                // return next child, we dont care about other events like stop/continue of children
                get_next_exited_child()
            }
        },
        Err(e) => {
            if e == nix::Error::ECHILD {
            } else {
                trace!("Error while waiting: {e}");
            }
            Some(Err(e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── SIGRTMIN+ signal computation tests ──────────────────────────────

    #[test]
    fn test_sigrtmin_offsets_are_sorted() {
        for window in SIGRTMIN_OFFSETS.windows(2) {
            assert!(
                window[0] < window[1],
                "SIGRTMIN_OFFSETS must be sorted: {} >= {}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn test_sigrtmin_offsets_are_non_negative() {
        for &offset in SIGRTMIN_OFFSETS {
            assert!(offset >= 0, "SIGRTMIN offset must be >= 0, got {offset}");
        }
    }

    #[test]
    fn test_sigrtmin_signals_returns_valid_signal_numbers() {
        let signals = sigrtmin_signals();
        assert!(
            !signals.is_empty(),
            "sigrtmin_signals() should return at least one signal"
        );
        let base = unsafe { libc::__libc_current_sigrtmin() };
        let max = unsafe { libc::__libc_current_sigrtmax() };
        for &sig in &signals {
            assert!(sig >= base, "Signal {sig} is below SIGRTMIN ({base})");
            assert!(sig <= max, "Signal {sig} is above SIGRTMAX ({max})");
        }
    }

    #[test]
    fn test_sigrtmin_signals_count_matches_offsets() {
        let signals = sigrtmin_signals();
        let base = unsafe { libc::__libc_current_sigrtmin() };
        let max = unsafe { libc::__libc_current_sigrtmax() };
        let expected_count = SIGRTMIN_OFFSETS
            .iter()
            .filter(|&&offset| base + offset <= max)
            .count();
        assert_eq!(signals.len(), expected_count);
    }

    #[test]
    fn test_sigrtmin_signals_include_key_offsets() {
        // The most important offsets (0=default.target, 4=poweroff, 5=reboot,
        // 13=reexec, 24=immediate exit) should always be available since
        // Linux guarantees at least 32 RT signals.
        let signals = sigrtmin_signals();
        let base = unsafe { libc::__libc_current_sigrtmin() };
        for key_offset in [0, 4, 5, 13, 24] {
            let expected_sig = base + key_offset;
            assert!(
                signals.contains(&expected_sig),
                "Expected SIGRTMIN+{key_offset} ({expected_sig}) in signal list"
            );
        }
    }

    // ── ChildTermination tests ──────────────────────────────────────────

    #[test]
    fn test_child_termination_exit_success() {
        let term = ChildTermination::Exit(0);
        assert!(term.success());
        assert_eq!(format!("{term}"), "exit code 0");
    }

    #[test]
    fn test_child_termination_exit_failure() {
        let term = ChildTermination::Exit(1);
        assert!(!term.success());
        assert_eq!(format!("{term}"), "exit code 1");
    }

    #[test]
    fn test_child_termination_signal() {
        let term = ChildTermination::Signal(nix::sys::signal::Signal::SIGTERM);
        assert!(!term.success());
        let display = format!("{term}");
        assert!(
            display.contains("signal"),
            "Display should contain 'signal': {display}"
        );
    }

    #[test]
    fn test_child_termination_clone_eq() {
        let term1 = ChildTermination::Exit(42);
        let term2 = term1;
        assert_eq!(term1, term2);

        let term3 = ChildTermination::Signal(nix::sys::signal::Signal::SIGKILL);
        let term4 = term3;
        assert_eq!(term3, term4);
        assert_ne!(term1, term3);
    }

    // ── Reexec state serialization tests ────────────────────────────────

    #[test]
    fn test_serialize_reexec_state_creates_file() {
        use std::sync::{Arc, Mutex, RwLock};

        let pid_table = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let run_info = Arc::new(RwLock::new(crate::runtime_info::RuntimeInfo {
            unit_table: std::collections::HashMap::new(),
            pid_table: pid_table.clone(),
            fd_store: std::sync::RwLock::new(crate::fd_store::FDStore::default()),
            config: crate::config::Config {
                unit_dirs: vec![],
                target_unit: "default.target".to_string(),
                notification_sockets_dir: std::path::PathBuf::from("/tmp"),
                self_path: std::path::PathBuf::from("/bin/rust-systemd"),
            },
            stdout_eventfd: crate::platform::make_event_fd().unwrap(),
            stderr_eventfd: crate::platform::make_event_fd().unwrap(),
            notification_eventfd: crate::platform::make_event_fd().unwrap(),
            socket_activation_eventfd: crate::platform::make_event_fd().unwrap(),
            pending_activations: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashSet::new(),
            )),
            manager_environment: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
        }));

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        // Remove the file so serialize_reexec_state creates it fresh
        drop(tmp);

        let result = serialize_reexec_state(&run_info, &path);
        assert!(result.is_ok(), "serialize_reexec_state failed: {result:?}");

        // With an empty PID table, the file should exist but be empty
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.is_empty(),
            "Expected empty state file for empty PID table"
        );

        // Clean up
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_serialize_reexec_state_with_entries() {
        use std::sync::{Arc, Mutex, RwLock};

        let pid_table = Arc::new(Mutex::new(std::collections::HashMap::new()));

        // Insert a fake PID entry
        {
            let mut pt = pid_table.lock().unwrap();
            let unit_id = crate::units::UnitId {
                kind: crate::units::UnitIdKind::Service,
                name: "test.service".to_string(),
            };
            pt.insert(
                nix::unistd::Pid::from_raw(12345),
                PidEntry::Service(unit_id, crate::units::ServiceType::Simple),
            );
        }

        let run_info = Arc::new(RwLock::new(crate::runtime_info::RuntimeInfo {
            unit_table: std::collections::HashMap::new(),
            pid_table: pid_table.clone(),
            fd_store: std::sync::RwLock::new(crate::fd_store::FDStore::default()),
            config: crate::config::Config {
                unit_dirs: vec![],
                target_unit: "default.target".to_string(),
                notification_sockets_dir: std::path::PathBuf::from("/tmp"),
                self_path: std::path::PathBuf::from("/bin/rust-systemd"),
            },
            stdout_eventfd: crate::platform::make_event_fd().unwrap(),
            stderr_eventfd: crate::platform::make_event_fd().unwrap(),
            notification_eventfd: crate::platform::make_event_fd().unwrap(),
            socket_activation_eventfd: crate::platform::make_event_fd().unwrap(),
            pending_activations: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashSet::new(),
            )),
            manager_environment: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
        }));

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        let result = serialize_reexec_state(&run_info, &path);
        assert!(result.is_ok(), "serialize_reexec_state failed: {result:?}");

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("test.service"),
            "State file should contain unit name: {content}"
        );
        assert!(
            content.contains("12345"),
            "State file should contain PID: {content}"
        );

        // Clean up
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_serialize_reexec_state_skips_non_service_entries() {
        use std::sync::{Arc, Mutex, RwLock};

        let pid_table = Arc::new(Mutex::new(std::collections::HashMap::new()));

        // Insert a helper entry (should not be serialized)
        {
            let mut pt = pid_table.lock().unwrap();
            let unit_id = crate::units::UnitId {
                kind: crate::units::UnitIdKind::Service,
                name: "helper-test.service".to_string(),
            };
            pt.insert(
                nix::unistd::Pid::from_raw(99999),
                PidEntry::Helper(unit_id, "prestart".to_string()),
            );
            // Also insert a ServiceExited entry
            pt.insert(
                nix::unistd::Pid::from_raw(88888),
                PidEntry::ServiceExited(ChildTermination::Exit(0)),
            );
        }

        let run_info = Arc::new(RwLock::new(crate::runtime_info::RuntimeInfo {
            unit_table: std::collections::HashMap::new(),
            pid_table: pid_table.clone(),
            fd_store: std::sync::RwLock::new(crate::fd_store::FDStore::default()),
            config: crate::config::Config {
                unit_dirs: vec![],
                target_unit: "default.target".to_string(),
                notification_sockets_dir: std::path::PathBuf::from("/tmp"),
                self_path: std::path::PathBuf::from("/bin/rust-systemd"),
            },
            stdout_eventfd: crate::platform::make_event_fd().unwrap(),
            stderr_eventfd: crate::platform::make_event_fd().unwrap(),
            notification_eventfd: crate::platform::make_event_fd().unwrap(),
            socket_activation_eventfd: crate::platform::make_event_fd().unwrap(),
            pending_activations: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashSet::new(),
            )),
            manager_environment: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
        }));

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        let result = serialize_reexec_state(&run_info, &path);
        assert!(result.is_ok());

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.is_empty(),
            "Only Service entries should be serialized, got: {content}"
        );

        let _ = std::fs::remove_file(&path);
    }

    // ── set_log_level test ──────────────────────────────────────────────

    #[test]
    fn test_set_log_level_changes_max_level() {
        // Save original level
        let original = log::max_level();

        set_log_level(log::LevelFilter::Debug);
        assert_eq!(log::max_level(), log::LevelFilter::Debug);

        set_log_level(log::LevelFilter::Info);
        assert_eq!(log::max_level(), log::LevelFilter::Info);

        // Restore
        log::set_max_level(original);
    }
}
