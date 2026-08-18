use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use clap::Parser;
use libsystemd::journal::entry::from_export_format;
use libsystemd::journal::storage::{JournalCompress, JournalStorage, StorageConfig};
use tiny_http::{Response, Server, SslConfig, StatusCode};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "systemd-journal-remote",
    version,
    about = "Receive journal messages over the network"
)]
struct Cli {
    /// Write output to FILE or DIR.
    #[arg(short, long)]
    output: Option<String>,

    /// How to split output: "none" or "host".
    #[arg(long, default_value = "host")]
    split_mode: String,

    /// Listen for HTTP POST on ADDRESS.
    #[arg(long)]
    listen_http: Option<String>,

    /// Listen for HTTPS POST on ADDRESS.
    #[arg(long)]
    listen_https: Option<String>,

    /// Fetch journal events from URL (via curl).
    #[arg(long)]
    url: Option<String>,

    /// Program to invoke to get journal events.
    #[arg(long)]
    getter: Option<String>,

    /// SSL key file in PEM format.
    #[arg(long)]
    key: Option<String>,

    /// SSL certificate file in PEM format.
    #[arg(long)]
    cert: Option<String>,

    /// SSL CA certificate file in PEM format, or "-" to disable verification.
    #[arg(long)]
    trust: Option<String>,

    /// Use compression in the destination journal.
    #[arg(long)]
    compress: Option<Option<bool>>,

    /// Use Forward Secure Sealing.
    #[arg(long)]
    seal: Option<Option<bool>>,

    /// Input files (journal export format).
    #[arg(trailing_var_arg = true)]
    files: Vec<String>,
}

// ---------------------------------------------------------------------------
// Configuration (from journal-remote.conf)
// ---------------------------------------------------------------------------

struct Config {
    output: String,
    server_key: Option<String>,
    server_cert: Option<String>,
    #[allow(dead_code)]
    trusted_cert: Option<String>,
}

fn read_config() -> (Option<String>, Option<String>, Option<String>) {
    let mut server_key = None;
    let mut server_cert = None;
    let mut trusted_cert = None;

    for dir in &["/usr/lib/systemd", "/run/systemd", "/etc/systemd"] {
        let conf = format!("{}/journal-remote.conf", dir);
        if let Ok(content) = fs::read_to_string(&conf) {
            parse_config_content(
                &content,
                &mut server_key,
                &mut server_cert,
                &mut trusted_cert,
            );
        }
        let drop_dir = format!("{}/journal-remote.conf.d", dir);
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
                        &mut server_key,
                        &mut server_cert,
                        &mut trusted_cert,
                    );
                }
            }
        }
    }
    (server_key, server_cert, trusted_cert)
}

