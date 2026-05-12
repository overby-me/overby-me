use std::cell::RefCell;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use crate::options::Options;
use crate::tls::{InsecureVerifier, make_tls_config};
use crate::url::ParsedUrl;

thread_local! {
    /// Last failing CONNECT response bytes — set by `connect()` and read by
    /// main.rs when emitting error output (tests 217, 287).
    pub(crate) static CONNECT_RESP: RefCell<Option<(u16, Vec<u8>)>> = const { RefCell::new(None) };
    /// Local socket (ip, port) of the last successful connect — used for
    /// `%{local_ip}` and `%{local_port}` (test 435).
    pub(crate) static LOCAL_ADDR: RefCell<Option<(String, u16)>> = const { RefCell::new(None) };
    /// Single-slot HTTP/1.1 keep-alive pool. Holds the previous request's
    /// connection plus its `(scheme, host, port)` key. The key carries the
    /// origin (or proxy) we connected to; `is_proxy` distinguishes a
    /// CONNECT-tunneled stream (which can only be reused for the same
    /// origin) from a plain proxy stream (reusable for any HTTP origin
    /// going through the same proxy host:port). Tests 48, 1134, 1078.
    pub(crate) static CONN_POOL: RefCell<Option<PooledConn>> = const { RefCell::new(None) };
    /// Set by `connect()` when a pooled connection that had a prior HTTP/1.0
    /// response was reused — the request layer downgrades the next request to
    /// HTTP/1.0 (test 1074).
    pub(crate) static POOL_REUSED_HTTP10: RefCell<bool> = const { RefCell::new(false) };
    /// Set when a pooled connection was reused. The response reader rejects
    /// HTTP/0.9 (body-only, no status line) on a reused connection as a weird
    /// server reply (test 1479).
    pub(crate) static POOL_REUSED: RefCell<bool> = const { RefCell::new(false) };
}

pub(crate) struct PooledConn {
    pub host: String,
    pub port: u16,
    /// `true` when this is the direct stream to the proxy (no CONNECT tunnel
    /// up). `false` for plain origin connections OR after a CONNECT tunnel.
    pub is_proxy: bool,
    /// `true` when the previous response on this connection was HTTP/1.0 —
    /// the next request reusing the connection downgrades to HTTP/1.0
    /// (test 1074).
    pub http10: bool,
    pub conn: Connection,
}

/// Match --connect-to entries ("HOST1:PORT1:HOST2:PORT2") against the
/// requested host/port. An empty HOST1 or PORT1 acts as a wildcard. Returns
/// the (host, port) we should actually connect to.
/// Split a `--connect-to` entry into its four fields, respecting bracketed
/// IPv6 literals so colons inside `[...]` aren't treated as separators
/// (test 2053 — `[fc00::1]:8082:...:...`).
pub(crate) fn parse_connect_to(entry: &str) -> Option<[String; 4]> {
    let mut fields: Vec<String> = Vec::with_capacity(4);
    let mut buf = String::new();
    let mut depth: u32 = 0;
    for c in entry.chars() {
        if c == '[' {
            depth += 1;
            buf.push(c);
        } else if c == ']' {
            depth = depth.saturating_sub(1);
            buf.push(c);
        } else if c == ':' && depth == 0 {
            fields.push(std::mem::take(&mut buf));
            if fields.len() == 3 {
                // Final field collects the rest.
                fields.push(entry[fields.iter().map(|f| f.len() + 1).sum::<usize>()..].to_string());
                break;
            }
        } else {
            buf.push(c);
        }
    }
    if fields.len() < 4 {
        fields.push(buf);
    }
    if fields.len() == 4 {
        let mut it = fields.into_iter();
        let a = it.next()?;
        let b = it.next()?;
        let c = it.next()?;
        let d = it.next()?;
        Some([a, b, c, d])
    } else {
        None
    }
}

