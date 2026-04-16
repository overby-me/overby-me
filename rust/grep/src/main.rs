use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process;

use fancy_regex::Regex as FancyRegex;
use regex::Regex;
use walkdir::WalkDir;

/// Parsed command-line options matching GNU grep flags.
#[derive(Clone)]
struct Options {
    patterns: Vec<String>,
    files: Vec<PathBuf>,
    // Matching control
    extended_regexp: bool, // -E
    fixed_strings: bool,   // -F
    basic_regexp: bool,    // -G (default)
    perl_regexp: bool,     // -P
    ignore_case: bool,     // -i
    invert_match: bool,    // -v
    word_regexp: bool,     // -w
    line_regexp: bool,     // -x
    // Output control
    count: bool,               // -c
    files_with_matches: bool,  // -l
    files_without_match: bool, // -L
    max_count: Option<usize>,  // -m
    only_matching: bool,       // -o
    quiet: bool,               // -q
    no_messages: bool,         // -s
    line_number: bool,         // -n
    with_filename: bool,       // -H
    no_filename: bool,         // -h
    byte_offset: bool,         // -b
    null_separator: bool,      // -Z
    // Context
    after_context: usize,    // -A
    before_context: usize,   // -B
    context: usize,          // -C
    context_requested: bool, // true if -A, -B, or -C was explicitly used
    // File/directory
    recursive: bool, // -r/-R
    include_glob: Vec<String>,
    exclude_glob: Vec<String>,
    exclude_dir_glob: Vec<String>,
    skip_devices: bool, // -D skip
    // Misc
    label: String, // --label
    color: ColorMode,
    null_data: bool,     // -z
    initial_tab: bool,   // -T
    text_mode: bool,     // -a (treat binary as text)
    match_color: String, // ANSI color code for matches (from GREP_COLORS/GREP_COLOR)
}

#[derive(Clone, PartialEq)]
enum ColorMode {
    Never,
    Auto,
    Always,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            patterns: Vec::new(),
            files: Vec::new(),
            extended_regexp: false,
            fixed_strings: false,
            basic_regexp: true,
            perl_regexp: false,
            ignore_case: false,
            invert_match: false,
            word_regexp: false,
            line_regexp: false,
            count: false,
            files_with_matches: false,
            files_without_match: false,
            max_count: None,
            only_matching: false,
            quiet: false,
            no_messages: false,
            line_number: false,
            with_filename: false,
            no_filename: false,
            byte_offset: false,
            null_separator: false,
            after_context: 0,
            before_context: 0,
            context: 0,
            context_requested: false,
            recursive: false,
            include_glob: Vec::new(),
            exclude_glob: Vec::new(),
            exclude_dir_glob: Vec::new(),
            skip_devices: false,
            label: "(standard input)".to_string(),
            color: ColorMode::Auto,
            null_data: false,
            initial_tab: false,
            text_mode: false,
            match_color: "01;31".to_string(),
        }
    }
}

/// Split a pattern string on newlines, as GNU grep does for -e patterns.
fn add_patterns(patterns: &mut Vec<String>, pattern: &str) {
    for p in pattern.split('\n') {
        patterns.push(p.to_string());
    }
}

