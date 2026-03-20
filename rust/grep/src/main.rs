use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
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
    line_number: bool,         // -n
    with_filename: bool,       // -H
    no_filename: bool,         // -h
    byte_offset: bool,         // -b
    null_separator: bool,      // -Z
    // Context
    after_context: usize,  // -A
    before_context: usize, // -B
    context: usize,        // -C
    // File/directory
    recursive: bool, // -r/-R
    include_glob: Vec<String>,
    exclude_glob: Vec<String>,
    // Misc
    label: String, // --label
    color: ColorMode,
    null_data: bool, // -z
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
            line_number: false,
            with_filename: false,
            no_filename: false,
            byte_offset: false,
            null_separator: false,
            after_context: 0,
            before_context: 0,
            context: 0,
            recursive: false,
            include_glob: Vec::new(),
            exclude_glob: Vec::new(),
            label: "(standard input)".to_string(),
            color: ColorMode::Auto,
            null_data: false,
        }
    }
}

fn parse_args() -> Options {
    let args: Vec<String> = env::args().collect();
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
                "null-data" => opts.null_data = true,
                "recursive" => opts.recursive = true,
                _ if long.starts_with("regexp=") => {
                    opts.patterns
                        .push(long.strip_prefix("regexp=").unwrap().to_string());
                    pattern_set = true;
                }
                _ if long.starts_with("max-count=") => {
                    let n = long.strip_prefix("max-count=").unwrap();
                    opts.max_count = Some(n.parse().unwrap_or(0));
                }
                _ if long.starts_with("after-context=") => {
                    let n = long.strip_prefix("after-context=").unwrap();
                    opts.after_context = n.parse().unwrap_or(0);
                }
                _ if long.starts_with("before-context=") => {
                    let n = long.strip_prefix("before-context=").unwrap();
                    opts.before_context = n.parse().unwrap_or(0);
                }
                _ if long.starts_with("context=") => {
                    let n = long.strip_prefix("context=").unwrap();
                    opts.context = n.parse().unwrap_or(0);
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
                    'q' | 's' => opts.quiet = true,
                    'n' => opts.line_number = true,
                    'H' => {
                        opts.with_filename = true;
                        explicit_filename = Some(true);
                    }
                    'h' => {
                        opts.no_filename = true;
                        explicit_filename = Some(false);
                    }
                    'b' => opts.byte_offset = true,
                    'Z' => opts.null_separator = true,
                    'z' => opts.null_data = true,
                    'r' | 'R' => opts.recursive = true,
                    'e' => {
                        // -e PATTERN (rest of chars or next arg)
                        let rest: String = chars[j + 1..].iter().collect();
                        if rest.is_empty() {
                            i += 1;
                            if i < args.len() {
                                opts.patterns.push(args[i].clone());
                            }
                        } else {
                            opts.patterns.push(rest);
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
                            j = chars.len();
                            &rest
                        };
                        match fs::read_to_string(path) {
                            Ok(content) => {
                                for line in content.lines() {
                                    if !line.is_empty() {
                                        opts.patterns.push(line.to_string());
                                    }
                                }
                                pattern_set = true;
                            }
                            Err(e) => {
                                eprintln!("grep: {path}: {e}");
                                process::exit(2);
                            }
                        }
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
            opts.patterns.push(arg.clone());
            pattern_set = true;
        } else {
            opts.files.push(PathBuf::from(arg));
        }
        i += 1;
    }

    if opts.patterns.is_empty() {
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

    opts
}

enum Matcher {
    Regex(Regex),
    Fancy(FancyRegex),
    Fixed(Vec<String>, bool), // patterns, ignore_case
}

impl Matcher {
    fn is_match(&self, text: &str) -> bool {
        match self {
            Matcher::Regex(re) => re.is_match(text),
            Matcher::Fancy(re) => re.is_match(text).unwrap_or(false),
            Matcher::Fixed(patterns, ic) => {
                if *ic {
                    let lower = text.to_lowercase();
                    patterns.iter().any(|p| lower.contains(&p.to_lowercase()))
                } else {
                    patterns.iter().any(|p| text.contains(p.as_str()))
                }
            }
        }
    }

    fn find_matches(&self, text: &str) -> Vec<(usize, usize)> {
        match self {
            Matcher::Regex(re) => re.find_iter(text).map(|m| (m.start(), m.end())).collect(),
            Matcher::Fancy(re) => {
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
            Matcher::Fixed(patterns, ic) => {
                let mut matches = Vec::new();
                for p in patterns {
                    let haystack = if *ic {
                        text.to_lowercase()
                    } else {
                        text.to_string()
                    };
                    let needle = if *ic { p.to_lowercase() } else { p.to_string() };
                    let mut start = 0;
                    while let Some(pos) = haystack[start..].find(&needle) {
                        let abs_start = start + pos;
                        matches.push((abs_start, abs_start + needle.len()));
                        start = abs_start + needle.len();
                    }
                }
                matches.sort_by_key(|m| m.0);
                matches
            }
        }
    }
}

fn build_matcher(opts: &Options) -> Matcher {
    if opts.fixed_strings {
        return Matcher::Fixed(opts.patterns.clone(), opts.ignore_case);
    }

    // For BRE mode, convert each individual pattern before combining.
    // This is necessary because the combination uses ERE syntax ((?:...) and |)
    // which would be mangled by the BRE-to-ERE converter.
    let is_bre = opts.basic_regexp && !opts.extended_regexp && !opts.perl_regexp;
    let converted_patterns: Vec<String> = opts
        .patterns
        .iter()
        .map(|p| {
            if is_bre {
                convert_bre_to_ere(p)
            } else {
                p.clone()
            }
        })
        .collect();

    // Build combined pattern
    let combined = if converted_patterns.len() == 1 {
        converted_patterns[0].clone()
    } else {
        converted_patterns
            .iter()
            .map(|p| format!("(?:{p})"))
            .collect::<Vec<_>>()
            .join("|")
    };

    // Wrap with word/line anchors
    let mut pattern = combined;
    if opts.word_regexp {
        pattern = format!(r"\b(?:{pattern})\b");
    }
    if opts.line_regexp {
        pattern = format!("^(?:{pattern})$");
    }

    // Case insensitive prefix
    if opts.ignore_case {
        pattern = format!("(?i){pattern}");
    }

    if opts.perl_regexp {
        match FancyRegex::new(&pattern) {
            Ok(re) => Matcher::Fancy(re),
            Err(e) => {
                eprintln!("grep: invalid Perl regex: {e}");
                process::exit(2);
            }
        }
    } else {
        // BRE conversion already applied per-pattern above, so use the
        // combined ERE pattern directly.
        match Regex::new(&pattern) {
            Ok(re) => Matcher::Regex(re),
            Err(e) => {
                if is_bre {
                    eprintln!("grep: invalid BRE pattern: {e}");
                } else {
                    eprintln!("grep: invalid regex: {e}");
                }
                process::exit(2);
            }
        }
    }
}

/// Convert BRE (Basic Regular Expression) to ERE for the regex crate.
/// In BRE: \( \) \{ \} \| \+ \? are meta, bare versions are literal.
/// We swap them so the regex crate (which expects ERE) works correctly.
fn convert_bre_to_ere(bre: &str) -> String {
    let mut result = String::with_capacity(bre.len());
    let chars: Vec<char> = bre.chars().collect();
    let mut i = 0;

    while i < chars.len() {
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
            // In BRE, bare | is literal — but we already handle \| → |
            // However, our combined pattern uses | for alternation.
            // Skip escaping | if it comes from pattern combination.
            result.push('|');
            i += 1;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn grep_reader<R: BufRead>(
    reader: R,
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

    let has_context = opts.before_context > 0 || opts.after_context > 0;

    if has_context && !opts.count && !opts.files_with_matches && !opts.files_without_match {
        // Context mode: collect all lines first
        let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();
        let mut remaining_after: usize = 0;
        let mut last_printed: Option<usize> = None;

        for (line_idx, line) in lines.iter().enumerate() {
            let matches = matcher.is_match(line) != opts.invert_match;

            if matches {
                if let Some(max) = opts.max_count
                    && match_count >= max
                {
                    break;
                }
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
                if show_filename {
                    let _ = write!(out, "{filename}{fname_sep}");
                }
                if opts.line_number {
                    let _ = write!(out, "{}{separator}", line_idx + 1);
                }
                let _ = writeln!(out, "{line}");
                last_printed = Some(line_idx);
                remaining_after = opts.after_context;
            } else if remaining_after > 0 {
                if last_printed.is_some_and(|lp| line_idx > lp + 1) {
                    let _ = writeln!(out, "--");
                }
                print_context_line(&mut out, line, line_idx + 1, filename, show_filename, opts);
                last_printed = Some(line_idx);
                remaining_after -= 1;
            }
        }

        return (match_count, match_count > 0);
    }

    // Non-context mode: stream lines
    for (line_idx, line_result) in reader.lines().enumerate() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line_len = line.len() + 1; // +1 for newline

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

            if !opts.count {
                if opts.only_matching && !opts.invert_match {
                    let found = matcher.find_matches(&line);
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
                        let _ = writeln!(out, "{}", &line[start..end]);
                    }
                } else {
                    if show_filename {
                        let _ = write!(out, "{filename}{fname_sep}");
                    }
                    if opts.line_number {
                        let _ = write!(out, "{}{separator}", line_idx + 1);
                    }
                    if opts.byte_offset {
                        let _ = write!(out, "{byte_offset}{separator}");
                    }
                    let _ = writeln!(out, "{line}");
                }
            }
        }

        byte_offset += line_len;
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
    if show_filename {
        let _ = write!(out, "{filename}-");
    }
    if opts.line_number {
        let _ = write!(out, "{line_num}-");
    }
    let _ = writeln!(out, "{line}");
}

fn matches_glob(name: &str, pattern: &str) -> bool {
    // Simple glob matching for --include/--exclude
    if let Some(suffix) = pattern.strip_prefix('*') {
        name.ends_with(suffix)
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else {
        name == pattern
    }
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
            for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
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

                    files.push(entry.into_path());
                }
            }
        } else {
            files.push(path.clone());
        }
    }

    files
}

