use log::{error, info, trace, warn};

use crate::lock_ext::RwLockExt;
use crate::runtime_info::{ArcMutRuntimeInfo, RuntimeInfo};
use crate::signal_handler::ChildTermination;
use crate::units::{
    ServiceRestart, ServiceType, Specific, SuccessExitStatus, Timeout, UnitAction, UnitId,
    UnitOperationErrorReason, UnitStatus,
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

/// Trigger OnSuccess= or OnFailure= units for a service that has reached
/// its final state. This resolves unit names from the config (loading from
/// disk if necessary), resets them to NeverStarted, and activates them
/// asynchronously in a new thread to avoid blocking the exit handler.
fn trigger_on_success_failure_units(
    unit_names: &[String],
    source_name: &str,
    kind: &str,
    _run_info: &RuntimeInfo,
    arc_run_info: &ArcMutRuntimeInfo,
) {
    for target_name in unit_names {
        trace!("Triggering {kind}={target_name} for unit {source_name}");
        // Activate asynchronously so we don't block restart logic.
        let arc_ri = arc_run_info.clone();
        let target_name_owned = target_name.clone();
        let source_name_owned = source_name.to_string();
        let kind_owned = kind.to_string();
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
            // Reset the target unit from Stopped → NeverStarted so it can be activated.
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
                    info!(
                        "{kind_owned} unit {target_name_owned} activated for {source_name_owned}"
                    );
                }
                Err(e) => {
                    warn!(
                        "Failed to activate {kind_owned} unit {target_name_owned} for {source_name_owned}: {e}"
                    );
                }
            }
        });
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
        if let Err(e) =
            service_exit_handler(pid, srvc_id, code, &run_info.read_poisoned(), &run_info)
        {
            error!("{e}");
        }
    });
}

