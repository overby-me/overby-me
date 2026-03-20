use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// CLI options
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Options {
    urls: Vec<String>,
    method: Option<String>,            // -X
    headers: Vec<(String, String)>,    // -H
    data: Option<Vec<u8>>,             // -d / --data
    data_raw: bool,                    // --data-raw (no @ interpretation)
    form_fields: Vec<FormField>,       // -F
    output: Option<PathBuf>,           // -o
    remote_name: bool,                 // -O
    location: bool,                    // -L
    max_redirs: usize,                 // --max-redirs
    verbose: bool,                     // -v
    silent: bool,                      // -s
    show_error: bool,                  // -S
    fail: bool,                        // -f
    include_headers: bool,             // -i
    head: bool,                        // -I
    user_agent: Option<String>,        // -A
    referer: Option<String>,           // -e
    cookie: Option<String>,            // -b
    cookie_jar: Option<PathBuf>,       // -c
    user: Option<String>,              // -u user:password
    connect_timeout: Option<Duration>, // --connect-timeout
    max_time: Option<Duration>,        // -m / --max-time
    insecure: bool,                    // -k
    compressed: bool,                  // --compressed
    dump_header: Option<PathBuf>,      // -D
    write_out: Option<String>,         // -w
    retry: usize,                      // --retry
    range: Option<String>,             // -r
    upload_file: Option<PathBuf>,      // -T
    http_version: Option<String>,      // --http1.0, --http1.1
    no_keepalive: bool,                // --no-keepalive
    cacert: Option<PathBuf>,           // --cacert
    cert: Option<PathBuf>,             // --cert (client cert)
    cert_key: Option<PathBuf>,         // --key (client key)
}

#[derive(Clone, Debug)]
struct FormField {
    name: String,
    value: String,
    is_file: bool,
    content_type: Option<String>,
    filename: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            urls: Vec::new(),
            method: None,
            headers: Vec::new(),
            data: None,
            data_raw: false,
            form_fields: Vec::new(),
            output: None,
            remote_name: false,
            location: false,
            max_redirs: 50,
            verbose: false,
            silent: false,
            show_error: false,
            fail: false,
            include_headers: false,
            head: false,
            user_agent: None,
            referer: None,
            cookie: None,
            cookie_jar: None,
            user: None,
            connect_timeout: None,
            max_time: None,
            insecure: false,
            compressed: false,
            dump_header: None,
            write_out: None,
            retry: 0,
            range: None,
            upload_file: None,
            http_version: None,
            no_keepalive: false,
            cacert: None,
            cert: None,
            cert_key: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Parsed URL
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct ParsedUrl {
    scheme: String,
    host: String,
    port: u16,
    path: String,
    raw: String,
}

fn parse_url(raw: &str) -> Result<ParsedUrl, String> {
    let url = if !raw.contains("://") {
        format!("http://{raw}")
    } else {
        raw.to_string()
    };

    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| format!("invalid URL: {raw}"))?;

    let scheme = scheme.to_lowercase();
    let default_port: u16 = match scheme.as_str() {
        "http" => 80,
        "https" => 443,
        _ => return Err(format!("unsupported scheme: {scheme}")),
    };

    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };

    // Handle userinfo@ prefix.
    let host_port = match authority.rfind('@') {
        Some(i) => &authority[i + 1..],
        None => authority,
    };

    let (host, port) = if host_port.starts_with('[') {
        // IPv6
        match host_port.find(']') {
            Some(end) => {
                let h = &host_port[1..end];
                let p = if host_port.len() > end + 1 && host_port.as_bytes()[end + 1] == b':' {
                    host_port[end + 2..]
                        .parse::<u16>()
                        .map_err(|e| format!("bad port: {e}"))?
                } else {
                    default_port
                };
                (h.to_string(), p)
            }
            None => return Err("unterminated IPv6 address".into()),
        }
    } else {
        match host_port.rsplit_once(':') {
            Some((h, p)) => {
                let port = p.parse::<u16>().map_err(|e| format!("bad port: {e}"))?;
                (h.to_string(), port)
            }
            None => (host_port.to_string(), default_port),
        }
    };

    if host.is_empty() {
        return Err("empty host".into());
    }

    Ok(ParsedUrl {
        scheme,
        host,
        port,
        path: path.to_string(),
        raw: url,
    })
}

// ---------------------------------------------------------------------------
// TLS
// ---------------------------------------------------------------------------

