// `--variable` and `--expand-*` support.
//
// Variables are declared with `--variable name=value`, `--variable name@file`,
// or `--variable %ENV[=default]`. They're expanded in `--expand-*` arguments
// via `{{name}}` or `{{name:func}}` placeholders. Functions: `trim`, `json`,
// `url`, `b64`, `64dec`. Unknown placeholders or non-existent names are kept
// verbatim. Backslash before `{{` produces a literal — the backslash itself
// is dropped (matching curl's escape rule, test 428).

#![allow(clippy::collapsible_if, clippy::manual_div_ceil, clippy::manual_is_multiple_of)]

use std::env;
use std::fs;

/// Look up a variable by name. Returns the most recent definition.
pub(crate) fn lookup<'a>(vars: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a [u8]> {
    vars.iter()
        .rev()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_slice())
}

/// Parse a single `--variable VALUE` argument and append to the variable list.
/// Forms:
///   - `name=value` — literal assign
///   - `name@file` — read content from file (`-` = stdin)
///   - `%ENV` — use environment variable; missing = error
///   - `%ENV=default` — use env, fall back to default literal
///   - `%ENV@file` — use env, fall back to file content
///
/// The name may be followed by a byte range `[N-M]` (inclusive, or `[N-]`
/// for open-ended). The range slices the source value (test 784, 790).
pub(crate) fn parse_variable(arg: &str, vars: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
    let (name_spec, sep_pos, sep_char) = {
        let eq = arg.find('=');
        let at = arg.find('@');
        match (eq, at) {
            (Some(e), Some(a)) if e < a => (&arg[..e], Some(e), Some('=')),
            (Some(e), Some(_)) => (&arg[..e], Some(e), Some('=')),
            (Some(e), None) => (&arg[..e], Some(e), Some('=')),
            (None, Some(a)) => (&arg[..a], Some(a), Some('@')),
            (None, None) => (arg, None, None),
        }
    };
    if name_spec.is_empty() {
        return Err("missing variable name".into());
    }
    let from_env = name_spec.starts_with('%');
    let name_with_range = if from_env { &name_spec[1..] } else { name_spec };
    let (name, range): (&str, Option<(usize, Option<usize>)>) =
        match name_with_range.split_once('[') {
            Some((n, r)) => match r.strip_suffix(']') {
                Some(spec) => match parse_range(spec) {
                    Some(rng) => (n, Some(rng)),
                    None => return Err(format!("invalid byte range: {spec}")),
                },
                None => {
                    return Err(format!(
                        "unterminated [ in variable name: {name_with_range}"
                    ));
                }
            },
            None => (name_with_range, None),
        };
    if name.is_empty() {
        return Err("missing variable name after %".into());
    }
    if !is_valid_var_name(name) {
        return Err(format!("invalid variable name: {name}"));
    }

    let mut value: Vec<u8> = if from_env {
        match env::var(name) {
            Ok(v) => v.into_bytes(),
            Err(_) => {
                // Fall back to default after `=` or `@`.
                match (sep_pos, sep_char) {
                    (Some(p), Some('=')) => arg.as_bytes()[p + 1..].to_vec(),
                    (Some(p), Some('@')) => read_value_file(&arg[p + 1..])?,
                    _ => return Err(format!("variable %{name} not set")),
                }
            }
        }
    } else {
        match (sep_pos, sep_char) {
            (Some(p), Some('=')) => arg.as_bytes()[p + 1..].to_vec(),
            (Some(p), Some('@')) => read_value_file(&arg[p + 1..])?,
            _ => return Err(format!("missing value for variable {name}")),
        }
    };

    if let Some((start, end_opt)) = range {
        if start >= value.len() {
            // Range starts past EOF — empty value (test 789, 791).
            value.clear();
        } else {
            let end = match end_opt {
                Some(e) => e.min(value.len() - 1) + 1,
                None => value.len(),
            };
            value = value[start..end].to_vec();
        }
    }

    vars.push((name.to_string(), value));
    Ok(())
}

fn parse_range(spec: &str) -> Option<(usize, Option<usize>)> {
    let (s, e) = spec.split_once('-')?;
    let start = s.parse::<usize>().ok()?;
    let end = if e.is_empty() {
        None
    } else {
        Some(e.parse::<usize>().ok()?)
    };
    if let Some(e) = end {
        if e < start {
            return None;
        }
    }
    Some((start, end))
}

fn read_value_file(path: &str) -> Result<Vec<u8>, String> {
    if path == "-" {
        use std::io::Read;
        let mut buf = Vec::new();
        io::stdin()
            .read_to_end(&mut buf)
            .map_err(|e| format!("read stdin: {e}"))?;
        Ok(buf)
    } else {
        fs::read(path).map_err(|e| format!("read {path}: {e}"))
    }
}

use std::io;

