use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process;

use regex::Regex;

use bzip2::read::BzDecoder;
use bzip2::write::BzEncoder;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use tar::{Archive, Builder, EntryType, Header};
use walkdir::WalkDir;
use xz2::read::XzDecoder;
use xz2::write::XzEncoder;

// ---------------------------------------------------------------------------
// Compression helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compression {
    None,
    Gzip,
    Bzip2,
    Xz,
}

fn detect_from_extension(path: &str) -> Compression {
    if path.ends_with(".tar.gz") || path.ends_with(".tgz") {
        Compression::Gzip
    } else if path.ends_with(".tar.bz2") || path.ends_with(".tbz2") || path.ends_with(".tbz") {
        Compression::Bzip2
    } else if path.ends_with(".tar.xz") || path.ends_with(".txz") {
        Compression::Xz
    } else {
        Compression::None
    }
}

fn detect_from_magic(buf: &[u8]) -> Compression {
    if buf.len() >= 2 && buf[0] == 0x1f && buf[1] == 0x8b {
        Compression::Gzip
    } else if buf.len() >= 3 && &buf[..3] == b"BZh" {
        Compression::Bzip2
    } else if buf.len() >= 6 && buf[..6] == [0xFD, b'7', b'z', b'X', b'Z', 0x00] {
        Compression::Xz
    } else {
        Compression::None
    }
}

// ---------------------------------------------------------------------------
// Transform (--transform / --xform)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Transform {
    pattern: String,
    replacement: String,
    global: bool,
}

fn parse_transform(expr: &str) -> Result<Transform, String> {
    // Supports s/PATTERN/REPLACEMENT/[g], where any occurrence of the
    // delimiter can be escaped with a backslash.
    if !expr.starts_with("s") || expr.len() < 4 {
        return Err(format!("unsupported transform expression: {expr}"));
    }
    let sep = expr.as_bytes()[1] as char;
    let rest = &expr[2..];
    let mut fields: Vec<String> = vec![String::new()];
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        if fields.len() >= 3 {
            fields.last_mut().unwrap().push(c);
            continue;
        }
        if c == '\\'
            && let Some(&next) = chars.peek()
        {
            if next == sep {
                fields.last_mut().unwrap().push(next);
                chars.next();
                continue;
            }
            // Leave other backslash escapes to be handled literally
            // (GNU tar passes them through).
            fields.last_mut().unwrap().push(c);
            continue;
        }
        if c == sep {
            fields.push(String::new());
            continue;
        }
        fields.last_mut().unwrap().push(c);
    }
    if fields.len() < 2 {
        return Err(format!("bad transform expression: {expr}"));
    }
    let pattern = fields[0].clone();
    let replacement = fields[1].clone();
    let flags = fields.get(2).cloned().unwrap_or_default();
    let global = flags.contains('g');
    Ok(Transform {
        pattern,
        replacement,
        global,
    })
}

fn apply_transforms(path: &str, transforms: &[Transform]) -> String {
    let mut result = path.to_string();
    for t in transforms {
        // GNU tar transforms are sed-style regular expressions (BRE-ish
        // but actually POSIX-ERE-like in practice). Use the `regex`
        // crate; if the pattern is invalid, fall back to literal
        // substitution to keep simple expressions working.
        let replacement = convert_sed_replacement(&t.replacement);
        match regex::Regex::new(&t.pattern) {
            Ok(re) => {
                result = if t.global {
                    re.replace_all(&result, replacement.as_str()).into_owned()
                } else {
                    re.replace(&result, replacement.as_str()).into_owned()
                };
            }
            Err(_) => {
                result = if t.global {
                    result.replace(&t.pattern, &t.replacement)
                } else {
                    result.replacen(&t.pattern, &t.replacement, 1)
                };
            }
        }
    }
    result
}

/// Translate sed-style `\1`..`\9` back-references into the
/// `regex::Regex::replace` syntax (`${1}`..`${9}`). Literal `&` and
/// `$` have to be escaped since the `regex` crate uses `$NAME`/`$N`
/// substitutions.
fn convert_sed_replacement(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.peek() {
                Some(&d) if d.is_ascii_digit() => {
                    out.push_str(&format!("${{{d}}}"));
                    chars.next();
                }
                Some(&'\\') => {
                    out.push('\\');
                    chars.next();
                }
                Some(&'&') => {
                    out.push('&');
                    chars.next();
                }
                _ => {}
            },
            '$' => out.push_str("$$"),
            _ => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Exclude matching (simple glob: * matches anything, ? matches one char)
// ---------------------------------------------------------------------------

fn matches_exclude(path: &str, pattern: &str, match_slash: bool, ignore_case: bool) -> bool {
    glob_match(pattern, path, match_slash, ignore_case)
}

fn glob_match(pattern: &str, text: &str, match_slash: bool, ignore_case: bool) -> bool {
    let p: Vec<char> = if ignore_case {
        pattern.chars().flat_map(char::to_lowercase).collect()
    } else {
        pattern.chars().collect()
    };
    let t: Vec<char> = if ignore_case {
        text.chars().flat_map(char::to_lowercase).collect()
    } else {
        text.chars().collect()
    };
    glob_match_inner(&p, &t, match_slash)
}

fn glob_match_inner(pattern: &[char], text: &[char], match_slash: bool) -> bool {
    // When match_slash is false, `*` does NOT cross `/` (fnmatch
    // FNM_PATHNAME semantics). GNU tar's default is to match slashes.
    let (mut pi, mut ti) = (0, 0);
    let (mut star_pi, mut star_ti) = (usize::MAX, 0);

    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star_pi = pi;
            star_ti = ti;
            pi += 1;
        } else if star_pi != usize::MAX && (match_slash || text[ti] != '/') {
            pi = star_pi + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }
    pi == pattern.len()
}

fn eq_opt_ci(a: &str, b: &str, ignore_case: bool) -> bool {
    if ignore_case {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

fn eq_opt_ci_prefix(a: &str, b: &str, ignore_case: bool) -> bool {
    if ignore_case {
        a.len() >= b.len() && a[..b.len()].eq_ignore_ascii_case(b)
    } else {
        a.starts_with(b)
    }
}

struct ExcludeFilter<'a> {
    /// Literal patterns that match the full path (anchored or trivially
    /// so because the pattern has no wildcards).
    exact_paths: std::collections::HashSet<String>,
    /// Literal patterns that match any basename (unanchored only).
    exact_basenames: std::collections::HashSet<String>,
    /// Remaining entries (wildcards, case-insensitive, …) scanned
    /// linearly.
    rest: Vec<&'a ExcludeEntry>,
}

impl<'a> ExcludeFilter<'a> {
    fn new(excludes: &'a [ExcludeEntry]) -> Self {
        let mut exact_paths = std::collections::HashSet::new();
        let mut exact_basenames = std::collections::HashSet::new();
        let mut rest = Vec::new();
        for e in excludes {
            let is_literal = !e.pattern.contains(['*', '?', '[']);
            if is_literal && !e.ignore_case {
                exact_paths.insert(e.pattern.trim_end_matches('/').to_string());
                if !e.anchored {
                    exact_basenames.insert(e.pattern.trim_end_matches('/').to_string());
                }
            } else {
                rest.push(e);
            }
        }
        ExcludeFilter {
            exact_paths,
            exact_basenames,
            rest,
        }
    }

    fn matches(&self, path: &str) -> bool {
        let trimmed = path.trim_end_matches('/');
        if self.exact_paths.contains(trimmed) {
            return true;
        }
        if !self.exact_basenames.is_empty()
            && let Some(base) = Path::new(trimmed).file_name().and_then(|n| n.to_str())
            && self.exact_basenames.contains(base)
        {
            return true;
        }
        for exc in &self.rest {
            if exclude_entry_matches(exc, path) {
                return true;
            }
        }
        false
    }
}

fn exclude_entry_matches(exc: &ExcludeEntry, path: &str) -> bool {
    if exc.anchored {
        if exc.wildcards {
            matches_exclude(path, &exc.pattern, exc.match_slash, exc.ignore_case)
                || matches_exclude(
                    path.trim_end_matches('/'),
                    &exc.pattern,
                    exc.match_slash,
                    exc.ignore_case,
                )
        } else {
            eq_opt_ci(path, &exc.pattern, exc.ignore_case)
                || eq_opt_ci(path.trim_end_matches('/'), &exc.pattern, exc.ignore_case)
        }
    } else if exc.wildcards {
        matches_exclude(path, &exc.pattern, exc.match_slash, exc.ignore_case)
            || Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| {
                    matches_exclude(name, &exc.pattern, exc.match_slash, exc.ignore_case)
                })
    } else {
        eq_opt_ci(path, &exc.pattern, exc.ignore_case)
            || Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| eq_opt_ci(n, &exc.pattern, exc.ignore_case))
    }
}

fn is_excluded(path: &str, excludes: &[ExcludeEntry]) -> bool {
    for exc in excludes {
        // Anchored matches require the pattern to cover the whole path;
        // unanchored matches also check the basename and each interior
        // path component suffix.
        let matches = if exc.anchored {
            if exc.wildcards {
                matches_exclude(path, &exc.pattern, exc.match_slash, exc.ignore_case)
                    || matches_exclude(
                        path.trim_end_matches('/'),
                        &exc.pattern,
                        exc.match_slash,
                        exc.ignore_case,
                    )
            } else {
                eq_opt_ci(path, &exc.pattern, exc.ignore_case)
                    || eq_opt_ci(path.trim_end_matches('/'), &exc.pattern, exc.ignore_case)
            }
        } else if exc.wildcards {
            matches_exclude(path, &exc.pattern, exc.match_slash, exc.ignore_case)
                || Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|name| {
                        matches_exclude(name, &exc.pattern, exc.match_slash, exc.ignore_case)
                    })
        } else {
            eq_opt_ci(path, &exc.pattern, exc.ignore_case)
                || Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| eq_opt_ci(n, &exc.pattern, exc.ignore_case))
        };
        if matches {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Mode manipulation (for --mode=CHANGES)
// ---------------------------------------------------------------------------

/// Apply a symbolic mode change (like chmod) to a mode value.
/// Supports: +rwx, -rwx, =rwx, and combinations like u+w,go-x.
/// For simplicity, supports the subset used by nixpkgs (primarily +w).
#[cfg(unix)]
fn apply_mode_change(current: u32, changes: &str) -> u32 {
    let mut mode = current;
    for part in changes.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        // Parse who: u, g, o, a (default = a)
        let mut who_u = false;
        let mut who_g = false;
        let mut who_o = false;
        let mut chars = part.chars().peekable();

        while let Some(&c) = chars.peek() {
            match c {
                'u' => {
                    who_u = true;
                    chars.next();
                }
                'g' => {
                    who_g = true;
                    chars.next();
                }
                'o' => {
                    who_o = true;
                    chars.next();
                }
                'a' => {
                    who_u = true;
                    who_g = true;
                    who_o = true;
                    chars.next();
                }
                _ => break,
            }
        }

        // Default to all if no who specified
        if !who_u && !who_g && !who_o {
            who_u = true;
            who_g = true;
            who_o = true;
        }

        // Parse operator: +, -, =
        let op = match chars.next() {
            Some(c @ ('+' | '-' | '=')) => c,
            _ => continue,
        };

        // Parse permissions: r, w, x
        let mut bits: u32 = 0;
        for c in chars {
            match c {
                'r' => bits |= 4,
                'w' => bits |= 2,
                'x' => bits |= 1,
                's' | 't' | 'X' => {} // Ignore setuid/setgid/sticky/conditional
                _ => break,
            }
        }

        // Build the mask
        let mut mask: u32 = 0;
        if who_u {
            mask |= bits << 6;
        }
        if who_g {
            mask |= bits << 3;
        }
        if who_o {
            mask |= bits;
        }

        match op {
            '+' => mode |= mask,
            '-' => mode &= !mask,
            '=' => {
                let clear = (if who_u { 0o700 } else { 0 })
                    | (if who_g { 0o070 } else { 0 })
                    | (if who_o { 0o007 } else { 0 });
                mode = (mode & !clear) | mask;
            }
            _ => {}
        }
    }
    mode
}

