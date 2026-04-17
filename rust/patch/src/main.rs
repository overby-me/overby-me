use regex::Regex;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process;

// Exit codes matching GNU patch
const EXIT_SUCCESS: i32 = 0;
const EXIT_HUNKS_FAILED: i32 = 1;
const EXIT_TROUBLE: i32 = 2;

#[derive(Debug, Clone)]
struct Options {
    strip: Option<usize>,
    directory: Option<String>,
    input: Option<String>,
    output: Option<String>,
    reverse: bool,
    dry_run: bool,
    silent: bool,
    verbose: bool,
    forward: bool,
    fuzz: usize,
    backup: bool,
    backup_prefix: Option<String>,   // -B / --prefix
    backup_suffix: Option<String>,   // -z / --suffix
    basename_prefix: Option<String>, // -Y / --basename-prefix
    version_control: Option<String>, // -V / --version-control
    no_backup_if_mismatch: bool,
    force: bool,
    remove_empty: bool,
    read_only: ReadOnlyMode,
    reject_file: Option<String>,
    reject_format: Option<String>, // unified | context (None = match input)
    posix: bool,
    positional_file: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ReadOnlyMode {
    Ignore,
    #[default]
    Warn,
    Fail,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            strip: None,
            directory: None,
            input: None,
            output: None,
            reverse: false,
            dry_run: false,
            silent: false,
            verbose: false,
            forward: false,
            fuzz: 2,
            backup: false,
            backup_prefix: None,
            backup_suffix: None,
            basename_prefix: None,
            version_control: None,
            no_backup_if_mismatch: false,
            force: false,
            remove_empty: false,
            read_only: ReadOnlyMode::Warn,
            reject_file: None,
            reject_format: None,
            posix: false,
            positional_file: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum DiffFormat {
    Unified,
    Context,
    Normal,
}

#[derive(Debug, Clone)]
struct Hunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    /// Text after the `@@ ... @@` range (e.g. function-name heuristic).
    header_suffix: String,
    lines: Vec<HunkLine>,
}

#[derive(Debug, Clone)]
enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

#[derive(Debug, Clone, Default)]
struct GitMeta {
    new_file_mode: Option<u32>,
    deleted_file_mode: Option<u32>,
    old_mode: Option<u32>,
    new_mode: Option<u32>,
    rename_from: Option<String>,
    rename_to: Option<String>,
    copy_from: Option<String>,
    copy_to: Option<String>,
}

#[derive(Debug, Clone)]
struct FilePatch {
    old_file: String,
    new_file: String,
    /// Raw text after `--- ` / `+++ ` — preserves the label (tab-separated
    /// timestamps, "label of X", etc.) for verbatim use in reject files.
    old_header: String,
    new_header: String,
    /// `Index: path` preamble line if present; preserved in rejects.
    index_line: Option<String>,
    hunks: Vec<Hunk>,
    format: DiffFormat,
    git: GitMeta,
}

fn parse_strip(val: &str) -> usize {
    match val.parse() {
        Ok(n) => n,
        Err(_) => {
            let argv0 = env::args().next().unwrap_or_else(|| "patch".to_string());
            eprintln!("{argv0}: **** strip count {val} is not a number");
            process::exit(EXIT_TROUBLE);
        }
    }
}

fn parse_args() -> Options {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut opts = Options::default();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];

        if arg == "--version" || arg == "-V" {
            println!("patch (rust-patch) {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }

        if arg == "--" {
            i += 1;
            if i < args.len() && opts.positional_file.is_none() {
                opts.positional_file = Some(args[i].clone());
            }
            i += 1;
            continue;
        }

        if let Some(val) = arg.strip_prefix("--strip=") {
            opts.strip = Some(parse_strip(val));
        } else if arg == "--strip" || arg == "-p" {
            i += 1;
            if i < args.len() {
                opts.strip = Some(parse_strip(&args[i]));
            }
        } else if let Some(val) = arg.strip_prefix("-p") {
            opts.strip = Some(parse_strip(val));
        } else if let Some(val) = arg.strip_prefix("--directory=") {
            opts.directory = Some(val.to_string());
        } else if arg == "--directory" || arg == "-d" {
            i += 1;
            if i < args.len() {
                opts.directory = Some(args[i].clone());
            }
        } else if let Some(val) = arg.strip_prefix("--input=") {
            opts.input = Some(val.to_string());
        } else if arg == "--input" || arg == "-i" {
            i += 1;
            if i < args.len() {
                opts.input = Some(args[i].clone());
            }
        } else if let Some(val) = arg.strip_prefix("--output=") {
            opts.output = Some(val.to_string());
        } else if arg == "--output" || arg == "-o" {
            i += 1;
            if i < args.len() {
                opts.output = Some(args[i].clone());
            }
        } else if arg == "-R" || arg == "--reverse" {
            opts.reverse = true;
        } else if arg == "--dry-run" {
            opts.dry_run = true;
        } else if arg == "-s" || arg == "--quiet" || arg == "--silent" {
            opts.silent = true;
        } else if arg == "-v" || arg == "--verbose" {
            opts.verbose = true;
        } else if arg == "-N" || arg == "--forward" {
            opts.forward = true;
        } else if let Some(val) = arg.strip_prefix("--fuzz=") {
            opts.fuzz = val.parse().unwrap_or(2);
        } else if arg == "--fuzz" || arg == "-F" {
            i += 1;
            if i < args.len() {
                opts.fuzz = args[i].parse().unwrap_or(2);
            }
        } else if let Some(val) = arg.strip_prefix("-F") {
            opts.fuzz = val.parse().unwrap_or(2);
        } else if arg == "-b" || arg == "--backup" {
            opts.backup = true;
        } else if arg == "--no-backup-if-mismatch" {
            opts.no_backup_if_mismatch = true;
        } else if let Some(val) = arg.strip_prefix("--prefix=") {
            opts.backup_prefix = Some(val.to_string());
            opts.backup = true;
        } else if arg == "-B" || arg == "--prefix" {
            i += 1;
            if i < args.len() {
                opts.backup_prefix = Some(args[i].clone());
                opts.backup = true;
            }
        } else if let Some(val) = arg.strip_prefix("--suffix=") {
            opts.backup_suffix = Some(val.to_string());
            opts.backup = true;
        } else if arg == "-z" || arg == "--suffix" {
            i += 1;
            if i < args.len() {
                opts.backup_suffix = Some(args[i].clone());
                opts.backup = true;
            }
        } else if let Some(val) = arg.strip_prefix("--basename-prefix=") {
            opts.basename_prefix = Some(val.to_string());
            opts.backup = true;
        } else if arg == "-Y" || arg == "--basename-prefix" {
            i += 1;
            if i < args.len() {
                opts.basename_prefix = Some(args[i].clone());
                opts.backup = true;
            }
        } else if let Some(val) = arg.strip_prefix("--version-control=") {
            opts.version_control = Some(val.to_string());
        } else if arg == "-V" || arg == "--version-control" {
            i += 1;
            if i < args.len() {
                opts.version_control = Some(args[i].clone());
            }
        } else if let Some(val) = arg.strip_prefix("--reject-file=") {
            opts.reject_file = Some(val.to_string());
        } else if arg == "-r" || arg == "--reject-file" {
            i += 1;
            if i < args.len() {
                opts.reject_file = Some(args[i].clone());
            }
        } else if let Some(val) = arg.strip_prefix("--reject-format=") {
            opts.reject_format = Some(val.to_string());
        } else if let Some(val) = arg.strip_prefix("--read-only=") {
            opts.read_only = match val {
                "ignore" => ReadOnlyMode::Ignore,
                "warn" => ReadOnlyMode::Warn,
                "fail" => ReadOnlyMode::Fail,
                _ => {
                    eprintln!("patch: invalid read-only mode: {val}");
                    process::exit(EXIT_TROUBLE);
                }
            };
        } else if arg == "--posix" {
            opts.posix = true;
        } else if arg == "-f" || arg == "--force" {
            opts.force = true;
        } else if arg == "-E" || arg == "--remove-empty-files" {
            opts.remove_empty = true;
        } else if !arg.starts_with('-') {
            if opts.positional_file.is_none() {
                opts.positional_file = Some(arg.clone());
            }
        } else {
            // Handle combined short flags like -sNf
            if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 2 {
                let chars: Vec<char> = arg[1..].chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        's' => opts.silent = true,
                        'R' => opts.reverse = true,
                        'N' => opts.forward = true,
                        'b' => opts.backup = true,
                        'f' => opts.force = true,
                        'E' => opts.remove_empty = true,
                        'v' => opts.verbose = true,
                        _ => {}
                    }
                    j += 1;
                }
            }
        }
        i += 1;
    }
    opts
}

