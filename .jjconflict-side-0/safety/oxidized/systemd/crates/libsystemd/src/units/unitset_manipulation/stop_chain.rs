//! Dispatcher stop chains (docs/EVENT-LOOP.md inc 3): the per-unit half of
//! stops-as-continuations. Mirrors `Service::kill`'s sequence
//! (ExecStop= commands, then SIGTERM with the TimeoutStopSec window, then
//! ExecStopPost=, then the SIGKILL sweep and final bookkeeping) as phases
//! advanced by ChildExit events on the dispatcher, so nothing waits for a
//! process while holding the table read guard or a state write lock.
//!
//! The dependency-ordered scheduling across units (dependents stop before
//! the unit, PropagatesStopTo= afterwards) lives in the dispatcher's stop
//! graph; this module owns one unit's chain. Wired by the control Stop
//! routing; without a live dispatcher the inline path in
//! `deactivate_unit_recursive` remains the only stop path.
use log::{trace, warn};

use crate::lock_ext::{MutexExt, RwLockExt};
use crate::runtime_info::ArcMutRuntimeInfo;
use crate::units::{Specific, UnitId, UnitStatus};

use super::activate::dispatcher_read;

pub struct ServiceStopChain {
    pub id: UnitId,
    pub phase: StopPhase,
    /// Errors collected across the phases; they become the final
    /// Stopped status errors, preserving kill()'s combined semantics.
    pub errors: Vec<String>,
    /// A control-stop job to finish when this unit's chain (and its
    /// transaction, for the root) completes; None for scheduler-internal
    /// chains.
    pub job: Option<crate::units::jobs::JobId>,
}

pub enum StopPhase {
    /// Running ExecStop= command `idx`.
    ExecStop(usize),
    /// SIGTERM initiated (or nothing left to signal); waiting for the main
    /// process's exit event.
    AwaitMainExit,
    /// Running ExecStopPost= command `idx`.
    Poststop(usize),
}

pub enum StopStepOutcome {
    /// A stop/poststop helper was forked; park on this pid, keeping the
    /// Child for the output drain.
    ForkedHelper {
        pid: nix::unistd::Pid,
        child: std::process::Child,
        cmd: crate::units::Commandline,
        timeout: Option<std::time::Duration>,
    },
    /// SIGTERM sent; park on the main pid with the TimeoutStopSec deadline.
    AwaitingMain {
        pid: nix::unistd::Pid,
        timeout: Option<std::time::Duration>,
    },
    /// The unit is finalized; the stop graph should credit its parents.
    Finished,
}

