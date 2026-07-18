//! Wait for sockets to activate their respective services
use log::error;
use log::trace;
use log::warn;

use crate::lock_ext::RwLockExt;
use crate::runtime_info::ArcMutRuntimeInfo;
use crate::units::{
    ActivationSource, SocketResult, Specific, StatusStarted, StatusStopped, UnitId, UnitIdKind,
    UnitOperationErrorReason, UnitStatus,
};
use log::info;
use std::os::unix::io::{BorrowedFd, RawFd};
use std::time::Instant;

/// Helper to create a BorrowedFd from a raw fd.
///
/// # Safety
/// The caller must ensure the fd is valid and will outlive the returned BorrowedFd.
unsafe fn borrow_fd(fd: i32) -> BorrowedFd<'static> {
    unsafe { BorrowedFd::borrow_raw(fd) }
}

/// Information gathered about a triggered socket before releasing the read lock.
struct SocketActivationInfo {
    socket_id: UnitId,
    is_accept: bool,
    /// Template service name (e.g. "foo@.service") for Accept=yes sockets.
    template_service_name: Option<String>,
    /// The raw fd of the listening socket (for calling accept()).
    listen_fd: Option<RawFd>,
    /// The service unit ID to activate (for Accept=no sockets).
    service_id: Option<UnitId>,
    /// Max connections allowed (Accept=yes).
    max_connections: u64,
    /// Max connections per source (Accept=yes).
    max_connections_per_source: u64,
    /// TriggerLimitIntervalSec — rate limit window (default 2s).
    trigger_limit_interval_sec: std::time::Duration,
    /// TriggerLimitBurst — max activations within the window (default 200).
    trigger_limit_burst: u32,
    /// PollLimitIntervalSec — poll rate limit window (default 0 = disabled).
    poll_limit_interval_sec: std::time::Duration,
    /// PollLimitBurst — max poll wakeups within the window (default 0 = disabled).
    poll_limit_burst: u32,
}

pub fn start_socketactivation_thread(run_info: ArcMutRuntimeInfo) {
    std::thread::spawn(move || {
        loop {
            // Exit the thread once a shutdown has been initiated — no new
            // services should be socket-activated while we are stopping.
            if crate::shutdown::is_shutting_down() {
                trace!("Socket activation thread exiting: shutdown in progress");
                return;
            }

            let wait_result = wait_for_socket(run_info.clone());
            match wait_result {
                Ok(ids) => {
                    if crate::shutdown::is_shutting_down() {
                        trace!("Socket activation thread exiting: shutdown in progress");
                        return;
                    }

                    // Phase 1: Gather info about each triggered socket.
                    // Use try_read() to yield to pending writers.
                    let infos: Vec<SocketActivationInfo> = {
                        let run_info_locked = match run_info.try_read() {
                            Ok(g) => g,
                            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
                            Err(std::sync::TryLockError::WouldBlock) => {
                                std::thread::sleep(std::time::Duration::from_millis(10));
                                continue;
                            }
                        };
                        let unit_table = &run_info_locked.unit_table;
                        ids.into_iter()
                            .filter_map(|socket_id| {
                                gather_socket_info(&socket_id, unit_table, &run_info_locked)
                            })
                            .collect()
                    };

                    // Phase 2: Process each socket.
                    for info in infos {
                        if info.is_accept {
                            handle_accept_yes(&run_info, &info);
                        } else {
                            handle_accept_no(&run_info, &info);
                        }
                    }
                }
                Err(e) => {
                    // During shutdown, sockets are closed which causes EBADF
                    // from select(). This is expected — exit silently.
                    if crate::shutdown::is_shutting_down() {
                        trace!("Socket activation thread exiting: shutdown in progress ({e})");
                    } else {
                        error!("Error in socket activation loop: {e}");
                    }
                    break;
                }
            }
        }
    });
}