pub(crate) fn connect_to_override(
    host: &str,
    port: u16,
    entries: &[String],
) -> Option<(String, u16)> {
    let host_norm = host.trim_end_matches('.').to_ascii_lowercase();
    for entry in entries {
        let Some(parts) = parse_connect_to(entry) else {
            continue;
        };
        let (h1, p1, h2, p2) = (
            parts[0].as_str(),
            parts[1].as_str(),
            parts[2].as_str(),
            parts[3].as_str(),
        );
        // Strip [..] brackets from IPv6 host literals before matching.
        let h1_unwrapped = h1
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(h1);
        if !h1_unwrapped.is_empty()
            && h1_unwrapped.trim_end_matches('.').to_ascii_lowercase() != host_norm
        {
            continue;
        }
        if !p1.is_empty() && p1.parse::<u16>().ok() != Some(port) {
            continue;
        }
        // Strip [..] brackets from IPv6 destination literal too.
        let h2_unwrapped = h2
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(h2);
        let new_host = if h2_unwrapped.is_empty() {
            host.to_string()
        } else {
            h2_unwrapped.to_string()
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
        // Wildcard host "*" matches any hostname (test 1458).
        if entry_host_norm != "*" && entry_host_norm != host_norm {
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
    // Strip scheme prefix if present; reject unsupported schemes. SOCKS
    // schemes parse to host:port so we can attempt the TCP connect (and
    // return exit 7 on connect failure, tests 704/705) — a successful
    // SOCKS connect would still fail later because we can't do the
    // SOCKS handshake.
    let stripped = if let Some(rest) = proxy.strip_prefix("http://") {
        rest
    } else if let Some(rest) = proxy.strip_prefix("https://") {
        rest
    } else if let Some(rest) = proxy.strip_prefix("socks4://") {
        rest
    } else if let Some(rest) = proxy.strip_prefix("socks4a://") {
        rest
    } else if let Some(rest) = proxy.strip_prefix("socks5://") {
        rest
    } else if let Some(rest) = proxy.strip_prefix("socks5h://") {
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
    // Default: not reusing an HTTP/1.0 pooled connection. Each connect call
    // resets this so a stale value from a previous request never leaks into
    // the request-line builder.
    POOL_REUSED_HTTP10.with(|h| *h.borrow_mut() = false);
    POOL_REUSED.with(|h| *h.borrow_mut() = false);
    // RFC 7686: curl refuses to resolve `.onion` TLDs (preventing accidental
    // DNS leakage for Tor hidden services). --resolve overrides still work.
    let host_norm = url.host.trim_end_matches('.').to_ascii_lowercase();
    if host_norm.ends_with(".onion")
        && resolve_override(&url.host, url.port, &opts.resolves).is_none()
    {
        return Err("onion: Not resolving .onion address (RFC 7686)".into());
    }

    // Determine whether we need a CONNECT tunnel through the proxy.
    // `--connect-to` through a proxy automatically engages tunnel mode so
    // the connect-to host actually gets contacted (test 2050).
    let has_connect_to_match =
        connect_to_override(&url.host, url.port, &opts.connect_tos).is_some();
    let use_tunnel = opts.proxy.is_some()
        && (opts.proxy_tunnel || url.scheme == "https" || has_connect_to_match);

    // HTTP/1.1 keep-alive pool reuse — supported for plain HTTP and for
    // HTTP through an established CONNECT tunnel (test 275). HTTPS and
    // unproxied tunnels still skip the pool (rustls owns the stream).
    if url.scheme == "http" {
        // Compute the connect target up front so we can compare against the
        // pool entry.
        //   - With CONNECT tunnel: key on origin (the tunnel endpoint).
        //   - With plain proxy: key on proxy host:port.
        //   - No proxy: key on origin.
        let key_host_port: Option<(String, u16, bool)> = if use_tunnel {
            Some((url.host.clone(), url.port, false))
        } else if let Some(ref proxy) = opts.proxy {
            parse_proxy(proxy).ok().map(|(h, p)| (h, p, true))
        } else {
            Some((url.host.clone(), url.port, false))
        };
        if let Some((khost, kport, is_proxy_key)) = key_host_port
            && let Some(reused) = CONN_POOL.with(|r| {
                let mut slot = r.borrow_mut();
                if let Some(p) = slot.as_ref()
                    && p.host == khost
                    && p.port == kport
                    && p.is_proxy == is_proxy_key
                {
                    let was_http10 = p.http10;
                    let taken = slot.take().map(|p| p.conn);
                    POOL_REUSED_HTTP10.with(|h| *h.borrow_mut() = was_http10);
                    POOL_REUSED.with(|h| *h.borrow_mut() = true);
                    return taken;
                }
                POOL_REUSED_HTTP10.with(|h| *h.borrow_mut() = false);
                None
            })
        {
            return Ok((reused, Vec::new()));
        }
        POOL_REUSED_HTTP10.with(|h| *h.borrow_mut() = false);
    }

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

    // Strip IPv6 zone/scope ID — the kernel handles it via sin6_scope_id, not
    // address text. Both raw (%scope) and URL-encoded (%25scope) forms appear.
    let connect_addr_host = crate::url::strip_ipv6_scope(&connect_host);
    let addr = if let Some(ref resolved) = resolved_addr {
        resolved.clone()
    } else if is_localhost {
        format!("127.0.0.1:{}", connect_port)
    } else if connect_addr_host.contains(':') {
        format!("[{}]:{}", connect_addr_host, connect_port)
    } else {
        format!("{}:{}", connect_addr_host, connect_port)
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

    // Capture the local socket address for `%{local_ip}` / `%{local_port}`
    // (-w substitutions, test 435). Cheap read; ignore failure.
    if let Ok(local) = tcp.local_addr() {
        LOCAL_ADDR.with(|r| {
            *r.borrow_mut() = Some((local.ip().to_string(), local.port()));
        });
    }

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
        // RFC 7230: bracket IPv6 literals in CONNECT/Host targets.
        // --connect-to substitutes the CONNECT target so the proxy tunnels
        // to a different origin than the URL host (test 2050).
        let (tgt_host, tgt_port) = connect_to_override(&url.host, url.port, &opts.connect_tos)
            .unwrap_or_else(|| (url.host.clone(), url.port));
        let target = if tgt_host.contains(':') {
            format!("[{}]:{}", tgt_host, tgt_port)
        } else {
            format!("{}:{}", tgt_host, tgt_port)
        };

        let mut req = format!("CONNECT {target} {http_ver}\r\n");
        // If the user already supplied a Host through --proxy-header,
        // skip the auto one — they expect their value to be the only Host
        // sent on the CONNECT (test 1802).
        let proxy_host = opts
            .proxy_headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("host"));
        if !proxy_host {
            req.push_str(&format!("Host: {target}\r\n"));
        }

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
        let mut saw_cl = false;
        let mut saw_te = false;
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
                // Reject a CONNECT response that doesn't start with `HTTP/`
                // — curl exits 43 (CURLE_BAD_FUNCTION_ARGUMENT, repurposed
                // as "Invalid response header" in the tool) for that case
                // (test 750).
                if !trimmed.starts_with("HTTP/") {
                    return Err("invalid_connect_response".into());
                }
                // Parse status code from "HTTP/1.x NNN ..."
                let parts: Vec<&str> = trimmed.splitn(3, ' ').collect();
                if parts.len() >= 2 {
                    status_code = parts[1].parse().unwrap_or(0);
                }
                first_line = false;
            } else if let Some((name, _)) = trimmed.split_once(':') {
                let n = name.trim().to_ascii_lowercase();
                if n == "content-length" {
                    saw_cl = true;
                } else if n == "transfer-encoding" {
                    saw_te = true;
                }
            }
        }

        // Verbose: announce that CL/TE on a CONNECT 2xx are ignored —
        // they would otherwise frame the tunnel body, which doesn't apply
        // (the tunnel just passes bytes through). Test 1287.
        if (200..300).contains(&status_code) && opts.verbose {
            if saw_cl {
                eprintln!("* Ignoring Content-Length in CONNECT {status_code} response");
            }
            if saw_te {
                eprintln!("* Ignoring Transfer-Encoding in CONNECT {status_code} response");
            }
        }

        if !(200..300).contains(&status_code) {
            // Stash the failing CONNECT response — main.rs reads it for
            // stdout output and `%{http_connect}` substitution (217, 287).
            CONNECT_RESP.with(|r| *r.borrow_mut() = Some((status_code, response_bytes.clone())));
            return Err(format!("CONNECT tunnel failed, response {status_code}"));
        }
        // CONNECT 2xx — record the status so `%{http_connect}` reflects it
        // even on a successful tunnel (test 1904 uses 204).
        CONNECT_RESP.with(|r| *r.borrow_mut() = Some((status_code, response_bytes.clone())));

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
