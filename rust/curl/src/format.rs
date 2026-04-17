use crate::response::Response;
use crate::url::ParsedUrl;

pub(crate) fn base64_encode(input: &[u8]) -> String {
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

pub(crate) fn format_write_out(
    fmt: &str,
    resp: &Response,
    url: &ParsedUrl,
    num_connects: usize,
    num_redirects: usize,
    method: &str,
) -> String {
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
    result = result.replace("%{url}", &url.raw);
    result = result.replace(
        "%{redirect_url}",
        resp.redirect_url.as_deref().unwrap_or(""),
    );
    result = result.replace("%{exitcode}", "0");
    result = result.replace("%{errormsg}", "");
    result = result.replace("%{num_connects}", &num_connects.to_string());
    result = result.replace("%{num_redirects}", &num_redirects.to_string());
    result = result.replace("%{num_retries}", "0");
    result = result.replace("%{method}", method);
    result = result.replace("%{remote_ip}", &url.host);
    result = result.replace("%{remote_port}", &url.port.to_string());
    result = result.replace("%{url.scheme}", &url.scheme);
    result = result.replace("%{url.host}", &url.host);
    result = result.replace("%{url.port}", &url.port.to_string());
    result = result.replace("%{scheme}", &url.scheme);
    result = result.replace("%{http_version}", "1.1");
    let (path_only, query) = url
        .path
        .split_once('?')
        .map(|(p, q)| (p.to_string(), q.to_string()))
        .unwrap_or((url.path.clone(), String::new()));
    result = result.replace("%{url.path}", &path_only);
    result = result.replace("%{url.query}", &query);
    result = result.replace("%{url.fragment}", "");
    let (u, p) = match url.userinfo.as_deref() {
        Some(ui) => ui.split_once(':').unwrap_or((ui, "")),
        None => ("", ""),
    };
    result = result.replace("%{url.user}", u);
    result = result.replace("%{url.password}", p);
    result = result.replace("\\n", "\n");
    result = result.replace("\\t", "\t");
    result
}

pub(crate) fn urlencode_field(val: &str) -> String {
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

pub(crate) fn urlencode(s: &str) -> String {
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
