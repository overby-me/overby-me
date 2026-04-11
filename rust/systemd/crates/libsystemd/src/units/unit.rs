use log::{error, info, trace};

use crate::lock_ext::{MutexExt, RwLockExt};
use crate::runtime_info::{PidEntry, RuntimeInfo};
use crate::services::Service;
use crate::sockets::{Socket, SocketKind, SpecializedSocketConfig};
use crate::units::{
    ActivationSource, BindIPv6Only, Commandline, CpuQuota, CpuWeight, DeferTrigger, Delegate,
    DevicePolicy, EnvVars, ExitType, FileDescriptorStorePreserve, IOSchedulingClass, IoDeviceLimit,
    IoWeight, KeyringMode, KillMode, MemoryLimit, MemoryPressureWatch, NotifyKind, OOMPolicy,
    OnFailureJobMode, ParsedMountSection, ParsedSliceSection, ParsedSwapSection, ProcSubset,
    ProtectHome, ProtectProc, ProtectSystem, ResourceLimit, RestartMode, RestrictNamespaces,
    RuntimeDirectoryPreserve, ServiceRestart, ServiceType, StandardInput, StatusStarted,
    StatusStopped, StdIoOption, TasksMax, Timeout, TimeoutFailureMode, Timestamping, UnitAction,
    UnitCondition, UnitId, UnitIdKind, UnitOperationError, UnitOperationErrorReason, UnitStatus,
    UtmpMode, acquire_locks,
};

use std::path::PathBuf;
use std::sync::RwLock;

/// A units has a common part that all units share, like dependencies and a description. The specific part containbs mutable state and
/// the unit-type specific configs
pub struct Unit {
    pub id: UnitId,
    pub common: Common,
    pub specific: Specific,
}

/// Common attributes of units
pub struct Common {
    pub unit: UnitConfig,
    pub dependencies: Dependencies,
    pub status: RwLock<UnitStatus>,
    /// Lifecycle timestamps (microseconds since epoch) matching systemd's
    /// InactiveExitTimestamp, ActiveEnterTimestamp, ActiveExitTimestamp,
    /// InactiveEnterTimestamp and StateChangeTimestamp.
    pub timestamps: RwLock<UnitTimestamps>,
    /// Atomic restart counter readable without the per-service state lock.
    /// The exit handler increments this before spawning a restart thread,
    /// so `systemctl show -P NRestarts` can read it even while the service
    /// state write-lock is held during `reactivate` → `wait_for_service`.
    pub n_restarts: std::sync::atomic::AtomicU64,
    /// Set when a deactivation (stop) is in progress for this unit.
    /// Used to detect conflicting start requests during ExecStop.
    pub deactivation_in_progress: std::sync::atomic::AtomicBool,
    /// Set when the deactivation is irreversible (--job-mode=replace-irreversibly).
    /// Conflicting start requests are silently rejected.
    pub deactivation_irreversible: std::sync::atomic::AtomicBool,
    /// Set by StartNoBlock when a start is requested during a non-irreversible
    /// deactivation. The Stop handler checks this after ExecStop to fail the stop.
    pub start_requested_during_deactivation: std::sync::atomic::AtomicBool,
    /// Lock-free InvocationID readable without the per-service state lock.
    /// Updated by start_service after generating the UUID, so `systemctl show
    /// -P InvocationID` can read it even while the service state write-lock
    /// is held during `activate` → `wait_for_service`.
    pub invocation_id: std::sync::Mutex<String>,
}

/// Lifecycle timestamps for a unit, tracking when it transitions between
/// the inactive → active → inactive states. All values are microseconds
/// since the Unix epoch (CLOCK_REALTIME).
#[derive(Default)]
pub struct UnitTimestamps {
    /// When the unit last left the inactive state (start began).
    pub inactive_exit: Option<u64>,
    /// When the unit last entered the active state (start finished).
    pub active_enter: Option<u64>,
    /// When the unit last left the active state (stop began).
    pub active_exit: Option<u64>,
    /// When the unit last entered the inactive state (stop finished).
    pub inactive_enter: Option<u64>,
    /// When the unit's state last changed (any transition).
    pub state_change: Option<u64>,
}

impl UnitTimestamps {
    pub fn now_usec() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64
    }

    pub fn record_inactive_exit(&mut self) {
        let now = Self::now_usec();
        self.inactive_exit = Some(now);
        self.state_change = Some(now);
    }

    pub fn record_active_enter(&mut self) {
        let now = Self::now_usec();
        self.active_enter = Some(now);
        self.state_change = Some(now);
    }

    pub fn record_active_exit(&mut self) {
        let now = Self::now_usec();
        self.active_exit = Some(now);
        self.state_change = Some(now);
    }

    pub fn record_inactive_enter(&mut self) {
        let now = Self::now_usec();
        self.inactive_enter = Some(now);
        self.state_change = Some(now);
    }
}

/// Different unit-types have different configs and state
pub enum Specific {
    Service(ServiceSpecific),
    Socket(SocketSpecific),
    Target(TargetSpecific),
    Slice(SliceSpecific),
    Mount(MountSpecific),
    Swap(SwapSpecific),
    Timer(TimerSpecific),
    Path(PathSpecific),
    Device(DeviceSpecific),
}

impl Specific {
    /// Update only the configuration from another `Specific`, preserving
    /// runtime state (PIDs, restart counts, etc.). Used by daemon-reload.
    ///
    /// If the unit type changed (e.g. service → socket), falls back to
    /// full replacement since the state types are incompatible.
    pub fn update_config_from(&mut self, other: Self) {
        match (self, other) {
            (Specific::Service(existing), Specific::Service(new)) => {
                existing.conf = new.conf;
            }
            (Specific::Socket(existing), Specific::Socket(new)) => {
                existing.conf = new.conf;
            }
            (Specific::Slice(existing), Specific::Slice(new)) => {
                existing.conf = new.conf;
            }
            (Specific::Mount(existing), Specific::Mount(new)) => {
                existing.conf = new.conf;
            }
            (Specific::Swap(existing), Specific::Swap(new)) => {
                existing.conf = new.conf;
            }
            (Specific::Timer(existing), Specific::Timer(new)) => {
                existing.conf = new.conf;
            }
            (Specific::Path(existing), Specific::Path(new)) => {
                existing.conf = new.conf;
            }
            (Specific::Device(existing), Specific::Device(new)) => {
                existing.conf = new.conf;
            }
            // Target has no conf, only state — nothing to update.
            (Specific::Target(_), Specific::Target(_)) => {}
            // Type changed — full replacement (state is incompatible anyway).
            (this, other) => {
                *this = other;
            }
        }
    }
}

pub struct ServiceSpecific {
    pub conf: ServiceConfig,
    pub state: RwLock<ServiceState>,
}

impl SocketState {
    fn activate(
        &mut self,
        id: &UnitId,
        conf: &SocketConfig,
        status: &RwLock<UnitStatus>,
        run_info: &RuntimeInfo,
    ) -> Result<UnitStatus, UnitOperationError> {
        let open_res = self
            .sock
            .open_all(
                conf,
                id.name.clone(),
                id.clone(),
                &mut run_info.fd_store.write_poisoned(),
            )
            .map_err(|e| UnitOperationError {
                unit_name: id.name.clone(),
                unit_id: id.clone(),
                reason: UnitOperationErrorReason::SocketOpenError(format!("{e}")),
            });
        match open_res {
            Ok(()) => {
                let mut status = status.write_poisoned();
                *status = UnitStatus::Started(StatusStarted::Running);
                run_info.notify_eventfds();
                Ok(UnitStatus::Started(StatusStarted::Running))
            }
            Err(e) => {
                let mut status = status.write_poisoned();
                *status =
                    UnitStatus::Stopped(StatusStopped::StoppedUnexpected, vec![e.reason.clone()]);
                Err(e)
            }
        }
    }

    fn deactivate(
        &mut self,
        id: &UnitId,
        conf: &SocketConfig,
        status: &RwLock<UnitStatus>,
        run_info: &RuntimeInfo,
    ) -> Result<(), UnitOperationError> {
        let close_result = self
            .sock
            .close_all(
                conf,
                id.name.clone(),
                &mut run_info.fd_store.write_poisoned(),
            )
            .map_err(|e| UnitOperationError {
                unit_name: id.name.clone(),
                unit_id: id.clone(),
                reason: UnitOperationErrorReason::SocketCloseError(e),
            });
        match &close_result {
            Ok(()) => {
                let mut status = status.write_poisoned();
                *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
            }
            Err(e) => {
                let mut status = status.write_poisoned();
                *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![e.reason.clone()]);
            }
        }
        close_result
    }

    fn reactivate(
        &mut self,
        id: &UnitId,
        conf: &SocketConfig,
        status: &RwLock<UnitStatus>,
        run_info: &RuntimeInfo,
    ) -> Result<(), UnitOperationError> {
        let close_result = self
            .sock
            .close_all(
                conf,
                id.name.clone(),
                &mut run_info.fd_store.write_poisoned(),
            )
            .map_err(|e| UnitOperationError {
                unit_name: id.name.clone(),
                unit_id: id.clone(),
                reason: UnitOperationErrorReason::SocketCloseError(e),
            });

        // If closing failed, dont try to restart but fail early
        if let Err(error) = close_result {
            let mut status = status.write_poisoned();
            *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![error.reason.clone()]);
            return Err(error);
        }

        // Reopen and set the status according to the result
        let open_res = self
            .sock
            .open_all(
                conf,
                id.name.clone(),
                id.clone(),
                &mut run_info.fd_store.write_poisoned(),
            )
            .map_err(|e| UnitOperationError {
                unit_name: id.name.clone(),
                unit_id: id.clone(),
                reason: UnitOperationErrorReason::SocketOpenError(format!("{e}")),
            });
        match open_res {
            Ok(()) => {
                let mut status = status.write_poisoned();
                *status = UnitStatus::Started(StatusStarted::Running);
                run_info.notify_eventfds();
                Ok(())
            }
            Err(e) => {
                let mut status = status.write_poisoned();
                *status =
                    UnitStatus::Stopped(StatusStopped::StoppedUnexpected, vec![e.reason.clone()]);
                Err(e)
            }
        }
    }
}

/// Flush pending connections/data on all fds belonging to a socket unit.
/// Called when `FlushPending=yes` is configured, before re-arming a socket
/// for activation, so that stale traffic doesn't immediately re-trigger.
fn flush_socket_fds(socket_id: &UnitId, run_info: &RuntimeInfo) {
    let fd_store = run_info.fd_store.read_poisoned();
    if let Some(fds) = fd_store.get_global(&socket_id.name) {
        for (_, _, fd_box) in fds {
            crate::sockets::flush_pending_connections(fd_box.as_raw_fd());
        }
    }
}

impl ServiceState {
    fn activate(
        &mut self,
        id: &UnitId,
        conf: &ServiceConfig,
        status: &RwLock<UnitStatus>,
        run_info: &RuntimeInfo,
        source: ActivationSource,
        common_invocation_id: &std::sync::Mutex<String>,
    ) -> Result<UnitStatus, UnitOperationError> {
        let start_res = self
            .srvc
            .start(
                conf,
                id.clone(),
                &id.name,
                run_info,
                source,
                common_invocation_id,
            )
            .map_err(|e| UnitOperationError {
                unit_name: id.name.clone(),
                unit_id: id.clone(),
                reason: UnitOperationErrorReason::ServiceStartError(e),
            });
        match start_res {
            Ok(crate::services::StartResult::Started) => {
                {
                    let mut status = status.write_poisoned();
                    // Only transition to Started if still Starting or
                    // Restarting.  A fast-exiting process (e.g. `false`)
                    // may have already triggered SIGCHLD → exit handler →
                    // Stopped/failed between the fork and this point.
                    // Overwriting that status would leave the unit stuck
                    // as "active" forever.
                    if matches!(&*status, UnitStatus::Starting | UnitStatus::Restarting) {
                        *status = UnitStatus::Started(StatusStarted::Running);
                    }
                }
                // Record the instant the service became Running so the
                // watchdog thread can enforce RuntimeMaxSec=.
                self.srvc.runtime_started_at = Some(std::time::Instant::now());
                let (desc, log_level_max) = if let Some(unit) = run_info.unit_table.get(id) {
                    (unit.common.unit.description.clone(), unit.log_level_max())
                } else {
                    (String::new(), None)
                };
                let msg = if desc.is_empty() {
                    format!("Started {}.", id.name)
                } else {
                    format!("Started {desc}.")
                };
                crate::control::varlink::journal_log_unit_lifecycle(
                    &msg,
                    &id.name,
                    log_level_max.as_deref(),
                );
                Ok(UnitStatus::Started(StatusStarted::Running))
            }
            Ok(crate::services::StartResult::ConditionSkipped) => {
                {
                    let mut status = status.write_poisoned();
                    *status = UnitStatus::Stopped(StatusStopped::ConditionSkipped, vec![]);
                }
                // Re-arm associated sockets so socket activation can
                // re-trigger. Without this, ConditionSkipped leaves the
                // socket's activated flag set and the trigger rate limit
                // is never reached (issue #2467).
                for socket_id in &conf.sockets {
                    if let Some(unit) = run_info.unit_table.get(socket_id)
                        && let Specific::Socket(sock) = &unit.specific
                    {
                        if sock.state.read_poisoned().result == SocketResult::TriggerLimitHit {
                            continue;
                        }
                        if sock.conf.flush_pending {
                            flush_socket_fds(socket_id, run_info);
                        }
                        sock.state.write_poisoned().sock.activated = false;
                    }
                }
                if !conf.sockets.is_empty() {
                    run_info.notify_eventfds();
                }
                Ok(UnitStatus::Stopped(StatusStopped::ConditionSkipped, vec![]))
            }
            Ok(crate::services::StartResult::WaitingForSocket) => {
                {
                    let mut status = status.write_poisoned();
                    *status = UnitStatus::Started(StatusStarted::WaitingForSocket);
                }
                // tell socket activation to listen to these sockets again
                for socket_id in &conf.sockets {
                    if let Some(unit) = run_info.unit_table.get(socket_id)
                        && let Specific::Socket(sock) = &unit.specific
                    {
                        // Don't re-arm sockets that hit their trigger rate limit
                        if sock.state.read_poisoned().result == SocketResult::TriggerLimitHit {
                            continue;
                        }
                        // FlushPending=yes: drain queued connections/data before
                        // re-arming, so that stale traffic doesn't immediately
                        // re-trigger socket activation.
                        if sock.conf.flush_pending {
                            flush_socket_fds(socket_id, run_info);
                        }
                        sock.state.write_poisoned().sock.activated = false;
                    }
                }
                run_info.notify_eventfds();
                Ok(UnitStatus::Started(StatusStarted::WaitingForSocket))
            }
            Err(e) => {
                let mut status = status.write_poisoned();
                // Always set StoppedUnexpected on failure.  The service
                // state lock is held by the caller (unit.activate /
                // unit.reactivate), so the exit handler thread cannot
                // concurrently modify the status — no guard needed.
                // Previously a Restarting guard existed here, but it
                // caused services restarted via the exit handler to get
                // permanently stuck in "activating" (Restarting) when the
                // reactivation path called activate() and the error path
                // skipped the status update.
                *status =
                    UnitStatus::Stopped(StatusStopped::StoppedUnexpected, vec![e.reason.clone()]);
                Err(e)
            }
        }
    }

