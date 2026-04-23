use std::fs;
use std::path::PathBuf;

use crate::url::ParsedUrl;

#[derive(Clone, Debug)]
pub(crate) struct Cookie {
    pub(crate) domain: String,
    pub(crate) include_subdomains: bool,
    pub(crate) path: String,
    pub(crate) secure: bool,
    pub(crate) http_only: bool,
    pub(crate) expires: i64,
    pub(crate) name: String,
    pub(crate) value: String,
}

impl Cookie {
    /// Returns `true` when the cookie carries an explicit expiry that is
    /// already in the past (or exactly "now").  `Max-Age=0` produces
    /// `expires == <current unix time>`, so by the time we check it will
    /// be <= now.  Session cookies (`expires == 0`) are NOT considered
    /// expired.
    pub(crate) fn is_expired(&self) -> bool {
        if self.expires <= 0 {
            return false; // session cookie – no expiry
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.expires <= now
    }

    pub(crate) fn to_jar_line(&self) -> String {
        let prefix = if self.http_only { "#HttpOnly_" } else { "" };
        let subdomains = if self.include_subdomains {
            "TRUE"
        } else {
            "FALSE"
        };
        let secure = if self.secure { "TRUE" } else { "FALSE" };
        format!(
            "{prefix}{domain}\t{subdomains}\t{path}\t{secure}\t{expires}\t{name}\t{value}",
            domain = self.domain,
            path = self.path,
            expires = self.expires,
            name = self.name,
            value = self.value,
        )
    }

    pub(crate) fn from_jar_line(line: &str) -> Option<Cookie> {
        let (http_only, line) = if let Some(rest) = line.strip_prefix("#HttpOnly_") {
            (true, rest)
        } else {
            (false, line)
        };

        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 7 {
            return None;
        }

        let domain = fields[0].to_string();
        let include_subdomains = fields[1].eq_ignore_ascii_case("TRUE");
        let path = fields[2].to_string();
        let secure = fields[3].eq_ignore_ascii_case("TRUE");
        let expires: i64 = fields[4].parse().unwrap_or(0);
        let name = fields[5].to_string();
        let value = fields[6..].join("\t");

        Some(Cookie {
            domain,
            include_subdomains,
            path,
            secure,
            http_only,
            expires,
            name,
            value,
        })
    }
}

fn has_control_chars(s: &str) -> bool {
    for b in s.bytes() {
        match b {
            0x00..=0x08 | 0x0A..=0x1F | 0x7F => return true,
            _ => {}
        }
    }
    false
}

fn default_path(request_path: &str) -> String {
    // Strip query string first.
    let path = request_path.split('?').next().unwrap_or(request_path);

    if path.is_empty() || !path.starts_with('/') {
        return "/".to_string();
    }

    // Find the rightmost '/'.
    match path.rfind('/') {
        Some(0) => "/".to_string(),
        Some(pos) => path[..pos].to_string(),
        None => "/".to_string(),
    }
}

pub(crate) fn is_ip_address(host: &str) -> bool {
    // IPv6: contains a colon
    if host.contains(':') {
        return true;
    }
    // IPv4: digits and dots, has at least one dot, starts with a digit
    if host.contains('.')
        && host.starts_with(|c: char| c.is_ascii_digit())
        && host.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        return true;
    }
    false
}

fn validate_set_cookie_domain(
    domain_attr: Option<&str>,
    request_host: &str,
) -> Option<(String, bool)> {
    let attr = match domain_attr {
        Some(d) if !d.is_empty() => d,
        _ => return Some((request_host.to_string(), false)),
    };

    // Preserve the original case of the Domain attribute (curl writes the
    // domain to the cookie jar verbatim) but use lowercased copies for
    // case-insensitive comparisons.
    let domain_orig = attr.strip_prefix('.').unwrap_or(attr).to_string();
    let domain_lc = domain_orig.to_lowercase();
    let host_lc = request_host.to_lowercase();

    if is_ip_address(&host_lc) {
        // For IP addresses, only exact match is allowed.
        if domain_lc == host_lc {
            return Some((domain_orig, false));
        }
        return None;
    }

    // The host must match the domain or be a subdomain of it.
    if host_lc != domain_lc && !host_lc.ends_with(&format!(".{domain_lc}")) {
        return None;
    }

    // Reject TLD-only domains (no dot in the stripped domain).
    // Exception: "localhost" is allowed per RFC 6761.
    if !domain_lc.contains('.') && domain_lc != "localhost" {
        return None;
    }

    // Reject malformed domains like "..com".
    if domain_lc.starts_with('.') {
        return None;
    }

    Some((format!(".{domain_orig}"), true))
}

