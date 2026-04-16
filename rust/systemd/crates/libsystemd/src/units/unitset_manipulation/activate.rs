//! Activate units (recursively and parallel along the dependency tree)

use crate::lock_ext::{MutexExt, RwLockExt};
use crate::runtime_info::{ArcMutRuntimeInfo, RuntimeInfo, UnitTable};
use crate::services::ServiceErrorReason;
use crate::units::{
    CommonState, Specific, StatusStarted, StatusStopped, Timeout, Unit, UnitAction, UnitId,
    UnitStatus,
};

use log::{debug, error, info, trace, warn};
use std::sync::{Arc, Mutex};
use threadpool::ThreadPool;

/// Record a start timestamp for rate limiting purposes, without checking
/// the rate limit.  Used to count the initial activation against
/// StartLimitBurst.
fn record_start_timestamp(unit: &Unit) {
    fn record(common: &mut CommonState) {
        common.start_timestamps.push(std::time::Instant::now());
    }
    match &unit.specific {
        Specific::Service(s) => record(&mut s.state.write_poisoned().common),
        Specific::Socket(s) => record(&mut s.state.write_poisoned().common),
        Specific::Target(s) => record(&mut s.state.write_poisoned().common),
        Specific::Slice(s) => record(&mut s.state.write_poisoned().common),
        Specific::Mount(s) => record(&mut s.state.write_poisoned().common),
        Specific::Swap(s) => record(&mut s.state.write_poisoned().common),
        Specific::Timer(s) => record(&mut s.state.write_poisoned().common),
        Specific::Path(s) => record(&mut s.state.write_poisoned().common),
        Specific::Device(s) => record(&mut s.state.write_poisoned().common),
    }
}

/// Check and enforce the start rate limit (StartLimitBurst=/StartLimitIntervalSec=).
/// Returns `true` if the unit is allowed to start, `false` if rate-limited.
/// Also records the current timestamp as a start attempt.
pub(crate) fn check_start_rate_limit(unit: &Unit) -> bool {
    let burst = unit.common.unit.start_limit_burst.unwrap_or(5);
    let interval = match &unit.common.unit.start_limit_interval_sec {
        Some(Timeout::Duration(d)) => *d,
        Some(Timeout::Infinity) | None => std::time::Duration::from_secs(10),
    };

    // If burst is 0 or interval is zero, rate limiting is disabled.
    if burst == 0 || interval.is_zero() {
        return true;
    }

    // Helper to access and update CommonState behind the type-specific RwLock.
    fn check_and_record(
        common: &mut CommonState,
        burst: u32,
        interval: std::time::Duration,
    ) -> bool {
        let now = std::time::Instant::now();
        // Remove timestamps outside the window.
        common
            .start_timestamps
            .retain(|t| now.duration_since(*t) < interval);
        if common.start_timestamps.len() >= burst as usize {
            return false;
        }
        common.start_timestamps.push(now);
        true
    }

    match &unit.specific {
        Specific::Service(s) => {
            check_and_record(&mut s.state.write_poisoned().common, burst, interval)
        }
        Specific::Socket(s) => {
            check_and_record(&mut s.state.write_poisoned().common, burst, interval)
        }
        Specific::Target(s) => {
            check_and_record(&mut s.state.write_poisoned().common, burst, interval)
        }
        Specific::Slice(s) => {
            check_and_record(&mut s.state.write_poisoned().common, burst, interval)
        }
        Specific::Mount(s) => {
            check_and_record(&mut s.state.write_poisoned().common, burst, interval)
        }
        Specific::Swap(s) => {
            check_and_record(&mut s.state.write_poisoned().common, burst, interval)
        }
        Specific::Timer(s) => {
            check_and_record(&mut s.state.write_poisoned().common, burst, interval)
        }
        Specific::Path(s) => {
            check_and_record(&mut s.state.write_poisoned().common, burst, interval)
        }
        Specific::Device(s) => {
            check_and_record(&mut s.state.write_poisoned().common, burst, interval)
        }
    }
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct UnitOperationError {
    pub reason: UnitOperationErrorReason,
    pub unit_name: String,
    pub unit_id: UnitId,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum UnitOperationErrorReason {
    GenericStartError(String),
    GenericStopError(String),
    SocketOpenError(String),
    SocketCloseError(String),
    ServiceStartError(ServiceErrorReason),
    ServiceStopError(ServiceErrorReason),
    DependencyError(Vec<UnitId>),
}

impl std::fmt::Display for UnitOperationErrorReason {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::GenericStartError(msg) => write!(f, "start error: {msg}"),
            Self::GenericStopError(msg) => write!(f, "stop error: {msg}"),
            Self::SocketOpenError(msg) => write!(f, "socket open error: {msg}"),
            Self::SocketCloseError(msg) => write!(f, "socket close error: {msg}"),
            Self::ServiceStartError(e) => write!(f, "{e}"),
            Self::ServiceStopError(e) => write!(f, "{e}"),
            Self::DependencyError(ids) => {
                write!(f, "dependency error: ")?;
                for (i, id) in ids.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", id.name)?;
                }
                Ok(())
            }
        }
    }
}

impl std::fmt::Display for UnitOperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match &self.reason {
            UnitOperationErrorReason::GenericStartError(msg) => {
                write!(
                    f,
                    "Unit {} (ID {}) failed to start because: {}",
                    self.unit_name, self.unit_id, msg
                )?;
            }
            UnitOperationErrorReason::GenericStopError(msg) => {
                write!(
                    f,
                    "Unit {} (ID {}) failed to stop cleanly because: {}",
                    self.unit_name, self.unit_id, msg
                )?;
            }
            UnitOperationErrorReason::ServiceStartError(msg) => {
                write!(
                    f,
                    "Service {} (ID {}) failed to start because: {}",
                    self.unit_name, self.unit_id, msg
                )?;
            }
            UnitOperationErrorReason::ServiceStopError(msg) => {
                write!(
                    f,
                    "Service {} (ID {}) failed to stop cleanly because: {}",
                    self.unit_name, self.unit_id, msg
                )?;
            }
            UnitOperationErrorReason::SocketOpenError(msg) => {
                write!(
                    f,
                    "Socket {} (ID {}) failed to open because: {}",
                    self.unit_name, self.unit_id, msg
                )?;
            }
            UnitOperationErrorReason::SocketCloseError(msg) => {
                write!(
                    f,
                    "Socket {} (ID {}) failed to close cleanly because: {}",
                    self.unit_name, self.unit_id, msg
                )?;
            }
            UnitOperationErrorReason::DependencyError(ids) => {
                write!(
                    f,
                    "The unit {} (ID {}) failed to start/stop because these related units did not have the expected state: {:?}",
                    self.unit_name, self.unit_id, ids
                )?;
            }
        }
        Ok(())
    }
}