    fn deactivate(
        &mut self,
        id: &UnitId,
        conf: &ServiceConfig,
        status: &RwLock<UnitStatus>,
        run_info: &RuntimeInfo,
    ) -> Result<(), UnitOperationError> {
        // Save the PID before killing so we can wait for the process to
        // be fully reaped.  This ensures that when systemctl stop returns,
        // all resources (including inherited socket fds) are released.
        let old_pid = self.srvc.pid;

        let kill_result = self
            .srvc
            .kill(conf, id.clone(), &id.name, run_info)
            .map_err(|e| UnitOperationError {
                unit_name: id.name.clone(),
                unit_id: id.clone(),
                reason: UnitOperationErrorReason::ServiceStopError(e),
            });

        // Wait for the signal handler thread to reap the old process.
        // Without this, the process may still hold inherited fds (e.g.
        // socket-activated listening sockets) when stop returns, causing
        // EADDRINUSE on subsequent socket restarts.
        if let Some(pid) = old_pid {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                {
                    let pt = run_info.pid_table.lock_poisoned();
                    match pt.get(&pid) {
                        Some(PidEntry::ServiceExited(_)) | None => break,
                        _ => {}
                    }
                }
                if std::time::Instant::now() >= deadline {
                    log::warn!(
                        "Timed out waiting for PID {} of {} to be reaped during stop",
                        pid,
                        id.name
                    );
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        match &kill_result {
            Ok(()) => {
                let mut status = status.write_poisoned();
                *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
            }
            Err(e) => {
                let mut status = status.write_poisoned();
                *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![e.reason.clone()]);
            }
        }
        // Reset socket activated flags so socket activation can restart
        // this service when a new connection arrives on its socket.
        if !conf.sockets.is_empty() {
            for socket_id in &conf.sockets {
                if let Some(unit) = run_info.unit_table.get(socket_id)
                    && let Specific::Socket(sock) = &unit.specific
                {
                    // Don't re-arm sockets that hit their trigger rate limit
                    if sock.state.read_poisoned().result == SocketResult::TriggerLimitHit {
                        continue;
                    }
                    if sock.conf.flush_pending {
                        flush_socket_fds(socket_id, run_info);
                    }
                    sock.state.write_poisoned().sock.activated = false;
                }
            }
            run_info.notify_eventfds();
        }
        // Clean up RuntimeDirectory= dirs unless RuntimeDirectoryPreserve=yes
        if conf.exec_config.runtime_directory_preserve != RuntimeDirectoryPreserve::Yes {
            for dir_name in &conf.exec_config.runtime_directory {
                let full_path = std::path::Path::new("/run").join(dir_name);
                if full_path.exists()
                    && let Err(e) = std::fs::remove_dir_all(&full_path)
                {
                    trace!(
                        "Failed to remove runtime directory {:?} for {}: {}",
                        full_path, id.name, e
                    );
                }
            }
        }
        kill_result
    }
    fn reactivate(
        &mut self,
        id: &UnitId,
        conf: &ServiceConfig,
        status: &RwLock<UnitStatus>,
        run_info: &RuntimeInfo,
        source: ActivationSource,
        common_invocation_id: &std::sync::Mutex<String>,
    ) -> Result<(), UnitOperationError> {
        // Mark the unit as Stopping before killing so that the exit handler
        // recognises the SIGKILL as intentional and doesn't flip the status
        // to StoppedUnexpected (which would race with the subsequent start).
        {
            let mut st = status.write_poisoned();
            *st = UnitStatus::Stopping;
        }

        // Save the old PID so we can wait for the signal handler to
        // process its exit before starting the replacement.
        let old_pid = self.srvc.pid;

        let kill_result = self
            .srvc
            .kill(conf, id.clone(), &id.name, run_info)
            .map_err(|e| UnitOperationError {
                unit_name: id.name.clone(),
                unit_id: id.clone(),
                reason: UnitOperationErrorReason::ServiceStopError(e),
            });

        // If killing failed, dont try to restart but fail early
        if let Err(error) = kill_result {
            let mut status = status.write_poisoned();
            *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![error.reason.clone()]);
            return Err(error);
        }

        // Wait for the signal handler thread to reap the old process and
        // update the PID table to ServiceExited.  This avoids racing with
        // the exit handler (which would call deactivate_unit_recursive and
        // kill the freshly-started replacement).  Do NOT call waitpid()
        // here — that would steal the reap from the signal handler.
        if let Some(pid) = old_pid {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                {
                    let pt = run_info.pid_table.lock_poisoned();
                    match pt.get(&pid) {
                        Some(PidEntry::ServiceExited(_)) | None => break,
                        _ => {}
                    }
                }
                if std::time::Instant::now() >= deadline {
                    log::warn!("Timed out waiting for old PID {} to be reaped", pid);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        // Clear any PID/PGID that may have been set by socket activation
        // during the window between killing the old process and starting
        // the new one.  Kill the socket-activated replacement first so we
        // don't leave an orphan.
        if let Some(new_pid) = self.srvc.pid
            && Some(new_pid) != old_pid
        {
            log::debug!(
                "Killing socket-activated replacement PID {} before restart",
                new_pid
            );
            let _ = nix::sys::signal::kill(new_pid, nix::sys::signal::Signal::SIGKILL);
            // Wait briefly for it to exit
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                {
                    let pt = run_info.pid_table.lock_poisoned();
                    match pt.get(&new_pid) {
                        Some(PidEntry::ServiceExited(_)) | None => break,
                        _ => {}
                    }
                }
                if std::time::Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        self.srvc.pid = None;
        self.srvc.process_group = None;
        info!(
            "reactivate {}: cleared PID/PGID, calling start (old_pid={:?})",
            id.name, old_pid
        );

        // Restart and set the status according to the result
        let start_res = self
            .srvc
            .start(
                conf,
                id.clone(),
                &id.name,
                run_info,
                source,
                common_invocation_id,
            )
            .map_err(|e| UnitOperationError {
                unit_name: id.name.clone(),
                unit_id: id.clone(),
                reason: UnitOperationErrorReason::ServiceStartError(e),
            });
        match start_res {
            Ok(crate::services::StartResult::Started) => {
                {
                    let mut status = status.write_poisoned();
                    // Guard against fast-exiting processes (same race as in activate)
                    if !matches!(&*status, UnitStatus::Stopped(..) | UnitStatus::NeverStarted) {
                        *status = UnitStatus::Started(StatusStarted::Running);
                    }
                }
                self.srvc.runtime_started_at = Some(std::time::Instant::now());
                Ok(())
            }
            Ok(crate::services::StartResult::ConditionSkipped) => {
                {
                    let mut status = status.write_poisoned();
                    *status = UnitStatus::Stopped(StatusStopped::ConditionSkipped, vec![]);
                }
                Ok(())
            }
            Ok(crate::services::StartResult::WaitingForSocket) => {
                {
                    let mut status = status.write_poisoned();
                    *status = UnitStatus::Started(StatusStarted::WaitingForSocket);
                }
                // tell socket activation to listen to these sockets again
                for socket_id in &conf.sockets {
                    if let Some(unit) = run_info.unit_table.get(socket_id)
                        && let Specific::Socket(sock) = &unit.specific
                    {
                        // Don't re-arm sockets that hit their trigger rate limit
                        if sock.state.read_poisoned().result == SocketResult::TriggerLimitHit {
                            continue;
                        }
                        if sock.conf.flush_pending {
                            flush_socket_fds(socket_id, run_info);
                        }
                        sock.state.write_poisoned().sock.activated = false;
                    }
                }
                run_info.notify_eventfds();
                Ok(())
            }
            Err(e) => {
                let mut status = status.write_poisoned();
                *status =
                    UnitStatus::Stopped(StatusStopped::StoppedUnexpected, vec![e.reason.clone()]);
                Err(e)
            }
        }
    }
}

impl ServiceSpecific {
    pub fn has_socket(&self, socket: &str) -> bool {
        self.conf.sockets.iter().any(|id| id.eq(socket))
    }
}

pub struct SocketSpecific {
    pub conf: SocketConfig,
    pub state: RwLock<SocketState>,
}

impl SocketSpecific {
    pub fn belongs_to_service(&self, service: &str) -> bool {
        self.conf.services.iter().any(|id| id.eq(service))
    }
}

pub struct TargetSpecific {
    pub state: RwLock<TargetState>,
}

pub struct SliceSpecific {
    pub conf: SliceConfig,
    pub state: RwLock<SliceState>,
}

/// The immutable configuration of a slice unit, parsed from the `[Slice]`
/// section. Slices accept the same resource-control directives as services.
pub struct SliceConfig {
    /// MemoryMin= — minimum memory guarantee. See systemd.resource-control(5).
    pub memory_min: Option<MemoryLimit>,
    /// MemoryLow= — low memory boundary. See systemd.resource-control(5).
    pub memory_low: Option<MemoryLimit>,
    /// MemoryHigh= — throttling memory boundary. See systemd.resource-control(5).
    pub memory_high: Option<MemoryLimit>,
    /// MemoryMax= — hard memory limit. See systemd.resource-control(5).
    pub memory_max: Option<MemoryLimit>,
    /// MemorySwapMax= — hard swap limit. See systemd.resource-control(5).
    pub memory_swap_max: Option<MemoryLimit>,
    /// CPUWeight= — CPU scheduling weight (1–10000). See systemd.resource-control(5).
    pub cpu_weight: Option<CpuWeight>,
    /// StartupCPUWeight= — CPU weight during startup. See systemd.resource-control(5).
    pub startup_cpu_weight: Option<CpuWeight>,
    /// CPUQuota= — CPU time quota as percentage. See systemd.resource-control(5).
    pub cpu_quota: Option<CpuQuota>,
    /// IOWeight= — I/O scheduling weight (1–10000). See systemd.resource-control(5).
    pub io_weight: Option<IoWeight>,
    /// StartupIOWeight= — I/O weight during startup. See systemd.resource-control(5).
    pub startup_io_weight: Option<IoWeight>,
    /// IODeviceWeight= — per-device I/O weight. See systemd.resource-control(5).
    pub io_device_weight: Vec<IoDeviceLimit>,
    /// IOReadBandwidthMax= — per-device read bandwidth limit. See systemd.resource-control(5).
    pub io_read_bandwidth_max: Vec<IoDeviceLimit>,
    /// IOWriteBandwidthMax= — per-device write bandwidth limit. See systemd.resource-control(5).
    pub io_write_bandwidth_max: Vec<IoDeviceLimit>,
    /// IOReadIOPSMax= — per-device read IOPS limit. See systemd.resource-control(5).
    pub io_read_iops_max: Vec<IoDeviceLimit>,
    /// IOWriteIOPSMax= — per-device write IOPS limit. See systemd.resource-control(5).
    pub io_write_iops_max: Vec<IoDeviceLimit>,
    /// TasksMax= — maximum number of tasks. See systemd.resource-control(5).
    pub tasks_max: Option<TasksMax>,
    /// Delegate= — delegate cgroup subtree. See systemd.resource-control(5).
    pub delegate: Delegate,
    /// CPUAccounting= — enable CPU accounting. See systemd.resource-control(5).
    pub cpu_accounting: Option<bool>,
    /// MemoryAccounting= — enable memory accounting. See systemd.resource-control(5).
    pub memory_accounting: Option<bool>,
    /// IOAccounting= — enable I/O accounting. See systemd.resource-control(5).
    pub io_accounting: Option<bool>,
    /// TasksAccounting= — enable task counting. See systemd.resource-control(5).
    pub tasks_accounting: Option<bool>,
    /// DeviceAllow= — per-device access control. See systemd.resource-control(5).
    pub device_allow: Vec<String>,
    /// DevicePolicy= — device access policy. See systemd.resource-control(5).
    pub device_policy: DevicePolicy,
    /// IPAddressAllow= — allowed IP address prefixes. See systemd.resource-control(5).
    pub ip_address_allow: Vec<String>,
    /// IPAddressDeny= — denied IP address prefixes. See systemd.resource-control(5).
    pub ip_address_deny: Vec<String>,
    /// ManagedOOMSwap= — managed OOM swap policy. See systemd.resource-control(5).
    pub managed_oom_swap: Option<String>,
    /// ManagedOOMMemoryPressure= — managed OOM memory pressure policy. See systemd.resource-control(5).
    pub managed_oom_memory_pressure: Option<String>,
    /// ManagedOOMMemoryPressureLimit= — managed OOM pressure threshold. See systemd.resource-control(5).
    pub managed_oom_memory_pressure_limit: Option<String>,
    /// ManagedOOMPreference= — managed OOM preference. See systemd.resource-control(5).
    pub managed_oom_preference: Option<String>,
    /// MemoryPressureWatch= — PSI memory pressure monitoring. See systemd.resource-control(5).
    pub memory_pressure_watch: MemoryPressureWatch,
    /// CPUQuotaPeriodSec= — the period for CPU quota enforcement. See systemd.resource-control(5).
    pub cpu_quota_period_sec: Option<Timeout>,
    /// AllowedCPUs= — CPU set restriction (cpuset controller). See systemd.resource-control(5).
    pub allowed_cpus: Option<String>,
    /// StartupAllowedCPUs= — CPU set restriction during startup. See systemd.resource-control(5).
    pub startup_allowed_cpus: Option<String>,
    /// AllowedMemoryNodes= — NUMA node restriction (cpuset controller). See systemd.resource-control(5).
    pub allowed_memory_nodes: Option<String>,
    /// StartupAllowedMemoryNodes= — NUMA node restriction during startup. See systemd.resource-control(5).
    pub startup_allowed_memory_nodes: Option<String>,
    /// DefaultMemoryMin= — default memory.min for children. See systemd.resource-control(5).
    pub default_memory_min: Option<MemoryLimit>,
    /// DefaultMemoryLow= — default memory.low for children. See systemd.resource-control(5).
    pub default_memory_low: Option<MemoryLimit>,
    /// MemoryZSwapMax= — hard zswap limit. See systemd.resource-control(5).
    pub memory_zswap_max: Option<MemoryLimit>,
    /// IODeviceLatencyTargetSec= — per-device I/O latency target. See systemd.resource-control(5).
    pub io_device_latency_target_sec: Vec<String>,
    /// DisableControllers= — controllers to disable. See systemd.resource-control(5).
    pub disable_controllers: Vec<String>,
    /// MemoryPressureThresholdSec= — memory pressure threshold duration. See systemd.resource-control(5).
    pub memory_pressure_threshold_sec: Option<Timeout>,
    /// IPIngressFilterPath= — BPF ingress filter paths. See systemd.resource-control(5).
    pub ip_ingress_filter_path: Vec<String>,
    /// IPEgressFilterPath= — BPF egress filter paths. See systemd.resource-control(5).
    pub ip_egress_filter_path: Vec<String>,
    /// BPFProgram= — BPF programs to attach. See systemd.resource-control(5).
    pub bpf_program: Vec<String>,
    /// SocketBindAllow= — socket bind allow rules. See systemd.resource-control(5).
    pub socket_bind_allow: Vec<String>,
    /// SocketBindDeny= — socket bind deny rules. See systemd.resource-control(5).
    pub socket_bind_deny: Vec<String>,
    /// RestrictNetworkInterfaces= — network interface restrictions. See systemd.resource-control(5).
    pub restrict_network_interfaces: Vec<String>,
    /// NFTSet= — nftables set attachments. See systemd.resource-control(5).
    pub nft_set: Vec<String>,
    /// DelegateSubgroup= — subgroup for delegated cgroup. See systemd.resource-control(5).
    pub delegate_subgroup: Option<String>,
}

impl From<ParsedSliceSection> for SliceConfig {
    fn from(s: ParsedSliceSection) -> Self {
        Self {
            memory_min: s.memory_min,
            memory_low: s.memory_low,
            memory_high: s.memory_high,
            memory_max: s.memory_max,
            memory_swap_max: s.memory_swap_max,
            cpu_weight: s.cpu_weight,
            startup_cpu_weight: s.startup_cpu_weight,
            cpu_quota: s.cpu_quota,
            io_weight: s.io_weight,
            startup_io_weight: s.startup_io_weight,
            io_device_weight: s.io_device_weight,
            io_read_bandwidth_max: s.io_read_bandwidth_max,
            io_write_bandwidth_max: s.io_write_bandwidth_max,
            io_read_iops_max: s.io_read_iops_max,
            io_write_iops_max: s.io_write_iops_max,
            tasks_max: s.tasks_max,
            delegate: s.delegate,
            cpu_accounting: s.cpu_accounting,
            memory_accounting: s.memory_accounting,
            io_accounting: s.io_accounting,
            tasks_accounting: s.tasks_accounting,
            device_allow: s.device_allow,
            device_policy: s.device_policy,
            ip_address_allow: s.ip_address_allow,
            ip_address_deny: s.ip_address_deny,
            managed_oom_swap: s.managed_oom_swap,
            managed_oom_memory_pressure: s.managed_oom_memory_pressure,
            managed_oom_memory_pressure_limit: s.managed_oom_memory_pressure_limit,
            managed_oom_preference: s.managed_oom_preference,
            memory_pressure_watch: s.memory_pressure_watch,
            cpu_quota_period_sec: s.cpu_quota_period_sec,
            allowed_cpus: s.allowed_cpus,
            startup_allowed_cpus: s.startup_allowed_cpus,
            allowed_memory_nodes: s.allowed_memory_nodes,
            startup_allowed_memory_nodes: s.startup_allowed_memory_nodes,
            default_memory_min: s.default_memory_min,
            default_memory_low: s.default_memory_low,
            memory_zswap_max: s.memory_zswap_max,
            io_device_latency_target_sec: s.io_device_latency_target_sec,
            disable_controllers: s.disable_controllers,
            memory_pressure_threshold_sec: s.memory_pressure_threshold_sec,
            ip_ingress_filter_path: s.ip_ingress_filter_path,
            ip_egress_filter_path: s.ip_egress_filter_path,
            bpf_program: s.bpf_program,
            socket_bind_allow: s.socket_bind_allow,
            socket_bind_deny: s.socket_bind_deny,
            restrict_network_interfaces: s.restrict_network_interfaces,
            nft_set: s.nft_set,
            delegate_subgroup: s.delegate_subgroup,
        }
    }
}

pub struct MountSpecific {
    pub conf: MountConfig,
    pub state: RwLock<MountState>,
}

pub struct TimerSpecific {
    pub conf: TimerConfig,
    pub state: RwLock<TimerState>,
}

pub struct PathSpecific {
    pub conf: PathConfig,
    pub state: RwLock<PathState>,
}

pub struct DeviceSpecific {
    pub conf: DeviceConfig,
    pub state: RwLock<DeviceState>,
}

pub struct SwapSpecific {
    pub conf: SwapConfig,
    pub state: RwLock<SwapState>,
}

/// Configuration for a `.device` unit. Device units are dynamically created
/// from udev events and represent kernel devices. They have no `[Device]`
/// section directives of their own — they only use `[Unit]` and `[Install]`.
///
/// The `SysFSPath` property exposes the sysfs path, and device units can
/// carry `SYSTEMD_WANTS=`, `SYSTEMD_ALIAS=`, and `SYSTEMD_READY=` udev
/// properties for integration with the service manager.
pub struct DeviceConfig {
    /// The sysfs path of the device (e.g. `/sys/devices/pci0000:00/...`).
    pub sysfs_path: Option<String>,
}

#[derive(Default)]
/// All units have some common mutable state
pub struct CommonState {
    pub up_since: Option<std::time::Instant>,
    pub restart_count: u64,
    /// Timestamps of recent start attempts for StartLimitBurst/StartLimitIntervalSec enforcement.
    pub start_timestamps: Vec<std::time::Instant>,
    /// Cgroup freezer state: "running" or "frozen".
    pub freezer_state: FreezerState,
}

/// Cgroup freezer state for a unit.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FreezerState {
    #[default]
    Running,
    Frozen,
    /// Frozen because a parent slice was frozen, not by direct request.
    FrozenByParent,
}

impl FreezerState {
    pub fn as_str(&self) -> &'static str {
        match self {
            FreezerState::Running => "running",
            FreezerState::Frozen => "frozen",
            FreezerState::FrozenByParent => "frozen-by-parent",
        }
    }
}

pub struct ServiceState {
    pub common: CommonState,
    pub srvc: Service,
}
pub struct SocketState {
    pub common: CommonState,
    pub sock: Socket,
    /// Result of the socket unit — "success" normally, "trigger-limit-hit" when
    /// the unit has been triggered too many times within the rate limit window.
    pub result: SocketResult,
}

/// The result state of a socket unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SocketResult {
    #[default]
    Success,
    TriggerLimitHit,
}
pub struct TargetState {
    pub common: CommonState,
}
pub struct SliceState {
    pub common: CommonState,
}

pub struct MountState {
    pub common: CommonState,
}

pub struct SwapState {
    pub common: CommonState,
}

pub struct TimerState {
    pub common: CommonState,
    /// Timestamp (microseconds since epoch) when the timer last triggered.
    pub last_trigger_usec: Option<u64>,
}

pub struct PathState {
    pub common: CommonState,
    /// Result of the path unit — "success" normally, "trigger-limit-hit" when
    /// the unit has been triggered too many times within the rate limit window.
    pub result: PathResult,
}

/// The result state of a path unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PathResult {
    #[default]
    Success,
    TriggerLimitHit,
}

pub struct DeviceState {
    pub common: CommonState,
}

/// Configuration for a `.timer` unit, derived from the parsed `[Timer]` section.
#[derive(Debug, Clone)]
pub struct TimerConfig {
    /// OnActiveSec= — relative to the time the timer unit itself is activated.
    pub on_active_sec: Vec<std::time::Duration>,
    /// OnBootSec= — relative to when the machine was booted up.
    pub on_boot_sec: Vec<std::time::Duration>,
    /// OnStartupSec= — relative to when the service manager was first started.
    pub on_startup_sec: Vec<std::time::Duration>,
    /// OnUnitActiveSec= — relative to when the unit the timer activates was last activated.
    pub on_unit_active_sec: Vec<std::time::Duration>,
    /// OnUnitInactiveSec= — relative to when the unit the timer activates was last deactivated.
    pub on_unit_inactive_sec: Vec<std::time::Duration>,
    /// OnCalendar= — calendar event expressions (stored as raw strings for now).
    pub on_calendar: Vec<String>,
    /// AccuracySec= — accuracy of the timer.
    pub accuracy_sec: std::time::Duration,
    /// RandomizedDelaySec= — random delay added on top of the timer.
    pub randomized_delay_sec: std::time::Duration,
    /// FixedRandomDelay= — if true, the randomized delay is deterministic
    /// (based on a hash of the unit name and machine ID) rather than
    /// truly random on each iteration. Defaults to false.
    /// See systemd.timer(5).
    pub fixed_random_delay: bool,
    /// Persistent= — if true, missed runs are triggered immediately on boot.
    pub persistent: bool,
    /// WakeSystem= — if true, wake the system from suspend to fire the timer.
    pub wake_system: bool,
    /// RemainAfterElapse= — if true, timer stays loaded after elapsing (default true).
    pub remain_after_elapse: bool,
    /// OnClockChange= — if true, the timer is triggered when the system clock
    /// jumps relative to the monotonic clock (e.g. DST change, NTP correction).
    /// See systemd.timer(5).
    pub on_clock_change: bool,
    /// OnTimezoneChange= — if true, the timer is triggered when the system
    /// timezone changes. See systemd.timer(5).
    pub on_timezone_change: bool,
    /// Unit= — the unit to activate when the timer elapses (defaults to same-name .service).
    pub unit: String,
}

/// A single path watch condition from a `.path` unit file.
#[derive(Debug, Clone)]
pub enum PathCondition {
    /// PathExists= — trigger when the path exists.
    PathExists(String),
    /// PathExistsGlob= — trigger when any path matching the glob exists.
    PathExistsGlob(String),
    /// PathChanged= — trigger when the path changes (inotify: create, delete, move, attrib).
    PathChanged(String),
    /// PathModified= — trigger when the path is modified (inotify: create, delete, move, attrib, close_write).
    PathModified(String),
    /// DirectoryNotEmpty= — trigger when the directory is not empty.
    DirectoryNotEmpty(String),
}

impl PathCondition {
    /// Returns the filesystem path being watched.
    pub fn path(&self) -> &str {
        match self {
            PathCondition::PathExists(p)
            | PathCondition::PathExistsGlob(p)
            | PathCondition::PathChanged(p)
            | PathCondition::PathModified(p)
            | PathCondition::DirectoryNotEmpty(p) => p,
        }
    }
}

/// Configuration for a `.path` unit, derived from the parsed `[Path]` section.
#[derive(Debug, Clone)]
pub struct PathConfig {
    /// The path conditions to watch for.
    pub conditions: Vec<PathCondition>,
    /// MakeDirectory= — create the watched directory before watching (default false).
    pub make_directory: bool,
    /// DirectoryMode= — permission mode for MakeDirectory (default 0o755).
    pub directory_mode: u32,
    /// TriggerLimitIntervalSec= — rate limit interval for path triggers.
    pub trigger_limit_interval_sec: std::time::Duration,
    /// TriggerLimitBurst= — rate limit burst for path triggers.
    pub trigger_limit_burst: u32,
    /// Unit= — the unit to activate when the path condition is met (defaults to same-name .service).
    pub unit: String,
}

/// Configuration for a `.mount` unit, derived from the parsed `[Mount]` section.
#[derive(Debug, Clone)]
/// The immutable configuration of a swap unit, parsed from the `[Swap]` section.
///
/// See systemd.swap(5) for documentation.
pub struct SwapConfig {
    /// What= — the absolute path of a device node or file to use as swap.
    pub what: String,

    /// Priority= — the swap priority passed to swapon(2).
    /// Range: -1 to 32767. None means use kernel default.
    pub priority: Option<i32>,

    /// Options= — mount-style options string for swapon (e.g. "discard").
    pub options: Option<String>,

    /// TimeoutSec= — timeout for swapon/swapoff operations.
    pub timeout_sec: Option<u64>,
}

impl From<ParsedSwapSection> for SwapConfig {
    fn from(s: ParsedSwapSection) -> Self {
        Self {
            what: s.what,
            priority: s.priority,
            options: s.options,
            timeout_sec: s.timeout_sec,
        }
    }
}

