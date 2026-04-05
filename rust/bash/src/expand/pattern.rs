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
                if !value.is_char_boundary(i) {
                    continue;
                }
                if shell_pattern_match(&value[..i], pattern) {
                    return value[i..].to_string();
                }
            }
            value.to_string()
        }
        TrimMode::LargeLeft => {
            for i in (0..=value.len()).rev() {
                if !value.is_char_boundary(i) {
                    continue;
                }
                if shell_pattern_match(&value[..i], pattern) {
                    return value[i..].to_string();
                }
            }
            value.to_string()
        }
        TrimMode::SmallRight => {
            for i in (0..=value.len()).rev() {
                if !value.is_char_boundary(i) {
                    continue;
                }
                if shell_pattern_match(&value[i..], pattern) {
                    return value[..i].to_string();
                }
            }
            value.to_string()
        }
        TrimMode::LargeRight => {
            for i in 0..=value.len() {
                if !value.is_char_boundary(i) {
                    continue;
                }
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
/// Check if a pattern is a literal string (no glob metacharacters).
fn is_literal_pattern(pattern: &str) -> bool {
    !pattern.contains(['*', '?', '[', '\\', '!', '@', '+'])
}

/// Check if a pattern matches exactly one character at a time (e.g. `?`, `[abc]`, `[^;]`).
/// Returns true if the pattern can only ever match a single character.
fn is_single_char_pattern(pattern: &str) -> bool {
    if pattern == "?" {
        return true;
    }
    // \x00-quoted single character: \x00X matches exactly the literal char X
    let chars: Vec<char> = pattern.chars().collect();
    if chars.len() == 2 && chars[0] == '\x00' {
        return true;
    }
    // Bracket expression: [...]  or [^...] or [!...]
    if chars.len() >= 3 && chars[0] == '[' && chars[chars.len() - 1] == ']' {
        // Make sure there's no nested `*` or `?` or other bracket inside
        // and no extglob patterns — just a simple bracket expression
        let inner = &chars[1..chars.len() - 1];
        // Skip leading ^ or ! (negation)
        let inner = if !inner.is_empty() && (inner[0] == '^' || inner[0] == '!') {
            &inner[1..]
        } else {
            inner
        };
        // Allow `]` as first char in bracket (literal `]`)
        let start = if !inner.is_empty() && inner[0] == ']' {
            1
        } else {
            0
        };
        // Check no nested brackets or glob chars in the rest,
        // but allow POSIX character classes like [:alnum:], [:digit:], etc.
        let mut j = start;
        while j < inner.len() {
            let c = inner[j];
            if c == '[' && j + 1 < inner.len() && inner[j + 1] == ':' {
                // POSIX character class [:name:] — skip to closing `:]`
                if let Some(close) = inner[j + 2..]
                    .iter()
                    .enumerate()
                    .position(|(k, &ch)| ch == ']' && k > 0 && inner[j + 2 + k - 1] == ':')
                {
                    j = j + 2 + close + 1;
                    continue;
                }
                // Malformed — treat as non-single-char pattern
                return false;
            }
            if matches!(c, '[' | '*' | '?') {
                return false;
            }
            j += 1;
        }
        return true;
    }
    false
}

pub(super) fn pattern_replace(value: &str, pattern: &str, replacement: &str, all: bool) -> String {
    if pattern.is_empty() {
        return value.to_string();
    }

    // Fast path: literal patterns use simple string matching — O(n) instead of O(n³)
    if is_literal_pattern(pattern) {
        if all {
            return value.replace(pattern, replacement);
        } else if let Some(pos) = value.find(pattern) {
            let mut result = String::with_capacity(value.len());
            result.push_str(&value[..pos]);
            result.push_str(replacement);
            result.push_str(&value[pos + pattern.len()..]);
            return result;
        } else {
            return value.to_string();
        }
    }

    // Fast path: single-char-matching patterns (`?`, `[abc]`, `[^;]`, etc.)
    // These match exactly one character at a time, so we can do O(n) replacement.
    if is_single_char_pattern(pattern) {
        let chars: Vec<char> = value.chars().collect();
        if chars.is_empty() {
            return value.to_string();
        }
        if all {
            let mut result = String::with_capacity(value.len());
            for &c in &chars {
                let s = c.to_string();
                if shell_pattern_match(&s, pattern) {
                    result.push_str(replacement);
                } else {
                    result.push(c);
                }
            }
            return result;
        } else {
            // Replace only the first matching character
            let mut result = String::with_capacity(value.len());
            let mut replaced = false;
            for &c in &chars {
                if !replaced {
                    let s = c.to_string();
                    if shell_pattern_match(&s, pattern) {
                        result.push_str(replacement);
                        replaced = true;
                        continue;
                    }
                }
                result.push(c);
            }
            return result;
        }
    }

    // Fast path: `*` matches the whole string (longest match from any position)
    if pattern == "*" {
        return replacement.to_string();
    }

    let mut result = String::new();
    let mut i = 0;
    let chars: Vec<char> = value.chars().collect();
    let pat_chars: Vec<char> = pattern.chars().collect();

    // Compute the minimum and maximum number of characters the pattern can match.
    // Walk the pattern tokens: `*` matches 0+ chars (unbounded max), `?` matches 1,
    // `[...]` matches 1, literal char matches 1.
    // Extglob patterns: `*(...)` and `?(...)` can match 0 chars, `+(...)` matches 1+,
    // `@(...)` matches exactly 1 alternative, `!(...)` matches 1+ chars.
    // When min == max (no `*` or variable-length construct), the match length is fixed,
    // so we only need to check one substring length per position — O(n) instead of O(n²).
    let mut has_variable_length = false;
    let min_match_len: usize = {
        let mut count = 0usize;
        let mut pi = 0;
        while pi < pat_chars.len() {
            // Check for extglob prefixes: *(...), ?(...), +(...), @(...), !(...)
            if pi + 1 < pat_chars.len()
                && pat_chars[pi + 1] == '('
                && matches!(pat_chars[pi], '*' | '?' | '+' | '@' | '!')
            {
                let prefix = pat_chars[pi];
                // Skip to matching closing `)`
                pi += 2; // skip prefix and `(`
                let mut depth = 1;
                while pi < pat_chars.len() && depth > 0 {
                    if pat_chars[pi] == '('
                        && pi > 0
                        && matches!(pat_chars[pi - 1], '*' | '?' | '+' | '@' | '!')
                    {
                        depth += 1;
                    } else if pat_chars[pi] == ')' {
                        depth -= 1;
                    }
                    pi += 1;
                }
                match prefix {
                    '*' | '?' => {
                        // *(...) matches 0+ repetitions, ?(...) matches 0 or 1
                        has_variable_length = true;
                        // min contribution: 0
                    }
                    '+' => {
                        // +(...) matches 1+ repetitions — min 1 char
                        has_variable_length = true;
                        count += 1;
                    }
                    '@' => {
                        // @(...) matches exactly one alternative — min 1 char
                        // (could be 0 if an alternative is empty, but conservatively 0)
                        has_variable_length = true;
                    }
                    '!' => {
                        // !(...) matches anything NOT matching — variable length
                        has_variable_length = true;
                    }
                    _ => unreachable!(),
                }
                continue;
            }
            match pat_chars[pi] {
                '*' => {
                    // `*` can match zero characters — contributes 0
                    has_variable_length = true;
                    pi += 1;
                }
                '?' => {
                    count += 1;
                    pi += 1;
                }
                '[' => {
                    // Bracket expression `[...]` matches exactly 1 character.
                    // Skip to the closing `]`.
                    pi += 1;
                    // `]` as first char (or after `!`/`^`) is literal
                    if pi < pat_chars.len() && (pat_chars[pi] == '!' || pat_chars[pi] == '^') {
                        pi += 1;
                    }
                    if pi < pat_chars.len() && pat_chars[pi] == ']' {
                        pi += 1;
                    }
                    while pi < pat_chars.len() && pat_chars[pi] != ']' {
                        pi += 1;
                    }
                    if pi < pat_chars.len() {
                        pi += 1; // skip closing `]`
                    }
                    count += 1;
                }
                '\x00' | '\\' => {
                    // \x00 or \ prefix: quoted literal — matches 1 character
                    count += 1;
                    pi += 2;
                }
                _ => {
                    count += 1;
                    pi += 1;
                }
            }
        }
        count
    };
    // When there are no variable-length constructs (`*`, extglob), min == max:
    // the pattern matches a fixed number of chars.
    let fixed_len = if !has_variable_length {
        Some(min_match_len)
    } else {
        None
    };

    while i < chars.len() {
        let mut found = false;
        let lo = (i + min_match_len.max(1)).min(chars.len() + 1);
        // When the match length is fixed, only try the one possible length.
        let hi = if let Some(fl) = fixed_len {
            (i + fl).min(chars.len())
        } else {
            chars.len()
        };
        for j in (lo..=hi).rev() {
            if pattern_match_impl(&chars[i..j], 0, &pat_chars, 0) {
                result.push_str(replacement);
                i = j;
                found = true;
                if !all {
                    for &c in &chars[i..] {
                        result.push(c);
                    }
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
    // Handle empty value: if pattern matches empty string, replace
    if chars.is_empty() && pattern_match_impl(&[], 0, &pat_chars, 0) {
        result.push_str(replacement);
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
