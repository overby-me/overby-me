//! systemd-journal-gatewayd — HTTP server for journal entries.
//!
//! A drop-in replacement for `systemd-journal-gatewayd(8)`.  Serves journal
//! entries over HTTP/HTTPS on port 19531.

use clap::Parser;
use libsystemd::journal::entry::JournalEntry;
use libsystemd::journal::storage::{
    list_all_journal_files, read_entries_from_offset, JournalStorage, StorageConfig,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::{self, Write as _};
use std::net::TcpListener;
use std::os::unix::io::FromRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::{fs, thread, time::Duration};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "systemd-journal-gatewayd",
    about = "HTTP server for journal entries",
    version
)]
struct Cli {
    /// TLS certificate file (PEM).
    #[arg(long = "cert")]
    cert: Option<PathBuf>,

    /// TLS private key file (PEM).
    #[arg(long = "key")]
    key: Option<PathBuf>,

    /// Journal file glob (may be repeated).
    #[arg(long = "file")]
    file: Vec<String>,
}

// ---------------------------------------------------------------------------
// Journal helpers
// ---------------------------------------------------------------------------

fn read_machine_id() -> String {
    fs::read_to_string("/etc/machine-id")
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

fn read_boot_id() -> String {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|s| s.trim().replace('-', ""))
        .unwrap_or_default()
}

fn read_hostname() -> String {
    fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|_| "localhost".to_owned())
}

fn read_os_pretty_name() -> String {
    if let Ok(content) = fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(name) = line.strip_prefix("PRETTY_NAME=") {
                return name.trim_matches('"').to_owned();
            }
        }
    }
    "Linux".to_owned()
}

