//! `udevadm verify` - validate udev rules files.
//!
//! This is a faithful port of upstream `src/udev/udev-rules.c` validation logic
//! (the `extra_checks` path) plus the `src/udev/udevadm-verify.c` CLI. It parses
//! each rules file, reports syntax / warning / style issues with byte-exact
//! diagnostic messages, prints a per-run summary, and exits non-zero when any
//! file has issues.
//!
//! The goal is message-for-message compatibility with upstream `udevadm verify`,
//! because the integration test (`test/units/TEST-17-UDEV.verify.sh`) diffs the
//! output exactly.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Maximum length of a single (logical) rules line. Mirrors upstream
/// `UDEV_LINE_SIZE`.
const UDEV_LINE_SIZE: usize = 16384;

/// Issue severity bits. Upstream buckets issues by syslog level number and the
/// CLI treats `LOG_ERR | LOG_WARNING` as "check failed" and `LOG_NOTICE` as a
/// style issue. We mirror the same three levels.
const ISSUE_ERR: u32 = 1 << 3; // LOG_ERR
const ISSUE_WARNING: u32 = 1 << 4; // LOG_WARNING
const ISSUE_NOTICE: u32 = 1 << 5; // LOG_NOTICE (style)

#[derive(Clone, Copy, PartialEq, Eq)]
enum Timing {
    Never,
    Late,
    Early,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Match,
    Nomatch,
    Add,
    Remove,
    Assign,
    AssignFinal,
}

impl Op {
    fn is_match(self) -> bool {
        matches!(self, Op::Match | Op::Nomatch)
    }
}

/// Match classification used for conflict / duplicate detection. Mirrors
/// upstream `UdevRuleMatchType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MatchType {
    Empty,
    Plain,
    PlainWithEmpty,
    Glob,
    GlobWithEmpty,
    Subsystem,
}

/// A parsed match/assign token, retaining only the fields needed for the
/// cross-token checks (conflicts, duplicates, RESULT-after-PROGRAM, and the
/// "line has no effect" determination).
struct Token {
    /// Numeric type id, distinct per key kind; used for equality and ordering.
    type_id: u32,
    is_result: bool,
    is_program: bool,
    op: Op,
    match_type: MatchType,
    /// `|`-split alternatives (for nulstr-valued match tokens); otherwise a
    /// single-element vec holding the raw value.
    values: Vec<String>,
    has_nulstr: bool,
    /// Attribute data for string-data tokens (ENV/ATTR/SYSCTL/...); compared in
    /// token equality.
    data: Option<String>,
}

/// A single logical rule line and the state accumulated while parsing it.
struct RuleLine {
    line_number: usize,
    tokens: Vec<Token>,
    has_label: bool,
    is_referenced: bool,
    label: Option<String>,
    has_goto: bool,
    goto_label: Option<String>,
    /// Whether any token gave the line an effect (assignment / program / import
    /// / name / devlink / static node / goto / label).
    has_effect: bool,
    /// Set once the line is dropped by GOTO resolution so later passes skip it.
    dropped: bool,
}

/// Accumulates diagnostics and the issue bitmask for one file.
struct FileCtx {
    filename: String,
    issues: u32,
    out: Vec<u8>,
    timing: Timing,
    sys_uid_max: u32,
    sys_gid_max: u32,
}

impl FileCtx {
    fn emit(&mut self, level: u32, line_nr: usize, msg: &str) {
        self.issues |= level;
        let _ = writeln!(self.out, "{}:{} {}", self.filename, line_nr, msg);
    }
    fn error(&mut self, line_nr: usize, msg: &str) {
        self.emit(ISSUE_ERR, line_nr, msg);
    }
    fn warning(&mut self, line_nr: usize, msg: &str) {
        self.emit(ISSUE_WARNING, line_nr, msg);
    }
    fn notice(&mut self, line_nr: usize, msg: &str) {
        self.emit(ISSUE_NOTICE, line_nr, msg);
    }
}

// --------------------------------------------------------------------------
// CLI entry point
// --------------------------------------------------------------------------

/// Parsed command-line options for `udevadm verify`.
struct Args {
    timing: Timing,
    root: Option<String>,
    summary: bool,
    style: bool,
    files: Vec<String>,
}

/// Entry point invoked from `main()` for `udevadm verify ...`. `args` is the
/// argument vector *after* the `verify` subcommand token.
pub fn verify_main(args: &[String]) -> ExitCode {
    let parsed = match parse_argv(args) {
        ParseResult::Ok(a) => a,
        ParseResult::ExitOk => return ExitCode::SUCCESS,
        ParseResult::ExitErr => return ExitCode::from(1),
    };

    let files = match collect_files(&parsed) {
        Ok(f) => f,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("{msg}");
            }
            return ExitCode::from(1);
        }
    };

    let (sys_uid_max, sys_gid_max) = read_system_id_maxes();

    let mut fail_count = 0usize;
    let mut success_count = 0usize;
    let stderr = std::io::stderr();
    let stdout = std::io::stdout();

    for (fs_path, display) in &files {
        let mut ctx = FileCtx {
            filename: display.clone(),
            issues: 0,
            out: Vec::new(),
            timing: parsed.timing,
            sys_uid_max,
            sys_gid_max,
        };
        let parse_ok = verify_file(fs_path, &mut ctx);

        // Flush accumulated per-line diagnostics.
        {
            let mut lock = stderr.lock();
            let _ = lock.write_all(&ctx.out);
        }

        let ok = match parse_ok {
            Err(errno_msg) => {
                // Failed to even parse (e.g. ENOBUFS on an over-long line).
                eprintln!("Failed to parse rules file {display}: {errno_msg}");
                false
            }
            Ok(()) => {
                let mask = ISSUE_ERR | ISSUE_WARNING;
                if ctx.issues & mask != 0 {
                    eprintln!("{display}: udev rules check failed.");
                    false
                } else if parsed.style && (ctx.issues & ISSUE_NOTICE) != 0 {
                    eprintln!("{display}: udev rules have style issues.");
                    false
                } else {
                    true
                }
            }
        };

        if ok {
            success_count += 1;
        } else {
            fail_count += 1;
        }
    }

    if parsed.summary {
        let mut lock = stdout.lock();
        let _ = write!(
            lock,
            "\n{} udev rules files have been checked.\n  Success: {}\n  Fail:    {}\n",
            fail_count + success_count,
            success_count,
            fail_count
        );
    }

    if fail_count > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

enum ParseResult {
    Ok(Args),
    ExitOk,
    ExitErr,
}

fn parse_argv(args: &[String]) -> ParseResult {
    let mut timing = Timing::Early;
    let mut root: Option<String> = None;
    let mut summary = true;
    let mut style = true;
    let mut files: Vec<String> = Vec::new();

    let mut i = 0;
    let mut no_more_opts = false;
    while i < args.len() {
        let a = &args[i];
        if no_more_opts || a == "-" || !a.starts_with('-') {
            files.push(a.clone());
            i += 1;
            continue;
        }
        if a == "--" {
            no_more_opts = true;
            i += 1;
            continue;
        }

        // Long options.
        if let Some(rest) = a.strip_prefix("--") {
            let (name, inline_val) = match rest.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (rest, None),
            };
            match name {
                "help" => {
                    print_help();
                    return ParseResult::ExitOk;
                }
                "version" => {
                    print_version();
                    return ParseResult::ExitOk;
                }
                "resolve-names" => {
                    let val = match inline_val {
                        Some(v) => v,
                        None => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!("udevadm: option '--resolve-names' requires an argument");
                                return ParseResult::ExitErr;
                            }
                            args[i].clone()
                        }
                    };
                    match parse_resolve_timing(&val) {
                        ResolveParse::Value(t) => timing = t,
                        ResolveParse::Help => return ParseResult::ExitOk,
                        ResolveParse::Invalid => return ParseResult::ExitErr,
                    }
                }
                "root" => {
                    let val = match inline_val {
                        Some(v) => v,
                        None => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!("udevadm: option '--root' requires an argument");
                                return ParseResult::ExitErr;
                            }
                            args[i].clone()
                        }
                    };
                    root = Some(val);
                }
                "no-summary" => summary = false,
                "no-style" => style = false,
                _ => {
                    eprintln!("udevadm: unrecognized option '--{name}'");
                    return ParseResult::ExitErr;
                }
            }
            i += 1;
            continue;
        }

        // Short options. `-h`/`-V` exit immediately, so there is no valid
        // bundling; `-N` optionally takes its value from the rest of the arg
        // (`-Nearly`) or the following arg (`-N early`).
        let chars: Vec<char> = a[1..].chars().collect();
        match chars.first() {
            Some('h') => {
                print_help();
                return ParseResult::ExitOk;
            }
            Some('V') => {
                print_version();
                return ParseResult::ExitOk;
            }
            Some('N') => {
                let val: String = if chars.len() > 1 {
                    chars[1..].iter().collect()
                } else {
                    i += 1;
                    if i >= args.len() {
                        eprintln!("udevadm: option requires an argument -- 'N'");
                        return ParseResult::ExitErr;
                    }
                    args[i].clone()
                };
                match parse_resolve_timing(&val) {
                    ResolveParse::Value(t) => timing = t,
                    ResolveParse::Help => return ParseResult::ExitOk,
                    ResolveParse::Invalid => return ParseResult::ExitErr,
                }
            }
            other => {
                let c = other.copied().unwrap_or('-');
                eprintln!("udevadm: invalid option -- '{c}'");
                return ParseResult::ExitErr;
            }
        }
        i += 1;
    }

    ParseResult::Ok(Args {
        timing,
        root,
        summary,
        style,
        files,
    })
}

