//! dotenv parser compatible with the Go direnv dotenv package.
//! Supports single-quoted, double-quoted, and unquoted values,
//! multiline quoted values, variable expansion, and comments.

use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;

static LINES_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\r\n]+").unwrap());

// The LINE regex matches:
// - blank/comment lines
// - KEY=VALUE lines with optional export prefix
// - Values can be single-quoted, double-quoted, or unquoted
static LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?x)
        \A
        \s*
        (?:|
          \#.*|
          (?:export\s+)?
          ([\w.]+)
          (?:\s*=\s*|:\s+?)
          (
            '(?:\\'|[^'])*'
            |
            "(?:\\"|[^"])*"
            |
            [^\s\#\n]+
          )?
          \s*
          (?:\#.*)?
        )
        \z
    "#,
    )
    .unwrap()
});

static ESC_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\([^$])").unwrap());

/// Parse a .env file string into a map of key=value pairs.
pub fn parse(data: &str) -> Result<BTreeMap<String, String>, String> {
    let mut dotenv: BTreeMap<String, String> = BTreeMap::new();
    let lines: Vec<&str> = LINES_RE.split(data).collect();

    let mut in_multiline = false;
    let mut multiline_value = String::new();
    let mut quote_char: u8 = 0;

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        if in_multiline {
            multiline_value.push('\n');
            multiline_value.push_str(line);

            // Check if this line completes the multi-line value
            if line.trim().ends_with(quote_char as char) {
                if let Some(caps) = LINE_RE.captures(&multiline_value)
                    && let Some(key_match) = caps.get(1)
                {
                    let key = key_match.as_str();
                    let value = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                    parse_value(key, value, &mut dotenv);
                }
                in_multiline = false;
            }
            i += 1;
            continue;
        }

        // Check for the beginning of a multi-line value
        if let Some(sep_idx) = line.find('=').or_else(|| line.find(':'))
            && sep_idx > 0
            && sep_idx + 1 < line.len()
        {
            let after_sep = &line[sep_idx + 1..];
            let trimmed = after_sep.trim_start();

            if !trimmed.is_empty()
                && (trimmed.as_bytes()[0] == b'"' || trimmed.as_bytes()[0] == b'\'')
            {
                let qc = trimmed.as_bytes()[0];
                let count = trimmed.as_bytes().iter().filter(|&&b| b == qc).count();
                if count == 1 {
                    in_multiline = true;
                    multiline_value = line.to_string();
                    quote_char = qc;
                    i += 1;
                    continue;
                }
            }
        }

        // Normal line processing
        if !line.trim().is_empty() && !line.trim().starts_with('#') && !LINE_RE.is_match(line) {
            return Err(format!("invalid line: {line}"));
        }

        if let Some(caps) = LINE_RE.captures(line)
            && let Some(key_match) = caps.get(1)
            && !key_match.as_str().is_empty()
        {
            let key = key_match.as_str();
            let value = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            parse_value(key, value, &mut dotenv);
        }

        i += 1;
    }

    if in_multiline {
        return Err("unclosed quoted value in .env file".to_string());
    }

    Ok(dotenv)
}

fn parse_value(key: &str, value: &str, dotenv: &mut BTreeMap<String, String>) {
    if value.len() <= 1 {
        dotenv.insert(key.to_string(), value.to_string());
        return;
    }

    let mut result = value.to_string();
    let mut single_quoted = false;

    if value.starts_with('\'') && value.ends_with('\'') {
        // Single-quoted: no expansion
        single_quoted = true;
        result = value[1..value.len() - 1].to_string();
    } else if value.starts_with('"') && value.ends_with('"') {
        // Double-quoted: expand newlines and unescape
        result = value[1..value.len() - 1].to_string();
        result = expand_newlines(&result);
        result = unescape_characters(&result);
    }

    if !single_quoted {
        result = expand_env(&result, dotenv);
    }

    dotenv.insert(key.to_string(), result);
}

fn unescape_characters(value: &str) -> String {
    ESC_RE.replace_all(value, "$1").to_string()
}

fn expand_newlines(value: &str) -> String {
    value.replace("\\n", "\n").replace("\\r", "\r")
}

fn expand_env(value: &str, dotenv: &BTreeMap<String, String>) -> String {
    // Handle $VAR and ${VAR} and ${VAR:-default} patterns
    let mut result = String::new();
    let bytes = value.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'$' && i + 1 < len {
            if bytes[i + 1] == b'{' {
                // ${VAR} or ${VAR:-default}
                if let Some(close) = value[i + 2..].find('}') {
                    let inner = &value[i + 2..i + 2 + close];
                    let (env_key, default_value, has_default) = split_key_and_default(inner, ":-");
                    let expanded = lookup(env_key, dotenv, default_value, has_default);
                    result.push_str(&expanded);
                    i = i + 2 + close + 1;
                    continue;
                }
            } else if bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_' {
                // $VAR
                let start = i + 1;
                let mut end = start;
                while end < len && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                    end += 1;
                }
                let env_key = &value[start..end];
                let expanded = lookup(env_key, dotenv, "", false);
                result.push_str(&expanded);
                i = end;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }

    result
}

fn split_key_and_default<'a>(value: &'a str, sep: &str) -> (&'a str, &'a str, bool) {
    if let Some(idx) = value.find(sep) {
        (&value[..idx], &value[idx + sep.len()..], true)
    } else {
        (value, "", false)
    }
}

fn lookup(
    key: &str,
    dotenv: &BTreeMap<String, String>,
    default: &str,
    has_default: bool,
) -> String {
    // Check dotenv first, then system env
    if let Some(val) = dotenv.get(key) {
        return val.clone();
    }
    if let Ok(val) = std::env::var(key)
        && !val.is_empty()
    {
        return val;
    }
    if has_default {
        default.to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple() {
        let result = parse("FOO=bar\nBAZ=qux").unwrap();
        assert_eq!(result.get("FOO").unwrap(), "bar");
        assert_eq!(result.get("BAZ").unwrap(), "qux");
    }

    #[test]
    fn test_quoted() {
        let result = parse("FOO='bar baz'\nBAR=\"hello world\"").unwrap();
        assert_eq!(result.get("FOO").unwrap(), "bar baz");
        assert_eq!(result.get("BAR").unwrap(), "hello world");
    }

    #[test]
    fn test_comments() {
        let result = parse("# comment\nFOO=bar # inline").unwrap();
        assert_eq!(result.get("FOO").unwrap(), "bar");
    }

    #[test]
    fn test_export_prefix() {
        let result = parse("export FOO=bar").unwrap();
        assert_eq!(result.get("FOO").unwrap(), "bar");
    }

    #[test]
    fn test_newline_expansion() {
        let result = parse(r#"FOO="hello\nworld""#).unwrap();
        assert_eq!(result.get("FOO").unwrap(), "hello\nworld");
    }

    #[test]
    fn test_no_expansion_single_quote() {
        let result = parse(r#"FOO='$HOME'"#).unwrap();
        assert_eq!(result.get("FOO").unwrap(), "$HOME");
    }

    #[test]
    fn test_variable_expansion() {
        let result = parse("FOO=bar\nBAZ=$FOO").unwrap();
        assert_eq!(result.get("BAZ").unwrap(), "bar");
    }

    #[test]
    fn test_default_value() {
        let result = parse("FOO=${MISSING:-default}").unwrap();
        assert_eq!(result.get("FOO").unwrap(), "default");
    }
}
