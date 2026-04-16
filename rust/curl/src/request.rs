use std::fs;
use std::io::{self, Read, Write};

use crate::connection::connect;
use crate::format::base64_encode;
use crate::options::Options;
use crate::response::{read_response, Response};
use crate::url::{normalize_url_path, parse_url, ParsedUrl};

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

    // Basic auth — curl sends Authorization right after Host.
    if let Some(ref user) = opts.user {
        let encoded = base64_encode(user.as_bytes());
        req.push_str(&format!("Authorization: Basic {encoded}\r\n"));
    }

    // Range — curl sends it early, before User-Agent.
    if let Some(ref range) = opts.range {
        req.push_str(&format!("Range: bytes={range}\r\n"));
    }

    // Check if custom headers override defaults.
    let has_custom = |name: &str| {
        opts.headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case(name))
    };

    // User-Agent.
    if !has_custom("user-agent") {
        let ua = opts.user_agent.as_deref().unwrap_or("curl/8.0.0");
        req.push_str(&format!("User-Agent: {ua}\r\n"));
    }
    if !has_custom("accept") {
        req.push_str("Accept: */*\r\n");
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

    // Referer.
    if let Some(ref referer) = opts.referer {
        req.push_str(&format!("Referer: {referer}\r\n"));
    }

    // Cookie.
    if let Some(ref cookie) = opts.cookie {
        if cookie.contains('=') {
            // Raw cookie string (contains name=value)
            req.push_str(&format!("Cookie: {cookie}\r\n"));
        } else if std::path::Path::new(cookie).is_file() {
            // File path — read cookies from Netscape cookie format
            if let Ok(contents) = fs::read_to_string(cookie) {
                let cookie_pairs: Vec<String> = contents
                    .lines()
                    .filter(|l| !l.starts_with('#') && l.contains('\t'))
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
        // If it doesn't contain '=' and isn't a file, it just enables
        // the cookie engine without sending cookies (like real curl).
    }

    // Custom headers (may override defaults).
    // An empty value (e.g. -H "X-Header:") removes the header entirely.
    // A semicolon instead of colon (e.g. -H "X-Header;") sends the header with no value.
    for (key, val) in &opts.headers {
        if val.is_empty() {
            // Empty value = suppress this header (don't send it)
            continue;
        }
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
        let content_len_hdr = format!("Content-Length: {}\r\n", body.len());
        if !has_content_type {
            if !opts.form_fields.is_empty() {
                let boundary = multipart_boundary(opts);
                req.push_str(&content_len_hdr);
                req.push_str(&format!(
                    "Content-Type: multipart/form-data; boundary={boundary}\r\n"
                ));
            } else if opts.data.is_some() {
                req.push_str(&content_len_hdr);
                req.push_str("Content-Type: application/x-www-form-urlencoded\r\n");
            } else {
                req.push_str(&content_len_hdr);
            }
        } else {
            req.push_str(&content_len_hdr);
        }
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

pub(crate) fn perform(url_str: &str, opts: &Options) -> Result<Response, String> {
    let mut current_url = url_str.to_string();
    let mut redirects = 0;
    let mut redirect_headers: Vec<u8> = Vec::new();

    loop {
        let url = parse_url(&current_url)?;

        let resp = execute_request(&url, opts)?;

        // Handle redirects.
        if opts.location && (301..=308).contains(&resp.status) {
            if redirects >= opts.max_redirs {
                return Err(format!("maximum redirects ({}) followed", opts.max_redirs));
            }
            if let Some((_, location)) = resp.headers.iter().find(|(k, _)| k == "location") {
                // Skip blank Location headers.
                let location = location.trim();
                if location.is_empty() {
                    let mut final_resp = resp;
                    final_resp.redirect_headers = redirect_headers;
                    return Ok(final_resp);
                }

                redirects += 1;

                // Collect intermediate response headers for -i output.
                redirect_headers.extend_from_slice(&resp.header_bytes);

                // Percent-encode spaces in the Location URL.
                let location = location.replace(' ', "%20");

                // Resolve relative URLs.
                if location.starts_with("http://") || location.starts_with("https://") {
                    current_url = location;
                } else if location.starts_with('/') {
                    current_url = format!("{}://{}:{}{}", url.scheme, url.host, url.port, location);
                } else {
                    let base = match current_url.rfind('/') {
                        Some(i) => &current_url[..=i],
                        None => &current_url,
                    };
                    current_url = format!("{base}{location}");
                }
                // Normalize path (resolve ../  ./ segments)
                current_url = normalize_url_path(&current_url);

                if opts.verbose {
                    eprintln!("* Following redirect to {current_url}");
                }
                continue;
            }
        }

        let mut final_resp = resp;
        final_resp.redirect_headers = redirect_headers;
        return Ok(final_resp);
    }
}
