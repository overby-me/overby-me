//! `.ninja_log` v6 reader/writer.
//!
//! The build log records, for each output file ninja has produced, the
//! command that produced it (hashed), the mtime ninja saw the output
//! at after the run, and a coarse start/end time pair (we write 0/0 —
//! no current consumer relies on durations).
//!
//! Format (one entry per line, tab-separated):
//!
//! ```text
//! # ninja log v6
//! <start_ms>\t<end_ms>\t<mtime_ns>\t<output_path>\t<hex_command_hash>
//! ```
//!
//! `mtime_ns` is the output file's mtime expressed as nanoseconds since
//! the Unix epoch — wide enough to survive any sub-second precision the
//! filesystem reports while staying a single integer column.
//!
//! The hash function is FNV-1a-64 over the executed command string. We
//! deliberately don't try to interoperate with reference ninja's
//! rapidhash: foreign-written log entries are detected by mismatched
//! hashes and trigger a one-time rebuild on the first cross-invocation,
//! which is the same conservative behavior reference ninja exhibits
//! when its log is missing.
//!
//! Version mismatch handling matches `test_issue_2048`: an older
//! version log is reported via the `-t recompact` warning, then
//! discarded so the next build re-populates a fresh v6 log.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

pub const CURRENT_VERSION: u32 = 6;

/// One per-output entry in the on-disk log.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub start_ms: u64,
    pub end_ms: u64,
    /// Output mtime in nanoseconds since the Unix epoch. `0` means
    /// "unknown" (e.g. the file was already gone when we recorded).
    pub mtime_ns: u128,
    pub command_hash: u64,
}

/// In-memory view of a parsed `.ninja_log`.
#[derive(Debug, Default)]
pub struct BuildLog {
    pub entries: HashMap<String, LogEntry>,
    /// True if the loader saw a version it can no longer use and the
    /// caller should warn + start over (the `test_issue_2048` path).
    pub too_old: bool,
}

/// Parse the contents of `.ninja_log` at `path`. Returns an empty log
/// (with `too_old = true` if appropriate) when the file is missing,
/// truncated, or wrongly versioned — never an error, because a missing
/// log is the cold-build path and reference ninja silently restarts.
pub fn load(path: &str) -> BuildLog {
    let mut log = BuildLog::default();
    let Ok(text) = std::fs::read_to_string(path) else {
        return log;
    };
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return log;
    };
    let Some(rest) = header.strip_prefix("# ninja log v") else {
        return log;
    };
    let version: u32 = rest.trim().parse().unwrap_or(0);
    if version != CURRENT_VERSION {
        log.too_old = true;
        return log;
    }
    for line in lines {
        let mut cols = line.splitn(5, '\t');
        let (Some(s), Some(e), Some(m), Some(out), Some(h)) = (
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
        ) else {
            continue;
        };
        let entry = LogEntry {
            start_ms: s.parse().unwrap_or(0),
            end_ms: e.parse().unwrap_or(0),
            mtime_ns: m.parse().unwrap_or(0),
            command_hash: u64::from_str_radix(h.trim(), 16).unwrap_or(0),
        };
        log.entries.insert(out.to_string(), entry);
    }
    log
}

