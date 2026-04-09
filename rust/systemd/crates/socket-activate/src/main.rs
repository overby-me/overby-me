//! systemd-socket-activate — Test socket activation by listening on sockets
//! and launching a program with the file descriptors.
//!
//! A drop-in replacement for `systemd-socket-activate(1)`. This tool creates
//! listening sockets (TCP, UDP, or Unix), then executes a specified program
//! with those sockets passed as file descriptors (starting at FD 3), along
//! with the `LISTEN_FDS` and `LISTEN_PID` environment variables set
//! according to the sd_listen_fds(3) protocol.
//!
//! This is primarily a development and debugging tool for testing services
//! that support socket activation without needing to configure actual
//! systemd socket units.
//!
//! Usage examples:
//!
//! ```sh
//! # Listen on TCP port 8080 and launch a web server
//! systemd-socket-activate -l 8080 /usr/bin/my-http-server
//!
//! # Listen on a Unix socket
//! systemd-socket-activate -l /tmp/my.sock /usr/bin/my-daemon
//!
//! # Listen on multiple sockets
//! systemd-socket-activate -l 8080 -l 8443 /usr/bin/my-server
//!
//! # UDP socket
//! systemd-socket-activate --datagram -l 5353 /usr/bin/my-dns
//!
//! # Accept connections and launch one instance per connection
//! systemd-socket-activate -a -l 8080 /usr/bin/my-handler
//! ```
//!
//! Exit codes:
//!   0 — The launched program exited successfully.
//!   Non-zero — Error during setup, or the launched program exited with
//!              a non-zero status.

use clap::Parser;
use std::ffi::CString;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::os::unix::io::{IntoRawFd, RawFd};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process;

/// The first file descriptor number used for socket activation (SD_LISTEN_FDS_START).
const LISTEN_FDS_START: RawFd = 3;

#[derive(Parser, Debug)]
#[command(
    name = "systemd-socket-activate",
    about = "Test socket activation of daemons",
    version,
    trailing_var_arg = true
)]
struct Cli {
    /// Add a socket address to listen on. This may be a port number
    /// (for TCP/UDP on all interfaces), a host:port pair, or an
    /// absolute path (for Unix domain sockets). May be specified
    /// multiple times.
    #[arg(short = 'l', long = "listen", value_name = "ADDR", required = true)]
    listen: Vec<String>,

    /// Use datagram (UDP/SOCK_DGRAM) sockets instead of stream
    /// (TCP/SOCK_STREAM) sockets for numeric addresses.
    #[arg(short = 'd', long)]
    datagram: bool,

    /// Accept a connection on each socket and spawn the program for
    /// each accepted connection, passing the connected socket instead
    /// of the listening socket.
    #[arg(short = 'a', long)]
    accept: bool,

    /// Set the socket receive buffer size in bytes.
    #[arg(long, value_name = "BYTES")]
    recv_buffer: Option<usize>,

    /// Launch only a single instance at a time when using --accept.
    /// Wait for each child to exit before accepting the next connection.
    #[arg(long)]
    foreground: bool,

    /// Set the environment variable name for the file descriptor count.
    /// Defaults to LISTEN_FDS.
    #[arg(short = 'E', long, value_name = "NAME", default_value = "LISTEN_FDS")]
    fdname: String,

    /// Set the listen backlog for stream sockets.
    #[arg(long, value_name = "NUM", default_value = "256")]
    backlog: i32,

    /// Specify the LISTEN_FDNAMES environment variable value.
    /// Colon-separated list of names for each listening socket.
    #[arg(long, value_name = "NAMES")]
    fdnames: Option<String>,

    /// The command to execute, followed by its arguments.
    #[arg(required = true)]
    command: Vec<String>,
}

/// Represents a listening socket that has been set up.
struct ListeningSocket {
    fd: RawFd,
    name: String,
    addr: String,
}

