use std::{
    os::unix::fs::PermissionsExt,
    os::unix::io::AsRawFd,
    os::unix::io::BorrowedFd,
    os::unix::io::RawFd,
    os::unix::net::{UnixDatagram, UnixListener},
};

use log::trace;

use crate::units::SocketConfig;

#[derive(Clone, Eq, PartialEq, Debug)]
pub enum UnixSocketConfig {
    Stream(String),
    Sequential(String),
    Datagram(String),
}

/// Returns true if the path uses the abstract socket namespace (`@` prefix).
fn is_abstract(path: &str) -> bool {
    path.starts_with('@')
}

/// Wrapper around a raw file descriptor for sockets created via libc
/// (abstract namespace sockets and SOCK_SEQPACKET).
#[derive(Debug)]
struct RawSocket {
    fd: Option<i32>,
    /// Filesystem path, if any. Abstract sockets have no path.
    path: Option<std::path::PathBuf>,
}

impl AsRawFd for RawSocket {
    fn as_raw_fd(&self) -> i32 {
        self.fd.unwrap()
    }
}

impl Drop for RawSocket {
    fn drop(&mut self) {
        if let Some(fd) = self.fd {
            super::close_raw_fd(fd);
        }
        self.fd = None;
        if let Some(ref path) = self.path
            && path.exists()
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Create, bind, and listen on an abstract namespace Unix socket.
fn bind_abstract(name: &str, sock_type: libc::c_int) -> Result<RawSocket, String> {
    // Strip the leading '@' to get the abstract name
    let abstract_name = &name[1..];
    let unix_addr = nix::sys::socket::UnixAddr::new_abstract(abstract_name.as_bytes())
        .map_err(|e| format!("failed to create abstract socket address @{abstract_name}: {e}"))?;

    let fd = unsafe { libc::socket(libc::AF_UNIX, sock_type, 0) };
    if fd < 0 {
        return Err(format!(
            "failed to create abstract socket @{abstract_name}: {}",
            std::io::Error::last_os_error()
        ));
    }
    nix::sys::socket::bind(fd, &unix_addr).map_err(|e| {
        unsafe { libc::close(fd) };
        format!("failed to bind abstract socket @{abstract_name}: {e}")
    })?;

    if sock_type == libc::SOCK_STREAM || sock_type == libc::SOCK_SEQPACKET {
        let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
        nix::sys::socket::listen(&borrowed, nix::sys::socket::Backlog::new(128).unwrap()).map_err(
            |e| {
                unsafe { libc::close(fd) };
                format!("failed to listen on abstract socket @{abstract_name}: {e}")
            },
        )?;
    }

    Ok(RawSocket {
        fd: Some(fd),
        path: None,
    })
}

/// Prepare the directory and remove old socket file if it exists.
/// Applies `DirectoryMode=` (default `0755` per systemd.socket(5)) to
/// newly created parent directories.
fn prepare_unix_socket_path(path: &std::path::Path, conf: &SocketConfig) -> Result<(), String> {
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|e| format!("Error removing old socket file {path:?}: {e}"))?;
    }
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Error creating UnixSocket directory {parent:?}: {e}"))?;
        }

        // DirectoryMode= — default is 0755 per systemd.socket(5).
        // Always apply, even if the directory already existed, because another
        // subsystem (e.g. ensure_home_directories) may have created it with
        // more restrictive permissions (e.g. 0700 for /run/dbus).
        let dir_mode = conf.directory_mode.unwrap_or(0o755);
        let permissions = std::fs::Permissions::from_mode(dir_mode);
        if let Err(e) = std::fs::set_permissions(parent, permissions) {
            trace!("Failed to set DirectoryMode on {:?}: {e}", parent);
        }
    }
    Ok(())
}

/// Apply SO_PASSCRED if PassCredentials= is set.
fn apply_passcred(fd: RawFd, conf: &SocketConfig) {
    if conf.pass_credentials {
        let val: libc::c_int = 1;
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }
}

impl UnixSocketConfig {
    pub fn close(&self, _rawfd: RawFd, remove_on_stop: bool) -> Result<(), String> {
        if remove_on_stop {
            let strpath = match self {
                Self::Stream(s) | Self::Datagram(s) | Self::Sequential(s) => s,
            };
            // Abstract sockets have no filesystem entry to remove.
            if !is_abstract(strpath) {
                let path = std::path::PathBuf::from(strpath);
                if path.exists() {
                    std::fs::remove_file(&path)
                        .map_err(|e| format!("Error removing file {path:?}: {e}"))?;
                }
            }
        }

        // fd is closed by OwnedFd::drop when the Box<dyn AsRawFd> is dropped
        Ok(())
    }

    pub fn open(&self, conf: &SocketConfig) -> Result<Box<dyn AsRawFd + Send + Sync>, String> {
        match self {
            Self::Stream(path) => {
                trace!("opening streaming unix socket: {path:?}");

                if is_abstract(path) {
                    let sock = bind_abstract(path, libc::SOCK_STREAM)?;
                    apply_passcred(sock.as_raw_fd(), conf);
                    return Ok(Box::new(sock));
                }

                let spath = std::path::Path::new(path);
                prepare_unix_socket_path(spath, conf)?;

                let stream = UnixListener::bind(spath)
                    .map_err(|e| format!("failed to bind unix stream socket {path}: {e}"))?;

                apply_passcred(stream.as_raw_fd(), conf);

                // Apply SocketUser=/SocketGroup=/SocketMode=
                super::apply_socket_ownership(spath, conf);

                Ok(Box::new(stream))
            }
            Self::Datagram(path) => {
                trace!("opening datagram unix socket: {path:?}");

                if is_abstract(path) {
                    let sock = bind_abstract(path, libc::SOCK_DGRAM)?;
                    apply_passcred(sock.as_raw_fd(), conf);
                    return Ok(Box::new(sock));
                }

                let spath = std::path::Path::new(path);
                prepare_unix_socket_path(spath, conf)?;

                let dgram = UnixDatagram::bind(spath)
                    .map_err(|e| format!("failed to bind unix datagram socket {path}: {e}"))?;

                apply_passcred(dgram.as_raw_fd(), conf);

                // Apply SocketUser=/SocketGroup=/SocketMode=
                super::apply_socket_ownership(spath, conf);

                Ok(Box::new(dgram))
            }
            Self::Sequential(path) => {
                trace!("opening sequential packet unix socket: {path:?}");

                if is_abstract(path) {
                    let sock = bind_abstract(path, libc::SOCK_SEQPACKET)?;
                    apply_passcred(sock.as_raw_fd(), conf);
                    return Ok(Box::new(sock));
                }

                let spath = std::path::Path::new(path);
                prepare_unix_socket_path(spath, conf)?;

                let path = std::path::PathBuf::from(path);
                match crate::platform::make_seqpacket_socket(&path) {
                    Ok(fd) => {
                        apply_passcred(fd, conf);

                        // Apply SocketUser=/SocketGroup=/SocketMode=
                        super::apply_socket_ownership(&path, conf);

                        Ok(Box::new(RawSocket {
                            fd: Some(fd),
                            path: Some(path),
                        }))
                    }
                    Err(e) => Err(e),
                }
            }
        }
    }
}
