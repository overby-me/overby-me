#[derive(Clone, Debug)]
pub struct ParsedUrl {
    pub(crate) scheme: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) path: String,
    pub(crate) raw: String,
}

pub fn parse_url(raw: &str) -> Result<ParsedUrl, String> {
    let url = if !raw.contains("://") {
        format!("http://{raw}")
    } else {
        raw.to_string()
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

    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };

    // Handle userinfo@ prefix.
    let host_port = match authority.rfind('@') {
        Some(i) => &authority[i + 1..],
        None => authority,
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

    Ok(ParsedUrl {
        scheme,
        host,
        port,
        path: path.to_string(),
        raw: url,
    })
}

/// Normalize a URL path by resolving `.` and `..` segments.
pub(crate) fn normalize_url_path(url: &str) -> String {
    // Find the path portion (after scheme://host:port)
    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];
        let path_start = after_scheme.find('/').map(|i| scheme_end + 3 + i);
        if let Some(ps) = path_start {
            let prefix = &url[..ps];
            let path = &url[ps..];
            let normalized = normalize_path(path);
            return format!("{prefix}{normalized}");
        }
    }
    url.to_string()
}

/// Normalize a path by resolving `.` and `..` segments.
fn normalize_path(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "." => {}
            ".." => {
                segments.pop();
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