impl ListeningSocket {
    fn from_udp(socket: UdpSocket, addr: &str) -> Self {
        let fd = socket.into_raw_fd();
        ListeningSocket {
            fd,
            name: format!("udp:{addr}"),
            addr: addr.to_string(),
        }
    }

    fn from_unix(listener: UnixListener, path: &str) -> Self {
        let fd = listener.into_raw_fd();
        ListeningSocket {
            fd,
            name: format!("unix:{path}"),
            addr: path.to_string(),
        }
    }
}

/// Parse a listen address and create the appropriate socket.
///
/// The address can be:
/// - A bare port number (e.g. "8080") — binds to 0.0.0.0:port
/// - A host:port pair (e.g. "127.0.0.1:8080")
/// - An absolute path (e.g. "/tmp/my.sock") — Unix domain socket
/// - A bracketed IPv6 address with port (e.g. "[::1]:8080")
fn create_socket(addr: &str, datagram: bool, backlog: i32) -> io::Result<ListeningSocket> {
    // Check if it's a Unix socket path (starts with / or .)
    if addr.starts_with('/') || addr.starts_with('.') {
        return create_unix_socket(addr);
    }

    // Try parsing as a bare port number
    if let Ok(port) = addr.parse::<u16>() {
        let bind_addr: SocketAddr = ([0, 0, 0, 0], port).into();
        return if datagram {
            create_udp_socket(&bind_addr, addr)
        } else {
            create_tcp_socket(&bind_addr, addr, backlog)
        };
    }

    // Try parsing as a socket address (host:port or [host]:port)
    if let Ok(sock_addr) = addr.parse::<SocketAddr>() {
        return if datagram {
            create_udp_socket(&sock_addr, addr)
        } else {
            create_tcp_socket(&sock_addr, addr, backlog)
        };
    }

    // Try resolving as hostname:port
    use std::net::ToSocketAddrs;
    if let Ok(mut addrs) = addr.to_socket_addrs()
        && let Some(sock_addr) = addrs.next()
    {
        return if datagram {
            create_udp_socket(&sock_addr, addr)
        } else {
            create_tcp_socket(&sock_addr, addr, backlog)
        };
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("cannot parse listen address: {addr}"),
    ))
}

// ---------------------------------------------------------------------------
// SOCK_DESTROY — kill orphaned TCP listening sockets via netlink
// ---------------------------------------------------------------------------

/// Destroy orphaned TCP listening sockets on `port` (both IPv4 and IPv6).
/// Uses the NETLINK_SOCK_DIAG / SOCK_DESTROY kernel interface.
fn destroy_tcp_listeners_on_port(port: u16) {
    for family in [libc::AF_INET as u8, libc::AF_INET6 as u8] {
        destroy_tcp_listeners_family(port, family);
    }
}

