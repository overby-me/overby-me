//! systemd-notify — Notify the service manager about start-up completion
//! and other service status changes.
//!
//! A drop-in replacement for `systemd-notify(1)` supporting:
//!
//! - `--ready`      — Send READY=1 to indicate service startup is complete
//! - `--reloading`  — Send RELOADING=1 to indicate the service is reloading
//! - `--stopping`   — Send STOPPING=1 to indicate the service is stopping
//! - `--status=`    — Send STATUS=... with a descriptive status string
//! - `--booted`     — Check if the system was booted with systemd
//! - `--pid=`       — Send MAINPID=... to set the main PID of the service
//! - `--uid=`       — Set the UID for the notification message
//! - `--no-block`   — Do not block (currently a no-op for compatibility)
//!
//! Messages are sent to the Unix socket specified by the `$NOTIFY_SOCKET`
//! environment variable, matching the sd_notify(3) protocol.

use clap::Parser;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::process;

#[derive(Parser, Debug)]
#[command(
    name = "systemd-notify",
    about = "Notify the service manager about start-up completion and other status changes",
    version
)]
struct Cli {
    /// Inform the service manager about service start-up or configuration
    /// reload completion. This sends READY=1.
    #[arg(long)]
    ready: bool,

    /// Inform the service manager that the service is beginning to reload
    /// its configuration. This sends RELOADING=1.
    #[arg(long)]
    reloading: bool,

    /// Inform the service manager that the service is beginning its
    /// shutdown. This sends STOPPING=1.
    #[arg(long)]
    stopping: bool,

    /// Send a free-form human-readable status string to the service
    /// manager. This sends STATUS=...
    #[arg(long, value_name = "TEXT")]
    status: Option<String>,

    /// Return 0 if the system was booted with systemd, non-zero otherwise.
    /// This checks for the existence of /run/systemd/system/.
    #[arg(long)]
    booted: bool,

    /// Inform the service manager of the main PID of the daemon.
    /// Takes a PID as argument. If the argument is "auto" or "self" or
    /// omitted, the PID of the calling process is used.
    /// This sends MAINPID=...
    #[arg(long, value_name = "PID")]
    pid: Option<Option<String>>,

    /// Set the UID to send the notification from. This uses SO_PEERCRED
    /// style credential passing. Note: this typically requires root
    /// privileges.
    #[arg(long, value_name = "UID")]
    uid: Option<u32>,

    /// Do not synchronously wait for the notification to be processed.
    /// Currently a no-op for compatibility with the real systemd-notify.
    #[arg(long)]
    no_block: bool,

    /// Send the notification, then exec the remaining command line.
    /// This is useful to send notifications before exec'ing the actual
    /// daemon binary (e.g., `systemd-notify --exec --ready -- /usr/bin/mydaemon`).
    #[arg(long)]
    exec: bool,

    /// Fork off a background process that will execute the command
    /// given after `--`.  The parent prints the child's PID on stdout
    /// and exits.  Useful for launching daemons from shell scripts
    /// where the PID needs to be captured (e.g.,
    /// `PID=$(systemd-notify --fork -- my-daemon)`).
    #[arg(long)]
    fork: bool,

    /// Additional variables to send, in VAR=VALUE format.
    /// When --exec is used, everything after `;` or `--` is the command to exec.
    #[arg(trailing_var_arg = true)]
    variables: Vec<String>,
}

/// Resolve the NOTIFY_SOCKET path.
///
/// The `$NOTIFY_SOCKET` variable can be:
/// - An absolute path to a Unix socket
/// - A path prefixed with `@` for an abstract socket
fn resolve_notify_socket() -> Result<String, String> {
    std::env::var("NOTIFY_SOCKET")
        .map_err(|_| "NOTIFY_SOCKET environment variable is not set".to_string())
}

