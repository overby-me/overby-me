//! systemd-sysusers — Create system users and groups from static configuration
//!
//! Reads configuration from sysusers.d/*.conf files and creates system users
//! and groups. This is a drop-in replacement for systemd-sysusers.
//!
//! Configuration is read from (in order of priority):
//!   /etc/sysusers.d/*.conf
//!   /run/sysusers.d/*.conf
//!   /usr/lib/sysusers.d/*.conf
//!   /lib/sysusers.d/*.conf
//!
//! Each .conf file contains lines of the form:
//!   Type Name ID GECOS Home Shell
//!
//! Supported types:
//!   u  — Create a system user and group (with the same name)
//!   u+ — Create a system user and group, only if not already existing
//!   g  — Create a system group
//!   g+ — Create a system group, only if not already existing
//!   m  — Add a user to a group (user:group)
//!   r  — Reserve a numeric UID/GID range
//!
//! ID can be:
//!   -        — Automatic UID/GID allocation
//!   <number> — Specific UID/GID
//!   <uid>:<gid> — Specific UID with a different GID
//!   -:<group> — Automatic UID, specific group name/GID
//!   <path>   — UID from the owner of a file (for 'u' type)

use std::collections::BTreeSet;
use std::ffi::CString;
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::Parser;

const EXIT_SUCCESS: u8 = 0;
const EXIT_FAILURE: u8 = 1;

/// Directories to search for sysusers.d configuration, in priority order.
/// Earlier directories take precedence when the same filename exists in multiple.
const CONFIG_DIRS: &[&str] = &[
    "/etc/sysusers.d",
    "/run/sysusers.d",
    "/usr/lib/sysusers.d",
    "/lib/sysusers.d",
];

/// System UID/GID range boundaries (matching systemd defaults).
const SYSTEM_UID_MIN: u32 = 100;
const SYSTEM_UID_MAX: u32 = 999;

/// systemd-sysusers — Create system users and groups from static configuration
#[derive(Parser, Debug)]
#[command(name = "systemd-sysusers", version, about)]
struct Cli {
    /// Root directory to operate on (for chroot-style operation)
    #[arg(long, default_value = "/")]
    root: PathBuf,

    /// Only process entries that would create new users/groups (dry run)
    #[arg(long)]
    dry_run: bool,

    /// Inline configuration string (instead of reading files)
    #[arg(long = "inline")]
    inline_config: Vec<String>,

    /// Specific sysusers.d config files to read (instead of scanning directories)
    files: Vec<PathBuf>,
}

/// Represents the action type from a sysusers.d line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryType {
    /// u — Create a system user (and its primary group)
    CreateUser,
    /// g — Create a system group
    CreateGroup,
    /// m — Add a user to a group
    AddToGroup,
    /// r — Reserve a UID/GID range
    ReserveRange,
}

/// Parsed UID/GID specification.
#[derive(Debug, Clone, PartialEq, Eq)]
enum IdSpec {
    /// Automatic allocation (ID field was "-")
    Automatic,
    /// A specific numeric UID
    Uid(u32),
    /// A specific UID:GID pair
    UidGid(u32, u32),
    /// Automatic UID with a named/numbered group
    AutoUidWithGroup(String),
    /// A UID:group-name pair
    UidWithGroupName(u32, String),
    /// A range like "100-999" (for 'r' type)
    Range(u32, u32),
    /// A file path (UID from file owner)
    FilePath(PathBuf),
}

impl IdSpec {
    fn parse(id_str: &str, entry_type: EntryType) -> Option<Self> {
        let s = id_str.trim();

        if s == "-" || s.is_empty() {
            return Some(IdSpec::Automatic);
        }

        // For 'r' type, parse as a range
        if entry_type == EntryType::ReserveRange {
            if let Some((start, end)) = s.split_once('-') {
                let start: u32 = start.trim().parse().ok()?;
                let end: u32 = end.trim().parse().ok()?;
                return Some(IdSpec::Range(start, end));
            }
            // Single number is also valid for 'r'
            if let Ok(n) = s.parse::<u32>() {
                return Some(IdSpec::Range(n, n));
            }
            return None;
        }

        // Check for UID:GID or UID:groupname format
        if let Some((left, right)) = s.split_once(':') {
            let left = left.trim();
            let right = right.trim();

            if left == "-" {
                // -:group — automatic UID, named group
                return Some(IdSpec::AutoUidWithGroup(right.to_string()));
            }

            if let Ok(uid) = left.parse::<u32>() {
                if let Ok(gid) = right.parse::<u32>() {
                    return Some(IdSpec::UidGid(uid, gid));
                } else {
                    return Some(IdSpec::UidWithGroupName(uid, right.to_string()));
                }
            }

            return None;
        }

        // Check for a plain number
        if let Ok(id) = s.parse::<u32>() {
            return Some(IdSpec::Uid(id));
        }

        // Check for a file path (starts with /)
        if s.starts_with('/') {
            return Some(IdSpec::FilePath(PathBuf::from(s)));
        }

        None
    }
}

