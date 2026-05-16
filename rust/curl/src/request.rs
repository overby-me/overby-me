use std::fs;
use std::io::{self, Read, Write};
use std::time::Duration;

use md5::{Digest, Md5};
use sha2::{Sha256, Sha512_256};

use crate::connection::connect;
use crate::format::base64_encode;
use crate::options::Options;
use crate::response::{Response, read_response};
use crate::url::{ParsedUrl, normalize_url_path, parse_url};

fn quote_digest_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn md5_hex(input: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    let mut s = String::with_capacity(32);
    for b in result.iter() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    let mut s = String::with_capacity(64);
    for b in result.iter() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn sha512_256_hex(input: &str) -> String {
    let mut hasher = Sha512_256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    let mut s = String::with_capacity(64);
    for b in result.iter() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Parse a single attribute value from a Digest WWW-Authenticate parameter list:
/// keys may be unquoted (`algorithm=MD5`) or quoted (`realm="ream"`); names are
/// matched case-insensitively. Duplicate keys: returns the LAST occurrence
/// (curl behaviour — test 1437).
fn parse_digest_attr(challenge: &str, name: &str) -> Option<String> {
    let lc = challenge.to_lowercase();
    let needle = format!("{}=", name.to_lowercase());
    let mut idx = 0;
    let mut last_value: Option<String> = None;
    while let Some(pos) = lc[idx..].find(&needle) {
        let abs = idx + pos;
        // ensure preceding char is start-of-string, comma, space, or 'Digest '
        if abs > 0 {
            let prev = challenge.as_bytes()[abs - 1];
            if !matches!(prev, b',' | b' ' | b'\t') {
                idx = abs + needle.len();
                continue;
            }
        }
        let after = &challenge[abs + needle.len()..];
        let value = if let Some(rest) = after.strip_prefix('"') {
            // Quoted string: unescape `\X` → `X` (HTTP quoted-string rules).
            // Stop at the first UNESCAPED `"`.
            let mut unescaped = String::new();
            let mut chars = rest.chars();
            let mut closed = false;
            while let Some(c) = chars.next() {
                if c == '\\' {
                    if let Some(next) = chars.next() {
                        unescaped.push(next);
                    }
                } else if c == '"' {
                    closed = true;
                    break;
                } else {
                    unescaped.push(c);
                }
            }
            if !closed {
                return last_value;
            }
            unescaped
        } else {
            let end = after
                .find(|c: char| c == ',' || c.is_ascii_whitespace())
                .unwrap_or(after.len());
            after[..end].to_string()
        };
        last_value = Some(value);
        idx = abs + needle.len();
    }
    last_value
}

/// Compute the RFC 2617 Digest "response" value (basic mode, no qop=auth-int).
///
/// `HA1 = MD5(user:realm:pass)`, `HA2 = MD5(method:uri)`,
/// `response = MD5(HA1:nonce:HA2)`. The qop="auth" variant adds nc and cnonce
/// to the response computation.
fn parse_connect_proxy_digest_challenge(response_bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(response_bytes).ok()?;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.len() < 19 {
            continue;
        }
        if line[..18].eq_ignore_ascii_case("Proxy-Authenticate") && line.as_bytes()[18] == b':' {
            let after = line[19..].trim_start();
            if after.len() >= 7 && after[..7].eq_ignore_ascii_case("Digest ") {
                return Some(after[7..].to_string());
            }
        }
    }
    None
}

pub(crate) fn build_digest_auth(
    user: &str,
    pass: &str,
    challenge: &str,
    method: &str,
    uri: &str,
) -> Option<String> {
    build_digest_auth_nc(user, pass, challenge, method, uri, 1)
}

pub(crate) fn build_digest_auth_nc(
    user: &str,
    pass: &str,
    challenge: &str,
    method: &str,
    uri: &str,
    nc: u32,
) -> Option<String> {
    let realm = parse_digest_attr(challenge, "realm")?;
    let nonce = parse_digest_attr(challenge, "nonce")?;
    let raw_algorithm = parse_digest_attr(challenge, "algorithm");
    let algorithm = raw_algorithm
        .as_deref()
        .unwrap_or("MD5")
        .to_uppercase();
    let userhash = parse_digest_attr(challenge, "userhash")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let hash: fn(&str) -> String = match algorithm.as_str() {
        "MD5" | "MD5-SESS" => md5_hex,
        "SHA-256" | "SHA-256-SESS" => sha256_hex,
        "SHA-512-256" | "SHA-512-256-SESS" => sha512_256_hex,
        _ => return None,
    };
    let qop_raw = parse_digest_attr(challenge, "qop");
    let ha1 = hash(&format!("{user}:{realm}:{pass}"));
    let ha2 = hash(&format!("{method}:{uri}"));
    let realm_q = quote_digest_value(&realm);
    let nonce_q = quote_digest_value(&nonce);
    // userhash=true: send H(user:realm) as the username (RFC 7616 §3.4.4).
    let username_out = if userhash {
        hash(&format!("{user}:{realm}"))
    } else {
        quote_digest_value(user)
    };
    let mut header = format!(
        r#"Digest username="{username_out}", realm="{realm_q}", nonce="{nonce_q}", uri="{uri}""#
    );
    if let Some(qop_val) = qop_raw.as_deref()
        && qop_val
            .split(',')
            .any(|q| q.trim().eq_ignore_ascii_case("auth"))
    {
        let cnonce = "0a4f113b";
        let nc_s = format!("{nc:08x}");
        let response = hash(&format!("{ha1}:{nonce}:{nc_s}:{cnonce}:auth:{ha2}"));
        header.push_str(&format!(
            r#", cnonce="{cnonce}", nc={nc_s}, qop=auth, response="{response}""#
        ));
    } else {
        let response = hash(&format!("{ha1}:{nonce}:{ha2}"));
        header.push_str(&format!(r#", response="{response}""#));
    }
    if let Some(opaque) = parse_digest_attr(challenge, "opaque") {
        header.push_str(&format!(r#", opaque="{opaque}""#));
    }
    // Echo algorithm only if the server sent one (curl matches this — when the
    // server defaults to MD5 implicitly, we don't add algorithm to the reply).
    if raw_algorithm.is_some() {
        header.push_str(&format!(", algorithm={algorithm}"));
    }
    if userhash {
        header.push_str(", userhash=true");
    }
    Some(header)
}

fn build_request(url: &ParsedUrl, opts: &Options) -> (Vec<u8>, Option<Vec<u8>>) {
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

    // SOCKS proxies leave a transparent tunnel post-handshake — the HTTP
    // layer treats them as a no-proxy direct connection (no absolute URL on
    // the request line, no Proxy-Authorization, no Proxy-Connection).
    let proxy_is_socks_p = opts
        .proxy
        .as_deref()
        .and_then(|p| crate::connection::parse_proxy_with_scheme(p).ok())
        .map(|(_, _, s)| {
            matches!(
                s,
                crate::connection::ProxyScheme::Socks4
                    | crate::connection::ProxyScheme::Socks4a
                    | crate::connection::ProxyScheme::Socks5
                    | crate::connection::ProxyScheme::Socks5h
            )
        })
        .unwrap_or(false);

    // Pool downgrade: when reusing a connection whose prior response was
    // HTTP/1.0, downgrade this request to HTTP/1.0 too (test 1074). The
    // `POOL_REUSED_HTTP10` flag is set by `connect()` immediately before
    // `build_request` runs.
    let pool_is_http10 = crate::connection::POOL_REUSED_HTTP10.with(|h| *h.borrow());
    let http_ver = match opts.http_version.as_deref() {
        Some("1.0") => "HTTP/1.0",
        _ if pool_is_http10 => "HTTP/1.0",
        _ => "HTTP/1.1",
    };

    // --request-target overrides the path portion of the request line entirely.
    if let Some(ref rt) = opts.request_target {
        let mut req = format!("{method} {rt} {http_ver}\r\n");
        // Still need Host header, User-Agent, custom headers, body.
        let custom_host = opts
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("host"))
            .map(|(_, v)| v.clone());
        let default_port = if url.scheme == "https" { 443 } else { 80 };
        match custom_host.as_deref() {
            Some("\x00") => req.push_str("Host:\r\n"),
            Some("") => {}
            Some(h) => req.push_str(&format!("Host: {h}\r\n")),
            None => {
                let bare_host = crate::url::strip_ipv6_scope(&url.host);
                let host_for_header: String = if bare_host.contains(':') {
                    format!("[{bare_host}]")
                } else {
                    bare_host
                };
                if url.port == default_port {
                    req.push_str(&format!("Host: {host_for_header}\r\n"));
                } else {
                    req.push_str(&format!("Host: {host_for_header}:{}\r\n", url.port));
                }
            }
        }
        if !opts
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
        {
            let ua = opts.user_agent.as_deref().unwrap_or("curl/8.0.0");
            req.push_str(&format!("User-Agent: {ua}\r\n"));
        }
        if !opts
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("accept"))
        {
            req.push_str("Accept: */*\r\n");
        }
        // Proxy-Connection (matches non-request-target path).
        if opts.proxy.is_some()
            && url.scheme == "http"
            && !opts.proxy_tunnel
            && !opts.no_keepalive
            && !opts
                .headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("proxy-connection"))
        {
            req.push_str("Proxy-Connection: Keep-Alive\r\n");
        }
        for (key, val) in &opts.headers {
            if key.eq_ignore_ascii_case("host") {
                continue;
            }
            if val.is_empty() {
                continue;
            }
            if val == "\x00" {
                req.push_str(&format!("{key}:\r\n"));
            } else {
                req.push_str(&format!("{key}: {val}\r\n"));
            }
        }
        req.push_str("\r\n");
        return (req.into_bytes(), None);
    }

    // When uploading with -T and the URL path ends with '/', append the basename of the
    // uploaded file (with URL-unsafe characters percent-encoded) — matches curl behavior.
    let mut path = url.path.clone();
    if let Some(ref upload) = opts.upload_file
        && path.ends_with('/')
        && upload.to_str() != Some("-")
        && upload.to_str() != Some(".")
        && let Some(name) = upload.file_name().and_then(|n| n.to_str())
    {
        path.push_str(&encode_path_component(name));
    }
    // Percent-encode high-bit bytes (e.g. raw UTF-8 received in a Location
    // header) so the request line stays ASCII-clean.
    let path = pct_encode_high(&path);

    // When going through an HTTP proxy, use the full absolute URL in the
    // request line. `--connect-to` to a different host through a proxy
    // engages tunnel mode automatically (test 2050) — the request that
    // travels through the tunnel uses the relative path, not the absolute URL.
    let connect_to_tunnel = opts.proxy.is_some()
        && !opts.connect_tos.is_empty()
        && crate::connection::connect_to_override(&url.host, url.port, &opts.connect_tos).is_some();
    let request_target =
        if opts.proxy.is_some()
            && url.scheme == "http"
            && !opts.proxy_tunnel
            && !connect_to_tunnel
            && !proxy_is_socks_p
        {
            let default_port: u16 = 80;
            if url.port == default_port {
                format!("http://{}{path}", url.host)
            } else {
                format!("http://{}:{}{path}", url.host, url.port)
            }
        } else {
            path.clone()
        };
    let mut req = format!("{method} {request_target} {http_ver}\r\n");

    // Host header — prefer custom Host: from -H if provided.
    let custom_host = opts
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("host"))
        .map(|(_, v)| v.clone());
    let default_port = if url.scheme == "https" { 443 } else { 80 };
    match custom_host.as_deref() {
        // "\x00" marker (from -H "Host;") → send "Host:" with no value.
        Some("\x00") => req.push_str("Host:\r\n"),
        // Empty string (from -H "Host:") → suppress entirely.
        Some("") => {}
        Some(h) => req.push_str(&format!("Host: {h}\r\n")),
        None => {
            let bare_host = crate::url::strip_ipv6_scope(&url.host);
            let host_for_header: String = if bare_host.contains(':') {
                format!("[{bare_host}]")
            } else {
                bare_host
            };
            if url.port == default_port {
                req.push_str(&format!("Host: {host_for_header}\r\n"));
            } else {
                req.push_str(&format!("Host: {host_for_header}:{}\r\n", url.port));
            }
        }
    }

    // Proxy-Authorization — sent when --proxy-user is set and we're going through a proxy.
    // curl sends Proxy-Authorization before site Authorization. With
    // --proxy-anyauth (and friends) curl waits for a 407 challenge before
    // sending Proxy-Authorization (test 1331). SOCKS does its own auth
    // during the handshake, so no Proxy-Authorization is sent on the wire.
    if proxy_is_socks_p {
        // Skip all Proxy-Authorization paths for SOCKS.
    } else if let Some(proxy_digest) = opts.proxy_digest_authorization.as_deref() {
        req.push_str(&format!("Proxy-Authorization: {proxy_digest}\r\n"));
    } else if let Some(proxy_ntlm_hdr) = opts.proxy_ntlm_authorization.as_deref() {
        // --proxy-ntlm follow-up: Type 3 stashed by the 407 handler.
        req.push_str(&format!("Proxy-Authorization: {proxy_ntlm_hdr}\r\n"));
    } else if opts.proxy_ntlm
        && !opts.proxy_ntlm_done
        && opts.proxy.is_some()
        && !opts.proxy_tunnel
        && opts.proxy_user.is_some()
    {
        // --proxy-ntlm initial: send Type 1 to elicit the 407 + Type 2.
        // Skip once the connection has already been authenticated (test 169).
        let t1 = crate::ntlm::type1_message();
        let b64 = crate::ntlm::base64_encode(&t1);
        req.push_str(&format!("Proxy-Authorization: NTLM {b64}\r\n"));
    } else if let Some(ref proxy_user) = opts.proxy_user
        && opts.proxy.is_some()
        && !opts.proxy_tunnel
        && !opts.defer_proxy_auth
        && !opts.proxy_ntlm
    {
        // Skip Basic when --proxy-ntlm picked a different scheme — after the
        // NTLM handshake the connection is authenticated and no header is
        // needed (test 169).
        let encoded = base64_encode(proxy_user.as_bytes());
        req.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
    }

    // Basic auth — curl sends Authorization right after Host.
    // Prefer -u / --user; otherwise fall back to userinfo from the URL
    // (e.g. http://user:pass@host.example/path). Skip entirely when --anyauth/--digest/
    // --ntlm is set (curl waits for server challenge).
    let auth_user = opts.user.as_deref().or(url.userinfo.as_deref());
    let digest_header_from_state = opts
        .digest_challenge_state
        .as_deref()
        .and_then(|chal| {
            auth_user
                .and_then(|c| c.split_once(':').map(|(u, p)| (u.to_string(), p.to_string())))
                .and_then(|(u, p)| {
                    let uri = if url.path.is_empty() {
                        "/".into()
                    } else {
                        url.path.clone()
                    };
                    build_digest_auth_nc(&u, &p, chal, &method, &uri, opts.digest_nc.max(1))
                })
        });
    if let Some(ntlm_header) = opts.ntlm_authorization.as_deref() {
        // --ntlm follow-up: the 401 handler computed the Type 3 response and
        // stashed it here. Replaces the Type 1 we sent on the probe.
        req.push_str(&format!("Authorization: {ntlm_header}\r\n"));
    } else if opts.ntlm && !opts.ntlm_done && (opts.user.is_some() || url.userinfo.is_some()) {
        // --ntlm initial: send Type 1 to elicit the Type 2 challenge.
        // Skipped once the connection has already been authenticated
        // (test 1100 — redirect on a NTLM-authenticated connection has no
        // Authorization header).
        let t1 = crate::ntlm::type1_message();
        let b64 = crate::ntlm::base64_encode(&t1);
        req.push_str(&format!("Authorization: NTLM {b64}\r\n"));
    } else if let Some(digest_header) = opts.digest_authorization.as_deref() {
        // --digest follow-up: the 401 handler in perform() computed the full
        // Digest challenge response and stashed it here.
        req.push_str(&format!("Authorization: {digest_header}\r\n"));
    } else if let Some(ref hdr) = digest_header_from_state {
        req.push_str(&format!("Authorization: {hdr}\r\n"));
    } else if let Some(token) = opts.oauth2_bearer.as_deref() {
        // --oauth2-bearer takes precedence over -u/userinfo Basic auth and
        // adds an `Authorization: Bearer <token>` header. On cross-host
        // redirects the bearer is dropped along with regular Authorization
        // (test 778).
        req.push_str(&format!("Authorization: Bearer {token}\r\n"));
    } else if let Some(user) = auth_user
        && !opts.defer_auth
        && !opts.no_basic
        && !opts.ntlm
    {
        // Basic suppressed when NTLM is the chosen scheme — NTLM Type 1
        // would have been sent above, and after auth completes the
        // connection no longer needs an Authorization header (test 1100).
        let encoded = base64_encode(user.as_bytes());
        req.push_str(&format!("Authorization: Basic {encoded}\r\n"));
    }

    // Range / Content-Range — curl sends these early, before User-Agent.
    let is_upload = opts.upload_file.is_some();
    if let Some(ref range) = opts.range {
        // Append "-" if the range has no dash (e.g. "-r 4" → "Range: bytes=4-").
        let range_suffix = if range.contains('-') { "" } else { "-" };
        req.push_str(&format!("Range: bytes={range}{range_suffix}\r\n"));
    } else if let Some(ref resume) = opts.resume_from {
        // `-C -` for a download: re-stat the output file each request so a
        // retry picks up the partial content from the previous attempt
        // (test 3035). For uploads `-C -` was already resolved to "0" earlier.
        let effective_resume: Option<String> = if resume == "-" {
            opts.outputs
                .first()
                .and_then(|p| fs::metadata(p).ok())
                .map(|m| m.len().to_string())
        } else {
            Some(resume.clone())
        };
        if let Some(resume) = effective_resume {
            if is_upload {
                // PUT with -C N: send Content-Range: bytes N-END/TOTAL
                if let Some(ref path) = opts.upload_file
                    && let Ok(meta) = fs::metadata(path)
                {
                    let total = meta.len();
                    let start: u64 = resume.parse().unwrap_or(0);
                    if start < total {
                        let end = total - 1;
                        req.push_str(&format!(
                            "Content-Range: bytes {start}-{end}/{total}\r\n"
                        ));
                    }
                }
            } else if let Ok(off) = resume.parse::<u64>()
                && off > 0
            {
                // GET with -C N: send Range: bytes=N- (skip when N=0).
                req.push_str(&format!("Range: bytes={off}-\r\n"));
            }
        }
    }

    // For HTTP-via-proxy (no CONNECT tunnel), --proxy-header values are sent
    // on the same request as -H, so check both lists when deciding whether to
    // emit a default header (e.g. Proxy-Connection).
    let proxy_headers_active =
        opts.proxy.is_some() && url.scheme == "http" && !opts.proxy_tunnel && !proxy_is_socks_p;
    let has_custom = |name: &str| {
        let in_headers = opts
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case(name));
        let in_proxy = proxy_headers_active
            && opts
                .proxy_headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case(name));
        in_headers || in_proxy
    };

    // User-Agent. An explicit empty string (-A "") suppresses the header,
    // matching curl's behaviour.
    if !has_custom("user-agent") {
        let ua = opts.user_agent.as_deref().unwrap_or("curl/8.0.0");
        if !ua.is_empty() {
            req.push_str(&format!("User-Agent: {ua}\r\n"));
        }
    }
    if !has_custom("accept") {
        req.push_str("Accept: */*\r\n");
    }

    // Alt-Used: emitted when a loaded --alt-svc cache entry rerouted this
    // request's TCP target. The header carries the alt host:port so the
    // server can detect the routing (test 412).
    if let Some(ref alt) = opts.alt_used {
        req.push_str(&format!("Alt-Used: {alt}\r\n"));
    }

    // Proxy-Connection header — curl sends this for HTTP proxy requests.
    // Suppressed for tunneled HTTP-via-CONNECT (test 2050) since the
    // request travels through the tunnel, not directly to the proxy.
    // A user-supplied -H "Proxy-Connection: ..." replaces ours (emitted later).
    if opts.proxy.is_some()
        && url.scheme == "http"
        && !opts.proxy_tunnel
        && !connect_to_tunnel
        && !opts.no_keepalive
        && !proxy_is_socks_p
        && !has_custom("proxy-connection")
    {
        req.push_str("Proxy-Connection: Keep-Alive\r\n");
    }

    // Connection header — only send "close" when explicitly requested.
    // Real curl defaults to keep-alive (implicit in HTTP/1.1).
    if opts.no_keepalive {
        req.push_str("Connection: close\r\n");
    }

    // --tr-encoding: announce we accept gzip Transfer-Encoding via TE.
    // Curl emits the TE header *before* Accept-Encoding (test 1277). The
    // matching Connection: TE comes after Accept-Encoding.
    if opts.tr_encoding {
        req.push_str("TE: gzip\r\n");
    }

    // Accept-Encoding.
    if opts.compressed {
        req.push_str("Accept-Encoding: gzip, deflate, br, zstd\r\n");
    }

    // --tr-encoding: emit Connection: TE after Accept-Encoding. If the user
    // supplied a `-H "Connection: ..."` header we append "TE" to the *first*
    // user value rather than emitting a separate Connection header (test 1125).
    if opts.tr_encoding {
        let user_conn = opts.headers.iter().position(|(k, v)| {
            k.eq_ignore_ascii_case("connection") && !v.is_empty() && v != "\x00"
        });
        if user_conn.is_none() {
            req.push_str("Connection: TE\r\n");
        }
        // The user's Connection header (if any) is rewritten in the user-header
        // emission loop below so we don't duplicate it here.
    }

    // --etag-compare: send If-None-Match with the etag read from FILE.
    // If the file is missing or empty, curl still sends the header with an
    // empty quoted value (`""`) — downstream servers treat that as "no etag".
    if let Some(ref path) = opts.etag_compare {
        let etag = fs::read_to_string(path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "\"\"".to_string());
        req.push_str(&format!("If-None-Match: {etag}\r\n"));
    }

    // --time-cond / -z: If-Modified-Since or If-Unmodified-Since
    if let Some(ref tc) = opts.time_cond {
        match tc {
            crate::options::TimeCond::IfModifiedSince(ts) => {
                req.push_str(&format!(
                    "If-Modified-Since: {}\r\n",
                    crate::args::format_http_date(*ts)
                ));
            }
            crate::options::TimeCond::IfUnmodifiedSince(ts) => {
                req.push_str(&format!(
                    "If-Unmodified-Since: {}\r\n",
                    crate::args::format_http_date(*ts)
                ));
            }
        }
    }

    // Referer.
    if let Some(ref referer) = opts.referer {
        req.push_str(&format!("Referer: {referer}\r\n"));
    }

    // Cookies — each -b is either an inline "name=value; ..." or a cookie file.
    // Use the custom Host header's hostname (if set) for domain matching, so
    // cookies are sent based on the logical host, not the IP used to connect.
    let cookie_match_host = custom_host
        .as_deref()
        .map(|h| h.split(':').next().unwrap_or(h))
        .unwrap_or(&url.host);
    // Strip query/fragment from the URL path before cookie path matching:
    // RFC 6265 §5.1.4 says cookie paths apply to the path component only, so
    // a request to /a?q=1 should still match a cookie set on /a (test 1258).
    let cookie_request_path = url
        .path
        .split_once('?')
        .map(|(p, _)| p)
        .unwrap_or(&url.path);
    // The secure-cookie loopback exception uses the logical request host
    // (`-H "Host:"` override when present), not the connect IP. Otherwise an
    // HTTP request to www.example.com via a 127.0.0.1 connect would pick up
    // secure cookies that belong only on HTTPS (test 1561).
    let cookie_header = build_cookie_header(
        cookie_match_host,
        cookie_request_path,
        url.scheme == "https" || is_loopback_host(cookie_match_host),
        opts,
    );
    if !cookie_header.is_empty() {
        req.push_str(&format!("Cookie: {cookie_header}\r\n"));
    }

    // Chunked request encoding: either the user set `Transfer-Encoding:
    // chunked` explicitly, or we inferred it because the body length is
    // unknown ahead of time (stdin upload via `-T -` on HTTP/1.1).
    let is_stdin_upload = matches!(
        opts.upload_file.as_deref().and_then(|p| p.to_str()),
        Some("-" | ".")
    );
    let http10 = opts.http_version.as_deref() == Some("1.0");
    let user_chunked = opts.headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked")
    });
    // If the user explicitly suppressed Transfer-Encoding (via -H "Transfer-Encoding:"),
    // don't auto-chunk even for stdin uploads.
    let user_suppressed_te = opts
        .headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.is_empty());
    let auto_chunked = is_stdin_upload && !http10 && !user_chunked && !user_suppressed_te;
    let user_chunked = user_chunked || auto_chunked;
    if auto_chunked {
        req.push_str("Transfer-Encoding: chunked\r\n");
    }

    // Generate multipart boundary up-front so we can append it to a user-supplied
    // Content-Type header (matches curl's RFC 1867 behavior).
    let boundary = if !opts.form_fields.is_empty() {
        Some(multipart_boundary())
    } else {
        None
    };

    // Custom headers (may override defaults).
    // - Empty value (e.g. -H "X-Header:") removes/suppresses the header entirely.
    // - "\x00" marker (from -H "X-Header;") sends header with no value.
    // - Normal value sends "Key: value\r\n".
    // Skip the Host header here — it was already emitted above.
    // For HTTP-via-proxy (no CONNECT), --proxy-header entries are sent on the
    // same request as -H so include them after the regular -H list.
    let user_header_iter = opts.headers.iter().chain(if proxy_headers_active {
        opts.proxy_headers.iter()
    } else {
        [].iter()
    });
    let mut conn_te_appended = false;
    // When -F builds a multipart body, defer any user-supplied Content-Type
    // until after we emit Content-Length: curl orders Content-Length first
    // and then the (boundary-appended) Content-Type (test 669).
    let mut deferred_content_type: Option<(String, String)> = None;
    // During --digest empty-body probe, suppress user-supplied Content-Length
    // (we emit our own Content-Length: 0 below).
    let suppress_user_content_length = opts.defer_auth
        && opts.auth_probe_empty_upload
        && opts.digest_authorization.is_none()
        && (opts.upload_file.is_some() || opts.data.is_some());
    for (key, val) in user_header_iter {
        if key.eq_ignore_ascii_case("host") {
            continue;
        }
        if val.is_empty() {
            continue;
        }
        if suppress_user_content_length && key.eq_ignore_ascii_case("content-length") {
            continue;
        }
        if key.eq_ignore_ascii_case("content-type")
            && let Some(b) = boundary.as_ref()
            && !val.to_ascii_lowercase().contains("boundary=")
        {
            // Curl canonicalizes the header name when it appends boundary
            // (test 669 — user passes `Content-type:`, output is `Content-Type:`).
            deferred_content_type =
                Some(("Content-Type".to_string(), format!("{val}; boundary={b}")));
            continue;
        }
        // When --tr-encoding is set, the first user-supplied Connection header
        // gets ", TE" appended to its value so we don't emit a duplicate
        // Connection: TE header (test 1125). A blanked-out user Connection
        // (`-H "Connection;"`, val == "\x00") is dropped entirely — the auto
        // `Connection: TE` already covers it (test 1171).
        if opts.tr_encoding && !conn_te_appended && key.eq_ignore_ascii_case("connection") {
            if val == "\x00" {
                continue;
            }
            req.push_str(&format!("{key}: {val}, TE\r\n"));
            conn_te_appended = true;
            continue;
        }
        if val == "\x00" {
            req.push_str(&format!("{key}:\r\n"));
        } else {
            req.push_str(&format!("{key}: {val}\r\n"));
        }
    }

    // --digest/--ntlm/--negotiate (but NOT --anyauth) probes with an empty
    // body for PUT/POST so the body isn't wasted on the 401 challenge — curl
    // sends Content-Length: 0 first, gets the challenge, then resends with
    // auth and the real body. After we've computed digest_authorization the
    // probe is over (test 88 vs test 156).
    // NTLM Type 1 also goes out with an empty body — the body waits for the
    // Type 2 challenge so it isn't burned on a request that's certain to 401
    // (tests 155, 170).
    let ntlm_probe = opts.ntlm
        && !opts.ntlm_done
        && opts.ntlm_authorization.is_none()
        && (opts.upload_file.is_some() || opts.data.is_some() || !opts.form_fields.is_empty());
    let proxy_ntlm_probe = opts.proxy_ntlm
        && !opts.proxy_ntlm_done
        && opts.proxy_ntlm_authorization.is_none()
        && opts.proxy.is_some()
        && !opts.proxy_tunnel
        && (opts.upload_file.is_some() || opts.data.is_some() || !opts.form_fields.is_empty());
    let auth_probe_empty = (opts.defer_auth
        && opts.auth_probe_empty_upload
        && opts.digest_authorization.is_none()
        && (opts.upload_file.is_some() || opts.data.is_some()))
        || ntlm_probe
        || proxy_ntlm_probe;
    let body = if auth_probe_empty {
        Some(Vec::new())
    } else {
        build_body(opts, boundary.as_deref())
    };

    if let Some(ref body) = body {
        // Set Content-Type and Content-Length if not already set by custom headers.
        let has_content_type = opts
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));
        let has_content_length = opts
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-length"));
        let has_expect = opts
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("expect"));

        let content_len_hdr = format!("Content-Length: {}\r\n", body.len());
        // Content-Length and Transfer-Encoding: chunked are mutually exclusive.
        // During the --digest empty-body probe we ALWAYS emit Content-Length: 0
        // (overriding any user-supplied Content-Length) so the server sees the
        // probe correctly (test 1284, 1285).
        let emit_content_length = (!has_content_length && !user_chunked) || auth_probe_empty;

        // Emit Content-Length first.
        if emit_content_length {
            req.push_str(&content_len_hdr);
        }
        // Emit the deferred user Content-Type (with boundary appended).
        if let Some((k, v)) = deferred_content_type.take() {
            req.push_str(&format!("{k}: {v}\r\n"));
        }

        // curl sends Expect: 100-continue for upload bodies >= 1 MiB or when
        // uploading from stdin (-T -). Skip for HTTP/1.0 or if user suppressed it.
        // See lib/http.h EXPECT_100_THRESHOLD in curl source.
        let auto_expect = !has_expect && !http10 && (is_stdin_upload || body.len() >= 1024 * 1024);
        // A user-supplied "Expect: 100-continue" also activates the handshake
        // so we wait for the server's interim response before sending the body.
        let user_expect = opts.headers.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("expect") && v.eq_ignore_ascii_case("100-continue")
        });
        let do_expect_handshake = auto_expect || (user_expect && !http10);

        // Content-Type comes before Expect (matches curl ordering). For the
        // auth probe (NTLM Type 1 / digest probe) the multipart Content-Type
        // is suppressed — the empty body has no MIME boundary to declare and
        // curl skips it (test 170). Urlencoded `-d` data still emits its
        // static Content-Type so the server can see the form encoding even
        // for the probe (test 267).
        if !has_content_type {
            if let Some(ref b) = boundary {
                if !auth_probe_empty {
                    req.push_str(&format!(
                        "Content-Type: multipart/form-data; boundary={b}\r\n"
                    ));
                }
            } else if opts.data.is_some() {
                req.push_str("Content-Type: application/x-www-form-urlencoded\r\n");
            }
        }
        // Expect: 100-continue last (after Content-Type), matching curl.
        if auto_expect {
            req.push_str("Expect: 100-continue\r\n");
        }
        // When Expect: 100-continue is in play (auto or user), return headers
        // and body separately so the caller can implement the handshake.
        if do_expect_handshake {
            req.push_str("\r\n");
            let header_bytes = req.into_bytes();
            let body_bytes = if user_chunked {
                encode_chunked(body)
            } else {
                body.clone()
            };
            return (header_bytes, Some(body_bytes));
        }
    }

    req.push_str("\r\n");

    let mut bytes = req.into_bytes();
    if let Some(body) = body {
        if user_chunked {
            bytes.extend_from_slice(&encode_chunked(&body));
        } else {
            bytes.extend_from_slice(&body);
        }
    }
    (bytes, None)
}

