//! String literal parser for assembler directives.
//!
//! Shared by all four assembler backends (x86, i686, ARM, RISC-V) to ensure
//! consistent handling of C/GNU assembler escape sequences.

/// Parse a string literal with escape sequences, returning raw bytes.
///
/// This is the **canonical** implementation shared by all assembler backends
/// (x86, i686, ARM, RISC-V). Having one implementation prevents bugs where
/// different backends handle escapes differently (e.g. returning `String`
/// instead of `Vec<u8>` causes multi-byte UTF-8 expansion of byte values > 127).
///
/// Supports the standard C/GNU assembler escape sequences:
///   `\n` `\t` `\r` `\\` `\"` `\a` `\b` `\f` `\v`
///   Octal: `\0` .. `\377` (1-3 digits)
///   Hex:   `\x00` .. `\xFF` (1-2 digits)
///
/// The input `s` should be a trimmed string starting with `"`. The parser scans
/// character-by-character until the closing `"` (rather than assuming it is the
/// last character), which correctly handles edge cases where extra content
/// follows the string literal.
pub fn parse_string_literal(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    if !s.starts_with('"') {
        return Err(format!("expected string literal: {}", s));
    }

    let mut bytes = Vec::new();
    let mut chars = s[1..].chars();
    loop {
        match chars.next() {
            None => return Err("unterminated string".to_string()),
            Some('"') => break,
            Some('\\') => {
                match chars.next() {
                    None => return Err("unterminated escape".to_string()),
                    Some('n') => bytes.push(b'\n'),
                    Some('t') => bytes.push(b'\t'),
                    Some('r') => bytes.push(b'\r'),
                    Some('\\') => bytes.push(b'\\'),
                    Some('"') => bytes.push(b'"'),
                    Some('a') => bytes.push(7),  // bell
                    Some('b') => bytes.push(8),  // backspace
                    Some('f') => bytes.push(12), // form feed
                    Some('v') => bytes.push(11), // vertical tab
                    Some(c) if ('0'..='7').contains(&c) => {
                        // Octal escape: \N, \NN, or \NNN (up to 3 digits)
                        let mut val = c as u32 - '0' as u32;
                        for _ in 0..2 {
                            if let Some(&next) = chars.as_str().as_bytes().first() {
                                if (b'0'..=b'7').contains(&next) {
                                    val = val * 8 + (next - b'0') as u32;
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                        }
                        bytes.push(val as u8);
                    }
                    Some('x') => {
                        // Hex escape: \xNN (up to 2 digits)
                        let mut val = 0u32;
                        for _ in 0..2 {
                            if let Some(&next) = chars.as_str().as_bytes().first() {
                                if next.is_ascii_hexdigit() {
                                    val = val * 16
                                        + match next {
                                            b'0'..=b'9' => (next - b'0') as u32,
                                            b'a'..=b'f' => (next - b'a' + 10) as u32,
                                            b'A'..=b'F' => (next - b'A' + 10) as u32,
                                            _ => unreachable!(),
                                        };
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                        }
                        bytes.push(val as u8);
                    }
                    Some(c) => {
                        // Unknown escape: emit the character as a raw byte.
                        // (GNU as treats unknown \X as literal X.)
                        bytes.push(c as u8);
                    }
                }
            }
            Some(c) => {
                // Regular character - encode as UTF-8
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf);
                bytes.extend_from_slice(encoded.as_bytes());
            }
        }
    }

    Ok(bytes)
}