fn make_tls_config(opts: &Options) -> Result<Arc<rustls::ClientConfig>, String> {
    let mut root_store = rustls::RootCertStore::empty();

    if let Some(ref ca_path) = opts.cacert {
        let pem = fs::read(ca_path).map_err(|e| format!("failed to read cacert: {e}"))?;
        let certs = rustls_pemfile::certs(&mut &pem[..])
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("failed to parse cacert PEM: {e}"))?;
        for cert in certs {
            root_store
                .add(cert)
                .map_err(|e| format!("failed to add CA cert: {e}"))?;
        }
    } else {
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        // Also try native certs.
        let native = rustls_native_certs::load_native_certs();
        for cert in native.certs {
            let _ = root_store.add(cert);
        }
    }

    let builder = rustls::ClientConfig::builder().with_root_certificates(root_store);

    let config = if let Some(ref cert_path) = opts.cert {
        let cert_pem =
            fs::read(cert_path).map_err(|e| format!("failed to read client cert: {e}"))?;
        let certs = rustls_pemfile::certs(&mut &cert_pem[..])
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("failed to parse client cert PEM: {e}"))?;

        let key_path = opts.cert_key.as_ref().unwrap_or(cert_path);
        let key_pem = fs::read(key_path).map_err(|e| format!("failed to read client key: {e}"))?;
        let key = rustls_pemfile::private_key(&mut &key_pem[..])
            .map_err(|e| format!("failed to parse client key PEM: {e}"))?
            .ok_or_else(|| "no private key found in PEM".to_string())?;

        builder
            .with_client_auth_cert(certs, key)
            .map_err(|e| format!("client auth setup failed: {e}"))?
    } else {
        builder.with_no_client_auth()
    };

    Ok(Arc::new(config))
}

/// A verifier that accepts any certificate (for -k / --insecure).
#[derive(Debug)]
struct InsecureVerifier;

impl rustls::client::danger::ServerCertVerifier for InsecureVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ED448,
        ]
    }
}

// ---------------------------------------------------------------------------
// HTTP connection
// ---------------------------------------------------------------------------

enum Connection {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Connection::Plain(s) => s.read(buf),
            Connection::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Connection::Plain(s) => s.write(buf),
            Connection::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Connection::Plain(s) => s.flush(),
            Connection::Tls(s) => s.flush(),
        }
    }
}

fn connect(url: &ParsedUrl, opts: &Options) -> Result<Connection, String> {
    let addr = format!("{}:{}", url.host, url.port);

    let tcp = if let Some(timeout) = opts.connect_timeout {
        let addrs: Vec<_> = std::net::ToSocketAddrs::to_socket_addrs(&addr)
            .map_err(|e| format!("DNS resolution failed for {}: {e}", url.host))?
            .collect();
        let mut last_err = String::from("no addresses resolved");
        let mut stream = None;
        for a in addrs {
            match TcpStream::connect_timeout(&a, timeout) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(e) => last_err = e.to_string(),
            }
        }
        stream.ok_or(last_err)?
    } else {
        TcpStream::connect(&addr).map_err(|e| format!("connection failed to {addr}: {e}"))?
    };

    if let Some(timeout) = opts.max_time {
        let _ = tcp.set_read_timeout(Some(timeout));
        let _ = tcp.set_write_timeout(Some(timeout));
    }

    if url.scheme == "https" {
        let tls_config = if opts.insecure {
            let mut config = rustls::ClientConfig::builder()
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth();
            config
                .dangerous()
                .set_certificate_verifier(Arc::new(InsecureVerifier));
            Arc::new(config)
        } else {
            make_tls_config(opts)?
        };

        let server_name = rustls::pki_types::ServerName::try_from(url.host.as_str())
            .map_err(|e| format!("invalid server name '{}': {e}", url.host))?
            .to_owned();
        let conn = rustls::ClientConnection::new(tls_config, server_name)
            .map_err(|e| format!("TLS handshake failed: {e}"))?;
        let stream = rustls::StreamOwned::new(conn, tcp);
        Ok(Connection::Tls(Box::new(stream)))
    } else {
        Ok(Connection::Plain(tcp))
    }
}

// ---------------------------------------------------------------------------
// HTTP response
// ---------------------------------------------------------------------------

struct Response {
    status: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    header_bytes: Vec<u8>,
}

fn read_response(conn: &mut Connection) -> Result<Response, String> {
    let mut reader = BufReader::new(conn);

    // Read status line.
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .map_err(|e| format!("failed to read status line: {e}"))?;
    let status_line = status_line.trim_end().to_string();

    let mut header_bytes = Vec::new();
    header_bytes.extend_from_slice(status_line.as_bytes());
    header_bytes.extend_from_slice(b"\r\n");

    let parts: Vec<&str> = status_line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(format!("malformed status line: {status_line}"));
    }
    let status: u16 = parts[1]
        .parse()
        .map_err(|_| format!("invalid status code: {}", parts[1]))?;
    let status_text = if parts.len() > 2 {
        parts[2].to_string()
    } else {
        String::new()
    };

    // Read headers.
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("failed to read header: {e}"))?;
        header_bytes.extend_from_slice(line.as_bytes());
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((key, val)) = trimmed.split_once(':') {
            headers.push((key.trim().to_lowercase(), val.trim().to_string()));
        }
    }

    // Read body based on Transfer-Encoding or Content-Length.
    let is_chunked = headers
        .iter()
        .any(|(k, v)| k == "transfer-encoding" && v.contains("chunked"));
    let content_length: Option<usize> = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok());

    let body = if is_chunked {
        read_chunked_body(&mut reader)?
    } else if let Some(len) = content_length {
        let mut buf = vec![0u8; len];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("failed to read body: {e}"))?;
        buf
    } else {
        // Read until EOF.
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        buf
    };

    Ok(Response {
        status,
        status_text,
        headers,
        body,
        header_bytes,
    })
}