fn encode_chunked(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 32);
    if !body.is_empty() {
        out.extend_from_slice(format!("{:x}\r\n", body.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"0\r\n\r\n");
    out
}

fn build_body(opts: &Options, boundary: Option<&str>) -> Option<Vec<u8>> {
    if let Some(ref data) = opts.data {
        return Some(data.clone());
    }

    // When -H "Content-Type:" overrides the multipart wrapper Content-Type
    // with a non-`multipart/form-data` value, curl emits part dispositions
    // as `attachment` rather than `form-data` (RFC 1867 style, test 277).
    // A user-supplied `multipart/form-data; charset=…` keeps the default
    // `form-data` disposition (test 669).
    let user_non_mp_ct = opts.headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("content-type")
            && !v
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("multipart/form-data")
    });
    let disposition = if user_non_mp_ct {
        "attachment"
    } else {
        "form-data"
    };

    if let Some(boundary) = boundary {
        let mut body = Vec::new();
        for field in &opts.form_fields {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            if field.is_file {
                if !field.extra_files.is_empty() {
                    // Multi-file: outer Content-Disposition (no filename) +
                    // Content-Type: multipart/mixed; boundary=INNER. Each file
                    // becomes a Content-Disposition: attachment part inside.
                    let inner = multipart_boundary();
                    body.extend_from_slice(
                        format!(
                            "Content-Disposition: {disposition}; name=\"{}\"\r\n\
                             Content-Type: multipart/mixed; boundary={inner}\r\n\r\n",
                            field.name
                        )
                        .as_bytes(),
                    );
                    let first = (
                        field.value.clone(),
                        field.content_type.clone(),
                        field.filename.clone(),
                    );
                    for (path, ct_opt, fn_opt) in
                        std::iter::once(first).chain(field.extra_files.iter().cloned())
                    {
                        body.extend_from_slice(format!("--{inner}\r\n").as_bytes());
                        let orig_filename = std::path::Path::new(&path)
                            .file_name()
                            .and_then(|f| f.to_str())
                            .unwrap_or("file");
                        let display_filename = fn_opt.as_deref().unwrap_or(orig_filename);
                        let ct = ct_opt
                            .as_deref()
                            .unwrap_or_else(|| guess_content_type(orig_filename));
                        let safe_filename = if opts.form_escape {
                            mime_backslash_escape(display_filename)
                        } else {
                            mime_percent_encode(display_filename)
                        };
                        body.extend_from_slice(
                            format!(
                                "Content-Disposition: attachment; filename=\"{safe_filename}\"\r\n\
                                 Content-Type: {ct}\r\n\r\n",
                            )
                            .as_bytes(),
                        );
                        if let Ok(data) = fs::read(&path) {
                            body.extend_from_slice(&data);
                        }
                        body.extend_from_slice(b"\r\n");
                    }
                    body.extend_from_slice(format!("--{inner}--\r\n").as_bytes());
                } else {
                    let is_stdin = field.value == "-";
                    // The display filename (for Content-Disposition) may be
                    // overridden by ;filename=, but Content-Type guessing always
                    // uses the ORIGINAL file path (matching curl behavior).
                    let orig_filename = if is_stdin {
                        "-"
                    } else {
                        std::path::Path::new(&field.value)
                            .file_name()
                            .and_then(|f| f.to_str())
                            .unwrap_or("file")
                    };
                    let display_filename = field.filename.as_deref().unwrap_or(orig_filename);
                    let ct = field
                        .content_type
                        .as_deref()
                        .unwrap_or_else(|| guess_content_type(orig_filename));
                    // Percent-encode \", \r, \n in the display filename
                    // (matching curl's mime_content_disposition encoding).
                    let safe_filename = if opts.form_escape {
                        mime_backslash_escape(display_filename)
                    } else {
                        mime_percent_encode(display_filename)
                    };
                    body.extend_from_slice(
                        format!(
                            "Content-Disposition: {disposition}; name=\"{}\"; filename=\"{safe_filename}\"\r\n\
                             Content-Type: {ct}\r\n\r\n",
                            field.name
                        )
                        .as_bytes(),
                    );
                    if is_stdin {
                        let mut data = Vec::new();
                        let _ = io::stdin().read_to_end(&mut data);
                        body.extend_from_slice(&data);
                    } else if let Ok(data) = fs::read(&field.value) {
                        body.extend_from_slice(&data);
                    }
                }
            } else {
                // `-F=value` (empty name) emits just `Content-Disposition:
                // form-data` with no `name=` attribute (test 1293). A
                // `;filename=…` modifier on a text field adds the filename
                // attribute and infers Content-Type from the filename
                // extension if no explicit `;type=` was given (test 2073).
                let cd = if field.name.is_empty() {
                    format!("Content-Disposition: {disposition}\r\n")
                } else if let Some(ref fname) = field.filename {
                    let safe = if opts.form_escape {
                        mime_backslash_escape(fname)
                    } else {
                        mime_percent_encode(fname)
                    };
                    format!(
                        "Content-Disposition: {disposition}; name=\"{}\"; filename=\"{safe}\"\r\n",
                        field.name
                    )
                } else {
                    format!(
                        "Content-Disposition: {disposition}; name=\"{}\"\r\n",
                        field.name
                    )
                };
                body.extend_from_slice(cd.as_bytes());
                let ct = field.content_type.as_deref().map(String::from).or_else(|| {
                    field
                        .filename
                        .as_deref()
                        .map(|f| guess_content_type(f).to_string())
                });
                if let Some(ct) = ct {
                    body.extend_from_slice(format!("Content-Type: {ct}\r\n").as_bytes());
                }
                body.extend_from_slice(b"\r\n");
                body.extend_from_slice(field.value.as_bytes());
            }
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        return Some(body);
    }

    if let Some(ref path) = opts.upload_file {
        if matches!(path.to_str(), Some("-" | ".")) {
            let mut data = Vec::new();
            let _ = io::stdin().read_to_end(&mut data);
            return Some(data);
        }
        let mut data = fs::read(path).ok()?;
        // Upload with -C <offset>: skip the first <offset> bytes of the file.
        if let Some(ref resume) = opts.resume_from
            && resume != "-"
            && let Ok(start) = resume.parse::<usize>()
            && start <= data.len()
        {
            data.drain(..start);
        }
        // CURL_UPLOAD_SIZE (Debug-gated env): truncate the upload to a fixed
        // number of bytes — simulates a "growing file" where curl committed to
        // a size at start-of-upload (test 447).
        if let Ok(s) = std::env::var("CURL_UPLOAD_SIZE")
            && let Ok(n) = s.parse::<usize>()
            && n <= data.len()
        {
            data.truncate(n);
        }
        return Some(data);
    }

    None
}