/// Begin or advance a unit's stop chain. Never waits: each call takes
/// brief guards, forks at most one helper or sends at most one signal, and
/// returns what to park on.
pub(crate) fn service_stop_chain_step(
    chain: &mut ServiceStopChain,
    run_info: &ArcMutRuntimeInfo,
) -> StopStepOutcome {
    enum Action {
        ForkHelper(crate::units::Commandline),
        InitiateTerm,
        NextPhase(StopPhase),
        Finalize,
        NonService,
    }
    loop {
        let action = {
            let ri = dispatcher_read(run_info);
            let Some(unit) = ri.unit_table.get(&chain.id) else {
                return StopStepOutcome::Finished;
            };
            match &unit.specific {
                Specific::Service(svc) => {
                    // Masked units skip ExecStop/ExecStopPost entirely
                    // (systemd issue #38802); the finalize's kill sweep and
                    // cleanup are all that runs.
                    if crate::services::is_unit_masked(&chain.id.name)
                        && !matches!(chain.phase, StopPhase::Poststop(_))
                    {
                        Action::Finalize
                    } else {
                        match &chain.phase {
                            StopPhase::ExecStop(idx) => {
                                if *idx >= svc.conf.stop.len() {
                                    Action::NextPhase(StopPhase::AwaitMainExit)
                                } else {
                                    Action::ForkHelper(svc.conf.stop[*idx].clone())
                                }
                            }
                            StopPhase::AwaitMainExit => Action::InitiateTerm,
                            StopPhase::Poststop(idx) => {
                                if *idx >= svc.conf.stoppost.len() {
                                    Action::Finalize
                                } else {
                                    Action::ForkHelper(svc.conf.stoppost[*idx].clone())
                                }
                            }
                        }
                    }
                }
                _ => Action::NonService,
            }
        };
        match action {
            Action::NonService => {
                // Non-service units keep the existing inline deactivation,
                // which is fast for every type the scheduler feeds here
                // (target/slice/device/socket/timer/path); mounts and swaps
                // are excluded from the stop graph until inc 5.
                let ri = dispatcher_read(run_info);
                if let Err(e) = crate::units::deactivate_unit(&chain.id, &ri) {
                    chain.errors.push(format!("{e}"));
                }
                return StopStepOutcome::Finished;
            }
            Action::NextPhase(phase) => {
                chain.phase = phase;
            }
            Action::ForkHelper(cmd) => {
                let name = chain.id.name.clone();
                let forked = {
                    let ri = dispatcher_read(run_info);
                    let Some(unit) = ri.unit_table.get(&chain.id) else {
                        return StopStepOutcome::Finished;
                    };
                    let Specific::Service(svc) = &unit.specific else {
                        return StopStepOutcome::Finished;
                    };
                    let working_dir = svc.conf.exec_config.working_directory.clone();
                    let mut st = svc.state.write_poisoned();
                    let timeout = st.srvc.get_stop_timeout(&svc.conf);
                    // Stop-side helpers see the service's filesystem view,
                    // like run_stop_cmd/run_poststop set up.
                    st.srvc.helper_mount_ns = crate::services::helper_mount_ns_from_conf(&svc.conf);
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
                        return StopStepOutcome::ForkedHelper {
                            pid: nix::unistd::Pid::from_raw(child.id() as i32),
                            child,
                            cmd,
                            timeout,
                        };
                    }
                    Err(e) => {
                        if cmd
                            .prefixes
                            .contains(&crate::units::CommandlinePrefix::Minus)
                        {
                            advance_stop_index(chain);
                        } else {
                            chain.errors.push(format!("Failed to spawn {cmd}: {e}"));
                            skip_current_stop_phase(chain);
                        }
                    }
                }
            }
            Action::InitiateTerm => {
                // Mirror kill(): mark the deliberate stop so the exit
                // handler suppresses Restart=, transition to Stopping, then
                // send SIGTERM per KillMode without any wait; the main
                // process's exit event advances the chain and the parked
                // deadline is the TimeoutStopSec window.
                let ri = dispatcher_read(run_info);
                let Some(unit) = ri.unit_table.get(&chain.id) else {
                    return StopStepOutcome::Finished;
                };
                let Specific::Service(svc) = &unit.specific else {
                    return StopStepOutcome::Finished;
                };
                {
                    let mut status = unit.common.status.write_poisoned();
                    if !matches!(&*status, UnitStatus::Stopped(..)) {
                        *status = UnitStatus::Stopping;
                    }
                }
                unit.common.timestamps.write_poisoned().record_active_exit();
                let (target, main_pid, timeout) = {
                    let mut st = svc.state.write_poisoned();
                    st.srvc.manual_stop = true;
                    let timeout = st.srvc.get_stop_timeout(&svc.conf);
                    if matches!(svc.conf.kill_mode, crate::units::KillMode::None) {
                        (None, None, timeout)
                    } else {
                        // Same pid resolution as terminate_gracefully: the
                        // sd_notify MAINPID overrides the forked pid, with
                        // the live PID table as the fallback.
                        let pid = st.srvc.main_pid.or(st.srvc.pid).or_else(|| {
                            let pt = ri.pid_table.lock_poisoned();
                            pt.iter().find_map(|(p, entry)| match entry {
                                crate::runtime_info::PidEntry::Service(uid, _)
                                    if uid.name == chain.id.name =>
                                {
                                    Some(*p)
                                }
                                _ => None,
                            })
                        });
                        let alive = pid.is_some_and(|p| {
                            let pt = ri.pid_table.lock_poisoned();
                            matches!(pt.get(&p), Some(crate::runtime_info::PidEntry::Service(..)))
                        });
                        if !alive {
                            (None, None, timeout)
                        } else {
                            let target = match svc.conf.kill_mode {
                                crate::units::KillMode::ControlGroup => {
                                    st.srvc.process_group.or(pid)
                                }
                                _ => pid,
                            };
                            (target, pid, timeout)
                        }
                    }
                };
                match (target, main_pid) {
                    (Some(target), Some(main_pid)) => {
                        match nix::sys::signal::kill(target, nix::sys::signal::Signal::SIGTERM) {
                            Ok(()) => {
                                trace!(
                                    "stop chain: sent SIGTERM to {} (target {target})",
                                    chain.id.name
                                );
                                return StopStepOutcome::AwaitingMain {
                                    pid: main_pid,
                                    timeout,
                                };
                            }
                            Err(nix::errno::Errno::ESRCH) => {
                                chain.phase = StopPhase::Poststop(0);
                            }
                            Err(e) => {
                                warn!("stop chain: SIGTERM to {} failed: {e}", chain.id.name);
                                chain.phase = StopPhase::Poststop(0);
                            }
                        }
                    }
                    _ => {
                        // Nothing alive to signal; straight to poststop.
                        chain.phase = StopPhase::Poststop(0);
                    }
                }
            }
            Action::Finalize => {
                finalize_stop_chain(chain, run_info);
                return StopStepOutcome::Finished;
            }
        }
    }
}

