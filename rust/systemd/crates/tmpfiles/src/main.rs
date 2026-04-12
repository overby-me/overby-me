//! systemd-tmpfiles — Create, delete, and clean up temporary files and directories
//!
//! Reads configuration from tmpfiles.d/*.conf files and creates, deletes, or cleans
//! temporary files and directories. This is a drop-in replacement for systemd-tmpfiles.
//!
//! Configuration is read from (in order of priority):
//!   /etc/tmpfiles.d/*.conf
//!   /run/tmpfiles.d/*.conf
//!   /usr/lib/tmpfiles.d/*.conf
//!   /lib/tmpfiles.d/*.conf
//!
//! Each .conf file contains lines of the form:
//!   Type Path Mode User Group Age Argument
//!
//! Supported types (from tmpfiles.d(5)):
//!   f/f+  — Create a file (optionally write argument as contents)
//!   w/w+  — Write argument to a file (truncate with +)
//!   d     — Create a directory
//!   D     — Create a directory; remove contents when --remove is used
//!   e     — Adjust permissions/ownership of existing directory; clean with age
//!   q     — Create a subvolume or directory (falls back to mkdir)
//!   Q     — Like q but also removes on --remove
//!   p/p+  — Create a named pipe (FIFO)
//!   L/L+  — Create a symlink
//!   c/c+  — Create a character device node
//!   b/b+  — Create a block device node
//!   C/C+  — Recursively copy a file/directory
//!   x     — Ignore path for cleaning (exclude from --clean)
//!   X     — Ignore path and everything below for cleaning
//!   r     — Remove a file or directory (on --remove)
//!   R     — Recursively remove a path (on --remove)
//!   z     — Adjust access mode, ownership, and SELinux context
//!   Z     — Recursively adjust access mode, ownership, and SELinux context
//!   t     — Set extended attributes
//!   T     — Recursively set extended attributes
//!   h     — Set file/directory attributes (chattr)
//!   H     — Recursively set file/directory attributes
//!   a/a+  — Set POSIX ACLs
//!   A/A+  — Recursively set POSIX ACLs

use std::collections::BTreeSet;
use std::ffi::CString;
use std::fs;
use std::io::{self, BufRead};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use clap::Parser;

const EXIT_SUCCESS: u8 = 0;
const EXIT_FAILURE: u8 = 1;

/// Directories to search for tmpfiles.d configuration, in priority order.
/// Earlier directories take precedence when the same filename exists in multiple.
const CONFIG_DIRS: &[&str] = &[
    "/etc/tmpfiles.d",
    "/run/tmpfiles.d",
    "/usr/lib/tmpfiles.d",
    "/lib/tmpfiles.d",
];

/// systemd-tmpfiles — Create, delete, and clean up temporary files and directories
#[derive(Parser, Debug)]
#[command(name = "systemd-tmpfiles", version, about)]
struct Cli {
    /// Create files and directories as specified in the configuration
    #[arg(long)]
    create: bool,

    /// Clean up files and directories older than the configured age
    #[arg(long)]
    clean: bool,

    /// Remove files and directories as specified in the configuration
    #[arg(long)]
    remove: bool,

    /// Also execute lines with an exclamation mark (for early boot)
    #[arg(long)]
    boot: bool,

    /// Only apply rules with paths that start with the specified prefix
    #[arg(long = "prefix")]
    prefixes: Vec<PathBuf>,

    /// Only apply rules with paths that do NOT start with the specified prefix
    #[arg(long = "exclude-prefix")]
    exclude_prefixes: Vec<PathBuf>,

    /// Only print what would be done, without making any changes
    #[arg(long = "dry-run")]
    dry_run: bool,

    /// Remove all items marked with the '$' modifier
    #[arg(long)]
    purge: bool,

    /// Treat all file system errors as non-fatal
    #[arg(long)]
    graceful: bool,

    /// Use an alternate root filesystem path for all file operations
    #[arg(long)]
    root: Option<PathBuf>,

    /// Specific tmpfiles.d config files to read (instead of scanning directories)
    files: Vec<PathBuf>,
}

/// Represents the action type from a tmpfiles.d line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ItemType {
    /// f — Create file (do not overwrite)
    CreateFile,
    /// F — Create or truncate file
    TruncateFile,
    /// w — Write to file
    WriteFile,
    /// d — Create directory
    CreateDirectory,
    /// D — Create or clean directory
    CreateOrCleanDirectory,
    /// e — Adjust existing directory
    AdjustDirectory,
    /// q — Create subvolume or directory
    CreateSubvolume,
    /// Q — Create subvolume or directory, clean on remove
    CreateOrCleanSubvolume,
    /// p — Create FIFO
    CreateFifo,
    /// L — Create symlink
    CreateSymlink,
    /// c — Create character device
    CreateCharDevice,
    /// b — Create block device
    CreateBlockDevice,
    /// C — Copy files/directories
    CopyFiles,
    /// x — Exclude path from cleaning
    IgnorePath,
    /// X — Exclude path and children from cleaning
    IgnoreDirectoryPath,
    /// r — Remove file/directory
    RemovePath,
    /// R — Recursively remove path
    RemoveRecursively,
    /// z — Adjust permissions
    AdjustPermissions,
    /// Z — Recursively adjust permissions
    AdjustPermissionsRecursively,
    /// t — Set extended attributes
    SetExtendedAttributes,
    /// T — Recursively set extended attributes
    SetExtendedAttributesRecursively,
    /// h — Set file attributes
    SetAttributes,
    /// H — Recursively set file attributes
    SetAttributesRecursively,
    /// a — Set POSIX ACLs
    SetACL,
    /// A — Recursively set POSIX ACLs
    SetACLRecursively,
}

impl ItemType {
    fn from_char(c: char, _plus: bool) -> Option<Self> {
        match c {
            'f' => Some(ItemType::CreateFile),
            'F' => Some(ItemType::TruncateFile),
            'w' => Some(ItemType::WriteFile),
            'd' => Some(ItemType::CreateDirectory),
            'D' => Some(ItemType::CreateOrCleanDirectory),
            'e' => Some(ItemType::AdjustDirectory),
            'q' => Some(ItemType::CreateSubvolume),
            'Q' => Some(ItemType::CreateOrCleanSubvolume),
            'p' => Some(ItemType::CreateFifo),
            'L' => Some(ItemType::CreateSymlink),
            'c' => Some(ItemType::CreateCharDevice),
            'b' => Some(ItemType::CreateBlockDevice),
            'C' => Some(ItemType::CopyFiles),
            'x' => Some(ItemType::IgnorePath),
            'X' => Some(ItemType::IgnoreDirectoryPath),
            'r' => Some(ItemType::RemovePath),
            'R' => Some(ItemType::RemoveRecursively),
            'z' => Some(ItemType::AdjustPermissions),
            'Z' => Some(ItemType::AdjustPermissionsRecursively),
            't' => Some(ItemType::SetExtendedAttributes),
            'T' => Some(ItemType::SetExtendedAttributesRecursively),
            'h' => Some(ItemType::SetAttributes),
            'H' => Some(ItemType::SetAttributesRecursively),
            'a' => Some(ItemType::SetACL),
            'A' => Some(ItemType::SetACLRecursively),
            _ => None,
        }
    }

    /// Whether this type is relevant during --create.
    fn is_create_type(self) -> bool {
        matches!(
            self,
            ItemType::CreateFile
                | ItemType::TruncateFile
                | ItemType::WriteFile
                | ItemType::CreateDirectory
                | ItemType::CreateOrCleanDirectory
                | ItemType::AdjustDirectory
                | ItemType::CreateSubvolume
                | ItemType::CreateOrCleanSubvolume
                | ItemType::CreateFifo
                | ItemType::CreateSymlink
                | ItemType::CreateCharDevice
                | ItemType::CreateBlockDevice
                | ItemType::CopyFiles
                | ItemType::AdjustPermissions
                | ItemType::AdjustPermissionsRecursively
                | ItemType::SetExtendedAttributes
                | ItemType::SetExtendedAttributesRecursively
                | ItemType::SetAttributes
                | ItemType::SetAttributesRecursively
                | ItemType::SetACL
                | ItemType::SetACLRecursively
        )
    }

    /// Whether this type is relevant during --remove.
    fn is_remove_type(self) -> bool {
        matches!(
            self,
            ItemType::CreateOrCleanDirectory
                | ItemType::CreateOrCleanSubvolume
                | ItemType::RemovePath
                | ItemType::RemoveRecursively
        )
    }

    /// Whether this type is relevant during --clean.
    fn is_clean_type(self) -> bool {
        matches!(
            self,
            ItemType::CreateDirectory
                | ItemType::CreateOrCleanDirectory
                | ItemType::AdjustDirectory
                | ItemType::CreateSubvolume
                | ItemType::CreateOrCleanSubvolume
                | ItemType::IgnorePath
                | ItemType::IgnoreDirectoryPath
        )
    }
}

/// A parsed tmpfiles.d configuration item.
#[derive(Debug, Clone)]
struct TmpfilesItem {
    /// The action type.
    item_type: ItemType,
    /// Whether the '+' modifier was specified (force/append behavior).
    force: bool,
    /// Whether the '!' modifier was specified (boot-only).
    boot_only: bool,
    /// Whether the '-' modifier was specified (ignore errors).
    minus: bool,
    /// Whether the '$' modifier was specified (purgeable).
    purgeable: bool,
    /// Whether the '?' modifier was specified (conditional on target existing).
    conditional: bool,
    /// The target path.
    path: PathBuf,
    /// File mode (e.g. 0755). None means use default.
    mode: Option<u32>,
    /// Owner user name or UID. None means root/default.
    user: Option<String>,
    /// Owner group name or GID. None means root/default.
    group: Option<String>,
    /// Whether mode has the ':' prefix (only apply on creation).
    mode_create_only: bool,
    /// Whether user has the ':' prefix (only apply on creation).
    user_create_only: bool,
    /// Whether group has the ':' prefix (only apply on creation).
    group_create_only: bool,
    /// Maximum age for cleaning. None means no age limit.
    age: Option<Duration>,
    /// Which timestamps to check for age (a=atime, m=mtime, c=ctime, b=btime, A/M/C/B=dirs).
    /// Empty means use the default (most recent of all timestamps).
    age_by: String,
    /// Argument field (contents for 'f', symlink target for 'L', etc.).
    argument: Option<String>,
    /// Source file for diagnostics.
    source: PathBuf,
    /// Line number in source file.
    line_number: usize,
}

/// Parse a mode string like "0755", "755", or "-" (default).
fn parse_mode(mode_str: &str, default: u32) -> u32 {
    let s = mode_str.trim();
    if s == "-" || s.is_empty() {
        return default;
    }
    // Strip a leading '~' (masked mode, which we treat as plain mode for now)
    let s = s.strip_prefix('~').unwrap_or(s);
    u32::from_str_radix(s, 8).unwrap_or(default)
}

