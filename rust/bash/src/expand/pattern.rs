use super::{get_nocasematch, get_patsub_replacement};

/// Case-insensitive character comparison for nocasematch.
/// Returns true if the two characters are equal, or if nocasematch is enabled
/// and they are equal ignoring case.
#[inline]
fn chars_eq(a: char, b: char, nocase: bool) -> bool {
    if a == b {
        return true;
    }
    if nocase {
        // Compare by lowercasing both sides (handles ASCII and Unicode)
        let mut la = a.to_lowercase();
        let mut lb = b.to_lowercase();
        loop {
            match (la.next(), lb.next()) {
                (Some(x), Some(y)) if x == y => continue,
                (None, None) => return true,
                _ => return false,
            }
        }
    }
    false
}

/// Case-insensitive range check for nocasematch.
#[inline]
fn char_in_range(ch: char, lo: char, hi: char, nocase: bool) -> bool {
    if ch >= lo && ch <= hi {
        return true;
    }
    if nocase {
        // Check if any case variant of ch falls in the range of case variants
        for lc in ch.to_lowercase() {
            if lc >= lo.to_lowercase().next().unwrap_or(lo)
                && lc <= hi.to_lowercase().next().unwrap_or(hi)
            {
                return true;
            }
        }
        for uc in ch.to_uppercase() {
            if uc >= lo.to_uppercase().next().unwrap_or(lo)
                && uc <= hi.to_uppercase().next().unwrap_or(hi)
            {
                return true;
            }
        }
    }
    false
}

use crate::builtins::{RAW_BYTE_BASE, is_pua_raw_byte};

/// If `ch` is a PUA-encoded raw byte, return the original byte value.
/// Otherwise return `None`.
pub(crate) fn pua_byte(ch: char) -> Option<u8> {
    let cp = ch as u32;
    if is_pua_raw_byte(cp) {
        Some((cp - RAW_BYTE_BASE) as u8)
    } else {
        None
    }
}

/// Check if a character (possibly PUA-encoded) belongs to a POSIX character class.
/// PUA-encoded raw bytes are decoded to their original byte value before checking.
pub(crate) fn char_in_class(ch: char, class_name: &str) -> bool {
    if let Some(b) = pua_byte(ch) {
        // Decode PUA to original byte and check against ASCII-based classes
        match class_name {
            "alpha" => b.is_ascii_alphabetic(),
            "digit" => b.is_ascii_digit(),
            "alnum" => b.is_ascii_alphanumeric(),
            "upper" => b.is_ascii_uppercase(),
            "lower" => b.is_ascii_lowercase(),
            "space" => b.is_ascii_whitespace(),
            "blank" => b == b' ' || b == b'\t',
            "print" => (0x20..0x7f).contains(&b),
            "graph" => b > 0x20 && b < 0x7f,
            "cntrl" => b < 0x20 || b == 0x7f,
            "punct" => b.is_ascii_punctuation(),
            "xdigit" => b.is_ascii_hexdigit(),
            "ascii" => b <= 0x7f,
            _ => false,
        }
    } else {
        match class_name {
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
        }
    }
}

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
    // When nocasematch is on, we cannot use the literal fast path because
    // string operations (find/replace) are case-sensitive.
    if get_nocasematch() {
        return false;
    }
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