/// Consume a stop helper's exit: drain its output, apply run_all_cmds
/// semantics (a leading '-' swallows failures, anything else records the
/// error and skips the rest of the phase), advance.
pub(crate) fn service_stop_chain_helper_exited(
    chain: &mut ServiceStopChain,
    cmd: &crate::units::Commandline,
    mut child: std::process::Child,
    termination: crate::signal_handler::ChildTermination,
    run_info: &ArcMutRuntimeInfo,
) -> StopStepOutcome {
    {
        let ri = dispatcher_read(run_info);
        if let Some(unit) = ri.unit_table.get(&chain.id)
            && let Specific::Service(svc) = &unit.specific
        {
            let status = unit.common.status.read_poisoned().clone();
            svc.state
                .write_poisoned()
                .srvc
                .drain_helper_output_nonblocking(&mut child, &chain.id.name, &status);
        }
    }
    drop(child);
    let ok = termination.success()
        || cmd
            .prefixes
            .contains(&crate::units::CommandlinePrefix::Minus);
    if ok {
        advance_stop_index(chain);
    } else {
        chain
            .errors
            .push(format!("{cmd} exited with {termination:?}"));
        skip_current_stop_phase(chain);
    }
    service_stop_chain_step(chain, run_info)
}

/// The main process did not exit within the TimeoutStopSec window: SIGKILL
/// it (kmsg first, a stop timeout is what wedge investigations look for)
/// and move on to poststop; the finalize sweep clears any survivors.
pub(crate) fn service_stop_chain_main_timeout(
    chain: &mut ServiceStopChain,
    pid: nix::unistd::Pid,
    run_info: &ArcMutRuntimeInfo,
) -> StopStepOutcome {
    crate::entrypoints::kmsg(&format!(
        "STOP-TIMEOUT {} did not exit after SIGTERM; sending SIGKILL",
        chain.id.name
    ));
    unsafe {
        libc::kill(pid.as_raw(), libc::SIGKILL);
    }
    chain.errors.push("stop timed out".to_owned());
    chain.phase = StopPhase::Poststop(0);
    service_stop_chain_step(chain, run_info)
}

/// A stop helper overran its window and was SIGKILLed by the wheel: skip
/// the rest of its phase (the error was recorded by the caller) and
/// advance.
pub(crate) fn service_stop_chain_helper_timeout_skip(
    chain: &mut ServiceStopChain,
    run_info: &ArcMutRuntimeInfo,
) -> StopStepOutcome {
    skip_current_stop_phase(chain);
    service_stop_chain_step(chain, run_info)
}

/// The main process exited while the chain awaited it: advance to poststop.
pub(crate) fn service_stop_chain_main_exited(
    chain: &mut ServiceStopChain,
    run_info: &ArcMutRuntimeInfo,
) -> StopStepOutcome {
    chain.phase = StopPhase::Poststop(0);
    service_stop_chain_step(chain, run_info)
}

fn advance_stop_index(chain: &mut ServiceStopChain) {
    chain.phase = match &chain.phase {
        StopPhase::ExecStop(idx) => StopPhase::ExecStop(idx + 1),
        StopPhase::AwaitMainExit => StopPhase::AwaitMainExit,
        StopPhase::Poststop(idx) => StopPhase::Poststop(idx + 1),
    };
}

/// A non-minus helper failure skips the remainder of its phase, mirroring
/// run_all_cmds aborting on the first error while kill() still proceeds to
/// the following stage.
fn skip_current_stop_phase(chain: &mut ServiceStopChain) {
    chain.phase = match &chain.phase {
        StopPhase::ExecStop(_) => StopPhase::AwaitMainExit,
        StopPhase::AwaitMainExit => StopPhase::Poststop(0),
        StopPhase::Poststop(_) => StopPhase::Poststop(usize::MAX),
    };
}