pub fn unstarted_deps(
    id: &UnitId,
    run_info: &RuntimeInfo,
    activation_set: Option<&[UnitId]>,
) -> Vec<UnitId> {
    let Some(unit) = run_info.unit_table.get(id) else {
        // If this occurs, there is a flaw in the handling of dependencies
        // IDs should be purged globally when units get removed
        return vec![];
    };

    // if not all dependencies are yet started ignore this call. This unit will be activated again when
    // the next dependency gets ready

    unit
        .common
        .dependencies
        .after
        .iter()
        .fold(Vec::new(), |mut acc, elem| {
            // Determine the relationship strength:
            // - "required" (Requires=/BindsTo=): must be Started
            // - "pulled" (Wants=): must have left NeverStarted
            // - "ordering only" (After= without any pull-dep): only block if
            //   the dep is actually being activated (not NeverStarted).  Pure
            //   ordering deps that stay in NeverStarted are ignored — they
            //   were never meant to be activated by this unit.  This matches
            //   real systemd where After=rescue.target does NOT activate
            //   rescue.target; it only orders them IF both are activated.
            let required = unit.common.dependencies.requires.contains(elem)
                || unit.common.dependencies.binds_to.contains(elem);
            let pulled = unit.common.dependencies.wants.contains(elem)
                || unit.common.dependencies.upholds.contains(elem);
            let is_pull_dep = required || pulled;

            let Some(elem_unit) = run_info.unit_table.get(elem) else {
                // Dependency not in unit table (e.g. optional unit that was
                // never loaded, or removed during pruning/cycle-breaking).
                // Treat it as ready so it doesn't block activation.
                warn!(
                    "Unit {:?} has an ordering dependency on {:?} which is not in the unit table. Ignoring.",
                    id, elem
                );
                return acc;
            };
            let status_locked = match elem_unit.common.status.read() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    warn!(
                        "Status lock poisoned for unit {:?} (dependency of {:?}). Treating as ready.",
                        elem, id
                    );
                    poisoned.into_inner()
                }
            };

            let ready = if required {
                // Hard dependency: must be fully started
                status_locked.is_started()
            } else if is_pull_dep {
                // Soft pull dependency (Wants=/Upholds=): the dep must have
                // completed (Started or Stopped) before we proceed.  After=
                // means "wait for completion", and Wants deps that are also
                // in the After list must finish before this unit starts.
                matches!(
                    &*status_locked,
                    UnitStatus::Started(_) | UnitStatus::Stopped(_, _)
                )
            } else {
                // Pure ordering dep (After= without Wants=/Requires=/BindsTo=):
                // Only block if the dep is actively being started (status is
                // Starting or some transient state).  If it's NeverStarted
                // AND not in the current activation subgraph, it's not going
                // to be activated — don't wait for it.
                // If it IS in the activation subgraph but still NeverStarted,
                // it's queued and we must wait (otherwise we'd start before
                // it runs, violating After= ordering).
                // If it's already finished (Started or Stopped), it's ready.
                match &*status_locked {
                    UnitStatus::NeverStarted => {
                        // Check if this dep is in the activation subgraph.
                        // If so, it WILL be activated and we must wait.
                        if let Some(set) = activation_set {
                            !set.contains(elem) // ready only if NOT in activation set
                        } else {
                            true // no activation set info — old behavior, treat as ready
                        }
                    }
                    UnitStatus::Starting => false,    // actively starting, wait for it
                    _ => true,                        // finished (started/stopped), ready
                }
            };

            if !ready {
                trace!(
                    "unstarted_deps: {:?} waiting for {:?} (required={}, pulled={}, status={}, ready={})",
                    id, elem, required, pulled, status_locked, ready
                );
                acc.push(elem.clone());
            }
            acc
        })
}

