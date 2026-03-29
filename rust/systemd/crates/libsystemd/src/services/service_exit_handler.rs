use log::{error, info, trace, warn};

use crate::lock_ext::RwLockExt;
use crate::runtime_info::{ArcMutRuntimeInfo, RuntimeInfo};
use crate::signal_handler::ChildTermination;
use crate::units::{
    ExitType, RestartMode, ServiceRestart, ServiceType, Specific, SuccessExitStatus, Timeout,
    UnitAction, UnitId, UnitOperationErrorReason, UnitStatus,
};

/// Compute a graduated restart delay that ramps from `base_sec` to
/// `max_delay_sec` over `steps` restarts. Returns the delay to use for the
/// given `restart_count`.
///
/// The delay for step *i* (0-indexed) is:
///   base + (max - base) * min(i, steps) / steps
///
/// If `max_delay_sec` is `None`, only `base_sec` is used.
fn compute_graduated_restart_delay(
    base_sec: &Option<Timeout>,
    max_delay_sec: &Option<Timeout>,
    steps: u32,
    restart_count: u64,
) -> Option<Timeout> {
    let base_dur = match base_sec {
        Some(Timeout::Duration(d)) => *d,
        _ => std::time::Duration::from_secs(0),
    };

    let max_dur = match max_delay_sec {
        Some(Timeout::Duration(d)) => *d,
        // No max configured — just use the base delay.
        _ => return base_sec.clone(),
    };

    if steps == 0 || max_dur <= base_dur {
        return base_sec.clone();
    }

    // Clamp the step index to [0, steps].
    let step_idx = std::cmp::min(restart_count, steps as u64) as u128;
    let range = max_dur.saturating_sub(base_dur);
    let increment = range.as_millis() * step_idx / (steps as u128);
    let delay = base_dur + std::time::Duration::from_millis(increment as u64);

    Some(Timeout::Duration(delay))
}

/// Check whether a cgroup directory still has live processes.
#[cfg(target_os = "linux")]
fn cgroup_has_processes(cgroup_path: &std::path::Path) -> bool {
    let procs_file = cgroup_path.join("cgroup.procs");
    std::fs::read_to_string(&procs_file)
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

/// Check whether a service has `RemainAfterExit=yes` configured.
fn has_remain_after_exit(unit: &crate::units::Unit) -> bool {
    if let Specific::Service(srvc) = &unit.specific {
        srvc.conf.remain_after_exit
    } else {
        false
    }
}

/// Retrieve the `SuccessExitStatus` config for a service unit, falling back
/// to the empty default if the unit is not a service.
fn get_success_exit_status(unit: &crate::units::Unit) -> SuccessExitStatus {
    if let Specific::Service(srvc) = &unit.specific {
        srvc.conf.success_exit_status.clone()
    } else {
        SuccessExitStatus::default()
    }
}

/// Info collected by the exit handler about OnSuccess=/OnFailure= units
/// that need to be triggered. The actual triggering happens AFTER the
/// RuntimeInfo read lock is dropped, to avoid writer-preferring RwLock
/// starvation when the trigger thread needs a write lock.
pub(crate) struct PendingTrigger {
    targets: Vec<String>,
    source_name: String,
    kind: String,
    monitor_env: crate::services::MonitorEnv,
}

/// Trigger OnSuccess= or OnFailure= units for a service that has reached
/// its final state. This resolves unit names from the config (loading from
/// disk if necessary), resets them to NeverStarted, and activates them
/// asynchronously in a new thread to avoid blocking the exit handler.
fn trigger_on_success_failure_units(
    unit_names: &[String],
    source_name: &str,
    kind: &str,
    arc_run_info: &ArcMutRuntimeInfo,
    monitor_env: crate::services::MonitorEnv,
) {
    for target_name in unit_names {
        trace!("Triggering {kind}={target_name} for unit {source_name}");
        // Activate asynchronously so we don't block restart logic.
        let arc_ri = arc_run_info.clone();
        let target_name_owned = target_name.clone();
        let source_name_owned = source_name.to_string();
        let kind_owned = kind.to_string();
        let mon = monitor_env.clone();
        std::thread::spawn(move || {
            // Find or load the target unit (may need to read from disk).
            let target_id = match crate::control::find_or_load_unit(&target_name_owned, &arc_ri) {
                Ok(id) => id,
                Err(e) => {
                    warn!(
                        "Could not find {kind_owned} unit {target_name_owned} for {source_name_owned}: {e}"
                    );
                    return;
                }
            };
            // Reset the target unit from Stopped → NeverStarted so it can be activated,
            // and set MONITOR_* env vars on the target service.
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
                    // Set MONITOR_* env vars on the target service.
                    if let Specific::Service(srvc) = &target_unit.specific {
                        let mut state = srvc.state.write_poisoned();
                        state.srvc.monitor_env = Some(mon);
                    }
                }
            }
            let errs = crate::units::activate_needed_units(target_id, arc_ri.clone());
            if errs.is_empty() {
                info!("{kind_owned} unit {target_name_owned} activated for {source_name_owned}");
            } else {
                for e in &errs {
                    warn!(
                        "Failed to activate {kind_owned} unit {target_name_owned} for {source_name_owned}: {e}"
                    );
                }
            }
        });
    }
}

/// Build a `MonitorEnv` from the source unit's termination status.
fn build_monitor_env(
    source_name: &str,
    code: &crate::signal_handler::ChildTermination,
    is_success: bool,
) -> crate::services::MonitorEnv {
    use crate::signal_handler::ChildTermination;
    let (exit_code, exit_status, service_result) = match code {
        ChildTermination::Exit(c) => (
            "exited".to_string(),
            c.to_string(),
            if is_success {
                "success".to_string()
            } else {
                "exit-code".to_string()
            },
        ),
        ChildTermination::Signal(s) => (
            "killed".to_string(),
            (*s as i32).to_string(),
            if is_success {
                "success".to_string()
            } else {
                "signal".to_string()
            },
        ),
    };
    crate::services::MonitorEnv {
        service_result,
        exit_code,
        exit_status,
        unit: source_name.to_string(),
    }
}

/// Determine whether a service should be restarted given its `Restart=`
/// policy and the way it terminated.
///
/// The logic mirrors systemd's restart table
/// (<https://www.freedesktop.org/software/systemd/man/systemd.service.html#Restart=>):
///
/// | Exit reason        | no | always | on-success | on-failure | on-abnormal | on-abort | on-watchdog |
/// |--------------------|----|--------|------------|------------|-------------|----------|-------------|
/// | Clean exit (0)     |    |   X    |     X      |            |             |          |             |
/// | Unclean exit (!=0) |    |   X    |            |     X      |             |          |             |
/// | Clean signal       |    |   X    |     X      |            |             |          |             |
/// | Unclean signal     |    |   X    |            |     X      |      X      |    X     |             |
/// | Timeout            |    |   X    |            |     X      |      X      |          |             |
/// | Watchdog           |    |   X    |            |     X      |      X      |          |      X      |
///
/// "Clean" signals are SIGHUP, SIGINT, SIGTERM, and SIGPIPE (matching systemd
/// defaults), plus any additional signals listed in `SuccessExitStatus=`.
///
/// "Clean" exit codes are 0 plus any additional codes listed in
/// `SuccessExitStatus=`.
///
/// `watchdog_fired` is `true` when the watchdog enforcement thread killed
/// this service due to a `WatchdogSec=` timeout (or `WATCHDOG=trigger`).
/// This enables `Restart=on-watchdog` and also counts as a failure for
/// `Restart=on-failure` and `Restart=on-abnormal`.
fn should_restart(
    policy: &ServiceRestart,
    termination: &ChildTermination,
    success_exit_status: &SuccessExitStatus,
    watchdog_fired: bool,
) -> bool {
    match policy {
        ServiceRestart::No => false,
        ServiceRestart::Always => true,
        ServiceRestart::OnSuccess => {
            success_exit_status.is_success(termination)
                || success_exit_status.is_clean_signal(termination)
        }
        ServiceRestart::OnFailure => {
            // Watchdog timeout counts as a failure.
            if watchdog_fired {
                return true;
            }
            // Non-zero exit (not in SuccessExitStatus), unclean signal, or
            // timeout (timeout is not currently tracked separately – a
            // killed-by-signal counts).
            match termination {
                ChildTermination::Exit(code) => {
                    *code != 0 && !success_exit_status.exit_codes.contains(code)
                }
                ChildTermination::Signal(sig) => {
                    !is_clean_signal_value(*sig) && !success_exit_status.signals.contains(sig)
                }
            }
        }
        ServiceRestart::OnAbnormal => {
            // Watchdog timeout counts as abnormal.
            if watchdog_fired {
                return true;
            }
            // Unclean signal or timeout – not on any exit code.
            match termination {
                ChildTermination::Exit(_) => false,
                ChildTermination::Signal(sig) => {
                    !is_clean_signal_value(*sig) && !success_exit_status.signals.contains(sig)
                }
            }
        }
        ServiceRestart::OnAbort => {
            // Unclean signal only (watchdog does NOT trigger on-abort).
            match termination {
                ChildTermination::Exit(_) => false,
                ChildTermination::Signal(sig) => {
                    !is_clean_signal_value(*sig) && !success_exit_status.signals.contains(sig)
                }
            }
        }
        ServiceRestart::OnWatchdog => {
            // Restart only on watchdog timeout.
            watchdog_fired
        }
    }
}

