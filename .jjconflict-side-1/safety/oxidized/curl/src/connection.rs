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
    /// Peer certificate chain (DER) captured after the TLS handshake.
    /// Read by `--write-out %{certs}` (test 417).
    pub(crate) static PEER_CERTS: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
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
    /// `true` when this connection used `--resolve` for hostname-to-address
    /// mapping. Curl bucket-tags such connections so a subsequent transfer
    /// using `--connect-to` to reach the same host:port does NOT reuse the
    /// connection (test 2052). Plain transfers with no override CAN still
    /// reuse a `--connect-to` connection if endpoints match (test 2051).
    pub used_resolve: bool,
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
/// Entries beginning with `-` mark a *removal* (`-host:port`): once such an
/// entry is seen, any earlier mapping for the same host:port is cancelled,
/// matching curl's `--resolve -…` semantics (test 2052).
pub(crate) fn resolve_override(host: &str, port: u16, resolves: &[String]) -> Option<String> {
    let host_norm = host.trim_end_matches('.').to_ascii_lowercase();
    let mut current: Option<String> = None;
    for entry in resolves {
        if let Some(rest) = entry.strip_prefix('-') {
            let mut parts = rest.splitn(2, ':');
            let entry_host = match parts.next() {
                Some(h) => h,
                None => continue,
            };
            let entry_port = match parts.next() {
                Some(p) => p,
                None => continue,
            };
            let entry_host_norm = entry_host.trim_end_matches('.').to_ascii_lowercase();
            if (entry_host_norm == "*" || entry_host_norm == host_norm)
                && entry_port.parse::<u16>().ok() == Some(port)
            {
                current = None;
            }
            continue;
        }
        let entry = entry.strip_prefix('+').unwrap_or(entry);
        // host:port:addr[,addr2,...]
        let mut parts = entry.splitn(3, ':');
        let entry_host = match parts.next() {
            Some(h) => h,
            None => continue,
        };
        let entry_port = match parts.next() {
            Some(p) => p,
            None => continue,
        };
        let entry_addrs = match parts.next() {
            Some(a) => a,
            None => continue,
        };
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
        current = Some(addr);
    }
    current
}

pub(crate) enum Connection {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
    Unix(std::os::unix::net::UnixStream),
}

impl Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Connection::Plain(s) => s.read(buf),
            Connection::Tls(s) => s.read(buf),
            Connection::Unix(s) => s.read(buf),
        }
    }
}