/// Parse a user specification. Returns None for "-" or empty.
fn parse_user(user_str: &str) -> Option<String> {
    let s = user_str.trim();
    if s == "-" || s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Parse an age specification like "10d", "12h", "1w", "a:1m", etc.
/// Returns (age_by_flags, duration). age_by contains letters like "a", "m", "c", "b",
/// "A", "M", "C", "B" indicating which timestamps to use for age comparison.
/// Returns (String, None) for "-" or empty.
fn parse_age(age_str: &str) -> (String, Option<Duration>) {
    let s = age_str.trim();
    if s == "-" || s.is_empty() {
        return (String::new(), None);
    }

    // Remove a leading '~' modifier (cleanup only if not accessed)
    let s = s.strip_prefix('~').unwrap_or(s);

    if s.is_empty() {
        return (String::new(), None);
    }

    // Check for "age-by" prefix: "flags:duration" (e.g. "a:1m", "amcb:3h")
    let (age_by, duration_str) = if let Some(colon_pos) = s.find(':') {
        let prefix = &s[..colon_pos].trim();
        let duration = &s[colon_pos + 1..].trim();
        // Validate that prefix contains only valid age-by chars
        let valid_age_by = prefix.chars().all(|c| "amcbAMCB ".contains(c));
        if valid_age_by && !prefix.is_empty() {
            let flags: String = prefix.chars().filter(|c| !c.is_whitespace()).collect();
            if duration.is_empty() {
                return (String::new(), None); // "m:" is invalid
            }
            (flags, *duration)
        } else {
            // Invalid prefix — the whole thing is invalid
            return (String::new(), None);
        }
    } else {
        (String::new(), s)
    };

    let s = duration_str;
    let mut total_secs: u64 = 0;
    let mut num_buf = String::new();

    for ch in s.chars() {
        if ch.is_ascii_digit() {
            num_buf.push(ch);
        } else {
            let n: u64 = if num_buf.is_empty() {
                1
            } else {
                num_buf.parse().unwrap_or(0)
            };
            num_buf.clear();

            let multiplier = match ch {
                'u' | 'μ' => 0, // microseconds (sub-second, round to 0)
                's' => 1,
                'm' => 60,
                'h' => 3600,
                'd' => 86400,
                'w' => 604800,
                'M' => 2592000,  // 30 days
                'y' => 31536000, // 365 days
                _ => {
                    // Unknown suffix, try parsing the whole thing as seconds
                    return (age_by, s.parse::<u64>().ok().map(Duration::from_secs));
                }
            };

            total_secs += n * multiplier;
        }
    }

    // Trailing number without suffix is treated as seconds
    if !num_buf.is_empty()
        && let Ok(n) = num_buf.parse::<u64>()
    {
        total_secs += n;
    }

    if total_secs == 0 && !s.contains('0') {
        // If we got 0 and the string doesn't contain '0', parsing probably failed
        return (age_by, None);
    }

    (age_by, Some(Duration::from_secs(total_secs)))
}

/// Split a tmpfiles.d line into fields, respecting quoting.
/// Fields are whitespace-separated, but quoted strings (single or double quotes)
/// are kept together, and backslash escapes are processed within quotes.
fn split_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quote: Option<char> = None;

    while let Some(&ch) = chars.peek() {
        match in_quote {
            Some(q) => {
                chars.next();
                if ch == q {
                    in_quote = None;
                } else if ch == '\\' {
                    if let Some(&next) = chars.peek() {
                        chars.next();
                        if next == q || next == '\\' {
                            // Escaped quote or backslash — consume the escape
                            current.push(next);
                        } else {
                            // Preserve backslash for C-style unescaping later
                            current.push('\\');
                            current.push(next);
                        }
                    } else {
                        current.push(ch);
                    }
                } else {
                    current.push(ch);
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    chars.next();
                    in_quote = Some(ch);
                } else if ch.is_whitespace() {
                    chars.next();
                    if !current.is_empty() {
                        fields.push(std::mem::take(&mut current));
                    }
                } else {
                    chars.next();
                    current.push(ch);
                }
            }
        }
    }

    if !current.is_empty() {
        fields.push(current);
    }

    fields
}

/// Expand systemd-tmpfiles specifiers in a string.
/// Supported: %u (user name), %U (numeric UID), %g (group name), %G (numeric GID),
/// %h (home directory), %H (hostname), %% (literal %).
/// Read a key from os-release file(s), optionally under a root prefix.
fn read_os_release_field(key: &str, root: Option<&Path>) -> String {
    let paths = if let Some(r) = root {
        vec![r.join("etc/os-release"), r.join("usr/lib/os-release")]
    } else {
        vec![
            PathBuf::from("/etc/os-release"),
            PathBuf::from("/usr/lib/os-release"),
        ]
    };
    for p in &paths {
        if let Ok(content) = fs::read_to_string(p) {
            for line in content.lines() {
                if let Some(val) = line
                    .strip_prefix(key)
                    .and_then(|rest| rest.strip_prefix('='))
                {
                    return val.trim_matches('"').to_string();
                }
            }
        }
    }
    String::new()
}

fn expand_specifiers(s: &str, root: Option<&Path>) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.peek() {
                Some('%') => {
                    chars.next();
                    result.push('%');
                }
                Some('u') => {
                    chars.next();
                    // Current user name
                    result.push_str(
                        &std::env::var("USER")
                            .or_else(|_| std::env::var("LOGNAME"))
                            .unwrap_or_else(|_| "root".to_string()),
                    );
                }
                Some('U') => {
                    chars.next();
                    // Current numeric UID
                    result.push_str(&unsafe { libc::getuid() }.to_string());
                }
                Some('g') => {
                    chars.next();
                    // Current group name
                    let gid = unsafe { libc::getgid() };
                    let gr = unsafe { libc::getgrgid(gid) };
                    if !gr.is_null() {
                        let name = unsafe { std::ffi::CStr::from_ptr((*gr).gr_name) };
                        result.push_str(&name.to_string_lossy());
                    } else {
                        result.push_str(&gid.to_string());
                    }
                }
                Some('G') => {
                    chars.next();
                    // Current numeric GID
                    result.push_str(&unsafe { libc::getgid() }.to_string());
                }
                Some('h') => {
                    chars.next();
                    // Home directory
                    result.push_str(&std::env::var("HOME").unwrap_or_else(|_| "/root".to_string()));
                }
                Some('H') => {
                    chars.next();
                    // Hostname
                    let mut buf = [0u8; 256];
                    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0 {
                        let hostname = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr().cast()) };
                        result.push_str(&hostname.to_string_lossy());
                    } else {
                        result.push_str("localhost");
                    }
                }
                // os-release specifiers
                Some('o') => {
                    chars.next();
                    result.push_str(&read_os_release_field("ID", root));
                }
                Some('w') => {
                    chars.next();
                    result.push_str(&read_os_release_field("VERSION_ID", root));
                }
                Some('B') => {
                    chars.next();
                    result.push_str(&read_os_release_field("BUILD_ID", root));
                }
                Some('W') => {
                    chars.next();
                    result.push_str(&read_os_release_field("VARIANT_ID", root));
                }
                Some('M') => {
                    chars.next();
                    result.push_str(&read_os_release_field("IMAGE_ID", root));
                }
                Some('A') => {
                    chars.next();
                    result.push_str(&read_os_release_field("IMAGE_VERSION", root));
                }
                _ => {
                    // Unknown specifier — keep as-is
                    result.push('%');
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Parse a single tmpfiles.d configuration line.
fn parse_line(
    line: &str,
    source: &Path,
    line_number: usize,
    root: Option<&Path>,
) -> Option<TmpfilesItem> {
    let trimmed = line.trim();

    // Skip empty lines and comments
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
        return None;
    }

    let fields = split_fields(trimmed);
    if fields.len() < 2 {
        eprintln!(
            "systemd-tmpfiles: {}:{}: too few fields, ignoring.",
            source.display(),
            line_number,
        );
        return None;
    }

    // Parse the type field: may include modifiers like '+', '!', '-', '='
    let type_str = &fields[0];
    let mut type_char = None;
    let mut force = false;
    let mut boot_only = false;
    let mut minus = false;
    let mut purgeable = false;
    let mut conditional = false;

    for c in type_str.chars() {
        match c {
            '+' => force = true,
            '!' => boot_only = true,
            '-' => minus = true,
            '$' => purgeable = true,
            '?' => conditional = true,
            '=' => { /* '=' modifier: don't apply to subdirs, we note but don't special-case */ }
            '~' => { /* '~' modifier: masked mode */ }
            _ => {
                if type_char.is_none() {
                    type_char = Some(c);
                }
            }
        }
    }

    let type_char = match type_char {
        Some(c) => c,
        None => {
            eprintln!(
                "systemd-tmpfiles: {}:{}: no type character found in '{}', ignoring.",
                source.display(),
                line_number,
                type_str,
            );
            return None;
        }
    };

    let item_type = match ItemType::from_char(type_char, force) {
        Some(t) => t,
        None => {
            eprintln!(
                "systemd-tmpfiles: {}:{}: unknown type '{}', ignoring.",
                source.display(),
                line_number,
                type_char,
            );
            return None;
        }
    };

    // Field 2: Path (required) — expand specifiers like %U, %u, %h, etc.
    let path = PathBuf::from(unescape_c_style(&expand_specifiers(&fields[1], root)));

    // Default mode depends on type
    let default_mode = match item_type {
        ItemType::CreateDirectory
        | ItemType::CreateOrCleanDirectory
        | ItemType::AdjustDirectory
        | ItemType::CreateSubvolume
        | ItemType::CreateOrCleanSubvolume => 0o755,
        ItemType::CreateFifo => 0o644,
        ItemType::CreateFile => 0o644,
        _ => 0o644,
    };

    // Field 3: Mode (optional)
    // "-" or empty means "use default when creating, don't change when adjusting existing"
    // ':' prefix means "only apply when creating, not on existing objects"
    let mode_field = if fields.len() > 2 {
        fields[2].trim()
    } else {
        ""
    };
    let mode_create_only = mode_field.starts_with(':');
    let mode_field = mode_field.strip_prefix(':').unwrap_or(mode_field);
    let mode = if mode_field == "-" || mode_field.is_empty() {
        None
    } else {
        Some(parse_mode(mode_field, default_mode))
    };

    // Field 4: User (optional) — ':' prefix means "only apply on creation"
    let user_field = if fields.len() > 3 {
        fields[3].trim()
    } else {
        ""
    };
    let user_create_only = user_field.starts_with(':');
    let user_field = user_field.strip_prefix(':').unwrap_or(user_field);
    let user = parse_user(user_field);

    // Field 5: Group (optional) — ':' prefix means "only apply on creation"
    let group_field = if fields.len() > 4 {
        fields[4].trim()
    } else {
        ""
    };
    let group_create_only = group_field.starts_with(':');
    let group_field = group_field.strip_prefix(':').unwrap_or(group_field);
    let group = parse_user(group_field);

    // Field 6: Age (optional), may include age-by prefix like "a:1m"
    let (age_by, age) = if fields.len() > 5 {
        let age_field = fields[5].trim();
        let result = parse_age(age_field);
        // Report invalid age formats (not "-" or empty, but couldn't parse)
        if result.1.is_none() && age_field != "-" && !age_field.is_empty() {
            eprintln!(
                "systemd-tmpfiles: {}:{}: Invalid age '{}', ignoring.",
                source.display(),
                line_number,
                age_field,
            );
        }
        result
    } else {
        (String::new(), None)
    };

    // Field 7+: Argument (optional — rest of line after skipping 6 fields)
    // We extract the raw tail of the line to preserve original spacing and quotes.
    let argument = {
        let mut pos = 0;
        let bytes = trimmed.as_bytes();
        let len = bytes.len();
        let mut field_count = 0;
        while field_count < 6 && pos < len {
            // Skip whitespace
            while pos < len && (bytes[pos] as char).is_whitespace() {
                pos += 1;
            }
            if pos >= len {
                break;
            }
            field_count += 1;
            // Skip field (handle quotes)
            if bytes[pos] == b'"' || bytes[pos] == b'\'' {
                let q = bytes[pos];
                pos += 1;
                while pos < len && bytes[pos] != q {
                    if bytes[pos] == b'\\' {
                        pos += 1;
                    } // skip escaped char
                    pos += 1;
                }
                if pos < len {
                    pos += 1;
                } // skip closing quote
            } else {
                while pos < len && !(bytes[pos] as char).is_whitespace() {
                    pos += 1;
                }
            }
        }
        if field_count == 6 {
            // Skip whitespace between field 6 and argument
            while pos < len && (bytes[pos] as char).is_whitespace() {
                pos += 1;
            }
            let raw_arg = &trimmed[pos..];
            if raw_arg.is_empty() || raw_arg == "-" {
                None
            } else {
                Some(expand_specifiers(raw_arg, root))
            }
        } else {
            None
        }
    };

    Some(TmpfilesItem {
        item_type,
        force,
        boot_only,
        minus,
        purgeable,
        conditional,
        path,
        mode,
        mode_create_only,
        user,
        user_create_only,
        group,
        group_create_only,
        age,
        age_by,
        argument,
        source: source.to_path_buf(),
        line_number,
    })
}

