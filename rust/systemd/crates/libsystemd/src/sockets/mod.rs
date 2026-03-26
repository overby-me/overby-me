//! Socket related code. Opening of all different kinds, match sockets to services etc

mod fifo;
mod netlink_sockets;
mod network_sockets;
mod special_file;
mod unix_sockets;
pub use fifo::*;
use log::trace;
pub use netlink_sockets::*;
pub use network_sockets::*;
pub use special_file::*;
pub use unix_sockets::*;

use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::{AsRawFd, BorrowedFd, RawFd};

use crate::fd_store::FDStore;
use crate::units::{BindIPv6Only, SocketConfig, UnitId};

pub fn close_raw_fd(fd: RawFd) {
    loop {
        let ret = unsafe { libc::close(fd) };
        if ret == 0 {
            break;
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EBADF) {
            break;
        }
        // Other errors (EINTR and EIO) mean that we should try again
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub enum SocketKind {
    Stream(String),
    Sequential(String),
    Datagram(String),
    Fifo(String),
    Netlink(String),
    Special(String),
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub enum SpecializedSocketConfig {
    UnixSocket(UnixSocketConfig),
    Fifo(FifoConfig),
    TcpSocket(TcpSocketConfig),
    UdpSocket(UdpSocketConfig),
    NetlinkSocket(NetlinkSocketConfig),
    SpecialFile(SpecialFileConfig),
}

impl SpecializedSocketConfig {
    fn open(&self, sock_conf: &SocketConfig) -> Result<Box<dyn AsRawFd + Send + Sync>, String> {
        match self {
            Self::UnixSocket(conf) => conf.open(sock_conf),
            Self::TcpSocket(conf) => conf.open(sock_conf),
            Self::UdpSocket(conf) => conf.open(sock_conf),
            Self::Fifo(conf) => conf.open(sock_conf),
            Self::NetlinkSocket(conf) => conf.open(sock_conf),
            Self::SpecialFile(conf) => conf.open(sock_conf),
        }
    }
    fn close(&self, rawfd: RawFd, remove_on_stop: bool) -> Result<(), String> {
        match self {
            Self::UnixSocket(conf) => conf.close(rawfd, remove_on_stop),
            Self::TcpSocket(conf) => conf.close(rawfd),
            Self::UdpSocket(conf) => conf.close(rawfd),
            Self::Fifo(conf) => conf.close(rawfd, remove_on_stop),
            Self::NetlinkSocket(conf) => conf.close(rawfd),
            Self::SpecialFile(conf) => conf.close(rawfd),
        }
    }
}

/// Apply socket-level options (setsockopt) to an already-created socket fd.
/// These are options that can be set after bind() — keepalive, nodelay, etc.
/// Options that must be set before bind() are handled in TcpSocketConfig/UdpSocketConfig::open().
pub(crate) fn apply_post_open_socket_options(fd: RawFd, conf: &SocketConfig) {
    // SO_KEEPALIVE
    if conf.keep_alive {
        let val: libc::c_int = 1;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_KEEPALIVE,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set SO_KEEPALIVE on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // TCP_KEEPIDLE
    if let Some(secs) = conf.keep_alive_time_sec {
        let val = secs as libc::c_int;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_KEEPIDLE,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set TCP_KEEPIDLE on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // TCP_KEEPINTVL
    if let Some(secs) = conf.keep_alive_interval_sec {
        let val = secs as libc::c_int;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_KEEPINTVL,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set TCP_KEEPINTVL on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // TCP_KEEPCNT
    if let Some(probes) = conf.keep_alive_probes {
        let val = probes as libc::c_int;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_KEEPCNT,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set TCP_KEEPCNT on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // TCP_NODELAY
    if conf.no_delay {
        let val: libc::c_int = 1;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_NODELAY,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set TCP_NODELAY on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // SO_PRIORITY
    if let Some(prio) = conf.priority {
        let val = prio as libc::c_int;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PRIORITY,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set SO_PRIORITY on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // SO_MARK
    if let Some(mark) = conf.mark {
        let val = mark as libc::c_int;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_MARK,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set SO_MARK on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // IP_TOS
    if let Some(tos) = conf.ip_tos {
        let val = tos as libc::c_int;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_IP,
                libc::IP_TOS,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set IP_TOS on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // IP_TTL
    if let Some(ttl) = conf.ip_ttl {
        let val = ttl as libc::c_int;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_IP,
                libc::IP_TTL,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set IP_TTL on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // SO_RCVBUF
    if let Some(size) = conf.receive_buffer {
        let val = size as libc::c_int;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set SO_RCVBUF on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // SO_SNDBUF
    if let Some(size) = conf.send_buffer {
        let val = size as libc::c_int;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set SO_SNDBUF on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // SO_PASSCRED
    if conf.pass_credentials {
        let val: libc::c_int = 1;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set SO_PASSCRED on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // SO_TIMESTAMP / SO_TIMESTAMPNS
    match conf.timestamping {
        crate::units::Timestamping::Off => {}
        crate::units::Timestamping::Microseconds => {
            let val: libc::c_int = 1;
            let ret = unsafe {
                libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_TIMESTAMP,
                    &val as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                )
            };
            if ret != 0 {
                trace!(
                    "Failed to set SO_TIMESTAMP on fd {fd}: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
        crate::units::Timestamping::Nanoseconds => {
            let val: libc::c_int = 1;
            let ret = unsafe {
                libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_TIMESTAMPNS,
                    &val as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                )
            };
            if ret != 0 {
                trace!(
                    "Failed to set SO_TIMESTAMPNS on fd {fd}: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
    }
}

/// Apply pre-bind socket options that must be set before bind() is called.
/// These include SO_REUSEPORT, IP_FREEBIND, SO_BINDTODEVICE, IPV6_V6ONLY,
/// SO_BROADCAST, IP_TRANSPARENT.
pub(crate) fn apply_pre_bind_socket_options(fd: RawFd, conf: &SocketConfig, is_ipv6: bool) {
    // SO_REUSEADDR — always set for listening sockets (like real systemd)
    let val: libc::c_int = 1;
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &val as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if ret != 0 {
        trace!(
            "Failed to set SO_REUSEADDR on fd {fd}: {}",
            std::io::Error::last_os_error()
        );
    }

    // SO_REUSEPORT
    if conf.reuse_port {
        let val: libc::c_int = 1;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_REUSEPORT,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set SO_REUSEPORT on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // IP_FREEBIND / IPV6_FREEBIND
    if conf.free_bind {
        if is_ipv6 {
            let val: libc::c_int = 1;
            let ret = unsafe {
                libc::setsockopt(
                    fd,
                    libc::IPPROTO_IPV6,
                    libc::IPV6_FREEBIND,
                    &val as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                )
            };
            if ret != 0 {
                trace!(
                    "Failed to set IPV6_FREEBIND on fd {fd}: {}",
                    std::io::Error::last_os_error()
                );
            }
        } else {
            let val: libc::c_int = 1;
            let ret = unsafe {
                libc::setsockopt(
                    fd,
                    libc::IPPROTO_IP,
                    libc::IP_FREEBIND,
                    &val as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                )
            };
            if ret != 0 {
                trace!(
                    "Failed to set IP_FREEBIND on fd {fd}: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
    }

    // IP_TRANSPARENT
    if conf.transparent {
        let val: libc::c_int = 1;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_IP,
                libc::IP_TRANSPARENT,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set IP_TRANSPARENT on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // SO_BROADCAST
    if conf.broadcast {
        let val: libc::c_int = 1;
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_BROADCAST,
                &val as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set SO_BROADCAST on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // IPV6_V6ONLY
    if is_ipv6 {
        match conf.bind_ipv6_only {
            BindIPv6Only::Default => {} // leave kernel default
            BindIPv6Only::Both => {
                let val: libc::c_int = 0;
                let ret = unsafe {
                    libc::setsockopt(
                        fd,
                        libc::IPPROTO_IPV6,
                        libc::IPV6_V6ONLY,
                        &val as *const _ as *const libc::c_void,
                        std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                    )
                };
                if ret != 0 {
                    trace!(
                        "Failed to clear IPV6_V6ONLY on fd {fd}: {}",
                        std::io::Error::last_os_error()
                    );
                }
            }
            BindIPv6Only::Ipv6Only => {
                let val: libc::c_int = 1;
                let ret = unsafe {
                    libc::setsockopt(
                        fd,
                        libc::IPPROTO_IPV6,
                        libc::IPV6_V6ONLY,
                        &val as *const _ as *const libc::c_void,
                        std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                    )
                };
                if ret != 0 {
                    trace!(
                        "Failed to set IPV6_V6ONLY on fd {fd}: {}",
                        std::io::Error::last_os_error()
                    );
                }
            }
        }
    }

    // SO_BINDTODEVICE
    if let Some(ref dev) = conf.bind_to_device {
        let c_dev =
            std::ffi::CString::new(dev.as_str()).unwrap_or_else(|_| std::ffi::CString::default());
        let ret = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_BINDTODEVICE,
                c_dev.as_ptr() as *const libc::c_void,
                (dev.len() + 1) as libc::socklen_t,
            )
        };
        if ret != 0 {
            trace!(
                "Failed to set SO_BINDTODEVICE={dev} on fd {fd}: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}

/// Apply SocketUser=/SocketGroup= ownership and SocketMode= permissions
/// to an AF_UNIX socket or FIFO file node.
pub(crate) fn apply_socket_ownership(path: &std::path::Path, conf: &SocketConfig) {
    // SocketMode= — default is 0666 per systemd.socket(5).
    // Without this, sockets created via bind() inherit the process umask
    // (typically 0022 → mode 0755), which prevents non-root services from
    // connecting (e.g. D-Bus "Permission denied" for timesyncd/networkd).
    let mode = conf.socket_mode.unwrap_or(0o666);
    let permissions = std::fs::Permissions::from_mode(mode);
    if let Err(e) = std::fs::set_permissions(path, permissions) {
        trace!("Failed to set SocketMode on {:?}: {e}", path);
    }

    // SocketUser= / SocketGroup=
    let uid = conf.socket_user.as_ref().and_then(|u| {
        if let Ok(n) = u.parse::<u32>() {
            Some(nix::unistd::Uid::from_raw(n))
        } else {
            nix::unistd::User::from_name(u)
                .ok()
                .flatten()
                .map(|u| u.uid)
        }
    });
    let gid = conf.socket_group.as_ref().and_then(|g| {
        if let Ok(n) = g.parse::<u32>() {
            Some(nix::unistd::Gid::from_raw(n))
        } else {
            nix::unistd::Group::from_name(g)
                .ok()
                .flatten()
                .map(|g| g.gid)
        }
    });
    if (uid.is_some() || gid.is_some())
        && let Err(e) = nix::unistd::chown(path, uid, gid)
    {
        trace!("Failed to set SocketUser/SocketGroup on {:?}: {e}", path,);
    }
}

impl Socket {
    #[must_use]
    pub fn build_name_list(&self, conf: SocketConfig) -> String {
        let mut name_list = String::with_capacity(
            conf.filedesc_name.len() * conf.sockets.len() + conf.sockets.len(),
        );
        name_list.push_str(&conf.filedesc_name);
        for _ in 0..conf.sockets.len() - 1 {
            name_list.push(':');
            name_list.push_str(&conf.filedesc_name);
        }
        name_list
    }

    pub fn open_all(
        &mut self,
        conf: &SocketConfig,
        name: String,
        id: UnitId,
        fd_store: &mut FDStore,
    ) -> std::io::Result<()> {
        let mut fds = Vec::new();
        for idx in 0..conf.sockets.len() {
            let single_conf = &conf.sockets[idx];
            let as_raw_fd = match single_conf.specialized.open(conf) {
                Ok(fd) => fd,
                Err(e) => {
                    return Err(std::io::Error::other(format!(
                        "Failed to open socket {} (index {}): {}",
                        name, idx, e
                    )));
                }
            };
            // close these fd's on exec. They must not show up in child processes
            // the ńeeded fd's will be duped which unsets the flag again
            let new_fd = as_raw_fd.as_raw_fd();
            nix::fcntl::fcntl(
                unsafe { BorrowedFd::borrow_raw(new_fd) },
                nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
            )
            .unwrap();
            fds.push((id.clone(), conf.filedesc_name.clone(), as_raw_fd));
            //need to stop the listener to drop which would close the filedescriptor
        }
        trace!(
            "Opened all sockets: {:?}",
            fds.iter()
                .map(|(_, _, fd)| fd.as_raw_fd())
                .collect::<Vec<_>>(),
        );
        fd_store.insert_global(name, fds);
        Ok(())
    }

    pub fn close_all(
        &mut self,
        conf: &SocketConfig,
        name: String,
        fd_store: &mut FDStore,
    ) -> Result<(), String> {
        if let Some(fds) = fd_store.remove_global(&name) {
            for (sock_conf, fd_entry) in conf.sockets.iter().zip(fds.iter()) {
                sock_conf
                    .specialized
                    .close(fd_entry.2.as_raw_fd(), conf.remove_on_stop)?;
            }
        }
        Ok(())
    }
}
