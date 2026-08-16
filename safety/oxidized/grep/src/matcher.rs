use std::process;

use fancy_regex::{Regex as FancyRegex, RegexBuilder as FancyRegexBuilder};
use regex::Regex;
use rust_pcre2::Regex as Pcre2Regex;

use crate::args::Options;
use crate::pattern::{
    convert_bre_to_ere, count_groups, escape_invalid_ere_intervals, has_backreferences,
    max_backref, warn_char_class_misuse,
};

enum MatcherInner {
    Regex(Regex),
    Fancy(FancyRegex),
    Pcre2(Pcre2Regex),
    Fixed(Vec<String>, bool), // patterns, ignore_case
}

pub(crate) struct Matcher {
    inner: MatcherInner,
    word_regexp: bool,
    line_regexp: bool,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_word_boundary(text: &str, start: usize, end: usize) -> bool {
    let before = if start == 0 {
        false
    } else {
        text[..start].chars().last().is_some_and(is_word_char)
    };
    let after = text[end..].chars().next().is_some_and(is_word_char);
    let match_start_word = if start == 0 { true } else { !before };
    let match_end_word = !after;

    // Also check that the match itself starts/ends with a word char
    // (or is empty, which matches at word boundaries on empty lines)
    let match_text = &text[start..end];
    if match_text.is_empty() {
        return !before && !after;
    }
    let first_is_word = match_text.chars().next().is_some_and(is_word_char);
    let last_is_word = match_text.chars().last().is_some_and(is_word_char);

    match_start_word && first_is_word && match_end_word && last_is_word
}

impl Matcher {
    pub(crate) fn is_match(&self, text: &str) -> bool {
        if self.line_regexp {
            // For line regexp, the entire line must match
            return self.find_line_match(text);
        }
        if self.word_regexp {
            // Empty pattern with -w: match only empty lines
            if self.has_empty_pattern() && text.is_empty() {
                return true;
            }
            // For word regexp, find matches at word boundaries
            return !self.find_matches(text).is_empty();
        }
        self.raw_is_match(text)
    }

    fn has_empty_pattern(&self) -> bool {
        match &self.inner {
            MatcherInner::Fixed(patterns, _) => patterns.iter().any(|p| p.is_empty()),
            MatcherInner::Regex(re) => re.is_match(""),
            MatcherInner::Fancy(re) => re.is_match("").unwrap_or(false),
            MatcherInner::Pcre2(re) => re.is_match(b"").unwrap_or(false),
        }
    }

    fn raw_is_match(&self, text: &str) -> bool {
        match &self.inner {
            MatcherInner::Regex(re) => re.is_match(text),
            MatcherInner::Fancy(re) => match re.is_match(text) {
                Ok(b) => b,
                Err(e) => {
                    let msg = format!("{e}");
                    if msg.contains("backtrack") {
                        eprintln!("grep: exceeded PCRE's backtracking limit");
                    } else {
                        eprintln!("grep: PCRE error: {e}");
                    }
                    process::exit(2);
                }
            },
            MatcherInner::Pcre2(re) => match re.is_match(text.as_bytes()) {
                Ok(b) => b,
                Err(e) => {
                    let msg = format!("{e}");
                    if msg.contains("match limit") || msg.contains("depth limit") {
                        eprintln!("grep: exceeded PCRE's backtracking limit");
                    } else {
                        eprintln!("grep: PCRE error: {e}");
                    }
                    process::exit(2);
                }
            },
            MatcherInner::Fixed(patterns, ic) => {
                if *ic {
                    let lower = text.to_lowercase();
                    patterns.iter().any(|p| lower.contains(&p.to_lowercase()))
                } else {
                    patterns.iter().any(|p| text.contains(p.as_str()))
                }
            }
        }
    }

    fn find_line_match(&self, text: &str) -> bool {
        match &self.inner {
            MatcherInner::Fixed(patterns, ic) => {
                if *ic {
                    let lower = text.to_lowercase();
                    patterns.iter().any(|p| lower == p.to_lowercase())
                } else {
                    patterns.iter().any(|p| text == p.as_str())
                }
            }
            _ => {
                // For regex, check if there's a match that covers the entire line
                let matches = self.raw_find_matches(text);
                matches.iter().any(|(s, e)| *s == 0 && *e == text.len())
            }
        }
    }

    pub(crate) fn find_matches(&self, text: &str) -> Vec<(usize, usize)> {
        let raw = self.raw_find_matches(text);
        if self.word_regexp {
            raw.into_iter()
                .filter(|&(s, e)| is_word_boundary(text, s, e))
                .collect()
        } else {
            raw
        }
    }

