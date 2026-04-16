use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;

/// Parsed command-line options matching GNU grep flags.
#[derive(Clone)]
pub(crate) struct Options {
    pub(crate) patterns: Vec<String>,
    pub(crate) files: Vec<PathBuf>,
    // Matching control
    pub(crate) extended_regexp: bool, // -E
    pub(crate) fixed_strings: bool,   // -F
    pub(crate) basic_regexp: bool,    // -G (default)
    pub(crate) perl_regexp: bool,     // -P
    pub(crate) ignore_case: bool,     // -i
    pub(crate) invert_match: bool,    // -v
    pub(crate) word_regexp: bool,     // -w
    pub(crate) line_regexp: bool,     // -x
    // Output control
    pub(crate) count: bool,               // -c
    pub(crate) files_with_matches: bool,  // -l
    pub(crate) files_without_match: bool, // -L
    pub(crate) max_count: Option<usize>,  // -m
    pub(crate) only_matching: bool,       // -o
    pub(crate) quiet: bool,               // -q
    pub(crate) no_messages: bool,         // -s
    pub(crate) line_number: bool,         // -n
    pub(crate) with_filename: bool,       // -H
    pub(crate) no_filename: bool,         // -h
    pub(crate) byte_offset: bool,         // -b
    pub(crate) null_separator: bool,      // -Z
    // Context
    pub(crate) after_context: usize,    // -A
    pub(crate) before_context: usize,   // -B
    pub(crate) context: usize,          // -C
    pub(crate) context_requested: bool, // true if -A, -B, or -C was explicitly used
    // File/directory
    pub(crate) recursive: bool, // -r/-R
    pub(crate) include_glob: Vec<String>,
    pub(crate) exclude_glob: Vec<String>,
    pub(crate) include_is_strict: bool, // true if --include should be a strict whitelist
    pub(crate) exclude_dir_glob: Vec<String>,
    pub(crate) skip_devices: bool,      // -D skip
    pub(crate) skip_directories: bool,  // -d skip / --directories=skip
    // Misc
    pub(crate) label: String, // --label
    pub(crate) color: ColorMode,
    pub(crate) null_data: bool,     // -z
    pub(crate) initial_tab: bool,   // -T
    pub(crate) text_mode: bool,     // -a (treat binary as text)
    pub(crate) match_color: String, // ANSI color code for matches (from GREP_COLORS/GREP_COLOR)
}

#[derive(Clone, PartialEq)]
pub(crate) enum ColorMode {
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
            include_is_strict: true,
            exclude_dir_glob: Vec::new(),
            skip_devices: false,
            skip_directories: false,
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
pub(crate) fn add_patterns(patterns: &mut Vec<String>, pattern: &str) {
    for p in pattern.split('\n') {
        patterns.push(p.to_string());
    }
}

pub(crate) fn parse_args() -> Options {
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
                "line-buffered" => {} // accepted but no-op (stdout is already line-buffered)
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
                    match val {
                        "recurse" => opts.recursive = true,
                        "skip" => opts.skip_directories = true,
                        _ => {} // "read" is default
                    }
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
                    // If any --exclude appeared before this --include,
                    // include is non-strict (files not matching include are still considered)
                    if !opts.exclude_glob.is_empty() && opts.include_glob.is_empty() {
                        opts.include_is_strict = false;
                    }
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
                        match action {
                            "recurse" => opts.recursive = true,
                            "skip" => opts.skip_directories = true,
                            _ => {}
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

pub(crate) fn print_usage() {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut err = false;
    macro_rules! outln {
        () => { if !err && writeln!(out).is_err() { err = true; } };
        ($($arg:tt)*) => { if !err && writeln!(out, $($arg)*).is_err() { err = true; } }
    }
    outln!("Usage: grep [OPTION]... PATTERN [FILE]...");
    outln!("Search for PATTERN in each FILE or standard input.");
    outln!();
    outln!("Pattern selection:");
    outln!("  -E, --extended-regexp     PATTERN is an extended regular expression");
    outln!("  -F, --fixed-strings       PATTERN is a set of newline-separated strings");
    outln!("  -G, --basic-regexp        PATTERN is a basic regular expression (default)");
    outln!("  -P, --perl-regexp         PATTERN is a Perl regular expression");
    outln!("  -e, --regexp=PATTERN      use PATTERN for matching");
    outln!("  -f, --file=FILE           obtain PATTERN from FILE");
    outln!("  -i, --ignore-case         ignore case distinctions");
    outln!("  -w, --word-regexp         match only whole words");
    outln!("  -x, --line-regexp         match only whole lines");
    outln!();
    outln!("Output control:");
    outln!("  -c, --count               print only a count of matching lines per FILE");
    outln!("  -l, --files-with-matches  print only names of FILEs with matches");
    outln!("  -L, --files-without-match print only names of FILEs without matches");
    outln!("  -m, --max-count=NUM       stop after NUM matches");
    outln!("  -n, --line-number         print line number with output lines");
    outln!("  -o, --only-matching       show only the part of a line matching PATTERN");
    outln!("  -q, --quiet, --silent     suppress all normal output");
    outln!("  -v, --invert-match        select non-matching lines");
    outln!("  -H, --with-filename       print the file name for each match");
    outln!("  -h, --no-filename         suppress the file name prefix");
    outln!("  -b, --byte-offset         print the byte offset with output lines");
    outln!("  -Z, --null                print 0 byte after FILE name");
    outln!("  -r, -R, --recursive       search directories recursively");
    outln!("  -A, --after-context=NUM   print NUM lines of trailing context");
    outln!("  -B, --before-context=NUM  print NUM lines of leading context");
    outln!("  -C, --context=NUM         print NUM lines of output context");
    if err || out.flush().is_err() {
        drop(out);
        eprintln!("grep: write error: {}", io::Error::last_os_error());
        process::exit(2);
    }
}