/// Parse a tmpfiles.d config file.
fn parse_config_stdin(root: Option<&Path>) -> io::Result<Vec<TmpfilesItem>> {
    let stdin = io::stdin();
    let reader = stdin.lock();
    let mut items = Vec::new();
    let mut continuation = String::new();
    let source = PathBuf::from("<stdin>");

    for (line_idx, line) in reader.lines().enumerate() {
        let line = line?;
        let line_number = line_idx + 1;

        if line.ends_with('\\') {
            continuation.push_str(&line[..line.len() - 1]);
            continuation.push(' ');
            continue;
        }

        let full_line = if !continuation.is_empty() {
            continuation.push_str(&line);
            let result = continuation.clone();
            continuation.clear();
            result
        } else {
            line
        };

        if let Some(item) = parse_line(&full_line, &source, line_number, root) {
            items.push(item);
        }
    }

    Ok(items)
}

fn parse_config_file(path: &Path, root: Option<&Path>) -> io::Result<Vec<TmpfilesItem>> {
    let file = fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut items = Vec::new();

    let mut continuation = String::new();

    for (line_idx, line) in reader.lines().enumerate() {
        let line = line?;
        let line_number = line_idx + 1;

        // Handle line continuation with trailing backslash
        if line.ends_with('\\') {
            continuation.push_str(&line[..line.len() - 1]);
            continuation.push(' ');
            continue;
        }

        let full_line = if !continuation.is_empty() {
            continuation.push_str(&line);
            let result = continuation.clone();
            continuation.clear();
            result
        } else {
            line
        };

        if let Some(item) = parse_line(&full_line, path, line_number, root) {
            items.push(item);
        }
    }

    Ok(items)
}

/// Discover all .conf files across the config directories, respecting priority.
/// Files in earlier directories shadow files with the same name in later directories.
fn discover_config_files() -> Vec<PathBuf> {
    let mut seen_names: BTreeSet<String> = BTreeSet::new();
    let mut result = Vec::new();

    // Collect files from directories in priority order (etc > run > usr/lib > lib).
    // When the same filename exists in multiple directories, the highest-priority one wins.
    for dir in CONFIG_DIRS {
        let dir_path = Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }

        let entries: Vec<PathBuf> = match fs::read_dir(dir_path) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|ext| ext == "conf").unwrap_or(false))
                .collect(),
            Err(e) => {
                eprintln!("systemd-tmpfiles: Failed to read directory {}: {}", dir, e);
                continue;
            }
        };

        for path in entries {
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                let name = file_name.to_string();
                if seen_names.contains(&name) {
                    continue;
                }
                seen_names.insert(name);
                result.push(path);
            }
        }
    }

    // Sort globally by filename so that processing order is alphabetical
    // across all directories (e.g. /usr/lib/L-a.conf before /etc/L-z.conf).
    result.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

    result
}

/// Resolve a username to a UID.
fn resolve_uid(user: &str) -> Option<u32> {
    // Try parsing as a numeric UID first
    if let Ok(uid) = user.parse::<u32>() {
        return Some(uid);
    }

    // Look up via getpwnam
    let c_user = CString::new(user).ok()?;
    unsafe {
        let pw = libc::getpwnam(c_user.as_ptr());
        if pw.is_null() {
            None
        } else {
            Some((*pw).pw_uid)
        }
    }
}

/// Resolve a group name to a GID.
fn resolve_gid(group: &str) -> Option<u32> {
    // Try parsing as a numeric GID first
    if let Ok(gid) = group.parse::<u32>() {
        return Some(gid);
    }

    // Look up via getgrnam
    let c_group = CString::new(group).ok()?;
    unsafe {
        let gr = libc::getgrnam(c_group.as_ptr());
        if gr.is_null() {
            None
        } else {
            Some((*gr).gr_gid)
        }
    }
}

/// Apply ownership (chown) to a path.
fn apply_ownership(path: &Path, user: &Option<String>, group: &Option<String>) -> io::Result<()> {
    let uid = match user {
        Some(u) => match resolve_uid(u) {
            Some(id) => id as libc::uid_t,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Unknown user: {}", u),
                ));
            }
        },
        None => libc::uid_t::MAX, // -1 means "don't change"
    };

    let gid = match group {
        Some(g) => match resolve_gid(g) {
            Some(id) => id as libc::gid_t,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Unknown group: {}", g),
                ));
            }
        },
        None => libc::gid_t::MAX, // -1 means "don't change"
    };

    // If neither needs changing, skip
    if uid == libc::uid_t::MAX && gid == libc::gid_t::MAX {
        return Ok(());
    }

    // Skip if ownership already matches (avoids EROFS on read-only filesystems)
    if let Ok(meta) = fs::symlink_metadata(path) {
        let cur_uid = meta.uid();
        let cur_gid = meta.gid();
        let uid_ok = uid == libc::uid_t::MAX || cur_uid == uid;
        let gid_ok = gid == libc::gid_t::MAX || cur_gid == gid;
        if uid_ok && gid_ok {
            return Ok(());
        }
    }

    // Use lchown to avoid following symlinks
    let c_path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    let ret = unsafe { libc::lchown(c_path.as_ptr(), uid, gid) };
    if ret != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Apply file mode (chmod) to a path.
fn apply_mode(path: &Path, mode: u32) -> io::Result<()> {
    // Skip if the mode already matches (avoids EROFS on read-only filesystems)
    if let Ok(meta) = fs::metadata(path)
        && meta.permissions().mode() & 0o7777 == mode & 0o7777
    {
        return Ok(());
    }
    let permissions = fs::Permissions::from_mode(mode);
    fs::set_permissions(path, permissions)
}

/// Check if a path matches any of the given prefixes.
fn matches_prefix(path: &Path, prefixes: &[PathBuf]) -> bool {
    if prefixes.is_empty() {
        return true;
    }
    prefixes.iter().any(|prefix| path.starts_with(prefix))
}