fn parse_config_content(
    content: &str,
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
                "ServerKeyFile" => *server_key = Some(value.trim().to_string()),
                "ServerCertificateFile" => *server_cert = Some(value.trim().to_string()),
                "TrustedCertificateFile" => *trusted_cert = Some(value.trim().to_string()),
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn open_output_storage(output: &str) -> io::Result<JournalStorage> {
    let path = Path::new(output);

    if output.ends_with(".journal") {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
    } else {
        fs::create_dir_all(path)?;
    }

    let directory = if output.ends_with(".journal") {
        path.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        path.to_path_buf()
    };

    let compress = std::env::var("SYSTEMD_JOURNAL_COMPRESS")
        .map(|s| JournalCompress::from_env_str(&s))
        .unwrap_or(JournalCompress::Zstd);

    let active_filename = if output.ends_with(".journal") {
        path.file_name().map(|n| n.to_string_lossy().into_owned())
    } else {
        None
    };

    let config = StorageConfig {
        directory,
        direct_directory: true,
        max_file_size: u64::MAX,
        max_disk_usage: u64::MAX,
        max_files: usize::MAX,
        persistent: false,
        keep_free: 0,
        compress,
        active_filename,
        ..Default::default()
    };
    JournalStorage::new(config)
}

fn import_entries_from_reader<R: BufRead>(
    reader: &mut R,
    storage: &mut JournalStorage,
) -> io::Result<usize> {
    let mut count = 0;
    while let Some(entry) = from_export_format(reader)? {
        storage.append(&entry)?;
        count += 1;
    }
    Ok(count)
}

// ---------------------------------------------------------------------------
// Active source modes (file, stdin, url, getter)
// ---------------------------------------------------------------------------

fn handle_active_sources(cli: &Cli, output: &str) -> io::Result<()> {
    let mut storage = open_output_storage(output)?;

    if let Some(url) = &cli.url {
        let mut cmd = Command::new("curl");
        cmd.arg("-LSfs")
            .arg("--header")
            .arg("Accept: application/vnd.fdo.journal")
            .arg(url)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = cmd
            .spawn()
            .map_err(|e| io::Error::other(format!("Failed to spawn curl: {}", e)))?;
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let count = import_entries_from_reader(&mut reader, &mut storage)?;
        let status = child.wait()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "curl exited with status {}",
                status
            )));
        }
        eprintln!("Imported {} entries from URL", count);
        return Ok(());
    }

    if let Some(getter) = &cli.getter {
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(getter)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = cmd
            .spawn()
            .map_err(|e| io::Error::other(format!("Failed to run getter: {}", e)))?;
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let count = import_entries_from_reader(&mut reader, &mut storage)?;
        let _ = child.wait();
        eprintln!("Imported {} entries via getter", count);
        return Ok(());
    }

    for file in &cli.files {
        if file == "-" {
            let stdin = io::stdin();
            let mut reader = BufReader::new(stdin.lock());
            let count = import_entries_from_reader(&mut reader, &mut storage)?;
            eprintln!("Imported {} entries from stdin", count);
        } else {
            let f = fs::File::open(file)?;
            let mut reader = BufReader::new(f);
            let count = import_entries_from_reader(&mut reader, &mut storage)?;
            eprintln!("Imported {} entries from {}", count, file);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// HTTP server request handler
// ---------------------------------------------------------------------------

fn handle_request(mut request: tiny_http::Request, storage: &Arc<Mutex<JournalStorage>>) {
    let method = request.method().as_str().to_uppercase();
    let url = request.url().to_string();

    if method != "POST" {
        let response =
            Response::from_string("Only POST is supported\n").with_status_code(StatusCode(405));
        let _ = request.respond(response);
        return;
    }

    if url != "/upload" {
        let response = Response::from_string("Not Found\n").with_status_code(StatusCode(404));
        let _ = request.respond(response);
        return;
    }

    let has_correct_ct = request.headers().iter().any(|h| {
        h.field
            .as_str()
            .as_str()
            .eq_ignore_ascii_case("content-type")
            && h.value.as_str() == "application/vnd.fdo.journal"
    });

    if !has_correct_ct {
        let response =
            Response::from_string("Unsupported Media Type\n").with_status_code(StatusCode(415));
        let _ = request.respond(response);
        return;
    }

    // Read body into memory, then parse and import
    let mut body = Vec::new();
    if let Err(e) = request.as_reader().read_to_end(&mut body) {
        eprintln!("Error reading request body: {}", e);
        let response = Response::from_string(format!("Internal Server Error: {}\n", e))
            .with_status_code(StatusCode(500));
        let _ = request.respond(response);
        return;
    }

    let mut cursor = io::Cursor::new(&body);
    let mut buf_reader = BufReader::new(&mut cursor);
    let mut storage = storage.lock().unwrap();

    match import_entries_from_reader(&mut buf_reader, &mut storage) {
        Ok(count) => {
            eprintln!("Imported {} entries via upload", count);
            let response = Response::from_string("OK\n").with_status_code(StatusCode(202));
            let _ = request.respond(response);
        }
        Err(e) => {
            eprintln!("Error importing entries: {}", e);
            let response = Response::from_string(format!("Internal Server Error: {}\n", e))
                .with_status_code(StatusCode(500));
            let _ = request.respond(response);
        }
    }
}

// ---------------------------------------------------------------------------
// Server loop
// ---------------------------------------------------------------------------

fn run_server(server: Server, config: &Config) -> io::Result<()> {
    let storage = Arc::new(Mutex::new(open_output_storage(&config.output)?));

    for request in server.incoming_requests() {
        handle_request(request, &storage);
    }

    Ok(())
}

fn read_ssl_config(config: &Config) -> io::Result<Option<SslConfig>> {
    let cert_path = config
        .server_cert
        .as_deref()
        .unwrap_or("/etc/ssl/certs/journal-remote.pem");
    let key_path = config
        .server_key
        .as_deref()
        .unwrap_or("/etc/ssl/private/journal-remote.pem");

    if cert_path == "-" || key_path == "-" {
        return Ok(None);
    }

    let cert_pem = fs::read(cert_path).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("Failed to read cert {}: {}", cert_path, e),
        )
    })?;
    let key_pem = fs::read(key_path)
        .map_err(|e| io::Error::new(e.kind(), format!("Failed to read key {}: {}", key_path, e)))?;

    Ok(Some(SslConfig {
        certificate: cert_pem,
        private_key: key_pem,
    }))
}