/// Variable names: alphanumerics, `_` and `-`. Anything else is invalid and
/// the placeholder is kept verbatim (e.g. `{{not.good}}`). Curl caps the
/// length below 128 chars — test 448's 128-A placeholder must stay literal.
fn is_valid_var_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Expand `{{name}}` / `{{name:func}}` placeholders in `template` against the
/// given variable list. Backslash before `{{` is treated as an escape — the
/// `{{...}}` is preserved verbatim and the backslash is dropped.
///
/// On any failure (unknown name, invalid name, bad function) the placeholder
/// is preserved verbatim. This matches curl's behavior for test 428's
/// `{{not.good}}` and `{{}}` cases — invalid placeholders pass through.
pub(crate) fn expand(template: &[u8], vars: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(template.len());
    let mut i = 0;
    while i < template.len() {
        // Backslash escape: `\{{` becomes literal `{{` (backslash dropped,
        // braces preserved verbatim through the rest of the placeholder).
        if template[i] == b'\\' && template[i + 1..].starts_with(b"{{") {
            // Find the matching `}}` and copy the entire `{{...}}` literal.
            if let Some(end) = find_close_brace(template, i + 1 + 2) {
                out.extend_from_slice(&template[i + 1..end + 2]);
                i = end + 2;
                continue;
            } else {
                // Unmatched — emit backslash + brace as-is.
                out.push(template[i]);
                i += 1;
                continue;
            }
        }
        if template[i..].starts_with(b"{{") {
            if let Some(end) = find_close_brace(template, i + 2) {
                let inner = &template[i + 2..end];
                let literal_placeholder = || {
                    // Emit `{{...}}` verbatim.
                    let mut v = Vec::with_capacity(end + 2 - i);
                    v.extend_from_slice(&template[i..end + 2]);
                    v
                };
                let inner_str = match std::str::from_utf8(inner) {
                    Ok(s) => s,
                    Err(_) => {
                        out.extend(literal_placeholder());
                        i = end + 2;
                        continue;
                    }
                };
                let (name, funcs) = match inner_str.split_once(':') {
                    Some((n, f)) => (n, f),
                    None => (inner_str, ""),
                };
                if !is_valid_var_name(name) {
                    out.extend(literal_placeholder());
                    i = end + 2;
                    continue;
                }
                // Unknown variable expands to empty string (test 448 —
                // `{{curl_NOT_SET}}` becomes "" when the var is never set).
                let empty_holder: Vec<u8> = Vec::new();
                let raw = lookup(vars, name).unwrap_or(empty_holder.as_slice());
                let mut current: Vec<u8> = raw.to_vec();
                let mut ok = true;
                if !funcs.is_empty() {
                    for f in funcs.split(':') {
                        match apply_func(f, &current) {
                            Some(v) => current = v,
                            None => {
                                ok = false;
                                break;
                            }
                        }
                    }
                }
                if ok {
                    out.extend_from_slice(&current);
                } else {
                    out.extend(literal_placeholder());
                }
                i = end + 2;
                continue;
            }
        }
        out.push(template[i]);
        i += 1;
    }
    out
}

fn find_close_brace(template: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < template.len() {
        if template[i] == b'}' && template[i + 1] == b'}' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Set of valid filter function names. Used by the expand-time validator
/// (test 452 — `--expand-data` with an unknown function exits 2 before any
/// network activity).
pub(crate) fn is_valid_func(name: &str) -> bool {
    matches!(name, "trim" | "json" | "url" | "b64" | "64dec")
}

/// Validate a `--expand-*` template — return Err on a placeholder that uses
/// an unknown function. Other malformed placeholders are preserved literally
/// at expand-time and don't error here.
pub(crate) fn validate(template: &[u8]) -> Result<(), String> {
    let mut i = 0;
    while i < template.len() {
        if template[i] == b'\\' && template[i + 1..].starts_with(b"{{") {
            if let Some(end) = find_close_brace(template, i + 1 + 2) {
                i = end + 2;
                continue;
            }
            i += 1;
            continue;
        }
        if template[i..].starts_with(b"{{") {
            if let Some(end) = find_close_brace(template, i + 2) {
                let inner = std::str::from_utf8(&template[i + 2..end]).unwrap_or("");
                if let Some((name, funcs)) = inner.split_once(':') {
                    if is_valid_var_name(name) {
                        for f in funcs.split(':') {
                            if !is_valid_func(f) {
                                return Err(format!("unknown variable function: {f}"));
                            }
                        }
                    }
                }
                i = end + 2;
                continue;
            }
        }
        i += 1;
    }
    Ok(())
}

/// Apply one filter function. Returns None for unknown functions OR for
/// `64dec` on bad base64 — caller turns that into the literal `[64dec-fail]`
/// substitution per curl semantics (test 487). For other failures we keep
/// the placeholder verbatim instead.
fn apply_func(name: &str, val: &[u8]) -> Option<Vec<u8>> {
    match name {
        "trim" => {
            // Strip ASCII whitespace from both ends.
            let s = val
                .iter()
                .position(|b| !b.is_ascii_whitespace())
                .unwrap_or(val.len());
            let e = val
                .iter()
                .rposition(|b| !b.is_ascii_whitespace())
                .map(|p| p + 1)
                .unwrap_or(0);
            Some(if s < e {
                val[s..e].to_vec()
            } else {
                Vec::new()
            })
        }
        "json" => Some(json_encode(val)),
        "url" => Some(url_encode(val)),
        "b64" => Some(base64_encode(val)),
        "64dec" => match base64_decode(val) {
            Ok(v) => Some(v),
            Err(_) => Some(b"[64dec-fail]".to_vec()),
        },
        _ => None,
    }
}

fn json_encode(val: &[u8]) -> Vec<u8> {
    // curl's json filter: backslash-escapes `"` and `\`, encodes control bytes
    // (<0x20) as \uNNNN, leaves everything else (including high bytes) as-is.
    let mut out = Vec::with_capacity(val.len());
    for &b in val {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0C => out.extend_from_slice(b"\\f"),
            0..=0x1F => {
                out.extend_from_slice(format!("\\u{:04x}", b).as_bytes());
            }
            _ => out.push(b),
        }
    }
    out
}

fn url_encode(val: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(val.len());
    for &b in val {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b);
        } else {
            out.extend_from_slice(format!("%{:02X}", b).as_bytes());
        }
    }
    out
}

