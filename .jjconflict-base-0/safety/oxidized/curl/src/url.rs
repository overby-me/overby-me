/// Expand curl-style URL globs: `{alt1,alt2,...}` lists, `[a-z]` char ranges,
/// `[N-M]` numeric ranges, `[N-M:S]` stepped ranges. Returns the flat list of
/// expanded URLs (Cartesian product for multiple globs in the same URL).
pub(crate) fn expand_glob(url: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut out: Vec<(String, Vec<String>)> = vec![(String::new(), Vec::new())];
    let bytes = url.as_bytes();
    let mut i = 0;
    // Mirror curl's `pos`: 1-based char index that advances for every consumed
    // char EXCEPT the `}` that closes a `{…}` set (case '}' in glob_set never
    // increments `*posp`). Used only to compute the position reported in
    // glob-error messages (test 761).
    let mut pos: usize = 1;
    // Mirror curl's `pnum`: count of patterns added so far. A "pattern" is
    // either a literal segment between sets/ranges or a `{…}` / `[…]` glob.
    // Curl errors when this would exceed 255 (`if(glob->pnum < 255)` check in
    // add_glob, with palloc starting at 2 and doubling). Limiting at 255 keeps
    // our behaviour byte-identical for the diagnostic.
    let mut pnum: usize = 0;
    let mut literal_pending = false;
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
                pos += 2;
                literal_pending = true;
                continue;
            }
        }
        if c == b'{' {
            // Flush a pending literal segment as one pattern before the set.
            if literal_pending {
                pnum += 1;
                literal_pending = false;
            }
            if let Some(end) = find_matching(bytes, i, b'{', b'}') {
                // pos after parsing the set: advance over `{` and inner chars
                // but not `}` (matches curl). For `{a}` the delta is 2.
                let pos_delta = end - i; // bytes from `{` to `}` inclusive minus 1 (skip `}`)
                let pos_at_close = pos + pos_delta;
                pnum += 1;
                if pnum > 255 {
                    return Err(format_glob_error("too many {} sets", pos_at_close, url));
                }
                let alts: Vec<String> = std::str::from_utf8(&bytes[i + 1..end])
                    .unwrap_or("")
                    .split(',')
                    .map(|s| s.to_string())
                    .collect();
                out = cross(out, alts);
                pos = pos_at_close;
                i = end + 1;
                continue;
            }
            // Unmatched `{` — curl reports CURLE_URL_MALFORMAT (test 1234).
            return Err(format!("curl: (3) unmatched brace in URL: {url}"));
        }
        if c == b'[' {
            if let Some(end) = find_matching(bytes, i, b'[', b']') {
                let spec = std::str::from_utf8(&bytes[i + 1..end]).unwrap_or("");
                match parse_range(spec) {
                    Ok(Some(expanded)) => {
                        if literal_pending {
                            pnum += 1;
                            literal_pending = false;
                        }
                        let pos_delta = end - i;
                        let pos_at_close = pos + pos_delta;
                        pnum += 1;
                        if pnum > 255 {
                            return Err(format_glob_error("too many [] ranges", pos_at_close, url));
                        }
                        out = cross(out, expanded);
                        pos = pos_at_close;
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
            } else {
                // Unmatched `[`. If the unbalanced run contains a `-` it looks
                // like an attempted range; reject as malformed (test 1289).
                // Otherwise fall through and treat as a literal byte (so plain
                // IPv4 URLs with no glob syntax keep working).
                let rest = std::str::from_utf8(&bytes[i + 1..]).unwrap_or("");
                if rest.contains('-') {
                    return Err(format!("curl: (3) unmatched bracket in URL: {url}"));
                }
            }
        }
        for item in out.iter_mut() {
            item.0.push(c as char);
        }
        literal_pending = true;
        i += 1;
        pos += 1;
    }
    Ok(out)
}

// Mirrors curl's tool_urlglob.c text[512] truncation so long URLs cut off
// the trailing `\n<spaces>^` (test 761's expected stderr).
fn format_glob_error(err: &str, pos: usize, url: &str) -> String {
    let head = format!("{err} in URL position {pos}:\n");
    let tail = format!("\n{}^", " ".repeat(pos - 1));
    // curl's text[512] is null-terminated, so 511 usable bytes.
    const TEXT_CAP: usize = 511;
    let mut text = String::with_capacity(TEXT_CAP);
    text.push_str(&head);
    let url_room = TEXT_CAP.saturating_sub(text.len());
    if url.len() <= url_room {
        text.push_str(url);
        let remaining = TEXT_CAP.saturating_sub(text.len());
        if tail.len() <= remaining {
            text.push_str(&tail);
        } else {
            text.push_str(&tail[..remaining]);
        }
    } else {
        text.push_str(&url[..url_room]);
    }
    format!("curl: (3) {text}")
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
    /// Fragment component (`#...`) — never sent on the wire but exposed
    /// via `-w %{url.fragment}` / `%{urle.fragment}`.
    pub(crate) fragment: Option<String>,
}

