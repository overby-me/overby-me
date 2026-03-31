//! Varlink server for the `io.systemd.Manager` interface.
//!
//! Listens on `/run/systemd/io.systemd.Manager` and handles the
//! `io.systemd.Manager.Describe` method, matching upstream systemd's
//! varlink interface closely enough for `varlinkctl` to work.
//!
//! The varlink wire protocol is JSON messages delimited by NUL bytes ('\0')
//! over a Unix stream socket.

use crate::runtime_info::ArcMutRuntimeInfo;
use log::{trace, warn};
use std::io::{Read, Write};
use std::os::unix::net::{UnixDatagram, UnixListener};
use std::sync::atomic::{AtomicU64, Ordering};

/// Global transaction ID counter for ordering cycles.
static NEXT_TRANSACTION_ID: AtomicU64 = AtomicU64::new(1);

/// Allocate a new unique transaction ID.
pub fn next_transaction_id() -> u64 {
    NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
}

const VARLINK_SOCKET_PATH: &str = "/run/systemd/io.systemd.Manager";

/// Start the varlink server on `/run/systemd/io.systemd.Manager`.
pub fn start_varlink_server(run_info: ArcMutRuntimeInfo) {
    let sock_path = std::path::Path::new(VARLINK_SOCKET_PATH);

    // Remove stale socket if it exists
    if sock_path.exists() {
        let _ = std::fs::remove_file(sock_path);
    }

    // Ensure parent directory exists
    if let Some(parent) = sock_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let listener = match UnixListener::bind(sock_path) {
        Ok(l) => l,
        Err(e) => {
            warn!("Failed to bind varlink socket at {VARLINK_SOCKET_PATH}: {e}");
            return;
        }
    };

    // Make socket world-accessible (like systemd does)
    let _ = std::fs::set_permissions(
        sock_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o666),
    );

    std::thread::spawn(move || {
        loop {
            match listener.accept() {
                Ok((stream, _addr)) => {
                    let ri = run_info.clone();
                    std::thread::spawn(move || {
                        handle_varlink_connection(stream, ri);
                    });
                }
                Err(e) => {
                    warn!("Varlink accept error: {e}");
                }
            }
        }
    });
}

/// Handle a single varlink connection.
/// Reads NUL-delimited JSON messages and dispatches methods.
fn handle_varlink_connection(
    mut stream: std::os::unix::net::UnixStream,
    run_info: ArcMutRuntimeInfo,
) {
    let mut buf = Vec::new();
    let mut read_buf = [0u8; 4096];

    loop {
        match stream.read(&mut read_buf) {
            Ok(0) => return, // EOF
            Ok(n) => {
                buf.extend_from_slice(&read_buf[..n]);
            }
            Err(e) => {
                trace!("Varlink read error: {e}");
                return;
            }
        }

        // Process all complete messages (NUL-delimited)
        while let Some(nul_pos) = buf.iter().position(|&b| b == 0) {
            let msg_bytes = buf[..nul_pos].to_vec();
            buf.drain(..=nul_pos);

            let response = match process_varlink_message(&msg_bytes, &run_info) {
                Ok(resp) => resp,
                Err(err_resp) => err_resp,
            };

            // Send response: JSON + NUL
            let mut resp_bytes = serde_json::to_vec(&response).unwrap_or_default();
            resp_bytes.push(0);
            if stream.write_all(&resp_bytes).is_err() {
                return;
            }
        }
    }
}

/// Parse and dispatch a single varlink message.
fn process_varlink_message(
    msg_bytes: &[u8],
    run_info: &ArcMutRuntimeInfo,
) -> Result<serde_json::Value, serde_json::Value> {
    let msg: serde_json::Value = serde_json::from_slice(msg_bytes).map_err(|e| {
        serde_json::json!({
            "error": "org.varlink.service.InvalidParameter",
            "parameters": {"parameter": format!("JSON parse error: {e}")}
        })
    })?;

    let method = msg.get("method").and_then(|v| v.as_str()).ok_or_else(|| {
        serde_json::json!({
            "error": "org.varlink.service.InvalidParameter",
            "parameters": {"parameter": "missing 'method' field"}
        })
    })?;

    match method {
        "io.systemd.Manager.Describe" => {
            let response = build_describe_response(run_info);
            Ok(response)
        }
        "org.varlink.service.GetInfo" => Ok(serde_json::json!({
            "parameters": {
                "vendor": "rust-systemd",
                "product": "rust-systemd",
                "version": "0.1.0",
                "url": "",
                "interfaces": ["io.systemd.Manager"]
            }
        })),
        _ => Err(serde_json::json!({
            "error": "org.varlink.service.MethodNotFound",
            "parameters": {"method": method}
        })),
    }
}

