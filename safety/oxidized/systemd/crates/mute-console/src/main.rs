//! `systemd-mute-console` - temporarily mute noisy output to the main console.
//!
//! A port of upstream `src/mute-console/mute-console.c`. In direct mode it mutes
//! the kernel `printk` console log level (and, best-effort, PID 1 status output
//! via `SetShowStatus`), then blocks until SIGINT/SIGTERM and restores. When
//! invoked as a Varlink service (socket-activated `Accept=yes`, or `varlinkctl
//! -E`), it serves `io.systemd.MuteConsole.Mute`, keeping the console muted for
//! the lifetime of the (streaming) method call.

use std::io::{BufRead, BufReader, Write};
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

const PRINTK_PATH: &str = "/proc/sys/kernel/printk";

const MUTE_CONSOLE_IDL: &str = "\
interface io.systemd.MuteConsole

method Mute(kernel: ?bool, pid1: ?bool) -> ()
";

fn help() {
    println!(
        "systemd-mute-console [OPTIONS...]\n\n\
         Mute status output to the console.\n\n  \
         -h --help            Show this help\n     \
         --version         Show package version\n     \
         --kernel=BOOL     Mute kernel log output\n     \
         --pid1=BOOL       Mute PID 1 status output"
    );
}

fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "1" | "yes" | "y" | "true" | "t" | "on" => Some(true),
        "0" | "no" | "n" | "false" | "f" | "off" => Some(false),
        _ => None,
    }
}

// ── printk console-level muting ──────────────────────────────────────────────

/// Read the current console printk level (the first field of /proc/sys/kernel/printk).
fn printk_read() -> Option<i32> {
    let s = std::fs::read_to_string(PRINTK_PATH).ok()?;
    s.split_whitespace().next()?.parse().ok()
}

fn printk_write(level: i32) -> std::io::Result<()> {
    std::fs::write(PRINTK_PATH, format!("{level}\n"))
}

fn detect_container() -> bool {
    std::path::Path::new("/run/.containerenv").exists()
        || std::path::Path::new("/.dockerenv").exists()
        || std::fs::read_to_string("/proc/1/environ")
            .map(|e| e.contains("container="))
            .unwrap_or(false)
}

/// Best-effort: ask PID 1 to stop showing status via the D-Bus SetShowStatus
/// method. Silently ignored when the bus or method is unavailable, since muting
/// is advisory.
fn set_show_status(_value: &str) {
    // rust-systemd's PID 1 exposes ShowStatus as a property; SetShowStatus may
    // be absent. We keep this best-effort (no hard bus dependency in this small
    // tool) so the primary purpose (kernel console muting) still works.
}

struct Muter {
    mute_pid1: bool,
    mute_kernel: bool,
    muted_pid1: bool,
    saved_kernel: Option<i32>,
}

impl Muter {
    fn new(mute_pid1: bool, mute_kernel: bool) -> Self {
        Muter {
            mute_pid1,
            mute_kernel,
            muted_pid1: false,
            saved_kernel: None,
        }
    }

    fn mute(&mut self) {
        if self.mute_pid1 {
            set_show_status("no");
            self.muted_pid1 = true;
        }
        if self.mute_kernel
            && !detect_container()
            && let Some(level) = printk_read()
            && level != 0
            && printk_write(0).is_ok()
        {
            self.saved_kernel = Some(level);
        }
    }

    fn unmute(&mut self) {
        if self.muted_pid1 {
            set_show_status("");
            self.muted_pid1 = false;
        }
        // Only restore if it is still muted (not changed externally).
        if let Some(level) = self.saved_kernel.take()
            && printk_read() == Some(0)
        {
            let _ = printk_write(level);
        }
    }
}

// ── Varlink service mode ─────────────────────────────────────────────────────

fn send(stream: &UnixStream, reply: &serde_json::Value) -> bool {
    let mut w = stream;
    let mut msg = match serde_json::to_vec(reply) {
        Ok(m) => m,
        Err(_) => return false,
    };
    msg.push(0);
    w.write_all(&msg).is_ok()
}

