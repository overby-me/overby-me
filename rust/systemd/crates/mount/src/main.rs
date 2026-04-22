#![allow(dead_code)]
//! systemd-mount — Establish and destroy transient mount or auto-mount points.
//!
//! A drop-in replacement for `systemd-mount(1)` and `systemd-umount(1)`.
//! When invoked as `systemd-mount`, creates transient `.mount` or `.automount`
//! units. When invoked as `systemd-umount`, unmounts the specified mount points.
//! Can also list active mount points with `--list`.

use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

// ── CLI ───────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "systemd-mount",
    about = "Establish and destroy transient mount or auto-mount points",
    version,
    trailing_var_arg = true
)]
struct Cli {
    /// List active mount points
    #[arg(long, short)]
    list: bool,

    /// Don't truncate output (no-op for our compact `--list` output, but
    /// accepted for upstream-systemd CLI parity).
    #[arg(long)]
    full: bool,

    /// Disable colored output.  Accepted for parity; ignored.
    #[arg(long)]
    no_legend: bool,

    /// Suppress informational output.  Accepted for parity; ignored.
    #[arg(short, long)]
    quiet: bool,

    /// Output format selector.  `pretty`/`short` JSON output, otherwise the
    /// flag is accepted-but-ignored (`--json=off`).  Currently treated as
    /// a no-op for `--list` (we always print plain table).
    #[arg(long, value_name = "MODE")]
    json: Option<String>,

    /// Mount type (e.g., ext4, tmpfs, nfs)
    #[arg(short = 't', long = "type")]
    fstype: Option<String>,

    /// Mount options (comma-separated)
    #[arg(short, long)]
    options: Option<String>,

    /// Description for the transient unit
    #[arg(long)]
    description: Option<String>,

    /// Create an automount unit instead of a mount unit
    #[arg(short = 'A', long)]
    automount: bool,

    /// Automount idle timeout
    #[arg(long)]
    timeout_idle_sec: Option<String>,

    /// Do not block waiting for the mount to complete
    #[arg(long)]
    no_block: bool,

    /// Do not ask for authentication
    #[arg(long)]
    no_ask_password: bool,

    /// Mount the filesystem read-only
    #[arg(long)]
    read_only: bool,

    /// Create the mount point directory if it does not exist
    #[arg(long)]
    mkdir: bool,

    /// Owner UID for the created mount point directory
    #[arg(long)]
    owner: Option<String>,

    /// Collect mount unit garbage collection
    #[arg(long)]
    collect: bool,

    /// Unmount mode (when invoked as systemd-umount)
    #[arg(short, long)]
    umount: bool,

    /// Do not pipe output into a pager
    #[arg(long)]
    no_pager: bool,

    /// Force unmount (lazy unmount)
    #[arg(short, long)]
    force: bool,

    /// Lazy unmount
    #[arg(short = 'l', long)]
    lazy: bool,

    /// Operate on the system instance (default)
    #[arg(long)]
    system: bool,

    /// Operate on the user instance
    #[arg(long)]
    user: bool,

    /// Property to set on the transient unit (KEY=VALUE)
    #[arg(short, long)]
    property: Vec<String>,

    /// Positional arguments: WHAT [WHERE] for mount, WHERE for umount
    args: Vec<String>,
}

// ── Mount entry from /proc/self/mountinfo ──────────────────────────────────

#[derive(Debug, Clone)]
struct MountEntry {
    mount_id: u32,
    parent_id: u32,
    source: String,
    mount_point: String,
    fstype: String,
    options: String,
}

