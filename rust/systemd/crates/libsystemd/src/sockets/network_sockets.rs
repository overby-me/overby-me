use std::os::unix::io::{AsRawFd, RawFd};

use log::trace;

use crate::units::SocketConfig;

/// Wrapper that owns a raw fd and closes it on drop.
#[derive(Debug)]
struct OwnedFd(RawFd);

impl AsRawFd for OwnedFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl Drop for OwnedFd {
    fn drop(&mut self) {
        super::close_raw_fd(self.0);
    }
}

/// Helper: create a socket, apply pre-bind options, bind, optionally listen,
/// then apply post-bind options. Returns the fd wrapped in a Box<dyn AsRawFd>.
fn create_and_bind_socket(
    domain: libc::c_int,
    sock_type: libc::c_int,
    protocol: libc::c_int,
    addr: &std::net::SocketAddr,
    listen: bool,
    conf: &SocketConfig,
) -> Result<Box<dyn AsRawFd + Send + Sync>, String> {
    // 1. Create socket
    let fd = unsafe { libc::socket(domain, sock_type | libc::SOCK_CLOEXEC, protocol) };
    if fd < 0 {
        return Err(format!(
            "Failed to create socket: {}",
            std::io::Error::last_os_error()
        ));
    }

    let is_ipv6 = domain == libc::AF_INET6;

    // 2. Apply pre-bind options (SO_REUSEADDR, SO_REUSEPORT, IP_FREEBIND, etc.)
    super::apply_pre_bind_socket_options(fd, conf, is_ipv6);

    // 3. Bind
    let bind_result = match addr {
        std::net::SocketAddr::V4(v4) => {
            let sockaddr = libc::sockaddr_in {
                sin_family: libc::AF_INET as libc::sa_family_t,
                sin_port: v4.port().to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from(*v4.ip()).to_be(),
                },
                sin_zero: [0; 8],
            };
            unsafe {
                libc::bind(
                    fd,
                    &sockaddr as *const _ as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
                )
            }
        }
        std::net::SocketAddr::V6(v6) => {
            let sockaddr = libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as libc::sa_family_t,
                sin6_port: v6.port().to_be(),
                sin6_flowinfo: v6.flowinfo(),
                sin6_addr: libc::in6_addr {
                    s6_addr: v6.ip().octets(),
                },
                sin6_scope_id: v6.scope_id(),
            };
            unsafe {
                libc::bind(
                    fd,
                    &sockaddr as *const _ as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
                )
            }
        }
    };

    if bind_result != 0 {
        let err = std::io::Error::last_os_error();
        unsafe {
            libc::close(fd);
        }
        return Err(format!("Failed to bind socket to {addr}: {err}"));
    }

    // 4. Listen (for stream sockets)
    if listen {
        let backlog = conf.backlog.unwrap_or(4096) as libc::c_int;
        let ret = unsafe { libc::listen(fd, backlog) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            unsafe {
                libc::close(fd);
            }
            return Err(format!("Failed to listen on socket {addr}: {err}"));
        }
    }

    // 5. Apply post-bind options (keepalive, nodelay, priority, etc.)
    super::apply_post_open_socket_options(fd, conf);

    Ok(Box::new(OwnedFd(fd)))
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct TcpSocketConfig {
    pub addr: std::net::SocketAddr,
}

impl TcpSocketConfig {
    pub fn open(&self, conf: &SocketConfig) -> Result<Box<dyn AsRawFd + Send + Sync>, String> {
        trace!("opening tcp socket: {:?}", self.addr);
        let domain = match self.addr {
            std::net::SocketAddr::V4(_) => libc::AF_INET,
            std::net::SocketAddr::V6(_) => libc::AF_INET6,
        };
        create_and_bind_socket(domain, libc::SOCK_STREAM, 0, &self.addr, true, conf)
    }
    pub fn close(&self, rawfd: RawFd) -> Result<(), String> {
        super::close_raw_fd(rawfd);
        Ok(())
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct UdpSocketConfig {
    pub addr: std::net::SocketAddr,
}

impl UdpSocketConfig {
    pub fn open(&self, conf: &SocketConfig) -> Result<Box<dyn AsRawFd + Send + Sync>, String> {
        trace!("opening udp socket: {:?}", self.addr);
        let domain = match self.addr {
            std::net::SocketAddr::V4(_) => libc::AF_INET,
            std::net::SocketAddr::V6(_) => libc::AF_INET6,
        };
        create_and_bind_socket(domain, libc::SOCK_DGRAM, 0, &self.addr, false, conf)
    }

    pub fn close(&self, rawfd: RawFd) -> Result<(), String> {
        super::close_raw_fd(rawfd);
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Socket {
    pub activated: bool,
    /// Monotonically increasing counter for generating Accept=yes instance names.
    pub accept_counter: u64,
    /// Number of currently active Accept=yes connections.
    pub active_accept_connections: u64,
}