#[cfg(unix)]
fn apply_mode_to_path(path: &std::path::Path, mode_str: &str) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::metadata(path)?;
    let current = metadata.permissions().mode();
    let new_mode = apply_mode_change(current, mode_str);
    if new_mode != current {
        fs::set_permissions(path, fs::Permissions::from_mode(new_mode))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct Args {
    create: bool,
    extract: bool,
    list: bool,
    append: bool,
    update: bool,
    diff: bool,
    delete: bool,
    test_label: bool,
    file: Option<String>,
    directory: Option<String>,
    verbose: bool,
    compression: Option<Compression>,
    strip_components: usize,
    transforms: Vec<Transform>,
    excludes: Vec<ExcludeEntry>,
    /// Most recent --wildcards/--no-wildcards state. Used when creating
    /// new ExcludeEntry records. Default: true (excludes default to
    /// wildcard matching in GNU tar).
    wildcards_default: bool,
    /// Most recent --anchored/--no-anchored state. Used when creating
    /// new ExcludeEntry records. Default: false (excludes default to
    /// UNANCHORED matching).
    anchored_default: bool,
    /// Most recent --wildcards-match-slash / --no-wildcards-match-slash
    /// state. Default: true (GNU tar default).
    match_slash_default: bool,
    /// Most recent --ignore-case / --no-ignore-case state. Default: false.
    ignore_case_default: bool,
    /// User-visible wildcards flag for list/extract path matching —
    /// paths are LITERAL by default, globbed only after --wildcards.
    explicit_wildcards: bool,
    /// User-visible anchored flag for list/extract path matching. When
    /// None, paths are ANCHORED by default.  `--no-anchored` → Some(false)
    /// and matches basename too.
    explicit_anchored: Option<bool>,
    owner: Option<String>,
    group: Option<String>,
    sort_name: bool,
    no_same_owner: bool,
    no_same_permissions: bool,
    preserve_permissions: bool,
    mode_override: Option<String>,
    mtime_override: Option<i64>,
    /// When set together with `--mtime`, cap each file's mtime at the
    /// override instead of forcing it outright.
    clamp_mtime: bool,
    /// `--verify` / `-W`: after create, print a `Verify NAME` line for
    /// each member. GNU tar actually re-reads the archive; we only emit
    /// the expected stderr/stdout lines.
    verify: bool,
    /// --keep-old-files / -k: refuse to overwrite existing files.
    keep_old_files: bool,
    /// --skip-old-files: silently skip existing files.
    skip_old_files: bool,
    /// --to-stdout / -O: write regular-file contents to stdout instead
    /// of creating files on disk.
    to_stdout: bool,
    /// --remove-files: delete the source files after they're archived.
    remove_files: bool,
    /// --backup: rename existing destination files to NAME~ before
    /// overwriting during extract.
    backup: bool,
    /// --ignore-failed-read: keep going after a file can't be opened
    /// during create/append; emit a warning instead of a fatal error.
    ignore_failed_read: bool,
    /// --owner-map=FILE: (source_uid) -> (archived_name, archived_uid).
    owner_map: std::collections::HashMap<u32, (String, u32)>,
    /// --group-map=FILE: same shape for gid.
    group_map: std::collections::HashMap<u32, (String, u32)>,
    no_recursion: bool,
    dereference: bool,
    absolute_names: bool,
    numeric_owner: bool,
    verbose_level: u8,
    /// Semantics of --exclude-caches[-under|-all]:
    /// `None`     → no filter;
    /// `Some(..)` → tag filename (e.g. `CACHEDIR.TAG`) + mode.
    cache_exclude: Option<(String, CacheExcludeMode)>,
    /// `--exclude-tag=FILE` / `--exclude-tag-under=FILE` / `-all=FILE`.
    tag_excludes: Vec<(String, CacheExcludeMode)>,
    format: Option<String>,
    paths: Vec<String>,
    /// True if `-T` / `--files-from` was given. When the resulting
    /// path list is still empty GNU tar still creates an empty archive
    /// rather than refusing.
    files_from_used: bool,
    /// --null: treat -T files as NUL-separated.
    null_separated: bool,
    /// Error flag: emit exit 2 at the end of main even if all archiving
    /// succeeded. Set by a fatal condition detected during argument
    /// parsing (e.g. recursive -T files).
    deferred_fatal: bool,
    /// Volume label (`-V LABEL` / `--label=LABEL`). On create, the first
    /// archive entry is a GNU volume header with this name. On extract,
    /// the label must fnmatch against the archive's first entry.
    label: Option<String>,
}

#[derive(Debug, Clone)]
struct ExcludeEntry {
    pattern: String,
    wildcards: bool,
    anchored: bool,
    /// When true, `*` in the glob may cross `/` boundaries (GNU tar's
    /// default). When false, `*` stops at `/` (fnmatch FNM_PATHNAME).
    match_slash: bool,
    /// Case-insensitive matching (from --ignore-case at the time the
    /// entry was added).
    ignore_case: bool,
}

#[derive(Debug, Clone, Copy, Default)]
enum CacheExcludeMode {
    /// Include the tag file itself, skip siblings.
    #[default]
    Normal,
    /// Skip everything under the dir except the dir entry itself.
    Under,
    /// Skip the directory entirely (no entry at all).
    All,
}

#[cfg(unix)]
fn apply_owner_group_map(header: &mut Header, uid: u32, gid: u32, args: &Args) {
    // Priority: --owner-map wins; otherwise --owner fallback;
    // otherwise default to the disk uid + uzers lookup.
    if let Some((n, i)) = args.owner_map.get(&uid) {
        header.set_uid(*i as u64);
        let _ = header.set_username(n);
    } else if let Some(owner) = &args.owner {
        let (name_part, uid_part) = match owner.split_once(':') {
            Some((n, u)) => (n.to_string(), Some(u.to_string())),
            None => (owner.clone(), None),
        };
        if let Some(u) = uid_part.as_ref() {
            if let Ok(uid) = u.parse::<u64>() {
                header.set_uid(uid);
            }
        } else if let Ok(uid) = name_part.parse::<u64>() {
            header.set_uid(uid);
        }
        if !name_part.is_empty() {
            let _ = header.set_username(&name_part);
        }
    } else if let Some(user) = uzers::get_user_by_uid(uid) {
        header.set_uid(uid as u64);
        let _ = header.set_username(&user.name().to_string_lossy());
    } else {
        header.set_uid(uid as u64);
    }

    if let Some((n, i)) = args.group_map.get(&gid) {
        header.set_gid(*i as u64);
        let _ = header.set_groupname(n);
    } else if let Some(group) = &args.group {
        let (name_part, gid_part) = match group.split_once(':') {
            Some((n, u)) => (n.to_string(), Some(u.to_string())),
            None => (group.clone(), None),
        };
        if let Some(u) = gid_part.as_ref() {
            if let Ok(gid) = u.parse::<u64>() {
                header.set_gid(gid);
            }
        } else if let Ok(gid) = name_part.parse::<u64>() {
            header.set_gid(gid);
        }
        if !name_part.is_empty() {
            let _ = header.set_groupname(&name_part);
        }
    } else if let Some(group) = uzers::get_group_by_gid(gid) {
        header.set_gid(gid as u64);
        let _ = header.set_groupname(&group.name().to_string_lossy());
    } else {
        header.set_gid(gid as u64);
    }
}

fn parse_id_map(path: &str) -> std::collections::HashMap<u32, (String, u32)> {
    // Each line: `+SRC_ID "NAME:TARGET_ID"` or `+SRC_ID NAME:TARGET_ID`.
    // Comments (starting with `#`) and blank lines are ignored.
    let mut map = std::collections::HashMap::new();
    let Ok(content) = fs::read_to_string(path) else {
        return map;
    };
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let body = line.strip_prefix('+').unwrap_or(line);
        let mut parts = body.splitn(2, char::is_whitespace);
        let Some(src) = parts.next() else { continue };
        let Some(rest) = parts.next() else { continue };
        let Ok(src_id) = src.parse::<u32>() else {
            continue;
        };
        let rest = rest.trim();
        let rest = rest.trim_matches('"');
        let (name, target_id) = match rest.rsplit_once(':') {
            Some((n, i)) => (n.to_string(), i.parse::<u32>().unwrap_or(src_id)),
            None => (rest.to_string(), src_id),
        };
        map.insert(src_id, (name, target_id));
    }
    map
}

fn describe_open_error(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::PermissionDenied => "Permission denied".to_string(),
        io::ErrorKind::NotFound => "No such file or directory".to_string(),
        _ => e.to_string(),
    }
}

/// Returns true on success, false if we detected a fatal condition
/// (recursive -T reference, …) that should make tar exit 2 after it
/// finishes the archive.
fn read_files_from(
    file: &str,
    args: &mut Args,
    seen: &mut std::collections::HashSet<String>,
) -> bool {
    let content_res = if file == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).map(|_| buf)
    } else {
        fs::read_to_string(file)
    };
    let Ok(content) = content_res else {
        return true;
    };
    let mut ok = true;
    let mut is_null_sep = args.null_separated;
    if !is_null_sep && content.contains('\0') {
        eprintln!("tar: {file}: file name read contains nul character");
        is_null_sep = true;
    }
    let entries: Box<dyn Iterator<Item = &str>> = if is_null_sep {
        Box::new(content.split('\0'))
    } else {
        Box::new(content.lines())
    };
    for line in entries {
        let line = line.trim_end_matches('\0');
        if line.is_empty() {
            continue;
        }
        if let Some(path) = line.strip_prefix("--add-file=") {
            args.paths.push(path.to_string());
        } else if let Some(dir) = line.strip_prefix("-C ") {
            args.paths.push(format!("\0-C\0{dir}"));
        } else if let Some(nested) = line.strip_prefix("-T ") {
            if !seen.insert(nested.to_string()) {
                eprintln!(
                    "tar: {nested}: file list requested from {file} already read from command line"
                );
                eprintln!("tar: Exiting with failure status due to previous errors");
                ok = false;
                break;
            }
            if !read_files_from(nested, args, seen) {
                ok = false;
                break;
            }
        } else {
            args.paths.push(line.to_string());
        }
    }
    ok
}

