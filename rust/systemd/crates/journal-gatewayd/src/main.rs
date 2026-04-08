//! systemd-journal-gatewayd — HTTP server for journal entries.
//!
//! A drop-in replacement for `systemd-journal-gatewayd(8)`.  Serves journal
//! entries over HTTP/HTTPS on port 19531.

use clap::Parser;
use libsystemd::journal::entry::JournalEntry;
use libsystemd::journal::storage::{JournalStorage, StorageConfig};
use serde_json::json;
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::os::unix::io::FromRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::{fs, thread};
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

    /// Listen address (host:port or just port).
    #[arg(long = "listen", default_value = "19531")]
    listen: String,
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

fn expand_file_globs(patterns: &[String]) -> Vec<PathBuf> {
    patterns
        .iter()
        .flat_map(|p| {
            if p.contains('*') || p.contains('?') || p.contains('[') {
                glob::glob(p)
                    .ok()
                    .map(|paths| paths.filter_map(|r| r.ok()).collect::<Vec<_>>())
                    .unwrap_or_else(|| vec![PathBuf::from(p)])
            } else {
                vec![PathBuf::from(p)]
            }
        })
        .collect()
}

fn open_storage(file_globs: &[String]) -> std::io::Result<JournalStorage> {
    let expanded = expand_file_globs(file_globs);

    let (directory, direct_directory, file_filter) = if !expanded.is_empty() {
        let dir = expanded
            .iter()
            .filter_map(|f| f.parent().map(|p| p.to_path_buf()))
            .next()
            .unwrap_or_else(|| PathBuf::from("."));
        (dir, true, expanded)
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
        "s=0;i={:x};b={};m={:x};t={:x};x=0",
        entry.seqnum,
        entry.boot_id().unwrap_or_default(),
        entry.monotonic_usec,
        entry.realtime_usec,
    )
}

fn read_all_entries(file_globs: &[String]) -> Vec<JournalEntry> {
    let mut entries = open_storage(file_globs)
        .and_then(|s| s.read_all())
        .unwrap_or_default();
    entries.sort_by_key(|e| (e.realtime_usec, e.seqnum));
    entries
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

    boots.sort_by_key(|b| b.first_entry);
    boots
}

// ---------------------------------------------------------------------------
// Cursor parsing
// ---------------------------------------------------------------------------