#[derive(Debug)]
pub enum StartResult {
    Started(Vec<UnitId>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationSource {
    Regular,
    SocketActivation,
    /// Activation triggered by a path or timer unit.  Like socket activation,
    /// this bypasses the StoppedFinal/StoppedUnexpected early-return guards so
    /// that a service can be re-started after it exits.
    TriggerActivation,
    /// Non-blocking activation: starts the service process but does NOT wait
    /// for READY=1 on Type=notify services.  This prevents the activation
    /// thread from holding the RuntimeInfo read lock indefinitely, which would
    /// deadlock write-lock requests (e.g. loading new unit files) due to
    /// glibc's writer-preferring RwLock.
    NonBlocking,
    /// Deferred notify wait: like Regular, but for Type=notify/NotifyReload
    /// services, the READY=1 wait is deferred to a background thread.
    /// The service process is forked and started, but the calling thread
    /// returns immediately.  A background thread polls `signaled_ready`
    /// (set by the global notification handler) and transitions the unit
    /// to Started when READY=1 is received.  This prevents thread-pool
    /// threads from holding the RuntimeInfo read lock indefinitely.
    DeferNotifyWait,
}

impl ActivationSource {
    #[must_use]
    pub const fn is_socket_activation(&self) -> bool {
        matches!(self, Self::SocketActivation)
    }

    /// Returns true for activation sources that should bypass the
    /// Stopped early-return guards (socket, path, timer triggers).
    #[must_use]
    pub const fn bypasses_stopped_guard(&self) -> bool {
        matches!(self, Self::SocketActivation | Self::TriggerActivation)
    }
}

/// Activate the unit and return all units that are ordered later than this unit
///
/// This also checks that all 'requires' relations are held up
pub fn activate_unit(
    id_to_start: UnitId,
    run_info: &RuntimeInfo,
    source: ActivationSource,
) -> std::result::Result<StartResult, UnitOperationError> {
    trace!("Activate id: {id_to_start:?}");

    let Some(unit) = run_info.unit_table.get(&id_to_start) else {
        // If this occurs, there is a flaw in the handling of dependencies
        // IDs should be purged globally when units get removed
        return Err(UnitOperationError {
            reason: UnitOperationErrorReason::GenericStartError(
                "Tried to activate a unit that can not be found".into(),
            ),
            unit_name: id_to_start.name.clone(),
            unit_id: id_to_start,
        });
    };

    // Early return if the unit has already been activated (or skipped/failed).
    // This prevents duplicate condition-check logs and redundant work when the
    // same unit appears in multiple before-chains.
    //
    // Socket activation bypasses the Stopped guards: when a socket receives
    // traffic the service must restart regardless of how it was stopped.
    {
        let status = unit.common.status.read_poisoned();
        match &*status {
            UnitStatus::Started(_) => {
                // Already running — nothing to do, don't re-dispatch before-chain
                return Ok(StartResult::Started(vec![]));
            }
            UnitStatus::Stopped(
                StatusStopped::StoppedFinal | StatusStopped::ConditionSkipped,
                _,
            ) if !source.bypasses_stopped_guard() => {
                // Already finished (e.g. condition-skipped oneshot) — don't re-check
                return Ok(StartResult::Started(vec![]));
            }
            UnitStatus::Stopped(StatusStopped::StoppedUnexpected, _)
                if !source.bypasses_stopped_guard() =>
            {
                // Already failed — don't retry during initial activation
                return Ok(StartResult::Started(vec![]));
            }
            UnitStatus::Starting => {
                // Already being started (e.g. DeferNotifyWait deferred the
                // READY=1 wait).  Don't try to activate (fork) again — the
                // process is already running.  Return empty before-chain
                // because the deferred wait thread already holds it and will
                // dispatch when READY=1 is received.
                return Ok(StartResult::Started(vec![]));
            }
            _ => {
                // NeverStarted, Stopping, Restarting — proceed.
                // Also: Stopped + SocketActivation — restart via socket traffic.
            }
        }
    }

    // Check unit assertions (AssertPathExists=, etc.) before activation.
    // Unlike conditions, if any assertion fails the unit enters a **failed**
    // state. This matches systemd's behavior where Assert* causes an error.
    for assertion in &unit.common.unit.assertions {
        if !assertion.check() {
            error!(
                "Assertion failed for unit {:?}: {:?}. Unit will fail.",
                id_to_start, assertion
            );
            let reason = UnitOperationErrorReason::GenericStartError(format!(
                "Assertion failed: {:?}",
                assertion
            ));
            {
                let mut status = unit.common.status.write_poisoned();
                *status =
                    UnitStatus::Stopped(StatusStopped::StoppedUnexpected, vec![reason.clone()]);
            }
            return Err(UnitOperationError {
                reason,
                unit_name: id_to_start.name.clone(),
                unit_id: id_to_start.clone(),
            });
        }
    }

    // Check unit conditions (ConditionPathExists=, etc.) before activation.
    // If any condition fails, the unit is skipped — this is not an error,
    // matching systemd's behavior of silently skipping condition-failed units.
    for condition in &unit.common.unit.conditions {
        if !condition.check() {
            trace!(
                "Condition failed for unit {:?}: {:?}. Skipping activation.",
                id_to_start, condition
            );
            // Mark the unit as stopped so that units with After= on this unit
            // see it as "no longer NeverStarted" and can proceed.  Real systemd
            // treats condition-failed units as successfully finished (they just
            // didn't need to do anything).
            {
                let mut status = unit.common.status.write_poisoned();
                *status = UnitStatus::Stopped(StatusStopped::StoppedFinal, vec![]);
            }
            // Return the next services so the dependency graph can still proceed.
            // The unit itself just won't be started.
            let next_services_ids = unit.common.dependencies.before.clone();
            return Ok(StartResult::Started(next_services_ids));
        }
    }

    // Stop any conflicting units before activating this one
    let conflicting_ids: Vec<UnitId> = unit
        .common
        .dependencies
        .conflicts
        .iter()
        .chain(unit.common.dependencies.conflicted_by.iter())
        .cloned()
        .collect();
    for conflict_id in &conflicting_ids {
        if let Some(conflict_unit) = run_info.unit_table.get(conflict_id) {
            let status = match conflict_unit.common.status.read() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    warn!(
                        "Status lock poisoned for conflicting unit {:?}. Skipping conflict check.",
                        conflict_id
                    );
                    poisoned.into_inner()
                }
            };
            if status.is_started() {
                drop(status);
                trace!(
                    "Stopping conflicting unit {:?} before starting {:?}",
                    conflict_id, id_to_start
                );
                if let Err(e) = conflict_unit.deactivate(run_info) {
                    warn!("Failed to stop conflicting unit {:?}: {}", conflict_id, e);
                }
            }
        }
    }

    let next_services_ids = unit.common.dependencies.before.clone();

    // Remember whether the unit was already started before we call activate().
    // If it was, we must NOT re-dispatch the before-chain — doing so would
    // cause an infinite livelock where already-started units keep returning
    // their full before-chain, which re-discovers them as "startable", etc.
    let was_already_started = {
        let status = unit.common.status.read_poisoned();
        status.is_started()
    };

    // Record a start timestamp for rate limiting.  This ensures the
    // initial activation counts against StartLimitBurst, matching real
    // systemd behavior.
    if !was_already_started {
        record_start_timestamp(unit);
        // Record lifecycle timestamp: leaving inactive state
        unit.common
            .timestamps
            .write_poisoned()
            .record_inactive_exit();
    }