enum ResolveParse {
    Value(Timing),
    Help,
    Invalid,
}

fn parse_resolve_timing(s: &str) -> ResolveParse {
    match s {
        "early" => ResolveParse::Value(Timing::Early),
        "late" => ResolveParse::Value(Timing::Late),
        "never" => ResolveParse::Value(Timing::Never),
        "help" => {
            println!("early\nlate\nnever");
            ResolveParse::Help
        }
        _ => {
            eprintln!("udevadm: --resolve-names= takes \"early\", \"late\", or \"never\"");
            ResolveParse::Invalid
        }
    }
}

fn print_help() {
    println!(
        "udevadm verify [OPTIONS] [FILE...]\n\n\
         Verify udev rules files.\n\n  \
         -h --help                            Show this help\n  \
         -V --version                         Show package version\n  \
         -N --resolve-names=early|late|never  When to resolve names\n     \
         --root=PATH                       Operate on an alternate filesystem root\n     \
         --no-summary                      Do not show summary\n     \
         --no-style                        Ignore style issues"
    );
}

fn print_version() {
    println!("systemd {} (udevadm)", env!("CARGO_PKG_VERSION"));
}

// --------------------------------------------------------------------------
// File discovery
// --------------------------------------------------------------------------

/// Standard udev rules directories, relative to (an optional) root. Mirrors
/// `CONF_PATHS_STRV("udev/rules.d")`.
const RULES_SUBDIRS: [&str; 4] = [
    "etc/udev/rules.d",
    "run/udev/rules.d",
    "usr/local/lib/udev/rules.d",
    "usr/lib/udev/rules.d",
];

/// Returns the list of `(filesystem_path, display_path)` rules files to check.
/// `display_path` is what appears in diagnostics.
fn collect_files(args: &Args) -> Result<Vec<(PathBuf, String)>, String> {
    if args.files.is_empty() {
        // No positional args: scan the standard directories (under root, if any).
        let mut out: Vec<(PathBuf, String)> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let root = args.root.as_deref().unwrap_or("");
        let mut names: Vec<(String, PathBuf)> = Vec::new();
        for sub in RULES_SUBDIRS {
            let dir = if root.is_empty() {
                PathBuf::from("/").join(sub)
            } else {
                Path::new(root).join(sub)
            };
            for (name, path) in list_rules_in_dir(&dir) {
                if seen.insert(name.clone()) {
                    names.push((name, path));
                }
            }
        }
        names.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, path) in names {
            let display = path.to_string_lossy().into_owned();
            out.push((path, display));
        }
        if args.root.is_some() && out.is_empty() {
            return Err(format!("No rules files found in '{root}'."));
        }
        Ok(out)
    } else {
        let mut out: Vec<(PathBuf, String)> = Vec::new();
        for s in &args.files {
            search_rules_file(s, args.root.as_deref(), &mut out)?;
        }
        Ok(out)
    }
}

/// Resolve a single positional argument (a file or a directory) into rules files.
fn search_rules_file(
    s: &str,
    root: Option<&str>,
    out: &mut Vec<(PathBuf, String)>,
) -> Result<(), String> {
    // A bare basename (e.g. `99-foo.rules`) is first looked up in the standard
    // rules directories under root, mirroring upstream
    // `search_rules_file_in_conf_dirs`.
    if search_rules_file_in_conf_dirs(s, root, out) {
        return Ok(());
    }

    // Resolve against root when given, else against CWD (absolutized so the
    // display path matches upstream's `$(pwd)/name`).
    let fs_path = if let Some(r) = root {
        Path::new(r).join(s.trim_start_matches('/'))
    } else {
        absolutize(Path::new(s))
    };

    let meta = std::fs::metadata(&fs_path);
    match meta {
        Ok(m) if m.is_dir() => {
            let mut entries = list_rules_in_dir(&fs_path);
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (_, path) in entries {
                let display = path.to_string_lossy().into_owned();
                out.push((path, display));
            }
            Ok(())
        }
        Ok(_) => {
            let display = fs_path.to_string_lossy().into_owned();
            out.push((fs_path, display));
            Ok(())
        }
        Err(e) => Err(format!(
            "Failed to parse rules file {}: {}",
            display_for(s, &fs_path),
            errno_string(&e)
        )),
    }
}

/// Look up a bare basename in the standard rules directories under `root`.
/// Returns `true` if it was found and pushed. Mirrors upstream
/// `search_rules_file_in_conf_dirs`: paths (containing `/`) are not handled here.
fn search_rules_file_in_conf_dirs(
    s: &str,
    root: Option<&str>,
    out: &mut Vec<(PathBuf, String)>,
) -> bool {
    if s.is_empty() || s.contains('/') {
        return false;
    }
    let name = if s.ends_with(".rules") {
        s.to_string()
    } else {
        format!("{s}.rules")
    };
    if name == "." || name == ".." || name.contains('\0') {
        return false;
    }
    for sub in RULES_SUBDIRS {
        let cand = match root {
            Some(r) => Path::new(r).join(sub).join(&name),
            None => PathBuf::from("/").join(sub).join(&name),
        };
        if std::fs::metadata(&cand).is_ok_and(|m| m.is_file()) {
            let display = cand.to_string_lossy().into_owned();
            out.push((cand, display));
            return true;
        }
    }
    false
}

/// Choose the nicer display string: keep the user's spelling for relative paths
/// with no root (matches upstream messages like `./nosuchfile`).
fn display_for(orig: &str, fs_path: &Path) -> String {
    if orig.starts_with('/') || orig.starts_with("./") || orig.starts_with("../") {
        orig.to_string()
    } else {
        fs_path.to_string_lossy().into_owned()
    }
}

/// List `*.rules` regular files in a directory as `(basename, path)`.
fn list_rules_in_dir(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return out,
    };
    for ent in rd.flatten() {
        let path = ent.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !name.ends_with(".rules") {
            continue;
        }
        // Follow symlinks; only accept regular files (a symlink loop pointing at
        // a directory must be skipped, not descended).
        match std::fs::metadata(&path) {
            Ok(m) if m.is_file() => out.push((name, path)),
            _ => {}
        }
    }
    out
}

fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(p),
            Err(_) => p.to_path_buf(),
        }
    }
}

fn errno_string(e: &std::io::Error) -> String {
    // Match the classic strerror spellings the test cares about.
    match e.raw_os_error() {
        Some(libc::ENOENT) => "No such file or directory".to_string(),
        Some(libc::ENOBUFS) => "No buffer space available".to_string(),
        Some(libc::EISDIR) => "Is a directory".to_string(),
        _ => e.to_string(),
    }
}

fn read_system_id_maxes() -> (u32, u32) {
    let mut uid_max = 999u32;
    let mut gid_max = 999u32;
    if let Ok(f) = std::fs::File::open("/etc/login.defs") {
        for line in BufReader::new(f).lines().map_while(Result::ok) {
            let mut it = line.split_whitespace();
            match (it.next(), it.next()) {
                (Some("SYS_UID_MAX"), Some(v)) => {
                    if let Ok(n) = v.parse() {
                        uid_max = n;
                    }
                }
                (Some("SYS_GID_MAX"), Some(v)) => {
                    if let Ok(n) = v.parse() {
                        gid_max = n;
                    }
                }
                _ => {}
            }
        }
    }
    (uid_max, gid_max)
}

// --------------------------------------------------------------------------
// File reading (line continuation + buffer-size handling)
// --------------------------------------------------------------------------

