//! systemd-machine-id-setup — Initialize or commit the machine ID.
//!
//! A drop-in replacement for `systemd-machine-id-setup(8)`. This tool
//! manages `/etc/machine-id`, which contains a unique 128-bit identifier
//! for the local system, formatted as a 32-character lowercase hex string
//! followed by a newline (33 bytes total).
//!
//! Modes of operation:
//!
//!   systemd-machine-id-setup
//!       Initialize `/etc/machine-id` if it is missing or empty.
//!       Generates a new random machine ID and writes it.
//!
//!   systemd-machine-id-setup --commit
//!       Commit a transient machine ID to disk. During early boot,
//!       `/etc/machine-id` may be bind-mounted as a transient file.
//!       `--commit` reads the current (transient) machine ID, unmounts
//!       the bind mount, and writes the ID persistently to the real file.
//!
//!   systemd-machine-id-setup --print
//!       Print the current machine ID to stdout (after initializing if
//!       necessary).
//!
//! The machine-id-commit.service unit uses:
//!   ExecStart=systemd-machine-id-setup --commit
//!
//! Exit codes:
//!   0 — success
//!   1 — error

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process;

const MACHINE_ID_LEN: usize = 32; // hex characters (128 bits)

#[cfg(test)]
const MACHINE_ID_FILE_LEN: usize = 33; // 32 hex + newline

/// Generate a new random machine ID (128-bit, formatted as 32 hex chars).
fn generate_machine_id() -> io::Result<String> {
    // Try /proc/sys/kernel/random/uuid first (kernel-provided UUID).
    if let Ok(uuid) = fs::read_to_string("/proc/sys/kernel/random/uuid") {
        let hex: String = uuid.trim().chars().filter(|c| *c != '-').collect();
        if hex.len() == 32 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(hex.to_lowercase());
        }
    }

    // Fallback: read 16 bytes from /dev/urandom.
    let mut f = fs::File::open("/dev/urandom")?;
    let mut buf = [0u8; 16];
    f.read_exact(&mut buf)?;

    // Format as 32-char lowercase hex string.
    let hex: String = buf.iter().map(|b| format!("{:02x}", b)).collect();
    Ok(hex)
}

/// Validate that a string is a valid machine ID (32 lowercase hex characters).
fn is_valid_machine_id(id: &str) -> bool {
    id.len() == MACHINE_ID_LEN && id.chars().all(|c| c.is_ascii_hexdigit())
}

/// Read the current machine ID from a file. Returns None if the file
/// doesn't exist, is empty, or contains "uninitialized".
fn read_machine_id(path: &Path) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    let trimmed = contents.trim();

    if trimmed.is_empty() || trimmed == "uninitialized" {
        return None;
    }

    if is_valid_machine_id(trimmed) {
        Some(trimmed.to_string())
    } else {
        eprintln!(
            "Warning: {} contains invalid machine ID: {:?}",
            path.display(),
            trimmed
        );
        None
    }
}

/// Write a machine ID to a file (32 hex chars + newline, mode 0o444).
fn write_machine_id(path: &Path, id: &str) -> io::Result<()> {
    // Write to a temporary file first for atomicity.
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("machine-id path has no parent"))?;
    let tmp_path = dir.join(".machine-id.tmp");

    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o444)
            .open(&tmp_path)?;

        writeln!(f, "{}", id)?;
        f.sync_all()?;
    }

    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Initialize the machine ID if it doesn't exist or is empty.
fn initialize(machine_id_path: &Path) -> Result<String, i32> {
    // Check if we already have a valid machine ID.
    if let Some(existing) = read_machine_id(machine_id_path) {
        eprintln!(
            "Machine ID {} already initialized: {}",
            machine_id_path.display(),
            existing
        );
        return Ok(existing);
    }

    // Try to read from other sources before generating a new one.
    let id = try_dbus_machine_id()
        .or_else(try_product_uuid)
        .map(Ok)
        .unwrap_or_else(|| {
            generate_machine_id().map_err(|e| {
                eprintln!("Error: failed to generate machine ID: {}", e);
                1
            })
        })?;

    match write_machine_id(machine_id_path, &id) {
        Ok(()) => {
            eprintln!(
                "Initialized {} with machine ID: {}",
                machine_id_path.display(),
                id
            );
            Ok(id)
        }
        Err(e) => {
            eprintln!(
                "Error: failed to write {}: {}",
                machine_id_path.display(),
                e
            );
            Err(1)
        }
    }
}