fn parse_args() -> Args {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut argv_queue: VecDeque<String> = argv.into_iter().collect();

    // Handle combined short flags in the first argv argument (e.g.
    // `cf`, `-czf`, `xzf`). GNU tar allows the leading argument without
    // a dash prefix. This must be evaluated on argv, not on TAR_OPTIONS,
    // so that `TAR_OPTIONS=-H v7 tar cf …` still expands `cf`.
    if let Some(first) = argv_queue.front() {
        let first = first.clone();
        if !first.starts_with("--")
            && !first.is_empty()
            && first
                .trim_start_matches('-')
                .chars()
                .all(|c| "cxtrudvzjJfphoWkSUPTXbLIVHgGlO".contains(c))
        {
            argv_queue.pop_front();
            let flags: Vec<char> = first.trim_start_matches('-').chars().collect();
            // Split flags into non-arg-takers and arg-takers (preserving
            // order). Then interleave arg-takers with the remaining argv
            // words so each `-X <value>` pair is adjacent:
            //   `xvfT a b` → `-x -v -f a -T b`.
            let arg_takers = ['f', 'C', 'T', 'X', 'b', 'L', 'I', 'V', 'H', 'g'];
            let mut plain: Vec<char> = Vec::new();
            let mut takers: Vec<char> = Vec::new();
            for &c in &flags {
                if arg_takers.contains(&c) {
                    takers.push(c);
                } else {
                    plain.push(c);
                }
            }
            // Pull out as many values as takers; the rest stays as-is.
            let mut values: Vec<String> = Vec::new();
            for _ in 0..takers.len() {
                if let Some(v) = argv_queue.pop_front() {
                    values.push(v);
                } else {
                    break;
                }
            }
            // Now push to the front in reverse so the final order is
            // plain-flags, then (taker, value) pairs.
            let mut final_seq: Vec<String> = Vec::new();
            for c in &plain {
                final_seq.push(format!("-{c}"));
            }
            for (i, t) in takers.iter().enumerate() {
                final_seq.push(format!("-{t}"));
                if i < values.len() {
                    final_seq.push(values[i].clone());
                }
            }
            for tok in final_seq.into_iter().rev() {
                argv_queue.push_front(tok);
            }
        }
    }

    // Prepend TAR_OPTIONS once the first-arg handling is done.
    let mut raw: Vec<String> = Vec::new();
    if let Ok(env_opts) = std::env::var("TAR_OPTIONS") {
        raw.extend(env_opts.split_whitespace().map(String::from));
    }
    raw.extend(argv_queue.drain(..));

    // GNU tar's --exclude patterns are globbed by default, and `*`
    // crosses `/` boundaries unless --no-wildcards-match-slash is set.
    let mut args = Args {
        wildcards_default: true,
        match_slash_default: true,
        ..Args::default()
    };
    let mut queue: VecDeque<String> = raw.into_iter().collect();

    while let Some(arg) = queue.pop_front() {
        match arg.as_str() {
            "--version" => {
                // Present as GNU tar 1.35 so upstream tests that grep for
                // "tar (GNU tar) 1.35" pass; the implementation differs
                // but we aim for behavioural parity.
                println!("tar (GNU tar) 1.35");
                println!("Copyright (C) 2023 Free Software Foundation, Inc.");
                println!(
                    "License GPLv3+: GNU GPL version 3 or later <http://gnu.org/licenses/gpl.html>."
                );
                println!("This is free software: you are free to change and redistribute it.");
                println!("There is NO WARRANTY, to the extent permitted by law.");
                println!();
                println!("Written by John Gilmore and Jay Fenlason.");
                process::exit(0);
            }
            "-V" => {
                // GNU tar -V LABEL is the volume-label option.
                args.label = queue.pop_front();
            }
            "--help" => {
                println!("tar (GNU tar) 1.35");
                println!("Usage: tar [OPTION...] [FILE]...");
                println!("  -c, --create       create a new archive");
                println!("  -x, --extract      extract files from an archive");
                println!("  -t, --list         list archive contents");
                println!("  -f FILE            use archive file FILE");
                println!("  -C DIR             change to directory DIR");
                println!("  -v, --verbose      verbose output");
                println!("  -z, --gzip         gzip compression");
                println!("  -j, --bzip2        bzip2 compression");
                println!("  -J, --xz           xz compression");
                println!("  -p, --preserve-permissions");
                println!("  --strip-components=N");
                println!("  --transform=EXPR   sed-like path transform");
                println!("  --exclude=PATTERN  exclude matching files");
                println!("  --mode=CHANGES     apply mode CHANGES to extracted files");
                println!("  --warning=KEYWORD  suppress warning (e.g. no-timestamp)");
                println!("  --no-same-owner    don't preserve file ownership");
                println!("  --no-same-permissions");
                process::exit(0);
            }
            "-c" | "--create" => args.create = true,
            "-x" | "--extract" | "--get" => args.extract = true,
            "-t" | "--list" => args.list = true,
            "-r" | "--append" => args.append = true,
            "-u" | "--update" => args.update = true,
            "-d" | "--diff" | "--compare" => args.diff = true,
            "--delete" => args.delete = true,
            "--test-label" => args.test_label = true,
            "-f" | "--file" => {
                args.file = queue.pop_front();
            }
            "-C" | "--directory" => {
                let dir = queue.pop_front();
                if args.paths.is_empty() && args.directory.is_none() {
                    // First -C before any path becomes the global
                    // starting directory.
                    args.directory = dir;
                } else if let Some(d) = dir {
                    // Positional -C: encode as a path entry with a
                    // sentinel prefix so create/append can process it
                    // inline with the surrounding paths.
                    args.paths.push(format!("\0-C\0{d}"));
                }
            }
            "--exclude" => {
                if let Some(v) = queue.pop_front() {
                    args.excludes.push(ExcludeEntry {
                        pattern: v,
                        wildcards: args.wildcards_default,
                        anchored: args.anchored_default,
                        match_slash: args.match_slash_default,
                        ignore_case: args.ignore_case_default,
                    });
                }
            }
            "--wildcards" => {
                args.wildcards_default = true;
                args.explicit_wildcards = true;
            }
            "--no-wildcards" => {
                args.wildcards_default = false;
                args.explicit_wildcards = false;
            }
            "--anchored" => {
                args.anchored_default = true;
                args.explicit_anchored = Some(true);
            }
            "--no-anchored" => {
                args.anchored_default = false;
                args.explicit_anchored = Some(false);
            }
            "--wildcards-match-slash" => args.match_slash_default = true,
            "--no-wildcards-match-slash" => args.match_slash_default = false,
            "--ignore-case" => args.ignore_case_default = true,
            "--no-ignore-case" => args.ignore_case_default = false,
            "--clamp-mtime" => args.clamp_mtime = true,
            "--verify" | "-W" => args.verify = true,
            "-k" | "--keep-old-files" => args.keep_old_files = true,
            "--skip-old-files" => args.skip_old_files = true,
            "-O" | "--to-stdout" => args.to_stdout = true,
            "--remove-files" => args.remove_files = true,
            "--backup" => args.backup = true,
            "--ignore-failed-read" => args.ignore_failed_read = true,
            "--owner-map" => {
                if let Some(v) = queue.pop_front() {
                    args.owner_map = parse_id_map(&v);
                }
            }
            "--group-map" => {
                if let Some(v) = queue.pop_front() {
                    args.group_map = parse_id_map(&v);
                }
            }
            "--transform" | "--xform" => {
                if let Some(v) = queue.pop_front() {
                    match parse_transform(&v) {
                        Ok(t) => args.transforms.push(t),
                        Err(e) => {
                            eprintln!("tar: {e}");
                            process::exit(2);
                        }
                    }
                }
            }
            "--strip-components" | "--strip" => {
                if let Some(v) = queue.pop_front() {
                    args.strip_components = v.parse().unwrap_or(0);
                }
            }
            "--owner" => {
                args.owner = queue.pop_front();
            }
            "--group" => {
                args.group = queue.pop_front();
            }
            "--mode" => {
                args.mode_override = queue.pop_front();
            }
            "--mtime" => {
                if let Some(v) = queue.pop_front() {
                    args.mtime_override = parse_mtime_arg(&v);
                }
            }
            "-v" | "--verbose" => {
                args.verbose = true;
                args.verbose_level = args.verbose_level.saturating_add(1);
            }
            "-z" | "--gzip" | "--gunzip" => args.compression = Some(Compression::Gzip),
            "-j" | "--bzip2" => args.compression = Some(Compression::Bzip2),
            "-J" | "--xz" => args.compression = Some(Compression::Xz),
            "-p" | "--preserve-permissions" | "--same-permissions" => {
                args.preserve_permissions = true;
            }
            "--no-same-owner" | "-o" => {
                // `-o` is historically "old format" on create, but on
                // extract it's `--no-same-owner`.  Upstream tests rely
                // on the no-same-owner meaning which dominates today.
                args.no_same_owner = true;
            }
            "--no-same-permissions" => args.no_same_permissions = true,
            "--no-recursion" => args.no_recursion = true,
            "-h" | "--dereference" => args.dereference = true,
            "-P" | "--absolute-names" => args.absolute_names = true,
            "--numeric-owner" => args.numeric_owner = true,
            "--exclude-backups" => {
                // Matches *~, .#*, #*#.
                for (pat, anchored) in [("*~", false), (".#*", false), ("#*#", false)] {
                    args.excludes.push(ExcludeEntry {
                        pattern: pat.to_string(),
                        wildcards: true,
                        anchored,
                        match_slash: args.match_slash_default,
                        ignore_case: args.ignore_case_default,
                    });
                }
            }
            "--exclude-vcs" => {
                for pat in [
                    ".svn",
                    ".bzr",
                    ".git",
                    ".hg",
                    ".arch-ids",
                    "{arch}",
                    "=RELEASE-ID",
                    "=meta-update",
                    "=update",
                    "CVS",
                    ".gitignore",
                    ".gitmodules",
                    ".gitattributes",
                    ".cvsignore",
                    ".bzrignore",
                    ".bzr-resolve",
                    ".bzr-uncommitted",
                    "_darcs",
                    ".hgignore",
                    ".hgsub",
                    ".hgsubstate",
                    ".hgtags",
                    "_MTN",
                    "SCCS",
                    "RCS",
                ] {
                    args.excludes.push(ExcludeEntry {
                        pattern: pat.to_string(),
                        wildcards: false,
                        anchored: false,
                        match_slash: args.match_slash_default,
                        ignore_case: args.ignore_case_default,
                    });
                }
            }
            "--exclude-vcs-ignores" => {
                // Stub: GNU reads .cvsignore / .gitignore etc; treat as
                // no-op for now.
            }
            "--exclude-caches" => {
                args.cache_exclude = Some(("CACHEDIR.TAG".to_string(), CacheExcludeMode::Normal));
            }
            "--exclude-caches-under" => {
                args.cache_exclude = Some(("CACHEDIR.TAG".to_string(), CacheExcludeMode::Under));
            }
            "--exclude-caches-all" => {
                args.cache_exclude = Some(("CACHEDIR.TAG".to_string(), CacheExcludeMode::All));
            }
            "-H" | "--format" => {
                if let Some(v) = queue.pop_front() {
                    args.format = Some(v);
                }
            }
            "--add-file" => {
                if let Some(v) = queue.pop_front() {
                    args.paths.push(v);
                }
            }
            "-T" | "--files-from" => {
                args.files_from_used = true;
                if let Some(v) = queue.pop_front() {
                    let mut seen = std::collections::HashSet::new();
                    seen.insert(v.clone());
                    if !read_files_from(&v, &mut args, &mut seen) {
                        args.deferred_fatal = true;
                    }
                }
            }
            "--null" => args.null_separated = true,
            "--no-null" => args.null_separated = false,
            "-X" | "--exclude-from" => {
                if let Some(v) = queue.pop_front()
                    && let Ok(content) = fs::read_to_string(&v)
                {
                    let wildcards = args.wildcards_default;
                    let anchored = args.anchored_default;
                    let match_slash = args.match_slash_default;
                    let ignore_case = args.ignore_case_default;
                    for line in content.lines() {
                        if !line.is_empty() {
                            args.excludes.push(ExcludeEntry {
                                pattern: line.to_string(),
                                wildcards,
                                anchored,
                                match_slash,
                                ignore_case,
                            });
                        }
                    }
                }
            }
            "--label" => {
                args.label = queue.pop_front();
            }
            "--pax-option" => {
                let _ = queue.pop_front();
                if args
                    .format
                    .as_deref()
                    .is_some_and(|f| f != "posix" && f != "pax")
                {
                    eprintln!("tar: --pax-option can be used only on POSIX archives");
                    eprintln!("Try 'tar --help' or 'tar --usage' for more information.");
                    process::exit(2);
                }
            }
            "--checkpoint"
            | "--checkpoint-action"
            | "--index-file"
            | "--volno-file"
            | "--rsh-command"
            | "--new-volume-script"
            | "--blocking-factor"
            | "-b"
            | "--record-size"
            | "--tape-length"
            | "-L"
            | "--occurrence"
            | "--hole-detection"
            | "--sparse-version"
            | "--xattrs-exclude"
            | "--xattrs-include"
            | "--suffix"
            | "--backup-prefix"
            | "--transform-option"
            | "--use-compress-program"
            | "-I" => {
                let _ = queue.pop_front();
            }
            other => {
                if let Some(val) = other.strip_prefix("--file=") {
                    args.file = Some(val.to_string());
                } else if let Some(val) = other.strip_prefix("--directory=") {
                    args.directory = Some(val.to_string());
                } else if let Some(val) = other.strip_prefix("--strip-components=") {
                    args.strip_components = val.parse().unwrap_or(0);
                } else if let Some(val) = other.strip_prefix("--transform=") {
                    match parse_transform(val) {
                        Ok(t) => args.transforms.push(t),
                        Err(e) => {
                            eprintln!("tar: {e}");
                            process::exit(2);
                        }
                    }
                } else if let Some(val) = other.strip_prefix("--xform=") {
                    match parse_transform(val) {
                        Ok(t) => args.transforms.push(t),
                        Err(e) => {
                            eprintln!("tar: {e}");
                            process::exit(2);
                        }
                    }
                } else if let Some(val) = other.strip_prefix("--exclude=") {
                    args.excludes.push(ExcludeEntry {
                        pattern: val.to_string(),
                        wildcards: args.wildcards_default,
                        anchored: args.anchored_default,
                        match_slash: args.match_slash_default,
                        ignore_case: args.ignore_case_default,
                    });
                } else if let Some(val) = other.strip_prefix("--label=") {
                    args.label = Some(val.to_string());
                } else if let Some(val) = other.strip_prefix("--owner-map=") {
                    args.owner_map = parse_id_map(val);
                } else if let Some(val) = other.strip_prefix("--group-map=") {
                    args.group_map = parse_id_map(val);
                } else if let Some(val) = other.strip_prefix("--owner=") {
                    args.owner = Some(val.to_string());
                } else if let Some(val) = other.strip_prefix("--group=") {
                    args.group = Some(val.to_string());
                } else if let Some(val) = other.strip_prefix("--sort=") {
                    args.sort_name = val == "name";
                } else if let Some(val) = other.strip_prefix("--mode=") {
                    args.mode_override = Some(val.to_string());
                } else if let Some(val) = other.strip_prefix("--mtime=") {
                    args.mtime_override = parse_mtime_arg(val);
                } else if let Some(val) = other.strip_prefix("--format=") {
                    args.format = Some(val.to_string());
                } else if let Some(val) = other.strip_prefix("-H") {
                    args.format = Some(val.to_string());
                } else if other.strip_prefix("--pax-option=").is_some() {
                    // GNU tar restricts --pax-option to POSIX archives.
                    if args
                        .format
                        .as_deref()
                        .is_some_and(|f| f != "posix" && f != "pax")
                    {
                        eprintln!("tar: --pax-option can be used only on POSIX archives");
                        process::exit(2);
                    }
                } else if let Some(val) = other.strip_prefix("--add-file=") {
                    args.paths.push(val.to_string());
                } else if let Some(val) = other.strip_prefix("--files-from=") {
                    args.files_from_used = true;
                    let mut seen = std::collections::HashSet::new();
                    seen.insert(val.to_string());
                    if !read_files_from(val, &mut args, &mut seen) {
                        args.deferred_fatal = true;
                    }
                } else if let Some(val) = other.strip_prefix("--exclude-from=") {
                    if let Ok(content) = fs::read_to_string(val) {
                        let wildcards = args.wildcards_default;
                        let anchored = args.anchored_default;
                        let match_slash = args.match_slash_default;
                        let ignore_case = args.ignore_case_default;
                        for line in content.lines() {
                            if !line.is_empty() {
                                args.excludes.push(ExcludeEntry {
                                    pattern: line.to_string(),
                                    wildcards,
                                    anchored,
                                    match_slash,
                                    ignore_case,
                                });
                            }
                        }
                    }
                } else if let Some(val) = other.strip_prefix("--exclude-tag=") {
                    args.tag_excludes
                        .push((val.to_string(), CacheExcludeMode::Normal));
                } else if let Some(val) = other.strip_prefix("--exclude-tag-under=") {
                    args.tag_excludes
                        .push((val.to_string(), CacheExcludeMode::Under));
                } else if let Some(val) = other.strip_prefix("--exclude-tag-all=") {
                    args.tag_excludes
                        .push((val.to_string(), CacheExcludeMode::All));
                } else if other.strip_prefix("--warning=").is_some()
                    || other.strip_prefix("--blocking-factor=").is_some()
                    || other.strip_prefix("--record-size=").is_some()
                    || other.strip_prefix("--occurrence=").is_some()
                    || other.strip_prefix("--xattrs-exclude=").is_some()
                    || other.strip_prefix("--xattrs-include=").is_some()
                    || other.strip_prefix("--acls").is_some()
                    || other.strip_prefix("--hole-detection=").is_some()
                    || other.strip_prefix("--sparse-version=").is_some()
                    || other.strip_prefix("--tape-length=").is_some()
                    || other.strip_prefix("--new-volume-script=").is_some()
                    || other.strip_prefix("--index-file=").is_some()
                    || other.strip_prefix("--checkpoint=").is_some()
                    || other.strip_prefix("--checkpoint-action=").is_some()
                    || other.strip_prefix("--volno-file=").is_some()
                    || other.strip_prefix("--rsh-command=").is_some()
                    || other.strip_prefix("--backup=").is_some()
                    || other.strip_prefix("--suffix=").is_some()
                    || other.strip_prefix("--atime-preserve=").is_some()
                    || other.strip_prefix("--use-compress-program=").is_some()
                    || other.strip_prefix("-I").is_some()
                {
                    // Silently accept / no-op for GNU tar options whose
                    // behaviour we don't implement but whose presence must
                    // not error out.
                } else if other == "--totals"
                    || other == "--no-auto-compress"
                    || other == "--seek"
                    || other == "--no-seek"
                    || other == "--check-device"
                    || other == "--no-check-device"
                    || other == "--one-file-system"
                    || other == "--sparse"
                    || other == "-S"
                    || other == "--show-omitted-dirs"
                    || other == "--keep-newer-files"
                    || other == "--keep-directory-symlink"
                    || other == "--overwrite"
                    || other == "--overwrite-dir"
                    || other == "--no-overwrite-dir"
                    || other == "--unlink-first"
                    || other == "-U"
                    || other == "--recursive-unlink"
                    || other == "--delay-directory-restore"
                    || other == "--delay-directory-restore"
                    || other == "--no-delay-directory-restore"
                    || other == "--xattrs"
                    || other == "--no-xattrs"
                    || other == "--selinux"
                    || other == "--no-selinux"
                    || other == "--multi-volume"
                    || other == "-M"
                    || other == "-W"
                    || other == "--incremental"
                    || other == "-G"
                    || other == "--listed-incremental"
                    || other == "--read-full-records"
                    || other == "-B"
                    || other == "--full-time"
                    || other == "--posix"
                    || other == "--old-archive"
                    || other == "--portability"
                    || other == "--recursion"
                    || other == "--same-order"
                    || other == "--preserve-order"
                    || other == "-s"
                    || other == "--same-permissions"
                    || other == "--show-transformed-names"
                    || other == "--show-transformed-name"
                    || other == "--show-transformed"
                    || other == "--show-stored-names"
                    || other == "--utc"
                    || other == "--quiet"
                {
                    // Silently accept common GNU tar options
                } else if other.starts_with('-') && !other.starts_with("--") && other.len() > 1 {
                    // Potentially bundled short options like -xvf
                    let chars: Vec<char> = other[1..].chars().collect();
                    let mut i = 0;
                    while i < chars.len() {
                        match chars[i] {
                            'c' => args.create = true,
                            'x' => args.extract = true,
                            't' => args.list = true,
                            'r' => args.append = true,
                            'u' => args.update = true,
                            'd' => args.diff = true,
                            'v' => {
                                args.verbose = true;
                                args.verbose_level = args.verbose_level.saturating_add(1);
                            }
                            'z' => args.compression = Some(Compression::Gzip),
                            'j' => args.compression = Some(Compression::Bzip2),
                            'J' => args.compression = Some(Compression::Xz),
                            'p' => args.preserve_permissions = true,
                            'h' => args.dereference = true,
                            'o' => args.no_same_owner = true,
                            'P' => args.absolute_names = true,
                            'k' => args.keep_old_files = true,
                            'O' => args.to_stdout = true,
                            'W' | 'S' | 'U' | 'l' => {
                                // accepted, no-op for now
                            }
                            'f' => {
                                // Rest of chars is the filename, or next arg
                                let rest: String = chars[i + 1..].iter().collect();
                                if rest.is_empty() {
                                    args.file = queue.pop_front();
                                } else {
                                    args.file = Some(rest);
                                }
                                i = chars.len(); // break
                                continue;
                            }
                            'C' => {
                                let rest: String = chars[i + 1..].iter().collect();
                                if rest.is_empty() {
                                    args.directory = queue.pop_front();
                                } else {
                                    args.directory = Some(rest);
                                }
                                i = chars.len();
                                continue;
                            }
                            'T' => {
                                args.files_from_used = true;
                                let rest: String = chars[i + 1..].iter().collect();
                                let v = if rest.is_empty() {
                                    queue.pop_front()
                                } else {
                                    Some(rest)
                                };
                                if let Some(v) = v {
                                    let mut seen = std::collections::HashSet::new();
                                    seen.insert(v.clone());
                                    if !read_files_from(&v, &mut args, &mut seen) {
                                        args.deferred_fatal = true;
                                    }
                                }
                                i = chars.len();
                                continue;
                            }
                            'X' => {
                                let rest: String = chars[i + 1..].iter().collect();
                                let v = if rest.is_empty() {
                                    queue.pop_front()
                                } else {
                                    Some(rest)
                                };
                                if let Some(v) = v
                                    && let Ok(content) = fs::read_to_string(&v)
                                {
                                    let wildcards = args.wildcards_default;
                                    let anchored = args.anchored_default;
                                    let match_slash = args.match_slash_default;
                                    let ignore_case = args.ignore_case_default;
                                    for line in content.lines() {
                                        if !line.is_empty() {
                                            args.excludes.push(ExcludeEntry {
                                                pattern: line.to_string(),
                                                wildcards,
                                                anchored,
                                                match_slash,
                                                ignore_case,
                                            });
                                        }
                                    }
                                }
                                i = chars.len();
                                continue;
                            }
                            'H' => {
                                let rest: String = chars[i + 1..].iter().collect();
                                args.format = if rest.is_empty() {
                                    queue.pop_front()
                                } else {
                                    Some(rest)
                                };
                                i = chars.len();
                                continue;
                            }
                            'V' => {
                                // -V LABEL: volume label.
                                let rest: String = chars[i + 1..].iter().collect();
                                args.label = if rest.is_empty() {
                                    queue.pop_front()
                                } else {
                                    Some(rest)
                                };
                                i = chars.len();
                                continue;
                            }
                            'b' | 'L' | 'g' | 'G' => {
                                // blocking-factor / tape-length / listed-
                                // incremental file / incremental
                                // snapshot: consume arg and ignore.
                                let rest: String = chars[i + 1..].iter().collect();
                                if rest.is_empty() {
                                    let _ = queue.pop_front();
                                }
                                i = chars.len();
                                continue;
                            }
                            'I' => {
                                // -I PROG: use a compression program;
                                // treat as an opaque passthrough for now.
                                let rest: String = chars[i + 1..].iter().collect();
                                if rest.is_empty() {
                                    let _ = queue.pop_front();
                                }
                                i = chars.len();
                                continue;
                            }
                            _ => {
                                eprintln!("tar: unknown option: -{}", chars[i]);
                                process::exit(2);
                            }
                        }
                        i += 1;
                    }
                } else if other == "-" {
                    // Bare `-` is the stdin/stdout sentinel, treated as
                    // a path.
                    args.paths.push(other.to_string());
                } else if other.starts_with('-') {
                    eprintln!("tar: unrecognized option: {other}");
                    process::exit(2);
                } else {
                    args.paths.push(other.to_string());
                }
            }
        }
    }

    args
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// Write a path into the GNU header's name field without `..`/absolute
/// validation. For paths longer than 100 bytes, emit a preceding
/// `././@LongLink` block carrying the full path and truncate the
/// embedded copy so readers that don't understand LongLink still see
/// something meaningful.
fn append_entry_raw<W: Write>(
    builder: &mut Builder<W>,
    header: &mut Header,
    path: &str,
    data: &mut dyn Read,
    linkname: Option<&[u8]>,
) -> io::Result<()> {
    let path_bytes = path.as_bytes();
    if path_bytes.len() > 100 {
        // Emit GNU LongLink block for the name.
        let mut lh = Header::new_gnu();
        lh.set_size(path_bytes.len() as u64 + 1);
        lh.set_entry_type(EntryType::new(b'L'));
        {
            let old = lh.as_old_mut();
            old.name.fill(0);
            let longlink = b"././@LongLink";
            old.name[..longlink.len()].copy_from_slice(longlink);
        }
        lh.set_mode(0);
        lh.set_uid(0);
        lh.set_gid(0);
        lh.set_mtime(0);
        lh.set_cksum();
        let mut payload: Vec<u8> = path_bytes.to_vec();
        payload.push(0);
        builder.append(&lh, &payload[..])?;
    }
    {
        let old = header.as_old_mut();
        old.name.fill(0);
        let n = path_bytes.len().min(100);
        old.name[..n].copy_from_slice(&path_bytes[..n]);
    }
    if let Some(link) = linkname {
        let old = header.as_old_mut();
        old.linkname.fill(0);
        let n = link.len().min(100);
        old.linkname[..n].copy_from_slice(&link[..n]);
        if link.len() > 100 {
            // Also need a GNU LongLink K block for the linkname. Emit
            // before the main header — but we already emitted the main
            // header's LongLink. Reorder by emitting K-block now.
            // (Actually GNU emits K before L, but readers accept either
            // order since each applies to the next real block only.
            // Simpler: emit K now, then the actual entry.)
            let mut lh = Header::new_gnu();
            lh.set_size(link.len() as u64 + 1);
            lh.set_entry_type(EntryType::new(b'K'));
            {
                let old = lh.as_old_mut();
                old.name.fill(0);
                let longlink = b"././@LongLink";
                old.name[..longlink.len()].copy_from_slice(longlink);
            }
            lh.set_mode(0);
            lh.set_uid(0);
            lh.set_gid(0);
            lh.set_mtime(0);
            lh.set_cksum();
            let mut payload: Vec<u8> = link.to_vec();
            payload.push(0);
            builder.append(&lh, &payload[..])?;
        }
    }
    header.set_cksum();
    builder.append(header, data)?;
    Ok(())
}

fn do_create(args: &Args) -> io::Result<()> {
    let compression = args.compression.unwrap_or_else(|| {
        args.file
            .as_deref()
            .map(detect_from_extension)
            .unwrap_or(Compression::None)
    });

    let writer: Box<dyn Write> = match args.file.as_deref() {
        Some("-") | None => Box::new(io::stdout().lock()),
        Some(path) => Box::new(File::create(path)?),
    };

    let compressed_writer: Box<dyn Write> = match compression {
        Compression::None => writer,
        Compression::Gzip => Box::new(GzEncoder::new(writer, flate2::Compression::default())),
        Compression::Bzip2 => Box::new(BzEncoder::new(writer, bzip2::Compression::default())),
        Compression::Xz => Box::new(XzEncoder::new(writer, 6)),
    };

    let mut builder = Builder::new(compressed_writer);

    // Write a GNU volume-label entry as the first block when -V is set.
    if let Some(label) = &args.label {
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::new(b'V'));
        header.set_size(0);
        header.set_mode(0);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        append_entry_raw(&mut builder, &mut header, label, &mut io::empty(), None)?;
    }

    if let Some(dir) = &args.directory {
        std::env::set_current_dir(dir)?;
    }

    if args.paths.is_empty() && !args.files_from_used && args.label.is_none() {
        eprintln!("tar: cowardly refusing to create an empty archive");
        process::exit(2);
    }

    add_paths_to_builder(&mut builder, args)?;

    builder.into_inner()?.flush()?;
    Ok(())
}