/// Send a notification message to the service manager via the notify socket.
fn send_notification(socket_path: &str, message: &str) -> Result<(), String> {
    let sock = UnixDatagram::unbound()
        .map_err(|e| format!("Failed to create Unix datagram socket: {e}"))?;

    if let Some(stripped) = socket_path.strip_prefix('@') {
        // Abstract socket: replace the leading '@' with a NUL byte.
        // Rust's UnixDatagram doesn't directly support abstract sockets
        // via the std API, so we use the raw address.
        let mut addr_bytes = vec![0u8]; // leading NUL for abstract namespace
        addr_bytes.extend_from_slice(stripped.as_bytes());

        // Use the lower-level nix or libc approach for abstract sockets.
        use std::os::unix::io::AsRawFd;

        let fd = sock.as_raw_fd();
        let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;

        // Abstract socket: sun_path[0] = 0, followed by the name
        let name = stripped;
        let name_bytes = name.as_bytes();
        let max_len = addr.sun_path.len() - 1; // -1 for leading NUL
        if name_bytes.len() > max_len {
            return Err(format!(
                "Abstract socket name too long: {} bytes (max {})",
                name_bytes.len(),
                max_len
            ));
        }
        // sun_path[0] is already 0 from zeroed()
        for (i, &b) in name_bytes.iter().enumerate() {
            addr.sun_path[i + 1] = b as libc::c_char;
        }

        let addr_len = std::mem::size_of::<libc::sa_family_t>() + 1 + name_bytes.len();
        let msg_bytes = message.as_bytes();

        let ret = unsafe {
            libc::sendto(
                fd,
                msg_bytes.as_ptr().cast(),
                msg_bytes.len(),
                libc::MSG_NOSIGNAL,
                (&addr as *const libc::sockaddr_un).cast(),
                addr_len as libc::socklen_t,
            )
        };

        if ret < 0 {
            return Err(format!(
                "Failed to send to abstract socket: {}",
                std::io::Error::last_os_error()
            ));
        }
    } else {
        // Regular filesystem socket
        let path = Path::new(socket_path);
        sock.send_to(message.as_bytes(), path)
            .map_err(|e| format!("Failed to send to {socket_path}: {e}"))?;
    }

    Ok(())
}

/// Check whether the system was booted with systemd.
///
/// This checks for the existence of `/run/systemd/system/` which is
/// created by systemd early during boot.
fn check_booted() -> bool {
    Path::new("/run/systemd/system").is_dir()
}

fn main() {
    let cli = Cli::parse();

    // --booted: just check and exit
    if cli.booted {
        if check_booted() {
            process::exit(0);
        } else {
            process::exit(1);
        }
    }

    // Build the notification message
    let mut parts: Vec<String> = Vec::new();

    if cli.ready {
        parts.push("READY=1".to_string());
    }

    if cli.reloading {
        parts.push("RELOADING=1".to_string());
        // systemd also sends MONOTONIC_USEC when reloading
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let ret = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        if ret == 0 {
            let usec = ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1_000;
            parts.push(format!("MONOTONIC_USEC={usec}"));
        }
    }

    if cli.stopping {
        parts.push("STOPPING=1".to_string());
    }

    if let Some(ref status) = cli.status {
        parts.push(format!("STATUS={status}"));
    }

    if let Some(ref pid_arg) = cli.pid {
        let pid = match pid_arg {
            Some(p) if p == "auto" || p == "self" => std::process::id(),
            Some(p) => match p.parse::<u32>() {
                Ok(pid) => pid,
                Err(e) => {
                    eprintln!("Error: invalid PID value '{p}': {e}");
                    process::exit(1);
                }
            },
            None => std::process::id(),
        };
        parts.push(format!("MAINPID={pid}"));
    }

    // Separate variables from exec command (when --exec or --fork is used).
    // Arguments after ";" or "--" are the command; bare tokens without '='
    // under --exec/--fork are also treated as command words.
    let use_cmd = cli.exec || cli.fork;
    let mut exec_cmd: Vec<String> = Vec::new();
    let mut found_separator = false;
    for var in &cli.variables {
        if use_cmd && (var == ";" || var == "--") {
            found_separator = true;
            continue;
        }
        if found_separator || (use_cmd && !var.contains('=')) {
            exec_cmd.push(var.clone());
        } else if var.contains('=') {
            parts.push(var.clone());
        } else if !use_cmd {
            eprintln!("Warning: ignoring argument without '=': {var}");
        }
    }

    // --fork with no notifications + command is valid — fork and exec.
    if cli.fork && !exec_cmd.is_empty() && parts.is_empty() {
        fork_exec_and_print_pid(&exec_cmd);
    }

    // If nothing to send, exit successfully
    if parts.is_empty() {
        // No notification to send — this is not an error per systemd-notify semantics.
        return;
    }

    let message = parts.join("\n");

    // Resolve the notify socket
    let socket_path = match resolve_notify_socket() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("Error: {e}");
            eprintln!("Note: systemd-notify must be invoked in a service context where ");
            eprintln!("$NOTIFY_SOCKET is set by the service manager.");
            process::exit(1);
        }
    };

    // If --uid was specified, we would need to change our effective UID
    // before sending. This requires root privileges.
    if let Some(uid) = cli.uid {
        let current_uid = unsafe { libc::getuid() };
        if current_uid != 0 && current_uid != uid {
            eprintln!(
                "Warning: --uid={uid} specified but running as UID {current_uid}; credential passing may not work"
            );
        }
        // We don't actually change UID as it would require privileges
        // and the kernel's SO_PEERCRED will use our real UID anyway.
        // Real systemd-notify uses SCM_CREDENTIALS for this.
    }

    if let Err(e) = send_notification(&socket_path, &message) {
        eprintln!("Error: {e}");
        process::exit(1);
    }

    // If --fork was specified with a command, fork+exec and print the child PID.
    if cli.fork && !exec_cmd.is_empty() {
        fork_exec_and_print_pid(&exec_cmd);
    }

    // If --exec was specified, exec the remaining command
    if cli.exec && !exec_cmd.is_empty() {
        use std::os::unix::process::CommandExt;
        let cmd = &exec_cmd[0];
        let args = &exec_cmd[1..];
        let err = process::Command::new(cmd).args(args).exec();
        eprintln!("Error: failed to exec {cmd}: {err}");
        process::exit(1);
    }
}