fn grep_file(path: &Path, matcher: &Matcher, opts: &Options) -> (usize, bool) {
    let filename = if path.as_os_str() == "-" {
        opts.label.clone()
    } else {
        path.to_string_lossy().to_string()
    };

    if path.as_os_str() == "-" {
        let stdin = io::stdin();
        let reader = stdin.lock();
        return grep_reader(reader, matcher, opts, &filename);
    }

    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            if !opts.quiet {
                eprintln!("grep: {}: {e}", path.display());
            }
            return (0, false);
        }
    };

    let reader = io::BufReader::new(file);
    grep_reader(reader, matcher, opts, &filename)
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
    let opts = parse_args();
    let matcher = build_matcher(&opts);

    let files = collect_files(&opts);

    let mut any_match = false;

    if files.is_empty() {
        // Read from stdin
        let (count, matched) = grep_file(Path::new("-"), &matcher, &opts);
        if matched {
            any_match = true;
        }
        if opts.count {
            let stdout = io::stdout();
            let mut out = stdout.lock();
            let _ = writeln!(out, "{count}");
        }
    } else {
        for path in &files {
            let (count, matched) = grep_file(path, &matcher, &opts);
            if matched {
                any_match = true;
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

    if opts.quiet {
        process::exit(if any_match { 0 } else { 1 });
    }

    process::exit(if any_match { 0 } else { 1 });
}
