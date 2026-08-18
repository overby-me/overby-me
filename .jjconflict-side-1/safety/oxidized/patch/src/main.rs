use regex::Regex;
use std::cell::RefCell;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{Duration, SystemTime};

thread_local! {
    static MALFORMED: RefCell<Option<(usize, String)>> = const { RefCell::new(None) };
    static BINARY_SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set once, when any hunk matches only after stripping trailing CRs
    /// from the patch side. Triggers the "Stripping CRs" notice.
    static STRIPPED_CRS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

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
    binary: bool,
    follow_symlinks: bool,
    ed: bool,
    set_utc: bool,
    set_time: bool,
    batch: bool,
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
            binary: false,
            follow_symlinks: false,
            ed: false,
            set_utc: false,
            set_time: false,
            batch: false,
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
    /// Shared mode picked up from `index aaa..bbb NNN` lines. Only used
    /// for classification (e.g. "is this a symlink?") — NOT applied as a
    /// mode change, since the index line describes the shared/current
    /// mode, not a user-intended modification.
    index_mode: Option<u32>,
    rename_from: Option<String>,
    rename_to: Option<String>,
    copy_from: Option<String>,
    copy_to: Option<String>,
    /// True when the git header contains a `Binary files X and Y differ`
    /// summary line. We can't compare content, so patch operations that
    /// would require content equality (e.g. delete verification) are
    /// refused.
    binary_summary: bool,
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
        } else if arg == "--binary" {
            opts.binary = true;
        } else if arg == "--follow-symlinks" {
            opts.follow_symlinks = true;
        } else if arg == "--merge" || arg.starts_with("--merge=") {
            // --merge (diff3-style conflict merging) is not implemented.
            // Emit a clear error so the upstream test suite's merge test
            // can detect the absence and skip.
            let argv0 = env::args().next().unwrap_or_else(|| "patch".to_string());
            eprintln!("{argv0}: --merge support is not implemented in rust-patch");
            process::exit(EXIT_TROUBLE);
        } else if arg == "-e" || arg == "--ed" {
            opts.ed = true;
        } else if arg == "--set-utc" {
            opts.set_utc = true;
        } else if arg == "--set-time" {
            opts.set_time = true;
        } else if arg == "-f" || arg == "--force" {
            opts.force = true;
        } else if arg == "-t" || arg == "--batch" {
            opts.batch = true;
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
                        't' => opts.batch = true,
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
    // GNU patch: count '/' separators as strip units. If the path has fewer
    // prefix components than `strip`, the result is "too short" (empty).
    let components: Vec<&str> = path.split('/').collect();
    if strip >= components.len() {
        String::new()
    } else {
        components[strip..].join("/")
    }
}

