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
        if let Err(e) = service_exit_handler(pid, srvc_id, code, &run_info.read_poisoned()) {
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
) -> Result<(), String> {
    trace!(
        "Exit handler for service {:?} with pid: {pid}",
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

    // kill oneshot service processes. There should be none but just in case...
    {
        if let Specific::Service(srvc) = &unit.specific
            && srvc.conf.srcv_type == ServiceType::OneShot
        {
            let mut_state = &mut *srvc.state.write_poisoned();
            mut_state
                .srvc
                .kill_all_remaining_processes(&srvc.conf, &unit.id.name);

            // RemainAfterExit=yes: keep the unit in Started status after a
            // clean exit, matching systemd's behaviour for oneshot services
            // that perform setup tasks.
            if srvc.conf.remain_after_exit && success_exit_status.is_success(&code) {
                trace!(
                    "Oneshot service {} exited cleanly with RemainAfterExit=yes, staying active",
                    unit.id.name
                );
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
        if !(status_locked.is_started() || *status_locked == UnitStatus::Starting) {
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

    if restart_unit {
        if let Some(ref timeout) = restart_sec {
            match timeout {
                Timeout::Duration(dur) => {
                    trace!(
                        "Waiting {:?} (RestartSec) before restarting service {name}",
                        dur
                    );
                    std::thread::sleep(*dur);
                }
                Timeout::Infinity => {
                    trace!("RestartSec=infinity for service {name}, not restarting");
                    return Ok(());
                }
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
        if !success_exit_status.is_success(&code)
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
