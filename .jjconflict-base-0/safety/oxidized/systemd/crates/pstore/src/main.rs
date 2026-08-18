//! systemd-pstore — Archive platform-specific persistent storage.
//!
//! A drop-in replacement for `systemd-pstore(8)`. This tool is invoked
//! by `systemd-pstore.service` during early boot to archive entries from
//! the kernel's pstore filesystem (`/sys/fs/pstore/`) into persistent
//! storage at `/var/lib/systemd/pstore/`.
//!
//! The pstore filesystem is a kernel facility that saves crash dumps,
//! console logs, and other diagnostic data across reboots using platform-
//! specific backends (EFI variables, RAM, etc.). This tool:
//!
//!   1. Reads all entries from `/sys/fs/pstore/`
//!   2. Copies them into a timestamped subdirectory under
//!      `/var/lib/systemd/pstore/`
//!   3. Optionally removes the originals from pstore (controlled by
//!      `pstore.conf`)
//!
//! Configuration is read from `/etc/systemd/pstore.conf` and drop-in
//! directories `/etc/systemd/pstore.conf.d/`, `/run/systemd/pstore.conf.d/`,
//! and `/usr/lib/systemd/pstore.conf.d/`.
//!
//! Exit codes:
//!   0 — success (entries archived, or no entries to archive)
//!   1 — error

use std::fs;
use std::io;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::process;

const PSTORE_SOURCE: &str = "/sys/fs/pstore";
const PSTORE_ARCHIVE: &str = "/var/lib/systemd/pstore";

/// Configuration for pstore archival.
#[derive(Debug, Clone)]
struct Config {
    /// Whether to archive pstore entries at all.
    storage: Storage,
    /// Whether to remove entries from pstore after archiving.
    unlink: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Storage {
    /// Archive entries to /var/lib/systemd/pstore/ (default).
    External,
    /// Record entries in the journal (not yet implemented, treated as external).
    Journal,
    /// Do nothing.
    None,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            storage: Storage::External,
            unlink: true,
        }
    }
}

/// Parse a pstore.conf file and update the config accordingly.
fn parse_config_file(path: &Path, config: &mut Config) {
    let contents = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut in_pstore_section = false;

    for line in contents.lines() {
        let line = line.trim();

        // Skip comments and empty lines.
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        // Section header.
        if line.starts_with('[') && line.ends_with(']') {
            in_pstore_section = line.eq_ignore_ascii_case("[pstore]");
            continue;
        }

        if !in_pstore_section {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();

            match key {
                "Storage" => match value.to_lowercase().as_str() {
                    "external" => config.storage = Storage::External,
                    "journal" => config.storage = Storage::Journal,
                    "none" => config.storage = Storage::None,
                    other => {
                        eprintln!("Warning: unknown Storage value '{}', using default", other);
                    }
                },
                "Unlink" => match value.to_lowercase().as_str() {
                    "yes" | "true" | "1" | "on" => config.unlink = true,
                    "no" | "false" | "0" | "off" => config.unlink = false,
                    other => {
                        eprintln!("Warning: unknown Unlink value '{}', using default", other);
                    }
                },
                _ => {
                    // Ignore unknown keys.
                }
            }
        }
    }
}

/// Load configuration from all pstore.conf locations.
fn load_config() -> Config {
    let mut config = Config::default();

    // Main config files (in order of increasing priority).
    let main_paths = ["/usr/lib/systemd/pstore.conf", "/etc/systemd/pstore.conf"];

    for path in &main_paths {
        parse_config_file(Path::new(path), &mut config);
    }

    // Drop-in directories (in order of increasing priority).
    let dropin_dirs = [
        "/usr/lib/systemd/pstore.conf.d",
        "/run/systemd/pstore.conf.d",
        "/etc/systemd/pstore.conf.d",
    ];

    for dir in &dropin_dirs {
        let dir_path = Path::new(dir);
        if let Ok(entries) = fs::read_dir(dir_path) {
            let mut files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "conf"))
                .collect();
            files.sort();

            for file in files {
                parse_config_file(&file, &mut config);
            }
        }
    }

    config
}

