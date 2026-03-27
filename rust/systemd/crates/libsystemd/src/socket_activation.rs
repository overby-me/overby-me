//! Wait for sockets to activate their respective services
use log::error;
use log::trace;

use crate::lock_ext::RwLockExt;
use crate::runtime_info::ArcMutRuntimeInfo;
use crate::units::{
    ActivationSource, Specific, StatusStarted, UnitId, UnitOperationErrorReason, UnitStatus,
};
use std::os::unix::io::BorrowedFd;

/// Helper to create a BorrowedFd from a raw fd.
///
/// # Safety
/// The caller must ensure the fd is valid and will outlive the returned BorrowedFd.
unsafe fn borrow_fd(fd: i32) -> BorrowedFd<'static> {
    unsafe { BorrowedFd::borrow_raw(fd) }
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
                    let run_info = run_info.read_poisoned();
                    let unit_table = &run_info.unit_table;
                    for socket_id in ids {
                        {
                            // Search the service this socket belongs to.
                            //
                            // Strategy:
                            // 1. First check the socket unit's own `services` list
                            //    (populated from the Service= directive or name-matching
                            //    in apply_sockets_to_services). This is the primary and
                            //    most reliable lookup — it's how real systemd works.
                            // 2. Fall back to scanning all services to find one that
                            //    lists this socket in its `sockets` field.
                            let mut srvc_unit = None;

                            // Strategy 1: look at the socket's own services list
                            let sock_unit = unit_table.get(&socket_id).unwrap();
                            if let Specific::Socket(specific) = &sock_unit.specific {
                                for srvc_id in &specific.conf.services {
                                    if let Some(unit) = unit_table.get(srvc_id) {
                                        trace!(
                                            "Start service {} by socket activation (from socket's Service= list)",
                                            unit.id.name
                                        );
                                        srvc_unit = Some(unit);
                                        break;
                                    }
                                }
                            }

                            // Strategy 2: fall back to scanning services
                            if srvc_unit.is_none() {
                                for unit in unit_table.values() {
                                    if let Specific::Service(specific) = &unit.specific
                                        && specific.has_socket(&socket_id.name)
                                    {
                                        srvc_unit = Some(unit);
                                        trace!(
                                            "Start service {} by socket activation (from service's Sockets= list)",
                                            unit.id.name
                                        );
                                        break;
                                    }
                                }
                            }

                            // mark socket as activated, removing it from the set of
                            // fds rust-systemd is actively listening on
                            if let Specific::Socket(specific) = &sock_unit.specific {
                                let mut_state = &mut *specific.state.write_poisoned();
                                mut_state.sock.activated = true;
                            }
                            if srvc_unit.is_none() {
                                error!(
                                    "Socket unit {socket_id:?} activated, but no matching service could be found \
                                     (checked socket's services list and all services' sockets lists)"
                                );
                            }
                            if let Some(srvc_unit) = srvc_unit {
                                let srvc_status = {
                                    let status_locked = &*srvc_unit.common.status.read_poisoned();
                                    status_locked.clone()
                                };

                                if srvc_status
                                    == UnitStatus::Started(StatusStarted::WaitingForSocket)
                                    || srvc_status == UnitStatus::NeverStarted
                                    || matches!(srvc_status, UnitStatus::Stopped(..))
                                {
                                    // the service unit gets activated — it was waiting
                                    // for socket activation, never started, or was
                                    // previously stopped and should restart on demand.
                                    match crate::units::activate_unit(
                                        srvc_unit.id.clone(),
                                        &run_info,
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
                                            if matches!(
                                                e.reason,
                                                UnitOperationErrorReason::DependencyError(_)
                                            ) {
                                                // Dependencies not yet satisfied — the normal
                                                // activation graph will start this service once
                                                // its After= deps are met.  This commonly
                                                // happens for socket-activated services like
                                                // systemd-udevd whose sockets are ready before
                                                // all ordering deps (e.g.
                                                // systemd-tmpfiles-setup-dev-early.service)
                                                // have completed.
                                                trace!(
                                                    "Socket activation deferred for {}: deps not yet ready, \
                                                     normal activation will handle it",
                                                    e.unit_name
                                                );
                                            } else {
                                                error!(
                                                    "Error while starting service from socket activation: {e}"
                                                );
                                            }
                                        }
                                    }
                                } else {
                                    // Service is already running or stopped — no need
                                    // to activate it again.
                                    trace!(
                                        "Ignore socket activation. Service has status: {srvc_status:?}"
                                    );
                                }
                            }
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
                    if !mut_state.sock.activated {
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
