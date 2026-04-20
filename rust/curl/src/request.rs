use std::fs;
use std::io::{self, Read, Write};

use crate::connection::connect;
use crate::format::base64_encode;
use crate::options::Options;
use crate::response::{Response, read_response};
use crate::url::{ParsedUrl, normalize_url_path, parse_url};

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
                if url.port == default_port {
                    req.push_str(&format!("Host: {}\r\n", url.host));
                } else {
                    req.push_str(&format!("Host: {}:{}\r\n", url.host, url.port));
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
        return req.into_bytes();
    }

    // When uploading with -T and the URL path ends with '/', append the basename of the
    // uploaded file (with URL-unsafe characters percent-encoded) — matches curl behavior.
    let mut path = url.path.clone();
    if let Some(ref upload) = opts.upload_file
        && path.ends_with('/')
        && upload.to_str() != Some("-")
        && let Some(name) = upload.file_name().and_then(|n| n.to_str())
    {
        path.push_str(&encode_path_component(name));
    }

    // When going through an HTTP proxy, use the full absolute URL in the request line.
    let request_target = if opts.proxy.is_some() && url.scheme == "http" && !opts.proxy_tunnel {
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
            if url.port == default_port {
                req.push_str(&format!("Host: {}\r\n", url.host));
            } else {
                req.push_str(&format!("Host: {}:{}\r\n", url.host, url.port));
            }
        }
    }

    // Proxy-Authorization — sent when --proxy-user is set and we're going through a proxy.
    // curl sends Proxy-Authorization before site Authorization.
    if let Some(ref proxy_user) = opts.proxy_user
        && opts.proxy.is_some()
        && !opts.proxy_tunnel
    {
        let encoded = base64_encode(proxy_user.as_bytes());
        req.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
    }

    // Basic auth — curl sends Authorization right after Host.
    // Prefer -u / --user; otherwise fall back to userinfo from the URL
    // (e.g. http://user:pass@host.example/path). Skip entirely when --anyauth/--digest/
    // --ntlm is set (curl waits for server challenge).
    let auth_user = opts.user.as_deref().or(url.userinfo.as_deref());
    if let Some(user) = auth_user
        && !opts.defer_auth
    {
        let encoded = base64_encode(user.as_bytes());
        req.push_str(&format!("Authorization: Basic {encoded}\r\n"));
    }

    // Range / Content-Range — curl sends these early, before User-Agent.
    let is_upload = opts.upload_file.is_some();
    if let Some(ref range) = opts.range {
        // Append "-" if the range has no dash (e.g. "-r 4" → "Range: bytes=4-").
        let range_suffix = if range.contains('-') { "" } else { "-" };
        req.push_str(&format!("Range: bytes={range}{range_suffix}\r\n"));
    } else if let Some(ref resume) = opts.resume_from
        && resume != "-"
    {
        if is_upload {
            // PUT with -C N: send Content-Range: bytes N-END/TOTAL
            if let Some(ref path) = opts.upload_file
                && let Ok(meta) = fs::metadata(path)
            {
                let total = meta.len();
                let start: u64 = resume.parse().unwrap_or(0);
                if start < total {
                    let end = total - 1;
                    req.push_str(&format!("Content-Range: bytes {start}-{end}/{total}\r\n"));
                }
            }
        } else {
            // GET with -C N: send Range: bytes=N-
            req.push_str(&format!("Range: bytes={resume}-\r\n"));
        }
    }

    // Check if custom headers override defaults.
    let has_custom = |name: &str| {
        opts.headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case(name))
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

    // Proxy-Connection header — curl sends this for HTTP proxy requests.
    if opts.proxy.is_some() && url.scheme == "http" && !opts.proxy_tunnel && !opts.no_keepalive {
        req.push_str("Proxy-Connection: Keep-Alive\r\n");
    }

    // Connection header — only send "close" when explicitly requested.
    // Real curl defaults to keep-alive (implicit in HTTP/1.1).
    if opts.no_keepalive {
        req.push_str("Connection: close\r\n");
    }

    // Accept-Encoding.
    if opts.compressed {
        req.push_str("Accept-Encoding: gzip, deflate\r\n");
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
    let cookie_header =
        build_cookie_header(cookie_match_host, &url.path, url.scheme == "https", opts);
    if !cookie_header.is_empty() {
        req.push_str(&format!("Cookie: {cookie_header}\r\n"));
    }

    // Chunked request encoding: either the user set `Transfer-Encoding:
    // chunked` explicitly, or we inferred it because the body length is
    // unknown ahead of time (stdin upload via `-T -` on HTTP/1.1).
    let is_stdin_upload = opts.upload_file.as_deref().and_then(|p| p.to_str()) == Some("-");
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

    // Custom headers (may override defaults).
    // - Empty value (e.g. -H "X-Header:") removes/suppresses the header entirely.
    // - "\x00" marker (from -H "X-Header;") sends header with no value.
    // - Normal value sends "Key: value\r\n".
    // Skip the Host header here — it was already emitted above.
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

    // Body handling — generate boundary once and reuse for header + body.
    let boundary = if !opts.form_fields.is_empty() {
        Some(multipart_boundary())
    } else {
        None
    };
    let body = build_body(opts, boundary.as_deref());

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
        let emit_content_length = !has_content_length && !user_chunked;

        // Emit Content-Length first.
        if emit_content_length {
            req.push_str(&content_len_hdr);
        }

        // curl sends Expect: 100-continue for upload bodies >= 1 MiB or when
        // uploading from stdin (-T -). Skip for HTTP/1.0 or if user suppressed it.
        // See lib/http.h EXPECT_100_THRESHOLD in curl source.
        if !has_expect && !http10 && (is_stdin_upload || body.len() >= 1024 * 1024) {
            req.push_str("Expect: 100-continue\r\n");
        }

        // Content-Type last.
        if !has_content_type {
            if let Some(ref b) = boundary {
                req.push_str(&format!(
                    "Content-Type: multipart/form-data; boundary={b}\r\n"
                ));
            } else if opts.data.is_some() {
                req.push_str("Content-Type: application/x-www-form-urlencoded\r\n");
            }
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
    bytes
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

    if let Some(boundary) = boundary {
        let mut body = Vec::new();
        for field in &opts.form_fields {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            if field.is_file {
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
                let safe_filename = mime_percent_encode(display_filename);
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"; filename=\"{safe_filename}\"\r\n\
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
            } else {
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"\r\n",
                        field.name
                    )
                    .as_bytes(),
                );
                if let Some(ref ct) = field.content_type {
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
        if path.to_str() == Some("-") {
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
fn build_cookie_header(host: &str, path: &str, secure_req: bool, opts: &Options) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let request_host = host.to_lowercase();

    // (path_length, domain_length, name_length, name=value) — kept for sort by specificity.
    let mut file_pairs: Vec<(usize, usize, usize, String)> = Vec::new();
    let mut inline_pairs: Vec<String> = Vec::new();

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
                                    if !path.starts_with(cookie_path) {
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

                    if expiry == 0 && opts.junk_session_cookies {
                        continue;
                    }
                    if expiry > 0 && expiry < now {
                        continue;
                    }
                    if secure && !secure_req {
                        continue;
                    }
                    if !domain_matches(&request_host, domain) {
                        continue;
                    }
                    if !path.starts_with(cookie_path) {
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
        if expiry > 0 && expiry < now {
            continue;
        }
        if secure && !secure_req {
            continue;
        }
        if !domain_matches(&request_host, domain) {
            continue;
        }
        if !path.starts_with(cookie_path) {
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

fn execute_request(url: &ParsedUrl, opts: &Options) -> Result<Response, String> {
    let (mut conn, connect_response) = connect(url, opts)?;
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

    let is_head = opts.head
        || opts
            .method
            .as_deref()
            .map(|m| m.eq_ignore_ascii_case("HEAD"))
            .unwrap_or(false);
    let mut resp = read_response(&mut conn, is_head, opts.http09, opts.compressed)?;

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
        let mut combined = connect_response;
        combined.extend_from_slice(&resp.header_bytes);
        resp.header_bytes = combined;
    }

    Ok(resp)
}

pub(crate) fn perform(url_str: &str, opts: &Options) -> Result<Response, String> {
    let mut opts = opts.clone();
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

    // NO_PROXY / no_proxy: bypass proxy for matching hosts.
    if opts.proxy.is_some() {
        let no_proxy = std::env::var("no_proxy")
            .or_else(|_| std::env::var("NO_PROXY"))
            .unwrap_or_default();
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

        let resp = execute_request(&url, &opts)?;
        connects += 1;

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
                    return Ok(final_resp);
                }

                redirects += 1;

                // Collect intermediate response headers for -i output.
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
                if location.starts_with("http://") || location.starts_with("https://") {
                    current_url = location;
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

                // RFC 7231: on 301/302/303, a POST becomes a GET (and its body
                // is dropped) unless the user explicitly opts in via --post301,
                // --post302, --post303. We don't implement those flags yet, so
                // convert unconditionally.
                if matches!(resp.status, 301..=303) {
                    opts.data = None;
                    opts.form_fields.clear();
                    opts.upload_file = None;
                    opts.method = Some("GET".to_string());
                }

                // If the redirect target is on a different host, drop any
                // custom Host header so it is not forwarded to the new host.
                if let Ok(new_url) = parse_url(&current_url)
                    && !url.host.eq_ignore_ascii_case(&new_url.host)
                {
                    opts.headers
                        .retain(|(k, _)| !k.eq_ignore_ascii_case("host"));
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
        final_resp.redirect_url = redirect_url;
        return Ok(final_resp);
    }
}

/// Resolve a Location header value against a base URL, producing the absolute
/// URL that would be followed. Mirrors the RFC 3986 §5.2 logic used during
/// redirect following.
fn resolve_redirect(url: &ParsedUrl, location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        // If the absolute URL has no path (only scheme://host[:port]), append
        // a trailing "/" so it matches curl's normalized form.
        let after_scheme = location
            .split_once("://")
            .map(|(_, r)| r)
            .unwrap_or(location);
        if !after_scheme.contains('/') {
            return format!("{location}/");
        }
        return location.to_string();
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