fn normalize_domain_lenient(domain_attr: Option<&str>, request_host: &str) -> (String, bool) {
    let attr = match domain_attr {
        Some(d) if !d.is_empty() => d,
        _ => return (request_host.to_lowercase(), false),
    };

    let domain = attr.strip_prefix('.').unwrap_or(attr).to_lowercase();
    (format!(".{domain}"), true)
}

pub(crate) fn parse_set_cookie_ex(
    cookie_str: &str,
    url: &ParsedUrl,
    request_host: &str,
    validate_domain: bool,
) -> Option<Cookie> {
    let parts: Vec<&str> = cookie_str.splitn(2, ';').collect();
    let name_value = parts[0].trim();

    let eq_pos = name_value.find('=')?;
    let raw_name = &name_value[..eq_pos];
    let raw_value = &name_value[eq_pos + 1..];

    // Check for control characters BEFORE trimming (whitespace-like
    // control chars such as VT/FF would be stripped by trim()).
    if has_control_chars(raw_name) || has_control_chars(raw_value) {
        return None;
    }

    let name = raw_name.trim().to_string();
    let value = raw_value.trim().to_string();

    if name.is_empty() {
        return None;
    }

    let mut domain_attr: Option<String> = None;
    let mut path_attr: Option<String> = None;
    let mut secure = false;
    let mut http_only = false;
    let mut expires: i64 = 0;
    let mut has_max_age = false;

    if parts.len() > 1 {
        for attr in parts[1].split(';') {
            let attr = attr.trim();
            if attr.is_empty() {
                continue;
            }
            if let Some(eq) = attr.find('=') {
                let key = attr[..eq].trim().to_lowercase();
                let val = attr[eq + 1..].trim();
                match key.as_str() {
                    "domain" => {
                        domain_attr = Some(val.to_string());
                    }
                    "path" => {
                        path_attr = Some(val.to_string());
                    }
                    "max-age" => {
                        if let Ok(n) = val.parse::<i64>() {
                            if n <= 0 {
                                // Max-Age=0 (or negative) means "delete immediately".
                                // Use expires=1 (epoch + 1s) so the cookie is
                                // unambiguously in the past regardless of clock
                                // precision.
                                expires = 1;
                            } else {
                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs() as i64;
                                expires = now + n;
                            }
                            has_max_age = true;
                        }
                    }
                    "expires" => {
                        if !has_max_age && let Some(ts) = parse_http_date(val) {
                            // Epoch (0) is a valid "already expired" date used
                            // for cookie deletion.  Use 1 so it is not confused
                            // with the session-cookie sentinel (0).
                            expires = if ts <= 0 { 1 } else { ts };
                        }
                    }
                    _ => {}
                }
            } else {
                let key = attr.to_lowercase();
                match key.as_str() {
                    "secure" => secure = true,
                    "httponly" => http_only = true,
                    _ => {}
                }
            }
        }
    }

    let path = path_attr.unwrap_or_else(|| default_path(&url.path));
    // Strip one trailing slash, matching curl's sanitize_cookie_path().
    let path = if path.len() > 1 && path.ends_with('/') {
        path[..path.len() - 1].to_string()
    } else {
        path
    };

    let (domain, include_subdomains) = if validate_domain {
        validate_set_cookie_domain(domain_attr.as_deref(), request_host)?
    } else {
        normalize_domain_lenient(domain_attr.as_deref(), request_host)
    };

    // Cap expires to 400 days from now (RFC 6265bis behavior).
    let expires = if expires > 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let cap = now + 400 * 24 * 3600 + 30;
        let cap = (cap / 60) * 60;
        if expires > cap { cap } else { expires }
    } else {
        expires
    };

    // Reject Secure cookies received over non-HTTPS (live responses only).
    // Exception: treat 127.0.0.1, ::1 and *.localhost as trustworthy origins
    // per W3C "Secure Contexts", matching curl's psl_loopback_p() exception.
    if validate_domain && secure && url.scheme != "https" {
        // Use the request host (Host header), not the connection target,
        // because curl's psl_loopback_p() check operates on the cookie's
        // effective hostname. A request to 127.0.0.1 with -H "Host: foo.com"
        // must NOT accept secure cookies.
        let host = request_host.to_lowercase();
        let is_loopback = host == "127.0.0.1"
            || host == "::1"
            || host == "localhost"
            || host.ends_with(".localhost");
        if !is_loopback {
            return None;
        }
    }

    Some(Cookie {
        domain,
        include_subdomains,
        path,
        secure,
        http_only,
        expires,
        name,
        value,
    })
}