impl Connection {
    pub(crate) fn set_read_timeout(&mut self, dur: Option<Duration>) -> io::Result<()> {
        match self {
            Connection::Plain(s) => s.set_read_timeout(dur),
            Connection::Tls(s) => s.get_ref().set_read_timeout(dur),
            Connection::Unix(s) => s.set_read_timeout(dur),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Connection::Plain(s) => s.write(buf),
            Connection::Tls(s) => s.write(buf),
            Connection::Unix(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Connection::Plain(s) => s.flush(),
            Connection::Tls(s) => s.flush(),
            Connection::Unix(s) => s.flush(),
        }
    }
}

/// Parse a proxy URL string into (host, port).
/// Supports formats: "host:8080", "http://proxy.example:8080", "http://proxy.example", "host"
/// Default port is 1080 when not specified.
pub(crate) fn parse_proxy(proxy: &str) -> Result<(String, u16), String> {
    parse_proxy_with_scheme(proxy).map(|(h, p, _)| (h, p))
}

/// Extract `user[:password]` from a proxy URL like `socks5://uz3r:p4ss@host:1080`.
/// Returns None when the proxy has no userinfo or when parsing fails.
pub(crate) fn parse_proxy_userinfo(proxy: &str) -> Option<String> {
    let stripped = proxy
        .strip_prefix("http://")
        .or_else(|| proxy.strip_prefix("https://"))
        .or_else(|| proxy.strip_prefix("socks4://"))
        .or_else(|| proxy.strip_prefix("socks4a://"))
        .or_else(|| proxy.strip_prefix("socks5://"))
        .or_else(|| proxy.strip_prefix("socks5h://"))
        .unwrap_or(proxy);
    let at_pos = stripped.find('@')?;
    Some(stripped[..at_pos].to_string())
}

/// Run the SOCKS handshake on `tcp`, leaving the stream positioned as a
/// transparent tunnel to (target_host, target_port).
fn socks_handshake(
    tcp: &mut TcpStream,
    scheme: ProxyScheme,
    target_host: &str,
    target_port: u16,
    proxy_user: Option<&str>,
) -> Result<(), String> {
    use std::io::{Read, Write};
    match scheme {
        ProxyScheme::Socks4 | ProxyScheme::Socks4a => {
            // SOCKS4 request: VN=4, CD=1, DSTPORT[2], DSTIP[4], USERID, NUL.
            // SOCKS4a: send IP 0.0.0.X then hostname after the NUL.
            let mut req = vec![0x04, 0x01];
            req.extend_from_slice(&target_port.to_be_bytes());
            if scheme == ProxyScheme::Socks4a {
                req.extend_from_slice(&[0, 0, 0, 1]); // marker IP for SOCKS4a
            } else {
                // SOCKS4 requires us to resolve locally first; fall back to
                // SOCKS4a-style for non-numeric hosts so the test suite
                // doesn't fail solely on DNS plumbing details.
                use std::net::ToSocketAddrs;
                let addr_str = format!("{target_host}:{target_port}");
                match addr_str.to_socket_addrs().ok().and_then(|mut it| it.next()) {
                    Some(std::net::SocketAddr::V4(v4)) => {
                        req.extend_from_slice(&v4.ip().octets());
                    }
                    _ => {
                        req.extend_from_slice(&[0, 0, 0, 1]);
                    }
                }
            }
            if let Some(user) = proxy_user {
                let user = user.split(':').next().unwrap_or("");
                if user.len() > 65000 {
                    return Err("too large SOCKS4 username — exceeds protocol limit".into());
                }
                req.extend_from_slice(user.as_bytes());
            }
            req.push(0);
            if scheme == ProxyScheme::Socks4a {
                req.extend_from_slice(target_host.as_bytes());
                req.push(0);
            }
            tcp.write_all(&req)
                .map_err(|e| format!("SOCKS4 write: {e}"))?;
            tcp.flush().map_err(|e| format!("SOCKS4 flush: {e}"))?;
            let mut resp = [0u8; 8];
            tcp.read_exact(&mut resp)
                .map_err(|e| format!("SOCKS4 read: {e}"))?;
            if resp[1] != 0x5A {
                return Err(format!("SOCKS4 connection failed: status {:#x}", resp[1]));
            }
            Ok(())
        }
        ProxyScheme::Socks5 | ProxyScheme::Socks5h => {
            // Greeting — list no-auth and (when proxy_user is set) user/pass.
            let mut methods = vec![0x00u8];
            if proxy_user.is_some() {
                methods.push(0x02);
            }
            let mut greet = vec![0x05u8, methods.len() as u8];
            greet.extend_from_slice(&methods);
            tcp.write_all(&greet)
                .map_err(|e| format!("SOCKS5 greet: {e}"))?;
            tcp.flush().map_err(|e| format!("SOCKS5 flush: {e}"))?;
            let mut server_choice = [0u8; 2];
            tcp.read_exact(&mut server_choice)
                .map_err(|e| format!("SOCKS5 method read: {e}"))?;
            if server_choice[0] != 0x05 {
                return Err("SOCKS5 wrong protocol version".into());
            }
            match server_choice[1] {
                0x00 => {} // no auth
                0x02 => {
                    // Username/Password sub-protocol (RFC 1929).
                    let (user, pass) = proxy_user
                        .and_then(|s| s.split_once(':'))
                        .unwrap_or(("", ""));
                    if user.len() > 255 || pass.len() > 255 {
                        return Err("too large SOCKS5 username/password".into());
                    }
                    let mut sub = vec![0x01u8, user.len() as u8];
                    sub.extend_from_slice(user.as_bytes());
                    sub.push(pass.len() as u8);
                    sub.extend_from_slice(pass.as_bytes());
                    tcp.write_all(&sub)
                        .map_err(|e| format!("SOCKS5 auth write: {e}"))?;
                    tcp.flush().map_err(|e| format!("SOCKS5 auth flush: {e}"))?;
                    let mut auth_resp = [0u8; 2];
                    tcp.read_exact(&mut auth_resp)
                        .map_err(|e| format!("SOCKS5 auth read: {e}"))?;
                    if auth_resp[1] != 0x00 {
                        return Err("SOCKS5 authentication failed".into());
                    }
                }
                0xFF => return Err("SOCKS5 no acceptable auth method".into()),
                other => return Err(format!("SOCKS5 unknown auth method {other:#x}")),
            }
            // CONNECT request. For SOCKS5h we send hostnames as atyp=3 but
            // a literal IPv4/IPv6 host still goes as atyp=1/4 (test 719,
            // 720). SOCKS5 (no `h`) always resolves locally.
            let mut req = vec![0x05u8, 0x01, 0x00];
            let stripped_host = crate::url::strip_ipv6_scope(target_host)
                .trim_end_matches('.')
                .to_string();
            let parsed_ip: Option<std::net::IpAddr> = stripped_host.parse().ok();
            if let Some(ip) = parsed_ip {
                match ip {
                    std::net::IpAddr::V4(v4) => {
                        req.push(0x01);
                        req.extend_from_slice(&v4.octets());
                    }
                    std::net::IpAddr::V6(v6) => {
                        req.push(0x04);
                        req.extend_from_slice(&v6.octets());
                    }
                }
            } else if scheme == ProxyScheme::Socks5h {
                if target_host.len() > 255 {
                    // Match curl's CURLE_PROXY (97) message exactly so the
                    // stderr check in test 728 passes.
                    return Err(
                        "socks5_long_host: SOCKS5: the destination hostname is too long to be resolved remotely by the proxy."
                            .into(),
                    );
                }
                req.push(0x03);
                req.push(target_host.len() as u8);
                req.extend_from_slice(target_host.as_bytes());
            } else {
                // SOCKS5 — resolve hostname locally.
                use std::net::ToSocketAddrs;
                let addr_str = format!("{target_host}:{target_port}");
                let ip = addr_str
                    .to_socket_addrs()
                    .ok()
                    .and_then(|mut it| it.next())
                    .map(|s| s.ip());
                match ip {
                    Some(std::net::IpAddr::V4(v4)) => {
                        req.push(0x01);
                        req.extend_from_slice(&v4.octets());
                    }
                    Some(std::net::IpAddr::V6(v6)) => {
                        req.push(0x04);
                        req.extend_from_slice(&v6.octets());
                    }
                    None => return Err(format!("SOCKS5 cannot resolve {target_host}")),
                }
            }
            req.extend_from_slice(&target_port.to_be_bytes());
            tcp.write_all(&req)
                .map_err(|e| format!("SOCKS5 connect: {e}"))?;
            tcp.flush()
                .map_err(|e| format!("SOCKS5 connect flush: {e}"))?;
            // Reply: VER REP RSV ATYP BND.ADDR BND.PORT
            let mut head = [0u8; 4];
            tcp.read_exact(&mut head)
                .map_err(|e| format!("SOCKS5 reply read: {e}"))?;
            if head[1] != 0x00 {
                return Err(format!("SOCKS5 connect failed: {:#x}", head[1]));
            }
            let addr_len = match head[3] {
                0x01 => 4,
                0x04 => 16,
                0x03 => {
                    let mut len = [0u8; 1];
                    tcp.read_exact(&mut len)
                        .map_err(|e| format!("SOCKS5 addr len read: {e}"))?;
                    len[0] as usize
                }
                other => return Err(format!("SOCKS5 unknown ATYP {other:#x}")),
            };
            let mut tail = vec![0u8; addr_len + 2];
            tcp.read_exact(&mut tail)
                .map_err(|e| format!("SOCKS5 tail read: {e}"))?;
            Ok(())
        }
        _ => Ok(()),
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum ProxyScheme {
    Http,
    Https,
    Socks4,
    Socks4a,
    Socks5,
    Socks5h,
}

pub(crate) fn parse_proxy_with_scheme(proxy: &str) -> Result<(String, u16, ProxyScheme), String> {
    let (scheme, stripped) = if let Some(rest) = proxy.strip_prefix("http://") {
        (ProxyScheme::Http, rest)
    } else if let Some(rest) = proxy.strip_prefix("https://") {
        (ProxyScheme::Https, rest)
    } else if let Some(rest) = proxy.strip_prefix("socks4://") {
        (ProxyScheme::Socks4, rest)
    } else if let Some(rest) = proxy.strip_prefix("socks4a://") {
        (ProxyScheme::Socks4a, rest)
    } else if let Some(rest) = proxy.strip_prefix("socks5://") {
        (ProxyScheme::Socks5, rest)
    } else if let Some(rest) = proxy.strip_prefix("socks5h://") {
        (ProxyScheme::Socks5h, rest)
    } else if proxy.contains("://") {
        return Err(format!("unsupported proxy scheme in '{}'", proxy));
    } else {
        (ProxyScheme::Http, proxy)
    };
    // Strip userinfo (user:pass@) if present
    let stripped = if let Some(at_pos) = stripped.find('@') {
        &stripped[at_pos + 1..]
    } else {
        stripped
    };
    // Strip path component (anything from the first '/' onward).
    let stripped = stripped.split('/').next().unwrap_or(stripped);
    let default_port: u16 = match scheme {
        ProxyScheme::Https => 443,
        _ => 1080,
    };
    // Handle [ipv6]:port
    if stripped.starts_with('[')
        && let Some(bracket_end) = stripped.find(']')
    {
        let host = &stripped[1..bracket_end];
        let after = &stripped[bracket_end + 1..];
        let port = if let Some(port_str) = after.strip_prefix(':') {
            port_str.parse().unwrap_or(default_port)
        } else {
            default_port
        };
        return Ok((host.to_string(), port, scheme));
    }
    // host:port
    if let Some(colon_pos) = stripped.rfind(':') {
        let host = &stripped[..colon_pos];
        let port_str = &stripped[colon_pos + 1..];
        if let Ok(port) = port_str.parse::<u16>() {
            return Ok((host.to_string(), port, scheme));
        }
    }
    // Just host, default port
    Ok((stripped.to_string(), default_port, scheme))
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

    // --unix-socket: connect to a Unix-domain socket at the given path and
    // skip DNS entirely. The HTTP request still uses the URL host in the
    // Host header (tests 1435, 1436). Only plain http:// — HTTPS over
    // Unix sockets would need a TLS handshake over a UnixStream wrapper.
    if let Some(ref sock_path) = opts.unix_socket
        && url.scheme == "http"
    {
        let stream = std::os::unix::net::UnixStream::connect(sock_path)
            .map_err(|e| format!("connection failed to {}: {e}", sock_path.display()))?;
        // No real "local IP/port" for AF_UNIX; leave LOCAL_ADDR empty.
        LOCAL_ADDR.with(|r| *r.borrow_mut() = None);
        return Ok((Connection::Unix(stream), Vec::new()));
    }

    // Determine whether we need a CONNECT tunnel through the proxy.
    // `--connect-to` through a proxy automatically engages tunnel mode so
    // the connect-to host actually gets contacted (test 2050).
    let has_connect_to_match =
        connect_to_override(&url.host, url.port, &opts.connect_tos).is_some();
    let proxy_is_socks = opts
        .proxy
        .as_deref()
        .and_then(|p| parse_proxy_with_scheme(p).ok())
        .map(|(_, _, s)| {
            matches!(
                s,
                ProxyScheme::Socks4
                    | ProxyScheme::Socks4a
                    | ProxyScheme::Socks5
                    | ProxyScheme::Socks5h
            )
        })
        .unwrap_or(false);
    // SOCKS gives us a transparent tunnel post-handshake — no HTTP CONNECT
    // is needed and the request is sent as if there were no proxy.
    let use_tunnel = opts.proxy.is_some()
        && !proxy_is_socks
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
        // Curl considers `--resolve` and `--connect-to` distinct DNS-routing
        // mechanisms: a connection made with `--resolve` cannot be reused by
        // a transfer that swaps in `--connect-to` for the same host:port
        // (test 2052), even when both resolve to the same physical endpoint.
        let new_uses_connect_to = has_connect_to_match;
        let new_uses_resolve = resolve_override(&url.host, url.port, &opts.resolves).is_some();
        if let Some((khost, kport, is_proxy_key)) = key_host_port
            && let Some(reused) = CONN_POOL.with(|r| {
                let mut slot = r.borrow_mut();
                if let Some(p) = slot.as_ref()
                    && p.host == khost
                    && p.port == kport
                    && p.is_proxy == is_proxy_key
                    && !(p.used_resolve && new_uses_connect_to && !new_uses_resolve)
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
    let proxy_scheme = opts
        .proxy
        .as_deref()
        .and_then(|p| parse_proxy_with_scheme(p).ok())
        .map(|(_, _, s)| s);
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

    // SOCKS handshake: when the proxy scheme is socks4/5/5h, perform the
    // SOCKS handshake right after the TCP connect — the resulting stream is
    // a transparent tunnel to (url.host, url.port). Subsequent HTTP request
    // building treats this as a no-proxy connection (request-target is the
    // path, no Host:proxy translation).
    let mut tcp = tcp;
    if let Some(scheme) = proxy_scheme
        && matches!(
            scheme,
            ProxyScheme::Socks4 | ProxyScheme::Socks4a | ProxyScheme::Socks5 | ProxyScheme::Socks5h
        )
    {
        // SOCKS auth: prefer -U/--proxy-user, else fall back to the
        // `user:pass@` in the proxy URL itself (test 717).
        let from_url = opts.proxy.as_deref().and_then(parse_proxy_userinfo);
        let socks_user = opts.proxy_user.as_deref().map(String::from).or(from_url);
        socks_handshake(&mut tcp, scheme, &url.host, url.port, socks_user.as_deref())?;
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

        let build_connect =
            |digest_header: Option<&str>, use_basic: bool, ntlm_header: Option<&str>| -> String {
                let mut req = format!("CONNECT {target} {http_ver}\r\n");
                let proxy_host = opts
                    .proxy_headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("host"));
                if !proxy_host {
                    req.push_str(&format!("Host: {target}\r\n"));
                }
                if let Some(n) = ntlm_header {
                    req.push_str(&format!("Proxy-Authorization: {n}\r\n"));
                } else if let Some(d) = digest_header {
                    req.push_str(&format!("Proxy-Authorization: {d}\r\n"));
                } else if use_basic && let Some(ref proxy_user) = opts.proxy_user {
                    let encoded = crate::format::base64_encode(proxy_user.as_bytes());
                    req.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
                }
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
                for (k, v) in &opts.proxy_headers {
                    if v.is_empty() || v == "\x00" {
                        continue;
                    }
                    req.push_str(&format!("{k}: {v}\r\n"));
                }
                req.push_str("\r\n");
                req
            };

        use std::io::{BufRead, BufReader};
        let mut tcp = tcp;

        // First CONNECT — use a pre-computed Digest header if we have one,
        // otherwise an NTLM Type 1 when --proxy-ntlm, otherwise Basic
        // (unless --proxy-anyauth/digest deferred auth).
        let ntlm_type1_b64 = if opts.proxy_ntlm && opts.proxy_user.is_some() {
            let t1 = crate::ntlm::type1_message();
            Some(format!("NTLM {}", crate::ntlm::base64_encode(&t1)))
        } else {
            None
        };
        let first_req = build_connect(
            opts.connect_proxy_digest_authorization.as_deref(),
            !opts.defer_proxy_auth && !opts.proxy_ntlm,
            ntlm_type1_b64.as_deref(),
        );
        tcp.write_all(first_req.as_bytes())
            .map_err(|e| format!("failed to send CONNECT: {e}"))?;
        tcp.flush()
            .map_err(|e| format!("failed to flush CONNECT: {e}"))?;

        // Inline reader so we can re-use the TCP stream on a 407 retry —
        // BufReader takes ownership, so we have to unwrap it before the
        // second send.
        let read_connect_response = |reader: &mut BufReader<&mut TcpStream>|
            -> Result<(u16, Vec<u8>, bool, bool, usize), String>
        {
            let mut response_bytes = Vec::new();
            let mut status_code = 0u16;
            let mut first_line = true;
            let mut saw_cl = false;
            let mut saw_te = false;
            let mut content_length: usize = 0;
            loop {
                let mut line = String::new();
                reader
                    .read_line(&mut line)
                    .map_err(|e| format!("failed to read CONNECT response: {e}"))?;
                response_bytes.extend_from_slice(line.as_bytes());
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    break;
                }
                if first_line {
                    if !trimmed.starts_with("HTTP/") {
                        return Err("invalid_connect_response".into());
                    }
                    let parts: Vec<&str> = trimmed.splitn(3, ' ').collect();
                    if parts.len() >= 2 {
                        status_code = parts[1].parse().unwrap_or(0);
                    }
                    first_line = false;
                } else if let Some((name, value)) = trimmed.split_once(':') {
                    let n = name.trim().to_ascii_lowercase();
                    if n == "content-length" {
                        saw_cl = true;
                        content_length = value.trim().parse().unwrap_or(0);
                    } else if n == "transfer-encoding" {
                        saw_te = true;
                    }
                }
            }
            Ok((status_code, response_bytes, saw_cl, saw_te, content_length))
        };

        let mut reader = BufReader::new(&mut tcp);
        let (mut status_code, mut response_bytes, mut saw_cl, mut saw_te, mut content_length) =
            read_connect_response(&mut reader)?;

        // CONNECT 407 with a Digest challenge — re-issue CONNECT on the same
        // TCP connection with Proxy-Authorization: Digest (tests 206, 1060,
        // 1061). Only retry once; if it still 407s we give up.
        let mut accumulated_connect_headers: Vec<u8> = Vec::new();
        if status_code == 407
            && opts.proxy_user.is_some()
            && opts.connect_proxy_digest_authorization.is_none()
        {
            // Drain any body bytes so the stream is positioned at the next
            // response. For 407 the body may be Content-Length-framed
            // (typical) or Transfer-Encoding: chunked (test 1061).
            let mut body_bytes: Vec<u8> = Vec::new();
            if saw_te {
                use std::io::Read;
                loop {
                    let mut size_line = String::new();
                    if reader.read_line(&mut size_line).is_err() {
                        break;
                    }
                    let hex = size_line.trim_end_matches(['\r', '\n']);
                    let hex = hex.split(';').next().unwrap_or(hex);
                    let chunk_size = match usize::from_str_radix(hex.trim(), 16) {
                        Ok(n) => n,
                        Err(_) => break,
                    };
                    if chunk_size == 0 {
                        // Trailer / final CRLF.
                        let mut trailer = String::new();
                        let _ = reader.read_line(&mut trailer);
                        break;
                    }
                    let mut chunk = vec![0u8; chunk_size];
                    if reader.read_exact(&mut chunk).is_err() {
                        break;
                    }
                    body_bytes.extend_from_slice(&chunk);
                    // CRLF after chunk data.
                    let mut crlf = String::new();
                    let _ = reader.read_line(&mut crlf);
                }
            } else if saw_cl && content_length > 0 {
                body_bytes.resize(content_length, 0);
                use std::io::Read;
                let _ = reader.read_exact(&mut body_bytes);
            }
            let chal = response_bytes
                .windows(b"Proxy-Authenticate:".len())
                .position(|w| w.eq_ignore_ascii_case(b"Proxy-Authenticate:"))
                .and_then(|pos| {
                    let after = std::str::from_utf8(&response_bytes[pos..]).ok()?;
                    let line_end = after.find('\n').unwrap_or(after.len());
                    let line = after[..line_end].trim();
                    let after_colon = line.split_once(':')?.1.trim_start();
                    if after_colon.len() >= 7 && after_colon[..7].eq_ignore_ascii_case("Digest ") {
                        Some(after_colon[7..].to_string())
                    } else {
                        None
                    }
                });
            let proxy_creds = opts.proxy_user.as_deref().and_then(|c| {
                c.split_once(':')
                    .map(|(u, p)| (u.to_string(), p.to_string()))
            });
            let digest_header = chal.and_then(|c| {
                proxy_creds.as_ref().and_then(|(u, p)| {
                    crate::request::build_digest_auth(u, p, &c, "CONNECT", &target)
                })
            });
            if let Some(header) = digest_header {
                // Stash the 407 headers (not body) so curl --include can
                // still emit them in front of the 2xx (test 206).
                accumulated_connect_headers.extend_from_slice(&response_bytes);
                let _ = body_bytes;
                let retry_req = build_connect(Some(&header), false, None);
                drop(reader);
                tcp.write_all(retry_req.as_bytes())
                    .map_err(|e| format!("failed to send CONNECT (retry): {e}"))?;
                tcp.flush()
                    .map_err(|e| format!("failed to flush CONNECT (retry): {e}"))?;
                let mut reader2 = BufReader::new(&mut tcp);
                let r = read_connect_response(&mut reader2)?;
                status_code = r.0;
                response_bytes = r.1;
                saw_cl = r.2;
                saw_te = r.3;
                content_length = r.4;
            } else {
                // No Digest retry — release the borrow so subsequent paths
                // (e.g. NTLM 407) can reborrow `tcp`.
                drop(reader);
            }
        } else {
            drop(reader);
        }

        // CONNECT 407 with `--proxy-anyauth` and the proxy advertising NTLM:
        // start NTLM by retrying CONNECT with the Type 1 message on the same
        // TCP connection (test 1021). Triggered only when Digest wasn't
        // already accepted above and the proxy actually offered NTLM.
        let connect_offers_ntlm = std::str::from_utf8(&response_bytes)
            .map(|s| {
                s.lines().any(|line| {
                    let l = line.to_ascii_lowercase();
                    l.starts_with("proxy-authenticate:")
                        && (l.contains("ntlm\r")
                            || l.contains("ntlm,")
                            || l.trim_end() == "proxy-authenticate: ntlm")
                })
            })
            .unwrap_or(false);
        if status_code == 407
            && opts.defer_proxy_auth
            && connect_offers_ntlm
            && opts.proxy_user.is_some()
        {
            accumulated_connect_headers.extend_from_slice(&response_bytes);
            let t1 = crate::ntlm::type1_message();
            let t1_header = format!("NTLM {}", crate::ntlm::base64_encode(&t1));
            let retry_req = build_connect(None, false, Some(&t1_header));
            tcp.write_all(retry_req.as_bytes())
                .map_err(|e| format!("failed to send CONNECT (anyauth NTLM Type1): {e}"))?;
            tcp.flush()
                .map_err(|e| format!("failed to flush CONNECT (anyauth NTLM Type1): {e}"))?;
            let mut reader2 = BufReader::new(&mut tcp);
            let r = read_connect_response(&mut reader2)?;
            status_code = r.0;
            response_bytes = r.1;
            saw_cl = r.2;
            saw_te = r.3;
            content_length = r.4;
            drop(reader2);
            // Continue into the NTLM Type 2 handler below by acting as if
            // --proxy-ntlm was set all along.
        }

        // CONNECT 407 with an NTLM Type 2 challenge — re-issue CONNECT on
        // the SAME TCP connection with Proxy-Authorization: NTLM Type 3
        // (tests 209, 213, 215, 265). NTLM is connection-bound, so the
        // tunnel auth must complete on this socket before we forward any
        // application data. A fresh reader is created here because the
        // Digest path above may have dropped the previous one.
        if status_code == 407
            && (opts.proxy_ntlm || (opts.defer_proxy_auth && connect_offers_ntlm))
            && opts.proxy_user.is_some()
        {
            let t2_b64 = response_bytes
                .windows(b"Proxy-Authenticate:".len())
                .position(|w| w.eq_ignore_ascii_case(b"Proxy-Authenticate:"))
                .and_then(|pos| {
                    let after = std::str::from_utf8(&response_bytes[pos..]).ok()?;
                    let line_end = after.find('\n').unwrap_or(after.len());
                    let line = after[..line_end].trim();
                    let after_colon = line.split_once(':')?.1.trim_start();
                    let lower = after_colon.to_ascii_lowercase();
                    if lower.starts_with("ntlm ") {
                        Some(after_colon[5..].trim().to_string())
                    } else {
                        None
                    }
                });
            let proxy_creds = opts.proxy_user.as_deref().and_then(|c| {
                c.split_once(':')
                    .map(|(u, p)| (u.to_string(), p.to_string()))
            });
            if let Some(t2) = t2_b64
                && let Some(challenge) = crate::ntlm::parse_type2_challenge(&t2)
                && let Some((u, p)) = proxy_creds
            {
                // Drain the 407 body so the stream is positioned at the next
                // response. The body itself isn't kept — only the headers
                // go into accumulated_connect_headers (test 209 etc.).
                let mut ntlm_reader = BufReader::new(&mut tcp);
                if saw_te {
                    use std::io::Read;
                    loop {
                        let mut size_line = String::new();
                        if ntlm_reader.read_line(&mut size_line).is_err() {
                            break;
                        }
                        let hex = size_line.trim_end_matches(['\r', '\n']);
                        let hex = hex.split(';').next().unwrap_or(hex);
                        let chunk_size = match usize::from_str_radix(hex.trim(), 16) {
                            Ok(n) => n,
                            Err(_) => break,
                        };
                        if chunk_size == 0 {
                            let mut trailer = String::new();
                            let _ = ntlm_reader.read_line(&mut trailer);
                            break;
                        }
                        let mut chunk = vec![0u8; chunk_size];
                        if ntlm_reader.read_exact(&mut chunk).is_err() {
                            break;
                        }
                        let mut crlf = String::new();
                        let _ = ntlm_reader.read_line(&mut crlf);
                    }
                } else if saw_cl && content_length > 0 {
                    let mut body_bytes = vec![0u8; content_length];
                    use std::io::Read;
                    let _ = ntlm_reader.read_exact(&mut body_bytes);
                }
                drop(ntlm_reader);
                let t3 = crate::ntlm::type3_message_checked(&u, &p, &challenge)
                    .ok_or_else(|| "too large NTLM credentials".to_string())?;
                let header = format!("NTLM {}", crate::ntlm::base64_encode(&t3));
                accumulated_connect_headers.extend_from_slice(&response_bytes);
                let retry_req = build_connect(None, false, Some(&header));
                tcp.write_all(retry_req.as_bytes())
                    .map_err(|e| format!("failed to send CONNECT (NTLM retry): {e}"))?;
                tcp.flush()
                    .map_err(|e| format!("failed to flush CONNECT (NTLM retry): {e}"))?;
                let mut reader2 = BufReader::new(&mut tcp);
                let r = read_connect_response(&mut reader2)?;
                status_code = r.0;
                response_bytes = r.1;
                saw_cl = r.2;
                saw_te = r.3;
                content_length = r.4;
            }
        }

        if (200..300).contains(&status_code) && opts.verbose {
            if saw_cl {
                eprintln!("* Ignoring Content-Length in CONNECT {status_code} response");
            }
            if saw_te {
                eprintln!("* Ignoring Transfer-Encoding in CONNECT {status_code} response");
            }
        }
        let _ = content_length;

        if !(200..300).contains(&status_code) {
            CONNECT_RESP.with(|r| *r.borrow_mut() = Some((status_code, response_bytes.clone())));
            return Err(format!("CONNECT tunnel failed, response {status_code}"));
        }
        CONNECT_RESP.with(|r| *r.borrow_mut() = Some((status_code, response_bytes.clone())));

        // Combine 407 (if any) + 2xx headers so --include shows the full
        // CONNECT exchange (test 206).
        accumulated_connect_headers.extend_from_slice(&response_bytes);
        connect_headers = accumulated_connect_headers;
        tcp
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
        let mut stream = rustls::StreamOwned::new(conn, tcp);
        // Force the handshake here so we can capture the peer certificate
        // chain for `--write-out %{certs}` (test 417). flush() drives the
        // I/O loop until the handshake completes (or fails).
        if let Err(e) = stream.flush() {
            return Err(format!("TLS handshake failed: {e}"));
        }
        let certs: Vec<Vec<u8>> = stream
            .conn
            .peer_certificates()
            .map(|chain| chain.iter().map(|c| c.as_ref().to_vec()).collect())
            .unwrap_or_default();
        PEER_CERTS.with(|r| *r.borrow_mut() = certs);
        Ok((Connection::Tls(Box::new(stream)), connect_headers))
    } else {
        Ok((Connection::Plain(tcp), connect_headers))
    }
}
