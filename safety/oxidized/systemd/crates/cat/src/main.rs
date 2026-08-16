//! systemd-cat — Connect a pipeline or program's output to the journal.
//!
//! A drop-in replacement for `systemd-cat(1)`. This tool connects
//! stdout and stderr of a command (or stdin if no command is given)
//! to the systemd journal using the journal stdout stream protocol.
//!
//! When a command is given, systemd-cat execs it directly with
//! stdout/stderr connected to the journal socket, so the kernel
//! attaches each writer's real PID via SCM_CREDENTIALS.
//!
//! Supported options:
//!
//! - `-t`, `--identifier=ID`  — Set the syslog identifier (default: "unknown")
//! - `-p`, `--priority=PRIO`  — Set the default log priority for stdout
//! - `--stderr-priority=PRIO` — Set the log priority for stderr
//! - `--level-prefix=BOOL`    — Strip kernel-style `<N>` priority prefixes from lines

use clap::Parser;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::process;

/// The path to the journal stdout stream socket.
const JOURNAL_STDOUT_SOCKET: &str = "/run/systemd/journal/stdout";

/// Syslog priority levels (matching <syslog.h> and systemd conventions).
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
enum Priority {
    Emergency = 0,
    Alert = 1,
    Critical = 2,
    Error = 3,
    Warning = 4,
    Notice = 5,
    Info = 6,
    Debug = 7,
}

impl Priority {
    fn from_str_or_num(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "emerg" | "emergency" | "0" => Ok(Priority::Emergency),
            "alert" | "1" => Ok(Priority::Alert),
            "crit" | "critical" | "2" => Ok(Priority::Critical),
            "err" | "error" | "3" => Ok(Priority::Error),
            "warning" | "warn" | "4" => Ok(Priority::Warning),
            "notice" | "5" => Ok(Priority::Notice),
            "info" | "6" => Ok(Priority::Info),
            "debug" | "7" => Ok(Priority::Debug),
            _ => Err(format!("Unknown priority: {s}")),
        }
    }

    fn as_u8(self) -> u8 {
        self as u8
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "systemd-cat",
    about = "Connect a pipeline or program's output to the journal",
    version
)]
struct Cli {
    /// Set the syslog identifier string. Defaults to the name of the
    /// executed command, or "unknown" if reading from stdin.
    #[arg(short = 't', long = "identifier", value_name = "ID")]
    identifier: Option<String>,

    /// Set the default priority for stdout messages.
    #[arg(short, long, default_value = "info", value_name = "PRIORITY")]
    priority: String,

    /// Set the priority for stderr messages.
    #[arg(long, value_name = "PRIORITY")]
    stderr_priority: Option<String>,

    /// Parse and strip kernel-style priority prefixes (<0> through <7>)
    /// from input lines.
    #[arg(long, default_value = "true", value_name = "BOOL")]
    level_prefix: String,

    /// Write to a journal namespace instead of the default journal.
    #[arg(long, value_name = "NAMESPACE")]
    namespace: Option<String>,

    /// The command to execute. If omitted, reads from stdin.
    #[arg(trailing_var_arg = true)]
    command: Vec<String>,
}

/// Open a journal stdout stream connection and send the protocol header.
///
/// The journal stdout stream protocol (SOCK_STREAM on /run/systemd/journal/stdout)
/// expects a 7-line header:
///   1. identifier (syslog tag)
///   2. unit_id (empty for standalone processes)
///   3. priority (decimal)
///   4. level_prefix (0 or 1)
///   5. forward_to_syslog (0 or 1)
///   6. forward_to_kmsg (0 or 1)
///   7. forward_to_console (0 or 1)
fn open_journal_stream(
    socket_path: &str,
    identifier: &str,
    priority: Priority,
    level_prefix: bool,
) -> Result<UnixStream, std::io::Error> {
    let mut stream = UnixStream::connect(socket_path)?;

    // Enable SO_PASSCRED on the sender socket so the kernel attaches
    // per-write credentials (PID/UID/GID) to every message.  This avoids
    // a race where the receiver hasn't set SO_PASSCRED yet at write time.
    let enabled: libc::c_int = 1;
    unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PASSCRED,
            &enabled as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }

    let header = format!(
        "{}\n\n{}\n{}\n0\n0\n0\n",
        identifier,
        priority.as_u8(),
        if level_prefix { 1 } else { 0 },
    );
    stream.write_all(header.as_bytes())?;

    Ok(stream)
}

/// Process lines from a reader and write them to the journal stream.
fn process_lines<R: Read>(
    reader: R,
    stream: &mut UnixStream,
    identifier: &str,
    default_priority: Priority,
    _level_prefix: bool,
) {
    let buf_reader = BufReader::new(reader);
    for line in buf_reader.lines() {
        match line {
            Ok(line) => {
                if let Err(e) = writeln!(stream, "{}", line) {
                    eprintln!(
                        "<{}>{}[{}]: {}",
                        default_priority.as_u8(),
                        identifier,
                        process::id(),
                        line
                    );
                    if e.kind() == std::io::ErrorKind::BrokenPipe {
                        break;
                    }
                }
            }
            Err(e) => {
                eprintln!("systemd-cat: error reading input: {e}");
                break;
            }
        }
    }
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
}

fn parse_bool_string(s: &str) -> bool {
    matches!(s.to_lowercase().as_str(), "true" | "1" | "yes")
}