/// Process `&` in a replacement string: unescaped `&` is replaced by
/// the matched text, `\&` becomes a literal `&`.  Only active when the
/// `patsub_replacement` shopt option is enabled.
fn apply_replacement_amp(replacement: &str, matched: &str) -> String {
    let mut result = String::with_capacity(replacement.len() + matched.len());
    let chars: Vec<char> = replacement.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\x00' && i + 1 < chars.len() {
            // \x00X → literal X (was quoted in the original word).
            // This covers \x00& (quoted &) and \x00\ (quoted \) so that
            // a quoted backslash doesn't accidentally escape a following &.
            result.push(chars[i + 1]);
            i += 2;
        } else if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '\\' {
            // \\ → literal \ (escaped backslash)
            result.push('\\');
            i += 2;
        } else if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '&' {
            // \& → literal &
            result.push('&');
            i += 2;
        } else if chars[i] == '&' {
            // & → matched text
            result.push_str(matched);
            i += 1;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// Check whether a replacement string contains an unescaped `&`.
/// `\x00&` (quoted marker) and `\&` (backslash-escaped) are NOT unescaped.
fn replacement_has_amp(replacement: &str) -> bool {
    let chars: Vec<char> = replacement.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\x00' && i + 1 < chars.len() {
            // Quoted char (& or \) — skip both
            i += 2;
        } else if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '\\' {
            // Escaped backslash — skip
            i += 2;
        } else if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '&' {
            // Escaped & — skip
            i += 2;
        } else if chars[i] == '&' {
            return true;
        } else {
            i += 1;
        }
    }
    false
}

