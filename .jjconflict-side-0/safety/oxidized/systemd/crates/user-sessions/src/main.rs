//! systemd-user-sessions — Permit or deny user sessions during boot/shutdown.
//!
//! A drop-in replacement for `systemd-user-sessions(8)`. This tiny helper
//! is invoked by the `systemd-user-sessions.service` unit:
//!
//!   ExecStart=systemd-user-sessions start   → removes /run/nologin (permit logins)
//!   ExecStop=systemd-user-sessions stop     → creates /run/nologin (deny logins)
//!
//! While `/run/nologin` exists, PAM's `pam_nologin` module blocks non-root
//! logins. During early boot the file is present (written by the initrd or
//! PID 1), and this service removes it once the system is sufficiently
//! initialized. On shutdown the file is recreated to prevent new logins
//! while services are being torn down.
//!
//! Exit codes:
//!   0 — success
//!   1 — error (but non-fatal; the service should not be considered failed
//!       just because the nologin file was already absent/present)

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process;

const NOLOGIN_PATH: &str = "/run/nologin";
const NOLOGIN_MESSAGE: &str = "System is going down.";

fn start(nologin: &Path) {
    // Remove /run/nologin to permit user logins.
    match fs::remove_file(nologin) {
        Ok(()) => {
            eprintln!("Removed {}", nologin.display());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Already absent — nothing to do, this is fine.
            eprintln!("{} already absent, nothing to do", nologin.display());
        }
        Err(e) => {
            eprintln!("Warning: failed to remove {}: {}", nologin.display(), e);
            // Non-fatal: we still exit 0 to avoid blocking boot.
        }
    }
}

fn stop(nologin: &Path) {
    // Create /run/nologin to deny further user logins.
    match fs::File::create(nologin) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(NOLOGIN_MESSAGE.as_bytes()) {
                eprintln!("Warning: failed to write to {}: {}", nologin.display(), e);
            } else {
                eprintln!("Created {}", nologin.display());
            }
        }
        Err(e) => {
            eprintln!("Warning: failed to create {}: {}", nologin.display(), e);
            // Non-fatal during shutdown.
        }
    }
}

fn usage() -> ! {
    eprintln!("Usage: systemd-user-sessions {{start|stop}}");
    eprintln!();
    eprintln!("  start  Remove /run/nologin to permit user logins");
    eprintln!("  stop   Create /run/nologin to deny user logins");
    process::exit(1);
}

fn run(args: &[String], nologin: &Path) -> i32 {
    // Skip argv[0], look for the command.
    let command = match args.get(1) {
        Some(cmd) => cmd.as_str(),
        None => {
            usage();
        }
    };

    match command {
        "start" => {
            start(nologin);
            0
        }
        "stop" => {
            stop(nologin);
            0
        }
        other => {
            eprintln!("Unknown command: {}", other);
            usage();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let nologin = Path::new(NOLOGIN_PATH);
    let code = run(&args, nologin);
    process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: create a temporary directory and return a nologin path inside it.
    fn temp_nologin() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("nologin");
        (dir, path)
    }

    #[test]
    fn test_start_removes_nologin() {
        let (_dir, path) = temp_nologin();
        fs::write(&path, "System is going down.").unwrap();
        assert!(path.exists());

        start(&path);
        assert!(!path.exists());
    }

    #[test]
    fn test_start_absent_is_ok() {
        let (_dir, path) = temp_nologin();
        assert!(!path.exists());

        // Should not panic or fail.
        start(&path);
        assert!(!path.exists());
    }

    #[test]
    fn test_stop_creates_nologin() {
        let (_dir, path) = temp_nologin();
        assert!(!path.exists());

        stop(&path);
        assert!(path.exists());

        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents, NOLOGIN_MESSAGE);
    }

    #[test]
    fn test_stop_overwrites_existing() {
        let (_dir, path) = temp_nologin();
        fs::write(&path, "old content").unwrap();

        stop(&path);
        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents, NOLOGIN_MESSAGE);
    }

    #[test]
    fn test_start_then_stop_roundtrip() {
        let (_dir, path) = temp_nologin();
        fs::write(&path, NOLOGIN_MESSAGE).unwrap();

        start(&path);
        assert!(!path.exists());

        stop(&path);
        assert!(path.exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), NOLOGIN_MESSAGE);
    }

    #[test]
    fn test_run_start_command() {
        let (_dir, path) = temp_nologin();
        fs::write(&path, "block").unwrap();

        let args = vec!["systemd-user-sessions".into(), "start".into()];
        let code = run(&args, &path);
        assert_eq!(code, 0);
        assert!(!path.exists());
    }

    #[test]
    fn test_run_stop_command() {
        let (_dir, path) = temp_nologin();

        let args = vec!["systemd-user-sessions".into(), "stop".into()];
        let code = run(&args, &path);
        assert_eq!(code, 0);
        assert!(path.exists());
    }

    #[test]
    fn test_nologin_message_content() {
        assert_eq!(NOLOGIN_MESSAGE, "System is going down.");
    }

    #[test]
    fn test_nologin_path_constant() {
        assert_eq!(NOLOGIN_PATH, "/run/nologin");
    }

    #[test]
    fn test_multiple_starts_idempotent() {
        let (_dir, path) = temp_nologin();
        fs::write(&path, "block").unwrap();

        start(&path);
        assert!(!path.exists());

        start(&path);
        assert!(!path.exists());
    }

    #[test]
    fn test_multiple_stops_idempotent() {
        let (_dir, path) = temp_nologin();

        stop(&path);
        assert!(path.exists());
        let c1 = fs::read_to_string(&path).unwrap();

        stop(&path);
        assert!(path.exists());
        let c2 = fs::read_to_string(&path).unwrap();

        assert_eq!(c1, c2);
    }
}
