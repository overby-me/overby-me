use std::os::unix::io::{AsRawFd, RawFd};

use log::trace;

use crate::units::SocketConfig;

/// Configuration for a netlink socket (AF_NETLINK).
///
/// Netlink sockets are used for communication between the kernel and
/// user-space processes. The `family` field specifies the netlink protocol
/// (e.g. "route", "kobject-uevent", "audit") and `group` specifies the
/// multicast group to bind to (defaults to 0).
///
/// See systemd.socket(5) ListenNetlink= for details.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct NetlinkSocketConfig {
    /// The netlink protocol family name (e.g. "route", "kobject-uevent").
    pub family: String,
    /// The multicast group number (default 0).
    pub group: u32,
}

/// Resolves a netlink family name to the corresponding protocol number.
///
/// Supports both named families (case-insensitive) and numeric values.
/// Returns `None` if the name is not recognized and not a valid number.
pub fn resolve_netlink_family(name: &str) -> Option<i32> {
    // Try numeric first
    if let Ok(num) = name.parse::<i32>() {
        return Some(num);
    }

    match name.to_lowercase().as_str() {
        "route" => Some(libc::NETLINK_ROUTE),
        "usersock" => Some(libc::NETLINK_USERSOCK),
        "firewall" => Some(libc::NETLINK_FIREWALL),
        "sock-diag" | "inet-diag" => Some(libc::NETLINK_SOCK_DIAG),
        "nflog" => Some(libc::NETLINK_NFLOG),
        "xfrm" => Some(libc::NETLINK_XFRM),
        "selinux" => Some(libc::NETLINK_SELINUX),
        "iscsi" => Some(libc::NETLINK_ISCSI),
        "audit" => Some(libc::NETLINK_AUDIT),
        "fib-lookup" => Some(libc::NETLINK_FIB_LOOKUP),
        "connector" => Some(libc::NETLINK_CONNECTOR),
        "netfilter" => Some(libc::NETLINK_NETFILTER),
        "ip6-fw" => Some(libc::NETLINK_IP6_FW),
        "dnrtmsg" => Some(libc::NETLINK_DNRTMSG),
        "kobject-uevent" => Some(libc::NETLINK_KOBJECT_UEVENT),
        "generic" => Some(libc::NETLINK_GENERIC),
        "scsitransport" => Some(libc::NETLINK_SCSITRANSPORT),
        "ecryptfs" => Some(libc::NETLINK_ECRYPTFS),
        "rdma" => Some(libc::NETLINK_RDMA),
        "crypto" => Some(libc::NETLINK_CRYPTO),
        _ => None,
    }
}

/// Wrapper to hold a netlink raw fd so it implements AsRawFd + Send + Sync.
#[derive(Debug)]
struct NetlinkFd(Option<RawFd>);

impl AsRawFd for NetlinkFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0.unwrap()
    }
}

// SAFETY: Netlink fds are just file descriptors, safe to send across threads.
unsafe impl Send for NetlinkFd {}
unsafe impl Sync for NetlinkFd {}

impl Drop for NetlinkFd {
    fn drop(&mut self) {
        if let Some(fd) = self.0.take() {
            super::close_raw_fd(fd);
        }
    }
}

impl NetlinkSocketConfig {
    pub fn open(&self, _conf: &SocketConfig) -> Result<Box<dyn AsRawFd + Send + Sync>, String> {
        let protocol = resolve_netlink_family(&self.family).ok_or_else(|| {
            format!(
                "Unknown netlink family '{}'. Use a known name or numeric value.",
                self.family
            )
        })?;

        trace!(
            "opening netlink socket: family={} (protocol={}), group={}",
            self.family, protocol, self.group
        );

        // Create the AF_NETLINK socket
        let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM, protocol) };
        if fd < 0 {
            return Err(format!(
                "Failed to create netlink socket (family={}, protocol={}): {}",
                self.family,
                protocol,
                std::io::Error::last_os_error()
            ));
        }

        // Bind to the specified multicast group
        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as u16;
        addr.nl_pid = 0; // Let the kernel assign the port ID
        addr.nl_groups = self.group;

        let ret = unsafe {
            libc::bind(
                fd,
                &addr as *const libc::sockaddr_nl as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };

        if ret < 0 {
            let err = std::io::Error::last_os_error();
            unsafe {
                libc::close(fd);
            }
            return Err(format!(
                "Failed to bind netlink socket (family={}, group={}): {}",
                self.family, self.group, err
            ));
        }

        Ok(Box::new(NetlinkFd(Some(fd))))
    }

    pub fn close(&self, _rawfd: RawFd) -> Result<(), String> {
        // fd is closed by the OwnedFd wrapper (NetlinkFd) when dropped
        Ok(())
    }
}