/// Fork a child that execs `cmd`.  The parent prints the child's PID
/// on stdout and exits 0.  If fork or exec fails, exits 1.
fn fork_exec_and_print_pid(cmd: &[String]) -> ! {
    use std::ffi::CString;
    match unsafe { libc::fork() } {
        -1 => {
            eprintln!("fork failed: {}", std::io::Error::last_os_error());
            process::exit(1);
        }
        0 => {
            // Detach from the controlling terminal / session so the child
            // survives when the invoking shell tears down its subshell
            // (e.g. `PID=$(systemd-notify --fork -- …)`).
            unsafe { libc::setsid() };
            // Redirect stdout/stderr to /dev/null so the `$(...)` reader
            // gets EOF as soon as the parent exits — otherwise the
            // subshell keeps blocking on the child's open stdout.
            unsafe {
                let devnull_ro = libc::open(b"/dev/null\0".as_ptr().cast(), libc::O_RDONLY);
                let devnull_wr = libc::open(b"/dev/null\0".as_ptr().cast(), libc::O_WRONLY);
                if devnull_ro >= 0 {
                    libc::dup2(devnull_ro, 0);
                    libc::close(devnull_ro);
                }
                if devnull_wr >= 0 {
                    libc::dup2(devnull_wr, 1);
                    libc::dup2(devnull_wr, 2);
                    libc::close(devnull_wr);
                }
            }
            let prog = match CString::new(cmd[0].as_str()) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("invalid program name: {e}");
                    process::exit(1);
                }
            };
            let args: Vec<CString> = cmd
                .iter()
                .map(|a| CString::new(a.as_str()).unwrap())
                .collect();
            let ptrs: Vec<*const libc::c_char> = args
                .iter()
                .map(|a| a.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();
            unsafe { libc::execvp(prog.as_ptr(), ptrs.as_ptr()) };
            eprintln!("exec {} failed: {}", cmd[0], std::io::Error::last_os_error());
            process::exit(1);
        }
        pid => {
            println!("{pid}");
            process::exit(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Mutex to serialize tests that manipulate the NOTIFY_SOCKET env var,
    /// since env vars are per-process global state and tests run in parallel.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_check_booted() {
        // This test is environment-dependent but should not panic
        let _ = check_booted();
    }

    #[test]
    fn test_resolve_notify_socket_missing() {
        let _lock = ENV_MUTEX.lock().unwrap();
        // Remove the env var if present (in test context)
        unsafe { std::env::remove_var("NOTIFY_SOCKET") };
        assert!(resolve_notify_socket().is_err());
    }

    #[test]
    fn test_resolve_notify_socket_present() {
        let _lock = ENV_MUTEX.lock().unwrap();
        unsafe { std::env::set_var("NOTIFY_SOCKET", "/run/systemd/notify") };
        assert_eq!(resolve_notify_socket().unwrap(), "/run/systemd/notify");
        unsafe { std::env::remove_var("NOTIFY_SOCKET") };
    }
}
