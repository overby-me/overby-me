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
    filename_effective: Option<&std::path::Path>,
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
    result = result.replace(
        "%{filename_effective}",
        filename_effective.and_then(|p| p.to_str()).unwrap_or(""),
    );
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
    // %{header_json} — JSON object mapping header name → array of values.
    // Same-named headers (e.g. multiple Set-Cookie) are accumulated into a list.
    if result.contains("%{header_json}") {
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for (k, v) in &resp.headers {
            if let Some(g) = groups.iter_mut().find(|(name, _)| name == k) {
                g.1.push(v.clone());
            } else {
                groups.push((k.clone(), vec![v.clone()]));
            }
        }
        let mut json = String::from("{");
        for (i, (name, vals)) in groups.iter().enumerate() {
            if i > 0 {
                json.push_str(",\n");
            }
            // Escape quotes/backslashes per JSON rules.
            let esc = |s: &str| {
                let mut out = String::with_capacity(s.len() + 2);
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                        c => out.push(c),
                    }
                }
                out
            };
            json.push('"');
            json.push_str(&esc(name));
            json.push_str("\":[");
            for (j, val) in vals.iter().enumerate() {
                if j > 0 {
                    json.push(',');
                }
                json.push('"');
                json.push_str(&esc(val));
                json.push('"');
            }
            json.push(']');
        }
        json.push_str("\n}");
        result = result.replace("%{header_json}", &json);
    }
    // %header{NAME} — replace with the value of response header NAME
    // (case-insensitive). Missing headers expand to the empty string.
    while let Some(start) = result.find("%header{") {
        let after = &result[start + "%header{".len()..];
        let Some(end_rel) = after.find('}') else {
            break;
        };
        let name = &after[..end_rel].to_lowercase();
        let value = resp
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let pat_end = start + "%header{".len() + end_rel + 1;
        result.replace_range(start..pat_end, &value);
    }
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

/// Destination for a -w write-out chunk. `Append=true` opens with O_APPEND.
pub(crate) enum WriteOutDest {
    Stdout,
    Stderr,
    File {
        path: std::path::PathBuf,
        append: bool,
    },
}

/// Split a -w format string into chunks routed to different destinations.
///
/// Recognized directives:
///   %{stdout}        — switch subsequent output to stdout (default).
///   %{stderr}        — switch subsequent output to stderr.
///   %output{PATH}    — emit subsequent output to PATH (truncated).
///   %output{>>PATH}  — emit subsequent output to PATH (appended).
///
/// The directives themselves are consumed and produce no output.
pub(crate) fn split_write_out(fmt: &str) -> Vec<(WriteOutDest, String)> {
    let mut chunks: Vec<(WriteOutDest, String)> = Vec::new();
    let mut current_dest = WriteOutDest::Stdout;
    let mut current_buf = String::new();
    let bytes = fmt.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for a directive starting at i.
        if bytes[i] == b'%' {
            // %{stdout} / %{stderr}
            if fmt[i..].starts_with("%{stdout}") {
                chunks.push((
                    std::mem::replace(&mut current_dest, WriteOutDest::Stdout),
                    std::mem::take(&mut current_buf),
                ));
                i += "%{stdout}".len();
                continue;
            }
            if fmt[i..].starts_with("%{stderr}") {
                chunks.push((
                    std::mem::replace(&mut current_dest, WriteOutDest::Stderr),
                    std::mem::take(&mut current_buf),
                ));
                i += "%{stderr}".len();
                continue;
            }
            // %output{PATH}
            if fmt[i..].starts_with("%output{")
                && let Some(end_rel) = fmt[i + "%output{".len()..].find('}')
            {
                let inner = &fmt[i + "%output{".len()..i + "%output{".len() + end_rel];
                let (path_str, append) = if let Some(rest) = inner.strip_prefix(">>") {
                    (rest, true)
                } else {
                    (inner, false)
                };
                let new_dest = WriteOutDest::File {
                    path: std::path::PathBuf::from(path_str),
                    append,
                };
                chunks.push((
                    std::mem::replace(&mut current_dest, new_dest),
                    std::mem::take(&mut current_buf),
                ));
                i += "%output{".len() + end_rel + 1;
                continue;
            }
        }
        // Default: append byte (we slice as char to keep UTF-8 safety).
        let ch_len = utf8_char_len(bytes[i]);
        current_buf.push_str(&fmt[i..i + ch_len]);
        i += ch_len;
    }
    chunks.push((current_dest, current_buf));
    chunks
}

fn utf8_char_len(b: u8) -> usize {
    if b < 0xC0 {
        1 // ASCII or continuation byte
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}
