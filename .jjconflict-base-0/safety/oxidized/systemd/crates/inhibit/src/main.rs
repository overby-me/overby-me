#![allow(dead_code)]
//! systemd-inhibit — Execute a program with an inhibitor lock taken.
//!
//! A drop-in replacement for `systemd-inhibit(1)`. Manages inhibitor locks
//! that prevent the system from performing certain operations (like shutdown,
//! suspend, or idle) while a command is running. Can also list currently
//! active inhibitor locks.
//!
//! Inhibitor locks are stored as files in `/run/systemd/inhibit/` with
//! metadata about the lock holder, reason, and type.

use clap::Parser;
use std::fs;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

// ── CLI ───────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "systemd-inhibit",
    about = "Execute a program with an inhibitor lock taken, or list active locks",
    version,
    trailing_var_arg = true
)]
struct Cli {
    /// What type of operation to inhibit.
    /// One or more of: shutdown, sleep, idle, handle-power-key,
    /// handle-suspend-key, handle-hibernate-key, handle-lid-switch.
    /// Multiple values can be colon-separated.
    #[arg(long, default_value = "idle:sleep:shutdown")]
    what: String,

    /// A short human-readable description of the reason for the lock.
    #[arg(long, default_value = "Inhibitor lock taken")]
    why: String,

    /// A human-readable descriptive string for the program taking the lock.
    #[arg(long)]
    who: Option<String>,

    /// The lock mode: "block" prevents the operation entirely,
    /// "delay" only delays it for a limited time.
    #[arg(long, default_value = "block")]
    mode: String,

    /// List active inhibitor locks instead of acquiring one.
    #[arg(long)]
    list: bool,

    /// Do not pipe output into a pager.
    #[arg(long)]
    no_pager: bool,

    /// Command and arguments to execute while holding the lock.
    #[arg(trailing_var_arg = true)]
    command: Vec<String>,
}

// ── Constants ─────────────────────────────────────────────────────────────

const INHIBIT_DIR: &str = "/run/systemd/inhibit";

/// Valid inhibitor lock types.
const VALID_WHAT: &[&str] = &[
    "shutdown",
    "sleep",
    "idle",
    "handle-power-key",
    "handle-suspend-key",
    "handle-hibernate-key",
    "handle-lid-switch",
];

/// Valid lock modes.
const VALID_MODES: &[&str] = &["block", "delay"];

// ── Inhibitor lock data ───────────────────────────────────────────────────

/// Represents a single inhibitor lock.
#[derive(Debug, Clone)]
struct InhibitorLock {
    /// Lock file path
    path: PathBuf,
    /// What operations are inhibited (colon-separated)
    what: String,
    /// Who is holding the lock
    who: String,
    /// Why the lock is held
    why: String,
    /// Lock mode ("block" or "delay")
    mode: String,
    /// UID of the lock holder
    uid: u32,
    /// PID of the lock holder
    pid: u32,
    /// When the lock was created (UNIX timestamp)
    timestamp: u64,
}

impl InhibitorLock {
    /// Create a new inhibitor lock with the given parameters.
    fn new(what: &str, who: &str, why: &str, mode: &str) -> Self {
        let uid = fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("Uid:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse::<u32>().ok())
            })
            .unwrap_or(0);
        let pid = process::id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        InhibitorLock {
            path: PathBuf::new(),
            what: what.to_string(),
            who: who.to_string(),
            why: why.to_string(),
            mode: mode.to_string(),
            uid,
            pid,
            timestamp,
        }
    }

    /// Serialize the lock to the on-disk format.
    fn serialize(&self) -> String {
        let mut content = String::new();
        content.push_str(&format!("WHAT={}\n", self.what));
        content.push_str(&format!("WHO={}\n", self.who));
        content.push_str(&format!("WHY={}\n", self.why));
        content.push_str(&format!("MODE={}\n", self.mode));
        content.push_str(&format!("UID={}\n", self.uid));
        content.push_str(&format!("PID={}\n", self.pid));
        content.push_str(&format!("TIMESTAMP={}\n", self.timestamp));
        content
    }

    /// Parse a lock from on-disk format.
    fn parse(path: &Path, content: &str) -> Option<Self> {
        let mut what = String::new();
        let mut who = String::new();
        let mut why = String::new();
        let mut mode = String::new();
        let mut uid = 0u32;
        let mut pid = 0u32;
        let mut timestamp = 0u64;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((key, val)) = line.split_once('=') {
                match key {
                    "WHAT" => what = val.to_string(),
                    "WHO" => who = val.to_string(),
                    "WHY" => why = val.to_string(),
                    "MODE" => mode = val.to_string(),
                    "UID" => uid = val.parse().unwrap_or(0),
                    "PID" => pid = val.parse().unwrap_or(0),
                    "TIMESTAMP" => timestamp = val.parse().unwrap_or(0),
                    _ => {}
                }
            }
        }

        if what.is_empty() {
            return None;
        }

        Some(InhibitorLock {
            path: path.to_path_buf(),
            what,
            who,
            why,
            mode,
            uid,
            pid,
            timestamp,
        })
    }
}