/// Check if a path matches any of the exclude prefixes.
fn excluded_by_prefix(path: &Path, exclude_prefixes: &[PathBuf]) -> bool {
    exclude_prefixes
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

/// Unescape C-style escape sequences in a string (\n, \t, \\, etc.)
fn unescape_c_style(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('n') => {
                    chars.next();
                    result.push('\n');
                }
                Some('t') => {
                    chars.next();
                    result.push('\t');
                }
                Some('r') => {
                    chars.next();
                    result.push('\r');
                }
                Some('\\') => {
                    chars.next();
                    result.push('\\');
                }
                Some('x') => {
                    chars.next(); // consume 'x'
                    let mut hex = String::new();
                    for _ in 0..2 {
                        if let Some(&h) = chars.peek() {
                            if h.is_ascii_hexdigit() {
                                hex.push(h);
                                chars.next();
                            } else {
                                break;
                            }
                        }
                    }
                    if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                        result.push(byte as char);
                    } else {
                        result.push('\\');
                        result.push('x');
                        result.push_str(&hex);
                    }
                }
                Some('0'..='7') => {
                    // Octal escape \NNN (up to 3 digits)
                    let mut oct = String::new();
                    for _ in 0..3 {
                        if let Some(&o) = chars.peek() {
                            if ('0'..='7').contains(&o) {
                                oct.push(o);
                                chars.next();
                            } else {
                                break;
                            }
                        }
                    }
                    if let Ok(byte) = u8::from_str_radix(&oct, 8) {
                        result.push(byte as char);
                    } else {
                        result.push('\\');
                        result.push_str(&oct);
                    }
                }
                Some(_) => {
                    result.push('\\');
                    result.push(chars.next().unwrap());
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Check if a path traversal is unsafe: any intermediate component is a symlink
/// owned by a non-root user, or there's an ownership transition from a non-root
/// directory to a root-owned subdirectory (which could indicate a TOCTOU attack).
fn path_has_unsafe_transition(path: &Path, root: Option<&Path>) -> bool {
    let mut check = PathBuf::new();
    let mut prev_uid: Option<u32> = None;
    let root_component_count = root.map(|r| r.components().count()).unwrap_or(0);
    for (i, component) in path.components().enumerate() {
        check.push(component);
        // Skip components that are part of the --root prefix (those are trusted)
        if i < root_component_count {
            continue;
        }
        if let Ok(meta) = fs::symlink_metadata(&check) {
            let uid = meta.uid();
            // Symlink owned by non-root user is always unsafe
            if meta.file_type().is_symlink() && uid != 0 {
                return true;
            }
            // Ownership transition: non-root dir → root-owned subdir is unsafe
            if meta.file_type().is_dir() {
                if let Some(prev) = prev_uid
                    && prev != 0
                    && uid == 0
                {
                    return true;
                }
                prev_uid = Some(uid);
            }
        }
    }
    false
}

/// Execute a single tmpfiles.d item for --create.
fn execute_create(item: &TmpfilesItem, graceful: bool, root: Option<&Path>) -> bool {
    let path = &item.path;

    // Refuse to traverse paths with symlinks owned by non-root users
    if path_has_unsafe_transition(path, root) {
        if !graceful && !item.minus {
            eprintln!(
                "systemd-tmpfiles: Unsafe symlink component in path {}.",
                path.display(),
            );
        }
        return false;
    }

    match item.item_type {
        ItemType::CreateFile => {
            // Refuse to follow symlinks — check lstat before anything else
            if let Ok(meta) = fs::symlink_metadata(path)
                && (meta.file_type().is_symlink() || !meta.file_type().is_file())
            {
                if !graceful && !item.minus {
                    eprintln!(
                        "systemd-tmpfiles: Existing path {} is not a regular file.",
                        path.display(),
                    );
                }
                return false;
            }

            if path.exists() && !item.force {
                // File already exists, just adjust permissions.
                // Apply ownership first, then mode — chown drops suid/sgid bits,
                // so we must (re)apply mode afterwards to preserve them.
                let mode_to_apply = item.mode.or_else(|| {
                    // If no explicit mode, preserve existing mode (chown may drop suid/sgid)
                    if item.user.is_some() || item.group.is_some() {
                        fs::metadata(path)
                            .ok()
                            .map(|m| m.permissions().mode() & 0o7777)
                    } else {
                        None
                    }
                });
                if let Err(e) = apply_ownership(path, &item.user, &item.group)
                    && !graceful
                    && !item.minus
                {
                    eprintln!(
                        "systemd-tmpfiles: Failed to set ownership on {}: {}",
                        path.display(),
                        e
                    );
                    return false;
                }
                if let Some(mode) = mode_to_apply
                    && let Err(e) = apply_mode(path, mode)
                    && !graceful
                    && !item.minus
                {
                    eprintln!(
                        "systemd-tmpfiles: Failed to set mode on {}: {}",
                        path.display(),
                        e
                    );
                    return false;
                }
                return true;
            }

            // Create parent directories
            if let Some(parent) = path.parent()
                && let Err(e) = fs::create_dir_all(parent)
                && !graceful
                && !item.minus
            {
                eprintln!(
                    "systemd-tmpfiles: Failed to create parent directory {}: {}",
                    parent.display(),
                    e
                );
                return false;
            }

            // Create the file
            let raw_content = item.argument.as_deref().unwrap_or("");
            let content = unescape_c_style(raw_content);
            match fs::write(path, &content) {
                Ok(()) => {
                    // Apply ownership first, then mode — chown drops suid/sgid
                    let _ = apply_ownership(path, &item.user, &item.group);
                    if let Some(mode) = item.mode {
                        let _ = apply_mode(path, mode);
                    }
                    true
                }
                Err(e) => {
                    if !graceful && !item.minus {
                        eprintln!(
                            "systemd-tmpfiles: Failed to create file {}: {}",
                            path.display(),
                            e
                        );
                        return false;
                    }
                    true
                }
            }
        }

        ItemType::TruncateFile => {
            // F — Create or truncate file (always writes content)
            // Refuse to follow symlinks
            if let Ok(meta) = fs::symlink_metadata(path) {
                if meta.file_type().is_symlink() || !meta.file_type().is_file() {
                    if !graceful && !item.minus {
                        eprintln!(
                            "systemd-tmpfiles: Existing path {} is not a regular file.",
                            path.display(),
                        );
                    }
                    return false;
                }
                // If file exists, check if we can skip the truncation (mode and content match)
                if meta.file_type().is_file() {
                    let content = item.argument.as_deref().unwrap_or("");
                    let file_empty = meta.len() == 0;
                    let content_empty = content.is_empty() || content == "-";
                    // Only skip write if both file and desired content are empty
                    if file_empty && content_empty {
                        if let Some(mode) = item.mode
                            && let Err(e) = apply_mode(path, mode)
                            && !graceful
                            && !item.minus
                        {
                            eprintln!(
                                "systemd-tmpfiles: Failed to set mode on {}: {}",
                                path.display(),
                                e
                            );
                            return false;
                        }
                        if let Err(e) = apply_ownership(path, &item.user, &item.group)
                            && !graceful
                            && !item.minus
                        {
                            eprintln!(
                                "systemd-tmpfiles: Failed to set ownership on {}: {}",
                                path.display(),
                                e
                            );
                            return false;
                        }
                        return true;
                    }
                }
            }

            // Create parent directories
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }

            let raw_content = item.argument.as_deref().unwrap_or("");
            let content = unescape_c_style(raw_content);
            match fs::write(path, &content) {
                Ok(()) => {
                    // Apply ownership first, then mode — chown drops suid/sgid
                    let _ = apply_ownership(path, &item.user, &item.group);
                    if let Some(mode) = item.mode {
                        let _ = apply_mode(path, mode);
                    }
                    true
                }
                Err(e) => {
                    if !graceful && !item.minus {
                        eprintln!(
                            "systemd-tmpfiles: Failed to create/truncate file {}: {}",
                            path.display(),
                            e
                        );
                        return false;
                    }
                    true
                }
            }
        }

        ItemType::WriteFile => {
            // 'w' requires a content argument
            if item.argument.is_none() {
                if !graceful && !item.minus {
                    eprintln!(
                        "systemd-tmpfiles: {}:{}: write requires an argument (content).",
                        item.source.display(),
                        item.line_number,
                    );
                }
                return false;
            }
            // 'w' only writes to existing files — skip if target doesn't exist
            if !path.exists() {
                return true;
            }
            let raw_content = item.argument.as_deref().unwrap_or("");
            let content = unescape_c_style(raw_content);
            let content = content.as_str();
            if !item.force {
                // 'w' (without +) overwrites
                match fs::write(path, content) {
                    Ok(()) => true,
                    Err(e) => {
                        if !graceful && !item.minus {
                            eprintln!(
                                "systemd-tmpfiles: Failed to write to {}: {}",
                                path.display(),
                                e
                            );
                            return false;
                        }
                        true
                    }
                }
            } else {
                // 'w+' appends
                match std::fs::OpenOptions::new().append(true).open(path) {
                    Ok(mut f) => {
                        use std::io::Write;
                        if let Err(e) = f.write_all(content.as_bytes())
                            && !graceful
                            && !item.minus
                        {
                            eprintln!(
                                "systemd-tmpfiles: Failed to write to {}: {}",
                                path.display(),
                                e
                            );
                            return false;
                        }
                        true
                    }
                    Err(e) => {
                        if !graceful && !item.minus {
                            eprintln!(
                                "systemd-tmpfiles: Failed to open {} for writing: {}",
                                path.display(),
                                e
                            );
                            return false;
                        }
                        true
                    }
                }
            }
        }

        ItemType::CreateDirectory
        | ItemType::CreateOrCleanDirectory
        | ItemType::AdjustDirectory
        | ItemType::CreateSubvolume
        | ItemType::CreateOrCleanSubvolume => {
            if item.item_type == ItemType::AdjustDirectory {
                // 'e' type supports glob patterns — expand and adjust each match.
                let path_str = path.to_string_lossy();
                if path_str.contains('*') || path_str.contains('?') || path_str.contains('[') {
                    if let Ok(entries) = glob::glob(&path_str) {
                        for entry in entries.flatten() {
                            if !entry.is_dir() {
                                continue;
                            }
                            if let Some(mode) = item.mode
                                && let Err(e) = apply_mode(&entry, mode)
                                && !graceful
                                && !item.minus
                            {
                                eprintln!(
                                    "systemd-tmpfiles: Failed to set mode on {}: {}",
                                    entry.display(),
                                    e
                                );
                            }
                            if let Err(e) = apply_ownership(&entry, &item.user, &item.group)
                                && !graceful
                                && !item.minus
                            {
                                eprintln!(
                                    "systemd-tmpfiles: Failed to set ownership on {}: {}",
                                    entry.display(),
                                    e
                                );
                            }
                        }
                    }
                    return true;
                }
                // Non-glob: 'e' only adjusts existing directories, doesn't create
                if !path.is_dir() {
                    return true;
                }
            } else if !path.exists() {
                // For q/Q, we could try creating a btrfs subvolume, but fall back to mkdir
                if let Err(e) = fs::create_dir_all(path) {
                    if !graceful && !item.minus {
                        eprintln!(
                            "systemd-tmpfiles: Failed to create directory {}: {}",
                            path.display(),
                            e
                        );
                        return false;
                    }
                    return true;
                }
            } else {
                // Directory already existed — respect ':' create-only modifiers
                let effective_mode = if item.mode_create_only {
                    None
                } else {
                    item.mode
                };
                let effective_user = if item.user_create_only {
                    &None
                } else {
                    &item.user
                };
                let effective_group = if item.group_create_only {
                    &None
                } else {
                    &item.group
                };
                if let Some(mode) = effective_mode
                    && let Err(e) = apply_mode(path, mode)
                    && !graceful
                    && !item.minus
                {
                    eprintln!(
                        "systemd-tmpfiles: Failed to set mode on {}: {}",
                        path.display(),
                        e
                    );
                }
                if let Err(e) = apply_ownership(path, effective_user, effective_group)
                    && !graceful
                    && !item.minus
                {
                    eprintln!(
                        "systemd-tmpfiles: Failed to set ownership on {}: {}",
                        path.display(),
                        e
                    );
                }
                return true;
            }

            // Apply mode and ownership (newly created)
            if path.is_dir() {
                if let Some(mode) = item.mode
                    && let Err(e) = apply_mode(path, mode)
                    && !graceful
                    && !item.minus
                {
                    eprintln!(
                        "systemd-tmpfiles: Failed to set mode on {}: {}",
                        path.display(),
                        e
                    );
                }
                if let Err(e) = apply_ownership(path, &item.user, &item.group)
                    && !graceful
                    && !item.minus
                {
                    eprintln!(
                        "systemd-tmpfiles: Failed to set ownership on {}: {}",
                        path.display(),
                        e
                    );
                }
            }

            true
        }

        ItemType::CreateFifo => {
            if path.exists() && !item.force {
                return true;
            }

            // With force (p+), remove existing path first
            if path.exists() && item.force {
                let _ = fs::remove_file(path);
            }

            // Create parent directories
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }

            let c_path = match CString::new(path.as_os_str().as_encoded_bytes()) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("systemd-tmpfiles: Invalid path {}: {}", path.display(), e);
                    return false;
                }
            };

            let mode = item.mode.unwrap_or(0o644);
            // Temporarily clear umask so mkfifo creates with exact mode
            let old_umask = unsafe { libc::umask(0) };
            let ret = unsafe { libc::mkfifo(c_path.as_ptr(), mode) };
            unsafe { libc::umask(old_umask) };
            if ret != 0 {
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::AlreadyExists && !graceful && !item.minus {
                    eprintln!(
                        "systemd-tmpfiles: Failed to create FIFO {}: {}",
                        path.display(),
                        e
                    );
                    return false;
                }
            }
            let _ = apply_ownership(path, &item.user, &item.group);
            true
        }

        ItemType::CreateSymlink => {
            // '?' modifier: only create if the target exists (inside root if --root)
            if item.conditional
                && let Some(target) = &item.argument
            {
                let check_path = if let Some(r) = root {
                    r.join(target.strip_prefix('/').unwrap_or(target))
                } else {
                    PathBuf::from(target)
                };
                if !check_path.exists() {
                    return true;
                }
            }

            if path.exists() || path.symlink_metadata().is_ok() {
                if !item.force {
                    return true;
                }
                // With '+' force, remove existing and recreate
                if path.is_dir() {
                    let _ = fs::remove_dir_all(path);
                } else {
                    let _ = fs::remove_file(path);
                }
            }

            // Create parent directories
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }

            let target = match &item.argument {
                Some(t) => t.as_str(),
                None => {
                    eprintln!(
                        "systemd-tmpfiles: {}:{}: symlink requires an argument (target path).",
                        item.source.display(),
                        item.line_number,
                    );
                    return false;
                }
            };

            match symlink(target, path) {
                Ok(()) => true,
                Err(e) => {
                    if !graceful && !item.minus {
                        eprintln!(
                            "systemd-tmpfiles: Failed to create symlink {} -> {}: {}",
                            path.display(),
                            target,
                            e
                        );
                        return false;
                    }
                    true
                }
            }
        }

        ItemType::CreateCharDevice | ItemType::CreateBlockDevice => {
            // Creating device nodes requires parsing the argument as "major:minor"
            // and calling mknod(). This requires root privileges.
            if path.exists() && !item.force {
                return true;
            }

            let dev_str = match &item.argument {
                Some(a) => a.as_str(),
                None => {
                    eprintln!(
                        "systemd-tmpfiles: {}:{}: device node requires an argument (major:minor).",
                        item.source.display(),
                        item.line_number,
                    );
                    return false;
                }
            };

            let parts: Vec<&str> = dev_str.split(':').collect();
            if parts.len() != 2 {
                eprintln!(
                    "systemd-tmpfiles: {}:{}: invalid device specification '{}', expected major:minor.",
                    item.source.display(),
                    item.line_number,
                    dev_str,
                );
                return false;
            }

            let major: u32 = match parts[0].trim().parse() {
                Ok(v) => v,
                Err(_) => {
                    eprintln!(
                        "systemd-tmpfiles: {}:{}: invalid major number '{}'.",
                        item.source.display(),
                        item.line_number,
                        parts[0],
                    );
                    return false;
                }
            };

            let minor: u32 = match parts[1].trim().parse() {
                Ok(v) => v,
                Err(_) => {
                    eprintln!(
                        "systemd-tmpfiles: {}:{}: invalid minor number '{}'.",
                        item.source.display(),
                        item.line_number,
                        parts[1],
                    );
                    return false;
                }
            };

            // Create parent directories
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }

            let c_path = match CString::new(path.as_os_str().as_encoded_bytes()) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("systemd-tmpfiles: Invalid path {}: {}", path.display(), e);
                    return false;
                }
            };

            let dev = libc::makedev(major, minor);
            let mode = item.mode.unwrap_or(0o644);
            let node_type = if item.item_type == ItemType::CreateCharDevice {
                libc::S_IFCHR
            } else {
                libc::S_IFBLK
            };

            if item.force {
                let _ = fs::remove_file(path);
            }

            let ret = unsafe { libc::mknod(c_path.as_ptr(), node_type | mode, dev) };
            if ret != 0 {
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::AlreadyExists && !graceful && !item.minus {
                    eprintln!(
                        "systemd-tmpfiles: Failed to create device node {}: {}",
                        path.display(),
                        e
                    );
                    return false;
                }
            }
            let _ = apply_ownership(path, &item.user, &item.group);
            true
        }

        ItemType::CopyFiles => {
            let source = match &item.argument {
                Some(s) => PathBuf::from(s),
                None => {
                    // Default was set in run() before --root prefixing; should not reach here
                    eprintln!(
                        "systemd-tmpfiles: {}:{}: copy requires an argument (source path).",
                        item.source.display(),
                        item.line_number,
                    );
                    return false;
                }
            };

            if !source.exists() {
                if !graceful && !item.minus {
                    eprintln!(
                        "systemd-tmpfiles: Failed to copy {} -> {}: No such file or directory (os error 2)",
                        source.display(),
                        path.display(),
                    );
                }
                // Missing source is not a fatal error — just skip.
                return true;
            }

            if path.exists() && !item.force && !source.is_dir() {
                // Skip existing files unless forced.
                // Directories are always merged (contents are copied into existing dir).
                return true;
            }

            // Create parent directories
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }

            if source.is_dir() {
                let owner_info = Some((&item.user, &item.group, item.mode));
                match copy_dir_recursive_with_owner(&source, path, false, owner_info) {
                    Ok(()) => true,
                    Err(e) => {
                        if !graceful && !item.minus {
                            eprintln!(
                                "systemd-tmpfiles: Failed to copy {} -> {}: {}",
                                source.display(),
                                path.display(),
                                e,
                            );
                            return false;
                        }
                        true
                    }
                }
            } else {
                match fs::copy(&source, path) {
                    Ok(_) => {
                        if let Some(mode) = item.mode {
                            let _ = apply_mode(path, mode);
                        }
                        let _ = apply_ownership(path, &item.user, &item.group);
                        true
                    }
                    Err(e) => {
                        if !graceful && !item.minus {
                            eprintln!(
                                "systemd-tmpfiles: Failed to copy {} -> {}: {}",
                                source.display(),
                                path.display(),
                                e,
                            );
                            return false;
                        }
                        true
                    }
                }
            }
        }

        ItemType::AdjustPermissions | ItemType::AdjustPermissionsRecursively => {
            let recursive = item.item_type == ItemType::AdjustPermissionsRecursively;

            // Support glob patterns for z/Z types
            let path_str = path.to_string_lossy();
            if path_str.contains('*') || path_str.contains('?') || path_str.contains('[') {
                if let Ok(entries) = glob::glob(&path_str) {
                    let mut ok = true;
                    for entry in entries.flatten() {
                        if let Some(mode) = item.mode
                            && let Err(e) = apply_mode(&entry, mode)
                            && !graceful
                            && !item.minus
                        {
                            eprintln!(
                                "systemd-tmpfiles: Failed to set mode on {}: {}",
                                entry.display(),
                                e
                            );
                            ok = false;
                        }
                        if let Err(e) = apply_ownership(&entry, &item.user, &item.group)
                            && !graceful
                            && !item.minus
                        {
                            eprintln!(
                                "systemd-tmpfiles: Failed to set ownership on {}: {}",
                                entry.display(),
                                e
                            );
                            ok = false;
                        }
                    }
                    return ok;
                }
                return true;
            }

            if !path.exists() {
                return true;
            }

            let apply_to = |p: &Path| -> bool {
                let mut ok = true;
                if let Some(mode) = item.mode
                    && let Err(e) = apply_mode(p, mode)
                    && !graceful
                    && !item.minus
                {
                    eprintln!(
                        "systemd-tmpfiles: Failed to set mode on {}: {}",
                        p.display(),
                        e
                    );
                    ok = false;
                }
                if let Err(e) = apply_ownership(p, &item.user, &item.group)
                    && !graceful
                    && !item.minus
                {
                    eprintln!(
                        "systemd-tmpfiles: Failed to set ownership on {}: {}",
                        p.display(),
                        e
                    );
                    ok = false;
                }
                ok
            };

            if recursive {
                let mut ok = apply_to(path);
                if path.is_dir()
                    && let Err(e) = walk_dir(path, &mut |p| {
                        if !apply_to(p) {
                            ok = false;
                        }
                    })
                    && !graceful
                    && !item.minus
                {
                    eprintln!(
                        "systemd-tmpfiles: Failed to walk directory {}: {}",
                        path.display(),
                        e
                    );
                }
                ok
            } else {
                apply_to(path)
            }
        }

        ItemType::SetExtendedAttributes
        | ItemType::SetExtendedAttributesRecursively
        | ItemType::SetAttributes
        | ItemType::SetAttributesRecursively => {
            // These are advanced features (xattrs, chattr flags).
            // We accept them without error but don't implement the actual
            // kernel interface calls — this is sufficient for most boot scenarios.
            true
        }

        ItemType::SetACL | ItemType::SetACLRecursively => {
            let acl_spec = match &item.argument {
                Some(a) => a.as_str(),
                None => return true,
            };
            if !path.exists() {
                return true;
            }

            // 'a' (no +) clears extended ACLs first; 'a+' appends/modifies
            if !item.force {
                let _ = std::process::Command::new("setfacl")
                    .arg("-b")
                    .arg(path)
                    .output();
            }

            let mut cmd = std::process::Command::new("setfacl");
            if item.item_type == ItemType::SetACLRecursively {
                cmd.arg("-R");
            }
            cmd.arg("-m").arg(acl_spec).arg(path);

            match cmd.output() {
                Ok(output) if output.status.success() => true,
                Ok(output) => {
                    if !graceful && !item.minus {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        eprintln!(
                            "systemd-tmpfiles: Failed to set ACL on {}: {}",
                            path.display(),
                            stderr.trim()
                        );
                        return false;
                    }
                    true
                }
                Err(e) => {
                    if !graceful && !item.minus {
                        eprintln!(
                            "systemd-tmpfiles: Failed to run setfacl on {}: {}",
                            path.display(),
                            e
                        );
                        return false;
                    }
                    true
                }
            }
        }

        // Types that are only relevant for --remove or --clean
        ItemType::IgnorePath
        | ItemType::IgnoreDirectoryPath
        | ItemType::RemovePath
        | ItemType::RemoveRecursively => true,
    }
}