/// Gather information about a triggered socket unit (under read lock).
fn gather_socket_info(
    socket_id: &UnitId,
    unit_table: &crate::runtime_info::UnitTable,
    run_info: &crate::runtime_info::RuntimeInfo,
) -> Option<SocketActivationInfo> {
    let sock_unit = unit_table.get(socket_id)?;
    let Specific::Socket(specific) = &sock_unit.specific else {
        return None;
    };

    let is_accept = specific.conf.accept;
    let max_connections = specific.conf.max_connections;
    let max_connections_per_source = specific.conf.max_connections_per_source;

    // Trigger limit defaults: 2 seconds interval, 200 burst (matching C systemd)
    let trigger_limit_interval_sec =
        std::time::Duration::from_secs(specific.conf.trigger_limit_interval_sec.unwrap_or(2));
    let trigger_limit_burst = specific.conf.trigger_limit_burst.unwrap_or(200);

    // Poll limit defaults: 0 = disabled (matching C systemd)
    let poll_limit_interval_sec =
        std::time::Duration::from_secs(specific.conf.poll_limit_interval_sec.unwrap_or(0));
    let poll_limit_burst = specific.conf.poll_limit_burst.unwrap_or(0);

    // Get the listening fd from the fd store
    let listen_fd = run_info
        .fd_store
        .read_poisoned()
        .get_global(&socket_id.name)
        .and_then(|fds| fds.first().map(|(_, _, fd)| fd.as_raw_fd()));

    // Find the associated service
    let mut service_id = None;
    let mut template_service_name = None;

    // Strategy 1: socket's own services list
    for srvc_id in &specific.conf.services {
        if is_accept {
            // For Accept=yes, we need the template name
            if crate::unit_name::is_template(&srvc_id.name) {
                template_service_name = Some(srvc_id.name.clone());
                break;
            }
            // If the service list has a non-template, derive the template
            if let Some(at_pos) = srvc_id.name.find('@') {
                let dot_pos = srvc_id.name.rfind('.').unwrap_or(srvc_id.name.len());
                let tmpl = format!(
                    "{}@.{}",
                    &srvc_id.name[..at_pos],
                    &srvc_id.name[dot_pos + 1..]
                );
                template_service_name = Some(tmpl);
                break;
            }
        } else if unit_table.contains_key(srvc_id) {
            service_id = Some(srvc_id.clone());
            break;
        }
    }

    // Strategy 2: derive template name from socket name
    if is_accept && template_service_name.is_none() {
        // e.g. "foo.socket" -> "foo@.service"
        let base = socket_id
            .name
            .strip_suffix(".socket")
            .unwrap_or(&socket_id.name);
        template_service_name = Some(format!("{base}@.service"));
    }

    // Strategy 2 for Accept=no: scan services
    if !is_accept && service_id.is_none() {
        for unit in unit_table.values() {
            if let Specific::Service(srvc_specific) = &unit.specific
                && srvc_specific.has_socket(&socket_id.name)
            {
                service_id = Some(unit.id.clone());
                break;
            }
        }
    }

    // Strategy 3 for Accept=no: fall back to the same-name service
    // (foo.socket -> foo.service).  systemd implicitly triggers the service
    // named like the socket when neither Service= nor the service's Sockets=
    // establishes the association.
    if !is_accept && service_id.is_none() {
        let base = socket_id
            .name
            .strip_suffix(".socket")
            .unwrap_or(&socket_id.name);
        let candidate = format!("{base}.service");
        service_id = unit_table
            .values()
            .find(|u| u.id.name == candidate)
            .map(|u| u.id.clone());
    }

    Some(SocketActivationInfo {
        socket_id: socket_id.clone(),
        is_accept,
        template_service_name,
        listen_fd,
        service_id,
        max_connections,
        max_connections_per_source,
        trigger_limit_interval_sec,
        trigger_limit_burst,
        poll_limit_interval_sec,
        poll_limit_burst,
    })
}