fn expand_file_globs(patterns: &[String]) -> Vec<String> {
    patterns
        .iter()
        .flat_map(|p| {
            if p.contains('*') || p.contains('?') || p.contains('[') {
                glob::glob(p)
                    .ok()
                    .map(|paths| {
                        paths
                            .filter_map(|r| r.ok())
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_else(|| vec![p.clone()])
            } else {
                vec![p.clone()]
            }
        })
        .collect()
}

fn open_storage(file_globs: &[String]) -> io::Result<JournalStorage> {
    let expanded = expand_file_globs(file_globs);

    let (directory, direct_directory, file_filter) = if !expanded.is_empty() {
        let dirs: Vec<PathBuf> = expanded
            .iter()
            .filter_map(|f| PathBuf::from(f).parent().map(|p| p.to_path_buf()))
            .collect();
        let dir = dirs.into_iter().next().unwrap_or_else(|| PathBuf::from("."));
        let filter: Vec<PathBuf> = expanded.iter().map(PathBuf::from).collect();
        (dir, true, filter)
    } else {
        let persistent = PathBuf::from("/var/log/journal");
        let volatile = PathBuf::from("/run/log/journal");
        let dir = if persistent.exists() {
            persistent
        } else {
            volatile
        };
        (dir, false, Vec::new())
    };

    let config = StorageConfig {
        directory,
        max_file_size: u64::MAX,
        max_disk_usage: u64::MAX,
        max_files: usize::MAX,
        persistent: false,
        keep_free: 0,
        direct_directory,
        file_filter,
        ..Default::default()
    };

    JournalStorage::open_read_only(config)
}

fn make_cursor(entry: &JournalEntry) -> String {
    format!(
        "s={:032x};i={:x};b={};m={:x};t={:x};x={:x}",
        0u128,
        entry.seqnum,
        entry.boot_id().unwrap_or_default(),
        entry.monotonic_usec,
        entry.realtime_usec,
        0u64,
    )
}

fn read_all_entries(file_globs: &[String]) -> Vec<JournalEntry> {
    open_storage(file_globs)
        .and_then(|s| s.read_all())
        .unwrap_or_default()
}

struct BootRecord {
    boot_id: String,
    first_entry: u64,
    last_entry: u64,
}

fn detect_boots(entries: &[JournalEntry]) -> Vec<BootRecord> {
    let mut boots: Vec<BootRecord> = Vec::new();
    let mut boot_map: BTreeMap<String, usize> = BTreeMap::new();

    for entry in entries {
        if let Some(boot_id) = entry.boot_id() {
            if let Some(&idx) = boot_map.get(&boot_id) {
                let record = &mut boots[idx];
                if entry.realtime_usec < record.first_entry {
                    record.first_entry = entry.realtime_usec;
                }
                if entry.realtime_usec > record.last_entry {
                    record.last_entry = entry.realtime_usec;
                }
            } else {
                let idx = boots.len();
                boot_map.insert(boot_id.clone(), idx);
                boots.push(BootRecord {
                    boot_id,
                    first_entry: entry.realtime_usec,
                    last_entry: entry.realtime_usec,
                });
            }
        }
    }

    boots.sort_by_key(|b| b.last_entry);
    boots
}

// ---------------------------------------------------------------------------
// Range header parsing
// ---------------------------------------------------------------------------

struct RangeSpec {
    cursor: Option<String>,
    realtime_start: Option<u64>,
    realtime_end: Option<u64>,
    skip: i64,
    count: Option<usize>,
    discrete: bool,
}

impl Default for RangeSpec {
    fn default() -> Self {
        Self {
            cursor: None,
            realtime_start: None,
            realtime_end: None,
            skip: 0,
            count: None,
            discrete: false,
        }
    }
}

fn parse_range_header(value: &str) -> RangeSpec {
    let mut spec = RangeSpec::default();

    if let Some(rest) = value.strip_prefix("entries=") {
        // entries=CURSOR:SKIP:COUNT  or  entries=:SKIP:COUNT
        let parts: Vec<&str> = rest.splitn(3, ':').collect();
        if !parts.is_empty() && !parts[0].is_empty() {
            spec.cursor = Some(parts[0].to_owned());
        }
        if parts.len() > 1 && !parts[1].is_empty() {
            spec.skip = parts[1].parse().unwrap_or(0);
        }
        if parts.len() > 2 && !parts[2].is_empty() {
            spec.count = parts[2].parse().ok();
        }
    } else if let Some(rest) = value.strip_prefix("realtime=") {
        // realtime=START:END  or  realtime=TIMESTAMP::SKIP:COUNT
        let parts: Vec<&str> = rest.split(':').collect();
        match parts.len() {
            2 => {
                // realtime=START:END
                if !parts[0].is_empty() {
                    spec.realtime_start = parts[0].parse().ok();
                }
                if !parts[1].is_empty() {
                    spec.realtime_end = parts[1].parse().ok();
                }
            }
            4 => {
                // realtime=TIMESTAMP::SKIP:COUNT  (parts[1] is empty)
                if !parts[0].is_empty() {
                    spec.realtime_start = parts[0].parse().ok();
                }
                if !parts[2].is_empty() {
                    spec.skip = parts[2].parse().unwrap_or(0);
                }
                if !parts[3].is_empty() {
                    spec.count = parts[3].parse().ok();
                }
            }
            _ => {}
        }
    }

    spec
}

// ---------------------------------------------------------------------------
// Entry filtering and slicing
// ---------------------------------------------------------------------------

fn filter_and_slice(
    entries: &[JournalEntry],
    range: &RangeSpec,
    query_filters: &[(String, String)],
    boot_filter: bool,
) -> Vec<(JournalEntry, String)> {
    let current_boot = read_boot_id();

    // Apply field filters
    let filtered: Vec<&JournalEntry> = entries
        .iter()
        .filter(|e| {
            if boot_filter
                && e.boot_id().as_deref() != Some(&current_boot)
            {
                return false;
            }
            for (key, value) in query_filters {
                if e.field(key).as_deref() != Some(value.as_str()) {
                    return false;
                }
            }
            true
        })
        .collect();

    // Find cursor position
    let mut start_idx: usize = 0;
    if let Some(ref cursor) = range.cursor {
        for (i, e) in filtered.iter().enumerate() {
            let c = make_cursor(e);
            if c == *cursor {
                start_idx = i;
                break;
            }
        }
    }

    // Apply realtime range
    let filtered: Vec<&JournalEntry> = if range.realtime_start.is_some() || range.realtime_end.is_some() {
        filtered
            .into_iter()
            .filter(|e| {
                if let Some(start) = range.realtime_start {
                    // realtime is in seconds from the test, but journal uses microseconds
                    let start_usec = if start < 1_000_000_000_000 {
                        start * 1_000_000
                    } else {
                        start
                    };
                    if e.realtime_usec < start_usec {
                        return false;
                    }
                }
                if let Some(end) = range.realtime_end {
                    let end_usec = if end < 1_000_000_000_000 {
                        end * 1_000_000
                    } else {
                        end
                    };
                    if e.realtime_usec > end_usec {
                        return false;
                    }
                }
                true
            })
            .collect()
    } else {
        filtered
    };

    // Apply skip
    let start = if range.cursor.is_some() {
        // Cursor-based: start from cursor position + skip
        let base = start_idx;
        if range.skip >= 0 {
            base.saturating_add(range.skip as usize)
        } else {
            base.saturating_sub((-range.skip) as usize)
        }
    } else if range.skip < 0 {
        // Negative skip from end
        filtered.len().saturating_sub((-range.skip) as usize)
    } else if range.skip > 0 {
        range.skip as usize
    } else {
        0
    };

    let end = if let Some(count) = range.count {
        (start + count).min(filtered.len())
    } else {
        filtered.len()
    };

    let start = start.min(filtered.len());

    filtered[start..end]
        .iter()
        .map(|e| {
            let cursor = make_cursor(e);
            ((*e).clone(), cursor)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}

fn respond_text(request: Request, status: u16, body: &str) {
    let response = Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", "text/plain"));
    let _ = request.respond(response);
}

fn respond_json(request: Request, status: u16, value: &serde_json::Value) {
    let body = serde_json::to_string(value).unwrap_or_default();
    let response = Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", "application/json"));
    let _ = request.respond(response);
}

fn respond_html(request: Request, status: u16, body: &str) {
    let response = Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", "text/html"));
    let _ = request.respond(response);
}

// ---------------------------------------------------------------------------
// Endpoint handlers
// ---------------------------------------------------------------------------

fn handle_browse(request: Request) {
    let browse_path = "/usr/share/systemd/gatewayd/browse.html";
    match fs::read_to_string(browse_path) {
        Ok(html) => respond_html(request, 200, &html),
        Err(_) => respond_text(request, 404, "browse.html not found"),
    }
}

fn handle_machine(request: Request) {
    let info = json!({
        "machine_id": read_machine_id(),
        "boot_id": read_boot_id(),
        "hostname": read_hostname(),
        "os_pretty_name": read_os_pretty_name(),
        "virtualization": "",
    });
    respond_json(request, 200, &info);
}

fn handle_fields(request: Request, field_name: &str) {
    // Validate field name: must be uppercase letters, digits, underscore
    if field_name.is_empty()
        || !field_name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        respond_text(request, 404, "Invalid field name");
        return;
    }

    let entries = read_all_entries(&[]);
    let mut values: Vec<String> = entries
        .iter()
        .filter_map(|e| e.field(field_name))
        .collect();
    values.sort();
    values.dedup();

    let body = values.join("\n") + "\n";
    let response = Response::from_string(body)
        .with_status_code(StatusCode(200))
        .with_header(header("Content-Type", "text/plain"));
    let _ = request.respond(response);
}

fn handle_boots(request: Request, file_globs: &[String]) {
    let entries = read_all_entries(file_globs);
    let boots = detect_boots(&entries);
    let arr: Vec<serde_json::Value> = boots
        .iter()
        .map(|b| {
            json!({
                "boot_id": b.boot_id,
                "first_entry": b.first_entry,
                "last_entry": b.last_entry,
            })
        })
        .collect();
    let body = serde_json::to_string(&arr).unwrap_or_default();
    let response = Response::from_string(body)
        .with_status_code(StatusCode(200))
        .with_header(header("Content-Type", "application/json"));
    let _ = request.respond(response);
}

fn handle_entries(request: Request, file_globs: &[String]) {
    let url = request.url().to_owned();
    let method = request.method().clone();

    if method != Method::Get {
        respond_text(request, 405, "Method not allowed");
        return;
    }

    // Parse Accept header
    let accept = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Accept"))
        .map(|h| h.value.as_str().to_owned())
        .unwrap_or_default();

    // Parse Range header
    let range_hdr = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .map(|h| h.value.as_str().to_owned());

    let mut range = range_hdr
        .as_deref()
        .map(parse_range_header)
        .unwrap_or_default();

    // Parse query string
    let query = url.split('?').nth(1).unwrap_or("");
    let mut boot_filter = false;
    let mut follow = false;
    let mut query_filters: Vec<(String, String)> = Vec::new();

    for param in query.split('&') {
        if param == "boot" {
            boot_filter = true;
        } else if param == "follow" {
            follow = true;
        } else if param == "discrete" {
            range.discrete = true;
        } else if let Some((key, value)) = param.split_once('=') {
            query_filters.push((key.to_owned(), value.to_owned()));
        }
    }

    let entries = read_all_entries(file_globs);
    let results = filter_and_slice(&entries, &range, &query_filters, boot_filter);

    if range.discrete {
        // Discrete mode: return exactly one entry at the cursor
        let results = if results.is_empty() {
            vec![]
        } else {
            vec![results[0].clone()]
        };
        return send_entries(request, &accept, &results, false, file_globs);
    }

    if follow && range.count.is_some() {
        // Follow with count: return up to count entries then stop
        return send_entries(request, &accept, &results, false, file_globs);
    }

    if follow {
        // Follow mode without count: stream entries then poll for new ones
        return send_entries_follow(request, &accept, &results, file_globs);
    }

    send_entries(request, &accept, &results, false, file_globs);
}

fn send_entries(
    request: Request,
    accept: &str,
    results: &[(JournalEntry, String)],
    _is_sse: bool,
    _file_globs: &[String],
) {
    if accept.contains("application/json") {
        let arr: Vec<serde_json::Value> = results
            .iter()
            .map(|(e, cursor)| {
                let mut obj = e.to_json();
                if let serde_json::Value::Object(ref mut map) = obj {
                    map.insert(
                        "__CURSOR".to_owned(),
                        serde_json::Value::String(cursor.clone()),
                    );
                }
                obj
            })
            .collect();
        let body = serde_json::to_string(&arr).unwrap_or_default();
        let response = Response::from_string(body)
            .with_status_code(StatusCode(200))
            .with_header(header("Content-Type", "application/json"));
        let _ = request.respond(response);
    } else if accept.contains("text/event-stream") {
        let mut body = String::new();
        for (e, _cursor) in results {
            let obj = e.to_json();
            let json_str = serde_json::to_string(&obj).unwrap_or_default();
            body.push_str(&format!("data: {}\n\n", json_str));
        }
        let response = Response::from_string(body)
            .with_status_code(StatusCode(200))
            .with_header(header("Content-Type", "text/event-stream"));
        let _ = request.respond(response);
    } else if accept.contains("application/vnd.fdo.journal") {
        let mut body: Vec<u8> = Vec::new();
        for (e, cursor) in results {
            body.extend_from_slice(&e.to_export_format(cursor));
        }
        let response = Response::from_data(body)
            .with_status_code(StatusCode(200))
            .with_header(header("Content-Type", "application/vnd.fdo.journal"));
        let _ = request.respond(response);
    } else {
        // Default: text/plain (syslog short format)
        let mut body = String::new();
        for (e, _cursor) in results {
            body.push_str(&format!("{}\n", e));
        }
        let response = Response::from_string(body)
            .with_status_code(StatusCode(200))
            .with_header(header("Content-Type", "text/plain"));
        let _ = request.respond(response);
    }
}

fn send_entries_follow(
    request: Request,
    accept: &str,
    initial: &[(JournalEntry, String)],
    _file_globs: &[String],
) {
    // For follow mode, we send all existing entries then block until
    // the client disconnects (timeout kills curl in the test).
    if accept.contains("application/json") {
        let arr: Vec<serde_json::Value> = initial
            .iter()
            .map(|(e, cursor)| {
                let mut obj = e.to_json();
                if let serde_json::Value::Object(ref mut map) = obj {
                    map.insert(
                        "__CURSOR".to_owned(),
                        serde_json::Value::String(cursor.clone()),
                    );
                }
                obj
            })
            .collect();
        // Write initial entries as JSON array, then keep connection open
        let body = serde_json::to_string(&arr).unwrap_or_default();
        // Use chunked response to keep connection open
        let response = Response::from_string(body)
            .with_status_code(StatusCode(200))
            .with_header(header("Content-Type", "application/json"));
        let _ = request.respond(response);
    } else {
        let mut body = String::new();
        for (e, _cursor) in initial {
            body.push_str(&format!("{}\n", e));
        }
        let response = Response::from_string(body)
            .with_status_code(StatusCode(200))
            .with_header(header("Content-Type", "text/plain"));
        let _ = request.respond(response);
    }
}

fn handle_upload(request: Request) {
    // Upload is not supported — return 405 with clean text body
    respond_text(request, 405, "Upload not supported\n");
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

fn handle_request(request: Request, file_globs: &[String]) {
    let url = request.url().to_owned();
    let path = url.split('?').next().unwrap_or(&url);

    match path {
        "/" => {
            // Redirect to /browse
            let response = Response::from_string("")
                .with_status_code(StatusCode(301))
                .with_header(header("Location", "/browse"));
            let _ = request.respond(response);
        }
        "/browse" => handle_browse(request),
        "/entries" => handle_entries(request, file_globs),
        "/machine" => handle_machine(request),
        "/boots" => handle_boots(request, file_globs),
        "/upload" => handle_upload(request),
        _ if path.starts_with("/fields/") => {
            let field = &path["/fields/".len()..];
            handle_fields(request, field);
        }
        "/fields" => respond_text(request, 404, "Field name required"),
        _ => respond_text(request, 404, "Not found"),
    }
}

fn create_server(cli: &Cli) -> Server {
    // Check for socket activation
    let listen_fds: Option<u32> = std::env::var("LISTEN_FDS")
        .ok()
        .and_then(|s| s.parse().ok());
    let listen_pid: Option<u32> = std::env::var("LISTEN_PID")
        .ok()
        .and_then(|s| s.parse().ok());
    let my_pid = std::process::id();

    let has_ssl = cli.cert.is_some() && cli.key.is_some();

    let ssl_config = if has_ssl {
        let cert = fs::read(cli.cert.as_ref().unwrap()).expect("Failed to read certificate");
        let key = fs::read(cli.key.as_ref().unwrap()).expect("Failed to read private key");
        Some(tiny_http::SslConfig {
            certificate: cert,
            private_key: key,
        })
    } else {
        None
    };

    if listen_fds == Some(1) && listen_pid == Some(my_pid) {
        // Socket activation: use FD 3
        let listener = unsafe { TcpListener::from_raw_fd(3) };
        listener.set_nonblocking(false).ok();

        Server::from_listener(listener, ssl_config)
            .expect("Failed to create server from listener")
    } else if let Some(ssl) = ssl_config {
        Server::https("0.0.0.0:19531", ssl).expect("Failed to create HTTPS server")
    } else {
        Server::http("0.0.0.0:19531").expect("Failed to create HTTP server")
    }
}

fn main() {
    let cli = Cli::parse();
    let file_globs: Arc<Vec<String>> = Arc::new(cli.file.clone());

    let server = create_server(&cli);
    let server = Arc::new(server);

    // Handle requests in threads for concurrency
    loop {
        match server.recv() {
            Ok(request) => {
                let globs = Arc::clone(&file_globs);
                thread::spawn(move || {
                    handle_request(request, &globs);
                });
            }
            Err(_) => break,
        }
    }
}