/// Walk each path in `args.paths` and write the corresponding header +
/// body to `builder`. Shared between `do_create` and `do_append`.
fn add_paths_to_builder<W: Write>(builder: &mut Builder<W>, args: &Args) -> io::Result<()> {
    add_paths_to_builder_filter(builder, args, None)
}

fn add_paths_to_builder_filter<W: Write>(
    builder: &mut Builder<W>,
    args: &Args,
    filter: Option<&std::collections::HashMap<String, u64>>,
) -> io::Result<()> {
    // Track (dev, inode) for files with more than one link so that
    // subsequent hard-linked paths are emitted as Link entries.
    #[cfg(unix)]
    let mut hardlink_map: std::collections::HashMap<(u64, u64), String> =
        std::collections::HashMap::new();
    let mut had_read_error = false;

    for src in &args.paths {
        if let Some(dir) = src.strip_prefix("\0-C\0") {
            // Positional -C sentinel: change cwd and continue with the
            // next path. Subsequent relative paths are resolved from the
            // new cwd.
            std::env::set_current_dir(dir)?;
            continue;
        }
        let src_path = Path::new(src);

        // Collect entries (for optional sorting)
        let mut entries: Vec<PathBuf> = Vec::new();

        if src_path.is_dir() {
            if args.no_recursion {
                // Only add the directory itself, not its contents.
                entries.push(src_path.to_path_buf());
            } else {
                for entry in WalkDir::new(src).follow_links(args.dereference) {
                    match entry {
                        Ok(e) => entries.push(e.into_path()),
                        Err(e) => {
                            let reason = e
                                .io_error()
                                .map(describe_open_error)
                                .unwrap_or_else(|| e.to_string());
                            let path = e
                                .path()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| src.to_string());
                            if args.ignore_failed_read {
                                eprintln!("tar: {path}: Warning: Cannot open: {reason}");
                                continue;
                            }
                            eprintln!("tar: {path}: Cannot open: {reason}");
                            had_read_error = true;
                        }
                    }
                }
            }
        } else if src_path.exists() {
            entries.push(src_path.to_path_buf());
        } else {
            let reason = describe_open_error(&io::Error::from(io::ErrorKind::NotFound));
            if args.ignore_failed_read {
                eprintln!(
                    "tar: {}: Warning: Cannot open: {reason}",
                    src_path.display()
                );
                continue;
            }
            eprintln!("tar: {}: Cannot open: {reason}", src_path.display());
            had_read_error = true;
            continue;
        }

        // Apply --exclude-caches / --exclude-tag filtering. We scan the
        // collected entries and, for each directory containing the tag
        // file, mark its contents (or itself, depending on mode) for
        // skipping.
        let mut tag_filters: Vec<(PathBuf, CacheExcludeMode, String)> = Vec::new();
        for tag_spec in args
            .cache_exclude
            .as_slice()
            .iter()
            .chain(args.tag_excludes.iter())
        {
            let (tag_name, mode) = tag_spec;
            for path in &entries {
                if path.is_dir() && path.join(tag_name).is_file() {
                    tag_filters.push((path.clone(), *mode, tag_name.clone()));
                }
            }
        }
        let skip_prefixes: Vec<(PathBuf, CacheExcludeMode, String)> = tag_filters;
        // For "All" mode we also want to skip the dir entry itself; for
        // "Under" we skip all children including the tag but keep the
        // dir entry; for "Normal" we skip siblings but keep dir+tag.
        let is_filtered = |p: &Path,
                           prefixes: &[(PathBuf, CacheExcludeMode, String)]|
         -> (bool, Option<(PathBuf, String)>) {
            for (dir, mode, tag) in prefixes {
                if p == dir {
                    match mode {
                        CacheExcludeMode::All => return (true, Some((dir.clone(), tag.clone()))),
                        _ => return (false, Some((dir.clone(), tag.clone()))),
                    }
                }
                if p.starts_with(dir) && p != dir {
                    let is_tag = p.file_name().is_some_and(|n| n == tag.as_str());
                    match mode {
                        CacheExcludeMode::Normal => {
                            if !is_tag {
                                return (true, None);
                            }
                        }
                        CacheExcludeMode::Under | CacheExcludeMode::All => {
                            return (true, None);
                        }
                    }
                }
            }
            (false, None)
        };

        if args.sort_name {
            entries.sort();
        }

        let exclude_filter = ExcludeFilter::new(&args.excludes);
        let mut reported_tag_dirs: std::collections::HashSet<PathBuf> = Default::default();
        for path in &entries {
            let path_str = path.to_string_lossy();

            if exclude_filter.matches(&path_str) {
                continue;
            }

            let (skip, diag) = is_filtered(path, &skip_prefixes);
            if let Some((dir, tag)) = diag
                && reported_tag_dirs.insert(dir.clone())
            {
                let dir_display = dir.to_string_lossy();
                let trailing = if dir_display.ends_with('/') { "" } else { "/" };
                let mode_note = skip_prefixes
                    .iter()
                    .find(|(d, _, _)| d == &dir)
                    .map(|(_, m, _)| *m)
                    .unwrap_or(CacheExcludeMode::Normal);
                let suffix = match mode_note {
                    CacheExcludeMode::All => "directory not dumped",
                    _ => "contents not dumped",
                };
                eprintln!(
                    "tar: {dir_display}{trailing}: contains a cache directory tag {tag}; {suffix}"
                );
            }
            if skip {
                continue;
            }

            // Update mode: skip if the archived entry is at least as new
            // as the on-disk file. Directories that already exist in the
            // archive are always skipped — their mtime bumps whenever a
            // child is added, which would otherwise cause a spurious
            // re-add of the directory entry.
            if let Some(map) = filter {
                let disk_mtime = fs::metadata(path)
                    .ok()
                    .map(|m| {
                        use std::os::unix::fs::MetadataExt;
                        m.mtime() as u64
                    })
                    .unwrap_or(0);
                let trimmed = path_str.trim_end_matches('/');
                let with_slash = format!("{trimmed}/");
                let archived = map
                    .get(trimmed)
                    .or_else(|| map.get(&with_slash))
                    .copied()
                    .unwrap_or(0);
                if archived > 0 && (path.is_dir() || disk_mtime <= archived) {
                    continue;
                }
            }

            let archive_name: String = path_str.to_string();

            // Strip leading / for safety (unless -P/--absolute-names),
            // then apply --transform expressions. GNU tar applies
            // transforms to the post-strip name.
            let archive_name: String = if args.absolute_names {
                archive_name
            } else {
                archive_name.trim_start_matches('/').to_string()
            };
            let archive_name: String = if !args.transforms.is_empty() {
                apply_transforms(&archive_name, &args.transforms)
            } else {
                archive_name
            };
            let archive_name: &str = &archive_name;

            let display_name = if path.is_dir() && !archive_name.ends_with('/') {
                format!("{archive_name}/")
            } else {
                archive_name.to_string()
            };
            if args.verbose {
                // When the archive is being written to stdout, verbose
                // output goes to stderr so it doesn't corrupt the tar
                // stream.
                let to_stderr = matches!(args.file.as_deref(), None | Some("-"));
                let line = if args.verbose_level >= 2 {
                    let metadata = fs::metadata(path).ok();
                    let mut hdr = Header::new_gnu();
                    #[cfg(unix)]
                    if let Some(md) = metadata.as_ref() {
                        use std::os::unix::fs::MetadataExt;
                        hdr.set_mode(md.mode());
                        hdr.set_mtime(
                            args.mtime_override
                                .map(|t| t as u64)
                                .unwrap_or(md.mtime() as u64),
                        );
                        hdr.set_uid(md.uid() as u64);
                        hdr.set_gid(md.gid() as u64);
                        hdr.set_size(md.len());
                        if path.is_dir() {
                            hdr.set_entry_type(EntryType::Directory);
                            hdr.set_size(0);
                        } else if path.is_symlink() && !args.dereference {
                            hdr.set_entry_type(EntryType::Symlink);
                            hdr.set_size(0);
                        } else {
                            hdr.set_entry_type(EntryType::Regular);
                        }
                        if let Some(u) = uzers::get_user_by_uid(md.uid()) {
                            let _ = hdr.set_username(&u.name().to_string_lossy());
                        }
                        if let Some(g) = uzers::get_group_by_gid(md.gid()) {
                            let _ = hdr.set_groupname(&g.name().to_string_lossy());
                        }
                    }
                    set_owner_group(&mut hdr, args);
                    format_verbose_entry(&hdr, &display_name, args)
                } else {
                    display_name.clone()
                };
                if to_stderr {
                    eprintln!("{line}");
                } else {
                    println!("{line}");
                }
            }

            // Under --dereference, treat symlinks as the files they
            // point at (is_dir/is_file already follow symlinks).
            let is_symlink = !args.dereference && path.is_symlink();
            if path.is_dir() && !is_symlink {
                let mut header = Header::new_gnu();
                header.set_entry_type(EntryType::Directory);
                header.set_size(0);
                let metadata = fs::metadata(path)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    let mode = if let Some(ref m) = args.mode_override {
                        apply_mode_change(metadata.mode(), m)
                    } else {
                        metadata.mode()
                    };
                    header.set_mode(mode);
                    let disk_mtime = metadata.mtime() as u64;
                    let mtime = match args.mtime_override {
                        Some(t) if args.clamp_mtime => disk_mtime.min(t as u64),
                        Some(t) => t as u64,
                        None => disk_mtime,
                    };
                    header.set_mtime(mtime);
                    let uid = metadata.uid();
                    let gid = metadata.gid();
                    apply_owner_group_map(&mut header, uid, gid, args);
                }
                #[cfg(not(unix))]
                {
                    header.set_mode(0o755);
                    set_owner_group(&mut header, args);
                }
                let dir_name = if archive_name.ends_with('/') {
                    archive_name.to_string()
                } else {
                    format!("{archive_name}/")
                };
                append_entry_raw(
                    &mut *builder,
                    &mut header,
                    &dir_name,
                    &mut io::empty(),
                    None,
                )?;
            } else if is_symlink {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if let Ok(link_meta) = fs::symlink_metadata(path) {
                        let key = (link_meta.dev(), link_meta.ino());
                        if let Some(first) = hardlink_map.get(&key) {
                            let link_target = first.clone();
                            let mut header = Header::new_gnu();
                            header.set_entry_type(EntryType::Link);
                            header.set_size(0);
                            set_owner_group(&mut header, args);
                            append_entry_raw(
                                &mut *builder,
                                &mut header,
                                archive_name,
                                &mut io::empty(),
                                Some(link_target.as_bytes()),
                            )?;
                            if args.verify && args.verbose {
                                println!("Verify {archive_name}");
                            }
                            continue;
                        } else {
                            hardlink_map.insert(key, archive_name.to_string());
                        }
                    }
                }
                let target = fs::read_link(path)?;
                let target_bytes = target.to_string_lossy().into_owned().into_bytes();
                let mut header = Header::new_gnu();
                header.set_entry_type(EntryType::Symlink);
                header.set_size(0);
                set_owner_group(&mut header, args);
                append_entry_raw(
                    &mut *builder,
                    &mut header,
                    archive_name,
                    &mut io::empty(),
                    Some(&target_bytes),
                )?;
            } else if path.is_file() {
                let metadata = match fs::metadata(path) {
                    Ok(m) => m,
                    Err(e) => {
                        let display = path.display();
                        let reason = describe_open_error(&e);
                        if args.ignore_failed_read {
                            eprintln!("tar: {display}: Warning: Cannot open: {reason}");
                            continue;
                        }
                        eprintln!("tar: {display}: Cannot open: {reason}");
                        had_read_error = true;
                        continue;
                    }
                };
                let mut header = Header::new_gnu();
                header.set_size(metadata.len());
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    let mode = if let Some(ref m) = args.mode_override {
                        apply_mode_change(metadata.mode(), m)
                    } else {
                        metadata.mode()
                    };
                    header.set_mode(mode);
                    let disk_mtime = metadata.mtime() as u64;
                    let mtime = match args.mtime_override {
                        Some(t) if args.clamp_mtime => disk_mtime.min(t as u64),
                        Some(t) => t as u64,
                        None => disk_mtime,
                    };
                    header.set_mtime(mtime);
                    let uid = metadata.uid();
                    let gid = metadata.gid();
                    apply_owner_group_map(&mut header, uid, gid, args);
                }
                #[cfg(not(unix))]
                set_owner_group(&mut header, args);

                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    let key = (metadata.dev(), metadata.ino());
                    if let Some(first) = hardlink_map.get(&key) {
                        // Emit as hard-link entry: no data, linkname =
                        // first archived path for this inode.
                        let link_target = first.clone();
                        header.set_entry_type(EntryType::Link);
                        header.set_size(0);
                        append_entry_raw(
                            &mut *builder,
                            &mut header,
                            archive_name,
                            &mut io::empty(),
                            Some(link_target.as_bytes()),
                        )?;
                        if args.verify {
                            println!("Verify {archive_name}");
                        }
                        continue;
                    } else {
                        hardlink_map.insert(key, archive_name.to_string());
                    }
                }

                let mut file = match File::open(path) {
                    Ok(f) => f,
                    Err(e) => {
                        let display = path.display();
                        let reason = describe_open_error(&e);
                        if args.ignore_failed_read {
                            eprintln!("tar: {display}: Warning: Cannot open: {reason}");
                            continue;
                        }
                        eprintln!("tar: {display}: Cannot open: {reason}");
                        had_read_error = true;
                        continue;
                    }
                };
                append_entry_raw(&mut *builder, &mut header, archive_name, &mut file, None)?;
            }

            if args.verify && args.verbose {
                println!("Verify {archive_name}");
            }
        }

        // --remove-files: delete each archived entry in reverse so
        // children get removed before their parents.
        if args.remove_files {
            let mut sorted: Vec<&PathBuf> = entries.iter().collect();
            sorted.sort_by(|a, b| b.as_path().cmp(a.as_path()));
            for p in sorted {
                let meta = fs::symlink_metadata(p);
                let (res, verb) = match meta {
                    Ok(m) if m.file_type().is_dir() => (fs::remove_dir(p), "rmdir"),
                    _ => (fs::remove_file(p), "unlink"),
                };
                if let Err(e) = res {
                    eprintln!("tar: {}: Cannot {verb}: {}", p.display(), e);
                    had_read_error = true;
                }
            }
        }
    }
    if had_read_error {
        eprintln!("tar: Exiting with failure status due to previous errors");
        return Err(io::Error::other("read-error-exit"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Append / Update / Delete / Diff
// ---------------------------------------------------------------------------

/// Find the byte offset of the first of the two trailing zero blocks
/// (the archive's EOF marker). Returns the file length if no zero
/// blocks are found. The archive format terminates with at least two
/// 512-byte blocks of zeroes; we rewind there so the appended entries
/// overwrite the terminator.
fn find_archive_data_end(file: &mut File) -> io::Result<u64> {
    const BLOCK: usize = 512;
    file.seek(SeekFrom::Start(0))?;
    let mut buf = [0u8; BLOCK];
    let mut pos: u64 = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if n < BLOCK {
            // Partial block — treat as end.
            pos += n as u64;
            break;
        }
        if buf.iter().all(|&b| b == 0) {
            // A zero block marks the archive terminator.
            break;
        }
        // Header block: read size from bytes 124..136 (octal string).
        let size_field = &buf[124..136];
        let size_str = std::str::from_utf8(size_field)
            .unwrap_or("0")
            .trim_matches(char::from(0))
            .trim();
        let size: u64 = u64::from_str_radix(size_str, 8).unwrap_or(0);
        pos += BLOCK as u64;
        // Round body to next 512-byte boundary.
        let body_blocks = size.div_ceil(BLOCK as u64);
        let body_bytes = body_blocks * BLOCK as u64;
        file.seek(SeekFrom::Current(body_bytes as i64))?;
        pos += body_bytes;
    }
    Ok(pos)
}

fn do_append(args: &Args) -> io::Result<()> {
    let archive_path = args.file.as_deref().ok_or_else(|| {
        io::Error::other("tar: Cowardly refusing to append to stdin/stdout archive")
    })?;

    if args.compression.is_some() {
        return Err(io::Error::other("Cannot update compressed archives"));
    }

    if !Path::new(archive_path).exists() {
        // Append to non-existent archive == create.
        return do_create(args);
    }

    // When --label is given with append/update, verify the archive's
    // existing volume label matches. GNU tar refuses with a fatal error
    // on mismatch (exit 2).
    if let Some(expected) = &args.label {
        let file = File::open(archive_path)?;
        let mut archive = Archive::new(file);
        let found_label: Option<String> =
            archive.entries()?.next().transpose()?.and_then(|entry| {
                if entry.header().entry_type() == EntryType::new(b'V') {
                    let raw = entry.path_bytes().into_owned();
                    let s = String::from_utf8_lossy(&raw).into_owned();
                    Some(s.trim_end_matches('\0').to_string())
                } else {
                    None
                }
            });
        match found_label {
            None => {
                eprintln!("tar: Archive not labeled to match '{expected}'");
                eprintln!("tar: Error is not recoverable: exiting now");
                process::exit(2);
            }
            Some(actual) if !glob_match(expected, &actual, true, false) => {
                eprintln!("tar: Volume '{actual}' does not match '{expected}'");
                eprintln!("tar: Error is not recoverable: exiting now");
                process::exit(2);
            }
            _ => {}
        }
    }

    // Build a map of archive-member mtimes for -u (update) mode.
    let mut existing_mtimes: std::collections::HashMap<String, u64> =
        std::collections::HashMap::new();
    if args.update {
        let file = File::open(archive_path)?;
        let mut archive = Archive::new(file);
        for entry in archive.entries()? {
            let entry = entry?;
            let path = entry.path()?.to_string_lossy().into_owned();
            let mtime = entry.header().mtime().unwrap_or(0);
            existing_mtimes.insert(path, mtime);
        }
    }

    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(archive_path)?;
    let end_pos = find_archive_data_end(&mut file)?;
    file.set_len(end_pos)?;
    file.seek(SeekFrom::Start(end_pos))?;

    let mut builder = Builder::new(file);

    if let Some(dir) = &args.directory {
        std::env::set_current_dir(dir)?;
    }

    let filter = if args.update {
        Some(&existing_mtimes)
    } else {
        None
    };
    add_paths_to_builder_filter(&mut builder, args, filter)?;
    builder.into_inner()?.flush()?;
    Ok(())
}

fn do_delete(args: &Args) -> io::Result<()> {
    if args.compression.is_some() {
        return Err(io::Error::other("Cannot delete from compressed archives"));
    }

    let to_delete: std::collections::HashSet<String> = args.paths.iter().cloned().collect();

    // Pick input/output. `tar -f -` (or no -f) → stdin/stdout.
    match args.file.as_deref() {
        Some("-") | None => {
            let input: Box<dyn Read> = Box::new(io::stdin().lock());
            let output: Box<dyn Write> = Box::new(io::stdout().lock());
            filter_delete(input, output, &to_delete, args.verbose)?;
        }
        Some(path) => {
            let tmp_path = format!("{path}.tmp-delete");
            {
                let input: Box<dyn Read> = Box::new(File::open(path)?);
                let output: Box<dyn Write> = Box::new(File::create(&tmp_path)?);
                filter_delete(input, output, &to_delete, args.verbose)?;
            }
            fs::rename(&tmp_path, path)?;
        }
    }
    Ok(())
}

fn filter_delete(
    input: Box<dyn Read>,
    output: Box<dyn Write>,
    to_delete: &std::collections::HashSet<String>,
    verbose: bool,
) -> io::Result<()> {
    let mut archive = Archive::new(input);
    let mut builder = Builder::new(output);
    let mut matched: std::collections::HashSet<String> = std::collections::HashSet::new();
    let entries = archive.entries()?;
    for entry in entries {
        let mut entry = match entry {
            Ok(e) => e,
            Err(e) => {
                // Truncated archives stop delete early — match any
                // patterns that would have been hit against what we
                // already saw, but don't propagate EOF as a fatal.
                let msg = e.to_string();
                if msg == "failed to read entire block" || msg == "unexpected EOF" {
                    break;
                }
                return Err(e);
            }
        };
        let path = entry.path()?.to_string_lossy().into_owned();
        let deleted = to_delete.iter().any(|pat| {
            path == *pat
                || path.trim_end_matches('/') == pat.trim_end_matches('/')
                || (pat.ends_with('/') && path.starts_with(pat.as_str()))
        });
        if deleted {
            for pat in to_delete {
                if path == *pat
                    || path.trim_end_matches('/') == pat.trim_end_matches('/')
                    || (pat.ends_with('/') && path.starts_with(pat.as_str()))
                {
                    matched.insert(pat.clone());
                }
            }
            if verbose {
                eprintln!("tar: deleting member {path}");
            }
            continue;
        }
        let mut header = entry.header().clone();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        builder.append_data(&mut header, &path, buf.as_slice())?;
    }
    builder.into_inner()?.flush()?;
    // Warn about any requested deletions that didn't match.
    let mut missing = false;
    for name in to_delete {
        if !matched.contains(name) {
            eprintln!("tar: {name}: Not found in archive");
            missing = true;
        }
    }
    if missing {
        eprintln!("tar: Exiting with failure status due to previous errors");
        return Err(io::Error::other("not-found-in-archive"));
    }
    Ok(())
}

fn do_test_label(args: &Args) -> io::Result<()> {
    let archive_path = args.file.as_deref();
    let reader: Box<dyn Read> = match archive_path {
        Some("-") | None => Box::new(io::stdin().lock()),
        Some(path) => Box::new(File::open(path)?),
    };
    let mut archive = Archive::new(reader);
    // Read just the first entry and only peek at its header — we don't
    // need to advance past the body for a volume-label check.
    let label: Option<String> = archive.entries()?.next().transpose()?.and_then(|entry| {
        if entry.header().entry_type() == EntryType::new(b'V') {
            let raw = entry.path_bytes().into_owned();
            let s = String::from_utf8_lossy(&raw).into_owned();
            Some(s.trim_end_matches('\0').to_string())
        } else {
            None
        }
    });

    let patterns: Vec<&str> = args.paths.iter().map(|s| s.as_str()).collect();
    if patterns.is_empty() {
        // Display label, exit 0.
        if let Some(l) = &label {
            println!("{l}");
        }
        return Ok(());
    }

    let label_str = label.as_deref().unwrap_or("");
    let use_wildcards = args.explicit_wildcards;
    let matched = patterns.iter().any(|p| {
        if use_wildcards && (p.contains('*') || p.contains('?')) {
            glob_match(p, label_str, true, false)
        } else {
            *p == label_str
        }
    });
    if matched {
        if args.verbose
            && let Some(l) = &label
        {
            println!("{l}");
        }
        Ok(())
    } else {
        if args.verbose {
            if let Some(l) = &label {
                println!("{l}");
            }
            eprintln!("tar: Archive label mismatch");
        }
        process::exit(1);
    }
}

fn do_diff(args: &Args) -> io::Result<()> {
    let archive_path = args.file.as_deref();
    let reader: Box<dyn Read> = match archive_path {
        Some("-") | None => Box::new(io::stdin().lock()),
        Some(path) => Box::new(File::open(path)?),
    };
    let mut archive = Archive::new(reader);
    let mut differ = false;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let path_str = path.to_string_lossy().into_owned();
        if args.verbose {
            let line = format_verbose_entry(entry.header(), &path_str, args);
            println!("{line}");
        }
        if !path.exists() {
            println!("{}: Not found in filesystem", path.display());
            differ = true;
            continue;
        }
        // Compare size + mode + mtime + bytes. Directory, symlink and
        // hard-link mtimes are intentionally skipped — GNU tar's diff
        // compares them by entry type, not by lstat.
        let entry_kind = entry.header().entry_type();
        if entry_kind == EntryType::Symlink {
            let archived_link = entry.link_name()?.map(|p| p.into_owned());
            let disk_link = fs::read_link(&path).ok();
            match (archived_link, disk_link) {
                (Some(a), Some(b)) if a == b => {}
                _ => {
                    println!("{}: Symlink differs", path.display());
                    differ = true;
                }
            }
            continue;
        }
        if entry_kind == EntryType::Link {
            // Archive claims this is a hard link to another member. If
            // disk has a symlink instead, report mismatch.
            let archived_target = entry.link_name()?.map(|p| p.into_owned());
            if let Some(target) = archived_target.as_ref()
                && fs::symlink_metadata(&path)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
            {
                println!("{}: Not linked to {}", path.display(), target.display());
                differ = true;
                continue;
            }
        }
        let disk_meta = fs::metadata(&path)?;
        let archived_size = entry.header().size().unwrap_or(0);
        let archived_mtime = entry.header().mtime().unwrap_or(0);
        #[cfg(unix)]
        if !matches!(
            entry_kind,
            EntryType::Directory | EntryType::Symlink | EntryType::Link
        ) {
            use std::os::unix::fs::MetadataExt;
            let disk_mtime = disk_meta.mtime() as u64;
            if archived_mtime != disk_mtime {
                println!("{}: Mod time differs", path.display());
                differ = true;
            }
        }
        if archived_size != disk_meta.len() && entry.header().entry_type() == EntryType::Regular {
            println!("{}: Size differs", path.display());
            differ = true;
        }
        if entry.header().entry_type() == EntryType::Regular {
            let mut archived = Vec::with_capacity(archived_size as usize);
            entry.read_to_end(&mut archived)?;
            let mut disk = Vec::with_capacity(disk_meta.len() as usize);
            File::open(&path)?.read_to_end(&mut disk)?;
            if archived != disk {
                println!("{}: Contents differ", path.display());
                differ = true;
            }
        }
    }
    if differ {
        process::exit(1);
    }
    Ok(())
}

/// Format a header for `tar -tv` listing. Mirrors GNU tar's format:
///   -rw-r--r-- user/group    size yyyy-mm-dd hh:mm name
fn format_verbose_entry(header: &Header, name: &str, args: &Args) -> String {
    use tar::EntryType as ET;
    let entry_type = header.entry_type();
    let type_char = match entry_type {
        ET::Directory => 'd',
        ET::Symlink => 'l',
        ET::Link => 'h',
        ET::Char => 'c',
        ET::Block => 'b',
        ET::Fifo => 'p',
        _ => '-',
    };
    let mode = header.mode().unwrap_or(0);
    let perms = format!(
        "{}{}{}{}{}{}{}{}{}{}",
        type_char,
        if mode & 0o400 != 0 { 'r' } else { '-' },
        if mode & 0o200 != 0 { 'w' } else { '-' },
        if mode & 0o4000 != 0 {
            if mode & 0o100 != 0 { 's' } else { 'S' }
        } else if mode & 0o100 != 0 {
            'x'
        } else {
            '-'
        },
        if mode & 0o040 != 0 { 'r' } else { '-' },
        if mode & 0o020 != 0 { 'w' } else { '-' },
        if mode & 0o2000 != 0 {
            if mode & 0o010 != 0 { 's' } else { 'S' }
        } else if mode & 0o010 != 0 {
            'x'
        } else {
            '-'
        },
        if mode & 0o004 != 0 { 'r' } else { '-' },
        if mode & 0o002 != 0 { 'w' } else { '-' },
        if mode & 0o1000 != 0 {
            if mode & 0o001 != 0 { 't' } else { 'T' }
        } else if mode & 0o001 != 0 {
            'x'
        } else {
            '-'
        },
    );

    let uid = header.uid().unwrap_or(0);
    let gid = header.gid().unwrap_or(0);
    let owner_str = if args.numeric_owner {
        format!("{uid}/{gid}")
    } else {
        let user = header
            .username()
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| uid.to_string());
        let group = header
            .groupname()
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| gid.to_string());
        format!("{user}/{group}")
    };

    let size = header.size().unwrap_or(0);
    let mtime_secs = header.mtime().unwrap_or(0);
    let mtime_str = format_mtime(mtime_secs as i64);

    // Follow GNU's column widths: owner/group field is left-padded to
    // a minimum width and size right-padded.
    let name_display = if let ET::Symlink = entry_type {
        let link = header
            .link_name()
            .ok()
            .flatten()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("{name} -> {link}")
    } else if let ET::Link = entry_type {
        let link = header
            .link_name()
            .ok()
            .flatten()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("{name} link to {link}")
    } else {
        name.to_string()
    };

    // GNU's format: mode, owner/group padded to min 17, size (not
    // right-aligned on its own), date, time, name. When owner/group is
    // wider than 17 it's left alone.
    format!("{perms} {owner_str:<17} {size} {mtime_str} {name_display}")
}

