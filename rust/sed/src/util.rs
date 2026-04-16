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
    for &b in data {
        match b {
            b'\\' => result.push_str("\\\\"),
            0x07 => result.push_str("\\a"),
            0x08 => result.push_str("\\b"),
            0x0c => result.push_str("\\f"),
            b'\n' => result.push_str("\\n"),
            b'\r' => result.push_str("\\r"),
            b'\t' => result.push_str("\\t"),
            0x0b => result.push_str("\\v"),
            0x00 => result.push_str("\\000"),
            b if b.is_ascii() && !b.is_ascii_control() => result.push(b as char),
            b => result.push_str(&format!("\\{:03o}", b)),
        }
    }
    result
}