    match unit.activate(run_info, source) {
        Ok(status) => {
            // If the unit was already in StoppedUnexpected state (i.e. it
            // previously failed and we skipped re-activation), don't
            // propagate the before-chain again — it was already propagated
            // when the unit first failed.  This prevents retry loops where
            // the activation graph keeps re-discovering the failed unit as
            // "startable" and spinning.
            if matches!(
                status,
                UnitStatus::Stopped(StatusStopped::StoppedUnexpected, _)
            ) {
                return Ok(StartResult::Started(vec![]));
            }

            // If the unit was already started before we tried to activate it,
            // unit.activate() returned early without doing anything.  Don't
            // re-dispatch the before-chain — those units have already been
            // (or are being) activated from the original activation.
            // Re-dispatching would cause an infinite loop where the same
            // units keep appearing as "startable" and re-enqueuing each other.
            if was_already_started {
                trace!(
                    "Unit {} was already started, not re-dispatching before-chain",
                    id_to_start.name
                );
                return Ok(StartResult::Started(vec![]));
            }

            // Deferred notify wait: the service process is started but the
            // READY=1 wait is deferred.  Return the before-chain so the
            // caller can dispatch it from a background thread after READY=1.
            // Skip Started-only steps (timestamps, slice activation) — the
            // background thread will handle them.
            if matches!(status, UnitStatus::Starting) {
                return Ok(StartResult::Started(next_services_ids));
            }

            // Record lifecycle timestamp: entered active state
            if !was_already_started {
                unit.common
                    .timestamps
                    .write_poisoned()
                    .record_active_enter();
            }

            // Activate the slice hierarchy for this unit so that
            // `systemctl is-active <slice>` returns "active".
            activate_slice_hierarchy(unit, run_info);

            Ok(StartResult::Started(next_services_ids))
        }
        Err(e) => {
            // FailureAction is handled in the deactivation path (unit.rs
            // deactivate / service_exit_handler) so that it fires when the
            // unit actually transitions to the failed state, not when the
            // start job itself returns an error.

            // For non-dependency errors (i.e. the unit genuinely failed to
            // start), still propagate the `before` chain so that units
            // ordered After= this one can proceed.  Real systemd does not
            // block After=-ordered units when a Wants= dependency fails;
            // only hard Requires=/BindsTo= failures propagate.  The
            // `unstarted_deps` check already handles that distinction:
            // optional (non-required) After= deps only need to have left
            // NeverStarted state, which a failed unit does.
            if !matches!(e.reason, UnitOperationErrorReason::DependencyError(_)) {
                trace!(
                    "Unit {} failed but propagating before-chain to {} units: {:?}",
                    id_to_start.name,
                    next_services_ids.len(),
                    next_services_ids
                        .iter()
                        .map(|id| id.name.as_str())
                        .collect::<Vec<&str>>()
                );
                // Also log the error so it's visible
                error!("Error while activating unit {e}");
                // Return the before-chain wrapped in Ok so the activation
                // graph keeps walking even though this unit failed.
                return Ok(StartResult::Started(next_services_ids));
            }

            Err(e)
        }
    }
}

/// Activate the slice hierarchy for a unit that just started.
/// In systemd, when a service runs in a slice, the slice and all its
/// ancestor slices are implicitly activated.
fn activate_slice_hierarchy(unit: &crate::units::Unit, run_info: &RuntimeInfo) {
    // Extract the slice name from the unit's specific config
    let slice_name = match &unit.specific {
        Specific::Service(svc) => svc.conf.slice.clone(),
        _ => None,
    };

    let Some(mut current_slice) = slice_name else {
        return;
    };

    // Walk up the slice hierarchy (e.g. "a-b-c.slice" → "a-b.slice" → "a.slice" → "-.slice")
    loop {
        if let Some(slice_unit) = run_info
            .unit_table
            .values()
            .find(|u| u.id.name == current_slice)
        {
            let mut status = slice_unit.common.status.write_poisoned();
            if !status.is_started() {
                trace!("Activating slice {} (member started)", current_slice);
                *status = UnitStatus::Started(StatusStarted::Running);
            }
        }

        // Move to parent slice
        if current_slice == "-.slice" {
            break;
        }
        let base = current_slice
            .strip_suffix(".slice")
            .unwrap_or(&current_slice);
        if let Some(last_dash) = base.rfind('-') {
            current_slice = format!("{}.slice", &base[..last_dash]);
        } else {
            // Reached a top-level slice, activate -.slice (root)
            current_slice = "-.slice".to_owned();
        }
    }
}

/// Execute a `SuccessAction=` or `FailureAction=` by initiating the
/// appropriate system transition.
///
/// For the `-force` variants the service manager exits immediately after
/// minimal cleanup.  For the `-immediate` variants we call
/// `std::process::exit` without any cleanup at all.  The non-force variants
/// trigger a clean shutdown via the existing `shutdown_sequence` path.
///
/// Because we may not have access to `ArcMutRuntimeInfo` at every call-site
/// (e.g. inside `activate_unit` which only borrows `&RuntimeInfo`), the
/// heavy system actions (`reboot`, `poweroff`, `halt`) are executed by
/// spawning the corresponding system command, which is the same strategy
/// systemd uses when it is *not* PID 1.  The clean-shutdown path is handled
/// by the global `SHUTTING_DOWN` flag in `crate::shutdown`.
pub fn execute_unit_action(action: &UnitAction, unit_name: &str) {
    match action {
        UnitAction::None => {}

        // ── exit ────────────────────────────────────────────────────
        UnitAction::Exit | UnitAction::ExitForce => {
            info!("{unit_name}: executing {action:?} — exiting service manager");
            std::process::exit(0);
        }

        // ── reboot ──────────────────────────────────────────────────
        UnitAction::Reboot | UnitAction::RebootForce => {
            info!("{unit_name}: executing {action:?} — requesting reboot");
            let _ = std::process::Command::new("reboot").status();
            // If the command fails (e.g. not PID 1), exit ourselves.
            std::process::exit(0);
        }
        UnitAction::RebootImmediate => {
            info!("{unit_name}: executing RebootImmediate");
            #[cfg(target_os = "linux")]
            unsafe {
                libc::reboot(libc::LINUX_REBOOT_CMD_RESTART);
            }
            std::process::exit(0);
        }

        // ── poweroff ────────────────────────────────────────────────
        UnitAction::Poweroff | UnitAction::PoweroffForce => {
            info!("{unit_name}: executing {action:?} — requesting poweroff");
            let _ = std::process::Command::new("poweroff").status();
            std::process::exit(0);
        }
        UnitAction::PoweroffImmediate => {
            info!("{unit_name}: executing PoweroffImmediate");
            #[cfg(target_os = "linux")]
            unsafe {
                libc::reboot(libc::LINUX_REBOOT_CMD_POWER_OFF);
            }
            std::process::exit(0);
        }

        // ── halt ────────────────────────────────────────────────────
        UnitAction::Halt | UnitAction::HaltForce => {
            info!("{unit_name}: executing {action:?} — requesting halt");
            let _ = std::process::Command::new("halt").status();
            std::process::exit(0);
        }
        UnitAction::HaltImmediate => {
            info!("{unit_name}: executing HaltImmediate");
            #[cfg(target_os = "linux")]
            unsafe {
                libc::reboot(libc::LINUX_REBOOT_CMD_HALT);
            }
            std::process::exit(0);
        }

        // ── kexec ───────────────────────────────────────────────────
        UnitAction::Kexec | UnitAction::KexecForce => {
            info!("{unit_name}: executing {action:?} — requesting kexec");
            // kexec is a specialised reboot; fall back to regular reboot
            // if kexec isn't available.
            let _ = std::process::Command::new("kexec").arg("-e").status();
            let _ = std::process::Command::new("reboot").status();
            std::process::exit(0);
        }
        UnitAction::KexecImmediate => {
            info!("{unit_name}: executing KexecImmediate");
            #[cfg(target_os = "linux")]
            unsafe {
                libc::reboot(libc::LINUX_REBOOT_CMD_KEXEC);
            }
            std::process::exit(0);
        }
    }
}