fn destroy_tcp_listeners_family(port: u16, family: u8) {
    const NETLINK_SOCK_DIAG: libc::c_int = 4;
    const SOCK_DIAG_BY_FAMILY: u16 = 20;
    const SOCK_DESTROY: u16 = 21;
    const NLM_F_REQUEST: u16 = 0x0001;
    const NLM_F_DUMP: u16 = 0x0300;
    const NLMSG_DONE: u16 = 3;
    const TCPF_LISTEN: u32 = 1 << 10;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct SockId {
        sport: u16,
        dport: u16,
        src: [u32; 4],
        dst: [u32; 4],
        if_idx: u32,
        cookie: [u32; 2],
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct DiagReq {
        family: u8,
        protocol: u8,
        ext: u8,
        pad: u8,
        states: u32,
        id: SockId,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NlHdr {
        len: u32,
        typ: u16,
        flags: u16,
        seq: u32,
        pid: u32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct DiagMsg {
        family: u8,
        state: u8,
        timer: u8,
        retrans: u8,
        id: SockId,
        _rest: [u32; 5],
    }

    let nl = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            NETLINK_SOCK_DIAG,
        )
    };
    if nl < 0 {
        return;
    }
    let tv = libc::timeval {
        tv_sec: 2,
        tv_usec: 0,
    };
    unsafe {
        libc::setsockopt(
            nl,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            (&tv as *const libc::timeval).cast(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
    }

    let hdr_sz = std::mem::size_of::<NlHdr>();
    let req_sz = std::mem::size_of::<DiagReq>();
    let msg_sz = std::mem::size_of::<DiagMsg>();
    let total = hdr_sz + req_sz;

    // Dump LISTEN sockets
    let hdr = NlHdr {
        len: total as u32,
        typ: SOCK_DIAG_BY_FAMILY,
        flags: NLM_F_REQUEST | NLM_F_DUMP,
        seq: 1,
        pid: 0,
    };
    let req = DiagReq {
        family,
        protocol: libc::IPPROTO_TCP as u8,
        ext: 0,
        pad: 0,
        states: TCPF_LISTEN,
        id: SockId::default(),
    };

    let mut buf = vec![0u8; total];
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&hdr as *const NlHdr).cast::<u8>(),
            buf.as_mut_ptr(),
            hdr_sz,
        );
        std::ptr::copy_nonoverlapping(
            (&req as *const DiagReq).cast::<u8>(),
            buf.as_mut_ptr().add(hdr_sz),
            req_sz,
        );
    }
    if unsafe { libc::send(nl, buf.as_ptr().cast(), buf.len(), 0) } < 0 {
        unsafe { libc::close(nl) };
        return;
    }

    let port_be = port.to_be();
    let mut targets: Vec<(u8, SockId)> = Vec::new();
    let mut rbuf = vec![0u8; 32768];

    'outer: loop {
        let n = unsafe { libc::recv(nl, rbuf.as_mut_ptr().cast(), rbuf.len(), 0) };
        if n <= 0 {
            break;
        }
        let n = n as usize;
        let mut off = 0;
        while off + hdr_sz <= n {
            let h: NlHdr = unsafe { std::ptr::read_unaligned(rbuf.as_ptr().add(off).cast()) };
            if h.typ == NLMSG_DONE || h.len == 0 {
                break 'outer;
            }
            let payload = off + hdr_sz;
            if h.typ == SOCK_DIAG_BY_FAMILY && payload + msg_sz <= n {
                let m: DiagMsg =
                    unsafe { std::ptr::read_unaligned(rbuf.as_ptr().add(payload).cast()) };
                if m.id.sport == port_be {
                    targets.push((m.family, m.id));
                }
            }
            off += ((h.len as usize) + 3) & !3;
            if off == 0 {
                break;
            }
        }
    }

    for (fam, id) in &targets {
        let dh = NlHdr {
            len: total as u32,
            typ: SOCK_DESTROY,
            flags: NLM_F_REQUEST,
            seq: 2,
            pid: 0,
        };
        let dr = DiagReq {
            family: *fam,
            protocol: libc::IPPROTO_TCP as u8,
            ext: 0,
            pad: 0,
            states: TCPF_LISTEN,
            id: *id,
        };
        let mut dbuf = vec![0u8; total];
        unsafe {
            std::ptr::copy_nonoverlapping(
                (&dh as *const NlHdr).cast::<u8>(),
                dbuf.as_mut_ptr(),
                hdr_sz,
            );
            std::ptr::copy_nonoverlapping(
                (&dr as *const DiagReq).cast::<u8>(),
                dbuf.as_mut_ptr().add(hdr_sz),
                req_sz,
            );
            libc::send(nl, dbuf.as_ptr().cast(), dbuf.len(), 0);
        }
    }

    unsafe { libc::close(nl) };
}