pub(crate) fn parse_set_cookie(
    cookie_str: &str,
    url: &ParsedUrl,
    request_host: &str,
) -> Option<Cookie> {
    parse_set_cookie_ex(cookie_str, url, request_host, true)
}

pub(crate) fn load_cookies_from_file(
    file_path: &str,
    request_host: &str,
    _request_path: &str,
    secure: bool,
) -> Vec<Cookie> {
    let contents = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut cookies = Vec::new();

    if contents.starts_with("HTTP/") {
        // HTTP response header format: parse Set-Cookie lines.
        let fake_url = ParsedUrl {
            scheme: if secure {
                "https".into()
            } else {
                "http".into()
            },
            host: request_host.to_string(),
            port: if secure { 443 } else { 80 },
            path: "/".to_string(),
            raw: String::new(),
            userinfo: None,
        };
        for header_line in contents.split('\n') {
            let header_line = header_line.trim_end_matches('\r');
            if header_line.len() > 11 && header_line[..11].eq_ignore_ascii_case("set-cookie:") {
                let cookie_str = header_line[11..].trim();
                // Use lenient domain normalization (validate_domain=false).
                if let Some(cookie) =
                    parse_set_cookie_ex(cookie_str, &fake_url, request_host, false)
                {
                    cookies.push(cookie);
                }
            }
        }
    } else {
        // Netscape cookie format.
        for line in contents.split('\n') {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            // Skip comment lines, but allow #HttpOnly_ prefix.
            if line.starts_with('#') && !line.starts_with("#HttpOnly_") {
                continue;
            }
            if let Some(cookie) = Cookie::from_jar_line(line) {
                cookies.push(cookie);
            }
        }
    }

    cookies
}

pub(crate) fn save_cookie_jar(
    path: &PathBuf,
    url: &ParsedUrl,
    request_host: &str,
    response_headers: &[(String, String)],
    input_cookie_args: &[String],
    memory_cookies: &[String],
) {
    let mut cookies: Vec<Cookie> = Vec::new();

    // Closure to add a cookie, deduplicating by (domain, path, name).
    // Expired cookies (Max-Age=0 / past Expires) act as deletions:
    // they remove matching entries and are NOT themselves kept.
    let mut add_cookie = |cookie: Cookie| {
        let domain_key = cookie
            .domain
            .strip_prefix("#HttpOnly_")
            .unwrap_or(&cookie.domain)
            .to_string();
        cookies.retain(|c| {
            let d = c.domain.strip_prefix("#HttpOnly_").unwrap_or(&c.domain);
            !(d == domain_key && c.path == cookie.path && c.name == cookie.name)
        });
        if !cookie.is_expired() {
            cookies.push(cookie);
        }
    };

    // Phase 1: Load from -b input files.
    for arg in input_cookie_args {
        // Skip inline "name=value" cookie strings (not files).
        if arg.contains('=') {
            continue;
        }
        if !std::path::Path::new(arg.as_str()).is_file() {
            continue;
        }
        let file_cookies =
            load_cookies_from_file(arg, request_host, &url.path, url.scheme == "https");
        for c in file_cookies {
            add_cookie(c);
        }
    }

    // Phase 2: Load memory cookies (Netscape-format lines from prior responses).
    for line in memory_cookies {
        if let Some(cookie) = Cookie::from_jar_line(line) {
            add_cookie(cookie);
        }
    }

    // Phase 3: Parse Set-Cookie headers from the current response.
    for (key, value) in response_headers {
        if key.eq_ignore_ascii_case("set-cookie")
            && let Some(cookie) = parse_set_cookie(value, url, request_host)
        {
            add_cookie(cookie);
        }
    }

    // Reverse to simulate curl's creation-time-descending sort.
    cookies.reverse();

    // Write Netscape cookie jar.
    let mut output = String::new();
    output.push_str("# Netscape HTTP Cookie File\n");
    output.push_str("# https://curl.se/docs/http-cookies.html");
    output.push('\n');
    output.push_str("# This file was generated by libcurl! Edit at your own risk.\n\n");

    for cookie in &cookies {
        output.push_str(&cookie.to_jar_line());
        output.push('\n');
    }

    let _ = fs::write(path, output);
}