fn main() {
    let cli = Cli::parse();
    let level_prefix = parse_bool_string(&cli.level_prefix);

    let stdout_priority = Priority::from_str_or_num(&cli.priority).unwrap_or_else(|e| {
        eprintln!("systemd-cat: {e}");
        process::exit(1);
    });

    let stderr_priority = match &cli.stderr_priority {
        Some(s) => Priority::from_str_or_num(s).unwrap_or_else(|e| {
            eprintln!("systemd-cat: {e}");
            process::exit(1);
        }),
        None => stdout_priority,
    };

    let journal_socket = match &cli.namespace {
        Some(ns) => format!("/run/systemd/journal.{ns}/stdout"),
        None => JOURNAL_STDOUT_SOCKET.to_string(),
    };

    if cli.command.is_empty() {
        // No command — read from stdin and relay to journal
        let identifier = cli.identifier.as_deref().unwrap_or("unknown");
        let mut stream =
            match open_journal_stream(&journal_socket, identifier, stdout_priority, level_prefix) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!(
                        "systemd-cat: failed to connect to journal stream {journal_socket}: {e}"
                    );
                    process::exit(1);
                }
            };
        let stdin = std::io::stdin();
        process_lines(
            stdin.lock(),
            &mut stream,
            identifier,
            stdout_priority,
            level_prefix,
        );
    } else {
        // Command given — exec it with stdout/stderr connected directly to
        // the journal socket.  This way the kernel's SCM_CREDENTIALS
        // reflects the real writer PID, not systemd-cat's.
        let cmd = &cli.command[0];
        let identifier = cli.identifier.as_deref().unwrap_or(cmd.as_str());

        let stdout_stream =
            match open_journal_stream(&journal_socket, identifier, stdout_priority, level_prefix) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("systemd-cat: failed to connect to journal stream: {e}");
                    process::exit(1);
                }
            };
        let stdout_fd = stdout_stream.as_raw_fd();

        // Get JOURNAL_STREAM env var (device:inode)
        let journal_stream_env = {
            let mut stat: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe { libc::fstat(stdout_fd, &mut stat) } == 0 {
                Some(format!("{}:{}", stat.st_dev, stat.st_ino))
            } else {
                None
            }
        };

        // Open separate stderr stream if priority differs, otherwise share stdout's
        let stderr_fd = if cli.stderr_priority.is_some() {
            match open_journal_stream(&journal_socket, identifier, stderr_priority, level_prefix) {
                Ok(s) => {
                    let fd = s.as_raw_fd();
                    // Prevent drop from closing the fd — we'll manage it manually
                    std::mem::forget(s);
                    fd
                }
                Err(_) => stdout_fd,
            }
        } else {
            stdout_fd
        };

        unsafe {
            // Wire up stdout → journal socket
            if stdout_fd != libc::STDOUT_FILENO && libc::dup2(stdout_fd, libc::STDOUT_FILENO) < 0 {
                eprintln!("systemd-cat: dup2 stdout failed");
                process::exit(1);
            }

            // Wire up stderr → journal socket (same or different fd)
            if stderr_fd != libc::STDERR_FILENO && libc::dup2(stderr_fd, libc::STDERR_FILENO) < 0 {
                // Can't eprintln here safely, but try anyway
                libc::_exit(1);
            }

            // Close originals if they're not stdio fds
            if stdout_fd > libc::STDERR_FILENO {
                libc::close(stdout_fd);
            }
            if stderr_fd > libc::STDERR_FILENO && stderr_fd != stdout_fd {
                libc::close(stderr_fd);
            }
        }

        // Prevent Rust from closing the fd via Drop
        std::mem::forget(stdout_stream);

        // Build argv for exec
        let c_cmd =
            std::ffi::CString::new(cli.command[0].as_str()).expect("command contains null byte");
        let c_args: Vec<std::ffi::CString> = cli
            .command
            .iter()
            .map(|a| std::ffi::CString::new(a.as_str()).expect("arg contains null byte"))
            .collect();
        let c_argv: Vec<*const libc::c_char> = c_args
            .iter()
            .map(|a| a.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();

        // Set JOURNAL_STREAM env
        if let Some(val) = journal_stream_env {
            // Safety: we are about to exec, no other threads are running
            unsafe { std::env::set_var("JOURNAL_STREAM", val) };
        }

        unsafe {
            libc::execvp(c_cmd.as_ptr(), c_argv.as_ptr());
            // exec failed
            let err = *libc::__errno_location();
            eprintln!(
                "systemd-cat: failed to exec {}: {}",
                cli.command[0],
                std::io::Error::from_raw_os_error(err)
            );
            libc::_exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_priority_from_str() {
        assert!(matches!(
            Priority::from_str_or_num("emerg"),
            Ok(Priority::Emergency)
        ));
        assert!(matches!(
            Priority::from_str_or_num("0"),
            Ok(Priority::Emergency)
        ));
        assert!(matches!(
            Priority::from_str_or_num("err"),
            Ok(Priority::Error)
        ));
        assert!(matches!(
            Priority::from_str_or_num("info"),
            Ok(Priority::Info)
        ));
        assert!(matches!(
            Priority::from_str_or_num("debug"),
            Ok(Priority::Debug)
        ));
        assert!(Priority::from_str_or_num("unknown").is_err());
    }
}