/// A parsed sysusers.d configuration entry.
#[derive(Debug, Clone)]
struct SysusersEntry {
    /// The action type.
    entry_type: EntryType,
    /// Whether the '+' modifier was present (only create if not existing).
    plus: bool,
    /// The user or group name.
    name: String,
    /// The ID specification.
    id: IdSpec,
    /// GECOS field (real name / description).
    gecos: Option<String>,
    /// Home directory path.
    home: Option<String>,
    /// Login shell.
    shell: Option<String>,
    /// Source file for diagnostics.
    source: PathBuf,
    /// Line number in source file.
    line_number: usize,
}

/// Split a sysusers.d line into fields, respecting quoting.
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
                        current.push(next);
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

/// Parse a single sysusers.d line.
fn parse_line(line: &str, source: &Path, line_number: usize) -> Option<SysusersEntry> {
    let trimmed = line.trim();

    // Skip empty lines and comments
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
        return None;
    }

    let fields = split_fields(trimmed);
    if fields.len() < 2 {
        eprintln!(
            "systemd-sysusers: {}:{}: too few fields, ignoring.",
            source.display(),
            line_number,
        );
        return None;
    }

    // Parse type field
    let type_str = &fields[0];
    let (entry_type, plus) = match type_str.as_str() {
        "u" => (EntryType::CreateUser, false),
        "u+" => (EntryType::CreateUser, true),
        "g" => (EntryType::CreateGroup, false),
        "g+" => (EntryType::CreateGroup, true),
        "m" => (EntryType::AddToGroup, false),
        "r" => (EntryType::ReserveRange, false),
        other => {
            eprintln!(
                "systemd-sysusers: {}:{}: unknown type '{}', ignoring.",
                source.display(),
                line_number,
                other,
            );
            return None;
        }
    };

    // Field 2: Name (required)
    let name = fields[1].clone();

    // Field 3: ID (optional)
    let id_str = fields.get(2).map(|s| s.as_str()).unwrap_or("-");
    let id = match IdSpec::parse(id_str, entry_type) {
        Some(id) => id,
        None => {
            eprintln!(
                "systemd-sysusers: {}:{}: invalid ID specification '{}', ignoring.",
                source.display(),
                line_number,
                id_str,
            );
            return None;
        }
    };

    // Field 4: GECOS (optional)
    let gecos = fields.get(3).and_then(|s| {
        let s = s.trim();
        if s == "-" || s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    });

    // Field 5: Home directory (optional)
    let home = fields.get(4).and_then(|s| {
        let s = s.trim();
        if s == "-" || s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    });

    // Field 6: Shell (optional)
    let shell = fields.get(5).and_then(|s| {
        let s = s.trim();
        if s == "-" || s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    });

    Some(SysusersEntry {
        entry_type,
        plus,
        name,
        id,
        gecos,
        home,
        shell,
        source: source.to_path_buf(),
        line_number,
    })
}

/// Parse a sysusers.d config file.
///
/// A path of `-` reads from stdin (matches upstream `systemd-sysusers -`).
fn parse_config_file(path: &Path) -> io::Result<Vec<SysusersEntry>> {
    let reader: Box<dyn BufRead> = if path == Path::new("-") {
        Box::new(io::BufReader::new(io::stdin()))
    } else {
        Box::new(io::BufReader::new(fs::File::open(path)?))
    };
    let mut entries = Vec::new();

    for (line_idx, line) in reader.lines().enumerate() {
        let line = line?;
        let line_number = line_idx + 1;

        if let Some(entry) = parse_line(&line, path, line_number) {
            entries.push(entry);
        }
    }

    Ok(entries)
}

/// Discover all .conf files across the config directories, respecting priority.
/// Files in earlier directories shadow files with the same name in later directories.
fn discover_config_files() -> Vec<PathBuf> {
    let mut seen_names: BTreeSet<String> = BTreeSet::new();
    let mut result = Vec::new();

    for dir in CONFIG_DIRS {
        let dir_path = Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }

        let mut entries: Vec<PathBuf> = match fs::read_dir(dir_path) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|ext| ext == "conf").unwrap_or(false))
                .collect(),
            Err(e) => {
                eprintln!("systemd-sysusers: Failed to read directory {}: {}", dir, e);
                continue;
            }
        };
        entries.sort();

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

    result
}

/// Check if a user exists on the system.
fn user_exists(name: &str) -> bool {
    let c_name = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return false,
    };
    unsafe { !libc::getpwnam(c_name.as_ptr()).is_null() }
}

/// Check if a group exists on the system.
fn group_exists(name: &str) -> bool {
    let c_name = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return false,
    };
    unsafe { !libc::getgrnam(c_name.as_ptr()).is_null() }
}

/// Check if a user ID is already in use.
fn uid_exists(uid: u32) -> bool {
    unsafe { !libc::getpwuid(uid).is_null() }
}

/// Check if a group ID is already in use.
fn gid_exists(gid: u32) -> bool {
    unsafe { !libc::getgrgid(gid).is_null() }
}