/// Handle the aftermath of a service process exiting.
///
/// The PID table has already been updated by the signal handler; this function
/// deals with utmp records, oneshot process cleanup, restart decisions, and
/// SuccessAction/FailureAction triggers.
pub fn service_exit_handler(
    pid: nix::unistd::Pid,
    srvc_id: UnitId,
    code: ChildTermination,
    run_info: &RuntimeInfo,
    arc_run_info: &ArcMutRuntimeInfo,
) -> Result<(), String> {
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
    }

    let success_exit_status = get_success_exit_status(unit);

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
                return Ok(());
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

                // Increment NRestarts counter
                {
                    let mut state = srvc.state.write_poisoned();
                    state.common.restart_count += 1;
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
                                        return Ok(());
                                    }
                                }
                                std::thread::sleep(std::time::Duration::from_millis(100));
                            }
                        }
                        Timeout::Infinity => {
                            trace!(
                                "RestartSec=infinity for oneshot service {name}, not restarting"
                            );
                            return Ok(());
                        }
                    }
                }

                // Recheck — may have been shortcut-started
                if let Some(unit) = run_info.unit_table.get(&srvc_id) {
                    let st = unit.common.status.read_poisoned();
                    if !matches!(&*st, crate::units::UnitStatus::Restarting) {
                        return Ok(());
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
                    return Ok(());
                }

                info!("Restarting oneshot service {name}");
                crate::units::reactivate_unit(srvc_id, run_info).map_err(|e| format!("{e}"))?;
                return Ok(());
            }

            crate::units::deactivate_unit_recursive(&srvc_id, run_info)
                .map_err(|e| format!("{e}"))?;

            // Mark as failed if the exit was not clean.
            let is_success = success_exit_status.is_success(&code);
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

            // Trigger OnSuccess=/OnFailure= units for the oneshot service.
            if let Some(unit) = run_info.unit_table.get(&srvc_id) {
                if is_success && !unit.common.unit.on_success.is_empty() {
                    let targets = unit.common.unit.on_success.clone();
                    trigger_on_success_failure_units(
                        &targets,
                        name,
                        "OnSuccess",
                        run_info,
                        arc_run_info,
                    );
                } else if !is_success && !unit.common.unit.on_failure.is_empty() {
                    let targets = unit.common.unit.on_failure.clone();
                    trigger_on_success_failure_units(
                        &targets,
                        name,
                        "OnFailure",
                        run_info,
                        arc_run_info,
                    );
                }
            }
            return Ok(());
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
    let (restart_unit, restart_sec) = {
        if let Specific::Service(srvc) = &unit.specific {
            trace!(
                "Service with id: {:?}, name: {} pid: {} exited with: {:?}",
                srvc_id, unit.id.name, pid, code
            );

            // Check whether the watchdog enforcement thread killed this service.
            let watchdog_fired = {
                let state = srvc.state.read_poisoned();
                state.srvc.watchdog_timeout_fired
            };
            if watchdog_fired {
                trace!(
                    "Service {}: exit was caused by watchdog timeout",
                    unit.id.name
                );
            }

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
                (false, srvc.conf.restart_sec.clone())
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
                        watchdog_fired,
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

                (do_restart, restart_sec)
            }
        } else {
            (false, None)
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
            return Ok(());
        }
    }

    // RemainAfterExit=yes: if the process exited cleanly, keep the service
    // in its current active state — do not restart, do not deactivate.
    if has_remain_after_exit(unit) && success_exit_status.is_success(&code) {
        trace!("Service {name} exited cleanly with RemainAfterExit=yes, staying active");
        return Ok(());
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
        // Trigger OnSuccess=/OnFailure= even when restarting — systemd fires
        // these on the state transition independent of restart.
        // A clean signal (SIGHUP/SIGINT/SIGTERM/SIGPIPE) or explicit success
        // triggers OnSuccess=; everything else triggers OnFailure=.
        if let Some(unit) = run_info.unit_table.get(&srvc_id) {
            let is_clean =
                success_exit_status.is_success(&code) || success_exit_status.is_clean_signal(&code);
            if is_clean && !unit.common.unit.on_success.is_empty() {
                let targets = unit.common.unit.on_success.clone();
                trigger_on_success_failure_units(
                    &targets,
                    name,
                    "OnSuccess",
                    run_info,
                    arc_run_info,
                );
            } else if !is_clean && !unit.common.unit.on_failure.is_empty() {
                let targets = unit.common.unit.on_failure.clone();
                trigger_on_success_failure_units(
                    &targets,
                    name,
                    "OnFailure",
                    run_info,
                    arc_run_info,
                );
            }
        }

        // Mark the unit as Restarting so that SubState shows "auto-restart"
        // and `systemctl start` can shortcut the pending restart.
        if let Some(unit) = run_info.unit_table.get(&srvc_id) {
            let mut status = unit.common.status.write_poisoned();
            *status = crate::units::UnitStatus::Restarting;
        }

        // Increment NRestarts counter.
        if let Some(unit) = run_info.unit_table.get(&srvc_id)
            && let Specific::Service(svc) = &unit.specific
        {
            let mut state = svc.state.write_poisoned();
            state.common.restart_count += 1;
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
                                return Ok(());
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
                Timeout::Infinity => {
                    trace!("RestartSec=infinity for service {name}, not restarting");
                    return Ok(());
                }
            }
        }

        // Recheck that the unit is still in Restarting state (not shortcut).
        if let Some(unit) = run_info.unit_table.get(&srvc_id) {
            let st = unit.common.status.read_poisoned();
            if !matches!(&*st, crate::units::UnitStatus::Restarting) {
                return Ok(());
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
            return Ok(());
        }
        trace!("Restart service {name} after it died");
        crate::units::reactivate_unit(srvc_id, run_info).map_err(|e| format!("{e}"))?;
    } else {
        // Capture whether the unit was intentionally being stopped *before*
        // deactivation — deactivation sets the status to Stopped, so checking
        // afterwards would always see Stopped and never mark the unit as failed.
        let was_intentionally_stopping = {
            if let Some(unit) = run_info.unit_table.get(&srvc_id) {
                let current = unit.common.status.read_poisoned();
                matches!(&*current, UnitStatus::Stopping)
            } else {
                false
            }
        };

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

        // If the exit was not clean (non-zero exit code or killed by signal),
        // override the status to "failed" so `systemctl show` reports
        // ActiveState=failed/SubState=failed.
        // However, if the unit was being intentionally stopped (status was Stopping
        // before deactivation), don't mark it as failed — signal death from an
        // intentional stop is expected behavior.
        let is_success = success_exit_status.is_success(&code);
        if !is_success
            && !was_intentionally_stopping
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

        // Trigger OnSuccess=/OnFailure= units.
        // A "clean" exit for OnSuccess purposes includes exit code 0, clean signals
        // (SIGHUP/SIGINT/SIGTERM/SIGPIPE), intentional stops, and any extra signals
        // in SuccessExitStatus=.
        if let Some(unit) = run_info.unit_table.get(&srvc_id) {
            let effective_success = is_success
                || was_intentionally_stopping
                || success_exit_status.is_clean_signal(&code);
            if effective_success && !unit.common.unit.on_success.is_empty() {
                let targets = unit.common.unit.on_success.clone();
                trigger_on_success_failure_units(
                    &targets,
                    name,
                    "OnSuccess",
                    run_info,
                    arc_run_info,
                );
            } else if !effective_success && !unit.common.unit.on_failure.is_empty() {
                let targets = unit.common.unit.on_failure.clone();
                trigger_on_success_failure_units(
                    &targets,
                    name,
                    "OnFailure",
                    run_info,
                    arc_run_info,
                );
            }
        }
    }
    Ok(())
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
