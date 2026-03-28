//! collect the different streams from the services
//! Stdout and stderr get redirected to the normal stdout/err but are prefixed with a unique string to identify their output
//! streams from the notification sockets get parsed and applied to the respective service

use log::trace;
use log::warn;

use crate::lock_ext::RwLockExt;
use crate::platform::reset_event_fd;
use crate::runtime_info::ArcMutRuntimeInfo;
use crate::services::Service;
use crate::services::StdIo;
use crate::units::{NotifyKind, Specific, UnitId};
use std::os::unix::io::BorrowedFd;
use std::os::unix::io::RawFd;
use std::{collections::HashMap, os::unix::io::AsRawFd};

/// Helper to create a BorrowedFd from a raw fd.
///
/// # Safety
/// The caller must ensure the fd is valid and will outlive the returned BorrowedFd.
unsafe fn borrow_fd(fd: i32) -> BorrowedFd<'static> {
    unsafe { BorrowedFd::borrow_raw(fd) }
}

fn collect_from_srvc<F>(run_info: ArcMutRuntimeInfo, f: F) -> HashMap<i32, UnitId>
where
    F: Fn(&mut HashMap<i32, UnitId>, &Service, UnitId),
{
    let run_info_locked = run_info.read_poisoned();
    let unit_table = &run_info_locked.unit_table;
    unit_table
        .iter()
        .fold(HashMap::new(), |mut map, (id, srvc_unit)| {
            if let Specific::Service(srvc) = &srvc_unit.specific {
                let state = &*srvc.state.read_poisoned();
                f(&mut map, &state.srvc, id.clone());
            }
            map
        })
}