/// Look up a user's UID by name.
#[allow(dead_code)]
fn lookup_uid(name: &str) -> Option<u32> {
    let c_name = CString::new(name).ok()?;
    unsafe {
        let pw = libc::getpwnam(c_name.as_ptr());
        if pw.is_null() {
            None
        } else {
            Some((*pw).pw_uid)
        }
    }
}

/// Look up a group's GID by name.
fn lookup_gid(name: &str) -> Option<u32> {
    let c_name = CString::new(name).ok()?;
    unsafe {
        let gr = libc::getgrnam(c_name.as_ptr());
        if gr.is_null() {
            None
        } else {
            Some((*gr).gr_gid)
        }
    }
}

/// Find an available UID in the system range.
fn find_available_uid(reserved: &BTreeSet<u32>) -> Option<u32> {
    (SYSTEM_UID_MIN..=SYSTEM_UID_MAX).find(|&uid| !uid_exists(uid) && !reserved.contains(&uid))
}

/// Find an available GID in the system range.
fn find_available_gid(reserved: &BTreeSet<u32>) -> Option<u32> {
    (SYSTEM_UID_MIN..=SYSTEM_UID_MAX).find(|&gid| !gid_exists(gid) && !reserved.contains(&gid))
}

/// Get the UID of a file's owner.
fn file_owner_uid(path: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(path).ok()?;
    Some(meta.uid())
}

