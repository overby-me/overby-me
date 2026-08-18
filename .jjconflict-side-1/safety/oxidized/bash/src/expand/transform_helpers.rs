use crate::builtins::{RAW_BYTE_BASE, is_pua_raw_byte};

/// Shell-quote a string using single quotes (or `$'...'` for control chars).
pub(crate) fn shell_quote(val: &str) -> String {
    if val.is_empty() {
        return "''".to_string();
    }
    // Check if the string contains control characters or PUA-encoded raw bytes
    // that need ANSI-C $'...' quoting
    let has_special = val.chars().any(|c| {
        let cp = c as u32;
        (cp < 0x20) || c == '\x7f' || is_pua_raw_byte(cp)
    });
    if has_special {
        let mut s = String::from("$'");
        for ch in val.chars() {
            let cp = ch as u32;
            // PUA-encoded raw byte — emit as octal escape of the original byte value
            if is_pua_raw_byte(cp) {
                let byte_val = (cp - RAW_BYTE_BASE) as u8;
                match byte_val {
                    0x07 => s.push_str("\\a"),
                    0x08 => s.push_str("\\b"),
                    0x09 => s.push_str("\\t"),
                    0x0a => s.push_str("\\n"),
                    0x0b => s.push_str("\\v"),
                    0x0c => s.push_str("\\f"),
                    0x0d => s.push_str("\\r"),
                    0x1b => s.push_str("\\E"),
                    0x7f => s.push_str("\\177"),
                    b if b < 0x20 => {
                        // Use octal format like bash does for control chars
                        s.push_str(&format!("\\{:03o}", b));
                    }
                    b'\'' => s.push_str("\\'"),
                    b'\\' => s.push_str("\\\\"),
                    b => {
                        // Printable or high byte — use octal for non-printable
                        if (0x20..0x7f).contains(&b) {
                            s.push(b as char);
                        } else {
                            s.push_str(&format!("\\{:03o}", b));
                        }
                    }
                }
                continue;
            }
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
                    s.push_str(&format!("\\{:03o}", c as u32));
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
