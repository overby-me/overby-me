use log::error;

use log::trace;
use log::warn;

use crate::lock_ext::MutexExt;
use crate::runtime_info::{PidEntry, RuntimeInfo};
use crate::services::RunCmdError;
use crate::services::Service;
use crate::units::ServiceConfig;
use crate::units::{CommandlinePrefix, ServiceType};

/// Read a PID from a PIDFile path, retrying a few times to allow the forking
/// daemon a moment to write the file.
fn read_pid_file(path: &std::path::Path) -> Option<nix::unistd::Pid> {
    // The daemon may not have written the PIDFile yet at the instant
    // the parent exits, so retry with a short back-off.
    for attempt in 0..20 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if let Ok(contents) = std::fs::read_to_string(path)
            && let Ok(pid) = contents.trim().parse::<i32>()
            && pid > 0
        {
            return Some(nix::unistd::Pid::from_raw(pid));
        }
    }
    None
}

pub fn wait_for_service(
    srvc: &mut Service,
    conf: &ServiceConfig,
    name: &str,
    run_info: &RuntimeInfo,
) -> Result<(), RunCmdError> {
    let pid_table = &run_info.pid_table;
    trace!(
        "[FORK_PARENT] Service: {} forked with pid: {}",
        name,
        srvc.pid.unwrap()
    );

    let start_time = std::time::Instant::now();
    let duration_timeout = srvc.get_start_timeout(conf);
    match conf.srcv_type {
        ServiceType::Notify | ServiceType::NotifyReload => {
            trace!(
                "[FORK_PARENT] Waiting for a notification for service {name} (timeout={duration_timeout:?})"
            );

            //let duration_timeout = Some(std::time::Duration::from_nanos(1_000_000_000_000));
            let mut buf = [0u8; 512];
            loop {
                let Some(stream) = &srvc.notifications else {
                    return Err(RunCmdError::Generic(
                        "No notification socket but is required".into(),
                    ));
                };

                {
                    let mut pid_table_locked = pid_table.lock_poisoned();
                    if let Some(PidEntry::ServiceExited(_)) =
                        pid_table_locked.get(&srvc.pid.unwrap())
                    {
                        trace!(
                            "The service {name} has exited before sending a READY=1 notification"
                        );
                        let pid_entry = pid_table_locked.remove(&srvc.pid.unwrap());
                        if let Some(PidEntry::ServiceExited(code)) = pid_entry {
                            return Err(RunCmdError::ExitBeforeNotify(name.to_owned(), code));
                        }
                    }
                }

                if let Some(duration_timeout) = duration_timeout {
                    // Check EXTEND_TIMEOUT_USEC: if the service has sent an
                    // extension, measure timeout from the extension timestamp
                    // using the extension duration instead of the original.
                    let timed_out = if let (Some(ext_usec), Some(ext_ts)) =
                        (srvc.extend_timeout_usec, srvc.extend_timeout_timestamp)
                    {
                        let ext_dur = std::time::Duration::from_micros(ext_usec);
                        ext_ts.elapsed() > ext_dur
                    } else {
                        start_time.elapsed() > duration_timeout
                    };
                    if timed_out {
                        trace!("[FORK_PARENT] Service {name} notification timed out");
                        return Err(RunCmdError::Timeout(
                            conf.exec
                                .last()
                                .map(|e| e.to_string())
                                .unwrap_or_else(|| "(no exec)".to_owned()),
                            format!("{duration_timeout:?}"),
                        ));
                    }
                    let duration_elapsed = if srvc.extend_timeout_timestamp.is_some() {
                        srvc.extend_timeout_timestamp.unwrap().elapsed()
                    } else {
                        start_time.elapsed()
                    };
                    let effective_timeout = if let Some(ext_usec) = srvc.extend_timeout_usec {
                        std::time::Duration::from_micros(ext_usec)
                    } else {
                        duration_timeout
                    };
                    let duration_till_timeout = effective_timeout.saturating_sub(duration_elapsed);
                    // Cap the recv timeout to 1 second so we frequently re-check
                    // the pid table for early process exits. Without this cap,
                    // a service that crashes immediately after fork still blocks
                    // this thread for the full TimeoutStartSec (default 90s),
                    // which can exhaust the activation thread pool and stall boot.
                    let recv_timeout = duration_till_timeout.min(std::time::Duration::from_secs(1));
                    stream.set_read_timeout(Some(recv_timeout)).unwrap();
                }
                let bytes = match stream.recv(&mut buf[..]) {
                    Ok(bytes) => {
                        if bytes > 0 {
                            let received =
                                core::str::from_utf8(&buf[..bytes]).unwrap_or("<non-utf8>");
                            trace!(
                                "[FORK_PARENT] Service {name}: received {bytes} bytes on notify socket: {:?}",
                                received
                            );
                        }
                        bytes
                    }
                    Err(e) => match e.kind() {
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted => {
                            trace!(
                                "[FORK_PARENT] Service {name}: recv returned {:?}, elapsed={:?}",
                                e.kind(),
                                start_time.elapsed()
                            );
                            0
                        }
                        _ => {
                            error!("[FORK_PARENT] Service {name}: recv error: {e}");
                            panic!("{}", e);
                        }
                    },
                };
                let received_str = core::str::from_utf8(&buf[..bytes]).unwrap();
                srvc.notifications_buffer.push_str(received_str);
                // Each recv() returns a complete datagram from sd_notify.
                // Datagrams use '\n' to separate key=value pairs internally,
                // but may not end with '\n'.  If we don't add a separator,
                // the last key=value of one datagram merges with the first
                // key=value of the next (e.g. "FDNAME=inotifyREADY=1"),
                // causing READY=1 to never be parsed.  Ensure there is
                // always a '\n' boundary between datagrams.
                if bytes > 0 && !received_str.ends_with('\n') {
                    srvc.notifications_buffer.push('\n');
                }
                if !srvc.notifications_buffer.is_empty() {
                    trace!(
                        "[FORK_PARENT] Service {name}: notification buffer before parse: {:?}",
                        srvc.notifications_buffer
                    );
                }
                crate::notification_handler::handle_notifications_from_buffer(srvc, name);
                if srvc.signaled_ready {
                    trace!("[FORK_PARENT] Service {name} sent READY=1 notification — proceeding!");
                    break;
                }
                trace!(
                    "[FORK_PARENT] Service {name} still not ready (elapsed={:?})",
                    start_time.elapsed()
                );
            }
            if let Some(stream) = &srvc.notifications {
                stream.set_read_timeout(None).unwrap();
            }
        }
        ServiceType::Simple | ServiceType::Idle => {
            trace!("[FORK_PARENT] service {name} doesnt notify");
        }
        ServiceType::Exec => {
            // Type=exec is like Simple, but we briefly verify that the
            // process actually started (i.e. the exec() call succeeded)
            // by checking that it hasn't already exited with an error.
            trace!("[FORK_PARENT] Waiting for exec confirmation for service {name}");
            let pid = srvc.pid.unwrap();
            // Poll the pid table for up to 500ms to catch exec() failures.
            // The exec helper exits with code 203 when execv() fails
            // (binary not found, permission denied, etc.). If the process
            // is still running after this window, exec() succeeded.
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
            loop {
                std::thread::sleep(std::time::Duration::from_millis(10));
                {
                    let mut pid_table_locked = pid_table.lock_poisoned();
                    let entry = pid_table_locked.get(&pid);
                    trace!(
                        "[FORK_PARENT] exec check for {name}: pid={pid}, entry={entry:?}, elapsed={:?}",
                        std::time::Instant::now()
                            .duration_since(deadline - std::time::Duration::from_millis(500))
                    );
                    if let Some(PidEntry::ServiceExited(
                        crate::signal_handler::ChildTermination::Exit(203),
                    )) = entry
                    {
                        let code = crate::signal_handler::ChildTermination::Exit(203);
                        pid_table_locked.remove(&pid);
                        return Err(RunCmdError::BadExitCode(
                            conf.exec
                                .last()
                                .map(|e| e.to_string())
                                .unwrap_or_else(|| "(no exec)".to_owned()),
                            code,
                        ));
                    }
                    // Process exited with a non-203 code — exec() succeeded
                    // but the program itself exited. Not a start failure.
                    if matches!(entry, Some(PidEntry::ServiceExited(_))) {
                        break;
                    }
                }
                if std::time::Instant::now() >= deadline {
                    trace!("[FORK_PARENT] exec check timed out for service {name}");
                    break;
                }
            }
            trace!("[FORK_PARENT] exec check done for service {name}");
        }
        ServiceType::OneShot => {
            trace!("[FORK_PARENT] Waiting for oneshot service to exit: {name}");
            let mut counter = 1u64;
            let pid = srvc.pid.unwrap();
            loop {
                if let Some(time_out) = duration_timeout
                    && start_time.elapsed() >= time_out
                {
                    error!("oneshot service {name} reached timeout");
                    return Err(RunCmdError::Timeout(
                        conf.exec
                            .last()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "(no exec)".to_owned()),
                        format!("{duration_timeout:?}"),
                    ));
                }
                {
                    let mut pid_table_locked = pid_table.lock_poisoned();
                    match pid_table_locked.get(&pid) {
                        Some(entry) => {
                            match entry {
                                PidEntry::Service(_, _) => {
                                    // Still running. Wait more
                                }
                                PidEntry::ServiceExited(_) => {
                                    trace!("End wait for {name}");
                                    let entry_owned = pid_table_locked.remove(&pid).unwrap();
                                    if let PidEntry::ServiceExited(code) = entry_owned
                                        && !conf.success_exit_status.is_success(&code)
                                        && !conf
                                            .exec
                                            .last()
                                            .map(|e| e.prefixes.contains(&CommandlinePrefix::Minus))
                                            .unwrap_or(false)
                                    {
                                        return Err(RunCmdError::BadExitCode(
                                            conf.exec
                                                .last()
                                                .map(|e| e.to_string())
                                                .unwrap_or_else(|| "(no exec)".to_owned()),
                                            code,
                                        ));
                                    }
                                    break;
                                }
                                PidEntry::Helper(_, _) => {
                                    // Should never happen
                                    unreachable!(
                                        "Was waiting on oneshot process but pid got saved as PidEntry::Helper"
                                    );
                                }
                                PidEntry::HelperExited(_) => {
                                    // Should never happen
                                    unreachable!(
                                        "Was waiting on oneshot process but pid got saved as PidEntry::HelperExited"
                                    );
                                }
                            }
                        }
                        None => {
                            // Should not happen. Either there is an Helper entry or a Exited entry
                            unreachable!("No entry for child found")
                        }
                    }
                }
                // exponential backoff to get low latencies for fast processes
                // but not hog the cpu for too long
                // start at 0.05 ms
                // capped to 10 ms to not introduce too big latencies
                // TODO review those numbers
                let sleep_dur = std::time::Duration::from_micros(counter * 50);
                let sleep_cap = std::time::Duration::from_millis(10);
                let sleep_dur = sleep_dur.min(sleep_cap);
                if sleep_dur < sleep_cap {
                    counter *= 2;
                }
                std::thread::sleep(sleep_dur);
            }
        }
        ServiceType::Forking => {
            trace!("[FORK_PARENT] Waiting for forking service to exit: {name}");
            let mut counter = 1u64;
            let pid = srvc.pid.unwrap();
            loop {
                if let Some(time_out) = duration_timeout
                    && start_time.elapsed() >= time_out
                {
                    error!("forking service {name} reached timeout waiting for parent to exit");
                    return Err(RunCmdError::Timeout(
                        conf.exec
                            .last()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "(no exec)".to_owned()),
                        format!("{duration_timeout:?}"),
                    ));
                }
                {
                    let mut pid_table_locked = pid_table.lock_poisoned();
                    match pid_table_locked.get(&pid) {
                        Some(PidEntry::Service(_, _)) => {
                            // Still running. Wait more.
                        }
                        Some(PidEntry::ServiceExited(_)) => {
                            trace!("[FORK_PARENT] Forking parent exited for {name}");
                            let entry_owned = pid_table_locked.remove(&pid).unwrap();
                            if let PidEntry::ServiceExited(code) = entry_owned
                                && !conf.success_exit_status.is_success(&code)
                                && !conf
                                    .exec
                                    .last()
                                    .map(|e| e.prefixes.contains(&CommandlinePrefix::Minus))
                                    .unwrap_or(false)
                            {
                                return Err(RunCmdError::BadExitCode(
                                    conf.exec
                                        .last()
                                        .map(|e| e.to_string())
                                        .unwrap_or_else(|| "(no exec)".to_owned()),
                                    code,
                                ));
                            }
                            // Parent exited successfully — the daemon should be
                            // running now. Try to pick up its PID from PIDFile.
                            if let Some(ref pid_file_path) = conf.pid_file {
                                if let Some(daemon_pid) = read_pid_file(pid_file_path) {
                                    trace!(
                                        "[FORK_PARENT] Read daemon PID {} from {:?} for {name}",
                                        daemon_pid, pid_file_path
                                    );
                                    srvc.pid = Some(daemon_pid);
                                    let now = crate::units::UnitTimestamps::now_usec();
                                    srvc.exec_main_start_timestamp = Some(now);
                                    srvc.exec_main_handoff_timestamp = Some(now);
                                    pid_table_locked.insert(
                                        daemon_pid,
                                        PidEntry::Service(
                                            run_info
                                                .unit_table
                                                .iter()
                                                .find(|(_, u)| u.id.name == name)
                                                .map(|(_, u)| u.id.clone())
                                                .unwrap(),
                                            conf.srcv_type,
                                        ),
                                    );
                                } else {
                                    warn!(
                                        "[FORK_PARENT] Could not read PIDFile {:?} for {name}",
                                        pid_file_path
                                    );
                                    srvc.pid = None;
                                }
                            } else {
                                trace!(
                                    "[FORK_PARENT] No PIDFile for forking service {name}, \
                                     clearing tracked PID"
                                );
                                srvc.pid = None;
                            }
                            break;
                        }
                        Some(PidEntry::Helper(_, _) | PidEntry::HelperExited(_)) => {
                            unreachable!(
                                "Was waiting on forking process but pid got saved as Helper entry"
                            );
                        }
                        None => {
                            unreachable!("No entry for child found");
                        }
                    }
                }
                let sleep_dur = std::time::Duration::from_micros(counter * 50);
                let sleep_cap = std::time::Duration::from_millis(10);
                let sleep_dur = sleep_dur.min(sleep_cap);
                if sleep_dur < sleep_cap {
                    counter *= 2;
                }
                std::thread::sleep(sleep_dur);
            }
        }
        ServiceType::Dbus => {
            if let Some(dbus_name) = &conf.dbus_name {
                trace!("[FORK_PARENT] Waiting for dbus name: {dbus_name}");
                match crate::dbus_wait::wait_for_name_system_bus(dbus_name, duration_timeout) {
                    Ok(res) => match res {
                        crate::dbus_wait::WaitResult::Ok => {
                            trace!("[FORK_PARENT] Found dbus name on bus: {dbus_name}");
                        }
                        crate::dbus_wait::WaitResult::Timedout => {
                            warn!("[FORK_PARENT] Did not find dbus name on bus: {dbus_name}");
                            return Err(RunCmdError::Timeout(
                                conf.exec
                                    .last()
                                    .map(|e| e.to_string())
                                    .unwrap_or_else(|| "(no exec)".to_owned()),
                                format!("{duration_timeout:?}"),
                            ));
                        }
                    },
                    Err(e) => {
                        return Err(RunCmdError::WaitError(
                            conf.exec
                                .last()
                                .map(|e| e.to_string())
                                .unwrap_or_else(|| "(no exec)".to_owned()),
                            format!("Error while waiting for dbus name: {e}"),
                        ));
                    }
                }
            } else {
                return Err(RunCmdError::Generic(format!(
                    "[FORK_PARENT] No busname given for service: {name:?}"
                )));
            }
        }
    }
    Ok(())
}