fn create_tcp_socket(addr: &SocketAddr, name: &str, backlog: i32) -> io::Result<ListeningSocket> {
    // Use low-level socket API to set SO_REUSEADDR and custom backlog
    let domain = if addr.is_ipv4() {
        libc::AF_INET
    } else {
        libc::AF_INET6
    };

    let fd = unsafe { libc::socket(domain, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    // Set SO_REUSEADDR
    let optval: libc::c_int = 1;
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            (&optval as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }

    // Bind (with retry + SOCK_DESTROY for orphaned LISTEN sockets)
    let (sockaddr, socklen) = socket_addr_to_raw(addr);
    let mut ret = unsafe { libc::bind(fd, sockaddr.as_ptr().cast(), socklen) };
    if ret < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EADDRINUSE) {
        destroy_tcp_listeners_on_port(addr.port());
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            ret = unsafe { libc::bind(fd, sockaddr.as_ptr().cast(), socklen) };
            if ret == 0 {
                break;
            }
        }
    }
    if ret < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err);
    }

    // Listen
    let ret = unsafe { libc::listen(fd, backlog) };
    if ret < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err);
    }

    Ok(ListeningSocket {
        fd,
        name: format!("tcp:{name}"),
        addr: name.to_string(),
    })
}

fn create_udp_socket(addr: &SocketAddr, name: &str) -> io::Result<ListeningSocket> {
    let socket = UdpSocket::bind(addr)?;
    Ok(ListeningSocket::from_udp(socket, name))
}

fn create_unix_socket(path: &str) -> io::Result<ListeningSocket> {
    // Remove stale socket file if it exists
    let socket_path = Path::new(path);
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }

    let listener = UnixListener::bind(path)?;
    Ok(ListeningSocket::from_unix(listener, path))
}

/// Convert a SocketAddr to raw sockaddr bytes for libc calls.
fn socket_addr_to_raw(addr: &SocketAddr) -> (Vec<u8>, libc::socklen_t) {
    match addr {
        SocketAddr::V4(v4) => {
            let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
            sa.sin_family = libc::AF_INET as libc::sa_family_t;
            sa.sin_port = v4.port().to_be();
            sa.sin_addr.s_addr = u32::from_ne_bytes(v4.ip().octets());
            let len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    (&sa as *const libc::sockaddr_in).cast::<u8>(),
                    std::mem::size_of::<libc::sockaddr_in>(),
                )
            };
            (bytes.to_vec(), len)
        }
        SocketAddr::V6(v6) => {
            let mut sa: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
            sa.sin6_family = libc::AF_INET6 as libc::sa_family_t;
            sa.sin6_port = v6.port().to_be();
            sa.sin6_addr.s6_addr = v6.ip().octets();
            sa.sin6_flowinfo = v6.flowinfo();
            sa.sin6_scope_id = v6.scope_id();
            let len = std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t;
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    (&sa as *const libc::sockaddr_in6).cast::<u8>(),
                    std::mem::size_of::<libc::sockaddr_in6>(),
                )
            };
            (bytes.to_vec(), len)
        }
    }
}

