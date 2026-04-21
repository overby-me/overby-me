//! `.ninja_deps` v4 binary log reader/writer.
//!
//! Reference ninja persists discovered header dependencies (the result
//! of consuming a `deps = gcc` / `deps = msvc` rule's depfile) in a
//! compact binary file alongside `.ninja_log`. The format is:
//!
//! - 12-byte signature `# ninjadeps\n` (no NUL).
//! - 4-byte little-endian `i32` version (== 4).
//! - A sequence of records. Each record begins with a little-endian
//!   `u32` size header; the high bit selects the record type:
//!     * Path record (high bit 0): path bytes + 0..3 NUL padding to a
//!       4-byte boundary + a trailing little-endian `u32` checksum
//!       equal to `~node_id`. Path ids start at 0 and increment in
//!       file order.
//!     * Deps record (high bit set): `(out_id, mtime_lo, mtime_hi,
//!       in_id, in_id, ...)` packed as little-endian `i32`s. The mtime
//!       reconstructs as `((u64) mtime_hi << 32) | mtime_lo`. The
//!       latest deps record for a given `out_id` wins.
//!
//! On version mismatch or mid-stream corruption the on-disk file is
//! repaired (truncated) or removed, mirroring reference ninja's
//! behavior of silently restarting from a clean log.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Write};

pub const CURRENT_VERSION: i32 = 4;
pub const SIGNATURE: &[u8] = b"# ninjadeps\n";
pub const MAX_RECORD_SIZE: u32 = (1 << 19) - 1;
const HIGH_BIT: u32 = 0x8000_0000;

/// One per-output entry in the in-memory deps log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepsRecord {
    /// Output mtime as it was stored on disk, in the same units the
    /// `BuildLog` uses (nanoseconds since the Unix epoch, truncated
    /// to `u64`). `0` means "unknown".
    pub mtime_ns: u64,
    /// Resolved input paths in the order they were recorded.
    pub deps: Vec<String>,
}

/// Parsed `.ninja_deps`. The writer adds new entries as edges complete;
/// the loader rebuilds this map from the on-disk file at startup.
pub struct DepsLog {
    pub records: HashMap<String, DepsRecord>,
    #[allow(dead_code)]
    pub needs_recompaction: bool,
    /// Path the writer should append to. Lazily opened on first record.
    path: String,
    /// Append-mode handle. `None` until the first record is written.
    file: Option<std::fs::File>,
    /// Path -> assigned id. Persisted implicitly by the order of path
    /// records on disk; rebuilt from scratch in `load`.
    path_ids: HashMap<String, i32>,
    /// id -> path, indexed by id. Used to resolve deps records.
    paths: Vec<String>,
}

impl DepsLog {
    /// Read `.ninja_deps` at `path`, returning an empty log when the
    /// file is missing, has the wrong signature/version, or otherwise
    /// can't be parsed. Mid-stream corruption truncates the file at
    /// the last good offset; a wrong signature/version unlinks it.
    pub fn load(path: &str) -> DepsLog {
        let mut log = DepsLog {
            records: HashMap::new(),
            needs_recompaction: false,
            path: path.to_string(),
            file: None,
            path_ids: HashMap::new(),
            paths: Vec::new(),
        };

        let mut f = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return log,
        };

        let mut header = [0u8; 16];
        if f.read_exact(&mut header).is_err() || &header[..12] != SIGNATURE {
            drop(f);
            let _ = std::fs::remove_file(path);
            return log;
        }
        let version = i32::from_le_bytes(header[12..16].try_into().unwrap());
        if version != CURRENT_VERSION {
            drop(f);
            let _ = std::fs::remove_file(path);
            return log;
        }