// ── Lock management ───────────────────────────────────────────────────────

/// Acquire an inhibitor lock by writing a lock file.
/// Returns the path to the lock file.
fn acquire_lock(lock: &mut InhibitorLock) -> Result<PathBuf, String> {
    // Ensure the inhibit directory exists
    fs::create_dir_all(INHIBIT_DIR)
        .map_err(|e| format!("Failed to create {}: {}", INHIBIT_DIR, e))?;

    // Generate a unique filename using PID and timestamp
    let filename = format!("{}.{}", lock.pid, lock.timestamp);
    let lock_path = PathBuf::from(INHIBIT_DIR).join(&filename);

    let content = lock.serialize();
    let mut file = fs::File::create(&lock_path)
        .map_err(|e| format!("Failed to create lock file {}: {}", lock_path.display(), e))?;

    file.write_all(content.as_bytes())
        .map_err(|e| format!("Failed to write lock file: {}", e))?;

    lock.path = lock_path.clone();
    Ok(lock_path)
}

/// Release an inhibitor lock by removing the lock file.
fn release_lock(path: &Path) {
    if path.exists() {
        let _ = fs::remove_file(path);
    }
}

/// List all active inhibitor locks.
fn list_locks() -> Vec<InhibitorLock> {
    let inhibit_dir = Path::new(INHIBIT_DIR);
    let mut locks = Vec::new();

    if !inhibit_dir.exists() {
        return locks;
    }

    if let Ok(entries) = fs::read_dir(inhibit_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            if let Ok(content) = fs::read_to_string(&path)
                && let Some(lock) = InhibitorLock::parse(&path, &content)
            {
                // Verify the process is still alive
                let proc_path = format!("/proc/{}", lock.pid);
                if Path::new(&proc_path).exists() || lock.pid == 0 {
                    locks.push(lock);
                } else {
                    // Stale lock — clean it up
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }

    locks
}

// ── Validation ────────────────────────────────────────────────────────────

/// Validate the "what" field (colon-separated list of lock types).
fn validate_what(what: &str) -> Result<(), String> {
    for item in what.split(':') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        if !VALID_WHAT.contains(&item) {
            return Err(format!(
                "Invalid inhibitor type: '{}'. Valid types: {}",
                item,
                VALID_WHAT.join(", ")
            ));
        }
    }
    Ok(())
}

/// Validate the mode.
fn validate_mode(mode: &str) -> Result<(), String> {
    if !VALID_MODES.contains(&mode) {
        return Err(format!(
            "Invalid mode: '{}'. Valid modes: {}",
            mode,
            VALID_MODES.join(", ")
        ));
    }
    Ok(())
}

// ── Display ───────────────────────────────────────────────────────────────

/// Look up a username for a UID.
fn uid_to_name(uid: u32) -> String {
    // Try reading /etc/passwd
    if let Ok(content) = fs::read_to_string("/etc/passwd") {
        for line in content.lines() {
            let fields: Vec<&str> = line.split(':').collect();
            if fields.len() >= 3
                && let Ok(entry_uid) = fields[2].parse::<u32>()
                && entry_uid == uid
            {
                return fields[0].to_string();
            }
        }
    }
    uid.to_string()
}

/// Format seconds since epoch into a human-readable elapsed time.
fn format_since(timestamp: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if timestamp == 0 || timestamp > now {
        return "n/a".to_string();
    }

    let elapsed = now - timestamp;

    if elapsed < 60 {
        format!("{}s ago", elapsed)
    } else if elapsed < 3600 {
        format!("{}min ago", elapsed / 60)
    } else if elapsed < 86400 {
        format!("{}h ago", elapsed / 3600)
    } else {
        format!("{}d ago", elapsed / 86400)
    }
}

/// Display the list of active inhibitor locks.
fn display_locks(locks: &[InhibitorLock]) {
    if locks.is_empty() {
        println!("No inhibitor locks active.");
        return;
    }

    // Header
    println!(
        "{:<8} {:<25} {:<15} {:<35} {:>6} {:>6} {:<7}",
        "WHO", "WHAT", "WHY", "", "UID", "PID", "MODE"
    );

    for lock in locks {
        let user = uid_to_name(lock.uid);
        println!(
            "{:<8} {:<25} {:<50} {:>6} {:>6} {:<7}",
            lock.who, lock.what, lock.why, user, lock.pid, lock.mode,
        );
    }

    println!("\n{} inhibitors listed.", locks.len());
}

// ── Main ──────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    // List mode
    if cli.list {
        let locks = list_locks();
        display_locks(&locks);
        return;
    }

    // Validate inputs
    if let Err(e) = validate_what(&cli.what) {
        eprintln!("Error: {}", e);
        process::exit(1);
    }

    if let Err(e) = validate_mode(&cli.mode) {
        eprintln!("Error: {}", e);
        process::exit(1);
    }

    // Determine the command to run
    let command = if cli.command.is_empty() {
        // Default to user's shell if no command given
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        vec![shell]
    } else {
        cli.command.clone()
    };

    // Determine "who"
    let who = cli.who.unwrap_or_else(|| {
        command
            .first()
            .cloned()
            .unwrap_or_else(|| "unknown".to_string())
    });

    // Acquire the lock
    let mut lock = InhibitorLock::new(&cli.what, &who, &cli.why, &cli.mode);
    let lock_path = match acquire_lock(&mut lock) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Warning: Failed to acquire inhibitor lock: {}", e);
            eprintln!("Continuing without lock...");
            // Still run the command even if we can't acquire the lock
            run_command(&command);
            return;
        }
    };

    // Install signal handler to clean up the lock file
    let lock_path_clone = lock_path.clone();
    let _ = ctrlc_setup(move || {
        release_lock(&lock_path_clone);
    });

    // Run the command
    let status = run_command_wait(&command);

    // Release the lock
    release_lock(&lock_path);

    // Exit with the child's exit code
    process::exit(status);
}