/// Log a processed pstore file to the journal: a MESSAGE plus, when the file
/// has content, a binary `FILE=` field carrying the raw bytes. The native
/// journal protocol's length-prefixed binary field form is used so multi-line
/// content round-trips — a plain `KEY=value\n` line cannot carry embedded
/// newlines. Mirrors systemd-pstore's `sd_journal_sendv()` (src/pstore/pstore.c).
fn journal_log_pstore(message: &str, content: &[u8]) {
    let sock = match UnixDatagram::unbound() {
        Ok(s) => s,
        Err(_) => return,
    };
    // Leave the socket blocking: systemd-pstore is a short-lived helper (not
    // PID 1), so it must wait for journald to drain rather than dropping a
    // datagram when the burst of per-file messages fills the socket buffer —
    // otherwise a reconstructed dmesg loses a chunk in the FILE= stream.

    let mut payload: Vec<u8> = Vec::new();
    payload.extend_from_slice(format!("MESSAGE={message}\n").as_bytes());
    payload.extend_from_slice(b"PRIORITY=6\n");
    payload.extend_from_slice(b"SYSLOG_IDENTIFIER=systemd-pstore\n");
    if !content.is_empty() {
        // Binary field: `FILE\n` + 8-byte little-endian length + bytes + `\n`.
        payload.extend_from_slice(b"FILE\n");
        payload.extend_from_slice(&(content.len() as u64).to_le_bytes());
        payload.extend_from_slice(content);
        payload.push(b'\n');
    }

    let _ = sock.send_to(&payload, "/run/systemd/journal/socket");
}

/// Build `archive_base[/subdir1[/subdir2]]`.
fn archive_subdir(archive_base: &Path, subdir1: Option<&str>, subdir2: Option<&str>) -> PathBuf {
    let mut dir = archive_base.to_path_buf();
    if let Some(s1) = subdir1 {
        dir.push(s1);
    }
    if let Some(s2) = subdir2 {
        dir.push(s2);
    }
    dir
}

/// Log a pstore file to the journal (always) and, when Storage=external, copy
/// it into the archive at `archive_base[/subdir1[/subdir2]]/name`. Removes the
/// original from pstore when `unlink` is set. Mirrors upstream `move_file()`.
#[allow(clippy::too_many_arguments)]
fn move_file(
    source: &Path,
    archive_base: &Path,
    name: &str,
    content: &[u8],
    subdir1: Option<&str>,
    subdir2: Option<&str>,
    external: bool,
    unlink: bool,
) {
    let dir = archive_subdir(archive_base, subdir1, subdir2);
    let dst_file = dir.join(name);

    // Always log to the journal, carrying the file content in FILE=.
    let message = if external {
        format!("PStore {} moved to {}", name, dst_file.display())
    } else {
        format!("PStore {}.", name)
    };
    journal_log_pstore(&message, content);

    if external {
        let _ = fs::create_dir_all(&dir);
        if let Err(e) = fs::write(&dst_file, content) {
            eprintln!("Warning: failed to archive {}: {}", name, e);
            return;
        }
        eprintln!("Archived {} -> {}", name, dst_file.display());
    }

    if unlink {
        let _ = fs::remove_file(source.join(name));
    }
}

/// Append a dmesg chunk's content to the reconstructed `dmesg.txt` under
/// `archive_base[/subdir1[/subdir2]]` (external storage only). Mirrors upstream
/// `append_dmesg()`.
fn append_dmesg(archive_base: &Path, subdir1: Option<&str>, subdir2: Option<&str>, content: &[u8]) {
    if content.is_empty() {
        return;
    }
    let dir = archive_subdir(archive_base, subdir1, subdir2);
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("dmesg.txt");
    use std::io::Write;
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(content);
    }
}

/// List entries in the pstore source directory.
fn list_pstore_entries(source: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(source)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .collect();

    // Sort by name for deterministic ordering.
    entries.sort_by_key(|e| e.file_name());
    Ok(entries)
}