pub fn handle_all_streams(run_info: ArcMutRuntimeInfo) {
    let eventfd = { run_info.read_poisoned().notification_eventfd };
    loop {
        if crate::shutdown::is_shutting_down() {
            trace!("Notification handler exiting: shutdown in progress");
            return;
        }
        let fd_to_srvc_id = collect_from_srvc(run_info.clone(), |map, srvc, id| {
            if let Some(socket) = &srvc.notifications {
                map.insert(socket.as_raw_fd(), id);
            }
        });

        let mut fdset = nix::sys::select::FdSet::new();
        for fd in fd_to_srvc_id.keys() {
            fdset.insert(unsafe { borrow_fd(*fd) });
        }
        fdset.insert(unsafe { borrow_fd(eventfd.read_end()) });

        let result = nix::sys::select::select(None, Some(&mut fdset), None, None, None);

        let run_info_locked = run_info.read_poisoned();
        let unit_table = &run_info_locked.unit_table;
        match result {
            Ok(_) => {
                if fdset.contains(unsafe { borrow_fd(eventfd.read_end()) }) {
                    trace!("Interrupted notification select because the eventfd fired");
                    reset_event_fd(eventfd);
                    trace!("Reset eventfd value");
                }
                let mut buf = [0u8; 4096];
                // Space for up to 8 file descriptors via SCM_RIGHTS.
                // nix::cmsg_space! computes the correct buffer size.
                let mut cmsg_buf = nix::cmsg_space!([RawFd; 8]);
                for (fd, id) in &fd_to_srvc_id {
                    if fdset.contains(unsafe { borrow_fd(*fd) })
                        && let Some(srvc_unit) = unit_table.get(id)
                        && let Specific::Service(srvc) = &srvc_unit.specific
                    {
                        let mut_state = &mut *srvc.state.write_poisoned();
                        if mut_state.srvc.notifications.is_some() {
                            let old_flags = nix::fcntl::fcntl(
                                unsafe { borrow_fd(*fd) },
                                nix::fcntl::FcntlArg::F_GETFL,
                            )
                            .unwrap();

                            let old_flags = nix::fcntl::OFlag::from_bits(old_flags).unwrap();
                            let mut new_flags = old_flags;
                            new_flags.insert(nix::fcntl::OFlag::O_NONBLOCK);
                            nix::fcntl::fcntl(
                                unsafe { borrow_fd(*fd) },
                                nix::fcntl::FcntlArg::F_SETFL(new_flags),
                            )
                            .unwrap();

                            // Use recvmsg() instead of recv() to capture
                            // SCM_RIGHTS ancillary data (file descriptors)
                            // sent alongside sd_notify messages for FDSTORE=1.
                            let mut iov = [std::io::IoSliceMut::new(&mut buf[..])];
                            let recv_result = nix::sys::socket::recvmsg::<()>(
                                *fd,
                                &mut iov,
                                Some(&mut cmsg_buf),
                                nix::sys::socket::MsgFlags::MSG_CMSG_CLOEXEC
                                    | nix::sys::socket::MsgFlags::MSG_DONTWAIT,
                            );

                            nix::fcntl::fcntl(
                                unsafe { borrow_fd(*fd) },
                                nix::fcntl::FcntlArg::F_SETFL(old_flags),
                            )
                            .unwrap();

                            match recv_result {
                                Ok(msg) => {
                                    let bytes = msg.bytes;
                                    if bytes == 0 {
                                        continue;
                                    }

                                    // Collect any file descriptors from SCM_RIGHTS.
                                    let mut received_fds: Vec<RawFd> = Vec::new();
                                    if let Ok(cmsgs) = msg.cmsgs() {
                                        for cmsg in cmsgs {
                                            if let nix::sys::socket::ControlMessageOwned::ScmRights(
                                                fds,
                                            ) = cmsg
                                            {
                                                received_fds.extend_from_slice(&fds);
                                            }
                                        }
                                    }

                                    let note_str =
                                        String::from_utf8_lossy(&buf[..bytes]).to_string();
                                    mut_state.srvc.notifications_buffer.push_str(&note_str);
                                    // Each recv() returns a complete datagram from sd_notify.
                                    // Datagrams use '\n' to separate key=value pairs internally,
                                    // but may not end with '\n'.  If we don't add a separator,
                                    // the last key=value of one datagram merges with the first
                                    // key=value of the next (e.g. "FDNAME=inotifyREADY=1"),
                                    // causing READY=1 to never be parsed.
                                    if !note_str.ends_with('\n') {
                                        mut_state.srvc.notifications_buffer.push('\n');
                                    }

                                    // Process text notifications first so FDNAME= is parsed
                                    // before we handle the received FDs.
                                    crate::notification_handler::handle_notifications_from_buffer(
                                        &mut mut_state.srvc,
                                        &srvc_unit.id.name,
                                    );

                                    // Now handle FDSTORE with any received file descriptors.
                                    if !received_fds.is_empty() {
                                        handle_received_fds(
                                            &note_str,
                                            received_fds,
                                            &mut mut_state.srvc,
                                            &srvc_unit.id.name,
                                            &srvc_unit.specific,
                                        );
                                    }
                                }
                                Err(nix::errno::Errno::EAGAIN) => {
                                    // No data available — normal for non-blocking.
                                    // (EWOULDBLOCK is the same value as EAGAIN on Linux.)
                                }
                                Err(e) => {
                                    warn!(
                                        "Error receiving notification for {}: {e}",
                                        srvc_unit.id.name
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                if crate::shutdown::is_shutting_down() {
                    trace!("Notification handler exiting: shutdown in progress ({e})");
                    return;
                }
                warn!("Error while selecting: {e}");
            }
        }
    }
}

/// Handle file descriptors received via SCM_RIGHTS alongside sd_notify messages.
///
/// When a service sends `FDSTORE=1` with file descriptors via `sd_pid_notify_with_fds()`,
/// those FDs arrive as SCM_RIGHTS ancillary data on the notification socket. The text
/// message contains `FDSTORE=1` and optionally `FDNAME=<name>` to label the stored FDs.
///
/// `FDSTOREREMOVE=1` with `FDNAME=<name>` removes previously stored FDs by name.
///
/// The `specific` parameter is used to look up `FileDescriptorStoreMax=`.
fn handle_received_fds(
    msg_text: &str,
    received_fds: Vec<RawFd>,
    srvc: &mut Service,
    name: &str,
    specific: &Specific,
) {
    let file_descriptor_store_max = if let Specific::Service(svc_specific) = specific {
        svc_specific.conf.file_descriptor_store_max
    } else {
        0
    };
    handle_received_fds_impl(
        msg_text,
        received_fds,
        srvc,
        name,
        file_descriptor_store_max,
    );
}

/// Inner implementation that takes `file_descriptor_store_max` directly.
/// This is used by tests to avoid constructing a full `Specific`.
#[cfg(test)]
pub fn handle_received_fds_with_max(
    msg_text: &str,
    received_fds: Vec<RawFd>,
    srvc: &mut Service,
    name: &str,
    file_descriptor_store_max: u64,
) {
    handle_received_fds_impl(
        msg_text,
        received_fds,
        srvc,
        name,
        file_descriptor_store_max,
    );
}

fn handle_received_fds_impl(
    msg_text: &str,
    received_fds: Vec<RawFd>,
    srvc: &mut Service,
    name: &str,
    file_descriptor_store_max: u64,
) {
    // Parse the datagram for FDSTORE, FDSTOREREMOVE, and FDNAME directives.
    let mut fdstore = false;
    let mut fdstoreremove = false;
    let mut fdname: Option<String> = None;

    for line in msg_text.lines() {
        let parts: Vec<&str> = line.splitn(2, '=').collect();
        if parts.len() < 2 {
            continue;
        }
        match parts[0] {
            "FDSTORE" => {
                fdstore = parts[1] == "1";
            }
            "FDSTOREREMOVE" => {
                fdstoreremove = parts[1] == "1";
            }
            "FDNAME" => {
                let n = parts[1].trim();
                if !n.is_empty() {
                    fdname = Some(n.to_owned());
                }
            }
            _ => {}
        }
    }

    let fd_label = fdname.unwrap_or_else(|| "stored".to_owned());

    if fdstoreremove {
        // FDSTOREREMOVE=1: remove all stored FDs matching the given name.
        let before = srvc.stored_fds.len();
        srvc.stored_fds.retain(|(n, fd)| {
            if n == &fd_label {
                // Close the FD before dropping it.
                let _ = nix::unistd::close(*fd);
                false
            } else {
                true
            }
        });
        let removed = before - srvc.stored_fds.len();
        trace!("Service {name}: FDSTOREREMOVE=1 FDNAME={fd_label} — removed {removed} fd(s)");
        // Also close the FDs that arrived with this message (they were
        // delivered alongside the remove request, which is unusual but we
        // should not leak them).
        for fd in &received_fds {
            let _ = nix::unistd::close(*fd);
        }
        return;
    }

    if fdstore {
        let max_fds = file_descriptor_store_max;

        if max_fds == 0 {
            trace!(
                "Service {name}: FDSTORE=1 received but FileDescriptorStoreMax=0, \
                 closing {} fd(s)",
                received_fds.len()
            );
            for fd in &received_fds {
                let _ = nix::unistd::close(*fd);
            }
            return;
        }

        let current_count = srvc.stored_fds.len() as u64;
        let space = max_fds.saturating_sub(current_count) as usize;

        if space == 0 {
            trace!(
                "Service {name}: FDSTORE=1 received but store is full \
                 ({current_count}/{max_fds}), closing {} fd(s)",
                received_fds.len()
            );
            for fd in &received_fds {
                let _ = nix::unistd::close(*fd);
            }
            return;
        }

        let to_store = received_fds.len().min(space);
        for (i, fd) in received_fds.iter().enumerate() {
            if i < to_store {
                srvc.stored_fds.push((fd_label.clone(), *fd));
                trace!(
                    "Service {name}: FDSTORE=1 FDNAME={fd_label} — stored fd {fd} \
                     ({}/{})",
                    srvc.stored_fds.len(),
                    max_fds
                );
            } else {
                // Over the limit — close excess FDs.
                trace!("Service {name}: FDSTORE=1 — store full, closing excess fd {fd}");
                let _ = nix::unistd::close(*fd);
            }
        }
    } else {
        // FDs received without FDSTORE=1 — close them to avoid leaking.
        if !received_fds.is_empty() {
            trace!(
                "Service {name}: received {} fd(s) without FDSTORE=1, closing",
                received_fds.len()
            );
            for fd in &received_fds {
                let _ = nix::unistd::close(*fd);
            }
        }
    }
}

pub fn handle_all_std_out(run_info: ArcMutRuntimeInfo) {
    let eventfd = { run_info.read_poisoned().stdout_eventfd };
    loop {
        if crate::shutdown::is_shutting_down() {
            trace!("Stdout handler exiting: shutdown in progress");
            return;
        }
        let fd_to_srvc_id = collect_from_srvc(run_info.clone(), |map, srvc, id| {
            if let Some(StdIo::Piped(r, _w)) = &srvc.stdout {
                map.insert(*r, id);
            }
        });

        let mut fdset = nix::sys::select::FdSet::new();
        for fd in fd_to_srvc_id.keys() {
            fdset.insert(unsafe { borrow_fd(*fd) });
        }
        fdset.insert(unsafe { borrow_fd(eventfd.read_end()) });

        let result = nix::sys::select::select(None, Some(&mut fdset), None, None, None);

        let run_info_locked = run_info.read_poisoned();
        let unit_table = &run_info_locked.unit_table;
        match result {
            Ok(_) => {
                if fdset.contains(unsafe { borrow_fd(eventfd.read_end()) }) {
                    trace!("Interrupted stdout select because the eventfd fired");
                    reset_event_fd(eventfd);
                    trace!("Reset eventfd value");
                }
                let mut buf = [0u8; 512];
                let mut eof_ids = Vec::new();
                for (fd, id) in &fd_to_srvc_id {
                    if fdset.contains(unsafe { borrow_fd(*fd) })
                        && let Some(srvc_unit) = unit_table.get(id)
                    {
                        let name = srvc_unit.id.name.clone();
                        if let Specific::Service(srvc) = &srvc_unit.specific {
                            let mut_state = &mut *srvc.state.write_poisoned();
                            let status = srvc_unit.common.status.read_poisoned();

                            let old_flags = nix::fcntl::fcntl(
                                unsafe { borrow_fd(*fd) },
                                nix::fcntl::FcntlArg::F_GETFL,
                            )
                            .unwrap();
                            let old_flags = nix::fcntl::OFlag::from_bits(old_flags).unwrap();
                            let mut new_flags = old_flags;
                            new_flags.insert(nix::fcntl::OFlag::O_NONBLOCK);
                            nix::fcntl::fcntl(
                                unsafe { borrow_fd(*fd) },
                                nix::fcntl::FcntlArg::F_SETFL(new_flags),
                            )
                            .unwrap();

                            ////
                            let bytes =
                                match nix::unistd::read(unsafe { borrow_fd(*fd) }, &mut buf[..]) {
                                    Ok(b) => b,
                                    Err(nix::Error::EWOULDBLOCK) => 0,
                                    Err(e) => panic!("{}", e),
                                };
                            ////

                            nix::fcntl::fcntl(
                                unsafe { borrow_fd(*fd) },
                                nix::fcntl::FcntlArg::F_SETFL(old_flags),
                            )
                            .unwrap();

                            if bytes == 0 {
                                // EOF: the write end of the pipe was closed.
                                // This happens when a service redirects its
                                // stdout to a TTY or file (e.g. debug-shell).
                                // Mark for cleanup to avoid a busy select loop.
                                eof_ids.push((id.clone(), name));
                            } else {
                                mut_state.srvc.stdout_buffer.extend(&buf[..bytes]);
                                mut_state.srvc.log_stdout_lines(&name, &status).unwrap();
                            }
                        }
                    }
                }
                // Close pipes that hit EOF so they are no longer selected.
                for (id, name) in eof_ids {
                    if let Some(srvc_unit) = unit_table.get(&id)
                        && let Specific::Service(srvc) = &srvc_unit.specific
                    {
                        let mut_state = &mut *srvc.state.write_poisoned();
                        if let Some(StdIo::Piped(r, _w)) = &mut_state.srvc.stdout {
                            trace!("stdout pipe EOF for service {name}, closing read end");
                            let _ = nix::unistd::close(*r);
                        }
                        mut_state.srvc.stdout = None;
                    }
                }
            }
            Err(e) => {
                if crate::shutdown::is_shutting_down() {
                    trace!("Stdout handler exiting: shutdown in progress ({e})");
                    return;
                }
                warn!("Error while selecting: {e}");
            }
        }
    }
}

pub fn handle_all_std_err(run_info: ArcMutRuntimeInfo) {
    let eventfd = { run_info.read_poisoned().stderr_eventfd };
    loop {
        if crate::shutdown::is_shutting_down() {
            trace!("Stderr handler exiting: shutdown in progress");
            return;
        }
        let fd_to_srvc_id = collect_from_srvc(run_info.clone(), |map, srvc, id| {
            if let Some(StdIo::Piped(r, _w)) = &srvc.stderr {
                map.insert(*r, id);
            }
        });

        let mut fdset = nix::sys::select::FdSet::new();
        for fd in fd_to_srvc_id.keys() {
            fdset.insert(unsafe { borrow_fd(*fd) });
        }
        fdset.insert(unsafe { borrow_fd(eventfd.read_end()) });

        let result = nix::sys::select::select(None, Some(&mut fdset), None, None, None);
        let run_info_locked = run_info.read_poisoned();
        let unit_table = &run_info_locked.unit_table;

        match result {
            Ok(_) => {
                if fdset.contains(unsafe { borrow_fd(eventfd.read_end()) }) {
                    trace!("Interrupted stderr select because the eventfd fired");
                    reset_event_fd(eventfd);
                    trace!("Reset eventfd value");
                }
                let mut buf = [0u8; 512];
                let mut eof_ids = Vec::new();
                for (fd, id) in &fd_to_srvc_id {
                    if fdset.contains(unsafe { borrow_fd(*fd) })
                        && let Some(srvc_unit) = unit_table.get(id)
                    {
                        let name = srvc_unit.id.name.clone();
                        if let Specific::Service(srvc) = &srvc_unit.specific {
                            let mut_state = &mut *srvc.state.write_poisoned();
                            let status = srvc_unit.common.status.read_poisoned();

                            let old_flags = nix::fcntl::fcntl(
                                unsafe { borrow_fd(*fd) },
                                nix::fcntl::FcntlArg::F_GETFL,
                            )
                            .unwrap();
                            let old_flags = nix::fcntl::OFlag::from_bits(old_flags).unwrap();
                            let mut new_flags = old_flags;
                            new_flags.insert(nix::fcntl::OFlag::O_NONBLOCK);
                            nix::fcntl::fcntl(
                                unsafe { borrow_fd(*fd) },
                                nix::fcntl::FcntlArg::F_SETFL(new_flags),
                            )
                            .unwrap();

                            ////
                            let bytes =
                                match nix::unistd::read(unsafe { borrow_fd(*fd) }, &mut buf[..]) {
                                    Ok(b) => b,
                                    Err(nix::Error::EWOULDBLOCK) => 0,
                                    Err(e) => panic!("{}", e),
                                };
                            ////
                            nix::fcntl::fcntl(
                                unsafe { borrow_fd(*fd) },
                                nix::fcntl::FcntlArg::F_SETFL(old_flags),
                            )
                            .unwrap();

                            if bytes == 0 {
                                // EOF: the write end of the pipe was closed.
                                // This happens when a service redirects its
                                // stderr to a TTY or file (e.g. debug-shell).
                                // Mark for cleanup to avoid a busy select loop.
                                eof_ids.push((id.clone(), name));
                            } else {
                                mut_state.srvc.stderr_buffer.extend(&buf[..bytes]);
                                mut_state.srvc.log_stderr_lines(&name, &status).unwrap();
                            }
                        }
                    }
                }
                // Close pipes that hit EOF so they are no longer selected.
                for (id, name) in eof_ids {
                    if let Some(srvc_unit) = unit_table.get(&id)
                        && let Specific::Service(srvc) = &srvc_unit.specific
                    {
                        let mut_state = &mut *srvc.state.write_poisoned();
                        if let Some(StdIo::Piped(r, _w)) = &mut_state.srvc.stderr {
                            trace!("stderr pipe EOF for service {name}, closing read end");
                            let _ = nix::unistd::close(*r);
                        }
                        mut_state.srvc.stderr = None;
                    }
                }
            }
            Err(e) => {
                if crate::shutdown::is_shutting_down() {
                    trace!("Stderr handler exiting: shutdown in progress ({e})");
                    return;
                }
                warn!("Error while selecting: {e}");
            }
        }
    }
}

pub fn handle_notification_message(msg: &str, srvc: &mut Service, name: &str) {
    let split: Vec<_> = msg.splitn(2, '=').collect();
    if split.is_empty() {
        return;
    }
    match split[0] {
        "STATUS" => {
            if split.len() > 1 {
                srvc.status_msgs.push(split[1].to_owned());
                trace!(
                    "New status message pushed from service {}: {}",
                    name,
                    srvc.status_msgs.last().unwrap()
                );
            }
        }
        "READY" => {
            srvc.signaled_ready = true;
            // Initialize the watchdog reference timestamp so that the
            // watchdog enforcement thread starts counting from the moment
            // the service signals readiness (matching real systemd).
            if srvc.watchdog_last_ping.is_none() {
                srvc.watchdog_last_ping = Some(std::time::Instant::now());
            }
            // READY=1 after RELOADING=1 means reload is complete
            if srvc.reloading {
                srvc.reloading = false;
                trace!("Service {name}: reload complete (READY=1 after RELOADING=1)");
            }
        }
        "RELOADING" => {
            if split.len() > 1 && split[1] == "1" {
                srvc.reloading = true;
                // Per sd_notify(3), RELOADING=1 implies the service is not
                // ready during reload. The service will send READY=1 again
                // when reload completes.
                srvc.signaled_ready = false;
                trace!("Service {name}: entering reload state (RELOADING=1)");
            }
        }
        "STOPPING" => {
            if split.len() > 1 && split[1] == "1" {
                srvc.stopping = true;
                trace!("Service {name}: entering stopping state (STOPPING=1)");
            }
        }
        "MAINPID" => {
            if split.len() > 1 {
                match split[1].parse::<i32>() {
                    Ok(pid) if pid > 0 => {
                        let new_pid = nix::unistd::Pid::from_raw(pid);
                        trace!(
                            "Service {name}: MAINPID updated to {} (was {:?})",
                            pid, srvc.main_pid
                        );
                        srvc.main_pid = Some(new_pid);
                    }
                    Ok(pid) => {
                        trace!("Service {name}: ignoring invalid MAINPID={pid}");
                    }
                    Err(e) => {
                        trace!(
                            "Service {name}: ignoring unparsable MAINPID={}: {e}",
                            split[1]
                        );
                    }
                }
            }
        }
        "WATCHDOG" => {
            if split.len() > 1 {
                match split[1] {
                    "1" => {
                        srvc.watchdog_last_ping = Some(std::time::Instant::now());
                        trace!("Service {name}: watchdog ping received (WATCHDOG=1)");
                    }
                    "trigger" => {
                        // The service is requesting that the watchdog action be
                        // triggered immediately (e.g. because it detected an
                        // internal error). We log this; the watchdog checker
                        // thread will act on it.
                        warn!("Service {name}: watchdog trigger requested (WATCHDOG=trigger)");
                        // Clear the last ping so the watchdog checker sees it
                        // as expired immediately.
                        srvc.watchdog_last_ping = None;
                    }
                    other => {
                        trace!("Service {name}: ignoring unknown WATCHDOG={other}");
                    }
                }
            }
        }
        "WATCHDOG_USEC" => {
            if split.len() > 1 {
                match split[1].trim().parse::<u64>() {
                    Ok(usec) => {
                        let old = srvc.watchdog_usec_override;
                        srvc.watchdog_usec_override = Some(usec);
                        // Also reset the ping timestamp so the new timeout
                        // starts fresh from this moment.
                        srvc.watchdog_last_ping = Some(std::time::Instant::now());
                        trace!(
                            "Service {name}: WATCHDOG_USEC={usec} (was {old:?}), \
                             watchdog timeout updated dynamically"
                        );
                    }
                    Err(e) => {
                        trace!(
                            "Service {name}: ignoring unparsable WATCHDOG_USEC={}: {e}",
                            split[1]
                        );
                    }
                }
            }
        }
        "ERRNO" => {
            if split.len() > 1 {
                match split[1].trim().parse::<i32>() {
                    Ok(errno) => {
                        srvc.notify_errno = Some(errno);
                        trace!("Service {name}: ERRNO={errno}");
                    }
                    Err(e) => {
                        trace!(
                            "Service {name}: ignoring unparsable ERRNO={}: {e}",
                            split[1]
                        );
                    }
                }
            }
        }
        "BUSERROR" => {
            if split.len() > 1 {
                let error = split[1].trim();
                if !error.is_empty() {
                    srvc.notify_bus_error = Some(error.to_owned());
                    trace!("Service {name}: BUSERROR={error}");
                }
            }
        }
        "EXIT_STATUS" => {
            if split.len() > 1 {
                let status = split[1].trim();
                if !status.is_empty() {
                    srvc.notify_exit_status = Some(status.to_owned());
                    trace!("Service {name}: EXIT_STATUS={status}");
                }
            }
        }
        "MONOTONIC_USEC" => {
            if split.len() > 1 {
                match split[1].trim().parse::<u64>() {
                    Ok(usec) => {
                        srvc.notify_monotonic_usec = Some(usec);
                        trace!("Service {name}: MONOTONIC_USEC={usec}");
                    }
                    Err(e) => {
                        trace!(
                            "Service {name}: ignoring unparsable MONOTONIC_USEC={}: {e}",
                            split[1]
                        );
                    }
                }
            }
        }
        "INVOCATION_ID" => {
            if split.len() > 1 {
                let id = split[1].trim();
                if !id.is_empty() {
                    srvc.invocation_id = Some(id.to_owned());
                    trace!("Service {name}: INVOCATION_ID={id}");
                }
            }
        }
        // Known sd_notify fields we accept but don't fully implement yet.
        // FDSTORE=/FDSTOREREMOVE=/FDNAME= require SCM_RIGHTS ancillary data
        // from recvmsg() which is handled separately in handle_all_streams.
        // If they arrive here (from the text portion), just log them.
        "FDSTORE" | "FDSTOREREMOVE" | "FDNAME" => {
            trace!("Service {name}: sd_notify {msg} (fd store handled in stream receiver)");
        }
        "NOTIFYACCESS" => {
            // NOTIFYACCESS= from sd_notify allows a service to change its
            // own NotifyAccess= setting at runtime (see sd_notify(3)).
            if split.len() > 1 {
                let value = split[1].trim();
                match value.to_lowercase().as_str() {
                    "none" => {
                        srvc.notify_access_override = Some(NotifyKind::None);
                        trace!("Service {name}: NOTIFYACCESS=none (runtime override applied)");
                    }
                    "main" => {
                        srvc.notify_access_override = Some(NotifyKind::Main);
                        trace!("Service {name}: NOTIFYACCESS=main (runtime override applied)");
                    }
                    "exec" => {
                        srvc.notify_access_override = Some(NotifyKind::Exec);
                        trace!("Service {name}: NOTIFYACCESS=exec (runtime override applied)");
                    }
                    "all" => {
                        srvc.notify_access_override = Some(NotifyKind::All);
                        trace!("Service {name}: NOTIFYACCESS=all (runtime override applied)");
                    }
                    other => {
                        trace!("Service {name}: ignoring unknown NOTIFYACCESS={other}");
                    }
                }
            }
        }
        "EXTEND_TIMEOUT_USEC" => {
            if split.len() > 1 {
                match split[1].trim().parse::<u64>() {
                    Ok(usec) => {
                        srvc.extend_timeout_usec = Some(usec);
                        srvc.extend_timeout_timestamp = Some(std::time::Instant::now());
                        trace!("Service {name}: EXTEND_TIMEOUT_USEC={usec} — timeout extended");
                    }
                    Err(e) => {
                        trace!(
                            "Service {name}: ignoring unparsable EXTEND_TIMEOUT_USEC={}: {e}",
                            split[1]
                        );
                    }
                }
            }
        }
        _ => {
            warn!("Unknown notification from service {name}: {msg}");
        }
    }
}

pub fn handle_notifications_from_buffer(srvc: &mut Service, name: &str) {
    while srvc.notifications_buffer.contains('\n') {
        let (line, rest) = srvc
            .notifications_buffer
            .split_at(srvc.notifications_buffer.find('\n').unwrap());
        let line = line.to_owned();
        srvc.notifications_buffer = rest[1..].to_owned();

        handle_notification_message(&line, srvc, name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::Service;

    fn make_test_service() -> Service {
        Service {
            pid: None,
            main_pid: None,
            status_msgs: Vec::new(),
            process_group: None,
            signaled_ready: false,
            reloading: false,
            stopping: false,
            watchdog_last_ping: None,
            notify_errno: None,
            notify_bus_error: None,
            notify_exit_status: None,
            notify_monotonic_usec: None,
            invocation_id: None,
            watchdog_usec_override: None,
            stored_fds: Vec::new(),
            notify_access_override: None,
            notifications: None,
            notifications_path: None,
            stdout: None,
            stderr: None,
            notifications_buffer: String::new(),
            stdout_buffer: Vec::new(),
            stderr_buffer: Vec::new(),
            watchdog_timeout_fired: false,
            runtime_max_timeout_fired: false,
            runtime_started_at: None,
            main_exit_status: None,
            main_exit_pid: None,
            trigger_path: None,
            trigger_unit: None,
            trigger_timer_realtime_usec: None,
            trigger_timer_monotonic_usec: None,
            monitor_env: None,
        }
    }

    #[test]
    fn test_ready_sets_signaled_ready() {
        let mut srvc = make_test_service();
        handle_notification_message("READY=1", &mut srvc, "test.service");
        assert!(srvc.signaled_ready);
    }

    #[test]
    fn test_status_pushes_message() {
        let mut srvc = make_test_service();
        handle_notification_message("STATUS=Starting up...", &mut srvc, "test.service");
        assert_eq!(srvc.status_msgs.len(), 1);
        assert_eq!(srvc.status_msgs[0], "Starting up...");
    }

    #[test]
    fn test_status_multiple_messages() {
        let mut srvc = make_test_service();
        handle_notification_message("STATUS=Starting", &mut srvc, "test.service");
        handle_notification_message("STATUS=Ready", &mut srvc, "test.service");
        assert_eq!(srvc.status_msgs.len(), 2);
        assert_eq!(srvc.status_msgs[0], "Starting");
        assert_eq!(srvc.status_msgs[1], "Ready");
    }

    #[test]
    fn test_mainpid_valid() {
        let mut srvc = make_test_service();
        handle_notification_message("MAINPID=12345", &mut srvc, "test.service");
        assert_eq!(srvc.main_pid, Some(nix::unistd::Pid::from_raw(12345)));
    }

    #[test]
    fn test_mainpid_updates_existing() {
        let mut srvc = make_test_service();
        srvc.main_pid = Some(nix::unistd::Pid::from_raw(100));
        handle_notification_message("MAINPID=200", &mut srvc, "test.service");
        assert_eq!(srvc.main_pid, Some(nix::unistd::Pid::from_raw(200)));
    }

    #[test]
    fn test_mainpid_zero_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("MAINPID=0", &mut srvc, "test.service");
        assert_eq!(srvc.main_pid, None);
    }

    #[test]
    fn test_mainpid_negative_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("MAINPID=-1", &mut srvc, "test.service");
        assert_eq!(srvc.main_pid, None);
    }

    #[test]
    fn test_mainpid_invalid_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("MAINPID=notanumber", &mut srvc, "test.service");
        assert_eq!(srvc.main_pid, None);
    }

    #[test]
    fn test_mainpid_no_value_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("MAINPID", &mut srvc, "test.service");
        assert_eq!(srvc.main_pid, None);
    }

    #[test]
    fn test_watchdog_ping() {
        let mut srvc = make_test_service();
        assert!(srvc.watchdog_last_ping.is_none());
        handle_notification_message("WATCHDOG=1", &mut srvc, "test.service");
        assert!(srvc.watchdog_last_ping.is_some());
    }

    #[test]
    fn test_watchdog_trigger_clears_ping() {
        let mut srvc = make_test_service();
        srvc.watchdog_last_ping = Some(std::time::Instant::now());
        handle_notification_message("WATCHDOG=trigger", &mut srvc, "test.service");
        assert!(srvc.watchdog_last_ping.is_none());
    }

    #[test]
    fn test_watchdog_unknown_value_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("WATCHDOG=0", &mut srvc, "test.service");
        assert!(srvc.watchdog_last_ping.is_none());
    }

    #[test]
    fn test_reloading_sets_flag() {
        let mut srvc = make_test_service();
        srvc.signaled_ready = true;
        handle_notification_message("RELOADING=1", &mut srvc, "test.service");
        assert!(srvc.reloading);
        // RELOADING=1 clears signaled_ready
        assert!(!srvc.signaled_ready);
    }

    #[test]
    fn test_reloading_then_ready_clears_reload() {
        let mut srvc = make_test_service();
        handle_notification_message("RELOADING=1", &mut srvc, "test.service");
        assert!(srvc.reloading);
        assert!(!srvc.signaled_ready);

        handle_notification_message("READY=1", &mut srvc, "test.service");
        assert!(!srvc.reloading);
        assert!(srvc.signaled_ready);
    }

    #[test]
    fn test_stopping_sets_flag() {
        let mut srvc = make_test_service();
        handle_notification_message("STOPPING=1", &mut srvc, "test.service");
        assert!(srvc.stopping);
    }

    #[test]
    fn test_stopping_zero_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("STOPPING=0", &mut srvc, "test.service");
        assert!(!srvc.stopping);
    }

    #[test]
    fn test_watchdog_usec_does_not_crash() {
        let mut srvc = make_test_service();
        handle_notification_message("WATCHDOG_USEC=30000000", &mut srvc, "test.service");
        // Just verifying no panic or crash
    }

    #[test]
    fn test_unknown_notification_does_not_crash() {
        let mut srvc = make_test_service();
        handle_notification_message("CUSTOM_FIELD=value", &mut srvc, "test.service");
        // Should not crash, just warn
    }

    #[test]
    fn test_empty_message_does_not_crash() {
        let mut srvc = make_test_service();
        handle_notification_message("", &mut srvc, "test.service");
    }

    #[test]
    fn test_fdstore_does_not_crash() {
        let mut srvc = make_test_service();
        handle_notification_message("FDSTORE=1", &mut srvc, "test.service");
        handle_notification_message("FDNAME=myfd", &mut srvc, "test.service");
        handle_notification_message("FDSTOREREMOVE=1", &mut srvc, "test.service");
    }

    #[test]
    fn test_errno_does_not_crash() {
        let mut srvc = make_test_service();
        handle_notification_message("ERRNO=2", &mut srvc, "test.service");
        handle_notification_message(
            "BUSERROR=org.freedesktop.DBus.Error.Failed",
            &mut srvc,
            "test.service",
        );
        handle_notification_message("EXIT_STATUS=1", &mut srvc, "test.service");
    }

    #[test]
    fn test_full_lifecycle_with_error_fields() {
        let mut srvc = make_test_service();

        // Service starts and signals ready
        handle_notification_message("READY=1", &mut srvc, "test.service");
        assert!(srvc.signaled_ready);

        // Service reports an error via ERRNO + BUSERROR
        handle_notification_message("ERRNO=11", &mut srvc, "test.service");
        handle_notification_message(
            "BUSERROR=org.freedesktop.DBus.Error.NoMemory",
            &mut srvc,
            "test.service",
        );
        assert_eq!(srvc.notify_errno, Some(11));
        assert_eq!(
            srvc.notify_bus_error.as_deref(),
            Some("org.freedesktop.DBus.Error.NoMemory")
        );
        // Service is still ready despite the error report
        assert!(srvc.signaled_ready);

        // Service stops and reports exit status
        handle_notification_message("STOPPING=1", &mut srvc, "test.service");
        handle_notification_message("EXIT_STATUS=0", &mut srvc, "test.service");
        assert!(srvc.stopping);
        assert_eq!(srvc.notify_exit_status.as_deref(), Some("0"));
    }

    #[test]
    fn test_watchdog_usec_resets_ping() {
        let mut srvc = make_test_service();
        // Initially no ping and no override
        assert!(srvc.watchdog_last_ping.is_none());
        assert!(srvc.watchdog_usec_override.is_none());

        // WATCHDOG_USEC= should set override AND reset the ping
        handle_notification_message("WATCHDOG_USEC=3000000", &mut srvc, "test.service");
        assert_eq!(srvc.watchdog_usec_override, Some(3_000_000));
        let first_ping = srvc.watchdog_last_ping.unwrap();

        // A regular watchdog ping should update the timestamp
        std::thread::sleep(std::time::Duration::from_millis(5));
        handle_notification_message("WATCHDOG=1", &mut srvc, "test.service");
        let second_ping = srvc.watchdog_last_ping.unwrap();
        assert!(second_ping > first_ping);
    }

    #[test]
    fn test_handle_notifications_from_buffer() {
        let mut srvc = make_test_service();
        srvc.notifications_buffer = "READY=1\nSTATUS=Running\n".to_owned();
        handle_notifications_from_buffer(&mut srvc, "test.service");
        assert!(srvc.signaled_ready);
        assert_eq!(srvc.status_msgs.len(), 1);
        assert_eq!(srvc.status_msgs[0], "Running");
        assert!(srvc.notifications_buffer.is_empty());
    }

    #[test]
    fn test_handle_notifications_from_buffer_partial() {
        let mut srvc = make_test_service();
        // Buffer with a complete line and an incomplete one
        srvc.notifications_buffer = "READY=1\nSTAT".to_owned();
        handle_notifications_from_buffer(&mut srvc, "test.service");
        assert!(srvc.signaled_ready);
        // The incomplete "STAT" should remain in the buffer
        assert_eq!(srvc.notifications_buffer, "STAT");
    }

    #[test]
    fn test_handle_notifications_from_buffer_mainpid_and_watchdog() {
        let mut srvc = make_test_service();
        srvc.notifications_buffer = "MAINPID=42\nWATCHDOG=1\n".to_owned();
        handle_notifications_from_buffer(&mut srvc, "test.service");
        assert_eq!(srvc.main_pid, Some(nix::unistd::Pid::from_raw(42)));
        assert!(srvc.watchdog_last_ping.is_some());
    }

    #[test]
    fn test_full_lifecycle_notify_reload_ready() {
        let mut srvc = make_test_service();

        // Service starts and signals ready
        handle_notification_message("READY=1", &mut srvc, "test.service");
        assert!(srvc.signaled_ready);
        assert!(!srvc.reloading);

        // Service begins reload
        handle_notification_message("RELOADING=1", &mut srvc, "test.service");
        assert!(srvc.reloading);
        assert!(!srvc.signaled_ready);

        // Service finishes reload
        handle_notification_message("READY=1", &mut srvc, "test.service");
        assert!(!srvc.reloading);
        assert!(srvc.signaled_ready);

        // Service begins stopping
        handle_notification_message("STOPPING=1", &mut srvc, "test.service");
        assert!(srvc.stopping);
    }

    #[test]
    fn test_watchdog_multiple_pings() {
        let mut srvc = make_test_service();

        handle_notification_message("WATCHDOG=1", &mut srvc, "test.service");
        let first_ping = srvc.watchdog_last_ping.unwrap();

        // Small delay to ensure distinct timestamps
        std::thread::sleep(std::time::Duration::from_millis(10));

        handle_notification_message("WATCHDOG=1", &mut srvc, "test.service");
        let second_ping = srvc.watchdog_last_ping.unwrap();

        assert!(second_ping >= first_ping);
    }

    // ── ERRNO= tests ─────────────────────────────────────────────────

    #[test]
    fn test_errno_valid() {
        let mut srvc = make_test_service();
        handle_notification_message("ERRNO=2", &mut srvc, "test.service");
        assert_eq!(srvc.notify_errno, Some(2));
    }

    #[test]
    fn test_errno_zero() {
        let mut srvc = make_test_service();
        handle_notification_message("ERRNO=0", &mut srvc, "test.service");
        assert_eq!(srvc.notify_errno, Some(0));
    }

    #[test]
    fn test_errno_updates_existing() {
        let mut srvc = make_test_service();
        handle_notification_message("ERRNO=2", &mut srvc, "test.service");
        assert_eq!(srvc.notify_errno, Some(2));
        handle_notification_message("ERRNO=11", &mut srvc, "test.service");
        assert_eq!(srvc.notify_errno, Some(11));
    }

    #[test]
    fn test_errno_invalid_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("ERRNO=abc", &mut srvc, "test.service");
        assert_eq!(srvc.notify_errno, None);
    }

    #[test]
    fn test_errno_no_value_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("ERRNO", &mut srvc, "test.service");
        assert_eq!(srvc.notify_errno, None);
    }

    #[test]
    fn test_errno_negative() {
        let mut srvc = make_test_service();
        handle_notification_message("ERRNO=-1", &mut srvc, "test.service");
        assert_eq!(srvc.notify_errno, Some(-1));
    }

    #[test]
    fn test_errno_whitespace_trimmed() {
        let mut srvc = make_test_service();
        handle_notification_message("ERRNO= 22 ", &mut srvc, "test.service");
        assert_eq!(srvc.notify_errno, Some(22));
    }

    // ── BUSERROR= tests ──────────────────────────────────────────────

    #[test]
    fn test_buserror_valid() {
        let mut srvc = make_test_service();
        handle_notification_message(
            "BUSERROR=org.freedesktop.DBus.Error.TimedOut",
            &mut srvc,
            "test.service",
        );
        assert_eq!(
            srvc.notify_bus_error.as_deref(),
            Some("org.freedesktop.DBus.Error.TimedOut")
        );
    }

    #[test]
    fn test_buserror_updates_existing() {
        let mut srvc = make_test_service();
        handle_notification_message(
            "BUSERROR=org.freedesktop.DBus.Error.NoMemory",
            &mut srvc,
            "test.service",
        );
        handle_notification_message(
            "BUSERROR=org.freedesktop.DBus.Error.Failed",
            &mut srvc,
            "test.service",
        );
        assert_eq!(
            srvc.notify_bus_error.as_deref(),
            Some("org.freedesktop.DBus.Error.Failed")
        );
    }

    #[test]
    fn test_buserror_empty_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("BUSERROR=", &mut srvc, "test.service");
        assert_eq!(srvc.notify_bus_error, None);
    }

    #[test]
    fn test_buserror_no_value_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("BUSERROR", &mut srvc, "test.service");
        assert_eq!(srvc.notify_bus_error, None);
    }

    // ── EXIT_STATUS= tests ───────────────────────────────────────────

    #[test]
    fn test_exit_status_numeric() {
        let mut srvc = make_test_service();
        handle_notification_message("EXIT_STATUS=0", &mut srvc, "test.service");
        assert_eq!(srvc.notify_exit_status.as_deref(), Some("0"));
    }

    #[test]
    fn test_exit_status_signal_name() {
        let mut srvc = make_test_service();
        handle_notification_message("EXIT_STATUS=SIGTERM", &mut srvc, "test.service");
        assert_eq!(srvc.notify_exit_status.as_deref(), Some("SIGTERM"));
    }

    #[test]
    fn test_exit_status_updates_existing() {
        let mut srvc = make_test_service();
        handle_notification_message("EXIT_STATUS=0", &mut srvc, "test.service");
        handle_notification_message("EXIT_STATUS=1", &mut srvc, "test.service");
        assert_eq!(srvc.notify_exit_status.as_deref(), Some("1"));
    }

    #[test]
    fn test_exit_status_empty_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("EXIT_STATUS=", &mut srvc, "test.service");
        assert_eq!(srvc.notify_exit_status, None);
    }

    // ── MONOTONIC_USEC= tests ────────────────────────────────────────

    #[test]
    fn test_monotonic_usec_valid() {
        let mut srvc = make_test_service();
        handle_notification_message("MONOTONIC_USEC=12345678", &mut srvc, "test.service");
        assert_eq!(srvc.notify_monotonic_usec, Some(12_345_678));
    }

    #[test]
    fn test_monotonic_usec_zero() {
        let mut srvc = make_test_service();
        handle_notification_message("MONOTONIC_USEC=0", &mut srvc, "test.service");
        assert_eq!(srvc.notify_monotonic_usec, Some(0));
    }

    #[test]
    fn test_monotonic_usec_invalid_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("MONOTONIC_USEC=notanumber", &mut srvc, "test.service");
        assert_eq!(srvc.notify_monotonic_usec, None);
    }

    #[test]
    fn test_monotonic_usec_negative_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("MONOTONIC_USEC=-1", &mut srvc, "test.service");
        // u64 parse fails for negative
        assert_eq!(srvc.notify_monotonic_usec, None);
    }

    #[test]
    fn test_monotonic_usec_whitespace_trimmed() {
        let mut srvc = make_test_service();
        handle_notification_message("MONOTONIC_USEC= 999 ", &mut srvc, "test.service");
        assert_eq!(srvc.notify_monotonic_usec, Some(999));
    }

    // ── INVOCATION_ID= tests ─────────────────────────────────────────

    #[test]
    fn test_invocation_id_valid() {
        let mut srvc = make_test_service();
        handle_notification_message(
            "INVOCATION_ID=a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6",
            &mut srvc,
            "test.service",
        );
        assert_eq!(
            srvc.invocation_id.as_deref(),
            Some("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6")
        );
    }

    #[test]
    fn test_invocation_id_updates_existing() {
        let mut srvc = make_test_service();
        handle_notification_message("INVOCATION_ID=aaa", &mut srvc, "test.service");
        handle_notification_message("INVOCATION_ID=bbb", &mut srvc, "test.service");
        assert_eq!(srvc.invocation_id.as_deref(), Some("bbb"));
    }

    #[test]
    fn test_invocation_id_empty_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("INVOCATION_ID=", &mut srvc, "test.service");
        assert_eq!(srvc.invocation_id, None);
    }

    // ── WATCHDOG_USEC= tests ─────────────────────────────────────────

    #[test]
    fn test_watchdog_usec_valid() {
        let mut srvc = make_test_service();
        handle_notification_message("WATCHDOG_USEC=5000000", &mut srvc, "test.service");
        assert_eq!(srvc.watchdog_usec_override, Some(5_000_000));
        // Should also reset the ping timestamp
        assert!(srvc.watchdog_last_ping.is_some());
    }

    #[test]
    fn test_watchdog_usec_zero() {
        let mut srvc = make_test_service();
        handle_notification_message("WATCHDOG_USEC=0", &mut srvc, "test.service");
        assert_eq!(srvc.watchdog_usec_override, Some(0));
    }

    #[test]
    fn test_watchdog_usec_updates_existing() {
        let mut srvc = make_test_service();
        handle_notification_message("WATCHDOG_USEC=1000000", &mut srvc, "test.service");
        assert_eq!(srvc.watchdog_usec_override, Some(1_000_000));
        handle_notification_message("WATCHDOG_USEC=2000000", &mut srvc, "test.service");
        assert_eq!(srvc.watchdog_usec_override, Some(2_000_000));
    }

    #[test]
    fn test_watchdog_usec_invalid_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("WATCHDOG_USEC=abc", &mut srvc, "test.service");
        assert_eq!(srvc.watchdog_usec_override, None);
        assert!(srvc.watchdog_last_ping.is_none());
    }

    #[test]
    fn test_watchdog_usec_whitespace_trimmed() {
        let mut srvc = make_test_service();
        handle_notification_message("WATCHDOG_USEC= 3000000 ", &mut srvc, "test.service");
        assert_eq!(srvc.watchdog_usec_override, Some(3_000_000));
    }

    // ── NOTIFYACCESS= tests ──────────────────────────────────────────

    #[test]
    fn test_notifyaccess_all() {
        let mut srvc = make_test_service();
        assert_eq!(srvc.notify_access_override, None);
        handle_notification_message("NOTIFYACCESS=all", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, Some(NotifyKind::All));
    }

    #[test]
    fn test_notifyaccess_main() {
        let mut srvc = make_test_service();
        handle_notification_message("NOTIFYACCESS=main", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, Some(NotifyKind::Main));
    }

    #[test]
    fn test_notifyaccess_exec() {
        let mut srvc = make_test_service();
        handle_notification_message("NOTIFYACCESS=exec", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, Some(NotifyKind::Exec));
    }

    #[test]
    fn test_notifyaccess_none() {
        let mut srvc = make_test_service();
        handle_notification_message("NOTIFYACCESS=none", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, Some(NotifyKind::None));
    }

    #[test]
    fn test_notifyaccess_case_insensitive() {
        let mut srvc = make_test_service();
        handle_notification_message("NOTIFYACCESS=All", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, Some(NotifyKind::All));

        handle_notification_message("NOTIFYACCESS=MAIN", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, Some(NotifyKind::Main));

        handle_notification_message("NOTIFYACCESS=Exec", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, Some(NotifyKind::Exec));

        handle_notification_message("NOTIFYACCESS=NONE", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, Some(NotifyKind::None));
    }

    #[test]
    fn test_notifyaccess_unknown_value_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("NOTIFYACCESS=bogus", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, None);
    }

    #[test]
    fn test_notifyaccess_empty_value_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("NOTIFYACCESS=", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, None);
    }

    #[test]
    fn test_notifyaccess_no_value_ignored() {
        let mut srvc = make_test_service();
        handle_notification_message("NOTIFYACCESS", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, None);
    }

    #[test]
    fn test_notifyaccess_whitespace_trimmed() {
        let mut srvc = make_test_service();
        handle_notification_message("NOTIFYACCESS= all ", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, Some(NotifyKind::All));
    }

    #[test]
    fn test_notifyaccess_sequential_overrides() {
        let mut srvc = make_test_service();
        handle_notification_message("NOTIFYACCESS=all", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, Some(NotifyKind::All));

        handle_notification_message("NOTIFYACCESS=main", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, Some(NotifyKind::Main));

        handle_notification_message("NOTIFYACCESS=none", &mut srvc, "test.service");
        assert_eq!(srvc.notify_access_override, Some(NotifyKind::None));
    }

    #[test]
    fn test_effective_notify_access_logic() {
        use crate::services::effective_notify_access_from_parts;

        // No override — config value is returned
        assert_eq!(
            effective_notify_access_from_parts(None, NotifyKind::Main),
            NotifyKind::Main,
        );
        assert_eq!(
            effective_notify_access_from_parts(None, NotifyKind::All),
            NotifyKind::All,
        );

        // Override takes precedence over config
        assert_eq!(
            effective_notify_access_from_parts(Some(NotifyKind::All), NotifyKind::Main),
            NotifyKind::All,
        );
        assert_eq!(
            effective_notify_access_from_parts(Some(NotifyKind::None), NotifyKind::All),
            NotifyKind::None,
        );
        assert_eq!(
            effective_notify_access_from_parts(Some(NotifyKind::Exec), NotifyKind::Main),
            NotifyKind::Exec,
        );
    }

    #[test]
    fn test_notifyaccess_then_effective() {
        use crate::services::effective_notify_access_from_parts;

        let mut srvc = make_test_service();
        // Initially no override
        assert_eq!(
            effective_notify_access_from_parts(srvc.notify_access_override, NotifyKind::Main),
            NotifyKind::Main,
        );

        // After runtime override, override wins
        handle_notification_message("NOTIFYACCESS=all", &mut srvc, "test.service");
        assert_eq!(
            effective_notify_access_from_parts(srvc.notify_access_override, NotifyKind::Main),
            NotifyKind::All,
        );

        // Service can restrict its own access to none
        handle_notification_message("NOTIFYACCESS=none", &mut srvc, "test.service");
        assert_eq!(
            effective_notify_access_from_parts(srvc.notify_access_override, NotifyKind::All),
            NotifyKind::None,
        );
    }

    // ── Buffer parsing with new fields ───────────────────────────────

    #[test]
    fn test_handle_notifications_from_buffer_errno_and_buserror() {
        let mut srvc = make_test_service();
        srvc.notifications_buffer =
            "ERRNO=5\nBUSERROR=org.freedesktop.DBus.Error.Failed\n".to_owned();
        handle_notifications_from_buffer(&mut srvc, "test.service");
        assert_eq!(srvc.notify_errno, Some(5));
        assert_eq!(
            srvc.notify_bus_error.as_deref(),
            Some("org.freedesktop.DBus.Error.Failed")
        );
    }

    #[test]
    fn test_handle_notifications_from_buffer_all_new_fields() {
        let mut srvc = make_test_service();
        srvc.notifications_buffer = [
            "READY=1",
            "ERRNO=2",
            "BUSERROR=org.test.Error",
            "EXIT_STATUS=42",
            "MONOTONIC_USEC=99999",
            "INVOCATION_ID=abcdef123456",
            "WATCHDOG_USEC=1000000",
            "",
        ]
        .join("\n");
        handle_notifications_from_buffer(&mut srvc, "test.service");
        assert!(srvc.signaled_ready);
        assert_eq!(srvc.notify_errno, Some(2));
        assert_eq!(srvc.notify_bus_error.as_deref(), Some("org.test.Error"));
        assert_eq!(srvc.notify_exit_status.as_deref(), Some("42"));
        assert_eq!(srvc.notify_monotonic_usec, Some(99999));
        assert_eq!(srvc.invocation_id.as_deref(), Some("abcdef123456"));
        assert_eq!(srvc.watchdog_usec_override, Some(1_000_000));
    }

    // ── Combined lifecycle with all fields ────────────────────────────

    // ── FDSTORE tests ─────────────────────────────────────────────────

    /// Helper to create a pipe and return the read-end fd for use as a
    /// "stored" file descriptor in tests. The write-end is leaked (the OS
    /// will close it when the test process exits).
    fn make_test_fd() -> RawFd {
        use std::os::unix::io::IntoRawFd;
        let (r, _w) = nix::unistd::pipe().expect("pipe() failed in test");
        // Leak the write-end so the read-end stays valid for the test.
        let _leak = _w.into_raw_fd();
        r.into_raw_fd()
    }

    /// Shorthand for calling the inner FDSTORE handler with a given max.
    fn fdstore(msg: &str, fds: Vec<RawFd>, srvc: &mut Service, max: u64) {
        super::handle_received_fds_with_max(msg, fds, srvc, "test.service", max);
    }

    #[test]
    fn test_fdstore_basic_store() {
        let mut srvc = make_test_service();
        let fd = make_test_fd();

        fdstore("FDSTORE=1\n", vec![fd], &mut srvc, 10);
        assert_eq!(srvc.stored_fds.len(), 1);
        assert_eq!(srvc.stored_fds[0].0, "stored"); // default name
        assert_eq!(srvc.stored_fds[0].1, fd);
    }

    #[test]
    fn test_fdstore_with_fdname() {
        let mut srvc = make_test_service();
        let fd = make_test_fd();

        fdstore("FDSTORE=1\nFDNAME=myfd\n", vec![fd], &mut srvc, 10);
        assert_eq!(srvc.stored_fds.len(), 1);
        assert_eq!(srvc.stored_fds[0].0, "myfd");
    }

    #[test]
    fn test_fdstore_multiple_fds() {
        let mut srvc = make_test_service();
        let fd1 = make_test_fd();
        let fd2 = make_test_fd();

        fdstore("FDSTORE=1\nFDNAME=pair\n", vec![fd1, fd2], &mut srvc, 10);
        assert_eq!(srvc.stored_fds.len(), 2);
        assert_eq!(srvc.stored_fds[0].0, "pair");
        assert_eq!(srvc.stored_fds[1].0, "pair");
    }

    #[test]
    fn test_fdstore_max_zero_rejects() {
        let mut srvc = make_test_service();
        use std::os::unix::io::IntoRawFd;
        let (r, w) = nix::unistd::pipe().unwrap();
        let r = r.into_raw_fd();
        let _ = nix::unistd::close(w.into_raw_fd());

        fdstore("FDSTORE=1\n", vec![r], &mut srvc, 0);
        assert_eq!(srvc.stored_fds.len(), 0);
    }

    #[test]
    fn test_fdstore_max_enforced() {
        let mut srvc = make_test_service();
        let fd1 = make_test_fd();
        let fd2 = make_test_fd();

        // Store 2 (fills up)
        fdstore("FDSTORE=1\nFDNAME=a\n", vec![fd1, fd2], &mut srvc, 2);
        assert_eq!(srvc.stored_fds.len(), 2);

        // Try to store 1 more — should be rejected (closed)
        use std::os::unix::io::IntoRawFd;
        let (r, w) = nix::unistd::pipe().unwrap();
        let r = r.into_raw_fd();
        let _ = nix::unistd::close(w.into_raw_fd());
        fdstore("FDSTORE=1\nFDNAME=b\n", vec![r], &mut srvc, 2);
        assert_eq!(srvc.stored_fds.len(), 2); // unchanged
    }

    #[test]
    fn test_fdstore_partial_accept_at_limit() {
        let mut srvc = make_test_service();
        let fd1 = make_test_fd();

        // Store 1 first
        fdstore("FDSTORE=1\n", vec![fd1], &mut srvc, 3);
        assert_eq!(srvc.stored_fds.len(), 1);

        // Try to store 3 more — should accept only 2
        let fd2 = make_test_fd();
        let fd3 = make_test_fd();
        use std::os::unix::io::IntoRawFd;
        let (r, w) = nix::unistd::pipe().unwrap();
        let r = r.into_raw_fd();
        let _ = nix::unistd::close(w.into_raw_fd());
        fdstore("FDSTORE=1\nFDNAME=extra\n", vec![fd2, fd3, r], &mut srvc, 3);
        assert_eq!(srvc.stored_fds.len(), 3); // 1 + 2 accepted, 1 excess closed
    }

    #[test]
    fn test_fdstoreremove_basic() {
        let mut srvc = make_test_service();
        let fd1 = make_test_fd();
        let fd2 = make_test_fd();

        // Store two FDs with the same name
        fdstore("FDSTORE=1\nFDNAME=myfd\n", vec![fd1, fd2], &mut srvc, 10);
        assert_eq!(srvc.stored_fds.len(), 2);

        // Remove by name
        fdstore("FDSTOREREMOVE=1\nFDNAME=myfd\n", vec![], &mut srvc, 10);
        assert_eq!(srvc.stored_fds.len(), 0);
    }

    #[test]
    fn test_fdstoreremove_only_matching_name() {
        let mut srvc = make_test_service();
        let fd1 = make_test_fd();
        let fd2 = make_test_fd();

        // Store FDs with different names
        fdstore("FDSTORE=1\nFDNAME=keep\n", vec![fd1], &mut srvc, 10);
        fdstore("FDSTORE=1\nFDNAME=remove\n", vec![fd2], &mut srvc, 10);
        assert_eq!(srvc.stored_fds.len(), 2);

        // Remove only "remove"
        fdstore("FDSTOREREMOVE=1\nFDNAME=remove\n", vec![], &mut srvc, 10);
        assert_eq!(srvc.stored_fds.len(), 1);
        assert_eq!(srvc.stored_fds[0].0, "keep");
    }

    #[test]
    fn test_fdstoreremove_nonexistent_name() {
        let mut srvc = make_test_service();
        let fd = make_test_fd();

        fdstore("FDSTORE=1\nFDNAME=myfd\n", vec![fd], &mut srvc, 10);
        assert_eq!(srvc.stored_fds.len(), 1);

        // Remove a name that doesn't exist — nothing changes
        fdstore("FDSTOREREMOVE=1\nFDNAME=other\n", vec![], &mut srvc, 10);
        assert_eq!(srvc.stored_fds.len(), 1);
    }

    #[test]
    fn test_fdstoreremove_default_name() {
        let mut srvc = make_test_service();
        let fd = make_test_fd();

        // Store with default name "stored"
        fdstore("FDSTORE=1\n", vec![fd], &mut srvc, 10);
        assert_eq!(srvc.stored_fds.len(), 1);

        // Remove with default name "stored"
        fdstore("FDSTOREREMOVE=1\n", vec![], &mut srvc, 10);
        assert_eq!(srvc.stored_fds.len(), 0);
    }

    #[test]
    fn test_fds_without_fdstore_are_closed() {
        let mut srvc = make_test_service();
        // Create a pipe, send the read-end as an fd without FDSTORE=1
        use std::os::unix::io::IntoRawFd;
        let (r, w) = nix::unistd::pipe().unwrap();
        let r = r.into_raw_fd();
        let _ = nix::unistd::close(w.into_raw_fd());

        fdstore("READY=1\n", vec![r], &mut srvc, 10);
        // The fd should have been closed, not stored
        assert_eq!(srvc.stored_fds.len(), 0);
        // Note: we intentionally do NOT assert the fd is closed via fcntl,
        // because fd numbers can be reused by other threads between the
        // close() call in the handler and our check, making it racy.
        // The stored_fds.len() == 0 assertion above is sufficient to verify
        // that the handler did not store the fd.
    }

    #[test]
    fn test_fdstore_fdname_empty_uses_default() {
        let mut srvc = make_test_service();
        let fd = make_test_fd();

        // FDNAME= with empty value should use default "stored"
        fdstore("FDSTORE=1\nFDNAME=\n", vec![fd], &mut srvc, 10);
        assert_eq!(srvc.stored_fds.len(), 1);
        assert_eq!(srvc.stored_fds[0].0, "stored");
    }

    #[test]
    fn test_fdstore_after_remove_can_store_again() {
        let mut srvc = make_test_service();
        let fd1 = make_test_fd();

        // Fill up (max=1)
        fdstore("FDSTORE=1\nFDNAME=first\n", vec![fd1], &mut srvc, 1);
        assert_eq!(srvc.stored_fds.len(), 1);

        // Remove
        fdstore("FDSTOREREMOVE=1\nFDNAME=first\n", vec![], &mut srvc, 1);
        assert_eq!(srvc.stored_fds.len(), 0);

        // Store again — should work now
        let fd2 = make_test_fd();
        fdstore("FDSTORE=1\nFDNAME=second\n", vec![fd2], &mut srvc, 1);
        assert_eq!(srvc.stored_fds.len(), 1);
        assert_eq!(srvc.stored_fds[0].0, "second");
    }

    #[test]
    fn test_full_lifecycle_all_notify_fields() {
        let mut srvc = make_test_service();

        // 1. Service starts and signals ready
        handle_notification_message("READY=1", &mut srvc, "test.service");
        handle_notification_message("STATUS=Initialized", &mut srvc, "test.service");
        handle_notification_message(
            "INVOCATION_ID=aaaabbbbccccddddeeeeffffaaaabbbb",
            &mut srvc,
            "test.service",
        );
        handle_notification_message("MONOTONIC_USEC=100000", &mut srvc, "test.service");
        assert!(srvc.signaled_ready);
        assert_eq!(srvc.status_msgs.last().unwrap(), "Initialized");
        assert_eq!(
            srvc.invocation_id.as_deref(),
            Some("aaaabbbbccccddddeeeeffffaaaabbbb")
        );
        assert_eq!(srvc.notify_monotonic_usec, Some(100_000));

        // 2. Service configures its own watchdog timeout
        handle_notification_message("WATCHDOG_USEC=5000000", &mut srvc, "test.service");
        assert_eq!(srvc.watchdog_usec_override, Some(5_000_000));
        assert!(srvc.watchdog_last_ping.is_some());

        // 3. Service pings watchdog regularly
        handle_notification_message("WATCHDOG=1", &mut srvc, "test.service");

        // 4. Service encounters a non-fatal error
        handle_notification_message("ERRNO=11", &mut srvc, "test.service");
        handle_notification_message(
            "BUSERROR=org.freedesktop.DBus.Error.NoReply",
            &mut srvc,
            "test.service",
        );
        handle_notification_message(
            "STATUS=Degraded: upstream timeout",
            &mut srvc,
            "test.service",
        );
        assert_eq!(srvc.notify_errno, Some(11));
        assert!(srvc.signaled_ready); // Still ready despite error

        // 5. Service begins reload
        handle_notification_message("RELOADING=1", &mut srvc, "test.service");
        assert!(srvc.reloading);
        assert!(!srvc.signaled_ready);

        // 6. Service finishes reload
        handle_notification_message("READY=1", &mut srvc, "test.service");
        handle_notification_message("STATUS=Running (reloaded)", &mut srvc, "test.service");
        assert!(!srvc.reloading);
        assert!(srvc.signaled_ready);

        // 7. Service stops gracefully
        handle_notification_message("STOPPING=1", &mut srvc, "test.service");
        handle_notification_message("EXIT_STATUS=0", &mut srvc, "test.service");
        assert!(srvc.stopping);
        assert_eq!(srvc.notify_exit_status.as_deref(), Some("0"));
    }
}