/// Parse and validate one rules file. Returns `Err(errno_message)` if the file
/// could not be read at all (e.g. an over-long line -> ENOBUFS).
fn verify_file(path: &Path, ctx: &mut FileCtx) -> Result<(), String> {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return Err(errno_string(&e)),
    };
    let mut raw = Vec::new();
    if let Err(e) = file.read_to_end(&mut raw) {
        return Err(errno_string(&e));
    }
    let content = String::from_utf8_lossy(&raw).into_owned();

    let mut lines: Vec<RuleLine> = Vec::new();
    let mut continuation: Option<String> = None;
    let mut line_nr = 0usize;
    let mut current_line_nr = 0usize;
    let mut ignore_line = false;

    for raw_line in content.split_inclusive('\n') {
        // A physical line longer than UDEV_LINE_SIZE is a hard read failure
        // (read_line returns -ENOBUFS upstream).
        let physical = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let physical = physical.strip_suffix('\r').unwrap_or(physical);
        if physical.len() >= UDEV_LINE_SIZE {
            return Err("No buffer space available".to_string());
        }

        current_line_nr += 1;
        if continuation.is_none() {
            line_nr = current_line_nr;
        }

        let line = physical.trim_start_matches([' ', '\t']);

        // Comment lines are ignored regardless of continuation state.
        if line.starts_with('#') {
            continue;
        }

        // `work` mirrors upstream's `line`: after a continuation join it aliases
        // the continuation buffer, so a trailing backslash must be stripped from
        // both in lock-step.
        let mut work = line.to_string();
        let mut len = work.len();

        if continuation.is_some() && !ignore_line {
            let cont = continuation.as_ref().unwrap();
            if cont.len() + len >= UDEV_LINE_SIZE {
                ignore_line = true;
            }
            let mut joined = continuation.take().unwrap();
            joined.push_str(&work);
            continuation = Some(joined);
            if !ignore_line {
                work = continuation.clone().unwrap();
                len = work.len();
            }
        }

        if len > 0 && work.ends_with('\\') {
            if ignore_line {
                continue;
            }
            work.pop(); // strip trailing backslash from the (aliased) line
            match continuation.take() {
                None => continuation = Some(work.clone()),
                Some(mut c) => {
                    c.pop(); // continuation aliases `work`: strip its backslash too
                    continuation = Some(c);
                }
            }
            continue;
        }

        if ignore_line {
            ctx.error(line_nr, "Line is too long, ignored.");
        } else if len > 0 {
            add_line(ctx, &work, line_nr, &mut lines);
        }

        continuation = None;
        ignore_line = false;
    }

    if continuation.is_some() {
        ctx.error(
            line_nr,
            "Unexpected EOF after line continuation, line ignored.",
        );
    }

    resolve_goto(ctx, &mut lines);
    for line in &lines {
        if line.dropped {
            continue;
        }
        check_unused_labels(ctx, line);
        check_conflicts_duplicates(ctx, line);
    }

    Ok(())
}

// --------------------------------------------------------------------------
// Line parsing
// --------------------------------------------------------------------------

fn add_line(ctx: &mut FileCtx, line: &str, line_nr: usize, lines: &mut Vec<RuleLine>) {
    let mut rl = RuleLine {
        line_number: line_nr,
        tokens: Vec::new(),
        has_label: false,
        is_referenced: false,
        label: None,
        has_goto: false,
        goto_label: None,
        has_effect: false,
        dropped: false,
    };

    let bytes = line.as_bytes();
    let mut pos = 0usize;
    let full_start = pos;
    loop {
        // Skip to the next token boundary for the style check, mirroring
        // `check_token_delimiters` which is fed the current cursor.
        check_token_delimiters(ctx, line, pos, full_start, line_nr);

        match parse_line(bytes, pos) {
            ParseLine::Empty => break,
            ParseLine::Invalid => {
                ctx.error(line_nr, "Invalid key/value pair, ignoring.");
                return;
            }
            ParseLine::Token {
                key,
                attr,
                op,
                value,
                is_case_insensitive,
                next,
            } => {
                if parse_token(
                    ctx,
                    &mut rl,
                    &key,
                    attr.as_deref(),
                    op,
                    &value,
                    is_case_insensitive,
                )
                .is_err()
                {
                    return;
                }
                pos = next;
            }
        }
    }

    if !rl.has_effect && !rl.has_goto && !rl.has_label {
        ctx.warning(line_nr, "The line has no effect, ignoring.");
        return;
    }

    check_tokens_order(ctx, &rl);
    lines.push(rl);
}

enum ParseLine {
    Empty,
    Invalid,
    Token {
        key: String,
        attr: Option<String>,
        op: Op,
        value: String,
        is_case_insensitive: bool,
        next: usize,
    },
}

fn is_ws(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\n' || b == b'\r'
}

fn parse_operator(bytes: &[u8], pos: usize) -> Option<(Op, usize)> {
    let rest = &bytes[pos..];
    if rest.starts_with(b"==") {
        Some((Op::Match, 2))
    } else if rest.starts_with(b"!=") {
        Some((Op::Nomatch, 2))
    } else if rest.starts_with(b"+=") {
        Some((Op::Add, 2))
    } else if rest.starts_with(b"-=") {
        Some((Op::Remove, 2))
    } else if rest.starts_with(b":=") {
        Some((Op::AssignFinal, 2))
    } else if rest.starts_with(b"=") {
        Some((Op::Assign, 1))
    } else {
        None
    }
}

/// Port of upstream `parse_line`: extract key, optional `{attr}`, operator and
/// quoted value starting at `start`. Returns the next cursor position.
fn parse_line(bytes: &[u8], start: usize) -> ParseLine {
    // Skip leading whitespace and commas.
    let mut kb = start;
    while kb < bytes.len() && (is_ws(bytes[kb]) || bytes[kb] == b',') {
        kb += 1;
    }
    if kb >= bytes.len() {
        return ParseLine::Empty;
    }

    // Scan the key up to whitespace, '=', '{', or an operator prefix.
    let mut ke = kb;
    loop {
        if ke >= bytes.len() {
            return ParseLine::Invalid;
        }
        let c = bytes[ke];
        if is_ws(c) || c == b'=' || c == b'{' {
            break;
        }
        if matches!(c, b'+' | b'-' | b'!' | b':') && ke + 1 < bytes.len() && bytes[ke + 1] == b'=' {
            break;
        }
        ke += 1;
    }

    let key = String::from_utf8_lossy(&bytes[kb..ke]).into_owned();

    // Optional {attr}.
    let mut attr: Option<String> = None;
    let mut after = ke;
    if bytes[ke] == b'{' {
        let astart = ke + 1;
        let mut aend = astart;
        while aend < bytes.len() && bytes[aend] != b'}' {
            aend += 1;
        }
        if aend >= bytes.len() {
            return ParseLine::Invalid;
        }
        attr = Some(String::from_utf8_lossy(&bytes[astart..aend]).into_owned());
        after = aend + 1;
    }

    // Skip whitespace before the operator.
    let mut op_pos = after;
    while op_pos < bytes.len() && is_ws(bytes[op_pos]) {
        op_pos += 1;
    }
    let (op, op_len) = match parse_operator(bytes, op_pos) {
        Some(v) => v,
        None => return ParseLine::Invalid,
    };

    // Skip whitespace before the value.
    let mut vpos = op_pos + op_len;
    while vpos < bytes.len() && is_ws(bytes[vpos]) {
        vpos += 1;
    }

    match parse_value(bytes, vpos) {
        Some((value, is_ci, next)) => ParseLine::Token {
            key,
            attr,
            op,
            value,
            is_case_insensitive: is_ci,
            next,
        },
        None => ParseLine::Invalid,
    }
}

/// Port of upstream `udev_rule_parse_value`: handle the optional `e`/`i`
/// prefixes and the double-quoted, backslash-escaped value. Returns
/// `(value, is_case_insensitive, next_pos)`.
fn parse_value(bytes: &[u8], start: usize) -> Option<(String, bool, usize)> {
    let mut is_escaped = false;
    let mut is_ci = false;
    let mut p = start;
    // Up to two prefix chars before the opening quote.
    let mut k = start;
    while k < bytes.len() && bytes[k] != b'"' && k < start + 2 {
        match bytes[k] {
            b'e' if !is_escaped => is_escaped = true,
            b'i' if !is_ci => is_ci = true,
            _ => return None,
        }
        k += 1;
    }
    p += (is_escaped as usize) + (is_ci as usize);
    if p >= bytes.len() || bytes[p] != b'"' {
        return None;
    }
    // Scan to the closing quote, honoring backslash escapes.
    let mut i = p + 1;
    let mut value = Vec::new();
    while i < bytes.len() && bytes[i] != b'"' {
        if bytes[i] == b'\\' {
            if is_escaped {
                // Keep the escape sequence's second char literally-ish; for
                // validation purposes the exact unescape is not needed, but we
                // do need to not stop at an escaped quote.
                if i + 1 < bytes.len() {
                    value.push(bytes[i]);
                    value.push(bytes[i + 1]);
                    i += 2;
                    continue;
                }
                return None;
            } else {
                // Non-escaped mode: a backslash before a quote escapes it.
                if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                    value.push(b'"');
                    i += 2;
                    continue;
                }
                value.push(bytes[i]);
                i += 1;
                continue;
            }
        }
        value.push(bytes[i]);
        i += 1;
    }
    if i >= bytes.len() {
        return None; // no closing quote
    }
    let s = String::from_utf8_lossy(&value).into_owned();
    Some((s, is_ci, i + 1))
}