/// Execute a single tmpfiles.d item for --remove.
fn execute_remove_with_managed(
    item: &TmpfilesItem,
    graceful: bool,
    managed_paths: &std::collections::HashSet<PathBuf>,
    exclude_dir_patterns: &[PathBuf],
) -> bool {
    execute_remove_inner(item, graceful, managed_paths, exclude_dir_patterns)
}

fn remove_recursive_with_exclusions(
    dir: &Path,
    managed: &std::collections::HashSet<PathBuf>,
    exclude_dir_patterns: &[PathBuf],
    graceful: bool,
    minus: bool,
) -> bool {
    let mut ok = true;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            // Skip managed paths (but recurse into managed dirs)
            if managed.contains(&entry_path) {
                if entry_path.is_dir()
                    && !remove_recursive_with_exclusions(
                        &entry_path,
                        managed,
                        exclude_dir_patterns,
                        graceful,
                        minus,
                    )
                {
                    ok = false;
                }
                continue;
            }
            // Check X patterns — protect the path but recurse into it
            if is_excluded_by_patterns(&entry_path, exclude_dir_patterns) {
                if entry_path.is_dir()
                    && !remove_recursive_with_exclusions(
                        &entry_path,
                        managed,
                        exclude_dir_patterns,
                        graceful,
                        minus,
                    )
                {
                    ok = false;
                }
                continue;
            }
            let result = if entry_path.is_dir() {
                fs::remove_dir_all(&entry_path)
            } else {
                fs::remove_file(&entry_path)
            };
            if let Err(e) = result
                && !graceful
                && !minus
            {
                eprintln!(
                    "systemd-tmpfiles: Failed to remove {}: {}",
                    entry_path.display(),
                    e
                );
                ok = false;
            }
        }
    }
    ok
}

fn execute_remove_inner(
    item: &TmpfilesItem,
    graceful: bool,
    managed_paths: &std::collections::HashSet<PathBuf>,
    exclude_dir_patterns: &[PathBuf],
) -> bool {
    let path = &item.path;

    match item.item_type {
        ItemType::RemovePath => {
            if !path.exists() && path.symlink_metadata().is_err() {
                return true;
            }
            match fs::remove_file(path) {
                Ok(()) => true,
                Err(_) => {
                    // Try removing as a directory (empty) — silently skip non-empty directories
                    match fs::remove_dir(path) {
                        Ok(()) => true,
                        Err(_) => true, // Not an error — skip non-empty dirs
                    }
                }
            }
        }

        ItemType::RemoveRecursively => {
            if !path.exists() && path.symlink_metadata().is_err() {
                return true;
            }
            if path.is_dir() {
                // Remove contents of the directory, preserving managed and X-excluded paths
                remove_recursive_with_exclusions(
                    path,
                    managed_paths,
                    exclude_dir_patterns,
                    graceful,
                    item.minus,
                )
            } else {
                match fs::remove_file(path) {
                    Ok(()) => true,
                    Err(e) => {
                        if !graceful && !item.minus {
                            eprintln!(
                                "systemd-tmpfiles: Failed to remove {}: {}",
                                path.display(),
                                e
                            );
                            return false;
                        }
                        true
                    }
                }
            }
        }

        ItemType::CreateOrCleanDirectory | ItemType::CreateOrCleanSubvolume => {
            // On --remove, clean out the contents of the directory (but keep the dir itself)
            if !path.is_dir() {
                return true;
            }
            match fs::read_dir(path) {
                Ok(entries) => {
                    let mut ok = true;
                    for entry in entries.filter_map(|e| e.ok()) {
                        let entry_path = entry.path();
                        if entry_path.is_dir() {
                            if let Err(e) = fs::remove_dir_all(&entry_path)
                                && !graceful
                                && !item.minus
                            {
                                eprintln!(
                                    "systemd-tmpfiles: Failed to remove {}: {}",
                                    entry_path.display(),
                                    e
                                );
                                ok = false;
                            }
                        } else if let Err(e) = fs::remove_file(&entry_path)
                            && !graceful
                            && !item.minus
                        {
                            eprintln!(
                                "systemd-tmpfiles: Failed to remove {}: {}",
                                entry_path.display(),
                                e
                            );
                            ok = false;
                        }
                    }
                    ok
                }
                Err(e) => {
                    if !graceful && !item.minus {
                        eprintln!(
                            "systemd-tmpfiles: Failed to read directory {}: {}",
                            path.display(),
                            e
                        );
                        return false;
                    }
                    true
                }
            }
        }

        _ => true,
    }
}

/// Execute cleanup for items with an age specification.
fn execute_clean(
    item: &TmpfilesItem,
    exclude_patterns: &[PathBuf],
    exclude_dir_patterns: &[PathBuf],
    graceful: bool,
) -> bool {
    let path = &item.path;
    let age = match item.age {
        Some(a) => a,
        None => return true,
    };

    match item.item_type {
        ItemType::CreateDirectory
        | ItemType::CreateOrCleanDirectory
        | ItemType::AdjustDirectory
        | ItemType::CreateSubvolume
        | ItemType::CreateOrCleanSubvolume => {
            if !path.is_dir() {
                return true;
            }
            clean_directory(
                path,
                age,
                &item.age_by,
                exclude_patterns,
                exclude_dir_patterns,
                graceful,
                item.minus,
            )
        }
        _ => true,
    }
}

/// Check if a path matches any exclude pattern (supports glob patterns).
/// For 'x' patterns, the path matches if it starts with (is under) the pattern.
/// For direct use, this checks exact match or starts_with for non-glob patterns,
/// and glob matching for patterns containing wildcards.
fn is_excluded_by_patterns(path: &Path, patterns: &[PathBuf]) -> bool {
    for pattern in patterns {
        let pat_str = pattern.to_string_lossy();
        if pat_str.contains('*') || pat_str.contains('?') || pat_str.contains('[') {
            // Glob pattern — match against the path
            if let Ok(entries) = glob::glob(&pat_str) {
                for entry in entries.flatten() {
                    if path.starts_with(&entry) || *path == entry {
                        return true;
                    }
                }
            }
        } else if path.starts_with(pattern.as_path()) {
            return true;
        }
    }
    false
}

/// Check if a path exactly matches any pattern (for X — exclude only the directory itself).
fn is_exact_match_by_patterns(path: &Path, patterns: &[PathBuf]) -> bool {
    for pattern in patterns {
        let pat_str = pattern.to_string_lossy();
        if pat_str.contains('*') || pat_str.contains('?') || pat_str.contains('[') {
            if let Ok(entries) = glob::glob(&pat_str) {
                for entry in entries.flatten() {
                    if *path == entry {
                        return true;
                    }
                }
            }
        } else if *path == **pattern {
            return true;
        }
    }
    false
}

