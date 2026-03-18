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
    no_backup_if_mismatch: bool,
    force: bool,
    remove_empty: bool,
    positional_file: Option<String>,
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
            no_backup_if_mismatch: false,
            force: false,
            remove_empty: false,
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
    #[allow(dead_code)]
    new_count: usize,
    lines: Vec<HunkLine>,
}

#[derive(Debug, Clone)]
enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

#[derive(Debug, Clone)]
struct FilePatch {
    old_file: String,
    new_file: String,
    hunks: Vec<Hunk>,
    #[allow(dead_code)]
    format: DiffFormat,
}

fn parse_args() -> Options {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut opts = Options::default();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];

        if arg == "--" {
            i += 1;
            if i < args.len() && opts.positional_file.is_none() {
                opts.positional_file = Some(args[i].clone());
            }
            i += 1;
            continue;
        }

        if arg.starts_with("--strip=") {
            opts.strip = arg["--strip=".len()..].parse().ok();
        } else if arg == "--strip" || arg == "-p" {
            i += 1;
            if i < args.len() {
                opts.strip = args[i].parse().ok();
            }
        } else if arg.starts_with("-p") {
            opts.strip = arg[2..].parse().ok();
        } else if arg.starts_with("--directory=") {
            opts.directory = Some(arg["--directory=".len()..].to_string());
        } else if arg == "--directory" || arg == "-d" {
            i += 1;
            if i < args.len() {
                opts.directory = Some(args[i].clone());
            }
        } else if arg.starts_with("--input=") {
            opts.input = Some(arg["--input=".len()..].to_string());
        } else if arg == "--input" || arg == "-i" {
            i += 1;
            if i < args.len() {
                opts.input = Some(args[i].clone());
            }
        } else if arg.starts_with("--output=") {
            opts.output = Some(arg["--output=".len()..].to_string());
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
        } else if arg.starts_with("--fuzz=") {
            opts.fuzz = arg["--fuzz=".len()..].parse().unwrap_or(2);
        } else if arg == "--fuzz" || arg == "-F" {
            i += 1;
            if i < args.len() {
                opts.fuzz = args[i].parse().unwrap_or(2);
            }
        } else if arg.starts_with("-F") {
            opts.fuzz = arg[2..].parse().unwrap_or(2);
        } else if arg == "-b" || arg == "--backup" {
            opts.backup = true;
        } else if arg == "--no-backup-if-mismatch" {
            opts.no_backup_if_mismatch = true;
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
    // Matches "*** start,end ****" or "--- start,end ----" or single line "*** start ****"
    let re = Regex::new(r"^[\*\-]{3}\s+(\d+)(?:,(\d+))?\s+[\*\-]{4}").unwrap();
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
    for i in start..lines.len() {
        let line = lines[i];
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

    // Skip preamble (diff --git, index, etc.)
    while i < lines.len()
        && !lines[i].starts_with("--- ")
        && !lines[i].starts_with("@@")
    {
        i += 1;
    }
    if i >= lines.len() {
        return None;
    }

    let mut old_file = String::new();
    let mut new_file = String::new();

    // Parse --- and +++ headers
    if lines[i].starts_with("--- ") {
        old_file = parse_file_path(lines[i], "--- ");
        i += 1;
    }
    if i < lines.len() && lines[i].starts_with("+++ ") {
        new_file = parse_file_path(lines[i], "+++ ");
        i += 1;
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
            hunks,
            format: DiffFormat::Unified,
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
            if line.starts_with("! ") {
                old_lines.push(('!', line[2..].to_string()));
            } else if line.starts_with("- ") {
                old_lines.push(('-', line[2..].to_string()));
            } else if line.starts_with("  ") {
                old_lines.push((' ', line[2..].to_string()));
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
            if line.starts_with("! ") {
                new_lines.push(('!', line[2..].to_string()));
            } else if line.starts_with("+ ") {
                new_lines.push(('+', line[2..].to_string()));
            } else if line.starts_with("  ") {
                new_lines.push((' ', line[2..].to_string()));
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
            lines: hunk_lines,
        });
    }

    if hunks.is_empty() {
        return None;
    }

    Some((
        FilePatch {
            old_file,
            new_file,
            hunks,
            format: DiffFormat::Context,
        },
        i,
    ))
}

fn context_to_unified_lines(
    old_lines: &[(char, String)],
    new_lines: &[(char, String)],
) -> Vec<HunkLine> {
    let mut result = Vec::new();
    let mut oi = 0;
    let mut ni = 0;

    while oi < old_lines.len() || ni < new_lines.len() {
        if oi < old_lines.len() && old_lines[oi].0 == ' ' {
            result.push(HunkLine::Context(old_lines[oi].1.clone()));
            oi += 1;
            // Skip matching context in new
            if ni < new_lines.len() && new_lines[ni].0 == ' ' {
                ni += 1;
            }
        } else if oi < old_lines.len() && (old_lines[oi].0 == '!' || old_lines[oi].0 == '-') {
            result.push(HunkLine::Remove(old_lines[oi].1.clone()));
            oi += 1;
        } else if ni < new_lines.len() && (new_lines[ni].0 == '!' || new_lines[ni].0 == '+') {
            result.push(HunkLine::Add(new_lines[ni].1.clone()));
            ni += 1;
        } else if ni < new_lines.len() && new_lines[ni].0 == ' ' {
            result.push(HunkLine::Context(new_lines[ni].1.clone()));
            ni += 1;
        } else {
            oi += 1;
            ni += 1;
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
            old_file,
            new_file,
            hunks,
            format: DiffFormat::Normal,
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
        // Default to new file path (for creating new files)
        new_path
    }
}

fn apply_hunk(
    file_lines: &[String],
    hunk: &Hunk,
    fuzz: usize,
    reverse: bool,
) -> Option<(Vec<String>, usize, usize)> {
    // Build expected old lines and replacement new lines from hunk
    let (remove_lines, _add_lines, context_map) = if reverse {
        extract_reversed_hunk(hunk)
    } else {
        extract_hunk(hunk)
    };

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

    // Try exact match first, then with increasing fuzz/offset
    for fuzz_level in 0..=fuzz {
        let max_offset = if fuzz_level == 0 {
            // On fuzz 0, still try nearby offsets
            file_lines.len()
        } else {
            file_lines.len()
        };

        for offset_mag in 0..=max_offset {
            for &sign in &[1i64, -1i64] {
                if offset_mag == 0 && sign == -1 {
                    continue;
                }
                let offset = offset_mag as i64 * sign;
                let actual_start = target_start as i64 + offset;

                if actual_start < 0 {
                    continue;
                }
                let actual_start = actual_start as usize;

                if try_match(
                    file_lines,
                    actual_start,
                    &remove_lines,
                    &context_map,
                    fuzz_level,
                ) {
                    // Apply the hunk
                    let mut result = Vec::new();
                    result.extend_from_slice(&file_lines[..actual_start]);

                    let mut fi = actual_start;
                    for line in &hunk.lines {
                        match (line, reverse) {
                            (HunkLine::Context(_), _) => {
                                if fi < file_lines.len() {
                                    result.push(file_lines[fi].clone());
                                    fi += 1;
                                }
                            }
                            (HunkLine::Remove(s), false) | (HunkLine::Add(s), true) => {
                                // Skip this line from original
                                if fi < file_lines.len() {
                                    fi += 1;
                                }
                                let _ = s;
                            }
                            (HunkLine::Add(s), false) | (HunkLine::Remove(s), true) => {
                                result.push(s.clone());
                            }
                        }
                    }

                    result.extend_from_slice(&file_lines[fi..]);

                    let applied_offset = if actual_start >= target_start {
                        actual_start - target_start
                    } else {
                        target_start - actual_start
                    };
                    return Some((result, fuzz_level, applied_offset));
                }
            }
        }
    }
    None
}

fn extract_hunk(hunk: &Hunk) -> (Vec<String>, Vec<String>, Vec<(usize, String)>) {
    let mut remove = Vec::new();
    let mut add = Vec::new();
    let mut context = Vec::new();
    let mut pos = 0;

    for line in &hunk.lines {
        match line {
            HunkLine::Context(s) => {
                context.push((pos, s.clone()));
                remove.push(s.clone());
                pos += 1;
            }
            HunkLine::Remove(s) => {
                remove.push(s.clone());
                pos += 1;
            }
            HunkLine::Add(s) => {
                add.push(s.clone());
            }
        }
    }
    (remove, add, context)
}

fn extract_reversed_hunk(hunk: &Hunk) -> (Vec<String>, Vec<String>, Vec<(usize, String)>) {
    let mut remove = Vec::new();
    let mut add = Vec::new();
    let mut context = Vec::new();
    let mut pos = 0;

    for line in &hunk.lines {
        match line {
            HunkLine::Context(s) => {
                context.push((pos, s.clone()));
                remove.push(s.clone());
                pos += 1;
            }
            HunkLine::Add(s) => {
                remove.push(s.clone());
                pos += 1;
            }
            HunkLine::Remove(s) => {
                add.push(s.clone());
            }
        }
    }
    (remove, add, context)
}

fn try_match(
    file_lines: &[String],
    start: usize,
    expected_old: &[String],
    context_lines: &[(usize, String)],
    fuzz: usize,
) -> bool {
    if expected_old.is_empty() {
        // Pure addition — start must be valid insertion point
        return start <= file_lines.len();
    }

    if start + expected_old.len() > file_lines.len() {
        return false;
    }

    if fuzz == 0 {
        // Exact match
        for (i, expected) in expected_old.iter().enumerate() {
            if file_lines[start + i] != *expected {
                return false;
            }
        }
        true
    } else {
        // With fuzz: skip first/last `fuzz` context lines
        let total_context = context_lines.len();
        let skip = fuzz.min(total_context);

        // Check all non-context (remove) lines match exactly
        let context_positions: std::collections::HashSet<usize> =
            context_lines.iter().map(|(p, _)| *p).collect();

        for (i, expected) in expected_old.iter().enumerate() {
            if context_positions.contains(&i) {
                // This is a context line — check if it's within fuzz skip range
                let ctx_idx = context_lines.iter().position(|(p, _)| *p == i).unwrap();
                if ctx_idx < skip || ctx_idx >= total_context - skip {
                    continue; // Skip fuzzed context lines
                }
            }
            if start + i >= file_lines.len() || file_lines[start + i] != *expected {
                return false;
            }
        }
        true
    }
}

fn apply_file_patch(patch: &FilePatch, opts: &Options) -> i32 {
    let target = resolve_target_file(patch, opts);
    let target_display = target.display().to_string();

    if !opts.silent {
        eprintln!(
            "patching file {}",
            target_display
        );
    }

    // Read existing file content (or empty for new files)
    let content = if target.exists() {
        fs::read_to_string(&target).unwrap_or_else(|e| {
            eprintln!("patch: can't read {}: {}", target_display, e);
            process::exit(EXIT_TROUBLE);
        })
    } else {
        // Check if this is creating a new file
        if patch.old_file == "/dev/null"
            || patch.old_file.is_empty()
            || patch.hunks.iter().all(|h| h.old_count == 0)
        {
            String::new()
        } else {
            eprintln!(
                "patch: can't open file {}: No such file or directory",
                target_display
            );
            return EXIT_TROUBLE;
        }
    };

    let mut file_lines: Vec<String> = if content.is_empty() {
        Vec::new()
    } else {
        content.lines().map(|l| l.to_string()).collect()
    };

    let mut failed_hunks = 0;
    let total_hunks = patch.hunks.len();

    for (hunk_idx, hunk) in patch.hunks.iter().enumerate() {
        match apply_hunk(&file_lines, hunk, opts.fuzz, opts.reverse) {
            Some((new_lines, fuzz_used, offset)) => {
                file_lines = new_lines;
                if opts.verbose {
                    let hunk_num = hunk_idx + 1;
                    if fuzz_used > 0 || offset > 0 {
                        eprintln!(
                            "Hunk #{} succeeded at offset {} (fuzz {}).",
                            hunk_num, offset, fuzz_used
                        );
                    } else {
                        eprintln!("Hunk #{} succeeded.", hunk_num);
                    }
                }
            }
            None => {
                if opts.forward {
                    // Check if already applied
                    let reverse_result =
                        apply_hunk(&file_lines, hunk, opts.fuzz, !opts.reverse);
                    if reverse_result.is_some() {
                        if !opts.silent {
                            eprintln!(
                                "Skipping patch -- already applied (hunk #{}).",
                                hunk_idx + 1
                            );
                        }
                        continue;
                    }
                }
                failed_hunks += 1;
                if !opts.silent {
                    eprintln!(
                        "Hunk #{} FAILED at {}.",
                        hunk_idx + 1,
                        if opts.reverse {
                            hunk.new_start
                        } else {
                            hunk.old_start
                        }
                    );
                }
            }
        }
    }

    if failed_hunks > 0 && !opts.silent {
        eprintln!(
            "{} out of {} hunk{} FAILED",
            failed_hunks,
            total_hunks,
            if total_hunks == 1 { "" } else { "s" }
        );
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
            let backup_path = format!("{}.orig", output_path.display());
            let _ = fs::copy(&output_path, &backup_path);
        }

        let mut output_content = file_lines.join("\n");
        // Preserve trailing newline if original had one
        if content.ends_with('\n') || !content.is_empty() {
            output_content.push('\n');
        }

        // Handle empty file removal
        if opts.remove_empty && output_content.trim().is_empty() {
            let _ = fs::remove_file(&output_path);
        } else {
            // Ensure parent directory exists for new files
            if let Some(parent) = output_path.parent() {
                if !parent.exists() {
                    let _ = fs::create_dir_all(parent);
                }
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
        eprintln!("patch: no valid patches found in input");
        process::exit(EXIT_TROUBLE);
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
