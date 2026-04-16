use std::process;

/// Check if a pattern contains backreferences (\1-\9).
pub(crate) fn has_backreferences(pattern: &str) -> bool {
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    let mut in_bracket = false;
    while i < chars.len() {
        if chars[i] == '[' && !in_bracket {
            in_bracket = true;
        } else if chars[i] == ']' && in_bracket {
            in_bracket = false;
        } else if chars[i] == '\\' && i + 1 < chars.len() && !in_bracket {
            if chars[i + 1].is_ascii_digit() && chars[i + 1] != '0' {
                return true;
            }
            i += 1; // skip escaped char
        }
        i += 1;
    }
    false
}

/// Count the number of capturing groups in a pattern (after BRE->ERE conversion).
pub(crate) fn count_groups(pattern: &str) -> usize {
    let chars: Vec<char> = pattern.chars().collect();
    let mut count = 0;
    let mut i = 0;
    let mut in_bracket = false;
    while i < chars.len() {
        if chars[i] == '[' && !in_bracket {
            in_bracket = true;
        } else if chars[i] == ']' && in_bracket {
            in_bracket = false;
        } else if chars[i] == '\\' && i + 1 < chars.len() {
            i += 2;
            continue;
        } else if chars[i] == '(' && !in_bracket {
            // Check it's not (?:...) non-capturing group
            if !(i + 1 < chars.len() && chars[i + 1] == '?') {
                count += 1;
            }
        }
        i += 1;
    }
    count
}