/// Clean files older than the specified age from a directory.
fn clean_directory(
    dir: &Path,
    max_age: Duration,
    age_by: &str,
    exclude_patterns: &[PathBuf],
    exclude_dir_patterns: &[PathBuf],
    graceful: bool,
    ignore_errors: bool,
) -> bool {
    let now = SystemTime::now();
    // Save atime before reading directory (read_dir updates atime on some filesystems)
    let saved_atime = fs::symlink_metadata(dir).ok().map(|m| m.atime());
    let entries = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            if !graceful && !ignore_errors {
                eprintln!(
                    "systemd-tmpfiles: Failed to read directory {}: {}",
                    dir.display(),
                    e
                );
            }
            return graceful || ignore_errors;
        }
    };

    let mut ok = true;

    for entry in entries.filter_map(|e| e.ok()) {
        let entry_path = entry.path();

        // Check 'x' patterns — excludes path and everything below
        if is_excluded_by_patterns(&entry_path, exclude_patterns) {
            continue;
        }

        // Check 'X' patterns — excludes the directory itself (exact match only)
        let is_x_excluded = is_exact_match_by_patterns(&entry_path, exclude_dir_patterns);

        let metadata = match entry_path.symlink_metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        if is_x_excluded {
            // X: directory is protected but contents should be cleaned
            if metadata.is_dir() {
                let _ = clean_directory(
                    &entry_path,
                    max_age,
                    age_by,
                    exclude_patterns,
                    exclude_dir_patterns,
                    graceful,
                    ignore_errors,
                );
            }
            continue;
        }

        // Compute file age based on age_by flags
        let file_age = {
            let is_dir = metadata.is_dir();
            let now_secs = now
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            if age_by.is_empty() {
                // Default: use most recent of all timestamps
                let mtime = metadata.mtime() as u64;
                let atime = metadata.atime() as u64;
                let ctime = metadata.ctime() as u64;
                let newest = mtime.max(atime).max(ctime);
                if now_secs > newest {
                    Duration::from_secs(now_secs - newest)
                } else {
                    Duration::from_secs(0)
                }
            } else {
                // age-by: use only specified timestamps. Lowercase = files, uppercase = dirs.
                // ALL specified timestamps must be older than max_age for the item to be cleaned.
                // We track the minimum age (newest timestamp) — only clean if even that is old.
                let mut min_age = Duration::from_secs(u64::MAX);
                let mut any = false;
                for flag in age_by.chars() {
                    let (applies_to_files, applies_to_dirs) = match flag {
                        'a' => (true, false),
                        'm' => (true, false),
                        'c' => (true, false),
                        'b' => (true, false),
                        'A' => (false, true),
                        'M' => (false, true),
                        'C' => (false, true),
                        'B' => (false, true),
                        _ => continue,
                    };
                    if (is_dir && !applies_to_dirs) || (!is_dir && !applies_to_files) {
                        continue;
                    }
                    let ts = match flag.to_ascii_lowercase() {
                        'a' => metadata.atime() as u64,
                        'm' => metadata.mtime() as u64,
                        'c' => metadata.ctime() as u64,
                        'b' => {
                            // btime (birth/creation time)
                            metadata
                                .created()
                                .ok()
                                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                                .map(|d| d.as_secs())
                                .unwrap_or(metadata.ctime() as u64)
                        }
                        _ => continue,
                    };
                    let age = if now_secs > ts {
                        Duration::from_secs(now_secs - ts)
                    } else {
                        Duration::from_secs(0)
                    };
                    if age < min_age {
                        min_age = age;
                    }
                    any = true;
                }
                if any { min_age } else { Duration::from_secs(0) }
            }
        };

        // age 0 means "clean everything", otherwise clean files older than or equal to max_age
        if max_age.is_zero() || file_age >= max_age {
            if metadata.is_dir() {
                // Recurse into subdirectories first
                let _ = clean_directory(
                    &entry_path,
                    max_age,
                    age_by,
                    exclude_patterns,
                    exclude_dir_patterns,
                    graceful,
                    ignore_errors,
                );

                // Try to remove the directory if it's now empty
                if let Err(e) = fs::remove_dir(&entry_path) {
                    // Directory not empty — that's fine, skip it
                    if e.kind() != io::ErrorKind::Other
                        && e.raw_os_error() != Some(libc::ENOTEMPTY)
                        && e.raw_os_error() != Some(libc::EEXIST)
                        && !graceful
                        && !ignore_errors
                    {
                        eprintln!(
                            "systemd-tmpfiles: Failed to remove old directory {}: {}",
                            entry_path.display(),
                            e
                        );
                        ok = false;
                    }
                }
            } else if let Err(e) = fs::remove_file(&entry_path)
                && !graceful
                && !ignore_errors
            {
                eprintln!(
                    "systemd-tmpfiles: Failed to remove old file {}: {}",
                    entry_path.display(),
                    e
                );
                ok = false;
            }
        } else if metadata.is_dir() && age_by.is_empty() {
            // Not old enough to remove, but recurse to clean contents.
            // Only recurse when not using age-by, to avoid updating atimes
            // (systemd uses O_NOATIME for this).
            let _ = clean_directory(
                &entry_path,
                max_age,
                age_by,
                exclude_patterns,
                exclude_dir_patterns,
                graceful,
                ignore_errors,
            );
        }
    }

    // Restore atime to avoid interfering with subsequent age-based cleaning
    // (like systemd's O_NOATIME approach)
    if let Some(orig_atime) = saved_atime {
        let mtime = fs::symlink_metadata(dir)
            .ok()
            .map(|m| m.mtime())
            .unwrap_or(0);
        let times = [
            libc::timespec {
                tv_sec: orig_atime,
                tv_nsec: 0,
            },
            libc::timespec {
                tv_sec: mtime,
                tv_nsec: 0,
            },
        ];
        if let Ok(c_path) = CString::new(dir.as_os_str().as_encoded_bytes()) {
            unsafe {
                libc::utimensat(
                    libc::AT_FDCWD,
                    c_path.as_ptr(),
                    times.as_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
        }
    }

    ok
}

/// Recursively copy a directory.
/// Copy directory recursively without ownership changes.
/// Copy directory recursively, optionally applying ownership/mode to newly copied entries.
/// When `force` is false, existing files are not overwritten.
/// `owner` provides (user, group, mode) to apply to newly created entries.
fn copy_dir_recursive_with_owner(
    src: &Path,
    dst: &Path,
    force: bool,
    owner: Option<(&Option<String>, &Option<String>, Option<u32>)>,
) -> io::Result<()> {
    let created_dir = !dst.exists();
    if created_dir {
        fs::create_dir_all(dst)?;
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let entry_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if entry_type.is_dir() {
            copy_dir_recursive_with_owner(&src_path, &dst_path, force, owner)?;
        } else if entry_type.is_symlink() {
            if !force && dst_path.exists() {
                continue;
            }
            let target = fs::read_link(&src_path)?;
            let _ = fs::remove_file(&dst_path);
            symlink(&target, &dst_path)?;
            if let Some((user, group, mode)) = owner {
                let _ = apply_ownership(&dst_path, user, group);
                if let Some(m) = mode {
                    let _ = apply_mode(&dst_path, m);
                }
            }
        } else {
            if !force && dst_path.exists() {
                continue;
            }
            fs::copy(&src_path, &dst_path)?;
            if let Some((user, group, mode)) = owner {
                let _ = apply_ownership(&dst_path, user, group);
                if let Some(m) = mode {
                    let _ = apply_mode(&dst_path, m);
                }
            }
        }
    }

    // Apply ownership/mode to the directory itself if we created it or force
    if (created_dir || force)
        && let Some((user, group, mode)) = owner
    {
        let _ = apply_ownership(dst, user, group);
        if let Some(m) = mode {
            let _ = apply_mode(dst, m);
        }
    }

    Ok(())
}

/// Walk a directory tree, calling the callback for each entry.
fn walk_dir(dir: &Path, callback: &mut dyn FnMut(&Path)) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        callback(&path);
        if path.is_dir() {
            walk_dir(&path, callback)?;
        }
    }
    Ok(())
}