fn read_chunked_body(reader: &mut impl BufRead) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        reader
            .read_line(&mut size_line)
            .map_err(|e| format!("failed to read chunk size: {e}"))?;
        let size_str = size_line.trim();
        // Strip chunk extensions.
        let size_str = size_str.split(';').next().unwrap_or(size_str);
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|_| format!("bad chunk size: {size_str}"))?;
        if size == 0 {
            // Read trailing CRLF.
            let mut trailer = String::new();
            let _ = reader.read_line(&mut trailer);
            break;
        }
        let mut chunk = vec![0u8; size];
        reader
            .read_exact(&mut chunk)
            .map_err(|e| format!("failed to read chunk: {e}"))?;
        body.extend_from_slice(&chunk);
        // Read trailing CRLF after chunk data.
        let mut crlf = [0u8; 2];
        let _ = reader.read_exact(&mut crlf);
    }
    Ok(body)
}

// ---------------------------------------------------------------------------
// Request building & execution
// ---------------------------------------------------------------------------

fn build_request(url: &ParsedUrl, opts: &Options) -> Vec<u8> {
    let method = if let Some(ref m) = opts.method {
        m.clone()
    } else if opts.head {
        "HEAD".into()
    } else if opts.data.is_some() || !opts.form_fields.is_empty() {
        "POST".into()
    } else if opts.upload_file.is_some() {
        "PUT".into()
    } else {
        "GET".into()
    };

    let http_ver = match opts.http_version.as_deref() {
        Some("1.0") => "HTTP/1.0",
        _ => "HTTP/1.1",
    };

    let mut req = format!("{method} {} {http_ver}\r\n", url.path);

    // Host header.
    let default_port = if url.scheme == "https" { 443 } else { 80 };
    if url.port == default_port {
        req.push_str(&format!("Host: {}\r\n", url.host));
    } else {
        req.push_str(&format!("Host: {}:{}\r\n", url.host, url.port));
    }

    // User-Agent.
    let ua = opts.user_agent.as_deref().unwrap_or("curl/8.0 (rust-curl)");
    req.push_str(&format!("User-Agent: {ua}\r\n"));
    req.push_str("Accept: */*\r\n");

    // Connection — always close since we don't reuse connections.
    req.push_str("Connection: close\r\n");

    // Accept-Encoding.
    if opts.compressed {
        req.push_str("Accept-Encoding: gzip, deflate\r\n");
    }

    // Referer.
    if let Some(ref referer) = opts.referer {
        req.push_str(&format!("Referer: {referer}\r\n"));
    }

    // Cookie.
    if let Some(ref cookie) = opts.cookie {
        // If it's a file path, read cookies from file.
        if std::path::Path::new(cookie).is_file() {
            if let Ok(contents) = fs::read_to_string(cookie) {
                let cookies: Vec<&str> = contents
                    .lines()
                    .filter(|l| !l.starts_with('#') && l.contains('\t'))
                    .collect();
                if !cookies.is_empty() {
                    // Netscape cookie format: domain, flag, path, secure, expiry, name, value
                    let cookie_pairs: Vec<String> = cookies
                        .iter()
                        .filter_map(|line| {
                            let fields: Vec<&str> = line.split('\t').collect();
                            if fields.len() >= 7 {
                                Some(format!("{}={}", fields[5], fields[6]))
                            } else {
                                None
                            }
                        })
                        .collect();
                    if !cookie_pairs.is_empty() {
                        req.push_str(&format!("Cookie: {}\r\n", cookie_pairs.join("; ")));
                    }
                }
            }
        } else {
            req.push_str(&format!("Cookie: {cookie}\r\n"));
        }
    }

    // Range.
    if let Some(ref range) = opts.range {
        req.push_str(&format!("Range: bytes={range}\r\n"));
    }

    // Basic auth.
    if let Some(ref user) = opts.user {
        let encoded = base64_encode(user.as_bytes());
        req.push_str(&format!("Authorization: Basic {encoded}\r\n"));
    }

    // Custom headers (may override defaults).
    for (key, val) in &opts.headers {
        req.push_str(&format!("{key}: {val}\r\n"));
    }

    // Body handling.
    let body = build_body(opts);

    if let Some(ref body) = body {
        // Set Content-Type if not already set by custom headers.
        let has_content_type = opts
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));
        if !has_content_type {
            if !opts.form_fields.is_empty() {
                let boundary = multipart_boundary(opts);
                req.push_str(&format!(
                    "Content-Type: multipart/form-data; boundary={boundary}\r\n"
                ));
            } else {
                req.push_str("Content-Type: application/x-www-form-urlencoded\r\n");
            }
        }
        req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }

    req.push_str("\r\n");

    let mut bytes = req.into_bytes();
    if let Some(body) = body {
        bytes.extend_from_slice(&body);
    }
    bytes
}

