/// Shell-quote a string using single quotes (or `$'...'` for control chars).
pub(crate) fn shell_quote(val: &str) -> String {
    if val.is_empty() {
        return "''".to_string();
    }
    let has_control = val.bytes().any(|b| b < 0x20 || b == 0x7f);
    if has_control {
        let mut s = String::from("$'");
        for ch in val.chars() {
            match ch {
                '\x07' => s.push_str("\\a"),
                '\x08' => s.push_str("\\b"),
                '\x1b' => s.push_str("\\E"),
                '\x0c' => s.push_str("\\f"),
                '\n' => s.push_str("\\n"),
                '\r' => s.push_str("\\r"),
                '\t' => s.push_str("\\t"),
                '\x0b' => s.push_str("\\v"),
                '\'' => s.push_str("\\'"),
                '\\' => s.push_str("\\\\"),
                c if (c as u32) < 0x20 || c == '\x7f' => {
                    s.push_str(&format!("\\x{:02x}", c as u32));
                }
                c => s.push(c),
            }
        }
        s.push('\'');
        s
    } else {
        format!("'{}'", val.replace('\'', "'\\''"))
    }
}

/// Expand backslash escape sequences (like `$'...'` quoting).
pub(crate) fn expand_backslash_escapes(val: &str) -> String {
    val.replace("\\n", "\n")
        .replace("\\t", "\t")
        .replace("\\r", "\r")
        .replace("\\\\", "\\")
        .replace("\\a", "\x07")
        .replace("\\b", "\x08")
}
