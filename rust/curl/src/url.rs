/// Expand curl-style URL globs: `{alt1,alt2,...}` lists, `[a-z]` char ranges,
/// `[N-M]` numeric ranges, `[N-M:S]` stepped ranges. Returns the flat list of
/// expanded URLs (Cartesian product for multiple globs in the same URL).
pub(crate) fn expand_glob(url: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut out: Vec<(String, Vec<String>)> = vec![(String::new(), Vec::new())];
    let bytes = url.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        // Backslash escapes glob metachars (\{, \}, \[, \]) — emit only the next char.
        // For any other char, the backslash is preserved as-is.
        if c == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if matches!(next, b'{' | b'}' | b'[' | b']') {
                for item in out.iter_mut() {
                    item.0.push(next as char);
                }
                i += 2;
                continue;
            }
        }
        if c == b'{'
            && let Some(end) = find_matching(bytes, i, b'{', b'}')
        {
            let alts: Vec<String> = std::str::from_utf8(&bytes[i + 1..end])
                .unwrap_or("")
                .split(',')
                .map(|s| s.to_string())
                .collect();
            out = cross(out, alts);
            i = end + 1;
            continue;
        }
        if c == b'['
            && let Some(end) = find_matching(bytes, i, b'[', b']')
        {
            let spec = std::str::from_utf8(&bytes[i + 1..end]).unwrap_or("");
            match parse_range(spec) {
                Ok(Some(expanded)) => {
                    out = cross(out, expanded);
                    i = end + 1;
                    continue;
                }
                Ok(None) => {
                    // Not a valid range spec -- treat as literal characters
                }
                Err(_) => {
                    // Bad range (e.g. start > end). Report with position info.
                    // Point the caret one past the closing ']', matching curl.
                    let pos = end + 2; // 1-based position after ']'
                    let spaces = " ".repeat(pos - 1);
                    return Err(format!(
                        "curl: (3) bad range in URL position {pos}:\n{url}\n{spaces}^"
                    ));
                }
            }
        }
        for item in out.iter_mut() {
            item.0.push(c as char);
        }
        i += 1;
    }
    Ok(out)
}

fn find_matching(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0;
    for (idx, &b) in bytes.iter().enumerate().skip(start) {
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(idx);
            }
        }
    }
    None
}

fn cross(
    prefixes: Vec<(String, Vec<String>)>,
    suffixes: Vec<String>,
) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::with_capacity(prefixes.len() * suffixes.len());
    for (p, vals) in &prefixes {
        for s in &suffixes {
            let mut new_vals = vals.clone();
            new_vals.push(s.clone());
            out.push((format!("{p}{s}"), new_vals));
        }
    }
    out
}

/// Parse a range spec like "a-z", "1-10", "01-100:2".
/// Returns Ok(Some(values)) on success, Ok(None) if the spec is not a valid
/// range format, or Err(()) when the range is syntactically valid but
/// semantically bad (e.g. numeric start > end).
fn parse_range(spec: &str) -> Result<Option<Vec<String>>, ()> {
    let (range, step) = if let Some((r, s)) = spec.rsplit_once(':') {
        match s.parse::<usize>() {
            Ok(v) => (r, v.max(1)),
            Err(_) => return Ok(None),
        }
    } else {
        (spec, 1)
    };
    let Some((start, end)) = range.split_once('-') else {
        return Ok(None);
    };
    if let (Ok(s), Ok(e)) = (start.parse::<i64>(), end.parse::<i64>()) {
        if s > e {
            return Err(());
        }
        let width = start.len().max(end.len());
        let zero_pad = start.starts_with('0') && start.len() > 1;
        let mut out = Vec::new();
        let mut n = s;
        while n <= e {
            out.push(if zero_pad {
                format!("{n:0width$}")
            } else {
                n.to_string()
            });
            n += step as i64;
        }
        return Ok(Some(out));
    }
    if start.len() == 1 && end.len() == 1 {
        let s = start.chars().next().unwrap();
        let e = end.chars().next().unwrap();
        if s.is_ascii() && e.is_ascii() {
            if s > e {
                return Err(());
            }
            let mut out = Vec::new();
            let mut c = s as u32;
            while c <= e as u32 {
                out.push((c as u8 as char).to_string());
                c += step as u32;
            }
            return Ok(Some(out));
        }
    }
    Ok(None)
}

#[derive(Clone, Debug)]
pub struct ParsedUrl {
    pub(crate) scheme: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) path: String,
    pub(crate) raw: String,
    /// "user:pass" extracted from "scheme://user:pass@host".
    pub(crate) userinfo: Option<String>,
}