/// Walk the unit graph and find all units that need to be started to be able to start all units in `ids_to_start`.
///
/// This extends the `ids_to_start` with the additional ids
pub fn collect_unit_start_subgraph(ids_to_start: &mut Vec<UnitId>, unit_table: &UnitTable) {
    // iterate until the set-size doesnt change anymore. This works because there is only a finite set of units that can be added here.
    // This requires that ids only appear once in the set
    loop {
        let mut new_ids = Vec::new();
        for id in ids_to_start.iter() {
            if let Some(unit) = unit_table.get(id) {
                new_ids.extend(unit.common.dependencies.start_before_this());
                new_ids.extend(unit.common.dependencies.start_concurrently_with_this());

                // Include socket-activation services in the subgraph so
                // they are eagerly started alongside their socket unit.
                // Without this, services like dbus.service (which are only
                // referenced via socket activation, not via Wants=/Requires=)
                // are filtered out of the activation subgraph and only start
                // on-demand when the first connection arrives.  This causes a
                // race: pam_systemd tries to talk to logind via D-Bus before
                // dbus-daemon is fully ready, producing repeated "System error"
                // login failures until D-Bus catches up.
                if let Specific::Socket(specific) = &unit.specific {
                    new_ids.extend(specific.conf.services.iter().cloned());
                }
            }
        }
        new_ids.sort();
        new_ids.dedup();
        new_ids.retain(|id| !ids_to_start.contains(id));

        if new_ids.is_empty() {
            break;
        }
        ids_to_start.extend(new_ids);
    }
}

/// Collects the subgraph of units that need to be started to reach the `target_id` (Note: not required to be a unit of type .target).
///
/// Then starts these units as concurrently as possible respecting the before <-> after ordering
pub fn activate_needed_units(
    target_id: UnitId,
    run_info: ArcMutRuntimeInfo,
) -> Vec<UnitOperationError> {
    activate_needed_units_with_source(target_id, run_info, ActivationSource::Regular)
}

/// Like [`activate_needed_units`] but with a custom [`ActivationSource`].
///
/// `NonBlocking` skips the READY=1 wait for Type=notify services, preventing
/// thread-pool threads from holding the RuntimeInfo read lock indefinitely.
pub fn activate_needed_units_with_source(
    target_id: UnitId,
    run_info: ArcMutRuntimeInfo,
    source: ActivationSource,
) -> Vec<UnitOperationError> {
    let mut needed_ids = vec![target_id.clone()];
    {
        let run_info = run_info.read_poisoned();
        collect_unit_start_subgraph(&mut needed_ids, &run_info.unit_table);
    }
    info!(
        "activate_needed_units: target={}, {} units in subgraph",
        target_id.name,
        needed_ids.len(),
    );
    if log::log_enabled!(log::Level::Trace) {
        let needed_names: Vec<&str> = needed_ids.iter().map(|id| id.name.as_str()).collect();
        trace!("activate_needed_units: units: {:?}", needed_names);
    }

    // Note on Type=idle services (e.g. getty):
    //
    // Real systemd delays idle services until all other active jobs have
    // been dispatched (with a 5s timeout).  We previously separated idle
    // services into a Phase 2 that ran after Phase 1 completed.  However,
    // this caused a deadlock: Phase 1 threads hold read locks on the
    // RuntimeInfo RwLock, and if any control command (e.g. from a udev
    // rule running `systemctl`) requests a write lock, glibc's
    // write-preferring RwLock blocks all subsequent read lock requests —
    // including Phase 2 threads trying to start idle services.
    //
    // The fix: include idle services in the normal activation graph.
    // They naturally end up ordered After= their dependencies (e.g.
    // systemd-user-sessions.service) and will be started when those
    // complete.  The slight output interleaving is acceptable; a
    // non-booting system is not.

    // collect all 'root' units. These are units that do not have any 'after' relations to other unstarted units.
    // These can be started and the the graph can be traversed and other units can be started as soon as
    // all other units they depend on are started. This works because the units form an DAG if only
    // the 'after' relations are considered for traversal.
    let root_units =
        { find_startable_units(&needed_ids, &run_info.read_poisoned(), Some(&needed_ids)) };
    let root_names: Vec<&str> = root_units.iter().map(|id| id.name.as_str()).collect();
    trace!(
        "activate_needed_units: root units count={}: {:?}",
        root_units.len(),
        root_names
    );

    // Use a generous thread pool so that slow-starting notify services
    // (which block a thread while waiting for READY=1) don't starve
    // oneshot/target activations that could complete immediately.
    let tpool = ThreadPool::new(32);
    let errors = Arc::new(Mutex::new(Vec::new()));
    activate_units_recursive(
        root_units,
        Arc::new(needed_ids),
        run_info.clone(),
        tpool.clone(),
        errors.clone(),
        source,
    );

    tpool.join();
    info!("activate_needed_units: activation complete, all jobs dispatched");

    // Post-activation: check for upheld units that failed to start.
    // If a unit is upheld by an active unit but failed to activate
    // (e.g. due to a dependency failure), spawn a retry loop.
    {
        let ri = run_info.read_poisoned();
        for (uid, unit) in ri.unit_table.iter() {
            if unit.common.dependencies.upheld_by.is_empty() {
                continue;
            }
            let status = unit.common.status.read_poisoned();
            if status.is_started() {
                continue; // already running
            }
            let any_active = unit
                .common
                .dependencies
                .upheld_by
                .iter()
                .any(|upholding_id| {
                    ri.unit_table
                        .get(upholding_id)
                        .map(|u| u.common.status.read_poisoned().is_started())
                        .unwrap_or(false)
                });
            if any_active {
                trace!(
                    "Upheld unit {} not started after activation, scheduling retry",
                    uid.name
                );
                drop(status);
                {
                    let mut st = unit.common.status.write_poisoned();
                    if !st.is_started() {
                        *st = UnitStatus::NeverStarted;
                    }
                }
                let uid_clone = uid.clone();
                let arc_ri = run_info.clone();
                std::thread::spawn(move || {
                    upholds_retry_loop(uid_clone, arc_ri);
                });
            }
        }
    }

    trace!("activate_needed_units: all activation complete");
    // TODO can we handle errors in a more meaningful way?
    let errs = (*errors.lock_poisoned()).clone();
    for err in &errs {
        error!("Error while activating unit graph: {err}");
    }
    errs
}