    fn raw_find_matches(&self, text: &str) -> Vec<(usize, usize)> {
        match &self.inner {
            MatcherInner::Regex(re) => re.find_iter(text).map(|m| (m.start(), m.end())).collect(),
            MatcherInner::Fancy(re) => {
                let mut matches = Vec::new();
                let mut start = 0;
                while start < text.len() {
                    match re.find_from_pos(text, start) {
                        Ok(Some(m)) => {
                            if m.start() == m.end() {
                                start = m.end() + 1;
                                continue;
                            }
                            matches.push((m.start(), m.end()));
                            start = m.end();
                        }
                        Ok(None) => break,
                        Err(e) => {
                            let msg = format!("{e}");
                            if msg.contains("backtrack") {
                                eprintln!("grep: exceeded PCRE's backtracking limit");
                            } else {
                                eprintln!("grep: PCRE error: {e}");
                            }
                            process::exit(2);
                        }
                    }
                }
                matches
            }
            MatcherInner::Pcre2(re) => {
                let mut matches = Vec::new();
                let bytes = text.as_bytes();
                let mut start = 0;
                while start < bytes.len() {
                    match re.find_at(bytes, start) {
                        Ok(Some(m)) => {
                            if m.start() == m.end() {
                                start = m.end() + 1;
                                continue;
                            }
                            matches.push((m.start(), m.end()));
                            start = m.end();
                        }
                        Ok(None) => break,
                        Err(e) => {
                            let msg = format!("{e}");
                            if msg.contains("match limit") || msg.contains("depth limit") {
                                eprintln!("grep: exceeded PCRE's backtracking limit");
                            } else {
                                eprintln!("grep: PCRE error: {e}");
                            }
                            process::exit(2);
                        }
                    }
                }
                matches
            }
            MatcherInner::Fixed(patterns, ic) => {
                let mut all_matches = Vec::new();
                for p in patterns {
                    let haystack = if *ic {
                        text.to_lowercase()
                    } else {
                        text.to_string()
                    };
                    let needle = if *ic { p.to_lowercase() } else { p.to_string() };
                    if needle.is_empty() {
                        continue;
                    }
                    let mut start = 0;
                    while let Some(pos) = haystack[start..].find(&needle) {
                        let abs_start = start + pos;
                        all_matches.push((abs_start, abs_start + needle.len()));
                        start = abs_start + 1; // advance by 1 to find overlapping matches
                    }
                }
                // Sort by position, then prefer longest match at same position
                all_matches.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
                // Deduplicate: keep longest non-overlapping matches
                let mut result = Vec::new();
                let mut last_end = 0;
                for (s, e) in all_matches {
                    if s >= last_end {
                        result.push((s, e));
                        last_end = e;
                    }
                }
                result
            }
        }
    }
}

pub(crate) fn build_matcher(opts: &Options) -> Matcher {
    if opts.fixed_strings {
        return Matcher {
            inner: MatcherInner::Fixed(opts.patterns.clone(), opts.ignore_case),
            word_regexp: opts.word_regexp,
            line_regexp: opts.line_regexp,
        };
    }

    // Warn about likely misuse of character class syntax like [:space:]
    // (should be [[:space:]])
    for p in &opts.patterns {
        warn_char_class_misuse(p);
    }

    // For BRE mode, convert each individual pattern before combining.
    // This is necessary because the combination uses ERE syntax ((?:...) and |)
    // which would be mangled by the BRE-to-ERE converter.
    let is_bre = opts.basic_regexp && !opts.extended_regexp && !opts.perl_regexp;
    let converted_patterns: Vec<String> = opts
        .patterns
        .iter()
        .map(|p| {
            let pat = if is_bre {
                convert_bre_to_ere(p)
            } else {
                p.clone()
            };
            // Escape invalid intervals so the regex crate treats them as literals
            if !opts.perl_regexp {
                escape_invalid_ere_intervals(&pat)
            } else {
                pat
            }
        })
        .collect();

    // Validate each pattern individually for unclosed bracket expressions
    for p in &converted_patterns {
        if !p.is_empty() {
            if let Err(e) = Regex::new(p) {
                let msg = format!("{e}");
                if msg.contains("unclosed") || msg.contains("character class") {
                    let clean = if msg.contains("invalid character class range") {
                        "Invalid range end".to_string()
                    } else if p.len() > 1000 {
                        "stack overflow".to_string()
                    } else {
                        msg
                    };
                    eprintln!("grep: {clean}");
                    process::exit(2);
                }
            }
        }
    }

    // Build combined pattern — sort longer patterns first so that alternation
    // prefers the longest match at each position (regex uses leftmost-first).
    let combined = if converted_patterns.len() == 1 {
        converted_patterns[0].clone()
    } else {
        let mut sorted = converted_patterns.clone();
        sorted.sort_by_key(|b| std::cmp::Reverse(b.len()));
        sorted
            .iter()
            .map(|p| format!("(?:{p})"))
            .collect::<Vec<_>>()
            .join("|")
    };

    // Wrap with word/line anchors
    let mut pattern = combined;
    let has_empty = converted_patterns.iter().any(|p| p.is_empty());
    if opts.word_regexp && !has_empty {
        if opts.perl_regexp {
            // PCRE uses negative lookahead/lookbehind for -w
            pattern = format!(r"(?<!\w)(?:{pattern})(?!\w)");
        } else {
            pattern = format!(r"\b(?:{pattern})\b");
        }
    }
    if opts.line_regexp && !has_empty {
        pattern = format!("^(?:{pattern})$");
    }

    // Track if word/line matching should be done by the Matcher
    // (needed when empty patterns are present, since \b doesn't match empty/empty)
    let matcher_word = opts.word_regexp && has_empty;
    let matcher_line = opts.line_regexp && has_empty;

    // Case insensitive prefix
    if opts.ignore_case {
        pattern = format!("(?i){pattern}");
    }

    // Detect backreferences (\1-\9) — requires fancy-regex
    let has_backrefs = has_backreferences(&pattern);

    // When multiple patterns have backrefs, validate that backrefs don't
    // cross pattern boundaries (each -e pattern's backrefs must reference
    // groups within the same pattern)
    if has_backrefs && converted_patterns.len() > 1 {
        for (idx, p) in converted_patterns.iter().enumerate() {
            let groups = count_groups(p);
            let max_ref = max_backref(p);
            if max_ref > groups {
                eprintln!(
                    "grep: Invalid back reference in pattern {} (has {} groups, references \\{})",
                    idx + 1,
                    groups,
                    max_ref
                );
                process::exit(2);
            }
        }
    }

    let inner = if opts.perl_regexp {
        // Use rust-pcre2 for -P mode (true PCRE2 semantics with backtrack limits)
        let mut compile_opts = rust_pcre2::CompileOptions::default();
        compile_opts.caseless = opts.ignore_case;
        compile_opts.multiline = false; // grep handles line-by-line
        compile_opts.dollar_endonly = opts.null_data; // -z: $ matches only at end of record
        // Remove (?i) prefix since we set it via compile options
        let pcre_pattern = if opts.ignore_case {
            pattern.strip_prefix("(?i)").unwrap_or(&pattern).to_string()
        } else {
            pattern.clone()
        };
        match Pcre2Regex::with_options(&pcre_pattern, compile_opts) {
            Ok(mut re) => {
                re.set_match_limit(10_000);
                MatcherInner::Pcre2(re)
            }
            Err(e) => {
                let msg = format!("{e}");
                let clean = if msg.contains("back reference") || msg.contains("subpattern") {
                    "reference to non-existent subpattern".to_string()
                } else {
                    msg
                };
                eprintln!("grep: {clean}");
                process::exit(2);
            }
        }
    } else if has_backrefs {
        // Use fancy-regex for BRE/ERE backreferences
        match FancyRegexBuilder::new(&pattern)
            .backtrack_limit(10_000)
            .build()
        {
            Ok(re) => MatcherInner::Fancy(re),
            Err(e) => {
                let msg = format!("{e}");
                let clean_msg = if let Some(inner) = msg.strip_prefix("Error compiling regex: ") {
                    if inner.contains("back reference") {
                        "reference to non-existent subpattern"
                    } else {
                        inner
                    }
                } else {
                    &msg
                };
                eprintln!("grep: {clean_msg}");
                process::exit(2);
            }
        }
    } else {
        // BRE conversion already applied per-pattern above, so use the
        // combined ERE pattern directly.
        match Regex::new(&pattern) {
            Ok(re) => MatcherInner::Regex(re),
            Err(e) => {
                let msg = format!("{e}");
                let clean = if msg.contains("invalid character class range") {
                    "Invalid range end".to_string()
                } else if msg.contains("nest") || pattern.len() > 1000 {
                    "stack overflow".to_string()
                } else {
                    msg
                };
                eprintln!("grep: {clean}");
                process::exit(2);
            }
        }
    };

    Matcher {
        inner,
        // For regex mode, word/line matching is usually in the pattern,
        // except when empty patterns need Matcher-level handling
        word_regexp: matcher_word,
        line_regexp: matcher_line,
    }
}