fn strip_path(path: &str, strip: usize) -> String {
    // /dev/null is special-cased: never strip, preserve verbatim.
    if path == "/dev/null" {
        return path.to_string();
    }
    if strip == 0 {
        return path.to_string();
    }
    let components: Vec<&str> = path.split('/').collect();
    if strip >= components.len() {
        components.last().unwrap_or(&"").to_string()
    } else {
        components[strip..].join("/")
    }
}

fn read_patch_input(opts: &Options) -> String {
    match &opts.input {
        Some(path) => fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("patch: can't open patch file {}: {}", path, e);
            process::exit(EXIT_TROUBLE);
        }),
        None => {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf).unwrap_or_else(|e| {
                eprintln!("patch: error reading stdin: {}", e);
                process::exit(EXIT_TROUBLE);
            });
            buf
        }
    }
}

fn parse_unified_hunk_header(line: &str) -> Option<(usize, usize, usize, usize)> {
    let re = Regex::new(r"^@@\s+-(\d+)(?:,(\d+))?\s+\+(\d+)(?:,(\d+))?\s+@@").unwrap();
    re.captures(line).map(|caps| {
        let old_start: usize = caps[1].parse().unwrap();
        let old_count: usize = caps.get(2).map_or(1, |m| m.as_str().parse().unwrap());
        let new_start: usize = caps[3].parse().unwrap();
        let new_count: usize = caps.get(4).map_or(1, |m| m.as_str().parse().unwrap());
        (old_start, old_count, new_start, new_count)
    })
}

fn parse_context_hunk_range(line: &str) -> Option<(usize, usize)> {
    // Matches "*** start,end ****" or "--- start,end ----" or single line
    // "*** start ****". Allow arbitrary whitespace around the comma.
    let re = Regex::new(r"^[\*\-]{3}\s+(\d+)\s*(?:,\s*(\d+))?\s+[\*\-]{4}").unwrap();
    re.captures(line).map(|caps| {
        let start: usize = caps[1].parse().unwrap();
        let end: usize = caps.get(2).map_or(start, |m| m.as_str().parse().unwrap());
        (start, end)
    })
}

fn parse_normal_command(line: &str) -> Option<(usize, usize, char, usize, usize)> {
    let re = Regex::new(r"^(\d+)(?:,(\d+))?([acd])(\d+)(?:,(\d+))?$").unwrap();
    re.captures(line).map(|caps| {
        let s1: usize = caps[1].parse().unwrap();
        let e1: usize = caps.get(2).map_or(s1, |m| m.as_str().parse().unwrap());
        let cmd = caps[3].chars().next().unwrap();
        let s2: usize = caps[4].parse().unwrap();
        let e2: usize = caps.get(5).map_or(s2, |m| m.as_str().parse().unwrap());
        (s1, e1, cmd, s2, e2)
    })
}

fn detect_format(lines: &[&str], start: usize) -> DiffFormat {
    for line in &lines[start..] {
        if line.starts_with("@@") {
            return DiffFormat::Unified;
        }
        if line.starts_with("***************") {
            return DiffFormat::Context;
        }
        if parse_normal_command(line).is_some() {
            return DiffFormat::Normal;
        }
        // If we hit a file header, keep looking after it
        if line.starts_with("--- ") || line.starts_with("+++ ") || line.starts_with("*** ") {
            continue;
        }
        // Skip other preamble lines (like "diff" lines)
        if line.starts_with("diff ") || line.starts_with("index ") {
            continue;
        }
    }
    DiffFormat::Unified // default
}

fn parse_file_path(line: &str, prefix: &str) -> String {
    let rest = &line[prefix.len()..];
    // Remove trailing timestamp (tab-separated)
    rest.split('\t').next().unwrap_or(rest).trim().to_string()
}

fn hunk_header_suffix(line: &str) -> String {
    // After the closing `@@`, the rest (if any) is the function-name heuristic.
    let re = Regex::new(r"^@@\s+-\d+(?:,\d+)?\s+\+\d+(?:,\d+)?\s+@@").unwrap();
    if let Some(m) = re.find(line) {
        line[m.end()..].to_string()
    } else {
        String::new()
    }
}