/// Check for all units in this Vec, if all units this depends on are running
fn find_startable_units(
    ids: &Vec<UnitId>,
    run_info: &RuntimeInfo,
    activation_set: Option<&[UnitId]>,
) -> Vec<UnitId> {
    let mut startable = Vec::new();

    for id in ids {
        if unstarted_deps(id, run_info, activation_set).is_empty() {
            startable.push(id.clone());
        }
    }
    startable
}

/// Start all units in `ids_to_start` and push jobs into the threadpool to start all following units.
///
/// Only do so for the units in `filter_ids`
fn activate_units_recursive(
    ids_to_start: Vec<UnitId>,
    filter_ids: Arc<Vec<UnitId>>,
    run_info: ArcMutRuntimeInfo,
    tpool: ThreadPool,
    errors: Arc<Mutex<Vec<UnitOperationError>>>,
    source: ActivationSource,
) {
    // Log what we were called with (only interesting units, not empty calls)
    if !ids_to_start.is_empty() {
        let input_names: Vec<&str> = ids_to_start.iter().map(|id| id.name.as_str()).collect();
        info!(
            "activate_units_recursive: called with {} ids: {:?}",
            ids_to_start.len(),
            input_names
        );
    }

    let startables =
        { find_startable_units(&ids_to_start, &run_info.read_poisoned(), Some(&filter_ids)) };
    let startables: Vec<UnitId> = startables
        .into_iter()
        .filter(|id| filter_ids.contains(id))
        .collect();

    if !startables.is_empty() {
        let names: Vec<&str> = startables.iter().map(|id| id.name.as_str()).collect();
        info!("activate_units_recursive: startable units: {:?}", names);
    }
    if !ids_to_start.is_empty() && startables.is_empty() {
        let run_info_guard = run_info.read_poisoned();
        for id in &ids_to_start {
            if filter_ids.contains(id) {
                let unstarted = unstarted_deps(id, &run_info_guard, Some(&filter_ids));
                if !unstarted.is_empty() {
                    let dep_names: Vec<&str> = unstarted.iter().map(|d| d.name.as_str()).collect();
                    debug!(
                        "activate_units_recursive: {} NOT startable, waiting for: {:?}",
                        id.name, dep_names
                    );
                }
            } else {
                info!(
                    "activate_units_recursive: {} filtered out (not in activation subgraph)",
                    id.name
                );
            }
        }
        drop(run_info_guard);
    }

    for id in startables {
        // make copies to move into the closure
        let run_info_copy = run_info.clone();
        let tpool_copy = tpool.clone();
        let errors_copy = errors.clone();
        let filter_ids_copy = filter_ids.clone();
        tpool.execute(move || {
            let unit_name = id.name.clone();
            let id_saved = id.clone();

            // Use DeferNotifyWait to prevent Type=notify services from
            // blocking the thread pool thread while holding the RuntimeInfo
            // read lock.  For non-notify services, DeferNotifyWait behaves
            // identically to Regular.
            let effective_source = match source {
                ActivationSource::Regular => ActivationSource::DeferNotifyWait,
                other => other,
            };

            // Hold the RuntimeInfo read lock in a named variable so we can
            // reuse it for the post-activation status check without acquiring
            // a second read lock (which would deadlock on glibc's
            // writer-preferring rwlock if a writer is pending).
            let ri_guard = run_info_copy.read_poisoned();
            let result = activate_unit(id, &ri_guard, effective_source);

            match result {
                Ok(StartResult::Started(next_services_ids)) => {
                    // activate_unit may swallow errors (converting to Ok) for
                    // graph walking.  Check actual unit status and trigger
                    // OnFailure= if the unit ended up in a failed state.
                    let needs_on_failure = if let Some(unit) =
                        ri_guard.unit_table.get(&id_saved)
                    {
                        let status = unit.common.status.read_poisoned();
                        matches!(
                            &*status,
                            UnitStatus::Stopped(StatusStopped::StoppedUnexpected, errs) if !errs.is_empty()
                        )
                    } else {
                        false
                    };

                    // Check if READY=1 wait was deferred (unit still Starting).
                    let is_deferred = if let Some(unit) =
                        ri_guard.unit_table.get(&id_saved)
                    {
                        matches!(
                            &*unit.common.status.read_poisoned(),
                            UnitStatus::Starting
                        )
                    } else {
                        false
                    };

                    // Drop the read lock before triggering OnFailure= (which
                    // may need a write lock via find_or_load_unit).
                    drop(ri_guard);
                    if needs_on_failure {
                        trigger_on_failure_units(&id_saved, &run_info_copy);
                    }

                    if is_deferred {
                        // Wake the global notification handler so it re-collects
                        // sockets (including the new service's notification
                        // socket) and can process READY=1 notifications.
                        {
                            let ri = run_info_copy.read_poisoned();
                            ri.notify_eventfds();
                        }

                        // Type=notify service with deferred READY=1 wait.
                        // Spawn a background thread (NOT in the thread pool,
                        // so tpool.join() won't wait for it) to poll for
                        // signaled_ready and dispatch the before-chain.
                        info!(
                            "activate_units_recursive: {} deferred notify wait, spawning background thread",
                            unit_name
                        );
                        let run_info_bg = run_info_copy;
                        let id_bg = id_saved;
                        let filter_ids_bg = filter_ids_copy;
                        let errors_bg = errors_copy;
                        std::thread::Builder::new()
                            .name(format!("notify-wait-{}", unit_name))
                            .spawn(move || {
                                deferred_notify_wait_and_dispatch(
                                    id_bg,
                                    next_services_ids,
                                    filter_ids_bg,
                                    run_info_bg,
                                    errors_bg,
                                    source,
                                );
                            })
                            .expect("Failed to spawn deferred notify wait thread");
                    } else {
                        // Normal path: dispatch before-chain immediately.
                        if !next_services_ids.is_empty() {
                            let next_names: Vec<&str> = next_services_ids
                                .iter()
                                .map(|id| id.name.as_str())
                                .collect();
                            info!(
                                "activate_units_recursive: {} completed, dispatching {} next: {:?}",
                                unit_name,
                                next_services_ids.len(),
                                next_names
                            );
                        } else {
                            info!(
                                "activate_units_recursive: {} completed with empty before-chain",
                                unit_name
                            );
                        }

                        // make copies to move into the closure
                        let run_info_copy2 = run_info_copy.clone();
                        let tpool_copy2 = tpool_copy.clone();
                        let errors_copy2 = errors_copy.clone();
                        let filter_ids_copy2 = filter_ids_copy.clone();

                        let next_services_job = move || {
                            activate_units_recursive(
                                next_services_ids,
                                filter_ids_copy2,
                                run_info_copy2,
                                tpool_copy2,
                                errors_copy2,
                                source,
                            );
                        };
                        tpool_copy.execute(next_services_job);
                    }
                }
                Err(e) => {
                    // Drop the read lock before triggering OnFailure= (which
                    // may need a write lock via find_or_load_unit).
                    drop(ri_guard);
                    if let UnitOperationErrorReason::DependencyError(_) = e.reason {
                        // Thats ok. The unit is waiting for more dependencies and will be
                        // activated again when another dependency has finished starting

                        // This should not happen though, since we filter the units beforehand
                        // to only get the startables
                    } else {
                        error!("Error while activating unit {e}");
                        // Trigger OnFailure= units for the failed unit.
                        trigger_on_failure_units(&e.unit_id, &run_info_copy);
                        errors_copy.lock_poisoned().push(e);
                    }
                }
            }
        });
    }
}