/// Archive pstore entries. dmesg chunks (EFI/ERST backends) are reconstructed
/// into a single `dmesg.txt` under a per-record subdirectory, and every file is
/// logged to the journal with its content in a `FILE=` field. Mirrors upstream
/// systemd-pstore's `process_dmesg_files()` + `move_file()` (src/pstore/pstore.c).
fn archive_entries(source: &Path, archive_base: &Path, config: &Config) -> io::Result<usize> {
    let entries = match list_pstore_entries(source) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            eprintln!(
                "pstore directory {} does not exist, nothing to archive.",
                source.display()
            );
            return Ok(0);
        }
        Err(e) => return Err(e),
    };

    if entries.is_empty() {
        eprintln!("No pstore entries to archive.");
        return Ok(0);
    }

    let external = config.storage == Storage::External;

    // Read each entry's name and content up front. list_pstore_entries returns
    // them sorted by name; dmesg chunks are processed in reverse order so the
    // parts append into the original dmesg order.
    let mut items: Vec<(String, Vec<u8>, bool)> = entries
        .iter()
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let content = fs::read(e.path()).unwrap_or_default();
            (name, content, false)
        })
        .collect();

    // Pass 1: reconstruct dmesg files (EFI and ERST backends).
    let mut last_record_id: u64 = 0;
    let mut erst_subdir: Option<String> = None;
    for i in (0..items.len()).rev() {
        if items[i].2 {
            continue;
        }
        let name = items[i].0.clone();
        // `.enc.z` indicates an encrypted/compressed record we can't reassemble.
        if name.ends_with(".enc.z") || !name.starts_with("dmesg-") {
            continue;
        }
        let content = items[i].1.clone();

        if let Some(rid) = name
            .strip_prefix("dmesg-efi-")
            .or_else(|| name.strip_prefix("dmesg-efi_pstore-"))
        {
            // EFI record id: <timestamp><part(2)><count(3)>. Group by the base
            // record id (timestamp) and the count; the 2-digit part is dropped.
            if rid.len() < 6 {
                continue;
            }
            let s1 = rid[..rid.len() - 5].to_owned();
            let s2 = rid[rid.len() - 3..].to_owned();
            move_file(
                source,
                archive_base,
                &name,
                &content,
                Some(&s1),
                Some(&s2),
                external,
                config.unlink,
            );
            if external {
                append_dmesg(archive_base, Some(&s1), Some(&s2), &content);
            }
            items[i].2 = true;
        } else if let Some(rid) = name.strip_prefix("dmesg-erst-") {
            // ERST record id is a monotonically increasing number. A break in
            // the sequence starts a new group whose subdir is that record id.
            let Ok(record_id) = rid.parse::<u64>() else {
                continue;
            };
            if last_record_id.wrapping_sub(1) != record_id {
                erst_subdir = Some(rid.to_owned());
            }
            move_file(
                source,
                archive_base,
                &name,
                &content,
                erst_subdir.as_deref(),
                None,
                external,
                config.unlink,
            );
            if external {
                append_dmesg(archive_base, erst_subdir.as_deref(), None, &content);
            }
            last_record_id = record_id;
            items[i].2 = true;
        }
    }

    // Pass 2: any remaining (non-dmesg) files go straight to the archive root.
    for item in &mut items {
        if item.2 {
            continue;
        }
        move_file(
            source,
            archive_base,
            &item.0,
            &item.1,
            None,
            None,
            external,
            config.unlink,
        );
        item.2 = true;
    }

    Ok(items.iter().filter(|it| it.2).count())
}

fn run(source: &Path, archive: &Path) -> i32 {
    let config = load_config();

    if config.storage == Storage::None {
        eprintln!("pstore archival is disabled (Storage=none).");
        return 0;
    }

    // Ensure the source pstore directory exists.
    if !source.exists() {
        eprintln!(
            "pstore directory {} does not exist, nothing to do.",
            source.display()
        );
        return 0;
    }

    // Check if there are any entries at all.
    match list_pstore_entries(source) {
        Ok(entries) if entries.is_empty() => {
            eprintln!("No pstore entries found.");
            return 0;
        }
        Err(e) => {
            eprintln!("Error reading {}: {}", source.display(), e);
            return 1;
        }
        _ => {}
    }

    // Ensure the archive base directory exists.
    if let Err(e) = fs::create_dir_all(archive) {
        eprintln!(
            "Error: failed to create archive directory {}: {}",
            archive.display(),
            e
        );
        return 1;
    }

    match archive_entries(source, archive, &config) {
        Ok(count) => {
            if count > 0 {
                eprintln!(
                    "Archived {} pstore entries to {}.",
                    count,
                    archive.display()
                );
            }
            0
        }
        Err(e) => {
            eprintln!("Error archiving pstore entries: {}", e);
            1
        }
    }
}