/// Try to read a machine ID from /var/lib/dbus/machine-id (D-Bus compat).
fn try_dbus_machine_id() -> Option<String> {
    let dbus_path = Path::new("/var/lib/dbus/machine-id");
    read_machine_id(dbus_path)
}

/// Try to derive a machine ID from /sys/class/dmi/id/product_uuid.
fn try_product_uuid() -> Option<String> {
    let uuid_path = Path::new("/sys/class/dmi/id/product_uuid");
    let contents = fs::read_to_string(uuid_path).ok()?;
    let hex: String = contents
        .trim()
        .chars()
        .filter(|c| *c != '-')
        .collect::<String>()
        .to_lowercase();

    if is_valid_machine_id(&hex) {
        Some(hex)
    } else {
        None
    }
}

/// Commit a transient machine ID to persistent storage.
///
/// During early boot, `/etc/machine-id` may be a transient bind mount.
/// This function:
///   1. Reads the current (transient) machine ID.
///   2. Attempts to unmount the bind mount.
///   3. Writes the ID to the real file on disk.
fn commit(machine_id_path: &Path) -> Result<String, i32> {
    // Read the current transient machine ID.
    let id = match read_machine_id(machine_id_path) {
        Some(id) => id,
        None => {
            eprintln!(
                "Error: {} does not contain a valid machine ID to commit",
                machine_id_path.display()
            );
            return Err(1);
        }
    };

    // Check if /etc/machine-id is a mount point. If so, try to unmount it
    // so we can write to the underlying file.
    if is_mount_point(machine_id_path) {
        eprintln!(
            "{} is a mount point, attempting to unmount...",
            machine_id_path.display()
        );

        match umount(machine_id_path) {
            Ok(()) => {
                eprintln!("Unmounted {}.", machine_id_path.display());
            }
            Err(e) => {
                eprintln!(
                    "Warning: failed to unmount {}: {} (trying to write anyway)",
                    machine_id_path.display(),
                    e
                );
            }
        }
    }

    // Write the machine ID to the real file.
    match write_machine_id(machine_id_path, &id) {
        Ok(()) => {
            eprintln!(
                "Committed machine ID {} to {}.",
                id,
                machine_id_path.display()
            );
            Ok(id)
        }
        Err(e) => {
            eprintln!(
                "Error: failed to commit machine ID to {}: {}",
                machine_id_path.display(),
                e
            );
            Err(1)
        }
    }
}

/// Check whether a path is a mount point by reading /proc/self/mountinfo.
fn is_mount_point(path: &Path) -> bool {
    let canonical = match fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => return false,
    };

    let mountinfo = match fs::read_to_string("/proc/self/mountinfo") {
        Ok(m) => m,
        Err(_) => return false,
    };

    // Each line in mountinfo has the mount point as the 5th field (0-indexed: 4).
    for line in mountinfo.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 5
            && let Ok(mount_path) = fs::canonicalize(fields[4])
            && mount_path == canonical
        {
            return true;
        }
    }

    false
}

/// Unmount a filesystem.
#[cfg(target_os = "linux")]
fn umount(path: &Path) -> io::Result<()> {
    use std::ffi::CString;

    let c_path = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    let ret = unsafe { libc::umount2(c_path.as_ptr(), 0) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
fn umount(_path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "umount not supported on this platform",
    ))
}

fn usage() {
    eprintln!("Usage: systemd-machine-id-setup [OPTIONS]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --commit         Commit transient machine ID to disk");
    eprintln!("  --print          Print the machine ID to stdout");
    eprintln!("  --root=PATH      Operate on an alternate root directory");
    eprintln!("  --help           Show this help");
}