/// Background thread for deferred Type=notify READY=1 wait.
///
/// Polls `signaled_ready` (set by the global notification handler when
/// READY=1 is received on the service's notification socket) and transitions
/// the unit from Starting → Started.  Then dispatches the before-chain so
/// dependent units can proceed.
///
/// This runs outside the activation thread pool so that `tpool.join()` can
/// complete without waiting for potentially-infinite READY=1 waits.
fn deferred_notify_wait_and_dispatch(
    id: UnitId,
    next_services_ids: Vec<UnitId>,
    filter_ids: Arc<Vec<UnitId>>,
    run_info: ArcMutRuntimeInfo,
    errors: Arc<Mutex<Vec<UnitOperationError>>>,
    source: ActivationSource,
) {
    let name = id.name.clone();

    // Extract the start timeout from the service config.
    let timeout = {
        let ri = run_info.read_poisoned();
        if let Some(unit) = ri.unit_table.get(&id)
            && let Specific::Service(svc) = &unit.specific
        {
            let state = svc.state.read_poisoned();
            state.srvc.get_start_timeout(&svc.conf)
        } else {
            None
        }
    };

    let start_time = std::time::Instant::now();

    // Poll until READY=1 is received, the unit leaves Starting state
    // (e.g. process exited / was killed), or the start timeout expires.
    loop {
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Check timeout
        if let Some(timeout) = timeout
            && start_time.elapsed() > timeout
        {
            warn!(
                "deferred_notify_wait: {} timed out after {:?} waiting for READY=1",
                name, timeout
            );
            // Kill the service process and clean up properly — matching the
            // cleanup in Service::deactivate_service (PID, process_group,
            // notification socket, stdout/stderr).
            if let Ok(ri) = run_info.try_read()
                && let Some(unit) = ri.unit_table.get(&id)
            {
                if let Specific::Service(svc) = &unit.specific {
                    let mut state = svc.state.write_poisoned();
                    state.srvc.kill_all_remaining_processes(&svc.conf, &name);
                    state.srvc.pid = None;
                    state.srvc.process_group = None;
                    if let Some(path) = state.srvc.notifications_path.take() {
                        let _ = std::fs::remove_file(&path);
                    }
                    state.srvc.notifications = None;
                    state.srvc.stdout = None;
                    state.srvc.stderr = None;
                    drop(state);
                }
                // Transition to Stopped/Failed
                let mut status = unit.common.status.write_poisoned();
                if matches!(&*status, UnitStatus::Starting) {
                    *status = UnitStatus::Stopped(
                        crate::units::status::StatusStopped::StoppedUnexpected,
                        vec![UnitOperationErrorReason::GenericStartError(format!(
                            "Timed out waiting for READY=1 ({:?})",
                            timeout
                        ))],
                    );
                }
            }
            return;
        }

        // Use try_read() to yield to pending writers (e.g., find_or_load_unit
        // needing a write lock to load a new unit from disk).
        let ri = match run_info.try_read() {
            Ok(g) => g,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => continue, // retry next iteration
        };
        let Some(unit) = ri.unit_table.get(&id) else {
            return;
        };

        // If the unit is no longer Starting, something else handled it
        // (e.g. SIGCHLD exit handler transitioned it to Stopped/Failed).
        {
            let status = unit.common.status.read_poisoned();
            if !matches!(&*status, UnitStatus::Starting) {
                trace!(
                    "deferred_notify_wait: {} left Starting state ({:?}), stopping poll",
                    name, &*status
                );
                return;
            }
        }

        // Check if the global notification handler has set signaled_ready.
        let ready = if let Specific::Service(svc) = &unit.specific {
            svc.state.read_poisoned().srvc.signaled_ready
        } else {
            return;
        };

        if ready {
            info!(
                "deferred_notify_wait: {} received READY=1, transitioning to Started",
                name
            );

            // Update service state and unit status under brief locks.
            if let Specific::Service(svc) = &unit.specific {
                let mut state = svc.state.write_poisoned();
                state.srvc.signaled_ready = false;
                state.srvc.runtime_started_at = Some(std::time::Instant::now());
                // Initialize watchdog reference timestamp from READY=1 moment.
                if svc.conf.watchdog_sec.is_some() && state.srvc.watchdog_last_ping.is_none() {
                    state.srvc.watchdog_last_ping = Some(std::time::Instant::now());
                }
            }

            // Transition unit status to Started.
            {
                let mut status = unit.common.status.write_poisoned();
                if matches!(&*status, UnitStatus::Starting) {
                    *status = UnitStatus::Started(StatusStarted::Running);
                }
            }

            // Record lifecycle timestamps.
            unit.common
                .timestamps
                .write_poisoned()
                .record_active_enter();

            // Activate slice hierarchy.
            activate_slice_hierarchy(unit, &ri);

            // Log the lifecycle event.
            let desc = unit.common.unit.description.clone();
            let log_level_max = unit.log_level_max();
            let msg = if desc.is_empty() {
                format!("Started {}.", name)
            } else {
                format!("Started {desc}.")
            };
            crate::control::varlink::journal_log_unit_lifecycle(
                &msg,
                &name,
                log_level_max.as_deref(),
            );

            drop(ri);

            // Dispatch the before-chain so dependent units can start.
            if !next_services_ids.is_empty() {
                let next_names: Vec<&str> = next_services_ids
                    .iter()
                    .map(|id| id.name.as_str())
                    .collect();
                info!(
                    "deferred_notify_wait: {} dispatching {} dependents: {:?}",
                    name,
                    next_services_ids.len(),
                    next_names
                );

                let tpool = ThreadPool::new(8);
                activate_units_recursive(
                    next_services_ids,
                    filter_ids,
                    run_info,
                    tpool.clone(),
                    errors,
                    source,
                );
                tpool.join();
            }
            return;
        }

        drop(ri);
    }
}