fn parse_args() -> Options {
    let args: Vec<String> = env::args_os()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let mut opts = Options::default();
    let mut i = 1;
    let mut pattern_set = false;
    let mut explicit_filename = None;

    while i < args.len() {
        let arg = &args[i];

        if arg == "--" {
            i += 1;
            // Everything after -- is files
            while i < args.len() {
                opts.files.push(PathBuf::from(&args[i]));
                i += 1;
            }
            break;
        }

        if arg == "--version" || arg == "-V" {
            println!("grep (rust-grep) {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }

        if let Some(long) = arg.strip_prefix("--") {
            match long {
                "extended-regexp" => opts.extended_regexp = true,
                "fixed-strings" => opts.fixed_strings = true,
                "basic-regexp" => opts.basic_regexp = true,
                "perl-regexp" => opts.perl_regexp = true,
                "ignore-case" => opts.ignore_case = true,
                "no-ignore-case" => opts.ignore_case = false,
                "invert-match" => opts.invert_match = true,
                "word-regexp" => opts.word_regexp = true,
                "line-regexp" => opts.line_regexp = true,
                "count" => opts.count = true,
                "files-with-matches" => opts.files_with_matches = true,
                "files-without-match" => opts.files_without_match = true,
                "only-matching" => opts.only_matching = true,
                "quiet" | "silent" => opts.quiet = true,
                "no-messages" => opts.no_messages = true,
                "line-number" => opts.line_number = true,
                "with-filename" => {
                    opts.with_filename = true;
                    explicit_filename = Some(true);
                }
                "no-filename" => {
                    opts.no_filename = true;
                    explicit_filename = Some(false);
                }
                "byte-offset" => opts.byte_offset = true,
                "null" => opts.null_separator = true,
                "text" => opts.text_mode = true,
                "initial-tab" => opts.initial_tab = true,
                _ if long.starts_with("binary-files=") => {
                    let val = long.strip_prefix("binary-files=").unwrap();
                    match val {
                        "text" => opts.text_mode = true,
                        "without-match" => {} // suppress binary matches
                        _ => {}               // "binary" is the default
                    }
                }
                _ if long.starts_with("devices=") => {
                    let val = long.strip_prefix("devices=").unwrap();
                    if val == "skip" {
                        opts.skip_devices = true;
                    }
                }
                _ if long.starts_with("directories=") => {
                    let val = long.strip_prefix("directories=").unwrap();
                    if val == "recurse" {
                        opts.recursive = true;
                    }
                    // "skip" and "read" are handled implicitly
                }
                "null-data" => opts.null_data = true,
                "recursive" => opts.recursive = true,
                _ if long.starts_with("regexp=") => {
                    add_patterns(&mut opts.patterns, long.strip_prefix("regexp=").unwrap());
                    pattern_set = true;
                }
                _ if long.starts_with("max-count=") => {
                    let n = long.strip_prefix("max-count=").unwrap();
                    opts.max_count = Some(n.parse().unwrap_or(0));
                }
                _ if long.starts_with("after-context=") => {
                    let n = long.strip_prefix("after-context=").unwrap();
                    opts.after_context = n.parse().unwrap_or(0);
                    opts.context_requested = true;
                }
                _ if long.starts_with("before-context=") => {
                    let n = long.strip_prefix("before-context=").unwrap();
                    opts.before_context = n.parse().unwrap_or(0);
                    opts.context_requested = true;
                }
                _ if long.starts_with("context=") => {
                    let n = long.strip_prefix("context=").unwrap();
                    opts.context = n.parse().unwrap_or(0);
                    opts.context_requested = true;
                }
                _ if long.starts_with("label=") => {
                    opts.label = long.strip_prefix("label=").unwrap().to_string();
                }
                _ if long.starts_with("color=") || long.starts_with("colour=") => {
                    let val = long.split('=').nth(1).unwrap_or("auto");
                    opts.color = match val {
                        "always" => ColorMode::Always,
                        "never" => ColorMode::Never,
                        _ => ColorMode::Auto,
                    };
                }
                "color" | "colour" => opts.color = ColorMode::Always,
                _ if long.starts_with("include=") => {
                    opts.include_glob
                        .push(long.strip_prefix("include=").unwrap().to_string());
                }
                _ if long.starts_with("exclude=") => {
                    opts.exclude_glob
                        .push(long.strip_prefix("exclude=").unwrap().to_string());
                }
                _ if long.starts_with("exclude-dir=") => {
                    let dir = long.strip_prefix("exclude-dir=").unwrap();
                    // Strip trailing / for matching
                    opts.exclude_dir_glob
                        .push(dir.trim_end_matches('/').to_string());
                }
                "version" => {
                    println!("grep (rust-grep) 0.1.0");
                    process::exit(0);
                }
                "help" => {
                    print_usage();
                    process::exit(0);
                }
                _ => {
                    eprintln!("grep: unrecognized option '--{long}'");
                    process::exit(2);
                }
            }
            i += 1;
            continue;
        }

        // Handle -NUM shorthand for -C NUM (e.g., -3 means -C 3)
        if arg.starts_with('-') && arg.len() > 1 && arg[1..].chars().all(|c| c.is_ascii_digit()) {
            if let Ok(n) = arg[1..].parse::<usize>() {
                opts.context = n;
                opts.after_context = n;
                opts.before_context = n;
                opts.context_requested = true;
            }
            i += 1;
            continue;
        }

        if arg.starts_with('-') && arg.len() > 1 {
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;
            while j < chars.len() {
                match chars[j] {
                    'E' => opts.extended_regexp = true,
                    'F' => opts.fixed_strings = true,
                    'G' => opts.basic_regexp = true,
                    'P' => opts.perl_regexp = true,
                    'i' | 'y' => opts.ignore_case = true,
                    'v' => opts.invert_match = true,
                    'w' => opts.word_regexp = true,
                    'x' => opts.line_regexp = true,
                    'c' => opts.count = true,
                    'l' => opts.files_with_matches = true,
                    'L' => opts.files_without_match = true,
                    'o' => opts.only_matching = true,
                    'q' => opts.quiet = true,
                    's' => opts.no_messages = true,
                    'n' => opts.line_number = true,
                    'H' => {
                        opts.with_filename = true;
                        opts.no_filename = false;
                        explicit_filename = Some(true);
                    }
                    'h' => {
                        opts.no_filename = true;
                        opts.with_filename = false;
                        explicit_filename = Some(false);
                    }
                    'b' => opts.byte_offset = true,
                    'Z' => opts.null_separator = true,
                    'a' => opts.text_mode = true,
                    'D' => {
                        // -D ACTION: skip or read device files
                        let rest: String = chars[j + 1..].iter().collect();
                        let action = if rest.is_empty() {
                            i += 1;
                            if i < args.len() { args[i].as_str() } else { "read" }
                        } else {
                            &rest
                        };
                        if action == "skip" {
                            opts.skip_devices = true;
                        }
                        j = chars.len();
                        continue;
                    }
                    'T' => opts.initial_tab = true,
                    'z' => opts.null_data = true,
                    'd' => {
                        // -d ACTION: recurse, skip, read
                        let rest: String = chars[j + 1..].iter().collect();
                        let action = if rest.is_empty() {
                            i += 1;
                            if i < args.len() { args[i].as_str() } else { "read" }
                        } else {
                            &rest
                        };
                        if action == "recurse" {
                            opts.recursive = true;
                        }
                        j = chars.len();
                        continue;
                    }
                    'r' | 'R' => opts.recursive = true,
                    'e' => {
                        // -e PATTERN (rest of chars or next arg)
                        let rest: String = chars[j + 1..].iter().collect();
                        if rest.is_empty() {
                            i += 1;
                            if i < args.len() {
                                add_patterns(&mut opts.patterns, &args[i]);
                            }
                        } else {
                            add_patterns(&mut opts.patterns, &rest);
                        }
                        pattern_set = true;
                        j = chars.len(); // consumed rest
                        continue;
                    }
                    'f' => {
                        // -f FILE
                        let rest: String = chars[j + 1..].iter().collect();
                        let path = if rest.is_empty() {
                            i += 1;
                            if i < args.len() {
                                &args[i]
                            } else {
                                eprintln!("grep: option requires an argument -- 'f'");
                                process::exit(2);
                            }
                        } else {
                            &rest
                        };
                        match fs::read_to_string(path) {
                            Ok(content) => {
                                for line in content.lines() {
                                    opts.patterns.push(line.to_string());
                                }
                                pattern_set = true;
                            }
                            Err(e) => {
                                eprintln!("grep: {path}: {e}");
                                process::exit(2);
                            }
                        }
                        j = chars.len();
                        continue;
                    }
                    'm' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if rest.is_empty() {
                            i += 1;
                            if i < args.len() {
                                opts.max_count = Some(args[i].parse().unwrap_or(0));
                            }
                        } else {
                            opts.max_count = Some(rest.parse().unwrap_or(0));
                        }
                        j = chars.len();
                        continue;
                    }
                    'A' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if rest.is_empty() {
                            i += 1;
                            if i < args.len() {
                                opts.after_context = args[i].parse().unwrap_or(0);
                            }
                        } else {
                            opts.after_context = rest.parse().unwrap_or(0);
                        }
                        opts.context_requested = true;
                        j = chars.len();
                        continue;
                    }
                    'B' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if rest.is_empty() {
                            i += 1;
                            if i < args.len() {
                                opts.before_context = args[i].parse().unwrap_or(0);
                            }
                        } else {
                            opts.before_context = rest.parse().unwrap_or(0);
                        }
                        opts.context_requested = true;
                        j = chars.len();
                        continue;
                    }
                    'C' => {
                        let rest: String = chars[j + 1..].iter().collect();
                        if rest.is_empty() {
                            i += 1;
                            if i < args.len() {
                                opts.context = args[i].parse().unwrap_or(0);
                            }
                        } else {
                            opts.context = rest.parse().unwrap_or(0);
                        }
                        opts.context_requested = true;
                        j = chars.len();
                        continue;
                    }
                    _ => {
                        eprintln!("grep: invalid option -- '{}'", chars[j]);
                        process::exit(2);
                    }
                }
                j += 1;
            }
            i += 1;
            continue;
        }

        // Positional arguments
        if !pattern_set && opts.patterns.is_empty() {
            add_patterns(&mut opts.patterns, arg);
            pattern_set = true;
        } else {
            opts.files.push(PathBuf::from(arg));
        }
        i += 1;
    }

    if opts.patterns.is_empty() {
        if pattern_set {
            // -f was used but file was empty (e.g. /dev/null) — match nothing
            process::exit(1);
        }
        eprintln!("grep: no pattern specified");
        eprintln!("Try 'grep --help' for more information.");
        process::exit(2);
    }

    // Context defaults
    if opts.context > 0 {
        if opts.after_context == 0 {
            opts.after_context = opts.context;
        }
        if opts.before_context == 0 {
            opts.before_context = opts.context;
        }
    }

    // Filename display: default to showing filenames when multiple files
    if explicit_filename.is_none() {
        let multi = opts.files.len() > 1 || opts.recursive;
        opts.with_filename = multi;
        opts.no_filename = !multi;
    }

    // Handle GREP_COLORS and GREP_COLOR environment variables
    if let Ok(colors) = env::var("GREP_COLORS") {
        for part in colors.split(':') {
            if let Some(val) = part.strip_prefix("mt=") {
                opts.match_color = val.to_string();
            } else if let Some(val) = part.strip_prefix("ms=") {
                opts.match_color = val.to_string();
            }
        }
    }
    if let Ok(color) = env::var("GREP_COLOR") {
        // GREP_COLOR is deprecated — emit warning and use it if GREP_COLORS
        // doesn't set mt=
        eprintln!(
            "grep: warning: GREP_COLOR='{}' is deprecated; use GREP_COLORS='mt={}'",
            color, color
        );
        // GREP_COLOR sets mt= if not already set by GREP_COLORS
        if env::var("GREP_COLORS").is_err()
            || !env::var("GREP_COLORS")
                .unwrap_or_default()
                .contains("mt=")
        {
            opts.match_color = color;
        }
    }

    opts
}

