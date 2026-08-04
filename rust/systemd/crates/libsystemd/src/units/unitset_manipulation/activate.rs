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
                // Dependency not in the unit table. A *required* (Requires=/
                // BindsTo=) dependency on a .device unit that isn't loaded yet
                // means udev has not announced the device — we must WAIT rather
                // than start prematurely. Otherwise systemd-fsck@… / …mount for
                // a by-label/by-uuid device runs before its /dev symlink exists
                // and fails, so the initrd root never mounts. The device unit is
                // created and marked Started from the later udev event, which
                // re-triggers activation of whatever was blocked on it.
                if required && matches!(elem.kind, crate::units::UnitIdKind::Device) {
                    trace!(
                        "unstarted_deps: {:?} waiting for not-yet-announced device {:?}",
                        id, elem
                    );
                    acc.push(elem.clone());
                    return acc;
                }
                // Otherwise (optional unit never loaded, or removed during
                // pruning/cycle-breaking): treat as ready so it doesn't block.
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

/// Are we the init of the machine itself, rather than a user manager or the
/// PID 1 of a container?
///
/// Mirrors the `MANAGER_IS_USER(m) || detect_container() > 0` test that guards
/// upstream's `exit` emergency action (emergency-action.c:153-170).  A user
/// manager is never PID 1, and a container payload is PID 1 only of its own
/// namespace, so in both cases exiting is harmless.  Exiting the machine's own
/// init is not: the kernel panics with "Attempted to kill init".
fn is_system_init() -> bool {
    if std::process::id() != 1 {
        return false;
    }
    // What nspawn and the other container managers set, and where PID 1
    // records it for everyone else.
    std::env::var_os("container").is_none()
        && !std::path::Path::new("/run/systemd/container").exists()
}

/// Resolve the status to propagate for a `SuccessAction=`/`FailureAction=`.
///
/// Upstream's `unit_success_action_exit_status()` /
/// `unit_failure_action_exit_status()` (unit.c:6283-6314): an explicit
/// `SuccessActionExitStatus=`/`FailureActionExitStatus=` wins, otherwise the
/// unit propagates its own main exit status, with 255 standing in for a process
/// that did not exit cleanly (upstream's -EBADE case).
pub fn resolve_action_exit_status(
    configured: Option<u8>,
    code: &crate::signal_handler::ChildTermination,
) -> Option<u8> {
    if let Some(status) = configured {
        return Some(status);
    }
    match code {
        crate::signal_handler::ChildTermination::Exit(c) => u8::try_from(*c).ok(),
        crate::signal_handler::ChildTermination::Signal(_) => Some(255),
    }
}