fn parse_cursor_seqnum(cursor: &str) -> Option<u64> {
    for part in cursor.split(';') {
        if let Some(val) = part.strip_prefix("i=") {
            return u64::from_str_radix(val, 16).ok();
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Range header parsing
// ---------------------------------------------------------------------------

#[derive(Default)]
struct RangeSpec {
    cursor: Option<String>,
    since: Option<u64>,
    until: Option<u64>,
    skip: i64,
    count: Option<usize>,
    discrete: bool,
}

/// Convert a timestamp to microseconds.  Values that look like seconds
/// (< 10^12) are multiplied by 1_000_000, matching C systemd's parse_sec().
fn to_usec(val: u64) -> u64 {
    if val < 1_000_000_000_000 {
        val * 1_000_000
    } else {
        val
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
        let parts: Vec<&str> = rest.split(':').collect();
        match parts.len() {
            // realtime=START:END
            2 => {
                if !parts[0].is_empty() {
                    spec.since = parts[0].parse::<u64>().ok().map(to_usec);
                }
                if !parts[1].is_empty() {
                    spec.until = parts[1].parse::<u64>().ok().map(to_usec);
                }
            }
            // realtime=TIMESTAMP::SKIP:COUNT  (parts[1] is empty)
            4 => {
                if !parts[0].is_empty() {
                    spec.since = parts[0].parse::<u64>().ok().map(to_usec);
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

    // Apply field and boot filters (matches sd_journal_add_match() in C)
    let filtered: Vec<&JournalEntry> = entries
        .iter()
        .filter(|e| {
            if boot_filter && e.boot_id().as_deref() != Some(&current_boot) {
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

    // C gatewayd uses since as a SEEK position (not a range filter), then
    // applies skip from there.  until is checked during forward iteration.
    //
    // Seek: find the starting index in the filtered array.
    // Priority: cursor > since > head/tail (depending on skip sign).
    let seek_idx = if let Some(cursor) = &range.cursor {
        let target_seqnum = parse_cursor_seqnum(cursor);
        target_seqnum.and_then(|seq| filtered.iter().position(|e| e.seqnum == seq))
    } else if let Some(since) = range.since {
        if range.skip >= 0 {
            // Forward: first entry with realtime >= since
            let pos = filtered.iter().position(|e| e.realtime_usec >= since);
            // If no exact match, start from begin (like sd_journal_seek_realtime)
            Some(pos.unwrap_or(filtered.len()))
        } else {
            // Backward: seek to since position, previous_skip goes backwards
            let pos = filtered.iter().position(|e| e.realtime_usec >= since);
            Some(pos.unwrap_or(filtered.len()))
        }
    } else if range.skip >= 0 {
        Some(0) // seek_head
    } else if let Some(until) = range.until {
        // Seek to until for negative skip
        let pos = filtered.iter().rposition(|e| e.realtime_usec <= until);
        Some(pos.map(|p| p + 1).unwrap_or(filtered.len()))
    } else {
        Some(filtered.len()) // seek_tail
    };

    let seek_idx = seek_idx.unwrap_or(0);

    // Apply skip from seek position.
    // C gatewayd: next_skip(n_skip+1) for positive, previous_skip(abs(n_skip)+1)
    // for negative.  In our flat array, seek_idx points AT the seek target.
    //   skip=0  → next_skip(1) → start at seek_idx
    //   skip>0  → next_skip(skip+1) → start at seek_idx + skip
    //   skip<0  → previous_skip(abs(skip)+1) → start at seek_idx - abs(skip) - 1
    let start = if range.skip >= 0 {
        seek_idx.saturating_add(range.skip as usize)
    } else {
        seek_idx.saturating_sub((-range.skip) as usize + 1)
    };

    let start = start.min(filtered.len());

    // Apply count limit and until bound (checked during forward iteration)
    let end = filtered.len();
    let mut result = Vec::new();
    for &e in &filtered[start..end] {
        if let Some(until) = range.until
            && e.realtime_usec > until
        {
            break;
        }
        if let Some(count) = range.count
            && result.len() >= count
        {
            break;
        }
        let cursor = make_cursor(e);
        result.push((e.clone(), cursor));
    }
    result
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

fn respond_json_value(request: Request, status: u16, body: &str) {
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
        Err(_) => respond_text(request, 404, "browse.html not found\n"),
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
    let body = serde_json::to_string(&info).unwrap_or_default() + "\n";
    respond_json_value(request, 200, &body);
}

fn handle_fields(request: Request, field_name: &str, file_globs: &[String]) {
    // Validate field name: must be uppercase letters, digits, underscore
    if field_name.is_empty()
        || !field_name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        respond_text(request, 404, "Invalid field name\n");
        return;
    }

    let entries = read_all_entries(file_globs);
    if entries.is_empty() {
        respond_text(request, 404, "No entries found\n");
        return;
    }

    let mut values: Vec<String> = entries.iter().filter_map(|e| e.field(field_name)).collect();
    values.sort();
    values.dedup();

    if values.is_empty() {
        respond_text(request, 404, "No values for field\n");
        return;
    }

    let body = values.join("\n") + "\n";
    let response = Response::from_string(body)
        .with_status_code(StatusCode(200))
        .with_header(header("Content-Type", "text/plain"));
    let _ = request.respond(response);
}

fn handle_boots(request: Request, file_globs: &[String]) {
    let entries = read_all_entries(file_globs);
    let boots = detect_boots(&entries);

    // Output as JSON text sequences (RFC 7464): RS-prefixed JSON objects
    let mut body = Vec::new();
    for b in &boots {
        let obj = json!({
            "boot_id": b.boot_id,
            "first_entry": b.first_entry,
            "last_entry": b.last_entry,
        });
        // RS (0x1E) prefix for JSON text sequence
        body.push(0x1E);
        body.extend_from_slice(serde_json::to_string(&obj).unwrap_or_default().as_bytes());
        body.push(b'\n');
    }

    let response = Response::from_data(body)
        .with_status_code(StatusCode(200))
        .with_header(header("Content-Type", "application/json"));
    let _ = request.respond(response);
}

fn handle_entries(request: Request, file_globs: &[String]) {
    let url = request.url().to_owned();

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
        if param.is_empty() {
            continue;
        }
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

    if range.discrete {
        range.count = Some(1);
    }

    let entries = read_all_entries(file_globs);
    let results = filter_and_slice(&entries, &range, &query_filters, boot_filter);

    if follow && range.count.is_none() {
        // Follow with no count: send existing entries then block
        send_entries_follow(request, &accept, &results);
    } else {
        send_entries(request, &accept, &results);
    }
}

fn entry_to_json_with_cursor(entry: &JournalEntry, cursor: &str) -> serde_json::Value {
    let mut obj = entry.to_json();
    if let serde_json::Value::Object(ref mut map) = obj {
        map.insert(
            "__CURSOR".to_owned(),
            serde_json::Value::String(cursor.to_owned()),
        );
    }
    obj
}

fn send_entries(request: Request, accept: &str, results: &[(JournalEntry, String)]) {
    if accept.contains("application/json") {
        // JSON Lines: one JSON object per line
        let mut body = String::new();
        for (e, cursor) in results {
            let obj = entry_to_json_with_cursor(e, cursor);
            body.push_str(&serde_json::to_string(&obj).unwrap_or_default());
            body.push('\n');
        }
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
        // Default: text/plain
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

/// A Read adapter that yields initial data then blocks indefinitely.
struct FollowReader {
    data: std::io::Cursor<Vec<u8>>,
    done_initial: bool,
}

impl std::io::Read for FollowReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if !self.done_initial {
            let n = self.data.read(buf)?;
            if n > 0 {
                return Ok(n);
            }
            self.done_initial = true;
        }
        // Block until the connection is closed by the client
        loop {
            thread::sleep(std::time::Duration::from_secs(60));
        }
    }
}

fn send_entries_follow(request: Request, accept: &str, initial: &[(JournalEntry, String)]) {
    let content_type = if accept.contains("application/json") {
        "application/json"
    } else if accept.contains("text/event-stream") {
        "text/event-stream"
    } else {
        "text/plain"
    };

    let mut data = Vec::new();
    for (e, cursor) in initial {
        let line = if content_type == "application/json" {
            let obj = entry_to_json_with_cursor(e, cursor);
            serde_json::to_string(&obj).unwrap_or_default() + "\n"
        } else if content_type == "text/event-stream" {
            let obj = e.to_json();
            format!(
                "data: {}\n\n",
                serde_json::to_string(&obj).unwrap_or_default()
            )
        } else {
            format!("{}\n", e)
        };
        data.extend_from_slice(line.as_bytes());
    }

    let reader = FollowReader {
        data: std::io::Cursor::new(data),
        done_initial: false,
    };

    let response = Response::new(
        StatusCode(200),
        vec![header("Content-Type", content_type)],
        reader,
        None,
        None,
    );
    let _ = request.respond(response);
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
    let method = request.method().clone();
    let path = url.split('?').next().unwrap_or(&url);

    // /upload accepts POST/PUT
    if path == "/upload" {
        handle_upload(request);
        return;
    }

    // Everything else must be GET
    if method != Method::Get {
        respond_text(request, 405, "Method not allowed\n");
        return;
    }

    // Reject any path with ".." (path traversal)
    if path.contains("..") {
        respond_text(request, 404, "Not found\n");
        return;
    }

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
        _ if path.starts_with("/fields/") => {
            let field = &path["/fields/".len()..];
            handle_fields(request, field, file_globs);
        }
        "/fields" => respond_text(request, 404, "Field name required\n"),
        _ => respond_text(request, 404, "Not found\n"),
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

    let listener = if listen_fds == Some(1) && listen_pid == Some(my_pid) {
        // Socket activation: use FD 3
        let l = unsafe { TcpListener::from_raw_fd(3) };
        l.set_nonblocking(false).ok();
        l
    } else {
        // Parse --listen: bare port or host:port
        let addr = if cli.listen.contains(':') {
            cli.listen.clone()
        } else {
            format!("0.0.0.0:{}", cli.listen)
        };
        // Create socket with SO_REUSEADDR to avoid EADDRINUSE after restart
        let socket = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )
        .expect("Failed to create socket");
        socket
            .set_reuse_address(true)
            .expect("Failed to set SO_REUSEADDR");
        let sock_addr: std::net::SocketAddr = addr.parse().expect("Invalid listen address");
        socket
            .bind(&sock_addr.into())
            .unwrap_or_else(|e| panic!("Failed to bind to {addr}: {e}"));
        socket.listen(128).expect("Failed to listen");
        let listener: TcpListener = socket.into();
        listener.set_nonblocking(false).ok();
        listener
    };

    Server::from_listener(listener, ssl_config).expect("Failed to create server from listener")
}

fn main() {
    let cli = Cli::parse();
    let file_globs: Arc<Vec<String>> = Arc::new(cli.file.clone());

    let server = create_server(&cli);
    let server = Arc::new(server);

    // Handle requests in threads for concurrency
    while let Ok(request) = server.recv() {
        let globs = Arc::clone(&file_globs);
        thread::spawn(move || {
            handle_request(request, &globs);
        });
    }
}