fn parse_patches(input: &str) -> Vec<FilePatch> {
    let lines: Vec<&str> = input.lines().collect();
    let mut patches = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        // Look for the start of a patch
        let format = detect_format(&lines, i);

        match format {
            DiffFormat::Unified => {
                if let Some((patch, next_i)) = parse_unified_patch(&lines, i) {
                    patches.push(patch);
                    i = next_i;
                } else {
                    i += 1;
                }
            }
            DiffFormat::Context => {
                if let Some((patch, next_i)) = parse_context_patch(&lines, i) {
                    patches.push(patch);
                    i = next_i;
                } else {
                    i += 1;
                }
            }
            DiffFormat::Normal => {
                if let Some((patch, next_i)) = parse_normal_patch(&lines, i) {
                    patches.push(patch);
                    i = next_i;
                } else {
                    i += 1;
                }
            }
        }
    }
    patches
}

fn parse_unified_patch(lines: &[&str], start: usize) -> Option<(FilePatch, usize)> {
    let mut i = start;
    let mut git = GitMeta::default();
    let mut git_a: Option<String> = None;
    let mut git_b: Option<String> = None;
    let mut index_line: Option<String> = None;

    // Walk preamble, picking up git-diff extensions if present.
    while i < lines.len() && !lines[i].starts_with("--- ") && !lines[i].starts_with("@@") {
        let line = lines[i];
        if let Some(_rest) = line.strip_prefix("Index: ") {
            index_line = Some(line.to_string());
        } else if let Some(rest) = line.strip_prefix("diff --git ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() == 2 {
                git_a = Some(parts[0].trim_start_matches("a/").to_string());
                git_b = Some(parts[1].trim_start_matches("b/").to_string());
            }
        } else if let Some(rest) = line.strip_prefix("new file mode ") {
            git.new_file_mode = u32::from_str_radix(rest.trim(), 8).ok();
        } else if let Some(rest) = line.strip_prefix("deleted file mode ") {
            git.deleted_file_mode = u32::from_str_radix(rest.trim(), 8).ok();
        } else if let Some(rest) = line.strip_prefix("old mode ") {
            git.old_mode = u32::from_str_radix(rest.trim(), 8).ok();
        } else if let Some(rest) = line.strip_prefix("new mode ") {
            git.new_mode = u32::from_str_radix(rest.trim(), 8).ok();
        } else if let Some(rest) = line.strip_prefix("rename from ") {
            git.rename_from = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("rename to ") {
            git.rename_to = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("copy from ") {
            git.copy_from = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("copy to ") {
            git.copy_to = Some(rest.to_string());
        } else if line == "GIT binary patch" {
            // Binary patches are not supported; bail.
            return None;
        }
        i += 1;

        // If we reached another "diff --git" without hitting ---/@@, we've
        // consumed a header-only patch (like rename-only or mode-only).
        if i < lines.len() && lines[i].starts_with("diff --git ") {
            if git_a.is_some() || git_b.is_some() {
                let old_file = git
                    .rename_from
                    .clone()
                    .or_else(|| git.copy_from.clone())
                    .or_else(|| git_a.clone())
                    .unwrap_or_default();
                let new_file = git
                    .rename_to
                    .clone()
                    .or_else(|| git.copy_to.clone())
                    .or_else(|| git_b.clone())
                    .unwrap_or_default();
                return Some((
                    FilePatch {
                        old_file: old_file.clone(),
                        new_file: new_file.clone(),
                        old_header: old_file,
                        new_header: new_file,
                        index_line: index_line.clone(),
                        hunks: Vec::new(),
                        format: DiffFormat::Unified,
                        git,
                    },
                    i,
                ));
            }
        }
    }
    if i >= lines.len() {
        // End-of-input with no hunks: if we had a git header, this is a
        // header-only patch.
        if git_a.is_some() || git_b.is_some() {
            let old_file = git
                .rename_from
                .clone()
                .or_else(|| git.copy_from.clone())
                .or_else(|| git_a.clone())
                .unwrap_or_default();
            let new_file = git
                .rename_to
                .clone()
                .or_else(|| git.copy_to.clone())
                .or_else(|| git_b.clone())
                .unwrap_or_default();
            return Some((
                FilePatch {
                    old_file: old_file.clone(),
                    new_file: new_file.clone(),
                    old_header: old_file,
                    new_header: new_file,
                    index_line,
                    hunks: Vec::new(),
                    format: DiffFormat::Unified,
                    git,
                },
                i,
            ));
        }
        return None;
    }

    let mut old_file = String::new();
    let mut new_file = String::new();
    let mut old_header = String::new();
    let mut new_header = String::new();

    // Parse --- and +++ headers
    if lines[i].starts_with("--- ") {
        old_file = parse_file_path(lines[i], "--- ");
        old_header = lines[i][4..].to_string();
        i += 1;
    }
    if i < lines.len() && lines[i].starts_with("+++ ") {
        new_file = parse_file_path(lines[i], "+++ ");
        new_header = lines[i][4..].to_string();
        i += 1;
    }

    // Prefer git header names when the --- /+++ paths are /dev/null or
    // empty, so we know which real file to touch.
    if old_file == "/dev/null" || old_file.is_empty() {
        if let Some(a) = &git_a {
            old_file = a.clone();
        }
    }
    if new_file == "/dev/null" || new_file.is_empty() {
        if let Some(b) = &git_b {
            new_file = b.clone();
        }
    }

    if old_file.is_empty() && new_file.is_empty() {
        return None;
    }

    let mut hunks = Vec::new();

    while i < lines.len() {
        if lines[i].starts_with("@@") {
            if let Some((old_start, old_count, new_start, new_count)) =
                parse_unified_hunk_header(lines[i])
            {
                // Capture text after the closing `@@` as a header suffix.
                let header_suffix = hunk_header_suffix(lines[i]);
                i += 1;
                let mut hunk_lines = Vec::new();

                while i < lines.len() {
                    let line = lines[i];
                    if line.starts_with("@@")
                        || line.starts_with("--- ")
                        || line.starts_with("diff ")
                    {
                        break;
                    }
                    if let Some(rest) = line.strip_prefix('-') {
                        hunk_lines.push(HunkLine::Remove(rest.to_string()));
                    } else if let Some(rest) = line.strip_prefix('+') {
                        hunk_lines.push(HunkLine::Add(rest.to_string()));
                    } else if let Some(rest) = line.strip_prefix(' ') {
                        hunk_lines.push(HunkLine::Context(rest.to_string()));
                    } else if line == r"\ No newline at end of file" {
                        // Skip this marker
                    } else if line.is_empty() {
                        // Empty context line (some diffs strip trailing space)
                        hunk_lines.push(HunkLine::Context(String::new()));
                    } else {
                        break;
                    }
                    i += 1;
                }

                hunks.push(Hunk {
                    old_start,
                    old_count,
                    new_start,
                    new_count,
                    header_suffix,
                    lines: hunk_lines,
                });
            } else {
                break;
            }
        } else {
            break;
        }
    }

    if hunks.is_empty() {
        return None;
    }

    Some((
        FilePatch {
            old_file,
            new_file,
            old_header,
            new_header,
            index_line,
            hunks,
            format: DiffFormat::Unified,
            git,
        },
        i,
    ))
}

fn parse_context_patch(lines: &[&str], start: usize) -> Option<(FilePatch, usize)> {
    let mut i = start;

    // Find *** header
    while i < lines.len() && !lines[i].starts_with("*** ") {
        i += 1;
    }
    if i >= lines.len() {
        return None;
    }

    let old_file = parse_file_path(lines[i], "*** ");
    i += 1;

    if i >= lines.len() || !lines[i].starts_with("--- ") {
        return None;
    }
    let new_file = parse_file_path(lines[i], "--- ");
    i += 1;

    let mut hunks = Vec::new();

    while i < lines.len() && lines[i].starts_with("***************") {
        // Preserve the "function name" suffix after the `***************`
        // header, as `*************** suffix`.
        let header_suffix = lines[i]["***************".len()..].to_string();
        i += 1;
        if i >= lines.len() {
            break;
        }

        // Parse old section "*** start,end ****"
        let (old_start, old_end) = match parse_context_hunk_range(lines[i]) {
            Some(r) => r,
            None => break,
        };
        i += 1;

        let mut old_lines: Vec<(char, String)> = Vec::new();
        while i < lines.len() && !lines[i].starts_with("--- ") {
            let line = lines[i];
            if let Some(rest) = line.strip_prefix("! ") {
                old_lines.push(('!', rest.to_string()));
            } else if let Some(rest) = line.strip_prefix("- ") {
                old_lines.push(('-', rest.to_string()));
            } else if let Some(rest) = line.strip_prefix("  ") {
                old_lines.push((' ', rest.to_string()));
            } else if line == "  " || line == " " {
                old_lines.push((' ', String::new()));
            }
            i += 1;
        }

        if i >= lines.len() {
            break;
        }

        // Parse new section "--- start,end ----"
        let (new_start, new_end) = match parse_context_hunk_range(lines[i]) {
            Some(r) => r,
            None => break,
        };
        i += 1;

        let mut new_lines: Vec<(char, String)> = Vec::new();
        while i < lines.len()
            && !lines[i].starts_with("***************")
            && !lines[i].starts_with("diff ")
            && !lines[i].starts_with("*** ")
        {
            let line = lines[i];
            if let Some(rest) = line.strip_prefix("! ") {
                new_lines.push(('!', rest.to_string()));
            } else if let Some(rest) = line.strip_prefix("+ ") {
                new_lines.push(('+', rest.to_string()));
            } else if let Some(rest) = line.strip_prefix("  ") {
                new_lines.push((' ', rest.to_string()));
            } else if line == "  " || line == " " {
                new_lines.push((' ', String::new()));
            } else {
                break;
            }
            i += 1;
        }

        // Convert context diff to unified hunk lines
        let hunk_lines = context_to_unified_lines(&old_lines, &new_lines);
        let old_count = if old_end >= old_start {
            old_end - old_start + 1
        } else {
            0
        };
        let new_count = if new_end >= new_start {
            new_end - new_start + 1
        } else {
            0
        };

        hunks.push(Hunk {
            old_start,
            old_count,
            new_start,
            new_count,
            header_suffix,
            lines: hunk_lines,
        });
    }

    if hunks.is_empty() {
        return None;
    }

    Some((
        FilePatch {
            old_file: old_file.clone(),
            new_file: new_file.clone(),
            old_header: old_file,
            new_header: new_file,
            index_line: None,
            hunks,
            format: DiffFormat::Context,
            git: GitMeta::default(),
        },
        i,
    ))
}

fn context_to_unified_lines(
    old_lines: &[(char, String)],
    new_lines: &[(char, String)],
) -> Vec<HunkLine> {
    // Context diffs have the same context lines on both sides; removals and
    // additions appear in their respective halves. Walk both sides together,
    // emitting a single context line when both sides agree, and emitting all
    // pending removals before switching to additions at a boundary.
    let mut result = Vec::new();
    let mut oi = 0;
    let mut ni = 0;

    while oi < old_lines.len() || ni < new_lines.len() {
        let old_is_ctx = oi < old_lines.len() && old_lines[oi].0 == ' ';
        let new_is_ctx = ni < new_lines.len() && new_lines[ni].0 == ' ';

        if old_is_ctx && new_is_ctx {
            result.push(HunkLine::Context(old_lines[oi].1.clone()));
            oi += 1;
            ni += 1;
        } else if oi < old_lines.len() && (old_lines[oi].0 == '!' || old_lines[oi].0 == '-')
        {
            result.push(HunkLine::Remove(old_lines[oi].1.clone()));
            oi += 1;
        } else if ni < new_lines.len() && (new_lines[ni].0 == '!' || new_lines[ni].0 == '+')
        {
            result.push(HunkLine::Add(new_lines[ni].1.clone()));
            ni += 1;
        } else if old_is_ctx {
            result.push(HunkLine::Context(old_lines[oi].1.clone()));
            oi += 1;
        } else if new_is_ctx {
            result.push(HunkLine::Context(new_lines[ni].1.clone()));
            ni += 1;
        } else {
            break;
        }
    }
    result
}

fn parse_normal_patch(lines: &[&str], start: usize) -> Option<(FilePatch, usize)> {
    let mut i = start;
    let mut hunks = Vec::new();

    // Normal diffs don't have file headers typically, but we try to find them
    let mut old_file = String::new();
    let mut new_file = String::new();

    // Check for optional diff header
    while i < lines.len() {
        if parse_normal_command(lines[i]).is_some() {
            break;
        }
        if lines[i].starts_with("diff ") {
            // Try to extract filenames from "diff" line
            let parts: Vec<&str> = lines[i].split_whitespace().collect();
            if parts.len() >= 3 {
                old_file = parts[parts.len() - 2].to_string();
                new_file = parts[parts.len() - 1].to_string();
            }
        }
        i += 1;
    }

    while i < lines.len() {
        if let Some((s1, e1, cmd, s2, e2)) = parse_normal_command(lines[i]) {
            i += 1;
            let mut hunk_lines = Vec::new();

            match cmd {
                'a' => {
                    // Add: lines after s1 in old, s2..e2 in new
                    while i < lines.len() && lines[i].starts_with("> ") {
                        hunk_lines.push(HunkLine::Add(lines[i][2..].to_string()));
                        i += 1;
                    }
                    hunks.push(Hunk {
                        old_start: s1,
                        old_count: 0,
                        new_start: s2,
                        new_count: e2 - s2 + 1,
                        header_suffix: String::new(),
                        lines: hunk_lines,
                    });
                }
                'd' => {
                    // Delete: s1..e1 in old, after s2 in new
                    while i < lines.len() && lines[i].starts_with("< ") {
                        hunk_lines.push(HunkLine::Remove(lines[i][2..].to_string()));
                        i += 1;
                    }
                    hunks.push(Hunk {
                        old_start: s1,
                        old_count: e1 - s1 + 1,
                        new_start: s2,
                        new_count: 0,
                        header_suffix: String::new(),
                        lines: hunk_lines,
                    });
                }
                'c' => {
                    // Change: s1..e1 replaced by s2..e2
                    while i < lines.len() && lines[i].starts_with("< ") {
                        hunk_lines.push(HunkLine::Remove(lines[i][2..].to_string()));
                        i += 1;
                    }
                    // Skip separator "---"
                    if i < lines.len() && lines[i] == "---" {
                        i += 1;
                    }
                    while i < lines.len() && lines[i].starts_with("> ") {
                        hunk_lines.push(HunkLine::Add(lines[i][2..].to_string()));
                        i += 1;
                    }
                    hunks.push(Hunk {
                        old_start: s1,
                        old_count: e1 - s1 + 1,
                        new_start: s2,
                        new_count: e2 - s2 + 1,
                        header_suffix: String::new(),
                        lines: hunk_lines,
                    });
                }
                _ => {}
            }
        } else {
            break;
        }
    }

    if hunks.is_empty() {
        return None;
    }

    Some((
        FilePatch {
            old_file: old_file.clone(),
            new_file: new_file.clone(),
            old_header: old_file,
            new_header: new_file,
            index_line: None,
            hunks,
            format: DiffFormat::Normal,
            git: GitMeta::default(),
        },
        i,
    ))
}

fn resolve_target_file(patch: &FilePatch, opts: &Options) -> PathBuf {
    if let Some(ref pos_file) = opts.positional_file {
        return PathBuf::from(pos_file);
    }

    let strip = opts.strip.unwrap_or(1);

    let old_stripped = strip_path(&patch.old_file, strip);
    let new_stripped = strip_path(&patch.new_file, strip);

    // Prefer the file that exists
    let base = opts.directory.as_deref().unwrap_or(".");

    let old_path = Path::new(base).join(&old_stripped);
    let new_path = Path::new(base).join(&new_stripped);

    if old_stripped == "/dev/null" {
        return new_path;
    }
    if new_stripped == "/dev/null" {
        return old_path;
    }

    if old_path.exists() {
        old_path
    } else if new_path.exists() {
        new_path
    } else {
        // Neither exists. Default to the old-side filename (matches GNU
        // patch's behaviour) unless old is empty/dev-null.
        if old_stripped.is_empty() || old_stripped == "/dev/null" {
            new_path
        } else {
            old_path
        }
    }
}

fn apply_hunk(
    file_lines: &[String],
    hunk: &Hunk,
    fuzz: usize,
    reverse: bool,
) -> Option<(Vec<String>, usize, i64, usize)> {
    let target_start = if reverse {
        if hunk.new_start == 0 {
            0
        } else {
            hunk.new_start - 1
        }
    } else if hunk.old_start == 0 {
        0
    } else {
        hunk.old_start - 1
    };

    // Normalize the hunk view for the current direction. For each HunkLine,
    // classify it as context/consume/produce. Reversing swaps Remove<->Add.
    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Context,
        Consume, // expected in old file, dropped
        Produce, // written to new file
    }
    let classified: Vec<(Kind, &String)> = hunk
        .lines
        .iter()
        .map(|l| match (l, reverse) {
            (HunkLine::Context(s), _) => (Kind::Context, s),
            (HunkLine::Remove(s), false) | (HunkLine::Add(s), true) => (Kind::Consume, s),
            (HunkLine::Add(s), false) | (HunkLine::Remove(s), true) => (Kind::Produce, s),
        })
        .collect();

    // Count leading/trailing context lines (for fuzz).
    let leading_ctx = classified
        .iter()
        .take_while(|(k, _)| *k == Kind::Context)
        .count();
    let trailing_ctx = classified
        .iter()
        .rev()
        .take_while(|(k, _)| *k == Kind::Context)
        .count();

    // Try exact match first, then with increasing fuzz/offset.
    for fuzz_level in 0..=fuzz {
        let skip_lead = fuzz_level.min(leading_ctx);
        let skip_trail = fuzz_level.min(trailing_ctx);

        // Build the effective "old" (file side) pattern, skipping fuzzed
        // context at both ends.
        let effective: Vec<(Kind, &String)> =
            classified[skip_lead..classified.len() - skip_trail].to_vec();
        let old_len = effective
            .iter()
            .filter(|(k, _)| *k != Kind::Produce)
            .count();

        for offset_mag in 0..=file_lines.len() {
            for &sign in &[1i64, -1i64] {
                if offset_mag == 0 && sign == -1 {
                    continue;
                }
                let offset = offset_mag as i64 * sign;
                // target_start already sits after `leading_ctx` lines of
                // leading context; shift forward by `skip_lead` to land on
                // the first non-skipped line.
                let raw_start = target_start as i64 + offset + skip_lead as i64;
                if raw_start < 0 {
                    continue;
                }
                let actual_start = raw_start as usize;
                if actual_start + old_len > file_lines.len() {
                    continue;
                }

                let mut fi = actual_start;
                let mut matched = true;
                for (k, s) in &effective {
                    match k {
                        Kind::Context | Kind::Consume => {
                            if file_lines[fi] != **s {
                                matched = false;
                                break;
                            }
                            fi += 1;
                        }
                        Kind::Produce => {} // doesn't consume a file line
                    }
                }
                if !matched {
                    continue;
                }

                // Apply: keep lines before actual_start, emit produced/kept
                // lines for the effective region, then keep trailing file.
                let mut result = Vec::new();
                result.extend_from_slice(&file_lines[..actual_start]);
                let mut fi = actual_start;
                for (k, s) in &effective {
                    match k {
                        Kind::Context => {
                            result.push(file_lines[fi].clone());
                            fi += 1;
                        }
                        Kind::Consume => {
                            fi += 1;
                        }
                        Kind::Produce => {
                            result.push((*s).clone());
                        }
                    }
                }
                result.extend_from_slice(&file_lines[fi..]);

                // Report the applied offset relative to the hunk's
                // originally-expected position (before adding skip_lead).
                let expected_start = target_start as i64 + skip_lead as i64;
                let applied_offset = actual_start as i64 - expected_start;
                return Some((result, fuzz_level, applied_offset, actual_start));
            }
        }
    }
    None
}

fn safe_exit_success() -> ! {
    process::exit(EXIT_SUCCESS);
}

fn compute_backup_path(target: &Path, opts: &Options) -> PathBuf {
    // Strip a leading "./" from target so prefix/suffix join cleanly.
    let target_owned: PathBuf;
    let target: &Path = if let Ok(stripped) = target.strip_prefix("./") {
        target_owned = stripped.to_path_buf();
        &target_owned
    } else {
        target
    };

    // Precedence: explicit prefix/suffix/basename-prefix options win over env.
    // Default: `<path>.orig`, or `<path><SIMPLE_BACKUP_SUFFIX>`.
    let env_suffix = env::var("SIMPLE_BACKUP_SUFFIX").ok();
    let suffix: String = opts
        .backup_suffix
        .clone()
        .or(env_suffix)
        .unwrap_or_else(|| ".orig".to_string());

    // Determine VERSION_CONTROL / PATCH_VERSION_CONTROL.
    let version_control = opts
        .version_control
        .clone()
        .or_else(|| env::var("PATCH_VERSION_CONTROL").ok())
        .or_else(|| env::var("VERSION_CONTROL").ok())
        .unwrap_or_else(|| "existing".to_string());

    let simple_backup = |target: &Path| -> PathBuf {
        if let Some(prefix) = &opts.backup_prefix {
            // Prepend prefix to the whole path.
            let mut s = prefix.clone();
            s.push_str(&target.to_string_lossy());
            s.push_str(&suffix_if_no_prefix_only(opts, &suffix));
            PathBuf::from(s)
        } else if let Some(bprefix) = &opts.basename_prefix {
            // Prepend prefix to the basename only.
            let parent = target.parent().unwrap_or(Path::new(""));
            let basename = target
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut new_name = bprefix.clone();
            new_name.push_str(&basename);
            new_name.push_str(&suffix_if_no_prefix_only(opts, &suffix));
            parent.join(new_name)
        } else {
            PathBuf::from(format!("{}{suffix}", target.to_string_lossy()))
        }
    };

    let numbered_backup = |target: &Path, n: u32| -> PathBuf {
        PathBuf::from(format!("{}.~{n}~", target.to_string_lossy()))
    };

    // Numbered backups only affect the default-suffix case without explicit
    // prefix/suffix.
    let use_numbered = match version_control.as_str() {
        "numbered" | "t" => true,
        "existing" | "nil" => {
            // Look for an existing .~N~ file.
            let base = target.to_string_lossy();
            let mut found = false;
            if let Some(dir) = target.parent() {
                if let Ok(entries) = fs::read_dir(if dir.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    dir
                }) {
                    let base_name = target
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let prefix = format!("{base_name}.~");
                    for e in entries.flatten() {
                        let name = e.file_name();
                        let name_s = name.to_string_lossy();
                        if name_s.starts_with(&prefix) && name_s.ends_with('~') {
                            found = true;
                            break;
                        }
                    }
                }
            }
            let _ = base;
            found
        }
        _ => false, // simple / never / off → simple backup
    };

    if use_numbered
        && opts.backup_prefix.is_none()
        && opts.basename_prefix.is_none()
        && opts.backup_suffix.is_none()
    {
        // Pick next N not already present.
        let mut n: u32 = 1;
        loop {
            let candidate = numbered_backup(target, n);
            if !candidate.exists() {
                return candidate;
            }
            n += 1;
        }
    }

    simple_backup(target)
}