pub fn parse_url(raw: &str) -> Result<ParsedUrl, String> {
    // Reject literal spaces in the URL outside of the path component when
    // the URL has a scheme — curl exits 3 (CURLE_URL_MALFORMAT) for these
    // (test 1469). Schemeless URLs (no `://` and no `:/`) are still allowed
    // through so the path-only form keeps working for downstream callers.
    if raw.contains(' ')
        && let Some(colon_slash) = raw.find(':')
        && raw[colon_slash..].starts_with(":/")
    {
        // Spaces in the authority/scheme/query are malformed.
        let path_start = raw.find('/').unwrap_or(raw.len());
        if raw[..path_start].contains(' ') || raw[path_start..].contains(' ') {
            return Err(format!("malformed URL: {raw}"));
        }
    }
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

    // Strip fragment from the wire-bound path but remember it for `-w
    // %{url.fragment}` (test 423).
    let (rest, fragment) = match rest.split_once('#') {
        Some((path, frag)) => (path, Some(frag.to_string())),
        None => (rest, None),
    };

    // Tolerate one extra leading slash after the scheme separator
    // (slashes between scheme and authority); two or more extra leading
    // slashes are malformed.
    let rest = if rest.starts_with("//") {
        return Err(format!("malformed URL: {raw}"));
    } else if let Some(stripped) = rest.strip_prefix('/') {
        stripped
    } else {
        rest
    };

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
    // Multiple `@` in the authority is malformed (RFC 3986); userinfo may not
    // itself contain a raw `@` — curl rejects such URLs (test 1260).
    let (userinfo, host_port) = match authority.rfind('@') {
        Some(i) => {
            let ui = &authority[..i];
            if ui.contains('@') {
                return Err("malformed URL: multiple @ in authority".to_string());
            }
            (Some(ui), &authority[i + 1..])
        }
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

    // IDN: if the host has non-ASCII bytes (UTF-8) AND no IPv6 brackets,
    // run it through punycode encoding (tests 165, 1034, 1035, 1448, 2046,
    // 2047, 763). A round-trip failure (e.g. malformed UTF-8 in the host)
    // maps to CURLE_URL_MALFORMAT (test 1034).
    // Percent-decode the host into a UTF-8 string when it carries %-escapes.
    // A Location: header for a non-ASCII hostname is typically delivered as
    // `http://%c3%a5...se` and the percent-decoded bytes are the original
    // UTF-8 — we want them as chars before running IDN (test 1448).
    let host = if !host_port.starts_with('[') && host.contains('%') {
        let bytes = host.as_bytes();
        let mut decoded_bytes = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .ok()
                    .and_then(|s| u8::from_str_radix(s, 16).ok());
                if let Some(b) = hex {
                    decoded_bytes.push(b);
                    i += 3;
                    continue;
                }
            }
            decoded_bytes.push(bytes[i]);
            i += 1;
        }
        match String::from_utf8(decoded_bytes) {
            Ok(s) => s,
            Err(_) => host,
        }
    } else {
        host
    };

    let host = if !host_port.starts_with('[') && !host.is_ascii() {
        // Perl runtests passes %hex[...]hex% bytes via execve in a way that
        // double-encodes them as UTF-8 (chr(0xc3) becomes the Ã character,
        // which perl then encodes as the 2-byte UTF-8 sequence). curl uses
        // mbstowcs which under C.UTF-8 decodes that as the original
        // single-byte sequence. We replicate the same: if every char in the
        // host fits in a byte (Latin-1) AND the reconstructed bytes parse as
        // UTF-8, use the decoded form; otherwise pass the original chars to
        // IDN unchanged. A failed reconstruction is NOT an error — the host
        // may genuinely be non-Latin-1 (test 1448's percent-decoded UTF-8).
        let input = if host.chars().all(|c| (c as u32) <= 0xFF) {
            let bytes: Vec<u8> = host.chars().map(|c| c as u8).collect();
            std::str::from_utf8(&bytes)
                .map(String::from)
                .unwrap_or_else(|_| host.clone())
        } else {
            host.clone()
        };
        let result = idna::domain_to_ascii(&input)
            .map_err(|_| format!("malformed URL: bad IDN host: {host}"))?;
        if result.is_empty() {
            return Err(format!("malformed URL: empty host after IDN: {host}"));
        }
        // DNS label limits: each label max 63 chars, total domain ≤ 255 chars
        // (test 1035). curl rejects with CURLE_URL_MALFORMAT.
        if result.len() > 255 || result.split('.').any(|label| label.len() > 63) {
            return Err(format!("malformed URL: IDN host too long: {host}"));
        }
        result
    } else {
        host
    };

    // Reject `:` characters in the (non-IPv6) host portion. After stripping
    // IPv6 brackets these would indicate rubbish like `host:8080:80` (test 1260).
    if !host_port.starts_with('[') && host.contains(':') {
        return Err(format!("malformed URL: stray colon in host: {host}"));
    }

    // Whitespace and ASCII control bytes in the host are not legal — reject
    // them outright with CURLE_URL_MALFORMAT (test 1264).
    if host.bytes().any(|b| b <= 0x20 || b == 0x7F) {
        return Err(format!("malformed URL: whitespace in host: {host}"));
    }
    // After stripping IPv6 brackets, additional rubbish like `[host]extra`
    // means the URL had garbage between `]` and the port/path (test 1263).
    if host_port.starts_with('[')
        && let Some(end) = host_port.find(']')
        && let Some(after) = host_port.get(end + 1..)
        && !after.is_empty()
        && !after.starts_with(':')
    {
        return Err(format!(
            "malformed URL: rubbish after IPv6 bracket: {host_port}"
        ));
    }

    // curl limits hostnames to 65535 bytes (CURLE_URL_MALFORMAT / exit 3).
    if host.len() > 65535 {
        return Err(format!("hostname too long ({} bytes)", host.len()));
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
        fragment,
    })
}

