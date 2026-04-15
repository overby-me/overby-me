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

pub fn escape_string(s: &str) -> String {
    let mut result = String::new();
    for ch in s.chars() {
        match ch {
            '\\' => result.push_str("\\\\"),
            '\x07' => result.push_str("\\a"),
            '\x08' => result.push_str("\\b"),
            '\x0c' => result.push_str("\\f"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\x0b' => result.push_str("\\v"),
            c if c.is_ascii() && !c.is_ascii_control() => result.push(c),
            c => {
                for b in c.to_string().bytes() {
                    result.push_str(&format!("\\{:03o}", b));
                }
            }
        }
    }
    result
}