fn main() {
    let source = Path::new(PSTORE_SOURCE);
    let archive = Path::new(PSTORE_ARCHIVE);

    let code = run(source, archive);
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
        assert_eq!(PSTORE_SOURCE, "/sys/fs/pstore");
        assert_eq!(PSTORE_ARCHIVE, "/var/lib/systemd/pstore");
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.storage, Storage::External);
        assert!(config.unlink);
    }

    #[test]
    fn test_parse_config_storage_none() {
        let dir = temp_dir();
        let conf = dir.path().join("pstore.conf");
        fs::write(&conf, "[PStore]\nStorage=none\n").unwrap();

        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::None);
    }

    #[test]
    fn test_parse_config_storage_external() {
        let dir = temp_dir();
        let conf = dir.path().join("pstore.conf");
        fs::write(&conf, "[PStore]\nStorage=external\n").unwrap();

        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::External);
    }

    #[test]
    fn test_parse_config_storage_journal() {
        let dir = temp_dir();
        let conf = dir.path().join("pstore.conf");
        fs::write(&conf, "[PStore]\nStorage=journal\n").unwrap();

        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::Journal);
    }

    #[test]
    fn test_parse_config_unlink_no() {
        let dir = temp_dir();
        let conf = dir.path().join("pstore.conf");
        fs::write(&conf, "[PStore]\nUnlink=no\n").unwrap();

        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert!(!config.unlink);
    }

    #[test]
    fn test_parse_config_unlink_yes() {
        let dir = temp_dir();
        let conf = dir.path().join("pstore.conf");
        fs::write(&conf, "[PStore]\nUnlink=yes\n").unwrap();

        let mut config = Config {
            unlink: false,
            ..Default::default()
        };
        parse_config_file(&conf, &mut config);
        assert!(config.unlink);
    }

    #[test]
    fn test_parse_config_missing_file() {
        let mut config = Config::default();
        parse_config_file(Path::new("/nonexistent/pstore.conf"), &mut config);
        // Should not change defaults.
        assert_eq!(config.storage, Storage::External);
        assert!(config.unlink);
    }

    #[test]
    fn test_parse_config_wrong_section_ignored() {
        let dir = temp_dir();
        let conf = dir.path().join("pstore.conf");
        fs::write(&conf, "[Other]\nStorage=none\nUnlink=no\n").unwrap();

        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        // Should not have changed since it's the wrong section.
        assert_eq!(config.storage, Storage::External);
        assert!(config.unlink);
    }

    #[test]
    fn test_parse_config_comments_and_blanks() {
        let dir = temp_dir();
        let conf = dir.path().join("pstore.conf");
        fs::write(
            &conf,
            "# comment\n\n; another comment\n[PStore]\n# inline\nStorage=none\n",
        )
        .unwrap();

        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::None);
    }

    #[test]
    fn test_list_pstore_entries_empty_dir() {
        let dir = temp_dir();
        let entries = list_pstore_entries(dir.path()).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_list_pstore_entries_with_files() {
        let dir = temp_dir();
        fs::write(dir.path().join("dmesg-ramoops-0"), "crash log 1").unwrap();
        fs::write(dir.path().join("dmesg-ramoops-1"), "crash log 2").unwrap();
        fs::write(dir.path().join("console-ramoops-0"), "console log").unwrap();

        let entries = list_pstore_entries(dir.path()).unwrap();
        assert_eq!(entries.len(), 3);

        // Should be sorted.
        let names: Vec<String> = entries
            .iter()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["console-ramoops-0", "dmesg-ramoops-0", "dmesg-ramoops-1"]
        );
    }

    #[test]
    fn test_list_pstore_entries_skips_directories() {
        let dir = temp_dir();
        fs::write(dir.path().join("dmesg-ramoops-0"), "crash log").unwrap();
        fs::create_dir(dir.path().join("subdir")).unwrap();

        let entries = list_pstore_entries(dir.path()).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_list_pstore_entries_nonexistent() {
        let result = list_pstore_entries(Path::new("/nonexistent/pstore"));
        assert!(result.is_err());
    }

    #[test]
    fn test_archive_entries_basic() {
        let dir = temp_dir();
        let source = dir.path().join("pstore");
        let archive = dir.path().join("archive");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&archive).unwrap();

        fs::write(source.join("dmesg-ramoops-0"), "crash data").unwrap();
        fs::write(source.join("console-ramoops-0"), "console data").unwrap();

        let config = Config {
            storage: Storage::External,
            unlink: false,
        };

        let count = archive_entries(&source, &archive, &config).unwrap();
        assert_eq!(count, 2);

        // Non-EFI/ERST backends aren't reconstructed: each file is archived
        // directly under the archive root (matching upstream move_file()).
        let archived_files: Vec<String> = fs::read_dir(&archive)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(archived_files.len(), 2);
        assert!(archive.join("dmesg-ramoops-0").is_file());
        assert!(archive.join("console-ramoops-0").is_file());

        // Source files should still exist (unlink=false).
        assert!(source.join("dmesg-ramoops-0").exists());
        assert!(source.join("console-ramoops-0").exists());
    }

    #[test]
    fn test_archive_entries_with_unlink() {
        let dir = temp_dir();
        let source = dir.path().join("pstore");
        let archive = dir.path().join("archive");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&archive).unwrap();

        fs::write(source.join("dmesg-ramoops-0"), "crash data").unwrap();

        let config = Config {
            storage: Storage::External,
            unlink: true,
        };

        let count = archive_entries(&source, &archive, &config).unwrap();
        assert_eq!(count, 1);

        // Source file should have been removed.
        assert!(!source.join("dmesg-ramoops-0").exists());
    }

    #[test]
    fn test_archive_entries_empty_source() {
        let dir = temp_dir();
        let source = dir.path().join("pstore");
        let archive = dir.path().join("archive");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&archive).unwrap();

        let config = Config::default();
        let count = archive_entries(&source, &archive, &config).unwrap();
        assert_eq!(count, 0);

        // No subdirectory should have been created.
        let subdirs: Vec<_> = fs::read_dir(&archive)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(subdirs.is_empty());
    }

    #[test]
    fn test_archive_entries_nonexistent_source() {
        let dir = temp_dir();
        let source = dir.path().join("nonexistent");
        let archive = dir.path().join("archive");
        fs::create_dir(&archive).unwrap();

        let config = Config::default();
        let count = archive_entries(&source, &archive, &config).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_archive_preserves_content() {
        let dir = temp_dir();
        let source = dir.path().join("pstore");
        let archive = dir.path().join("archive");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&archive).unwrap();

        let crash_data = "BUG: kernel NULL pointer dereference at 0x0000042";
        fs::write(source.join("dmesg-ramoops-0"), crash_data).unwrap();

        let config = Config {
            storage: Storage::External,
            unlink: false,
        };

        archive_entries(&source, &archive, &config).unwrap();

        // Non-EFI/ERST files are archived directly under the archive root.
        let archived = fs::read_to_string(archive.join("dmesg-ramoops-0")).unwrap();
        assert_eq!(archived, crash_data);
    }

    #[test]
    fn test_run_no_source_dir() {
        let dir = temp_dir();
        let source = dir.path().join("no_pstore");
        let archive = dir.path().join("archive");

        let code = run(&source, &archive);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_run_empty_source() {
        let dir = temp_dir();
        let source = dir.path().join("pstore");
        let archive = dir.path().join("archive");
        fs::create_dir(&source).unwrap();

        let code = run(&source, &archive);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_run_with_entries() {
        let dir = temp_dir();
        let source = dir.path().join("pstore");
        let archive = dir.path().join("archive");
        fs::create_dir(&source).unwrap();

        fs::write(source.join("dmesg-ramoops-0"), "kernel panic data").unwrap();

        let code = run(&source, &archive);
        assert_eq!(code, 0);
        assert!(archive.exists());
    }

    #[test]
    fn test_parse_config_case_insensitive_section() {
        let dir = temp_dir();
        let conf = dir.path().join("pstore.conf");
        fs::write(&conf, "[pstore]\nStorage=none\n").unwrap();

        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::None);
    }

    #[test]
    fn test_parse_config_multiple_values_last_wins() {
        let dir = temp_dir();
        let conf = dir.path().join("pstore.conf");
        fs::write(
            &conf,
            "[PStore]\nStorage=none\nStorage=external\nUnlink=yes\nUnlink=no\n",
        )
        .unwrap();

        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::External);
        assert!(!config.unlink);
    }

    #[test]
    fn test_parse_config_whitespace_handling() {
        let dir = temp_dir();
        let conf = dir.path().join("pstore.conf");
        fs::write(&conf, "[PStore]\n  Storage = none  \n  Unlink = no  \n").unwrap();

        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::None);
        assert!(!config.unlink);
    }

    #[test]
    fn test_parse_config_unlink_boolean_variants() {
        let dir = temp_dir();

        for (value, expected) in &[
            ("true", true),
            ("false", false),
            ("1", true),
            ("0", false),
            ("on", true),
            ("off", false),
            ("yes", true),
            ("no", false),
        ] {
            let conf = dir.path().join(format!("pstore-{}.conf", value));
            fs::write(&conf, format!("[PStore]\nUnlink={}\n", value)).unwrap();

            let mut config = Config {
                unlink: !expected, // Set to opposite to verify it changes.
                ..Default::default()
            };
            parse_config_file(&conf, &mut config);
            assert_eq!(config.unlink, *expected, "Failed for Unlink={}", value);
        }
    }
}
