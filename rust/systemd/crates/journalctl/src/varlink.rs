//! `io.systemd.JournalAccess` Varlink server for journalctl.
//!
//! When journalctl is socket-activated via `systemd-journalctl.socket`
//! (`Accept=yes`, listening on `/run/systemd/io.systemd.JournalAccess`), the
//! accepted connection is passed as fd 3 and journalctl serves the
//! `io.systemd.JournalAccess` interface. The single method, `GetEntries`,
//! streams journal entries (filtered by unit / priority / limit) as flat JSON
//! objects, matching `journalctl --output=json`.
//!
//! Rather than duplicate the (large) journal reader, the server re-executes the
//! journalctl binary itself in normal mode (`--output=json -n <limit> …`) and
//! wraps each emitted line as a `GetEntries` reply. This reuses the exact same
//! seek/match/format code path a `journalctl -n … -u … -p …` invocation uses.

use std::io::{BufRead, BufReader, Write};
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};

const INTERFACE: &str = "io.systemd.JournalAccess";

/// The interface description returned by `GetInterfaceDescription`. Must contain
/// `method GetEntries(` (the test greps for it via `varlinkctl introspect`).
const JOURNAL_ACCESS_IDL: &str = "\
interface io.systemd.JournalAccess

method GetEntries(
\tunits: ?[]string,
\tuid: ?int,
\tuserUnits: ?[]string,
\tnamespace: ?string,
\tpriority: ?int,
\tlimit: ?int
) -> (
\tentry: object
)

error NoMatches()
error NoEntries()
";

/// Whether we were invoked as a Varlink service (a connected socket passed on
/// fd 3 via LISTEN_FDS, per the systemd socket-activation convention).
pub fn invoked_as_varlink() -> bool {
    let listen_pid: Option<i32> = std::env::var("LISTEN_PID").ok().and_then(|s| s.parse().ok());
    if listen_pid != Some(std::process::id() as i32) {
        return false;
    }
    matches!(
        std::env::var("LISTEN_FDS").ok().and_then(|s| s.parse::<i32>().ok()),
        Some(n) if n >= 1
    )
}

/// Serve one connection (Accept=yes passes one accepted connection per
/// invocation on fd 3), then exit the process.
pub fn serve() -> ! {
    let stream = unsafe { UnixStream::from_raw_fd(3) };
    let code = match handle(stream) {
        Ok(()) => 0,
        Err(_) => 1,
    };
    std::process::exit(code);
}

fn send(stream: &UnixStream, reply: &serde_json::Value) -> std::io::Result<()> {
    let mut w = stream;
    let mut msg = serde_json::to_vec(reply)?;
    msg.push(0); // NUL frame terminator
    w.write_all(&msg)
}

fn handle(stream: UnixStream) -> std::io::Result<()> {
    let write_stream = stream.try_clone()?;
    let mut reader = BufReader::new(stream);

    loop {
        let mut buf = Vec::new();
        let n = reader.read_until(0, &mut buf)?;
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
                    "product": "systemd (journalctl)",
                    "version": env!("CARGO_PKG_VERSION"),
                    "url": "https://systemd.io/",
                    "interfaces": ["org.varlink.service", INTERFACE],
                }});
                send(&write_stream, &reply)?;
            }
            "org.varlink.service.GetInterfaceDescription" => {
                let iface = req
                    .get("parameters")
                    .and_then(|p| p.get("interface"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let reply = if iface == INTERFACE {
                    serde_json::json!({"parameters": {"description": JOURNAL_ACCESS_IDL}})
                } else {
                    serde_json::json!({"error": "org.varlink.service.InterfaceNotFound",
                                       "parameters": {"interface": iface}})
                };
                send(&write_stream, &reply)?;
            }
            "io.systemd.JournalAccess.GetEntries" => {
                get_entries(&write_stream, &req)?;
                break; // one method call per connection
            }
            other => {
                let reply = serde_json::json!({"error": "org.varlink.service.MethodNotFound",
                                               "parameters": {"method": other}});
                send(&write_stream, &reply)?;
                break;
            }
        }
    }
    Ok(())
}