pub fn parse_url(raw: &str) -> Result<ParsedUrl, String> {
    // Recognize a scheme prefix only at the start (not "://" embedded in a query).
    // Also accept single-slash form like "http:/host/..." (curl normalizes to "://").
    let has_scheme = raw
        .bytes()
        .take_while(|&b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
        .count();
    let url = if has_scheme > 0 && raw[has_scheme..].starts_with("://") {
        raw.to_string()
    } else if has_scheme > 0
        && raw[has_scheme..].starts_with(":/")
        && !raw[has_scheme..].starts_with("://")
    {
        // scheme:/host/... → treat the "/" after the colon as part of "://".
        format!("{}://{}", &raw[..has_scheme], &raw[has_scheme + 2..])
    } else {
        format!("http://{raw}")
    };

    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| format!("invalid URL: {raw}"))?;

    let scheme = scheme.to_lowercase();
    let default_port: u16 = match scheme.as_str() {
        "http" => 80,
        "https" => 443,
        _ => return Err(format!("unsupported scheme: {scheme}")),
    };

    // Strip fragment — it's never sent to the server.
    let rest = rest.split('#').next().unwrap_or(rest);

    // Split at first / or ? (whichever comes first) to separate authority from path.
    let slash_pos = rest.find('/');
    let query_pos = rest.find('?');
    let path_start = match (slash_pos, query_pos) {
        (Some(s), Some(q)) => s.min(q),
        (Some(s), None) => s,
        (None, Some(q)) => q,
        (None, None) => rest.len(),
    };
    let authority = &rest[..path_start];
    let path = if path_start < rest.len() {
        let p = &rest[path_start..];
        // If path starts with ?, prepend /
        if p.starts_with('?') {
            format!("/{p}")
        } else {
            p.to_string()
        }
    } else {
        "/".to_string()
    };

    // Handle userinfo@ prefix — if present, return it for Basic auth use.
    let (userinfo, host_port) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };

    let (host, port) = if host_port.starts_with('[') {
        // IPv6
        match host_port.find(']') {
            Some(end) => {
                let h = &host_port[1..end];
                let p = if host_port.len() > end + 1 && host_port.as_bytes()[end + 1] == b':' {
                    host_port[end + 2..]
                        .parse::<u16>()
                        .map_err(|e| format!("bad port: {e}"))?
                } else {
                    default_port
                };
                (h.to_string(), p)
            }
            None => return Err("unterminated IPv6 address".into()),
        }
    } else {
        match host_port.rsplit_once(':') {
            Some((h, p)) => {
                let port = p.parse::<u16>().map_err(|e| format!("bad port: {e}"))?;
                (h.to_string(), port)
            }
            None => (host_port.to_string(), default_port),
        }
    };

    if host.is_empty() {
        return Err("empty host".into());
    }

    // URL-decode userinfo so Basic auth credentials reflect their literal value
    // (e.g. "user%0aname:password" → "user\nname:password").
    let userinfo = userinfo.map(percent_decode);
    Ok(ParsedUrl {
        scheme,
        host,
        port,
        path: path.to_string(),
        raw: url,
        userinfo,
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(&e.into_bytes()).into_owned())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Normalize a URL path by resolving `.` and `..` segments. Query and fragment
/// are preserved verbatim (not normalized).
pub(crate) fn normalize_url_path(url: &str) -> String {
    // Find the path portion (after scheme://host[:port])
    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];
        let path_start = after_scheme.find('/').map(|i| scheme_end + 3 + i);
        if let Some(ps) = path_start {
            let prefix = &url[..ps];
            let rest = &url[ps..];
            // Split path / query / fragment.
            let (path, qf) = rest.split_once(['?', '#']).map_or((rest, ""), |(p, qf)| {
                // Re-attach the separator that split consumed.
                let sep = &rest[p.len()..p.len() + 1];
                let qf_with_sep = &rest[p.len()..];
                let _ = qf;
                let _ = sep;
                (p, qf_with_sep)
            });
            let normalized = normalize_path(path);
            return format!("{prefix}{normalized}{qf}");
        }
    }
    url.to_string()
}

/// Normalize a path by resolving `.` and `..` segments.
fn normalize_path(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "." | "" if !segments.is_empty() && seg == "." => {}
            ".." => {
                // Don't pop above root (keep at least the empty first segment from "/")
                if segments.len() > 1 {
                    segments.pop();
                }
            }
            _ => segments.push(seg),
        }
    }
    let result = segments.join("/");
    if result.is_empty() {
        "/".to_string()
    } else {
        result
    }
}