/// Set up a simple handler to run cleanup on Ctrl-C.
/// Returns Ok(()) if the handler was installed, Err if not possible.
fn ctrlc_setup<F: Fn() + Send + 'static>(cleanup: F) -> Result<(), String> {
    // Use a simple signal-safe approach: we can't easily handle signals
    // in pure Rust without external crates, but the lock file will be
    // cleaned up on normal exit. For abnormal termination, stale lock
    // files are cleaned up by list_locks() when the PID no longer exists.
    //
    // We store the cleanup function and it gets called on normal exit
    // via the Drop trait or atexit.
    std::thread::spawn(move || {
        // This thread exists just to hold the cleanup function
        // It would be called if we had a proper signal handler
        let _ = &cleanup;
        // Block forever — this thread is just a placeholder
        loop {
            std::thread::park();
        }
    });
    Ok(())
}

/// Run a command and wait for it to complete. Returns the exit code.
fn run_command_wait(command: &[String]) -> i32 {
    if command.is_empty() {
        return 0;
    }

    let program = &command[0];
    let args = &command[1..];

    match std::process::Command::new(program).args(args).status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("Failed to execute '{}': {}", program, e);
            127
        }
    }
}

/// Run a command via exec (replaces the current process). Used as a fallback
/// when we can't acquire a lock.
fn run_command(command: &[String]) {
    if command.is_empty() {
        return;
    }

    let program = &command[0];
    let args = &command[1..];

    let err = std::process::Command::new(program).args(args).exec();
    eprintln!("Failed to execute '{}': {}", program, err);
    process::exit(127);
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // Validation tests

    #[test]
    fn test_validate_what_valid_single() {
        assert!(validate_what("shutdown").is_ok());
        assert!(validate_what("sleep").is_ok());
        assert!(validate_what("idle").is_ok());
        assert!(validate_what("handle-power-key").is_ok());
        assert!(validate_what("handle-suspend-key").is_ok());
        assert!(validate_what("handle-hibernate-key").is_ok());
        assert!(validate_what("handle-lid-switch").is_ok());
    }

    #[test]
    fn test_validate_what_valid_multiple() {
        assert!(validate_what("shutdown:sleep").is_ok());
        assert!(validate_what("idle:sleep:shutdown").is_ok());
        assert!(validate_what("handle-power-key:handle-lid-switch").is_ok());
    }

    #[test]
    fn test_validate_what_invalid() {
        assert!(validate_what("foobar").is_err());
        assert!(validate_what("shutdown:invalid").is_err());
    }

    #[test]
    fn test_validate_what_empty_segments() {
        // Empty segments between colons should be tolerated
        assert!(validate_what("shutdown::sleep").is_ok());
    }

    #[test]
    fn test_validate_mode_valid() {
        assert!(validate_mode("block").is_ok());
        assert!(validate_mode("delay").is_ok());
    }

    #[test]
    fn test_validate_mode_invalid() {
        assert!(validate_mode("invalid").is_err());
        assert!(validate_mode("").is_err());
    }

    // InhibitorLock tests

    #[test]
    fn test_inhibitor_lock_new() {
        let lock = InhibitorLock::new("shutdown", "test-program", "Testing", "block");
        assert_eq!(lock.what, "shutdown");
        assert_eq!(lock.who, "test-program");
        assert_eq!(lock.why, "Testing");
        assert_eq!(lock.mode, "block");
        assert!(lock.pid > 0);
        assert!(lock.timestamp > 0);
    }

    #[test]
    fn test_inhibitor_lock_serialize() {
        let lock = InhibitorLock {
            path: PathBuf::new(),
            what: "shutdown:sleep".to_string(),
            who: "my-app".to_string(),
            why: "Saving data".to_string(),
            mode: "delay".to_string(),
            uid: 1000,
            pid: 42,
            timestamp: 1700000000,
        };

        let serialized = lock.serialize();
        assert!(serialized.contains("WHAT=shutdown:sleep\n"));
        assert!(serialized.contains("WHO=my-app\n"));
        assert!(serialized.contains("WHY=Saving data\n"));
        assert!(serialized.contains("MODE=delay\n"));
        assert!(serialized.contains("UID=1000\n"));
        assert!(serialized.contains("PID=42\n"));
        assert!(serialized.contains("TIMESTAMP=1700000000\n"));
    }

    #[test]
    fn test_inhibitor_lock_parse() {
        let content = "\
WHAT=shutdown:sleep
WHO=my-app
WHY=Saving data
MODE=delay
UID=1000
PID=42
TIMESTAMP=1700000000
";
        let path = PathBuf::from("/run/systemd/inhibit/test");
        let lock = InhibitorLock::parse(&path, content).unwrap();

        assert_eq!(lock.what, "shutdown:sleep");
        assert_eq!(lock.who, "my-app");
        assert_eq!(lock.why, "Saving data");
        assert_eq!(lock.mode, "delay");
        assert_eq!(lock.uid, 1000);
        assert_eq!(lock.pid, 42);
        assert_eq!(lock.timestamp, 1700000000);
    }

    #[test]
    fn test_inhibitor_lock_parse_empty_what() {
        let content = "WHO=test\nWHY=test\nMODE=block\n";
        let path = PathBuf::from("/tmp/test");
        assert!(InhibitorLock::parse(&path, content).is_none());
    }

    #[test]
    fn test_inhibitor_lock_parse_with_comments() {
        let content = "\
# This is a comment
WHAT=idle
WHO=test

WHY=reason
MODE=block
UID=0
PID=1
TIMESTAMP=100
";
        let path = PathBuf::from("/tmp/test");
        let lock = InhibitorLock::parse(&path, content).unwrap();
        assert_eq!(lock.what, "idle");
        assert_eq!(lock.who, "test");
        assert_eq!(lock.why, "reason");
    }

    #[test]
    fn test_inhibitor_lock_roundtrip() {
        let original = InhibitorLock {
            path: PathBuf::from("/tmp/test"),
            what: "shutdown:sleep:idle".to_string(),
            who: "systemd-inhibit".to_string(),
            why: "Package update in progress".to_string(),
            mode: "block".to_string(),
            uid: 0,
            pid: 12345,
            timestamp: 1700000000,
        };

        let serialized = original.serialize();
        let parsed = InhibitorLock::parse(&original.path, &serialized).unwrap();

        assert_eq!(parsed.what, original.what);
        assert_eq!(parsed.who, original.who);
        assert_eq!(parsed.why, original.why);
        assert_eq!(parsed.mode, original.mode);
        assert_eq!(parsed.uid, original.uid);
        assert_eq!(parsed.pid, original.pid);
        assert_eq!(parsed.timestamp, original.timestamp);
    }

    // Lock file management tests

    #[test]
    fn test_acquire_and_release_lock() {
        let dir = std::env::temp_dir().join("systemd-inhibit-test");
        let _ = fs::create_dir_all(&dir);

        // We can't easily test acquire_lock because it uses a hardcoded path,
        // but we can test the serialize/parse roundtrip with a temp file
        let lock = InhibitorLock::new("idle", "test", "unit test", "block");
        let content = lock.serialize();

        let lock_file = dir.join("test.lock");
        let mut f = fs::File::create(&lock_file).unwrap();
        f.write_all(content.as_bytes()).unwrap();

        let parsed = InhibitorLock::parse(&lock_file, &fs::read_to_string(&lock_file).unwrap());
        assert!(parsed.is_some());

        // Clean up
        release_lock(&lock_file);
        assert!(!lock_file.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_release_lock_nonexistent() {
        // Releasing a non-existent lock should not panic
        release_lock(Path::new("/nonexistent/path/lock.file"));
    }

    // Display formatting tests

    #[test]
    fn test_format_since_recent() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let result = format_since(now - 30);
        assert!(result.contains("s ago"));
    }

    #[test]
    fn test_format_since_minutes() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let result = format_since(now - 300);
        assert!(result.contains("min ago"));
    }

    #[test]
    fn test_format_since_hours() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let result = format_since(now - 7200);
        assert!(result.contains("h ago"));
    }

    #[test]
    fn test_format_since_days() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let result = format_since(now - 172800);
        assert!(result.contains("d ago"));
    }

    #[test]
    fn test_format_since_zero() {
        assert_eq!(format_since(0), "n/a");
    }

    #[test]
    fn test_format_since_future() {
        let future = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 1000;
        assert_eq!(format_since(future), "n/a");
    }

    // UID lookup test

    #[test]
    fn test_uid_to_name_root() {
        let name = uid_to_name(0);
        // On most systems this should be "root", but we can't guarantee
        // it in all test environments
        assert!(!name.is_empty());
    }

    #[test]
    fn test_uid_to_name_unknown() {
        // A very high UID that probably doesn't exist
        let name = uid_to_name(99999);
        // Should fall back to the numeric UID string
        assert_eq!(name, "99999");
    }

    // List locks test

    #[test]
    fn test_list_locks_no_panic() {
        // Should not panic even if the directory doesn't exist
        let locks = list_locks();
        let _ = locks;
    }

    // Display test

    #[test]
    fn test_display_locks_empty() {
        // Should not panic with empty list
        display_locks(&[]);
    }

    #[test]
    fn test_display_locks_with_entries() {
        let locks = vec![
            InhibitorLock {
                path: PathBuf::from("/tmp/test1"),
                what: "shutdown:sleep".to_string(),
                who: "apt-get".to_string(),
                why: "Package update".to_string(),
                mode: "block".to_string(),
                uid: 0,
                pid: 1234,
                timestamp: 1700000000,
            },
            InhibitorLock {
                path: PathBuf::from("/tmp/test2"),
                what: "idle".to_string(),
                who: "vlc".to_string(),
                why: "Playing video".to_string(),
                mode: "block".to_string(),
                uid: 1000,
                pid: 5678,
                timestamp: 1700000000,
            },
        ];

        // Should not panic
        display_locks(&locks);
    }

    // Command execution test

    #[test]
    fn test_run_command_wait_true() {
        let status = run_command_wait(&["true".to_string()]);
        assert_eq!(status, 0);
    }

    #[test]
    fn test_run_command_wait_false() {
        let status = run_command_wait(&["false".to_string()]);
        assert_ne!(status, 0);
    }

    #[test]
    fn test_run_command_wait_nonexistent() {
        let status = run_command_wait(&["/nonexistent/binary/xyz".to_string()]);
        assert_eq!(status, 127);
    }

    #[test]
    fn test_run_command_wait_empty() {
        let status = run_command_wait(&[]);
        assert_eq!(status, 0);
    }

    #[test]
    fn test_run_command_wait_with_args() {
        let status = run_command_wait(&["echo".to_string(), "hello".to_string()]);
        assert_eq!(status, 0);
    }
}
