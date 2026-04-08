use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use clap::Parser;
use libsystemd::journal::entry::JournalEntry;
use libsystemd::journal::storage::{JournalStorage, StorageConfig};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "systemd-journal-upload",
    version,
    about = "Send journal messages to a remote host"
)]
struct Cli {
    /// Upload to this address (http:// or https://).
    #[arg(short, long)]
    url: Option<String>,

    /// SSL key file in PEM format.
    #[arg(long)]
    key: Option<String>,

    /// SSL certificate file in PEM format.
    #[arg(long)]
    cert: Option<String>,

    /// SSL CA certificate file in PEM format, or "all"/"-" to disable.
    #[arg(long)]
    trust: Option<String>,

    /// Use system journal.
    #[arg(long)]
    system: bool,

    /// Use current user's journal.
    #[arg(long)]
    user: bool,

    /// Merge all available journals.
    #[arg(short, long)]
    merge: bool,

    /// Use journal files from directory.
    #[arg(short = 'D', long)]
    directory: Option<String>,

    /// Use specific journal file.
    #[arg(long)]
    file: Vec<String>,

    /// Journal namespace.
    #[arg(long)]
    namespace: Option<String>,

    /// Start at specified cursor.
    #[arg(long)]
    cursor: Option<String>,

    /// Start after specified cursor.
    #[arg(long)]
    after_cursor: Option<String>,

    /// Save upload state (cursor) to file.
    #[arg(long)]
    save_state: Option<Option<String>>,

    /// Wait for new entries.
    #[arg(long)]
    follow: Option<Option<bool>>,
}

// ---------------------------------------------------------------------------
// Configuration (from journal-upload.conf)
// ---------------------------------------------------------------------------

fn read_config() -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let mut url = None;
    let mut server_key = None;
    let mut server_cert = None;
    let mut trusted_cert = None;

    for dir in &["/usr/lib/systemd", "/run/systemd", "/etc/systemd"] {
        let conf = format!("{}/journal-upload.conf", dir);
        if let Ok(content) = fs::read_to_string(&conf) {
            parse_config_content(
                &content,
                &mut url,
                &mut server_key,
                &mut server_cert,
                &mut trusted_cert,
            );
        }
        let drop_dir = format!("{}/journal-upload.conf.d", dir);
        if let Ok(entries) = fs::read_dir(&drop_dir) {
            let mut files: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            files.sort_by_key(|e| e.file_name());
            for entry in files {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "conf")
                    && let Ok(content) = fs::read_to_string(&path)
                {
                    parse_config_content(
                        &content,
                        &mut url,
                        &mut server_key,
                        &mut server_cert,
                        &mut trusted_cert,
                    );
                }
            }
        }
    }
    (url, server_key, server_cert, trusted_cert)
}

