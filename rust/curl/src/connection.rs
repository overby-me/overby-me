use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use crate::options::Options;
use crate::tls::{InsecureVerifier, make_tls_config};
use crate::url::ParsedUrl;

/// Match --connect-to entries ("HOST1:PORT1:HOST2:PORT2") against the
/// requested host/port. An empty HOST1 or PORT1 acts as a wildcard. Returns
/// the (host, port) we should actually connect to.
fn connect_to_override(host: &str, port: u16, entries: &[String]) -> Option<(String, u16)> {
    let host_norm = host.trim_end_matches('.').to_ascii_lowercase();
    for entry in entries {
        let parts: Vec<&str> = entry.splitn(4, ':').collect();
        if parts.len() != 4 {
            continue;
        }
        let (h1, p1, h2, p2) = (parts[0], parts[1], parts[2], parts[3]);
        if !h1.is_empty() && h1.trim_end_matches('.').to_ascii_lowercase() != host_norm {
            continue;
        }
        if !p1.is_empty() && p1.parse::<u16>().ok() != Some(port) {
            continue;
        }
        let new_host = if h2.is_empty() {
            host.to_string()
        } else {
            h2.to_string()
        };
        let new_port = if p2.is_empty() {
            port
        } else if let Ok(p) = p2.parse::<u16>() {
            p
        } else {
            port
        };
        return Some((new_host, new_port));
    }
    None
}

/// Match --resolve entries ("host:port:addr") against the requested host:port.
/// Returns the destination "addr:port" string when a match is found.
/// Hostnames match case-insensitively and a trailing dot is ignored on either side.
fn resolve_override(host: &str, port: u16, resolves: &[String]) -> Option<String> {
    let host_norm = host.trim_end_matches('.').to_ascii_lowercase();
    for entry in resolves {
        let entry = entry.trim_start_matches('+');
        // host:port:addr[,addr2,...]
        let mut parts = entry.splitn(3, ':');
        let entry_host = parts.next()?;
        let entry_port = parts.next()?;
        let entry_addrs = parts.next()?;
        let entry_host_norm = entry_host.trim_end_matches('.').to_ascii_lowercase();
        if entry_host_norm != host_norm {
            continue;
        }
        if entry_port.parse::<u16>().ok() != Some(port) {
            continue;
        }
        // Take first address (comma-separated list possible).
        let first_addr = entry_addrs.split(',').next().unwrap_or(entry_addrs);
        let addr = if first_addr.contains(':') && !first_addr.starts_with('[') {
            format!("[{first_addr}]:{port}")
        } else {
            format!("{first_addr}:{port}")
        };
        return Some(addr);
    }
    None
}