/// Port of `check_token_delimiters` (style checks around commas / whitespace).
fn check_token_delimiters(
    ctx: &mut FileCtx,
    line: &str,
    pos: usize,
    full_start: usize,
    line_nr: usize,
) {
    let bytes = line.as_bytes();
    let mut n_comma = 0usize;
    let mut ws_before_comma = false;
    let mut ws_after_comma = false;
    let mut p = pos;
    while p < bytes.len() {
        let c = bytes[p];
        if c == b',' {
            n_comma += 1;
        } else if is_ws(c) {
            if n_comma > 0 {
                ws_after_comma = true;
            } else {
                ws_before_comma = true;
            }
        } else {
            break;
        }
        p += 1;
    }

    if pos == full_start {
        // First token of the rule.
        if n_comma > 0 {
            ctx.notice(line_nr, "style: stray leading comma.");
        }
    } else if p >= bytes.len() {
        // No more tokens on the rule.
        if n_comma > 0 {
            ctx.notice(line_nr, "style: stray trailing comma.");
        }
    } else {
        if n_comma == 0 {
            ctx.notice(line_nr, "style: a comma between tokens is expected.");
        } else if n_comma > 1 {
            ctx.notice(line_nr, "style: more than one comma between tokens.");
        }
        if n_comma > 0 {
            if ws_before_comma {
                ctx.notice(line_nr, "style: stray whitespace before comma.");
            }
            if !ws_after_comma {
                ctx.notice(line_nr, "style: whitespace after comma is expected.");
            }
        } else if !ws_before_comma && !ws_after_comma {
            ctx.notice(line_nr, "style: whitespace between tokens is expected.");
        }
    }
}

fn check_tokens_order(ctx: &mut FileCtx, rl: &RuleLine) {
    let mut has_result = false;
    for t in &rl.tokens {
        if t.is_result {
            has_result = true;
        } else if has_result && t.is_program {
            ctx.warning(
                rl.line_number,
                "Reordering RESULT check after PROGRAM assignment.",
            );
            break;
        }
    }
}

// --------------------------------------------------------------------------
// Substitution format checker (udev_check_format)
// --------------------------------------------------------------------------

/// The `%x` / `$name` substitution names/chars udev understands.
const SUBST_MAP: &[(&str, char, u8)] = &[
    // name, fmt char, kind: 0=other, 1=attr(s), 2=env(E), 3=result(c)
    ("devnode", 'N', 0),
    ("tempnode", 'N', 0),
    ("attr", 's', 1),
    ("sysfs", 's', 1),
    ("env", 'E', 2),
    ("kernel", 'k', 0),
    ("number", 'n', 0),
    ("driver", 'd', 0),
    ("devpath", 'p', 0),
    ("id", 'b', 0),
    ("major", 'M', 0),
    ("minor", 'm', 0),
    ("result", 'c', 3),
    ("parent", 'P', 0),
    ("name", 'D', 0),
    ("links", 'L', 0),
    ("root", 'r', 0),
    ("sys", 'S', 0),
];

/// Returns `Err((offset, hint))` on the first invalid substitution, mirroring
/// `udev_check_format`. `offset` is the 0-based byte position of the offending
/// sigil; callers report `offset + 1` as the char position.
fn udev_check_format(value: &str) -> Result<(), (usize, &'static str)> {
    let bytes = value.as_bytes();
    let mut s = 0usize;
    while s < bytes.len() {
        match get_subst_type(bytes, s) {
            SubstResult::None => {
                s += 1;
            }
            SubstResult::Escaped(second_sigil) => {
                // `%%` / `$$`: upstream points past the first sigil then advances
                // once more, skipping both characters.
                s = second_sigil + 1;
            }
            SubstResult::Invalid => {
                return Err((s, "invalid substitution type"));
            }
            SubstResult::Ok { kind, attr, next } => {
                if (kind == 1 || kind == 2) && attr.is_empty() {
                    return Err((s, "attribute value missing"));
                }
                if kind == 3 && !attr.is_empty() {
                    // RESULT index: optional number with optional trailing '+'.
                    let core = attr.strip_suffix('+').unwrap_or(&attr);
                    if core.is_empty() || !core.bytes().all(|b| b.is_ascii_digit()) {
                        return Err((s, "attribute value not a valid number"));
                    }
                }
                s = next;
            }
        }
    }
    Ok(())
}

enum SubstResult {
    None,
    Escaped(usize),
    Invalid,
    Ok { kind: u8, attr: String, next: usize },
}

/// Port of `get_subst_type` (strict mode).
fn get_subst_type(bytes: &[u8], pos: usize) -> SubstResult {
    let c = bytes[pos];
    let (is_dollar, mut q, kind) = if c == b'$' {
        // $$ is a literal.
        if pos + 1 < bytes.len() && bytes[pos + 1] == b'$' {
            return SubstResult::Escaped(pos + 1);
        }
        // Match a name.
        let after = pos + 1;
        let mut found: Option<(usize, u8)> = None;
        for (name, _fmt, kind) in SUBST_MAP {
            if bytes[after..].starts_with(name.as_bytes()) {
                found = Some((after + name.len(), *kind));
                break;
            }
        }
        match found {
            Some((q, kind)) => (true, q, kind),
            None => return SubstResult::Invalid,
        }
    } else if c == b'%' {
        if pos + 1 < bytes.len() && bytes[pos + 1] == b'%' {
            return SubstResult::Escaped(pos + 1);
        }
        let after = pos + 1;
        if after >= bytes.len() {
            return SubstResult::Invalid;
        }
        let mut found: Option<(usize, u8)> = None;
        for (_name, fmt, kind) in SUBST_MAP {
            if bytes[after] == *fmt as u8 {
                found = Some((after + 1, *kind));
                break;
            }
        }
        match found {
            Some((q, kind)) => (false, q, kind),
            None => return SubstResult::Invalid,
        }
    } else {
        return SubstResult::None;
    };
    let _ = is_dollar;

    // Optional {attr}.
    let mut attr = String::new();
    if q < bytes.len() && bytes[q] == b'{' {
        let start = q + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != b'}' {
            end += 1;
        }
        if end >= bytes.len() {
            return SubstResult::Invalid; // unterminated -> invalid substitution type
        }
        let len = end - start;
        if len == 0 {
            // Empty braces: attr stays empty; caller flags attr-missing for
            // attr/env kinds, matches upstream (which returns type + empty attr).
        }
        attr = String::from_utf8_lossy(&bytes[start..end]).into_owned();
        q = end + 1;
    }

    SubstResult::Ok {
        kind,
        attr,
        next: q,
    }
}

// --------------------------------------------------------------------------
// The per-key token validator (parse_token / rule_line_add_token)
// --------------------------------------------------------------------------

/// Result of validating one key/op/value token. `Err(())` means a hard error
/// was emitted and the whole line must be abandoned.
type TokResult = Result<(), ()>;

// Distinct type ids for tokens, in the same relative order as upstream so that
// `type_has_nulstr_value` (type < TK_M_TEST || type == TK_M_RESULT) can be
// modeled. Match keys occupy the low range; TEST is the boundary.
mod tid {
    pub const ACTION: u32 = 0;
    pub const DEVPATH: u32 = 1;
    pub const KERNEL: u32 = 2;
    pub const DEVLINK: u32 = 3; // SYMLINK match
    pub const NAME_M: u32 = 4;
    pub const ENV: u32 = 5;
    pub const CONST: u32 = 6;
    pub const TAG: u32 = 7;
    pub const SUBSYSTEM: u32 = 8;
    pub const DRIVER: u32 = 9;
    pub const ATTR: u32 = 10;
    pub const SYSCTL: u32 = 11;
    pub const KERNELS: u32 = 12;
    pub const SUBSYSTEMS: u32 = 13;
    pub const DRIVERS: u32 = 14;
    pub const ATTRS: u32 = 15;
    pub const TAGS: u32 = 16;
    pub const TEST: u32 = 17; // boundary: nulstr = type < TEST || RESULT
    pub const PROGRAM: u32 = 18;
    pub const IMPORT: u32 = 19;
    pub const RESULT: u32 = 20;
    // Assign tokens (all give the line an effect).
    pub const A_DEVLINK: u32 = 100;
    pub const A_NAME: u32 = 101;
    pub const A_ENV: u32 = 102;
    pub const A_TAG: u32 = 103;
    pub const A_ATTR: u32 = 104;
    pub const A_SYSCTL: u32 = 105;
    pub const A_OPTIONS: u32 = 106;
    pub const A_OWNER: u32 = 107;
    pub const A_GROUP: u32 = 108;
    pub const A_MODE: u32 = 109;
    pub const A_SECLABEL: u32 = 110;
    pub const A_RUN: u32 = 111;
}

fn type_has_nulstr(type_id: u32) -> bool {
    type_id < tid::TEST || type_id == tid::RESULT
}