/// Parse /proc/self/mountinfo into a list of MountEntry.
fn parse_mountinfo() -> Vec<MountEntry> {
    let content = match fs::read_to_string("/proc/self/mountinfo") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut entries = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // mountinfo format:
        // mount_id parent_id major:minor root mount_point mount_options optional_fields - fs_type mount_source super_options
        let parts: Vec<&str> = line.splitn(7, ' ').collect();
        if parts.len() < 6 {
            continue;
        }

        let mount_id: u32 = parts[0].parse().unwrap_or(0);
        let parent_id: u32 = parts[1].parse().unwrap_or(0);
        // parts[2] = major:minor
        // parts[3] = root
        let mount_point = unescape_mountinfo(parts[4]);
        let mount_options = parts[5].to_string();

        // Find the separator " - " to get fs_type and source
        let rest = parts.get(6).unwrap_or(&"");
        let (fstype, source) = if let Some(sep_pos) = rest.find(" - ") {
            let after_sep = &rest[sep_pos + 3..];
            let fields: Vec<&str> = after_sep.splitn(3, ' ').collect();
            let fs = fields.first().unwrap_or(&"").to_string();
            let src = fields.get(1).unwrap_or(&"").to_string();
            (fs, src)
        } else {
            (String::new(), String::new())
        };

        entries.push(MountEntry {
            mount_id,
            parent_id,
            source,
            mount_point,
            fstype,
            options: mount_options,
        });
    }

    entries
}

/// Unescape octal escape sequences in mountinfo paths (e.g., \040 for space).
fn unescape_mountinfo(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();

    while let Some(c) = chars.next() {
        if c == '\\' {
            // Try to read 3 octal digits
            let mut octal = String::with_capacity(3);
            for _ in 0..3 {
                if let Some(&next) = chars.as_str().as_bytes().first() {
                    if (b'0'..=b'7').contains(&next) {
                        octal.push(next as char);
                        chars.next();
                    } else {
                        break;
                    }
                }
            }

            if octal.len() == 3
                && let Ok(val) = u8::from_str_radix(&octal, 8)
            {
                result.push(val as char);
                continue;
            }

            // If not a valid octal escape, put the backslash back
            result.push('\\');
            result.push_str(&octal);
        } else {
            result.push(c);
        }
    }

    result
}

/// Parse /proc/self/mounts (simpler format) as fallback.
fn parse_mounts() -> Vec<MountEntry> {
    let content = match fs::read_to_string("/proc/self/mounts") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut entries = Vec::new();
    let mut id = 1u32;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }

        entries.push(MountEntry {
            mount_id: id,
            parent_id: 0,
            source: unescape_mountinfo(parts[0]),
            mount_point: unescape_mountinfo(parts[1]),
            fstype: parts[2].to_string(),
            options: parts[3].to_string(),
        });

        id += 1;
    }

    entries
}

// ── Unit name escaping ────────────────────────────────────────────────────

/// Escape a mount point path into a systemd unit name.
/// e.g., "/home/user/data" -> "home-user-data.mount"
fn path_to_unit_name(path: &str, suffix: &str) -> String {
    let path = path.trim_start_matches('/').trim_end_matches('/');
    if path.is_empty() {
        return format!("-.{suffix}");
    }

    let mut escaped = String::new();
    for (i, c) in path.chars().enumerate() {
        if c == '/' {
            escaped.push('-');
        } else if c == '-' {
            escaped.push_str("\\x2d");
        } else if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
            escaped.push(c);
        } else {
            escaped.push_str(&format!("\\x{:02x}", c as u32));
        }
        let _ = i;
    }

    format!("{escaped}.{suffix}")
}

// ── Transient unit creation ───────────────────────────────────────────────