/// Execute a `SuccessAction=` or `FailureAction=` by initiating the
/// appropriate system transition.
///
/// `exit_status` is the value to propagate for the `exit` variants, as resolved
/// by [`resolve_action_exit_status`]; it is ignored by every other action.
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
pub fn execute_unit_action(action: &UnitAction, unit_name: &str, exit_status: Option<u8>) {
    match action {
        UnitAction::None => {}

        // ── exit ────────────────────────────────────────────────────
        UnitAction::Exit | UnitAction::ExitForce => {
            let status = exit_status.unwrap_or(0);
            if is_system_init() {
                // Upstream refuses to exit the machine's init: "exit" degrades
                // to "poweroff" and "exit-force" to "poweroff-force"
                // (emergency-action.c:164-170).  Exiting here would panic the
                // kernel instead of shutting the machine down.
                info!(
                    "{unit_name}: doing \"poweroff\" action instead of an \"exit\" emergency action"
                );
                let _ = std::process::Command::new("poweroff").status();
                std::process::exit(status as i32);
            }
            info!("{unit_name}: executing {action:?} — exiting service manager with {status}");
            std::process::exit(status as i32);
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
/// The unit the manager is currently trying to reach. Starts as the boot
/// target and is replaced whenever `systemctl isolate` switches goals (e.g.
/// the initrd isolating to `initrd-switch-root.target`). The background
/// re-drive uses this so asynchronous completions re-evaluate the CURRENT goal
/// rather than the static boot target — without it, a completion that occurs
/// after an isolate re-drives the already-reached boot target and the isolate
/// goal stalls.
static ACTIVE_GOAL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Record the unit the manager is now trying to reach.
pub fn set_active_goal(name: &str) {
    if let Ok(mut g) = ACTIVE_GOAL.lock() {
        *g = Some(name.to_owned());
    }
}

/// The unit the manager is currently trying to reach, if set.
pub fn active_goal() -> Option<String> {
    ACTIVE_GOAL.lock().ok().and_then(|g| g.clone())
}

/// Count of in-flight `activate_needed_units_with_source` calls. Used so the
/// background goal re-drive can skip while another activation is already
/// running, instead of launching a concurrent full activation (each spins up a
/// 32-thread pool) that would just contend locks and starve real progress.
static ACTIVATION_DEPTH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// True if at least one activation pass is currently running.
pub fn activation_in_flight() -> bool {
    ACTIVATION_DEPTH.load(std::sync::atomic::Ordering::SeqCst) > 0
}

struct ActivationDepthGuard;
impl ActivationDepthGuard {
    fn new() -> Self {
        ACTIVATION_DEPTH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}
impl Drop for ActivationDepthGuard {
    fn drop(&mut self) {
        ACTIVATION_DEPTH.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Pending-writer gate (the quiescent-point mechanism upstream gets for free
/// from its single-threaded event loop).  A table-wide mutator (daemon-reload)
/// sets this before spinning for the RuntimeInfo write lock; the hot
/// background readers (deferred completion pollers, the goal re-drive, new
/// activation pool jobs) back off while it is set, so a zero-reader window
/// reliably appears instead of the writer livelocking against 32 overlapping
/// 100ms read pulses.
static WRITER_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True while a table-wide writer (daemon-reload) is waiting for the lock.
/// Cooperative background readers should skip their next acquisition.
pub fn writer_pending() -> bool {
    WRITER_PENDING.load(std::sync::atomic::Ordering::SeqCst)
}

/// RAII guard announcing a pending table-wide writer.
pub struct WriterPendingGuard;
impl WriterPendingGuard {
    pub fn announce() -> Self {
        WRITER_PENDING.store(true, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}
impl Drop for WriterPendingGuard {
    fn drop(&mut self) {
        WRITER_PENDING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

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
    let _depth_guard = ActivationDepthGuard::new();
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
    // Use a generous thread pool so that slow-starting notify services
    // (which block a thread while waiting for READY=1) don't starve
    // oneshot/target activations that could complete immediately.
    let tpool = ThreadPool::new(32);
    let errors = Arc::new(Mutex::new(Vec::new()));
    let needed_ids = Arc::new(needed_ids);

    // Fixpoint sweep: re-run the forward walk until a full pass starts no new
    // unit. rust-systemd's activation propagates a "before-chain" as each unit
    // completes, but a unit with several ordering dependencies can be reached
    // (and dropped as not-yet-startable) before its LAST dependency completes,
    // and that last dependency only re-dispatches its own before-chain. Without
    // a fixpoint sweep, the tail of a fan-in chain — e.g. the /nix/store overlay
    // mount that waits on the ro-store mount, the rw-store mount, AND the
    // upper/work mkdir service — is nondeterministically left NeverStarted, so
    // the initrd never finishes and switch-root never fires. Re-walking until no
    // progress closes that systemic gap. activate_unit is idempotent for
    // already-Started/Starting/failed units (it early-returns with an empty
    // before-chain), so extra passes are cheap and cannot double-start anything.
    let count_started = |ids: &[UnitId]| -> usize {
        let ri = run_info.read_poisoned();
        ids.iter()
            .filter(|id| {
                ri.unit_table
                    .get(id)
                    .map(|u| u.common.status.read_poisoned().is_started())
                    .unwrap_or(false)
            })
            .count()
    };

    let max_passes = needed_ids.len() + 2;
    for pass in 0..max_passes {
        let started_before = count_started(&needed_ids);

        let root_units =
            { find_startable_units(&needed_ids, &run_info.read_poisoned(), Some(&needed_ids)) };

        activate_units_recursive(
            root_units,
            needed_ids.clone(),
            run_info.clone(),
            tpool.clone(),
            errors.clone(),
            source,
        );
        tpool.join();

        let started_after = count_started(&needed_ids);
        if started_after == started_before {
            // Fixpoint reached: this pass started nothing new. Any remaining
            // not-started units are either blocked on async completions handled
            // elsewhere (Type=notify deferred waits, udev device events) or
            // genuinely unsatisfiable.
            break;
        }
        if pass + 1 == max_passes {
            warn!(
                "activate_needed_units: hit max passes ({}) for target {} without reaching a fixpoint",
                max_passes, target_id.name
            );
        }
    }
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
        // `.device` units are activated exclusively by udev (see udev_event.rs
        // marking them Started(Plugged) when the kernel announces the device),
        // never by the job machinery. Force-starting one here would mark it
        // Started before its /dev node/symlink exists, so a unit that
        // BindsTo=/After= it — systemd-fsck@<dev>.service, the by-label mount —
        // would run against a missing device (fsck exits 1, the root never
        // mounts). Leave devices out of the startable set; a unit depending on a
        // device waits (unstarted_deps) until the udev event plugs it and
        // re-activates the dependents.
        if matches!(id.kind, crate::units::UnitIdKind::Device) {
            continue;
        }
        if unstarted_deps(id, run_info, activation_set).is_empty() {
            startable.push(id.clone());
        }
    }
    startable
}

/// Classify a just-activated service's deferral, all under the caller's
/// already-held table read guard (a second read acquisition would risk the
/// writer-preferring rwlock deadlock): whether the start is still pending (the
/// unit is left `Starting`), and if so whether it deferred at the
/// `ExecCondition=`/`ExecStartPre=` helper phase (`is_prestart_chain`) or as a
/// multi-command `Type=oneshot` preliminary-exec phase (`is_oneshot_prelim`).
/// Both use the lock-free `main_pid` atomic so no per-unit state lock is taken
/// under the table guard. Shared by the pool path and the increment-4 job-graph
/// drain so the two cannot diverge (docs/EVENT-LOOP.md).
fn detect_deferred_kind(ri: &RuntimeInfo, id: &UnitId) -> (bool, bool, bool) {
    let is_deferred = ri
        .unit_table
        .get(id)
        .is_some_and(|unit| matches!(&*unit.common.status.read_poisoned(), UnitStatus::Starting));
    let is_oneshot_prelim = if let Some(unit) = ri.unit_table.get(id)
        && let Specific::Service(svc) = &unit.specific
    {
        svc.conf.srcv_type == crate::units::ServiceType::OneShot
            && svc.conf.exec.len() > 1
            && unit.common.main_pid.load(std::sync::atomic::Ordering::Acquire) == 0
    } else {
        false
    };
    let is_prestart_chain = if let Some(unit) = ri.unit_table.get(id)
        && let Specific::Service(svc) = &unit.specific
    {
        (!svc.conf.exec_condition.is_empty() || !svc.conf.startpre.is_empty())
            && unit.common.main_pid.load(std::sync::atomic::Ordering::Acquire) == 0
    } else {
        false
    };
    (is_deferred, is_prestart_chain, is_oneshot_prelim)
}

/// Park a deferred service start on the dispatcher: send the
/// `StartServiceChain` / `StartOneshotChain` / `StartServiceWait` event that
/// matches the deferral kind, after waking the notification reader so it
/// collects the new service's socket (docs/EVENT-LOOP.md inc 2). Extracted from
/// `activate_units_recursive` so the increment-4 job-graph drain reuses the
/// exact parking path rather than diverging from it. `next_services_ids` is the
/// before-chain to dispatch when the start completes (the pool passes the real
/// dependents; the job graph passes empty and schedules dependents itself).
#[allow(clippy::too_many_arguments)]
fn park_deferred_start(
    id: UnitId,
    is_prestart_chain: bool,
    is_oneshot_prelim: bool,
    next_services_ids: Vec<UnitId>,
    filter_ids: Arc<Vec<UnitId>>,
    errors: Arc<Mutex<Vec<UnitOperationError>>>,
    source: ActivationSource,
    run_info: ArcMutRuntimeInfo,
    unit_name: &str,
) {
    // Wake the global notification handler so it re-collects sockets (including
    // the new service's notification socket) and can process READY=1.
    {
        let ri = run_info.read_poisoned();
        ri.notify_eventfds();
    }
    info!("activate_units_recursive: {unit_name} deferred start, parking on the dispatcher");
    if is_prestart_chain {
        // The ExecCondition=/ExecStartPre= phase runs as a dispatcher chain;
        // its main phase then routes to the oneshot chain or a start wait.
        let dispatcher = {
            let ri = run_info.read_poisoned();
            ri.dispatcher.clone()
        };
        dispatcher.send_normal(crate::entrypoints::dispatcher::Event::StartServiceChain(
            ServiceStartChain {
                id,
                next_services_ids,
                filter_ids,
                errors,
                source,
                phase: StartChainPhase::Condition(0),
            },
        ));
    } else if is_oneshot_prelim {
        // Hand the exec chain to the dispatcher: each preliminary command is
        // forked initiate-only and advanced by its ChildExit event.
        let dispatcher = {
            let ri = run_info.read_poisoned();
            ri.dispatcher.clone()
        };
        dispatcher.send_normal(crate::entrypoints::dispatcher::Event::StartOneshotChain(
            OneshotChainStart {
                id,
                next_services_ids,
                filter_ids,
                errors,
                source,
            },
        ));
    } else {
        // Park the deferred start; the dispatcher re-evaluates it on
        // Notify/ChildExit events and enforces its timeouts.
        match crate::entrypoints::dispatcher::global() {
            Some(handle) => handle.send_normal(
                crate::entrypoints::dispatcher::Event::StartServiceWait(StartWaitParams {
                    id,
                    next_services_ids: Some(next_services_ids),
                    filter_ids,
                    errors,
                    source,
                    check_starting: false,
                }),
            ),
            None => error!("no dispatcher to park the deferred start of {unit_name}"),
        }
    }
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

            // Yield to a pending table-wide writer before pinning a read
            // guard for the whole activation.  Bounded so a stalled writer
            // cannot wedge activation forever.
            let gate_start = std::time::Instant::now();
            while writer_pending() && gate_start.elapsed() < std::time::Duration::from_secs(10) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }

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

                    // Classify the deferral under the guard we already hold
                    // (shared with the increment-4 job-graph drain).
                    let (is_deferred, is_prestart_chain, is_oneshot_prelim) =
                        detect_deferred_kind(&ri_guard, &id_saved);

                    // Drop the read lock before triggering OnFailure= (which
                    // may need a write lock via find_or_load_unit).
                    drop(ri_guard);
                    if needs_on_failure {
                        trigger_on_failure_units(&id_saved, &run_info_copy);
                    }

                    if is_deferred {
                        park_deferred_start(
                            id_saved.clone(),
                            is_prestart_chain,
                            is_oneshot_prelim,
                            next_services_ids,
                            filter_ids_copy.clone(),
                            errors_copy.clone(),
                            source,
                            run_info_copy.clone(),
                            &unit_name,
                        );
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

/// Bounded worker pool for increment-4 job-graph activations. Activating a unit
/// can block (a mount waits on its device), so the drive must never run it on
/// the dispatcher thread; it hands each activation to this pool instead, which
/// also restores the concurrency the fixpoint sweep got from its own 32-worker
/// pool. Shared across drive passes and initialised on first use.
static JOB_GRAPH_POOL: std::sync::OnceLock<ThreadPool> = std::sync::OnceLock::new();

/// Increment-4 job-graph drive, run as step 4 of the single dispatcher loop
/// behind `SYSTEMD_RS_JOB_GRAPH=1` (docs/EVENT-LOOP.md, "The flag-on dispatch
/// belongs in the existing dispatcher loop"). One pass:
///
/// 1. Retire every `Running` job whose unit reached a terminal status (Started
///    or Stopped); a vanished unit's job is canceled.
/// 2. Fire JobTimeout (`pop_expired`) for any job past its deadline, so a stuck
///    unit self-terminates the closure instead of hanging the dispatcher.
/// 3. Re-prime readiness (`enqueue_ready`): every `Waiting` job whose ordering
///    prerequisites are now met joins the run queue. This is the completion
///    requeue — running it each pass, triggered by the dispatcher's own wakes
///    (Notify, ChildExit) rather than a per-completion hook threaded through
///    `dispatch_one`.
/// 4. Dispatch (`pop_ready`): activate each ready unit initiate-only, mirroring
///    the pool path (`DeferNotifyWait` for the fork, deferred starts parked on
///    this same dispatcher via [`park_deferred_start`] with an *empty*
///    before-chain because the graph schedules dependents itself), and arm the
///    job's timeout.
///
/// A no-op when the registry is empty, which is always the case with the flag
/// off, so the default dispatcher loop is untouched. Activation here must never
/// block the dispatcher; that holds for services (fork-and-return) and is why
/// the first slice is service-only (mounts move off-thread in inc 5).
pub fn drive_run_queue(run_info: &ArcMutRuntimeInfo) {
    use crate::units::jobs::{JobId, JobResult, JobState};

    let jobs = { run_info.read_poisoned().jobs.clone() };
    // Fast path: nothing enqueued means nothing to drive (the flag-off case and
    // the common flag-on idle case).
    if jobs.lock().unwrap().is_empty() {
        return;
    }

    // Phase 1: retire completed and timed-out jobs, then re-prime readiness.
    {
        let ri = run_info.read_poisoned();
        let mut reg = jobs.lock().unwrap();

        // The activation set for readiness is every unit that still holds a job.
        let activation_set: Vec<UnitId> = reg.iter().map(|job| job.unit.clone()).collect();
        let is_ready = |ri: &RuntimeInfo, u: &UnitId| {
            !matches!(u.kind, crate::units::UnitIdKind::Device)
                && unstarted_deps(u, ri, Some(&activation_set)).is_empty()
        };

        let running: Vec<(JobId, UnitId)> = reg
            .iter()
            .filter(|job| job.state == JobState::Running)
            .map(|job| (job.id, job.unit.clone()))
            .collect();
        for (jid, unit_id) in running {
            let status = ri
                .unit_table
                .get(&unit_id)
                .map(|u| u.common.status.read_poisoned().clone());
            match status {
                // Unit removed (e.g. daemon-reload): the job can never complete.
                None => {
                    reg.finish(jid, JobResult::Canceled);
                }
                Some(UnitStatus::Started(_)) => {
                    reg.finish(jid, JobResult::Done);
                }
                Some(UnitStatus::Stopped(_, errs)) => {
                    reg.finish(
                        jid,
                        if errs.is_empty() {
                            JobResult::Done
                        } else {
                            JobResult::Failed
                        },
                    );
                }
                // Still activating or not yet begun: leave the job running.
                _ => {}
            }
        }

        let now = std::time::Instant::now();
        for jid in reg.pop_expired(now) {
            warn!("job-graph: job {jid} exceeded its deadline, finishing as Timeout");
            reg.finish(jid, JobResult::Timeout);
        }

        reg.enqueue_ready(|u| is_ready(&ri, u));
    }

    // Phase 2: dispatch every ready job by activating its unit on the bounded
    // pool, never inline on the dispatcher (a blocking mount would stall every
    // other unit) and in parallel across the frontier. The job is marked Running
    // with its deadline before the activation is handed off, so Phase 1's scan
    // on the next dispatcher wake sees it in flight; the scan retires it once its
    // unit reaches a terminal status. `filter`/`errors` are the arguments the
    // parked-start continuation expects; the job graph hands an empty
    // before-chain, so `filter` only scopes readiness for a continuation that
    // will not dispatch dependents anyway.
    let filter: Arc<Vec<UnitId>> =
        Arc::new(jobs.lock().unwrap().iter().map(|j| j.unit.clone()).collect());
    let errors: Arc<Mutex<Vec<UnitOperationError>>> = Arc::new(Mutex::new(Vec::new()));
    let pool = JOB_GRAPH_POOL.get_or_init(|| ThreadPool::new(32));
    loop {
        let jid = { jobs.lock().unwrap().pop_ready() };
        let Some(jid) = jid else { break };
        let unit_id = match jobs.lock().unwrap().get(jid) {
            Some(job) => job.unit.clone(),
            None => continue,
        };
        {
            let mut reg = jobs.lock().unwrap();
            reg.set_running(jid);
            reg.set_deadline(
                jid,
                std::time::Instant::now() + std::time::Duration::from_secs(90),
            );
        }

        let run_info_c = run_info.clone();
        let jobs_c = jobs.clone();
        let filter_c = filter.clone();
        let errors_c = errors.clone();
        pool.execute(move || {
            let ri = run_info_c.read_poisoned();
            match activate_unit(unit_id.clone(), &ri, ActivationSource::DeferNotifyWait) {
                Ok(StartResult::Started(_before_chain)) => {
                    let (is_deferred, is_prestart_chain, is_oneshot_prelim) =
                        detect_deferred_kind(&ri, &unit_id);
                    drop(ri);
                    if is_deferred {
                        park_deferred_start(
                            unit_id.clone(),
                            is_prestart_chain,
                            is_oneshot_prelim,
                            Vec::new(),
                            filter_c,
                            errors_c,
                            ActivationSource::Regular,
                            run_info_c.clone(),
                            &unit_id.name,
                        );
                    }
                    // Non-deferred completions, and the eventual deferred
                    // completion, are retired by the dispatcher's next scan.
                }
                Err(e) => {
                    drop(ri);
                    error!("job-graph: failed to activate {}: {e}", unit_id.name);
                    trigger_on_failure_units(&unit_id, &run_info_c);
                    jobs_c.lock().unwrap().finish(jid, JobResult::Failed);
                }
            }
        });
    }
}

/// Increment-4 job-graph replacement for [`activate_needed_units`], used behind
/// `SYSTEMD_RS_JOB_GRAPH` to bring a whole start closure up through the single
/// dispatcher instead of the fixpoint sweep. Installs a job per unit in the
/// closure, primes the initially-startable ones and wakes the dispatcher (whose
/// [`drive_run_queue`] activates each on the bounded pool, retires completions on
/// its next scan, and requeues), then blocks until the registry drains so the
/// caller keeps the synchronous activation contract. The periodic wake catches
/// process-less completions (targets, condition-skips) that fire no event; the
/// overall deadline is a safety net above the per-job timeouts. The returned
/// vector is always empty (errors surface per job); the signature matches
/// `activate_needed_units` for a drop-in swap at the call site.
pub fn activate_needed_units_via_job_graph(
    target_id: UnitId,
    run_info: ArcMutRuntimeInfo,
) -> Vec<UnitOperationError> {
    use crate::units::jobs::{JobKind, JobMode};

    let jobs = { run_info.read_poisoned().jobs.clone() };

    let subgraph = {
        let ri = run_info.read_poisoned();
        let mut ids = vec![target_id.clone()];
        collect_unit_start_subgraph(&mut ids, &ri.unit_table);
        ids
    };
    info!(
        "activate_needed_units_via_job_graph: target={}, {} units in closure",
        target_id.name,
        subgraph.len()
    );

    {
        let ri = run_info.read_poisoned();
        let mut reg = jobs.lock().unwrap();
        for uid in &subgraph {
            let _ = reg.create(
                uid.clone(),
                JobKind::Start,
                ActivationSource::Regular,
                JobMode::Replace,
            );
        }
        reg.enqueue_ready(|u| {
            !matches!(u.kind, crate::units::UnitIdKind::Device)
                && unstarted_deps(u, &ri, Some(&subgraph)).is_empty()
        });
        ri.dispatcher
            .send_normal(crate::entrypoints::dispatcher::Event::JobQueued);
    }

    // The dispatcher drive owns the closure now; block until it drains. The wake
    // every 200ms advances process-less frontiers (which fire no completion
    // event); real starts advance the drive through their Notify/ChildExit
    // events. The overall deadline is a boot-wide safety net.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        if jobs.lock().unwrap().is_empty() {
            break;
        }
        if std::time::Instant::now() > deadline {
            warn!("activate_needed_units_via_job_graph: closure did not drain within 300s");
            break;
        }
        {
            let ri = run_info.read_poisoned();
            ri.dispatcher
                .send_normal(crate::entrypoints::dispatcher::Event::JobQueued);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    info!(
        "activate_needed_units_via_job_graph: {} closure settled",
        target_id.name
    );
    Vec::new()
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
/// Kill the service's remaining processes, clear its runtime fds and mark it
/// StoppedUnexpected — shared cleanup for the deferred start failure paths
/// (start timeout, dbus-name timeout, exec confirmation failure, failing
/// forking parent).  Mirrors the cleanup in Service::deactivate_service
/// (PID, process_group, notification socket, stdout/stderr).
fn deferred_start_fail_cleanup(
    run_info: &ArcMutRuntimeInfo,
    id: &UnitId,
    name: &str,
    reason: String,
) {
    if let Ok(ri) = run_info.try_read()
        && let Some(unit) = ri.unit_table.get(id)
    {
        if let Specific::Service(svc) = &unit.specific {
            let mut state = svc.state.write_poisoned();
            state.srvc.kill_all_remaining_processes(&svc.conf, name);
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
        // Transition to Stopped/Failed. Scope the status guard so it drops
        // before the restart scheduler below reacquires it.
        {
            let mut status = unit.common.status.write_poisoned();
            if matches!(&*status, UnitStatus::Starting) {
                *status = UnitStatus::Stopped(
                    crate::units::status::StatusStopped::StoppedUnexpected,
                    vec![UnitOperationErrorReason::GenericStartError(reason)],
                );
            }
        }
        // A failed start still owes OnFailure= and Restart=: the exit tail
        // normally fires them for a service, but a deferred start failure is
        // finalized here on the dispatcher where the tail never runs (the
        // notify-before-READY case is suppressed there, and a start job never
        // reaches the runtime exit path). This finalizer is the single owner,
        // so it fires OnFailure= and, per Restart=, flips the unit from the
        // failed status above to Restarting and reactivates it after
        // RestartSec on spawned threads. OnSuccess= is never fired for a
        // failed start.
        crate::services::on_service_start_failed(id, &ri, run_info);
    }
}

/// Yield-to-writers read acquisition for dispatcher-side code: the
/// dispatcher must never block-wait on the RuntimeInfo RwLock. With a
/// writer queued (writer-preferring rwlock) a blocking read stalls the
/// dispatcher, while the writer itself waits on activation readers whose
/// readiness only the dispatcher can apply: a three-way deadlock. Spin on
/// try_read with a short sleep instead, like every other cooperative
/// reader.
pub(crate) fn dispatcher_read(
    run_info: &ArcMutRuntimeInfo,
) -> std::sync::RwLockReadGuard<'_, crate::runtime_info::RuntimeInfo> {
    let spin_start = std::time::Instant::now();
    let mut warned = false;
    loop {
        match run_info.try_read() {
            Ok(guard) => {
                if warned {
                    crate::entrypoints::kmsg(&format!(
                        "DISPATCHER-READ recovered after {:?}",
                        spin_start.elapsed()
                    ));
                }
                return guard;
            }
            Err(std::sync::TryLockError::Poisoned(poisoned)) => return poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                // A long spin here is the dispatcher-throughput stall that
                // flakes 16/59: make it self-diagnosing in kmsg instead of
                // silent (the margin analysis lives in the task ledger).
                if !warned && spin_start.elapsed() >= std::time::Duration::from_millis(500) {
                    warned = true;
                    crate::entrypoints::kmsg(
                        "DISPATCHER-READ stalled >500ms behind a queued writer",
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    }
}

/// Everything a deferred multi-command Type=oneshot start needs to advance:
/// carried by the dispatcher between steps (docs/EVENT-LOOP.md inc 2).
pub struct OneshotChainStart {
    pub id: UnitId,
    pub next_services_ids: Vec<UnitId>,
    pub filter_ids: Arc<Vec<UnitId>>,
    pub errors: Arc<Mutex<Vec<UnitOperationError>>>,
    pub source: ActivationSource,
}

pub enum OneshotStepOutcome {
    /// A preliminary command was forked; the chain now waits for this pid's
    /// ChildExit event, bounded by the per-command start timeout.
    Advanced {
        pid: nix::unistd::Pid,
        cmd: crate::units::Commandline,
        timeout: Option<std::time::Duration>,
    },
    /// The main process was forked (or the oneshot completed with no
    /// command left): the dispatcher should park a start wait for the
    /// unit's completion, carrying the chain's dispatch context.
    AwaitCompletion,
    /// A preliminary command failed on a unit with ExecStopPost=: the
    /// dispatcher should drive this poststop chain before the failure is
    /// finalized.
    PoststopChain(ServiceStartChain),
    /// The chain is done: a failure was recorded, or the unit vanished.
    Finished,
}

/// One decide-and-fork step of a multi-command Type=oneshot start whose
/// preliminary (non-last) `ExecStart=` commands were deferred by
/// `Service::start` (returning `StartResult::DeferredOneshotExec`). Runs on
/// the dispatcher: each call takes one brief read guard, relocates in the
/// (possibly reloaded) exec list via `current_exec_argv` exactly like
/// Service::start's inline loop, and either forks the next preliminary
/// command, forks the last command as the main process, or completes the
/// oneshot. It never waits: the fork's exit comes back as a ChildExit event
/// that [`oneshot_chain_child_exited`] consumes, so a concurrent
/// `systemctl daemon-reload` can commit between commands and the final
/// command reflects the reloaded config (07-pid1-exec-deserialization).
pub(crate) fn oneshot_chain_step(
    chain: &OneshotChainStart,
    run_info: &ArcMutRuntimeInfo,
) -> OneshotStepOutcome {
    let id = chain.id.clone();
    let name = id.name.clone();

    enum Step {
        // Carries the command so the exit handler can apply the same
        // exit-status rules as Service::run_cmd, including the `-` prefix.
        Wait(
            std::process::Child,
            Option<std::time::Duration>,
            crate::units::Commandline,
        ),
        Forked(Result<Option<crate::services::StartResult>, ServiceErrorReason>),
        Completed,
        SpawnErr(String),
        Gone,
    }

    let step = {
        let ri = dispatcher_read(run_info);
        match ri.unit_table.get(&id) {
            None => Step::Gone,
            Some(unit) => match &unit.specific {
                Specific::Service(svc) => {
                    let conf = &svc.conf;
                    let exec_list = conf.exec.clone();
                    let working_dir = conf.exec_config.working_directory.clone();
                    let mut st = svc.state.write_poisoned();
                    let timeout = st.srvc.get_start_timeout(conf);

                    // Relocate in the (possibly reloaded) exec list via the
                    // argv of the command we just ran, exactly like
                    // Service::start's inline loop.
                    let mut idx = 0usize;
                    let mut removed = false;
                    if let Some(last_argv) = st.srvc.current_exec_argv.take() {
                        match exec_list.iter().position(|c| c.to_string() == last_argv) {
                            Some(pos) => idx = pos + 1,
                            None => removed = true,
                        }
                    }

                    if removed {
                        // The currently-running command vanished from the
                        // reloaded config: finish the oneshot without running
                        // any further command (empty log), matching upstream
                        // ("the currently executed command vanished ... simply
                        // finish executing the unit"). Signal completion via
                        // main_exit_status=0; the parked start wait
                        // performs the Started transition + dispatch.
                        st.srvc.current_exec_argv = None;
                        st.srvc.main_exit_status = Some(0);
                        Step::Completed
                    } else if idx + 1 >= exec_list.len() {
                        // Only the last command remains: fork it as the main
                        // process.
                        Step::Forked(st.srvc.fork_main_and_maybe_defer(
                            conf,
                            id.clone(),
                            &name,
                            &ri,
                            chain.source,
                            &unit.common,
                        ))
                    } else {
                        let cmd = exec_list[idx].clone();
                        st.srvc.current_exec_argv = Some(cmd.to_string());
                        match st.srvc.spawn_helper_child(
                            &cmd,
                            id.clone(),
                            &name,
                            &ri,
                            working_dir.as_ref(),
                        ) {
                            Ok(child) => Step::Wait(child, timeout, cmd),
                            Err(e) => Step::SpawnErr(format!("{e}")),
                        }
                    }
                }
                _ => Step::Gone,
            },
        }
    };

    match step {
        Step::Wait(child, timeout, cmd) => OneshotStepOutcome::Advanced {
            pid: nix::unistd::Pid::from_raw(child.id() as i32),
            cmd,
            timeout,
        },
        Step::Forked(Ok(_)) | Step::Completed => {
            // Main process forked (its completion wait is itself deferred),
            // or the oneshot finished with no further command. The
            // dispatcher parks a start wait to own the completion.
            OneshotStepOutcome::AwaitCompletion
        }
        Step::Forked(Err(e)) => {
            chain.errors.lock_poisoned().push(UnitOperationError {
                unit_name: name.clone(),
                unit_id: id.clone(),
                reason: UnitOperationErrorReason::ServiceStartError(e),
            });
            OneshotStepOutcome::Finished
        }
        Step::SpawnErr(e) => {
            // Preserved shape from the thread-based driver: the error is
            // recorded but the unit stays in Starting (the deferred fail
            // cleanup is only wired to the wait path).
            error!("deferred oneshot {name}: failed to spawn preliminary command: {e}");
            chain.errors.lock_poisoned().push(UnitOperationError {
                unit_name: name.clone(),
                unit_id: id.clone(),
                reason: UnitOperationErrorReason::GenericStartError(e),
            });
            OneshotStepOutcome::Finished
        }
        Step::Gone => OneshotStepOutcome::Finished,
    }
}

/// Consume a chain child's exit on the dispatcher: apply the same
/// exit-status rules as Service::run_cmd (success or a leading `-` prefix
/// continues; anything else aborts the start), then run the next step.
pub(crate) fn oneshot_chain_child_exited(
    chain: &OneshotChainStart,
    cmd: &crate::units::Commandline,
    termination: crate::signal_handler::ChildTermination,
    run_info: &ArcMutRuntimeInfo,
) -> OneshotStepOutcome {
    // A preliminary ExecStart= that fails must abort the rest of the
    // oneshot, exactly as Service::run_cmd does for the inline path.
    if termination.success() || cmd.prefixes.contains(&crate::units::CommandlinePrefix::Minus) {
        return oneshot_chain_step(chain, run_info);
    }
    match oneshot_chain_fail(
        chain,
        crate::services::RunCmdError::BadExitCode(cmd.to_string(), termination),
        run_info,
    ) {
        Some(post) => OneshotStepOutcome::PoststopChain(post),
        None => OneshotStepOutcome::Finished,
    }
}

/// The chain's per-command start timeout fired: SIGKILL the preliminary
/// command (mirroring the thread driver's child.kill()) and fail the start.
/// The kill's reap arrives later as an unmatched ChildExit and is dropped.
pub(crate) fn oneshot_chain_timed_out(
    chain: &OneshotChainStart,
    cmd: &crate::units::Commandline,
    pid: nix::unistd::Pid,
    timeout: Option<std::time::Duration>,
    run_info: &ArcMutRuntimeInfo,
) -> OneshotStepOutcome {
    unsafe {
        libc::kill(pid.as_raw(), libc::SIGKILL);
    }
    match oneshot_chain_fail(
        chain,
        crate::services::RunCmdError::Timeout(
            cmd.to_string(),
            format!("Timeout ({timeout:?}) reached"),
        ),
        run_info,
    ) {
        Some(post) => OneshotStepOutcome::PoststopChain(post),
        None => OneshotStepOutcome::Finished,
    }
}

/// A deferred ExecCondition=/ExecStartPre= helper phase advanced on the
/// dispatcher (docs/EVENT-LOOP.md inc 2), carrying the same dispatch
/// context as the oneshot chain. Created when `Service::start` returns
/// `StartResult::DeferredPrestart` for a pool-path activation.
pub struct ServiceStartChain {
    pub id: UnitId,
    pub next_services_ids: Vec<UnitId>,
    pub filter_ids: Arc<Vec<UnitId>>,
    pub errors: Arc<Mutex<Vec<UnitOperationError>>>,
    pub source: ActivationSource,
    pub phase: StartChainPhase,
}

pub enum StartChainPhase {
    /// Running ExecCondition= command `idx`.
    Condition(usize),
    /// Running ExecStartPre= command `idx`.
    Prestart(usize),
    /// A condition or prestart command errored; ExecStopPost= runs before
    /// the start is failed with `error` (mirrors the inline path's
    /// PrestartFailed-plus-poststop handling).
    PoststopAfterError { idx: usize, error: String },
}

pub enum StartChainOutcome {
    /// A helper was forked; the chain waits for this pid's exit, holding
    /// the Child so its piped output can be drained then.
    Advanced {
        pid: nix::unistd::Pid,
        child: std::process::Child,
        cmd: crate::units::Commandline,
        timeout: Option<std::time::Duration>,
    },
    /// The helper phase completed and the main phase produced this start
    /// result for the dispatcher to route.
    MainDispatched(crate::services::StartResult),
    /// The chain ended without a main fork (skip, failure, or the unit
    /// vanished or left Starting).
    Finished,
}

/// Advance the chain: run through empty or exhausted phases, fork the next
/// helper command, or run the main phase. Never waits.
pub(crate) fn service_start_chain_step(
    chain: &mut ServiceStartChain,
    run_info: &ArcMutRuntimeInfo,
) -> StartChainOutcome {
    enum Action {
        Fork(crate::units::Commandline),
        NextPhase(StartChainPhase),
        RunMain,
        FinalizeError(String),
    }
    loop {
        let action = {
            let ri = dispatcher_read(run_info);
            let Some(unit) = ri.unit_table.get(&chain.id) else {
                return StartChainOutcome::Finished;
            };
            if !matches!(&*unit.common.status.read_poisoned(), UnitStatus::Starting) {
                return StartChainOutcome::Finished;
            }
            let Specific::Service(svc) = &unit.specific else {
                return StartChainOutcome::Finished;
            };
            match &chain.phase {
                StartChainPhase::Condition(idx) => {
                    if *idx >= svc.conf.exec_condition.len() {
                        Action::NextPhase(StartChainPhase::Prestart(0))
                    } else {
                        Action::Fork(svc.conf.exec_condition[*idx].clone())
                    }
                }
                StartChainPhase::Prestart(idx) => {
                    if *idx >= svc.conf.startpre.len() {
                        Action::RunMain
                    } else {
                        Action::Fork(svc.conf.startpre[*idx].clone())
                    }
                }
                StartChainPhase::PoststopAfterError { idx, error } => {
                    if *idx >= svc.conf.stoppost.len() {
                        Action::FinalizeError(error.clone())
                    } else {
                        Action::Fork(svc.conf.stoppost[*idx].clone())
                    }
                }
            }
        };
        match action {
            Action::NextPhase(phase) => {
                chain.phase = phase;
            }
            Action::FinalizeError(error) => {
                start_chain_finalize_error(chain, error, run_info);
                return StartChainOutcome::Finished;
            }
            Action::Fork(cmd) => {
                let name = chain.id.name.clone();
                let forked = {
                    let ri = dispatcher_read(run_info);
                    let Some(unit) = ri.unit_table.get(&chain.id) else {
                        return StartChainOutcome::Finished;
                    };
                    let Specific::Service(svc) = &unit.specific else {
                        return StartChainOutcome::Finished;
                    };
                    let working_dir = svc.conf.exec_config.working_directory.clone();
                    let mut st = svc.state.write_poisoned();
                    let timeout = st.srvc.get_start_timeout(&svc.conf);
                    // The helper must see the service's filesystem view
                    // (BindPaths=, PrivateTmp=), like the inline phases.
                    st.srvc.helper_mount_ns =
                        crate::services::helper_mount_ns_from_conf(&svc.conf);
                    let res = st.srvc.spawn_helper_child(
                        &cmd,
                        chain.id.clone(),
                        &name,
                        &ri,
                        working_dir.as_ref(),
                    );
                    st.srvc.helper_mount_ns = None;
                    res.map(|child| (child, timeout))
                };
                match forked {
                    Ok((child, timeout)) => {
                        return StartChainOutcome::Advanced {
                            pid: nix::unistd::Pid::from_raw(child.id() as i32),
                            child,
                            cmd,
                            timeout,
                        };
                    }
                    Err(e) => {
                        // Mirror run_cmd's spawn-error handling: a leading
                        // '-' swallows the failure and the chain continues;
                        // anything else is a start error.
                        if cmd
                            .prefixes
                            .contains(&crate::units::CommandlinePrefix::Minus)
                        {
                            advance_chain_index(chain);
                        } else {
                            chain_error_to_poststop(chain, format!("Failed to spawn {cmd}: {e}"));
                        }
                    }
                }
            }
            Action::RunMain => {
                let name = chain.id.name.clone();
                let main_result = {
                    let ri = dispatcher_read(run_info);
                    let Some(unit) = ri.unit_table.get(&chain.id) else {
                        return StartChainOutcome::Finished;
                    };
                    let Specific::Service(svc) = &unit.specific else {
                        return StartChainOutcome::Finished;
                    };
                    let conf = svc.conf.clone();
                    let mut st = svc.state.write_poisoned();
                    st.srvc.start_main_phase(
                        &conf,
                        chain.id.clone(),
                        &name,
                        &ri,
                        ActivationSource::DeferNotifyWait,
                        &unit.common,
                    )
                };
                match main_result {
                    Ok(result) => return StartChainOutcome::MainDispatched(result),
                    Err(e) => {
                        chain.errors.lock_poisoned().push(UnitOperationError {
                            unit_name: name,
                            unit_id: chain.id.clone(),
                            reason: UnitOperationErrorReason::ServiceStartError(e),
                        });
                        // Route through the chain's own poststop phase so
                        // ExecStopPost= runs before the failure finalizes.
                        chain_error_to_poststop(
                            chain,
                            "main ExecStart dispatch failed".to_owned(),
                        );
                    }
                }
            }
        }
    }
}

/// Consume a chain helper's exit on the dispatcher, mirroring run_cmd's
/// wait semantics per phase, then advance.
pub(crate) fn service_start_chain_child_exited(
    chain: &mut ServiceStartChain,
    cmd: &crate::units::Commandline,
    mut child: std::process::Child,
    termination: crate::signal_handler::ChildTermination,
    run_info: &ArcMutRuntimeInfo,
) -> StartChainOutcome {
    // Drain the helper's piped output into the service buffers, as the
    // inline run_cmd does after its wait.
    {
        let ri = dispatcher_read(run_info);
        if let Some(unit) = ri.unit_table.get(&chain.id)
            && let Specific::Service(svc) = &unit.specific
        {
            let status = unit.common.status.read_poisoned().clone();
            svc.state.write_poisoned().srvc.drain_helper_output_nonblocking(
                &mut child,
                &chain.id.name,
                &status,
            );
        }
    }
    drop(child);

    let ok = termination.success()
        || cmd.prefixes.contains(&crate::units::CommandlinePrefix::Minus);
    match &chain.phase {
        StartChainPhase::Condition(_) => {
            if ok {
                advance_chain_index(chain);
            } else if let crate::signal_handler::ChildTermination::Exit(code) = termination
                && (1..=254).contains(&code)
            {
                // ExecCondition= exit 1-254: skip the service, not a failure.
                apply_condition_skip(&chain.id, run_info);
                return StartChainOutcome::Finished;
            } else {
                chain_error_to_poststop(
                    chain,
                    format!("ExecCondition {cmd} failed: {termination:?}"),
                );
            }
        }
        StartChainPhase::Prestart(_) => {
            if ok {
                advance_chain_index(chain);
            } else {
                chain_error_to_poststop(
                    chain,
                    format!("ExecStartPre {cmd} failed: {termination:?}"),
                );
            }
        }
        StartChainPhase::PoststopAfterError { error, .. } => {
            if ok {
                advance_chain_index(chain);
            } else {
                // Mirror run_all_cmds: the first poststop failure aborts the
                // rest; both errors are reported.
                let combined = format!("{error}; ExecStopPost {cmd} failed: {termination:?}");
                start_chain_finalize_error(chain, combined, run_info);
                return StartChainOutcome::Finished;
            }
        }
    }
    service_start_chain_step(chain, run_info)
}

/// The chain helper's per-command timeout fired: SIGKILL it and route the
/// timeout through the same per-phase error handling as a bad exit.
pub(crate) fn service_start_chain_timed_out(
    chain: &mut ServiceStartChain,
    cmd: &crate::units::Commandline,
    pid: nix::unistd::Pid,
    timeout: Option<std::time::Duration>,
    run_info: &ArcMutRuntimeInfo,
) -> StartChainOutcome {
    unsafe {
        libc::kill(pid.as_raw(), libc::SIGKILL);
    }
    let error = format!("{cmd} timed out ({timeout:?})");
    match &chain.phase {
        StartChainPhase::Condition(_) | StartChainPhase::Prestart(_) => {
            chain_error_to_poststop(chain, error);
            service_start_chain_step(chain, run_info)
        }
        StartChainPhase::PoststopAfterError {
            error: original, ..
        } => {
            let combined = format!("{original}; {error}");
            start_chain_finalize_error(chain, combined, run_info);
            StartChainOutcome::Finished
        }
    }
}

fn advance_chain_index(chain: &mut ServiceStartChain) {
    chain.phase = match &chain.phase {
        StartChainPhase::Condition(idx) => StartChainPhase::Condition(idx + 1),
        StartChainPhase::Prestart(idx) => StartChainPhase::Prestart(idx + 1),
        StartChainPhase::PoststopAfterError { idx, error } => StartChainPhase::PoststopAfterError {
            idx: idx + 1,
            error: error.clone(),
        },
    };
}

fn chain_error_to_poststop(chain: &mut ServiceStartChain, error: String) {
    chain.phase = StartChainPhase::PoststopAfterError { idx: 0, error };
}

fn start_chain_finalize_error(
    chain: &ServiceStartChain,
    error: String,
    run_info: &ArcMutRuntimeInfo,
) {
    chain.errors.lock_poisoned().push(UnitOperationError {
        unit_name: chain.id.name.clone(),
        unit_id: chain.id.clone(),
        reason: UnitOperationErrorReason::GenericStartError(error.clone()),
    });
    finalize_deferred_start_failure(run_info, &chain.id, error);
    trigger_on_failure_units(&chain.id, run_info);
}

/// ExecCondition= said no: mark the unit skipped and re-arm its sockets.
/// Mirrors Unit::activate's ConditionSkipped arm (units/unit.rs), which
/// stays inline for the non-chain sources.
fn apply_condition_skip(id: &UnitId, run_info: &ArcMutRuntimeInfo) {
    let ri = dispatcher_read(run_info);
    let Some(unit) = ri.unit_table.get(id) else {
        return;
    };
    {
        let mut status = unit.common.status.write_poisoned();
        if matches!(&*status, UnitStatus::Starting) {
            *status = UnitStatus::Stopped(crate::units::StatusStopped::ConditionSkipped, vec![]);
        }
    }
    if let Specific::Service(svc) = &unit.specific {
        for socket_id in &svc.conf.sockets {
            if let Some(sock_unit) = ri.unit_table.get(socket_id)
                && let Specific::Socket(sock) = &sock_unit.specific
            {
                if sock.state.read_poisoned().result
                    == crate::units::SocketResult::TriggerLimitHit
                {
                    continue;
                }
                if sock.conf.flush_pending {
                    crate::units::flush_socket_fds(socket_id, &ri);
                }
                sock.state.write_poisoned().sock.activated = false;
            }
        }
        if !svc.conf.sockets.is_empty() {
            ri.notify_eventfds();
        }
    }
}

/// The chain's main phase reported WaitingForSocket: apply the status and
/// socket re-arm that Unit::activate's arm applies for inline starts.
pub(crate) fn apply_waiting_for_socket(id: &UnitId, run_info: &ArcMutRuntimeInfo) {
    let ri = dispatcher_read(run_info);
    let Some(unit) = ri.unit_table.get(id) else {
        return;
    };
    {
        let mut status = unit.common.status.write_poisoned();
        if matches!(&*status, UnitStatus::Starting) {
            *status =
                UnitStatus::Started(crate::units::StatusStarted::WaitingForSocket);
        }
    }
    if let Specific::Service(svc) = &unit.specific {
        for socket_id in &svc.conf.sockets {
            if let Some(sock_unit) = ri.unit_table.get(socket_id)
                && let Specific::Socket(sock) = &sock_unit.specific
            {
                if sock.state.read_poisoned().result
                    == crate::units::SocketResult::TriggerLimitHit
                {
                    continue;
                }
                if sock.conf.flush_pending {
                    crate::units::flush_socket_fds(socket_id, &ri);
                }
                sock.state.write_poisoned().sock.activated = false;
            }
        }
        ri.notify_eventfds();
    }
}

/// Finalize a deferred start failure without running ExecStopPost=: kill
/// remaining processes, clear runtime fds, StoppedUnexpected. The poststop
/// chain's own finalizer, and the fallback when no ExecStopPost= exists.
pub(crate) fn finalize_deferred_start_failure(
    run_info: &ArcMutRuntimeInfo,
    id: &UnitId,
    reason: String,
) {
    warn!("deferred start of {} failed: {reason}", id.name);
    deferred_start_fail_cleanup(run_info, id, &id.name.clone(), reason);
}

/// Fail a deferred start the way upstream does: the service's processes are
/// killed, then ExecStopPost= runs, then the unit is marked failed. The
/// poststop commands must not be waited for here (the historical inline
/// attempt deadlocked PID 1, see the 23-unit-file-execstoppost wrapper), so
/// when the unit has ExecStopPost= this returns a chain parked in its
/// poststop phase for the dispatcher to drive; the chain's finalizer sets
/// the failed status afterwards. With no ExecStopPost= the failure is
/// finalized immediately and None is returned.
#[must_use]
pub(crate) fn fail_deferred_start(
    run_info: &ArcMutRuntimeInfo,
    id: &UnitId,
    reason: String,
) -> Option<ServiceStartChain> {
    // Without a live dispatcher (unit tests, very early boot) nothing can
    // drive the chain, so the failure must finalize directly or the unit
    // leaks in Starting.
    let has_stoppost = crate::entrypoints::dispatcher::global().is_some() && {
        let ri = dispatcher_read(run_info);
        match ri.unit_table.get(id) {
            Some(unit) => match &unit.specific {
                // Kill the started processes first, like the plain cleanup,
                // but leave the status in Starting so the poststop chain's
                // steps still own the unit.
                Specific::Service(svc) if !svc.conf.stoppost.is_empty() => {
                    let mut state = svc.state.write_poisoned();
                    state
                        .srvc
                        .kill_all_remaining_processes(&svc.conf, &id.name);
                    state.srvc.pid = None;
                    state.srvc.process_group = None;
                    true
                }
                _ => false,
            },
            None => false,
        }
    };
    if has_stoppost {
        warn!(
            "deferred start of {} failed: {reason}; running ExecStopPost before failing",
            id.name
        );
        Some(ServiceStartChain {
            id: id.clone(),
            next_services_ids: Vec::new(),
            filter_ids: Arc::new(Vec::new()),
            errors: Arc::new(Mutex::new(Vec::new())),
            source: ActivationSource::Regular,
            phase: StartChainPhase::PoststopAfterError {
                idx: 0,
                error: reason,
            },
        })
    } else {
        finalize_deferred_start_failure(run_info, id, reason);
        None
    }
}

#[must_use]
fn oneshot_chain_fail(
    chain: &OneshotChainStart,
    e: crate::services::RunCmdError,
    run_info: &ArcMutRuntimeInfo,
) -> Option<ServiceStartChain> {
    let id = &chain.id;
    let name = &id.name;
    // Clear the relocation breadcrumb so nothing can resume the sequence
    // from the failed command.
    {
        let ri = dispatcher_read(run_info);
        if let Some(unit) = ri.unit_table.get(id)
            && let Specific::Service(svc) = &unit.specific
        {
            svc.state.write_poisoned().srvc.current_exec_argv = None;
        }
    }
    let reason = format!("{e}");
    chain.errors.lock_poisoned().push(UnitOperationError {
        unit_name: name.clone(),
        unit_id: id.clone(),
        reason: UnitOperationErrorReason::ServiceStartError(ServiceErrorReason::StartFailed(e)),
    });
    // Reporting the error is not enough: the unit would stay in
    // UnitStatus::Starting and be started again from the top, re-running
    // the preliminary commands forever. Fail the start; with ExecStopPost=
    // configured that hands back a poststop chain to drive first.
    fail_deferred_start(run_info, id, reason)
}

/// What a parked deferred start needs the dispatcher to know at
/// registration (docs/EVENT-LOOP.md inc 2).
pub struct StartWaitParams {
    pub id: UnitId,
    /// The before-chain to dispatch on completion; None means "collect the
    /// unit's Before= dependents at registration" (the trigger-path
    /// wrappers' behavior).
    pub next_services_ids: Option<Vec<UnitId>>,
    pub filter_ids: Arc<Vec<UnitId>>,
    pub errors: Arc<Mutex<Vec<UnitOperationError>>>,
    pub source: ActivationSource,
    /// Only park if the unit is (still) in Starting.
    pub check_starting: bool,
}

/// Registration-time facts for a parked start, read under one brief guard.
pub struct StartWaitSetup {
    pub svc_type: crate::units::ServiceType,
    pub timeout: Option<std::time::Duration>,
    pub stop_timeout: Option<std::time::Duration>,
    pub dbus_name: Option<String>,
    pub next_services_ids: Vec<UnitId>,
}

/// One brief guard: bail out if the unit is gone or (when requested) no
/// longer Starting, wake the notification reader so it collects the new
/// service's socket, and copy out the type-relevant config.
pub(crate) fn start_wait_setup(params: &StartWaitParams, run_info: &ArcMutRuntimeInfo) -> Option<StartWaitSetup> {
    let ri = dispatcher_read(run_info);
    let unit = ri.unit_table.get(&params.id)?;
    if params.check_starting
        && !matches!(&*unit.common.status.read_poisoned(), UnitStatus::Starting)
    {
        return None;
    }
    ri.notify_eventfds();
    let next_services_ids = params
        .next_services_ids
        .clone()
        .unwrap_or_else(|| unit.common.dependencies.before.clone());
    if let Specific::Service(svc) = &unit.specific {
        let state = svc.state.read_poisoned();
        Some(StartWaitSetup {
            svc_type: svc.conf.srcv_type,
            timeout: state.srvc.get_start_timeout(&svc.conf),
            stop_timeout: state.srvc.get_stop_timeout(&svc.conf),
            dbus_name: svc.conf.dbus_name.clone(),
            next_services_ids,
        })
    } else {
        None
    }
}

/// The dispatcher's verdict when re-evaluating a parked start.
pub(crate) enum StartWaitVerdict {
    /// No readiness signal yet; keep waiting.
    Pending,
    /// Readiness arrived: run [`finish_deferred_start`] on a finisher thread.
    Ready,
    /// The unit vanished or left Starting (someone else owns the outcome,
    /// e.g. the exit handler after a failing oneshot); end the wait silently.
    Abandoned,
    /// The start failed in a way this wait owns: run the deferred cleanup
    /// with this reason and end the wait.
    Fail(String),
}

/// Re-evaluate a parked deferred start against current unit state. Runs on
/// the dispatcher under one brief read guard; mirrors the per-type readiness
/// logic of the retired polling completion handler, including the forking
/// daemon-PID pickup side effect.
pub(crate) fn evaluate_start_wait(
    id: &UnitId,
    svc_type: crate::units::ServiceType,
    exec_confirm_elapsed: bool,
    dbus_done: bool,
    run_info: &ArcMutRuntimeInfo,
) -> StartWaitVerdict {
    use crate::units::ServiceType;
    let ri = dispatcher_read(run_info);
    let Some(unit) = ri.unit_table.get(id) else {
        return StartWaitVerdict::Abandoned;
    };
    if !matches!(&*unit.common.status.read_poisoned(), UnitStatus::Starting) {
        return StartWaitVerdict::Abandoned;
    }
    let Specific::Service(svc) = &unit.specific else {
        return StartWaitVerdict::Abandoned;
    };

    // For oneshot/forking services, completion is the ExecStart process
    // exiting. main_exit_status is set by the exit head when the process is
    // reaped (and cleared on each new spawn), so it is a race-free "has it
    // finished" signal.
    let exit_success = if matches!(svc_type, ServiceType::OneShot | ServiceType::Forking) {
        let state = svc.state.read_poisoned();
        state.srvc.main_exit_status.map(|code| {
            code == 0
                || svc.conf.success_exit_status.exit_codes.contains(&code)
                || svc
                    .conf
                    .exec
                    .last()
                    .map(|e| e.prefixes.contains(&crate::units::CommandlinePrefix::Minus))
                    .unwrap_or(false)
        })
    } else {
        None
    };

    match svc_type {
        ServiceType::OneShot => match exit_success {
            Some(true) => StartWaitVerdict::Ready,
            // The exit handler owns the failed/OnFailure/Restart transition.
            Some(false) => StartWaitVerdict::Abandoned,
            None => StartWaitVerdict::Pending,
        },
        ServiceType::Forking => match exit_success {
            Some(true) => {
                // The parent exiting cleanly means the daemon is up. Pick up
                // the daemon PID (PIDFile, else MAINPID from sd_notify) and
                // track it, mirroring the inline fork_parent Forking arm.
                let mut state = svc.state.write_poisoned();
                // Consume the signal so the daemon's own later exit is not
                // mistaken for another parent exit.
                state.srvc.main_exit_status = None;
                let daemon_pid = if let Some(ref pid_file_path) = svc.conf.pid_file {
                    let p = crate::services::fork_parent::read_pid_file(pid_file_path);
                    if p.is_none() {
                        warn!(
                            "deferred start: could not read PIDFile {:?} for {}",
                            pid_file_path, id.name
                        );
                    }
                    p
                } else {
                    state.srvc.main_pid
                };
                if let Some(daemon_pid) = daemon_pid {
                    state.srvc.pid = Some(daemon_pid);
                    let now = crate::units::UnitTimestamps::now_usec();
                    state.srvc.exec_main_start_timestamp = Some(now);
                    state.srvc.exec_main_handoff_timestamp = Some(now);
                    ri.pid_table.lock_poisoned().insert(
                        daemon_pid,
                        crate::runtime_info::PidEntry::Service(id.clone(), svc.conf.srcv_type),
                    );
                } else {
                    state.srvc.pid = None;
                }
                StartWaitVerdict::Ready
            }
            Some(false) => {
                StartWaitVerdict::Fail("Forking ExecStart parent exited with failure".to_owned())
            }
            None => StartWaitVerdict::Pending,
        },
        ServiceType::Exec => {
            let exec_failed = svc.state.read_poisoned().srvc.main_exit_status == Some(203);
            if exec_failed {
                StartWaitVerdict::Fail("exec() of the service binary failed".to_owned())
            } else if exec_confirm_elapsed {
                StartWaitVerdict::Ready
            } else {
                StartWaitVerdict::Pending
            }
        }
        ServiceType::Dbus => {
            if dbus_done {
                StartWaitVerdict::Ready
            } else if svc.state.read_poisoned().srvc.main_exit_status.is_some() {
                // The main process exited before the bus name appeared: the
                // exit handler owns the outcome (clean exit deactivates,
                // failure fails), so the wait must not keep the deadline
                // armed for a unit that is about to leave Starting.
                StartWaitVerdict::Abandoned
            } else {
                StartWaitVerdict::Pending
            }
        }
        // RELOADING=1 implies a previous READY=1, so a reloading service
        // counts as started. A notify main exiting before READY=1 is
        // upstream's 'protocol' failure; the exit head suppressed death
        // processing so this wait owns it, ExecStopPost= included.
        ServiceType::Notify | ServiceType::NotifyReload => {
            let state = svc.state.read_poisoned();
            if state.srvc.signaled_ready || state.srvc.reloading {
                StartWaitVerdict::Ready
            } else if state.srvc.main_exit_status.is_some() {
                StartWaitVerdict::Fail(
                    "Service exited before sending READY=1".to_owned(),
                )
            } else {
                StartWaitVerdict::Pending
            }
        }
        // Everything else that lands here waits for READY=1.
        _ => {
            let state = svc.state.read_poisoned();
            if state.srvc.signaled_ready || state.srvc.reloading {
                StartWaitVerdict::Ready
            } else {
                StartWaitVerdict::Pending
            }
        }
    }
}

/// Compute the effective start deadline: the base TimeoutStartSec deadline,
/// pushed out by an EXTEND_TIMEOUT_USEC extension when one is active
/// (extension never shortens; timing out requires BOTH elapsed).
pub(crate) fn effective_start_deadline(
    id: &UnitId,
    base_deadline: Option<std::time::Instant>,
    run_info: &ArcMutRuntimeInfo,
) -> Option<std::time::Instant> {
    let base = base_deadline?;
    let ri = dispatcher_read(run_info);
    if let Some(unit) = ri.unit_table.get(id)
        && let Specific::Service(svc) = &unit.specific
    {
        let state = svc.state.read_poisoned();
        if let (Some(usec), Some(ts)) = (
            state.srvc.extend_timeout_usec,
            state.srvc.extend_timeout_timestamp,
        ) {
            let ext = ts + std::time::Duration::from_micros(usec);
            return Some(base.max(ext));
        }
    }
    Some(base)
}

/// Send the start-timeout SIGTERM (process group first, else the main pid),
/// mirroring the retired poller's graceful first escalation step.
pub(crate) fn start_wait_send_sigterm(id: &UnitId, run_info: &ArcMutRuntimeInfo) {
    let ri = dispatcher_read(run_info);
    if let Some(unit) = ri.unit_table.get(id)
        && let Specific::Service(svc) = &unit.specific
    {
        let state = svc.state.read_poisoned();
        if let Some(pg) = state.srvc.process_group {
            let _ = nix::sys::signal::kill(pg, nix::sys::signal::Signal::SIGTERM);
        } else if let Some(pid) = state.srvc.pid {
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
        }
    }
}

/// Complete a deferred start whose readiness signal arrived: run
/// ExecStartPost=, flip the unit to Started with its bookkeeping, and
/// dispatch the before-chain. Shared by the polling completion handler and
/// the dispatcher's finisher thread (docs/EVENT-LOOP.md inc 2). May block
/// on the poststart helper wait, so it must never run on the dispatcher
/// itself.
pub(crate) fn finish_deferred_start(
    id: &UnitId,
    name: &str,
    next_services_ids: Vec<UnitId>,
    filter_ids: Arc<Vec<UnitId>>,
    run_info: ArcMutRuntimeInfo,
    errors: Arc<Mutex<Vec<UnitOperationError>>>,
    source: ActivationSource,
) {
    {
        let ri = run_info.read_poisoned();
        let Some(unit) = ri.unit_table.get(id) else {
            return;
        };
        // Something else may have transitioned the unit in the window since
        // readiness was observed (e.g. the exit handler); completing twice
        // must not happen.
        if !matches!(&*unit.common.status.read_poisoned(), UnitStatus::Starting) {
            return;
        }

        // Run ExecStartPost= before flipping Started, mirroring the
        // inline path (run_poststart after wait_for_service).  On failure
        // run ExecStopPost and fail the start.  NOTE: this holds the
        // RuntimeInfo read guard + state write lock across the bounded
        // poststart helper wait; taking helper waits fully off the locks
        // is docs/ARCHITECTURE.md invariant I1.
        if let Specific::Service(svc) = &unit.specific
            && !svc.conf.startpost.is_empty()
        {
            let poststart_res = {
                let mut state = svc.state.write_poisoned();
                state.srvc.run_poststart(&svc.conf, id.clone(), name, &ri)
            };
            if let Err(e) = poststart_res {
                warn!("deferred start: ExecStartPost failed for {name}: {e}");
                {
                    let mut state = svc.state.write_poisoned();
                    let _ = state.srvc.run_poststop(&svc.conf, id.clone(), name, &ri);
                }
                let mut status = unit.common.status.write_poisoned();
                if matches!(&*status, UnitStatus::Starting) {
                    *status = UnitStatus::Stopped(
                        crate::units::status::StatusStopped::StoppedUnexpected,
                        vec![UnitOperationErrorReason::GenericStartError(format!(
                            "ExecStartPost failed: {e}"
                        ))],
                    );
                }
                return;
            }
        }

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
        crate::control::varlink::journal_log_unit_lifecycle(&msg, name, log_level_max.as_deref());
    }

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
}

/// Park the deferred completion of a service whose start wait was deferred
/// (`start_service` returned `StartResult::DeferredNotifyWait`, so the unit
/// is left in `Starting`).  Used by activation paths that call
/// `activate_unit` directly and therefore do not go through
/// `activate_units_recursive`'s deferral — notably socket activation.
/// Sends an event rather than taking any lock: several callers hold a
/// RuntimeInfo read guard, and a second same-thread read acquisition
/// deadlocks against a queued writer on std's futex rwlock, which is why
/// the dispatcher handle comes from the process-global accessor.
pub(crate) fn spawn_deferred_service_wait(id: UnitId, _run_info: ArcMutRuntimeInfo) {
    send_start_wait_event(id, false);
}

/// Convenience wrapper for trigger paths (timer, path, udev, exit-handler)
/// that call `activate_unit` directly: if the unit was left `Starting`
/// (i.e. its start wait was deferred), park its completion.  Without this
/// the unit has no completion owner — nothing enforces its start timeout
/// and nothing transitions it to Started, so it can sit in `activating`
/// forever.
pub(crate) fn spawn_deferred_service_wait_if_starting(id: &UnitId, _run_info: &ArcMutRuntimeInfo) {
    send_start_wait_event(id.clone(), true);
}

fn send_start_wait_event(id: UnitId, check_starting: bool) {
    let name = id.name.clone();
    match crate::entrypoints::dispatcher::global() {
        Some(handle) => handle.send_normal(
            crate::entrypoints::dispatcher::Event::StartServiceWait(StartWaitParams {
                id,
                next_services_ids: None,
                filter_ids: Arc::new(Vec::new()),
                errors: Arc::new(Mutex::new(Vec::new())),
                source: ActivationSource::Regular,
                check_starting,
            }),
        ),
        None => error!("no dispatcher to park the deferred start of {name}"),
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

#[cfg(test)]
mod action_exit_status_tests {
    use super::resolve_action_exit_status;
    use crate::signal_handler::ChildTermination;
    use nix::sys::signal::Signal;

    #[test]
    fn configured_status_wins_over_the_exit_code() {
        // TEST-18-FAILUREACTION relies on this: it runs `false` (exit 1) with
        // -p FailureActionExitStatus=123 and expects 123, not 1.
        let code = ChildTermination::Exit(1);
        assert_eq!(resolve_action_exit_status(Some(123), &code), Some(123));
    }

    #[test]
    fn without_a_configured_status_the_exit_code_is_propagated() {
        let code = ChildTermination::Exit(7);
        assert_eq!(resolve_action_exit_status(None, &code), Some(7));
    }

    #[test]
    fn a_signalled_process_propagates_255() {
        // Upstream's -EBADE case: exited, but not cleanly (unit.c:6293).
        let code = ChildTermination::Signal(Signal::SIGKILL);
        assert_eq!(resolve_action_exit_status(None, &code), Some(255));
    }

    #[test]
    fn an_out_of_range_exit_code_propagates_nothing() {
        // Exit statuses are a byte; anything else has nothing to propagate.
        let code = ChildTermination::Exit(300);
        assert_eq!(resolve_action_exit_status(None, &code), None);
        // ...but an explicit setting still wins.
        assert_eq!(resolve_action_exit_status(Some(5), &code), Some(5));
    }
}