fn parse_config_content(
    content: &str,
    url: &mut Option<String>,
    server_key: &mut Option<String>,
    server_cert: &mut Option<String>,
    trusted_cert: &mut Option<String>,
) {
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() || line.starts_with('[') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            match key.trim() {
                "URL" => *url = Some(value.trim().to_string()),
                "ServerKeyFile" => *server_key = Some(value.trim().to_string()),
                "ServerCertificateFile" => *server_cert = Some(value.trim().to_string()),
                "TrustedCertificateFile" => *trusted_cert = Some(value.trim().to_string()),
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// State (cursor) management
// ---------------------------------------------------------------------------

fn load_cursor(state_file: &str) -> Option<String> {
    let content = fs::read_to_string(state_file).ok()?;
    for line in content.lines() {
        if let Some(cursor) = line.strip_prefix("LAST_CURSOR=") {
            return Some(cursor.to_string());
        }
    }
    None
}

fn save_cursor(state_file: &str, cursor: &str) -> io::Result<()> {
    let tmp = format!("{}.tmp", state_file);
    let content = format!(
        "# This is private data. Do not parse.\nLAST_CURSOR={}\n",
        cursor
    );
    fs::write(&tmp, content)?;
    fs::rename(&tmp, state_file)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Journal reading
// ---------------------------------------------------------------------------

fn read_journal_entries(cli: &Cli) -> io::Result<Vec<JournalEntry>> {
    let journal_dir = if let Some(dir) = &cli.directory {
        PathBuf::from(dir)
    } else {
        let persistent = PathBuf::from("/var/log/journal");
        let volatile = PathBuf::from("/run/log/journal");
        if persistent.exists() {
            persistent
        } else {
            volatile
        }
    };

    let config = StorageConfig {
        directory: journal_dir,
        direct_directory: cli.directory.is_some(),
        max_file_size: u64::MAX,
        max_disk_usage: u64::MAX,
        max_files: usize::MAX,
        persistent: false,
        keep_free: 0,
        ..Default::default()
    };

    let storage = JournalStorage::open_read_only(config)?;
    let mut entries = storage.read_all()?;
    entries.sort_by_key(|e| (e.realtime_usec, e.seqnum));
    Ok(entries)
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

// ---------------------------------------------------------------------------
// Upload via curl
// ---------------------------------------------------------------------------

fn upload_entries(
    entries: &[JournalEntry],
    url: &str,
    key: Option<&str>,
    cert: Option<&str>,
    trust: Option<&str>,
) -> io::Result<String> {
    // Serialize entries to export format
    let mut body = Vec::new();
    let mut last_cursor = String::new();
    for entry in entries {
        let cursor = make_cursor(entry);
        body.extend_from_slice(&entry.to_export_format(&cursor));
        body.push(b'\n');
        last_cursor = cursor;
    }

    if entries.is_empty() {
        return Ok(String::new());
    }

    let upload_url = format!("{}/upload", url.trim_end_matches('/'));

    let mut cmd = Command::new("curl");
    cmd.arg("-LSfs")
        .arg("--header")
        .arg("Content-Type: application/vnd.fdo.journal")
        .arg("--data-binary")
        .arg("@-")
        .arg(&upload_url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // TLS options
    if let Some(key) = key
        && key != "-"
    {
        cmd.arg("--key").arg(key);
    }
    if let Some(cert) = cert
        && cert != "-"
    {
        cmd.arg("--cert").arg(cert);
    }
    match trust {
        Some("all") | Some("-") | None => {
            cmd.arg("--insecure");
        }
        Some(ca) => {
            cmd.arg("--cacert").arg(ca);
        }
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| io::Error::other(format!("Failed to spawn curl: {}", e)))?;

    // Write body to stdin
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&body)?;
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!(
            "Upload to {} failed with code {}: {}",
            upload_url,
            output.status.code().unwrap_or(-1),
            stderr.trim()
        )));
    }

    Ok(last_cursor)
}

// ---------------------------------------------------------------------------
// Server health check
// ---------------------------------------------------------------------------

fn check_server(
    url: &str,
    key: Option<&str>,
    cert: Option<&str>,
    trust: Option<&str>,
) -> io::Result<()> {
    let upload_url = format!("{}/upload", url.trim_end_matches('/'));

    let mut cmd = Command::new("curl");
    cmd.arg("-LSfs")
        .arg("--head")
        .arg("--max-time")
        .arg("5")
        .arg(&upload_url)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    if let Some(key) = key
        && key != "-"
    {
        cmd.arg("--key").arg(key);
    }
    if let Some(cert) = cert
        && cert != "-"
    {
        cmd.arg("--cert").arg(cert);
    }
    match trust {
        Some("all") | Some("-") | None => {
            cmd.arg("--insecure");
        }
        Some(ca) => {
            cmd.arg("--cacert").arg(ca);
        }
    }

    let output = cmd
        .output()
        .map_err(|e| io::Error::other(format!("Failed to run curl: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!(
            "Server unreachable at {}: {}",
            upload_url,
            stderr.trim()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    let (conf_url, conf_key, conf_cert, conf_trust) = read_config();

    let url = cli.url.clone().or(conf_url);
    let key = cli.key.clone().or(conf_key);
    let cert = cli.cert.clone().or(conf_cert);
    let trust = cli.trust.clone().or(conf_trust);

    let url = match url {
        Some(u) => u,
        None => {
            eprintln!("No URL specified. Use --url or configure URL in journal-upload.conf");
            std::process::exit(1);
        }
    };

    // Load cursor from state file or CLI
    let state_file = cli.save_state.as_ref().map(|s| {
        s.clone()
            .unwrap_or_else(|| "/var/lib/systemd/journal-upload/state".to_string())
    });

    let mut last_cursor = cli
        .after_cursor
        .clone()
        .or_else(|| cli.cursor.clone())
        .or_else(|| state_file.as_ref().and_then(|f| load_cursor(f)));

    // Determine if we should keep running (follow mode).
    // --save-state implies continuous operation (like the C implementation).
    let follow = cli
        .follow
        .as_ref()
        .map(|v| v.unwrap_or(true))
        .unwrap_or(state_file.is_some());

    loop {
        // Read journal entries
        let entries = match read_journal_entries(&cli) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Failed to read journal: {}", e);
                if !follow {
                    std::process::exit(1);
                }
                std::thread::sleep(std::time::Duration::from_secs(5));
                continue;
            }
        };

        // Filter entries after cursor
        let entries: Vec<&JournalEntry> = if let Some(ref cursor) = last_cursor {
            let cursor_seqnum = parse_cursor_seqnum(cursor);
            let pos = cursor_seqnum.and_then(|seq| entries.iter().position(|e| e.seqnum == seq));
            match pos {
                Some(idx) => entries[idx + 1..].iter().collect(),
                None => entries.iter().collect(),
            }
        } else {
            entries.iter().collect()
        };

        if entries.is_empty() {
            if !follow {
                eprintln!("No new entries to upload");
                break;
            }
            // Check server is still reachable while idle
            if let Err(e) = check_server(&url, key.as_deref(), cert.as_deref(), trust.as_deref()) {
                eprintln!("Server check failed: {}", e);
                std::process::exit(1);
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
            continue;
        }

        eprintln!("Uploading {} entries to {}", entries.len(), url);

        let owned_entries: Vec<JournalEntry> = entries.into_iter().cloned().collect();

        match upload_entries(
            &owned_entries,
            &url,
            key.as_deref(),
            cert.as_deref(),
            trust.as_deref(),
        ) {
            Ok(cursor) => {
                if !cursor.is_empty() {
                    if let Some(ref state_file) = state_file
                        && let Err(e) = save_cursor(state_file, &cursor)
                    {
                        eprintln!("Warning: failed to save state: {}", e);
                    }
                    last_cursor = Some(cursor);
                }
                eprintln!("Upload complete");
            }
            Err(e) => {
                // Exit on upload failure — let systemd's Restart= handle retries
                eprintln!("Upload failed: {}", e);
                std::process::exit(1);
            }
        }

        if !follow {
            break;
        }

        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}

fn parse_cursor_seqnum(cursor: &str) -> Option<u64> {
    for part in cursor.split(';') {
        if let Some(val) = part.strip_prefix("i=") {
            return u64::from_str_radix(val, 16).ok();
        }
    }
    None
}
