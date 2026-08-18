//! systemd-update-done — Mark /etc and /var as updated.
//!
//! A drop-in replacement for `systemd-update-done(8)`. This tool is invoked
//! by `systemd-update-done.service` during early boot to create or update
//! stamp files that record when the OS was last updated:
//!
//!   /etc/.updated
//!   /var/.updated
//!
//! These stamp files are used by the `ConditionNeedsUpdate=` directive in
//! unit files. When the OS image (usr partition) is newer than these stamps,
//! units with `ConditionNeedsUpdate=/etc` or `ConditionNeedsUpdate=/var`
//! will be activated to perform necessary migration/update steps.
//!
//! The tool compares the modification time of `/usr/` against the stamp
//! files. If `/usr/` is newer (or the stamp doesn't exist), the stamp
//! file is created/updated with the current timestamp.
//!
//! Exit codes:
//!   0 — success (stamps updated or already up-to-date)
//!   1 — error

use std::fs;
use std::io;
use std::path::Path;
use std::process;
use std::time::SystemTime;

const USR_PATH: &str = "/usr";
const ETC_UPDATED: &str = "/etc/.updated";
const VAR_UPDATED: &str = "/var/.updated";

/// Get the modification time of a path, if it exists.
fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

/// Touch a stamp file: create it (or truncate it) and set its mtime to now.
fn touch_stamp(path: &Path) -> io::Result<()> {
    // Create or truncate the file
    fs::File::create(path)?;
    // The mtime is automatically set to "now" by the filesystem on create.
    Ok(())
}

/// Check whether a stamp file needs updating by comparing it against /usr.
///
/// Returns `true` if the stamp should be updated (i.e., /usr is newer than
/// the stamp, or the stamp doesn't exist).
fn needs_update(usr_mtime: Option<SystemTime>, stamp_path: &Path) -> bool {
    let usr_t = match usr_mtime {
        Some(t) => t,
        None => {
            // Can't stat /usr — nothing to compare against, skip.
            return false;
        }
    };

    match mtime(stamp_path) {
        Some(stamp_t) => {
            // Stamp exists: update if /usr is strictly newer.
            usr_t > stamp_t
        }
        None => {
            // Stamp doesn't exist: needs update.
            true
        }
    }
}

fn run(usr_path: &Path, etc_stamp: &Path, var_stamp: &Path) -> i32 {
    let usr_mtime = mtime(usr_path);

    let mut errors = 0;

    // Process /etc/.updated
    if needs_update(usr_mtime, etc_stamp) {
        match touch_stamp(etc_stamp) {
            Ok(()) => {
                eprintln!("Marked {} as updated.", etc_stamp.display());
            }
            Err(e) => {
                // /etc might be read-only (e.g. NixOS with an immutable /etc).
                // This is not fatal — many NixOS systems have a read-only /etc
                // and the stamp file is not strictly required.
                eprintln!(
                    "Notice: could not update {}: {} (continuing)",
                    etc_stamp.display(),
                    e
                );
            }
        }
    } else {
        eprintln!("{} is up-to-date.", etc_stamp.display());
    }

    // Process /var/.updated
    if needs_update(usr_mtime, var_stamp) {
        match touch_stamp(var_stamp) {
            Ok(()) => {
                eprintln!("Marked {} as updated.", var_stamp.display());
            }
            Err(e) => {
                eprintln!("Error: could not update {}: {}", var_stamp.display(), e);
                errors += 1;
            }
        }
    } else {
        eprintln!("{} is up-to-date.", var_stamp.display());
    }

    if errors > 0 { 1 } else { 0 }
}