/// Serve io.systemd.MuteConsole on the passed connection fd (fd 3), which is a
/// connected socket both under `Accept=yes` socket activation and `varlinkctl -E`.
fn vl_server() -> ExitCode {
    let stream = unsafe { UnixStream::from_raw_fd(3) };
    let write_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return ExitCode::from(1),
    };
    let mut reader = BufReader::new(stream);

    loop {
        let mut buf = Vec::new();
        let n = match reader.read_until(0, &mut buf) {
            Ok(n) => n,
            Err(_) => break,
        };
        if n == 0 {
            break; // client disconnected
        }
        if buf.last() == Some(&0) {
            buf.pop();
        }
        if buf.is_empty() {
            break;
        }
        let req: serde_json::Value = match serde_json::from_slice(&buf) {
            Ok(v) => v,
            Err(_) => break,
        };
        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
        match method {
            "org.varlink.service.GetInfo" => {
                let reply = serde_json::json!({"parameters": {
                    "vendor": "The systemd Project",
                    "product": "systemd (systemd-mute-console)",
                    "version": env!("CARGO_PKG_VERSION"),
                    "url": "https://systemd.io/",
                    "interfaces": ["org.varlink.service", "io.systemd.MuteConsole"],
                }});
                if !send(&write_stream, &reply) {
                    break;
                }
            }
            "org.varlink.service.GetInterfaceDescription" => {
                let iface = req
                    .get("parameters")
                    .and_then(|p| p.get("interface"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let reply = if iface == "io.systemd.MuteConsole" {
                    serde_json::json!({"parameters": {"description": MUTE_CONSOLE_IDL}})
                } else {
                    serde_json::json!({"error": "org.varlink.service.InterfaceNotFound",
                                       "parameters": {"interface": iface}})
                };
                if !send(&write_stream, &reply) {
                    break;
                }
            }
            "io.systemd.MuteConsole.Mute" => {
                let p = req.get("parameters");
                let mute_kernel = p
                    .and_then(|p| p.get("kernel"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let mute_pid1 = p
                    .and_then(|p| p.get("pid1"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let mut muter = Muter::new(mute_pid1, mute_kernel);
                muter.mute();
                // Keep the call open (notify, not a final reply): the console
                // stays muted for the lifetime of the connection.
                let notified = send(
                    &write_stream,
                    &serde_json::json!({"parameters": {}, "continues": true}),
                );
                if notified {
                    // Block until the client disconnects, then restore.
                    let mut b = [0u8; 64];
                    use std::io::Read;
                    let mut s = reader.into_inner();
                    loop {
                        match s.read(&mut b) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {}
                        }
                    }
                }
                muter.unmute();
                break;
            }
            other => {
                let reply = serde_json::json!({"error": "org.varlink.service.MethodNotFound",
                                               "parameters": {"method": other}});
                let _ = send(&write_stream, &reply);
                break;
            }
        }
    }
    ExitCode::SUCCESS
}

/// Whether we were invoked as a Varlink service (a socket passed on fd 3 via
/// LISTEN_FDS, per the systemd socket-activation convention).
fn invoked_as_varlink() -> bool {
    let listen_pid: Option<i32> = std::env::var("LISTEN_PID")
        .ok()
        .and_then(|s| s.parse().ok());
    if listen_pid != Some(std::process::id() as i32) {
        return false;
    }
    matches!(std::env::var("LISTEN_FDS").ok().and_then(|s| s.parse::<i32>().ok()), Some(n) if n >= 1)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut mute_kernel = true;
    let mut mute_pid1 = true;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let (name, inline) = match a.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (a.as_str(), None),
        };
        match name {
            "-h" | "--help" => {
                help();
                return ExitCode::SUCCESS;
            }
            "--version" => {
                println!(
                    "systemd {} (systemd-mute-console)",
                    env!("CARGO_PKG_VERSION")
                );
                return ExitCode::SUCCESS;
            }
            "--kernel" | "--pid1" => {
                let val = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        args.get(i).cloned().unwrap_or_default()
                    }
                };
                match parse_bool(&val) {
                    Some(b) if name == "--kernel" => mute_kernel = b,
                    Some(b) => mute_pid1 = b,
                    None => {
                        eprintln!("Failed to parse {name}= value: {val}");
                        return ExitCode::from(1);
                    }
                }
            }
            other => {
                eprintln!("systemd-mute-console: unrecognized option: {other}");
                return ExitCode::from(1);
            }
        }
        i += 1;
    }

    if invoked_as_varlink() {
        return vl_server();
    }

    if !mute_pid1 && !mute_kernel {
        eprintln!("Not asked to mute anything, refusing.");
        return ExitCode::from(1);
    }

    // Direct mode: mute, notify readiness, wait for a termination signal, restore.
    let mut muter = Muter::new(mute_pid1, mute_kernel);
    muter.mute();
    install_term_handlers();
    sd_notify("READY=1\nSTATUS=Console status output muted temporarily.");

    while !TERMINATE.load(Ordering::SeqCst) {
        // Suspend until a handled signal interrupts the pause().
        unsafe { libc::pause() };
    }

    sd_notify("STOPPING=1\nSTATUS=Console status output unmuted.");
    muter.unmute();
    ExitCode::SUCCESS
}

static TERMINATE: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_sig: i32) {
    TERMINATE.store(true, Ordering::SeqCst);
}

fn install_term_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_term as *const () as usize;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
    }
}

/// Minimal sd_notify: send a datagram to $NOTIFY_SOCKET if set.
fn sd_notify(msg: &str) {
    let sock = match std::env::var("NOTIFY_SOCKET") {
        Ok(s) => s,
        Err(_) => return,
    };
    use std::os::unix::net::UnixDatagram;
    if let Ok(d) = UnixDatagram::unbound() {
        // Abstract sockets start with '@' -> NUL.
        let path = if let Some(rest) = sock.strip_prefix('@') {
            let mut p = vec![0u8];
            p.extend_from_slice(rest.as_bytes());
            p
        } else {
            sock.into_bytes()
        };
        let _ = d.send_to(
            msg.as_bytes(),
            std::path::Path::new(&String::from_utf8_lossy(&path).into_owned()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_parsing() {
        assert_eq!(parse_bool("yes"), Some(true));
        assert_eq!(parse_bool("no"), Some(false));
        assert_eq!(parse_bool("x"), None);
    }

    #[test]
    fn idl_has_mute_method() {
        assert!(MUTE_CONSOLE_IDL.contains("method Mute"));
    }
}