/// ENV property names that cannot be assigned (read-only device properties).
const ENV_READONLY: &[&str] = &[
    "ACTION",
    "SEQNUM",
    "SYNTH_UUID",
    "DEVPATH",
    "DEVPATH_OLD",
    "SUBSYSTEM",
    "DEVTYPE",
    "DRIVER",
    "MODALIAS",
    "DEVNAME",
    "DEVMODE",
    "DEVUID",
    "DEVGID",
    "MAJOR",
    "MINOR",
    "DISKSEQ",
    "PARTN",
    "IFINDEX",
    "INTERFACE",
    "INTERFACE_OLD",
    "DEVLINKS",
    "TAGS",
    "CURRENT_TAGS",
    "USEC_INITIALIZED",
    "UDEV_DATABASE_VERSION",
];

fn device_property_can_set(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if name.starts_with("SYNTH_ARG_") {
        return false;
    }
    !ENV_READONLY.contains(&name)
}

const BUILTINS: &[&str] = &[
    "blkid",
    "btrfs",
    "dissect_image",
    "factory_reset",
    "hwdb",
    "input_id",
    "keyboard",
    "kmod",
    "net_driver",
    "net_id",
    "net_setup_link",
    "path_id",
    "tpm2_id",
    "usb_id",
    "uaccess",
];

fn builtin_known(name: &str) -> bool {
    BUILTINS.contains(&name)
}

fn string_is_glob(s: &str) -> bool {
    s.bytes().any(|b| matches!(b, b'*' | b'?' | b'['))
}

/// Compute match type + nulstr alternatives for a match token, applying the
/// `?*` -> `!=""` conversion. Returns `(match_type, values, op)`.
fn compute_match(
    type_id: u32,
    is_subsystem: bool,
    value: &str,
    mut op: Op,
) -> (MatchType, Vec<String>, Op) {
    let mut match_type = if is_subsystem && matches!(value, "subsystem" | "bus" | "class") {
        MatchType::Subsystem
    } else if value.is_empty() {
        MatchType::Empty
    } else if value == "?*" {
        op = if op == Op::Match {
            Op::Nomatch
        } else {
            Op::Match
        };
        MatchType::Empty
    } else if string_is_glob(value) {
        MatchType::Glob
    } else {
        MatchType::Plain
    };

    let mut values = vec![value.to_string()];
    if type_has_nulstr(type_id) {
        // Split on '|' into alternatives, tracking whether any empty alternative
        // is present (leading/trailing/double bar).
        let mut alts: Vec<String> = Vec::new();
        let mut empty = false;
        let mut cur = String::new();
        let mut bar = true;
        for ch in value.chars() {
            if ch != '|' {
                cur.push(ch);
                bar = false;
            } else {
                if bar {
                    empty = true;
                } else {
                    alts.push(std::mem::take(&mut cur));
                }
                bar = true;
            }
        }
        if !cur.is_empty() {
            alts.push(cur);
        } else if bar {
            empty = true;
        }
        values = alts;
        if empty {
            if match_type == MatchType::Glob {
                match_type = MatchType::GlobWithEmpty;
            } else if match_type == MatchType::Plain {
                match_type = MatchType::PlainWithEmpty;
            }
        }
    }
    (match_type, values, op)
}

/// Push a match token onto the line.
#[allow(clippy::too_many_arguments)]
fn push_match(
    rl: &mut RuleLine,
    type_id: u32,
    op: Op,
    value: &str,
    data: Option<String>,
    is_subsystem: bool,
    is_program: bool,
    is_result: bool,
) {
    let (match_type, values, op) = compute_match(type_id, is_subsystem, value, op);
    let has_nulstr = type_has_nulstr(type_id);
    rl.tokens.push(Token {
        type_id,
        is_result,
        is_program,
        op,
        match_type,
        values,
        has_nulstr,
        data,
    });
    if is_program {
        rl.has_effect = true;
    }
}

/// Push an assignment token; assignments always give the line an effect. The
/// value is retained so two assignments to the same key with different values
/// are not mistaken for duplicates.
fn push_assign(rl: &mut RuleLine, type_id: u32, value: &str) {
    rl.tokens.push(Token {
        type_id,
        is_result: false,
        is_program: false,
        op: Op::Assign,
        match_type: MatchType::Plain,
        values: vec![value.to_string()],
        has_nulstr: false,
        data: None,
    });
    rl.has_effect = true;
}

/// Validate one key/op/value triple, emitting diagnostics and (on success)
/// recording token state on the line. Mirrors `parse_token`.
fn parse_token(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    attr: Option<&str>,
    op: Op,
    value: &str,
    is_ci: bool,
) -> TokResult {
    let ln = rl.line_number;

    // Global 'i' prefix rule: only valid for match operators.
    if !op.is_match() && is_ci {
        ctx.error(
            ln,
            &format!(
                "Invalid prefix 'i' for '{key}'. The 'i' prefix can be specified only for '==' or '!=' operator."
            ),
        );
        return Err(());
    }

    match key {
        "ACTION" => simple_match(ctx, rl, key, tid::ACTION, attr, op, value),
        "DEVPATH" => simple_match(ctx, rl, key, tid::DEVPATH, attr, op, value),
        "KERNEL" => simple_match(ctx, rl, key, tid::KERNEL, attr, op, value),
        "SYMLINK" => {
            if attr.is_some() {
                return invalid_attr(ctx, ln, key);
            }
            if op.is_match() {
                push_match(rl, tid::DEVLINK, op, value, None, false, false, false);
            } else {
                check_value_format(ctx, ln, key, value, false)?;
                push_assign(rl, tid::A_DEVLINK, value);
            }
            Ok(())
        }
        "NAME" => parse_name(ctx, rl, key, attr, op, value),
        "ENV" => parse_env(ctx, rl, key, attr, op, value),
        "CONST" => {
            match attr {
                Some(a) if a == "arch" || a == "virt" => {}
                _ => return invalid_attr(ctx, ln, key),
            }
            if !op.is_match() {
                return invalid_op(ctx, ln, key);
            }
            push_match(
                rl,
                tid::CONST,
                op,
                value,
                attr.map(|s| s.to_string()),
                false,
                false,
                false,
            );
            Ok(())
        }
        "TAG" => {
            if attr.is_some() {
                return invalid_attr(ctx, ln, key);
            }
            let mut op = op;
            if op == Op::AssignFinal {
                ctx.warning(
                    ln,
                    "TAG key takes '==', '!=', '=', or '+=' operator, assuming '='.",
                );
                op = Op::Assign;
            }
            if op.is_match() {
                push_match(rl, tid::TAG, op, value, None, false, false, false);
            } else {
                check_value_format(ctx, ln, key, value, true)?;
                push_assign(rl, tid::A_TAG, value);
            }
            Ok(())
        }
        "SUBSYSTEM" => {
            if attr.is_some() {
                return invalid_attr(ctx, ln, key);
            }
            if !op.is_match() {
                return invalid_op(ctx, ln, key);
            }
            if value == "bus" || value == "class" {
                ctx.warning(
                    ln,
                    &format!("\"{value}\" must be specified as \"subsystem\"."),
                );
            }
            push_match(rl, tid::SUBSYSTEM, op, value, None, true, false, false);
            Ok(())
        }
        "DRIVER" => simple_match(ctx, rl, key, tid::DRIVER, attr, op, value),
        "ATTR" => parse_attr_like(ctx, rl, key, tid::ATTR, tid::A_ATTR, attr, op, value),
        "SYSCTL" => parse_attr_like(ctx, rl, key, tid::SYSCTL, tid::A_SYSCTL, attr, op, value),
        "KERNELS" => simple_match(ctx, rl, key, tid::KERNELS, attr, op, value),
        "SUBSYSTEMS" => simple_match(ctx, rl, key, tid::SUBSYSTEMS, attr, op, value),
        "DRIVERS" => simple_match(ctx, rl, key, tid::DRIVERS, attr, op, value),
        "ATTRS" => parse_attrs(ctx, rl, key, attr, op, value),
        "TAGS" => simple_match(ctx, rl, key, tid::TAGS, attr, op, value),
        "TEST" => parse_test(ctx, rl, key, attr, op, value, is_ci),
        "PROGRAM" => parse_program(ctx, rl, key, attr, op, value, is_ci),
        "IMPORT" => parse_import(ctx, rl, key, attr, op, value, is_ci),
        "RESULT" => {
            if attr.is_some() {
                return invalid_attr(ctx, ln, key);
            }
            if !op.is_match() {
                return invalid_op(ctx, ln, key);
            }
            push_match(rl, tid::RESULT, op, value, None, false, false, true);
            Ok(())
        }
        "OPTIONS" => parse_options(ctx, rl, key, attr, op, value),
        "OWNER" => parse_owner_group(ctx, rl, key, attr, op, value, true),
        "GROUP" => parse_owner_group(ctx, rl, key, attr, op, value, false),
        "MODE" => parse_mode_key(ctx, rl, key, attr, op, value),
        "SECLABEL" => parse_seclabel(ctx, rl, key, attr, op, value),
        "RUN" => parse_run(ctx, rl, key, attr, op, value),
        "GOTO" => parse_goto(ctx, rl, key, attr, op, value),
        "LABEL" => parse_label(ctx, rl, key, attr, op, value),
        _ => {
            ctx.error(ln, &format!("Invalid key '{key}'."));
            Err(())
        }
    }
}

