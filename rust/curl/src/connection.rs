use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use crate::options::Options;
use crate::tls::{InsecureVerifier, make_tls_config};
use crate::url::ParsedUrl;

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
        // IPv6 may have colons — wrap in brackets if so and not already bracketed.
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
        // socks4://, socks5://, etc. — not supported
        return Err(format!("unsupported proxy scheme in '{}'", proxy));
    } else {
        proxy
    };
    // Strip trailing slash
    let stripped = stripped.trim_end_matches('/');
    // Strip userinfo (user:pass@) if present
    let stripped = if let Some(at_pos) = stripped.find('@') {
        &stripped[at_pos + 1..]
    } else {
        stripped
    };
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

pub(crate) fn connect(url: &ParsedUrl, opts: &Options) -> Result<Connection, String> {
    // RFC 7686: curl refuses to resolve `.onion` TLDs (preventing accidental
    // DNS leakage for Tor hidden services). --resolve overrides still work.
    let host_norm = url.host.trim_end_matches('.').to_ascii_lowercase();
    if host_norm.ends_with(".onion")
        && resolve_override(&url.host, url.port, &opts.resolves).is_none()
    {
        return Err("onion: Not resolving .onion address (RFC 7686)".into());
    }

    // When an HTTP proxy is configured and the target is plain HTTP,
    // connect to the proxy instead of the target.
    let (connect_host, connect_port) = if let Some(ref proxy) = opts.proxy {
        if url.scheme == "http" {
            parse_proxy(proxy)?
        } else {
            (url.host.clone(), url.port)
        }
    } else {
        (url.host.clone(), url.port)
    };

    // --resolve overrides: "host:port:addr" (or multi-addr) replaces the
    // hostname lookup for the matching host:port.
    let resolved_addr = resolve_override(&connect_host, connect_port, &opts.resolves);
    let addr = resolved_addr
        .clone()
        .unwrap_or_else(|| format!("{}:{}", connect_host, connect_port));

    // Resolve DNS first, so DNS failures can be distinguished from connection failures.
    let addrs: Vec<_> = std::net::ToSocketAddrs::to_socket_addrs(&addr)
        .map_err(|e| format!("DNS resolution failed for {}: {e}", connect_host))?
        .collect();

    if addrs.is_empty() {
        return Err(format!(
            "DNS resolution failed for {}: no addresses returned",
            connect_host
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
    // Content-Length or chunked framing. curl's real behaviour is also bounded
    // by SO_KEEPALIVE + OS defaults; 60s is a safe upper bound that still
    // accommodates slow responses in integration tests.
    if let Some(timeout) = opts.max_time {
        let _ = tcp.set_read_timeout(Some(timeout));
        let _ = tcp.set_write_timeout(Some(timeout));
    } else {
        let _ = tcp.set_read_timeout(Some(std::time::Duration::from_secs(60)));
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