/// If patsub_replacement is active and the replacement contains `\&` or
/// `\x00&` but no unescaped `&`, we still need to unescape them → `&`.
fn unescape_replacement_amp(replacement: &str) -> String {
    let mut result = String::with_capacity(replacement.len());
    let chars: Vec<char> = replacement.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\x00' && i + 1 < chars.len() {
            // Quoted char marker → literal char (covers \x00& and \x00\)
            result.push(chars[i + 1]);
            i += 2;
        } else if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '\\' {
            // Escaped backslash → literal \
            result.push('\\');
            i += 2;
        } else if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '&' {
            // Escaped & → literal &
            result.push('&');
            i += 2;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// Produce the replacement string for a given match.  When
/// `patsub_replacement` is enabled and the replacement contains `&`,
/// substitute `&` with the matched text.
fn make_replacement(replacement: &str, matched: &str, use_amp: bool) -> String {
    if use_amp {
        apply_replacement_amp(replacement, matched)
    } else {
        replacement.to_string()
    }
}

pub(super) fn pattern_replace(value: &str, pattern: &str, replacement: &str, all: bool) -> String {
    if pattern.is_empty() {
        return value.to_string();
    }

    // Determine whether `&` replacement is active for this call.
    let patsub = get_patsub_replacement();
    // We need `&` processing when patsub_replacement is on AND the
    // replacement contains `&` or `\&` or `\x00&` (quoted marker).
    let has_amp = patsub && (replacement.contains('&'));
    // Even if there are no unescaped `&`, we must unescape `\&` → `&`
    // and `\x00&` → `&` when the option is active.
    let needs_amp_unescape =
        patsub && (replacement.contains("\\&") || replacement.contains('\x00'));
    let use_amp = has_amp && replacement_has_amp(replacement);
    // Precompute a "plain" replacement for fast paths when `&` processing
    // isn't needed but `\&` unescaping is.
    let plain_rep;
    let rep = if use_amp {
        // Will be handled per-match via make_replacement()
        replacement
    } else if needs_amp_unescape {
        plain_rep = unescape_replacement_amp(replacement);
        &plain_rep
    } else {
        replacement
    };

    // Fast path: literal patterns use simple string matching — O(n) instead of O(n³)
    if is_literal_pattern(pattern) {
        if use_amp {
            // Need per-match replacement (matched text = pattern literal)
            if all {
                let mut result = String::new();
                let mut start = 0;
                while let Some(pos) = value[start..].find(pattern) {
                    let abs = start + pos;
                    result.push_str(&value[start..abs]);
                    result.push_str(&make_replacement(
                        replacement,
                        &value[abs..abs + pattern.len()],
                        true,
                    ));
                    start = abs + pattern.len();
                }
                result.push_str(&value[start..]);
                return result;
            } else if let Some(pos) = value.find(pattern) {
                let mut result = String::with_capacity(value.len());
                result.push_str(&value[..pos]);
                result.push_str(&make_replacement(
                    replacement,
                    &value[pos..pos + pattern.len()],
                    true,
                ));
                result.push_str(&value[pos + pattern.len()..]);
                return result;
            } else {
                return value.to_string();
            }
        }
        if all {
            return value.replace(pattern, rep);
        } else if let Some(pos) = value.find(pattern) {
            let mut result = String::with_capacity(value.len());
            result.push_str(&value[..pos]);
            result.push_str(rep);
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
                    if use_amp {
                        result.push_str(&make_replacement(replacement, &s, true));
                    } else {
                        result.push_str(rep);
                    }
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
                        if use_amp {
                            result.push_str(&make_replacement(replacement, &s, true));
                        } else {
                            result.push_str(rep);
                        }
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
        if use_amp {
            return make_replacement(replacement, value, true);
        }
        return rep.to_string();
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

    // Whether the pattern can match an empty string (e.g. ?(b), *(b), *)
    let can_match_empty = min_match_len == 0 && pattern_match_impl(&[], 0, &pat_chars, 0);

    while i < chars.len() {
        let mut found = false;
        // When the pattern can match empty, start from i (try empty match at
        // this position).  Otherwise start from i + min_match_len (at least 1).
        let lo = if can_match_empty {
            i
        } else {
            (i + min_match_len.max(1)).min(chars.len() + 1)
        };
        // When the match length is fixed, only try the one possible length.
        let hi = if let Some(fl) = fixed_len {
            (i + fl).min(chars.len())
        } else {
            chars.len()
        };
        // Try longest match first (greedy).
        for j in (lo..=hi).rev() {
            if pattern_match_impl(&chars[i..j], 0, &pat_chars, 0) {
                if use_amp {
                    let matched: String = chars[i..j].iter().collect();
                    result.push_str(&make_replacement(replacement, &matched, true));
                } else {
                    result.push_str(rep);
                }
                if j == i {
                    // Empty match: output the replacement, then consume the
                    // current character to avoid an infinite loop.  This
                    // matches bash behaviour for replace-all with patterns
                    // like ?(b) and *(b).
                    result.push(chars[i]);
                    i += 1;
                } else {
                    i = j;
                }
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
    // Handle empty value: if the value is empty and the pattern matches the
    // empty string, produce one replacement (e.g. ${x/*/z} where x="").
    // Do NOT add a trailing empty-match replacement after processing a
    // non-empty value — bash doesn't do this.
    if can_match_empty && chars.is_empty() {
        if use_amp {
            result.push_str(&make_replacement(replacement, "", true));
        } else {
            result.push_str(rep);
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
    let nocase = get_nocasematch();
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
                if ti >= text.len() || !chars_eq(text[ti], pattern[pi], nocase) {
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
                        if chars_eq(ch, pattern[pi], nocase) {
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
                        let in_class = char_in_class(ch, &class_name);
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
                        if equiv.len() == 1 && chars_eq(ch, equiv.chars().next().unwrap(), nocase) {
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
                                        && char_in_range(ch, rs, re, nocase)
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
                                    && char_in_range(ch, rs, range_end, nocase)
                                {
                                    matched = true;
                                }
                                pi = collating_end_pi + 2;
                                continue;
                            }
                        }
                        if let Some(cc) = collating_char
                            && chars_eq(ch, cc, nocase)
                        {
                            matched = true;
                        }
                        pi = collating_end_pi;
                        continue;
                    }
                    if pi + 2 < pattern.len() && pattern[pi + 1] == '-' && pattern[pi + 2] != ']' {
                        if char_in_range(ch, pattern[pi], pattern[pi + 2], nocase) {
                            matched = true;
                        }
                        pi += 3;
                    } else {
                        if chars_eq(ch, pattern[pi], nocase) {
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
                if !chars_eq(text[ti], pattern[pi], nocase) {
                    return false;
                }
                ti += 1;
                pi += 1;
            }
            ch => {
                if ti >= text.len() || !chars_eq(text[ti], ch, nocase) {
                    return false;
                }
                ti += 1;
                pi += 1;
            }
        }
    }

    ti == text.len()
}