/// Percent-encode characters in a MIME filename that would break
/// Content-Disposition header syntax: `"` → `%22`, `\r` → `%0d`, `\n` → `%0a`.
/// This matches curl's `mime_content_disposition` encoding.
fn mime_percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'"' => out.push_str("%22"),
            b'\r' => out.push_str("%0d"),
            b'\n' => out.push_str("%0a"),
            _ => out.push(b as char),
        }
    }
    out
}

fn mime_backslash_escape(s: &str) -> String {
    // --form-escape mode: escape `"`/`\r`/`\n`/`\\` with a leading backslash
    // instead of percent-encoding them (test 1186, 1189).
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'\\' | b'"' => {
                out.push('\\');
                out.push(b as char);
            }
            b'\r' => out.push_str("\\r"),
            b'\n' => out.push_str("\\n"),
            _ => out.push(b as char),
        }
    }
    out
}

fn guess_content_type(filename: &str) -> &'static str {
    let lower = filename.to_lowercase();
    if lower.ends_with(".txt") || lower.ends_with(".text") || lower.ends_with(".log") {
        "text/plain"
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html"
    } else if lower.ends_with(".xml") {
        "application/xml"
    } else if lower.ends_with(".json") {
        "application/json"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".css") {
        "text/css"
    } else if lower.ends_with(".js") {
        "application/javascript"
    } else if lower.ends_with(".pdf") {
        "application/pdf"
    } else {
        "application/octet-stream"
    }
}

/// Build a combined Cookie header from -b flags (files and inline strings).
/// File cookies apply domain/path matching; inline cookies are sent verbatim.
fn is_loopback_host(host: &str) -> bool {
    host == "127.0.0.1" || host == "::1" || host == "localhost" || host.ends_with(".localhost")
}

/// curl's `replace_existing()` path-prefix rule (lib/cookie.c): a non-secure
/// cookie may overlay an existing secure cookie only when the new path falls
/// OUTSIDE the existing path's first directory segment. For existing path
/// `/a` (no inner '/'), the prefix is the whole `/a`. For `/1561/login`, the
/// prefix is just `/1561` (chopped at the next '/'). The new cookie is
/// rejected when its path equals or starts with that prefix.
fn secure_path_overlay(existing_path: &str, new_path: &str) -> bool {
    if !existing_path.starts_with('/') {
        return false;
    }
    let prefix_len = match existing_path.get(1..).and_then(|s| s.find('/')) {
        Some(idx) => idx + 1, // include the leading '/' that we skipped
        None => existing_path.len(),
    };
    new_path.len() >= prefix_len && new_path[..prefix_len] == existing_path[..prefix_len]
}

fn build_cookie_header(host: &str, path: &str, secure_req: bool, opts: &Options) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let request_host = host.to_lowercase();

    // (path_length, domain_length, name_length, name=value) — kept for sort by specificity.
    let mut file_pairs: Vec<(usize, usize, usize, String)> = Vec::new();
    let mut inline_pairs: Vec<String> = Vec::new();
    // Per-domain cap: curl's CMAX_COOKIES_PER_DOMAIN is 150. New entries past
    // that limit for a given domain are rejected at insert time (test 442).
    let mut per_domain_count: Vec<(String, usize)> = Vec::new();
    const MAX_COOKIES_PER_DOMAIN: usize = 150;

    for cookie in &opts.cookies {
        if cookie.contains('=') {
            inline_pairs.push(cookie.clone());
        } else if std::path::Path::new(cookie).is_file()
            && let Ok(contents) = fs::read_to_string(cookie)
        {
            if contents.starts_with("HTTP/") {
                // HTTP response header format: extract Set-Cookie lines and
                // convert to Netscape format for matching.
                for header_line in contents.split('\n') {
                    let header_line = header_line.trim_end_matches('\r');
                    // Match "Set-Cookie:" case-insensitively.
                    if header_line.len() > 11
                        && header_line[..11].eq_ignore_ascii_case("set-cookie:")
                    {
                        let rest = &header_line[11..];
                        let cookie_str = rest.trim();
                        // Parse the Set-Cookie value into a Netscape format line
                        // using the request host as the default domain.
                        let fake_url = ParsedUrl {
                            scheme: if secure_req {
                                "https".into()
                            } else {
                                "http".into()
                            },
                            host: host.to_string(),
                            port: if secure_req { 443 } else { 80 },
                            path: "/".to_string(), // file-loaded cookies default to path "/"
                            raw: String::new(),
                            userinfo: None,
                            fragment: None,
                        };
                        if let Some(nline) =
                            crate::cookie::format_cookie_line(cookie_str, &fake_url, &request_host)
                        {
                            // Now parse the Netscape format line for matching.
                            let nline_ref = if let Some(r) = nline.strip_prefix("#HttpOnly_") {
                                r
                            } else {
                                nline.as_str()
                            };
                            if nline_ref.contains('\t') {
                                let fields: Vec<&str> = nline_ref.split('\t').collect();
                                if fields.len() >= 7 {
                                    let domain = fields[0];
                                    let cookie_path = fields[2];
                                    let secure_flag = fields[3].eq_ignore_ascii_case("TRUE");
                                    let expiry: i64 = fields[4].parse().unwrap_or(0);
                                    let name = fields[5];
                                    let value = fields[6];

                                    // Skip cookies deleted via Max-Age=0.
                                    {
                                        let clean_domain = domain
                                            .strip_prefix('.')
                                            .unwrap_or(domain)
                                            .to_lowercase();
                                        let clean_path = cookie_path.trim_end_matches('/');
                                        if opts.deleted_cookies.iter().any(|(dd, dp, dn)| {
                                            let dd_clean =
                                                dd.strip_prefix('.').unwrap_or(dd).to_lowercase();
                                            let dp_clean = dp.trim_end_matches('/');
                                            dd_clean == clean_domain
                                                && dp_clean == clean_path
                                                && dn == name
                                        }) {
                                            continue;
                                        }
                                    }

                                    if expiry == 0 && opts.junk_session_cookies {
                                        continue;
                                    }
                                    if expiry > 0 && expiry < now {
                                        continue;
                                    }
                                    if secure_flag && !secure_req {
                                        continue;
                                    }
                                    if !domain_matches(&request_host, domain) {
                                        continue;
                                    }
                                    // If no Path attribute was set, curl loads the cookie
                                    // with NULL path (matches anything, sorts at length 0).
                                    let had_path_attr = cookie_str.split(";").skip(1).any(|a| {
                                        let a = a.trim();
                                        let key = a.split("=").next().unwrap_or("").trim();
                                        key.eq_ignore_ascii_case("path")
                                    });
                                    let effective_path: &str =
                                        if had_path_attr { cookie_path } else { "" };
                                    if !effective_path.is_empty()
                                        && !path.starts_with(effective_path)
                                    {
                                        continue;
                                    }
                                    // Likewise, if no Domain attribute was set, curl uses
                                    // co->domain = NULL (length 0 in cookie_sort).
                                    let had_domain_attr = cookie_str.split(";").skip(1).any(|a| {
                                        let a = a.trim();
                                        let key = a.split("=").next().unwrap_or("").trim();
                                        key.eq_ignore_ascii_case("domain")
                                    });
                                    let sort_domain_len =
                                        if had_domain_attr { domain.len() } else { 0 };
                                    file_pairs.push((
                                        effective_path.len(),
                                        sort_domain_len,
                                        name.len(),
                                        format!("{name}={value}"),
                                    ));
                                }
                            }
                        }
                    }
                }
            } else {
                // Netscape cookie format
                for line in contents.split('\n') {
                    let line = line.trim_end_matches('\r');
                    let line = if let Some(rest) = line.strip_prefix("#HttpOnly_") {
                        rest
                    } else if line.starts_with('#') {
                        continue;
                    } else {
                        line
                    };
                    if !line.contains('\t') {
                        continue;
                    }
                    let fields: Vec<&str> = line.split('\t').collect();
                    if fields.len() < 7 {
                        continue;
                    }
                    let domain = fields[0];
                    let cookie_path = fields[2];
                    let secure = fields[3].eq_ignore_ascii_case("TRUE");
                    let expiry: i64 = fields[4].parse().unwrap_or(0);
                    let name = fields[5];
                    let value = fields[6];

                    // Skip cookies that were deleted via Max-Age=0 in a previous response.
                    {
                        let clean_domain =
                            domain.strip_prefix('.').unwrap_or(domain).to_lowercase();
                        let clean_path = cookie_path.trim_end_matches('/');
                        if opts.deleted_cookies.iter().any(|(dd, dp, dn)| {
                            let dd_clean = dd.strip_prefix('.').unwrap_or(dd).to_lowercase();
                            let dp_clean = dp.trim_end_matches('/');
                            dd_clean == clean_domain && dp_clean == clean_path && dn == name
                        }) {
                            continue;
                        }
                    }

                    if expiry == 0 && opts.junk_session_cookies {
                        continue;
                    }
                    if expiry > 0 && expiry <= now {
                        continue;
                    }
                    if secure && !secure_req {
                        continue;
                    }
                    // Apply per-domain cap BEFORE match filtering so a huge
                    // file gets capped at the cookie-store level, not just
                    // at the matching set (test 442).
                    let cap_key = domain.strip_prefix('.').unwrap_or(domain).to_lowercase();
                    let count = if let Some(slot) =
                        per_domain_count.iter_mut().find(|(k, _)| k == &cap_key)
                    {
                        &mut slot.1
                    } else {
                        per_domain_count.push((cap_key, 0));
                        &mut per_domain_count.last_mut().unwrap().1
                    };
                    if *count >= MAX_COOKIES_PER_DOMAIN {
                        continue;
                    }
                    *count += 1;
                    if !domain_matches(&request_host, domain) {
                        continue;
                    }
                    if !path_matches(path, cookie_path) {
                        continue;
                    }
                    file_pairs.push((
                        cookie_path.len(),
                        domain.len(),
                        name.len(),
                        format!("{name}={value}"),
                    ));
                }
            }
        }
    }

    // Also check in-memory cookies accumulated from previous responses.
    for line in &opts.memory_cookies {
        let line = if let Some(rest) = line.strip_prefix("#HttpOnly_") {
            rest
        } else if line.starts_with('#') {
            continue;
        } else {
            line.as_str()
        };
        if !line.contains('\t') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 7 {
            continue;
        }
        let domain = fields[0];
        let cookie_path = fields[2];
        let secure = fields[3].eq_ignore_ascii_case("TRUE");
        let expiry: i64 = fields[4].parse().unwrap_or(0);
        let name = fields[5];
        let value = fields[6];

        if expiry == 0 && opts.junk_session_cookies {
            continue;
        }
        if expiry > 0 && expiry <= now {
            continue;
        }
        if secure && !secure_req {
            continue;
        }
        if !domain_matches(&request_host, domain) {
            continue;
        }
        if !path_matches(path, cookie_path) {
            continue;
        }
        file_pairs.push((
            cookie_path.len(),
            domain.len(),
            name.len(),
            format!("{name}={value}"),
        ));
    }

    // curl stores cookies in a LIFO linked list (newest first). Among cookies
    // with equal specificity it therefore emits them in reverse load order.
    // Reverse first so stable sort preserves the LIFO order within equal keys.
    file_pairs.reverse();
    // Sort: longer path first, then longer domain, then longer name (curl cookie_sort).
    // Within equal specificity, the reverse() above preserves LIFO (creation-time desc) order.
    file_pairs.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));

    let mut all: Vec<String> = file_pairs.into_iter().map(|(_, _, _, p)| p).collect();
    all.extend(inline_pairs);

    // curl caps the Cookie header at MAX_COOKIE_HEADER_LEN bytes (8190). Drop
    // the oldest entries (end of the sorted list) until the joined string
    // fits. This mirrors the behaviour expected by upstream tests such as 443.
    const MAX: usize = 8190;
    loop {
        let total: usize = all
            .iter()
            .map(|s| s.len() + 2)
            .sum::<usize>()
            .saturating_sub(2);
        if total <= MAX || all.len() <= 1 {
            break;
        }
        all.pop();
    }
    all.join("; ")
}