/// Parse listen address. If it starts with `-`, it's a negative fd number
/// (e.g. `-3` means use fd 3 as a pre-opened socket).
fn create_server(listen_addr: &str, ssl: Option<SslConfig>) -> io::Result<Server> {
    if let Some(fd_str) = listen_addr.strip_prefix('-') {
        let fd: i32 = fd_str.parse().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Invalid fd: {}", fd_str),
            )
        })?;
        use std::os::unix::io::FromRawFd;
        let listener = unsafe { std::net::TcpListener::from_raw_fd(fd) };
        Server::from_listener(listener, ssl).map_err(|e| io::Error::other(e.to_string()))
    } else if let Some(ssl) = ssl {
        Server::https(listen_addr, ssl).map_err(|e| io::Error::other(e.to_string()))
    } else {
        Server::http(listen_addr).map_err(|e| io::Error::other(e.to_string()))
    }
}

fn handle_passive_mode(config: &Config, listen_addr: &str, use_https: bool) -> io::Result<()> {
    let ssl = if use_https {
        read_ssl_config(config)?
    } else {
        None
    };

    let server = create_server(listen_addr, ssl)?;
    eprintln!("Listening on {}", listen_addr);
    run_server(server, config)
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    let (conf_key, conf_cert, conf_trust) = read_config();

    let server_key = cli.key.clone().or(conf_key);
    let server_cert = cli.cert.clone().or(conf_cert);
    let trusted_cert = cli.trust.clone().or(conf_trust);

    let output = cli.output.clone().unwrap_or_else(|| {
        if cli.split_mode == "none" {
            "/var/log/journal/remote/remote.journal".to_string()
        } else {
            "/var/log/journal/remote".to_string()
        }
    });

    let config = Config {
        output: output.clone(),
        server_key,
        server_cert,
        trusted_cert,
    };

    // Active source modes: file, url, getter
    if !cli.files.is_empty() || cli.url.is_some() || cli.getter.is_some() {
        if let Err(e) = handle_active_sources(&cli, &output) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    // Passive mode: HTTP/HTTPS listener
    if let Some(addr) = &cli.listen_https {
        if let Err(e) = handle_passive_mode(&config, addr, true) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if let Some(addr) = &cli.listen_http {
        if let Err(e) = handle_passive_mode(&config, addr, false) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    eprintln!(
        "No input source specified. Use --url, --getter, --listen-http, --listen-https, or provide files."
    );
    std::process::exit(1);
}