fn base64_encode(val: &[u8]) -> Vec<u8> {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(((val.len() + 2) / 3) * 4);
    let mut i = 0;
    while i + 3 <= val.len() {
        let n = ((val[i] as u32) << 16) | ((val[i + 1] as u32) << 8) | (val[i + 2] as u32);
        out.push(ALPHA[(n >> 18) as usize & 0x3F]);
        out.push(ALPHA[(n >> 12) as usize & 0x3F]);
        out.push(ALPHA[(n >> 6) as usize & 0x3F]);
        out.push(ALPHA[n as usize & 0x3F]);
        i += 3;
    }
    let rem = val.len() - i;
    if rem == 1 {
        let n = (val[i] as u32) << 16;
        out.push(ALPHA[(n >> 18) as usize & 0x3F]);
        out.push(ALPHA[(n >> 12) as usize & 0x3F]);
        out.push(b'=');
        out.push(b'=');
    } else if rem == 2 {
        let n = ((val[i] as u32) << 16) | ((val[i + 1] as u32) << 8);
        out.push(ALPHA[(n >> 18) as usize & 0x3F]);
        out.push(ALPHA[(n >> 12) as usize & 0x3F]);
        out.push(ALPHA[(n >> 6) as usize & 0x3F]);
        out.push(b'=');
    }
    out
}

fn base64_decode(val: &[u8]) -> Result<Vec<u8>, ()> {
    fn dec(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    // Strip trailing `=` padding; reject any other non-alphabet byte.
    let mut clean: Vec<u8> = Vec::with_capacity(val.len());
    let mut padding = 0;
    for &b in val {
        if b == b'=' {
            padding += 1;
        } else if padding > 0 {
            // padding before end is invalid
            return Err(());
        } else if b.is_ascii_whitespace() {
            // skip
        } else {
            clean.push(b);
        }
    }
    if (clean.len() + padding) % 4 != 0 {
        return Err(());
    }
    let mut out = Vec::with_capacity((clean.len() / 4) * 3 + 2);
    let mut i = 0;
    while i + 4 <= clean.len() {
        let v0 = dec(clean[i]).ok_or(())? as u32;
        let v1 = dec(clean[i + 1]).ok_or(())? as u32;
        let v2 = dec(clean[i + 2]).ok_or(())? as u32;
        let v3 = dec(clean[i + 3]).ok_or(())? as u32;
        let n = (v0 << 18) | (v1 << 12) | (v2 << 6) | v3;
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
        out.push(n as u8);
        i += 4;
    }
    let rem = clean.len() - i;
    if rem == 2 {
        let v0 = dec(clean[i]).ok_or(())? as u32;
        let v1 = dec(clean[i + 1]).ok_or(())? as u32;
        let n = (v0 << 18) | (v1 << 12);
        out.push((n >> 16) as u8);
    } else if rem == 3 {
        let v0 = dec(clean[i]).ok_or(())? as u32;
        let v1 = dec(clean[i + 1]).ok_or(())? as u32;
        let v2 = dec(clean[i + 2]).ok_or(())? as u32;
        let n = (v0 << 18) | (v1 << 12) | (v2 << 6);
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
    } else if rem != 0 {
        return Err(());
    }
    Ok(out)
}