/// RFC 6265 §5.1.4 path matching: the cookie's path-attribute matches the
/// request URI's path component if (a) they're identical, (b) the cookie path
/// is a prefix of the request path AND ends with `/`, or (c) the cookie path
/// is a prefix and the next byte in the request path is `/`. Anything else
/// is a non-match — `/hoge` does NOT cover `/hogege` (test 1228).
fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    if cookie_path.is_empty() {
        return true;
    }
    if request_path == cookie_path {
        return true;
    }
    if !request_path.starts_with(cookie_path) {
        return false;
    }
    cookie_path.ends_with('/') || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/')
}

/// Netscape-style cookie domain match.
/// Cookie domain starting with '.' matches host with that suffix.
/// Cookie domain without '.' matches the exact host.
fn domain_matches(request_host: &str, cookie_domain: &str) -> bool {
    let cookie_domain = cookie_domain.to_lowercase();
    if let Some(suffix) = cookie_domain.strip_prefix('.') {
        if suffix.is_empty() {
            return false;
        }
        // Host must end with cookie_domain (without the leading dot)
        // and either equal it or have a '.' before it.
        if request_host == suffix {
            return true;
        }
        if request_host.ends_with(&format!(".{suffix}")) {
            return true;
        }
        false
    } else {
        request_host == cookie_domain
    }
}

fn encode_path_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{b:02x}"));
            }
        }
    }
    out
}

/// Percent-encode high-bit (non-ASCII) bytes in a request-line path. Existing
/// %XX escapes and ASCII bytes are passed through unchanged. Used for redirect
/// targets where the Location header may contain raw UTF-8 (test 1138).
fn pct_encode_high(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b >= 0x80 {
            out.push_str(&format!("%{b:02X}"));
        } else {
            out.push(b as char);
        }
    }
    out
}

fn multipart_boundary() -> String {
    // Match curl's boundary format: 24 dashes + 22 random alphanumeric chars.
    // See curl lib/mime.h: MIME_BOUNDARY_DASHES=24, MIME_RAND_BOUNDARY_CHARS=22.
    const CHARSET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut rand_part = String::with_capacity(22);
    let mut state = ts as u64;
    for _ in 0..22 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let idx = (state >> 33) as usize % CHARSET.len();
        rand_part.push(CHARSET[idx] as char);
    }
    format!("------------------------{rand_part}")
}

/// Absorb `Set-Cookie` headers from `resp` into `opts.memory_cookies` so the
/// next request from the same `perform()` (auth retry, redirect, …) can
/// emit them on its `Cookie:` header (test 1331). When the cookie engine is
/// disabled this is a no-op.
fn absorb_response_cookies(opts: &mut Options, resp: &Response, url: &ParsedUrl) {
    if !opts.cookie_engine {
        return;
    }
    let host_for_cookies = opts
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("host"))
        .map(|(_, v)| v.split(':').next().unwrap_or(v).to_string())
        .unwrap_or_else(|| url.host.clone());
    for (k, v) in &resp.headers {
        if k != "set-cookie" {
            continue;
        }
        let Some(line) = crate::cookie::format_cookie_line(v, url, &host_for_cookies) else {
            continue;
        };
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() >= 7 {
            let new_dom = fields[0].strip_prefix("#HttpOnly_").unwrap_or(fields[0]);
            let np = fields[2];
            let nn = fields[5];
            // Per RFC 6265bis §5.5 step 12.4: a non-secure source cannot
            // override an existing secure-flagged cookie (test 414, with
            // curl's broader name-only match — applies even when the host-
            // only-flag differs).
            let new_secure = fields[3].eq_ignore_ascii_case("TRUE");
            let from_https = url.scheme == "https";
            if !from_https && !new_secure {
                // curl stores the domain without the leading dot regardless of
                // whether the cookie was set host-only or with a Domain attr.
                // We keep the dot in the Netscape line, so strip it before
                // comparing so a host-only secure cookie still blocks a Domain=
                // overlay (test 414).
                let new_dom_key = new_dom.strip_prefix('.').unwrap_or(new_dom);
                let blocked = opts.memory_cookies.iter().any(|existing| {
                    let ef: Vec<&str> = existing.split('\t').collect();
                    if ef.len() < 7 || ef[5] != nn || !ef[3].eq_ignore_ascii_case("TRUE") {
                        return false;
                    }
                    let ed = ef[0].strip_prefix("#HttpOnly_").unwrap_or(ef[0]);
                    let ed_key = ed.strip_prefix('.').unwrap_or(ed);
                    ed_key.eq_ignore_ascii_case(new_dom_key) && secure_path_overlay(ef[2], np)
                });
                if blocked {
                    continue;
                }
            }
            opts.memory_cookies.retain(|existing| {
                let ef: Vec<&str> = existing.split('\t').collect();
                if ef.len() >= 7 {
                    let ed = ef[0].strip_prefix("#HttpOnly_").unwrap_or(ef[0]);
                    !(ed == new_dom && ef[2] == np && ef[5] == nn)
                } else {
                    true
                }
            });
        }
        if !crate::cookie::is_jar_line_expired(&line) {
            let new_dom = line
                .split('\t')
                .next()
                .map(|s| s.strip_prefix("#HttpOnly_").unwrap_or(s).to_string())
                .unwrap_or_default();
            let cnt = opts
                .memory_cookies
                .iter()
                .filter(|c| {
                    c.split('\t')
                        .next()
                        .map(|s| s.strip_prefix("#HttpOnly_").unwrap_or(s) == new_dom.as_str())
                        .unwrap_or(false)
                })
                .count();
            if cnt < 50 {
                opts.memory_cookies.push(line);
            }
        }
    }
}

fn execute_request(
    url: &ParsedUrl,
    opts: &Options,
    accumulated_header_bytes: usize,
) -> Result<Response, String> {
    // Try once; if the connection came from the pool and the server has
    // already closed it (e.g. test 4's `swsclose` directive shuts down the
    // socket between requests), retry with a fresh connection.
    match execute_request_inner(url, opts, accumulated_header_bytes) {
        Err(e)
            if is_stale_pool_error(&e) && crate::connection::POOL_REUSED.with(|h| *h.borrow()) =>
        {
            // Drop the bad pool entry and reconnect fresh.
            crate::connection::CONN_POOL.with(|r| *r.borrow_mut() = None);
            crate::connection::POOL_REUSED.with(|h| *h.borrow_mut() = false);
            execute_request_inner(url, opts, accumulated_header_bytes)
        }
        other => other,
    }
}

fn is_stale_pool_error(e: &str) -> bool {
    e.contains("empty reply")
        || e.contains("failed to read status line")
        || e.contains("failed to send request")
        || e.contains("Broken pipe")
        || e.contains("Connection reset")
}

fn is_got_nothing_error(e: &str) -> bool {
    e.contains("empty reply") || e.contains("failed to read status line")
}