fn suffix_if_no_prefix_only(_opts: &Options, suffix: &str) -> String {
    // When a custom prefix is used together with a custom suffix, both apply.
    // When only a prefix is used, no suffix is appended.
    if _opts.backup_suffix.is_some() {
        suffix.to_string()
    } else if _opts.backup_prefix.is_some() || _opts.basename_prefix.is_some() {
        String::new()
    } else {
        suffix.to_string()
    }
}

fn write_reject_file(
    path: &str,
    patch: &FilePatch,
    rejected: &[&Hunk],
    opts: &Options,
) -> io::Result<()> {
    use std::io::Write;
    let mut f = fs::File::create(path)?;
    let old_label = if !patch.old_header.is_empty() {
        patch.old_header.clone()
    } else if !patch.old_file.is_empty() {
        patch.old_file.clone()
    } else {
        "a".to_string()
    };
    let new_label = if !patch.new_header.is_empty() {
        patch.new_header.clone()
    } else if !patch.new_file.is_empty() {
        patch.new_file.clone()
    } else {
        "b".to_string()
    };

    if let Some(idx) = &patch.index_line {
        writeln!(f, "{idx}")?;
    }

    // Decide output format: --reject-format wins, else mirror input.
    let use_context = match opts.reject_format.as_deref() {
        Some("context") => true,
        Some("unified") => false,
        _ => patch.format == DiffFormat::Context,
    };

    if use_context {
        writeln!(f, "*** {old_label}")?;
        writeln!(f, "--- {new_label}")?;
        for hunk in rejected {
            writeln!(f, "***************{}", hunk.header_suffix)?;
            write_context_hunk(&mut f, hunk)?;
        }
    } else {
        writeln!(f, "--- {old_label}")?;
        writeln!(f, "+++ {new_label}")?;
        for hunk in rejected {
            let old_range = format_hunk_range(hunk.old_start, hunk.old_count);
            let new_range = format_hunk_range(hunk.new_start, hunk.new_count);
            writeln!(f, "@@ -{old_range} +{new_range} @@{}", hunk.header_suffix)?;
            for line in &hunk.lines {
                match line {
                    HunkLine::Context(s) => writeln!(f, " {s}")?,
                    HunkLine::Remove(s) => writeln!(f, "-{s}")?,
                    HunkLine::Add(s) => writeln!(f, "+{s}")?,
                }
            }
        }
    }
    Ok(())
}