fn build_body(opts: &Options) -> Option<Vec<u8>> {
    if let Some(ref data) = opts.data {
        return Some(data.clone());
    }

    if !opts.form_fields.is_empty() {
        let boundary = multipart_boundary(opts);
        let mut body = Vec::new();
        for field in &opts.form_fields {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            if field.is_file {
                let filename = field.filename.as_deref().unwrap_or_else(|| {
                    std::path::Path::new(&field.value)
                        .file_name()
                        .and_then(|f| f.to_str())
                        .unwrap_or("file")
                });
                let ct = field
                    .content_type
                    .as_deref()
                    .unwrap_or("application/octet-stream");
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"; filename=\"{filename}\"\r\n\
                         Content-Type: {ct}\r\n\r\n",
                        field.name
                    )
                    .as_bytes(),
                );
                if let Ok(data) = fs::read(&field.value) {
                    body.extend_from_slice(&data);
                }
            } else {
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"\r\n\r\n{}",
                        field.name, field.value
                    )
                    .as_bytes(),
                );
            }
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        return Some(body);
    }

    if let Some(ref path) = opts.upload_file {
        if path.to_str() == Some("-") {
            let mut data = Vec::new();
            let _ = io::stdin().read_to_end(&mut data);
            return Some(data);
        }
        return fs::read(path).ok();
    }

    None
}

fn multipart_boundary(_opts: &Options) -> String {
    // Generate a stable but unique-ish boundary.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("------------------------{ts:032x}")
}

fn execute_request(url: &ParsedUrl, opts: &Options) -> Result<Response, String> {
    let mut conn = connect(url, opts)?;
    let request = build_request(url, opts);

    if opts.verbose {
        // Print request headers to stderr.
        if let Ok(req_str) = std::str::from_utf8(&request) {
            for line in req_str.split("\r\n") {
                if line.is_empty() {
                    break;
                }
                eprintln!("> {line}");
            }
            eprintln!(">");
        }
    }

    conn.write_all(&request)
        .map_err(|e| format!("failed to send request: {e}"))?;
    conn.flush()
        .map_err(|e| format!("failed to flush request: {e}"))?;

    let resp = read_response(&mut conn)?;

    if opts.verbose
        && let Ok(hdr_str) = std::str::from_utf8(&resp.header_bytes)
    {
        for line in hdr_str.split("\r\n") {
            if !line.is_empty() {
                eprintln!("< {line}");
            }
        }
        eprintln!("<");
    }

    Ok(resp)
}

fn perform(url_str: &str, opts: &Options) -> Result<Response, String> {
    let mut current_url = url_str.to_string();
    let mut redirects = 0;

    loop {
        let url = parse_url(&current_url)?;

        let resp = execute_request(&url, opts)?;

        // Handle redirects.
        if opts.location && (301..=308).contains(&resp.status) {
            if redirects >= opts.max_redirs {
                return Err(format!("maximum redirects ({}) followed", opts.max_redirs));
            }
            if let Some((_, location)) = resp.headers.iter().find(|(k, _)| k == "location") {
                redirects += 1;
                // Resolve relative URLs.
                if location.starts_with("http://") || location.starts_with("https://") {
                    current_url = location.clone();
                } else if location.starts_with('/') {
                    current_url = format!("{}://{}:{}{}", url.scheme, url.host, url.port, location);
                } else {
                    let base = match current_url.rfind('/') {
                        Some(i) => &current_url[..=i],
                        None => &current_url,
                    };
                    current_url = format!("{base}{location}");
                }
                if opts.verbose {
                    eprintln!("* Following redirect to {current_url}");
                }
                continue;
            }
        }

        return Ok(resp);
    }
}

// ---------------------------------------------------------------------------
// Minimal Base64 encoder (to avoid an extra dep)
// ---------------------------------------------------------------------------

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Cookie jar
// ---------------------------------------------------------------------------