/// Send a structured log message to the journal with custom fields.
/// Uses the native journal protocol (datagram to /run/systemd/journal/socket).
/// `priority` follows syslog levels: 4=warning, 6=info, etc.
pub fn journal_log_with_fields(message: &str, priority: u8, fields: &[(&str, &str)]) {
    let sock = match UnixDatagram::unbound() {
        Ok(s) => s,
        Err(_) => return,
    };

    let mut payload = String::new();
    payload.push_str(&format!("MESSAGE={message}\n"));
    payload.push_str(&format!("PRIORITY={priority}\n"));
    payload.push_str("SYSLOG_IDENTIFIER=systemd\n");
    for (key, value) in fields {
        payload.push_str(&format!("{key}={value}\n"));
    }

    let _ = sock.send_to(payload.as_bytes(), "/run/systemd/journal/socket");
}

/// Convert a syslog level name to its numeric value.
fn parse_log_level(level: &str) -> Option<u8> {
    match level {
        "emerg" | "0" => Some(0),
        "alert" | "1" => Some(1),
        "crit" | "2" => Some(2),
        "err" | "error" | "3" => Some(3),
        "warning" | "warn" | "4" => Some(4),
        "notice" | "5" => Some(5),
        "info" | "6" => Some(6),
        "debug" | "7" => Some(7),
        _ => None,
    }
}

/// Log a unit lifecycle event to the journal (e.g. "Starting ...", "Started ...",
/// "Deactivated successfully."). These correspond to the structured messages
/// that C systemd's PID 1 sends with UNIT= and SYSLOG_IDENTIFIER=systemd.
///
/// Respects `LogLevelMax=`: if the unit has a max log level set, lifecycle
/// messages (priority 6/info) are suppressed when the max is below info.
pub fn journal_log_unit_lifecycle(message: &str, unit_name: &str, log_level_max: Option<&str>) {
    const LIFECYCLE_PRIORITY: u8 = 6; // LOG_INFO
    if let Some(max_str) = log_level_max
        && let Some(max_level) = parse_log_level(max_str)
        && LIFECYCLE_PRIORITY > max_level
    {
        return;
    }
    journal_log_with_fields(message, LIFECYCLE_PRIORITY, &[("UNIT", unit_name)]);
}

/// Build the response for `io.systemd.Manager.Describe`.
fn build_describe_response(run_info: &ArcMutRuntimeInfo) -> serde_json::Value {
    let ri = run_info.read().unwrap();

    // Collect transaction IDs for ordering cycles
    let cycle_txns: Vec<u64> = ri.transactions_with_cycle.lock().unwrap().clone();
    let cycle_txns_json: Vec<serde_json::Value> = cycle_txns
        .into_iter()
        .map(serde_json::Value::from)
        .collect();

    let n_names = ri.unit_table.len();
    let n_failed = ri
        .unit_table
        .values()
        .filter(|u| {
            let status = u.common.status.read().unwrap();
            matches!(&*status, crate::units::UnitStatus::Stopped(_, errs) if !errs.is_empty())
        })
        .count();

    serde_json::json!({
        "parameters": {
            "context": {
                "ShowStatus": false,
                "LogLevel": {
                    "console": "info",
                    "kmsg": "info",
                    "syslog": "info",
                    "journal": "info"
                },
                "LogTarget": "journal-or-kmsg",
                "ServiceWatchdogs": true
            },
            "runtime": {
                "Version": "rust-systemd 0.1.0",
                "Architecture": std::env::consts::ARCH,
                "Features": "",
                "Virtualization": "",
                "ConfidentialVirtualization": "",
                "NNames": n_names,
                "NFailedUnits": n_failed,
                "NJobs": 0,
                "NInstalledJobs": 0,
                "NFailedJobs": 0,
                "TransactionsWithOrderingCycle": if cycle_txns_json.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::Array(cycle_txns_json)
                },
                "Progress": 1.0,
                "SystemState": "running",
                "ExitCode": 0,
                "SoftRebootsCount": 0
            }
        }
    })
}