pub struct MountConfig {
    /// What= — the device, file, or resource to mount.
    pub what: String,
    /// Where= — the mount point path.
    pub where_: String,
    /// Type= — filesystem type (e.g. "ext4", "tmpfs").
    pub fs_type: Option<String>,
    /// Options= — comma-separated mount options.
    pub options: Option<String>,
    /// SloppyOptions= — tolerate unknown mount options.
    pub sloppy_options: bool,
    /// LazyUnmount= — use MNT_DETACH when unmounting.
    pub lazy_unmount: bool,
    /// ReadWriteOnly= — fail if can't mount read-write.
    pub read_write_only: bool,
    /// ForceUnmount= — use MNT_FORCE when unmounting.
    pub force_unmount: bool,
    /// DirectoryMode= — mode for auto-created mount point directory.
    pub directory_mode: u32,
    /// TimeoutSec= — mount operation timeout.
    pub timeout_sec: Option<u64>,
}

impl From<ParsedMountSection> for MountConfig {
    fn from(parsed: ParsedMountSection) -> Self {
        Self {
            what: parsed.what,
            where_: parsed.where_,
            fs_type: parsed.fs_type,
            options: parsed.options,
            sloppy_options: parsed.sloppy_options,
            lazy_unmount: parsed.lazy_unmount,
            read_write_only: parsed.read_write_only,
            force_unmount: parsed.force_unmount,
            directory_mode: parsed.directory_mode,
            timeout_sec: parsed.timeout_sec,
        }
    }
}