fn execute_request_inner(
    url: &ParsedUrl,
    opts: &Options,
    accumulated_header_bytes: usize,
) -> Result<Response, String> {
    // On a CONNECT-tunnel 407 with a Digest challenge, compute the response
    // and retry once with Proxy-Authorization: Digest on the CONNECT line
    // (tests 206, 1060, 1061). The retry needs its own `opts` so we don't
    // contaminate the live one.
    let (mut conn, connect_response) = match connect(url, opts) {
        Ok(v) => v,
        Err(e) if e.contains("CONNECT tunnel failed, response 407")
            && opts.proxy_user.is_some()
            && opts.connect_proxy_digest_authorization.is_none() =>
        {
            let connect_bytes = crate::connection::CONNECT_RESP
                .with(|r| r.borrow().clone())
                .map(|(_, b)| b)
                .unwrap_or_default();
            let chal = parse_connect_proxy_digest_challenge(&connect_bytes);
            let proxy_creds = opts.proxy_user.as_deref().and_then(|c| {
                c.split_once(':')
                    .map(|(u, p)| (u.to_string(), p.to_string()))
            });
            let (tgt_host, tgt_port) = crate::connection::connect_to_override(
                &url.host,
                url.port,
                &opts.connect_tos,
            )
            .unwrap_or_else(|| (url.host.clone(), url.port));
            let target = if tgt_host.contains(':') {
                format!("[{}]:{}", tgt_host, tgt_port)
            } else {
                format!("{}:{}", tgt_host, tgt_port)
            };
            let digest_header = chal.and_then(|c| {
                proxy_creds
                    .as_ref()
                    .and_then(|(u, p)| build_digest_auth(u, p, &c, "CONNECT", &target))
            });
            if let Some(header) = digest_header {
                let mut retry_opts = opts.clone();
                retry_opts.connect_proxy_digest_authorization = Some(header);
                retry_opts.defer_proxy_auth = false;
                connect(url, &retry_opts)?
            } else {
                return Err(e);
            }
        }
        Err(e) => return Err(e),
    };
    // --haproxy-protocol: prepend a PROXY v1 header to the connection
    // before the HTTP request (test 3028). For proxy-tunneled requests
    // (CONNECT) the header goes onto the established tunnel.
    if opts.haproxy_protocol {
        let local = crate::connection::LOCAL_ADDR
            .with(|r| r.borrow().clone())
            .unwrap_or_default();
        let src_ip = opts
            .haproxy_clientip
            .clone()
            .unwrap_or_else(|| local.0.clone());
        let src_port = local.1;
        let (dst_ip, dst_port) = if let Some(ref p) = opts.proxy {
            // Strip scheme + auth + path; we only want host:port.
            let no_scheme = p.split("://").nth(1).unwrap_or(p.as_str());
            let no_path = no_scheme.split('/').next().unwrap_or(no_scheme);
            let after_at = no_path.split('@').next_back().unwrap_or(no_path);
            // Bracketed IPv6 literal vs plain host:port.
            let (h, port_s) = if let Some(rest) = after_at.strip_prefix('[') {
                let end = rest.find(']').unwrap_or(rest.len());
                let host = &rest[..end];
                let pt = rest
                    .get(end + 1..)
                    .and_then(|s| s.strip_prefix(':'))
                    .unwrap_or("");
                (host.to_string(), pt.to_string())
            } else {
                match after_at.rsplit_once(':') {
                    Some((h, pt)) => (h.to_string(), pt.to_string()),
                    None => (after_at.to_string(), String::new()),
                }
            };
            let p_num = port_s.parse::<u16>().unwrap_or(0);
            (h, p_num)
        } else {
            (url.host.clone(), url.port)
        };
        let bare_dst = crate::url::strip_ipv6_scope(&dst_ip);
        let bare_src = crate::url::strip_ipv6_scope(&src_ip);
        let is_ipv6 = bare_src.contains(':') || bare_dst.contains(':');
        let proto = if is_ipv6 { "TCP6" } else { "TCP4" };
        let line = format!("PROXY {proto} {bare_src} {bare_dst} {src_port} {dst_port}\r\n");
        conn.write_all(line.as_bytes())
            .map_err(|e| format!("failed to write PROXY header: {e}"))?;
    }
    let (request_headers, expect_body) = build_request(url, opts);

    let is_head = opts.head
        || opts
            .method
            .as_deref()
            .map(|m| m.eq_ignore_ascii_case("HEAD"))
            .unwrap_or(false);
    let max_filesize_overflow = opts
        .max_filesize_str
        .as_ref()
        .is_some_and(|s| s.parse::<u64>().is_err());

    if opts.verbose {
        // Print request headers to stderr.
        if let Ok(req_str) = std::str::from_utf8(&request_headers) {
            for line in req_str.split("\r\n") {
                if line.is_empty() {
                    break;
                }
                eprintln!("> {line}");
            }
            eprintln!(">");
        }
    }

    // When Expect: 100-continue was auto-added, implement the handshake:
    // send headers, wait for server response, then decide whether to send body.
    if let Some(body_bytes) = expect_body {
        // Step 1: Send only the headers (including the trailing \r\n\r\n).
        conn.write_all(&request_headers)
            .map_err(|e| format!("failed to send request headers: {e}"))?;
        conn.flush()
            .map_err(|e| format!("failed to flush request headers: {e}"))?;

        // Step 2: Wait for server response with a short timeout.
        conn.set_read_timeout(Some(Duration::from_secs(1)))
            .map_err(|e| format!("failed to set read timeout: {e}"))?;

        let mut peek_buf = Vec::new();
        let mut tmp = [0u8; 1];
        let got_response = loop {
            match conn.read(&mut tmp) {
                Ok(0) => break true, // EOF — server closed, treat as response available
                Ok(n) => {
                    peek_buf.extend_from_slice(&tmp[..n]);
                    // Check if we have enough to see the status code
                    if peek_buf.len() >= 12 {
                        // e.g. "HTTP/1.1 100" is 12 chars
                        break true;
                    }
                }
                Err(ref e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    // Timeout — no response within 1s
                    break false;
                }
                Err(e) => return Err(format!("failed to read 100-continue response: {e}")),
            }
        };

        // Restore no timeout for subsequent reads.
        let _ = conn.set_read_timeout(None);

        if !got_response {
            // Timeout: server didn't respond within 1s. Send the body anyway
            // (some servers don't send 100 but still expect the body).
            conn.write_all(&body_bytes)
                .map_err(|e| format!("failed to send request body: {e}"))?;
            conn.flush()
                .map_err(|e| format!("failed to flush request body: {e}"))?;

            let mut resp = read_response(
                &mut conn,
                is_head,
                opts.http09,
                opts.compressed,
                opts.tr_encoding,
                opts.raw,
                opts.max_filesize,
                max_filesize_overflow,
                accumulated_header_bytes,
                opts.ignore_content_length,
            )?;

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

            if !connect_response.is_empty() {
                resp.connect_header_size = connect_response.len();
                if !opts.suppress_connect_headers {
                    let mut combined = connect_response;
                    combined.extend_from_slice(&resp.header_bytes);
                    resp.header_bytes = combined;
                }
            }
            return Ok(resp);
        }

        // We got some response bytes. Parse the status line from what we peeked.
        let peeked_str = String::from_utf8_lossy(&peek_buf);

        if peeked_str.len() >= 12 {
            let status_str = &peeked_str[9..12];
            if let Ok(status_code) = status_str.parse::<u16>() {
                if status_code == 100 {
                    // Got 100 Continue. Read and discard the rest of the 100
                    // response headers, then send body.
                    // We need to read until we see \r\n\r\n after the status line.
                    // First, consume the rest of what we have plus more from the socket
                    // until we find the blank line terminating the 100 headers.
                    let mut interim_buf = peek_buf;
                    let mut tmp2 = [0u8; 512];
                    loop {
                        // Check if we have the end-of-headers marker
                        if let Some(pos) = find_subsequence(&interim_buf, b"\r\n\r\n") {
                            // We may have read past the 100 headers into the next response.
                            // Keep any leftover bytes after the \r\n\r\n.
                            let leftover_start = pos + 4;
                            let _leftover = interim_buf[leftover_start..].to_vec();
                            // (leftover would need to be prepended to next read,
                            // but read_response uses BufReader which handles this.
                            // Since we consumed from the raw connection, any leftover
                            // is lost. In practice, the 100 response is small and there
                            // is no data following it until we send the body.)

                            if opts.verbose
                                && let Ok(s) = std::str::from_utf8(&interim_buf[..pos])
                            {
                                for line in s.split("\r\n") {
                                    if !line.is_empty() {
                                        eprintln!("< {line}");
                                    }
                                }
                                eprintln!("<");
                            }
                            break;
                        }
                        match conn.read(&mut tmp2) {
                            Ok(0) => break,
                            Ok(n) => interim_buf.extend_from_slice(&tmp2[..n]),
                            Err(_) => break,
                        }
                    }

                    // Send the body.
                    conn.write_all(&body_bytes)
                        .map_err(|e| format!("failed to send request body: {e}"))?;
                    conn.flush()
                        .map_err(|e| format!("failed to flush request body: {e}"))?;

                    // Read the real response.
                    let mut resp = read_response(
                        &mut conn,
                        is_head,
                        opts.http09,
                        opts.compressed,
                        opts.tr_encoding,
                        opts.raw,
                        opts.max_filesize,
                        max_filesize_overflow,
                        accumulated_header_bytes,
                        opts.ignore_content_length,
                    )?;

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

                    if !connect_response.is_empty() {
                        resp.connect_header_size = connect_response.len();
                        if !opts.suppress_connect_headers {
                            let mut combined = connect_response;
                            combined.extend_from_slice(&resp.header_bytes);
                            resp.header_bytes = combined;
                        }
                    }
                    return Ok(resp);
                } else if status_code == 417 {
                    // 417 Expectation Failed — read the full 417 response first,
                    // then reconnect and retry without Expect.  The 417 headers
                    // are prepended to the final output (like interim responses).

                    // Read the rest of the 417 response.  We already have `peek_buf`
                    // which contains the status line bytes.  Chain them with the
                    // connection so read_response sees a complete HTTP response.
                    let cursor = std::io::Cursor::new(peek_buf);
                    let mut chained_417 = cursor.chain(&mut conn);
                    let resp_417 = read_response(
                        &mut chained_417,
                        false, // not HEAD
                        opts.http09,
                        false, // no decompression for the 417
                        false, // no tr_encoding
                        true,  // raw
                        None,  // no max_filesize
                        false,
                        0,
                        false,
                    )?;
                    let headers_417 = resp_417.header_bytes;

                    drop(chained_417);
                    drop(conn);
                    let mut retry_opts = opts.clone();
                    retry_opts
                        .headers
                        .push(("Expect".to_string(), String::new()));
                    let (mut conn2, connect_response2) = connect(url, &retry_opts)?;
                    let (request2, _) = build_request(url, &retry_opts);

                    if retry_opts.verbose
                        && let Ok(req_str) = std::str::from_utf8(&request2)
                    {
                        for line in req_str.split("\r\n") {
                            if line.is_empty() {
                                break;
                            }
                            eprintln!("> {line}");
                        }
                        eprintln!(">");
                    }

                    conn2
                        .write_all(&request2)
                        .map_err(|e| format!("failed to send request: {e}"))?;
                    conn2
                        .flush()
                        .map_err(|e| format!("failed to flush request: {e}"))?;

                    let mut resp = read_response(
                        &mut conn2,
                        is_head,
                        opts.http09,
                        opts.compressed,
                        opts.tr_encoding,
                        opts.raw,
                        opts.max_filesize,
                        max_filesize_overflow,
                        accumulated_header_bytes,
                        opts.ignore_content_length,
                    )?;

                    if retry_opts.verbose
                        && let Ok(hdr_str) = std::str::from_utf8(&resp.header_bytes)
                    {
                        for line in hdr_str.split("\r\n") {
                            if !line.is_empty() {
                                eprintln!("< {line}");
                            }
                        }
                        eprintln!("<");
                    }

                    // Prepend 417 headers + CONNECT headers to the retry response.
                    // With --suppress-connect-headers the CONNECT bytes still
                    // count toward `%{size_header}` but stay out of the output.
                    let connect_total = connect_response.len() + connect_response2.len();
                    resp.connect_header_size = connect_total;
                    let mut combined = Vec::new();
                    if !opts.suppress_connect_headers && !connect_response.is_empty() {
                        combined.extend_from_slice(&connect_response);
                    }
                    combined.extend_from_slice(&headers_417);
                    if !opts.suppress_connect_headers && !connect_response2.is_empty() {
                        combined.extend_from_slice(&connect_response2);
                    }
                    combined.extend_from_slice(&resp.header_bytes);
                    resp.header_bytes = combined;

                    return Ok(resp);
                } else {
                    // Other status code — server sent a final response instead
                    // of 100. Don't send body. Chain peeked bytes with the
                    // connection so read_response sees the full response.
                    let mut chain = io::Cursor::new(peek_buf).chain(&mut conn);
                    let mut resp = read_response(
                        &mut chain,
                        is_head,
                        opts.http09,
                        opts.compressed,
                        opts.tr_encoding,
                        opts.raw,
                        opts.max_filesize,
                        max_filesize_overflow,
                        accumulated_header_bytes,
                        opts.ignore_content_length,
                    )?;

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

                    if !connect_response.is_empty() {
                        let mut combined = connect_response;
                        combined.extend_from_slice(&resp.header_bytes);
                        resp.header_bytes = combined;
                    }
                    return Ok(resp);
                }
            }
        }

        // Could not parse status — just send body and read response normally.
        conn.write_all(&body_bytes)
            .map_err(|e| format!("failed to send request body: {e}"))?;
        conn.flush()
            .map_err(|e| format!("failed to flush request body: {e}"))?;

        // Prepend peeked bytes via chained reader.
        let mut chain = io::Cursor::new(peek_buf).chain(&mut conn);
        let mut resp = read_response(
            &mut chain,
            is_head,
            opts.http09,
            opts.compressed,
            opts.tr_encoding,
            opts.raw,
            opts.max_filesize,
            max_filesize_overflow,
            accumulated_header_bytes,
            opts.ignore_content_length,
        )?;

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

        if !connect_response.is_empty() {
            resp.connect_header_size = connect_response.len();
            if !opts.suppress_connect_headers {
                let mut combined = connect_response;
                combined.extend_from_slice(&resp.header_bytes);
                resp.header_bytes = combined;
            }
        }
        return Ok(resp);
    }

    // No Expect header — send the full request (headers + body) at once.
    conn.write_all(&request_headers)
        .map_err(|e| format!("failed to send request: {e}"))?;
    conn.flush()
        .map_err(|e| format!("failed to flush request: {e}"))?;

    let mut resp = read_response(
        &mut conn,
        is_head,
        opts.http09,
        opts.compressed,
        opts.tr_encoding,
        opts.raw,
        opts.max_filesize,
        max_filesize_overflow,
        accumulated_header_bytes,
        opts.ignore_content_length,
    )?;

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

    // Prepend CONNECT tunnel response headers so callers can see them.
    if !connect_response.is_empty() {
        resp.connect_header_size = connect_response.len();
        if !opts.suppress_connect_headers {
            let mut combined = connect_response;
            combined.extend_from_slice(&resp.header_bytes);
            resp.header_bytes = combined;
        }
    }

    // HTTP/1.1 keep-alive pool: save the connection back unless the server
    // (or our own request) signaled `Connection: close`, the response was
    // chunked-and-we-bailed-early, or we used a CONNECT tunnel / TLS.
    let conn_close = resp
        .headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("connection") && v.eq_ignore_ascii_case("close"));
    let user_close = opts
        .headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("connection") && v.eq_ignore_ascii_case("close"));
    let resp_http10 = resp.http10_response;
    // HTTP/1.0 connections default to close. Only keep them in the pool when
    // the response carried `Connection: keep-alive` (test 1074).
    let keep_alive = resp
        .headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("connection") && v.eq_ignore_ascii_case("keep-alive"));
    // HTTPS via TLS-on-tunnel still can't be pooled (rustls owns the stream).
    let block_tls_save = opts.proxy.is_some() && url.scheme == "https";
    // After a successful CONNECT tunnel for HTTP, the underlying TCP stream
    // is a direct pipe to the origin and CAN be pooled — the next request
    // to the same origin reuses it WITHOUT another CONNECT (test 275).
    let use_tunnel = opts.proxy.is_some() && (opts.proxy_tunnel || url.scheme == "https");
    let pool_ok = !conn_close
        && !user_close
        && (!resp_http10 || keep_alive)
        && url.scheme == "http"
        && !block_tls_save
        && !resp.recv_error
        && !resp.partial_file
        && !resp.timed_out
        && resp.status > 0;
    if pool_ok {
        // Pool key:
        //   - With CONNECT tunnel established: the stream goes to the origin,
        //     so key on origin host:port (is_proxy=false).
        //   - With plain proxy (HTTP, no tunnel): key on proxy host:port
        //     (is_proxy=true).
        //   - No proxy: key on origin (is_proxy=false).
        let (khost, kport, is_proxy_key) = if use_tunnel {
            (url.host.clone(), url.port, false)
        } else if let Some(ref proxy) = opts.proxy {
            match crate::connection::parse_proxy(proxy) {
                Ok((h, p)) => (h, p, true),
                Err(_) => return Ok(resp),
            }
        } else {
            (url.host.clone(), url.port, false)
        };
        let used_resolve =
            crate::connection::resolve_override(&url.host, url.port, &opts.resolves).is_some();
        crate::connection::CONN_POOL.with(|r| {
            *r.borrow_mut() = Some(crate::connection::PooledConn {
                host: khost,
                port: kport,
                is_proxy: is_proxy_key,
                http10: resp_http10,
                used_resolve,
                conn,
            });
        });
    }

    Ok(resp)
}

/// Search for a subsequence in a byte slice.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