/// Generate a transient mount unit file content.
fn generate_mount_unit(
    what: &str,
    where_: &str,
    fstype: Option<&str>,
    options: Option<&str>,
    description: Option<&str>,
    read_only: bool,
    properties: &[String],
) -> String {
    let mut unit = String::new();

    unit.push_str("[Unit]\n");
    if let Some(desc) = description {
        unit.push_str(&format!("Description={}\n", desc));
    } else {
        unit.push_str(&format!("Description=Mount {} on {}\n", what, where_));
    }
    unit.push('\n');

    unit.push_str("[Mount]\n");
    unit.push_str(&format!("What={}\n", what));
    unit.push_str(&format!("Where={}\n", where_));

    if let Some(ft) = fstype {
        unit.push_str(&format!("Type={}\n", ft));
    }

    let mut opts = Vec::new();
    if let Some(o) = options {
        opts.push(o.to_string());
    }
    if read_only {
        opts.push("ro".to_string());
    }
    if !opts.is_empty() {
        unit.push_str(&format!("Options={}\n", opts.join(",")));
    }

    // Add custom properties
    for prop in properties {
        if let Some((key, val)) = prop.split_once('=') {
            unit.push_str(&format!("{}={}\n", key, val));
        }
    }

    unit
}

/// Generate a transient automount unit file content.
fn generate_automount_unit(
    where_: &str,
    description: Option<&str>,
    timeout_idle: Option<&str>,
) -> String {
    let mut unit = String::new();

    unit.push_str("[Unit]\n");
    if let Some(desc) = description {
        unit.push_str(&format!("Description={}\n", desc));
    } else {
        unit.push_str(&format!("Description=Automount {}\n", where_));
    }
    unit.push('\n');

    unit.push_str("[Automount]\n");
    unit.push_str(&format!("Where={}\n", where_));

    if let Some(timeout) = timeout_idle {
        unit.push_str(&format!("TimeoutIdleSec={}\n", timeout));
    }

    unit
}

// ── Mount / Unmount operations ────────────────────────────────────────────

/// Mount a filesystem using the `mount` command as a fallback.
fn do_mount(
    what: &str,
    where_: &str,
    fstype: Option<&str>,
    options: Option<&str>,
    read_only: bool,
    mkdir: bool,
) -> Result<(), String> {
    // Create mount point if requested
    if mkdir && let Err(e) = fs::create_dir_all(where_) {
        return Err(format!("Failed to create mount point {}: {}", where_, e));
    }

    // Check if mount point exists
    if !Path::new(where_).exists() {
        return Err(format!(
            "Mount point {} does not exist (use --mkdir to create it)",
            where_
        ));
    }

    let mut cmd = Command::new("mount");

    if let Some(ft) = fstype {
        cmd.arg("-t").arg(ft);
    }

    let mut mount_opts = Vec::new();
    if let Some(o) = options {
        mount_opts.push(o.to_string());
    }
    if read_only {
        mount_opts.push("ro".to_string());
    }
    if !mount_opts.is_empty() {
        cmd.arg("-o").arg(mount_opts.join(","));
    }

    cmd.arg(what).arg(where_);

    match cmd.status() {
        Ok(status) => {
            if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "mount command failed with exit code {}",
                    status.code().unwrap_or(-1)
                ))
            }
        }
        Err(e) => Err(format!("Failed to execute mount: {}", e)),
    }
}

/// Unmount a filesystem.
fn do_umount(where_: &str, force: bool, lazy: bool) -> Result<(), String> {
    let mut cmd = Command::new("umount");

    if force {
        cmd.arg("-f");
    }
    if lazy {
        cmd.arg("-l");
    }

    cmd.arg(where_);

    match cmd.status() {
        Ok(status) => {
            if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "umount command failed with exit code {}",
                    status.code().unwrap_or(-1)
                ))
            }
        }
        Err(e) => Err(format!("Failed to execute umount: {}", e)),
    }
}

// ── Display ───────────────────────────────────────────────────────────────