fn is_clean_signal_value(sig: nix::sys::signal::Signal) -> bool {
    use nix::sys::signal::Signal;
    matches!(
        sig,
        Signal::SIGHUP | Signal::SIGINT | Signal::SIGTERM | Signal::SIGPIPE
    )
}

/// Spawn a new thread to handle service exit cleanup and restart logic.
///
/// **Important**: The PID table entry has already been updated to
/// `ServiceExited` by the signal handler (see `signal_handler.rs`).  This
/// function only handles the *aftermath* — utmp records, oneshot cleanup,
/// restart policy, SuccessAction/FailureAction, etc.  It acquires the
/// `RuntimeInfo` read lock, which is safe because the critical PID-table
/// update is already visible to `wait_for_service`.
pub fn service_exit_handler_new_thread(
    pid: nix::unistd::Pid,
    srvc_id: UnitId,
    code: ChildTermination,
    run_info: ArcMutRuntimeInfo,
) {
    std::thread::spawn(move || {
        let pending_trigger = {
            let guard = run_info.read_poisoned();
            match service_exit_handler(pid, srvc_id, code, &guard, &run_info) {
                Ok(trigger) => trigger,
                Err(e) => {
                    error!("{e}");
                    None
                }
            }
        };
        // The RuntimeInfo read lock is now dropped. Trigger OnSuccess=/OnFailure=
        // units without holding any lock, so the trigger thread can freely
        // acquire write locks (e.g. in find_or_load_unit) without causing
        // writer-preferring RwLock starvation.
        if let Some(trigger) = pending_trigger {
            trigger_on_success_failure_units(
                &trigger.targets,
                &trigger.source_name,
                &trigger.kind,
                &run_info,
                trigger.monitor_env,
            );
        }
    });
}