/// Remove FD_CLOEXEC from a file descriptor so it is inherited across exec.
fn clear_cloexec(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let ret = unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Move a file descriptor to the target FD number if it's not already there.
fn move_fd(from: RawFd, to: RawFd) -> io::Result<()> {
    if from == to {
        clear_cloexec(to)?;
        return Ok(());
    }

    let ret = unsafe { libc::dup2(from, to) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    unsafe {
        libc::close(from);
    }
    clear_cloexec(to)?;
    Ok(())
}

/// Set the receive buffer size on a socket.
fn set_recv_buffer(fd: RawFd, size: usize) -> io::Result<()> {
    let size_val = size as libc::c_int;
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            (&size_val as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Execute the command with the socket FDs set up.
fn exec_command(command: &[String], num_fds: usize, fd_names: Option<&str>) -> io::Result<()> {
    if command.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no command specified",
        ));
    }

    // Set LISTEN_FDS and LISTEN_PID
    let pid = process::id();
    unsafe {
        std::env::set_var("LISTEN_FDS", num_fds.to_string());
        std::env::set_var("LISTEN_PID", pid.to_string());
    }

    // Set LISTEN_FDNAMES if provided
    if let Some(names) = fd_names {
        unsafe {
            std::env::set_var("LISTEN_FDNAMES", names);
        }
    }

    // Convert command to CStrings for execvp
    let prog = CString::new(command[0].as_str())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    let args: Vec<CString> = command
        .iter()
        .map(|a| CString::new(a.as_str()).unwrap())
        .collect();

    let arg_ptrs: Vec<*const libc::c_char> = args
        .iter()
        .map(|a| a.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    unsafe {
        libc::execvp(prog.as_ptr(), arg_ptrs.as_ptr());
    }

    // If we get here, exec failed
    Err(io::Error::last_os_error())
}

/// Accept a connection and spawn a child process to handle it.
fn accept_and_spawn(
    listen_fd: RawFd,
    command: &[String],
    fd_names: Option<&str>,
    foreground: bool,
) -> io::Result<()> {
    let conn_fd = unsafe {
        libc::accept4(
            listen_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            libc::SOCK_CLOEXEC,
        )
    };
    if conn_fd < 0 {
        return Err(io::Error::last_os_error());
    }

    let pid = unsafe { libc::fork() };
    match pid {
        -1 => {
            unsafe { libc::close(conn_fd) };
            Err(io::Error::last_os_error())
        }
        0 => {
            // Child process
            // Close the listening socket
            unsafe { libc::close(listen_fd) };

            // Move the connected socket to FD 3
            move_fd(conn_fd, LISTEN_FDS_START)?;

            // exec the command
            exec_command(command, 1, fd_names)?;
            process::exit(1);
        }
        child_pid => {
            // Parent process
            unsafe { libc::close(conn_fd) };

            if foreground {
                // Wait for child to exit
                let mut status: libc::c_int = 0;
                unsafe {
                    libc::waitpid(child_pid, &mut status, 0);
                }
            }

            Ok(())
        }
    }
}

/// Clean up Unix socket files on exit.
fn cleanup_unix_sockets(sockets: &[ListeningSocket]) {
    for sock in sockets {
        if sock.addr.starts_with('/') || sock.addr.starts_with('.') {
            let _ = std::fs::remove_file(&sock.addr);
        }
    }
}

fn main() {
    let cli = Cli::parse();

    // Create all listening sockets
    let mut sockets: Vec<ListeningSocket> = Vec::new();

    for addr in &cli.listen {
        match create_socket(addr, cli.datagram, cli.backlog) {
            Ok(sock) => {
                eprintln!("Listening on {} (fd {})", sock.name, sock.fd);
                sockets.push(sock);
            }
            Err(e) => {
                eprintln!("Failed to listen on {addr}: {e}");
                cleanup_unix_sockets(&sockets);
                process::exit(1);
            }
        }
    }

    if sockets.is_empty() {
        eprintln!("No sockets to listen on.");
        process::exit(1);
    }

    // Set receive buffer size if requested
    if let Some(recv_buf) = cli.recv_buffer {
        for sock in &sockets {
            if let Err(e) = set_recv_buffer(sock.fd, recv_buf) {
                eprintln!(
                    "Warning: failed to set receive buffer on {}: {e}",
                    sock.name
                );
            }
        }
    }

    // Build LISTEN_FDNAMES
    let fd_names = if let Some(ref names) = cli.fdnames {
        Some(names.clone())
    } else {
        let names: Vec<&str> = sockets.iter().map(|s| s.name.as_str()).collect();
        Some(names.join(":"))
    };

    if cli.accept {
        // Accept mode: wait for connections on each socket and spawn
        // a child process for each one. For simplicity with multiple
        // listen sockets, we use the first one.
        if sockets.len() > 1 {
            eprintln!("Warning: --accept mode with multiple sockets uses only the first socket.");
            eprintln!("Use separate instances for multiple sockets with --accept.");
        }

        let listen_fd = sockets[0].fd;
        clear_cloexec(listen_fd).unwrap_or_else(|e| {
            eprintln!("Failed to clear CLOEXEC on fd: {e}");
            process::exit(1);
        });

        // Install SIGCHLD handler to reap zombies (unless foreground mode)
        if !cli.foreground {
            unsafe {
                let mut sa: libc::sigaction = std::mem::zeroed();
                sa.sa_flags = libc::SA_NOCLDWAIT;
                sa.sa_sigaction = libc::SIG_DFL;
                libc::sigaction(libc::SIGCHLD, &sa, std::ptr::null_mut());
            }
        }

        eprintln!(
            "Accepting connections on {} and spawning: {}",
            sockets[0].name,
            cli.command.join(" ")
        );

        loop {
            if let Err(e) =
                accept_and_spawn(listen_fd, &cli.command, fd_names.as_deref(), cli.foreground)
            {
                eprintln!("Error accepting connection: {e}");
                // Brief sleep to avoid tight error loop
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    } else {
        // Standard mode: move all socket FDs to the correct positions
        // (FD 3, 4, 5, ...) and exec the command.
        let num_fds = sockets.len();

        // First, collect all current FDs and the target FDs to avoid
        // conflicts during dup2.
        let mut fd_moves: Vec<(RawFd, RawFd)> = Vec::new();
        for (i, sock) in sockets.iter().enumerate() {
            let target_fd = LISTEN_FDS_START + i as RawFd;
            fd_moves.push((sock.fd, target_fd));
        }

        // Check for conflicts: if any source FD equals another's target FD,
        // we need to dup it out of the way first.
        let target_fds: Vec<RawFd> = fd_moves.iter().map(|&(_, t)| t).collect();
        let mut safe_moves: Vec<(RawFd, RawFd)> = Vec::new();

        for &(src, tgt) in &fd_moves {
            if src == tgt {
                // Already in the right place, just clear CLOEXEC
                safe_moves.push((src, tgt));
            } else if target_fds.contains(&src) {
                // Source conflicts with another target — dup it first
                let tmp = unsafe { libc::dup(src) };
                if tmp < 0 {
                    eprintln!("Failed to dup fd {src}: {}", io::Error::last_os_error());
                    cleanup_unix_sockets(&sockets);
                    process::exit(1);
                }
                unsafe { libc::close(src) };
                safe_moves.push((tmp, tgt));
            } else {
                safe_moves.push((src, tgt));
            }
        }

        for (src, tgt) in safe_moves {
            if let Err(e) = move_fd(src, tgt) {
                eprintln!("Failed to move fd {src} -> {tgt}: {e}");
                cleanup_unix_sockets(&sockets);
                process::exit(1);
            }
        }

        eprintln!(
            "Executing: {} (with {} socket fds)",
            cli.command.join(" "),
            num_fds
        );

        // exec replaces our process, so sockets won't be dropped normally.
        // Forget them to avoid the Drop impl closing the FDs.
        std::mem::forget(sockets);

        if let Err(e) = exec_command(&cli.command, num_fds, fd_names.as_deref()) {
            eprintln!("Failed to exec '{}': {e}", cli.command[0]);
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_listen_fds_start() {
        assert_eq!(LISTEN_FDS_START, 3);
    }

    #[test]
    fn test_socket_addr_to_raw_v4() {
        let addr: SocketAddr = ([127, 0, 0, 1], 8080).into();
        let (bytes, len) = socket_addr_to_raw(&addr);
        assert!(!bytes.is_empty());
        assert_eq!(len as usize, std::mem::size_of::<libc::sockaddr_in>());
    }

    #[test]
    fn test_socket_addr_to_raw_v6() {
        let addr: SocketAddr =
            SocketAddr::new(std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), 9090);
        let (bytes, len) = socket_addr_to_raw(&addr);
        assert!(!bytes.is_empty());
        assert_eq!(len as usize, std::mem::size_of::<libc::sockaddr_in6>());
    }

    #[test]
    fn test_create_tcp_socket() {
        // Use port 0 to let the OS assign a free port
        let addr: SocketAddr = ([127, 0, 0, 1], 0).into();
        let sock = create_tcp_socket(&addr, "127.0.0.1:0", 128).unwrap();
        assert!(sock.fd >= 0);
        assert!(sock.name.starts_with("tcp:"));
        unsafe { libc::close(sock.fd) };
    }

    #[test]
    fn test_create_udp_socket() {
        let addr: SocketAddr = ([127, 0, 0, 1], 0).into();
        let sock = create_udp_socket(&addr, "127.0.0.1:0").unwrap();
        assert!(sock.fd >= 0);
        assert!(sock.name.starts_with("udp:"));
        unsafe { libc::close(sock.fd) };
    }

    #[test]
    fn test_create_unix_socket() {
        let dir = std::env::temp_dir();
        let path = dir.join("socket-activate-test.sock");
        let path_str = path.to_str().unwrap();

        let sock = create_unix_socket(path_str).unwrap();
        assert!(sock.fd >= 0);
        assert!(sock.name.starts_with("unix:"));

        unsafe { libc::close(sock.fd) };
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_create_socket_port_number() {
        let sock = create_socket("0", false, 128).unwrap();
        assert!(sock.fd >= 0);
        unsafe { libc::close(sock.fd) };
    }

    #[test]
    fn test_create_socket_invalid() {
        let result = create_socket("not-a-valid-address-at-all:99999", false, 128);
        assert!(result.is_err());
    }

    #[test]
    fn test_clear_cloexec() {
        // Create a pipe and test clearing CLOEXEC
        let mut fds = [0i32; 2];
        let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
        assert_eq!(ret, 0);

        // Verify CLOEXEC is set
        let flags = unsafe { libc::fcntl(fds[0], libc::F_GETFD) };
        assert!(flags & libc::FD_CLOEXEC != 0);

        // Clear it
        clear_cloexec(fds[0]).unwrap();

        // Verify CLOEXEC is cleared
        let flags = unsafe { libc::fcntl(fds[0], libc::F_GETFD) };
        assert!(flags & libc::FD_CLOEXEC == 0);

        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }

    #[test]
    fn test_move_fd_same() {
        let mut fds = [0i32; 2];
        let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
        assert_eq!(ret, 0);

        // Moving to same FD should just clear CLOEXEC
        move_fd(fds[0], fds[0]).unwrap();

        let flags = unsafe { libc::fcntl(fds[0], libc::F_GETFD) };
        assert!(flags & libc::FD_CLOEXEC == 0);

        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }

    #[test]
    fn test_set_recv_buffer() {
        let addr: SocketAddr = ([127, 0, 0, 1], 0).into();
        let sock = create_tcp_socket(&addr, "test", 128).unwrap();

        // Setting recv buffer should succeed
        let result = set_recv_buffer(sock.fd, 65536);
        assert!(result.is_ok());

        unsafe { libc::close(sock.fd) };
    }

    #[test]
    fn test_cleanup_unix_sockets() {
        let dir = std::env::temp_dir();
        let path = dir.join("socket-activate-cleanup-test.sock");
        let path_str = path.to_str().unwrap().to_string();

        // Create a file to simulate a socket
        std::fs::write(&path, "test").unwrap();
        assert!(path.exists());

        let sockets = vec![ListeningSocket {
            fd: -1,
            name: format!("unix:{path_str}"),
            addr: path_str,
        }];

        cleanup_unix_sockets(&sockets);
        assert!(!path.exists());
    }
}
