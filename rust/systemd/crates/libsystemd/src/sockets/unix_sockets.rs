use std::{
    os::unix::fs::PermissionsExt,
    os::unix::io::AsRawFd,
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

#[derive(Debug)]
struct UnixSeqPacket(Option<i32>, std::path::PathBuf);

impl AsRawFd for UnixSeqPacket {
    fn as_raw_fd(&self) -> i32 {
        self.0.unwrap()
    }
}

impl Drop for UnixSeqPacket {
    fn drop(&mut self) {
        if self.1.exists() {
            self.close();
            std::fs::remove_file(&self.1).unwrap();
        }
    }
}

impl UnixSeqPacket {
    fn close(&mut self) {
        if let Some(fd) = self.0 {
            super::close_raw_fd(fd);
        }
        self.0 = None;
    }
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

impl UnixSocketConfig {
    pub fn close(&self, rawfd: RawFd, remove_on_stop: bool) -> Result<(), String> {
        if remove_on_stop {
            let strpath = match self {
                Self::Stream(s) | Self::Datagram(s) | Self::Sequential(s) => s,
            };
            let path = std::path::PathBuf::from(strpath);
            if path.exists() {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("Error removing file {path:?}: {e}"))?;
            }
        }

        super::close_raw_fd(rawfd);
        Ok(())
    }

    pub fn open(&self, conf: &SocketConfig) -> Result<Box<dyn AsRawFd + Send + Sync>, String> {
        match self {
            Self::Stream(path) => {
                let spath = std::path::Path::new(path);
                prepare_unix_socket_path(spath, conf)?;

                trace!("opening streaming unix socket: {path:?}");
                let stream = UnixListener::bind(spath)
                    .map_err(|e| format!("failed to bind unix stream socket {path}: {e}"))?;

                // Apply PassCredentials via SO_PASSCRED on the raw fd
                if conf.pass_credentials {
                    let fd = stream.as_raw_fd();
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

                // Apply SocketUser=/SocketGroup=/SocketMode=
                super::apply_socket_ownership(spath, conf);

                Ok(Box::new(stream))
            }
            Self::Datagram(path) => {
                let spath = std::path::Path::new(path);
                prepare_unix_socket_path(spath, conf)?;

                trace!("opening datagram unix socket: {path:?}");
                let dgram = UnixDatagram::bind(spath)
                    .map_err(|e| format!("failed to bind unix datagram socket {path}: {e}"))?;

                // Apply PassCredentials via SO_PASSCRED on the raw fd
                if conf.pass_credentials {
                    let fd = dgram.as_raw_fd();
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

                // Apply SocketUser=/SocketGroup=/SocketMode=
                super::apply_socket_ownership(spath, conf);

                Ok(Box::new(dgram))
            }
            Self::Sequential(path) => {
                let spath = std::path::Path::new(path);
                prepare_unix_socket_path(spath, conf)?;

                let path = std::path::PathBuf::from(path);
                trace!("opening sequential packet unix socket: {path:?}");
                match crate::platform::make_seqpacket_socket(&path) {
                    Ok(fd) => {
                        // Apply PassCredentials
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

                        // Apply SocketUser=/SocketGroup=/SocketMode=
                        super::apply_socket_ownership(&path, conf);

                        Ok(Box::new(UnixSeqPacket(Some(fd), path)))
                    }
                    Err(e) => Err(e),
                }
            }
        }
    }
}