/// Handle the aftermath of a service process exiting.
///
/// The PID table has already been updated by the signal handler; this function
/// deals with utmp records, oneshot process cleanup, restart decisions, and
/// SuccessAction/FailureAction triggers.
pub(crate) fn service_exit_handler(
    pid: nix::unistd::Pid,
    srvc_id: UnitId,
    code: ChildTermination,
    run_info: &RuntimeInfo,
    arc_run_info: &ArcMutRuntimeInfo,
) -> Result<Option<PendingTrigger>, String> {
    trace!(
        "Exit handler for service {:?} with pid: {pid} code: {code:?}",
        srvc_id.name
    );

    let Some(unit) = run_info.unit_table.get(&srvc_id) else {
        panic!("Tried to run a unit that has been removed from the map");
    };

    // Write DEAD_PROCESS utmp/wtmp record if the service had UtmpIdentifier= set.
    if let Specific::Service(srvc) = &unit.specific
        && let Some(ref utmp_id) = srvc.conf.exec_config.utmp_identifier
    {
        crate::entrypoints::write_utmp_dead_record(
            utmp_id,
            srvc.conf.exec_config.tty_path.as_deref(),
            pid,
        );
    }

    // Record the main process exit status and PID for ExecMainStatus/ExecMainPID properties.
    if let Specific::Service(srvc) = &unit.specific {
        let mut state = srvc.state.write_poisoned();
        let exit_code = match &code {
            ChildTermination::Exit(c) => *c,
            ChildTermination::Signal(s) => *s as i32,
        };
        state.srvc.main_exit_status = Some(exit_code);
        state.srvc.main_exit_pid = Some(pid);
        state.srvc.exec_main_exit_timestamp = Some(crate::units::UnitTimestamps::now_usec());
    }

    let success_exit_status = get_success_exit_status(unit);

    // ExitType=cgroup: if the service is configured to only be considered dead
    // when its cgroup is empty, check whether other processes remain. If so,
    // keep the service active and poll until the cgroup drains.
    // Skip this if the unit is already being stopped or is stopped (explicit
    // `systemctl stop` — that command handles deactivation itself).
    #[cfg(target_os = "linux")]
    if let Specific::Service(srvc) = &unit.specific
        && srvc.conf.exit_type == ExitType::Cgroup
        && srvc.conf.srcv_type != ServiceType::OneShot
    {
        let status_locked = unit.common.status.read_poisoned();
        let is_active = status_locked.is_started() || *status_locked == UnitStatus::Starting;
        drop(status_locked);

        let cgroup_path = &srvc.conf.platform_specific.cgroup_path;
        if is_active && cgroup_has_processes(cgroup_path) {
            let name = unit.id.name.clone();
            trace!(
                "Service {name}: main PID exited but ExitType=cgroup and cgroup still has processes, staying active"
            );
            // Clear the main PID since the process has exited, but keep
            // the service in Started state.
            {
                let mut state = srvc.state.write_poisoned();
                state.srvc.pid = None;
            }

            // Record the exit code from the main process for later use.
            let main_code = code;
            let main_success = success_exit_status.is_success(&code);

            // Poll the cgroup until it's empty, then deactivate the service.
            let cg = cgroup_path.clone();
            let id = srvc_id.clone();
            let ri = arc_run_info.clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    if !cgroup_has_processes(&cg) {
                        trace!("Service {name}: cgroup is now empty, completing exit");
                        break;
                    }
                }
                let guard = ri.read_poisoned();
                // Deactivate the service now that the cgroup is empty.
                loop {
                    let res = crate::units::deactivate_unit_recursive(&id, &guard);
                    let retry = if let Err(e) = &res {
                        matches!(e.reason, UnitOperationErrorReason::DependencyError(_))
                    } else {
                        false
                    };
                    if !retry {
                        if let Err(e) = res {
                            error!("Failed to deactivate {name} after cgroup drain: {e}");
                        }
                        break;
                    }
                }

                // If the main process exit was not clean, mark as failed.
                if !main_success && let Some(unit) = guard.unit_table.get(&id) {
                    let reason = match &main_code {
                        ChildTermination::Exit(c) => UnitOperationErrorReason::GenericStartError(
                            format!("process exited with status {c}"),
                        ),
                        ChildTermination::Signal(s) => UnitOperationErrorReason::GenericStartError(
                            format!("process killed by signal {s}"),
                        ),
                    };
                    let mut status = unit.common.status.write_poisoned();
                    *status = UnitStatus::Stopped(
                        crate::units::StatusStopped::StoppedUnexpected,
                        vec![reason],
                    );
                }

                // Clean up the cgroup directory.
                if cg.exists() {
                    let _ = std::fs::remove_dir(&cg);
                }
            });
            return Ok(None);
        }
    }

    // Handle oneshot service exit: clean up remaining processes and decide
    // whether to keep the service active (RemainAfterExit=yes) or deactivate it.
    {
        if let Specific::Service(srvc) = &unit.specific
            && srvc.conf.srcv_type == ServiceType::OneShot
        {
            {
                let mut_state = &mut *srvc.state.write_poisoned();
                mut_state
                    .srvc
                    .kill_all_remaining_processes(&srvc.conf, &unit.id.name);
            }

            // RemainAfterExit=yes: keep the unit in Started status after a
            // clean exit, matching systemd's behaviour for oneshot services
            // that perform setup tasks.
            if srvc.conf.remain_after_exit && success_exit_status.is_success(&code) {
                trace!(
                    "Oneshot service {} exited cleanly with RemainAfterExit=yes, staying active",
                    unit.id.name
                );
                return Ok(None);
            }

            // RemainAfterExit=no (default): deactivate the oneshot service
            // after its main process exits. This ensures `systemctl is-active`
            // correctly reports "inactive" (issue #27953).
            let name = &unit.id.name;
            trace!("Oneshot service {name} exited, deactivating (RemainAfterExit=no)");

            // Check SuccessAction/FailureAction for oneshot services too.
            let success_action = &unit.common.unit.success_action;
            let failure_action = &unit.common.unit.failure_action;
            if success_exit_status.is_success(&code) {
                if *success_action != UnitAction::None {
                    info!(
                        "Service {} exited successfully, triggering SuccessAction={:?}",
                        name, success_action
                    );
                    crate::units::execute_unit_action(success_action, name);
                }
            } else if *failure_action != UnitAction::None {
                info!(
                    "Service {} failed ({:?}), triggering FailureAction={:?}",
                    name, code, failure_action
                );
                crate::units::execute_unit_action(failure_action, name);
            }

            // For failed oneshot services, check if we should restart.
            trace!(
                "Oneshot service {name} exit handler: success={}, restart_policy={:?}",
                success_exit_status.is_success(&code),
                srvc.conf.restart
            );
            let oneshot_should_restart = if !success_exit_status.is_success(&code) {
                let prevent_restart = {
                    let rps = &srvc.conf.restart_prevent_exit_status;
                    match &code {
                        ChildTermination::Exit(c) => rps.exit_codes.contains(c),
                        ChildTermination::Signal(s) => rps.signals.contains(s),
                    }
                };
                let force_restart = {
                    let rfs = &srvc.conf.restart_force_exit_status;
                    match &code {
                        ChildTermination::Exit(c) => rfs.exit_codes.contains(c),
                        ChildTermination::Signal(s) => rfs.signals.contains(s),
                    }
                };
                !prevent_restart
                    && (force_restart
                        || should_restart(&srvc.conf.restart, &code, &success_exit_status, false))
            } else {
                false
            };

            if oneshot_should_restart {
                // Compute restart delay
                let restart_sec = if srvc.conf.restart_steps > 0 {
                    let restart_count = srvc.state.read_poisoned().common.restart_count;
                    compute_graduated_restart_delay(
                        &srvc.conf.restart_sec,
                        &srvc.conf.restart_max_delay_sec,
                        srvc.conf.restart_steps,
                        restart_count,
                    )
                } else {
                    srvc.conf.restart_sec.clone()
                };

                // Mark as Restarting so SubState shows "auto-restart"
                if let Some(unit) = run_info.unit_table.get(&srvc_id) {
                    let mut status = unit.common.status.write_poisoned();
                    *status = crate::units::UnitStatus::Restarting;
                }

                // Increment NRestarts counter (both the state-locked field
                // and the lock-free atomic on Common so that `systemctl show`
                // can read it without acquiring the service state lock).
                {
                    let mut state = srvc.state.write_poisoned();
                    state.common.restart_count += 1;
                }
                if let Some(unit) = run_info.unit_table.get(&srvc_id) {
                    unit.common
                        .n_restarts
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }

                // Sleep for RestartSec, checking for shortcut restart
                if let Some(ref timeout) = restart_sec {
                    match timeout {
                        Timeout::Duration(dur) => {
                            trace!(
                                "Waiting {:?} (RestartSec) before restarting oneshot service {name}",
                                dur
                            );
                            let deadline = std::time::Instant::now() + *dur;
                            loop {
                                if std::time::Instant::now() >= deadline {
                                    break;
                                }
                                if let Some(unit) = run_info.unit_table.get(&srvc_id) {
                                    let st = unit.common.status.read_poisoned();
                                    if !matches!(&*st, crate::units::UnitStatus::Restarting) {
                                        trace!(
                                            "Restart of oneshot {name} shortcut by external start"
                                        );
                                        return Ok(None);
                                    }
                                }
                                std::thread::sleep(std::time::Duration::from_millis(100));
                            }
                        }
                        Timeout::Infinity => {
                            trace!(
                                "RestartSec=infinity for oneshot service {name}, not restarting"
                            );
                            return Ok(None);
                        }
                    }
                }

                // Recheck — may have been shortcut-started
                if let Some(unit) = run_info.unit_table.get(&srvc_id) {
                    let st = unit.common.status.read_poisoned();
                    if !matches!(&*st, crate::units::UnitStatus::Restarting) {
                        return Ok(None);
                    }
                }

                // Check start rate limit
                if let Some(unit) = run_info.unit_table.get(&srvc_id)
                    && !crate::units::check_start_rate_limit(unit)
                {
                    warn!("Oneshot unit {name} hit start rate limit, refusing automatic restart");
                    let reason = crate::units::UnitOperationErrorReason::GenericStartError(
                        "Start request repeated too quickly".into(),
                    );
                    let mut status = unit.common.status.write_poisoned();
                    *status = crate::units::UnitStatus::Stopped(
                        crate::units::StatusStopped::StoppedUnexpected,
                        vec![reason],
                    );
                    drop(status);
                    // Propagate failure to required_by deps (e.g.
                    // targets with Requires= on this service).
                    for dep_id in &unit.common.dependencies.required_by {
                        if let Some(dep) = run_info.unit_table.get(dep_id)
                            && dep.common.status.read_poisoned().is_started()
                        {
                            let mut dep_st = dep.common.status.write_poisoned();
                            *dep_st = crate::units::UnitStatus::Stopped(
                                crate::units::StatusStopped::StoppedFinal,
                                vec![],
                            );
                        }
                    }
                    return Ok(None);
                }

                info!("Restarting oneshot service {name}");

                // With RestartMode=normal, propagate the stop to deps before
                // restarting, matching real systemd's full deactivation path.
                // With RestartMode=direct, skip dep propagation.
                if srvc.conf.restart_mode == RestartMode::Normal
                    && let Some(unit) = run_info.unit_table.get(&srvc_id)
                {
                    for dep_id in unit
                        .common
                        .dependencies
                        .bound_by
                        .iter()
                        .chain(unit.common.dependencies.required_by.iter())
                        .chain(unit.common.dependencies.part_of_by.iter())
                    {
                        if let Err(e) = crate::units::deactivate_unit_recursive(dep_id, run_info) {
                            trace!(
                                "Failed to propagate stop to {} during RestartMode=normal oneshot restart of {name}: {e}",
                                dep_id.name
                            );
                        }
                    }
                }

                // Spawn the restart on a separate thread so that this exit
                // handler can return and release the RuntimeInfo read lock.
                // Oneshot reactivation blocks in wait_for_service until the
                // new process exits, which can take a long time.  Holding
                // the read lock during that window triggers a priority
                // inversion with glibc's writer-preferring rwlock: any
                // pending write lock (e.g. from find_or_load_unit loading a
                // unit from disk) will block all new readers, deadlocking
                // control-socket commands like `systemctl show`.
                let arc_ri = arc_run_info.clone();
                let restart_id = srvc_id.clone();
                std::thread::spawn(move || {
                    let ri = arc_ri.read_poisoned();
                    if let Err(e) = crate::units::reactivate_unit(restart_id, &ri) {
                        error!("Failed to restart oneshot service: {e}");
                    }
                });
                return Ok(None);
            }

            let is_success = success_exit_status.is_success(&code);
            if is_success {
                // Clean oneshot exit: do NOT deactivate.  In real
                // systemd, Requires= is a start-time dependency — the
                // service's start job completed, so reverse deps
                // (targets) stay active.  Calling deactivate_unit_recursive
                // here would try to stop them (and fail because the
                // target's own deps are still running, which just logs
                // an error), but the attempt also prevents THIS service
                // from being marked Stopped in time for concurrent
                // activation of later units that depend on it.  By
                // leaving the service in its Started state we match
                // real systemd's effective behavior.
            } else {
                // Failed exit: full recursive deactivation including
                // required_by to propagate the failure.
                crate::units::deactivate_unit_recursive(&srvc_id, run_info)
                    .map_err(|e| format!("{e}"))?;
            }

            // Mark as failed if the exit was not clean.
            if !is_success && let Some(unit) = run_info.unit_table.get(&srvc_id) {
                let mut status = unit.common.status.write_poisoned();
                let reason = match &code {
                    ChildTermination::Exit(c) => UnitOperationErrorReason::GenericStartError(
                        format!("process exited with status {c}"),
                    ),
                    ChildTermination::Signal(s) => UnitOperationErrorReason::GenericStartError(
                        format!("process killed by signal {s}"),
                    ),
                };
                *status = UnitStatus::Stopped(
                    crate::units::StatusStopped::StoppedUnexpected,
                    vec![reason],
                );
                info!(
                    "Oneshot service {name} failed with {:?}, marked as failed",
                    code
                );
            }

            // Collect OnSuccess=/OnFailure= trigger info for the oneshot service.
            // Actual triggering happens after the read lock is dropped.
            if let Some(unit) = run_info.unit_table.get(&srvc_id) {
                if is_success && !unit.common.unit.on_success.is_empty() {
                    return Ok(Some(PendingTrigger {
                        targets: unit.common.unit.on_success.clone(),
                        source_name: name.to_string(),
                        kind: "OnSuccess".to_string(),
                        monitor_env: build_monitor_env(name, &code, true),
                    }));
                } else if !is_success && !unit.common.unit.on_failure.is_empty() {
                    return Ok(Some(PendingTrigger {
                        targets: unit.common.unit.on_failure.clone(),
                        source_name: name.to_string(),
                        kind: "OnFailure".to_string(),
                        monitor_env: build_monitor_env(name, &code, false),
                    }));
                }
            }
            return Ok(None);
        }
    }

    // Determine SuccessAction / FailureAction for this unit and whether
    // the exit counts as success or failure.
    let success_action = &unit.common.unit.success_action;
    let failure_action = &unit.common.unit.failure_action;

    if success_exit_status.is_success(&code) {
        if *success_action != UnitAction::None {
            info!(
                "Service {} exited successfully, triggering SuccessAction={:?}",
                unit.id.name, success_action
            );
            crate::units::execute_unit_action(success_action, &unit.id.name);
        }
    } else if *failure_action != UnitAction::None {
        info!(
            "Service {} failed ({:?}), triggering FailureAction={:?}",
            unit.id.name, code, failure_action
        );
        crate::units::execute_unit_action(failure_action, &unit.id.name);
    }

    trace!("Check if we want to restart the unit");
    let name = &unit.id.name;
    let (restart_unit, restart_sec, restart_mode) = {
        if let Specific::Service(srvc) = &unit.specific {
            trace!(
                "Service with id: {:?}, name: {} pid: {} exited with: {:?}",
                srvc_id, unit.id.name, pid, code
            );

            let restart_mode = srvc.conf.restart_mode;

            // Check whether the watchdog or RuntimeMaxSec enforcement
            // thread killed this service.
            let (watchdog_fired, runtime_max_fired) = {
                let state = srvc.state.read_poisoned();
                (
                    state.srvc.watchdog_timeout_fired,
                    state.srvc.runtime_max_timeout_fired,
                )
            };
            if watchdog_fired {
                trace!(
                    "Service {}: exit was caused by watchdog timeout",
                    unit.id.name
                );
            }
            if runtime_max_fired {
                trace!(
                    "Service {}: exit was caused by RuntimeMaxSec timeout",
                    unit.id.name
                );
            }
            // Both watchdog and runtime-max timeouts count as a "timeout"
            // failure for restart policy purposes.
            let timeout_fired = watchdog_fired || runtime_max_fired;

            // RestartPreventExitStatus= overrides the Restart= policy: if the
            // termination status matches any entry, prevent restart regardless
            // of the configured Restart= setting.
            let prevent_restart = {
                let rps = &srvc.conf.restart_prevent_exit_status;
                match &code {
                    ChildTermination::Exit(c) => rps.exit_codes.contains(c),
                    ChildTermination::Signal(s) => rps.signals.contains(s),
                }
            };

            if prevent_restart {
                trace!(
                    "Service {}: exit status {:?} matches RestartPreventExitStatus=, skipping restart",
                    unit.id.name, code
                );
                (false, srvc.conf.restart_sec.clone(), restart_mode)
            } else {
                // RestartForceExitStatus= overrides the Restart= policy: if the
                // termination status matches any entry, force a restart.
                let force_restart = {
                    let rfs = &srvc.conf.restart_force_exit_status;
                    match &code {
                        ChildTermination::Exit(c) => rfs.exit_codes.contains(c),
                        ChildTermination::Signal(s) => rfs.signals.contains(s),
                    }
                };

                let do_restart = force_restart
                    || should_restart(
                        &srvc.conf.restart,
                        &code,
                        &success_exit_status,
                        timeout_fired,
                    );

                // Graduated restart delay: if RestartSteps= > 0 and
                // RestartMaxDelaySec= is set, compute an increasing delay
                // that ramps up from RestartSec= to RestartMaxDelaySec= over
                // the configured number of steps based on the restart count.
                let restart_sec = if do_restart && srvc.conf.restart_steps > 0 {
                    let restart_count = srvc.state.read_poisoned().common.restart_count;
                    compute_graduated_restart_delay(
                        &srvc.conf.restart_sec,
                        &srvc.conf.restart_max_delay_sec,
                        srvc.conf.restart_steps,
                        restart_count,
                    )
                } else {
                    srvc.conf.restart_sec.clone()
                };

                (do_restart, restart_sec, restart_mode)
            }
        } else {
            (false, None, RestartMode::default())
        }
    };

    // check that the status is "Started". If thats not the case this service got killed by something else (control interface for example) so dont interfere
    {
        let status_locked = &*unit.common.status.read_poisoned();
        let dominated = status_locked.is_started() || *status_locked == UnitStatus::Starting;
        // For oneshot services, the activation thread may have already set
        // the status to StoppedUnexpected before the exit handler runs.
        // If we need to restart, allow the handler to proceed anyway.
        let stopped_but_restartable = restart_unit
            && matches!(
                status_locked,
                UnitStatus::Stopped(crate::units::StatusStopped::StoppedUnexpected, _)
            );
        if !dominated && !stopped_but_restartable {
            trace!(
                "Exit handler ignores exit of service {}. Its status is not 'Started'/'Starting', it is: {:?}",
                name, *status_locked
            );
            return Ok(None);
        }
    }

    // RemainAfterExit=yes: if the process exited cleanly, keep the service
    // in its current active state — do not restart, do not deactivate.
    if has_remain_after_exit(unit) && success_exit_status.is_success(&code) {
        trace!("Service {name} exited cleanly with RemainAfterExit=yes, staying active");
        return Ok(None);
    }

    // Run ExecStopPost= commands. In real systemd these always run when a
    // service process exits, regardless of exit status or restart policy.
    if let Specific::Service(srvc) = &unit.specific
        && !srvc.conf.stoppost.is_empty()
    {
        trace!("Running ExecStopPost for service {name}");
        let mut state = srvc.state.write_poisoned();
        let timeout = state.srvc.get_stop_timeout(&srvc.conf);
        let cmds = srvc.conf.stoppost.clone();
        if let Err(e) = state.srvc.run_all_cmds(
            &cmds,
            srvc_id.clone(),
            name,
            timeout,
            run_info,
            srvc.conf.exec_config.working_directory.as_ref(),
        ) {
            warn!("ExecStopPost for service {name} failed: {e:?}");
        }
    }

    if restart_unit {
        // With RestartMode=direct, skip OnSuccess=/OnFailure= triggering and
        // dependency propagation — the service restarts "directly" without
        // going through the full deactivation path. With RestartMode=normal,
        // fire these triggers as real systemd does.
        if restart_mode == RestartMode::Normal
            && let Some(unit) = run_info.unit_table.get(&srvc_id)
        {
            let is_clean =
                success_exit_status.is_success(&code) || success_exit_status.is_clean_signal(&code);
            if is_clean && !unit.common.unit.on_success.is_empty() {
                let targets = unit.common.unit.on_success.clone();
                let mon = build_monitor_env(name, &code, true);
                trigger_on_success_failure_units(&targets, name, "OnSuccess", arc_run_info, mon);
            } else if !is_clean && !unit.common.unit.on_failure.is_empty() {
                let targets = unit.common.unit.on_failure.clone();
                let mon = build_monitor_env(name, &code, false);
                trigger_on_success_failure_units(&targets, name, "OnFailure", arc_run_info, mon);
            }
        }

        // With RestartMode=normal, propagate the stop to bound/required_by
        // units before restarting. This matches real systemd: the service goes
        // through the full deactivation path, so BindsTo= deps are stopped.
        // With RestartMode=direct, skip this — deps don't see the restart.
        //
        // We track which deps to RE-ACTIVATE after restart separately:
        // only bound_by gets re-activated (BindsTo= semantics). required_by
        // and part_of_by deps are NOT re-activated because their start jobs
        // already completed/failed — re-activating them would differ from
        // real systemd where the job queue doesn't re-enqueue them.
        let stopped_deps = if restart_mode == RestartMode::Normal {
            let mut all_deps = Vec::new();
            let mut reactivate_deps = Vec::new();
            if let Some(unit) = run_info.unit_table.get(&srvc_id) {
                reactivate_deps.extend(unit.common.dependencies.bound_by.clone());
                all_deps.extend(unit.common.dependencies.bound_by.clone());
                all_deps.extend(unit.common.dependencies.required_by.clone());
                all_deps.extend(unit.common.dependencies.part_of_by.clone());
            }
            for dep_id in &all_deps {
                if let Err(e) = crate::units::deactivate_unit_recursive(dep_id, run_info) {
                    trace!(
                        "Failed to propagate stop to {} during RestartMode=normal restart of {name}: {e}",
                        dep_id.name
                    );
                }
            }
            reactivate_deps
        } else {
            Vec::new()
        };

        // Mark the unit as Restarting so that SubState shows "auto-restart"
        // and `systemctl start` can shortcut the pending restart.
        if let Some(unit) = run_info.unit_table.get(&srvc_id) {
            let mut status = unit.common.status.write_poisoned();
            *status = crate::units::UnitStatus::Restarting;
        }

        // Increment NRestarts counter (both state-locked and lock-free atomic).
        if let Some(unit) = run_info.unit_table.get(&srvc_id) {
            if let Specific::Service(svc) = &unit.specific {
                let mut state = svc.state.write_poisoned();
                state.common.restart_count += 1;
            }
            unit.common
                .n_restarts
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        if let Some(ref timeout) = restart_sec {
            match timeout {
                Timeout::Duration(dur) => {
                    trace!(
                        "Waiting {:?} (RestartSec) before restarting service {name}",
                        dur
                    );
                    // Sleep in small increments so that an explicit
                    // `systemctl start` can shortcut the pending restart.
                    let deadline = std::time::Instant::now() + *dur;
                    loop {
                        if std::time::Instant::now() >= deadline {
                            break;
                        }
                        // Check if another thread (e.g. `systemctl start`)
                        // has already changed the status away from Restarting.
                        if let Some(unit) = run_info.unit_table.get(&srvc_id) {
                            let st = unit.common.status.read_poisoned();
                            if !matches!(&*st, crate::units::UnitStatus::Restarting) {
                                trace!(
                                    "Restart of {name} shortcut by external start; aborting auto-restart"
                                );
                                return Ok(None);
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
                Timeout::Infinity => {
                    trace!("RestartSec=infinity for service {name}, not restarting");
                    return Ok(None);
                }
            }
        }

        // Recheck that the unit is still in Restarting state (not shortcut).
        if let Some(unit) = run_info.unit_table.get(&srvc_id) {
            let st = unit.common.status.read_poisoned();
            if !matches!(&*st, crate::units::UnitStatus::Restarting) {
                return Ok(None);
            }
        }

        // Check start rate limit before automatic restart.
        if let Some(unit) = run_info.unit_table.get(&srvc_id)
            && !crate::units::check_start_rate_limit(unit)
        {
            warn!("Unit {name} hit start rate limit, refusing automatic restart");
            let reason = crate::units::UnitOperationErrorReason::GenericStartError(
                "Start request repeated too quickly".into(),
            );
            {
                let mut status = unit.common.status.write_poisoned();
                *status = crate::units::UnitStatus::Stopped(
                    crate::units::StatusStopped::StoppedUnexpected,
                    vec![reason],
                );
            }
            let start_limit_action = &unit.common.unit.start_limit_action;
            if *start_limit_action != crate::units::UnitAction::None {
                info!(
                    "Unit {name} hit start limit, triggering StartLimitAction={:?}",
                    start_limit_action
                );
                crate::units::execute_unit_action(start_limit_action, name);
            }
            return Ok(None);
        }
        trace!("Restart service {name} after it died");
        // Clear the old PID and process group so that srvc.start() does not
        // reject the restart with AlreadyHasPID.  The old process is already
        // dead (we are in its exit handler).
        if let Some(unit) = run_info.unit_table.get(&srvc_id)
            && let Specific::Service(srvc) = &unit.specific
        {
            let mut state = srvc.state.write_poisoned();
            state.srvc.pid = None;
            state.srvc.process_group = None;
        }
        crate::units::reactivate_unit(srvc_id, run_info).map_err(|e| format!("{e}"))?;

        // For RestartMode=normal, re-activate only the BindsTo= deps that
        // were stopped. required_by/part_of_by deps are NOT re-activated
        // because their original start jobs already completed/failed.
        for dep_id in &stopped_deps {
            if let Some(dep_unit) = run_info.unit_table.get(dep_id) {
                let dep_status = dep_unit.common.status.read_poisoned().clone();
                if matches!(
                    dep_status,
                    UnitStatus::Stopped(..) | UnitStatus::NeverStarted
                ) {
                    // Reset to NeverStarted so activate_unit doesn't skip the
                    // unit (it returns early for StoppedUnexpected).
                    {
                        let mut status = dep_unit.common.status.write_poisoned();
                        if status.is_stopped() {
                            *status = UnitStatus::NeverStarted;
                        }
                    }
                    if let Err(e) = crate::units::activate_unit(
                        dep_id.clone(),
                        run_info,
                        crate::units::ActivationSource::Regular,
                    ) {
                        trace!(
                            "Failed to re-activate {} after RestartMode=normal restart of {name}: {e}",
                            dep_id.name
                        );
                    }
                }
            }
        }
    } else {
        // Detect whether this exit is for a stale process that was replaced
        // by a restart.  If the service already has a different PID (the new
        // process started by reactivate()), or no PID at all (killed and
        // cleared by reactivate→kill), skip recursive deactivation — it would
        // race and kill the freshly-started replacement.  Also check the
        // Stopping status as a fallback for the window between kill() and
        // start().
        let was_replaced_or_stopping = {
            if let Some(unit) = run_info.unit_table.get(&srvc_id) {
                let current = unit.common.status.read_poisoned();
                if matches!(&*current, UnitStatus::Stopping) {
                    true
                } else if let Specific::Service(srvc) = &unit.specific {
                    let state = srvc.state.read_poisoned();
                    // If the service now has a different PID, reactivate()
                    // already started a replacement — this exit is stale.
                    match state.srvc.pid {
                        Some(current_pid) => current_pid != pid,
                        None => false,
                    }
                } else {
                    false
                }
            } else {
                false
            }
        };

        if !was_replaced_or_stopping {
            trace!("Recursively killing all services requiring service {name}");
            loop {
                let res = crate::units::deactivate_unit_recursive(&srvc_id, run_info);
                let retry = if let Err(e) = &res {
                    if let UnitOperationErrorReason::DependencyError(_) = e.reason {
                        // Only retry if this is the case. This only occurs if, while the units are being deactivated,
                        // another unit got activated that would not be able to run with this unit deactivated.
                        // This should generally be pretty rare but it should be handled properly.
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !retry {
                    res.map_err(|e| format!("{e}"))?;
                    break;
                }
            }
        } else {
            trace!(
                "Skipping recursive deactivation for service {name} (intentional stop, likely restart)"
            );
        }

        // If the exit was not clean (non-zero exit code or killed by signal),
        // override the status to "failed" so `systemctl show` reports
        // ActiveState=failed/SubState=failed.
        // However, if the unit was being intentionally stopped (status was Stopping
        // before deactivation), don't mark it as failed — signal death from an
        // intentional stop is expected behavior.
        let is_success = success_exit_status.is_success(&code);
        if !is_success
            && !was_replaced_or_stopping
            && let Some(unit) = run_info.unit_table.get(&srvc_id)
        {
            let mut status = unit.common.status.write_poisoned();
            let reason = match &code {
                ChildTermination::Exit(c) => UnitOperationErrorReason::GenericStartError(format!(
                    "process exited with status {c}"
                )),
                ChildTermination::Signal(s) => UnitOperationErrorReason::GenericStartError(
                    format!("process killed by signal {s}",),
                ),
            };
            *status =
                UnitStatus::Stopped(crate::units::StatusStopped::StoppedUnexpected, vec![reason]);
            info!("Service {name} failed with {:?}, marked as failed", code);
        }

        // Clean up the cgroup directory now that the service has stopped.
        // This matches real systemd's behavior of removing empty cgroup dirs.
        #[cfg(target_os = "linux")]
        if let Some(unit) = run_info.unit_table.get(&srvc_id)
            && let Specific::Service(svc) = &unit.specific
        {
            let cgroup_path = &svc.conf.platform_specific.cgroup_path;
            if cgroup_path.exists() {
                // Only remove if the cgroup is empty (no more processes).
                let procs_file = cgroup_path.join("cgroup.procs");
                let is_empty = std::fs::read_to_string(&procs_file)
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true);
                if is_empty {
                    if let Err(e) = std::fs::remove_dir(cgroup_path) {
                        trace!(
                            "Could not remove cgroup dir {}: {}",
                            cgroup_path.display(),
                            e
                        );
                    } else {
                        trace!(
                            "Cleaned up cgroup dir {} for {}",
                            cgroup_path.display(),
                            srvc_id.name
                        );
                        // Try to remove the parent slice cgroup dir if empty.
                        if let Some(parent) = cgroup_path.parent() {
                            let parent_procs = parent.join("cgroup.procs");
                            let parent_empty = std::fs::read_to_string(&parent_procs)
                                .map(|s| s.trim().is_empty())
                                .unwrap_or(false);
                            if parent_empty {
                                // Check no child dirs remain
                                let has_children = std::fs::read_dir(parent)
                                    .map(|entries| {
                                        entries.filter_map(|e| e.ok()).any(|e| e.path().is_dir())
                                    })
                                    .unwrap_or(false);
                                if !has_children {
                                    let _ = std::fs::remove_dir(parent);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Collect OnSuccess=/OnFailure= trigger info.
        // Actual triggering happens after the read lock is dropped.
        let mut pending_trigger = None;
        if let Some(unit) = run_info.unit_table.get(&srvc_id) {
            let effective_success = is_success
                || was_replaced_or_stopping
                || success_exit_status.is_clean_signal(&code);
            if effective_success && !unit.common.unit.on_success.is_empty() {
                pending_trigger = Some(PendingTrigger {
                    targets: unit.common.unit.on_success.clone(),
                    source_name: name.to_string(),
                    kind: "OnSuccess".to_string(),
                    monitor_env: build_monitor_env(name, &code, true),
                });
            } else if !effective_success && !unit.common.unit.on_failure.is_empty() {
                pending_trigger = Some(PendingTrigger {
                    targets: unit.common.unit.on_failure.clone(),
                    source_name: name.to_string(),
                    kind: "OnFailure".to_string(),
                    monitor_env: build_monitor_env(name, &code, false),
                });
            }
        }

        // Upholds= enforcement: if this service is upheld by any active unit,
        // restart it asynchronously. This implements systemd's Upholds= semantics:
        // as long as the upholding unit is active, the upheld unit is kept running.
        if !was_replaced_or_stopping && let Some(unit) = run_info.unit_table.get(&srvc_id) {
            let upheld_by = unit.common.dependencies.upheld_by.clone();
            if !upheld_by.is_empty() {
                let any_active = upheld_by.iter().any(|upholding_id| {
                    run_info
                        .unit_table
                        .get(upholding_id)
                        .map(|u| u.common.status.read_poisoned().is_started())
                        .unwrap_or(false)
                });
                if any_active {
                    trace!("Service {name} is upheld by an active unit, scheduling restart");
                    // Reset to NeverStarted so it can be re-activated.
                    {
                        let mut status = unit.common.status.write_poisoned();
                        *status = UnitStatus::NeverStarted;
                    }
                    let srvc_id_clone = srvc_id.clone();
                    let arc_ri = arc_run_info.clone();
                    std::thread::spawn(move || {
                        crate::units::upholds_retry_loop(srvc_id_clone, arc_ri);
                    });
                }
            }
        }

        return Ok(pending_trigger);
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::{ServiceRestart, SuccessExitStatus, Timeout};
    use nix::sys::signal::Signal;
    use std::time::Duration;

    // --- compute_graduated_restart_delay ---

    #[test]
    fn test_graduated_delay_no_steps() {
        // steps=0 should just return the base delay
        let base = Some(Timeout::Duration(Duration::from_secs(1)));
        let max = Some(Timeout::Duration(Duration::from_secs(60)));
        let result = compute_graduated_restart_delay(&base, &max, 0, 5);
        assert_eq!(result, base);
    }

    #[test]
    fn test_graduated_delay_no_max() {
        // No max configured — just use the base delay
        let base = Some(Timeout::Duration(Duration::from_secs(1)));
        let result = compute_graduated_restart_delay(&base, &None, 5, 3);
        assert_eq!(result, base);
    }

    #[test]
    fn test_graduated_delay_step_zero() {
        // First restart (restart_count=0) should return base delay
        let base = Some(Timeout::Duration(Duration::from_secs(1)));
        let max = Some(Timeout::Duration(Duration::from_secs(60)));
        let result = compute_graduated_restart_delay(&base, &max, 5, 0);
        assert_eq!(result, Some(Timeout::Duration(Duration::from_secs(1))));
    }

    #[test]
    fn test_graduated_delay_midpoint() {
        // Halfway through steps: base=0s, max=10s, steps=10, restart_count=5
        // delay = 0 + (10000ms * 5 / 10) = 5000ms = 5s
        let base = Some(Timeout::Duration(Duration::from_secs(0)));
        let max = Some(Timeout::Duration(Duration::from_secs(10)));
        let result = compute_graduated_restart_delay(&base, &max, 10, 5);
        assert_eq!(result, Some(Timeout::Duration(Duration::from_secs(5))));
    }

    #[test]
    fn test_graduated_delay_full_steps() {
        // At max steps: base=1s, max=60s, steps=5, restart_count=5
        // delay = 1s + (59000ms * 5 / 5) = 1s + 59s = 60s
        let base = Some(Timeout::Duration(Duration::from_secs(1)));
        let max = Some(Timeout::Duration(Duration::from_secs(60)));
        let result = compute_graduated_restart_delay(&base, &max, 5, 5);
        assert_eq!(result, Some(Timeout::Duration(Duration::from_secs(60))));
    }

    #[test]
    fn test_graduated_delay_beyond_steps_clamped() {
        // restart_count exceeds steps — should be clamped to steps
        let base = Some(Timeout::Duration(Duration::from_secs(1)));
        let max = Some(Timeout::Duration(Duration::from_secs(60)));
        let result = compute_graduated_restart_delay(&base, &max, 5, 100);
        assert_eq!(result, Some(Timeout::Duration(Duration::from_secs(60))));
    }

    #[test]
    fn test_graduated_delay_max_less_than_base() {
        // max <= base — just return base
        let base = Some(Timeout::Duration(Duration::from_secs(60)));
        let max = Some(Timeout::Duration(Duration::from_secs(10)));
        let result = compute_graduated_restart_delay(&base, &max, 5, 3);
        assert_eq!(result, base);
    }

    #[test]
    fn test_graduated_delay_base_none() {
        // No base — base_dur defaults to 0
        let max = Some(Timeout::Duration(Duration::from_secs(10)));
        let result = compute_graduated_restart_delay(&None, &max, 10, 10);
        assert_eq!(result, Some(Timeout::Duration(Duration::from_secs(10))));
    }

    #[test]
    fn test_graduated_delay_base_infinity() {
        // Base is infinity — base_dur defaults to 0
        let base = Some(Timeout::Infinity);
        let max = Some(Timeout::Duration(Duration::from_secs(10)));
        let result = compute_graduated_restart_delay(&base, &max, 10, 5);
        assert_eq!(result, Some(Timeout::Duration(Duration::from_secs(5))));
    }

    #[test]
    fn test_graduated_delay_max_infinity() {
        // Max is infinity — returns base (no max configured path)
        let base = Some(Timeout::Duration(Duration::from_secs(5)));
        let max = Some(Timeout::Infinity);
        let result = compute_graduated_restart_delay(&base, &max, 10, 5);
        assert_eq!(result, base);
    }

    // --- should_restart ---

    #[test]
    fn test_should_restart_no_never_restarts() {
        let ses = SuccessExitStatus::default();
        assert!(!should_restart(
            &ServiceRestart::No,
            &ChildTermination::Exit(0),
            &ses,
            false,
        ));
        assert!(!should_restart(
            &ServiceRestart::No,
            &ChildTermination::Exit(1),
            &ses,
            false,
        ));
        assert!(!should_restart(
            &ServiceRestart::No,
            &ChildTermination::Signal(Signal::SIGKILL),
            &ses,
            false,
        ));
    }

    #[test]
    fn test_should_restart_always() {
        let ses = SuccessExitStatus::default();
        assert!(should_restart(
            &ServiceRestart::Always,
            &ChildTermination::Exit(0),
            &ses,
            false,
        ));
        assert!(should_restart(
            &ServiceRestart::Always,
            &ChildTermination::Exit(1),
            &ses,
            false,
        ));
        assert!(should_restart(
            &ServiceRestart::Always,
            &ChildTermination::Signal(Signal::SIGKILL),
            &ses,
            false,
        ));
        assert!(should_restart(
            &ServiceRestart::Always,
            &ChildTermination::Signal(Signal::SIGTERM),
            &ses,
            false,
        ));
    }

    #[test]
    fn test_should_restart_on_success_clean_exit() {
        let ses = SuccessExitStatus::default();
        // Exit code 0 is clean
        assert!(should_restart(
            &ServiceRestart::OnSuccess,
            &ChildTermination::Exit(0),
            &ses,
            false,
        ));
        // Exit code 1 is not clean
        assert!(!should_restart(
            &ServiceRestart::OnSuccess,
            &ChildTermination::Exit(1),
            &ses,
            false,
        ));
    }

    #[test]
    fn test_should_restart_on_success_clean_signal() {
        let ses = SuccessExitStatus::default();
        // SIGTERM is a clean signal
        assert!(should_restart(
            &ServiceRestart::OnSuccess,
            &ChildTermination::Signal(Signal::SIGTERM),
            &ses,
            false,
        ));
        // SIGKILL is not a clean signal
        assert!(!should_restart(
            &ServiceRestart::OnSuccess,
            &ChildTermination::Signal(Signal::SIGKILL),
            &ses,
            false,
        ));
    }

    #[test]
    fn test_should_restart_on_success_custom_exit_code() {
        let ses = SuccessExitStatus {
            exit_codes: vec![42],
            signals: vec![],
        };
        // 42 is in SuccessExitStatus — counts as clean
        assert!(should_restart(
            &ServiceRestart::OnSuccess,
            &ChildTermination::Exit(42),
            &ses,
            false,
        ));
    }

    #[test]
    fn test_should_restart_on_failure_nonzero_exit() {
        let ses = SuccessExitStatus::default();
        // Non-zero exit should restart on-failure
        assert!(should_restart(
            &ServiceRestart::OnFailure,
            &ChildTermination::Exit(1),
            &ses,
            false,
        ));
        // Zero exit should not restart on-failure
        assert!(!should_restart(
            &ServiceRestart::OnFailure,
            &ChildTermination::Exit(0),
            &ses,
            false,
        ));
    }

    #[test]
    fn test_should_restart_on_failure_unclean_signal() {
        let ses = SuccessExitStatus::default();
        // SIGKILL is unclean — should restart
        assert!(should_restart(
            &ServiceRestart::OnFailure,
            &ChildTermination::Signal(Signal::SIGKILL),
            &ses,
            false,
        ));
        // SIGTERM is clean — should not restart
        assert!(!should_restart(
            &ServiceRestart::OnFailure,
            &ChildTermination::Signal(Signal::SIGTERM),
            &ses,
            false,
        ));
    }

    #[test]
    fn test_should_restart_on_failure_custom_success_code_not_failure() {
        let ses = SuccessExitStatus {
            exit_codes: vec![42],
            signals: vec![],
        };
        // 42 is in SuccessExitStatus — on-failure should NOT restart
        assert!(!should_restart(
            &ServiceRestart::OnFailure,
            &ChildTermination::Exit(42),
            &ses,
            false,
        ));
    }

    #[test]
    fn test_should_restart_on_abnormal_signal_only() {
        let ses = SuccessExitStatus::default();
        // on-abnormal: unclean signal → restart
        assert!(should_restart(
            &ServiceRestart::OnAbnormal,
            &ChildTermination::Signal(Signal::SIGKILL),
            &ses,
            false,
        ));
        // on-abnormal: clean signal → no restart
        assert!(!should_restart(
            &ServiceRestart::OnAbnormal,
            &ChildTermination::Signal(Signal::SIGTERM),
            &ses,
            false,
        ));
        // on-abnormal: any exit code → no restart
        assert!(!should_restart(
            &ServiceRestart::OnAbnormal,
            &ChildTermination::Exit(1),
            &ses,
            false,
        ));
        assert!(!should_restart(
            &ServiceRestart::OnAbnormal,
            &ChildTermination::Exit(0),
            &ses,
            false,
        ));
    }

    #[test]
    fn test_should_restart_on_abort() {
        let ses = SuccessExitStatus::default();
        // on-abort: unclean signal → restart
        assert!(should_restart(
            &ServiceRestart::OnAbort,
            &ChildTermination::Signal(Signal::SIGKILL),
            &ses,
            false,
        ));
        // on-abort: clean signal → no restart
        assert!(!should_restart(
            &ServiceRestart::OnAbort,
            &ChildTermination::Signal(Signal::SIGTERM),
            &ses,
            false,
        ));
        // on-abort: exit code → no restart
        assert!(!should_restart(
            &ServiceRestart::OnAbort,
            &ChildTermination::Exit(1),
            &ses,
            false,
        ));
    }

    #[test]
    fn test_should_restart_on_watchdog_without_flag() {
        let ses = SuccessExitStatus::default();
        // on-watchdog should NOT restart when watchdog_fired is false
        assert!(!should_restart(
            &ServiceRestart::OnWatchdog,
            &ChildTermination::Exit(0),
            &ses,
            false,
        ));
        assert!(!should_restart(
            &ServiceRestart::OnWatchdog,
            &ChildTermination::Signal(Signal::SIGKILL),
            &ses,
            false,
        ));
    }

    #[test]
    fn test_should_restart_on_watchdog_with_flag() {
        let ses = SuccessExitStatus::default();
        // on-watchdog should restart when watchdog_fired is true
        assert!(should_restart(
            &ServiceRestart::OnWatchdog,
            &ChildTermination::Signal(Signal::SIGABRT),
            &ses,
            true,
        ));
        // Even with a clean exit code, watchdog flag triggers restart
        assert!(should_restart(
            &ServiceRestart::OnWatchdog,
            &ChildTermination::Exit(0),
            &ses,
            true,
        ));
    }

    #[test]
    fn test_should_restart_on_failure_with_watchdog() {
        let ses = SuccessExitStatus::default();
        // on-failure should restart when watchdog fires (watchdog counts as failure)
        assert!(should_restart(
            &ServiceRestart::OnFailure,
            &ChildTermination::Exit(0),
            &ses,
            true,
        ));
        // Even a clean exit with watchdog_fired is a failure
        assert!(should_restart(
            &ServiceRestart::OnFailure,
            &ChildTermination::Signal(Signal::SIGTERM),
            &ses,
            true,
        ));
    }

    #[test]
    fn test_should_restart_on_abnormal_with_watchdog() {
        let ses = SuccessExitStatus::default();
        // on-abnormal should restart when watchdog fires
        assert!(should_restart(
            &ServiceRestart::OnAbnormal,
            &ChildTermination::Exit(0),
            &ses,
            true,
        ));
        assert!(should_restart(
            &ServiceRestart::OnAbnormal,
            &ChildTermination::Signal(Signal::SIGABRT),
            &ses,
            true,
        ));
    }

    #[test]
    fn test_should_restart_on_abort_ignores_watchdog() {
        let ses = SuccessExitStatus::default();
        // on-abort does NOT restart on watchdog (only on unclean signals)
        // Clean exit + watchdog → no restart for on-abort
        assert!(!should_restart(
            &ServiceRestart::OnAbort,
            &ChildTermination::Exit(0),
            &ses,
            true,
        ));
        // Clean signal + watchdog → no restart for on-abort
        assert!(!should_restart(
            &ServiceRestart::OnAbort,
            &ChildTermination::Signal(Signal::SIGTERM),
            &ses,
            true,
        ));
        // Unclean signal + watchdog → restart (because of the signal, not watchdog)
        assert!(should_restart(
            &ServiceRestart::OnAbort,
            &ChildTermination::Signal(Signal::SIGABRT),
            &ses,
            true,
        ));
    }

    #[test]
    fn test_should_restart_on_success_ignores_watchdog() {
        let ses = SuccessExitStatus::default();
        // on-success does NOT restart on watchdog (only on clean exits)
        assert!(!should_restart(
            &ServiceRestart::OnSuccess,
            &ChildTermination::Exit(1),
            &ses,
            true,
        ));
        // Clean exit + watchdog → restart (because exit was clean)
        assert!(should_restart(
            &ServiceRestart::OnSuccess,
            &ChildTermination::Exit(0),
            &ses,
            true,
        ));
    }

    #[test]
    fn test_should_restart_no_ignores_watchdog() {
        let ses = SuccessExitStatus::default();
        // Restart=no never restarts, even with watchdog
        assert!(!should_restart(
            &ServiceRestart::No,
            &ChildTermination::Signal(Signal::SIGABRT),
            &ses,
            true,
        ));
    }

    #[test]
    fn test_should_restart_always_with_watchdog() {
        let ses = SuccessExitStatus::default();
        // Restart=always restarts with or without watchdog
        assert!(should_restart(
            &ServiceRestart::Always,
            &ChildTermination::Signal(Signal::SIGABRT),
            &ses,
            true,
        ));
    }

    #[test]
    fn test_is_clean_signal_value() {
        assert!(is_clean_signal_value(Signal::SIGHUP));
        assert!(is_clean_signal_value(Signal::SIGINT));
        assert!(is_clean_signal_value(Signal::SIGTERM));
        assert!(is_clean_signal_value(Signal::SIGPIPE));
        assert!(!is_clean_signal_value(Signal::SIGKILL));
        assert!(!is_clean_signal_value(Signal::SIGABRT));
        assert!(!is_clean_signal_value(Signal::SIGSEGV));
        assert!(!is_clean_signal_value(Signal::SIGUSR1));
    }

    #[test]
    fn test_should_restart_on_failure_custom_success_signal() {
        let ses = SuccessExitStatus {
            exit_codes: vec![],
            signals: vec![Signal::SIGUSR1],
        };
        // SIGUSR1 is in SuccessExitStatus — on-failure should NOT restart
        assert!(!should_restart(
            &ServiceRestart::OnFailure,
            &ChildTermination::Signal(Signal::SIGUSR1),
            &ses,
            false,
        ));
        // SIGUSR2 is not — on-failure should restart
        assert!(should_restart(
            &ServiceRestart::OnFailure,
            &ChildTermination::Signal(Signal::SIGUSR2),
            &ses,
            false,
        ));
    }

    #[test]
    fn test_graduated_delay_incremental_ramp() {
        // Verify that each step increases the delay
        let base = Some(Timeout::Duration(Duration::from_secs(1)));
        let max = Some(Timeout::Duration(Duration::from_secs(11)));
        let steps = 5;

        let mut prev_ms = 0u128;
        for i in 0..=5 {
            let result = compute_graduated_restart_delay(&base, &max, steps, i);
            if let Some(Timeout::Duration(d)) = result {
                let ms = d.as_millis();
                assert!(ms >= prev_ms, "step {i}: {ms}ms should be >= {prev_ms}ms");
                prev_ms = ms;
            } else {
                panic!("Expected Duration at step {i}");
            }
        }
        // Final step should be exactly max
        assert_eq!(prev_ms, 11_000);
    }
}