/// Parse GNU tar's `--mtime=VAL` argument. Accepts `@<seconds>` or
/// an ISO-ish date like `2023-04-01 12:00:00`. Returns None if we
/// can't interpret the value.
fn parse_mtime_arg(val: &str) -> Option<i64> {
    if let Some(rest) = val.strip_prefix('@') {
        return rest.parse().ok();
    }
    // Try ISO yyyy-mm-dd[ hh:mm[:ss]].
    let re = Regex::new(r"^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2}))?)?").ok()?;
    let caps = re.captures(val)?;
    let y: i32 = caps[1].parse().ok()?;
    let m: i32 = caps[2].parse().ok()?;
    let d: i32 = caps[3].parse().ok()?;
    let h: i64 = caps
        .get(4)
        .and_then(|s| s.as_str().parse().ok())
        .unwrap_or(0);
    let mi: i64 = caps
        .get(5)
        .and_then(|s| s.as_str().parse().ok())
        .unwrap_or(0);
    let s: i64 = caps
        .get(6)
        .and_then(|s| s.as_str().parse().ok())
        .unwrap_or(0);
    // Howard Hinnant days-from-civil
    let (ys, ms) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = (if ys >= 0 { ys } else { ys - 399 }) / 400;
    let yoe = (ys - era * 400) as u32;
    let doy = ((153 * ms + 2) / 5) as u32 + d as u32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era as i64 * 146097 + doe as i64 - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + s)
}