enum MatcherInner {
    Regex(Regex),
    Fancy(FancyRegex),
    Fixed(Vec<String>, bool), // patterns, ignore_case
}

struct Matcher {
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
    fn is_match(&self, text: &str) -> bool {
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
        }
    }

    fn raw_is_match(&self, text: &str) -> bool {
        match &self.inner {
            MatcherInner::Regex(re) => re.is_match(text),
            MatcherInner::Fancy(re) => re.is_match(text).unwrap_or(false),
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

    fn find_matches(&self, text: &str) -> Vec<(usize, usize)> {
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
                        _ => break,
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

/// Check if a pattern contains backreferences (\1-\9).
fn has_backreferences(pattern: &str) -> bool {
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

/// Count the number of capturing groups in a pattern (after BRE→ERE conversion).
fn count_groups(pattern: &str) -> usize {
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
fn max_backref(pattern: &str) -> usize {
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
fn warn_char_class_misuse(pattern: &str) {
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
                    if start == 0 || chars[start - 1] != '[' {
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

fn build_matcher(opts: &Options) -> Matcher {
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
        pattern = format!(r"\b(?:{pattern})\b");
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

    let inner = if opts.perl_regexp || has_backrefs {
        match FancyRegex::new(&pattern) {
            Ok(re) => MatcherInner::Fancy(re),
            Err(e) => {
                let msg = format!("{e}");
                // Extract a GNU grep-compatible error message
                let clean_msg = if let Some(inner) = msg.strip_prefix("Error compiling regex: ") {
                    // Convert fancy-regex messages to GNU grep style
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

/// Convert BRE (Basic Regular Expression) to ERE for the regex crate.
/// In BRE: \( \) \{ \} \| \+ \? are meta, bare versions are literal.
/// We swap them so the regex crate (which expects ERE) works correctly.
/// Check if interval content is a valid POSIX interval: n, n,, n,m
/// where n and m are decimal numbers and n <= 32767.
/// Escape invalid interval expressions in ERE patterns so the regex crate
/// treats them as literals. POSIX says invalid intervals like {, {1, {,2}
/// should be treated as literal characters.
/// Validate a POSIX character class name. Exits with code 2 if invalid.
fn validate_posix_class(name: &str) {
    const VALID_CLASSES: &[&str] = &[
        "alnum", "alpha", "blank", "cntrl", "digit", "graph", "lower", "print", "punct", "space",
        "upper", "xdigit",
    ];
    if !VALID_CLASSES.contains(&name) {
        eprintln!("grep: Invalid character class name");
        process::exit(2);
    }
}

fn escape_invalid_ere_intervals(pattern: &str) -> String {
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
                if chars[i] == '[' && i + 1 < len && chars[i + 1] == ':' {
                    // POSIX character class [:name:]
                    let class_start = i;
                    i += 2; // skip [:
                    let name_start = i;
                    // Find closing :]
                    let mut found = false;
                    while i + 1 < len {
                        if chars[i] == ':' && chars[i + 1] == ']' {
                            found = true;
                            break;
                        }
                        i += 1;
                    }
                    if found {
                        let class_name: String = chars[name_start..i].iter().collect();
                        validate_posix_class(&class_name);
                        // Output the class as-is
                        for c in &chars[class_start..i + 2] {
                            result.push(*c);
                        }
                        i += 2; // skip :]
                    } else {
                        // No closing :] — escape [ as literal
                        result.push_str("\\[");
                        result.push(':');
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
fn is_valid_interval(content: &str) -> bool {
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
fn is_interval_too_large(content: &str) -> bool {
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

fn convert_bre_to_ere(bre: &str) -> String {
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
                if chars[i] == '[' && i + 1 < len && chars[i + 1] == ':' {
                    // POSIX character class [:name:]
                    let class_start = i;
                    i += 2;
                    let name_start = i;
                    let mut found_close = false;
                    while i + 1 < len {
                        if chars[i] == ':' && chars[i + 1] == ']' {
                            found_close = true;
                            break;
                        }
                        i += 1;
                    }
                    if found_close {
                        let class_name: String = chars[name_start..i].iter().collect();
                        validate_posix_class(&class_name);
                        for c in &chars[class_start..i + 2] {
                            result.push(*c);
                        }
                        i += 2;
                    } else {
                        result.push_str("\\[");
                        result.push(':');
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

/// Apply color highlighting to a line by wrapping matched portions in ANSI escape codes.
fn colorize_line(line: &str, matcher: &Matcher, color_code: &str) -> String {
    let color_start = format!("\x1b[{}m\x1b[K", color_code);
    let color_end = "\x1b[m\x1b[K";

    let matches: Vec<_> = matcher
        .find_matches(line)
        .into_iter()
        .filter(|(s, e)| s != e) // skip empty matches
        .collect();
    if matches.is_empty() {
        return line.to_string();
    }

    let mut result = String::with_capacity(line.len() + matches.len() * 20);
    let mut last_end = 0;

    for (start, end) in matches {
        if start < last_end {
            continue; // skip overlapping matches
        }
        result.push_str(&line[last_end..start]);
        result.push_str(&color_start);
        result.push_str(&line[start..end]);
        result.push_str(color_end);
        last_end = end;
    }
    result.push_str(&line[last_end..]);
    result
}

fn grep_reader<R: BufRead>(
    mut reader: R,
    matcher: &Matcher,
    opts: &Options,
    filename: &str,
) -> (usize, bool) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut match_count: usize = 0;
    let mut byte_offset: usize = 0;

    let show_filename = opts.with_filename && !opts.no_filename;
    let separator = if opts.null_separator { '\0' } else { ':' };
    let fname_sep = if opts.null_separator { '\0' } else { ':' };
    let use_color = opts.color == ColorMode::Always;

    // Binary file detection: peek at the first chunk for NUL bytes
    let mut is_binary = false;
    if !opts.null_data && !opts.text_mode {
        let buf = reader.fill_buf().unwrap_or(&[]);
        if buf.contains(&0) {
            is_binary = true;
        }
    }

    let has_context = opts.context_requested || opts.before_context > 0 || opts.after_context > 0;

    if has_context
        && !opts.count
        && !opts.files_with_matches
        && !opts.files_without_match
        && !opts.only_matching
    {
        // Context mode: collect all lines first (read raw bytes for non-UTF-8)
        let mut lines = Vec::new();
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) => break,
                Ok(_) => {
                    if buf.last() == Some(&b'\n') {
                        buf.pop();
                    }
                    lines.push(String::from_utf8_lossy(&buf).into_owned());
                }
                Err(_) => break,
            }
        }
        let mut remaining_after: usize = 0;
        let mut last_printed: Option<usize> = None;

        let max_reached = |count: usize| opts.max_count.is_some_and(|max| count >= max);

        for (line_idx, line) in lines.iter().enumerate() {
            let matches = matcher.is_match(line) != opts.invert_match;

            if matches && !max_reached(match_count) {
                match_count += 1;

                // Print before context
                let ctx_start = line_idx.saturating_sub(opts.before_context);
                #[allow(clippy::needless_range_loop)]
                for ctx_idx in ctx_start..line_idx {
                    if last_printed.is_some_and(|lp| ctx_idx <= lp) {
                        continue;
                    }
                    if last_printed.is_some_and(|lp| ctx_idx > lp + 1) {
                        let _ = writeln!(out, "--");
                    }
                    print_context_line(
                        &mut out,
                        &lines[ctx_idx],
                        ctx_idx + 1,
                        filename,
                        show_filename,
                        opts,
                    );
                    last_printed = Some(ctx_idx);
                }

                // Print separator if needed
                if last_printed.is_some_and(|lp| line_idx > lp + 1) {
                    let _ = writeln!(out, "--");
                }

                // Print matching line
                let has_prefix = show_filename || opts.line_number;
                if show_filename {
                    let _ = write!(out, "{filename}{fname_sep}");
                }
                if opts.line_number {
                    let _ = write!(out, "{}{separator}", line_idx + 1);
                }
                if opts.initial_tab && has_prefix && !line.is_empty() {
                    let _ = write!(out, "\t");
                }
                if use_color {
                    let _ = writeln!(out, "{}", colorize_line(line, matcher, &opts.match_color));
                } else {
                    let _ = writeln!(out, "{line}");
                }
                last_printed = Some(line_idx);
                remaining_after = opts.after_context;
            } else if remaining_after > 0 {
                if last_printed.is_some_and(|lp| line_idx > lp + 1) {
                    let _ = writeln!(out, "--");
                }
                print_context_line(&mut out, line, line_idx + 1, filename, show_filename, opts);
                last_printed = Some(line_idx);
                remaining_after -= 1;
            } else if max_reached(match_count) {
                break;
            }
        }

        return (match_count, match_count > 0);
    }

    // Non-context mode: stream lines (read raw bytes for non-UTF-8 support)
    let mut line_idx = 0;
    let mut line_buf = Vec::new();
    loop {
        line_buf.clear();
        let bytes_read = match reader.read_until(b'\n', &mut line_buf) {
            Ok(n) => n,
            Err(_) => break,
        };
        if bytes_read == 0 {
            break;
        }
        // Strip trailing newline
        if line_buf.last() == Some(&b'\n') {
            line_buf.pop();
        }
        let line_len = line_buf.len() + 1;
        let line = String::from_utf8_lossy(&line_buf);

        // Check for binary content in this line
        if !is_binary && !opts.null_data && !opts.text_mode && line_buf.contains(&0) {
            is_binary = true;
        }

        let matches = matcher.is_match(&line) != opts.invert_match;

        if matches {
            if let Some(max) = opts.max_count
                && match_count >= max
            {
                break;
            }
            match_count += 1;

            if opts.quiet {
                return (match_count, true);
            }

            if opts.files_with_matches {
                return (match_count, true);
            }

            if is_binary {
                eprintln!("grep: {filename}: binary file matches");
                return (match_count, true);
            }

            if !opts.count {
                if opts.only_matching && !opts.invert_match {
                    let found: Vec<_> = matcher
                        .find_matches(&line)
                        .into_iter()
                        .filter(|(s, e)| s != e) // skip empty matches
                        .collect();
                    for (start, end) in found {
                        if show_filename {
                            let _ = write!(out, "{filename}{fname_sep}");
                        }
                        if opts.line_number {
                            let _ = write!(out, "{}{separator}", line_idx + 1);
                        }
                        if opts.byte_offset {
                            let _ = write!(out, "{}{separator}", byte_offset + start);
                        }
                        if use_color {
                            let _ =
                                writeln!(out, "\x1b[{}m\x1b[K{}\x1b[m\x1b[K", opts.match_color, &line[start..end]);
                        } else {
                            let _ = writeln!(out, "{}", &line[start..end]);
                        }
                    }
                } else {
                    let has_prefix = show_filename || opts.line_number || opts.byte_offset;
                    if show_filename {
                        let _ = write!(out, "{filename}{fname_sep}");
                    }
                    if opts.line_number {
                        if opts.initial_tab && show_filename {
                            let _ = write!(out, " ");
                        }
                        let _ = write!(out, "{}{separator}", line_idx + 1);
                    }
                    if opts.byte_offset {
                        let _ = write!(out, "{byte_offset}{separator}");
                    }
                    if opts.initial_tab && has_prefix && !line.is_empty() {
                        let _ = write!(out, "\t");
                    }
                    if use_color {
                        let _ = writeln!(out, "{}", colorize_line(&line, matcher, &opts.match_color));
                    } else {
                        // Write raw bytes to preserve non-UTF-8 content
                        let _ = out.write_all(&line_buf);
                        let _ = out.write_all(b"\n");
                    }
                }
            }
        }

        byte_offset += line_len;
        line_idx += 1;
    }

    (match_count, match_count > 0)
}

fn print_context_line<W: Write>(
    out: &mut W,
    line: &str,
    line_num: usize,
    filename: &str,
    show_filename: bool,
    opts: &Options,
) {
    let has_prefix = show_filename || opts.line_number;
    if show_filename {
        let _ = write!(out, "{filename}-");
    }
    if opts.line_number {
        let _ = write!(out, "{line_num}-");
    }
    if opts.initial_tab && has_prefix && !line.is_empty() {
        let _ = write!(out, "\t");
    }
    let _ = writeln!(out, "{line}");
}

fn matches_glob(name: &str, pattern: &str) -> bool {
    let name_chars: Vec<char> = name.chars().collect();
    let pat_chars: Vec<char> = pattern.chars().collect();
    glob_match(&name_chars, &pat_chars, 0, 0)
}

fn glob_match(name: &[char], pat: &[char], mut ni: usize, mut pi: usize) -> bool {
    while pi < pat.len() {
        if pat[pi] == '*' {
            pi += 1;
            // Skip consecutive *
            while pi < pat.len() && pat[pi] == '*' {
                pi += 1;
            }
            if pi == pat.len() {
                return true; // trailing * matches everything
            }
            // Try matching rest of pattern at each position
            while ni <= name.len() {
                if glob_match(name, pat, ni, pi) {
                    return true;
                }
                ni += 1;
            }
            return false;
        } else if ni >= name.len() {
            return false;
        } else if pat[pi] == '?' {
            ni += 1;
            pi += 1;
        } else if pat[pi] == '[' {
            pi += 1;
            let negate = pi < pat.len() && (pat[pi] == '!' || pat[pi] == '^');
            if negate {
                pi += 1;
            }
            let mut matched = false;
            let mut first = true;
            while pi < pat.len() && (pat[pi] != ']' || first) {
                first = false;
                if pi + 2 < pat.len() && pat[pi + 1] == '-' {
                    if name[ni] >= pat[pi] && name[ni] <= pat[pi + 2] {
                        matched = true;
                    }
                    pi += 3;
                } else {
                    if name[ni] == pat[pi] {
                        matched = true;
                    }
                    pi += 1;
                }
            }
            if pi < pat.len() {
                pi += 1; // skip ]
            }
            if matched == negate {
                return false;
            }
            ni += 1;
        } else if pat[pi] == name[ni] {
            ni += 1;
            pi += 1;
        } else {
            return false;
        }
    }
    ni == name.len()
}

fn collect_files(opts: &Options) -> Vec<PathBuf> {
    let mut files = Vec::new();

    if opts.files.is_empty() {
        return files; // stdin mode
    }

    for path in &opts.files {
        if path.as_os_str() == "-" {
            files.push(PathBuf::from("-"));
            continue;
        }

        if opts.recursive && path.is_dir() {
            let walker = WalkDir::new(path).into_iter();
            for entry in walker.filter_entry(|e| {
                // Filter out excluded directories
                if e.file_type().is_dir() && !opts.exclude_dir_glob.is_empty() {
                    let name = e.file_name().to_string_lossy();
                    if opts.exclude_dir_glob.iter().any(|g| matches_glob(&name, g)) {
                        return false;
                    }
                }
                true
            }) {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                if entry.file_type().is_file() {
                    let name = entry.file_name().to_string_lossy();

                    // Apply include/exclude filters
                    if !opts.include_glob.is_empty()
                        && !opts.include_glob.iter().any(|g| matches_glob(&name, g))
                    {
                        continue;
                    }
                    if opts.exclude_glob.iter().any(|g| matches_glob(&name, g)) {
                        continue;
                    }

                    let entry_path = entry.into_path();
                    // Strip leading ./ for cleaner output
                    let clean_path = entry_path
                        .strip_prefix("./")
                        .unwrap_or(&entry_path)
                        .to_path_buf();
                    files.push(clean_path);
                }
            }
        } else {
            files.push(path.clone());
        }
    }

    files
}

/// Check if a file path refers to the same file as stdout (inode comparison).
#[cfg(unix)]
fn is_input_same_as_stdout(path: &Path) -> bool {
    use std::os::unix::io::AsRawFd;
    let stdout = io::stdout();
    let stdout_fd = stdout.lock().as_raw_fd();
    let stdout_meta = unsafe {
        let mut stat: libc::stat = std::mem::zeroed();
        if libc::fstat(stdout_fd, &mut stat) != 0 {
            return false;
        }
        (stat.st_dev, stat.st_ino)
    };
    if let Ok(file_meta) = fs::metadata(path) {
        return (file_meta.dev(), file_meta.ino()) == stdout_meta;
    }
    false
}

#[cfg(not(unix))]
fn is_input_same_as_stdout(_path: &Path) -> bool {
    false
}

/// Returns (match_count, matched, had_error)
fn grep_file(path: &Path, matcher: &Matcher, opts: &Options) -> (usize, bool, bool) {
    let filename = if path.as_os_str() == "-" {
        opts.label.clone()
    } else {
        path.to_string_lossy().to_string()
    };

    // Check if input file is also the output (would cause infinite loop)
    let check_same = !opts.quiet && !opts.files_with_matches && !opts.files_without_match;
    if check_same && path.as_os_str() != "-" && is_input_same_as_stdout(path) {
        eprintln!("grep: {}: input file is also the output", path.display());
        return (0, false, true);
    }
    #[cfg(unix)]
    if check_same && path.as_os_str() == "-" {
        // Check if stdin is the same as stdout
        use std::os::unix::io::AsRawFd;
        let stdin_fd = io::stdin().as_raw_fd();
        let stdout_fd = io::stdout().lock().as_raw_fd();
        unsafe {
            let mut stdin_stat: libc::stat = std::mem::zeroed();
            let mut stdout_stat: libc::stat = std::mem::zeroed();
            if libc::fstat(stdin_fd, &mut stdin_stat) == 0
                && libc::fstat(stdout_fd, &mut stdout_stat) == 0
                && stdin_stat.st_dev == stdout_stat.st_dev
                && stdin_stat.st_ino == stdout_stat.st_ino
                && stdin_stat.st_ino != 0
            {
                eprintln!("grep: (standard input): input file is also the output");
                return (0, false, true);
            }
        }
    }

    if path.as_os_str() == "-" {
        let stdin = io::stdin();
        let reader = stdin.lock();
        let (count, matched) = grep_reader(reader, matcher, opts, &filename);
        return (count, matched, false);
    }

    // Skip device files when -D skip is used
    if opts.skip_devices && path.as_os_str() != "-" {
        if let Ok(metadata) = fs::metadata(path) {
            let ft = metadata.file_type();
            if !ft.is_file() && !ft.is_dir() && !ft.is_symlink() {
                return (0, false, false);
            }
        }
    }

    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            if !opts.no_messages {
                eprintln!("grep: {}: {e}", path.display());
            }
            return (0, false, true);
        }
    };

    let reader = io::BufReader::new(file);
    let (count, matched) = grep_reader(reader, matcher, opts, &filename);
    (count, matched, false)
}

fn print_usage() {
    eprintln!("Usage: grep [OPTION]... PATTERN [FILE]...");
    eprintln!("Search for PATTERN in each FILE or standard input.");
    eprintln!();
    eprintln!("Pattern selection:");
    eprintln!("  -E, --extended-regexp     PATTERN is an extended regular expression");
    eprintln!("  -F, --fixed-strings       PATTERN is a set of newline-separated strings");
    eprintln!("  -G, --basic-regexp        PATTERN is a basic regular expression (default)");
    eprintln!("  -P, --perl-regexp         PATTERN is a Perl regular expression");
    eprintln!("  -e, --regexp=PATTERN      use PATTERN for matching");
    eprintln!("  -f, --file=FILE           obtain PATTERN from FILE");
    eprintln!("  -i, --ignore-case         ignore case distinctions");
    eprintln!("  -w, --word-regexp         match only whole words");
    eprintln!("  -x, --line-regexp         match only whole lines");
    eprintln!();
    eprintln!("Output control:");
    eprintln!("  -c, --count               print only a count of matching lines per FILE");
    eprintln!("  -l, --files-with-matches  print only names of FILEs with matches");
    eprintln!("  -L, --files-without-match print only names of FILEs without matches");
    eprintln!("  -m, --max-count=NUM       stop after NUM matches");
    eprintln!("  -n, --line-number         print line number with output lines");
    eprintln!("  -o, --only-matching       show only the part of a line matching PATTERN");
    eprintln!("  -q, --quiet, --silent     suppress all normal output");
    eprintln!("  -v, --invert-match        select non-matching lines");
    eprintln!("  -H, --with-filename       print the file name for each match");
    eprintln!("  -h, --no-filename         suppress the file name prefix");
    eprintln!("  -b, --byte-offset         print the byte offset with output lines");
    eprintln!("  -Z, --null                print 0 byte after FILE name");
    eprintln!("  -r, -R, --recursive       search directories recursively");
    eprintln!("  -A, --after-context=NUM   print NUM lines of trailing context");
    eprintln!("  -B, --before-context=NUM  print NUM lines of leading context");
    eprintln!("  -C, --context=NUM         print NUM lines of output context");
}

fn main() {
    let mut opts = parse_args();

    // With -r and no files, default to current directory
    if opts.recursive && opts.files.is_empty() {
        opts.files.push(PathBuf::from("."));
    }

    let matcher = build_matcher(&opts);

    let files = collect_files(&opts);

    let mut any_match = false;
    let mut had_error = false;

    if files.is_empty() {
        // Read from stdin
        let (count, matched, errored) = grep_file(Path::new("-"), &matcher, &opts);
        if matched {
            any_match = true;
        }
        if errored {
            had_error = true;
        }
        if opts.count {
            let stdout = io::stdout();
            let mut out = stdout.lock();
            let _ = writeln!(out, "{count}");
        }
    } else {
        for path in &files {
            let (count, matched, errored) = grep_file(path, &matcher, &opts);
            if matched {
                any_match = true;
            }
            if errored {
                had_error = true;
            }

            let filename = if path.as_os_str() == "-" {
                opts.label.clone()
            } else {
                path.to_string_lossy().to_string()
            };

            let stdout = io::stdout();
            let mut out = stdout.lock();

            if opts.count {
                if opts.with_filename && !opts.no_filename {
                    let _ = writeln!(out, "{filename}:{count}");
                } else {
                    let _ = writeln!(out, "{count}");
                }
            } else if (opts.files_with_matches && matched) || (opts.files_without_match && !matched)
            {
                if opts.null_separator {
                    let _ = write!(out, "{filename}\0");
                } else {
                    let _ = writeln!(out, "{filename}");
                }
            }
        }
    }

    if any_match {
        process::exit(0);
    } else if had_error {
        process::exit(2);
    } else {
        process::exit(1);
    }
}
