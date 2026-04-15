/// Fix POSIX character class patterns that Rust's regex crate can't handle.
/// In POSIX, `[]...]` means a class containing `]` — the `]` right after `[` or `[^`
/// is a literal. Rust regex doesn't support this, so we transform it.
pub fn fix_posix_char_class(pattern: &str) -> String {
    let mut result = String::with_capacity(pattern.len() + 8);
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '[' {
            result.push('[');
            i += 1;
            // Check for negation
            if i < chars.len() && chars[i] == '^' {
                result.push('^');
                i += 1;
            }
            // If next char is ], it's a literal ] in POSIX.
            // We collect the rest of the class first, then append \] at the end
            // so Rust regex doesn't confuse it with the class-closing ].
            let mut has_leading_close = false;
            if i < chars.len() && chars[i] == ']' {
                has_leading_close = true;
                i += 1;
            }
            // Collect rest of character class, escaping bare [ for Rust regex
            let mut class_content = String::new();
            while i < chars.len() && chars[i] != ']' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    class_content.push(chars[i]);
                    class_content.push(chars[i + 1]);
                    i += 2;
                } else if chars[i] == '[' && !(i + 1 < chars.len() && chars[i + 1] == ':') {
                    // Bare [ that's not a POSIX class like [:alpha:]
                    class_content.push_str("\\[");
                    i += 1;
                } else {
                    class_content.push(chars[i]);
                    i += 1;
                }
            }
            result.push_str(&class_content);
            if has_leading_close {
                result.push_str("\\]");
            }
            if i < chars.len() {
                result.push(']');
                i += 1;
            }
        } else if chars[i] == '\\' && i + 1 < chars.len() {
            result.push(chars[i]);
            result.push(chars[i + 1]);
            i += 2;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

pub fn bre_to_ere(bre: &str) -> String {
    let mut result = String::with_capacity(bre.len());
    let chars: Vec<char> = bre.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // Pass through character classes unchanged — inside [...], all chars are literal
        if chars[i] == '[' {
            result.push('[');
            i += 1;
            // Handle negation and ] as first char
            if i < chars.len() && chars[i] == '^' {
                result.push('^');
                i += 1;
            }
            if i < chars.len() && chars[i] == ']' {
                result.push(']');
                i += 1;
            }
            // Copy until closing ]
            // In POSIX BRE, \ inside [] is literal, but Rust regex treats it as escape
            while i < chars.len() && chars[i] != ']' {
                // Handle POSIX classes like [:alpha:]
                if chars[i] == '[' && i + 1 < chars.len() && chars[i + 1] == ':' {
                    result.push('[');
                    result.push(':');
                    i += 2;
                    while i < chars.len() {
                        if chars[i] == ':' && i + 1 < chars.len() && chars[i + 1] == ']' {
                            result.push(':');
                            result.push(']');
                            i += 2;
                            break;
                        }
                        result.push(chars[i]);
                        i += 1;
                    }
                } else if chars[i] == '\\' && i + 1 < chars.len() {
                    // In POSIX BRE, \ inside [] is literal
                    // But Rust regex uses \ for escapes inside [] too
                    // If followed by a char Rust regex recognizes as escape, pass through
                    let next = chars[i + 1];
                    if "dDsSwWtnrfvp0".contains(next)
                        || next == '\\'
                        || next == ']'
                    {
                        // Known Rust regex escape — pass through as-is
                        result.push('\\');
                        result.push(next);
                        i += 2;
                    } else {
                        // Unknown escape in Rust regex — treat \ as literal
                        result.push_str("\\\\");
                        result.push(next);
                        i += 2;
                    }
                } else if chars[i] == '\\' {
                    // \ at end of char class — literal backslash
                    result.push_str("\\\\");
                    i += 1;
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            if i < chars.len() {
                result.push(']');
                i += 1;
            }
            continue;
        }

        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                '(' => {
                    result.push('(');
                    i += 2;
                }
                ')' => {
                    result.push(')');
                    i += 2;
                }
                '{' => {
                    result.push('{');
                    i += 2;
                }
                '}' => {
                    result.push('}');
                    i += 2;
                }
                '|' => {
                    result.push('|');
                    i += 2;
                }
                '+' => {
                    result.push('+');
                    i += 2;
                }
                '?' => {
                    result.push('?');
                    i += 2;
                }
                'n' => {
                    result.push('\n');
                    i += 2;
                }
                't' => {
                    result.push('\t');
                    i += 2;
                }
                '1'..='9' => {
                    result.push('\\');
                    result.push(chars[i + 1]);
                    i += 2;
                }
                _ => {
                    result.push('\\');
                    result.push(chars[i + 1]);
                    i += 2;
                }
            }
        } else if chars[i] == '(' {
            result.push_str("\\(");
            i += 1;
        } else if chars[i] == ')' {
            result.push_str("\\)");
            i += 1;
        } else if chars[i] == '{' {
            result.push_str("\\{");
            i += 1;
        } else if chars[i] == '}' {
            result.push_str("\\}");
            i += 1;
        } else if chars[i] == '|' {
            result.push_str("\\|");
            i += 1;
        } else if chars[i] == '+' {
            result.push_str("\\+");
            i += 1;
        } else if chars[i] == '?' {
            result.push_str("\\?");
            i += 1;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}