pub(crate) fn format_cookie_line(
    cookie_str: &str,
    url: &ParsedUrl,
    request_host: &str,
) -> Option<String> {
    parse_set_cookie(cookie_str, url, request_host).map(|c| c.to_jar_line())
}

/// Returns `true` when a Netscape-format jar line has an expiry timestamp
/// that is in the past (or exactly now).  Used by the cookie engine to
/// detect deletion cookies (`Max-Age=0`) already serialised to jar lines.
pub(crate) fn is_jar_line_expired(line: &str) -> bool {
    Cookie::from_jar_line(line).is_some_and(|c| c.is_expired())
}

// ---------------------------------------------------------------------------
// HTTP date parsing (RFC 6265 section 5.1.1 / RFC 2616 / Netscape-style)
// ---------------------------------------------------------------------------

pub(crate) fn parse_http_date(s: &str) -> Option<i64> {
    let mut year: Option<i64> = None;
    let mut month: Option<u32> = None;
    let mut day: Option<u32> = None;
    let mut hour: Option<u32> = None;
    let mut minute: Option<u32> = None;
    let mut second: Option<u32> = None;

    // Tokenize on whitespace, comma, dash.
    for token in s.split(|c: char| c.is_ascii_whitespace() || c == ',' || c == '-') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }

        // Skip day names.
        if is_day_name(token) {
            continue;
        }

        // Try month name.
        if month.is_none()
            && let Some(m) = parse_month(token)
        {
            month = Some(m);
            continue;
        }

        // Try time (contains ':').
        if hour.is_none() && token.contains(':') {
            let time_parts: Vec<&str> = token.split(':').collect();
            if time_parts.len() >= 3
                && let (Ok(h), Ok(m), Ok(sec)) = (
                    time_parts[0].parse::<u32>(),
                    time_parts[1].parse::<u32>(),
                    time_parts[2].parse::<u32>(),
                )
            {
                hour = Some(h);
                minute = Some(m);
                second = Some(sec);
                continue;
            }
        }

        // Try number.
        if let Ok(n) = token.parse::<i64>() {
            if n > 99 {
                // Year.
                year = Some(n);
            } else if day.is_none() && (1..=31).contains(&n) {
                day = Some(n as u32);
            } else if year.is_none() {
                // Two-digit year.
                if n < 70 {
                    year = Some(2000 + n);
                } else {
                    year = Some(1900 + n);
                }
            }
            continue;
        }
    }

    let y = year?;
    let m = month?;
    let d = day?;
    let h = hour.unwrap_or(0);
    let min = minute.unwrap_or(0);
    let sec = second.unwrap_or(0);

    date_to_timestamp(y, m, d, h, min, sec)
}

fn is_day_name(token: &str) -> bool {
    let lower = token.to_lowercase();
    matches!(
        lower.as_str(),
        "mon"
            | "tue"
            | "wed"
            | "thu"
            | "fri"
            | "sat"
            | "sun"
            | "monday"
            | "tuesday"
            | "wednesday"
            | "thursday"
            | "friday"
            | "saturday"
            | "sunday"
    )
}

fn parse_month(token: &str) -> Option<u32> {
    if token.len() < 3 {
        return None;
    }
    let prefix: String = token[..3].to_lowercase();
    match prefix.as_str() {
        "jan" => Some(1),
        "feb" => Some(2),
        "mar" => Some(3),
        "apr" => Some(4),
        "may" => Some(5),
        "jun" => Some(6),
        "jul" => Some(7),
        "aug" => Some(8),
        "sep" => Some(9),
        "oct" => Some(10),
        "nov" => Some(11),
        "dec" => Some(12),
        _ => None,
    }
}

fn date_to_timestamp(
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> Option<i64> {
    // Validate ranges.
    if !(1..=12).contains(&month) {
        return None;
    }
    if !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    if year < 1970 {
        return None;
    }

    // Days in each month (non-leap).
    let days_in_month = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    // Compute days from epoch.
    let mut total_days: i64 = 0;

    // Add days for full years from 1970 to year-1.
    for y in 1970..year {
        total_days += if is_leap_year(y) { 366 } else { 365 };
    }

    // Add days for full months in the current year.
    for m in 1..month {
        total_days += days_in_month[m as usize] as i64;
        if m == 2 && is_leap_year(year) {
            total_days += 1;
        }
    }

    // Add remaining days.
    total_days += (day - 1) as i64;

    let timestamp = total_days * 86400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64;
    Some(timestamp)
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}