pub(crate) fn perform(url_str: &str, opts: &Options) -> Result<Response, String> {
    let mut opts = opts.clone();

    // Early URL parse: a malformed URL must take priority over later
    // local checks (e.g. missing -T file). Test 1469 expects exit 3
    // for a URL with whitespace, even when -T points at a non-existent
    // file. We re-parse below within the redirect loop too.
    {
        let probe = if url_str.starts_with("http://") || url_str.starts_with("https://") {
            url_str.to_string()
        } else {
            format!("http://{url_str}")
        };
        if let Err(e) = crate::url::parse_url(&probe)
            && e.starts_with("malformed URL")
        {
            return Err(e);
        }
    }

    // Check that all form file uploads exist before connecting.
    // "-" means stdin — always allowed.
    for field in &opts.form_fields {
        if field.is_file && field.value != "-" && !std::path::Path::new(&field.value).exists() {
            return Err(format!(
                "read form file: couldn't open file \"{}\"",
                field.value
            ));
        }
    }

    // -T file: validate the upload-source file exists before connecting,
    // so a missing file maps to exit 26 (CURLE_READ_ERROR) rather than a
    // connection failure (test 496). "-" / "." are stdin and always allowed.
    if let Some(ref up) = opts.upload_file
        && let Some(s) = up.to_str()
        && s != "-"
        && s != "."
        && !std::path::Path::new(s).exists()
    {
        return Err(format!("read form file: couldn't open file \"{s}\""));
    }

    // HTTP/1.0 + stdin upload: no chunked encoding available so we can't ship
    // the body without an a-priori Content-Length. curl exits 25 (test 1069).
    let is_stdin_upload = matches!(
        opts.upload_file.as_deref().and_then(|p| p.to_str()),
        Some("-" | ".")
    );
    let http10 = opts.http_version.as_deref() == Some("1.0");
    let user_chunked = opts.headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked")
    });
    if is_stdin_upload && http10 && !user_chunked {
        return Err(
            "upload_failed: HTTP/1.0 cannot upload from stdin without Content-Length".into(),
        );
    }

    // Pick up proxy from env when none was given via the CLI: http_proxy
    // (HTTP only — historical: only the lowercase form) or ALL_PROXY.
    if opts.proxy.is_none() {
        let scheme_lc = url_str
            .split_once("://")
            .map(|(s, _)| s.to_ascii_lowercase())
            .unwrap_or_default();
        let env = if scheme_lc == "https" {
            std::env::var("HTTPS_PROXY")
                .or_else(|_| std::env::var("https_proxy"))
                .ok()
        } else if scheme_lc == "http" {
            std::env::var("http_proxy").ok()
        } else {
            None
        };
        let proxy = env.or_else(|| {
            std::env::var("ALL_PROXY")
                .or_else(|_| std::env::var("all_proxy"))
                .ok()
        });
        if let Some(p) = proxy
            && !p.is_empty()
        {
            opts.proxy = Some(p);
        }
    }

    // NO_PROXY / no_proxy: bypass proxy for matching hosts. The CLI
    // `--noproxy` option overrides the env var.
    if opts.proxy.is_some() {
        let no_proxy = opts.noproxy.clone().unwrap_or_else(|| {
            std::env::var("no_proxy")
                .or_else(|_| std::env::var("NO_PROXY"))
                .unwrap_or_default()
        });
        if no_proxy == "*" {
            opts.proxy = None;
            opts.proxy_user = None;
        } else if !no_proxy.is_empty() {
            // Extract the target host from the URL for matching.
            let target_host = url_str
                .find("://")
                .and_then(|i| {
                    let after = &url_str[i + 3..];
                    let host_end = after.find('/').unwrap_or(after.len());
                    let authority = &after[..host_end];
                    // Strip userinfo@
                    let host_port = authority
                        .rfind('@')
                        .map(|j| &authority[j + 1..])
                        .unwrap_or(authority);
                    // Strip :port
                    let host = if host_port.starts_with('[') {
                        host_port.find(']').map(|j| &host_port[1..j])
                    } else {
                        host_port
                            .rfind(':')
                            .map(|j| &host_port[..j])
                            .or(Some(host_port))
                    };
                    host.map(|h| h.to_lowercase())
                })
                .unwrap_or_default();
            for entry in no_proxy.split(',') {
                let entry = entry.trim().to_lowercase();
                if entry.is_empty() {
                    continue;
                }
                if target_host == entry
                    || target_host.ends_with(&format!(".{entry}"))
                    || entry.starts_with('.') && target_host.ends_with(&entry)
                {
                    opts.proxy = None;
                    opts.proxy_user = None;
                    break;
                }
            }
        }
    }

    // Extract userinfo (user:pass) from proxy URL if present and --proxy-user not set.
    if opts.proxy_user.is_none()
        && let Some(proxy) = &opts.proxy
    {
        let stripped = proxy
            .strip_prefix("http://")
            .or_else(|| proxy.strip_prefix("https://"))
            .unwrap_or(proxy);
        if let Some(at_pos) = stripped.find('@') {
            let userinfo = &stripped[..at_pos];
            if !userinfo.is_empty() {
                let decoded = crate::url::percent_decode(userinfo);
                // curl treats "user" as "user:" (empty password)
                let with_colon = if decoded.contains(':') {
                    decoded
                } else {
                    format!("{decoded}:")
                };
                opts.proxy_user = Some(with_colon);
            }
        }
    }

    // Normalize to a scheme-prefixed URL so relative-redirect resolution works
    // even if the input lacked a scheme (e.g. "host:port/path").
    // Detect scheme by looking for "scheme:[/]" at the start; parse_url handles
    // the single-slash "http:/host" form too, so don't prepend "http://" if we
    // already have a scheme-looking prefix.
    let has_scheme_prefix = url_str
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '+' && c != '-' && c != '.')
        .is_some_and(|i| i > 0 && url_str[i..].starts_with(':'));
    let mut current_url = if has_scheme_prefix {
        url_str.to_string()
    } else {
        format!("http://{url_str}")
    };
    // Normalize `..` / `.` segments in the initial URL (curl does this by default;
    // --path-as-is disables it).
    if !opts.path_as_is {
        current_url = normalize_url_path(&current_url);
    }
    let mut redirects = 0;
    let mut connects = 0;
    let mut redirect_headers: Vec<u8> = Vec::new();

    loop {
        let url = parse_url(&current_url)?;

        // Lift URL-embedded userinfo into opts.user so it persists across
        // relative-path redirects — those redirects build a new URL without
        // userinfo, so without this lift the Authorization header would
        // disappear on same-host redirects (test 2081). When a redirect
        // target carries its OWN userinfo with BOTH user and password (a
        // colon), that overrides what was lifted from the URL (test 899
        // — new credentials beat the original). `-u` from the CLI always
        // wins (test 979). A user-only userinfo (e.g.
        // `http://user1@host/`) does NOT override an existing opts.user
        // that already has a password (test 682 — netrc-derived password
        // must survive URL parsing).
        if let Some(ref ui) = url.userinfo
            && !opts.user_from_cli
        {
            let new_has_password = ui.contains(':');
            let existing_has_password = opts.user.as_deref().is_some_and(|s| s.contains(':'));
            if opts.user.is_none() || new_has_password || !existing_has_password {
                opts.user = Some(ui.clone());
            }
        }

        let mut resp = execute_request(&url, &opts, redirect_headers.len())?;
        // After issuing a request that used the stored Digest challenge
        // state (i.e. one set by an earlier 401 retry), bump nc so the next
        // request increments per RFC 2617 §3.2.2 (test 1286).
        if opts.digest_challenge_state.is_some() && opts.digest_authorization.is_none() {
            opts.digest_nc = opts.digest_nc.saturating_add(1).max(2);
        }
        // `num_connects` counts NEW TCP connects only — reused pool entries
        // don't bump it (test 2051's `%{num_connects}` is 0 on reuse).
        if !crate::connection::POOL_REUSED.with(|h| *h.borrow()) {
            connects += 1;
        }

        // Process Set-Cookie from this response BEFORE any retry-with-auth
        // path so the retry sees the new cookies (test 1331). The cookie
        // engine reprocesses the (final) response below; this earlier pass
        // is a no-op when no retry happens.
        absorb_response_cookies(&mut opts, &resp, &url);

        // --anyauth/--digest/--ntlm: when the server replies 401, parse the
        // challenge(s) and resend with the strongest scheme we support.
        // Digest is preferred over Basic (RFC 7235). NTLM is tried only when
        // --ntlm is in effect or --anyauth got an NTLM-only challenge.
        // After a redirect with --ntlm we re-enter even with `defer_auth =
        // false`, since the post-redirect challenge needs a fresh round-trip.
        let creds_present = opts.user.is_some() || url.userinfo.is_some();
        if resp.status == 401
            && creds_present
            && (opts.defer_auth || (opts.ntlm && opts.ntlm_authorization.is_none()))
        {
            // NTLM handling — split into the Type-2 case (challenge already
            // received) and the bare-NTLM case (we need to send Type 1).
            // WWW-Authenticate may be a single scheme per header OR multiple
            // comma-separated schemes in one header (test 76).
            let auth_schemes: Vec<String> = resp
                .headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("www-authenticate"))
                .flat_map(|(_, v)| v.split(',').map(|s| s.trim().to_string()))
                .collect();
            let offers_ntlm = auth_schemes.iter().any(|s| {
                let l = s.to_ascii_lowercase();
                l == "ntlm" || l.starts_with("ntlm ")
            });
            let offers_digest_now = auth_schemes
                .iter()
                .any(|s| s.to_ascii_lowercase().starts_with("digest "));
            // --anyauth without --digest/--ntlm explicit: choose NTLM only when
            // the server didn't also offer Digest (Digest still wins, per
            // test 70). Once chosen, set opts.ntlm so the NTLM Type-1 path
            // below sends the probe.
            if !opts.ntlm && offers_ntlm && !offers_digest_now {
                opts.ntlm = true;
            }
            if opts.ntlm && opts.ntlm_authorization.is_none() {
                let t2_b64 = resp.headers.iter().find_map(|(k, v)| {
                    if k.eq_ignore_ascii_case("www-authenticate") {
                        let t = v.trim_start();
                        t.strip_prefix("NTLM ")
                            .or_else(|| t.strip_prefix("ntlm "))
                            .map(str::trim)
                    } else {
                        None
                    }
                });
                let creds = opts
                    .user
                    .as_deref()
                    .or(url.userinfo.as_deref())
                    .and_then(|c| {
                        c.split_once(':').map(|(u, p)| (u.to_string(), p.to_string()))
                    });
                // A Type 2 too large to parse is curl's CURLE_TOO_LARGE
                // (test 776) — surface even if we can't extract the
                // challenge.
                if let Some(t2) = t2_b64
                    && t2.len() > 65_000
                {
                    resp.ntlm_too_large = true;
                    resp.body.clear();
                    return Ok(resp);
                }
                if let Some(t2) = t2_b64
                    && let Some(challenge) = crate::ntlm::parse_type2_challenge(t2)
                    && let Some((u, p)) = creds
                {
                    // NTLM is connection-bound. If the 401 carried
                    // `Connection: close` or the response was HTTP/1.0, a
                    // fresh TCP connect would invalidate the negotiation —
                    // curl just stops (test 159 is intentionally "known to
                    // fail" for that reason).
                    let conn_close = resp.headers.iter().any(|(k, v)| {
                        k.eq_ignore_ascii_case("connection")
                            && v.to_ascii_lowercase().contains("close")
                    });
                    if !conn_close && !resp.http10_response {
                        let t3 = match crate::ntlm::type3_message_checked(&u, &p, &challenge) {
                            Some(b) => b,
                            None => {
                                resp.ntlm_too_large = true;
                                resp.body.clear();
                                return Ok(resp);
                            }
                        };
                        let b64 = crate::ntlm::base64_encode(&t3);
                        redirect_headers.extend_from_slice(&resp.header_bytes);
                        opts.defer_auth = false;
                        opts.ntlm_authorization = Some(format!("NTLM {b64}"));
                        let r = execute_request(&url, &opts, redirect_headers.len())?;
                        resp = r;
                        connects += 1;
                        // Mark the connection as NTLM-authenticated so a
                        // follow-redirect on the same pooled connection
                        // doesn't re-trigger the Type 1 → Type 2 → Type 3
                        // dance (test 1100).
                        if resp.status != 401 {
                            opts.ntlm_done = true;
                            opts.ntlm_authorization = None;
                        }
                    }
                } else if offers_ntlm {
                    // Bare `WWW-Authenticate: NTLM` (no Type 2 yet) → retry
                    // with the NTLM Type 1 message. The server will respond
                    // with Type 2, handled on the next iteration of this loop.
                    redirect_headers.extend_from_slice(&resp.header_bytes);
                    opts.defer_auth = false;
                    let r = execute_request(&url, &opts, redirect_headers.len())?;
                    resp = r;
                    connects += 1;
                    if resp.status == 401 {
                        let t2_b64 = resp.headers.iter().find_map(|(k, v)| {
                            if k.eq_ignore_ascii_case("www-authenticate") {
                                let t = v.trim_start();
                                t.strip_prefix("NTLM ")
                                    .or_else(|| t.strip_prefix("ntlm "))
                                    .map(str::trim)
                            } else {
                                None
                            }
                        });
                        let creds2 = opts.user.as_deref().or(url.userinfo.as_deref()).and_then(
                            |c| c.split_once(':').map(|(u, p)| (u.to_string(), p.to_string())),
                        );
                        if let Some(t2) = t2_b64
                            && let Some(challenge) = crate::ntlm::parse_type2_challenge(t2)
                            && let Some((u, p)) = creds2
                        {
                            let t3 = match crate::ntlm::type3_message_checked(&u, &p, &challenge) {
                            Some(b) => b,
                            None => {
                                resp.ntlm_too_large = true;
                                resp.body.clear();
                                return Ok(resp);
                            }
                        };
                            let b64 = crate::ntlm::base64_encode(&t3);
                            redirect_headers.extend_from_slice(&resp.header_bytes);
                            opts.ntlm_authorization = Some(format!("NTLM {b64}"));
                            let r = execute_request(&url, &opts, redirect_headers.len())?;
                            resp = r;
                            connects += 1;
                        }
                    }
                }
            }
            let digest_challenge = resp.headers.iter().find_map(|(k, v)| {
                if k.eq_ignore_ascii_case("www-authenticate") {
                    let trimmed = v.trim_start();
                    if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("Digest ") {
                        return Some(trimmed[7..].to_string());
                    }
                }
                None
            });
            let offers_basic = resp.headers.iter().any(|(k, v)| {
                k.eq_ignore_ascii_case("www-authenticate")
                    && v.split(',').any(|tok| {
                        tok.trim()
                            .split_ascii_whitespace()
                            .next()
                            .is_some_and(|s| s.eq_ignore_ascii_case("basic"))
                    })
            });
            let auth_creds = opts.user.as_deref().or(url.userinfo.as_deref());
            let creds_parsed = auth_creds.and_then(|c| {
                c.split_once(':')
                    .map(|(u, p)| (u.to_string(), p.to_string()))
            });
            let digest_header_and_chal = digest_challenge.and_then(|chal| {
                creds_parsed.as_ref().and_then(|(u, p)| {
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
                    let uri = if url.path.is_empty() {
                        "/".to_string()
                    } else {
                        url.path.clone()
                    };
                    build_digest_auth_nc(u, p, &chal, &method, &uri, 1)
                        .map(|h| (h, chal))
                })
            });
            if let Some((header, chal)) = digest_header_and_chal {
                // If the upload source is stdin and HTTP/1.0 forces no
                // chunked encoding, we cannot replay the body — match curl's
                // CURLE_UPLOAD_FAILED (exit 25) and bail without re-sending
                // (test 1072).
                let is_stdin_upload = opts.upload_file.as_deref().and_then(|p| p.to_str())
                    == Some("-")
                    || opts.upload_file.as_deref().and_then(|p| p.to_str()) == Some(".");
                if resp.http10_response && is_stdin_upload {
                    resp.upload_redirect_failed = true;
                    return Ok(resp);
                }
                redirect_headers.extend_from_slice(&resp.header_bytes);
                opts.defer_auth = false;
                opts.digest_authorization = Some(header);
                opts.digest_challenge_state = Some(chal);
                opts.digest_nc = 2; // next request after the retry uses nc=2
                // If the server spoke HTTP/1.0 (e.g. `HTTP/1.0 401 ...
                // swsclose`), pin the retry to HTTP/1.0 since the connection
                // was closed and a fresh one has to use the same version
                // (test 1071, 1072).
                let saved_http_version = opts.http_version.clone();
                if resp.http10_response && opts.http_version.is_none() {
                    opts.http_version = Some("1.0".into());
                }
                let r = match execute_request(&url, &opts, redirect_headers.len()) {
                    Ok(r) => Ok(r),
                    Err(e) if is_got_nothing_error(&e) => {
                        // Server closed without sending a response to the
                        // authed retry. Keep the 401 headers in the redirect
                        // chain and surface CURLE_GOT_NOTHING via status=0
                        // (test 1079).
                        resp.status = 0;
                        resp.header_bytes.clear();
                        resp.body.clear();
                        Err(())
                    }
                    Err(e) => {
                        opts.http_version = saved_http_version;
                        return Err(e);
                    }
                };
                opts.http_version = saved_http_version;
                if let Ok(r) = r {
                    resp = r;
                    connects += 1;
                }
            } else if offers_basic {
                // Capture the 401 headers as part of the redirect chain so
                // -i shows both responses.
                redirect_headers.extend_from_slice(&resp.header_bytes);
                // Disable defer_auth on the live `opts` so subsequent
                // redirects in this same `perform()` call also send Basic
                // (test 1087, 1088).
                opts.defer_auth = false;
                resp = execute_request(&url, &opts, redirect_headers.len())?;
                connects += 1;
            }
        }

        // RFC 2617 stale=true: server says the nonce we used is now stale,
        // and includes a fresh one. Recompute Digest and retry once (test
        // 388). We get here when the previous request already sent Digest
        // (digest_authorization is set) and the response is another 401.
        if resp.status == 401
            && opts.digest_authorization.is_some()
            && (opts.user.is_some() || url.userinfo.is_some())
        {
            let stale_challenge = resp.headers.iter().find_map(|(k, v)| {
                if k.eq_ignore_ascii_case("www-authenticate") {
                    let trimmed = v.trim_start();
                    if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("Digest ") {
                        let rest = &trimmed[7..];
                        if parse_digest_attr(rest, "stale")
                            .map(|s| s.eq_ignore_ascii_case("true"))
                            .unwrap_or(false)
                        {
                            return Some(rest.to_string());
                        }
                    }
                }
                None
            });
            if let Some(chal) = stale_challenge {
                let auth_creds = opts.user.as_deref().or(url.userinfo.as_deref());
                let creds_parsed = auth_creds.and_then(|c| {
                    c.split_once(':')
                        .map(|(u, p)| (u.to_string(), p.to_string()))
                });
                if let Some((u, p)) = creds_parsed {
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
                    let uri = if url.path.is_empty() {
                        "/".to_string()
                    } else {
                        url.path.clone()
                    };
                    if let Some(header) = build_digest_auth(&u, &p, &chal, &method, &uri) {
                        redirect_headers.extend_from_slice(&resp.header_bytes);
                        opts.digest_authorization = Some(header);
                        resp = execute_request(&url, &opts, redirect_headers.len())?;
                        connects += 1;
                    }
                }
            }
        }

        // --digest/--ntlm with an upload probe: if the probe got a 2xx
        // (server accepted but didn't challenge), we still need to send the
        // real body in a second request — without an Authorization header
        // (tests 175, 176). A redirect (3xx) or other failure stops here
        // (test 177).
        if (200..300).contains(&resp.status)
            && opts.defer_auth
            && opts.auth_probe_empty_upload
            && (opts.upload_file.is_some() || opts.data.is_some())
            && opts.digest_authorization.is_none()
            && opts.ntlm_authorization.is_none()
        {
            redirect_headers.extend_from_slice(&resp.header_bytes);
            opts.defer_auth = false;
            opts.auth_probe_empty_upload = false;
            // Send the body but NO Authorization (server didn't ask).
            opts.no_basic = true;
            // Suppress the auto NTLM Type 1 on this no-auth retry by
            // pretending the connection is already authenticated.
            let saved_ntlm_done = opts.ntlm_done;
            if opts.ntlm {
                opts.ntlm_done = true;
            }
            resp = execute_request(&url, &opts, redirect_headers.len())?;
            opts.ntlm_done = saved_ntlm_done;
            opts.no_basic = false;
            connects += 1;
        }

        // Proxy 407 challenge with --proxy-anyauth/digest/ntlm/negotiate:
        // resend with Proxy-Authorization (Digest preferred, then Basic) and
        // any cookies the 407 set on the proxy response (test 1331, 168).
        // --proxy-ntlm: 407 carries Proxy-Authenticate: NTLM <b64-Type 2>.
        // Parse it and retry with Proxy-Authorization: NTLM <Type 3> (test 81).
        let did_proxy_auth_retry = resp.status == 407
            && opts.defer_proxy_auth
            && opts.proxy_user.is_some();
        if resp.status == 407
            && opts.proxy_ntlm
            && opts.proxy_ntlm_authorization.is_none()
            && opts.proxy_user.is_some()
        {
            let t2_b64 = resp.headers.iter().find_map(|(k, v)| {
                if k.eq_ignore_ascii_case("proxy-authenticate") {
                    let t = v.trim_start();
                    t.strip_prefix("NTLM ")
                        .or_else(|| t.strip_prefix("ntlm "))
                        .map(str::trim)
                } else {
                    None
                }
            });
            let proxy_creds = opts.proxy_user.as_deref().and_then(|c| {
                c.split_once(':').map(|(u, p)| (u.to_string(), p.to_string()))
            });
            if let Some(t2) = t2_b64
                && let Some(challenge) = crate::ntlm::parse_type2_challenge(t2)
                && let Some((u, p)) = proxy_creds
            {
                let t3 = match crate::ntlm::type3_message_checked(&u, &p, &challenge) {
                    Some(b) => b,
                    None => {
                        // NTLM credentials over the limit — surface as
                        // CURLE_TOO_LARGE (test 775, 776) but keep the 401
                        // response headers so they still appear in stdout/-i.
                        // The body is discarded to match curl which stops
                        // writing as soon as it decides the auth retry can't
                        // proceed.
                        resp.ntlm_too_large = true;
                        resp.body.clear();
                        return Ok(resp);
                    }
                };
                let b64 = crate::ntlm::base64_encode(&t3);
                redirect_headers.extend_from_slice(&resp.header_bytes);
                opts.proxy_ntlm_authorization = Some(format!("NTLM {b64}"));
                let r = execute_request(&url, &opts, redirect_headers.len())?;
                resp = r;
                connects += 1;
                // Type 3 succeeded — proxy is now authenticated for the
                // remainder of this TCP connection. Drop the stored header
                // AND the auto-Type-1 trigger so subsequent requests don't
                // repeat either (test 169's site 401-Digest retry must not
                // carry Proxy-Authorization: NTLM).
                if resp.status != 407 {
                    opts.proxy_ntlm_authorization = None;
                    opts.proxy_ntlm_done = true;
                }
                // After proxy NTLM finishes, the site may issue 401 with its
                // own auth challenge. Run the site digest/basic flow against
                // the new response (test 169 chains proxy NTLM → site Digest).
                if resp.status == 401
                    && (opts.user.is_some() || url.userinfo.is_some())
                {
                    let site_digest = resp.headers.iter().find_map(|(k, v)| {
                        if k.eq_ignore_ascii_case("www-authenticate") {
                            let t = v.trim_start();
                            if t.len() >= 7 && t[..7].eq_ignore_ascii_case("Digest ") {
                                return Some(t[7..].to_string());
                            }
                        }
                        None
                    });
                    let site_creds = opts
                        .user
                        .as_deref()
                        .or(url.userinfo.as_deref())
                        .and_then(|c| {
                            c.split_once(':').map(|(u, p)| (u.to_string(), p.to_string()))
                        });
                    if let Some(chal) = site_digest
                        && let Some((u, p)) = site_creds
                    {
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
                        let uri = if url.path.is_empty() {
                            "/".to_string()
                        } else {
                            url.path.clone()
                        };
                        if let Some(header) =
                            build_digest_auth_nc(&u, &p, &chal, &method, &uri, 1)
                        {
                            redirect_headers.extend_from_slice(&resp.header_bytes);
                            opts.defer_auth = false;
                            opts.digest_authorization = Some(header);
                            opts.digest_challenge_state = Some(chal);
                            opts.digest_nc = 2;
                            let r =
                                execute_request(&url, &opts, redirect_headers.len())?;
                            resp = r;
                            connects += 1;
                        }
                    }
                }
            }
        }
        if resp.status == 407 && opts.defer_proxy_auth && opts.proxy_user.is_some() {
            let proxy_digest_challenge = resp.headers.iter().find_map(|(k, v)| {
                if k.eq_ignore_ascii_case("proxy-authenticate") {
                    let trimmed = v.trim_start();
                    if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("Digest ") {
                        return Some(trimmed[7..].to_string());
                    }
                }
                None
            });
            let proxy_offers_ntlm = resp.headers.iter().any(|(k, v)| {
                k.eq_ignore_ascii_case("proxy-authenticate") && {
                    let t = v.trim_start().to_ascii_lowercase();
                    t == "ntlm" || t.starts_with("ntlm ")
                }
            });
            // --proxy-anyauth: if Digest isn't offered but NTLM is, switch
            // to NTLM and continue with the Type 1 → Type 2 → Type 3 dance
            // inline (test 243). Done here so we don't have to re-enter the
            // outer auth loop.
            if proxy_digest_challenge.is_none() && proxy_offers_ntlm && !opts.proxy_ntlm {
                opts.proxy_ntlm = true;
                opts.defer_proxy_auth = false;
                redirect_headers.extend_from_slice(&resp.header_bytes);
                // Type 1 retry.
                let r = execute_request(&url, &opts, redirect_headers.len())?;
                resp = r;
                connects += 1;
                if resp.status == 407 {
                    let t2_b64 = resp.headers.iter().find_map(|(k, v)| {
                        if k.eq_ignore_ascii_case("proxy-authenticate") {
                            let t = v.trim_start();
                            t.strip_prefix("NTLM ")
                                .or_else(|| t.strip_prefix("ntlm "))
                                .map(str::trim)
                        } else {
                            None
                        }
                    });
                    let proxy_creds2 = opts.proxy_user.as_deref().and_then(|c| {
                        c.split_once(':').map(|(u, p)| (u.to_string(), p.to_string()))
                    });
                    if let Some(t2) = t2_b64
                        && let Some(challenge) = crate::ntlm::parse_type2_challenge(t2)
                        && let Some((u, p)) = proxy_creds2
                    {
                        let t3 = match crate::ntlm::type3_message_checked(&u, &p, &challenge) {
                            Some(b) => b,
                            None => {
                                resp.ntlm_too_large = true;
                                resp.body.clear();
                                return Ok(resp);
                            }
                        };
                        let b64 = crate::ntlm::base64_encode(&t3);
                        redirect_headers.extend_from_slice(&resp.header_bytes);
                        opts.proxy_ntlm_authorization = Some(format!("NTLM {b64}"));
                        let r = execute_request(&url, &opts, redirect_headers.len())?;
                        resp = r;
                        connects += 1;
                        if resp.status != 407 {
                            opts.proxy_ntlm_authorization = None;
                            opts.proxy_ntlm_done = true;
                        }
                    }
                }
            }
            let offers_basic = resp.headers.iter().any(|(k, v)| {
                k.eq_ignore_ascii_case("proxy-authenticate")
                    && v.split(',').any(|tok| {
                        tok.trim()
                            .split_ascii_whitespace()
                            .next()
                            .is_some_and(|s| s.eq_ignore_ascii_case("basic"))
                    })
            });
            let proxy_creds = opts
                .proxy_user
                .as_deref()
                .and_then(|c| c.split_once(':').map(|(u, p)| (u.to_string(), p.to_string())));
            let proxy_digest_header = proxy_digest_challenge.and_then(|chal| {
                proxy_creds.as_ref().and_then(|(u, p)| {
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
                    let uri = if url.path.is_empty() {
                        "/".to_string()
                    } else {
                        url.path.clone()
                    };
                    build_digest_auth(u, p, &chal, &method, &uri)
                })
            });
            if let Some(header) = proxy_digest_header {
                redirect_headers.extend_from_slice(&resp.header_bytes);
                opts.defer_proxy_auth = false;
                opts.proxy_digest_authorization = Some(header);
                resp = execute_request(&url, &opts, redirect_headers.len())?;
                connects += 1;
            } else if offers_basic {
                redirect_headers.extend_from_slice(&resp.header_bytes);
                opts.defer_proxy_auth = false;
                // Apply cookies that came in via the 407 response — the
                // cookie engine may have stored them on the matching host.
                resp = execute_request(&url, &opts, redirect_headers.len())?;
                connects += 1;
            }
        }

        // After a proxy auth retry, the server may still require auth (401).
        // Re-run the 401 handler so we add Authorization: Digest on top of
        // the now-permanent Proxy-Authorization: Digest (test 168).
        if did_proxy_auth_retry
            && resp.status == 401
            && opts.defer_auth
            && (opts.user.is_some() || url.userinfo.is_some())
        {
            let digest_challenge = resp.headers.iter().find_map(|(k, v)| {
                if k.eq_ignore_ascii_case("www-authenticate") {
                    let trimmed = v.trim_start();
                    if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("Digest ") {
                        return Some(trimmed[7..].to_string());
                    }
                }
                None
            });
            let auth_creds = opts.user.as_deref().or(url.userinfo.as_deref());
            let creds_parsed = auth_creds.and_then(|c| {
                c.split_once(':')
                    .map(|(u, p)| (u.to_string(), p.to_string()))
            });
            let digest_header = digest_challenge.and_then(|chal| {
                creds_parsed.as_ref().and_then(|(u, p)| {
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
                    let uri = if url.path.is_empty() {
                        "/".to_string()
                    } else {
                        url.path.clone()
                    };
                    build_digest_auth(u, p, &chal, &method, &uri)
                })
            });
            if let Some(header) = digest_header {
                redirect_headers.extend_from_slice(&resp.header_bytes);
                opts.defer_auth = false;
                opts.digest_authorization = Some(header);
                resp = execute_request(&url, &opts, redirect_headers.len())?;
                connects += 1;
            }
        }

        // When the cookie engine is active, accumulate Set-Cookie response
        // headers so subsequent redirects within this perform() call (and
        // future per-URL cookie lookups in main.rs) see them. The same logic
        // runs again in main.rs against the final response — that's a no-op
        // when the cookie was already captured here.
        if opts.cookie_engine {
            let host_for_cookies = opts
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("host"))
                .map(|(_, v)| v.split(':').next().unwrap_or(v).to_string())
                .unwrap_or_else(|| url.host.clone());
            for (k, v) in &resp.headers {
                if k == "set-cookie"
                    && let Some(line) =
                        crate::cookie::format_cookie_line(v, &url, &host_for_cookies)
                {
                    let fields: Vec<&str> = line.split('\t').collect();
                    if fields.len() >= 7 {
                        let new_dom = fields[0].strip_prefix("#HttpOnly_").unwrap_or(fields[0]);
                        let np = fields[2];
                        let nn = fields[5];
                        // Secure-cookie protection: a non-secure source cannot
                        // shadow an existing secure cookie with the same name
                        // (test 414, RFC 6265bis §5.5.12.4 with curl's broader
                        // name-only match).
                        let new_secure = fields[3].eq_ignore_ascii_case("TRUE");
                        let from_https = url.scheme == "https";
                        if !from_https && !new_secure {
                            let blocked = opts.memory_cookies.iter().any(|existing| {
                                let ef: Vec<&str> = existing.split('\t').collect();
                                ef.len() >= 7 && ef[5] == nn && ef[3].eq_ignore_ascii_case("TRUE")
                            });
                            if blocked {
                                continue;
                            }
                        }
                        opts.memory_cookies.retain(|existing| {
                            let ef: Vec<&str> = existing.split('\t').collect();
                            if ef.len() >= 7 {
                                let ed = ef[0].strip_prefix("#HttpOnly_").unwrap_or(ef[0]);
                                !(ed == new_dom && ef[2] == np && ef[5] == nn)
                            } else {
                                true
                            }
                        });
                    }
                    if !crate::cookie::is_jar_line_expired(&line) {
                        // Cap at 50 cookies per domain (matches curl's
                        // CMAX_COOKIES_PER_DOMAIN; new entries past the cap are
                        // dropped — test 444).
                        let new_dom = line
                            .split('\t')
                            .next()
                            .map(|s| s.strip_prefix("#HttpOnly_").unwrap_or(s).to_string())
                            .unwrap_or_default();
                        let cnt = opts
                            .memory_cookies
                            .iter()
                            .filter(|c| {
                                c.split('\t')
                                    .next()
                                    .map(|s| {
                                        s.strip_prefix("#HttpOnly_").unwrap_or(s)
                                            == new_dom.as_str()
                                    })
                                    .unwrap_or(false)
                            })
                            .count();
                        if cnt < 50 {
                            opts.memory_cookies.push(line);
                        }
                    }
                }
            }
        }

        // Handle redirects.
        if opts.location && (301..=308).contains(&resp.status) {
            if redirects >= opts.max_redirs {
                // Max redirects reached — return the last response so its headers
                // and body still show up in output. main.rs maps `max_redirects`
                // to exit code 47.
                let redirect_url = resp
                    .headers
                    .iter()
                    .find(|(k, _)| k == "location")
                    .map(|(_, loc)| resolve_redirect(&url, loc.trim()));
                let mut final_resp = resp;
                final_resp.redirect_headers = redirect_headers;
                final_resp.num_connects = connects;
                final_resp.num_redirects = redirects;
                final_resp.max_redirects_reached = true;
                final_resp.final_url = Some(current_url.clone());
                final_resp.final_referer = opts.referer.clone();
                final_resp.redirect_url = redirect_url;
                return Ok(final_resp);
            }
            if let Some((_, location)) = resp.headers.iter().find(|(k, _)| k == "location") {
                // Skip blank Location headers.
                let location = location.trim();
                if location.is_empty() {
                    let mut final_resp = resp;
                    final_resp.redirect_headers = redirect_headers;
                    final_resp.num_connects = connects;
                    final_resp.num_redirects = redirects;
                    final_resp.final_url = Some(current_url.clone());
                    final_resp.final_referer = opts.referer.clone();
                    return Ok(final_resp);
                }

                redirects += 1;

                // Collect intermediate response headers for -i output.
                let prev_redirect_headers_len = redirect_headers.len();
                redirect_headers.extend_from_slice(&resp.header_bytes);

                // Percent-encode spaces in the Location URL.
                // In the query string portion, use + for spaces; in the path, use %20.
                let location = if let Some(qpos) = location.find('?') {
                    let path_part = location[..qpos].replace(' ', "%20");
                    let query_part = location[qpos..].replace(' ', "+");
                    format!("{path_part}{query_part}")
                } else {
                    location.replace(' ', "%20")
                };

                // Resolve relative URLs per RFC 3986 §5.2. For a relative path,
                // strip the query from the base and replace the last path segment.
                // Any "scheme://..." form (even unsupported schemes like
                // gopher://) is treated as absolute so it is later rejected as
                // an unsupported protocol rather than appended to the base
                // path (test 1563).
                let abs_scheme = location.split_once("://").is_some_and(|(s, _)| {
                    !s.is_empty()
                        && s.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
                });
                if abs_scheme {
                    current_url = location;
                } else if location.starts_with("//") {
                    // Protocol-relative URL — keep the base scheme, replace
                    // host+path with what follows the leading `//`.
                    current_url = format!("{}:{location}", url.scheme);
                } else if location.starts_with('/') {
                    current_url = format!("{}://{}:{}{}", url.scheme, url.host, url.port, location);
                } else if location.starts_with('?') {
                    // Location is a query-only reference — per RFC 3986 §5.2.3,
                    // keep the base path (including its filename) and replace
                    // only the query component.
                    let path_no_query = url.path.split('?').next().unwrap_or(&url.path);
                    current_url = format!(
                        "{}://{}:{}{path_no_query}{location}",
                        url.scheme, url.host, url.port
                    );
                } else {
                    // Relative path — strip base's filename, strip its query,
                    // then merge.
                    let path_no_query = url.path.split('?').next().unwrap_or(&url.path);
                    let base_path = match path_no_query.rfind('/') {
                        Some(i) => &path_no_query[..=i],
                        None => "/",
                    };
                    current_url = format!(
                        "{}://{}:{}{base_path}{location}",
                        url.scheme, url.host, url.port
                    );
                }
                // Normalize redirect URL (resolve ../ ./ segments per RFC 3986).
                // --path-as-is only affects the *initial* URL, not relative-URL
                // resolution during redirects.
                current_url = normalize_url_path(&current_url);

                // If the redirect target itself is malformed (e.g. four
                // slashes after the scheme like `http:////host`), bail out
                // with the original 3xx response and let main.rs map this to
                // CURLE_URL_MALFORMAT (exit 3, test 1142). Unsupported schemes
                // (e.g. `gopher://`) keep flowing so test 1563 still gets the
                // proper "unsupported protocol" exit 1.
                if matches!(parse_url(&current_url), Err(ref e) if e.starts_with("malformed URL")) {
                    // Drop the current response's headers from the
                    // accumulated redirect_headers (we add them back via
                    // final_resp.header_bytes below) so the 3xx response is
                    // not emitted twice.
                    redirect_headers.truncate(prev_redirect_headers_len);
                    let mut final_resp = resp;
                    final_resp.redirect_headers = redirect_headers;
                    final_resp.num_connects = connects;
                    final_resp.num_redirects = redirects - 1;
                    final_resp.redirect_url_malformed = true;
                    final_resp.final_url = Some(current_url.clone());
                    final_resp.final_referer = opts.referer.clone();
                    final_resp.redirect_url = Some(current_url);
                    return Ok(final_resp);
                }

                // --proto-redir: check whether the redirect target's scheme is allowed.
                if let Some(spec) = &opts.proto_redir {
                    let new_scheme = current_url
                        .split_once("://")
                        .map(|(s, _)| s.to_ascii_lowercase())
                        .unwrap_or_default();
                    let mut allow_http = true;
                    let mut allow_https = true;
                    for tok in spec.split(',') {
                        let tok = tok.trim();
                        if tok.is_empty() {
                            continue;
                        }
                        let (op, name) = match tok.as_bytes()[0] {
                            b'+' => ('+', &tok[1..]),
                            b'-' => ('-', &tok[1..]),
                            b'=' => ('=', &tok[1..]),
                            _ => ('+', tok),
                        };
                        let name = name.to_ascii_lowercase();
                        if op == '=' {
                            allow_http = false;
                            allow_https = false;
                        }
                        let val = op != '-';
                        match name.as_str() {
                            "all" => {
                                allow_http = val;
                                allow_https = val;
                            }
                            "none" => {
                                allow_http = false;
                                allow_https = false;
                            }
                            "http" => allow_http = val,
                            "https" => allow_https = val,
                            _ => {}
                        }
                    }
                    let allowed = match new_scheme.as_str() {
                        "http" => allow_http,
                        "https" => allow_https,
                        _ => true,
                    };
                    if !allowed {
                        // Block: drop the current resp from accumulated
                        // redirect_headers so it is not duplicated, then
                        // return the resp itself with proto_redir_blocked set.
                        redirect_headers.truncate(prev_redirect_headers_len);
                        let mut final_resp = resp;
                        final_resp.redirect_headers = redirect_headers;
                        final_resp.num_connects = connects;
                        final_resp.num_redirects = redirects - 1;
                        final_resp.proto_redir_blocked = true;
                        final_resp.final_url = Some(current_url.clone());
                        final_resp.final_referer = opts.referer.clone();
                        final_resp.redirect_url = Some(current_url.clone());
                        return Ok(final_resp);
                    }
                }

                // RFC 7231: on 301/302, a POST becomes a GET (and its body is
                // dropped) unless the user opts in via --post301/--post302 to
                // preserve the POST. PUT is preserved by default (test 1051).
                // 303 ALWAYS converts to GET (POST or PUT) unless --post303 is
                // set on a POST (tests 1332, 1524).
                if matches!(resp.status, 301..=303) {
                    let is_post = opts.data.is_some() || !opts.form_fields.is_empty();
                    let is_put = opts.upload_file.is_some();
                    let preserve = match resp.status {
                        301 => opts.post301,
                        302 => opts.post302,
                        303 => opts.post303 && is_post,
                        _ => false,
                    };
                    // 303 converts both POST and PUT; 301/302 only convert POST.
                    let convert = !preserve
                        && match resp.status {
                            301 | 302 => is_post,
                            303 => is_post || is_put,
                            _ => false,
                        };
                    if convert {
                        opts.data = None;
                        opts.form_fields.clear();
                        opts.upload_file = None;
                        opts.method = Some("GET".to_string());
                    }
                }

                // Stdin-source uploads can't be replayed on a redirect.
                // Return the response we already have (with -i in the test
                // framework, its headers go to stdout) and flag the failure
                // so main.rs can set exit 25 (test 1073).
                let upload_from_stdin = matches!(
                    opts.upload_file.as_deref().and_then(|p| p.to_str()),
                    Some("-" | ".")
                );
                if upload_from_stdin {
                    redirect_headers.truncate(prev_redirect_headers_len);
                    let mut final_resp = resp;
                    final_resp.redirect_headers = redirect_headers;
                    final_resp.num_connects = connects;
                    final_resp.num_redirects = redirects - 1;
                    final_resp.upload_redirect_failed = true;
                    final_resp.final_url = Some(url_str.to_string());
                    final_resp.final_referer = opts.referer.clone();
                    final_resp.redirect_url = Some(current_url.clone());
                    return Ok(final_resp);
                }

                // If the redirect target is on a different host, drop any
                // custom Host header so it is not forwarded to the new host.
                // Also drop credentials (Authorization header + -u user/pass)
                // unless the user opted in via --location-trusted, mirroring
                // curl's CURLOPT_UNRESTRICTED_AUTH semantics.
                if let Ok(new_url) = parse_url(&current_url)
                    && (!url.host.eq_ignore_ascii_case(&new_url.host)
                        || url.port != new_url.port
                        || url.scheme != new_url.scheme)
                {
                    opts.headers
                        .retain(|(k, _)| !k.eq_ignore_ascii_case("host"));
                    // Drop custom Cookie headers on cross-host redirects —
                    // curl does not forward user-supplied cookies to other hosts.
                    opts.headers
                        .retain(|(k, _)| !k.eq_ignore_ascii_case("cookie"));
                    if !opts.location_trusted {
                        opts.headers
                            .retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
                        opts.user = None;
                        opts.oauth2_bearer = None;
                    } else if opts.user.is_none()
                        && new_url.userinfo.is_none()
                        && let Some(ui) = url.userinfo.clone()
                    {
                        // --location-trusted: propagate URL-userinfo credentials
                        // across cross-host redirects when the target URL has no
                        // userinfo of its own.
                        opts.user = Some(ui);
                    }
                    // If `--netrc` is on and we just dropped credentials, try
                    // to recover them from the netrc file for the new host
                    // (test 257, 478).
                    if opts.netrc_mode != 0 && opts.user.is_none() {
                        let path = opts.netrc_file.clone().or_else(|| {
                            std::env::var_os("HOME")
                                .map(|h| std::path::PathBuf::from(h).join(".netrc"))
                        });
                        if let Some(p) = path
                            && let Ok(Some((login, password))) =
                                crate::netrc::lookup(&p, &new_url.host, None)
                            && let Some(u) = login
                        {
                            let pw = password.unwrap_or_default();
                            opts.user = Some(format!("{u}:{pw}"));
                        }
                    }
                }

                // --referer "...;auto" updates Referer to the URL we just
                // came from on each redirect (test 1067).
                if opts.auto_referer {
                    opts.referer = Some(format!(
                        "{}://{}:{}{}",
                        url.scheme, url.host, url.port, url.path
                    ));
                }

                // Clear the one-shot Digest header so the new URL re-derives
                // it via digest_challenge_state with an incremented nc and
                // the new URI (test 1286).
                opts.digest_authorization = None;
                // NTLM is per-connection — a redirect on a connection that
                // is closing (`Connection: close` or HTTP/1.0) restarts the
                // negotiation (Type 1 → Type 2 → Type 3). When the
                // connection persists (test 1100), NTLM auth carries over
                // and the redirected request sends no Authorization at all.
                let redir_closes = resp.headers.iter().any(|(k, v)| {
                    k.eq_ignore_ascii_case("connection")
                        && v.to_ascii_lowercase().contains("close")
                }) || resp.http10_response;
                if redir_closes {
                    opts.ntlm_authorization = None;
                    opts.proxy_ntlm_authorization = None;
                    opts.ntlm_done = false;
                    opts.proxy_ntlm_done = false;
                }
                // For --anyauth we re-probe the new origin no matter what
                // (the new URL could advertise different schemes — test 90).
                if opts.anyauth {
                    opts.ntlm = false;
                    opts.defer_auth = true;
                }
                if opts.verbose {
                    eprintln!("* Following redirect to {current_url}");
                }
                continue;
            }
        }

        // Compute redirect_url for %{redirect_url} — the URL that would be
        // followed if redirects were enabled. Only meaningful on 3xx responses.
        let redirect_url = if (301..=308).contains(&resp.status) {
            resp.headers
                .iter()
                .find(|(k, _)| k == "location")
                .map(|(_, loc)| resolve_redirect(&url, loc.trim()))
        } else {
            None
        };

        let mut final_resp = resp;
        final_resp.redirect_headers = redirect_headers;
        final_resp.num_connects = connects;
        final_resp.num_redirects = redirects;
        final_resp.final_url = Some(current_url.clone());
        final_resp.final_referer = opts.referer.clone();
        final_resp.redirect_url = redirect_url;
        return Ok(final_resp);
    }
}