fn invalid_attr(ctx: &mut FileCtx, ln: usize, key: &str) -> TokResult {
    ctx.error(ln, &format!("Invalid attribute for {key}."));
    Err(())
}

fn invalid_op(ctx: &mut FileCtx, ln: usize, key: &str) -> TokResult {
    ctx.error(ln, &format!("Invalid operator for {key}."));
    Err(())
}

/// Run the value substitution-format check, emitting on failure. `nonempty`
/// makes an empty value an error.
fn check_value_format(
    ctx: &mut FileCtx,
    ln: usize,
    key: &str,
    value: &str,
    nonempty: bool,
) -> TokResult {
    if nonempty && value.is_empty() {
        ctx.error(
            ln,
            &format!("Invalid value \"\" for {key} (char 0: empty value), ignoring."),
        );
        return Ok(()); // non-fatal (logs but continues) upstream
    }
    if let Err((offset, hint)) = udev_check_format(value) {
        ctx.error(
            ln,
            &format!(
                "Invalid value \"{value}\" for {key} (char {}: {hint}), ignoring.",
                offset + 1
            ),
        );
    }
    Ok(())
}

/// A plain match-only key: attribute forbidden, match operators only, no value
/// format check.
fn simple_match(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    type_id: u32,
    attr: Option<&str>,
    op: Op,
    value: &str,
) -> TokResult {
    let ln = rl.line_number;
    if attr.is_some() {
        return invalid_attr(ctx, ln, key);
    }
    if !op.is_match() {
        return invalid_op(ctx, ln, key);
    }
    push_match(rl, type_id, op, value, None, false, false, false);
    Ok(())
}

fn parse_name(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    attr: Option<&str>,
    op: Op,
    value: &str,
) -> TokResult {
    let ln = rl.line_number;
    if attr.is_some() {
        return invalid_attr(ctx, ln, key);
    }
    let mut op = op;
    if op == Op::Remove {
        return invalid_op(ctx, ln, key);
    }
    if op == Op::Add {
        ctx.warning(
            ln,
            "NAME key takes '==', '!=', '=', or ':=' operator, assuming '='.",
        );
        op = Op::Assign;
    }
    if op.is_match() {
        push_match(rl, tid::NAME_M, op, value, None, false, false, false);
        return Ok(());
    }
    if value == "%k" {
        ctx.error(ln, "Ignoring NAME=\"%k\", as it will take no effect.");
        return Err(());
    }
    if value.is_empty() {
        ctx.error(
            ln,
            "Ignoring NAME=\"\", as udev will not delete any network interfaces.",
        );
        return Err(());
    }
    check_value_format(ctx, ln, key, value, false)?;
    push_assign(rl, tid::A_NAME, value);
    Ok(())
}

fn parse_env(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    attr: Option<&str>,
    op: Op,
    value: &str,
) -> TokResult {
    let ln = rl.line_number;
    let attr = match attr {
        Some(a) if !a.is_empty() => a,
        _ => return invalid_attr(ctx, ln, key),
    };
    let mut op = op;
    if op == Op::Remove {
        return invalid_op(ctx, ln, key);
    }
    if op == Op::AssignFinal {
        ctx.warning(
            ln,
            "ENV key takes '==', '!=', '=', or '+=' operator, assuming '='.",
        );
        op = Op::Assign;
    }
    if op.is_match() {
        push_match(
            rl,
            tid::ENV,
            op,
            value,
            Some(attr.to_string()),
            false,
            false,
            false,
        );
        return Ok(());
    }
    if !device_property_can_set(attr) {
        ctx.error(
            ln,
            &format!("Invalid ENV attribute. '{attr}' cannot be set."),
        );
        return Err(());
    }
    check_value_format(ctx, ln, key, value, false)?;
    push_assign(rl, tid::A_ENV, value);
    Ok(())
}

/// ATTR / SYSCTL: attribute is format-checked, match+`=` allowed, `+=`/`:=`
/// soft-reassign to `=`.
#[allow(clippy::too_many_arguments)]
fn parse_attr_like(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    m_type: u32,
    a_type: u32,
    attr: Option<&str>,
    op: Op,
    value: &str,
) -> TokResult {
    let ln = rl.line_number;
    let attr = attr.unwrap_or("");
    if !check_attr_format(ctx, ln, key, attr)? {
        return Err(());
    }
    let mut op = op;
    if op == Op::Remove {
        return invalid_op(ctx, ln, key);
    }
    if op == Op::Add || op == Op::AssignFinal {
        ctx.warning(
            ln,
            &format!("{key} key takes '==', '!=', or '=' operator, assuming '='."),
        );
        op = Op::Assign;
    }
    if op.is_match() {
        push_match(
            rl,
            m_type,
            op,
            value,
            Some(attr.to_string()),
            false,
            false,
            false,
        );
    } else {
        check_value_format(ctx, ln, key, value, false)?;
        push_assign(rl, a_type, value);
    }
    Ok(())
}

/// Check an ATTR/SYSCTL/ATTRS `{attr}`: empty -> hard error (returns Ok(false)),
/// bad substitution -> error but continue (returns Ok(true)).
fn check_attr_format(ctx: &mut FileCtx, ln: usize, key: &str, attr: &str) -> Result<bool, ()> {
    if attr.is_empty() {
        ctx.error(ln, &format!("Invalid attribute for {key}."));
        return Ok(false);
    }
    if let Err((offset, hint)) = udev_check_format(attr) {
        ctx.error(
            ln,
            &format!(
                "Invalid attribute \"{attr}\" for {key} (char {}: {hint}), ignoring.",
                offset + 1
            ),
        );
    }
    Ok(true)
}

fn parse_attrs(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    attr: Option<&str>,
    op: Op,
    value: &str,
) -> TokResult {
    let ln = rl.line_number;
    let attr = attr.unwrap_or("");
    if !check_attr_format(ctx, ln, key, attr)? {
        return Err(());
    }
    if !op.is_match() {
        return invalid_op(ctx, ln, key);
    }
    if attr.starts_with("device/") {
        ctx.warning(ln, "'device' link may not be available in future kernels.");
    }
    if attr.contains("../") {
        ctx.warning(
            ln,
            "Direct reference to parent sysfs directory, may break in future kernels.",
        );
    }
    push_match(
        rl,
        tid::ATTRS,
        op,
        value,
        Some(attr.to_string()),
        false,
        false,
        false,
    );
    Ok(())
}

fn parse_test(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    attr: Option<&str>,
    op: Op,
    value: &str,
    is_ci: bool,
) -> TokResult {
    let ln = rl.line_number;
    if let Some(a) = attr
        && !a.is_empty()
        && parse_mode(a).is_none()
    {
        ctx.error(ln, &format!("Failed to parse mode '{a}': Invalid argument"));
        return Err(());
    }
    check_value_format(ctx, ln, key, value, true)?;
    if !op.is_match() {
        return invalid_op(ctx, ln, key);
    }
    if is_ci {
        ctx.error(ln, &format!("Invalid prefix 'i' for {key}."));
        return Err(());
    }
    push_match(rl, tid::TEST, op, value, None, false, false, false);
    Ok(())
}

fn parse_program(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    attr: Option<&str>,
    op: Op,
    value: &str,
    is_ci: bool,
) -> TokResult {
    let ln = rl.line_number;
    if attr.is_some() {
        return invalid_attr(ctx, ln, key);
    }
    check_value_format(ctx, ln, key, value, true)?;
    if op == Op::Remove {
        return invalid_op(ctx, ln, key);
    }
    if is_ci {
        ctx.error(ln, &format!("Invalid prefix 'i' for {key}."));
        return Err(());
    }
    // PROGRAM assignment forms are silently coerced to match.
    push_match(rl, tid::PROGRAM, Op::Match, value, None, false, true, false);
    Ok(())
}

fn parse_import(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    attr: Option<&str>,
    op: Op,
    value: &str,
    is_ci: bool,
) -> TokResult {
    let ln = rl.line_number;
    let attr = match attr {
        Some(a) if !a.is_empty() => a,
        _ => return invalid_attr(ctx, ln, key),
    };
    check_value_format(ctx, ln, key, value, true)?;
    if op == Op::Remove {
        return invalid_op(ctx, ln, key);
    }
    if is_ci {
        ctx.error(ln, &format!("Invalid prefix 'i' for {key}."));
        return Err(());
    }
    match attr {
        "file" | "program" | "db" | "cmdline" | "parent" => {}
        "builtin" => {
            if !builtin_known(value) {
                ctx.error(ln, &format!("Unknown builtin command: {value}"));
                return Err(());
            }
        }
        _ => return invalid_attr(ctx, ln, key),
    }
    rl.tokens.push(Token {
        type_id: tid::IMPORT,
        is_result: false,
        is_program: false,
        op: Op::Match,
        match_type: MatchType::Plain,
        values: vec![value.to_string()],
        has_nulstr: false,
        data: None,
    });
    rl.has_effect = true;
    Ok(())
}

