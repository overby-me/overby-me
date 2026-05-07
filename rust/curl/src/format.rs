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

#[allow(clippy::too_many_arguments)]
pub(crate) fn format_write_out(
    fmt: &str,
    resp: &Response,
    url: &ParsedUrl,
    num_connects: usize,
    num_redirects: usize,
    method: &str,
    filename_effective: Option<&std::path::Path>,
    url_num: usize,
    exit_code: i32,
    error_msg: &str,
    referer: &str,
) -> String {
    let mut result = fmt.to_string();
    result = result.replace("%{http_code}", &resp.status.to_string());
    result = result.replace("%{response_code}", &resp.status.to_string());
    result = result.replace("%{urlnum}", &url_num.to_string());
    result = result.replace("%{exitcode}", &exit_code.to_string());
    result = result.replace("%{errormsg}", error_msg);
    result = result.replace("%{referer}", referer);
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
    result = result.replace("%{num_connects}", &num_connects.to_string());
    result = result.replace("%{num_redirects}", &num_redirects.to_string());
    result = result.replace("%{num_retries}", "0");
    result = result.replace("%{num_headers}", &resp.headers.len().to_string());
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

    // `%{urle.…}` — same components, but for the *effective* URL after any
    // redirects. Falls back to the initial URL when no redirect happened.
    let eff = resp
        .final_url
        .as_deref()
        .and_then(|s| crate::url::parse_url(s).ok());
    if let Some(ref eu) = eff {
        result = result.replace("%{urle.scheme}", &eu.scheme);
        result = result.replace("%{urle.host}", &eu.host);
        result = result.replace("%{urle.port}", &eu.port.to_string());
        let (ep, eq) = eu
            .path
            .split_once('?')
            .map(|(p, q)| (p.to_string(), q.to_string()))
            .unwrap_or((eu.path.clone(), String::new()));
        result = result.replace("%{urle.path}", &ep);
        result = result.replace("%{urle.query}", &eq);
    } else {
        result = result.replace("%{urle.scheme}", &url.scheme);
        result = result.replace("%{urle.host}", &url.host);
        result = result.replace("%{urle.port}", &url.port.to_string());
        result = result.replace("%{urle.path}", &path_only);
        result = result.replace("%{urle.query}", &query);
    }
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
    // --data-urlencode forms (curl tool_getparam.c data_urlencode):
    //   content                — encode the whole string
    //   =content               — encode content (no name)
    //   name=content           — encode content, prefix with "name="
    //   @file                  — read file, encode contents
    //   name@file              — read file, encode contents, prefix with "name="
    // Spaces in the encoded output are written as `+` (RFC1866) — curl
    // post-processes %20 → +.
    let (name, sep, rest) = if let Some(eq) = val.find('=') {
        (&val[..eq], '=', &val[eq + 1..])
    } else if let Some(at) = val.find('@') {
        (&val[..at], '@', &val[at + 1..])
    } else {
        ("", ' ', val)
    };

    let raw_bytes: Vec<u8> = if sep == '@' {
        // Read file contents (or stdin for "-").
        if rest == "-" {
            use std::io::Read;
            let mut buf = Vec::new();
            let _ = std::io::stdin().read_to_end(&mut buf);
            buf
        } else {
            std::fs::read(rest).unwrap_or_default()
        }
    } else {
        rest.as_bytes().to_vec()
    };

    let encoded = urlencode_bytes_with_plus(&raw_bytes);
    if name.is_empty() {
        encoded
    } else {
        format!("{name}={encoded}")
    }
}

fn urlencode_bytes_with_plus(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for &b in bytes {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
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
///   %{onerror}       — only emit subsequent chunks when the transfer errored.
///
/// The directives themselves are consumed and produce no output. Each chunk is
/// tagged with a `gated_on_error` flag — chunks past `%{onerror}` are emitted
/// only when the transfer's exit code is non-zero.
pub(crate) fn split_write_out(fmt: &str) -> Vec<(WriteOutDest, bool, String)> {
    let mut chunks: Vec<(WriteOutDest, bool, String)> = Vec::new();
    let mut current_dest = WriteOutDest::Stdout;
    let mut current_buf = String::new();
    let mut gated = false;
    let bytes = fmt.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for a directive starting at i.
        if bytes[i] == b'%' {
            // %{stdout} / %{stderr}
            if fmt[i..].starts_with("%{stdout}") {
                chunks.push((
                    std::mem::replace(&mut current_dest, WriteOutDest::Stdout),
                    gated,
                    std::mem::take(&mut current_buf),
                ));
                i += "%{stdout}".len();
                continue;
            }
            if fmt[i..].starts_with("%{stderr}") {
                chunks.push((
                    std::mem::replace(&mut current_dest, WriteOutDest::Stderr),
                    gated,
                    std::mem::take(&mut current_buf),
                ));
                i += "%{stderr}".len();
                continue;
            }
            // %{onerror}: gate all subsequent output on a non-zero exit code.
            // Doesn't change the destination but does end the current chunk so
            // any preceding (ungated) text is emitted unconditionally.
            if fmt[i..].starts_with("%{onerror}") {
                if !current_buf.is_empty() {
                    let cloned = current_dest_clone(&current_dest);
                    let dest = std::mem::replace(&mut current_dest, cloned);
                    chunks.push((dest, gated, std::mem::take(&mut current_buf)));
                }
                gated = true;
                i += "%{onerror}".len();
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
                    gated,
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
    chunks.push((current_dest, gated, current_buf));
    chunks
}

fn current_dest_clone(d: &WriteOutDest) -> WriteOutDest {
    match d {
        WriteOutDest::Stdout => WriteOutDest::Stdout,
        WriteOutDest::Stderr => WriteOutDest::Stderr,
        WriteOutDest::File { path, append } => WriteOutDest::File {
            path: path.clone(),
            append: *append,
        },
    }
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