fn run(args: &[String]) -> i32 {
    let mut do_commit = false;
    let mut do_print = false;
    let mut root = PathBuf::from("/");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--commit" => do_commit = true,
            "--print" => do_print = true,
            "--help" | "-h" => {
                usage();
                return 0;
            }
            "--version" => {
                println!("systemd-machine-id-setup 258");
                return 0;
            }
            arg if arg.starts_with("--root=") => {
                let r = &arg["--root=".len()..];
                root = if r.is_empty() {
                    PathBuf::from("/")
                } else {
                    PathBuf::from(r)
                };
            }
            "--root" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --root requires an argument");
                    return 1;
                }
                root = PathBuf::from(&args[i]);
            }
            other => {
                eprintln!("Unknown option: {}", other);
                usage();
                return 1;
            }
        }
        i += 1;
    }

    let machine_id_path = root.join("etc/machine-id");

    let result = if do_commit {
        commit(&machine_id_path)
    } else {
        initialize(&machine_id_path)
    };

    match result {
        Ok(id) => {
            if do_print {
                println!("{}", id);
            }
            0
        }
        Err(code) => code,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = run(&args);
    process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("failed to create temp dir")
    }

    #[test]
    fn test_constants() {
        assert_eq!(MACHINE_ID_LEN, 32);
        assert_eq!(MACHINE_ID_FILE_LEN, 33);
    }

    #[test]
    fn test_is_valid_machine_id() {
        assert!(is_valid_machine_id("0123456789abcdef0123456789abcdef"));
        assert!(is_valid_machine_id("ABCDEF0123456789ABCDEF0123456789"));
        assert!(is_valid_machine_id("aaaabbbbccccddddeeeeffffaaaabbbb"));
    }

    #[test]
    fn test_is_valid_machine_id_invalid() {
        // Too short
        assert!(!is_valid_machine_id("0123456789abcdef"));
        // Too long
        assert!(!is_valid_machine_id("0123456789abcdef0123456789abcdef0"));
        // Non-hex characters
        assert!(!is_valid_machine_id("0123456789abcdefghijklmnopqrstuv"));
        // Empty
        assert!(!is_valid_machine_id(""));
        // With dashes (UUID format, not machine-id format)
        assert!(!is_valid_machine_id("01234567-89ab-cdef-0123-456789abcdef"));
    }

    #[test]
    fn test_generate_machine_id() {
        let id = generate_machine_id().unwrap();
        assert!(is_valid_machine_id(&id), "Generated invalid ID: {}", id);
    }

    #[test]
    fn test_generate_machine_id_uniqueness() {
        let id1 = generate_machine_id().unwrap();
        let id2 = generate_machine_id().unwrap();
        assert_ne!(id1, id2, "Two generated IDs should be different");
    }

    #[test]
    fn test_read_machine_id_valid() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");
        fs::write(&path, "0123456789abcdef0123456789abcdef\n").unwrap();

        let id = read_machine_id(&path);
        assert_eq!(id, Some("0123456789abcdef0123456789abcdef".to_string()));
    }

    #[test]
    fn test_read_machine_id_no_newline() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");
        fs::write(&path, "0123456789abcdef0123456789abcdef").unwrap();

        let id = read_machine_id(&path);
        assert_eq!(id, Some("0123456789abcdef0123456789abcdef".to_string()));
    }

    #[test]
    fn test_read_machine_id_empty() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");
        fs::write(&path, "").unwrap();

        assert!(read_machine_id(&path).is_none());
    }

    #[test]
    fn test_read_machine_id_uninitialized() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");
        fs::write(&path, "uninitialized\n").unwrap();

        assert!(read_machine_id(&path).is_none());
    }

    #[test]
    fn test_read_machine_id_missing() {
        let dir = temp_dir();
        let path = dir.path().join("nonexistent");

        assert!(read_machine_id(&path).is_none());
    }

    #[test]
    fn test_read_machine_id_invalid_content() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");
        fs::write(&path, "not-a-valid-machine-id\n").unwrap();

        assert!(read_machine_id(&path).is_none());
    }

    #[test]
    fn test_write_machine_id() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");
        let id = "0123456789abcdef0123456789abcdef";

        write_machine_id(&path, id).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "0123456789abcdef0123456789abcdef\n");
    }

    #[test]
    fn test_write_machine_id_permissions() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");
        write_machine_id(&path, "0123456789abcdef0123456789abcdef").unwrap();

        use std::os::unix::fs::MetadataExt;
        let meta = fs::metadata(&path).unwrap();
        let mode = meta.mode() & 0o777;
        assert_eq!(mode, 0o444);
    }

    #[test]
    fn test_write_machine_id_overwrites() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");
        fs::write(&path, "old-content\n").unwrap();

        let id = "aaaabbbbccccddddeeeeffffaaaabbbb";
        write_machine_id(&path, id).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "aaaabbbbccccddddeeeeffffaaaabbbb\n");
    }

    #[test]
    fn test_initialize_creates_new() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");

        let result = initialize(&path);
        assert!(result.is_ok());

        let id = result.unwrap();
        assert!(is_valid_machine_id(&id));

        // File should exist with the ID.
        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents.trim(), id);
    }

    #[test]
    fn test_initialize_preserves_existing() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");
        let existing = "0123456789abcdef0123456789abcdef";
        fs::write(&path, format!("{}\n", existing)).unwrap();

        let result = initialize(&path);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), existing);

        // File should not have changed.
        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents.trim(), existing);
    }

    #[test]
    fn test_initialize_empty_file() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");
        fs::write(&path, "").unwrap();

        let result = initialize(&path);
        assert!(result.is_ok());

        let id = result.unwrap();
        assert!(is_valid_machine_id(&id));
    }

    #[test]
    fn test_initialize_uninitialized_file() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");
        fs::write(&path, "uninitialized\n").unwrap();

        let result = initialize(&path);
        assert!(result.is_ok());

        let id = result.unwrap();
        assert!(is_valid_machine_id(&id));
    }

    #[test]
    fn test_commit_valid_id() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");
        let id = "aabbccdd11223344aabbccdd11223344";
        fs::write(&path, format!("{}\n", id)).unwrap();

        let result = commit(&path);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), id);
    }

    #[test]
    fn test_commit_empty_file_fails() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");
        fs::write(&path, "").unwrap();

        let result = commit(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_commit_missing_file_fails() {
        let dir = temp_dir();
        let path = dir.path().join("nonexistent");

        let result = commit(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_run_default_init() {
        let dir = temp_dir();
        let path = dir.path().join("etc");
        fs::create_dir(&path).unwrap();

        let args = vec![
            "systemd-machine-id-setup".to_string(),
            format!("--root={}", dir.path().display()),
        ];
        let code = run(&args);
        assert_eq!(code, 0);

        let machine_id = dir.path().join("etc/machine-id");
        assert!(machine_id.exists());

        let contents = fs::read_to_string(&machine_id).unwrap();
        assert!(is_valid_machine_id(contents.trim()));
    }

    #[test]
    fn test_run_commit() {
        let dir = temp_dir();
        let etc = dir.path().join("etc");
        fs::create_dir(&etc).unwrap();

        let id = "deadbeefcafebabe1234567890abcdef";
        fs::write(etc.join("machine-id"), format!("{}\n", id)).unwrap();

        let args = vec![
            "systemd-machine-id-setup".to_string(),
            "--commit".to_string(),
            format!("--root={}", dir.path().display()),
        ];
        let code = run(&args);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_run_help() {
        let args = vec!["systemd-machine-id-setup".to_string(), "--help".to_string()];
        let code = run(&args);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_generate_machine_id_is_lowercase() {
        let id = generate_machine_id().unwrap();
        assert_eq!(id, id.to_lowercase());
    }

    #[test]
    fn test_machine_id_file_length() {
        let dir = temp_dir();
        let path = dir.path().join("machine-id");
        let id = "0123456789abcdef0123456789abcdef";

        write_machine_id(&path, id).unwrap();

        let contents = fs::read(&path).unwrap();
        assert_eq!(contents.len(), MACHINE_ID_FILE_LEN);
    }
}
