//! systemd-cat — Connect a pipeline or program's output to the journal.
//!
//! A drop-in replacement for `systemd-cat(1)`. This tool connects
//! stdout and stderr of a command (or stdin if no command is given)
//! to the systemd journal using the journal stdout stream protocol.
//!
//! Supported options:
//!
//! - `-t`, `--identifier=ID`  — Set the syslog identifier (default: "unknown")
//! - `-p`, `--priority=PRIO`  — Set the default log priority for stdout
//! - `--stderr-priority=PRIO` — Set the log priority for stderr
//! - `--level-prefix=BOOL`    — Strip kernel-style `<N>` priority prefixes from lines

use clap::Parser;
use std::io::{BufRead, BufReader, Read, Write};
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
        let cmd = &cli.command[0];
        let args = &cli.command[1..];
        let identifier = cli.identifier.as_deref().unwrap_or(cmd.as_str());

        let mut child = match process::Command::new(cmd)
            .args(args)
            .stdout(process::Stdio::piped())
            .stderr(process::Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                eprintln!("systemd-cat: failed to execute {cmd}: {e}");
                process::exit(1);
            }
        };

        let child_stdout = child.stdout.take();
        let child_stderr = child.stderr.take();
        let identifier_owned = identifier.to_string();
        let journal_socket_clone = journal_socket.clone();

        let stderr_handle = if let Some(stderr) = child_stderr {
            let id = identifier_owned.clone();
            Some(std::thread::spawn(move || {
                match open_journal_stream(&journal_socket_clone, &id, stderr_priority, level_prefix)
                {
                    Ok(mut stream) => {
                        process_lines(stderr, &mut stream, &id, stderr_priority, level_prefix);
                    }
                    Err(e) => {
                        eprintln!("systemd-cat: failed to open stderr stream: {e}");
                    }
                }
            }))
        } else {
            None
        };

        if let Some(stdout) = child_stdout {
            match open_journal_stream(
                &journal_socket,
                &identifier_owned,
                stdout_priority,
                level_prefix,
            ) {
                Ok(mut stream) => {
                    process_lines(
                        stdout,
                        &mut stream,
                        &identifier_owned,
                        stdout_priority,
                        level_prefix,
                    );
                }
                Err(e) => {
                    eprintln!("systemd-cat: failed to open stdout stream: {e}");
                }
            }
        }

        if let Some(handle) = stderr_handle {
            let _ = handle.join();
        }

        match child.wait() {
            Ok(status) => {
                process::exit(status.code().unwrap_or(1));
            }
            Err(e) => {
                eprintln!("systemd-cat: failed to wait for {cmd}: {e}");
                process::exit(1);
            }
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