/// Check whether the socket has hit its trigger rate limit.
/// Records the current trigger timestamp and returns `true` if the limit is exceeded.
fn check_trigger_rate_limit(run_info: &ArcMutRuntimeInfo, info: &SocketActivationInfo) -> bool {
    let burst = info.trigger_limit_burst;
    let interval = info.trigger_limit_interval_sec;

    // A burst of 0 or zero interval disables rate limiting
    if burst == 0 || interval.is_zero() {
        return false;
    }

    let now = Instant::now();
    let run_info_locked = run_info.read_poisoned();
    if let Some(sock_unit) = run_info_locked.unit_table.get(&info.socket_id)
        && let Specific::Socket(specific) = &sock_unit.specific
    {
        let rate_limited = {
            let mut guard = specific.state.write_poisoned();
            let timestamps = &mut guard.sock.trigger_timestamps;

            // Prune timestamps outside the rate limit window
            timestamps.retain(|t| now.duration_since(*t) < interval);

            if timestamps.len() >= burst as usize {
                // Rate limit hit — mark socket as failed
                info!(
                    "Socket unit {} hit trigger rate limit ({} triggers in {:?}), transitioning to failed",
                    info.socket_id.name, burst, interval
                );
                guard.result = SocketResult::TriggerLimitHit;
                guard.sock.activated = true; // prevent further select() triggering
                true
            } else {
                // Record this trigger
                timestamps.push(now);
                false
            }
        }; // guard dropped here

        if rate_limited {
            // Transition unit status to failed (non-empty errors = ActiveState=failed)
            if let Some(sock_unit) = run_info_locked.unit_table.get(&info.socket_id) {
                let mut status = sock_unit.common.status.write_poisoned();
                *status = UnitStatus::Stopped(
                    StatusStopped::StoppedFinal,
                    vec![UnitOperationErrorReason::GenericStartError(
                        "trigger-limit-hit".to_string(),
                    )],
                );
            }
            return true;
        }
    }
    false
}

/// Check whether the socket has hit its poll rate limit.
/// Records the current poll timestamp and returns `true` if the limit is exceeded,
/// in which case the socket will be temporarily paused.
fn check_poll_rate_limit(run_info: &ArcMutRuntimeInfo, info: &SocketActivationInfo) -> bool {
    let burst = info.poll_limit_burst;
    let interval = info.poll_limit_interval_sec;

    // A burst of 0 or zero interval disables poll rate limiting
    if burst == 0 || interval.is_zero() {
        return false;
    }

    let now = Instant::now();
    let run_info_locked = run_info.read_poisoned();
    if let Some(sock_unit) = run_info_locked.unit_table.get(&info.socket_id)
        && let Specific::Socket(specific) = &sock_unit.specific
    {
        let mut guard = specific.state.write_poisoned();
        let timestamps = &mut guard.sock.poll_timestamps;

        // Prune timestamps outside the rate limit window
        timestamps.retain(|t| now.duration_since(*t) < interval);

        if timestamps.len() >= burst as usize {
            // Rate limit hit — pause the socket until end of interval
            let oldest = timestamps.first().copied().unwrap_or(now);
            let resume_at = oldest + interval;
            info!(
                "Socket unit {} hit poll rate limit ({} wakeups in {:?}), pausing until {:?}",
                info.socket_id.name, burst, interval, resume_at
            );
            guard.sock.poll_limit_paused_until = Some(resume_at);
            return true;
        }

        // Record this wakeup
        timestamps.push(now);
    }
    false
}

/// Whether the target service is currently blocked from being started by a
/// `Conflicts=` unit that is activating (Starting) or active (Started).  The
/// implicit `shutdown.target` conflict added by DefaultDependencies is ignored:
/// it is a passive artifact present on every unit and must not block socket
/// activation (only a real shutdown, which stops everything, matters).
fn conflict_blocks_activation(
    deps: &crate::units::Dependencies,
    unit_table: &crate::runtime_info::UnitTable,
) -> bool {
    deps.conflicts.iter().any(|cid| {
        // The implicit Conflicts=shutdown.target (from DefaultDependencies) is
        // not a real activation conflict; ignore it.
        if cid.name == "shutdown.target" {
            return false;
        }
        let Some(cu) = unit_table.get(cid) else {
            return false;
        };
        // A unit that is activating (Starting) or active (Started) blocks the
        // conflicting service from starting. Drop the status lock before we
        // reach for the service state lock below.
        let started = {
            match &*cu.common.status.read_poisoned() {
                UnitStatus::Starting => return true,
                UnitStatus::Started(_) => true,
                _ => false,
            }
        };
        if !started {
            return false;
        }
        // A clean-exited oneshot with RemainAfterExit=no keeps its status as
        // Started only so the boot activation graph walker can see it as
        // completed, but it is effectively inactive (issue #27953) and must
        // not keep a deferred socket blocked. Mirror the `is-active` check.
        if let Specific::Service(srvc) = &cu.specific
            && srvc.conf.srcv_type == crate::units::ServiceType::OneShot
            && !srvc.conf.remain_after_exit
            && srvc.state.read_poisoned().srvc.pid.is_none()
        {
            return false;
        }
        true
    })
}