/// Find the highest backreference number in a pattern.
pub(crate) fn max_backref(pattern: &str) -> usize {
    let chars: Vec<char> = pattern.chars().collect();
    let mut max = 0;
    let mut i = 0;
    let mut in_bracket = false;
    while i < chars.len() {
        if chars[i] == '[' && !in_bracket {
            in_bracket = true;
        } else if chars[i] == ']' && in_bracket {
            in_bracket = false;
        } else if chars[i] == '\\' && i + 1 < chars.len() && !in_bracket {
            if chars[i + 1].is_ascii_digit() && chars[i + 1] != '0' {
                let n = (chars[i + 1] as u32 - '0' as u32) as usize;
                if n > max {
                    max = n;
                }
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    max
}

/// Warn if a pattern looks like a misused POSIX character class (e.g., [:space:]).
/// The correct syntax is [[:space:]] (with outer brackets).
pub(crate) fn warn_char_class_misuse(pattern: &str) {
    const VALID_CLASSES: &[&str] = &[
        "alnum", "alpha", "blank", "cntrl", "digit", "graph", "lower", "print", "punct", "space",
        "upper", "xdigit",
    ];
    // Check if the entire pattern is [:classname:] (a bracket expression that
    // looks like a POSIX class)
    let chars: Vec<char> = pattern.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        if chars[i] == '[' && i + 1 < len && chars[i + 1] == ':' {
            // Found [: — check if this is inside another bracket expr
            // If the pattern starts with [: (or has [: not preceded by another [),
            // it might be a misuse
            let start = i;
            i += 2;
            let name_start = i;
            while i < len && chars[i] != ':' && chars[i] != ']' {
                i += 1;
            }
            if i + 1 < len && chars[i] == ':' && chars[i + 1] == ']' {
                let name: String = chars[name_start..i].iter().collect();
                if VALID_CLASSES.contains(&name.as_str()) {
                    // Check if this [: is at the start of a bracket expression
                    // (i.e., the [ that starts [: IS the bracket open)
                    // This means [: is the bracket expression, not inside one
                    // Check if [: is properly inside a bracket expression
                    let inside_bracket = start > 0
                        && (chars[start - 1] == '['
                            || (start > 1
                                && chars[start - 1] == '^'
                                && chars[start - 2] == '['));
                    if !inside_bracket {
                        eprintln!(
                            "grep: character class syntax is [[:{name}:]], not [:{name}:]"
                        );
                        process::exit(2);
                    }
                }
            }
        }
        i += 1;
    }
}

/// Validate a POSIX character class name. Exits with code 2 if invalid.
pub(crate) fn validate_posix_class(name: &str) {
    const VALID_CLASSES: &[&str] = &[
        "alnum", "alpha", "blank", "cntrl", "digit", "graph", "lower", "print", "punct", "space",
        "upper", "xdigit",
    ];
    if !VALID_CLASSES.contains(&name) {
        eprintln!("grep: Invalid character class name");
        process::exit(2);
    }
}

/// Escape invalid interval expressions in ERE patterns so the regex crate
/// treats them as literals. POSIX says invalid intervals like {, {1, {,2}
/// should be treated as literal characters.
pub(crate) fn escape_invalid_ere_intervals(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let len = chars.len();
    let mut result = String::with_capacity(len);
    let mut i = 0;
    let mut in_bracket = false;
    let mut paren_depth: i32 = 0;
    let mut at_expr_start = true; // track if we're at the start of an expression

    while i < len {
        // Handle * at start of expression (treat as literal)
        if chars[i] == '*' && at_expr_start && !in_bracket {
            result.push_str("\\*");
            i += 1;
            continue;
        }

        if chars[i] == '\\' && i + 1 < len {
            result.push(chars[i]);
            result.push(chars[i + 1]);
            i += 2;
            at_expr_start = false;
            continue;
        }
        if chars[i] == '[' && !in_bracket {
            in_bracket = true;
            result.push('[');
            i += 1;
            // Handle negation
            if i < len && chars[i] == '^' {
                result.push('^');
                i += 1;
            }
            // Handle ] as first char in bracket
            if i < len && chars[i] == ']' {
                result.push(']');
                i += 1;
            }
            // Process bracket content
            while i < len && chars[i] != ']' {
                if chars[i] == '[' && i + 1 < len
                    && (chars[i + 1] == ':' || chars[i + 1] == '.' || chars[i + 1] == '=')
                {
                    let delim = chars[i + 1];
                    let class_start = i;
                    i += 2;
                    let name_start = i;
                    let mut found = false;
                    while i + 1 < len {
                        if chars[i] == delim && chars[i + 1] == ']' {
                            found = true;
                            break;
                        }
                        i += 1;
                    }
                    if found {
                        let name: String = chars[name_start..i].iter().collect();
                        if delim == ':' {
                            validate_posix_class(&name);
                            for c in &chars[class_start..i + 2] {
                                result.push(*c);
                            }
                        } else {
                            // Collating element [.x.] or equivalence class [=x=]
                            // In C locale, just use the character directly
                            result.push_str(&name);
                        }
                        i += 2;
                    } else {
                        result.push_str("\\[");
                        result.push(delim);
                        i = class_start + 2;
                    }
                } else if chars[i] == '[' {
                    // Bare [ inside bracket expr — escape for regex crate
                    result.push_str("\\[");
                    i += 1;
                } else if chars[i] == '\\' {
                    // In POSIX bracket expressions, \ is literal
                    result.push_str("\\\\");
                    i += 1;
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            if i < len {
                result.push(']');
                i += 1;
            }
            in_bracket = false;
            at_expr_start = false;
            continue;
        }
        if chars[i] == '{' && !in_bracket {
            // Try to parse as interval
            let mut j = i + 1;
            while j < len && chars[j] != '}' && chars[j] != '{' {
                j += 1;
            }
            if j < len && chars[j] == '}' {
                let content: String = chars[i + 1..j].iter().collect();
                if is_valid_interval(&content) {
                    // Valid interval — pass through
                    result.push('{');
                    result.push_str(&content);
                    result.push('}');
                    i = j + 1;
                    continue;
                }
                if content.is_empty() || is_interval_too_large(&content) {
                    eprintln!("grep: Invalid content of \\{{\\}}");
                    process::exit(2);
                }
            }
            // Invalid or unclosed — escape as literal
            result.push_str("\\{");
            i += 1;
            continue;
        }
        if chars[i] == '(' {
            paren_depth += 1;
            result.push('(');
            i += 1;
            at_expr_start = true;
            continue;
        }
        if chars[i] == ')' {
            if paren_depth > 0 {
                paren_depth -= 1;
                result.push(')');
            } else {
                // Unmatched ) — escape as literal
                result.push_str("\\)");
            }
            i += 1;
            at_expr_start = false;
            continue;
        }
        if chars[i] == '|' {
            result.push('|');
            i += 1;
            at_expr_start = true;
            continue;
        }
        result.push(chars[i]);
        i += 1;
        at_expr_start = false;
    }
    result
}

/// Check if interval content is a valid POSIX interval: n, n,, n,m
/// where n and m are decimal numbers and n <= 32767.
pub(crate) fn is_valid_interval(content: &str) -> bool {
    let parts: Vec<&str> = content.splitn(2, ',').collect();
    match parts.len() {
        1 => {
            // {n} — single number
            parts[0].parse::<u32>().is_ok_and(|n| n <= 32767 && !parts[0].is_empty())
        }
        2 => {
            // {n,} or {n,m}
            let min_ok = parts[0].parse::<u32>().is_ok_and(|n| n <= 32767);
            if !min_ok || parts[0].is_empty() {
                return false;
            }
            if parts[1].is_empty() {
                return true; // {n,}
            }
            parts[1].parse::<u32>().is_ok_and(|m| m <= 32767)
        }
        _ => false,
    }
}

/// Check if an interval has numbers that are too large (> 32767).
/// Only returns true for well-formed intervals with oversized numbers.
pub(crate) fn is_interval_too_large(content: &str) -> bool {
    let parts: Vec<&str> = content.splitn(2, ',').collect();
    // Must be all digits and commas to be considered an interval attempt
    let all_valid = content.chars().all(|c| c.is_ascii_digit() || c == ',');
    if !all_valid {
        return false; // Contains non-numeric chars — treat as literal, not error
    }
    for part in &parts {
        if let Ok(n) = part.parse::<u64>() {
            if n > 32767 {
                return true;
            }
        }
    }
    false
}

/// Convert BRE (Basic Regular Expression) to ERE for the regex crate.
/// In BRE: \( \) \{ \} \| \+ \? are meta, bare versions are literal.
/// We swap them so the regex crate (which expects ERE) works correctly.
pub(crate) fn convert_bre_to_ere(bre: &str) -> String {
    let mut result = String::with_capacity(bre.len());
    let chars: Vec<char> = bre.chars().collect();
    let len = chars.len();
    let mut i = 0;
    // Track nesting depth for \( \) to determine anchor context
    let mut _depth = 0;
    // Track if we're at the "start" of a group or pattern
    let mut at_start = true;

    while i < len {
        if chars[i] == '[' {
            // Pass through bracket expressions unchanged
            result.push('[');
            i += 1;
            // Handle negation and ] as first char
            if i < len && chars[i] == '^' {
                result.push('^');
                i += 1;
            }
            if i < len && chars[i] == ']' {
                result.push(']');
                i += 1;
            }
            while i < len && chars[i] != ']' {
                if chars[i] == '[' && i + 1 < len
                    && (chars[i + 1] == ':' || chars[i + 1] == '.' || chars[i + 1] == '=')
                {
                    let delim = chars[i + 1];
                    let class_start = i;
                    i += 2;
                    let name_start = i;
                    let mut found_close = false;
                    while i + 1 < len {
                        if chars[i] == delim && chars[i + 1] == ']' {
                            found_close = true;
                            break;
                        }
                        i += 1;
                    }
                    if found_close {
                        let name: String = chars[name_start..i].iter().collect();
                        if delim == ':' {
                            validate_posix_class(&name);
                            for c in &chars[class_start..i + 2] {
                                result.push(*c);
                            }
                        } else {
                            // Collating/equivalence — use char directly in C locale
                            result.push_str(&name);
                        }
                        i += 2;
                    } else {
                        result.push_str("\\[");
                        result.push(delim);
                        i = class_start + 2;
                    }
                } else if chars[i] == '[' {
                    // Bare [ inside bracket expr — escape for regex crate
                    result.push_str("\\[");
                    i += 1;
                } else if chars[i] == '\\' {
                    // In POSIX bracket expressions, \ is literal — double-escape for regex crate
                    result.push_str("\\\\");
                    i += 1;
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            if i < len {
                result.push(']');
                i += 1;
            }
            at_start = false;
            continue;
        }

        if chars[i] == '\\' && i + 1 < len {
            match chars[i + 1] {
                '(' => {
                    result.push('(');
                    _depth += 1;
                    at_start = true;
                    i += 2;
                }
                ')' => {
                    if _depth <= 0 {
                        eprintln!("grep: Unmatched \\)");
                        process::exit(2);
                    }
                    result.push(')');
                    _depth -= 1;
                    at_start = false;
                    i += 2;
                }
                '{' => {
                    // Check if this is a valid BRE interval \{n\}, \{n,\}, \{n,m\}
                    // Find the closing \}
                    let interval_start = i + 2;
                    let mut j = interval_start;
                    let mut found_close = false;
                    while j + 1 < len {
                        if chars[j] == '\\' && j + 1 < len && chars[j + 1] == '}' {
                            found_close = true;
                            break;
                        }
                        j += 1;
                    }
                    if found_close {
                        let content: String = chars[interval_start..j].iter().collect();
                        if is_valid_interval(&content) {
                            result.push('{');
                            result.push_str(&content);
                            result.push('}');
                            i = j + 2; // skip past \}
                        } else {
                            // In BRE, \{...\} is interval syntax.
                            // Check if content looks like a malformed interval (error)
                            // vs just not a valid interval (literal)
                            let has_non_interval_chars = content
                                .chars()
                                .any(|c| !c.is_ascii_digit() && c != ',');
                            if content.is_empty()
                                || has_non_interval_chars
                                || is_interval_too_large(&content)
                            {
                                eprintln!("grep: Invalid content of \\{{\\}}");
                                process::exit(2);
                            }
                            // Content like {,2} or {,} — treat \{...\} as literal
                            // including the backslashes
                            result.push_str("\\\\\\{");
                            result.push_str(&content);
                            result.push_str("\\\\\\}");
                            i = j + 2;
                        }
                    } else {
                        // No closing \} — check if it looks like an interval attempt
                        let remaining: String = chars[interval_start..].iter().collect();
                        if remaining.chars().next().is_some_and(|c| c.is_ascii_digit() || c == ',')
                        {
                            eprintln!("grep: Unmatched \\{{");
                            process::exit(2);
                        }
                        result.push_str("\\{");
                        i += 2;
                    }
                    at_start = false;
                }
                '}' => {
                    // Stray \} — literal
                    result.push_str("\\}");
                    i += 2;
                    at_start = false;
                }
                '|' => {
                    result.push('|');
                    at_start = true;
                    i += 2;
                }
                '+' => {
                    result.push('+');
                    i += 2;
                    at_start = false;
                }
                '?' => {
                    result.push('?');
                    i += 2;
                    at_start = false;
                }
                _ => {
                    result.push('\\');
                    result.push(chars[i + 1]);
                    i += 2;
                    at_start = false;
                }
            }
        } else if chars[i] == '*' && at_start {
            // In BRE, * at start of pattern/group is literal
            result.push_str("\\*");
            i += 1;
            at_start = false;
        } else if chars[i] == '^' {
            if at_start {
                result.push('^');
            } else {
                result.push_str("\\^");
            }
            i += 1;
            // Don't change at_start — ^ at start is still "at start" for subsequent chars
        } else if chars[i] == '$' {
            // $ is anchor only at end of pattern or before \)
            let at_end =
                i + 1 == len || (i + 2 < len && chars[i + 1] == '\\' && chars[i + 2] == ')');
            if at_end {
                result.push('$');
            } else {
                result.push_str("\\$");
            }
            i += 1;
            at_start = false;
        } else if chars[i] == '(' {
            result.push_str("\\(");
            i += 1;
            at_start = false;
        } else if chars[i] == ')' {
            result.push_str("\\)");
            i += 1;
            at_start = false;
        } else if chars[i] == '{' {
            result.push_str("\\{");
            i += 1;
            at_start = false;
        } else if chars[i] == '}' {
            result.push_str("\\}");
            i += 1;
            at_start = false;
        } else if chars[i] == '|' {
            // In BRE, bare | is literal
            result.push_str("\\|");
            i += 1;
            at_start = false;
        } else {
            result.push(chars[i]);
            i += 1;
            at_start = false;
        }
    }
    result
}