fn parse_options(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    attr: Option<&str>,
    op: Op,
    value: &str,
) -> TokResult {
    let ln = rl.line_number;
    if attr.is_some() {
        return invalid_attr(ctx, ln, key);
    }
    if op.is_match() || op == Op::Remove {
        return invalid_op(ctx, ln, key);
    }
    // OP_ADD silently becomes assign; ':=' kept.
    const PLAIN_OPTS: &[&str] = &[
        "string_escape=none",
        "string_escape=replace",
        "db_persist",
        "watch",
        "nowatch",
        "dump",
        "dump-json",
    ];
    if PLAIN_OPTS.contains(&value) {
        // Recognized boolean/escape option with no further validation.
    } else if value.strip_prefix("static_node=").is_some() {
        push_assign(rl, tid::A_OPTIONS, value);
        return Ok(());
    } else if let Some(rest) = value.strip_prefix("link_priority=") {
        if rest.parse::<i32>().is_err() {
            ctx.error(
                ln,
                &format!("Failed to parse link priority '{rest}': Invalid argument"),
            );
            return Err(());
        }
    } else if let Some(rest) = value.strip_prefix("log_level=") {
        if rest != "reset" && log_level_from_string(rest).is_none() {
            ctx.error(
                ln,
                &format!("Failed to parse log level '{rest}': Invalid argument"),
            );
            return Err(());
        }
    } else {
        ctx.warning(
            ln,
            &format!("Invalid value for OPTIONS key, ignoring: '{value}'"),
        );
        return Ok(());
    }
    push_assign(rl, tid::A_OPTIONS, value);
    Ok(())
}

fn parse_owner_group(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    attr: Option<&str>,
    op: Op,
    value: &str,
    is_owner: bool,
) -> TokResult {
    let ln = rl.line_number;
    if attr.is_some() {
        return invalid_attr(ctx, ln, key);
    }
    let mut op = op;
    if op.is_match() || op == Op::Remove {
        return invalid_op(ctx, ln, key);
    }
    if op == Op::Add {
        ctx.warning(
            ln,
            &format!("{key} key takes '=' or ':=' operator, assuming '='."),
        );
        op = Op::Assign;
    }
    let _ = op;

    let all_digits = !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
    let plain = subst_type_plain(value);

    if all_digits || (ctx.timing == Timing::Early && plain) {
        if let Err(reason) = resolve_id(ctx, value, is_owner) {
            let noun = if is_owner { "user" } else { "group" };
            ctx.error(
                ln,
                &format!("Failed to resolve {noun} '{value}', ignoring: {reason}"),
            );
            return Err(());
        }
        push_assign(
            rl,
            if is_owner { tid::A_OWNER } else { tid::A_GROUP },
            value,
        );
    } else if ctx.timing != Timing::Never {
        check_value_format(ctx, ln, key, value, true)?;
        push_assign(
            rl,
            if is_owner { tid::A_OWNER } else { tid::A_GROUP },
            value,
        );
    } else {
        // RESOLVE_NAME_NEVER, non-numeric: silently ignored (debug only).
        return Ok(());
    }
    Ok(())
}

fn parse_mode_key(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    attr: Option<&str>,
    op: Op,
    value: &str,
) -> TokResult {
    let ln = rl.line_number;
    if attr.is_some() {
        return invalid_attr(ctx, ln, key);
    }
    let mut op = op;
    if op.is_match() || op == Op::Remove {
        return invalid_op(ctx, ln, key);
    }
    if op == Op::Add {
        ctx.warning(ln, "MODE key takes '=' or ':=' operator, assuming '='.");
        op = Op::Assign;
    }
    let _ = op;
    if parse_mode(value).is_none() {
        check_value_format(ctx, ln, key, value, true)?;
    }
    push_assign(rl, tid::A_MODE, value);
    Ok(())
}

fn parse_seclabel(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    attr: Option<&str>,
    op: Op,
    value: &str,
) -> TokResult {
    let ln = rl.line_number;
    match attr {
        Some(a) if !a.is_empty() => {}
        _ => return invalid_attr(ctx, ln, key),
    }
    check_value_format(ctx, ln, key, value, true)?;
    let mut op = op;
    if op.is_match() || op == Op::Remove {
        return invalid_op(ctx, ln, key);
    }
    if op == Op::AssignFinal {
        ctx.warning(ln, "SECLABEL key takes '=' or '+=' operator, assuming '='.");
        op = Op::Assign;
    }
    let _ = op;
    push_assign(rl, tid::A_SECLABEL, value);
    Ok(())
}

fn parse_run(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    attr: Option<&str>,
    op: Op,
    value: &str,
) -> TokResult {
    let ln = rl.line_number;
    if op.is_match() || op == Op::Remove {
        return invalid_op(ctx, ln, key);
    }
    check_value_format(ctx, ln, key, value, true)?;
    match attr {
        None | Some("program") => {}
        Some("builtin") => {
            if !builtin_known(value) {
                ctx.error(ln, &format!("Unknown builtin command '{value}', ignoring."));
                return Err(());
            }
        }
        Some(_) => return invalid_attr(ctx, ln, key),
    }
    push_assign(rl, tid::A_RUN, value);
    Ok(())
}

fn parse_goto(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    attr: Option<&str>,
    op: Op,
    value: &str,
) -> TokResult {
    let ln = rl.line_number;
    if attr.is_some() {
        return invalid_attr(ctx, ln, key);
    }
    if op != Op::Assign {
        return invalid_op(ctx, ln, key);
    }
    if rl.has_goto {
        ctx.warning(
            ln,
            &format!("Contains multiple GOTO keys, ignoring GOTO=\"{value}\"."),
        );
        return Ok(());
    }
    rl.has_goto = true;
    rl.goto_label = Some(value.to_string());
    Ok(())
}

fn parse_label(
    ctx: &mut FileCtx,
    rl: &mut RuleLine,
    key: &str,
    attr: Option<&str>,
    op: Op,
    value: &str,
) -> TokResult {
    let ln = rl.line_number;
    if attr.is_some() {
        return invalid_attr(ctx, ln, key);
    }
    if op != Op::Assign {
        return invalid_op(ctx, ln, key);
    }
    if rl.has_label {
        // Logs the PREVIOUS label, then overwrites.
        let prev = rl.label.clone().unwrap_or_default();
        ctx.warning(
            ln,
            &format!("Contains multiple LABEL keys, ignoring LABEL=\"{prev}\"."),
        );
    }
    rl.has_label = true;
    rl.label = Some(value.to_string());
    Ok(())
}

fn subst_type_plain(s: &str) -> bool {
    if s.starts_with('[') {
        return false;
    }
    !s.contains('%') && !s.contains('$')
}

/// Parse an octal mode string (0..=0o7777). Mirrors `parse_mode` closely enough
/// for validation: base 8, no sign.
fn parse_mode(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    if s.contains(['+', '-']) {
        return None;
    }
    match u32::from_str_radix(s, 8) {
        Ok(m) if m <= 0o7777 => Some(m),
        _ => None,
    }
}

fn log_level_from_string(s: &str) -> Option<i32> {
    match s {
        "emerg" | "0" => Some(0),
        "alert" | "1" => Some(1),
        "crit" | "2" => Some(2),
        "err" | "error" | "3" => Some(3),
        "warning" | "warn" | "4" => Some(4),
        "notice" | "5" => Some(5),
        "info" | "6" => Some(6),
        "debug" | "7" => Some(7),
        _ => None,
    }
}

/// Resolve a user/group name or numeric id, returning `Err(reason)` on failure
/// where `reason` is the strerror-style suffix ("Unknown user" / "Unknown
/// group" / "Invalid argument").
fn resolve_id(ctx: &FileCtx, value: &str, is_owner: bool) -> Result<(), String> {
    let noun_unknown = if is_owner {
        "Unknown user"
    } else {
        "Unknown group"
    };

    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        // Numeric id: valid if it exists, or falls within the system range.
        let id: u64 = value.parse().map_err(|_| "Invalid argument".to_string())?;
        let exists = if is_owner {
            unsafe { !libc::getpwuid(id as libc::uid_t).is_null() }
        } else {
            unsafe { !libc::getgrgid(id as libc::gid_t).is_null() }
        };
        let sys_max = if is_owner {
            ctx.sys_uid_max
        } else {
            ctx.sys_gid_max
        } as u64;
        if exists || id <= sys_max {
            return Ok(());
        }
        return Err(noun_unknown.to_string());
    }

    // Name: reject invalid names, then look up.
    if !valid_user_group_name(value) {
        return Err("Invalid argument".to_string());
    }
    let cname = match std::ffi::CString::new(value) {
        Ok(c) => c,
        Err(_) => return Err("Invalid argument".to_string()),
    };
    let found = if is_owner {
        unsafe { !libc::getpwnam(cname.as_ptr()).is_null() }
    } else {
        unsafe { !libc::getgrnam(cname.as_ptr()).is_null() }
    };
    if found {
        Ok(())
    } else {
        Err(noun_unknown.to_string())
    }
}