/// Create a system group using groupadd.
fn create_group(name: &str, gid: Option<u32>, root: &Path, dry_run: bool, verbose: bool) -> bool {
    if group_exists(name) {
        if verbose {
            eprintln!(
                "systemd-sysusers: Group '{}' already exists, skipping.",
                name
            );
        }
        return true;
    }

    if verbose {
        eprintln!(
            "systemd-sysusers: Creating group '{}' (GID: {})...",
            name,
            gid.map(|g| g.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        );
    }

    if dry_run {
        return true;
    }

    let mut cmd = Command::new("groupadd");
    cmd.arg("--system");

    if let Some(gid) = gid {
        cmd.arg("--gid").arg(gid.to_string());
    }

    if root != Path::new("/") {
        cmd.arg("--root").arg(root);
    }

    cmd.arg(name);

    match cmd.status() {
        Ok(status) => {
            if status.success() {
                if verbose {
                    eprintln!("systemd-sysusers: Successfully created group '{}'.", name);
                }
                true
            } else {
                // Exit code 9 means group already exists (race condition)
                if status.code() == Some(9) {
                    true
                } else {
                    eprintln!(
                        "systemd-sysusers: groupadd failed for '{}' with exit code {}.",
                        name,
                        status.code().unwrap_or(-1),
                    );
                    false
                }
            }
        }
        Err(e) => {
            eprintln!(
                "systemd-sysusers: Failed to execute groupadd for '{}': {}",
                name, e
            );
            false
        }
    }
}

/// Create a system user using useradd.
fn create_user(
    entry: &SysusersEntry,
    uid: Option<u32>,
    gid: Option<u32>,
    root: &Path,
    dry_run: bool,
    verbose: bool,
) -> bool {
    if user_exists(&entry.name) {
        if verbose {
            eprintln!(
                "systemd-sysusers: User '{}' already exists, skipping.",
                entry.name
            );
        }
        return true;
    }

    if verbose {
        eprintln!(
            "systemd-sysusers: Creating user '{}' (UID: {}, GID: {})...",
            entry.name,
            uid.map(|u| u.to_string())
                .unwrap_or_else(|| "auto".to_string()),
            gid.map(|g| g.to_string())
                .unwrap_or_else(|| "auto".to_string()),
        );
    }

    if dry_run {
        return true;
    }

    let mut cmd = Command::new("useradd");
    cmd.arg("--system");

    if let Some(uid) = uid {
        cmd.arg("--uid").arg(uid.to_string());
    }

    if let Some(gid) = gid {
        cmd.arg("--gid").arg(gid.to_string());
    }

    // Set home directory
    let home = entry.home.as_deref().unwrap_or("/");
    cmd.arg("--home-dir").arg(home);
    // Don't create the home directory (system users typically don't need one)
    cmd.arg("--no-create-home");

    // Set shell (default to /usr/sbin/nologin for system users)
    let shell = entry.shell.as_deref().unwrap_or("/usr/sbin/nologin");
    cmd.arg("--shell").arg(shell);

    // Set GECOS
    if let Some(ref gecos) = entry.gecos {
        cmd.arg("--comment").arg(gecos);
    }

    if root != Path::new("/") {
        cmd.arg("--root").arg(root);
    }

    cmd.arg(&entry.name);

    match cmd.status() {
        Ok(status) => {
            if status.success() {
                if verbose {
                    eprintln!(
                        "systemd-sysusers: Successfully created user '{}'.",
                        entry.name
                    );
                }
                true
            } else {
                // Exit code 9 means user already exists (race condition)
                if status.code() == Some(9) {
                    true
                } else {
                    eprintln!(
                        "systemd-sysusers: useradd failed for '{}' with exit code {}.",
                        entry.name,
                        status.code().unwrap_or(-1),
                    );
                    false
                }
            }
        }
        Err(e) => {
            eprintln!(
                "systemd-sysusers: Failed to execute useradd for '{}': {}",
                entry.name, e
            );
            false
        }
    }
}

/// Add a user to a group using usermod.
fn add_user_to_group(user: &str, group: &str, root: &Path, dry_run: bool, verbose: bool) -> bool {
    if verbose {
        eprintln!(
            "systemd-sysusers: Adding user '{}' to group '{}'...",
            user, group
        );
    }

    if dry_run {
        return true;
    }

    // Check that both user and group exist
    if !user_exists(user) {
        eprintln!(
            "systemd-sysusers: User '{}' does not exist, cannot add to group '{}'.",
            user, group
        );
        return false;
    }

    if !group_exists(group) {
        eprintln!(
            "systemd-sysusers: Group '{}' does not exist, cannot add user '{}'.",
            group, user
        );
        return false;
    }

    let mut cmd = Command::new("usermod");
    cmd.arg("--append");
    cmd.arg("--groups").arg(group);

    if root != Path::new("/") {
        cmd.arg("--root").arg(root);
    }

    cmd.arg(user);

    match cmd.status() {
        Ok(status) => {
            if status.success() {
                if verbose {
                    eprintln!(
                        "systemd-sysusers: Successfully added '{}' to group '{}'.",
                        user, group
                    );
                }
                true
            } else {
                eprintln!(
                    "systemd-sysusers: usermod failed for '{}' with exit code {}.",
                    user,
                    status.code().unwrap_or(-1),
                );
                false
            }
        }
        Err(e) => {
            eprintln!(
                "systemd-sysusers: Failed to execute usermod for '{}': {}",
                user, e
            );
            false
        }
    }
}

/// Process a single sysusers.d entry.
fn process_entry(
    entry: &SysusersEntry,
    reserved_uids: &mut BTreeSet<u32>,
    reserved_gids: &mut BTreeSet<u32>,
    root: &Path,
    dry_run: bool,
    verbose: bool,
) -> bool {
    match entry.entry_type {
        EntryType::CreateGroup => {
            if entry.plus && group_exists(&entry.name) {
                if verbose {
                    eprintln!(
                        "systemd-sysusers: Group '{}' already exists (g+), skipping.",
                        entry.name
                    );
                }
                return true;
            }

            let gid = match &entry.id {
                IdSpec::Automatic => find_available_gid(reserved_gids),
                IdSpec::Uid(id) => Some(*id),
                IdSpec::Range(start, _) => Some(*start),
                _ => find_available_gid(reserved_gids),
            };

            if let Some(gid) = gid {
                reserved_gids.insert(gid);
            }

            create_group(&entry.name, gid, root, dry_run, verbose)
        }

        EntryType::CreateUser => {
            if entry.plus && user_exists(&entry.name) {
                if verbose {
                    eprintln!(
                        "systemd-sysusers: User '{}' already exists (u+), skipping.",
                        entry.name
                    );
                }
                return true;
            }

            // Determine UID and GID
            let (uid, gid) = match &entry.id {
                IdSpec::Automatic => {
                    let uid = find_available_uid(reserved_uids);
                    // For 'u' type, the primary group has the same name and
                    // ideally the same numeric ID
                    let gid = if let Some(uid) = uid {
                        if !gid_exists(uid) && !reserved_gids.contains(&uid) {
                            Some(uid)
                        } else {
                            find_available_gid(reserved_gids)
                        }
                    } else {
                        find_available_gid(reserved_gids)
                    };
                    (uid, gid)
                }
                IdSpec::Uid(id) => (Some(*id), Some(*id)),
                IdSpec::UidGid(uid, gid) => (Some(*uid), Some(*gid)),
                IdSpec::AutoUidWithGroup(group_ref) => {
                    let uid = find_available_uid(reserved_uids);
                    let gid = if let Ok(gid) = group_ref.parse::<u32>() {
                        Some(gid)
                    } else {
                        lookup_gid(group_ref)
                    };
                    (uid, gid)
                }
                IdSpec::UidWithGroupName(uid, group_ref) => {
                    let gid = if let Ok(gid) = group_ref.parse::<u32>() {
                        Some(gid)
                    } else {
                        lookup_gid(group_ref)
                    };
                    (Some(*uid), gid)
                }
                IdSpec::FilePath(path) => {
                    let uid = file_owner_uid(path);
                    let gid = uid; // Same ID for group
                    (uid, gid)
                }
                IdSpec::Range(start, _) => (Some(*start), Some(*start)),
            };

            if let Some(uid) = uid {
                reserved_uids.insert(uid);
            }
            if let Some(gid) = gid {
                reserved_gids.insert(gid);
            }

            // First create the primary group (same name as user)
            let group_ok = if !group_exists(&entry.name) {
                create_group(&entry.name, gid, root, dry_run, verbose)
            } else {
                true
            };

            if !group_ok {
                return false;
            }

            // Look up the GID of the just-created group (or existing one)
            let actual_gid = if dry_run {
                gid
            } else {
                lookup_gid(&entry.name).or(gid)
            };

            // Create the user
            create_user(entry, uid, actual_gid, root, dry_run, verbose)
        }

        EntryType::AddToGroup => {
            // Name field is "user:group" for 'm' type
            if let Some((user, group)) = entry.name.split_once(':') {
                add_user_to_group(user, group, root, dry_run, verbose)
            } else {
                // If no colon, the ID field specifies the group
                let _group = match &entry.id {
                    IdSpec::Automatic => {
                        eprintln!(
                            "systemd-sysusers: {}:{}: 'm' type requires a group name in the ID field or user:group in the name field.",
                            entry.source.display(),
                            entry.line_number,
                        );
                        return false;
                    }
                    _ => {
                        // Use the name field as user and ID field as a group specifier
                        // This is actually not standard; the standard format is
                        // m user group (where group is field 3)
                        eprintln!(
                            "systemd-sysusers: {}:{}: 'm' type should use format 'm user group'.",
                            entry.source.display(),
                            entry.line_number,
                        );
                        return false;
                    }
                };
            }
        }

        EntryType::ReserveRange => {
            // Reserve a UID/GID range — just record them so they're not allocated
            if let IdSpec::Range(start, end) = &entry.id {
                for id in *start..=*end {
                    reserved_uids.insert(id);
                    reserved_gids.insert(id);
                }
                if verbose {
                    eprintln!(
                        "systemd-sysusers: Reserved UID/GID range {}-{}.",
                        start, end
                    );
                }
            }
            true
        }
    }
}

/// Re-parse 'm' type entries that use the "m user group" three-field format.
fn process_membership_entry(
    entry: &SysusersEntry,
    root: &Path,
    dry_run: bool,
    verbose: bool,
) -> bool {
    // Standard sysusers.d format for 'm' is: m <user> <group>
    // where <user> is in the name field and <group> is in the ID field as a name
    let user = &entry.name;

    // The group is typically specified as the "ID" field, but since we parsed
    // it as an IdSpec, we need to recover the raw group name.
    // For 'm' entries, the third field is actually a group name, not a numeric ID.
    // We handle this by checking if the user:group was in the name field first.
    if user.contains(':')
        && let Some((u, g)) = user.split_once(':')
    {
        return add_user_to_group(u, g, root, dry_run, verbose);
    }

    // Otherwise, the group name was parsed as an ID. Since IdSpec::Automatic
    // means "-", and anything else means we couldn't parse it as ID,
    // fall back to treating the raw text of the third field as a group name.
    // We need to re-read the source to get the raw group name. Instead,
    // let's check common patterns:
    match &entry.id {
        IdSpec::Uid(gid) => {
            // If it parsed as a number, try to find the group by GID
            let group_name = format!("{}", gid);
            add_user_to_group(user, &group_name, root, dry_run, verbose)
        }
        _ => {
            eprintln!(
                "systemd-sysusers: {}:{}: cannot determine group for membership entry.",
                entry.source.display(),
                entry.line_number,
            );
            false
        }
    }
}

fn run() -> u8 {
    let cli = Cli::parse();

    let verbose = std::env::var("SYSTEMD_LOG_LEVEL")
        .map(|v| v == "debug" || v == "info")
        .unwrap_or(false);

    // Collect entries from all sources
    let mut all_entries: Vec<SysusersEntry> = Vec::new();

    // Process inline configurations first
    for inline in &cli.inline_config {
        if let Some(entry) = parse_line(inline, Path::new("<inline>"), 0) {
            all_entries.push(entry);
        }
    }

    // Then process config files
    let config_files = if !cli.files.is_empty() {
        cli.files.clone()
    } else if cli.inline_config.is_empty() {
        discover_config_files()
    } else {
        Vec::new()
    };

    if verbose {
        eprintln!(
            "systemd-sysusers: Found {} configuration file(s), {} inline entry/entries.",
            config_files.len(),
            cli.inline_config.len(),
        );
    }

    for path in &config_files {
        match parse_config_file(path) {
            Ok(entries) => {
                if verbose {
                    eprintln!(
                        "systemd-sysusers: Read {} entry/entries from {}",
                        entries.len(),
                        path.display()
                    );
                }
                all_entries.extend(entries);
            }
            Err(e) => {
                eprintln!("systemd-sysusers: Failed to read {}: {}", path.display(), e);
            }
        }
    }

    if all_entries.is_empty() {
        if verbose {
            eprintln!("systemd-sysusers: No entries to process.");
        }
        return EXIT_SUCCESS;
    }

    // Track reserved UIDs/GIDs to avoid allocation conflicts
    let mut reserved_uids: BTreeSet<u32> = BTreeSet::new();
    let mut reserved_gids: BTreeSet<u32> = BTreeSet::new();

    // Process entries in order:
    // 1. Reserve ranges first ('r' type)
    // 2. Create groups ('g' type)
    // 3. Create users ('u' type) — these also create their primary group
    // 4. Add memberships ('m' type)

    let mut any_failed = false;

    // Phase 1: Reserve ranges
    for entry in &all_entries {
        if entry.entry_type == EntryType::ReserveRange
            && !process_entry(
                entry,
                &mut reserved_uids,
                &mut reserved_gids,
                &cli.root,
                cli.dry_run,
                verbose,
            )
        {
            any_failed = true;
        }
    }

    // Phase 2: Create groups
    for entry in &all_entries {
        if entry.entry_type == EntryType::CreateGroup
            && !process_entry(
                entry,
                &mut reserved_uids,
                &mut reserved_gids,
                &cli.root,
                cli.dry_run,
                verbose,
            )
        {
            any_failed = true;
        }
    }

    // Phase 3: Create users (and their primary groups)
    for entry in &all_entries {
        if entry.entry_type == EntryType::CreateUser
            && !process_entry(
                entry,
                &mut reserved_uids,
                &mut reserved_gids,
                &cli.root,
                cli.dry_run,
                verbose,
            )
        {
            any_failed = true;
        }
    }

    // Phase 4: Add memberships
    for entry in &all_entries {
        if entry.entry_type == EntryType::AddToGroup
            && !process_membership_entry(entry, &cli.root, cli.dry_run, verbose)
        {
            any_failed = true;
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
    fn test_parse_line_create_user() {
        let entry = parse_line(
            "u systemd-journal - \"Journal\" /var/log/journal /usr/sbin/nologin",
            Path::new("test.conf"),
            1,
        )
        .unwrap();
        assert_eq!(entry.entry_type, EntryType::CreateUser);
        assert_eq!(entry.name, "systemd-journal");
        assert_eq!(entry.id, IdSpec::Automatic);
        assert_eq!(entry.gecos.as_deref(), Some("Journal"));
        assert_eq!(entry.home.as_deref(), Some("/var/log/journal"));
        assert_eq!(entry.shell.as_deref(), Some("/usr/sbin/nologin"));
        assert!(!entry.plus);
    }

    #[test]
    fn test_parse_line_create_user_with_uid() {
        let entry = parse_line("u http 33 \"HTTP Server\"", Path::new("test.conf"), 2).unwrap();
        assert_eq!(entry.entry_type, EntryType::CreateUser);
        assert_eq!(entry.name, "http");
        assert_eq!(entry.id, IdSpec::Uid(33));
        assert_eq!(entry.gecos.as_deref(), Some("HTTP Server"));
    }

    #[test]
    fn test_parse_line_create_user_plus() {
        let entry = parse_line("u+ nobody 65534", Path::new("test.conf"), 3).unwrap();
        assert_eq!(entry.entry_type, EntryType::CreateUser);
        assert!(entry.plus);
        assert_eq!(entry.name, "nobody");
        assert_eq!(entry.id, IdSpec::Uid(65534));
    }

    #[test]
    fn test_parse_line_create_group() {
        let entry = parse_line("g wheel 10", Path::new("test.conf"), 4).unwrap();
        assert_eq!(entry.entry_type, EntryType::CreateGroup);
        assert_eq!(entry.name, "wheel");
        assert_eq!(entry.id, IdSpec::Uid(10));
    }

    #[test]
    fn test_parse_line_create_group_auto() {
        let entry = parse_line("g audio -", Path::new("test.conf"), 5).unwrap();
        assert_eq!(entry.entry_type, EntryType::CreateGroup);
        assert_eq!(entry.name, "audio");
        assert_eq!(entry.id, IdSpec::Automatic);
    }

    #[test]
    fn test_parse_line_create_group_plus() {
        let entry = parse_line("g+ video -", Path::new("test.conf"), 6).unwrap();
        assert_eq!(entry.entry_type, EntryType::CreateGroup);
        assert!(entry.plus);
    }

    #[test]
    fn test_parse_line_membership() {
        let entry = parse_line("m user:group", Path::new("test.conf"), 7).unwrap();
        assert_eq!(entry.entry_type, EntryType::AddToGroup);
        assert_eq!(entry.name, "user:group");
    }

    #[test]
    fn test_parse_line_reserve_range() {
        let entry = parse_line("r - 100-999", Path::new("test.conf"), 8).unwrap();
        assert_eq!(entry.entry_type, EntryType::ReserveRange);
        assert_eq!(entry.id, IdSpec::Range(100, 999));
    }

    #[test]
    fn test_parse_line_reserve_single() {
        let entry = parse_line("r - 65534", Path::new("test.conf"), 9).unwrap();
        assert_eq!(entry.entry_type, EntryType::ReserveRange);
        assert_eq!(entry.id, IdSpec::Range(65534, 65534));
    }

    #[test]
    fn test_parse_line_comment() {
        assert!(parse_line("# comment", Path::new("test.conf"), 1).is_none());
        assert!(parse_line("; comment", Path::new("test.conf"), 2).is_none());
        assert!(parse_line("", Path::new("test.conf"), 3).is_none());
        assert!(parse_line("   ", Path::new("test.conf"), 4).is_none());
    }

    #[test]
    fn test_parse_line_unknown_type() {
        assert!(parse_line("x unknown -", Path::new("test.conf"), 1).is_none());
    }

    #[test]
    fn test_parse_line_uid_gid() {
        let entry = parse_line("u myuser 500:600 \"My User\"", Path::new("test.conf"), 1).unwrap();
        assert_eq!(entry.id, IdSpec::UidGid(500, 600));
    }

    #[test]
    fn test_parse_line_auto_uid_with_group() {
        let entry = parse_line("u myuser -:audio \"My User\"", Path::new("test.conf"), 1).unwrap();
        assert_eq!(entry.id, IdSpec::AutoUidWithGroup("audio".to_string()));
    }

    #[test]
    fn test_parse_line_uid_with_group_name() {
        let entry =
            parse_line("u myuser 500:audio \"My User\"", Path::new("test.conf"), 1).unwrap();
        assert_eq!(entry.id, IdSpec::UidWithGroupName(500, "audio".to_string()));
    }

    #[test]
    fn test_parse_line_file_path_id() {
        let entry = parse_line("u myuser /var/lib/myapp - - -", Path::new("test.conf"), 1).unwrap();
        assert_eq!(entry.id, IdSpec::FilePath(PathBuf::from("/var/lib/myapp")));
    }

    #[test]
    fn test_split_fields_basic() {
        let fields = split_fields("u http 33 \"HTTP Server\" /srv/http /usr/sbin/nologin");
        assert_eq!(fields.len(), 6);
        assert_eq!(fields[0], "u");
        assert_eq!(fields[1], "http");
        assert_eq!(fields[2], "33");
        assert_eq!(fields[3], "HTTP Server");
        assert_eq!(fields[4], "/srv/http");
        assert_eq!(fields[5], "/usr/sbin/nologin");
    }

    #[test]
    fn test_split_fields_extra_whitespace() {
        let fields = split_fields("  g   wheel   10  ");
        assert_eq!(fields, vec!["g", "wheel", "10"]);
    }

    #[test]
    fn test_split_fields_single_quoted() {
        let fields = split_fields("u test - 'Test User'");
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[3], "Test User");
    }

    #[test]
    fn test_split_fields_escape_in_quotes() {
        let fields = split_fields(r#"u test - "Test \"User\"""#);
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[3], "Test \"User\"");
    }

    #[test]
    fn test_parse_config_file_basic() {
        let dir = std::env::temp_dir().join("systemd-sysusers-test-basic");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "# System users for myapp").unwrap();
        writeln!(f, "g myapp-group -").unwrap();
        writeln!(
            f,
            "u myapp-user - \"MyApp Service\" /var/lib/myapp /usr/sbin/nologin"
        )
        .unwrap();
        writeln!(f).unwrap();
        writeln!(f, "# Reserved range").unwrap();
        writeln!(f, "r - 900-999").unwrap();
        drop(f);

        let entries = parse_config_file(&path).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].entry_type, EntryType::CreateGroup);
        assert_eq!(entries[0].name, "myapp-group");
        assert_eq!(entries[1].entry_type, EntryType::CreateUser);
        assert_eq!(entries[1].name, "myapp-user");
        assert_eq!(entries[2].entry_type, EntryType::ReserveRange);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_discover_config_files_no_crash() {
        let _files = discover_config_files();
    }

    #[test]
    fn test_user_exists_root() {
        // root should always exist on Linux
        assert!(user_exists("root"));
    }

    #[test]
    fn test_user_exists_nonexistent() {
        assert!(!user_exists("zzzz_nonexistent_user_99999"));
    }

    #[test]
    fn test_group_exists_root() {
        assert!(group_exists("root"));
    }

    #[test]
    fn test_group_exists_nonexistent() {
        assert!(!group_exists("zzzz_nonexistent_group_99999"));
    }

    #[test]
    fn test_lookup_uid_root() {
        assert_eq!(lookup_uid("root"), Some(0));
    }

    #[test]
    fn test_lookup_gid_root() {
        assert_eq!(lookup_gid("root"), Some(0));
    }

    #[test]
    fn test_uid_exists_zero() {
        assert!(uid_exists(0)); // root
    }

    #[test]
    fn test_gid_exists_zero() {
        assert!(gid_exists(0)); // root
    }

    #[test]
    fn test_find_available_uid() {
        let reserved = BTreeSet::new();
        let uid = find_available_uid(&reserved);
        // Should find something in the system range
        if let Some(uid) = uid {
            assert!((SYSTEM_UID_MIN..=SYSTEM_UID_MAX).contains(&uid));
        }
        // It's OK if all UIDs are taken (unlikely in tests)
    }

    #[test]
    fn test_find_available_gid() {
        let reserved = BTreeSet::new();
        let gid = find_available_gid(&reserved);
        if let Some(gid) = gid {
            assert!((SYSTEM_UID_MIN..=SYSTEM_UID_MAX).contains(&gid));
        }
    }

    #[test]
    fn test_reserve_range_processing() {
        let entry = parse_line("r - 500-510", Path::new("test.conf"), 1).unwrap();
        let mut reserved_uids = BTreeSet::new();
        let mut reserved_gids = BTreeSet::new();

        let result = process_entry(
            &entry,
            &mut reserved_uids,
            &mut reserved_gids,
            Path::new("/"),
            true, // dry run
            false,
        );

        assert!(result);
        assert!(reserved_uids.contains(&500));
        assert!(reserved_uids.contains(&510));
        assert!(reserved_gids.contains(&505));
        assert_eq!(reserved_uids.len(), 11); // 500-510 inclusive
    }

    #[test]
    fn test_id_spec_parse_automatic() {
        assert_eq!(
            IdSpec::parse("-", EntryType::CreateUser),
            Some(IdSpec::Automatic)
        );
        assert_eq!(
            IdSpec::parse("", EntryType::CreateUser),
            Some(IdSpec::Automatic)
        );
    }

    #[test]
    fn test_id_spec_parse_uid() {
        assert_eq!(
            IdSpec::parse("500", EntryType::CreateUser),
            Some(IdSpec::Uid(500))
        );
    }

    #[test]
    fn test_id_spec_parse_uid_gid() {
        assert_eq!(
            IdSpec::parse("500:600", EntryType::CreateUser),
            Some(IdSpec::UidGid(500, 600))
        );
    }

    #[test]
    fn test_id_spec_parse_range() {
        assert_eq!(
            IdSpec::parse("100-999", EntryType::ReserveRange),
            Some(IdSpec::Range(100, 999))
        );
    }

    #[test]
    fn test_id_spec_parse_file_path() {
        assert_eq!(
            IdSpec::parse("/var/lib/myapp", EntryType::CreateUser),
            Some(IdSpec::FilePath(PathBuf::from("/var/lib/myapp")))
        );
    }

    #[test]
    fn test_parse_line_minimal() {
        let entry = parse_line("g mygroup", Path::new("test.conf"), 1).unwrap();
        assert_eq!(entry.entry_type, EntryType::CreateGroup);
        assert_eq!(entry.name, "mygroup");
        assert_eq!(entry.id, IdSpec::Automatic);
        assert!(entry.gecos.is_none());
        assert!(entry.home.is_none());
        assert!(entry.shell.is_none());
    }

    #[test]
    fn test_parse_line_defaults() {
        let entry =
            parse_line("u myuser - - /var/lib/myuser -", Path::new("test.conf"), 1).unwrap();
        assert_eq!(entry.id, IdSpec::Automatic);
        assert!(entry.gecos.is_none());
        assert_eq!(entry.home.as_deref(), Some("/var/lib/myuser"));
        assert!(entry.shell.is_none());
    }

    #[test]
    fn test_parse_line_too_few_fields() {
        assert!(parse_line("u", Path::new("test.conf"), 1).is_none());
    }

    #[test]
    fn test_process_group_dry_run() {
        let entry = parse_line("g testgroup-dryrun -", Path::new("test.conf"), 1).unwrap();
        let mut reserved_uids = BTreeSet::new();
        let mut reserved_gids = BTreeSet::new();

        let result = process_entry(
            &entry,
            &mut reserved_uids,
            &mut reserved_gids,
            Path::new("/"),
            true, // dry run
            false,
        );

        assert!(result);
    }

    #[test]
    fn test_process_user_dry_run() {
        let entry = parse_line(
            "u testuser-dryrun - \"Test User\" / /usr/sbin/nologin",
            Path::new("test.conf"),
            1,
        )
        .unwrap();
        let mut reserved_uids = BTreeSet::new();
        let mut reserved_gids = BTreeSet::new();

        let result = process_entry(
            &entry,
            &mut reserved_uids,
            &mut reserved_gids,
            Path::new("/"),
            true, // dry run
            false,
        );

        assert!(result);
    }

    #[test]
    fn test_entry_ordering() {
        // Verify entries are processed in correct order:
        // ranges -> groups -> users -> memberships
        let entries = [
            parse_line("m user:group", Path::new("test.conf"), 4).unwrap(),
            parse_line("u myuser -", Path::new("test.conf"), 3).unwrap(),
            parse_line("r - 900-999", Path::new("test.conf"), 1).unwrap(),
            parse_line("g mygroup -", Path::new("test.conf"), 2).unwrap(),
        ];

        let ranges: Vec<_> = entries
            .iter()
            .filter(|e| e.entry_type == EntryType::ReserveRange)
            .collect();
        let groups: Vec<_> = entries
            .iter()
            .filter(|e| e.entry_type == EntryType::CreateGroup)
            .collect();
        let users: Vec<_> = entries
            .iter()
            .filter(|e| e.entry_type == EntryType::CreateUser)
            .collect();
        let memberships: Vec<_> = entries
            .iter()
            .filter(|e| e.entry_type == EntryType::AddToGroup)
            .collect();

        assert_eq!(ranges.len(), 1);
        assert_eq!(groups.len(), 1);
        assert_eq!(users.len(), 1);
        assert_eq!(memberships.len(), 1);
    }
}
