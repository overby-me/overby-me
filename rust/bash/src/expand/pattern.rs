pub(super) enum TrimMode {
    SmallLeft,
    LargeLeft,
    SmallRight,
    LargeRight,
}

pub(super) fn trim_pattern(value: &str, pattern: &str, mode: TrimMode) -> String {
    match mode {
        TrimMode::SmallLeft => {
            for i in 0..=value.len() {
                if shell_pattern_match(&value[..i], pattern) {
                    return value[i..].to_string();
                }
            }
            value.to_string()
        }
        TrimMode::LargeLeft => {
            for i in (0..=value.len()).rev() {
                if shell_pattern_match(&value[..i], pattern) {
                    return value[i..].to_string();
                }
            }
            value.to_string()
        }
        TrimMode::SmallRight => {
            for i in (0..=value.len()).rev() {
                if shell_pattern_match(&value[i..], pattern) {
                    return value[..i].to_string();
                }
            }
            value.to_string()
        }
        TrimMode::LargeRight => {
            for i in 0..=value.len() {
                if shell_pattern_match(&value[i..], pattern) {
                    return value[..i].to_string();
                }
            }
            value.to_string()
        }
    }
}

/// Preprocess backtick command substitution content.
/// The lexer already handles \\→\, \`→`, \$→$ during backtick scanning.
/// This function only handles \<newline> → removed (line continuation).
pub(super) fn pattern_replace(value: &str, pattern: &str, replacement: &str, all: bool) -> String {
    if pattern.is_empty() {
        return value.to_string();
    }

    let mut result = String::new();
    let mut i = 0;
    let chars: Vec<char> = value.chars().collect();

    while i < chars.len() {
        let mut found = false;
        for j in (i + 1..=chars.len()).rev() {
            let substr: String = chars[i..j].iter().collect();
            if shell_pattern_match(&substr, pattern) {
                result.push_str(replacement);
                i = j;
                found = true;
                if !all {
                    let rest: String = chars[i..].iter().collect();
                    result.push_str(&rest);
                    return result;
                }
                break;
            }
        }
        if !found {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

pub(super) fn shell_pattern_match(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    pattern_match_impl(&t, 0, &p, 0)
}

fn extglob_star_match_ex(
    text: &[char],
    ti: usize,
    alts: &[Vec<char>],
    pattern: &[char],
    rest_pi: usize,
) -> bool {
    if pattern_match_impl(text, ti, pattern, rest_pi) {
        return true;
    }
    for alt in alts {
        for end in ti + 1..=text.len() {
            if pattern_match_impl(&text[ti..end], 0, alt, 0)
                && extglob_star_match_ex(text, end, alts, pattern, rest_pi)
            {
                return true;
            }
        }
    }
    false
}

fn extglob_plus_match_ex(
    text: &[char],
    ti: usize,
    alts: &[Vec<char>],
    pattern: &[char],
    rest_pi: usize,
) -> bool {
    for alt in alts {
        for end in ti + 1..=text.len() {
            if pattern_match_impl(&text[ti..end], 0, alt, 0) {
                if pattern_match_impl(text, end, pattern, rest_pi) {
                    return true;
                }
                if extglob_star_match_ex(text, end, alts, pattern, rest_pi) {
                    return true;
                }
            }
        }
    }
    false
}

fn find_extglob_close_ex(pattern: &[char], start: usize) -> Option<usize> {
    let mut depth = 1;
    let mut i = start;
    while i < pattern.len() {
        if pattern[i] == '(' && i > 0 && matches!(pattern[i - 1], '@' | '?' | '*' | '+' | '!') {
            depth += 1;
        } else if pattern[i] == ')' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn split_extglob_alts_ex(pattern: &[char]) -> Vec<Vec<char>> {
    let mut alts = Vec::new();
    let mut current = Vec::new();
    let mut depth = 0;
    for &ch in pattern {
        if ch == '(' {
            depth += 1;
            current.push(ch);
        } else if ch == ')' {
            depth -= 1;
            current.push(ch);
        } else if ch == '|' && depth == 0 {
            alts.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    alts.push(current);
    alts
}

fn pattern_match_impl(text: &[char], ti: usize, pattern: &[char], pi: usize) -> bool {
    let mut ti = ti;
    let mut pi = pi;

    while pi < pattern.len() {
        // Extglob
        if pi + 1 < pattern.len()
            && pattern[pi + 1] == '('
            && matches!(pattern[pi], '@' | '?' | '*' | '+' | '!')
        {
            let op = pattern[pi];
            if let Some(close) = find_extglob_close_ex(pattern, pi + 2) {
                let inner: Vec<char> = pattern[pi + 2..close].to_vec();
                let rest_pi = close + 1;
                let alts = split_extglob_alts_ex(&inner);
                match op {
                    '@' => {
                        for alt in &alts {
                            let mut combined = alt.clone();
                            combined.extend_from_slice(&pattern[rest_pi..]);
                            if pattern_match_impl(text, ti, &combined, 0) {
                                return true;
                            }
                        }
                        return false;
                    }
                    '?' => {
                        if pattern_match_impl(text, ti, pattern, rest_pi) {
                            return true;
                        }
                        for alt in &alts {
                            let mut combined = alt.clone();
                            combined.extend_from_slice(&pattern[rest_pi..]);
                            if pattern_match_impl(text, ti, &combined, 0) {
                                return true;
                            }
                        }
                        return false;
                    }
                    '*' => return extglob_star_match_ex(text, ti, &alts, pattern, rest_pi),
                    '+' => return extglob_plus_match_ex(text, ti, &alts, pattern, rest_pi),
                    '!' => {
                        for end in ti..=text.len() {
                            let mut any_match = false;
                            for alt in &alts {
                                if pattern_match_impl(&text[ti..end], 0, alt, 0) {
                                    any_match = true;
                                    break;
                                }
                            }
                            if !any_match && pattern_match_impl(text, end, pattern, rest_pi) {
                                return true;
                            }
                        }
                        return false;
                    }
                    _ => unreachable!(),
                }
            }
        }

        match pattern[pi] {
            // \x00 prefix means the next char is quoted (literal, not a glob char)
            '\x00' => {
                pi += 1;
                if pi >= pattern.len() {
                    return false;
                }
                if ti >= text.len() || text[ti] != pattern[pi] {
                    return false;
                }
                ti += 1;
                pi += 1;
            }
            '*' => {
                pi += 1;
                while pi < pattern.len() && pattern[pi] == '*' {
                    pi += 1;
                }
                if pi == pattern.len() {
                    return true;
                }
                for i in ti..=text.len() {
                    if pattern_match_impl(text, i, pattern, pi) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if ti >= text.len() {
                    return false;
                }
                ti += 1;
                pi += 1;
            }
            '[' => {
                if ti >= text.len() {
                    return false;
                }
                let bracket_start = pi;
                pi += 1;
                let negate = pi < pattern.len() && (pattern[pi] == '!' || pattern[pi] == '^');
                if negate {
                    pi += 1;
                }
                let mut matched = false;
                let ch = text[ti];
                // In POSIX, ] at the start of a bracket expression is a literal
                let bracket_first = pi;
                while pi < pattern.len() && (pattern[pi] != ']' || pi == bracket_first) {
                    // Handle backslash or \x00 escape inside bracket
                    if (pattern[pi] == '\\' || pattern[pi] == '\x00') && pi + 1 < pattern.len() {
                        pi += 1;
                        if pattern[pi] == ch {
                            matched = true;
                        }
                        pi += 1;
                        continue;
                    }
                    // POSIX character class: [:class:]
                    if pi + 1 < pattern.len()
                        && pattern[pi] == '['
                        && pattern[pi + 1] == ':'
                        && let Some(end) =
                            pattern[pi + 2..]
                                .iter()
                                .position(|&c| c == ':')
                                .filter(|&pos| {
                                    pi + 2 + pos + 1 < pattern.len()
                                        && pattern[pi + 2 + pos + 1] == ']'
                                })
                    {
                        let class_name: String = pattern[pi + 2..pi + 2 + end].iter().collect();
                        let in_class = match class_name.as_str() {
                            "alpha" => ch.is_alphabetic(),
                            "digit" => ch.is_ascii_digit(),
                            "alnum" => ch.is_alphanumeric(),
                            "upper" => ch.is_uppercase(),
                            "lower" => ch.is_lowercase(),
                            "space" => ch.is_whitespace(),
                            "blank" => ch == ' ' || ch == '\t',
                            "print" => !ch.is_control() || ch == ' ',
                            "graph" => !ch.is_control() && ch != ' ',
                            "cntrl" => ch.is_control(),
                            "punct" => ch.is_ascii_punctuation(),
                            "xdigit" => ch.is_ascii_hexdigit(),
                            "ascii" => ch.is_ascii(),
                            _ => false,
                        };
                        if in_class {
                            matched = true;
                        }
                        pi = pi + 2 + end + 2;
                        continue;
                    }
                    // POSIX equivalence class: [=x=]
                    if pi + 1 < pattern.len()
                        && pattern[pi] == '['
                        && pattern[pi + 1] == '='
                        && let Some(end) =
                            pattern[pi + 2..]
                                .iter()
                                .position(|&c| c == '=')
                                .filter(|&pos| {
                                    pi + 2 + pos + 1 < pattern.len()
                                        && pattern[pi + 2 + pos + 1] == ']'
                                })
                    {
                        // In C locale, equivalence class matches the character itself
                        let equiv: String = pattern[pi + 2..pi + 2 + end].iter().collect();
                        if equiv.len() == 1 && ch == equiv.chars().next().unwrap() {
                            matched = true;
                        }
                        pi = pi + 2 + end + 2;
                        continue;
                    }
                    // POSIX collating symbol: [.x.] or [.name.]
                    if pi + 1 < pattern.len()
                        && pattern[pi] == '['
                        && pattern[pi + 1] == '.'
                        && let Some(end) =
                            pattern[pi + 2..]
                                .iter()
                                .position(|&c| c == '.')
                                .filter(|&pos| {
                                    pi + 2 + pos + 1 < pattern.len()
                                        && pattern[pi + 2 + pos + 1] == ']'
                                })
                    {
                        // Extract the collating element name
                        let elem: String = pattern[pi + 2..pi + 2 + end].iter().collect();
                        // For single-char elements, match directly
                        // For multi-char or named elements, use lookup
                        let collating_char = match elem.as_str() {
                            "hyphen" | "-" => Some('-'),
                            "space" | " " => Some(' '),
                            "tab" => Some('\t'),
                            "newline" => Some('\n'),
                            "grave-accent" | "`" => Some('`'),
                            s if s.len() == 1 => s.chars().next(),
                            _ => None, // multi-char collating elements not fully supported
                        };
                        // Check if this is part of a range: [.a.]-[.z.]
                        let collating_end_pi = pi + 2 + end + 2;
                        if collating_end_pi + 1 < pattern.len()
                            && pattern[collating_end_pi] == '-'
                            && pattern[collating_end_pi + 1] != ']'
                        {
                            // Check if range end is another collating symbol or a literal
                            if collating_end_pi + 2 < pattern.len()
                                && pattern[collating_end_pi + 1] == '['
                                && pattern[collating_end_pi + 2] == '.'
                            {
                                // Range: [.x.]-[.y.]
                                if let Some(end2) = pattern[collating_end_pi + 3..]
                                    .iter()
                                    .position(|&c| c == '.')
                                    .filter(|&pos| {
                                        collating_end_pi + 3 + pos + 1 < pattern.len()
                                            && pattern[collating_end_pi + 3 + pos + 1] == ']'
                                    })
                                {
                                    let elem2: String = pattern
                                        [collating_end_pi + 3..collating_end_pi + 3 + end2]
                                        .iter()
                                        .collect();
                                    let range_start = match elem.as_str() {
                                        s if s.len() == 1 => s.chars().next(),
                                        _ => collating_char,
                                    };
                                    let range_end = match elem2.as_str() {
                                        s if s.len() == 1 => s.chars().next(),
                                        _ => None,
                                    };
                                    if let (Some(rs), Some(re)) = (range_start, range_end)
                                        && ch >= rs
                                        && ch <= re
                                    {
                                        matched = true;
                                    }
                                    pi = collating_end_pi + 3 + end2 + 2;
                                    continue;
                                }
                            } else {
                                // Range: [.x.]-y (collating start, literal end)
                                let range_end = pattern[collating_end_pi + 1];
                                if let Some(rs) = collating_char
                                    && ch >= rs
                                    && ch <= range_end
                                {
                                    matched = true;
                                }
                                pi = collating_end_pi + 2;
                                continue;
                            }
                        }
                        if let Some(cc) = collating_char
                            && ch == cc
                        {
                            matched = true;
                        }
                        pi = collating_end_pi;
                        continue;
                    }
                    if pi + 2 < pattern.len() && pattern[pi + 1] == '-' && pattern[pi + 2] != ']' {
                        if ch >= pattern[pi] && ch <= pattern[pi + 2] {
                            matched = true;
                        }
                        pi += 3;
                    } else {
                        if ch == pattern[pi] {
                            matched = true;
                        }
                        pi += 1;
                    }
                }
                if pi < pattern.len() {
                    pi += 1; // skip closing ]
                } else {
                    // Unclosed bracket — treat [ as literal, ignore any partial matches
                    if ti >= text.len() || text[ti] != '[' {
                        return false;
                    }
                    ti += 1;
                    pi = bracket_start + 1;
                    continue;
                }
                if matched == negate {
                    return false;
                }
                ti += 1;
            }
            '\\' => {
                pi += 1;
                if pi >= pattern.len() || ti >= text.len() {
                    return false;
                }
                if text[ti] != pattern[pi] {
                    return false;
                }
                ti += 1;
                pi += 1;
            }
            ch => {
                if ti >= text.len() || text[ti] != ch {
                    return false;
                }
                ti += 1;
                pi += 1;
            }
        }
    }

    ti == text.len()
}