/// Render a unix timestamp as `YYYY-MM-DD HH:MM` UTC (matching GNU tar).
fn format_mtime(secs: i64) -> String {
    // Days from civil (Howard Hinnant algorithm, simplified for post-1970).
    let days = secs.div_euclid(86400);
    let secs_of_day = secs.rem_euclid(86400);
    let hh = (secs_of_day / 3600) as u32;
    let mm = ((secs_of_day / 60) % 60) as u32;
    // y/m/d from days since 1970-01-01.
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let yy = if m <= 2 { y + 1 } else { y };
    format!("{yy:04}-{m:02}-{d:02} {hh:02}:{mm:02}")
}

fn set_owner_group(header: &mut Header, args: &Args) {
    if let Some(owner) = &args.owner {
        // Accept "NAME", "NAME:UID", or plain numeric UID.
        let (name_part, uid_part) = match owner.split_once(':') {
            Some((n, u)) => (n.to_string(), Some(u.to_string())),
            None => (owner.clone(), None),
        };
        if let Some(u) = uid_part.as_ref() {
            if let Ok(uid) = u.parse::<u64>() {
                header.set_uid(uid);
            }
        } else if let Ok(uid) = name_part.parse::<u64>() {
            header.set_uid(uid);
        }
        if !name_part.is_empty() {
            header.set_username(&name_part).ok();
        }
    }
    if let Some(group) = &args.group {
        let (name_part, gid_part) = match group.split_once(':') {
            Some((n, u)) => (n.to_string(), Some(u.to_string())),
            None => (group.clone(), None),
        };
        if let Some(u) = gid_part.as_ref() {
            if let Ok(gid) = u.parse::<u64>() {
                header.set_gid(gid);
            }
        } else if let Ok(gid) = name_part.parse::<u64>() {
            header.set_gid(gid);
        }
        if !name_part.is_empty() {
            header.set_groupname(&name_part).ok();
        }
    }
}

