//! test-journal-append — Fuzz-test journal corruption resilience.
//!
//! A Rust equivalent of the C `test-journal-append` manual test from
//! upstream systemd.  Creates a journal file, writes initial entries,
//! then iteratively corrupts bytes at various offsets, reopens the
//! journal, and tries to append new entries.  Success means no crash
//! (write failures are acceptable).
//!
//! Supported options (matching the C version):
//!
//! - `--start-offset=OFFSET`  — byte offset to begin corruption (default: random)
//! - `--iterations=N`         — number of test iterations (default: 100)
//! - `--iteration-step=STEP`  — offset step between sequential iterations (default: 1)
//! - `--corrupt-step=STEP`    — byte interval for bit-flipping within a run (default: 31)
//! - `--sequential`           — use sequential offsets instead of random
//! - `--run-one=OFFSET`       — single-shot reproducer mode

use clap::Parser;
use libsystemd::journal::entry::JournalEntry;
use libsystemd::journal::storage::{JournalStorage, StorageConfig};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process;

#[derive(Parser, Debug)]
#[command(
    name = "test-journal-append",
    about = "Test journal corruption resilience by flipping bits and appending",
    version
)]
struct Cli {
    /// Byte offset at which to start corrupting the journal.
    /// Default: random offset (unless --sequential, then 0 + iteration).
    #[arg(long, value_name = "OFFSET")]
    start_offset: Option<u64>,

    /// Number of iterations to perform before exiting.
    #[arg(long, default_value = "100", value_name = "N")]
    iterations: u64,

    /// Iteration step for sequential mode.
    #[arg(long, default_value = "1", value_name = "STEP")]
    iteration_step: u64,

    /// Corrupt every N-th byte starting from the offset.
    #[arg(long, default_value = "31", value_name = "STEP")]
    corrupt_step: u64,

    /// Go through offsets sequentially instead of picking random ones.
    #[arg(long)]
    sequential: bool,

    /// Single-shot reproducer mode: run one iteration at the given offset.
    #[arg(long, value_name = "OFFSET")]
    run_one: Option<u64>,
}

/// Create a journal entry with a MESSAGE field.
fn make_entry(message: &str) -> JournalEntry {
    let mut entry = JournalEntry::new();
    entry
        .fields
        .insert("MESSAGE".to_string(), message.as_bytes().to_vec());
    entry.fields.insert("PRIORITY".to_string(), b"6".to_vec());
    entry
}

/// Simple pseudo-random number using /dev/urandom.
fn random_u64() -> u64 {
    let mut buf = [0u8; 8];
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut buf);
    }
    u64::from_le_bytes(buf)
}

/// Find all .journal files in a directory and return their paths.
fn find_journal_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "journal") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Run one corruption+append test iteration.
///
/// Returns Ok(true) if the offset exceeded the file size (caller should stop),
/// Ok(false) on normal completion, or Err on fatal error.
fn journal_corrupt_and_append(
    start_offset: Option<u64>,
    corrupt_step: u64,
) -> Result<bool, String> {
    // Create a temporary directory for this iteration
    let tempdir = std::env::temp_dir().join(format!("journal-append-{}", random_u64()));
    fs::create_dir_all(&tempdir).map_err(|e| format!("Failed to create tempdir: {e}"))?;

    let config = StorageConfig {
        directory: tempdir.clone(),
        max_file_size: u64::MAX,
        max_disk_usage: u64::MAX,
        max_files: usize::MAX,
        persistent: false,
        keep_free: 0,
        direct_directory: true,
    };

    // Create journal and write 10 initial messages
    {
        let mut storage =
            JournalStorage::new(config).map_err(|e| format!("Failed to open journal: {e}"))?;

        for i in 0..10 {
            let entry = make_entry(&format!("Initial message {i}"));
            if let Err(e) = storage.append(&entry) {
                let _ = fs::remove_dir_all(&tempdir);
                return Err(format!("Failed to write initial entry: {e}"));
            }
        }

        if let Err(e) = storage.flush() {
            let _ = fs::remove_dir_all(&tempdir);
            return Err(format!("Failed to flush journal: {e}"));
        }
    }

    // Find the journal file we just created
    let journal_files = find_journal_files(&tempdir);
    if journal_files.is_empty() {
        let _ = fs::remove_dir_all(&tempdir);
        return Err("No journal file found after writing".to_string());
    }
    let journal_path = &journal_files[0];

    // Get file size
    let file_size = fs::metadata(journal_path)
        .map_err(|e| format!("Failed to stat journal file: {e}"))?
        .len();

    // Determine start offset
    let start = match start_offset {
        Some(off) => off,
        None => random_u64() % file_size,
    };

    if start >= file_size {
        eprintln!("Start offset {start} >= journal size {file_size}, skipping");
        let _ = fs::remove_dir_all(&tempdir);
        return Ok(true);
    }

    eprintln!("Start offset: {start}, corrupt-step: {corrupt_step}, file size: {file_size}");

    // Iterate through offsets, flipping bits and trying to append
    let mut offset = start;
    while offset < file_size {
        // Flip a bit in the journal file
        {
            let mut file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(journal_path)
                .map_err(|e| format!("Failed to open journal for corruption: {e}"))?;

            file.seek(SeekFrom::Start(offset))
                .map_err(|e| format!("Failed to seek: {e}"))?;

            let mut byte = [0u8; 1];
            if file.read_exact(&mut byte).is_err() {
                break;
            }
            byte[0] |= 0x01;

            file.seek(SeekFrom::Start(offset))
                .map_err(|e| format!("Failed to seek for write: {e}"))?;
            file.write_all(&byte)
                .map_err(|e| format!("Failed to write corrupted byte: {e}"))?;
        }

        // Try to reopen the journal and write to it
        let config = StorageConfig {
            directory: tempdir.clone(),
            max_file_size: u64::MAX,
            max_disk_usage: u64::MAX,
            max_files: usize::MAX,
            persistent: false,
            keep_free: 0,
            direct_directory: true,
        };

        match JournalStorage::new(config) {
            Ok(mut storage) => {
                let entry = make_entry(&format!("Hello world {offset}"));
                match storage.append(&entry) {
                    Ok(_) => {
                        let _ = storage.flush();
                    }
                    Err(e) => {
                        // Write failure without crash is success
                        eprintln!("Failed to write to corrupted journal: {e}");
                        break;
                    }
                }
            }
            Err(e) => {
                // Reopen failure without crash is success
                eprintln!("Failed to reopen corrupted journal: {e}");
                break;
            }
        }

        offset += corrupt_step;
    }

    // Clean up
    let _ = fs::remove_dir_all(&tempdir);
    Ok(false)
}

fn main() {
    let cli = Cli::parse();

    // Single-shot reproducer mode
    if let Some(offset) = cli.run_one {
        match journal_corrupt_and_append(Some(offset), cli.corrupt_step) {
            Ok(_) => process::exit(0),
            Err(e) => {
                eprintln!("test-journal-append: {e}");
                process::exit(1);
            }
        }
    }

    for i in 0..cli.iterations {
        eprintln!("Iteration #{i}, step: {}", cli.iteration_step);

        let offset = if cli.sequential {
            Some(cli.start_offset.unwrap_or(0) + i * cli.iteration_step)
        } else {
            cli.start_offset
        };

        match journal_corrupt_and_append(offset, cli.corrupt_step) {
            Ok(true) => {
                // Reached end of journal file
                break;
            }
            Ok(false) => {}
            Err(e) => {
                eprintln!("test-journal-append: {e}");
                process::exit(1);
            }
        }
    }
}