/// Handle socket activation for Accept=no sockets (traditional mode).
fn handle_accept_no(run_info: &ArcMutRuntimeInfo, info: &SocketActivationInfo) {
    // Check poll rate limit before processing
    if check_poll_rate_limit(run_info, info) {
        return;
    }
    // Check trigger rate limit before activating
    if check_trigger_rate_limit(run_info, info) {
        return;
    }

    let run_info_locked = run_info.read_poisoned();
    let unit_table = &run_info_locked.unit_table;

    // Mark socket as activated and increment NAccepted counter
    if let Some(sock_unit) = unit_table.get(&info.socket_id)
        && let Specific::Socket(specific) = &sock_unit.specific
    {
        let mut_state = &mut *specific.state.write_poisoned();
        mut_state.sock.activated = true;
        mut_state.sock.accept_counter += 1;
    }

    let Some(ref service_id) = info.service_id else {
        error!(
            "Socket unit {:?} activated, but no matching service could be found",
            info.socket_id
        );
        return;
    };

    let Some(srvc_unit) = unit_table.get(service_id) else {
        error!("Service unit {service_id:?} not found in unit table");
        return;
    };

    let srvc_status = {
        let status_locked = &*srvc_unit.common.status.read_poisoned();
        status_locked.clone()
    };

    // DeferTrigger=: if the target service has a Conflicts= unit that is
    // currently activating (Starting) or active (Started), starting the service
    // now would be blocked.  Instead of failing, enter the `deferred` state and
    // let the wait loop retry once the conflict clears (or fail after
    // DeferTriggerMaxSec=).  Only applies when DeferTrigger= is Yes/Patient.
    let defer_trigger = match unit_table.get(&info.socket_id).map(|u| &u.specific) {
        Some(Specific::Socket(s)) => s.conf.defer_trigger.clone(),
        _ => crate::units::DeferTrigger::No,
    };
    if !matches!(defer_trigger, crate::units::DeferTrigger::No) {
        let conflict_active =
            conflict_blocks_activation(&srvc_unit.common.dependencies, unit_table);
        if conflict_active {
            if let Some(Specific::Socket(specific)) =
                unit_table.get(&info.socket_id).map(|u| &u.specific)
            {
                let mut st = specific.state.write_poisoned();
                // We are deferring: undo the activated mark set above so the
                // service is not considered running, and record the deferral.
                st.sock.activated = false;
                if !st.sock.deferred {
                    st.sock.deferred = true;
                    st.sock.deferred_since = Some(std::time::Instant::now());
                }
                st.sock.deferred_service = Some(service_id.clone());
            }
            trace!(
                "Socket {} deferring activation: a Conflicts= unit is still active",
                info.socket_id.name
            );
            return;
        }
    }

    if srvc_status == UnitStatus::Started(StatusStarted::WaitingForSocket)
        || srvc_status == UnitStatus::NeverStarted
        || matches!(srvc_status, UnitStatus::Stopped(..))
    {
        trace!("Start service {} by socket activation", srvc_unit.id.name);
        let activation_result = crate::units::activate_unit(
            srvc_unit.id.clone(),
            &run_info_locked,
            ActivationSource::SocketActivation,
        );

        match &activation_result {
            Ok(_) => {
                trace!(
                    "New status after socket activation: {:?}",
                    *unit_table
                        .get(&srvc_unit.id)
                        .unwrap()
                        .common
                        .status
                        .read()
                        .unwrap()
                );
                // If the service deferred its start wait (Type=notify /
                // oneshot returned DeferredNotifyWait, leaving it Starting),
                // spawn the background completion handler.  Otherwise this
                // thread would hold the RuntimeInfo read lock across the wait
                // and starve control-socket writers (e.g. daemon-reload).
                let deferred = unit_table
                    .get(service_id)
                    .map(|u| matches!(&*u.common.status.read_poisoned(), UnitStatus::Starting))
                    .unwrap_or(false);
                if deferred {
                    crate::units::spawn_deferred_service_wait(service_id.clone(), run_info.clone());
                }
            }
            Err(e) => {
                if matches!(e.reason, UnitOperationErrorReason::DependencyError(_)) {
                    trace!(
                        "Socket activation deferred for {}: deps not yet ready",
                        e.unit_name
                    );
                } else {
                    error!("Error while starting service from socket activation: {e}");
                }
            }
        }

        // Re-arm the socket if the service did not end up running (e.g.
        // ConditionPathExistsGlob= failed, or activation error). Without
        // this, the socket stays activated=true and the select() loop
        // skips it, preventing future trigger events — which means the
        // trigger rate limit can never be reached (issue #2467).
        let should_rearm = if activation_result.is_err() {
            true
        } else if let Some(srvc) = unit_table.get(service_id) {
            let status = srvc.common.status.read_poisoned();
            !status.is_started() && !matches!(&*status, UnitStatus::Starting)
        } else {
            false
        };

        if should_rearm {
            if let Some(sock_unit) = unit_table.get(&info.socket_id)
                && let Specific::Socket(specific) = &sock_unit.specific
                && specific.state.read_poisoned().result != SocketResult::TriggerLimitHit
            {
                specific.state.write_poisoned().sock.activated = false;
                trace!(
                    "Re-armed socket {:?} after service {:?} failed to start",
                    info.socket_id.name, service_id.name,
                );
            }
            run_info_locked.notify_eventfds();
        }
    } else {
        trace!("Ignore socket activation. Service has status: {srvc_status:?}");
    }
}

