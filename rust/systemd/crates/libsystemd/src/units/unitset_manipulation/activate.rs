//! Activate units (recursively and parallel along the dependency tree)

use crate::lock_ext::{MutexExt, RwLockExt};
use crate::runtime_info::{ArcMutRuntimeInfo, RuntimeInfo, UnitTable};
use crate::services::ServiceErrorReason;
use crate::units::{Specific, StatusStarted, StatusStopped, UnitAction, UnitId, UnitStatus};

use log::{debug, error, info, trace, warn};
use std::sync::{Arc, Mutex};
use threadpool::ThreadPool;

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
            let pulled = unit.common.dependencies.wants.contains(elem);
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
                // Soft pull dependency (Wants=): must have left NeverStarted
                *status_locked != UnitStatus::NeverStarted
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
}

impl ActivationSource {
    #[must_use]
    pub const fn is_socket_activation(&self) -> bool {
        matches!(self, Self::SocketActivation)
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
    {
        let status = unit.common.status.read_poisoned();
        match &*status {
            UnitStatus::Started(_) => {
                // Already running — nothing to do, don't re-dispatch before-chain
                return Ok(StartResult::Started(vec![]));
            }
            UnitStatus::Stopped(StatusStopped::StoppedFinal, _) => {
                // Already finished (e.g. condition-skipped oneshot) — don't re-check
                return Ok(StartResult::Started(vec![]));
            }
            UnitStatus::Stopped(StatusStopped::StoppedUnexpected, _) => {
                // Already failed — don't retry during initial activation
                return Ok(StartResult::Started(vec![]));
            }
            _ => {
                // NeverStarted, Starting, Stopping, Restarting — proceed
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
    let success_action = unit.common.unit.success_action.clone();
    let failure_action = unit.common.unit.failure_action.clone();

    // Remember whether the unit was already started before we call activate().
    // If it was, we must NOT re-dispatch the before-chain — doing so would
    // cause an infinite livelock where already-started units keep returning
    // their full before-chain, which re-discovers them as "startable", etc.
    let was_already_started = {
        let status = unit.common.status.read_poisoned();
        status.is_started()
    };

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

            // Activate the slice hierarchy for this unit so that
            // `systemctl is-active <slice>` returns "active".
            activate_slice_hierarchy(unit, run_info);

            if success_action != UnitAction::None {
                info!(
                    "Unit {} succeeded, triggering SuccessAction={:?}",
                    id_to_start.name, success_action
                );
                execute_unit_action(&success_action, &id_to_start.name);
            }
            Ok(StartResult::Started(next_services_ids))
        }
        Err(e) => {
            if failure_action != UnitAction::None {
                // Don't trigger FailureAction for dependency errors — those
                // just mean the unit is waiting and will be retried.
                if !matches!(e.reason, UnitOperationErrorReason::DependencyError(_)) {
                    info!(
                        "Unit {} failed, triggering FailureAction={:?}",
                        id_to_start.name, failure_action
                    );
                    execute_unit_action(&failure_action, &id_to_start.name);
                }
            }

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
    );

    tpool.join();
    info!("activate_needed_units: activation complete, all jobs dispatched");

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
            match activate_unit(
                id,
                &run_info_copy.read_poisoned(),
                ActivationSource::Regular,
            ) {
                Ok(StartResult::Started(next_services_ids)) => {
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
                        );
                    };
                    tpool_copy.execute(next_services_job);
                }
                Err(e) => {
                    if let UnitOperationErrorReason::DependencyError(_) = e.reason {
                        // Thats ok. The unit is waiting for more dependencies and will be
                        // activated again when another dependency has finished starting

                        // This should not happen though, since we filter the units beforehand
                        // to only get the startables
                    } else {
                        error!("Error while activating unit {e}");
                        errors_copy.lock_poisoned().push(e);
                    }
                }
            }
        });
    }
}