/// Parse a URL into ParsedUrl WITHOUT rejecting unsupported schemes — used
/// by `--write-out` to populate `%{url.*}` / `%{urle.*}` even when the URL
/// would be rejected at transfer time (test 423). Falls back to empty fields
/// when the URL has no scheme separator at all.
pub(crate) fn parse_url_lenient(raw: &str) -> ParsedUrl {
    if let Ok(p) = parse_url(raw) {
        return p;
    }
    // Look for "scheme://rest". Without it, return all-empty fields.
    let Some((scheme, rest)) = raw.split_once("://") else {
        return ParsedUrl {
            scheme: String::new(),
            host: String::new(),
            port: 0,
            path: String::new(),
            raw: raw.to_string(),
            userinfo: None,
            fragment: None,
        };
    };
    let scheme = scheme.to_ascii_lowercase();
    // Strip fragment off whatever is to come.
    let (rest, fragment) = match rest.split_once('#') {
        Some((p, f)) => (p, Some(f.to_string())),
        None => (rest, None),
    };
    // Split authority from path.
    let (authority, path) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let (userinfo, host_port) = match authority.rfind('@') {
        Some(i) => (Some(authority[..i].to_string()), &authority[i + 1..]),
        None => (None, authority),
    };
    let (host, port) = if let Some(stripped) = host_port.strip_prefix('[') {
        match stripped.find(']') {
            Some(end) => {
                let h = &stripped[..end];
                let p = stripped
                    .get(end + 1..)
                    .and_then(|s| s.strip_prefix(':'))
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                (h.to_string(), p)
            }
            None => (stripped.to_string(), 0),
        }
    } else {
        match host_port.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().unwrap_or(0)),
            None => (host_port.to_string(), 0),
        }
    };
    let path = if path.is_empty() {
        String::new()
    } else {
        path.to_string()
    };
    ParsedUrl {
        scheme,
        host,
        port,
        path,
        raw: raw.to_string(),
        userinfo,
        fragment,
    }
}

/// Strip an IPv6 zone/scope ID from a bare host. Accepts both the raw
/// `%scope` and percent-encoded `%25scope` forms used in URLs. Non-IPv6
/// hosts (no `:`) are returned unchanged.
pub(crate) fn strip_ipv6_scope(host: &str) -> String {
    if !host.contains(':') {
        return host.to_string();
    }
    if let Some(i) = host.find("%25") {
        return host[..i].to_string();
    }
    if let Some(i) = host.find('%') {
        return host[..i].to_string();
    }
    host.to_string()
}

pub(crate) fn percent_decode(s: &str) -> String {
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