pub(crate) enum Connection {
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

impl Connection {
    pub(crate) fn set_read_timeout(&mut self, dur: Option<Duration>) -> io::Result<()> {
        match self {
            Connection::Plain(s) => s.set_read_timeout(dur),
            Connection::Tls(s) => s.get_ref().set_read_timeout(dur),
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

/// Parse a proxy URL string into (host, port).
/// Supports formats: "host:port", "http://proxy.example:port", "http://proxy.example", "host"
/// Default port is 1080 when not specified.
pub(crate) fn parse_proxy(proxy: &str) -> Result<(String, u16), String> {
    // Strip scheme prefix if present; reject unsupported schemes.
    let stripped = if let Some(rest) = proxy.strip_prefix("http://") {
        rest
    } else if let Some(rest) = proxy.strip_prefix("https://") {
        rest
    } else if proxy.contains("://") {
        return Err(format!("unsupported proxy scheme in '{}'", proxy));
    } else {
        proxy
    };
    // Strip userinfo (user:pass@) if present
    let stripped = if let Some(at_pos) = stripped.find('@') {
        &stripped[at_pos + 1..]
    } else {
        stripped
    };
    // Strip path component (anything from the first '/' onward).
    let stripped = stripped.split('/').next().unwrap_or(stripped);
    // Handle [ipv6]:port
    if stripped.starts_with('[')
        && let Some(bracket_end) = stripped.find(']')
    {
        let host = &stripped[1..bracket_end];
        let after = &stripped[bracket_end + 1..];
        let port = if let Some(port_str) = after.strip_prefix(':') {
            port_str.parse().unwrap_or(1080)
        } else {
            1080
        };
        return Ok((host.to_string(), port));
    }
    // host:port
    if let Some(colon_pos) = stripped.rfind(':') {
        let host = &stripped[..colon_pos];
        let port_str = &stripped[colon_pos + 1..];
        if let Ok(port) = port_str.parse::<u16>() {
            return Ok((host.to_string(), port));
        }
    }
    // Just host, default port
    Ok((stripped.to_string(), 1080))
}

pub(crate) fn connect(url: &ParsedUrl, opts: &Options) -> Result<(Connection, Vec<u8>), String> {
    // RFC 7686: curl refuses to resolve `.onion` TLDs (preventing accidental
    // DNS leakage for Tor hidden services). --resolve overrides still work.
    let host_norm = url.host.trim_end_matches('.').to_ascii_lowercase();
    if host_norm.ends_with(".onion")
        && resolve_override(&url.host, url.port, &opts.resolves).is_none()
    {
        return Err("onion: Not resolving .onion address (RFC 7686)".into());
    }

    // Determine whether we need a CONNECT tunnel through the proxy.
    let use_tunnel = opts.proxy.is_some() && (opts.proxy_tunnel || url.scheme == "https");

    // When a proxy is configured, always connect to the proxy.
    // The decision about plain proxy vs CONNECT tunnel is handled separately.
    let (connect_host, connect_port) = if let Some(ref proxy) = opts.proxy {
        parse_proxy(proxy)?
    } else if let Some((h, p)) = connect_to_override(&url.host, url.port, &opts.connect_tos) {
        (h, p)
    } else {
        (url.host.clone(), url.port)
    };

    // --resolve overrides: "host:port:addr" (or multi-addr) replaces the
    // hostname lookup for the matching host:port.
    let resolved_addr = resolve_override(&connect_host, connect_port, &opts.resolves);

    // RFC 6761: .localhost TLD always resolves to loopback (127.0.0.1).
    // "localhost" itself and any subdomain (e.g. "foo.localhost") are handled.
    let connect_host_norm = connect_host.trim_end_matches('.').to_ascii_lowercase();
    let is_localhost =
        connect_host_norm == "localhost" || connect_host_norm.ends_with(".localhost");

    let addr = if let Some(ref resolved) = resolved_addr {
        resolved.clone()
    } else if is_localhost {
        format!("127.0.0.1:{}", connect_port)
    } else {
        format!("{}:{}", connect_host, connect_port)
    };

    // Resolve DNS first, so DNS failures can be distinguished from connection failures.
    let dns_kind = if opts.proxy.is_some() {
        "proxy"
    } else {
        "host"
    };
    let addrs: Vec<_> = std::net::ToSocketAddrs::to_socket_addrs(&addr)
        .map_err(|e| format!("DNS resolution failed for {dns_kind} {connect_host}: {e}"))?
        .collect();

    if addrs.is_empty() {
        return Err(format!(
            "DNS resolution failed for {dns_kind} {connect_host}: no addresses returned"
        ));
    }

    let tcp = if let Some(timeout) = opts.connect_timeout {
        let mut last_err = String::from("no addresses resolved");
        let mut stream = None;
        for a in &addrs {
            match TcpStream::connect_timeout(a, timeout) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(e) => last_err = format!("connection failed to {addr}: {e}"),
            }
        }
        stream.ok_or(last_err)?
    } else {
        let mut last_err = String::from("no addresses resolved");
        let mut stream = None;
        for a in &addrs {
            match TcpStream::connect(a) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(e) => last_err = format!("connection failed to {addr}: {e}"),
            }
        }
        stream.ok_or(last_err)?
    };

    // Apply --max-time when set; otherwise a 60s default read timeout prevents
    // indefinite hangs when a server keeps the connection open without sending
    // Content-Length or chunked framing.
    if let Some(timeout) = opts.max_time {
        let _ = tcp.set_read_timeout(Some(timeout));
        let _ = tcp.set_write_timeout(Some(timeout));
    } else {
        let _ = tcp.set_read_timeout(Some(std::time::Duration::from_secs(60)));
    }

    // CONNECT tunnel: when tunneling through a proxy, perform the CONNECT
    // handshake so the proxy opens a transparent tunnel to the target.
    let mut connect_headers = Vec::new();
    let tcp = if use_tunnel {
        let http_ver = if opts.proxy_1_0 {
            "HTTP/1.0"
        } else {
            "HTTP/1.1"
        };
        let target = format!("{}:{}", url.host, url.port);

        let mut req = format!("CONNECT {target} {http_ver}\r\n");
        req.push_str(&format!("Host: {target}\r\n"));

        // Proxy-Authorization
        if let Some(ref proxy_user) = opts.proxy_user {
            let encoded = crate::format::base64_encode(proxy_user.as_bytes());
            req.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
        }

        // User-Agent on CONNECT: default UA (or `--user-agent`) comes before
        // Proxy-Connection (test 749). When `--proxy-header User-Agent: ...`
        // is set, the default is suppressed and the proxy-header version is
        // emitted at the very end with the rest of the proxy headers (test 287).
        let proxy_ua = opts
            .proxy_headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("user-agent"));
        if !proxy_ua {
            let ua = opts.user_agent.as_deref().unwrap_or("curl/8.0.0");
            if !ua.is_empty() {
                req.push_str(&format!("User-Agent: {ua}\r\n"));
            }
        }

        req.push_str("Proxy-Connection: Keep-Alive\r\n");

        // --proxy-header values go after Proxy-Connection on CONNECT.
        for (k, v) in &opts.proxy_headers {
            if v.is_empty() || v == "\x00" {
                continue;
            }
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");

        use std::io::{BufRead, BufReader};
        let mut tcp = tcp;
        tcp.write_all(req.as_bytes())
            .map_err(|e| format!("failed to send CONNECT: {e}"))?;
        tcp.flush()
            .map_err(|e| format!("failed to flush CONNECT: {e}"))?;

        // Read CONNECT response headers.
        let mut reader = BufReader::new(tcp);
        let mut response_bytes = Vec::new();
        let mut status_code = 0u16;
        let mut first_line = true;
        loop {
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .map_err(|e| format!("failed to read CONNECT response: {e}"))?;
            response_bytes.extend_from_slice(line.as_bytes());
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break; // end of headers
            }
            if first_line {
                // Parse status code from "HTTP/1.x NNN ..."
                let parts: Vec<&str> = trimmed.splitn(3, ' ').collect();
                if parts.len() >= 2 {
                    status_code = parts[1].parse().unwrap_or(0);
                }
                first_line = false;
            }
        }

        if status_code != 200 {
            return Err(format!("CONNECT tunnel failed, response {status_code}"));
        }

        connect_headers = response_bytes;

        // Unwrap the TcpStream from the BufReader to continue using it.
        // The proxy should only send response headers then pass through,
        // so no application data should be buffered beyond what we read.
        reader.into_inner()
    } else {
        tcp
    };

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
        Ok((Connection::Tls(Box::new(stream)), connect_headers))
    } else {
        Ok((Connection::Plain(tcp), connect_headers))
    }
}