fn save_cookie_jar(path: &PathBuf, url: &ParsedUrl, headers: &[(String, String)]) {
    let cookies: Vec<&str> = headers
        .iter()
        .filter(|(k, _)| k == "set-cookie")
        .map(|(_, v)| v.as_str())
        .collect();

    if cookies.is_empty() {
        return;
    }

    let mut lines = Vec::new();
    lines.push("# Netscape HTTP Cookie File".to_string());
    lines.push("# This file was generated by rust-curl".to_string());

    for cookie in cookies {
        let parts: Vec<&str> = cookie.split(';').collect();
        if let Some((name, value)) = parts[0].split_once('=') {
            let domain = url.host.clone();
            let path = "/";
            let secure = cookie.to_lowercase().contains("secure");
            let expire = "0";
            lines.push(format!(
                "{domain}\tTRUE\t{path}\t{}\t{expire}\t{}\t{}",
                if secure { "TRUE" } else { "FALSE" },
                name.trim(),
                value.trim()
            ));
        }
    }

    let _ = fs::write(path, lines.join("\n") + "\n");
}

// ---------------------------------------------------------------------------
// Write-out format
// ---------------------------------------------------------------------------

fn format_write_out(fmt: &str, resp: &Response, url: &ParsedUrl) -> String {
    let mut result = fmt.to_string();
    result = result.replace("%{http_code}", &resp.status.to_string());
    result = result.replace("%{response_code}", &resp.status.to_string());
    result = result.replace(
        "%{content_type}",
        resp.headers
            .iter()
            .find(|(k, _)| k == "content-type")
            .map(|(_, v)| v.as_str())
            .unwrap_or(""),
    );
    result = result.replace("%{size_download}", &resp.body.len().to_string());
    result = result.replace("%{size_header}", &resp.header_bytes.len().to_string());
    result = result.replace("%{url_effective}", &url.raw);
    result = result.replace("\\n", "\n");
    result = result.replace("\\t", "\t");
    result
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

fn parse_args() -> Options {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut opts = Options::default();

    if args.is_empty() {
        print_usage();
        process::exit(0);
    }

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                process::exit(0);
            }
            "-V" | "--version" => {
                println!("curl 8.0 (rust-curl 0.1.0)");
                process::exit(0);
            }
            "-X" | "--request" => {
                i += 1;
                opts.method = Some(next_arg(&args, i, "-X"));
            }
            "-H" | "--header" => {
                i += 1;
                let h = next_arg(&args, i, "-H");
                if let Some((k, v)) = h.split_once(':') {
                    opts.headers
                        .push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            "-d" | "--data" | "--data-ascii" => {
                i += 1;
                let val = next_arg(&args, i, "-d");
                append_data(&mut opts, &val, false);
            }
            "--data-raw" => {
                i += 1;
                let val = next_arg(&args, i, "--data-raw");
                opts.data_raw = true;
                append_data(&mut opts, &val, true);
            }
            "--data-binary" => {
                i += 1;
                let val = next_arg(&args, i, "--data-binary");
                append_data_binary(&mut opts, &val);
            }
            "--data-urlencode" => {
                i += 1;
                let val = next_arg(&args, i, "--data-urlencode");
                let encoded = urlencode_field(&val);
                append_data(&mut opts, &encoded, true);
            }
            "-F" | "--form" => {
                i += 1;
                let val = next_arg(&args, i, "-F");
                parse_form_field(&mut opts, &val);
            }
            "-o" | "--output" => {
                i += 1;
                opts.output = Some(PathBuf::from(next_arg(&args, i, "-o")));
            }
            "-O" | "--remote-name" => {
                opts.remote_name = true;
            }
            "-L" | "--location" => {
                opts.location = true;
            }
            "--max-redirs" => {
                i += 1;
                opts.max_redirs = next_arg(&args, i, "--max-redirs").parse().unwrap_or(50);
            }
            "-v" | "--verbose" => {
                opts.verbose = true;
            }
            "-s" | "--silent" => {
                opts.silent = true;
            }
            "-S" | "--show-error" => {
                opts.show_error = true;
            }
            "-f" | "--fail" => {
                opts.fail = true;
            }
            "-i" | "--include" => {
                opts.include_headers = true;
            }
            "-I" | "--head" => {
                opts.head = true;
            }
            "-A" | "--user-agent" => {
                i += 1;
                opts.user_agent = Some(next_arg(&args, i, "-A"));
            }
            "-e" | "--referer" => {
                i += 1;
                opts.referer = Some(next_arg(&args, i, "-e"));
            }
            "-b" | "--cookie" => {
                i += 1;
                opts.cookie = Some(next_arg(&args, i, "-b"));
            }
            "-c" | "--cookie-jar" => {
                i += 1;
                opts.cookie_jar = Some(PathBuf::from(next_arg(&args, i, "-c")));
            }
            "-u" | "--user" => {
                i += 1;
                opts.user = Some(next_arg(&args, i, "-u"));
            }
            "--connect-timeout" => {
                i += 1;
                let secs: f64 = next_arg(&args, i, "--connect-timeout")
                    .parse()
                    .unwrap_or(0.0);
                opts.connect_timeout = Some(Duration::from_secs_f64(secs));
            }
            "-m" | "--max-time" => {
                i += 1;
                let secs: f64 = next_arg(&args, i, "--max-time").parse().unwrap_or(0.0);
                opts.max_time = Some(Duration::from_secs_f64(secs));
            }
            "-k" | "--insecure" => {
                opts.insecure = true;
            }
            "--compressed" => {
                opts.compressed = true;
            }
            "-D" | "--dump-header" => {
                i += 1;
                opts.dump_header = Some(PathBuf::from(next_arg(&args, i, "-D")));
            }
            "-w" | "--write-out" => {
                i += 1;
                opts.write_out = Some(next_arg(&args, i, "-w"));
            }
            "--retry" => {
                i += 1;
                opts.retry = next_arg(&args, i, "--retry").parse().unwrap_or(0);
            }
            "-r" | "--range" => {
                i += 1;
                opts.range = Some(next_arg(&args, i, "-r"));
            }
            "-T" | "--upload-file" => {
                i += 1;
                opts.upload_file = Some(PathBuf::from(next_arg(&args, i, "-T")));
            }
            "--http1.0" | "-0" => {
                opts.http_version = Some("1.0".into());
            }
            "--http1.1" => {
                opts.http_version = Some("1.1".into());
            }
            "--no-keepalive" => {
                opts.no_keepalive = true;
            }
            "--cacert" => {
                i += 1;
                opts.cacert = Some(PathBuf::from(next_arg(&args, i, "--cacert")));
            }
            "--cert" | "-E" => {
                i += 1;
                opts.cert = Some(PathBuf::from(next_arg(&args, i, "--cert")));
            }
            "--key" => {
                i += 1;
                opts.cert_key = Some(PathBuf::from(next_arg(&args, i, "--key")));
            }
            "--url" => {
                i += 1;
                opts.urls.push(next_arg(&args, i, "--url"));
            }
            _ => {
                if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
                    // Handle combined short flags like -sSL
                    let chars: Vec<char> = arg[1..].chars().collect();
                    let mut j = 0;
                    while j < chars.len() {
                        match chars[j] {
                            'v' => opts.verbose = true,
                            's' => opts.silent = true,
                            'S' => opts.show_error = true,
                            'f' => opts.fail = true,
                            'i' => opts.include_headers = true,
                            'I' => opts.head = true,
                            'L' => opts.location = true,
                            'k' => opts.insecure = true,
                            'O' => opts.remote_name = true,
                            '0' => opts.http_version = Some("1.0".into()),
                            // Flags that consume the rest or next arg.
                            'o' => {
                                let rest: String = chars[j + 1..].iter().collect();
                                if rest.is_empty() {
                                    i += 1;
                                    opts.output = Some(PathBuf::from(next_arg(&args, i, "-o")));
                                } else {
                                    opts.output = Some(PathBuf::from(rest));
                                }
                                j = chars.len(); // consumed rest
                                continue;
                            }
                            'X' => {
                                let rest: String = chars[j + 1..].iter().collect();
                                if rest.is_empty() {
                                    i += 1;
                                    opts.method = Some(next_arg(&args, i, "-X"));
                                } else {
                                    opts.method = Some(rest);
                                }
                                j = chars.len();
                                continue;
                            }
                            'H' => {
                                i += 1;
                                let h = next_arg(&args, i, "-H");
                                if let Some((k, v)) = h.split_once(':') {
                                    opts.headers
                                        .push((k.trim().to_string(), v.trim().to_string()));
                                }
                                j = chars.len();
                                continue;
                            }
                            'd' => {
                                i += 1;
                                let val = next_arg(&args, i, "-d");
                                append_data(&mut opts, &val, false);
                                j = chars.len();
                                continue;
                            }
                            'u' => {
                                i += 1;
                                opts.user = Some(next_arg(&args, i, "-u"));
                                j = chars.len();
                                continue;
                            }
                            'A' => {
                                i += 1;
                                opts.user_agent = Some(next_arg(&args, i, "-A"));
                                j = chars.len();
                                continue;
                            }
                            'e' => {
                                i += 1;
                                opts.referer = Some(next_arg(&args, i, "-e"));
                                j = chars.len();
                                continue;
                            }
                            'b' => {
                                i += 1;
                                opts.cookie = Some(next_arg(&args, i, "-b"));
                                j = chars.len();
                                continue;
                            }
                            'c' => {
                                i += 1;
                                opts.cookie_jar = Some(PathBuf::from(next_arg(&args, i, "-c")));
                                j = chars.len();
                                continue;
                            }
                            'F' => {
                                i += 1;
                                let val = next_arg(&args, i, "-F");
                                parse_form_field(&mut opts, &val);
                                j = chars.len();
                                continue;
                            }
                            'D' => {
                                i += 1;
                                opts.dump_header = Some(PathBuf::from(next_arg(&args, i, "-D")));
                                j = chars.len();
                                continue;
                            }
                            'w' => {
                                i += 1;
                                opts.write_out = Some(next_arg(&args, i, "-w"));
                                j = chars.len();
                                continue;
                            }
                            'r' => {
                                i += 1;
                                opts.range = Some(next_arg(&args, i, "-r"));
                                j = chars.len();
                                continue;
                            }
                            'T' => {
                                i += 1;
                                opts.upload_file = Some(PathBuf::from(next_arg(&args, i, "-T")));
                                j = chars.len();
                                continue;
                            }
                            'm' => {
                                i += 1;
                                let secs: f64 = next_arg(&args, i, "-m").parse().unwrap_or(0.0);
                                opts.max_time = Some(Duration::from_secs_f64(secs));
                                j = chars.len();
                                continue;
                            }
                            'E' => {
                                i += 1;
                                opts.cert = Some(PathBuf::from(next_arg(&args, i, "-E")));
                                j = chars.len();
                                continue;
                            }
                            c => {
                                eprintln!("curl: unknown option '-{c}'");
                                process::exit(2);
                            }
                        }
                        j += 1;
                    }
                } else if arg.starts_with("--") {
                    eprintln!("curl: unknown option '{arg}'");
                    process::exit(2);
                } else {
                    opts.urls.push(arg.clone());
                }
            }
        }
        i += 1;
    }

    if opts.urls.is_empty() {
        eprintln!("curl: no URL specified");
        process::exit(2);
    }

    opts
}