/// Resolve a Location header value against a base URL, producing the absolute
/// URL that would be followed. Mirrors the RFC 3986 §5.2 logic used during
/// redirect following.
fn resolve_redirect(url: &ParsedUrl, location: &str) -> String {
    // Any "scheme://..." form is treated as an absolute URL — even when the
    // scheme is one we don't support (test 1159's `ht3p://...`). curl returns
    // the literal Location value via %{redirect_url} in that case.
    if let Some((scheme, rest)) = location.split_once("://") {
        let scheme_ok = !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.');
        if scheme_ok {
            if (scheme == "http" || scheme == "https") && !rest.contains('/') {
                return format!("{location}/");
            }
            return location.to_string();
        }
    }
    if let Some(rest) = location.strip_prefix("//") {
        return format!("{}://{}", url.scheme, rest);
    }
    let base = format!("{}://{}:{}", url.scheme, url.host, url.port);
    if location.starts_with('/') {
        return normalize_url_path(&format!("{base}{location}"));
    }
    let path_no_query = url.path.split('?').next().unwrap_or(&url.path);
    if location.starts_with('?') {
        return normalize_url_path(&format!("{base}{path_no_query}{location}"));
    }
    let base_path = match path_no_query.rfind('/') {
        Some(i) => &path_no_query[..=i],
        None => "/",
    };
    normalize_url_path(&format!("{base}{base_path}{location}"))
}