/// Trigger `OnFailure=` units when a non-service unit (e.g. socket) fails
/// to activate. Service units handle OnFailure in the exit handler; this
/// covers all other unit types.
fn trigger_on_failure_units(failed_id: &UnitId, run_info: &ArcMutRuntimeInfo) {
    let on_failure_targets: Vec<String> = {
        let ri = run_info.read_poisoned();
        let Some(unit) = ri.unit_table.get(failed_id) else {
            return;
        };
        // Services handle OnFailure in service_exit_handler — skip them here.
        if matches!(&unit.specific, Specific::Service(_)) {
            return;
        }
        unit.common.unit.on_failure.clone()
    };
    if on_failure_targets.is_empty() {
        return;
    }
    let source_name = failed_id.name.clone();
    for target_name in on_failure_targets {
        trace!("Triggering OnFailure={target_name} for unit {source_name}");
        let arc_ri = run_info.clone();
        let source = source_name.clone();
        std::thread::spawn(move || {
            let target_id = match crate::control::find_or_load_unit(&target_name, &arc_ri) {
                Ok(id) => id,
                Err(e) => {
                    warn!("Could not find OnFailure unit {target_name} for {source}: {e}");
                    return;
                }
            };
            // Reset the target from Stopped → NeverStarted so it can be activated.
            {
                let ri = arc_ri.read_poisoned();
                if let Some(target_unit) = ri.unit_table.get(&target_id) {
                    let mut status = target_unit.common.status.write_poisoned();
                    if matches!(
                        &*status,
                        UnitStatus::Stopped(_, _) | UnitStatus::NeverStarted
                    ) {
                        *status = UnitStatus::NeverStarted;
                    }
                }
            }
            let ri = arc_ri.read_poisoned();
            match crate::units::activate_unit(
                target_id,
                &ri,
                crate::units::ActivationSource::Regular,
            ) {
                Ok(_) => {
                    info!("OnFailure unit {target_name} activated for {source}");
                }
                Err(e) => {
                    warn!("Failed to activate OnFailure unit {target_name} for {source}: {e}");
                }
            }
        });
    }
}

/// Retry loop for upheld units. Keeps trying to restart an upheld unit
/// as long as any of its upholding units remain active. Uses exponential
/// backoff (500ms → 1s → 2s → ... capped at 30s) to avoid busy loops
/// when a dependency keeps failing.
pub fn upholds_retry_loop(unit_id: UnitId, arc_ri: ArcMutRuntimeInfo) {
    let mut delay = std::time::Duration::from_millis(500);
    let max_delay = std::time::Duration::from_secs(30);
    let max_retries = 120; // ~30 minutes with max delay

    for attempt in 0..max_retries {
        std::thread::sleep(delay);

        // Check if the unit is still upheld by an active unit
        let (should_retry, is_never_started) = {
            let ri = arc_ri.read_poisoned();
            let Some(unit) = ri.unit_table.get(&unit_id) else {
                return; // unit removed
            };
            let status = unit.common.status.read_poisoned();
            let is_started = status.is_started();
            if is_started {
                return; // already running, no need to retry
            }
            let any_upholding_active = unit.common.dependencies.upheld_by.iter().any(|uid| {
                ri.unit_table
                    .get(uid)
                    .map(|u| u.common.status.read_poisoned().is_started())
                    .unwrap_or(false)
            });
            if !any_upholding_active {
                return; // no upholding unit is active anymore
            }
            let is_ns = matches!(&*status, UnitStatus::NeverStarted);
            (true, is_ns)
        };

        if !should_retry {
            return;
        }

        // Reset to NeverStarted if needed so activate_unit picks it up
        if !is_never_started {
            let ri = arc_ri.read_poisoned();
            if let Some(unit) = ri.unit_table.get(&unit_id) {
                let mut status = unit.common.status.write_poisoned();
                if !status.is_started() {
                    *status = UnitStatus::NeverStarted;
                }
            }
        }

        // Also reset stopped dependencies so they can be retried
        {
            let ri = arc_ri.read_poisoned();
            if let Some(unit) = ri.unit_table.get(&unit_id) {
                let dep_ids: Vec<UnitId> = unit
                    .common
                    .dependencies
                    .requires
                    .iter()
                    .chain(unit.common.dependencies.wants.iter())
                    .cloned()
                    .collect();
                for dep_id in &dep_ids {
                    if let Some(dep) = ri.unit_table.get(dep_id) {
                        let mut st = dep.common.status.write_poisoned();
                        if st.is_stopped() {
                            *st = UnitStatus::NeverStarted;
                        }
                    }
                }
            }
        }

        let errs = activate_needed_units(unit_id.clone(), arc_ri.clone());
        if errs.is_empty() {
            info!(
                "Upholds= restarted {} (attempt {})",
                unit_id.name,
                attempt + 1
            );
            return;
        }
        for e in &errs {
            trace!(
                "Upholds= retry {} for {} failed: {}",
                attempt + 1,
                unit_id.name,
                e
            );
        }

        // Exponential backoff
        delay = std::cmp::min(delay * 2, max_delay);
    }
    warn!(
        "Upholds= gave up restarting {} after {} retries",
        unit_id.name, max_retries
    );
}