fn main() {
    let usr = Path::new(USR_PATH);
    let etc_stamp = Path::new(ETC_UPDATED);
    let var_stamp = Path::new(VAR_UPDATED);

    let code = run(usr, etc_stamp, var_stamp);
    process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread;
    use std::time::Duration;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("failed to create temp dir")
    }

    #[test]
    fn test_needs_update_no_stamp() {
        let dir = temp_dir();
        let usr = dir.path().join("usr");
        fs::create_dir(&usr).unwrap();
        let stamp = dir.path().join(".updated");

        let usr_mtime = mtime(&usr);
        assert!(needs_update(usr_mtime, &stamp));
    }

    #[test]
    fn test_needs_update_stamp_older() {
        let dir = temp_dir();
        let stamp = dir.path().join(".updated");
        fs::write(&stamp, "").unwrap();

        // Wait a moment, then create /usr so its mtime is newer.
        thread::sleep(Duration::from_millis(50));
        let usr = dir.path().join("usr");
        fs::create_dir(&usr).unwrap();

        let usr_mtime = mtime(&usr);
        assert!(needs_update(usr_mtime, &stamp));
    }

    #[test]
    fn test_needs_update_stamp_newer() {
        let dir = temp_dir();
        let usr = dir.path().join("usr");
        fs::create_dir(&usr).unwrap();

        // Wait a moment, then create the stamp so its mtime is newer.
        thread::sleep(Duration::from_millis(50));
        let stamp = dir.path().join(".updated");
        fs::write(&stamp, "").unwrap();

        let usr_mtime = mtime(&usr);
        assert!(!needs_update(usr_mtime, &stamp));
    }

    #[test]
    fn test_needs_update_no_usr() {
        let dir = temp_dir();
        let stamp = dir.path().join(".updated");

        // /usr doesn't exist → needs_update should return false (nothing to do).
        assert!(!needs_update(None, &stamp));
    }

    #[test]
    fn test_touch_stamp_creates_file() {
        let dir = temp_dir();
        let stamp = dir.path().join(".updated");
        assert!(!stamp.exists());

        touch_stamp(&stamp).unwrap();
        assert!(stamp.exists());
    }

    #[test]
    fn test_touch_stamp_truncates_existing() {
        let dir = temp_dir();
        let stamp = dir.path().join(".updated");
        fs::write(&stamp, "old content").unwrap();

        touch_stamp(&stamp).unwrap();
        let contents = fs::read_to_string(&stamp).unwrap();
        assert!(contents.is_empty());
    }

    #[test]
    fn test_mtime_existing_file() {
        let dir = temp_dir();
        let file = dir.path().join("test");
        fs::write(&file, "hello").unwrap();

        let t = mtime(&file);
        assert!(t.is_some());
    }

    #[test]
    fn test_mtime_nonexistent() {
        let dir = temp_dir();
        let file = dir.path().join("nonexistent");
        assert!(mtime(&file).is_none());
    }

    #[test]
    fn test_run_creates_stamps() {
        let dir = temp_dir();
        let usr = dir.path().join("usr");
        fs::create_dir(&usr).unwrap();

        let etc_stamp = dir.path().join("etc_updated");
        let var_stamp = dir.path().join("var_updated");

        let code = run(&usr, &etc_stamp, &var_stamp);
        assert_eq!(code, 0);
        assert!(etc_stamp.exists());
        assert!(var_stamp.exists());
    }

    #[test]
    fn test_run_already_up_to_date() {
        let dir = temp_dir();
        let usr = dir.path().join("usr");
        fs::create_dir(&usr).unwrap();

        // Wait, then create stamps that are newer than /usr.
        thread::sleep(Duration::from_millis(50));
        let etc_stamp = dir.path().join("etc_updated");
        let var_stamp = dir.path().join("var_updated");
        fs::write(&etc_stamp, "").unwrap();
        fs::write(&var_stamp, "").unwrap();

        let code = run(&usr, &etc_stamp, &var_stamp);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_run_no_usr() {
        let dir = temp_dir();
        let usr = dir.path().join("nonexistent_usr");
        let etc_stamp = dir.path().join("etc_updated");
        let var_stamp = dir.path().join("var_updated");

        let code = run(&usr, &etc_stamp, &var_stamp);
        assert_eq!(code, 0);
        // Stamps should not be created since /usr doesn't exist.
        assert!(!etc_stamp.exists());
        assert!(!var_stamp.exists());
    }

    #[test]
    fn test_constants() {
        assert_eq!(USR_PATH, "/usr");
        assert_eq!(ETC_UPDATED, "/etc/.updated");
        assert_eq!(VAR_UPDATED, "/var/.updated");
    }

    #[test]
    fn test_touch_stamp_updates_mtime() {
        let dir = temp_dir();
        let stamp = dir.path().join(".updated");
        fs::write(&stamp, "").unwrap();
        let t1 = mtime(&stamp).unwrap();

        thread::sleep(Duration::from_millis(50));
        touch_stamp(&stamp).unwrap();
        let t2 = mtime(&stamp).unwrap();

        assert!(t2 >= t1);
    }

    #[test]
    fn test_run_updates_old_stamps() {
        let dir = temp_dir();

        // Create old stamps first.
        let etc_stamp = dir.path().join("etc_updated");
        let var_stamp = dir.path().join("var_updated");
        fs::write(&etc_stamp, "").unwrap();
        fs::write(&var_stamp, "").unwrap();

        // Wait, then create /usr so it's newer.
        thread::sleep(Duration::from_millis(50));
        let usr = dir.path().join("usr");
        fs::create_dir(&usr).unwrap();

        let code = run(&usr, &etc_stamp, &var_stamp);
        assert_eq!(code, 0);

        // Stamps should have been updated to be newer than /usr now.
        let usr_t = mtime(&usr).unwrap();
        let etc_t = mtime(&etc_stamp).unwrap();
        let var_t = mtime(&var_stamp).unwrap();
        assert!(etc_t >= usr_t);
        assert!(var_t >= usr_t);
    }
}