fn next_arg(args: &[String], i: usize, flag: &str) -> String {
    if i >= args.len() {
        eprintln!("curl: option {flag} requires an argument");
        process::exit(2);
    }
    args[i].clone()
}

fn append_data(opts: &mut Options, val: &str, raw: bool) {
    let data = if let Some(path) = (!raw).then_some(()).and_then(|()| val.strip_prefix('@')) {
        if path == "-" {
            let mut buf = String::new();
            let _ = io::stdin().read_to_string(&mut buf);
            buf.into_bytes()
        } else {
            match fs::read(path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("curl: failed to read {path}: {e}");
                    process::exit(2);
                }
            }
        }
    } else {
        val.as_bytes().to_vec()
    };

    match opts.data {
        Some(ref mut existing) => {
            existing.push(b'&');
            existing.extend_from_slice(&data);
        }
        None => {
            opts.data = Some(data);
        }
    }
}

fn append_data_binary(opts: &mut Options, val: &str) {
    let data = if let Some(path) = val.strip_prefix('@') {
        if path == "-" {
            let mut buf = Vec::new();
            let _ = io::stdin().read_to_end(&mut buf);
            buf
        } else {
            match fs::read(path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("curl: failed to read {path}: {e}");
                    process::exit(2);
                }
            }
        }
    } else {
        val.as_bytes().to_vec()
    };

    match opts.data {
        Some(ref mut existing) => {
            existing.extend_from_slice(&data);
        }
        None => {
            opts.data = Some(data);
        }
    }
}

