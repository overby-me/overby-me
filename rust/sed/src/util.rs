/// Convert a byte to its control character equivalent.
/// For ASCII letters, uppercases first so \ca == \cA.
/// For other chars, XOR with 0x40.
pub fn ctrl_char(b: u8) -> u8 {
    if b.is_ascii_lowercase() {
        b.to_ascii_uppercase() ^ 0x40
    } else {
        b ^ 0x40
    }
}

pub fn escape_string_bytes(data: &[u8]) -> String {
    let mut result = String::new();
    for t in escape_string_tokens(data) {
        result.push_str(&t);
    }
    result
}

/// Escape bytes for `l` command output, producing atomic tokens.
/// Each token is either a single visible char or a full escape sequence
/// (e.g. `\035`) that must not be split across line wraps.
pub fn escape_string_tokens(data: &[u8]) -> Vec<String> {
    let mut result = Vec::with_capacity(data.len());
    for &b in data {
        let s = match b {
            b'\\' => "\\\\".to_string(),
            0x07 => "\\a".to_string(),
            0x08 => "\\b".to_string(),
            0x0c => "\\f".to_string(),
            b'\n' => "\\n".to_string(),
            b'\r' => "\\r".to_string(),
            b'\t' => "\\t".to_string(),
            0x0b => "\\v".to_string(),
            0x00 => "\\000".to_string(),
            b if b.is_ascii() && !b.is_ascii_control() => (b as char).to_string(),
            b => format!("\\{:03o}", b),
        };
        result.push(s);
    }
    result
}