/// Handle socket activation for Accept=yes sockets (per-connection mode).
fn handle_accept_yes(run_info: &ArcMutRuntimeInfo, info: &SocketActivationInfo) {
    // Check poll rate limit before processing
    if check_poll_rate_limit(run_info, info) {
        return;
    }
    // Check trigger rate limit before activating
    if check_trigger_rate_limit(run_info, info) {
        return;
    }

    let Some(listen_fd) = info.listen_fd else {
        error!("Accept=yes socket {:?} has no listening fd", info.socket_id);
        return;
    };

    let Some(ref template_name) = info.template_service_name else {
        error!(
            "Accept=yes socket {:?} has no template service",
            info.socket_id
        );
        return;
    };

    // Accept the incoming connection
    let accepted_fd =
        unsafe { libc::accept(listen_fd, std::ptr::null_mut(), std::ptr::null_mut()) };
    if accepted_fd < 0 {
        error!(
            "accept() failed on socket {:?}: {}",
            info.socket_id,
            std::io::Error::last_os_error()
        );
        return;
    }
    // Set FD_CLOEXEC on the accepted fd (will be unset by fork_child for the service)
    let _ = nix::fcntl::fcntl(
        unsafe { BorrowedFd::borrow_raw(accepted_fd) },
        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
    );

    // Get peer credentials for per-source tracking (Unix sockets)
    let accepted_bfd = unsafe { BorrowedFd::borrow_raw(accepted_fd) };
    let peer_uid =
        nix::sys::socket::getsockopt(&accepted_bfd, nix::sys::socket::sockopt::PeerCredentials)
            .ok()
            .map(|cred| cred.uid());

    // Generate instance name and check connection limits
    let (instance_counter, active_connections) = {
        let ri = run_info.read_poisoned();
        if let Some(sock_unit) = ri.unit_table.get(&info.socket_id) {
            if let Specific::Socket(specific) = &sock_unit.specific {
                let state = specific.state.read_poisoned();
                (
                    state.sock.accept_counter,
                    state.sock.active_accept_connections,
                )
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        }
    };

    // Check MaxConnections
    if active_connections >= info.max_connections {
        warn!(
            "Accept=yes socket {:?}: MaxConnections={} reached ({} active), rejecting",
            info.socket_id, info.max_connections, active_connections
        );
        unsafe { libc::close(accepted_fd) };
        return;
    }

    // Check MaxConnectionsPerSource
    if let Some(uid) = peer_uid {
        let per_source_count = count_connections_per_source(run_info, &info.socket_id.name, uid);
        if per_source_count >= info.max_connections_per_source {
            warn!(
                "Accept=yes socket {:?}: MaxConnectionsPerSource={} reached for uid {} ({} active), rejecting",
                info.socket_id, info.max_connections_per_source, uid, per_source_count
            );
            unsafe { libc::close(accepted_fd) };
            return;
        }
    }

    // Generate instance name
    let counter = instance_counter;
    let instance_str = counter.to_string();
    let instance_name = match crate::unit_name::template_instantiate(template_name, &instance_str) {
        Some(name) => name,
        None => {
            error!(
                "Failed to instantiate template {} with instance {}",
                template_name, instance_str
            );
            unsafe { libc::close(accepted_fd) };
            return;
        }
    };

    trace!(
        "Accept=yes socket {:?}: accepted connection fd={}, spawning {}",
        info.socket_id, accepted_fd, instance_name
    );

    // Increment the accept counter on the socket
    {
        let ri = run_info.read_poisoned();
        if let Some(sock_unit) = ri.unit_table.get(&info.socket_id)
            && let Specific::Socket(specific) = &sock_unit.specific
        {
            let mut state = specific.state.write_poisoned();
            state.sock.accept_counter += 1;
            state.sock.active_accept_connections += 1;
        }
    }

    // Instantiate the template and insert into unit table.
    // This requires a write lock on RuntimeInfo.
    let instance_id = UnitId {
        kind: UnitIdKind::Service,
        name: instance_name.clone(),
    };

    {
        let mut ri = run_info.write_poisoned_nonblocking();
        let unit_dirs = ri.config.unit_dirs.clone();

        // Check if already exists
        if !ri.unit_table.contains_key(&instance_id) {
            if let Some(mut unit) = crate::units::loading::directory_deps::instantiate_template(
                template_name,
                &instance_str,
                &instance_name,
                &unit_dirs,
                &std::collections::HashMap::new(),
            ) {
                // Set up socket reference on the service so FDs can be found
                if let Specific::Service(ref mut srvc_specific) = unit.specific {
                    srvc_specific.conf.sockets.push(info.socket_id.clone());
                    // Set accepted_fd on the service state
                    srvc_specific.state.write_poisoned().srvc.accepted_fd = Some(accepted_fd);
                    // Store the peer UID for per-source tracking
                    if let Some(uid) = peer_uid {
                        srvc_specific.state.write_poisoned().srvc.accepted_peer_uid = Some(uid);
                    }
                }
                ri.unit_table.insert(instance_id.clone(), unit);
                trace!("Instantiated Accept=yes service instance: {instance_name}");
            } else {
                error!(
                    "Failed to instantiate template {} for Accept=yes instance {}",
                    template_name, instance_name
                );
                unsafe { libc::close(accepted_fd) };
                // Decrement connection count
                if let Some(sock_unit) = ri.unit_table.get(&info.socket_id)
                    && let Specific::Socket(specific) = &sock_unit.specific
                {
                    let mut state = specific.state.write_poisoned();
                    state.sock.active_accept_connections =
                        state.sock.active_accept_connections.saturating_sub(1);
                }
                return;
            }
        } else {
            // Instance already exists — set the accepted fd on it
            if let Some(unit) = ri.unit_table.get(&instance_id)
                && let Specific::Service(ref srvc_specific) = unit.specific
            {
                srvc_specific.state.write_poisoned().srvc.accepted_fd = Some(accepted_fd);
            }
        }
    }

    // Activate the instance (needs read lock)
    {
        let ri = run_info.read_poisoned();
        match crate::units::activate_unit(
            instance_id.clone(),
            &ri,
            ActivationSource::SocketActivation,
        ) {
            Ok(_) => {
                trace!(
                    "Accept=yes service instance {} activated successfully",
                    instance_name
                );
                // Spawn the background completion handler if the instance
                // deferred its start wait (see handle_accept_no).
                let deferred = ri
                    .unit_table
                    .get(&instance_id)
                    .map(|u| matches!(&*u.common.status.read_poisoned(), UnitStatus::Starting))
                    .unwrap_or(false);
                if deferred {
                    crate::units::spawn_deferred_service_wait(
                        instance_id.clone(),
                        run_info.clone(),
                    );
                }
            }
            Err(e) => {
                error!(
                    "Failed to activate Accept=yes service instance {}: {e}",
                    instance_name
                );
            }
        }
    }
}

/// Count the number of active Accept=yes connections from a specific UID
/// for a given socket.
fn count_connections_per_source(run_info: &ArcMutRuntimeInfo, socket_name: &str, uid: u32) -> u64 {
    let ri = run_info.read_poisoned();
    let mut count = 0u64;

    // Derive the template prefix from the socket name (e.g. "foo.socket" -> "foo@")
    let base = socket_name.strip_suffix(".socket").unwrap_or(socket_name);
    let prefix = format!("{base}@");

    for (id, unit) in &ri.unit_table {
        if !id.name.starts_with(&prefix) || !id.name.ends_with(".service") {
            continue;
        }
        if let Specific::Service(srvc_specific) = &unit.specific {
            let state = srvc_specific.state.read_poisoned();
            // Only count running instances with matching peer UID
            if state.srvc.pid.is_some()
                && let Some(peer_uid) = state.srvc.accepted_peer_uid
                && peer_uid == uid
            {
                count += 1;
            }
        }
    }

    count
}

pub fn wait_for_socket(run_info: ArcMutRuntimeInfo) -> Result<Vec<UnitId>, String> {
    let eventfd = { run_info.read_poisoned().socket_activation_eventfd };
    let (mut fdset, fd_to_sock_id, mut select_timeout, un_deferred_ids) = {
        // Use try_read() to yield to pending writers (e.g., find_or_load_unit).
        let run_info_locked = match run_info.try_read() {
            Ok(g) => g,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Ok(Vec::new());
            }
        };
        let now = Instant::now();

        let fd_to_sock_id = run_info_locked.fd_store.read_poisoned().global_fds_to_ids();
        let mut fdset = nix::sys::select::FdSet::new();
        let mut earliest_resume: Option<Instant> = None;
        // Sockets whose DeferTrigger= deferral just cleared this cycle; they are
        // returned as triggered so the caller re-runs handle_accept_no and
        // activates the service (the original connection may no longer keep the
        // fd readable, so we cannot rely on select() re-reporting it).
        let mut un_deferred_ids: Vec<UnitId> = Vec::new();
        {
            let unit_table_locked = &run_info_locked.unit_table;

            // Re-evaluate DeferTrigger= deferred sockets: if the blocking
            // Conflicts= unit has cleared, un-defer so the socket's fd is
            // re-included below and handle_accept_no retries the activation.
            // Read the conflict statuses WITHOUT holding the socket state write
            // lock, to avoid a lock-order inversion with the service exit
            // handler (which takes a service status lock then a socket state
            // lock while re-arming sockets).
            for (_fd, id) in &fd_to_sock_id {
                let Some(unit) = unit_table_locked.get(id) else {
                    continue;
                };
                let Specific::Socket(specific) = &unit.specific else {
                    continue;
                };
                let (svc_id, deferred_since) = {
                    let st = specific.state.read_poisoned();
                    if !st.sock.deferred {
                        continue;
                    }
                    (st.sock.deferred_service.clone(), st.sock.deferred_since)
                };
                // DeferTriggerMaxSec=: fail the socket if it stayed deferred too
                // long.  Set activated=true so it is excluded from the select
                // set (no re-trigger) and transition the unit to failed.
                if let Some(max) = specific.conf.defer_trigger_max_sec
                    && let Some(since) = deferred_since
                    && now.duration_since(since) >= std::time::Duration::from_secs(max)
                {
                    {
                        let mut st = specific.state.write_poisoned();
                        st.sock.deferred = false;
                        st.sock.deferred_since = None;
                        st.sock.deferred_service = None;
                        st.sock.activated = true;
                    }
                    {
                        let mut status = unit.common.status.write_poisoned();
                        *status = UnitStatus::Stopped(
                            StatusStopped::StoppedFinal,
                            vec![UnitOperationErrorReason::GenericStartError(
                                "DeferTriggerMaxSec= elapsed".to_string(),
                            )],
                        );
                    }
                    continue;
                }
                let still_blocked = svc_id
                    .as_ref()
                    .and_then(|sid| unit_table_locked.get(sid))
                    .map(|svc| conflict_blocks_activation(&svc.common.dependencies, unit_table_locked))
                    .unwrap_or(false);
                if !still_blocked {
                    {
                        let mut st = specific.state.write_poisoned();
                        st.sock.deferred = false;
                        st.sock.deferred_since = None;
                        st.sock.deferred_service = None;
                    }
                    // Re-run activation now that the conflict has cleared.
                    un_deferred_ids.push((*id).clone());
                }
            }

            for (fd, id) in &fd_to_sock_id {
                let Some(unit) = unit_table_locked.get(id) else {
                    // Unit was removed (e.g. during daemon-reload) but its
                    // FDs are still in the fd_store — skip it.
                    continue;
                };
                if let Specific::Socket(specific) = &unit.specific {
                    let mut state = specific.state.write_poisoned();
                    // Skip sockets that hit their trigger rate limit
                    if state.result == SocketResult::TriggerLimitHit {
                        continue;
                    }
                    // Check poll limit pause
                    if let Some(resume_at) = state.sock.poll_limit_paused_until {
                        if now < resume_at {
                            // Still paused — track earliest resume time
                            earliest_resume = Some(match earliest_resume {
                                Some(e) => e.min(resume_at),
                                None => resume_at,
                            });
                            continue;
                        } else {
                            // Pause expired — clear it and resume
                            state.sock.poll_limit_paused_until = None;
                            state.sock.poll_timestamps.clear();
                        }
                    }
                    // DeferTrigger=: a deferred socket is excluded from the
                    // select set and re-checked on a short timer (the re-eval
                    // pass above clears `deferred` once the conflict lifts).
                    if state.sock.deferred {
                        let recheck = now + std::time::Duration::from_millis(200);
                        earliest_resume = Some(match earliest_resume {
                            Some(e) => e.min(recheck),
                            None => recheck,
                        });
                        continue;
                    }
                    // For Accept=yes sockets, always keep listening (never mark activated)
                    // For Accept=no sockets, skip if already activated
                    if !state.sock.activated || specific.conf.accept {
                        fdset.insert(unsafe { borrow_fd(*fd) });
                    }
                }
            }
            fdset.insert(unsafe { borrow_fd(eventfd.read_end()) });
        }

        // If any sockets are paused, use a timeout so we resume them on time
        let timeout = earliest_resume.map(|resume_at| {
            let remaining = resume_at.saturating_duration_since(now);
            let mut tv = nix::sys::time::TimeVal::new(
                remaining.as_secs() as i64,
                remaining.subsec_micros() as i64,
            );
            // Ensure at least 1ms timeout to avoid busy-spinning
            if tv.tv_sec() == 0 && tv.tv_usec() == 0 {
                tv = nix::sys::time::TimeVal::new(0, 1000);
            }
            tv
        });

        (fdset, fd_to_sock_id, timeout, un_deferred_ids)
    };

    let result =
        nix::sys::select::select(None, Some(&mut fdset), None, None, select_timeout.as_mut());
    match result {
        Ok(_) => {
            let mut activated_ids = Vec::new();
            if fdset.contains(unsafe { borrow_fd(eventfd.read_end()) }) {
                trace!("Interrupted socketactivation select because the eventfd fired");
                crate::platform::reset_event_fd(eventfd);
                trace!("Reset eventfd value");
            }
            // Always check socket FDs — even when the eventfd fired,
            // a socket FD may also be ready and should not be missed.
            for (fd, id) in &fd_to_sock_id {
                if fdset.contains(unsafe { borrow_fd(*fd) }) {
                    activated_ids.push(id.clone());
                }
            }
            // Sockets whose DeferTrigger= deferral cleared this cycle must be
            // (re)activated even if their fd is no longer readable.
            for id in un_deferred_ids {
                if !activated_ids.contains(&id) {
                    activated_ids.push(id);
                }
            }
            Ok(activated_ids)
        }
        Err(e) => {
            if e == nix::Error::EINTR {
                Ok(Vec::new())
            } else if e == nix::Error::EBADF {
                if crate::shutdown::is_shutting_down() {
                    // During shutdown, socket fds are closed before this thread
                    // exits, causing EBADF from select().  Return an empty vec
                    // so the caller can check the shutdown flag and exit cleanly.
                    Ok(Vec::new())
                } else {
                    // An FD became invalid during normal operation (e.g. a
                    // socket unit was restarted or its FDs were replaced).
                    // Return empty so the caller rebuilds the FD set from
                    // the current fd_store on the next iteration.
                    trace!("select() returned EBADF, will rebuild FD set");
                    Ok(Vec::new())
                }
            } else {
                Err(format!("Error while selecting: {e}"))
            }
        }
    }
}