/// The kill() and deactivate() tails: SIGKILL sweep, pid and fd cleanup,
/// final status with the collected errors, timestamps, cgroup prune and
/// socket re-arm.
fn finalize_stop_chain(chain: &ServiceStopChain, run_info: &ArcMutRuntimeInfo) {
    let ri = dispatcher_read(run_info);
    let Some(unit) = ri.unit_table.get(&chain.id) else {
        return;
    };
    if let Specific::Service(svc) = &unit.specific {
        let conf = &svc.conf;
        {
            let mut st = svc.state.write_poisoned();
            st.srvc.kill_all_remaining_processes(conf, &chain.id.name);
            st.srvc.pid = None;
            st.srvc.process_group = None;
            if let Some(path) = st.srvc.notifications_path.take() {
                let _ = std::fs::remove_file(&path);
            }
            st.srvc.notifications = None;
            st.srvc.stdout = None;
            st.srvc.stderr = None;
        }
        {
            let mut status = unit.common.status.write_poisoned();
            let errors = chain
                .errors
                .iter()
                .map(|e| crate::units::UnitOperationErrorReason::GenericStartError(e.clone()))
                .collect();
            *status = UnitStatus::Stopped(crate::units::StatusStopped::StoppedFinal, errors);
        }
        // RemoveIPC=yes: remove SysV/POSIX IPC owned by the service's (static)
        // User= now that its processes are gone. Skipped for root and for
        // DynamicUser= (whose allocated UID is not stored yet). Mirrors systemd's
        // clean_ipc_by_uid on the stop path.
        if conf.exec_config.remove_ipc {
            // Prefer the DynamicUser= UID recorded at start; fall back to a static
            // User= (re-resolved). Never clean root's IPC.
            let uid = svc.state.read_poisoned().srvc.dynamic_uid.or_else(|| {
                crate::services::start_service::resolve_uid(&conf.exec_config.user).ok()
            });
            if let Some(uid) = uid
                && uid != 0
            {
                crate::services::clean_ipc::clean_ipc_by_uid(uid);
            }
        }
        // Clean up RuntimeDirectory= dirs unless RuntimeDirectoryPreserve=yes.
        // The inline deactivate() path (unit.rs) does this, but the dispatcher
        // stop chain that in-VM `systemctl stop` actually drives did not, so
        // /run/<name> leaked across every stop (e.g. RemainAfterExit oneshots).
        if conf.exec_config.runtime_directory_preserve
            != crate::units::RuntimeDirectoryPreserve::Yes
        {
            for dir_name in &conf.exec_config.runtime_directory {
                let full_path = std::path::Path::new("/run").join(dir_name);
                if full_path.exists()
                    && let Err(e) = std::fs::remove_dir_all(&full_path)
                {
                    trace!(
                        "stop chain: failed to remove runtime directory {:?} for {}: {}",
                        full_path, chain.id.name, e
                    );
                }
            }
        }
        #[cfg(target_os = "linux")]
        {
            let cgroup_path = &conf.platform_specific.cgroup_path;
            if cgroup_path.exists() {
                let mut removed = true;
                if let Err(e) = crate::platform::cgroups::remove_cgroup_recursive(cgroup_path) {
                    removed = false;
                    if std::env::var_os("SYSTEMD_USER_MANAGER").is_some() {
                        match crate::control::escalate_remove_cgroup(cgroup_path) {
                            Ok(()) => removed = true,
                            Err(e2) => log::warn!(
                                "stop chain: escalated cgroup removal for {} failed: {e2} (local: {e})",
                                chain.id.name,
                            ),
                        }
                    } else {
                        log::warn!(
                            "stop chain: could not remove cgroup {} for {}: {e}",
                            cgroup_path.display(),
                            chain.id.name,
                        );
                    }
                }
                if removed {
                    crate::platform::cgroups::prune_empty_parent_cgroups(
                        cgroup_path,
                        std::path::Path::new("/sys/fs/cgroup"),
                    );
                }
            }
        }
        // Re-arm associated sockets so socket activation can restart the
        // service on new traffic, mirroring the deactivate tail.
        if !conf.sockets.is_empty() {
            for socket_id in &conf.sockets {
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
}