fn invalid_parameter(stream: &UnixStream, name: &str) -> std::io::Result<()> {
    send(
        stream,
        &serde_json::json!({"error": "org.varlink.service.InvalidParameter",
                            "parameters": {"parameter": name}}),
    )
}

/// Handle `io.systemd.JournalAccess.GetEntries`: validate parameters, run the
/// equivalent `journalctl --output=json …` query, and stream each entry.
fn get_entries(stream: &UnixStream, req: &serde_json::Value) -> std::io::Result<()> {
    let params = req.get("parameters");

    // limit: default 100, capped at 10000 (over that is an invalid parameter).
    let limit = params.and_then(|p| p.get("limit")).and_then(|v| v.as_u64());
    if let Some(l) = limit
        && l > 10000
    {
        return invalid_parameter(stream, "limit");
    }
    let n = match limit {
        Some(0) | None => 100,
        Some(l) => l,
    };

    // priority: a log level 0..=7 (over that is an invalid parameter).
    let priority = params.and_then(|p| p.get("priority")).and_then(|v| v.as_i64());
    if let Some(pri) = priority
        && !(0..=7).contains(&pri)
    {
        return invalid_parameter(stream, "priority");
    }

    let str_array = |key: &str| -> Vec<String> {
        params
            .and_then(|p| p.get(key))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default()
    };
    let units = str_array("units");
    let user_units = str_array("userUnits");
    let uid = params.and_then(|p| p.get("uid")).and_then(|v| v.as_u64());
    let namespace = params
        .and_then(|p| p.get("namespace"))
        .and_then(|v| v.as_str())
        .map(String::from);

    // Build the equivalent journalctl invocation and reuse the reader wholesale.
    let mut args: Vec<String> = vec![
        "--output=json".to_string(),
        "--no-pager".to_string(),
        format!("--lines={n}"),
    ];
    if let Some(pri) = priority {
        args.push(format!("--priority={pri}"));
    }
    for u in &units {
        args.push(format!("--unit={u}"));
    }
    for u in &user_units {
        args.push(format!("--user-unit={u}"));
    }
    if let Some(ns) = &namespace {
        args.push(format!("--namespace={ns}"));
    }
    if let Some(uid) = uid {
        args.push(format!("_UID={uid}"));
    }

    // The accepted connection is on fd 3 (non-CLOEXEC, from LISTEN_FDS). Mark it
    // CLOEXEC so the journalctl child we spawn to read the journal does not
    // inherit the Varlink socket (inheriting it wedges the connection). Our own
    // reply path uses a separate CLOEXEC clone, so fd 3 stays usable here.
    unsafe {
        libc::fcntl(3, libc::F_SETFD, libc::FD_CLOEXEC);
    }

    let exe = std::env::current_exe().unwrap_or_else(|_| "journalctl".into());
    let output = Command::new(exe)
        .args(&args)
        // Run as an ordinary journalctl (not a nested varlink server).
        .env_remove("LISTEN_FDS")
        .env_remove("LISTEN_PID")
        .env_remove("LISTEN_FDNAMES")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()?;

    // Collect the compact-JSON entries (one per line).
    let entries: Vec<serde_json::Value> = output
        .stdout
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_slice::<serde_json::Value>(l).ok())
        .collect();

    if entries.is_empty() {
        return send(
            stream,
            &serde_json::json!({"error": "io.systemd.JournalAccess.NoEntries", "parameters": {}}),
        );
    }

    // Stream each entry; the last reply carries continues=false to close the
    // `more` stream.
    let last = entries.len() - 1;
    for (i, entry) in entries.into_iter().enumerate() {
        let reply = serde_json::json!({
            "parameters": {"entry": entry},
            "continues": i != last,
        });
        send(stream, &reply)?;
    }
    Ok(())
}