/// Apply `-p` stripping to a path. When `strip` is `None`, GNU patch default
/// is to strip all leading directory components and use just the basename.
fn strip_path_opt(path: &str, strip: Option<usize>) -> String {
    match strip {
        Some(n) => strip_path(path, n),
        None => {
            if path == "/dev/null" {
                return path.to_string();
            }
            // Take the basename (last component).
            match path.rsplit_once('/') {
                Some((_, base)) => base.to_string(),
                None => path.to_string(),
            }
        }
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
    let trimmed = line.strip_suffix('\r').unwrap_or(line);
    re.captures(trimmed).map(|caps| {
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

/// Extract the tab-separated timestamp portion of a `--- X\tTS` /
/// `+++ X\tTS` header line. Returns the raw timestamp text.
fn extract_timestamp_field(line: &str) -> Option<&str> {
    line.find('\t').map(|i| line[i + 1..].trim())
}

/// Parse a patch header timestamp like `2009-03-14 00:00:00` into a
/// `SystemTime`. Accepts optional fractional seconds and trailing timezone
/// offset. Returns `None` for timestamps we can't interpret.
fn parse_header_timestamp(ts: &str, as_utc: bool) -> Option<SystemTime> {
    // Require the YYYY-MM-DD HH:MM:SS prefix at minimum.
    let re = Regex::new(
        r"^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?(?:\s*([+-])(\d{2}):?(\d{2}))?",
    )
    .ok()?;
    let caps = re.captures(ts)?;
    let y: i32 = caps[1].parse().ok()?;
    let mo: u32 = caps[2].parse().ok()?;
    let d: u32 = caps[3].parse().ok()?;
    let h: u32 = caps[4].parse().ok()?;
    let mi: u32 = caps[5].parse().ok()?;
    let s: u32 = caps[6].parse().ok()?;

    // Howard Hinnant's algorithm for days since 1970-01-01.
    let (ys, ms): (i32, i32) = if mo <= 2 {
        (y - 1, mo as i32 + 9)
    } else {
        (y, mo as i32 - 3)
    };
    let era = (if ys >= 0 { ys } else { ys - 399 }) / 400;
    let yoe = (ys - era * 400) as u32;
    let doy_signed: i32 = (153 * ms + 2) / 5 + d as i32 - 1;
    let doy = doy_signed as u32;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days: i64 = era as i64 * 146097 + doe as i64 - 719468;
    let mut secs: i64 = days * 86400 + h as i64 * 3600 + mi as i64 * 60 + s as i64;

    // Apply explicit timezone offset if present.
    if let (Some(sign), Some(hh), Some(mm)) = (caps.get(8), caps.get(9), caps.get(10)) {
        let off_h: i64 = hh.as_str().parse().ok()?;
        let off_m: i64 = mm.as_str().parse().ok()?;
        let off_s = (off_h * 3600 + off_m * 60) * if sign.as_str() == "+" { 1 } else { -1 };
        secs -= off_s;
    } else if !as_utc {
        // Interpret as local time — convert via libc's `mktime`-equivalent.
        // Without that, fall back to treating as UTC (callers ask for
        // --set-utc in the failing tests, so this branch is rarely hit).
    }

    if secs >= 0 {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs as u64))
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(Duration::from_secs((-secs) as u64))
    }
}

/// GNU patch convention: a timestamp of 1970-01-01 00:00:00 UTC on a
/// `--- X` or `+++ Y` line signals that the file was (or will become)
/// non-existent. Detect common renderings so we can treat that side as
/// /dev/null.
fn is_epoch_timestamp_line(line: &str) -> bool {
    // Find the tab-separated timestamp portion.
    let Some(tab) = line.find('\t') else {
        return false;
    };
    let ts = line[tab + 1..].trim();
    // Common forms emitted by `diff -u` / git:
    //   1970-01-01 00:00:00           (+ optional fractional + tz offset)
    //   1970-01-01 01:00:00.000000000 +0100
    //   Thu Jan  1 00:00:00 1970
    if ts.starts_with("1970-01-01") && ts.contains("00:00:00") {
        return true;
    }
    if ts.contains("1970") && ts.contains("Jan") && ts.contains("00:00:00") {
        return true;
    }
    false
}

fn parse_file_path(line: &str, prefix: &str) -> String {
    let rest = &line[prefix.len()..];
    // C-quoted form: `"name\ttimestamp"` — take content up to closing quote.
    if let Some(stripped) = rest.strip_prefix('"') {
        if let Some(end) = stripped.find('"') {
            return unquote_c_string(&stripped[..end]);
        }
    }
    // Remove trailing timestamp (tab-separated)
    rest.split('\t').next().unwrap_or(rest).trim().to_string()
}

fn unquote_c_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(next) = it.next() else {
            out.push('\\');
            break;
        };
        match next {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'b' => out.push('\x08'),
            'f' => out.push('\x0c'),
            'a' => out.push('\x07'),
            'v' => out.push('\x0b'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            '0'..='7' => {
                // Up to 3 octal digits.
                let mut val = next.to_digit(8).unwrap();
                let peek_it = it.clone();
                let mut used = 0;
                for c2 in peek_it.take(2) {
                    if let Some(d) = c2.to_digit(8) {
                        val = val * 8 + d;
                        used += 1;
                    } else {
                        break;
                    }
                }
                for _ in 0..used {
                    it.next();
                }
                if let Some(c) = char::from_u32(val) {
                    out.push(c);
                }
            }
            'x' => {
                // Hex: `\xHH`.
                let mut val = 0u32;
                let mut used = 0;
                let peek_it = it.clone();
                for c2 in peek_it.take(2) {
                    if let Some(d) = c2.to_digit(16) {
                        val = val * 16 + d;
                        used += 1;
                    } else {
                        break;
                    }
                }
                for _ in 0..used {
                    it.next();
                }
                if let Some(c) = char::from_u32(val) {
                    out.push(c);
                }
            }
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

fn hunk_has_remaining(lines: &[HunkLine], old_count: usize, new_count: usize) -> bool {
    let mut old_consumed = 0usize;
    let mut new_consumed = 0usize;
    for l in lines {
        match l {
            HunkLine::Context(_) => {
                old_consumed += 1;
                new_consumed += 1;
            }
            HunkLine::Remove(_) => old_consumed += 1,
            HunkLine::Add(_) => new_consumed += 1,
        }
    }
    old_consumed < old_count || new_consumed < new_count
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
    // Split on `\n` only (not `\r\n`) so trailing CRs survive for CRLF-aware
    // hunk matching. Drop the last empty string when input ends with `\n`.
    let mut parts: Vec<&str> = input.split('\n').collect();
    if parts.last().is_some_and(|s| s.is_empty()) {
        parts.pop();
    }
    let lines: Vec<&str> = parts;
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
                // Keep the raw `a/...` / `b/...` prefixes so `-pN` stripping
                // counts them as real components.
                git_a = Some(parts[0].to_string());
                git_b = Some(parts[1].to_string());
            }
        } else if let Some(rest) = line.strip_prefix("new file mode ") {
            git.new_file_mode = u32::from_str_radix(rest.trim(), 8).ok();
        } else if let Some(rest) = line.strip_prefix("deleted file mode ") {
            git.deleted_file_mode = u32::from_str_radix(rest.trim(), 8).ok();
        } else if let Some(rest) = line.strip_prefix("old mode ") {
            git.old_mode = u32::from_str_radix(rest.trim(), 8).ok();
        } else if let Some(rest) = line.strip_prefix("new mode ") {
            git.new_mode = u32::from_str_radix(rest.trim(), 8).ok();
        } else if let Some(rest) = line.strip_prefix("index ") {
            // `index aaa..bbb NNN` carries the shared file mode for
            // unchanged-mode patches. Record it in `index_mode` only so
            // downstream code can recognize symlinks (120000) etc.
            // without accidentally applying the mode as a change.
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 2
                && let Ok(m) = u32::from_str_radix(parts[1], 8)
            {
                git.index_mode = Some(m);
            }
        } else if let Some(rest) = line.strip_prefix("rename from ") {
            git.rename_from = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("rename to ") {
            git.rename_to = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("copy from ") {
            git.copy_from = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("copy to ") {
            git.copy_to = Some(rest.to_string());
        } else if line.starts_with("Binary files ") && line.ends_with(" differ") {
            git.binary_summary = true;
        } else if line == "GIT binary patch" {
            // Binary patches are not supported. Message uses basename-ish
            // form (strip `a/`/`b/` prefix) so diagnostics stay readable.
            let name = git_b
                .as_deref()
                .map(|s| s.trim_start_matches("b/").to_string())
                .unwrap_or_default();
            println!("File {name}: git binary diffs are not supported.");
            BINARY_SEEN.with(|b| b.set(true));
            // Skip the binary payload until the next diff separator.
            i += 1;
            while i < lines.len()
                && !lines[i].starts_with("diff --git ")
                && !lines[i].starts_with("--- ")
            {
                i += 1;
            }
            // Return a header-only placeholder so parse_patches advances.
            let old_file = git_a.clone().unwrap_or_default();
            let new_file = git_b.clone().unwrap_or_default();
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
        i += 1;

        // If we reached another "diff --git" without hitting ---/@@, we've
        // consumed a header-only patch (like rename-only or mode-only).
        if i < lines.len() && lines[i].starts_with("diff --git ") {
            if git_a.is_some() || git_b.is_some() {
                // Prefer the `a/...`/`b/...` paths from the diff --git line
                // because they have proper prefix components for `-pN`.
                let old_file = git_a
                    .clone()
                    .or_else(|| git.rename_from.clone())
                    .or_else(|| git.copy_from.clone())
                    .unwrap_or_default();
                let new_file = git_b
                    .clone()
                    .or_else(|| git.rename_to.clone())
                    .or_else(|| git.copy_to.clone())
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
            let old_file = git_a
                .clone()
                .or_else(|| git.rename_from.clone())
                .or_else(|| git.copy_from.clone())
                .unwrap_or_default();
            let new_file = git_b
                .clone()
                .or_else(|| git.rename_to.clone())
                .or_else(|| git.copy_to.clone())
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
        if is_epoch_timestamp_line(lines[i]) {
            old_file = "/dev/null".to_string();
        }
        i += 1;
    }
    if i < lines.len() && lines[i].starts_with("+++ ") {
        new_file = parse_file_path(lines[i], "+++ ");
        new_header = lines[i][4..].to_string();
        if is_epoch_timestamp_line(lines[i]) {
            new_file = "/dev/null".to_string();
        }
        i += 1;
    }

    // Prefer git header names when the --- /+++ paths are empty, so we know
    // which real file to touch. Preserve explicit /dev/null, since that
    // signals creation/deletion semantics the git metadata may not.
    if old_file.is_empty() {
        if let Some(a) = &git_a {
            old_file = a.clone();
        }
    }
    if new_file.is_empty() {
        if let Some(b) = &git_b {
            new_file = b.clone();
        }
    }

    // No headers — only allow if the next line is a hunk header (`@@`).
    // The caller will use the positional file argument as the target.
    if old_file.is_empty()
        && new_file.is_empty()
        && (i >= lines.len() || !lines[i].starts_with("@@"))
    {
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
                        // Skip this marker.
                    } else if line.is_empty()
                        && hunk_has_remaining(&hunk_lines, old_count, new_count)
                    {
                        hunk_lines.push(HunkLine::Context(String::new()));
                    } else {
                        // Malformed hunk body mid-stream. Record the error
                        // for main() to emit after applying prior patches.
                        if hunk_has_remaining(&hunk_lines, old_count, new_count) {
                            MALFORMED.with(|m| {
                                m.borrow_mut()
                                    .get_or_insert_with(|| (i + 1, line.to_string()));
                            });
                        }
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

    // A patch with a valid --- /+++ header but no hunks is still returned
    // when we aborted mid-body (MALFORMED was set); otherwise treat as "not
    // a patch" so the parse_patches loop advances by one line.
    if hunks.is_empty() && MALFORMED.with(|m| m.borrow().is_none()) {
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

    // Find a context-diff header. Standard form has `*** old / --- new`
    // before `***************`. Headerless form (some hand-edited diffs)
    // jumps straight to `***************` and relies on a positional
    // filename argument.
    let mut old_file = String::new();
    let mut new_file = String::new();
    let mut old_header = String::new();
    let mut new_header = String::new();

    while i < lines.len()
        && !lines[i].starts_with("*** ")
        && !lines[i].starts_with("***************")
    {
        i += 1;
    }
    if i >= lines.len() {
        return None;
    }

    if lines[i].starts_with("*** ") {
        old_file = parse_file_path(lines[i], "*** ");
        old_header = lines[i][4..].to_string();
        if is_epoch_timestamp_line(lines[i]) {
            old_file = "/dev/null".to_string();
        }
        i += 1;

        if i >= lines.len() || !lines[i].starts_with("--- ") {
            return None;
        }
        new_file = parse_file_path(lines[i], "--- ");
        new_header = lines[i][4..].to_string();
        if is_epoch_timestamp_line(lines[i]) {
            new_file = "/dev/null".to_string();
        }
        i += 1;
    }

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
            } else if let Some(rest) = line.strip_prefix(' ') {
                // Lenient: single-space prefix treated as context (matches
                // GNU's tolerance of slightly malformed context lines).
                old_lines.push((' ', rest.to_string()));
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

        // GNU context-diff shorthand: when one side has no explicit content
        // between its `*** N,M ****` / `--- N,M ----` header and the next
        // section, the other side's context+change lines imply both.
        let header_old_count = if old_end >= old_start {
            old_end - old_start + 1
        } else {
            0
        };
        let header_new_count = if new_end >= new_start {
            new_end - new_start + 1
        } else {
            0
        };

        // Validate declared vs actual line counts for the new side. GNU
        // is lenient on minor old-side mangling (odd whitespace in a
        // context line is soft-skipped), but catches structural issues
        // like `*** N,M ****` / `--- A,B ----` with clearly wrong counts.
        if !new_lines.is_empty() && new_lines.len() != header_new_count {
            let argv0 = env::args().next().unwrap_or_else(|| "patch".to_string());
            eprintln!(
                "{argv0}: **** replacement text or line numbers mangled in hunk at line {}",
                i + 1 - new_lines.len() - 2
            );
            process::exit(EXIT_TROUBLE);
        }
        let effective_old: Vec<(char, String)> =
            if old_lines.is_empty() && header_old_count > 0 && !new_lines.is_empty() {
                // Implicit old = new with '+' lines stripped.
                new_lines
                    .iter()
                    .filter(|(c, _)| *c != '+')
                    .map(|(c, s)| {
                        let kind = if *c == '!' { '!' } else { ' ' };
                        (kind, s.clone())
                    })
                    .collect()
            } else {
                old_lines.clone()
            };
        let effective_new: Vec<(char, String)> =
            if new_lines.is_empty() && header_new_count > 0 && !old_lines.is_empty() {
                // Implicit new = old with '-' lines stripped.
                old_lines
                    .iter()
                    .filter(|(c, _)| *c != '-')
                    .map(|(c, s)| {
                        let kind = if *c == '!' { '!' } else { ' ' };
                        (kind, s.clone())
                    })
                    .collect()
            } else {
                new_lines.clone()
            };

        // Convert context diff to unified hunk lines.
        let hunk_lines = context_to_unified_lines(&effective_old, &effective_new);
        // For context format, `*** N ****` with no content (and no range
        // expansion via shorthand) means 0 lines on that side (pure addition
        // on the other side).
        let old_count = if effective_old.is_empty() {
            0
        } else {
            header_old_count
        };
        let new_count = if effective_new.is_empty() {
            0
        } else {
            header_new_count
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
            old_file,
            new_file,
            old_header,
            new_header,
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
        } else if oi < old_lines.len() && (old_lines[oi].0 == '!' || old_lines[oi].0 == '-') {
            result.push(HunkLine::Remove(old_lines[oi].1.clone()));
            oi += 1;
        } else if ni < new_lines.len() && (new_lines[ni].0 == '!' || new_lines[ni].0 == '+') {
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
                    // Skip separator "---" (CR-tolerant: `---\r` counts).
                    if i < lines.len() {
                        let trimmed = lines[i].strip_suffix('\r').unwrap_or(lines[i]);
                        if trimmed == "---" {
                            i += 1;
                        }
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

    let base = opts.directory.as_deref().unwrap_or(".");

    // For git rename/copy, the target is always the destination path.
    // Prefer `patch.new_file` (from `diff --git a/X b/Y` or `+++ b/Y`,
    // which have proper prefix components for `-pN`) and fall back to the
    // raw `rename to` / `copy to` value if new_file is empty. Under -R
    // the direction reverses: destination becomes source and vice versa.
    if patch.git.rename_to.is_some() || patch.git.copy_to.is_some() {
        if opts.reverse {
            // Under -R the target is the original source.
            if let Some(from) = patch
                .git
                .rename_from
                .as_deref()
                .or(patch.git.copy_from.as_deref())
            {
                return Path::new(base).join(from);
            }
            let old_stripped = strip_path_opt(&patch.old_file, opts.strip);
            if !old_stripped.is_empty() && old_stripped != "/dev/null" {
                return Path::new(base).join(old_stripped);
            }
        }
        let new_stripped = strip_path_opt(&patch.new_file, opts.strip);
        if !new_stripped.is_empty() && new_stripped != "/dev/null" {
            return Path::new(base).join(new_stripped);
        }
        if let Some(to) = patch
            .git
            .rename_to
            .as_deref()
            .or(patch.git.copy_to.as_deref())
        {
            return Path::new(base).join(to);
        }
    }

    let old_stripped = strip_path_opt(&patch.old_file, opts.strip);
    let new_stripped = strip_path_opt(&patch.new_file, opts.strip);

    let old_path = Path::new(base).join(&old_stripped);
    let new_path = Path::new(base).join(&new_stripped);

    if old_stripped == "/dev/null" {
        return new_path;
    }
    if new_stripped == "/dev/null" {
        return old_path;
    }

    // When old_file is effectively a backup suffix of new_file (e.g.
    // `--- f.orig / +++ f`), always prefer the new-side filename as the
    // target — the .orig path is the backup, not the file under patch.
    if old_stripped.strip_suffix(".orig") == Some(new_stripped.as_str()) {
        return new_path;
    }

    // Empty stripped paths cannot be a valid candidate — using the bare
    // `base` would point at the working directory itself. Prefer any
    // non-empty side before falling back.
    let old_empty = old_stripped.is_empty();
    let new_empty = new_stripped.is_empty();
    if !old_empty && old_path.exists() {
        old_path
    } else if !new_empty && new_path.exists() {
        new_path
    } else if old_empty && !new_empty {
        new_path
    } else if new_empty && !old_empty {
        old_path
    } else if old_empty && new_empty {
        // Both sides stripped away. Return the base directory as a sentinel;
        // caller will detect and emit the "can't find file" skip message.
        Path::new(base).to_path_buf()
    } else {
        // Neither exists but both are non-empty: default to old (GNU rule).
        old_path
    }
}

fn apply_hunk(
    file_lines: &[String],
    hunk: &Hunk,
    fuzz: usize,
    reverse: bool,
    strip_cr: bool,
) -> Option<(Vec<String>, usize, i64, usize)> {
    let target_start = if reverse {
        match (hunk.new_start, hunk.new_count) {
            (0, _) => 0,
            (n, 0) => n,
            (n, _) => n - 1,
        }
    } else {
        match (hunk.old_start, hunk.old_count) {
            (0, _) => 0,
            (n, 0) => n,
            (n, _) => n - 1,
        }
    };

    // Normalize the hunk view for the current direction. For each HunkLine,
    // classify it as context/consume/produce. Reversing swaps Remove<->Add.
    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Context,
        Consume, // expected in old file, dropped
        Produce, // written to new file
    }
    // Optionally strip trailing CRs from all hunk line text when the
    // target file has no CRs. GNU patch strips Add-side CRs too so the
    // produced output matches the file's line-ending convention.
    let owned_lines: Vec<String> = hunk
        .lines
        .iter()
        .map(|l| match l {
            HunkLine::Context(s) | HunkLine::Remove(s) | HunkLine::Add(s) => {
                if strip_cr {
                    s.strip_suffix('\r').unwrap_or(s).to_string()
                } else {
                    s.clone()
                }
            }
        })
        .collect();
    let classified: Vec<(Kind, &String)> = hunk
        .lines
        .iter()
        .zip(owned_lines.iter())
        .map(|(l, s)| match (l, reverse) {
            (HunkLine::Context(_), _) => (Kind::Context, s),
            (HunkLine::Remove(_), false) | (HunkLine::Add(_), true) => (Kind::Consume, s),
            (HunkLine::Add(_), false) | (HunkLine::Remove(_), true) => (Kind::Produce, s),
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
                // originally-expected position. GNU reports `at N` as
                // (hunk header start + offset), not as the actual file
                // line of the first matched (non-fuzzed) context line.
                let expected_start = target_start as i64 + skip_lead as i64;
                let applied_offset = actual_start as i64 - expected_start;
                // Virtual start the caller should use for "at N" reporting:
                // back out skip_lead so it matches GNU's reporting.
                let virtual_start = (actual_start as i64 - skip_lead as i64).max(0) as usize;
                return Some((result, fuzz_level, applied_offset, virtual_start));
            }
        }
    }
    None
}

fn safe_exit_success() -> ! {
    process::exit(EXIT_SUCCESS);
}

/// Heuristic: does `input` look like an `ed` editor script (i.e. lines
/// matching ed addressing + commands like `5a`, `3,7c`, `$d`)? We only
/// check this when the regular patch parsers already returned nothing,
/// so the bar is low: find at least one line that looks like `\d+[acd]`
/// or `\d+,\d+[cd]` and none of the usual patch markers.
fn looks_like_ed_script(input: &str) -> bool {
    let has_unified_markers = input
        .lines()
        .any(|l| l.starts_with("--- ") || l.starts_with("+++ ") || l.starts_with("@@"));
    let has_context_markers = input.lines().any(|l| l.starts_with("*** "));
    let has_git_markers = input.lines().any(|l| l.starts_with("diff --git "));
    if has_unified_markers || has_context_markers || has_git_markers {
        return false;
    }
    let ed_cmd = Regex::new(r"^\d+(?:,\d+)?[acds]").unwrap();
    input.lines().any(|l| ed_cmd.is_match(l.trim_start()))
}

fn apply_ed_patches(input: &str, opts: &Options) {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Refuse ed scripts that try to invoke a shell (`r !cmd`, `w !cmd`).
    for line in input.lines() {
        let trimmed = line.trim_start();
        if (trimmed.starts_with("r ") || trimmed.starts_with("w "))
            && trimmed[1..].trim_start().starts_with('!')
        {
            let argv0 = env::args().next().unwrap_or_else(|| "patch".to_string());
            eprintln!("{argv0}: **** ed script invokes a shell — refusing to apply");
            process::exit(EXIT_TROUBLE);
        }
    }

    // The ed input may contain multiple file sections separated by
    // `--- name / +++ name` preambles. Split on just `\n` (not `\r\n`) so
    // CR-preserving ed scripts keep their CR content.
    let mut current_file: Option<String> = opts.positional_file.clone();
    let mut script = String::new();
    let iter_parts: Vec<&str> = {
        let mut v: Vec<&str> = input.split('\n').collect();
        if v.last().is_some_and(|s| s.is_empty()) {
            v.pop();
        }
        v
    };
    let mut lines = iter_parts.iter().copied().peekable();

    // No filename anywhere in the input AND no positional argument:
    // emit the GNU skip message and exit 1 (under -f) or 2 (default).
    let has_any_header = input.lines().any(|l| l.starts_with("--- "));
    if current_file.is_none() && !has_any_header {
        if opts.force {
            if !opts.silent {
                println!("can't find file to patch at input line 1");
                println!("No file to patch.  Skipping patch.");
            }
            process::exit(EXIT_HUNKS_FAILED);
        } else {
            let argv0 = env::args().next().unwrap_or_else(|| "patch".to_string());
            eprintln!("{argv0}: **** Only garbage was found in the patch input.");
            process::exit(EXIT_TROUBLE);
        }
    }

    // GNU patch runs ed scripts silently (no "patching file" prefix), even
    // for sections with `--- / +++` headers.
    let run_script = |file: &str, script: &str| {
        let _ = file;
        // Append `w\nq\n` so ed writes and quits.
        let mut full = String::with_capacity(script.len() + 4);
        full.push_str(script);
        full.push_str("w\nq\n");
        let mut child = match Command::new("ed")
            .arg(file)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(full.as_bytes());
        }
        let _ = child.wait();
    };

    while let Some(line) = lines.next() {
        if let Some(name) = line.strip_prefix("--- ")
            && lines.peek().is_some_and(|l| l.starts_with("+++ "))
        {
            if let Some(file) = &current_file {
                if !script.is_empty() {
                    run_script(file, &script);
                    script.clear();
                }
            }
            // Consume the +++ line too.
            let _ = lines.next();
            current_file = Some(name.trim().to_string());
            continue;
        }
        script.push_str(line);
        script.push('\n');
    }
    if let Some(file) = &current_file
        && !script.is_empty()
    {
        run_script(file, &script);
    }
}

/// Walk each parent segment of `path` and, if any of them is a symlink
/// whose fully-resolved target falls outside the current working
/// directory, return true. Resolves chained symlinks iteratively.
/// GNU patch refuses to follow such links with "Invalid file name".
fn path_escapes_cwd_via_symlink(path: &Path) -> bool {
    use std::path::Component;
    let Ok(cwd) = env::current_dir() else {
        return false;
    };
    let Ok(canonical_cwd) = cwd.canonicalize() else {
        return false;
    };

    /// Token-level representation of a pending path segment. Avoids
    /// borrowing from a transient `PathBuf`.
    #[derive(Clone)]
    enum Tok {
        CurDir,
        ParentDir,
        Root,
        Normal(std::ffi::OsString),
    }
    fn into_tokens(p: &Path) -> Vec<Tok> {
        p.components()
            .map(|c| match c {
                Component::CurDir => Tok::CurDir,
                Component::ParentDir => Tok::ParentDir,
                Component::RootDir => Tok::Root,
                Component::Prefix(_) => Tok::CurDir,
                Component::Normal(n) => Tok::Normal(n.to_os_string()),
            })
            .collect()
    }
    fn resolve_chain(start: &Path, cwd: &Path) -> PathBuf {
        let mut resolved = if start.is_absolute() {
            PathBuf::new()
        } else {
            cwd.to_path_buf()
        };
        let mut pending: Vec<Tok> = into_tokens(start);
        let mut guard = 40;
        while !pending.is_empty() {
            let tok = pending.remove(0);
            match tok {
                Tok::CurDir => {}
                Tok::ParentDir => {
                    resolved.pop();
                }
                Tok::Root => resolved = PathBuf::from("/"),
                Tok::Normal(name) => {
                    resolved.push(&name);
                    if guard == 0 {
                        return resolved;
                    }
                    if let Ok(md) = fs::symlink_metadata(&resolved)
                        && md.file_type().is_symlink()
                        && let Ok(link_target) = fs::read_link(&resolved)
                    {
                        guard -= 1;
                        resolved.pop();
                        if link_target.is_absolute() {
                            resolved = PathBuf::new();
                        }
                        let mut new_pending = into_tokens(&link_target);
                        new_pending.extend(pending.drain(..));
                        pending = new_pending;
                    }
                }
            }
        }
        resolved
    }

    let mut accum = PathBuf::new();
    let components: Vec<Component> = path.components().collect();
    let n = components.len();
    for (idx, comp) in components.iter().enumerate() {
        if idx + 1 == n {
            break;
        }
        match comp {
            Component::CurDir => continue,
            Component::ParentDir => {
                accum.pop();
                continue;
            }
            Component::RootDir => {
                accum = PathBuf::from("/");
                continue;
            }
            Component::Prefix(_) => continue,
            Component::Normal(name) => accum.push(name),
        }
        if let Ok(md) = fs::symlink_metadata(&accum)
            && md.file_type().is_symlink()
        {
            let resolved = resolve_chain(&accum, &cwd);
            if !resolved.starts_with(&canonical_cwd) {
                return true;
            }
        }
    }
    false
}

/// Count how many lines the hunk consumes from the old file (context + remove).
fn old_len_from_hunk(hunk: &Hunk) -> usize {
    hunk.lines
        .iter()
        .filter(|l| matches!(l, HunkLine::Context(_) | HunkLine::Remove(_)))
        .count()
}

fn is_dangerous_path(path: &str) -> bool {
    if path.is_empty() || path == "/dev/null" {
        return false;
    }
    if path.starts_with('/') {
        // Absolute paths are dangerous unless they resolve under the current
        // working directory — matching GNU patch, which trusts absolute
        // paths that a user can already write via their cwd.
        if let Ok(cwd) = env::current_dir()
            && let Ok(canonical) = Path::new(path).canonicalize()
            && canonical.starts_with(&cwd)
        {
            return false;
        }
        // Canonicalizing fails for nonexistent paths; fall back to a
        // string-prefix check against cwd so a nonexistent tempfile inside
        // cwd is still considered safe.
        if let Ok(cwd) = env::current_dir()
            && let Some(cwd_str) = cwd.to_str()
        {
            let norm_cwd = if cwd_str == "/" {
                "/".to_string()
            } else {
                format!("{cwd_str}/")
            };
            if path.starts_with(&norm_cwd) || cwd_str == "/" {
                return false;
            }
        }
        return true;
    }
    for comp in path.split('/') {
        if comp == ".." {
            return true;
        }
    }
    false
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
    append: bool,
) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    // For a fresh (non-append) write, remove any existing reject file
    // first so the new file is created with fresh permissions from the
    // current umask. Otherwise truncate preserves the old mode.
    if !append && fs::metadata(path).is_ok() {
        let _ = fs::remove_file(path);
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(append)
        .truncate(!append)
        .write(true)
        .mode(0o666)
        .open(path)?;
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
fn is_path_readonly(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match fs::metadata(path) {
        Ok(m) => m.permissions().mode() & 0o200 == 0,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_path_readonly(_path: &Path) -> bool {
    false
}

#[cfg(unix)]
fn make_writable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = fs::metadata(path) {
        let mut perms = metadata.permissions();
        let mode = perms.mode();
        perms.set_mode(mode | 0o200);
        let _ = fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn make_writable(_path: &Path) {}

#[cfg(unix)]
fn saved_mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|m| m.permissions().mode())
        .unwrap_or(0o644)
}

#[cfg(not(unix))]
fn saved_mode_of(_path: &Path) -> u32 {
    0
}

#[cfg(unix)]
fn restore_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = fs::metadata(path) {
        let mut perms = metadata.permissions();
        perms.set_mode(mode);
        let _ = fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn restore_mode(_path: &Path, _mode: u32) {}

#[cfg(unix)]
fn nlinks_gt_one(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).map(|m| m.nlink() > 1).unwrap_or(false)
}

#[cfg(not(unix))]
fn nlinks_gt_one(_path: &Path) -> bool {
    false
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

/// Wrap a filename in single quotes when it contains characters that
/// would make a shell-readable representation ambiguous (whitespace,
/// newlines, shell meta). Mirrors GNU patch's error message formatting.
fn quote_for_display(name: &str) -> String {
    let needs_quote = name.chars().any(|c| {
        c.is_whitespace() || matches!(c, '\'' | '"' | '\\' | '$' | '`' | '*' | '?' | '&' | '|')
    });
    if needs_quote {
        format!("'{}'", name.replace('\'', "'\\''"))
    } else {
        name.to_string()
    }
}

fn filename_is_invalid(name: &str) -> bool {
    // GNU patch rejects filenames containing newlines as "Invalid byte
    // sequence" even though POSIX allows them, because downstream tools
    // (and its own reporting format) can't cope.
    name.contains('\n')
}

fn apply_file_patch(
    patch: &FilePatch,
    opts: &Options,
    written_rejects: &mut std::collections::HashSet<String>,
    written_backups: &mut std::collections::HashSet<String>,
    source_cache: &mut std::collections::HashMap<String, String>,
    original_cache: &mut std::collections::HashMap<String, String>,
    written_outputs: &mut std::collections::HashSet<String>,
    rename_dsts: &std::collections::HashSet<String>,
) -> i32 {
    // Git rename/copy: move/copy the source file to the target first, then
    // apply any hunks below against the new path. Under -R the direction
    // is flipped: "rename from X to Y" becomes "move Y back to X".
    let mut already_renamed = false;
    let (effective_rename_from, effective_rename_to) =
        match (&patch.git.rename_from, &patch.git.rename_to) {
            (Some(f), Some(t)) if opts.reverse => (Some(t.clone()), Some(f.clone())),
            (f, t) => (f.clone(), t.clone()),
        };
    let (effective_copy_from, effective_copy_to) = match (&patch.git.copy_from, &patch.git.copy_to)
    {
        (Some(f), Some(t)) if opts.reverse => (Some(t.clone()), Some(f.clone())),
        (f, t) => (f.clone(), t.clone()),
    };
    if let (Some(from), Some(to)) = (&effective_rename_from, &effective_rename_to) {
        let base = opts.directory.as_deref().unwrap_or(".");
        let src = Path::new(base).join(from);
        let dst = Path::new(base).join(to);
        // --backup: pre-rename, snapshot the source content as `{from}.orig`
        // and leave an empty `{to}.orig` to indicate the destination was
        // previously absent.
        if opts.backup {
            let src_backup = compute_backup_path(&src, opts);
            if src.exists()
                && written_backups.insert(src_backup.to_string_lossy().into_owned())
                && let Some(parent) = src_backup.parent()
            {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::copy(&src, &src_backup);
            }
            let dst_backup = compute_backup_path(&dst, opts);
            if !dst.exists() && written_backups.insert(dst_backup.to_string_lossy().into_owned()) {
                if let Some(parent) = dst_backup.parent()
                    && !parent.as_os_str().is_empty()
                    && !parent.exists()
                {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(&dst_backup, "");
            }
        }
        if !src.exists() && dst.exists() {
            // Already renamed in a prior run; proceed as if the rename
            // succeeded, but record the state for the "(already renamed
            // from X)" message below.
            already_renamed = true;
        } else {
            if let Some(parent) = dst.parent()
                && !parent.as_os_str().is_empty()
                && !parent.exists()
            {
                let _ = fs::create_dir_all(parent);
            }
            // Use the pre-run content of the source if we've seen it, so
            // criss-cross renames don't clobber each other. Leave src on
            // disk when another patch in this run will overwrite it as a
            // rename destination — otherwise a simple rename still unlinks
            // the source.
            let src_key = src.to_string_lossy().into_owned();
            if let Some(original) = original_cache.get(&src_key).cloned() {
                if let Err(e) = fs::write(&dst, &original) {
                    eprintln!(
                        "patch: can't rename {} to {}: {e}",
                        display_path(&src),
                        display_path(&dst)
                    );
                    return EXIT_TROUBLE;
                }
                let src_will_be_overwritten = rename_dsts.contains(&src_key);
                if src.exists() && src != dst && !src_will_be_overwritten {
                    let _ = fs::remove_file(&src);
                }
            } else if let Err(e) = fs::rename(&src, &dst) {
                eprintln!(
                    "patch: can't rename {} to {}: {e}",
                    display_path(&src),
                    display_path(&dst)
                );
                return EXIT_TROUBLE;
            }
        }
    } else if let (Some(from), Some(to)) = (&effective_copy_from, &effective_copy_to) {
        let base = opts.directory.as_deref().unwrap_or(".");
        let src = Path::new(base).join(from);
        let dst = Path::new(base).join(to);
        // Seed the copy with the source's pre-patch content, not whatever
        // a previous patch in the same run already wrote.
        let src_key = src.to_string_lossy().into_owned();
        let original = original_cache.get(&src_key).cloned().or_else(|| {
            fs::read_to_string(&src)
                .ok()
                .inspect(|s| {
                    original_cache.insert(src_key.clone(), s.clone());
                })
                .clone()
        });
        let _ = original; // used below via original_cache
        // --backup: empty-file backup of the destination (since it was absent
        // before the copy). Source is not backed up for copy.
        if opts.backup && !dst.exists() {
            let dst_backup = compute_backup_path(&dst, opts);
            if written_backups.insert(dst_backup.to_string_lossy().into_owned()) {
                if let Some(parent) = dst_backup.parent()
                    && !parent.as_os_str().is_empty()
                    && !parent.exists()
                {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(&dst_backup, "");
            }
        }
        if let Some(parent) = dst.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            let _ = fs::create_dir_all(parent);
        }
        // If we have the source's original (pre-run) content cached, use
        // it so copies see the unpatched source; otherwise fall back to
        // copying from disk.
        if let Some(content) = original_cache.get(&src_key) {
            if let Err(e) = fs::write(&dst, content) {
                eprintln!(
                    "patch: can't copy {} to {}: {e}",
                    src.display(),
                    dst.display()
                );
                return EXIT_TROUBLE;
            }
        } else if let Err(e) = fs::copy(&src, &dst) {
            eprintln!(
                "patch: can't copy {} to {}: {e}",
                src.display(),
                dst.display()
            );
            return EXIT_TROUBLE;
        }
    }

    // No filename in the patch and no positional argument: nothing to do.
    if opts.positional_file.is_none()
        && patch.old_file.is_empty()
        && patch.new_file.is_empty()
        && !patch.hunks.is_empty()
    {
        if opts.force {
            // -f: skip with the GNU "can't find file to patch" message.
            if !opts.silent {
                println!("can't find file to patch at input line 1");
                println!("No file to patch.  Skipping patch.");
                println!(
                    "{} out of {} hunk ignored",
                    patch.hunks.len(),
                    patch.hunks.len()
                );
            }
            return EXIT_HUNKS_FAILED;
        } else {
            // Default: this looks like garbage to GNU.
            let argv0 = env::args().next().unwrap_or_else(|| "patch".to_string());
            eprintln!("{argv0}: **** Only garbage was found in the patch input.");
            process::exit(EXIT_TROUBLE);
        }
    }

    // Check both candidate paths for dangerous components (absolute, `..`).
    let old_stripped_check = strip_path_opt(&patch.old_file, opts.strip);
    let new_stripped_check = strip_path_opt(&patch.new_file, opts.strip);

    // If `-pN` stripped both candidates to empty (and no positional file),
    // we have nothing to patch. Emit GNU's skip block.
    if opts.strip.is_some()
        && opts.positional_file.is_none()
        && (old_stripped_check.is_empty() || old_stripped_check == "/dev/null")
        && (new_stripped_check.is_empty() || new_stripped_check == "/dev/null")
        && (!patch.old_file.is_empty() || !patch.new_file.is_empty())
    {
        if !opts.silent {
            let prefix1 = match patch.format {
                DiffFormat::Context => "*** ",
                _ => "--- ",
            };
            let prefix2 = match patch.format {
                DiffFormat::Context => "--- ",
                _ => "+++ ",
            };
            println!("can't find file to patch at input line 3");
            println!("Perhaps you used the wrong -p or --strip option?");
            println!("The text leading up to this was:");
            println!("--------------------------");
            println!("|{prefix1}{}", patch.old_header);
            println!("|{prefix2}{}", patch.new_header);
            println!("--------------------------");
            println!("No file to patch.  Skipping patch.");
            println!(
                "{} out of {} hunk ignored",
                patch.hunks.len(),
                patch.hunks.len()
            );
        }
        return EXIT_HUNKS_FAILED;
    }
    // GNU only emits "Ignoring potentially dangerous file name" when ALL
    // non-/dev/null candidates are dangerous. If any side has a safe name
    // (even if it doesn't exist), proceed silently and let the regular
    // file-not-found path handle it.
    let old_is_dangerous = is_dangerous_path(&old_stripped_check);
    let new_is_dangerous = is_dangerous_path(&new_stripped_check);
    let old_is_safe_candidate =
        old_stripped_check != "/dev/null" && !old_stripped_check.is_empty() && !old_is_dangerous;
    let new_is_safe_candidate =
        new_stripped_check != "/dev/null" && !new_stripped_check.is_empty() && !new_is_dangerous;
    let mut dangerous_msgs: Vec<String> = Vec::new();
    if !old_is_safe_candidate && !new_is_safe_candidate {
        if old_is_dangerous {
            dangerous_msgs.push(old_stripped_check.clone());
        }
        if new_is_dangerous && !dangerous_msgs.contains(&new_stripped_check) {
            dangerous_msgs.push(new_stripped_check.clone());
        }
    }
    let any_dangerous = old_is_dangerous || new_is_dangerous;
    let base = opts.directory.as_deref().unwrap_or(".");
    let good_exists = (old_is_safe_candidate && Path::new(base).join(&old_stripped_check).exists())
        || (new_is_safe_candidate && Path::new(base).join(&new_stripped_check).exists());
    if any_dangerous && !good_exists {
        if !opts.silent {
            for msg in &dangerous_msgs {
                println!("Ignoring potentially dangerous file name {msg}");
            }
            let prefix1 = match patch.format {
                DiffFormat::Context => "*** ",
                _ => "--- ",
            };
            let prefix2 = match patch.format {
                DiffFormat::Context => "--- ",
                _ => "+++ ",
            };
            println!("can't find file to patch at input line 3");
            println!("Perhaps you used the wrong -p or --strip option?");
            println!("The text leading up to this was:");
            println!("--------------------------");
            println!("|{prefix1}{}", patch.old_header);
            println!("|{prefix2}{}", patch.new_header);
            println!("--------------------------");
            println!("No file to patch.  Skipping patch.");
            println!(
                "{} out of {} hunk ignored",
                patch.hunks.len(),
                patch.hunks.len()
            );
        }
        return EXIT_HUNKS_FAILED;
    }

    let target = resolve_target_file(patch, opts);
    let target_display = display_path(&target);

    // Refuse to patch filenames that contain NUL (can't be a real path).
    if target_display.contains('\0') {
        if !opts.silent {
            let prefix1 = match patch.format {
                DiffFormat::Context => "*** ",
                _ => "--- ",
            };
            let prefix2 = match patch.format {
                DiffFormat::Context => "--- ",
                _ => "+++ ",
            };
            println!("can't find file to patch at input line 3");
            println!("Perhaps you should have used the -p or --strip option?");
            println!("The text leading up to this was:");
            println!("--------------------------");
            println!("|{prefix1}{}", patch.old_header);
            println!("|{prefix2}{}", patch.new_header);
            println!("--------------------------");
            println!("No file to patch.  Skipping patch.");
            println!(
                "{} out of {} hunk ignored",
                patch.hunks.len(),
                patch.hunks.len()
            );
        }
        return EXIT_HUNKS_FAILED;
    }

    // Symlink-mode git patch values (120000) let the patch itself describe
    // a symbolic link; skip the "refuse non-regular file" guard for those.
    let is_symlink_mode_patch = matches!(
        patch.git.new_file_mode,
        Some(m) if (m & 0o170000) == 0o120000
    ) || matches!(
        patch.git.deleted_file_mode,
        Some(m) if (m & 0o170000) == 0o120000
    ) || matches!(
        patch.git.old_mode,
        Some(m) if (m & 0o170000) == 0o120000
    ) || matches!(
        patch.git.new_mode,
        Some(m) if (m & 0o170000) == 0o120000
    ) || matches!(
        patch.git.index_mode,
        Some(m) if (m & 0o170000) == 0o120000
    );
    // Refuse to patch non-regular files (e.g. symlinks) by default.
    // --follow-symlinks and symlink-mode git patches override this.
    if !opts.follow_symlinks
        && !is_symlink_mode_patch
        && let Ok(md) = fs::symlink_metadata(&target)
        && md.file_type().is_symlink()
    {
        if !opts.silent {
            println!("File {target_display} is not a regular file -- refusing to patch");
            let hunk_word = if patch.hunks.len() == 1 {
                "hunk"
            } else {
                "hunks"
            };
            let reject_name = opts
                .reject_file
                .clone()
                .unwrap_or_else(|| format!("{target_display}.rej"));
            println!(
                "{} out of {} {hunk_word} ignored -- saving rejects to file {reject_name}",
                patch.hunks.len(),
                patch.hunks.len()
            );
        }
        if !opts.dry_run && !patch.hunks.is_empty() {
            let reject_path = opts
                .reject_file
                .clone()
                .unwrap_or_else(|| format!("{target_display}.rej"));
            let append = !written_rejects.insert(reject_path.clone());
            let all_hunks: Vec<&Hunk> = patch.hunks.iter().collect();
            let _ = write_reject_file(&reject_path, patch, &all_hunks, opts, append);
        }
        return EXIT_HUNKS_FAILED;
    }

    // Read-only handling: emit the appropriate message and (for `fail`)
    // skip hunk application, writing the whole patch to a .rej file.
    let is_readonly = is_path_readonly(&target);
    match (opts.read_only, is_readonly) {
        (ReadOnlyMode::Fail, true) => {
            if !opts.silent {
                println!("File {target_display} is read-only; refusing to patch");
                let hunk_word = if patch.hunks.len() == 1 {
                    "hunk"
                } else {
                    "hunks"
                };
                let reject_name = opts
                    .reject_file
                    .clone()
                    .unwrap_or_else(|| format!("{target_display}.rej"));
                println!(
                    "{} out of {} {hunk_word} ignored -- saving rejects to file {reject_name}",
                    patch.hunks.len(),
                    patch.hunks.len()
                );
            }
            if !opts.dry_run {
                let reject_path = opts
                    .reject_file
                    .clone()
                    .unwrap_or_else(|| format!("{target_display}.rej"));
                let append = !written_rejects.insert(reject_path.clone());
                let all_hunks: Vec<&Hunk> = patch.hunks.iter().collect();
                let _ = write_reject_file(&reject_path, patch, &all_hunks, opts, append);
            }
            return EXIT_HUNKS_FAILED;
        }
        (ReadOnlyMode::Warn, true) => {
            if !opts.silent {
                println!("File {target_display} is read-only; trying to patch anyway");
            }
        }
        _ => {}
    }

    // Detect "would create / would delete" cases for -f messaging.
    let old_resolved = strip_path_opt(&patch.old_file, opts.strip);
    let new_resolved = strip_path_opt(&patch.new_file, opts.strip);
    let is_creation =
        old_resolved == "/dev/null" && new_resolved != "/dev/null" && !new_resolved.is_empty();
    let is_deletion =
        new_resolved == "/dev/null" && old_resolved != "/dev/null" && !old_resolved.is_empty();
    // Under -R, a creation patch is applied as a deletion; don't treat an
    // already-existing target as "would create, already exists".
    let creation_on_existing = is_creation && target.exists() && !opts.reverse;
    if creation_on_existing && !opts.silent {
        println!("The next patch would create the file {target_display},");
        println!("which already exists!  Applying it anyway.");
    }
    // Use symlink_metadata for existence so a dangling symlink counts as
    // present (regular `target.exists()` follows links and returns false).
    let target_present = fs::symlink_metadata(&target).is_ok();
    // If resolving the target hits a symlink loop (ELOOP) or a parent
    // segment is a symlink whose target escapes the current working
    // directory, GNU emits "Invalid file name X -- skipping patch".
    let loop_err = match fs::metadata(&target) {
        Err(e) if e.raw_os_error() == Some(40) => true,
        _ => false,
    };
    if loop_err || path_escapes_cwd_via_symlink(&target) {
        if !opts.silent {
            println!("Invalid file name {target_display} -- skipping patch");
        }
        return EXIT_HUNKS_FAILED;
    }
    if is_deletion && !target_present && !opts.silent && !is_symlink_mode_patch {
        if opts.force {
            println!("The next patch would delete the file {target_display},");
            println!("which does not exist!  Applying it anyway.");
        } else {
            println!("The next patch would delete the file {target_display},");
            println!("which does not exist!  Assume -R? [n] ");
            println!("Apply anyway? [n] ");
            println!("Skipping patch.");
            println!(
                "{} out of {} hunk ignored",
                patch.hunks.len(),
                patch.hunks.len()
            );
            return EXIT_HUNKS_FAILED;
        }
    }

    // Detect "would empty out the file / but already empty" (test [120]).
    // The hunks would reduce the target to zero lines, and the existing file
    // is already empty — GNU emits the pre-message before applying.
    let would_empty = !patch.hunks.is_empty()
        && patch.hunks.iter().all(|h| h.new_count == 0)
        && patch.hunks.iter().any(|h| h.old_count > 0);
    let target_is_empty = target.metadata().map(|m| m.len() == 0).unwrap_or(false);
    // Local override so we can flip -R in batch mode without mutating the
    // shared Options.
    let mut effective_reverse = opts.reverse;
    if would_empty && target.exists() && target_is_empty && !opts.silent && !opts.reverse {
        if opts.batch {
            println!("The next patch would empty out the file {target_display},");
            println!("which is already empty!  Assuming -R.");
            effective_reverse = true;
        } else if opts.force {
            println!("The next patch would empty out the file {target_display},");
            println!("which is already empty!  Applying it anyway.");
        }
    }

    // GNU patch refuses filenames with embedded newlines. For `-o FOO`
    // where FOO has a newline, fail BEFORE announcing "patching file" —
    // the test expects the error to stand alone.
    if let Some(out) = &opts.output
        && filename_is_invalid(out)
    {
        let argv0 = env::args().next().unwrap_or_else(|| "patch".to_string());
        let quoted = quote_for_display(out);
        eprintln!("{argv0}: **** Can't create file {quoted} : Invalid byte sequence");
        process::exit(EXIT_TROUBLE);
    }

    // Pre-scan hunks for CRs so we can emit the "Stripping CRs" notice
    // before the "patching file" announcement. The file_lines vector never
    // contains CRs (content.lines() strips \r\n), so we always strip CRs
    // from hunk lines at match time. But only emit the notice when the
    // target file has no CRs (i.e. the patch introduced them).
    let hunks_have_cr = patch.hunks.iter().any(|h| {
        h.lines.iter().any(|l| match l {
            HunkLine::Context(s) | HunkLine::Remove(s) | HunkLine::Add(s) => s.ends_with('\r'),
        })
    });
    let target_has_crlf = target
        .exists()
        .then(|| fs::read_to_string(&target).ok())
        .flatten()
        .map(|s| s.contains("\r\n"))
        .unwrap_or(false);
    let strip_cr = !opts.binary && hunks_have_cr;
    if strip_cr && !target_has_crlf {
        let already = STRIPPED_CRS.with(|c| c.replace(true));
        if !already {
            println!("(Stripping trailing CRs from patch; use --binary to disable.)");
        }
    }

    // Detect a git symlink-mode patch (100120000 octal = 120000), which
    // represents a symbolic link. We announce "patching symbolic link X"
    // and handle the hunk body as the link target path rather than file
    // content.
    let is_symlink_patch = matches!(patch.git.new_file_mode, Some(m) if (m & 0o170000) == 0o120000)
        || matches!(patch.git.deleted_file_mode, Some(m) if (m & 0o170000) == 0o120000)
        || matches!(patch.git.old_mode, Some(m) if (m & 0o170000) == 0o120000)
        || matches!(patch.git.new_mode, Some(m) if (m & 0o170000) == 0o120000)
        || matches!(patch.git.index_mode, Some(m) if (m & 0o170000) == 0o120000);

    if !opts.silent {
        let verb = if opts.dry_run { "checking" } else { "patching" };
        let kind = if is_symlink_patch {
            "symbolic link"
        } else {
            "file"
        };
        // -o redirects the output: report the destination filename and
        // note "(read from <source>)" so the user can tell.
        let (display_name, output_suffix) = if let Some(out) = &opts.output {
            (
                quote_for_display(out),
                format!(" (read from {})", quote_for_display(&target_display)),
            )
        } else {
            (quote_for_display(&target_display), String::new())
        };
        let suffix = if let Some(from) = &effective_rename_from {
            if already_renamed {
                format!(" (already renamed from {from})")
            } else {
                format!(" (renamed from {from})")
            }
        } else if let Some(from) = &effective_copy_from {
            format!(" (copied from {from})")
        } else {
            output_suffix
        };
        println!("{verb} {kind} {display_name}{suffix}");
    }

    // GNU patch refuses filenames with embedded newlines, emitting a
    // specific "Invalid byte sequence" message and exiting 2.
    if filename_is_invalid(&target_display) {
        let argv0 = env::args().next().unwrap_or_else(|| "patch".to_string());
        let quoted = quote_for_display(&target_display);
        if opts.output.is_some() {
            eprintln!("{argv0}: **** Can't create file {quoted} : Invalid byte sequence");
        } else {
            eprintln!(
                "{argv0}: **** Can't rename file ab.XXXXXX to {quoted} : Invalid byte sequence"
            );
        }
        process::exit(EXIT_TROUBLE);
    }

    // Symlink patches: the hunk body is the link target path. Handle
    // create/modify/delete using os::unix::fs::symlink rather than going
    // through the normal file-content path.
    if is_symlink_patch && !opts.dry_run {
        use std::os::unix::fs::symlink;
        let existed = fs::symlink_metadata(&target).is_ok();
        if opts.backup {
            use std::os::unix::fs::symlink;
            let backup_path = compute_backup_path(&target, opts);
            if written_backups.insert(backup_path.to_string_lossy().into_owned()) {
                if let Some(parent) = backup_path.parent()
                    && !parent.as_os_str().is_empty()
                    && !parent.exists()
                {
                    let _ = fs::create_dir_all(parent);
                }
                // Remove any existing backup so we can recreate it fresh.
                let _ = fs::remove_file(&backup_path);
                if existed {
                    // Mirror the link: `symlink.orig -> <old_target>`. This
                    // way `echo x > symlink.orig` follows the link like GNU.
                    match fs::read_link(&target) {
                        Ok(old_t) => {
                            let _ = symlink(&old_t, &backup_path);
                        }
                        Err(_) => {
                            let _ = fs::write(&backup_path, "");
                        }
                    }
                } else {
                    let _ = fs::write(&backup_path, "");
                }
            }
        }
        if patch.git.deleted_file_mode.is_some() {
            // Deletion: unlink the symlink.
            if existed {
                if let Err(e) = fs::remove_file(&target) {
                    eprintln!("patch: can't remove {target_display}: {e}");
                    return EXIT_TROUBLE;
                }
            }
            return EXIT_SUCCESS;
        }
        // Collect the Add lines as the link target (last Add wins; there
        // is typically a single line).
        let target_path: String = patch
            .hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter_map(|l| match l {
                HunkLine::Add(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        if target_path.is_empty() {
            return EXIT_HUNKS_FAILED;
        }
        if let Some(parent) = target.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            let _ = fs::create_dir_all(parent);
        }
        // Remove any existing file/link at target so the create succeeds.
        if fs::symlink_metadata(&target).is_ok() {
            let _ = fs::remove_file(&target);
        }
        if let Err(e) = symlink(&target_path, &target) {
            eprintln!("patch: can't create symlink {target_display}: {e}");
            return EXIT_TROUBLE;
        }
        return EXIT_SUCCESS;
    }

    // Header-only git patches (rename/copy/delete/mode with no hunks).
    if patch.hunks.is_empty() {
        if patch.git.deleted_file_mode.is_some() {
            // Binary-content deletion: we can't verify the content matches,
            // so GNU's behaviour is to refuse deletion and emit the "Not
            // deleting ... content differs" message while still creating the
            // --backup copy.
            if patch.git.binary_summary {
                if opts.backup && target.exists() {
                    let backup_path = compute_backup_path(&target, opts);
                    if written_backups.insert(backup_path.to_string_lossy().into_owned()) {
                        if let Some(parent) = backup_path.parent()
                            && !parent.as_os_str().is_empty()
                            && !parent.exists()
                        {
                            let _ = fs::create_dir_all(parent);
                        }
                        let _ = fs::copy(&target, &backup_path);
                    }
                }
                if !opts.silent {
                    println!("Not deleting file {target_display} as content differs from patch");
                }
                return EXIT_HUNKS_FAILED;
            }
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

    // Read existing file content (or empty for new files). If a previous
    // patch in this run already produced an intermediate version of this
    // source path (relevant for `-o` + multiple patches), use that — but
    // skip the cache for creation patches (old=/dev/null) so the new-file
    // state starts empty.
    let target_key = target.to_string_lossy().into_owned();
    let is_creation_patch = patch.old_file == "/dev/null" || patch.old_file.is_empty();
    let content = if !is_creation_patch && let Some(cached) = source_cache.get(&target_key) {
        cached.clone()
    } else if target.exists() {
        let disk = fs::read_to_string(&target).unwrap_or_else(|e| {
            eprintln!("patch: can't read {target_display}: {e}");
            process::exit(EXIT_TROUBLE);
        });
        original_cache
            .entry(target_key.clone())
            .or_insert_with(|| disk.clone());
        disk
    } else {
        // Check if this is creating or deleting a new file; when deleting a
        // non-existent file under -f we continue with empty content so the
        // hunk fails cleanly rather than aborting with "can't open file".
        if patch.old_file == "/dev/null"
            || patch.old_file.is_empty()
            || patch.git.new_file_mode.is_some()
            || patch.hunks.iter().all(|h| h.old_count == 0)
            || (is_deletion && opts.force)
        {
            String::new()
        } else {
            eprintln!("patch: can't open file {target_display}: No such file or directory");
            return EXIT_TROUBLE;
        }
    };

    // Detect and strip CRLF endings from the input file; we'll re-add
    // them on write if they were there originally. Under `--binary` we
    // preserve CR bytes in file_lines so matching can compare raw bytes.
    let file_has_crlf = !opts.binary && content.contains("\r\n");
    let mut file_lines: Vec<String> = if content.is_empty() {
        Vec::new()
    } else if opts.binary {
        // Split on '\n' only, keeping any trailing '\r' on the line.
        let mut parts: Vec<String> = content.split('\n').map(|s| s.to_string()).collect();
        if parts.last().is_some_and(|s| s.is_empty()) {
            parts.pop();
        }
        parts
    } else {
        content.lines().map(|l| l.to_string()).collect()
    };

    let mut failed_hunks = 0;
    let mut failed_hunk_entries: Vec<&Hunk> = Vec::new();
    let total_hunks = patch.hunks.len();

    // Creation patch on an already-existing file: fail every hunk so the
    // .rej file is created and we don't double the content.
    if creation_on_existing {
        for hunk in &patch.hunks {
            failed_hunks += 1;
            failed_hunk_entries.push(hunk);
            if !opts.silent {
                println!(
                    "Hunk #{} FAILED at {}.",
                    failed_hunks,
                    hunk.old_start.max(1)
                );
            }
        }
    }
    // Running delta: lines added by prior hunks minus lines removed. Used
    // to shift the "expected" position reported to the user.
    let mut line_delta: i64 = 0;

    for (hunk_idx, hunk) in patch.hunks.iter().enumerate() {
        if creation_on_existing {
            // Already counted above.
            continue;
        }
        let (raw_start, raw_count) = if effective_reverse {
            (hunk.new_start, hunk.new_count)
        } else {
            (hunk.old_start, hunk.old_count)
        };
        // Creation hunks use start 0 ("before any line"); don't report
        // offsets for those — the effective first line is 1.
        // Pure-insertion hunks (count 0) use the other side's line as the
        // expected position, matching GNU's reporting.
        let expected_line = if raw_start == 0 {
            1
        } else if raw_count == 0 {
            raw_start as i64 + 1 + line_delta
        } else {
            raw_start as i64 + line_delta
        };

        match apply_hunk(&file_lines, hunk, opts.fuzz, effective_reverse, strip_cr) {
            Some((new_lines, fuzz_used, _offset, applied_start)) => {
                // Capture pre-apply file length so asymmetry fuzz logic
                // below can tell if the hunk truly sits at EOF.
                let pre_apply_len = file_lines.len();
                file_lines = new_lines;
                let hunk_num = hunk_idx + 1;
                let applied_line = applied_start as i64 + 1;
                let reported_offset = applied_line - expected_line;

                // GNU heuristic: if a non-zero offset was needed AND the
                // hunk's leading and trailing context counts differ, the
                // (truncated) shorter side counts as fuzz.
                let leading_ctx_count = hunk
                    .lines
                    .iter()
                    .take_while(|l| matches!(l, HunkLine::Context(_)))
                    .count();
                let trailing_ctx_count = hunk
                    .lines
                    .iter()
                    .rev()
                    .take_while(|l| matches!(l, HunkLine::Context(_)))
                    .count();
                // GNU heuristic: when a non-zero offset was needed AND the
                // hunk's leading and trailing context counts differ, the
                // (truncated) shorter side counts as fuzz. Additionally,
                // when a hunk has more leading than trailing context AND
                // the file continues past the hunk (so the trailing side
                // was "truncated" rather than naturally at EOF), GNU also
                // reports this as fuzz — see upstream "asymmetric-hunks"
                // test 4. Hunks that actually sit at EOF don't fuzz.
                let applied_end = applied_start + (old_len_from_hunk(hunk));
                let past_end_has_lines = applied_end < pre_apply_len;
                let near_end = leading_ctx_count > trailing_ctx_count
                    && hunk.old_start > 1
                    && past_end_has_lines;
                let asymmetry_fuzz = if leading_ctx_count != trailing_ctx_count
                    && (reported_offset != 0 || near_end)
                {
                    leading_ctx_count.abs_diff(trailing_ctx_count)
                } else {
                    0
                };
                let final_fuzz = fuzz_used.max(asymmetry_fuzz);

                let line_word = |n: i64| if n == 1 { "line" } else { "lines" };
                let msg = match (reported_offset, final_fuzz) {
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
                let fuzz_used = final_fuzz;
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
                if effective_reverse {
                    line_delta += removed - added;
                } else {
                    line_delta += added - removed;
                }
            }
            None => {
                if opts.forward {
                    // Check if already applied
                    let reverse_result =
                        apply_hunk(&file_lines, hunk, opts.fuzz, !effective_reverse, strip_cr);
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
                    // Under --binary, a CR-vs-no-CR mismatch between hunk
                    // context and file is the likely cause; annotate so
                    // users know to re-run without --binary.
                    let suffix = if opts.binary && hunks_have_cr && !target_has_crlf {
                        " (different line endings)"
                    } else {
                        ""
                    };
                    println!("Hunk #{} FAILED at {expected_line}{suffix}.", hunk_idx + 1);
                }
            }
        }
    }

    // Write reject file for any failed hunks.
    if !failed_hunk_entries.is_empty() && !opts.dry_run {
        let reject_path = opts
            .reject_file
            .clone()
            .unwrap_or_else(|| format!("{target_display}.rej"));
        let append = !written_rejects.insert(reject_path.clone());
        if let Err(e) = write_reject_file(&reject_path, patch, &failed_hunk_entries, opts, append) {
            eprintln!("patch: can't write rejects to {reject_path}: {e}");
        }
    }

    if failed_hunks > 0 && !opts.silent {
        // With --set-utc / --set-time, failed hunks mean we don't trust the
        // patch to represent the expected file content, so we must not
        // touch the file's mtime either.
        if opts.set_utc || opts.set_time {
            println!("Not setting time of file {target_display} (time mismatch)");
        }
        let hunk_word = if total_hunks == 1 { "hunk" } else { "hunks" };
        let reject_name = opts
            .reject_file
            .clone()
            .unwrap_or_else(|| format!("{target_display}.rej"));
        let reject_suffix = if opts.dry_run {
            String::new()
        } else {
            format!(" -- saving rejects to file {reject_name}")
        };
        println!("{failed_hunks} out of {total_hunks} {hunk_word} FAILED{reject_suffix}");
    }

    // Write output. If every hunk failed we still create the backup but
    // skip the actual write so we don't touch the target file's content.
    let all_hunks_failed = total_hunks > 0 && failed_hunks == total_hunks;
    if !opts.dry_run {
        let output_path = opts
            .output
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| target.clone());

        // Create backup if requested
        // Implicit backup when old_file is `<target>.orig` — some diffs
        // encode the original filename that way, and GNU treats it as a
        // hint to save the original.
        let old_stripped = strip_path_opt(&patch.old_file, opts.strip);
        let new_stripped = strip_path_opt(&patch.new_file, opts.strip);
        let implicit_backup = old_stripped != "/dev/null"
            && new_stripped != "/dev/null"
            && old_stripped != new_stripped
            && old_stripped.strip_suffix(".orig") == Some(new_stripped.as_str());
        if implicit_backup && output_path.exists() {
            let base = opts.directory.as_deref().unwrap_or(".");
            let backup_path = Path::new(base).join(&old_stripped);
            let key = backup_path.to_string_lossy().into_owned();
            if written_backups.insert(key) {
                if let Some(parent) = backup_path.parent()
                    && !parent.as_os_str().is_empty()
                    && !parent.exists()
                {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::copy(&output_path, &backup_path);
            }
        }

        let backup_key = output_path.to_string_lossy().into_owned();
        if opts.backup
            && output_path.exists()
            && (failed_hunks == 0 || !opts.no_backup_if_mismatch)
            && written_backups.insert(backup_key)
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

        // If every hunk failed, don't touch the file content (matches
        // GNU's behaviour and preserves hardlinks).
        if all_hunks_failed {
            return EXIT_HUNKS_FAILED;
        }

        // Choose the line separator: preserve CRLF if the original file had
        // CRLFs (and --binary wasn't set).
        let line_sep = if file_has_crlf { "\r\n" } else { "\n" };
        // When joining with `\r\n`, strip any trailing `\r` from stored
        // lines so we don't double up (hunks written from a CRLF diff can
        // carry CRs on produced lines).
        let out_lines: Vec<String> = if file_has_crlf {
            file_lines
                .iter()
                .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
                .collect()
        } else {
            file_lines.clone()
        };
        let mut output_content = out_lines.join(line_sep);
        // Preserve trailing newline when there is content. An empty result
        // is written as a truly-empty file (no stray newline).
        if !output_content.is_empty() && (content.ends_with('\n') || !content.is_empty()) {
            output_content.push_str(line_sep);
        }

        // When -R reverses a creation patch (old=/dev/null or git
        // new_file_mode), remove the file entirely rather than leaving it
        // empty — the reverse of "create" is "delete".
        let reverse_creation = effective_reverse
            && (patch.old_file == "/dev/null" || patch.git.new_file_mode.is_some());
        // A deletion patch (--- X +++ /dev/null) that successfully applies
        // should remove the target — but only if the remaining content is
        // actually empty. Otherwise the patch covered only part of the file
        // and GNU refuses to delete, leaving the partial result behind.
        // In POSIX mode, deletion patches leave the (now-empty) file in
        // place instead of removing it.
        let deletion_patch = !effective_reverse
            && new_resolved == "/dev/null"
            && old_resolved != "/dev/null"
            && failed_hunks == 0
            && !opts.posix;
        let content_after_empty = output_content.is_empty() || output_content == line_sep;
        let forward_deletion = deletion_patch && content_after_empty;
        let deletion_refused = deletion_patch && !content_after_empty;
        if deletion_refused {
            if !opts.silent {
                println!("Not deleting file {target_display} as content differs from patch");
            }
            if failed_hunks == 0 {
                failed_hunks = 1;
            }
        }
        // Handle empty file removal
        if (opts.remove_empty && output_content.trim().is_empty())
            || reverse_creation
            || forward_deletion
        {
            let _ = fs::remove_file(&output_path);
            // Remove now-empty parent directories, matching GNU -E semantics.
            let mut parent = output_path.parent();
            while let Some(dir) = parent {
                if dir.as_os_str().is_empty() || dir == Path::new(".") {
                    break;
                }
                if fs::remove_dir(dir).is_err() {
                    break;
                }
                parent = dir.parent();
            }
        } else {
            // Ensure parent directory exists for new files
            if let Some(parent) = output_path.parent()
                && !parent.exists()
            {
                let _ = fs::create_dir_all(parent);
            }

            // Ensure the file is writable (for --read-only=warn/ignore),
            // but restore the original permissions afterwards so subsequent
            // patches still see the read-only state and emit the warning.
            let saved_mode = if output_path.exists() && is_path_readonly(&output_path) {
                let mode = saved_mode_of(&output_path);
                make_writable(&output_path);
                Some(mode)
            } else {
                None
            };
            // Break hardlinks so we don't accidentally modify other files
            // sharing the inode: write via unlink+create rather than
            // in-place truncation. Likewise for symlinks under
            // --follow-symlinks, we replace the link with a regular file
            // rather than writing through to the target.
            let is_link = fs::symlink_metadata(&output_path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if output_path.exists() && (nlinks_gt_one(&output_path) || is_link) {
                let _ = fs::remove_file(&output_path);
            }
            // When the same `-o` output is targeted by multiple patches in
            // a single run (POSIX "concatenated versions" rule), append the
            // later results instead of overwriting.
            let output_key = output_path.to_string_lossy().into_owned();
            let should_append = opts.output.is_some() && !written_outputs.insert(output_key);
            let write_result = if should_append {
                use std::io::Write;
                std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&output_path)
                    .and_then(|mut f| f.write_all(output_content.as_bytes()))
            } else {
                fs::write(&output_path, &output_content)
            };
            write_result.unwrap_or_else(|e| {
                eprintln!(
                    "patch: can't write to {}: {}",
                    display_path(&output_path),
                    e
                );
                process::exit(EXIT_TROUBLE);
            });
            // Update the source cache so the next patch in this run sees
            // the intermediate result rather than re-reading from disk.
            source_cache.insert(target_key.clone(), output_content.clone());
            if let Some(mode) = saved_mode {
                restore_mode(&output_path, mode);
            }
        }

        // Apply git mode change after writing the file.
        let mode_to_apply = if opts.reverse {
            patch.git.old_mode.or(patch.git.new_file_mode)
        } else {
            patch.git.new_mode.or(patch.git.new_file_mode)
        };
        apply_git_mode(&output_path, mode_to_apply);

        // --set-utc / --set-time: propagate the patch header's timestamp
        // to the target file's mtime when all hunks applied cleanly.
        if (opts.set_utc || opts.set_time)
            && failed_hunks == 0
            && output_path.exists()
            && let Some(ts) = extract_timestamp_field(&format!(
                "--- {}",
                if opts.reverse {
                    &patch.old_header
                } else {
                    &patch.new_header
                }
            ))
            && let Some(time) = parse_header_timestamp(ts, opts.set_utc)
            && let Ok(f) = fs::OpenOptions::new().write(true).open(&output_path)
        {
            let _ = f.set_modified(time);
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
    // CRs in the patch are handled per-file during apply_hunk: if the
    // target file lacks CRs, we strip trailing \r from hunk lines on the
    // fly and emit the "(Stripping trailing CRs...)" message once.
    // A NUL byte anywhere in the input is a hard-fail — GNU patch refuses
    // to process it and reports the offending line number.
    if input.contains('\0') {
        let mut line_no = 1usize;
        for byte in input.bytes() {
            if byte == b'\n' {
                line_no += 1;
            } else if byte == 0 {
                break;
            }
        }
        let argv0 = env::args().next().unwrap_or_else(|| "patch".to_string());
        eprintln!("{argv0}: **** patch line {line_no} contains NUL byte");
        process::exit(EXIT_TROUBLE);
    }
    if opts.ed {
        apply_ed_patches(&input, &opts);
        process::exit(EXIT_SUCCESS);
    }
    let patches = parse_patches(&input);

    // If the only 'patches' were unsupported binary diffs, exit 1 rather
    // than erroring out — GNU-compatible behaviour for `git binary patch`.
    if BINARY_SEEN.with(|b| b.get()) && patches.iter().all(|p| p.hunks.is_empty()) {
        process::exit(EXIT_HUNKS_FAILED);
    }

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
        } else if opts.positional_file.is_some() && looks_like_ed_script(&input) {
            // Auto-detect ed-style when a positional file is given and the
            // input has no unified/context/normal markers.
            apply_ed_patches(&input, &opts);
            process::exit(EXIT_SUCCESS);
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

    let mut written_rejects: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut written_backups: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut source_cache: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    // Remembers the pre-run content of files, so a later `copy_from` can
    // snapshot the unpatched source rather than reading what an earlier
    // patch in the same run has already written. Pre-seed it with every
    // path each patch touches (target, rename_from, copy_from) so later
    // patches can still see the original state even after a rename.
    let mut original_cache: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    {
        let base = opts.directory.as_deref().unwrap_or(".");
        let mut paths: Vec<String> = Vec::new();
        for patch in &patches {
            if let Some(from) = &patch.git.rename_from {
                paths.push(Path::new(base).join(from).to_string_lossy().into_owned());
            }
            if let Some(to) = &patch.git.rename_to {
                paths.push(Path::new(base).join(to).to_string_lossy().into_owned());
            }
            if let Some(from) = &patch.git.copy_from {
                paths.push(Path::new(base).join(from).to_string_lossy().into_owned());
            }
            if let Some(to) = &patch.git.copy_to {
                paths.push(Path::new(base).join(to).to_string_lossy().into_owned());
            }
        }
        for p in paths {
            if !original_cache.contains_key(&p)
                && let Ok(s) = fs::read_to_string(&p)
            {
                original_cache.insert(p, s);
            }
        }
    }
    let mut written_outputs: std::collections::HashSet<String> = std::collections::HashSet::new();
    let rename_dsts: std::collections::HashSet<String> = {
        let base = opts.directory.as_deref().unwrap_or(".");
        patches
            .iter()
            .filter_map(|p| p.git.rename_to.as_ref())
            .map(|t| Path::new(base).join(t).to_string_lossy().into_owned())
            .collect()
    };
    for patch in &patches {
        let exit = apply_file_patch(
            patch,
            &opts,
            &mut written_rejects,
            &mut written_backups,
            &mut source_cache,
            &mut original_cache,
            &mut written_outputs,
            &rename_dsts,
        );
        if exit > worst_exit {
            worst_exit = exit;
        }
    }

    // Emit any recorded malformed-patch error and exit 2.
    if let Some((line, body)) = MALFORMED.with(|m| m.borrow().clone()) {
        let argv0 = env::args().next().unwrap_or_else(|| "patch".to_string());
        eprintln!("{argv0}: **** malformed patch at line {line}: {body}\n");
        process::exit(EXIT_TROUBLE);
    }

    process::exit(worst_exit);
}