/// FNV-1a-64 over the command string. Stable across runs and platforms;
/// we don't need cryptographic strength, only collision resistance for
/// distinct command lines.
pub fn hash_command(command: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in command.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Convert a filesystem `SystemTime` to a `u128` nanosecond timestamp.
/// Returns `0` for the epoch / pre-epoch sentinel "unknown".
pub fn mtime_to_ns(t: Option<SystemTime>) -> u128 {
    match t {
        Some(t) => t
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        None => 0,
    }
}

/// Append-mode writer that ensures a header is present before the
/// first entry. Mirrors reference ninja's `OpenForWriteIfNeeded` —
/// the log file is created lazily on first record.
pub struct Writer {
    path: String,
    file: Option<std::fs::File>,
    /// True if the file already had a valid v6 header when we opened
    /// it. Determined by checking the file's existing first line on
    /// the first record.
    header_present: bool,
}

impl Writer {
    pub fn new(path: &str) -> Self {
        let header_present = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| s.lines().next().map(str::to_string))
            .is_some_and(|h| h == format!("# ninja log v{CURRENT_VERSION}"));
        Self {
            path: path.to_string(),
            file: None,
            header_present,
        }
    }

    fn ensure_open(&mut self) -> std::io::Result<()> {
        if self.file.is_some() {
            return Ok(());
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        if !self.header_present {
            writeln!(f, "# ninja log v{CURRENT_VERSION}")?;
            self.header_present = true;
        }
        self.file = Some(f);
        Ok(())
    }

    /// Append one entry per output. Failures to write the log are
    /// non-fatal — the build itself succeeded and we'd rather lose
    /// incremental-build accuracy than abort the user's run.
    pub fn record(&mut self, outputs: &[String], entry: &LogEntry) {
        if self.ensure_open().is_err() {
            return;
        }
        let Some(f) = self.file.as_mut() else { return };
        for out in outputs {
            let _ = writeln!(
                f,
                "{}\t{}\t{}\t{}\t{:x}",
                entry.start_ms, entry.end_ms, entry.mtime_ns, out, entry.command_hash
            );
        }
    }
}

/// Recompact `.ninja_log`: rewrite the file containing only the latest
/// entry for each output, dropping duplicates. Used by `-t recompact`
/// and after detecting an old version.
pub fn recompact(path: &str) -> std::io::Result<()> {
    let log = load(path);
    if log.entries.is_empty() && !log.too_old {
        // Nothing to do — but still ensure the header exists so a
        // later append starts from a valid file.
    }
    let tmp = format!("{path}.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        writeln!(f, "# ninja log v{CURRENT_VERSION}")?;
        for (out, e) in &log.entries {
            writeln!(
                f,
                "{}\t{}\t{}\t{}\t{:x}",
                e.start_ms, e.end_ms, e.mtime_ns, out, e.command_hash
            )?;
        }
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_disk() {
        let dir = std::env::temp_dir().join(format!("ninja-log-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".ninja_log");
        let path_str = path.to_str().unwrap();

        // Empty load: no entries, not too old.
        let log = load(path_str);
        assert!(log.entries.is_empty());
        assert!(!log.too_old);

        // Write two entries via the writer.
        let mut w = Writer::new(path_str);
        let e = LogEntry {
            start_ms: 100,
            end_ms: 200,
            mtime_ns: 1234567890,
            command_hash: hash_command("echo hi"),
        };
        w.record(&["out1.o".to_string()], &e);
        w.record(&["out2.o".to_string()], &e);
        drop(w);

        // Read them back.
        let log = load(path_str);
        assert!(!log.too_old);
        assert_eq!(log.entries.len(), 2);
        let got = &log.entries["out1.o"];
        assert_eq!(got.start_ms, 100);
        assert_eq!(got.mtime_ns, 1234567890);
        assert_eq!(got.command_hash, hash_command("echo hi"));

        // Recompact (rewrite header + entries) — should be a no-op for content.
        recompact(path_str).unwrap();
        let log = load(path_str);
        assert_eq!(log.entries.len(), 2);

        // An older-version log triggers `too_old`.
        std::fs::write(&path, "# ninja log v4\n").unwrap();
        let log = load(path_str);
        assert!(log.too_old);
        assert!(log.entries.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hash_command_is_stable_and_distinguishes() {
        assert_eq!(hash_command(""), hash_command(""));
        assert_eq!(
            hash_command("cc -o foo foo.c"),
            hash_command("cc -o foo foo.c")
        );
        assert_ne!(
            hash_command("cc -o foo foo.c"),
            hash_command("cc -O2 -o foo foo.c")
        );
    }
}
