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

// ---------------------------------------------------------------------------
// SOCK_DESTROY — kill orphaned TCP sockets via netlink
// ---------------------------------------------------------------------------

// Netlink / sock_diag constants (from linux/netlink.h, linux/sock_diag.h,
// linux/inet_diag.h).
const NETLINK_SOCK_DIAG: libc::c_int = 4;
const SOCK_DIAG_BY_FAMILY: u16 = 20;
const SOCK_DESTROY: u16 = 21;
const NLM_F_REQUEST: u16 = 0x0001;
const NLM_F_DUMP: u16 = 0x0300;
const NLMSG_DONE: u16 = 3;

/// TCP state bit for LISTEN (1 << TCP_LISTEN where TCP_LISTEN = 10).
const TCPF_LISTEN: u32 = 1 << 10;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InetDiagSockId {
    sport: u16, // big-endian
    dport: u16,
    src: [u32; 4],
    dst: [u32; 4],
    if_idx: u32,
    cookie: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InetDiagReqV2 {
    sdiag_family: u8,
    sdiag_protocol: u8,
    idiag_ext: u8,
    pad: u8,
    idiag_states: u32,
    id: InetDiagSockId,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NlMsgHdr {
    nlmsg_len: u32,
    nlmsg_type: u16,
    nlmsg_flags: u16,
    nlmsg_seq: u32,
    nlmsg_pid: u32,
}

/// Response message from SOCK_DIAG_BY_FAMILY.
#[repr(C)]
#[derive(Clone, Copy)]
struct InetDiagMsg {
    idiag_family: u8,
    idiag_state: u8,
    idiag_timer: u8,
    idiag_retrans: u8,
    id: InetDiagSockId,
    idiag_expires: u32,
    idiag_rqueue: u32,
    idiag_wqueue: u32,
    idiag_uid: u32,
    idiag_inode: u32,
}

/// Destroy orphaned TCP listening sockets on `port` (both IPv4 and IPv6).
///
/// Uses the `NETLINK_SOCK_DIAG` / `SOCK_DESTROY` kernel interface,
/// equivalent to `ss --kill state listening src :PORT`.  Best-effort:
/// errors are silently ignored.
pub fn destroy_tcp_listeners_on_port(port: u16) {
    destroy_tcp_listeners_on_port_family(port, libc::AF_INET as u8);
    destroy_tcp_listeners_on_port_family(port, libc::AF_INET6 as u8);
}

fn destroy_tcp_listeners_on_port_family(port: u16, family: u8) {
    let nl_fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            NETLINK_SOCK_DIAG,
        )
    };
    if nl_fd < 0 {
        return;
    }

    // Set a short timeout so we don't block PID 1 forever.
    let tv = libc::timeval {
        tv_sec: 2,
        tv_usec: 0,
    };
    unsafe {
        libc::setsockopt(
            nl_fd,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            (&tv as *const libc::timeval).cast(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
    }

    // Step 1: dump all TCP LISTEN sockets for this address family.
    let req_payload = InetDiagReqV2 {
        sdiag_family: family,
        sdiag_protocol: libc::IPPROTO_TCP as u8,
        idiag_ext: 0,
        pad: 0,
        idiag_states: TCPF_LISTEN,
        id: InetDiagSockId::default(),
    };

    let nlh = NlMsgHdr {
        nlmsg_len: (std::mem::size_of::<NlMsgHdr>() + std::mem::size_of::<InetDiagReqV2>()) as u32,
        nlmsg_type: SOCK_DIAG_BY_FAMILY,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_DUMP,
        nlmsg_seq: 1,
        nlmsg_pid: 0,
    };

    let mut buf = vec![0u8; nlh.nlmsg_len as usize];
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&nlh as *const NlMsgHdr).cast::<u8>(),
            buf.as_mut_ptr(),
            std::mem::size_of::<NlMsgHdr>(),
        );
        std::ptr::copy_nonoverlapping(
            (&req_payload as *const InetDiagReqV2).cast::<u8>(),
            buf.as_mut_ptr().add(std::mem::size_of::<NlMsgHdr>()),
            std::mem::size_of::<InetDiagReqV2>(),
        );
    }

    let sent = unsafe { libc::send(nl_fd, buf.as_ptr().cast(), buf.len(), 0) };
    if sent < 0 {
        unsafe { libc::close(nl_fd) };
        return;
    }

    // Step 2: receive responses and collect matching socket identities.
    let port_be = port.to_be();
    let mut targets: Vec<(u8, InetDiagSockId)> = Vec::new();
    let mut recv_buf = vec![0u8; 32768];

    'outer: loop {
        let n = unsafe { libc::recv(nl_fd, recv_buf.as_mut_ptr().cast(), recv_buf.len(), 0) };
        if n <= 0 {
            break;
        }
        let n = n as usize;

        let mut offset = 0usize;
        while offset + std::mem::size_of::<NlMsgHdr>() <= n {
            let hdr: NlMsgHdr =
                unsafe { std::ptr::read_unaligned(recv_buf.as_ptr().add(offset).cast()) };

            if hdr.nlmsg_type == NLMSG_DONE || hdr.nlmsg_len == 0 {
                break 'outer;
            }

            let payload_off = offset + std::mem::size_of::<NlMsgHdr>();
            if hdr.nlmsg_type == SOCK_DIAG_BY_FAMILY
                && payload_off + std::mem::size_of::<InetDiagMsg>() <= n
            {
                let msg: InetDiagMsg =
                    unsafe { std::ptr::read_unaligned(recv_buf.as_ptr().add(payload_off).cast()) };
                if msg.id.sport == port_be {
                    targets.push((msg.idiag_family, msg.id));
                }
            }

            // Advance to next message (NLMSG_ALIGN rounds up to 4).
            let aligned_len = (hdr.nlmsg_len as usize + 3) & !3;
            offset += aligned_len.max(std::mem::size_of::<NlMsgHdr>());
        }
    }

    // Step 3: destroy each matching socket.
    for (fam, sock_id) in &targets {
        let destroy_payload = InetDiagReqV2 {
            sdiag_family: *fam,
            sdiag_protocol: libc::IPPROTO_TCP as u8,
            idiag_ext: 0,
            pad: 0,
            idiag_states: TCPF_LISTEN,
            id: *sock_id,
        };
        let destroy_hdr = NlMsgHdr {
            nlmsg_len: (std::mem::size_of::<NlMsgHdr>() + std::mem::size_of::<InetDiagReqV2>())
                as u32,
            nlmsg_type: SOCK_DESTROY,
            nlmsg_flags: NLM_F_REQUEST,
            nlmsg_seq: 2,
            nlmsg_pid: 0,
        };

        let mut dbuf = vec![0u8; destroy_hdr.nlmsg_len as usize];
        unsafe {
            std::ptr::copy_nonoverlapping(
                (&destroy_hdr as *const NlMsgHdr).cast::<u8>(),
                dbuf.as_mut_ptr(),
                std::mem::size_of::<NlMsgHdr>(),
            );
            std::ptr::copy_nonoverlapping(
                (&destroy_payload as *const InetDiagReqV2).cast::<u8>(),
                dbuf.as_mut_ptr().add(std::mem::size_of::<NlMsgHdr>()),
                std::mem::size_of::<InetDiagReqV2>(),
            );
        }
        unsafe {
            libc::send(nl_fd, dbuf.as_ptr().cast(), dbuf.len(), 0);
        }
    }

    unsafe { libc::close(nl_fd) };

    if !targets.is_empty() {
        trace!(
            "Destroyed {} orphaned TCP LISTEN socket(s) on port {}",
            targets.len(),
            port,
        );
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

    // 3. Bind (with retry for EADDRINUSE from orphaned kernel sockets)
    let do_bind = |fd: libc::c_int, addr: &std::net::SocketAddr| -> libc::c_int {
        match addr {
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
        }
    };

    let mut bind_ok = do_bind(fd, addr) == 0;
    if !bind_ok {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EADDRINUSE) {
            // Kill any orphaned TCP LISTEN sockets on this port, then retry.
            // Orphans can persist indefinitely after a socket-activated
            // service exits (no process holds the fd, yet the kernel socket
            // remains in LISTEN state).  SOCK_DESTROY is the only reliable
            // way to reclaim the port.
            destroy_tcp_listeners_on_port(addr.port());
            for attempt in 1..=10 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if do_bind(fd, addr) == 0 {
                    log::trace!("bind to {addr} succeeded on retry {attempt} after EADDRINUSE");
                    bind_ok = true;
                    break;
                }
            }
        }
        if !bind_ok {
            let err = std::io::Error::last_os_error();
            unsafe {
                libc::close(fd);
            }
            return Err(format!("Failed to bind socket to {addr}: {err}"));
        }
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
    pub fn close(&self, _rawfd: RawFd) -> Result<(), String> {
        // fd is closed by OwnedFd::drop when the Box<dyn AsRawFd> is dropped
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

    pub fn close(&self, _rawfd: RawFd) -> Result<(), String> {
        // fd is closed by OwnedFd::drop when the Box<dyn AsRawFd> is dropped
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
    /// Timestamps of recent trigger events for TriggerLimitIntervalSec/TriggerLimitBurst enforcement.
    pub trigger_timestamps: Vec<std::time::Instant>,
    /// Timestamps of recent poll/select wakeups for PollLimitIntervalSec/PollLimitBurst enforcement.
    pub poll_timestamps: Vec<std::time::Instant>,
    /// When set, the socket is paused due to poll rate limiting until this instant.
    pub poll_limit_paused_until: Option<std::time::Instant>,
}