// Fields are held to keep RwLockWriteGuards alive during activate/deactivate/reactivate
#[allow(dead_code)]
enum LockedState<'a> {
    Service(
        std::sync::RwLockWriteGuard<'a, ServiceState>,
        &'a ServiceConfig,
    ),
    Socket(
        std::sync::RwLockWriteGuard<'a, SocketState>,
        &'a SocketConfig,
    ),
    Target(std::sync::RwLockWriteGuard<'a, TargetState>),
    Slice(std::sync::RwLockWriteGuard<'a, SliceState>),
    Mount(std::sync::RwLockWriteGuard<'a, MountState>, &'a MountConfig),
    Swap(std::sync::RwLockWriteGuard<'a, SwapState>, &'a SwapConfig),
    Timer(std::sync::RwLockWriteGuard<'a, TimerState>, &'a TimerConfig),
    Path(std::sync::RwLockWriteGuard<'a, PathState>, &'a PathConfig),
    Device(std::sync::RwLockWriteGuard<'a, DeviceState>),
}

impl Unit {
    pub const fn is_service(&self) -> bool {
        matches!(self.id.kind, UnitIdKind::Service)
    }
    pub const fn is_socket(&self) -> bool {
        matches!(self.id.kind, UnitIdKind::Socket)
    }
    pub const fn is_target(&self) -> bool {
        matches!(self.id.kind, UnitIdKind::Target)
    }
    pub const fn is_slice(&self) -> bool {
        matches!(self.id.kind, UnitIdKind::Slice)
    }
    pub const fn is_mount(&self) -> bool {
        matches!(self.id.kind, UnitIdKind::Mount)
    }
    pub const fn is_swap(&self) -> bool {
        matches!(self.id.kind, UnitIdKind::Swap)
    }
    pub const fn is_device(&self) -> bool {
        matches!(self.id.kind, UnitIdKind::Device)
    }
    pub const fn is_path(&self) -> bool {
        matches!(self.id.kind, UnitIdKind::Path)
    }

    pub fn name_without_suffix(&self) -> String {
        let split: Vec<_> = self.id.name.split('.').collect();
        split[0..split.len() - 1].join(".")
    }

    pub fn dedup_dependencies(&mut self) {
        self.common.dependencies.dedup();
    }

    /// Return the `LogLevelMax=` value from the service's exec config, if set.
    pub fn log_level_max(&self) -> Option<String> {
        match &self.specific {
            Specific::Service(srvc) => srvc.conf.exec_config.log_level_max.clone(),
            _ => None,
        }
    }

    /// Check if the transition to state 'Starting' can be done
    ///
    /// This is the case if:
    /// 1. All units that have a before relation to this unit have been run at least once
    /// 1. All of the above that are required by this unit are in the state 'Started'
    fn state_transition_starting(&self, run_info: &RuntimeInfo) -> Result<(), Vec<UnitId>> {
        let (mut self_lock, others) = acquire_locks(
            vec![self.id.clone()],
            self.common.dependencies.after.clone(),
            &run_info.unit_table,
        );

        let unstarted_deps = others
            .iter()
            .fold(Vec::new(), |mut acc, (id, status_locked)| {
                let required = self.common.dependencies.requires.contains(id)
                    || self.common.dependencies.binds_to.contains(id);
                let pulled = self.common.dependencies.wants.contains(id)
                    || self.common.dependencies.upholds.contains(id);
                let is_pull_dep = required || pulled;
                let ready = if required {
                    // StoppedFinal means the unit completed successfully
                    // (e.g. Type=oneshot without RemainAfterExit).  Treat
                    // it as ready so dependents aren't blocked.
                    status_locked.is_started()
                        || matches!(
                            &**status_locked,
                            UnitStatus::Stopped(
                                StatusStopped::StoppedFinal | StatusStopped::ConditionSkipped,
                                _
                            )
                        )
                } else if is_pull_dep {
                    **status_locked != UnitStatus::NeverStarted
                } else {
                    // Pure ordering dep (After= without pull-dep):
                    // NeverStarted means it's not being activated — treat as ready.
                    // Starting means it's actively starting — wait for it.
                    match &**status_locked {
                        UnitStatus::NeverStarted => true,
                        UnitStatus::Starting => false,
                        _ => true,
                    }
                };

                if !ready {
                    acc.push(id.clone());
                }
                acc
            });

        if unstarted_deps.is_empty() {
            **self_lock.get_mut(&self.id).unwrap() = UnitStatus::Starting;
            let desc = &self.common.unit.description;
            let msg = if desc.is_empty() {
                format!("Starting {}...", self.id.name)
            } else {
                format!("Starting {desc}...")
            };
            let log_level_max = self.log_level_max();
            crate::control::varlink::journal_log_unit_lifecycle(
                &msg,
                &self.id.name,
                log_level_max.as_deref(),
            );
            Ok(())
        } else {
            Err(unstarted_deps)
        }
        // All locks are released again here
    }

    /// Check if the transition to state 'Restarting' can be done. Returns whether the status before was
    /// Started, which requires a full restart.
    ///
    /// This is the case if:
    /// 1. All units that have a before relation to this unit have been run at least once
    /// 1. All of the above that are required by this unit are in the state 'Started'
    fn state_transition_restarting(&self, run_info: &RuntimeInfo) -> Result<bool, Vec<UnitId>> {
        let (mut self_lock, others) = acquire_locks(
            vec![self.id.clone()],
            self.common.dependencies.after.clone(),
            &run_info.unit_table,
        );

        let unstarted_deps = others
            .iter()
            .fold(Vec::new(), |mut acc, (id, status_locked)| {
                let required = self.common.dependencies.requires.contains(id)
                    || self.common.dependencies.binds_to.contains(id);
                let pulled = self.common.dependencies.wants.contains(id)
                    || self.common.dependencies.upholds.contains(id);
                let is_pull_dep = required || pulled;
                let ready = if required {
                    status_locked.is_started()
                        || matches!(
                            &**status_locked,
                            UnitStatus::Stopped(
                                StatusStopped::StoppedFinal | StatusStopped::ConditionSkipped,
                                _
                            )
                        )
                } else if is_pull_dep {
                    **status_locked != UnitStatus::NeverStarted
                } else {
                    // Pure ordering dep: treat NeverStarted as ready
                    match &**status_locked {
                        UnitStatus::NeverStarted => true,
                        UnitStatus::Starting => false,
                        _ => true,
                    }
                };

                if !ready {
                    acc.push(id.clone());
                }
                acc
            });

        if unstarted_deps.is_empty() {
            let need_full_restart = self_lock.get_mut(&self.id).unwrap().is_started();
            **self_lock.get_mut(&self.id).unwrap() = UnitStatus::Restarting;
            Ok(need_full_restart)
        } else {
            Err(unstarted_deps)
        }
        // All locks are released again here
    }

    /// Check if the transition to state 'Stopping' can be done
    ///
    /// This is the case if:
    /// 1. All units that have a requires relation to this unit have been stopped
    fn state_transition_stopping(&self, run_info: &RuntimeInfo) -> Result<(), Vec<UnitId>> {
        let (mut self_lock, others) = acquire_locks(
            vec![self.id.clone()],
            self.common.dependencies.kill_before_this(),
            &run_info.unit_table,
        );

        let unkilled_depending = others
            .iter()
            .fold(Vec::new(), |mut acc, (id, status_locked)| {
                if status_locked.is_started() {
                    acc.push(id.clone());
                }
                acc
            });

        if unkilled_depending.is_empty() {
            **self_lock.get_mut(&self.id).unwrap() = UnitStatus::Stopping;
            Ok(())
        } else {
            Err(unkilled_depending)
        }
        // All locks are released again here
    }

    /// This activates the unit and manages the state transitions. It reports back the new unit status or any
    /// errors encountered while starting the unit. Note that these errors are also recorded in the units status.
    pub fn activate(
        &self,
        run_info: &RuntimeInfo,
        source: ActivationSource,
    ) -> Result<UnitStatus, UnitOperationError> {
        let state = match &self.specific {
            Specific::Service(specific) => {
                LockedState::Service(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Socket(specific) => {
                LockedState::Socket(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Target(specific) => LockedState::Target(specific.state.write_poisoned()),
            Specific::Slice(specific) => LockedState::Slice(specific.state.write_poisoned()),
            Specific::Mount(specific) => {
                LockedState::Mount(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Swap(specific) => {
                LockedState::Swap(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Timer(specific) => {
                LockedState::Timer(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Path(specific) => {
                LockedState::Path(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Device(specific) => LockedState::Device(specific.state.write_poisoned()),
        };

        {
            let self_status = &*self.common.status.read_poisoned();
            match self_status {
                UnitStatus::Started(StatusStarted::WaitingForSocket) => {
                    if source == ActivationSource::SocketActivation {
                        // Need activation
                    } else {
                        // Dont need activation
                        return Ok(self_status.clone());
                    }
                }
                UnitStatus::Started(_) => {
                    // Dont need activation
                    return Ok(self_status.clone());
                }
                UnitStatus::Stopped(StatusStopped::StoppedUnexpected, _)
                    if !source.is_socket_activation() =>
                {
                    // Unit already tried to start and failed.  Don't retry
                    // during the initial activation graph walk — only the
                    // restart mechanism (service_exit_handler / reactivate)
                    // should retry failed units if Restart= policy allows it.
                    // Socket activation is allowed through so that incoming
                    // connections can still trigger a restart.
                    return Ok(self_status.clone());
                }
                UnitStatus::Stopped(_, _) => {
                    // Need activation — both regular and socket activation
                    // should start a stopped unit.
                }
                _ => {
                    // Need activation
                }
            }
        }

        self.state_transition_starting(run_info).map_err(|bad_ids| {
            trace!(
                "Unit: {} ignores activation. Not all dependencies have been started (still waiting for: {:?})",
                self.id.name,
                bad_ids,
            );
            UnitOperationError {
                reason: UnitOperationErrorReason::DependencyError(bad_ids),
                unit_name: self.id.name.clone(),
                unit_id: self.id.clone(),
            }
        })?;

        match state {
            LockedState::Target(_) | LockedState::Slice(_) | LockedState::Device(_) => {
                {
                    let mut status = self.common.status.write_poisoned();
                    if status.is_started() {
                        return Ok(status.clone());
                    }
                    *status = UnitStatus::Started(StatusStarted::Running);
                }
                trace!("Reached target {}", self.id.name);
                Ok(UnitStatus::Started(StatusStarted::Running))
            }
            LockedState::Socket(mut state, conf) => {
                let state = &mut *state;
                state.activate(&self.id, conf, &self.common.status, run_info)
            }
            LockedState::Service(mut state, conf) => {
                let state = &mut *state;
                // Resolve JoinsNamespaceOf= before starting the service
                state.srvc.join_namespace_pid = crate::services::resolve_joins_namespace_of(
                    &self.id.name,
                    &self.common.unit.joins_namespace_of,
                    run_info,
                );
                state.activate(
                    &self.id,
                    conf,
                    &self.common.status,
                    run_info,
                    source,
                    &self.common.invocation_id,
                )
            }
            LockedState::Mount(_, conf) => activate_mount(&self.id, conf, &self.common.status),
            LockedState::Swap(_, conf) => activate_swap(&self.id, conf, &self.common.status),
            LockedState::Timer(mut timer_state, conf) => {
                // Timer units are "started" by marking them as running.
                // The actual scheduling is handled by the timer thread.
                let mut status = self.common.status.write_poisoned();
                if status.is_started() {
                    return Ok(status.clone());
                }
                // If Persistent=true, read the stamp file to recover
                // LastTriggerUSec from a previous boot.
                if conf.persistent {
                    let stamp_path = format!("/var/lib/systemd/timers/stamp-{}", self.id.name);
                    if let Ok(meta) = std::fs::metadata(&stamp_path)
                        && let Ok(mtime) = meta.modified()
                        && let Ok(dur) = mtime.duration_since(std::time::SystemTime::UNIX_EPOCH)
                    {
                        let usec = dur.as_micros() as u64;
                        trace!(
                            "Timer {}: read stamp file {}, last_trigger={}",
                            self.id.name, stamp_path, usec
                        );
                        timer_state.last_trigger_usec = Some(usec);
                    }
                }
                *status = UnitStatus::Started(StatusStarted::Running);
                trace!("Started timer {}", self.id.name);
                Ok(UnitStatus::Started(StatusStarted::Running))
            }
            LockedState::Path(_, conf) => {
                // Path units are "started" by marking them as running.
                // The actual path monitoring is handled by the path watcher thread.
                // If MakeDirectory= is set, create the directory before watching.
                if conf.make_directory {
                    for cond in &conf.conditions {
                        let p = cond.path();
                        if let Err(e) = std::fs::create_dir_all(p) {
                            trace!(
                                "Path unit {}: MakeDirectory failed for {}: {}",
                                self.id.name, p, e
                            );
                        } else {
                            // Apply DirectoryMode=
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                let _ = std::fs::set_permissions(
                                    p,
                                    std::fs::Permissions::from_mode(conf.directory_mode),
                                );
                            }
                        }
                    }
                }
                // Record baseline snapshots for PathChanged/PathModified
                // conditions synchronously, before returning from activation.
                // The path watcher thread will pick these up and use them as
                // the reference point for detecting changes.
                crate::path_watcher::record_activation_baselines(&self.id.name, conf);

                let mut status = self.common.status.write_poisoned();
                if status.is_started() {
                    return Ok(status.clone());
                }
                *status = UnitStatus::Started(StatusStarted::Running);
                trace!("Started path unit {}", self.id.name);
                Ok(UnitStatus::Started(StatusStarted::Running))
            }
        }
    }

    /// This dectivates the unit and manages the state transitions. It reports back any
    /// errors encountered while stopping the unit
    pub fn deactivate(&self, run_info: &RuntimeInfo) -> Result<(), UnitOperationError> {
        let state = match &self.specific {
            Specific::Service(specific) => {
                LockedState::Service(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Socket(specific) => {
                LockedState::Socket(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Target(specific) => LockedState::Target(specific.state.write_poisoned()),
            Specific::Slice(specific) => LockedState::Slice(specific.state.write_poisoned()),
            Specific::Mount(specific) => {
                LockedState::Mount(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Swap(specific) => {
                LockedState::Swap(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Timer(specific) => {
                LockedState::Timer(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Path(specific) => {
                LockedState::Path(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Device(specific) => LockedState::Device(specific.state.write_poisoned()),
        };

        {
            let self_status = &*self.common.status.read_poisoned();
            if let UnitStatus::Stopped(_, _) = self_status {
                return Ok(());
            }
            // Need deactivation
        }

        self.state_transition_stopping(run_info).map_err(|bad_ids| {
            trace!(
                "Unit: {} ignores deactivation. Not all units depending on this unit have been started (still waiting for: {:?})",
                self.id.name,
                bad_ids,
            );
            UnitOperationError {
                reason: UnitOperationErrorReason::DependencyError(bad_ids),
                unit_name: self.id.name.clone(),
                unit_id: self.id.clone(),
            }
        })?;

        // Record lifecycle timestamp: leaving active state
        self.common.timestamps.write_poisoned().record_active_exit();

        trace!("Deactivate unit: {}", self.id.name);
        let result = match state {
            LockedState::Target(_) | LockedState::Slice(_) | LockedState::Device(_) => {
                let mut status = self.common.status.write_poisoned();
                *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
                Ok(())
            }
            LockedState::Socket(mut state, conf) => {
                let state = &mut *state;
                state.deactivate(&self.id, conf, &self.common.status, run_info)
            }
            LockedState::Service(mut state, conf) => {
                let state = &mut *state;
                state.deactivate(&self.id, conf, &self.common.status, run_info)
            }
            LockedState::Mount(_, conf) => deactivate_mount(&self.id, conf, &self.common.status),
            LockedState::Swap(_, conf) => deactivate_swap(&self.id, conf, &self.common.status),
            LockedState::Timer(_, _) | LockedState::Path(_, _) => {
                let mut status = self.common.status.write_poisoned();
                *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
                Ok(())
            }
        };

        if result.is_ok() {
            // Record lifecycle timestamp: entered inactive state
            self.common
                .timestamps
                .write_poisoned()
                .record_inactive_enter();
            let desc = &self.common.unit.description;
            let msg = if desc.is_empty() {
                format!("Deactivated successfully: {}.", self.id.name)
            } else {
                format!("Deactivated successfully: {desc}.")
            };
            let log_level_max = self.log_level_max();
            crate::control::varlink::journal_log_unit_lifecycle(
                &msg,
                &self.id.name,
                log_level_max.as_deref(),
            );
        }

        result
    }

    /// This rectivates the unit and manages the state transitions. It reports back any
    /// errors encountered while stopping the unit.
    ///
    /// If the unit was stopped this just calls activate.
    pub fn reactivate(
        &self,
        run_info: &RuntimeInfo,
        source: ActivationSource,
    ) -> Result<(), UnitOperationError> {
        trace!("Reactivate unit: {}", self.id.name);

        let state = match &self.specific {
            Specific::Service(specific) => {
                LockedState::Service(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Socket(specific) => {
                LockedState::Socket(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Target(specific) => LockedState::Target(specific.state.write_poisoned()),
            Specific::Slice(specific) => LockedState::Slice(specific.state.write_poisoned()),
            Specific::Mount(specific) => {
                LockedState::Mount(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Swap(specific) => {
                LockedState::Swap(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Timer(specific) => {
                LockedState::Timer(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Path(specific) => {
                LockedState::Path(specific.state.write_poisoned(), &specific.conf)
            }
            Specific::Device(specific) => LockedState::Device(specific.state.write_poisoned()),
        };

        let need_full_restart = self.state_transition_restarting(run_info).map_err(|bad_ids| {
            trace!(
                "Unit: {} ignores deactivation. Not all units depending on this unit have been started (still waiting for: {:?})",
                self.id.name,
                bad_ids,
            );
            UnitOperationError {
                reason: UnitOperationErrorReason::DependencyError(bad_ids),
                unit_name: self.id.name.clone(),
                unit_id: self.id.clone(),
            }
        })?;

        if need_full_restart {
            match state {
                LockedState::Target(_) | LockedState::Slice(_) | LockedState::Device(_) => {
                    let mut status = self.common.status.write_poisoned();
                    *status = UnitStatus::Started(StatusStarted::Running);
                    Ok(())
                }
                LockedState::Socket(mut state, conf) => {
                    let state = &mut *state;
                    state.reactivate(&self.id, conf, &self.common.status, run_info)
                }
                LockedState::Service(mut state, conf) => {
                    let state = &mut *state;
                    state.srvc.join_namespace_pid = crate::services::resolve_joins_namespace_of(
                        &self.id.name,
                        &self.common.unit.joins_namespace_of,
                        run_info,
                    );
                    state.reactivate(
                        &self.id,
                        conf,
                        &self.common.status,
                        run_info,
                        source,
                        &self.common.invocation_id,
                    )
                }
                LockedState::Mount(_, conf) => {
                    deactivate_mount(&self.id, conf, &self.common.status).ok();
                    activate_mount(&self.id, conf, &self.common.status).map(|_| ())
                }
                LockedState::Swap(_, conf) => {
                    deactivate_swap(&self.id, conf, &self.common.status).ok();
                    activate_swap(&self.id, conf, &self.common.status).map(|_| ())
                }
                LockedState::Timer(_, _) | LockedState::Path(_, _) => {
                    let mut status = self.common.status.write_poisoned();
                    *status = UnitStatus::Started(StatusStarted::Running);
                    Ok(())
                }
            }
        } else {
            match state {
                LockedState::Target(_) | LockedState::Slice(_) | LockedState::Device(_) => {
                    let mut status = self.common.status.write_poisoned();
                    *status = UnitStatus::Started(StatusStarted::Running);
                    Ok(())
                }
                LockedState::Socket(mut state, conf) => {
                    let state = &mut *state;
                    state
                        .activate(&self.id, conf, &self.common.status, run_info)
                        .map(|_| ())
                }
                LockedState::Service(mut state, conf) => {
                    let state = &mut *state;
                    // During a restart where the service is no longer in
                    // Started state (e.g. exited and got socket-activated),
                    // clear any stale PID/PGID so activate() doesn't fail
                    // with AlreadyHasPID.  Kill the orphan first.
                    if let Some(stale_pid) = state.srvc.pid {
                        log::debug!(
                            "reactivate (activate path) {}: killing stale PID {} before activate",
                            self.id.name,
                            stale_pid
                        );
                        let _ =
                            nix::sys::signal::kill(stale_pid, nix::sys::signal::Signal::SIGKILL);
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(2);
                        loop {
                            {
                                let pt = run_info.pid_table.lock_poisoned();
                                match pt.get(&stale_pid) {
                                    Some(PidEntry::ServiceExited(_)) | None => break,
                                    _ => {}
                                }
                            }
                            if std::time::Instant::now() >= deadline {
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                    state.srvc.pid = None;
                    state.srvc.process_group = None;
                    state
                        .activate(
                            &self.id,
                            conf,
                            &self.common.status,
                            run_info,
                            source,
                            &self.common.invocation_id,
                        )
                        .map(|_| ())
                }
                LockedState::Mount(_, conf) => {
                    activate_mount(&self.id, conf, &self.common.status).map(|_| ())
                }
                LockedState::Swap(_, conf) => {
                    activate_swap(&self.id, conf, &self.common.status).map(|_| ())
                }
                LockedState::Timer(_, _) | LockedState::Path(_, _) => {
                    let mut status = self.common.status.write_poisoned();
                    *status = UnitStatus::Started(StatusStarted::Running);
                    Ok(())
                }
            }
        }
    }
}

/// Perform the mount(2) syscall for a mount unit.
///
/// This creates the mount point directory if needed, then calls mount(2)
/// with the parameters from the `[Mount]` section. On success the unit
/// status is set to `Started(Running)`; on failure it is set to
/// `Stopped(StoppedUnexpected)`.
#[cfg(target_os = "linux")]
fn activate_mount(
    id: &UnitId,
    conf: &MountConfig,
    status: &RwLock<UnitStatus>,
) -> Result<UnitStatus, UnitOperationError> {
    let where_path = std::path::Path::new(&conf.where_);

    // Check if already mounted by reading /proc/mounts
    if is_already_mounted(&conf.where_) {
        info!(
            "Mount point {} is already mounted, marking as active",
            conf.where_
        );
        let mut status = status.write_poisoned();
        *status = UnitStatus::Started(StatusStarted::Running);
        return Ok(UnitStatus::Started(StatusStarted::Running));
    }

    // Create mount point directory if it doesn't exist
    if !where_path.exists() {
        let mode = conf.directory_mode;
        trace!(
            "Creating mount point directory {} with mode {:o}",
            conf.where_, mode
        );
        if let Err(e) = std::fs::create_dir_all(where_path) {
            let reason = UnitOperationErrorReason::GenericStartError(format!(
                "Failed to create mount point {}: {}",
                conf.where_, e
            ));
            let mut status = status.write_poisoned();
            *status = UnitStatus::Stopped(StatusStopped::StoppedUnexpected, vec![reason.clone()]);
            return Err(UnitOperationError {
                reason,
                unit_name: id.name.clone(),
                unit_id: id.clone(),
            });
        }
        // Set directory permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(where_path, std::fs::Permissions::from_mode(mode));
        }
    }

    // Build mount flags from Options=
    let mut flags = nix::mount::MsFlags::empty();
    let mut filtered_options: Vec<String> = Vec::new();

    if let Some(ref options) = conf.options {
        for opt in options.split(',') {
            let opt = opt.trim();
            match opt {
                "ro" | "rdonly" => flags |= nix::mount::MsFlags::MS_RDONLY,
                "rw" => flags &= !nix::mount::MsFlags::MS_RDONLY,
                "nosuid" => flags |= nix::mount::MsFlags::MS_NOSUID,
                "suid" => flags &= !nix::mount::MsFlags::MS_NOSUID,
                "nodev" => flags |= nix::mount::MsFlags::MS_NODEV,
                "dev" => flags &= !nix::mount::MsFlags::MS_NODEV,
                "noexec" => flags |= nix::mount::MsFlags::MS_NOEXEC,
                "exec" => flags &= !nix::mount::MsFlags::MS_NOEXEC,
                "sync" => flags |= nix::mount::MsFlags::MS_SYNCHRONOUS,
                "async" => flags &= !nix::mount::MsFlags::MS_SYNCHRONOUS,
                "remount" => flags |= nix::mount::MsFlags::MS_REMOUNT,
                "bind" => flags |= nix::mount::MsFlags::MS_BIND,
                "rbind" => flags |= nix::mount::MsFlags::MS_BIND | nix::mount::MsFlags::MS_REC,
                "move" => flags |= nix::mount::MsFlags::MS_MOVE,
                "noatime" => flags |= nix::mount::MsFlags::MS_NOATIME,
                "nodiratime" => flags |= nix::mount::MsFlags::MS_NODIRATIME,
                "relatime" => flags |= nix::mount::MsFlags::MS_RELATIME,
                "strictatime" => flags |= nix::mount::MsFlags::MS_STRICTATIME,
                "lazytime" => flags |= nix::mount::MsFlags::MS_LAZYTIME,
                "silent" => flags |= nix::mount::MsFlags::MS_SILENT,
                "loud" => flags &= !nix::mount::MsFlags::MS_SILENT,
                "dirsync" => flags |= nix::mount::MsFlags::MS_DIRSYNC,
                "mand" => flags |= nix::mount::MsFlags::MS_MANDLOCK,
                "nomand" => flags &= !nix::mount::MsFlags::MS_MANDLOCK,
                "private" => flags |= nix::mount::MsFlags::MS_PRIVATE,
                "rprivate" => {
                    flags |= nix::mount::MsFlags::MS_PRIVATE | nix::mount::MsFlags::MS_REC
                }
                "shared" => flags |= nix::mount::MsFlags::MS_SHARED,
                "rshared" => flags |= nix::mount::MsFlags::MS_SHARED | nix::mount::MsFlags::MS_REC,
                "slave" => flags |= nix::mount::MsFlags::MS_SLAVE,
                "rslave" => flags |= nix::mount::MsFlags::MS_SLAVE | nix::mount::MsFlags::MS_REC,
                "unbindable" => flags |= nix::mount::MsFlags::MS_UNBINDABLE,
                "runbindable" => {
                    flags |= nix::mount::MsFlags::MS_UNBINDABLE | nix::mount::MsFlags::MS_REC
                }
                "defaults" => { /* defaults = rw,suid,dev,exec,auto,nouser,async */ }
                "auto"
                | "noauto"
                | "user"
                | "nouser"
                | "users"
                | "group"
                | "_netdev"
                | "comment"
                | "x-systemd.automount"
                | "nofail" => {
                    // These are fstab-only options, not passed to mount(2)
                }
                _ => {
                    // Pass unknown options through as data for the filesystem driver
                    filtered_options.push(opt.to_owned());
                }
            }
        }
    }

    if conf.read_write_only {
        flags &= !nix::mount::MsFlags::MS_RDONLY;
    }

    let data_str = if filtered_options.is_empty() {
        None
    } else {
        Some(filtered_options.join(","))
    };

    let what: Option<&str> = if conf.what.is_empty() {
        None
    } else {
        Some(&conf.what)
    };
    let fs_type: Option<&str> = conf.fs_type.as_deref();
    let data: Option<&str> = data_str.as_deref();

    info!(
        "Mounting {} on {} (type={}, flags={:?}, data={:?})",
        conf.what,
        conf.where_,
        fs_type.unwrap_or("auto"),
        flags,
        data
    );

    match nix::mount::mount(what, conf.where_.as_str(), fs_type, flags, data) {
        Ok(()) => {
            info!("Successfully mounted {} on {}", conf.what, conf.where_);
            let mut status = status.write_poisoned();
            *status = UnitStatus::Started(StatusStarted::Running);
            Ok(UnitStatus::Started(StatusStarted::Running))
        }
        Err(e) => {
            error!("Failed to mount {} on {}: {}", conf.what, conf.where_, e);
            let reason = UnitOperationErrorReason::GenericStartError(format!(
                "mount({}, {}, {:?}): {}",
                conf.what, conf.where_, fs_type, e
            ));
            let mut status = status.write_poisoned();
            *status = UnitStatus::Stopped(StatusStopped::StoppedUnexpected, vec![reason.clone()]);
            Err(UnitOperationError {
                reason,
                unit_name: id.name.clone(),
                unit_id: id.clone(),
            })
        }
    }
}

/// Non-Linux stub for mount activation — always succeeds and marks the unit
/// as started.
#[cfg(not(target_os = "linux"))]
fn activate_mount(
    id: &UnitId,
    _conf: &MountConfig,
    status: &RwLock<UnitStatus>,
) -> Result<UnitStatus, UnitOperationError> {
    trace!("Mount activation is a no-op on non-Linux ({})", id.name);
    let mut status = status.write_poisoned();
    *status = UnitStatus::Started(StatusStarted::Running);
    Ok(UnitStatus::Started(StatusStarted::Running))
}

/// Perform the umount(2) syscall for a mount unit.
#[cfg(target_os = "linux")]
fn deactivate_mount(
    id: &UnitId,
    conf: &MountConfig,
    status: &RwLock<UnitStatus>,
) -> Result<(), UnitOperationError> {
    // If not currently mounted, just mark as stopped
    if !is_already_mounted(&conf.where_) {
        trace!(
            "Mount point {} is not mounted, nothing to unmount",
            conf.where_
        );
        let mut status = status.write_poisoned();
        *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
        return Ok(());
    }

    let mut umount_flags = nix::mount::MntFlags::empty();
    if conf.lazy_unmount {
        umount_flags |= nix::mount::MntFlags::MNT_DETACH;
    }
    if conf.force_unmount {
        umount_flags |= nix::mount::MntFlags::MNT_FORCE;
    }

    info!("Unmounting {} (flags={:?})", conf.where_, umount_flags);

    match nix::mount::umount2(conf.where_.as_str(), umount_flags) {
        Ok(()) => {
            info!("Successfully unmounted {}", conf.where_);
            let mut status = status.write_poisoned();
            *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
            Ok(())
        }
        Err(e) => {
            error!("Failed to unmount {}: {}", conf.where_, e);
            let reason = UnitOperationErrorReason::GenericStopError(format!(
                "umount({}): {}",
                conf.where_, e
            ));
            let mut status = status.write_poisoned();
            *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![reason.clone()]);
            Err(UnitOperationError {
                reason,
                unit_name: id.name.clone(),
                unit_id: id.clone(),
            })
        }
    }
}

/// Non-Linux stub for mount deactivation.
#[cfg(not(target_os = "linux"))]
fn deactivate_mount(
    id: &UnitId,
    _conf: &MountConfig,
    status: &RwLock<UnitStatus>,
) -> Result<(), UnitOperationError> {
    trace!("Mount deactivation is a no-op on non-Linux ({})", id.name);
    let mut status = status.write_poisoned();
    *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
    Ok(())
}

/// Check whether a path is already mounted by reading /proc/mounts.
#[cfg(target_os = "linux")]
fn is_already_mounted(path: &str) -> bool {
    let normalized = path.trim_end_matches('/');
    let check_path = if normalized.is_empty() {
        "/"
    } else {
        normalized
    };

    match std::fs::read_to_string("/proc/mounts") {
        Ok(contents) => {
            for line in contents.lines() {
                // /proc/mounts format: device mountpoint fstype options dump pass
                let mut fields = line.split_whitespace();
                if let Some(_device) = fields.next()
                    && let Some(mountpoint) = fields.next()
                {
                    let mp = mountpoint.trim_end_matches('/');
                    let mp = if mp.is_empty() { "/" } else { mp };
                    if mp == check_path {
                        return true;
                    }
                }
            }
            false
        }
        Err(_) => false,
    }
}

/// Check whether a device/file is already active as swap by reading /proc/swaps.
#[cfg(target_os = "linux")]
fn is_already_swapped(what: &str) -> bool {
    match std::fs::read_to_string("/proc/swaps") {
        Ok(contents) => {
            for line in contents.lines().skip(1) {
                // /proc/swaps format: Filename Type Size Used Priority
                if let Some(filename) = line.split_whitespace().next()
                    && filename == what
                {
                    return true;
                }
            }
            false
        }
        Err(_) => false,
    }
}

#[cfg(not(target_os = "linux"))]
fn is_already_swapped(_what: &str) -> bool {
    false
}

/// Activate a swap unit by calling swapon(2).
///
/// If the swap device is already active, the unit is simply marked as running.
/// The `Priority=` setting is passed via the `SWAP_FLAG_PREFER` flag and the
/// priority value encoded in the flags argument to swapon(2).
#[cfg(target_os = "linux")]
fn activate_swap(
    id: &UnitId,
    conf: &SwapConfig,
    status: &RwLock<UnitStatus>,
) -> Result<UnitStatus, UnitOperationError> {
    // Check if already active
    if is_already_swapped(&conf.what) {
        info!(
            "Swap device {} is already active, marking as started",
            conf.what
        );
        let mut status = status.write_poisoned();
        *status = UnitStatus::Started(StatusStarted::Running);
        return Ok(UnitStatus::Started(StatusStarted::Running));
    }

    // Build swapon flags
    // SWAP_FLAG_PREFER = 0x8000, priority is in the low 15 bits (0–32767)
    const SWAP_FLAG_PREFER: libc::c_int = 0x8000;
    const SWAP_FLAG_DISCARD: libc::c_int = 0x10000;

    let mut flags: libc::c_int = 0;

    if let Some(pri) = conf.priority
        && pri >= 0
    {
        flags |= SWAP_FLAG_PREFER | (pri & 0x7FFF) as libc::c_int;
    }
    // Negative priorities are handled by the kernel default when
    // SWAP_FLAG_PREFER is not set.

    // Check options for "discard"
    if let Some(ref options) = conf.options {
        for opt in options.split(',') {
            let opt = opt.trim();
            match opt {
                "discard" => flags |= SWAP_FLAG_DISCARD,
                _ if opt.starts_with("pri=") => {
                    // pri=N in options overrides Priority= directive
                    if let Ok(p) = opt[4..].parse::<i32>()
                        && p >= 0
                    {
                        flags = (flags & !0x7FFF) | SWAP_FLAG_PREFER | (p & 0x7FFF) as libc::c_int;
                    }
                }
                _ => {
                    // Other options are logged but not actionable via swapon(2)
                    trace!("Swap option ignored for swapon(2): {}", opt);
                }
            }
        }
    }

    let what_cstr = match std::ffi::CString::new(conf.what.as_str()) {
        Ok(c) => c,
        Err(e) => {
            let reason = UnitOperationErrorReason::GenericStartError(format!(
                "Invalid swap path {}: {}",
                conf.what, e
            ));
            let mut status = status.write_poisoned();
            *status = UnitStatus::Stopped(StatusStopped::StoppedUnexpected, vec![reason.clone()]);
            return Err(UnitOperationError {
                reason,
                unit_name: id.name.clone(),
                unit_id: id.clone(),
            });
        }
    };

    info!("Activating swap {} (flags=0x{:x})", conf.what, flags);

    // SAFETY: swapon(2) is a standard Linux syscall. The path is a valid
    // NUL-terminated C string and flags is a simple integer bitmask.
    let ret = unsafe { libc::swapon(what_cstr.as_ptr(), flags) };

    if ret == 0 {
        info!("Successfully activated swap {}", conf.what);
        let mut status = status.write_poisoned();
        *status = UnitStatus::Started(StatusStarted::Running);
        Ok(UnitStatus::Started(StatusStarted::Running))
    } else {
        let errno = std::io::Error::last_os_error();
        error!("Failed to activate swap {}: {}", conf.what, errno);
        let reason = UnitOperationErrorReason::GenericStartError(format!(
            "swapon({}): {}",
            conf.what, errno
        ));
        let mut status = status.write_poisoned();
        *status = UnitStatus::Stopped(StatusStopped::StoppedUnexpected, vec![reason.clone()]);
        Err(UnitOperationError {
            reason,
            unit_name: id.name.clone(),
            unit_id: id.clone(),
        })
    }
}

/// Non-Linux stub for swap activation — always succeeds and marks the unit
/// as started.
#[cfg(not(target_os = "linux"))]
fn activate_swap(
    id: &UnitId,
    _conf: &SwapConfig,
    status: &RwLock<UnitStatus>,
) -> Result<UnitStatus, UnitOperationError> {
    trace!("Swap activation is a no-op on non-Linux ({})", id.name);
    let mut status = status.write_poisoned();
    *status = UnitStatus::Started(StatusStarted::Running);
    Ok(UnitStatus::Started(StatusStarted::Running))
}

/// Deactivate a swap unit by calling swapoff(2).
#[cfg(target_os = "linux")]
fn deactivate_swap(
    id: &UnitId,
    conf: &SwapConfig,
    status: &RwLock<UnitStatus>,
) -> Result<(), UnitOperationError> {
    // If not currently active, just mark as stopped
    if !is_already_swapped(&conf.what) {
        trace!(
            "Swap device {} is not active, nothing to deactivate",
            conf.what
        );
        let mut status = status.write_poisoned();
        *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
        return Ok(());
    }

    let what_cstr = match std::ffi::CString::new(conf.what.as_str()) {
        Ok(c) => c,
        Err(e) => {
            let reason = UnitOperationErrorReason::GenericStopError(format!(
                "Invalid swap path {}: {}",
                conf.what, e
            ));
            let mut status = status.write_poisoned();
            *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![reason.clone()]);
            return Err(UnitOperationError {
                reason,
                unit_name: id.name.clone(),
                unit_id: id.clone(),
            });
        }
    };

    info!("Deactivating swap {}", conf.what);

    // SAFETY: swapoff(2) is a standard Linux syscall. The path is a valid
    // NUL-terminated C string.
    let ret = unsafe { libc::swapoff(what_cstr.as_ptr()) };

    if ret == 0 {
        info!("Successfully deactivated swap {}", conf.what);
        let mut status = status.write_poisoned();
        *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
        Ok(())
    } else {
        let errno = std::io::Error::last_os_error();
        error!("Failed to deactivate swap {}: {}", conf.what, errno);
        let reason = UnitOperationErrorReason::GenericStopError(format!(
            "swapoff({}): {}",
            conf.what, errno
        ));
        let mut status = status.write_poisoned();
        *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![reason.clone()]);
        Err(UnitOperationError {
            reason,
            unit_name: id.name.clone(),
            unit_id: id.clone(),
        })
    }
}

/// Non-Linux stub for swap deactivation.
#[cfg(not(target_os = "linux"))]
fn deactivate_swap(
    id: &UnitId,
    _conf: &SwapConfig,
    status: &RwLock<UnitStatus>,
) -> Result<(), UnitOperationError> {
    trace!("Swap deactivation is a no-op on non-Linux ({})", id.name);
    let mut status = status.write_poisoned();
    *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
    Ok(())
}

#[derive(Debug, Clone)]
pub struct UnitConfig {
    pub description: String,
    pub documentation: Vec<String>,

    /// The path to the unit file on disk (systemd calls this `FragmentPath`).
    /// `None` for units synthesised at runtime (e.g. by generators) that have
    /// no backing file, or for units whose path was not recorded at parse time.
    pub fragment_path: Option<PathBuf>,

    /// This is needed for adding/removing units. All units in this set must be present
    /// or this unit is considered invalid os it has to be removed too / cannot be added.
    pub refs_by_name: Vec<UnitId>,

    /// Whether to add implicit default dependencies (e.g. on sysinit.target / shutdown.target).
    /// Defaults to true, matching systemd behavior.
    pub default_dependencies: bool,

    /// Conditions that must all be true for the unit to activate.
    /// If any condition fails, the unit is skipped (not treated as an error).
    /// Matches systemd's ConditionPathExists=, ConditionPathIsDirectory=, etc.
    pub conditions: Vec<UnitCondition>,

    /// Assertions that must all be true for the unit to activate.
    /// Unlike conditions, if any assertion fails the unit enters a **failed**
    /// state (not silently skipped). Uses the same `UnitCondition` type since
    /// the check logic is identical — only the failure semantics differ.
    /// Matches systemd's AssertPathExists=, AssertPathIsDirectory=, etc.
    pub assertions: Vec<UnitCondition>,

    /// Action to take when the unit finishes successfully.
    /// Matches systemd's `SuccessAction=` setting.
    pub success_action: UnitAction,

    /// Action to take when the unit fails.
    /// Matches systemd's `FailureAction=` setting.
    pub failure_action: UnitAction,

    /// Alternative names for this unit from `Alias=` in the `[Install]` section.
    /// In systemd, these create symlinks when the unit is enabled.
    /// In rust-systemd, units can be looked up by any of their aliases.
    pub aliases: Vec<String>,

    /// If true, this unit will not be stopped when isolating to another target.
    /// Defaults to false, matching systemd's `IgnoreOnIsolate=` setting.
    pub ignore_on_isolate: bool,

    /// Default instance name for template units (e.g. `foo@.service`).
    /// When a template is enabled without an explicit instance, this value is used.
    /// Matches systemd's `DefaultInstance=` setting in the `[Install]` section.
    pub default_instance: Option<String>,

    /// If true, this unit may be used with `systemctl isolate`.
    /// Defaults to false, matching systemd's `AllowIsolate=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub allow_isolate: bool,

    /// Timeout before a job for this unit is cancelled.
    /// Matches systemd's `JobTimeoutSec=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub job_timeout_sec: Option<Timeout>,

    /// Action to take when a job for this unit times out.
    /// Matches systemd's `JobTimeoutAction=` setting.
    /// Uses the same action values as `SuccessAction=`/`FailureAction=`.
    /// Parsed and stored; no runtime enforcement yet.
    pub job_timeout_action: UnitAction,

    /// If true, this unit may not be started manually (e.g. via `systemctl start`).
    /// It can only be started as a dependency of another unit.
    /// Defaults to false, matching systemd's `RefuseManualStart=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub refuse_manual_start: bool,

    /// If true, this unit may not be stopped manually (e.g. via `systemctl stop`).
    /// It can only be stopped as a dependency of another unit.
    /// Defaults to false, matching systemd's `RefuseManualStop=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub refuse_manual_stop: bool,

    /// Units to activate when this unit finishes successfully.
    /// Matches systemd's `OnSuccess=` setting.
    pub on_success: Vec<String>,

    /// Job mode for enqueuing OnSuccess= units.
    /// Defaults to `Replace`, matching systemd's default.
    /// Matches systemd's `OnSuccessJobMode=` setting.
    pub on_success_job_mode: OnFailureJobMode,

    /// Units to activate when this unit enters the "failed" state.
    /// Matches systemd's `OnFailure=` setting.
    pub on_failure: Vec<String>,

    /// Job mode for enqueuing OnFailure= units.
    /// Defaults to `Replace`, matching systemd's default.
    /// Matches systemd's `OnFailureJobMode=` setting.
    pub on_failure_job_mode: OnFailureJobMode,

    /// Time interval for rate limiting unit starts.
    /// If the unit is started more than `start_limit_burst` times within this interval,
    /// further starts are denied.
    /// Matches systemd's `StartLimitIntervalSec=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub start_limit_interval_sec: Option<Timeout>,

    /// Number of starts allowed within the `start_limit_interval_sec` interval.
    /// Defaults to `None` (use manager default, typically 5).
    /// Matches systemd's `StartLimitBurst=` setting.
    /// Parsed and stored; no runtime enforcement yet.
    pub start_limit_burst: Option<u32>,

    /// Action to take when the start rate limit is hit.
    /// Matches systemd's `StartLimitAction=` setting.
    /// Uses the same action values as `SuccessAction=`/`FailureAction=`.
    /// Parsed and stored; no runtime enforcement yet.
    pub start_limit_action: UnitAction,

    /// Timestamp when this unit's configuration was last loaded (parsed from disk).
    /// Used by `NeedDaemonReload` to detect on-disk changes since the last reload.
    pub loaded_at: std::time::SystemTime,

    /// Paths to drop-in `.conf` files that were applied when this unit was loaded.
    /// Used by `NeedDaemonReload` to detect added or removed drop-in files.
    pub loaded_dropin_files: Vec<PathBuf>,

    /// Units that share the same mount namespace as this unit.
    /// Matches systemd's `JoinsNamespaceOf=` setting.
    /// Resolved at start time to find a running service's PID to join via setns(2).
    pub joins_namespace_of: Vec<String>,
}

#[derive(Debug, Clone)]
/// This are the runtime dependencies. They are extended when the unit is added into the unit set
/// so all dependencies go both ways.
///
/// These vecs are meant like this:
/// `Dependencies::after`: this unit should start after these units have been started
/// `Dependencies::before`: this unit should start before these units have been started
/// ....
pub struct Dependencies {
    pub wants: Vec<UnitId>,
    pub wanted_by: Vec<UnitId>,
    pub requires: Vec<UnitId>,
    pub required_by: Vec<UnitId>,
    pub conflicts: Vec<UnitId>,
    pub conflicted_by: Vec<UnitId>,
    pub before: Vec<UnitId>,
    pub after: Vec<UnitId>,

    /// Units this unit is "part of". When the listed units are stopped or
    /// restarted, this unit is also stopped or restarted.
    /// Matches systemd's `PartOf=` setting.
    pub part_of: Vec<UnitId>,
    /// Reverse of `part_of`: units that declared `PartOf=` pointing to this unit.
    /// When this unit is stopped or restarted, all `part_of_by` units are too.
    pub part_of_by: Vec<UnitId>,

    /// Units this unit "binds to". Similar to `Requires=`, but additionally
    /// causes this unit to stop when the listed units stop (even cleanly).
    /// Matches systemd's `BindsTo=` setting.
    /// Parsed and stored; stop-propagation not yet enforced at runtime.
    pub binds_to: Vec<UnitId>,
    /// Reverse of `binds_to`: units that declared `BindsTo=` pointing to this unit.
    /// When this unit stops, all `bound_by` units should also stop.
    pub bound_by: Vec<UnitId>,

    /// Units this unit "upholds". As long as this unit is active, the listed
    /// units will be restarted if they stop. Acts like `Wants=` for activation.
    /// Matches systemd's `Upholds=` setting.
    pub upholds: Vec<UnitId>,
    /// Reverse of `upholds`: units that declared `Upholds=` pointing to this unit.
    /// When this unit stops, any active unit in `upheld_by` will restart it.
    pub upheld_by: Vec<UnitId>,

    /// Units to propagate stop requests to.
    /// When this unit is stopped, the listed units are also stopped.
    /// Matches systemd's `PropagatesStopTo=` setting.
    pub propagates_stop_to: Vec<UnitId>,
}

impl Dependencies {
    pub fn dedup(&mut self) {
        self.wants.sort();
        self.wanted_by.sort();
        self.required_by.sort();
        self.conflicts.sort();
        self.conflicted_by.sort();
        self.before.sort();
        self.after.sort();
        self.requires.sort();
        self.part_of.sort();
        self.part_of_by.sort();
        self.binds_to.sort();
        self.bound_by.sort();
        self.upholds.sort();
        self.upheld_by.sort();
        self.propagates_stop_to.sort();
        // dedup after sorting
        self.wants.dedup();
        self.requires.dedup();
        self.wanted_by.dedup();
        self.required_by.dedup();
        self.conflicts.dedup();
        self.conflicted_by.dedup();
        self.before.dedup();
        self.after.dedup();
        self.part_of.dedup();
        self.part_of_by.dedup();
        self.binds_to.dedup();
        self.bound_by.dedup();
        self.upholds.dedup();
        self.upheld_by.dedup();
        self.propagates_stop_to.dedup();
    }

    #[must_use]
    pub fn kill_before_this(&self) -> Vec<UnitId> {
        let mut ids = Vec::new();
        ids.extend(self.required_by.iter().cloned());
        // Units that declared PartOf= this unit should also stop when this unit stops
        ids.extend(self.part_of_by.iter().cloned());
        // Units that declared BindsTo= this unit should also stop when this unit stops
        ids.extend(self.bound_by.iter().cloned());
        ids
    }
    /// Return units that must be started before this one AND that this unit
    /// actually pulls in (via Wants=/Requires=/BindsTo=).
    ///
    /// Pure `After=` ordering without a pull-dep does NOT cause a unit to be
    /// included in the activation subgraph — it only affects ordering IF
    /// both units happen to be activated.  This matches real systemd behavior
    /// where `After=rescue.target` on `multi-user.target` does NOT cause
    /// `rescue.target` to be activated during normal boot.
    #[must_use]
    pub fn start_before_this(&self) -> Vec<UnitId> {
        // Only return After= deps that are also pull-deps.
        // Units that are only in `after` (pure ordering) are NOT included —
        // they don't need to be started, only ordered if they happen to start.
        self.after
            .iter()
            .filter(|id| {
                self.wants.contains(id)
                    || self.requires.contains(id)
                    || self.binds_to.contains(id)
                    || self.upholds.contains(id)
            })
            .cloned()
            .collect()
    }
    #[must_use]
    pub fn start_concurrently_with_this(&self) -> Vec<UnitId> {
        let mut ids = Vec::new();
        ids.extend(self.wants.iter().cloned());
        ids.extend(self.requires.iter().cloned());
        // BindsTo= implies the same start dependency as Requires=
        ids.extend(self.binds_to.iter().cloned());
        // Upholds= implies a Wants=-like start dependency
        ids.extend(self.upholds.iter().cloned());

        ids.into_iter()
            .filter(|id| !self.after.contains(id))
            .collect()
    }

    /// Remove all occurrences of this id from the vec
    fn remove_from_vec(ids: &mut Vec<UnitId>, id: &UnitId) {
        while let Some(idx) = ids.iter().position(|e| *e == *id) {
            ids.remove(idx);
        }
    }

    pub fn remove_id(&mut self, id: &UnitId) {
        Self::remove_from_vec(&mut self.wants, id);
        Self::remove_from_vec(&mut self.wanted_by, id);
        Self::remove_from_vec(&mut self.requires, id);
        Self::remove_from_vec(&mut self.required_by, id);
        Self::remove_from_vec(&mut self.conflicts, id);
        Self::remove_from_vec(&mut self.conflicted_by, id);
        Self::remove_from_vec(&mut self.before, id);
        Self::remove_from_vec(&mut self.after, id);
        Self::remove_from_vec(&mut self.part_of, id);
        Self::remove_from_vec(&mut self.part_of_by, id);
        Self::remove_from_vec(&mut self.binds_to, id);
        Self::remove_from_vec(&mut self.bound_by, id);
        Self::remove_from_vec(&mut self.upholds, id);
        Self::remove_from_vec(&mut self.upheld_by, id);
        Self::remove_from_vec(&mut self.propagates_stop_to, id);
    }

    #[must_use]
    pub fn comes_after(&self, name: &str) -> bool {
        for id in &self.after {
            if id.eq(name) {
                return true;
            }
        }
        false
    }
    #[must_use]
    pub fn comes_before(&self, name: &str) -> bool {
        for id in &self.before {
            if id.eq(name) {
                return true;
            }
        }
        false
    }
    #[must_use]
    pub fn requires(&self, name: &str) -> bool {
        for id in &self.requires {
            if id.eq(name) {
                return true;
            }
        }
        false
    }
    #[must_use]
    pub fn required_by(&self, name: &str) -> bool {
        for id in &self.required_by {
            if id.eq(name) {
                return true;
            }
        }
        false
    }
    #[must_use]
    pub fn wants(&self, name: &str) -> bool {
        for id in &self.wants {
            if id.eq(name) {
                return true;
            }
        }
        false
    }
    #[must_use]
    pub fn wanted_by(&self, name: &str) -> bool {
        for id in &self.wanted_by {
            if id.eq(name) {
                return true;
            }
        }
        false
    }
}

/// Describes a single socket that should be opened. One Socket unit may contain multiple of these
#[derive(Clone, Debug)]
pub struct SingleSocketConfig {
    pub kind: SocketKind,
    pub specialized: SpecializedSocketConfig,
}

/// All settings from the Exec section of a unit
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct ExecConfig {
    /// Raw user name or numeric UID from User= directive. Resolved at exec time.
    pub user: Option<String>,
    /// Raw group name or numeric GID from Group= directive. Resolved at exec time.
    pub group: Option<String>,
    /// Raw supplementary group names or numeric GIDs. Resolved at exec time.
    pub supplementary_groups: Vec<String>,
    pub stdin_option: StandardInput,
    pub stdout_path: Option<StdIoOption>,
    pub stderr_path: Option<StdIoOption>,
    pub environment: Option<EnvVars>,
    /// Paths from EnvironmentFile= directives. Each entry is (path, optional)
    /// where optional=true means a leading '-' was present (file may not exist).
    pub environment_files: Vec<(std::path::PathBuf, bool)>,
    pub working_directory: Option<std::path::PathBuf>,
    pub state_directory: Vec<String>,
    /// LogsDirectory= — directories to create under /var/log/ before the
    /// service starts. Ownership is set to the service user/group and the
    /// LOGS_DIRECTORY environment variable is set to a colon-separated
    /// list of the absolute paths. Matches systemd.exec(5).
    pub logs_directory: Vec<String>,
    /// LogsDirectoryMode= — the file system access mode to use when creating
    /// the logs directories specified with LogsDirectory=. Takes an access
    /// mode in octal notation (e.g. 0755). Defaults to 0755.
    /// Parsed and stored; runtime enforcement applies the mode at directory
    /// creation time. See systemd.exec(5).
    pub logs_directory_mode: Option<u32>,
    /// RuntimeDirectory= — directories to create under /run/ before the
    /// service starts. Ownership is set to the service user/group and the
    /// RUNTIME_DIRECTORY environment variable is set to a colon-separated
    /// list of the absolute paths. Matches systemd.exec(5).
    pub runtime_directory: Vec<String>,
    /// RuntimeDirectoryPreserve= — controls whether runtime directories
    /// (created by `RuntimeDirectory=`) are removed when the service stops.
    /// Can be `no` (default, always remove), `yes` (always preserve), or
    /// `restart` (preserve across restarts, remove on full stop). Parsed and
    /// stored; no runtime enforcement yet. See systemd.exec(5).
    pub runtime_directory_preserve: RuntimeDirectoryPreserve,
    pub tty_path: Option<std::path::PathBuf>,
    /// TTYReset= — reset the TTY to sane defaults before use.
    /// Matches systemd: resets termios, keyboard mode, switches to text mode.
    pub tty_reset: bool,
    /// TTYVHangup= — send TIOCVHANGUP to the TTY before use.
    /// Disconnects prior sessions so the new service gets a clean terminal.
    pub tty_vhangup: bool,
    /// TTYVTDisallocate= — deallocate or clear the VT before use.
    pub tty_vt_disallocate: bool,
    /// IgnoreSIGPIPE= — if true (the default), SIGPIPE is set to SIG_IGN before
    /// exec'ing the service binary. When false, the default SIGPIPE disposition
    /// (terminate) is left in place. Matches systemd.exec(5).
    pub ignore_sigpipe: bool,
    /// UtmpIdentifier= — the 4-character identifier string to write to the utmp
    /// and wtmp entries when the service runs on a TTY. Defaults to the TTY
    /// basename when unset. See systemd.exec(5).
    pub utmp_identifier: Option<String>,
    /// UtmpMode= — the type of utmp/wtmp record to write. Defaults to `Init`.
    /// See systemd.exec(5).
    pub utmp_mode: UtmpMode,
    /// ImportCredential= — glob patterns for credentials to import from the
    /// system credential store into the service's credential directory.
    /// Multiple patterns may be specified (the setting accumulates).
    /// See systemd.exec(5).
    pub import_credentials: Vec<String>,
    /// LoadCredential=ID:PATH — load a credential from a file at PATH and
    /// make it available in the service's credential directory under the name
    /// ID. Multiple directives accumulate; an empty assignment resets.
    /// See systemd.exec(5).
    pub load_credentials: Vec<(String, String)>,
    /// LoadCredentialEncrypted=ID:PATH — like LoadCredential= but the file
    /// content is expected to be encrypted with a system credential key.
    /// Parsed but decryption is not yet implemented (credential is loaded
    /// as-is). See systemd.exec(5).
    pub load_credentials_encrypted: Vec<(String, String)>,
    /// SetCredential=ID:DATA — set a credential from inline data. The data
    /// is written verbatim to the credential directory under the given ID.
    /// The DATA part may contain colons. Multiple directives accumulate;
    /// an empty assignment resets. See systemd.exec(5).
    pub set_credentials: Vec<(String, String)>,
    /// SetCredentialEncrypted=ID:DATA — like SetCredential= but the inline
    /// data is expected to be encrypted (base64-encoded). Parsed but
    /// decryption is not yet implemented. See systemd.exec(5).
    pub set_credentials_encrypted: Vec<(String, String)>,
    /// PassEnvironment= — a list of environment variable names to import from
    /// the system manager's (PID 1) environment into the service's execution
    /// environment. Only variables that are actually set in the manager's
    /// environment are passed; unset variables are silently ignored. Multiple
    /// directives accumulate; an empty assignment resets the list. Applied
    /// after Environment= but before internal variables and UnsetEnvironment=.
    /// See systemd.exec(5).
    pub pass_environment: Vec<String>,
    /// UnsetEnvironment= — a list of environment variable names or variable
    /// assignments (VAR=VALUE) to remove from the final environment passed to
    /// executed processes. If a plain name is given, any assignment with that
    /// name is removed regardless of value. If a VAR=VALUE assignment is given,
    /// only an exact match is removed. Applied as the final step when
    /// compiling the environment block. See systemd.exec(5).
    pub unset_environment: Vec<String>,
    /// OOMScoreAdjust= — sets the OOM score adjustment for executed processes.
    /// Takes an integer between -1000 (least likely to be killed) and 1000
    /// (most likely to be killed). Written to /proc/self/oom_score_adj before
    /// exec. See systemd.exec(5).
    pub oom_score_adjust: Option<i32>,
    /// LogExtraFields= — additional journal fields to include in log entries
    /// for this unit. Each entry is a KEY=VALUE string. Multiple directives
    /// accumulate. Parsed and stored; not yet used at runtime. See systemd.exec(5).
    pub log_extra_fields: Vec<String>,
    /// DynamicUser= — if true, a UNIX user and group pair is dynamically
    /// allocated for this unit at runtime and released when the unit is stopped.
    /// Defaults to false. Parsed and stored; no runtime enforcement yet.
    /// See systemd.exec(5).
    pub dynamic_user: bool,
    /// SystemCallFilter= — a list of syscall names or `@group` names for
    /// seccomp-based system-call filtering. Entries prefixed with `~` form a
    /// deny-list; without the prefix they form an allow-list. Multiple
    /// directives accumulate; an empty assignment resets the list. Parsed and
    /// stored; no runtime enforcement yet. See systemd.exec(5).
    pub system_call_filter: Vec<String>,
    /// SystemCallLog= — a list of syscall names or `@group` names for
    /// seccomp-based system-call logging (audit). Same syntax as
    /// `SystemCallFilter=`: entries prefixed with `~` exclude from logging;
    /// without the prefix they are logged. Multiple directives accumulate;
    /// an empty assignment resets the list. Parsed and stored; no runtime
    /// seccomp audit enforcement yet. See systemd.exec(5).
    pub system_call_log: Vec<String>,
    /// ProtectSystem= — controls whether the service has read-only access to
    /// the OS file system hierarchy. Parsed and stored; no runtime enforcement
    /// yet (requires mount namespace support). See systemd.exec(5).
    pub protect_system: ProtectSystem,
    /// RestrictNamespaces= — restricts access to Linux namespace types for the
    /// service. Can be a boolean (`yes` restricts all, `no` allows all) or a
    /// space-separated list of namespace type identifiers (cgroup, ipc, net,
    /// mnt, pid, user, uts). A `~` prefix inverts the list. Parsed and stored;
    /// no runtime seccomp enforcement yet. See systemd.exec(5).
    pub restrict_namespaces: RestrictNamespaces,
    /// RestrictFileSystems= — a list of Linux file system type names (e.g.
    /// `ext4`, `tmpfs`, `proc`, `btrfs`) for BPF LSM-based file system type
    /// restriction. Entries prefixed with `~` form a deny-list; without the
    /// prefix they form an allow-list. Multiple directives accumulate; an
    /// empty assignment resets the list. Parsed and stored; no runtime BPF
    /// LSM enforcement yet. See systemd.exec(5).
    pub restrict_file_systems: Vec<String>,
    /// RestrictRealtime= — if true, any attempts to enable realtime scheduling
    /// in a process of the unit are refused via seccomp. Defaults to false.
    /// Parsed and stored; no runtime seccomp enforcement yet. See systemd.exec(5).
    pub restrict_realtime: bool,
    /// RestrictAddressFamilies= — a list of address family names (e.g.
    /// AF_UNIX, AF_INET, AF_INET6) for seccomp-based socket address family
    /// filtering. Entries prefixed with `~` form a deny-list; without the
    /// prefix they form an allow-list. Multiple directives accumulate; an
    /// empty assignment resets the list. Parsed and stored; no runtime seccomp
    /// enforcement yet. See systemd.exec(5).
    pub restrict_address_families: Vec<String>,
    /// SystemCallErrorNumber= — the errno to return when a system call is
    /// blocked by `SystemCallFilter=`. Takes an errno name such as `EPERM`
    /// or `EACCES`. When not set, blocked calls result in SIGSYS (process
    /// kill). Parsed and stored; no runtime seccomp enforcement yet.
    /// See systemd.exec(5).
    pub system_call_error_number: Option<String>,
    /// NoNewPrivileges= — if true, ensures that the service process and all
    /// its children can never gain new privileges through execve() (e.g.
    /// via setuid/setgid bits or file capabilities). Defaults to false.
    /// Parsed and stored; no runtime enforcement yet. See systemd.exec(5).
    pub no_new_privileges: bool,
    /// ProtectControlGroups= — if true, the Linux Control Groups (cgroups)
    /// hierarchies accessible through /sys/fs/cgroup/ will be made read-only
    /// to all processes of the unit. Defaults to false. Parsed and stored;
    /// no runtime enforcement yet (requires mount namespace support).
    /// See systemd.exec(5).
    pub protect_control_groups: bool,
    /// ProtectKernelModules= — if true, explicit kernel module loading and
    /// unloading is denied. This also makes /usr/lib/modules/ inaccessible.
    /// Defaults to false. Parsed and stored; no runtime enforcement yet
    /// (requires mount namespace and seccomp support). See systemd.exec(5).
    pub protect_kernel_modules: bool,
    /// RestrictSUIDSGID= — if true, any attempts to set the set-user-ID
    /// (SUID) or set-group-ID (SGID) bits on files or directories will be
    /// denied. Defaults to false. Parsed and stored; no runtime enforcement
    /// yet (requires seccomp support). See systemd.exec(5).
    pub restrict_suid_sgid: bool,
    /// ProtectKernelLogs= — if true, access to the kernel log ring buffer
    /// (/dev/kmsg, /proc/kmsg, dmesg) is denied. Defaults to false. Parsed
    /// and stored; no runtime enforcement yet (requires mount namespace and
    /// seccomp support). See systemd.exec(5).
    pub protect_kernel_logs: bool,
    /// ProtectKernelTunables= — if true, kernel variables accessible through
    /// /proc/sys/, /sys/, /proc/sysrq-trigger, /proc/latency_stats,
    /// /proc/acpi, /proc/timer_stats, /proc/fs, and /proc/irq will be made
    /// read-only to all processes of the unit. Defaults to false. Parsed and
    /// stored; no runtime enforcement yet (requires mount namespace support).
    /// See systemd.exec(5).
    pub protect_kernel_tunables: bool,
    /// ProtectClock= — if true, writes to the system and hardware clock are
    /// denied. Defaults to false. Parsed and stored; no runtime enforcement
    /// yet (requires seccomp and device access restrictions).
    /// See systemd.exec(5).
    pub protect_clock: bool,
    /// CapabilityBoundingSet= — a list of Linux capability names (e.g.
    /// CAP_NET_ADMIN, CAP_SYS_PTRACE) controlling the capability bounding
    /// set for executed processes. Entries prefixed with `~` form a deny-list;
    /// without the prefix they form an allow-list. Multiple directives
    /// accumulate; an empty assignment resets the list. Parsed and stored;
    /// no runtime enforcement yet. See systemd.exec(5).
    pub capability_bounding_set: Vec<String>,
    /// AmbientCapabilities= — a list of Linux capability names (e.g.
    /// CAP_NET_BIND_SERVICE, CAP_SYS_NICE) controlling the ambient
    /// capability set for executed processes. Ambient capabilities are
    /// inherited across execve() even without file capabilities. Entries
    /// prefixed with `~` form a deny-list; without the prefix they form an
    /// allow-list. Multiple directives accumulate; an empty assignment resets
    /// the list. Parsed and stored; no runtime enforcement yet.
    /// See systemd.exec(5).
    pub ambient_capabilities: Vec<String>,
    /// ProtectHome= — controls whether /home, /root, and /run/user are
    /// accessible to the service. Can be `no` (default), `yes` (inaccessible),
    /// `read-only`, or `tmpfs`. Parsed and stored; no runtime enforcement yet
    /// (requires mount namespace support). See systemd.exec(5).
    pub protect_home: ProtectHome,
    /// ProtectHostname= — if true, sets up a new UTS namespace for the
    /// service and prevents it from changing the hostname or domainname.
    /// Defaults to false. Parsed and stored; no runtime enforcement yet
    /// (requires UTS namespace and seccomp support). See systemd.exec(5).
    pub protect_hostname: bool,
    /// ProtectHostname= mode — "yes" (read-only, prevents sethostname) or
    /// "private" (new UTS namespace, allows sethostname within it).
    /// None means not set explicitly (defaults to "yes" when protect_hostname=true).
    pub protect_hostname_mode: Option<String>,
    /// ProtectHostname= hostname — optional hostname to set in the UTS
    /// namespace when using `ProtectHostname=yes:hostname` syntax.
    pub protect_hostname_name: Option<String>,
    /// SystemCallArchitectures= — a space-separated list of architecture
    /// identifiers (e.g. `native`, `x86`, `x86-64`) restricting which
    /// architectures system calls may be invoked from. Multiple directives
    /// accumulate; an empty assignment resets the list. Parsed and stored;
    /// no runtime seccomp enforcement yet. See systemd.exec(5).
    pub system_call_architectures: Vec<String>,
    /// ReadWritePaths= — a space-separated list of file system paths that
    /// should be made accessible read-write for the service, even if other
    /// settings (like `ProtectSystem=strict`) would otherwise make them
    /// read-only. Multiple directives accumulate; an empty assignment resets
    /// the list. Parsed and stored; no runtime mount-namespace enforcement
    /// yet. See systemd.exec(5).
    pub read_write_paths: Vec<String>,
    /// MemoryDenyWriteExecute= — if true, attempts to create memory mappings
    /// that are both writable and executable, or to change existing writable
    /// mappings to executable, are prohibited. Defaults to false. Parsed and
    /// stored; no runtime seccomp enforcement yet. See systemd.exec(5).
    pub memory_deny_write_execute: bool,
    /// LockPersonality= — if true, locks down the personality(2) system call
    /// so that the kernel execution domain may not be changed from the default
    /// or the personality selected with Personality=. Defaults to false.
    /// Parsed and stored; no runtime seccomp enforcement yet. See systemd.exec(5).
    pub lock_personality: bool,
    /// ProtectProc= — controls the `hidepid=` mount option of the procfs
    /// instance for the unit. Takes one of "default", "noaccess", "invisible",
    /// or "ptraceable". Defaults to "default". Parsed and stored; no runtime
    /// mount-namespace enforcement yet. See systemd.exec(5).
    pub protect_proc: ProtectProc,
    /// PrivateTmp= — if true, sets up a private /tmp and /var/tmp namespace
    /// for the executed processes. Files in these directories are not visible
    /// to other processes and vice versa. Defaults to false. Parsed and
    /// stored; no runtime mount-namespace enforcement yet. See systemd.exec(5).
    pub private_tmp: bool,
    /// PrivateDevices= — if true, sets up a private /dev/ namespace for the
    /// executed processes with only pseudo-devices such as /dev/null,
    /// /dev/zero, /dev/random available. Physical devices are not accessible.
    /// Defaults to false. Parsed and stored; no runtime mount-namespace
    /// enforcement yet. See systemd.exec(5).
    pub private_devices: bool,
    /// PrivateNetwork= — if true, sets up a new network namespace for the
    /// executed processes with only the loopback device "lo" (and no
    /// external network connectivity). Defaults to false. Parsed and stored;
    /// no runtime namespace enforcement yet. See systemd.exec(5).
    pub private_network: bool,
    /// PrivateUsers= — if true, sets up a new user namespace for the
    /// executed processes and configures a minimal user/group mapping:
    /// the root user/group as well as the unit's own user/group are
    /// mapped to themselves, everything else is mapped to the "nobody"
    /// user/group. Defaults to false. Parsed and stored; no runtime
    /// user-namespace enforcement yet. See systemd.exec(5).
    pub private_users: bool,
    /// PrivateMounts= — if true, the processes of this unit will be run in
    /// their own private file system (mount) namespace with all mount
    /// propagation from the processes towards the host's main file system
    /// namespace turned off. File system mount points established or removed
    /// on the host will still be propagated to the unit's processes.
    /// Defaults to false. Parsed and stored; no runtime mount-namespace
    /// enforcement yet. See systemd.exec(5).
    pub private_mounts: bool,
    /// MountFlags= — mount propagation flags for the mount namespace.
    /// "shared" = MS_SHARED, "slave" = MS_SLAVE (default), "private" = MS_PRIVATE.
    /// See systemd.exec(5).
    pub mount_flags: Option<String>,
    /// IOSchedulingClass= — sets the I/O scheduling class for executed
    /// processes. Takes one of "none" (or "0"), "realtime" (or "1"),
    /// "best-effort" (or "2"), or "idle" (or "3"). Defaults to None
    /// (kernel default, which is best-effort). Parsed and stored; no
    /// runtime ioprio_set() enforcement yet. See systemd.exec(5).
    pub io_scheduling_class: IOSchedulingClass,
    /// IOSchedulingPriority= — sets the I/O scheduling priority for executed
    /// processes. Takes an integer between 0 (highest priority) and 7
    /// (lowest priority). The default priority for the best-effort scheduling
    /// class is 4. Parsed and stored; no runtime enforcement yet.
    /// See systemd.exec(5).
    pub io_scheduling_priority: Option<u8>,
    /// UMask= — sets the file mode creation mask (umask) for executed
    /// processes. Takes an octal value (e.g. 0022, 0077). Defaults to 0022.
    /// Parsed and stored; no runtime enforcement yet. See systemd.exec(5).
    pub umask: Option<u32>,
    /// ProcSubset= — controls which subset of /proc/ is mounted for the
    /// unit. Takes one of "all" (full /proc, default) or "pid" (only
    /// process-specific subdirectories). Parsed and stored; no runtime
    /// mount-namespace enforcement yet. See systemd.exec(5).
    pub proc_subset: ProcSubset,
    /// Nice= — sets the default nice level (scheduling priority) for
    /// executed processes. Takes an integer between -20 (highest priority)
    /// and 19 (lowest priority). Parsed and stored; no runtime enforcement
    /// yet. See systemd.exec(5).
    pub nice: Option<i32>,
    /// RemoveIPC= — if true, all System V and POSIX IPC objects owned by
    /// the user and group of the executed processes are removed when the
    /// unit is stopped. Defaults to false. Parsed and stored; no runtime
    /// enforcement yet. See systemd.exec(5).
    pub remove_ipc: bool,
    /// PAMName= — sets the PAM service name to set up a session as.
    /// If set, PAM authentication, account management, and session open/close
    /// calls are performed for the executed processes. Takes a string such as
    /// "login", "systemd-user", or "sshd". Defaults to None (no PAM session).
    /// Parsed and stored; no runtime PAM enforcement yet. See systemd.exec(5).
    pub pam_name: Option<String>,

    // ── Resource limits (LimitXXX=) ──────────────────────────────────
    /// LimitCPU= — maximum CPU time in seconds. Applied via
    /// `setrlimit(RLIMIT_CPU)`. See systemd.exec(5).
    pub limit_cpu: Option<ResourceLimit>,
    /// LimitCORE= — maximum size of core dump files. Applied via
    /// `setrlimit(RLIMIT_CORE)`. See systemd.exec(5).
    pub limit_core: Option<ResourceLimit>,
    /// LimitFSIZE= — maximum size of files the process may create. Applied
    /// via `setrlimit(RLIMIT_FSIZE)`. See systemd.exec(5).
    pub limit_fsize: Option<ResourceLimit>,
    /// LimitDATA= — maximum size of the process's data segment. Applied via
    /// `setrlimit(RLIMIT_DATA)`. See systemd.exec(5).
    pub limit_data: Option<ResourceLimit>,
    /// LimitSTACK= — maximum size of the process stack. Applied via
    /// `setrlimit(RLIMIT_STACK)`. See systemd.exec(5).
    pub limit_stack: Option<ResourceLimit>,
    /// LimitRSS= — maximum resident set size. Applied via
    /// `setrlimit(RLIMIT_RSS)`. See systemd.exec(5).
    pub limit_rss: Option<ResourceLimit>,
    /// LimitNPROC= — maximum number of processes for the real user ID.
    /// Applied via `setrlimit(RLIMIT_NPROC)`. See systemd.exec(5).
    pub limit_nproc: Option<ResourceLimit>,
    /// LimitMEMLOCK= — maximum number of bytes of memory that may be locked.
    /// Applied via `setrlimit(RLIMIT_MEMLOCK)`. See systemd.exec(5).
    pub limit_memlock: Option<ResourceLimit>,
    /// LimitAS= — maximum size of the process's virtual memory (address
    /// space). Applied via `setrlimit(RLIMIT_AS)`. See systemd.exec(5).
    pub limit_as: Option<ResourceLimit>,
    /// LimitLOCKS= — maximum number of flock/fcntl locks. Applied via
    /// `setrlimit(RLIMIT_LOCKS)`. See systemd.exec(5).
    pub limit_locks: Option<ResourceLimit>,
    /// LimitSIGPENDING= — maximum number of signals that may be queued.
    /// Applied via `setrlimit(RLIMIT_SIGPENDING)`. See systemd.exec(5).
    pub limit_sigpending: Option<ResourceLimit>,
    /// LimitMSGQUEUE= — maximum number of bytes in POSIX message queues.
    /// Applied via `setrlimit(RLIMIT_MSGQUEUE)`. See systemd.exec(5).
    pub limit_msgqueue: Option<ResourceLimit>,
    /// LimitNICE= — maximum nice priority allowed to raise to (range 0–40,
    /// mapped to nice values 20..-20). Applied via `setrlimit(RLIMIT_NICE)`.
    /// See systemd.exec(5).
    pub limit_nice: Option<ResourceLimit>,
    /// LimitRTPRIO= — maximum realtime scheduling priority. Applied via
    /// `setrlimit(RLIMIT_RTPRIO)`. See systemd.exec(5).
    pub limit_rtprio: Option<ResourceLimit>,
    /// LimitRTTIME= — maximum number of microseconds a realtime-scheduled
    /// process may consume without blocking. Applied via
    /// `setrlimit(RLIMIT_RTTIME)`. See systemd.exec(5).
    pub limit_rttime: Option<ResourceLimit>,

    // ── Directory management ─────────────────────────────────────────
    /// CacheDirectory= — directories to create under /var/cache/ before the
    /// service starts. Ownership is set to the service user/group and the
    /// CACHE_DIRECTORY environment variable is set to a colon-separated
    /// list of the absolute paths. Matches systemd.exec(5).
    pub cache_directory: Vec<String>,
    /// CacheDirectoryMode= — the file system access mode to use when creating
    /// the cache directories specified with CacheDirectory=. Takes an access
    /// mode in octal notation (e.g. 0755). Defaults to 0755.
    /// See systemd.exec(5).
    pub cache_directory_mode: Option<u32>,
    /// ConfigurationDirectory= — directories to create under /etc/ before the
    /// service starts. Ownership is set to the service user/group and the
    /// CONFIGURATION_DIRECTORY environment variable is set to a
    /// colon-separated list of the absolute paths. Matches systemd.exec(5).
    pub configuration_directory: Vec<String>,
    /// ConfigurationDirectoryMode= — the file system access mode to use when
    /// creating the configuration directories specified with
    /// ConfigurationDirectory=. Takes an access mode in octal notation
    /// (e.g. 0755). Defaults to 0755. See systemd.exec(5).
    pub configuration_directory_mode: Option<u32>,
    /// StateDirectoryMode= — the file system access mode to use when creating
    /// the state directories specified with StateDirectory=. Takes an access
    /// mode in octal notation (e.g. 0755). Defaults to 0755.
    /// See systemd.exec(5).
    pub state_directory_mode: Option<u32>,
    /// RuntimeDirectoryMode= — the file system access mode to use when
    /// creating the runtime directories specified with RuntimeDirectory=.
    /// Takes an access mode in octal notation (e.g. 0755). Defaults to 0755.
    /// See systemd.exec(5).
    pub runtime_directory_mode: Option<u32>,

    // ── Path-based mount namespace directives ────────────────────────
    /// ReadOnlyPaths= — a space-separated list of file system paths that
    /// should be made read-only for the service within its mount namespace.
    /// Multiple directives accumulate; an empty assignment resets the list.
    /// See systemd.exec(5).
    pub read_only_paths: Vec<String>,
    /// InaccessiblePaths= — a space-separated list of file system paths that
    /// should be made completely inaccessible (hidden) for the service within
    /// its mount namespace. Multiple directives accumulate; an empty
    /// assignment resets the list. See systemd.exec(5).
    pub inaccessible_paths: Vec<String>,
    /// BindPaths= — bind-mount host paths into the service's mount namespace
    /// (read-write). Each entry is "SOURCE[:DEST[:OPTIONS]]". Multiple
    /// directives accumulate; an empty assignment resets the list.
    /// See systemd.exec(5).
    pub bind_paths: Vec<String>,
    /// BindReadOnlyPaths= — bind-mount host paths into the service's mount
    /// namespace (read-only). Same format as BindPaths=. Multiple directives
    /// accumulate; an empty assignment resets the list. See systemd.exec(5).
    pub bind_read_only_paths: Vec<String>,
    /// TemporaryFileSystem= — mount tmpfs file systems on the specified paths
    /// within the service's mount namespace. Each entry is "PATH[:OPTIONS]".
    /// Multiple directives accumulate; an empty assignment resets the list.
    /// See systemd.exec(5).
    pub temporary_file_system: Vec<String>,

    // ── Logging directives ───────────────────────────────────────────
    /// SyslogIdentifier= — sets the process name ("tag") to prefix log
    /// messages with. Defaults to the name of the executed process.
    /// See systemd.exec(5).
    pub syslog_identifier: Option<String>,

    // ── Additional logging directives ────────────────────────────────
    /// SyslogFacility= — sets the syslog facility identifier to use when
    /// logging to syslog. Takes one of `kern`, `user`, `mail`, `daemon`,
    /// `auth`, `syslog`, `lpr`, `news`, `uucp`, `cron`, `authpriv`, `ftp`,
    /// `local0`–`local7`. Defaults to `daemon`. See systemd.exec(5).
    pub syslog_facility: Option<String>,
    /// SyslogLevel= — sets the default syslog level to use when logging to
    /// syslog. Takes one of `emerg`, `alert`, `crit`, `err`, `warning`,
    /// `notice`, `info`, `debug`. Defaults to `info`. See systemd.exec(5).
    pub syslog_level: Option<String>,
    /// SyslogLevelPrefix= — if true (default), syslog-style log level prefix
    /// strings (`<N>`) at the beginning of log lines are interpreted and
    /// stripped from the log message. See systemd.exec(5).
    pub syslog_level_prefix: Option<bool>,
    /// LogLevelMax= — filters log messages by level: messages with a log
    /// level higher (i.e. less important) than the specified level are
    /// dropped. Takes the same values as `SyslogLevel=`. See systemd.exec(5).
    pub log_level_max: Option<String>,
    /// LogRateLimitIntervalSec= — configures the rate limiting interval for
    /// messages logged by this unit. See systemd.exec(5).
    pub log_rate_limit_interval_sec: Option<String>,
    /// LogRateLimitBurst= — configures the burst limit for rate limiting of
    /// log messages. Defaults to `10000`. See systemd.exec(5).
    pub log_rate_limit_burst: Option<u32>,
    /// LogFilterPatterns= — defines extended regular expression patterns for
    /// filtering log messages. Multiple directives accumulate; empty resets.
    /// See systemd.exec(5).
    pub log_filter_patterns: Vec<String>,
    /// LogNamespace= — run the service in the specified journal namespace.
    /// See systemd.exec(5).
    pub log_namespace: Option<String>,

    // ── CPU scheduling directives ────────────────────────────────────
    /// CPUSchedulingPolicy= — sets the CPU scheduling policy for executed
    /// processes. Takes one of `other`, `batch`, `idle`, `fifo`, or `rr`.
    /// See systemd.exec(5).
    pub cpu_scheduling_policy: Option<String>,
    /// CPUSchedulingPriority= — sets the CPU scheduling priority for
    /// executed processes. See systemd.exec(5).
    pub cpu_scheduling_priority: Option<u32>,
    /// CPUSchedulingResetOnFork= — if true, elevated CPU scheduling
    /// priorities and policies are reset when the executed processes fork.
    /// Defaults to false. See systemd.exec(5).
    pub cpu_scheduling_reset_on_fork: Option<bool>,
    /// CPUAffinity= — sets the CPU affinity for executed processes. Takes a
    /// list of CPU indices or ranges. Multiple directives accumulate; empty
    /// resets. See systemd.exec(5).
    pub cpu_affinity: Vec<String>,
    /// NUMAPolicy= — sets the NUMA memory allocation policy. Takes one of
    /// `default`, `preferred`, `bind`, `interleave`, or `local`.
    /// See systemd.exec(5).
    pub numa_policy: Option<String>,
    /// NUMAMask= — sets the NUMA node mask for the policy configured with
    /// `NUMAPolicy=`. See systemd.exec(5).
    pub numa_mask: Option<String>,

    // ── Root filesystem / image directives ───────────────────────────
    /// RootDirectory= — sets the root directory for executed processes,
    /// using `pivot_root(2)` or `chroot(2)`. See systemd.exec(5).
    pub root_directory: Option<String>,
    /// RootImage= — sets the root file system image to mount as the root
    /// directory for executed processes. See systemd.exec(5).
    pub root_image: Option<String>,
    /// RootImageOptions= — mount options for partitions within the root
    /// image. Multiple directives accumulate; empty resets.
    /// See systemd.exec(5).
    pub root_image_options: Vec<String>,
    /// RootHash= — dm-verity data root hash for the root image.
    /// See systemd.exec(5).
    pub root_hash: Option<String>,
    /// RootHashSignature= — dm-verity root hash signature.
    /// See systemd.exec(5).
    pub root_hash_signature: Option<String>,
    /// RootVerity= — path to a dm-verity data file for the root image.
    /// See systemd.exec(5).
    pub root_verity: Option<String>,
    /// RootEphemeral= — if true, an ephemeral snapshot of the root image
    /// is created and used. See systemd.exec(5).
    pub root_ephemeral: Option<bool>,
    /// MountAPIVFS= — if true, /proc/, /sys/, /dev/ and /tmp/ are mounted
    /// inside the root directory or image before the service starts.
    /// See systemd.exec(5).
    pub mount_api_vfs: Option<bool>,
    /// ExtensionDirectories= — directories containing system extension
    /// hierarchy trees to overlay on top of the root filesystem.
    /// Multiple directives accumulate; empty resets. See systemd.exec(5).
    pub extension_directories: Vec<String>,
    /// ExtensionImages= — disk image paths containing system extension
    /// hierarchies to overlay. Multiple directives accumulate; empty resets.
    /// See systemd.exec(5).
    pub extension_images: Vec<String>,
    /// MountImages= — disk image paths to mount at specified mount points.
    /// Each entry is `SOURCE:DEST[:OPTIONS]`. Multiple directives accumulate;
    /// empty resets. See systemd.exec(5).
    pub mount_images: Vec<String>,
    /// BindLogSockets= — if true, the journal logging sockets are bound
    /// into the service's mount namespace. See systemd.exec(5).
    pub bind_log_sockets: Option<bool>,

    // ── Additional namespace directives ──────────────────────────────
    /// PrivateIPC= — if true, sets up a new IPC namespace for the executed
    /// processes. Defaults to false. See systemd.exec(5).
    pub private_ipc: Option<bool>,
    /// PrivatePIDs= — if true, sets up a new PID namespace for the executed
    /// processes. Defaults to false. Added in systemd 257.
    /// See systemd.exec(5).
    pub private_pids: Option<bool>,
    /// IPCNamespacePath= — run the service in the specified existing IPC
    /// namespace. See systemd.exec(5).
    pub ipc_namespace_path: Option<String>,
    /// NetworkNamespacePath= — run the service in the specified existing
    /// network namespace. See systemd.exec(5).
    pub network_namespace_path: Option<String>,

    // ── Security directives ──────────────────────────────────────────
    /// SecureBits= — controls the secure-bits flags of the executed process.
    /// Takes a space-separated combination of `keep-caps`, `keep-caps-locked`,
    /// `no-setuid-fixup`, `no-setuid-fixup-locked`, `noroot`,
    /// `noroot-locked`. See systemd.exec(5).
    pub secure_bits: Vec<String>,
    /// Personality= — controls the process personality / execution domain.
    /// See systemd.exec(5).
    pub personality: Option<String>,
    /// SELinuxContext= — sets the SELinux security context for executed
    /// processes. See systemd.exec(5).
    pub selinux_context: Option<String>,
    /// AppArmorProfile= — sets the AppArmor profile for executed processes.
    /// See systemd.exec(5).
    pub apparmor_profile: Option<String>,
    /// SmackProcessLabel= — sets the SMACK security label for executed
    /// processes. See systemd.exec(5).
    pub smack_process_label: Option<String>,
    /// KeyringMode= — controls the kernel keyring set up for the service.
    /// Takes one of `inherit`, `private`, or `shared`. See systemd.exec(5).
    pub keyring_mode_exec: Option<String>,
    /// NoExecPaths= — paths from which execution of programs is forbidden.
    /// Multiple directives accumulate; empty resets. See systemd.exec(5).
    pub no_exec_paths: Vec<String>,
    /// ExecPaths= — paths from which execution of programs is permitted.
    /// Multiple directives accumulate; empty resets. See systemd.exec(5).
    pub exec_paths: Vec<String>,
    /// CoredumpFilter= — sets the coredump filter bitmask for the executed
    /// processes. See systemd.exec(5).
    pub coredump_filter: Option<String>,

    // ── Misc directives ─────────────────────────────────────────────
    /// TimerSlackNSec= — sets the timer slack for executed processes.
    /// See systemd.exec(5).
    pub timer_slack_nsec: Option<String>,
    /// StandardInputText= — configures arbitrary text data to pass via
    /// standard input to the executed process. Multiple directives
    /// accumulate. See systemd.exec(5).
    pub standard_input_text: Vec<String>,
    /// StandardInputData= — like StandardInputText=, but the data is
    /// specified in Base64 encoding. Multiple directives accumulate.
    /// See systemd.exec(5).
    pub standard_input_data: Vec<String>,
    /// SetLoginEnvironment= — if true, PAM login session environment
    /// variables are set. See systemd.exec(5).
    pub set_login_environment: Option<bool>,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Eq, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct PlatformSpecificServiceFields {
    pub cgroup_path: std::path::PathBuf,
}

#[cfg(not(target_os = "linux"))]
#[derive(Clone, Eq, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct PlatformSpecificServiceFields {}

/// Additional exit codes and signals that should be considered a successful
/// termination, as configured by `SuccessExitStatus=` in the `[Service]` section.
///
/// By default only exit code 0 and the "clean" signals (SIGHUP, SIGINT,
/// SIGTERM, SIGPIPE) count as success.  This struct extends that set on a
/// per-service basis.
#[derive(Clone, Eq, PartialEq, Debug, Default)]
pub struct SuccessExitStatus {
    pub exit_codes: Vec<i32>,
    pub signals: Vec<nix::sys::signal::Signal>,
}

impl SuccessExitStatus {
    /// Returns `true` when `termination` should be treated as a successful
    /// exit, considering both the built-in rules (exit 0) and any extra
    /// codes/signals configured via `SuccessExitStatus=`.
    pub fn is_success(&self, termination: &crate::signal_handler::ChildTermination) -> bool {
        match termination {
            crate::signal_handler::ChildTermination::Exit(code) => {
                *code == 0 || self.exit_codes.contains(code)
            }
            crate::signal_handler::ChildTermination::Signal(sig) => self.signals.contains(sig),
        }
    }

    /// Like `is_clean_signal` but also considers extra signals from this
    /// config as "clean".
    pub fn is_clean_signal(&self, termination: &crate::signal_handler::ChildTermination) -> bool {
        use nix::sys::signal::Signal;
        match termination {
            crate::signal_handler::ChildTermination::Signal(sig) => {
                matches!(
                    sig,
                    Signal::SIGHUP | Signal::SIGINT | Signal::SIGTERM | Signal::SIGPIPE
                ) || self.signals.contains(sig)
            }
            crate::signal_handler::ChildTermination::Exit(_) => false,
        }
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
/// The immutable config of a service unit
pub struct ServiceConfig {
    pub restart: ServiceRestart,
    pub restart_sec: Option<Timeout>,
    pub kill_mode: KillMode,
    pub delegate: Delegate,
    pub tasks_max: Option<TasksMax>,
    pub limit_nofile: Option<ResourceLimit>,
    pub accept: bool,
    pub notifyaccess: NotifyKind,
    pub exec: Vec<Commandline>,
    pub reload: Vec<Commandline>,
    pub stop: Vec<Commandline>,
    pub stoppost: Vec<Commandline>,
    pub startpre: Vec<Commandline>,
    pub startpost: Vec<Commandline>,
    pub srcv_type: ServiceType,
    pub starttimeout: Option<Timeout>,
    pub stoptimeout: Option<Timeout>,
    pub generaltimeout: Option<Timeout>,
    pub exec_config: ExecConfig,
    pub platform_specific: PlatformSpecificServiceFields,
    pub dbus_name: Option<String>,
    /// PIDFile= — path to a file that contains the PID of the main daemon
    /// process after a Type=forking service has started.
    pub pid_file: Option<std::path::PathBuf>,
    pub sockets: Vec<UnitId>,
    /// Slice= — the slice unit to place this service in for resource management
    pub slice: Option<String>,
    /// RemainAfterExit= — whether the service is considered active even after
    /// the main process exits. Defaults to false. Commonly used with Type=oneshot.
    pub remain_after_exit: bool,
    /// SuccessExitStatus= — additional exit codes and signals that are
    /// considered a successful (clean) service termination.
    pub success_exit_status: SuccessExitStatus,
    /// RestartForceExitStatus= — a list of exit status definitions (exit
    /// codes and/or signal names) that will force automatic service restart
    /// regardless of the configured Restart= setting. The format is the
    /// same as SuccessExitStatus=. See systemd.service(5).
    pub restart_force_exit_status: SuccessExitStatus,
    /// SendSIGHUP= — if true, send SIGHUP to remaining processes immediately
    /// after the stop signal (e.g. SIGTERM). This is useful for shell-like
    /// services that need to be notified their connection has been severed.
    /// Defaults to false. See systemd.kill(5).
    pub send_sighup: bool,

    /// MemoryPressureWatch= — configures whether to watch for memory pressure
    /// events via PSI. Parsed and stored; no runtime enforcement.
    /// See systemd.resource-control(5).
    pub memory_pressure_watch: MemoryPressureWatch,

    /// ReloadSignal= — configures the UNIX process signal to send to the
    /// service's main process when asked to reload. Defaults to SIGHUP.
    /// Only effective with Type=notify-reload. Stored as a raw signal
    /// number to support realtime signals (RTMIN+N). Parsed and stored;
    /// not yet used at runtime. See systemd.service(5).
    pub reload_signal: Option<i32>,

    /// KillSignal= — specifies which signal to use when stopping the service.
    /// Defaults to SIGTERM. Takes a signal name such as SIGTERM, SIGINT,
    /// SIGKILL, RTMIN+N, or a numeric signal number. Stored as a raw signal
    /// number to support realtime signals. Parsed and stored; not yet used
    /// at runtime. See systemd.kill(5).
    pub kill_signal: Option<i32>,

    /// DelegateSubgroup= — place unit processes in the specified subgroup of
    /// the unit's control group. Only effective when Delegate= is enabled.
    /// Parsed and stored; not yet used at runtime. See systemd.resource-control(5).
    pub delegate_subgroup: Option<String>,

    /// KeyringMode= — controls how the kernel session keyring is set up for
    /// the service. Defaults to `private` for system services and `inherit`
    /// for non-service units / user services. Parsed and stored; not yet
    /// enforced at runtime. See systemd.exec(5).
    pub keyring_mode: KeyringMode,

    /// DeviceAllow= — controls access to specific device nodes for the unit.
    /// Each entry is a device node path (or class like "char-*", "block-*")
    /// optionally followed by access characters (r=read, w=write, m=mknod).
    /// Multiple directives accumulate; an empty assignment resets the list.
    /// Enforced via BPF cgroup device controller on cgroup v2.
    /// See systemd.resource-control(5).
    pub device_allow: Vec<String>,

    /// DevicePolicy= — controls the policy for device access for the unit.
    /// Takes "auto" (default, no restrictions), "closed" (only standard
    /// pseudo devices plus DeviceAllow= entries), or "strict" (only
    /// DeviceAllow= entries). Enforced via BPF cgroup device controller
    /// on cgroup v2. See systemd.resource-control(5).
    pub device_policy: DevicePolicy,

    /// WatchdogSec= — configures the watchdog timeout for the service. The
    /// service must send a "WATCHDOG=1" notification via sd_notify() at least
    /// once within this interval, or it will be considered failed. A value of
    /// 0 or empty disables the watchdog. Enforced at runtime by the
    /// background watchdog thread (`libsystemd::watchdog`), which
    /// periodically checks running services and sends `WatchdogSignal=`
    /// (default SIGABRT) to unresponsive ones. See systemd.service(5).
    pub watchdog_sec: Option<Timeout>,

    /// IPAddressAllow= — a list of IP address prefixes (CIDR notation) or
    /// special keywords (`any`, `localhost`, `link-local`, `multicast`) that
    /// are allowed for IP communication by this unit. Multiple directives
    /// accumulate; an empty assignment resets the list. Parsed and stored;
    /// no runtime enforcement yet (requires eBPF/cgroup support).
    /// See systemd.resource-control(5).
    pub ip_address_allow: Vec<String>,

    /// IPAddressDeny= — a list of IP address prefixes (CIDR notation) or
    /// special keywords (`any`, `localhost`, `link-local`, `multicast`) that
    /// are denied for IP communication by this unit. Multiple directives
    /// accumulate; an empty assignment resets the list. Parsed and stored;
    /// no runtime enforcement yet (requires eBPF/cgroup support).
    /// See systemd.resource-control(5).
    pub ip_address_deny: Vec<String>,

    /// FileDescriptorStoreMax= — the maximum number of file descriptors that
    /// may be stored in the service manager for this service using
    /// sd_pid_notify_with_fds()'s FDSTORE=1 messages. Defaults to 0
    /// (file descriptor store disabled). Parsed and stored; no runtime
    /// enforcement yet. See systemd.service(5).
    pub file_descriptor_store_max: u64,

    /// FileDescriptorStorePreserve= — controls whether file descriptors
    /// stored in the service manager (via FDSTORE=1 sd_notify messages) are
    /// preserved across service restarts or stops. Takes `no` (default,
    /// discard on stop), `yes` (preserve as long as the unit exists), or
    /// `restart` (preserve across restarts, discard on full stop). Parsed
    /// and stored; no runtime enforcement yet. See systemd.service(5).
    pub file_descriptor_store_preserve: FileDescriptorStorePreserve,

    /// MemoryMin= — minimum memory guarantee for the unit's cgroup. The
    /// memory controller will try to protect at least this much memory from
    /// reclaim. Accepts a byte value with optional K/M/G/T/P/E suffix
    /// (base 1024), a percentage, or "infinity". Parsed and stored; applied
    /// to cgroup memory.min at runtime. See systemd.resource-control(5).
    pub memory_min: Option<MemoryLimit>,

    /// MemoryLow= — low memory boundary for the unit's cgroup. Below this
    /// threshold the kernel memory reclaimer will avoid reclaiming memory
    /// from the unit. Accepts a byte value with optional K/M/G/T/P/E suffix
    /// (base 1024), a percentage, or "infinity". Parsed and stored; applied
    /// to cgroup memory.low at runtime. See systemd.resource-control(5).
    pub memory_low: Option<MemoryLimit>,

    /// MemoryHigh= — throttling memory boundary for the unit's cgroup. If
    /// memory usage goes above this threshold, processes are heavily throttled
    /// and put under pressure. Accepts a byte value with optional K/M/G/T/P/E
    /// suffix (base 1024), a percentage, or "infinity". Applied to cgroup
    /// memory.high at runtime. See systemd.resource-control(5).
    pub memory_high: Option<MemoryLimit>,

    /// MemoryMax= — hard memory limit for the unit's cgroup. If memory usage
    /// exceeds this limit, the OOM killer is invoked. Accepts a byte value with
    /// optional K/M/G/T/P/E suffix (base 1024), a percentage, or "infinity".
    /// Applied to cgroup memory.max at runtime. See systemd.resource-control(5).
    pub memory_max: Option<MemoryLimit>,

    /// MemorySwapMax= — hard swap limit for the unit's cgroup. Accepts a byte
    /// value with optional K/M/G/T/P/E suffix (base 1024), a percentage, or
    /// "infinity". Applied to cgroup memory.swap.max at runtime.
    /// See systemd.resource-control(5).
    pub memory_swap_max: Option<MemoryLimit>,

    /// CPUWeight= — CPU weight for the unit's cgroup (1–10000, default 100).
    /// Applied to cgroup cpu.weight at runtime. See systemd.resource-control(5).
    pub cpu_weight: Option<CpuWeight>,

    /// StartupCPUWeight= — CPU weight during system startup (1–10000).
    /// Parsed and stored; no runtime enforcement yet (falls back to CPUWeight=).
    /// See systemd.resource-control(5).
    pub startup_cpu_weight: Option<CpuWeight>,

    /// CPUQuota= — CPU time quota as a percentage (e.g. "20%", "200%").
    /// Applied to cgroup cpu.max at runtime. See systemd.resource-control(5).
    pub cpu_quota: Option<CpuQuota>,

    /// IOWeight= — I/O weight for the unit's cgroup (1–10000, default 100).
    /// Applied to cgroup io.weight at runtime. See systemd.resource-control(5).
    pub io_weight: Option<IoWeight>,

    /// StartupIOWeight= — I/O weight during system startup (1–10000).
    /// Parsed and stored; no runtime enforcement yet (falls back to IOWeight=).
    /// See systemd.resource-control(5).
    pub startup_io_weight: Option<IoWeight>,

    /// IODeviceWeight= — per-device I/O weight (format: "/dev/path WEIGHT").
    /// Applied to cgroup io.weight at runtime. See systemd.resource-control(5).
    pub io_device_weight: Vec<IoDeviceLimit>,

    /// IOReadBandwidthMax= — per-device read bandwidth limit (format:
    /// "/dev/path BYTES"). Applied to cgroup io.max at runtime.
    /// See systemd.resource-control(5).
    pub io_read_bandwidth_max: Vec<IoDeviceLimit>,

    /// IOWriteBandwidthMax= — per-device write bandwidth limit (format:
    /// "/dev/path BYTES"). Applied to cgroup io.max at runtime.
    /// See systemd.resource-control(5).
    pub io_write_bandwidth_max: Vec<IoDeviceLimit>,

    /// IOReadIOPSMax= — per-device read IOPS limit (format: "/dev/path IOPS").
    /// Applied to cgroup io.max at runtime. See systemd.resource-control(5).
    pub io_read_iops_max: Vec<IoDeviceLimit>,

    /// IOWriteIOPSMax= — per-device write IOPS limit (format: "/dev/path IOPS").
    /// Applied to cgroup io.max at runtime. See systemd.resource-control(5).
    pub io_write_iops_max: Vec<IoDeviceLimit>,

    /// CPUAccounting= — enable CPU accounting for the unit's cgroup.
    /// When true, the cpu controller is enabled. Defaults to the system default.
    /// See systemd.resource-control(5).
    pub cpu_accounting: Option<bool>,

    /// MemoryAccounting= — enable memory accounting for the unit's cgroup.
    /// When true, the memory controller is enabled. Defaults to the system default.
    /// See systemd.resource-control(5).
    pub memory_accounting: Option<bool>,

    /// IOAccounting= — enable I/O accounting for the unit's cgroup.
    /// When true, the io controller is enabled. Defaults to the system default.
    /// See systemd.resource-control(5).
    pub io_accounting: Option<bool>,

    /// TasksAccounting= — enable task counting for the unit's cgroup.
    /// When true, the pids controller is enabled. Defaults to the system default.
    /// See systemd.resource-control(5).
    pub tasks_accounting: Option<bool>,

    /// RuntimeMaxSec= — configures a maximum time for the service to run.
    /// If the service has been active for longer than the specified time it
    /// is terminated and put into a failure state. When set to `infinity`
    /// (the default) or not set, no time limit is configured. A value of 0
    /// also means no limit. Parsed and stored; no runtime enforcement yet.
    /// See systemd.service(5).
    pub runtime_max_sec: Option<Timeout>,

    /// CoredumpReceive= — boolean, defaults to false. When enabled, the
    /// service manager sets up a coredump socket so the service can receive
    /// coredumps from systemd-coredump. Parsed and stored; no runtime
    /// enforcement yet. See systemd.service(5).
    pub coredump_receive: bool,

    /// SendSIGKILL= — if true (the default), send SIGKILL to remaining
    /// processes after the stop timeout if they haven't terminated yet.
    /// If false, leave processes running after the timeout.
    /// See systemd.kill(5).
    pub send_sigkill: bool,

    /// RestartKillSignal= — specifies which signal to use when restarting
    /// a service (i.e. stop-before-restart). Takes a signal name or number.
    /// When not set, KillSignal= is used. See systemd.kill(5).
    pub restart_kill_signal: Option<i32>,

    /// FinalKillSignal= — specifies the signal to send as the very last
    /// step when terminating a service, after SIGKILL has been sent and the
    /// processes still haven't terminated. Defaults to SIGKILL.
    /// Takes a signal name or number. See systemd.kill(5).
    pub final_kill_signal: Option<i32>,

    /// WatchdogSignal= — specifies the signal to use to kill a service
    /// when the watchdog timeout expires. Defaults to SIGABRT.
    /// Takes a signal name or number. See systemd.kill(5).
    pub watchdog_signal: Option<i32>,

    /// ExitType= — configures when a service is considered exited/dead.
    /// `main` (default): the service is dead when the main process exits.
    /// `cgroup`: the service is dead only when the last process in its
    /// cgroup exits. See systemd.service(5).
    pub exit_type: ExitType,

    /// OOMPolicy= — configures the Out-Of-Memory killer policy for the
    /// service. Takes `continue`, `stop` (default), or `kill`.
    /// See systemd.service(5).
    pub oom_policy: OOMPolicy,

    /// TimeoutAbortSec= — configures the timeout for aborting the service
    /// (e.g. when the watchdog triggers). If not set, TimeoutStopSec= is
    /// used. See systemd.service(5).
    pub timeout_abort_sec: Option<Timeout>,

    /// TimeoutCleanSec= — configures the timeout for the clean-up phase
    /// of ExecStopPost= commands when cleaning resources on explicit request.
    /// If not set, TimeoutStopSec= is used. See systemd.service(5).
    pub timeout_clean_sec: Option<Timeout>,

    /// RestartPreventExitStatus= — a list of exit status definitions (exit
    /// codes and/or signal names) that will prevent automatic service restart
    /// regardless of the configured Restart= setting. The format is the
    /// same as SuccessExitStatus=. See systemd.service(5).
    pub restart_prevent_exit_status: SuccessExitStatus,

    /// RestartMode= — configures how a service is restarted. Takes `direct`
    /// (default) or `normal`. See systemd.service(5).
    pub restart_mode: RestartMode,

    /// RestartSteps= — configures the number of steps in the graduated
    /// restart delay ramp-up from RestartSec= to RestartMaxDelaySec=.
    /// A value of 0 (default) disables the graduated delay feature.
    /// See systemd.service(5).
    pub restart_steps: u32,

    /// RestartMaxDelaySec= — configures the longest restart delay when
    /// graduated restart delay is in use (RestartSteps= > 0). If not set
    /// or set to `infinity`, no upper bound is imposed.
    /// See systemd.service(5).
    pub restart_max_delay_sec: Option<Timeout>,

    /// ExecCondition= — commands to execute before ExecStartPre=. If any
    /// command exits with a non-zero status (and without the `-` prefix),
    /// the rest of the commands are skipped and the unit is not started,
    /// but this is not considered a failure. Exit code 254 causes the
    /// condition to be treated as failed (unit skipped).
    /// See systemd.service(5).
    pub exec_condition: Vec<Commandline>,

    // --- New service directives (systemd.service(5)) ---
    /// GuessMainPID= — if true (default), the service manager will try to
    /// guess the main PID of a forking service if PIDFile= is not set.
    /// See systemd.service(5).
    pub guess_main_pid: bool,

    /// TimeoutStartFailureMode= — configures the action to take when a
    /// start timeout is reached. Takes "terminate" (default) or "abort".
    /// See systemd.service(5).
    pub timeout_start_failure_mode: TimeoutFailureMode,

    /// TimeoutStopFailureMode= — configures the action to take when a
    /// stop timeout is reached. Takes "terminate" (default) or "abort".
    /// See systemd.service(5).
    pub timeout_stop_failure_mode: TimeoutFailureMode,

    /// RuntimeRandomizedExtraSec= — an additional random time added on
    /// top of RuntimeMaxSec=. Defaults to 0 (disabled). Parsed and stored;
    /// no runtime enforcement yet. See systemd.service(5).
    pub runtime_randomized_extra_sec: Option<Timeout>,

    /// RootDirectoryStartOnly= — if true, the root directory (as configured
    /// with RootDirectory=) is only applied to ExecStart= and not to the
    /// other Exec*= settings. Defaults to false. See systemd.service(5).
    pub root_directory_start_only: bool,

    /// NonBlocking= — if true, all file descriptors passed via socket
    /// activation are set to non-blocking mode (O_NONBLOCK). Defaults to
    /// false. See systemd.service(5).
    pub non_blocking: bool,

    /// USBFunctionDescriptors= — path to a file containing USB FunctionFS
    /// descriptors for the USB gadget. See systemd.service(5).
    pub usb_function_descriptors: Option<std::path::PathBuf>,

    /// USBFunctionStrings= — path to a file containing USB FunctionFS
    /// strings for the USB gadget. See systemd.service(5).
    pub usb_function_strings: Option<std::path::PathBuf>,

    /// OpenFile= — a list of file paths to open and pass as file descriptors
    /// to the service. Format: "path[:fdname[:options]]". Multiple directives
    /// accumulate; an empty assignment resets the list. See systemd.service(5).
    pub open_file: Vec<String>,

    // --- New resource-control directives (systemd.resource-control(5)) ---
    /// CPUQuotaPeriodSec= — the period for CPU quota enforcement. Defaults
    /// to 100ms. Applied to cgroup cpu.max at runtime.
    /// See systemd.resource-control(5).
    pub cpu_quota_period_sec: Option<Timeout>,

    /// AllowedCPUs= — restricts processes to the specified CPUs (cpuset
    /// controller). Takes a CPU index list with ranges. Applied to
    /// cgroup cpuset.cpus at runtime. See systemd.resource-control(5).
    pub allowed_cpus: Option<String>,

    /// StartupAllowedCPUs= — like AllowedCPUs= but only during startup.
    /// See systemd.resource-control(5).
    pub startup_allowed_cpus: Option<String>,

    /// AllowedMemoryNodes= — restricts processes to the specified NUMA
    /// memory nodes. Takes a node index list with ranges. Applied to
    /// cgroup cpuset.mems at runtime. See systemd.resource-control(5).
    pub allowed_memory_nodes: Option<String>,

    /// StartupAllowedMemoryNodes= — like AllowedMemoryNodes= but only
    /// during startup. See systemd.resource-control(5).
    pub startup_allowed_memory_nodes: Option<String>,

    /// DefaultMemoryMin= — default memory.min for children of this unit's
    /// cgroup. See systemd.resource-control(5).
    pub default_memory_min: Option<MemoryLimit>,

    /// DefaultMemoryLow= — default memory.low for children of this unit's
    /// cgroup. See systemd.resource-control(5).
    pub default_memory_low: Option<MemoryLimit>,

    /// MemoryZSwapMax= — hard zswap limit for the unit's cgroup. Accepts
    /// a byte value with optional K/M/G/T/P/E suffix, a percentage, or
    /// "infinity". Applied to cgroup memory.zswap.max at runtime.
    /// See systemd.resource-control(5).
    pub memory_zswap_max: Option<MemoryLimit>,

    /// IODeviceLatencyTargetSec= — per-device I/O latency target (format:
    /// "/dev/path TIMESPAN"). Applied to cgroup io.latency at runtime.
    /// See systemd.resource-control(5).
    pub io_device_latency_target_sec: Vec<String>,

    /// DisableControllers= — a space-separated list of cgroup controllers
    /// to disable for the unit. See systemd.resource-control(5).
    pub disable_controllers: Vec<String>,

    /// MemoryPressureThresholdSec= — memory pressure threshold duration.
    /// Used with MemoryPressureWatch=. See systemd.resource-control(5).
    pub memory_pressure_threshold_sec: Option<Timeout>,

    /// IPIngressFilterPath= — absolute path to a pinned BPF program for
    /// ingress IP packet filtering. Multiple directives accumulate; an
    /// empty assignment resets. See systemd.resource-control(5).
    pub ip_ingress_filter_path: Vec<String>,

    /// IPEgressFilterPath= — absolute path to a pinned BPF program for
    /// egress IP packet filtering. Multiple directives accumulate; an
    /// empty assignment resets. See systemd.resource-control(5).
    pub ip_egress_filter_path: Vec<String>,

    /// BPFProgram= — attach a BPF program to the unit's cgroup. Format:
    /// "type:path". Multiple directives accumulate; an empty assignment
    /// resets. See systemd.resource-control(5).
    pub bpf_program: Vec<String>,

    /// SocketBindAllow= — allow binding to specific socket address families,
    /// protocols, and ports. Multiple directives accumulate; an empty
    /// assignment resets. See systemd.resource-control(5).
    pub socket_bind_allow: Vec<String>,

    /// SocketBindDeny= — deny binding to specific socket address families,
    /// protocols, and ports. Multiple directives accumulate; an empty
    /// assignment resets. See systemd.resource-control(5).
    pub socket_bind_deny: Vec<String>,

    /// RestrictNetworkInterfaces= — restrict network interfaces accessible
    /// to the unit. Accepts a space-separated list of interface names
    /// (prefixed with "~" for deny). Multiple directives accumulate; an
    /// empty assignment resets. See systemd.resource-control(5).
    pub restrict_network_interfaces: Vec<String>,

    /// NFTSet= — attach the unit's cgroup to an nftables set. Format:
    /// "family:table:set". Multiple directives accumulate; an empty
    /// assignment resets. See systemd.resource-control(5).
    pub nft_set: Vec<String>,
}

/// The immutable config of a socket unit
pub struct SocketConfig {
    pub sockets: Vec<SingleSocketConfig>,
    pub filedesc_name: String,
    pub services: Vec<UnitId>,

    /// Accept= — whether the socket uses Accept mode (inetd-style).
    /// If true, for each incoming connection a new service instance is
    /// spawned. Defaults to false. Parsed and stored; inetd-style
    /// activation is not yet supported at runtime. See systemd.socket(5).
    pub accept: bool,

    /// MaxConnections= — the maximum number of simultaneous connections for
    /// Accept=yes sockets. Defaults to 64. Parsed and stored; no runtime
    /// enforcement yet. See systemd.socket(5).
    pub max_connections: u64,

    /// MaxConnectionsPerSource= — the maximum number of simultaneous
    /// connections for a service per source IP address. Defaults to the value
    /// of MaxConnections=. Parsed and stored; no runtime enforcement yet.
    /// See systemd.socket(5).
    pub max_connections_per_source: u64,

    /// SocketMode= — the file system access mode for the socket node
    /// (AF_UNIX sockets and FIFOs). Takes an octal access mode, e.g. 0666.
    /// Defaults to 0666. Parsed and stored; no runtime enforcement yet.
    /// See systemd.socket(5).
    pub socket_mode: Option<u32>,

    /// DirectoryMode= — the file system access mode for directories created
    /// for the socket (if needed). Takes an octal access mode, e.g. 0755.
    /// Defaults to 0755. Parsed and stored; no runtime enforcement yet.
    /// See systemd.socket(5).
    pub directory_mode: Option<u32>,

    /// PassCredentials= — whether to enable SO_PASSCRED on the socket, so
    /// that the receiving process can obtain peer credentials via an
    /// ancillary message. Defaults to false. Parsed and stored; no runtime
    /// enforcement yet. See systemd.socket(5).
    pub pass_credentials: bool,

    /// PassSecurity= — whether to enable SO_PASSSEC on the socket, so
    /// that AF_UNIX sockets receive the security context of the sending
    /// process in an ancillary message. Defaults to false. Parsed and
    /// stored; no runtime enforcement yet. See systemd.socket(5).
    pub pass_security: bool,

    /// AcceptFileDescriptors= — whether to enable SO_PASSRIGHTS on the
    /// socket, which when disabled prohibits the peer from sending
    /// SCM_RIGHTS ancillary messages (file descriptors) via AF_UNIX
    /// sockets. Defaults to true. Parsed and stored; no runtime
    /// enforcement yet. See systemd.socket(5).
    pub accept_file_descriptors: bool,

    /// RemoveOnStop= — whether to remove file nodes (AF_UNIX sockets,
    /// FIFOs, message queues, and symlinks configured with Symlinks=)
    /// created by this socket unit when it is stopped. Defaults to false.
    /// Parsed and stored; no runtime enforcement yet. See systemd.socket(5).
    pub remove_on_stop: bool,

    /// ReceiveBuffer= — the receive buffer size (SO_RCVBUF) in bytes for the
    /// socket. Takes an integer value. Parsed and stored; no runtime
    /// enforcement yet. See systemd.socket(5).
    pub receive_buffer: Option<u64>,

    /// SendBuffer= — the send buffer size (SO_SNDBUF) in bytes for the
    /// socket. Takes an integer value. Parsed and stored; no runtime
    /// enforcement yet. See systemd.socket(5).
    pub send_buffer: Option<u64>,

    /// Symlinks= — a list of file system paths to create as symlinks to
    /// the AF_UNIX socket path or FIFO path of this socket unit. Multiple
    /// paths may be specified space-separated; the directive may appear
    /// multiple times to extend the list. An empty value resets the list.
    /// Defaults to an empty list. Parsed and stored; no runtime enforcement
    /// yet. See systemd.socket(5).
    pub symlinks: Vec<String>,

    /// Timestamping= — controls the SO_TIMESTAMP or SO_TIMESTAMPNS socket
    /// options, enabling timestamping metadata on ingress network traffic.
    /// Takes one of "off", "us"/"usec"/"μs", or "ns"/"nsec".
    /// Defaults to Off. Parsed and stored; no runtime enforcement yet.
    /// See systemd.socket(5).
    pub timestamping: Timestamping,

    /// DeferTrigger= — controls whether to defer triggering the associated
    /// service when a connection comes in. Takes a boolean or "patient".
    /// Defaults to No. Parsed and stored; no runtime enforcement yet.
    /// See systemd.socket(5).
    pub defer_trigger: DeferTrigger,

    /// Writable= — whether to open the FIFO or special file for writing
    /// as well (i.e. O_RDWR rather than O_RDONLY). Defaults to false.
    /// Parsed and stored; no runtime enforcement yet. See systemd.socket(5).
    pub writable: bool,

    /// Backlog= — the maximum length of the queue of pending connections
    /// for stream sockets (the second argument to `listen()`). Takes an
    /// unsigned integer. Defaults to SOMAXCONN (typically 4096).
    /// See systemd.socket(5).
    pub backlog: Option<u32>,

    /// BindIPv6Only= — controls the IPV6_V6ONLY socket option. Takes
    /// "default", "both", or "ipv6-only". See systemd.socket(5).
    pub bind_ipv6_only: BindIPv6Only,

    /// BindToDevice= — bind the socket to a specific network interface
    /// (SO_BINDTODEVICE). Takes a network interface name. See systemd.socket(5).
    pub bind_to_device: Option<String>,

    /// SocketUser= — the UNIX user that owns the AF_UNIX socket file node.
    /// Takes a username or numeric UID. Defaults to root. See systemd.socket(5).
    pub socket_user: Option<String>,

    /// SocketGroup= — the UNIX group that owns the AF_UNIX socket file node.
    /// Takes a group name or numeric GID. Defaults to root. See systemd.socket(5).
    pub socket_group: Option<String>,

    /// FreeBind= — whether to set IP_FREEBIND/IPV6_FREEBIND, allowing binding
    /// to addresses not yet assigned to an interface. Defaults to false.
    /// See systemd.socket(5).
    pub free_bind: bool,

    /// Transparent= — whether to set IP_TRANSPARENT, allowing binding to
    /// non-local addresses and transparent proxying. Defaults to false.
    /// See systemd.socket(5).
    pub transparent: bool,

    /// Broadcast= — whether to set SO_BROADCAST on datagram sockets.
    /// Defaults to false. See systemd.socket(5).
    pub broadcast: bool,

    /// ReusePort= — whether to set SO_REUSEPORT, allowing multiple sockets
    /// to bind to the same port. Defaults to false. See systemd.socket(5).
    pub reuse_port: bool,

    /// KeepAlive= — whether to enable SO_KEEPALIVE on TCP sockets.
    /// Defaults to false. See systemd.socket(5).
    pub keep_alive: bool,

    /// KeepAliveTimeSec= — idle time before TCP keepalive probes start
    /// (TCP_KEEPIDLE). Takes a timespan in seconds. See systemd.socket(5).
    pub keep_alive_time_sec: Option<u64>,

    /// KeepAliveIntervalSec= — interval between TCP keepalive probes
    /// (TCP_KEEPINTVL). Takes a timespan in seconds. See systemd.socket(5).
    pub keep_alive_interval_sec: Option<u64>,

    /// KeepAliveProbes= — number of unacknowledged keepalive probes before
    /// connection is considered dead (TCP_KEEPCNT). See systemd.socket(5).
    pub keep_alive_probes: Option<u32>,

    /// NoDelay= — whether to enable TCP_NODELAY, disabling Nagle's algorithm.
    /// Defaults to false. See systemd.socket(5).
    pub no_delay: bool,

    /// Priority= — socket priority (SO_PRIORITY). See systemd.socket(5).
    pub priority: Option<i32>,

    /// Mark= — firewall mark (SO_MARK) for packets. See systemd.socket(5).
    pub mark: Option<u32>,

    /// IPTOS= — IP Type-Of-Service byte (IP_TOS). Takes an integer or
    /// one of "low-delay", "throughput", "reliability", "low-cost".
    /// See systemd.socket(5).
    pub ip_tos: Option<i32>,

    /// IPTTL= — IP Time-To-Live (IP_TTL), 1–255. See systemd.socket(5).
    pub ip_ttl: Option<u32>,

    /// PipeSize= — pipe buffer size (F_SETPIPE_SZ) in bytes for FIFOs.
    /// See systemd.socket(5).
    pub pipe_size: Option<u64>,

    /// FlushPending= — whether to flush the socket of pending data on
    /// the first accepted connection. Defaults to false. See systemd.socket(5).
    pub flush_pending: bool,

    /// TriggerLimitIntervalSec= — rate-limiting interval within which
    /// TriggerLimitBurst= activations are permitted. Defaults to 2s.
    /// See systemd.socket(5).
    pub trigger_limit_interval_sec: Option<u64>,

    /// TriggerLimitBurst= — maximum activations within the interval before
    /// entering failure state. Defaults to 200. See systemd.socket(5).
    pub trigger_limit_burst: Option<u32>,

    /// SocketProtocol= — socket protocol ("udplite" or "sctp").
    /// See systemd.socket(5).
    pub socket_protocol: Option<String>,

    /// SELinuxContextFromNet= — set SELinux context from the network peer.
    /// Defaults to false. See systemd.socket(5).
    pub selinux_context_from_net: bool,

    /// SmackLabel= — SMACK security label for the socket file node.
    /// See systemd.socket(5).
    pub smack_label: Option<String>,

    /// SmackLabelIPIn= — SMACK label for incoming IP packets.
    /// See systemd.socket(5).
    pub smack_label_ipin: Option<String>,

    /// SmackLabelIPOut= — SMACK label for outgoing IP packets.
    /// See systemd.socket(5).
    pub smack_label_ipout: Option<String>,

    /// PassPacketInfo= — whether to enable IP_PKTINFO (IPv4) or
    /// IPV6_RECVPKTINFO (IPv6) on the socket, so that the receiving
    /// process can determine the destination address and interface of
    /// incoming packets via ancillary messages. Defaults to false.
    /// See systemd.socket(5).
    pub pass_packet_info: bool,

    /// TCPCongestion= — the TCP congestion control algorithm to set on
    /// the socket (TCP_CONGESTION). Takes a string such as "cubic",
    /// "reno", "bbr", etc. If not set, the kernel default is used.
    /// See systemd.socket(5).
    pub tcp_congestion: Option<String>,

    /// ExecStartPre= — commands to execute before listening on the socket(s).
    /// Multiple lines are allowed and executed in order. Supports the same
    /// command prefixes as ExecStart= in service units ("-" for ignore-failure,
    /// "@" for argv[0] override). See systemd.socket(5).
    pub exec_start_pre: Vec<Commandline>,

    /// ExecStartPost= — commands to execute after listening on the socket(s)
    /// has been set up. See systemd.socket(5).
    pub exec_start_post: Vec<Commandline>,

    /// ExecStopPre= — commands to execute before closing the socket(s).
    /// See systemd.socket(5).
    pub exec_stop_pre: Vec<Commandline>,

    /// ExecStopPost= — commands to execute after closing the socket(s).
    /// See systemd.socket(5).
    pub exec_stop_post: Vec<Commandline>,

    /// TimeoutSec= — timeout for ExecStartPre=, ExecStartPost=,
    /// ExecStopPre=, and ExecStopPost= commands. If any of these commands
    /// does not finish within the configured time, the socket unit is
    /// considered failed. Takes a timespan value or "infinity".
    /// Defaults to DefaultTimeoutStartSec= from manager configuration.
    /// See systemd.socket(5).
    pub timeout_sec: Option<Timeout>,

    /// PassFileDescriptorsToExec= — whether to pass the socket file
    /// descriptors to ExecStartPost=, ExecStopPre=, and ExecStopPost=
    /// commands via the sd_listen_fds(3) protocol. Takes a boolean.
    /// Defaults to false. See systemd.socket(5). (systemd 257+)
    pub pass_file_descriptors_to_exec: bool,

    /// PollLimitIntervalSec= — rate-limiting interval for poll/select
    /// wakeups. If more than PollLimitBurst= wakeups occur within this
    /// interval, the socket is temporarily removed from poll. Defaults
    /// to 0 (disabled). See systemd.socket(5).
    pub poll_limit_interval_sec: Option<u64>,

    /// PollLimitBurst= — maximum poll/select wakeups within
    /// PollLimitIntervalSec= before the socket is temporarily paused.
    /// Defaults to 0 (disabled). See systemd.socket(5).
    pub poll_limit_burst: Option<u32>,

    pub exec_config: ExecConfig,
}
