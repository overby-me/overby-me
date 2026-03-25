use super::StdIo;
use crate::services::Service;
use crate::units::ServiceConfig;
use crate::units::StdIoOption;
use std::os::unix::io::{AsRawFd, BorrowedFd, FromRawFd, IntoRawFd};
use std::os::unix::net::UnixDatagram;

fn open_stdio(setting: &Option<StdIoOption>) -> Result<StdIo, String> {
    match setting {
        Some(StdIoOption::File(path)) => {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .read(true)
                .open(path)
                .map_err(|e| format!("Error opening file: {path:?}: {e}"))?;
            Ok(StdIo::File(file))
        }
        Some(StdIoOption::AppendFile(path)) => {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .read(true)
                .open(path)
                .map_err(|e| format!("Error opening file: {path:?}: {e}"))?;
            Ok(StdIo::File(file))
        }
        Some(StdIoOption::Null) => {
            // Open /dev/null for output
            let file = std::fs::OpenOptions::new()
                .write(true)
                .read(true)
                .open("/dev/null")
                .map_err(|e| format!("Error opening /dev/null: {e}"))?;
            Ok(StdIo::File(file))
        }
        Some(StdIoOption::Inherit)
        | Some(StdIoOption::Journal)
        | Some(StdIoOption::Kmsg)
        | Some(StdIoOption::Tty) => {
            // For inherit/journal/kmsg/tty: use a pipe so the service manager can
            // capture and forward the output. The exec_helper will handle
            // overriding stdout/stderr to the TTY when StandardInput=tty is set
            // or when StandardOutput/StandardError=tty is set independently.
            let (r, w) = nix::unistd::pipe().unwrap();
            Ok(super::StdIo::Piped(r.into_raw_fd(), w.into_raw_fd()))
        }
        None => {
            let (r, w) = nix::unistd::pipe().unwrap();
            Ok(super::StdIo::Piped(r.into_raw_fd(), w.into_raw_fd()))
        }
    }
}

pub fn prepare_service(
    srvc: &mut Service,
    conf: &ServiceConfig,
    name: &str,
    notification_socket_path: &std::path::PathBuf,
) -> Result<(), String> {
    // setup socket for notifications from the service
    if !notification_socket_path.exists() {
        std::fs::create_dir_all(notification_socket_path).unwrap();
    }
    let daemon_socket_path = notification_socket_path.join(format!("{}.notify_socket", &name));

    // NOTIFY_SOCKET
    let notify_socket_env_var = if daemon_socket_path.starts_with(".") {
        let cur_dir = std::env::current_dir().unwrap();
        cur_dir.join(&daemon_socket_path)
    } else {
        daemon_socket_path
    };

    if srvc.notifications.is_none() {
        if notify_socket_env_var.exists() {
            std::fs::remove_file(&notify_socket_env_var).unwrap();
        }
        let stream = UnixDatagram::bind(&notify_socket_env_var).unwrap();

        // Make the socket world-writable so that services running as non-root
        // users (e.g. User=nscd) can send sd_notify readiness notifications.
        // Without this, sendto() on the socket fails with EACCES for non-root
        // services, causing Type=notify services to time out waiting for READY=1.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            &notify_socket_env_var,
            std::fs::Permissions::from_mode(0o666),
        )
        .unwrap();

        // close these fd's on exec. They must not show up in child processes
        let new_listener_fd = stream.as_raw_fd();
        nix::fcntl::fcntl(
            unsafe { BorrowedFd::borrow_raw(new_listener_fd) },
            nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
        )
        .unwrap();

        srvc.notifications = Some(stream);
    }

    if srvc.stdout.is_none() {
        srvc.stdout = Some(open_stdio(&conf.exec_config.stdout_path)?);
    }
    open_stderr_inherit_stdout(srvc, conf)?;

    srvc.notifications_path = Some(notify_socket_env_var);

    Ok(())
}

/// Re-open stdout/stderr file descriptors after ExecStartPre may have
/// modified or deleted the output files.
pub fn reopen_stdio(srvc: &mut Service, conf: &ServiceConfig) -> Result<(), String> {
    if srvc.stdout.is_none() {
        srvc.stdout = Some(open_stdio(&conf.exec_config.stdout_path)?);
    }
    open_stderr_inherit_stdout(srvc, conf)?;
    Ok(())
}

/// Open stderr, using stdout's fd when StandardError defaults to inherit.
/// In systemd, `StandardError=` defaults to `inherit` which means stderr
/// duplicates stdout's file descriptor.
fn open_stderr_inherit_stdout(srvc: &mut Service, conf: &ServiceConfig) -> Result<(), String> {
    if srvc.stderr.is_some() {
        return Ok(());
    }
    match &conf.exec_config.stderr_path {
        // No explicit StandardError= or StandardError=inherit: dup stdout
        None | Some(StdIoOption::Inherit) => {
            if let Some(ref stdout) = srvc.stdout {
                let write_fd = stdout.write_fd();
                let dup_fd = unsafe { libc::dup(write_fd) };
                if dup_fd < 0 {
                    return Err(format!(
                        "Failed to dup stdout fd for stderr: {}",
                        std::io::Error::last_os_error()
                    ));
                }
                // Wrap the duped fd in a File so it gets closed on drop
                let file = unsafe { std::fs::File::from_raw_fd(dup_fd) };
                srvc.stderr = Some(StdIo::File(file));
            } else {
                srvc.stderr = Some(open_stdio(&conf.exec_config.stderr_path)?);
            }
        }
        _ => {
            srvc.stderr = Some(open_stdio(&conf.exec_config.stderr_path)?);
        }
    }
    Ok(())
}