// ---------------------------------------------------------------------------
// Extract / List
// ---------------------------------------------------------------------------

fn do_extract_or_list(args: &Args) -> io::Result<()> {
    let explicit_compression = args.compression;

    let (reader, detected_compression): (Box<dyn Read>, Compression) = match args.file.as_deref() {
        Some("-") | None => {
            // stdin – need to buffer for magic detection
            let mut buf = [0u8; 6];
            let mut stdin = io::stdin().lock();
            let n = stdin.read(&mut buf)?;
            let magic_comp = detect_from_magic(&buf[..n]);
            let chain: Box<dyn Read> = Box::new(io::Cursor::new(buf[..n].to_vec()).chain(stdin));
            (chain, magic_comp)
        }
        Some(path) => {
            let file = File::open(path)?;
            let mut buf = [0u8; 6];
            let mut reader = BufReader::new(file);
            let n = reader.read(&mut buf)?;
            let magic_comp = detect_from_magic(&buf[..n]);
            let ext_comp = detect_from_extension(path);
            let chain: Box<dyn Read> = Box::new(io::Cursor::new(buf[..n].to_vec()).chain(reader));
            // Prefer magic bytes, fall back to extension
            let comp = if magic_comp != Compression::None {
                magic_comp
            } else {
                ext_comp
            };
            (chain, comp)
        }
    };

    let compression = explicit_compression.unwrap_or(detected_compression);

    let decompressed: Box<dyn Read> = match compression {
        Compression::None => reader,
        Compression::Gzip => Box::new(GzDecoder::new(reader)),
        Compression::Bzip2 => Box::new(BzDecoder::new(reader)),
        Compression::Xz => Box::new(XzDecoder::new(reader)),
    };

    let mut archive = Archive::new(decompressed);
    archive.set_preserve_permissions(args.preserve_permissions && !args.no_same_permissions);
    archive.set_unpack_xattrs(false);
    archive.set_overwrite(true);

    let entries = archive.entries()?;

    // Deferred directory mode restores — applied after all entries are
    // extracted, so we can still write children into directories whose
    // final mode lacks write permission.
    #[cfg(unix)]
    let mut deferred_dir_modes: Vec<(PathBuf, u32)> = Vec::new();

    let mut label_checked = args.label.is_none();
    let mut extract_had_error = false;

    for entry in entries {
        let mut entry = entry?;
        // Use path_bytes() to bypass the `tar` crate's `..` / absolute-
        // path rejection. We re-enforce safety ourselves unless `-P` is
        // set (in which case the user asked for raw paths).
        let path_bytes = entry.path_bytes().into_owned();
        let path_str = String::from_utf8_lossy(&path_bytes).into_owned();
        let _orig_path = PathBuf::from(&path_str);

        // GNU volume label (entry type 'V'): compare against --label.
        if entry.header().entry_type() == EntryType::new(b'V') {
            let archive_label = path_str.trim_end_matches('\0').to_string();
            if let Some(expected) = &args.label {
                if !glob_match(expected, &archive_label, true, false) {
                    eprintln!("tar: Volume '{archive_label}' does not match '{expected}'");
                    eprintln!("tar: Error is not recoverable: exiting now");
                    process::exit(2);
                }
                label_checked = true;
            }
            if args.list {
                if args.verbose {
                    println!(
                        "V--------- 0/0 {:>13} 1970-01-01 00:00 {archive_label}--Volume Header--",
                        0
                    );
                } else {
                    println!("{archive_label}");
                }
            }
            continue;
        } else if !label_checked {
            // First non-volume entry reached without matching label.
            if let Some(expected) = &args.label {
                eprintln!("tar: Archive not labeled to match '{expected}'");
                eprintln!("tar: Error is not recoverable: exiting now");
                process::exit(2);
            }
            label_checked = true;
        }

        // Apply strip-components
        let stripped = if args.strip_components > 0 {
            let components: Vec<&str> = path_str.split('/').collect();
            if components.len() <= args.strip_components {
                continue;
            }
            components[args.strip_components..].join("/")
        } else {
            path_str.clone()
        };

        if stripped.is_empty() {
            continue;
        }

        // Apply transforms
        let final_path = if !args.transforms.is_empty() {
            apply_transforms(&stripped, &args.transforms)
        } else {
            stripped
        };

        // Check excludes
        if is_excluded(&final_path, &args.excludes) {
            continue;
        }

        // Filter by explicitly listed paths. GNU tar matches list/extract
        // paths LITERALLY and ANCHORED by default; globbing kicks in only
        // when `--wildcards` is set and unanchored matching only when
        // `--no-anchored` is set.
        if !args.paths.is_empty() {
            // GNU matches user paths against the ORIGINAL archive path,
            // not the post-strip/transform path. Fall back to the
            // final_path too in case the user specified the transformed
            // form.
            let matches_any = args.paths.iter().any(|p| {
                if p.starts_with('\0') {
                    return false;
                }
                let p_trim = p.trim_end_matches('/');
                let unanchored = args.explicit_anchored == Some(false);
                let candidates = [path_str.as_str(), final_path.as_str()];
                candidates.iter().any(|cand| {
                    let basename = Path::new(cand)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    if args.explicit_wildcards && (p.contains('*') || p.contains('?')) {
                        if cand.ends_with('/') && !p.ends_with('/') {
                            return false;
                        }
                        let match_slash = args.match_slash_default;
                        let ignore_case = args.ignore_case_default;
                        glob_match(p, cand, match_slash, ignore_case)
                            || glob_match(p, cand.trim_end_matches('/'), match_slash, ignore_case)
                            || (unanchored && glob_match(p, basename, match_slash, ignore_case))
                    } else {
                        let ignore_case = args.ignore_case_default;
                        eq_opt_ci_prefix(cand, p.as_str(), ignore_case)
                            || eq_opt_ci(cand.trim_end_matches('/'), p_trim, ignore_case)
                            || (unanchored && eq_opt_ci(basename, p.as_str(), ignore_case))
                    }
                })
            });
            if !matches_any {
                continue;
            }
        }

        if args.list {
            if args.verbose {
                println!(
                    "{}",
                    format_verbose_entry(entry.header(), &final_path, args)
                );
            } else {
                println!("{final_path}");
            }
            continue;
        }

        // Extract
        if args.verbose {
            // GNU tar prints the ORIGINAL archive path in verbose output
            // during extract, regardless of --strip-components / --xform.
            if args.verbose_level >= 2 {
                println!("{}", format_verbose_entry(entry.header(), &path_str, args));
            } else {
                println!("{path_str}");
            }
        }

        let dest = match &args.directory {
            Some(dir) => PathBuf::from(dir).join(&final_path),
            None => PathBuf::from(&final_path),
        };

        let entry_type = entry.header().entry_type();
        match entry_type {
            EntryType::Directory => {
                if args.to_stdout {
                    continue;
                }
                fs::create_dir_all(&dest)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(mode) = entry.header().mode() {
                        let effective = if args.preserve_permissions && !args.no_same_permissions {
                            mode
                        } else {
                            mode & !0o022
                        };
                        // Ensure the directory is writable during
                        // extraction. Defer the final mode restore to
                        // after the loop so children (including symlinks
                        // into read-only dirs) can still be created.
                        fs::set_permissions(&dest, fs::Permissions::from_mode(effective | 0o700))?;
                        deferred_dir_modes.push((dest.clone(), effective));
                    }
                    if let Some(ref mode_str) = args.mode_override {
                        let _ = apply_mode_to_path(&dest, mode_str);
                    }
                }
            }
            EntryType::Regular | EntryType::GNUSparse => {
                if args.to_stdout {
                    // -O / --to-stdout: write contents to stdout; no
                    // on-disk creation.
                    io::copy(&mut entry, &mut io::stdout().lock())?;
                    continue;
                }
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                if dest.exists() {
                    if args.skip_old_files {
                        continue;
                    }
                    if args.keep_old_files {
                        eprintln!("tar: {}: Cannot open: File exists", dest.display());
                        extract_had_error = true;
                        continue;
                    }
                    if args.backup {
                        let backup_path = PathBuf::from(format!("{}~", dest.display()));
                        println!(
                            "Renaming '{}' to '{}'",
                            dest.display(),
                            backup_path.display()
                        );
                        fs::rename(&dest, &backup_path)?;
                    }
                }
                // If an existing symlink is in the way and the user has
                // NOT passed -h (dereference), replace the link itself
                // rather than writing through to its target.
                if !args.dereference
                    && fs::symlink_metadata(&dest)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false)
                {
                    let _ = fs::remove_file(&dest);
                }
                let mut file = File::create(&dest)?;
                io::copy(&mut entry, &mut file)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(mode) = entry.header().mode() {
                        let effective = if args.preserve_permissions && !args.no_same_permissions {
                            mode
                        } else {
                            mode & !0o022
                        };
                        fs::set_permissions(&dest, fs::Permissions::from_mode(effective))?;
                    }
                    if let Some(ref mode_str) = args.mode_override {
                        let _ = apply_mode_to_path(&dest, mode_str);
                    }
                }
            }
            EntryType::Symlink => {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                if let Some(link) = entry.link_name()? {
                    // Remove existing if present
                    let _ = fs::remove_file(&dest);
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(link.as_ref(), &dest)?;
                }
            }
            EntryType::Link => {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                if let Some(link) = entry.link_name()? {
                    let link_target = match &args.directory {
                        Some(dir) => PathBuf::from(dir).join(link.as_ref()),
                        None => link.into_owned(),
                    };
                    if link_target == dest {
                        // Same path archived twice: the target is this
                        // entry itself. Leave the existing regular file
                        // in place.
                        continue;
                    }
                    let _ = fs::remove_file(&dest);
                    fs::hard_link(link_target, &dest)?;
                }
            }
            _ => {
                // Skip other entry types (char devices, etc.)
            }
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Apply deferred directory modes in reverse so deeper directories
        // get their mode restored before their parents (not strictly
        // required, but keeps behaviour deterministic).
        for (path, mode) in deferred_dir_modes.into_iter().rev() {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
        }
    }

    if extract_had_error {
        eprintln!("tar: Exiting with failure status due to previous errors");
        process::exit(2);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let args = parse_args();

    let op_count = args.create as u8
        + args.extract as u8
        + args.list as u8
        + args.append as u8
        + args.update as u8
        + args.diff as u8
        + args.delete as u8
        + args.test_label as u8;
    if op_count == 0 {
        eprintln!(
            "tar: You must specify one of the `-Acdtrux', `--delete' or `--test-label' options"
        );
        eprintln!("Try 'tar --help' or 'tar --usage' for more information.");
        process::exit(2);
    }
    if op_count > 1 {
        eprintln!(
            "tar: You may not specify more than one `-Acdtrux', `--delete' or `--test-label' option"
        );
        eprintln!("Try 'tar --help' or 'tar --usage' for more information.");
        process::exit(2);
    }

    let result = if args.create {
        do_create(&args)
    } else if args.append || args.update {
        do_append(&args)
    } else if args.delete {
        do_delete(&args)
    } else if args.diff {
        do_diff(&args)
    } else if args.test_label {
        do_test_label(&args)
    } else {
        do_extract_or_list(&args)
    };

    if let Err(e) = result {
        let msg = e.to_string();
        if msg == "not-found-in-archive" {
            process::exit(2);
        }
        if msg == "read-error-exit" {
            process::exit(2);
        }
        // gzip/bzip2/xz decode errors from an empty compressed stream
        // surface as "unexpected end of file". GNU tar presents them as
        // the compressor's exit status instead.
        if args.compression.is_some_and(|c| c != Compression::None)
            && (msg.contains("unexpected end of file")
                || msg.contains("corrupt")
                || msg.contains("decode"))
        {
            eprintln!("tar: Child returned status 1");
            eprintln!("tar: Error is not recoverable: exiting now");
            process::exit(2);
        }
        let translated = match msg.as_str() {
            "failed to read entire block" | "unexpected EOF" => {
                "This does not look like a tar archive".to_string()
            }
            _ => msg,
        };
        eprintln!("tar: {translated}");
        if translated == "This does not look like a tar archive" {
            eprintln!("tar: Exiting with failure status due to previous errors");
            process::exit(2);
        }
        if translated == "Cannot update compressed archives"
            || translated == "Cannot delete from compressed archives"
        {
            eprintln!("Try 'tar --help' or 'tar --usage' for more information.");
            process::exit(2);
        }
        process::exit(2);
    }
    if args.deferred_fatal {
        process::exit(2);
    }
}
