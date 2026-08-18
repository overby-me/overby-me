//! Shell string escaping functions.

// ASCII character constants
const ACK: u8 = 6;
const TAB: u8 = 9;
const LF: u8 = 10;
const CR: u8 = 13;
const US: u8 = 31;
const SPACE: u8 = 32;
const AMPERSAND: u8 = 38;
const SINGLE_QUOTE: u8 = 39;
const PLUS: u8 = 43;
const NINE: u8 = 57;
const QUESTION: u8 = 63;
const UPPERCASE_Z: u8 = 90;
const OPEN_BRACKET: u8 = 91;
const BACKSLASH: u8 = 92;
const CLOSE_BRACKET: u8 = 93;
const UNDERSCORE: u8 = 95;
const BACKTICK: u8 = 96;
const LOWERCASE_Z: u8 = 122;
const TILDE: u8 = 126;
const DEL: u8 = 127;

/// Escape a string for safe use in Bash.
/// Based on the Go direnv BashEscape function.
pub fn bash_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }

    let bytes = s.as_bytes();
    let mut out = String::new();
    let mut escape = false;

    for &ch in bytes {
        match ch {
            ACK => {
                escape = true;
                out.push_str(&format!("\\x{ch:02x}"));
            }
            TAB => {
                escape = true;
                out.push_str("\\t");
            }
            LF => {
                escape = true;
                out.push_str("\\n");
            }
            CR => {
                escape = true;
                out.push_str("\\r");
            }
            0..=US => {
                escape = true;
                out.push_str(&format!("\\x{ch:02x}"));
            }
            // SPACE..=AMPERSAND (32..=38) - quoted
            c if c <= AMPERSAND => {
                escape = true;
                out.push(c as char);
            }
            SINGLE_QUOTE => {
                escape = true;
                out.push('\\');
                out.push('\'');
            }
            // (..=PLUS) 40..=43 - quoted
            c if c <= PLUS => {
                escape = true;
                out.push(c as char);
            }
            // 44..=57 - literal (includes comma, dash, dot, slash, digits)
            c if c <= NINE => {
                out.push(c as char);
            }
            // 58..=63 - quoted (colon, semicolon, <, =, >, ?)
            c if c <= QUESTION => {
                escape = true;
                out.push(c as char);
            }
            // 64..=90 - literal (@, A-Z)
            c if c <= UPPERCASE_Z => {
                out.push(c as char);
            }
            OPEN_BRACKET => {
                escape = true;
                out.push('[');
            }
            BACKSLASH => {
                escape = true;
                out.push('\\');
                out.push('\\');
            }
            UNDERSCORE => {
                out.push('_');
            }
            // CLOSE_BRACKET (93) and CARET (94) - quoted
            c if c <= CLOSE_BRACKET => {
                escape = true;
                out.push(c as char);
            }
            BACKTICK => {
                escape = true;
                out.push(ch as char);
            }
            // lowercase a-z (97-122) - literal
            c if c > BACKTICK && c <= LOWERCASE_Z => {
                out.push(c as char);
            }
            // {, |, } and ~ (123-126) - quoted
            c if c <= TILDE => {
                escape = true;
                out.push(c as char);
            }
            DEL => {
                escape = true;
                out.push_str(&format!("\\x{ch:02x}"));
            }
            _ => {
                escape = true;
                out.push_str(&format!("\\x{ch:02x}"));
            }
        }
    }

    if escape { format!("$'{out}'") } else { out }
}

/// Escape a string for safe use in Fish shell.
pub fn fish_escape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::from("'");

    for &ch in bytes {
        match ch {
            TAB => out.push_str("'\\t'"),
            LF => out.push_str("'\\n'"),
            CR => out.push_str("'\\r'"),
            0..=US => out.push_str(&format!("'\\X{ch:02x}'")),
            SINGLE_QUOTE => {
                out.push('\\');
                out.push('\'');
            }
            BACKSLASH => {
                out.push('\\');
                out.push('\\');
            }
            c if c <= TILDE => out.push(c as char),
            DEL => out.push_str(&format!("'\\X{ch:02x}'")),
            _ => out.push_str(&format!("'\\X{ch:02x}'")),
        }
    }

    out.push('\'');
    out
}

/// Escape a string for safe use in tcsh.
pub fn tcsh_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }

    let bytes = s.as_bytes();
    let mut out = String::new();

    for &ch in bytes {
        match ch {
            ACK => out.push_str(&format!("\\x{ch:02x}")),
            TAB => out.push_str("\\t"),
            LF => out.push_str("\\n"),
            CR => out.push_str("\\r"),
            SPACE => {
                out.push('\\');
                out.push(' ');
            }
            0..=US => out.push_str(&format!("\\x{ch:02x}")),
            c if c <= AMPERSAND => {
                out.push('"');
                out.push(c as char);
                out.push('"');
            }
            SINGLE_QUOTE => {
                out.push('\\');
                out.push('\'');
            }
            c if c <= PLUS => {
                out.push('"');
                out.push(c as char);
                out.push('"');
            }
            c if c <= NINE => out.push(c as char),
            c if c <= QUESTION => {
                out.push('"');
                out.push(c as char);
                out.push('"');
            }
            c if c <= UPPERCASE_Z => out.push(c as char),
            OPEN_BRACKET => {
                out.push('"');
                out.push('[');
                out.push('"');
            }
            BACKSLASH => {
                out.push('\\');
                out.push('\\');
            }
            UNDERSCORE => out.push('_'),
            c if c <= CLOSE_BRACKET => {
                out.push('"');
                out.push(c as char);
                out.push('"');
            }
            c if c <= LOWERCASE_Z => out.push(c as char),
            c if c <= BACKTICK => {
                out.push('"');
                out.push(c as char);
                out.push('"');
            }
            c if c <= TILDE => {
                out.push('"');
                out.push(c as char);
                out.push('"');
            }
            DEL => out.push_str(&format!("\\x{ch:02x}")),
            _ => out.push_str(&format!("\\x{ch:02x}")),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bash_escape_empty() {
        assert_eq!(bash_escape(""), "''");
    }

    #[test]
    fn test_bash_escape_simple() {
        assert_eq!(bash_escape("hello"), "hello");
    }

    #[test]
    fn test_bash_escape_with_spaces() {
        assert_eq!(bash_escape("hello world"), "$'hello world'");
    }

    #[test]
    fn test_bash_escape_single_quote() {
        assert_eq!(bash_escape("it's"), "$'it\\'s'");
    }
}