/// A permissive port of `valid_user_group_name`: first char alpha or '_',
/// remaining chars alnum/'_'/'-', length within POSIX limits.
fn valid_user_group_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 255 {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '$') {
            return false;
        }
    }
    true
}

// --------------------------------------------------------------------------
// Post-parse passes: GOTO resolution, unused labels, conflicts / duplicates
// --------------------------------------------------------------------------

fn resolve_goto(ctx: &mut FileCtx, lines: &mut [RuleLine]) {
    for i in 0..lines.len() {
        if !lines[i].has_goto {
            continue;
        }
        let target = lines[i].goto_label.clone().unwrap_or_default();
        // Find the first later line whose label matches.
        let mut found = None;
        for (j, l) in lines.iter().enumerate().skip(i + 1) {
            if l.label.as_deref() == Some(target.as_str()) {
                found = Some(j);
                break;
            }
        }
        match found {
            Some(j) => {
                lines[j].is_referenced = true;
            }
            None => {
                let ln = lines[i].line_number;
                ctx.error(
                    ln,
                    &format!("GOTO=\"{target}\" has no matching label, ignoring."),
                );
                lines[i].has_goto = false;
                lines[i].goto_label = None;
                // If nothing but this GOTO gave the line effect, drop it.
                if !lines[i].has_effect && !lines[i].has_label {
                    ctx.warning(ln, "The line has no effect any more, dropping.");
                    lines[i].dropped = true;
                }
            }
        }
    }
}

fn check_unused_labels(ctx: &mut FileCtx, line: &RuleLine) {
    if line.has_label && !line.is_referenced {
        let label = line.label.clone().unwrap_or_default();
        ctx.notice(
            line.line_number,
            &format!("style: LABEL=\"{label}\" is unused."),
        );
    }
}

fn check_conflicts_duplicates(ctx: &mut FileCtx, line: &RuleLine) {
    let mut conflicts = false;
    let mut duplicates = false;
    let toks = &line.tokens;
    for a in 0..toks.len() {
        for b in (a + 1)..toks.len() {
            let ta = &toks[a];
            let tb = &toks[b];
            let mut new_conflicts = false;
            let mut new_duplicates = false;

            if tokens_eq(ta, tb) {
                if !duplicates && ta.op == tb.op {
                    new_duplicates = true;
                }
                if !conflicts && conflicting_op(ta.op, tb.op) {
                    new_conflicts = true;
                }
            } else if !conflicts && nulstr_tokens_conflict(ta, tb) {
                new_conflicts = true;
            } else {
                continue;
            }

            if new_duplicates {
                duplicates = true;
                ctx.warning(line.line_number, "duplicate expressions.");
            }
            if new_conflicts {
                conflicts = true;
                ctx.error(
                    line.line_number,
                    "conflicting match expressions, the line has no effect.",
                );
            }
            if conflicts && duplicates {
                return;
            }
        }
    }
}

fn conflicting_op(a: Op, b: Op) -> bool {
    (a == Op::Match && b == Op::Nomatch) || (a == Op::Nomatch && b == Op::Match)
}

/// nulstr set equality (order-independent).
fn nulstr_eq(a: &[String], b: &[String]) -> bool {
    a.iter().all(|x| b.contains(x)) && b.iter().all(|x| a.contains(x))
}

fn token_type_and_value_eq(a: &Token, b: &Token) -> bool {
    if a.type_id != b.type_id || a.match_type != b.match_type {
        return false;
    }
    if matches!(a.match_type, MatchType::Empty | MatchType::Subsystem) {
        return true;
    }
    if a.has_nulstr {
        nulstr_eq(&a.values, &b.values)
    } else {
        a.values == b.values
    }
}

fn token_type_and_data_eq(a: &Token, b: &Token) -> bool {
    a.type_id == b.type_id && a.data == b.data
}

fn tokens_eq(a: &Token, b: &Token) -> bool {
    token_type_and_value_eq(a, b) && token_type_and_data_eq(a, b)
}

/// Port of `nulstr_tokens_conflict`: two positive globs/plains that can never
/// both match.
fn nulstr_tokens_conflict(a: &Token, b: &Token) -> bool {
    if !(a.type_id == b.type_id
        && a.has_nulstr
        && a.op == b.op
        && a.op == Op::Match
        && a.match_type == b.match_type
        && token_type_and_data_eq(a, b))
    {
        return false;
    }

    match a.match_type {
        MatchType::Plain => {
            for i in &a.values {
                if b.values.contains(i) {
                    return false;
                }
            }
            true
        }
        MatchType::Glob => {
            for i in &a.values {
                let i_n = glob_prefix_len(i);
                if i_n == 0 {
                    return false;
                }
                for j in &b.values {
                    let j_n = glob_prefix_len(j);
                    let m = i_n.min(j_n);
                    if j_n == 0 || i.as_bytes()[..m] == j.as_bytes()[..m] {
                        return false;
                    }
                }
            }
            true
        }
        _ => false,
    }
}

/// Length of the non-glob prefix (strcspn against GLOB_CHARS).
fn glob_prefix_len(s: &str) -> usize {
    s.bytes()
        .take_while(|b| !matches!(b, b'*' | b'?' | b'['))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_format_bad_subst() {
        // "%?" is an invalid substitution at char 1 (offset 0).
        assert_eq!(
            udev_check_format("%?"),
            Err((0, "invalid substitution type"))
        );
        // A bare trailing '%' is invalid at char 1.
        assert_eq!(
            udev_check_format("%"),
            Err((0, "invalid substitution type"))
        );
        // A bad substitution mid-string reports the sigil's offset.
        assert_eq!(
            udev_check_format("abc%?"),
            Err((3, "invalid substitution type"))
        );
    }

    #[test]
    fn check_format_valid() {
        assert!(udev_check_format("plain text").is_ok());
        assert!(udev_check_format("%N").is_ok());
        assert!(udev_check_format("$devpath").is_ok());
        assert!(udev_check_format("%%literal").is_ok());
        assert!(udev_check_format("$$literal").is_ok());
        assert!(udev_check_format("%b{id}").is_ok());
        // env / attr with empty attr -> attribute value missing.
        assert_eq!(
            udev_check_format("%s{}"),
            Err((0, "attribute value missing"))
        );
        // result index must be numeric.
        assert_eq!(
            udev_check_format("%c{x}"),
            Err((0, "attribute value not a valid number"))
        );
        assert!(udev_check_format("%c{2}").is_ok());
        assert!(udev_check_format("%c{2+}").is_ok());
    }

    #[test]
    fn mode_parse() {
        assert_eq!(parse_mode("0644"), Some(0o644));
        assert_eq!(parse_mode("644"), Some(0o644));
        assert_eq!(parse_mode("a"), None);
        assert_eq!(parse_mode("77777"), None); // > 07777
        assert_eq!(parse_mode(""), None);
        assert_eq!(parse_mode("+7"), None);
    }

    #[test]
    fn glob_detection() {
        assert!(string_is_glob("a*"));
        assert!(string_is_glob("?"));
        assert!(string_is_glob("[abc]"));
        assert!(!string_is_glob("plain"));
        assert_eq!(glob_prefix_len("a*"), 1);
        assert_eq!(glob_prefix_len("*"), 0);
        assert_eq!(glob_prefix_len("abc"), 3);
    }

    #[test]
    fn user_name_validity() {
        assert!(valid_user_group_name("nosuchuser"));
        assert!(valid_user_group_name("_svc"));
        assert!(!valid_user_group_name(":nosuchuser:"));
        assert!(!valid_user_group_name(""));
        assert!(!valid_user_group_name("1abc")); // must not start with a digit
    }

    #[test]
    fn env_readonly() {
        assert!(!device_property_can_set("ACTION"));
        assert!(!device_property_can_set("SYNTH_ARG_FOO"));
        assert!(!device_property_can_set(""));
        assert!(device_property_can_set("MY_VAR"));
    }

    #[test]
    fn builtin_lookup() {
        assert!(builtin_known("net_id"));
        assert!(builtin_known("hwdb"));
        assert!(!builtin_known("foo"));
    }

    #[test]
    fn qmark_star_becomes_nonempty() {
        // KERNEL=="?*" is converted to a NOMATCH-empty token.
        let (mt, _v, op) = compute_match(tid::KERNEL, false, "?*", Op::Match);
        assert_eq!(mt, MatchType::Empty);
        assert_eq!(op, Op::Nomatch);
    }

    #[test]
    fn nulstr_split_tracks_empty() {
        // "|a|b" -> alternatives {a, b} with an empty alternative flagged.
        let (mt, values, _op) = compute_match(tid::KERNEL, false, "|a|b", Op::Match);
        assert_eq!(mt, MatchType::PlainWithEmpty);
        assert!(values.contains(&"a".to_string()) && values.contains(&"b".to_string()));
    }
}