fn write_context_hunk(f: &mut fs::File, hunk: &Hunk) -> io::Result<()> {
    use std::io::Write;
    // Old side: `*** S,E ****` followed by lines marked ` `, `- `, or `! `.
    let old_end = hunk.old_start + hunk.old_count.saturating_sub(1);
    let new_end = hunk.new_start + hunk.new_count.saturating_sub(1);
    let old_range = if hunk.old_count == 1 {
        hunk.old_start.to_string()
    } else {
        format!("{},{}", hunk.old_start, old_end)
    };
    let new_range = if hunk.new_count == 1 {
        hunk.new_start.to_string()
    } else {
        format!("{},{}", hunk.new_start, new_end)
    };

    // Determine whether each removed/added line is a change (!) or pure
    // remove/add (-/+). GNU uses ! for lines that have a paired counterpart
    // in the other side of the hunk (i.e. substitution), and -/+ for pure
    // deletions/additions.
    let has_adds = hunk.lines.iter().any(|l| matches!(l, HunkLine::Add(_)));
    let has_removes = hunk.lines.iter().any(|l| matches!(l, HunkLine::Remove(_)));
    let paired = has_adds && has_removes;

    writeln!(f, "*** {old_range} ****")?;
    for line in &hunk.lines {
        match line {
            HunkLine::Context(s) => writeln!(f, "  {s}")?,
            HunkLine::Remove(s) => {
                if paired {
                    writeln!(f, "! {s}")?;
                } else {
                    writeln!(f, "- {s}")?;
                }
            }
            HunkLine::Add(_) => {}
        }
    }
    writeln!(f, "--- {new_range} ----")?;
    for line in &hunk.lines {
        match line {
            HunkLine::Context(s) => writeln!(f, "  {s}")?,
            HunkLine::Add(s) => {
                if paired {
                    writeln!(f, "! {s}")?;
                } else {
                    writeln!(f, "+ {s}")?;
                }
            }
            HunkLine::Remove(_) => {}
        }
    }
    Ok(())
}