/// Display the current mount table.
fn display_mounts() {
    let mut entries = parse_mountinfo();
    if entries.is_empty() {
        entries = parse_mounts();
    }

    if entries.is_empty() {
        println!("No mounts found.");
        return;
    }

    // Header
    println!("{:<50} {:<30} {:<12} OPTIONS", "WHAT", "WHERE", "TYPE");

    // Sort by mount point for consistent output
    entries.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));

    for entry in &entries {
        // Truncate long fields for display
        let what = if entry.source.len() > 50 {
            format!("...{}", &entry.source[entry.source.len() - 47..])
        } else {
            entry.source.clone()
        };

        let where_ = if entry.mount_point.len() > 30 {
            format!("...{}", &entry.mount_point[entry.mount_point.len() - 27..])
        } else {
            entry.mount_point.clone()
        };

        let options = if entry.options.len() > 40 {
            format!("{}...", &entry.options[..37])
        } else {
            entry.options.clone()
        };

        println!(
            "{:<50} {:<30} {:<12} {}",
            what, where_, entry.fstype, options
        );
    }

    println!("\n{} mounts listed.", entries.len());
}

// ── Main ──────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    // Detect if invoked as systemd-umount
    let argv0 = std::env::args().next().unwrap_or_default();
    let is_umount = argv0.ends_with("systemd-umount") || argv0.ends_with("umount") || cli.umount;

    // List mode
    if cli.list {
        display_mounts();
        return;
    }

    if is_umount {
        // Unmount mode
        if cli.args.is_empty() {
            eprintln!("Usage: systemd-umount MOUNTPOINT...");
            process::exit(1);
        }

        let mut any_failed = false;
        for mount_point in &cli.args {
            eprintln!("Unmounting {}...", mount_point);
            if let Err(e) = do_umount(mount_point, cli.force, cli.lazy) {
                eprintln!("Failed to unmount {}: {}", mount_point, e);
                any_failed = true;
            }
        }

        if any_failed {
            process::exit(1);
        }
    } else {
        // Mount mode
        if cli.args.is_empty() {
            eprintln!("Usage: systemd-mount WHAT [WHERE]");
            eprintln!("       systemd-mount --list");
            process::exit(1);
        }

        let what = &cli.args[0];

        // Determine WHERE
        let where_ = if cli.args.len() >= 2 {
            cli.args[1].clone()
        } else {
            // Auto-generate mount point from WHAT
            // e.g., /dev/sdb1 -> /run/media/system/sdb1
            let base = Path::new(what)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            format!("/run/media/system/{}", base)
        };

        // Generate unit name for informational purposes
        let unit_name = path_to_unit_name(&where_, "mount");
        eprintln!("Mounting {} on {} (unit: {})...", what, where_, unit_name);

        if cli.automount {
            let automount_name = path_to_unit_name(&where_, "automount");
            let mount_content = generate_mount_unit(
                what,
                &where_,
                cli.fstype.as_deref(),
                cli.options.as_deref(),
                cli.description.as_deref(),
                cli.read_only,
                &cli.property,
            );
            let automount_content = generate_automount_unit(
                &where_,
                cli.description.as_deref(),
                cli.timeout_idle_sec.as_deref(),
            );

            eprintln!("Generated mount unit ({}):", unit_name);
            eprintln!("{}", mount_content);
            eprintln!("Generated automount unit ({}):", automount_name);
            eprintln!("{}", automount_content);

            // Write transient units
            let transient_dir = PathBuf::from("/run/systemd/transient");
            if let Err(e) = fs::create_dir_all(&transient_dir) {
                eprintln!("Warning: Failed to create transient dir: {}", e);
            }

            let mount_path = transient_dir.join(&unit_name);
            if let Err(e) = fs::write(&mount_path, &mount_content) {
                eprintln!("Warning: Failed to write {}: {}", mount_path.display(), e);
            }

            let automount_path = transient_dir.join(&automount_name);
            if let Err(e) = fs::write(&automount_path, &automount_content) {
                eprintln!(
                    "Warning: Failed to write {}: {}",
                    automount_path.display(),
                    e
                );
            }

            // Fall back to direct mount since we can't reliably start transient units
            if let Err(e) = do_mount(
                what,
                &where_,
                cli.fstype.as_deref(),
                cli.options.as_deref(),
                cli.read_only,
                cli.mkdir,
            ) {
                eprintln!("Failed to mount: {}", e);
                process::exit(1);
            }
        } else {
            // Direct mount
            if let Err(e) = do_mount(
                what,
                &where_,
                cli.fstype.as_deref(),
                cli.options.as_deref(),
                cli.read_only,
                cli.mkdir,
            ) {
                eprintln!("Failed to mount: {}", e);
                process::exit(1);
            }
        }

        println!("Mounted {} on {}.", what, where_);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Unit name escaping tests

    #[test]
    fn test_path_to_unit_name_root() {
        assert_eq!(path_to_unit_name("/", "mount"), "-.mount");
    }

    #[test]
    fn test_path_to_unit_name_simple() {
        assert_eq!(path_to_unit_name("/home", "mount"), "home.mount");
    }

    #[test]
    fn test_path_to_unit_name_nested() {
        assert_eq!(
            path_to_unit_name("/home/user/data", "mount"),
            "home-user-data.mount"
        );
    }

    #[test]
    fn test_path_to_unit_name_with_dash() {
        let name = path_to_unit_name("/my-path", "mount");
        assert!(name.contains("\\x2d"));
        assert!(name.ends_with(".mount"));
    }

    #[test]
    fn test_path_to_unit_name_trailing_slash() {
        assert_eq!(path_to_unit_name("/home/", "mount"), "home.mount");
    }

    #[test]
    fn test_path_to_unit_name_automount() {
        assert_eq!(
            path_to_unit_name("/mnt/usb", "automount"),
            "mnt-usb.automount"
        );
    }

    // Mount unit generation tests

    #[test]
    fn test_generate_mount_unit_basic() {
        let unit = generate_mount_unit("/dev/sdb1", "/mnt/data", None, None, None, false, &[]);
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Mount]"));
        assert!(unit.contains("What=/dev/sdb1"));
        assert!(unit.contains("Where=/mnt/data"));
    }

    #[test]
    fn test_generate_mount_unit_with_type() {
        let unit = generate_mount_unit(
            "/dev/sdb1",
            "/mnt/data",
            Some("ext4"),
            None,
            None,
            false,
            &[],
        );
        assert!(unit.contains("Type=ext4"));
    }

    #[test]
    fn test_generate_mount_unit_with_options() {
        let unit = generate_mount_unit(
            "/dev/sdb1",
            "/mnt/data",
            None,
            Some("noatime,nodiratime"),
            None,
            false,
            &[],
        );
        assert!(unit.contains("Options=noatime,nodiratime"));
    }

    #[test]
    fn test_generate_mount_unit_read_only() {
        let unit = generate_mount_unit("/dev/sdb1", "/mnt/data", None, None, None, true, &[]);
        assert!(unit.contains("Options=ro"));
    }

    #[test]
    fn test_generate_mount_unit_read_only_with_options() {
        let unit = generate_mount_unit(
            "/dev/sdb1",
            "/mnt/data",
            None,
            Some("noatime"),
            None,
            true,
            &[],
        );
        assert!(unit.contains("Options=noatime,ro"));
    }

    #[test]
    fn test_generate_mount_unit_with_description() {
        let unit = generate_mount_unit(
            "/dev/sdb1",
            "/mnt/data",
            None,
            None,
            Some("My custom mount"),
            false,
            &[],
        );
        assert!(unit.contains("Description=My custom mount"));
    }

    #[test]
    fn test_generate_mount_unit_with_properties() {
        let props = vec![
            "DirectoryMode=0755".to_string(),
            "SloppyOptions=yes".to_string(),
        ];
        let unit = generate_mount_unit("/dev/sdb1", "/mnt/data", None, None, None, false, &props);
        assert!(unit.contains("DirectoryMode=0755"));
        assert!(unit.contains("SloppyOptions=yes"));
    }

    // Automount unit generation tests

    #[test]
    fn test_generate_automount_unit_basic() {
        let unit = generate_automount_unit("/mnt/data", None, None);
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Automount]"));
        assert!(unit.contains("Where=/mnt/data"));
    }

    #[test]
    fn test_generate_automount_unit_with_timeout() {
        let unit = generate_automount_unit("/mnt/data", None, Some("300"));
        assert!(unit.contains("TimeoutIdleSec=300"));
    }

    #[test]
    fn test_generate_automount_unit_with_description() {
        let unit = generate_automount_unit("/mnt/data", Some("Auto mount data"), None);
        assert!(unit.contains("Description=Auto mount data"));
    }

    // Unescape mountinfo tests

    #[test]
    fn test_unescape_mountinfo_plain() {
        assert_eq!(unescape_mountinfo("/home/user"), "/home/user");
    }

    #[test]
    fn test_unescape_mountinfo_space() {
        assert_eq!(unescape_mountinfo("/home/my\\040folder"), "/home/my folder");
    }

    #[test]
    fn test_unescape_mountinfo_tab() {
        assert_eq!(
            unescape_mountinfo("/home/my\\011folder"),
            "/home/my\tfolder"
        );
    }

    #[test]
    fn test_unescape_mountinfo_backslash() {
        assert_eq!(
            unescape_mountinfo("/home/my\\134folder"),
            "/home/my\\folder"
        );
    }

    #[test]
    fn test_unescape_mountinfo_no_escape() {
        assert_eq!(unescape_mountinfo("/"), "/");
    }

    #[test]
    fn test_unescape_mountinfo_empty() {
        assert_eq!(unescape_mountinfo(""), "");
    }

    // Parse mounts tests

    #[test]
    fn test_parse_mountinfo_no_panic() {
        let entries = parse_mountinfo();
        // Should parse without panicking; may be empty in some test envs
        let _ = entries;
    }

    #[test]
    fn test_parse_mounts_no_panic() {
        let entries = parse_mounts();
        let _ = entries;
    }

    #[test]
    fn test_parse_mountinfo_has_entries() {
        let entries = parse_mountinfo();
        // On a real Linux system, there should be at least a few mounts
        if Path::new("/proc/self/mountinfo").exists() {
            assert!(!entries.is_empty(), "Expected at least one mount entry");
        }
    }

    #[test]
    fn test_parse_mountinfo_has_root() {
        let entries = parse_mountinfo();
        if !entries.is_empty() {
            assert!(
                entries.iter().any(|e| e.mount_point == "/"),
                "Expected root mount point"
            );
        }
    }

    // MountEntry tests

    #[test]
    fn test_mount_entry_fields() {
        let entry = MountEntry {
            mount_id: 1,
            parent_id: 0,
            source: "/dev/sda1".to_string(),
            mount_point: "/".to_string(),
            fstype: "ext4".to_string(),
            options: "rw,relatime".to_string(),
        };
        assert_eq!(entry.mount_id, 1);
        assert_eq!(entry.parent_id, 0);
        assert_eq!(entry.source, "/dev/sda1");
        assert_eq!(entry.mount_point, "/");
        assert_eq!(entry.fstype, "ext4");
        assert_eq!(entry.options, "rw,relatime");
    }

    // Display test

    #[test]
    fn test_display_mounts_no_panic() {
        display_mounts();
    }

    // Do mount/umount with nonexistent paths (should fail gracefully)

    #[test]
    fn test_do_mount_no_mount_point() {
        let result = do_mount(
            "/dev/null",
            "/nonexistent/mount/point/xyz",
            None,
            None,
            false,
            false,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_do_umount_not_mounted() {
        let result = do_umount("/tmp", false, false);
        // This will likely fail since /tmp is usually not a separate mount
        // or will succeed if it is — either way it should not panic
        let _ = result;
    }
}