fn run() -> u8 {
    let cli = Cli::parse();

    let verbose = std::env::var("SYSTEMD_LOG_LEVEL")
        .map(|v| v == "debug" || v == "info")
        .unwrap_or(false);

    // Need at least one action
    if !cli.create && !cli.clean && !cli.remove && !cli.purge {
        eprintln!("systemd-tmpfiles: No action specified. Use --create, --clean, or --remove.");
        return EXIT_FAILURE;
    }

    // Collect config files to read
    let has_stdin = cli.files.iter().any(|f| f.as_os_str() == "-");
    let config_files: Vec<PathBuf> = if !cli.files.is_empty() {
        cli.files
            .iter()
            .filter(|f| f.as_os_str() != "-")
            .cloned()
            .collect()
    } else {
        discover_config_files()
    };

    if verbose {
        eprintln!(
            "systemd-tmpfiles: Found {} configuration file(s){}.",
            config_files.len(),
            if has_stdin { " (+ stdin)" } else { "" }
        );
    }

    // Parse all configuration
    let mut items = Vec::new();

    // Read from stdin if "-" was specified
    if has_stdin {
        match parse_config_stdin(cli.root.as_deref()) {
            Ok(stdin_items) => {
                if verbose {
                    eprintln!(
                        "systemd-tmpfiles: Read {} item(s) from stdin",
                        stdin_items.len(),
                    );
                }
                items.extend(stdin_items);
            }
            Err(e) => {
                eprintln!("systemd-tmpfiles: Failed to read stdin: {}", e);
            }
        }
    }

    for path in &config_files {
        match parse_config_file(path, cli.root.as_deref()) {
            Ok(file_items) => {
                if verbose {
                    eprintln!(
                        "systemd-tmpfiles: Read {} item(s) from {}",
                        file_items.len(),
                        path.display()
                    );
                }
                items.extend(file_items);
            }
            Err(e) => {
                eprintln!("systemd-tmpfiles: Failed to read {}: {}", path.display(), e);
            }
        }
    }

    // Deduplicate items: for types other than w+ (WriteFile with force/append),
    // only the first entry per (path, type) is kept.  This matches systemd behaviour
    // where earlier (higher-priority) config files win for the same path and type.
    {
        let mut seen: std::collections::HashSet<(PathBuf, ItemType)> =
            std::collections::HashSet::new();
        items.retain(|item| {
            // w+ (WriteFile with force flag) entries are always kept — they append.
            if item.item_type == ItemType::WriteFile && item.force {
                return true;
            }
            // For all other types, keep only the first entry per (path, type).
            seen.insert((item.path.clone(), item.item_type))
        });
    }

    // Set default symlink/copy source before root prefixing.
    // When no argument is given, L and C types default to /usr/share/factory/<path>.
    for item in &mut items {
        if (item.item_type == ItemType::CopyFiles || item.item_type == ItemType::CreateSymlink)
            && item.argument.is_none()
        {
            let rel = item.path.strip_prefix("/").unwrap_or(&item.path);
            item.argument = Some(
                PathBuf::from("/usr/share/factory")
                    .join(rel)
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }

    // Apply --root prefix to all item paths and arguments
    if let Some(ref root) = cli.root {
        for item in &mut items {
            // Prefix the path: root + path
            item.path = root.join(item.path.strip_prefix("/").unwrap_or(&item.path));
            // Prefix the argument if it looks like an absolute path.
            // Symlink targets are NOT prefixed — they are stored as-is in the symlink
            // and will resolve correctly when the rootfs is used at boot.
            if item.item_type != ItemType::CreateSymlink
                && let Some(ref arg) = item.argument
                && arg.starts_with('/')
            {
                item.argument = Some(
                    root.join(arg.strip_prefix('/').unwrap_or(arg))
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }

    // Filter by boot-only flag
    if !cli.boot {
        items.retain(|item| !item.boot_only);
    }

    // Collect exclude patterns from 'x' and 'X' items for clean operations.
    // 'x' excludes the path and everything below; 'X' excludes only the directory itself.
    let exclude_patterns: Vec<PathBuf> = items
        .iter()
        .filter(|item| item.item_type == ItemType::IgnorePath)
        .map(|item| item.path.clone())
        .collect();
    let exclude_dir_patterns: Vec<PathBuf> = items
        .iter()
        .filter(|item| item.item_type == ItemType::IgnoreDirectoryPath)
        .map(|item| item.path.clone())
        .collect();

    let mut any_failed = false;

    // Execute --purge first (before --create, so --create can recreate items)
    if cli.purge {
        // Sort deepest-first so children are removed before parents
        let mut purge_items: Vec<&TmpfilesItem> =
            items.iter().filter(|item| item.purgeable).collect();
        purge_items.sort_by(|a, b| {
            b.path
                .components()
                .count()
                .cmp(&a.path.components().count())
        });
        for item in &purge_items {
            if !matches_prefix(&item.path, &cli.prefixes)
                || excluded_by_prefix(&item.path, &cli.exclude_prefixes)
            {
                continue;
            }
            if cli.dry_run {
                eprintln!(
                    "Would purge: {} (type {:?}, from {}:{})",
                    item.path.display(),
                    item.item_type,
                    item.source.display(),
                    item.line_number,
                );
            } else {
                let path = &item.path;
                if path.is_dir() {
                    let _ = fs::remove_dir_all(path);
                } else if path.exists() || path.symlink_metadata().is_ok() {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }

    // Execute --create
    if cli.create {
        for item in &items {
            if !item.item_type.is_create_type() {
                continue;
            }

            // Apply prefix filters
            if !matches_prefix(&item.path, &cli.prefixes) {
                continue;
            }
            if excluded_by_prefix(&item.path, &cli.exclude_prefixes) {
                continue;
            }

            if verbose {
                eprintln!(
                    "systemd-tmpfiles: Creating: {} (from {}:{})",
                    item.path.display(),
                    item.source.display(),
                    item.line_number,
                );
            }

            if cli.dry_run {
                eprintln!(
                    "Would create: {} (type {:?}, from {}:{})",
                    item.path.display(),
                    item.item_type,
                    item.source.display(),
                    item.line_number,
                );
            } else if !execute_create(item, cli.graceful, cli.root.as_deref()) {
                any_failed = true;
            }
        }
    }

    // Execute --remove
    if cli.remove {
        // Collect "managed" paths (paths with create-type entries) to protect from removal
        let managed_paths: std::collections::HashSet<PathBuf> = items
            .iter()
            .filter(|item| item.item_type.is_create_type())
            .map(|item| item.path.clone())
            .collect();

        // Sort remove items deepest-first so children are removed before parents
        let mut remove_items: Vec<&TmpfilesItem> = items
            .iter()
            .filter(|item| {
                item.item_type.is_remove_type()
                    && matches_prefix(&item.path, &cli.prefixes)
                    && !excluded_by_prefix(&item.path, &cli.exclude_prefixes)
            })
            .collect();
        remove_items.sort_by(|a, b| {
            b.path
                .components()
                .count()
                .cmp(&a.path.components().count())
        });

        for item in &remove_items {
            if verbose {
                eprintln!(
                    "systemd-tmpfiles: Removing: {} (from {}:{})",
                    item.path.display(),
                    item.source.display(),
                    item.line_number,
                );
            }

            if cli.dry_run {
                eprintln!(
                    "Would remove: {} (type {:?}, from {}:{})",
                    item.path.display(),
                    item.item_type,
                    item.source.display(),
                    item.line_number,
                );
            } else if !execute_remove_with_managed(
                item,
                cli.graceful,
                &managed_paths,
                &exclude_dir_patterns,
            ) {
                any_failed = true;
            }
        }
    }

    // Execute --clean
    if cli.clean {
        for item in &items {
            if !item.item_type.is_clean_type() {
                continue;
            }

            if !matches_prefix(&item.path, &cli.prefixes) {
                continue;
            }
            if excluded_by_prefix(&item.path, &cli.exclude_prefixes) {
                continue;
            }

            if verbose {
                eprintln!(
                    "systemd-tmpfiles: Cleaning: {} (from {}:{})",
                    item.path.display(),
                    item.source.display(),
                    item.line_number,
                );
            }

            if cli.dry_run {
                eprintln!(
                    "Would clean: {} (type {:?}, from {}:{})",
                    item.path.display(),
                    item.item_type,
                    item.source.display(),
                    item.line_number,
                );
            } else if !execute_clean(item, &exclude_patterns, &exclude_dir_patterns, cli.graceful) {
                any_failed = true;
            }
        }
    }

    if any_failed {
        EXIT_FAILURE
    } else {
        EXIT_SUCCESS
    }
}

fn main() -> ExitCode {
    ExitCode::from(run())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_parse_mode_octal() {
        assert_eq!(parse_mode("0755", 0o644), 0o755);
        assert_eq!(parse_mode("755", 0o644), 0o755);
        assert_eq!(parse_mode("0644", 0o755), 0o644);
        assert_eq!(parse_mode("0700", 0o644), 0o700);
    }

    #[test]
    fn test_parse_mode_default() {
        assert_eq!(parse_mode("-", 0o644), 0o644);
        assert_eq!(parse_mode("", 0o755), 0o755);
    }

    #[test]
    fn test_parse_mode_masked() {
        // '~' prefix (masked mode) — we treat it as plain mode
        assert_eq!(parse_mode("~0755", 0o644), 0o755);
    }

    #[test]
    fn test_parse_user() {
        assert_eq!(parse_user("root"), Some("root".to_string()));
        assert_eq!(parse_user("-"), None);
        assert_eq!(parse_user(""), None);
        assert_eq!(parse_user("  nobody  "), Some("nobody".to_string()));
    }

    #[test]
    fn test_parse_age_seconds() {
        assert_eq!(parse_age("30").1, Some(Duration::from_secs(30)));
        assert_eq!(parse_age("60s").1, Some(Duration::from_secs(60)));
    }

    #[test]
    fn test_parse_age_minutes() {
        assert_eq!(parse_age("5m").1, Some(Duration::from_secs(300)));
    }

    #[test]
    fn test_parse_age_hours() {
        assert_eq!(parse_age("2h").1, Some(Duration::from_secs(7200)));
    }

    #[test]
    fn test_parse_age_days() {
        assert_eq!(parse_age("10d").1, Some(Duration::from_secs(864000)));
    }

    #[test]
    fn test_parse_age_weeks() {
        assert_eq!(parse_age("1w").1, Some(Duration::from_secs(604800)));
    }

    #[test]
    fn test_parse_age_default() {
        assert_eq!(parse_age("-").1, None);
        assert_eq!(parse_age("").1, None);
    }

    #[test]
    fn test_parse_age_tilde() {
        // '~' prefix means "only clean if not accessed"
        assert_eq!(parse_age("~10d").1, Some(Duration::from_secs(864000)));
    }

    #[test]
    fn test_parse_age_compound() {
        // 1d12h = 86400 + 43200 = 129600
        assert_eq!(parse_age("1d12h").1, Some(Duration::from_secs(129600)));
    }

    #[test]
    fn test_split_fields_basic() {
        let fields = split_fields("d /tmp 1777 root root 10d -");
        assert_eq!(
            fields,
            vec!["d", "/tmp", "1777", "root", "root", "10d", "-"]
        );
    }

    #[test]
    fn test_split_fields_quoted() {
        let fields = split_fields(r#"f /etc/hostname - - - - "my hostname""#);
        assert_eq!(fields.len(), 7);
        assert_eq!(fields[6], "my hostname");
    }

    #[test]
    fn test_split_fields_single_quoted() {
        let fields = split_fields("f /etc/test - - - - 'hello world'");
        assert_eq!(fields.len(), 7);
        assert_eq!(fields[6], "hello world");
    }

    #[test]
    fn test_split_fields_extra_whitespace() {
        let fields = split_fields("  d   /tmp   0755   root   root   -   -  ");
        assert_eq!(fields, vec!["d", "/tmp", "0755", "root", "root", "-", "-"]);
    }

    #[test]
    fn test_parse_line_directory() {
        let item = parse_line(
            "d /tmp 1777 root root 10d -",
            Path::new("test.conf"),
            1,
            None,
        )
        .unwrap();
        assert_eq!(item.item_type, ItemType::CreateDirectory);
        assert_eq!(item.path, PathBuf::from("/tmp"));
        assert_eq!(item.mode, Some(0o1777));
        assert_eq!(item.user, Some("root".to_string()));
        assert_eq!(item.group, Some("root".to_string()));
        assert_eq!(item.age, Some(Duration::from_secs(864000)));
        assert!(item.argument.is_none());
        assert!(!item.force);
        assert!(!item.boot_only);
    }

    #[test]
    fn test_parse_line_file() {
        let item = parse_line(
            "f /etc/hostname 0644 root root - myhostname",
            Path::new("test.conf"),
            2,
            None,
        )
        .unwrap();
        assert_eq!(item.item_type, ItemType::CreateFile);
        assert_eq!(item.path, PathBuf::from("/etc/hostname"));
        assert_eq!(item.mode, Some(0o644));
        assert_eq!(item.argument, Some("myhostname".to_string()));
    }

    #[test]
    fn test_parse_line_symlink() {
        let item = parse_line(
            "L /etc/localtime - - - - /usr/share/zoneinfo/UTC",
            Path::new("test.conf"),
            3,
            None,
        )
        .unwrap();
        assert_eq!(item.item_type, ItemType::CreateSymlink);
        assert_eq!(item.path, PathBuf::from("/etc/localtime"));
        assert_eq!(item.argument, Some("/usr/share/zoneinfo/UTC".to_string()));
    }

    #[test]
    fn test_parse_line_force() {
        let item = parse_line(
            "L+ /etc/localtime - - - - /usr/share/zoneinfo/UTC",
            Path::new("test.conf"),
            4,
            None,
        )
        .unwrap();
        assert_eq!(item.item_type, ItemType::CreateSymlink);
        assert!(item.force);
    }

    #[test]
    fn test_parse_line_boot_only() {
        let item = parse_line(
            "d! /run/test 0755 root root - -",
            Path::new("test.conf"),
            5,
            None,
        )
        .unwrap();
        assert!(item.boot_only);
    }

    #[test]
    fn test_parse_line_minus() {
        let item = parse_line(
            "d- /run/test 0755 root root - -",
            Path::new("test.conf"),
            6,
            None,
        )
        .unwrap();
        assert!(item.minus);
    }

    #[test]
    fn test_parse_line_comment() {
        assert!(parse_line("# This is a comment", Path::new("test.conf"), 1, None).is_none());
        assert!(parse_line("; Another comment", Path::new("test.conf"), 2, None).is_none());
        assert!(parse_line("", Path::new("test.conf"), 3, None).is_none());
        assert!(parse_line("   ", Path::new("test.conf"), 4, None).is_none());
    }

    #[test]
    fn test_parse_line_remove() {
        let item = parse_line("r /tmp/old-files", Path::new("test.conf"), 7, None).unwrap();
        assert_eq!(item.item_type, ItemType::RemovePath);
        assert_eq!(item.path, PathBuf::from("/tmp/old-files"));
    }

    #[test]
    fn test_parse_line_remove_recursive() {
        let item = parse_line("R /tmp/old-dir", Path::new("test.conf"), 8, None).unwrap();
        assert_eq!(item.item_type, ItemType::RemoveRecursively);
    }

    #[test]
    fn test_parse_line_clean_directory() {
        let item = parse_line(
            "D /tmp/cache 0755 root root 1w -",
            Path::new("test.conf"),
            9,
            None,
        )
        .unwrap();
        assert_eq!(item.item_type, ItemType::CreateOrCleanDirectory);
        assert_eq!(item.age, Some(Duration::from_secs(604800)));
    }

    #[test]
    fn test_parse_line_adjust_directory() {
        let item = parse_line(
            "e /var/log 0755 root root - -",
            Path::new("test.conf"),
            10,
            None,
        )
        .unwrap();
        assert_eq!(item.item_type, ItemType::AdjustDirectory);
    }

    #[test]
    fn test_parse_line_ignore_path() {
        let item = parse_line("x /tmp/important-*", Path::new("test.conf"), 11, None).unwrap();
        assert_eq!(item.item_type, ItemType::IgnorePath);
    }

    #[test]
    fn test_parse_line_adjust_permissions() {
        let item = parse_line(
            "z /etc/shadow 0640 root shadow - -",
            Path::new("test.conf"),
            12,
            None,
        )
        .unwrap();
        assert_eq!(item.item_type, ItemType::AdjustPermissions);
        assert_eq!(item.mode, Some(0o640));
        assert_eq!(item.user, Some("root".to_string()));
        assert_eq!(item.group, Some("shadow".to_string()));
    }

    #[test]
    fn test_parse_line_fifo() {
        let item = parse_line(
            "p /run/my-fifo 0600 root root - -",
            Path::new("test.conf"),
            13,
            None,
        )
        .unwrap();
        assert_eq!(item.item_type, ItemType::CreateFifo);
        assert_eq!(item.mode, Some(0o600));
    }

    #[test]
    fn test_parse_line_unknown_type() {
        assert!(parse_line("? /tmp/test", Path::new("test.conf"), 14, None).is_none());
    }

    #[test]
    fn test_parse_line_minimal_fields() {
        // Just type and path (minimum required)
        let item = parse_line("d /tmp", Path::new("test.conf"), 15, None).unwrap();
        assert_eq!(item.item_type, ItemType::CreateDirectory);
        assert_eq!(item.path, PathBuf::from("/tmp"));
        assert_eq!(item.mode, None); // no mode specified
    }

    #[test]
    fn test_parse_config_file_basic() {
        let dir = std::env::temp_dir().join("systemd-tmpfiles-test-basic");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "# Example tmpfiles.d configuration").unwrap();
        writeln!(f, "d /tmp 1777 root root 10d -").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "f /etc/hostname 0644 root root - myhostname").unwrap();
        writeln!(f, "L /etc/localtime - - - - /usr/share/zoneinfo/UTC").unwrap();
        drop(f);

        let items = parse_config_file(&path, None).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].item_type, ItemType::CreateDirectory);
        assert_eq!(items[1].item_type, ItemType::CreateFile);
        assert_eq!(items[2].item_type, ItemType::CreateSymlink);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_is_create_type() {
        assert!(ItemType::CreateFile.is_create_type());
        assert!(ItemType::CreateDirectory.is_create_type());
        assert!(ItemType::CreateSymlink.is_create_type());
        assert!(ItemType::AdjustPermissions.is_create_type());
        assert!(!ItemType::RemovePath.is_create_type());
        assert!(!ItemType::IgnorePath.is_create_type());
    }

    #[test]
    fn test_is_remove_type() {
        assert!(ItemType::RemovePath.is_remove_type());
        assert!(ItemType::RemoveRecursively.is_remove_type());
        assert!(ItemType::CreateOrCleanDirectory.is_remove_type());
        assert!(!ItemType::CreateFile.is_remove_type());
        assert!(!ItemType::CreateDirectory.is_remove_type());
    }

    #[test]
    fn test_is_clean_type() {
        assert!(ItemType::CreateDirectory.is_clean_type());
        assert!(ItemType::CreateOrCleanDirectory.is_clean_type());
        assert!(ItemType::AdjustDirectory.is_clean_type());
        assert!(ItemType::IgnorePath.is_clean_type());
        assert!(!ItemType::CreateFile.is_clean_type());
        assert!(!ItemType::RemovePath.is_clean_type());
    }

    #[test]
    fn test_matches_prefix_empty() {
        assert!(matches_prefix(Path::new("/tmp/test"), &[]));
    }

    #[test]
    fn test_matches_prefix_match() {
        let prefixes = vec![PathBuf::from("/tmp"), PathBuf::from("/run")];
        assert!(matches_prefix(Path::new("/tmp/test"), &prefixes));
        assert!(matches_prefix(Path::new("/run/something"), &prefixes));
        assert!(!matches_prefix(Path::new("/etc/test"), &prefixes));
    }

    #[test]
    fn test_excluded_by_prefix() {
        let excludes = vec![PathBuf::from("/dev"), PathBuf::from("/sys")];
        assert!(excluded_by_prefix(Path::new("/dev/null"), &excludes));
        assert!(excluded_by_prefix(Path::new("/sys/class"), &excludes));
        assert!(!excluded_by_prefix(Path::new("/tmp/test"), &excludes));
    }

    #[test]
    fn test_discover_config_files_no_crash() {
        let _files = discover_config_files();
    }

    #[test]
    fn test_execute_create_directory() {
        let dir = std::env::temp_dir().join("systemd-tmpfiles-test-create-dir");
        let _ = fs::remove_dir_all(&dir);

        let item = TmpfilesItem {
            item_type: ItemType::CreateDirectory,
            force: false,
            boot_only: false,
            minus: false,
            purgeable: false,
            conditional: false,
            path: dir.clone(),
            mode: Some(0o755),
            user: None,
            group: None,
            age: None,
            mode_create_only: false,
            user_create_only: false,
            group_create_only: false,
            age_by: String::new(),
            argument: None,
            source: PathBuf::from("test.conf"),
            line_number: 1,
        };

        assert!(execute_create(&item, true, None));
        assert!(dir.is_dir());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_execute_create_file() {
        let dir = std::env::temp_dir().join("systemd-tmpfiles-test-create-file");
        let _ = fs::create_dir_all(&dir);
        let file = dir.join("test.txt");

        let item = TmpfilesItem {
            item_type: ItemType::CreateFile,
            force: false,
            boot_only: false,
            minus: false,
            purgeable: false,
            conditional: false,
            path: file.clone(),
            mode: Some(0o644),
            user: None,
            group: None,
            age: None,
            mode_create_only: false,
            user_create_only: false,
            group_create_only: false,
            age_by: String::new(),
            argument: Some("hello world".to_string()),
            source: PathBuf::from("test.conf"),
            line_number: 1,
        };

        assert!(execute_create(&item, true, None));
        assert!(file.is_file());
        assert_eq!(fs::read_to_string(&file).unwrap(), "hello world");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_execute_create_symlink() {
        let dir = std::env::temp_dir().join("systemd-tmpfiles-test-create-symlink");
        let _ = fs::create_dir_all(&dir);
        let link = dir.join("mylink");
        let _ = fs::remove_file(&link);

        let item = TmpfilesItem {
            item_type: ItemType::CreateSymlink,
            force: false,
            boot_only: false,
            minus: false,
            purgeable: false,
            conditional: false,
            path: link.clone(),
            mode: None,
            user: None,
            group: None,
            age: None,
            mode_create_only: false,
            user_create_only: false,
            group_create_only: false,
            age_by: String::new(),
            argument: Some("/tmp".to_string()),
            source: PathBuf::from("test.conf"),
            line_number: 1,
        };

        assert!(execute_create(&item, true, None));
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read_link(&link).unwrap(), PathBuf::from("/tmp"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_execute_remove_file() {
        let dir = std::env::temp_dir().join("systemd-tmpfiles-test-remove-file");
        let _ = fs::create_dir_all(&dir);
        let file = dir.join("to-remove.txt");
        fs::write(&file, "temporary").unwrap();
        assert!(file.exists());

        let item = TmpfilesItem {
            item_type: ItemType::RemovePath,
            force: false,
            boot_only: false,
            minus: false,
            purgeable: false,
            conditional: false,
            path: file.clone(),
            mode: None,
            user: None,
            group: None,
            age: None,
            mode_create_only: false,
            user_create_only: false,
            group_create_only: false,
            age_by: String::new(),
            argument: None,
            source: PathBuf::from("test.conf"),
            line_number: 1,
        };

        assert!(execute_remove_inner(
            &item,
            false,
            &std::collections::HashSet::new(),
            &[]
        ));
        assert!(!file.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_execute_remove_recursive() {
        let dir = std::env::temp_dir().join("systemd-tmpfiles-test-remove-recursive");
        let sub = dir.join("sub");
        let _ = fs::create_dir_all(&sub);
        fs::write(sub.join("file.txt"), "content").unwrap();

        let item = TmpfilesItem {
            item_type: ItemType::RemoveRecursively,
            force: false,
            boot_only: false,
            minus: false,
            purgeable: false,
            conditional: false,
            path: dir.clone(),
            mode: None,
            user: None,
            group: None,
            age: None,
            mode_create_only: false,
            user_create_only: false,
            group_create_only: false,
            age_by: String::new(),
            argument: None,
            source: PathBuf::from("test.conf"),
            line_number: 1,
        };

        assert!(execute_remove_inner(
            &item,
            false,
            &std::collections::HashSet::new(),
            &[]
        ));
        // R removes contents, not the directory itself
        assert!(dir.exists());
        assert!(!sub.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_execute_remove_nonexistent() {
        let item = TmpfilesItem {
            item_type: ItemType::RemovePath,
            force: false,
            boot_only: false,
            minus: false,
            purgeable: false,
            conditional: false,
            path: PathBuf::from("/tmp/systemd-tmpfiles-test-nonexistent-xyz"),
            mode: None,
            user: None,
            group: None,
            age: None,
            mode_create_only: false,
            user_create_only: false,
            group_create_only: false,
            age_by: String::new(),
            argument: None,
            source: PathBuf::from("test.conf"),
            line_number: 1,
        };

        // Removing a nonexistent file should succeed silently
        assert!(execute_remove_inner(
            &item,
            false,
            &std::collections::HashSet::new(),
            &[]
        ));
    }

    #[test]
    fn test_item_type_from_char() {
        assert_eq!(ItemType::from_char('f', false), Some(ItemType::CreateFile));
        assert_eq!(
            ItemType::from_char('d', false),
            Some(ItemType::CreateDirectory)
        );
        assert_eq!(
            ItemType::from_char('D', false),
            Some(ItemType::CreateOrCleanDirectory)
        );
        assert_eq!(
            ItemType::from_char('L', false),
            Some(ItemType::CreateSymlink)
        );
        assert_eq!(ItemType::from_char('r', false), Some(ItemType::RemovePath));
        assert_eq!(
            ItemType::from_char('R', false),
            Some(ItemType::RemoveRecursively)
        );
        assert_eq!(
            ItemType::from_char('z', false),
            Some(ItemType::AdjustPermissions)
        );
        assert_eq!(
            ItemType::from_char('Z', false),
            Some(ItemType::AdjustPermissionsRecursively)
        );
        assert_eq!(ItemType::from_char('?', false), None);
    }

    #[test]
    fn test_parse_line_write_file() {
        let item = parse_line(
            "w /proc/sys/net/ipv4/ip_forward - - - - 1",
            Path::new("test.conf"),
            1,
            None,
        )
        .unwrap();
        assert_eq!(item.item_type, ItemType::WriteFile);
        assert_eq!(item.argument, Some("1".to_string()));
    }

    #[test]
    fn test_parse_line_copy_files() {
        let item = parse_line(
            "C /etc/skel - - - - /usr/share/skel",
            Path::new("test.conf"),
            1,
            None,
        )
        .unwrap();
        assert_eq!(item.item_type, ItemType::CopyFiles);
        assert_eq!(item.argument, Some("/usr/share/skel".to_string()));
    }

    #[test]
    fn test_parse_line_char_device() {
        let item = parse_line(
            "c /dev/null 0666 root root - 1:3",
            Path::new("test.conf"),
            1,
            None,
        )
        .unwrap();
        assert_eq!(item.item_type, ItemType::CreateCharDevice);
        assert_eq!(item.argument, Some("1:3".to_string()));
    }

    #[test]
    fn test_parse_line_block_device() {
        let item = parse_line(
            "b /dev/sda 0660 root disk - 8:0",
            Path::new("test.conf"),
            1,
            None,
        )
        .unwrap();
        assert_eq!(item.item_type, ItemType::CreateBlockDevice);
        assert_eq!(item.argument, Some("8:0".to_string()));
    }

    #[test]
    fn test_resolve_uid_numeric() {
        assert_eq!(resolve_uid("0"), Some(0));
        assert_eq!(resolve_uid("1000"), Some(1000));
    }

    #[test]
    fn test_resolve_uid_root() {
        // root should always exist
        assert_eq!(resolve_uid("root"), Some(0));
    }

    #[test]
    fn test_resolve_gid_numeric() {
        assert_eq!(resolve_gid("0"), Some(0));
        assert_eq!(resolve_gid("1000"), Some(1000));
    }

    #[test]
    fn test_resolve_gid_root() {
        assert_eq!(resolve_gid("root"), Some(0));
    }

    #[test]
    fn test_copy_dir_recursive_basic() {
        let dir = std::env::temp_dir().join("systemd-tmpfiles-test-copy-recursive");
        let src = dir.join("src");
        let dst = dir.join("dst");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(src.join("subdir")).unwrap();
        fs::write(src.join("file1.txt"), "hello").unwrap();
        fs::write(src.join("subdir/file2.txt"), "world").unwrap();

        copy_dir_recursive_with_owner(&src, &dst, true, None).unwrap();

        assert!(dst.join("file1.txt").is_file());
        assert!(dst.join("subdir/file2.txt").is_file());
        assert_eq!(fs::read_to_string(dst.join("file1.txt")).unwrap(), "hello");
        assert_eq!(
            fs::read_to_string(dst.join("subdir/file2.txt")).unwrap(),
            "world"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_line_continuation() {
        let dir = std::env::temp_dir().join("systemd-tmpfiles-test-continuation");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "d /tmp/test \\").unwrap();
        writeln!(f, "  1777 root root - -").unwrap();
        drop(f);

        let items = parse_config_file(&path, None).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].item_type, ItemType::CreateDirectory);
        assert_eq!(items[0].path, PathBuf::from("/tmp/test"));
        assert_eq!(items[0].mode, Some(0o1777));

        let _ = fs::remove_dir_all(&dir);
    }
}