fn format_hunk_range(start: usize, count: usize) -> String {
    // `@@ -N +M @@` for single-line counts; `@@ -N,C +M,C @@` otherwise.
    if count == 1 {
        start.to_string()
    } else {
        format!("{start},{count}")
    }
}

#[cfg(unix)]
fn apply_git_mode(path: &Path, mode: Option<u32>) {
    use std::os::unix::fs::PermissionsExt;
    if let Some(m) = mode
        && let Ok(metadata) = fs::metadata(path)
    {
        let mut perms = metadata.permissions();
        perms.set_mode(m & 0o7777);
        let _ = fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn apply_git_mode(_path: &Path, _mode: Option<u32>) {}

fn display_path(path: &Path) -> String {
    // Strip leading "./" so messages read "patching file foo", not "patching file ./foo".
    let s = path.to_string_lossy().into_owned();
    if let Some(stripped) = s.strip_prefix("./") {
        stripped.to_string()
    } else {
        s
    }
}

fn apply_file_patch(patch: &FilePatch, opts: &Options) -> i32 {
    // Git rename/copy: move/copy the source file to the target first, then
    // apply any hunks below against the new path.
    if let (Some(from), Some(to)) = (&patch.git.rename_from, &patch.git.rename_to) {
        let base = opts.directory.as_deref().unwrap_or(".");
        let src = Path::new(base).join(from);
        let dst = Path::new(base).join(to);
        if let Some(parent) = dst.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = fs::rename(&src, &dst) {
            eprintln!(
                "patch: can't rename {} to {}: {e}",
                src.display(),
                dst.display()
            );
            return EXIT_TROUBLE;
        }
    } else if let (Some(from), Some(to)) = (&patch.git.copy_from, &patch.git.copy_to) {
        let base = opts.directory.as_deref().unwrap_or(".");
        let src = Path::new(base).join(from);
        let dst = Path::new(base).join(to);
        if let Some(parent) = dst.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = fs::copy(&src, &dst) {
            eprintln!(
                "patch: can't copy {} to {}: {e}",
                src.display(),
                dst.display()
            );
            return EXIT_TROUBLE;
        }
    }

    let target = resolve_target_file(patch, opts);
    let target_display = display_path(&target);

    if !opts.silent {
        let verb = if opts.dry_run { "checking" } else { "patching" };
        let suffix = if let Some(from) = &patch.git.rename_from {
            format!(" (renamed from {from})")
        } else if let Some(from) = &patch.git.copy_from {
            format!(" (copied from {from})")
        } else {
            String::new()
        };
        println!("{verb} file {target_display}{suffix}");
    }

    // Header-only git patches (rename/copy/delete/mode with no hunks).
    if patch.hunks.is_empty() {
        if patch.git.deleted_file_mode.is_some() {
            if let Err(e) = fs::remove_file(&target) {
                eprintln!("patch: can't remove {target_display}: {e}");
                return EXIT_TROUBLE;
            }
        } else if patch.git.new_file_mode.is_some() {
            // Create empty file.
            if let Some(parent) = target.parent()
                && !parent.as_os_str().is_empty()
                && !parent.exists()
            {
                let _ = fs::create_dir_all(parent);
            }
            if let Err(e) = fs::write(&target, "") {
                eprintln!("patch: can't create {target_display}: {e}");
                return EXIT_TROUBLE;
            }
            apply_git_mode(&target, patch.git.new_file_mode);
        } else if patch.git.new_mode.is_some() {
            apply_git_mode(&target, patch.git.new_mode);
        }
        return EXIT_SUCCESS;
    }

    // Read existing file content (or empty for new files)
    let content = if target.exists() {
        fs::read_to_string(&target).unwrap_or_else(|e| {
            eprintln!("patch: can't read {target_display}: {e}");
            process::exit(EXIT_TROUBLE);
        })
    } else {
        // Check if this is creating a new file
        if patch.old_file == "/dev/null"
            || patch.old_file.is_empty()
            || patch.git.new_file_mode.is_some()
            || patch.hunks.iter().all(|h| h.old_count == 0)
        {
            String::new()
        } else {
            eprintln!("patch: can't open file {target_display}: No such file or directory");
            return EXIT_TROUBLE;
        }
    };

    let mut file_lines: Vec<String> = if content.is_empty() {
        Vec::new()
    } else {
        content.lines().map(|l| l.to_string()).collect()
    };

    let mut failed_hunks = 0;
    let mut failed_hunk_entries: Vec<&Hunk> = Vec::new();
    let total_hunks = patch.hunks.len();
    // Running delta: lines added by prior hunks minus lines removed. Used
    // to shift the "expected" position reported to the user.
    let mut line_delta: i64 = 0;

    for (hunk_idx, hunk) in patch.hunks.iter().enumerate() {
        let raw_start = if opts.reverse {
            hunk.new_start
        } else {
            hunk.old_start
        };
        // Creation hunks use start 0 ("before any line"); don't report
        // offsets for those — the effective first line is 1.
        let expected_line = if raw_start == 0 {
            1
        } else {
            raw_start as i64 + line_delta
        };

        match apply_hunk(&file_lines, hunk, opts.fuzz, opts.reverse) {
            Some((new_lines, fuzz_used, _offset, applied_start)) => {
                file_lines = new_lines;
                let hunk_num = hunk_idx + 1;
                let applied_line = applied_start as i64 + 1;
                let reported_offset = applied_line - expected_line;
                let line_word = |n: i64| if n == 1 { "line" } else { "lines" };
                let msg = match (reported_offset, fuzz_used) {
                    (0, 0) => format!("Hunk #{hunk_num} succeeded at {applied_line}."),
                    (0, f) => {
                        format!("Hunk #{hunk_num} succeeded at {applied_line} with fuzz {f}.")
                    }
                    (o, 0) => format!(
                        "Hunk #{hunk_num} succeeded at {applied_line} (offset {o} {}).",
                        line_word(o)
                    ),
                    (o, f) => format!(
                        "Hunk #{hunk_num} succeeded at {applied_line} with fuzz {f} (offset {o} {}).",
                        line_word(o)
                    ),
                };
                // GNU prints every success message when there is any offset
                // or fuzz, regardless of --verbose; verbose adds a line for
                // exact matches too.
                if reported_offset != 0 || fuzz_used > 0 || opts.verbose {
                    println!("{msg}");
                }

                // Track line delta for subsequent hunks.
                let (removed, added) = hunk.lines.iter().fold((0i64, 0i64), |(r, a), l| match l {
                    HunkLine::Context(_) => (r, a),
                    HunkLine::Remove(_) => (r + 1, a),
                    HunkLine::Add(_) => (r, a + 1),
                });
                if opts.reverse {
                    line_delta += removed - added;
                } else {
                    line_delta += added - removed;
                }
            }
            None => {
                if opts.forward {
                    // Check if already applied
                    let reverse_result = apply_hunk(&file_lines, hunk, opts.fuzz, !opts.reverse);
                    if reverse_result.is_some() {
                        if !opts.silent {
                            println!(
                                "Skipping patch -- already applied (hunk #{}).",
                                hunk_idx + 1
                            );
                        }
                        continue;
                    }
                }
                failed_hunks += 1;
                failed_hunk_entries.push(hunk);
                if !opts.silent {
                    println!("Hunk #{} FAILED at {expected_line}.", hunk_idx + 1);
                }
            }
        }
    }

    // Write reject file for any failed hunks.
    if !failed_hunk_entries.is_empty() && !opts.dry_run {
        let reject_path = format!("{target_display}.rej");
        if let Err(e) = write_reject_file(&reject_path, patch, &failed_hunk_entries, opts) {
            eprintln!("patch: can't write rejects to {reject_path}: {e}");
        }
    }

    if failed_hunks > 0 && !opts.silent {
        let hunk_word = if total_hunks == 1 { "hunk" } else { "hunks" };
        let reject_suffix = if opts.dry_run {
            String::new()
        } else {
            format!(" -- saving rejects to file {target_display}.rej")
        };
        println!("{failed_hunks} out of {total_hunks} {hunk_word} FAILED{reject_suffix}");
    }

    // Write output
    if !opts.dry_run {
        let output_path = opts
            .output
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| target.clone());

        // Create backup if requested
        if opts.backup && output_path.exists() && (failed_hunks == 0 || !opts.no_backup_if_mismatch)
        {
            let backup_path = compute_backup_path(&output_path, opts);
            if let Some(parent) = backup_path.parent()
                && !parent.as_os_str().is_empty()
                && !parent.exists()
            {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::copy(&output_path, &backup_path);
        }

        let mut output_content = file_lines.join("\n");
        // Preserve trailing newline when there is content. An empty result
        // is written as a truly-empty file (no stray newline).
        if !output_content.is_empty() && (content.ends_with('\n') || !content.is_empty()) {
            output_content.push('\n');
        }

        // Handle empty file removal
        if opts.remove_empty && output_content.trim().is_empty() {
            let _ = fs::remove_file(&output_path);
        } else {
            // Ensure parent directory exists for new files
            if let Some(parent) = output_path.parent()
                && !parent.exists()
            {
                let _ = fs::create_dir_all(parent);
            }

            fs::write(&output_path, output_content).unwrap_or_else(|e| {
                eprintln!("patch: can't write to {}: {}", output_path.display(), e);
                process::exit(EXIT_TROUBLE);
            });
        }
    }

    if failed_hunks > 0 {
        EXIT_HUNKS_FAILED
    } else {
        EXIT_SUCCESS
    }
}

fn main() {
    let opts = parse_args();

    // Change directory if requested
    if let Some(ref dir) = opts.directory {
        env::set_current_dir(dir).unwrap_or_else(|e| {
            eprintln!("patch: can't change to directory {}: {}", dir, e);
            process::exit(EXIT_TROUBLE);
        });
    }

    let input = read_patch_input(&opts);
    let patches = parse_patches(&input);

    if patches.is_empty() {
        // Distinguish empty input from garbage. GNU uses this specific
        // message when the input contains content but no valid hunks.
        let argv0 = env::args().next().unwrap_or_else(|| "patch".to_string());
        if input.trim().is_empty() {
            // Empty input and no -o output — nothing to do, exit 0.
            if opts.output.is_none() {
                process::exit(EXIT_SUCCESS);
            }
            // With -o on an empty patch, fall through to handle below.
        } else {
            eprintln!("{argv0}: **** Only garbage was found in the patch input.");
            process::exit(EXIT_TROUBLE);
        }
    }

    // Empty patch with -o: copy input file to output.
    if patches.is_empty() && opts.output.is_some() {
        if let Some(src) = &opts.positional_file {
            let dst = opts.output.as_ref().unwrap();
            if !opts.silent {
                println!("patching file {dst} (read from {src})");
            }
            if let Err(e) = fs::copy(src, dst) {
                eprintln!("patch: can't copy {src} to {dst}: {e}");
                process::exit(EXIT_TROUBLE);
            }
        }
        safe_exit_success();
    }

    let mut worst_exit = EXIT_SUCCESS;

    for patch in &patches {
        let exit = apply_file_patch(patch, &opts);
        if exit > worst_exit {
            worst_exit = exit;
        }
    }

    process::exit(worst_exit);
}
