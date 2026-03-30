//! Wait for sockets to activate their respective services
use log::error;
use log::trace;
use log::warn;

use crate::lock_ext::RwLockExt;
use crate::runtime_info::ArcMutRuntimeInfo;
use crate::units::{
    ActivationSource, Specific, StatusStarted, UnitId, UnitIdKind, UnitOperationErrorReason,
    UnitStatus,
};
use std::os::unix::io::{BorrowedFd, RawFd};

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

                    // Phase 1: Gather info about each triggered socket with a read lock.
                    let infos: Vec<SocketActivationInfo> = {
                        let run_info_locked = run_info.read_poisoned();
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

    Some(SocketActivationInfo {
        socket_id: socket_id.clone(),
        is_accept,
        template_service_name,
        listen_fd,
        service_id,
        max_connections,
        max_connections_per_source,
    })
}

/// Handle socket activation for Accept=no sockets (traditional mode).
fn handle_accept_no(run_info: &ArcMutRuntimeInfo, info: &SocketActivationInfo) {
    let run_info_locked = run_info.read_poisoned();
    let unit_table = &run_info_locked.unit_table;

    // Mark socket as activated
    if let Some(sock_unit) = unit_table.get(&info.socket_id)
        && let Specific::Socket(specific) = &sock_unit.specific
    {
        let mut_state = &mut *specific.state.write_poisoned();
        mut_state.sock.activated = true;
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

    if srvc_status == UnitStatus::Started(StatusStarted::WaitingForSocket)
        || srvc_status == UnitStatus::NeverStarted
        || matches!(srvc_status, UnitStatus::Stopped(..))
    {
        trace!("Start service {} by socket activation", srvc_unit.id.name);
        match crate::units::activate_unit(
            srvc_unit.id.clone(),
            &run_info_locked,
            ActivationSource::SocketActivation,
        ) {
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
    } else {
        trace!("Ignore socket activation. Service has status: {srvc_status:?}");
    }
}

/// Handle socket activation for Accept=yes sockets (per-connection mode).
fn handle_accept_yes(run_info: &ArcMutRuntimeInfo, info: &SocketActivationInfo) {
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
        let mut ri = run_info.write_poisoned();
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
    let (mut fdset, fd_to_sock_id) = {
        let run_info_locked = &*run_info.read_poisoned();

        let fd_to_sock_id = run_info_locked.fd_store.read_poisoned().global_fds_to_ids();
        let mut fdset = nix::sys::select::FdSet::new();
        {
            let unit_table_locked = &run_info_locked.unit_table;
            for (fd, id) in &fd_to_sock_id {
                let unit = unit_table_locked.get(id).unwrap();
                if let Specific::Socket(specific) = &unit.specific {
                    let mut_state = &*specific.state.read_poisoned();
                    // For Accept=yes sockets, always keep listening (never mark activated)
                    // For Accept=no sockets, skip if already activated
                    if !mut_state.sock.activated || specific.conf.accept {
                        fdset.insert(unsafe { borrow_fd(*fd) });
                    }
                }
            }
            fdset.insert(unsafe { borrow_fd(eventfd.read_end()) });
        }
        (fdset, fd_to_sock_id)
    };

    let result = nix::sys::select::select(None, Some(&mut fdset), None, None, None);
    match result {
        Ok(_) => {
            let mut activated_ids = Vec::new();
            if fdset.contains(unsafe { borrow_fd(eventfd.read_end()) }) {
                trace!("Interrupted socketactivation select because the eventfd fired");
                crate::platform::reset_event_fd(eventfd);
                trace!("Reset eventfd value");
            } else {
                for (fd, id) in &fd_to_sock_id {
                    if fdset.contains(unsafe { borrow_fd(*fd) }) {
                        activated_ids.push(id.clone());
                    }
                }
            }
            Ok(activated_ids)
        }
        Err(e) => {
            if e == nix::Error::EINTR {
                Ok(Vec::new())
            } else if e == nix::Error::EBADF && crate::shutdown::is_shutting_down() {
                // During shutdown, socket fds are closed before this thread
                // exits, causing EBADF from select().  Return an empty vec
                // so the caller can check the shutdown flag and exit cleanly.
                Ok(Vec::new())
            } else {
                Err(format!("Error while selecting: {e}"))
            }
        }
    }
}