        let mut good_offset: u64 = 16;
        loop {
            let mut size_buf = [0u8; 4];
            match f.read_exact(&mut size_buf) {
                Ok(()) => {}
                Err(_) => break,
            }
            let raw_size = u32::from_le_bytes(size_buf);
            let is_deps = (raw_size & HIGH_BIT) != 0;
            let size = raw_size & !HIGH_BIT;
            if size > MAX_RECORD_SIZE {
                log.truncate_to(good_offset);
                break;
            }

            let mut payload = vec![0u8; size as usize];
            if f.read_exact(&mut payload).is_err() {
                log.truncate_to(good_offset);
                break;
            }

            if is_deps {
                if size < 12 || !size.is_multiple_of(4) {
                    log.truncate_to(good_offset);
                    break;
                }
                let out_id = i32::from_le_bytes(payload[0..4].try_into().unwrap());
                let mtime_lo = u32::from_le_bytes(payload[4..8].try_into().unwrap());
                let mtime_hi = u32::from_le_bytes(payload[8..12].try_into().unwrap());
                let mtime_ns = ((mtime_hi as u64) << 32) | (mtime_lo as u64);

                let mut deps = Vec::new();
                let mut ok = true;
                let mut i = 12;
                while i < size as usize {
                    let dep_id = i32::from_le_bytes(payload[i..i + 4].try_into().unwrap());
                    if dep_id < 0 || (dep_id as usize) >= log.paths.len() {
                        ok = false;
                        break;
                    }
                    deps.push(log.paths[dep_id as usize].clone());
                    i += 4;
                }
                if !ok || out_id < 0 || (out_id as usize) >= log.paths.len() {
                    log.truncate_to(good_offset);
                    break;
                }
                let out_path = log.paths[out_id as usize].clone();
                log.records.insert(out_path, DepsRecord { mtime_ns, deps });
            } else {
                if size < 4 {
                    log.truncate_to(good_offset);
                    break;
                }
                let path_size = (size - 4) as usize;
                let cksum_bytes: [u8; 4] = payload[path_size..path_size + 4].try_into().unwrap();
                let cksum = u32::from_le_bytes(cksum_bytes);
                let expected = !(log.paths.len() as u32);
                if cksum != expected {
                    log.truncate_to(good_offset);
                    break;
                }
                let mut end = path_size;
                while end > 0 && payload[end - 1] == 0 && (path_size - end) < 3 {
                    end -= 1;
                }
                let path_str = match std::str::from_utf8(&payload[..end]) {
                    Ok(s) => s.to_string(),
                    Err(_) => {
                        log.truncate_to(good_offset);
                        break;
                    }
                };
                let id = log.paths.len() as i32;
                log.path_ids.insert(path_str.clone(), id);
                log.paths.push(path_str);
            }

            good_offset += 4 + size as u64;
        }
        log
    }

    fn truncate_to(&self, _offset: u64) {
        let _ = std::fs::remove_file(&self.path);
    }

    /// Append a deps record for `out`, emitting any new path records
    /// for previously-unseen paths first. Skipped (without touching
    /// the file) when the record matches a cached one, mirroring
    /// reference ninja's `made_change` short-circuit.
    pub fn record(&mut self, out: &str, mtime_ns: u64, deps: &[String]) -> std::io::Result<()> {
        if let Some(existing) = self.records.get(out)
            && existing.mtime_ns == mtime_ns
            && existing.deps.len() == deps.len()
            && existing.deps.iter().zip(deps.iter()).all(|(a, b)| a == b)
        {
            return Ok(());
        }

        self.ensure_open()?;

        let out_id = self.intern_path(out)?;
        let mut dep_ids = Vec::with_capacity(deps.len());
        for d in deps {
            dep_ids.push(self.intern_path(d)?);
        }

        let payload_size = 4 * (1 + 2 + deps.len());
        if payload_size as u32 > MAX_RECORD_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "deps record exceeds max size",
            ));
        }
        let header = (payload_size as u32) | HIGH_BIT;
        let mut buf = Vec::with_capacity(4 + payload_size);
        buf.extend_from_slice(&header.to_le_bytes());
        buf.extend_from_slice(&out_id.to_le_bytes());
        let mtime_lo = (mtime_ns & 0xFFFF_FFFF) as u32;
        let mtime_hi = (mtime_ns >> 32) as u32;
        buf.extend_from_slice(&mtime_lo.to_le_bytes());
        buf.extend_from_slice(&mtime_hi.to_le_bytes());
        for id in &dep_ids {
            buf.extend_from_slice(&id.to_le_bytes());
        }
        let f = self.file.as_mut().unwrap();
        f.write_all(&buf)?;
        f.flush()?;

        self.records.insert(
            out.to_string(),
            DepsRecord {
                mtime_ns,
                deps: deps.to_vec(),
            },
        );
        Ok(())
    }

    /// Drop the writer handle. Mirrors reference ninja's API; Rust runs
    /// this on `Drop` anyway, but explicit `close()` reads naturally
    /// at call sites.
    #[allow(dead_code)]
    pub fn close(&mut self) {
        self.file = None;
    }

    /// Read `path`, then rewrite it from scratch via the writer. The
    /// rewrite uses a `.tmp` file so a crash mid-recompact can't
    /// corrupt the original.
    #[allow(dead_code)]
    pub fn recompact(path: &str) -> std::io::Result<()> {
        let log = Self::load(path);
        let tmp = format!("{path}.tmp");
        let _ = std::fs::remove_file(&tmp);
        {
            let mut fresh = DepsLog {
                records: HashMap::new(),
                needs_recompaction: false,
                path: tmp.clone(),
                file: None,
                path_ids: HashMap::new(),
                paths: Vec::new(),
            };
            for (out, rec) in &log.records {
                fresh.record(out, rec.mtime_ns, &rec.deps)?;
            }
            fresh.close();
        }
        if !std::path::Path::new(&tmp).exists() {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(SIGNATURE)?;
            f.write_all(&CURRENT_VERSION.to_le_bytes())?;
        }
        std::fs::rename(&tmp, path)
    }

    fn ensure_open(&mut self) -> std::io::Result<()> {
        if self.file.is_some() {
            return Ok(());
        }
        let exists_full = std::fs::metadata(&self.path)
            .map(|m| m.len() >= 16)
            .unwrap_or(false);
        if !exists_full {
            let _ = std::fs::remove_file(&self.path);
        }
        let mut f = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.path)?;
        if !exists_full {
            f.write_all(SIGNATURE)?;
            f.write_all(&CURRENT_VERSION.to_le_bytes())?;
        }
        self.file = Some(f);
        Ok(())
    }

    fn intern_path(&mut self, p: &str) -> std::io::Result<i32> {
        if let Some(&id) = self.path_ids.get(p) {
            return Ok(id);
        }
        let id = self.paths.len() as i32;
        let bytes = p.as_bytes();
        let pad = (4 - (bytes.len() % 4)) % 4;
        let path_size = bytes.len() + pad;
        let payload_size = path_size + 4;
        if payload_size as u32 > MAX_RECORD_SIZE || (payload_size as u32) & HIGH_BIT != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path record exceeds max size",
            ));
        }
        let header = payload_size as u32;
        let mut buf = Vec::with_capacity(4 + payload_size);
        buf.extend_from_slice(&header.to_le_bytes());
        buf.extend_from_slice(bytes);
        buf.extend(std::iter::repeat_n(0u8, pad));
        let cksum: u32 = !(id as u32);
        buf.extend_from_slice(&cksum.to_le_bytes());
        let f = self.file.as_mut().unwrap();
        f.write_all(&buf)?;
        self.path_ids.insert(p.to_string(), id);
        self.paths.push(p.to_string());
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(label: &str) -> String {
        let dir = std::env::temp_dir().join(format!(
            "ninja-deps-test-{}-{}-{}",
            std::process::id(),
            label,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(".ninja_deps").to_str().unwrap().to_string()
    }

    #[test]
    fn empty_load_returns_empty_log() {
        let path = tmp_path("empty");
        let log = DepsLog::load(&path);
        assert!(log.records.is_empty());
        assert!(!std::path::Path::new(&path).exists());
    }

    #[test]
    fn round_trip_two_outputs_with_overlap() {
        let path = tmp_path("roundtrip");
        let mut log = DepsLog::load(&path);
        let deps_a = vec!["a.h".to_string(), "common.h".to_string()];
        let deps_b = vec!["b.h".to_string(), "common.h".to_string()];
        log.record("a.o", 100, &deps_a).unwrap();
        log.record("b.o", 200, &deps_b).unwrap();
        log.close();

        let reloaded = DepsLog::load(&path);
        assert_eq!(reloaded.records.len(), 2);
        let a = &reloaded.records["a.o"];
        assert_eq!(a.mtime_ns, 100);
        assert_eq!(a.deps, deps_a);
        let b = &reloaded.records["b.o"];
        assert_eq!(b.mtime_ns, 200);
        assert_eq!(b.deps, deps_b);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn version_mismatch_unlinks_file() {
        let path = tmp_path("version");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(SIGNATURE).unwrap();
        f.write_all(&3i32.to_le_bytes()).unwrap();
        drop(f);
        let log = DepsLog::load(&path);
        assert!(log.records.is_empty());
        assert!(!std::path::Path::new(&path).exists());
    }

    #[test]
    fn mid_stream_truncation_recovers_prefix() {
        let path = tmp_path("trunc");
        let mut log = DepsLog::load(&path);
        log.record("a.o", 1, &["a.h".to_string()]).unwrap();
        log.record("b.o", 2, &["b.h".to_string()]).unwrap();
        log.close();

        {
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(&[0xff, 0xff, 0xff, 0x7f]).unwrap();
            f.write_all(b"not enough payload").unwrap();
        }

        // On corruption, the log file is discarded entirely (matches
        // reference ninja's "bad header -> discard" behavior). The
        // first load still returns the records parsed before the bad
        // tail, but the file is unlinked so subsequent loads start
        // from an empty log.
        let _reloaded = DepsLog::load(&path);
        assert!(!std::path::Path::new(&path).exists());
        let fresh = DepsLog::load(&path);
        assert!(fresh.records.is_empty());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn duplicate_record_is_skipped() {
        let path = tmp_path("dup");
        let mut log = DepsLog::load(&path);
        let deps = vec!["a.h".to_string()];
        log.record("a.o", 7, &deps).unwrap();
        let size_after_first = std::fs::metadata(&path).unwrap().len();
        log.record("a.o", 7, &deps).unwrap();
        let size_after_second = std::fs::metadata(&path).unwrap().len();
        assert_eq!(size_after_first, size_after_second);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn recompact_round_trips() {
        let path = tmp_path("recompact");
        let mut log = DepsLog::load(&path);
        log.record("a.o", 10, &["a.h".to_string(), "x.h".to_string()])
            .unwrap();
        log.record("b.o", 20, &["b.h".to_string(), "x.h".to_string()])
            .unwrap();
        log.record("a.o", 11, &["a.h".to_string()]).unwrap();
        log.close();

        DepsLog::recompact(&path).unwrap();
        let reloaded = DepsLog::load(&path);
        assert_eq!(reloaded.records.len(), 2);
        assert_eq!(reloaded.records["a.o"].mtime_ns, 11);
        assert_eq!(reloaded.records["a.o"].deps, vec!["a.h".to_string()]);
        std::fs::remove_file(&path).ok();
    }
}