fn parse_form_field(opts: &mut Options, val: &str) {
    if let Some((name, rest)) = val.split_once('=') {
        if let Some(file_part) = rest.strip_prefix('@') {
            let mut path = file_part.to_string();
            let mut content_type = None;
            let mut filename = None;

            // Parse ;type= and ;filename= modifiers.
            if let Some(semicolon) = file_part.find(';') {
                path = file_part[..semicolon].to_string();
                for modifier in file_part[semicolon + 1..].split(';') {
                    let modifier = modifier.trim();
                    if let Some(ct) = modifier.strip_prefix("type=") {
                        content_type = Some(ct.to_string());
                    } else if let Some(fn_) = modifier.strip_prefix("filename=") {
                        filename = Some(fn_.to_string());
                    }
                }
            }

            opts.form_fields.push(FormField {
                name: name.to_string(),
                value: path,
                is_file: true,
                content_type,
                filename,
            });
        } else {
            opts.form_fields.push(FormField {
                name: name.to_string(),
                value: rest.to_string(),
                is_file: false,
                content_type: None,
                filename: None,
            });
        }
    }
}

fn urlencode_field(val: &str) -> String {
    // Format: name=content or =content or content
    if let Some((name, content)) = val.split_once('=') {
        if name.is_empty() {
            urlencode(content)
        } else {
            format!("{}={}", name, urlencode(content))
        }
    } else {
        urlencode(val)
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

fn print_usage() {
    println!(
        "\
Usage: curl [options...] <url>

Options:
  -X, --request <method>    HTTP method (GET, POST, PUT, DELETE, etc.)
  -H, --header <header>     Custom header (e.g. \"Content-Type: application/json\")
  -d, --data <data>         POST data (use @file to read from file)
  --data-raw <data>         POST data without @ file interpretation
  --data-binary <data>      POST binary data
  --data-urlencode <data>   URL-encode POST data
  -F, --form <name=value>   Multipart form data (use @file for file upload)
  -o, --output <file>       Write output to file
  -O, --remote-name         Write output to file named from URL
  -L, --location            Follow redirects
  --max-redirs <num>        Maximum number of redirects (default: 50)
  -v, --verbose             Verbose output
  -s, --silent              Silent mode
  -S, --show-error          Show errors in silent mode
  -f, --fail                Fail silently on HTTP errors
  -i, --include             Include response headers in output
  -I, --head                HEAD request (headers only)
  -A, --user-agent <agent>  User-Agent header
  -e, --referer <url>       Referer header
  -b, --cookie <data|file>  Send cookies
  -c, --cookie-jar <file>   Save cookies to file
  -u, --user <user:pass>    Basic authentication
  --connect-timeout <secs>  Connection timeout
  -m, --max-time <secs>     Maximum transfer time
  -k, --insecure            Skip TLS verification
  --compressed              Request compressed response
  -D, --dump-header <file>  Dump headers to file
  -w, --write-out <format>  Output format after transfer
  --retry <num>             Retry count on failure
  -r, --range <range>       Byte range (e.g. 0-499)
  -T, --upload-file <file>  Upload file (PUT)
  -0, --http1.0             Use HTTP/1.0
  --http1.1                 Use HTTP/1.1
  --no-keepalive            Disable keepalive
  --cacert <file>           CA certificate bundle
  -E, --cert <file>         Client certificate
  --key <file>              Client private key
  --url <url>               Explicit URL
  -h, --help                Show this help
  -V, --version             Show version"
    );
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let opts = parse_args();
    let mut exit_code = 0;

    for url_str in &opts.urls {
        let result = if opts.retry > 0 {
            let mut last_err = String::new();
            let mut resp = None;
            for attempt in 0..=opts.retry {
                if attempt > 0 {
                    if !opts.silent {
                        eprintln!(
                            "Warning: Transient problem. Will retry in {} seconds. ({attempt}/{} retries)",
                            attempt * 2,
                            opts.retry,
                        );
                    }
                    std::thread::sleep(Duration::from_secs((attempt * 2) as u64));
                }
                match perform(url_str, &opts) {
                    Ok(r) => {
                        // Retry on 5xx.
                        if r.status >= 500 && attempt < opts.retry {
                            last_err = format!("HTTP {}", r.status);
                            continue;
                        }
                        resp = Some(r);
                        break;
                    }
                    Err(e) => {
                        last_err = e;
                        if attempt == opts.retry {
                            break;
                        }
                    }
                }
            }
            resp.ok_or(last_err)
        } else {
            perform(url_str, &opts)
        };

        match result {
            Ok(resp) => {
                let url = parse_url(url_str).unwrap();

                if opts.fail && resp.status >= 400 {
                    if opts.show_error || !opts.silent {
                        eprintln!(
                            "curl: (22) The requested URL returned error: {} {}",
                            resp.status, resp.status_text
                        );
                    }
                    exit_code = 22;
                    continue;
                }

                // Dump headers to file.
                if let Some(ref dump_path) = opts.dump_header {
                    let _ = fs::write(dump_path, &resp.header_bytes);
                }

                // Save cookie jar.
                if let Some(ref jar_path) = opts.cookie_jar {
                    save_cookie_jar(jar_path, &url, &resp.headers);
                }

                // Determine output destination.
                let output_path = if opts.remote_name {
                    // Derive filename from URL path.
                    let name = url
                        .path
                        .rsplit('/')
                        .next()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("index.html");
                    Some(PathBuf::from(name))
                } else {
                    opts.output.clone()
                };

                // Write output.
                if let Some(ref path) = output_path {
                    if opts.include_headers || opts.head {
                        let mut data = resp.header_bytes.clone();
                        if !opts.head {
                            data.extend_from_slice(&resp.body);
                        }
                        if let Err(e) = fs::write(path, &data) {
                            eprintln!("curl: failed to write to {}: {e}", path.display());
                            exit_code = 23;
                        }
                    } else if let Err(e) = fs::write(path, &resp.body) {
                        eprintln!("curl: failed to write to {}: {e}", path.display());
                        exit_code = 23;
                    }
                } else {
                    let stdout = io::stdout();
                    let mut out = stdout.lock();

                    if opts.include_headers || opts.head {
                        let _ = out.write_all(&resp.header_bytes);
                    }
                    if !opts.head {
                        let _ = out.write_all(&resp.body);
                    }
                    let _ = out.flush();
                }

                // Write-out.
                if let Some(ref fmt) = opts.write_out {
                    let formatted = format_write_out(fmt, &resp, &url);
                    eprint!("{formatted}");
                }
            }
            Err(e) => {
                if !opts.silent || opts.show_error {
                    eprintln!("curl: {e}");
                }
                exit_code = 6; // Could not resolve / connect.
            }
        }
    }

    process::exit(exit_code);
}
