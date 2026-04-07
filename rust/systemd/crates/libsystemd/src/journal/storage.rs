//! Journal storage — manages journal file storage.
//!
//! This module provides a simple append-only journal file format that stores
//! structured log entries on disk.  The format is designed to be:
//!
//! - **Append-only** — entries are always appended, never modified in place.
//! - **Crash-safe** — entries are delimited so that a truncated write at the
//!   end of the file can be detected and skipped during replay.
//! - **Queryable** — entries can be iterated, filtered by field values,
//!   and sought by timestamp or sequence number.
//! - **Rotatable** — when a journal file exceeds its configured size limit
//!   a new file is started and old files can be vacuumed.
//!
//! ## On-disk format
//!
//! Each journal file has the extension `.journal` and consists of:
//!
//! 1. A 64-byte file header (magic, version, file ID, head/tail seqnum, …)
//! 2. A sequence of **entry frames**, each structured as:
//!    - `u32` LE — frame length (excluding this u32 itself)
//!    - `u64` LE — realtime timestamp (µs since UNIX epoch)
//!    - `u64` LE — monotonic timestamp (µs since boot)
//!    - `u64` LE — sequence number
//!    - `u32` LE — number of fields
//!    - For each field:
//!      - `u16` LE — key length
//!      - `u32` LE — value length
//!      - key bytes (UTF-8)
//!      - value bytes (arbitrary)
//!    - `u32` LE — frame length again (trailer, for reverse iteration)
//!
//! The header is written once when the file is created and updated
//! (head/tail seqnum, entry count, file size) after each append via a
//! single atomic `pwrite`.
//!
//! ## JournalReader
//!
//! The [`JournalReader`] struct provides an iterator-style interface for
//! reading entries with optional field-based filtering and cursor/timestamp
//! seeking.  It is the primary read interface used by `journalctl` and
//! other consumers.

use super::c_journal;
use super::entry::{FieldMatch, JournalEntry};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Magic bytes at the start of every journal file.
const JOURNAL_MAGIC: &[u8; 8] = b"JRNL_RS\0";

/// Current format version.
const FORMAT_VERSION: u32 = 1;

/// Size of the file header in bytes.
const HEADER_SIZE: u64 = 64;

/// Default maximum size of a single journal file (64 MiB).
const DEFAULT_MAX_FILE_SIZE: u64 = 64 * 1024 * 1024;

/// Default maximum total disk usage across all journal files (512 MiB).
const DEFAULT_MAX_DISK_USAGE: u64 = 512 * 1024 * 1024;

/// Default maximum number of journal files to keep.
const DEFAULT_MAX_FILES: usize = 100;

// ---------------------------------------------------------------------------
// File header
// ---------------------------------------------------------------------------

/// Compression algorithm stored in the journal file header.
///
/// The value is stored as a single byte at offset 60 in the header.
/// Data is currently always stored uncompressed; this field records
/// the *requested* algorithm so `journalctl --verify` can report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum JournalCompress {
    None = 0,
    Xz = 1,
    Lz4 = 2,
    Zstd = 3,
}

impl JournalCompress {
    /// Parse from the `SYSTEMD_JOURNAL_COMPRESS` env var or config string.
    pub fn from_env_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "NONE" | "NO" | "0" | "FALSE" => JournalCompress::None,
            "XZ" => JournalCompress::Xz,
            "LZ4" => JournalCompress::Lz4,
            "ZSTD" | "YES" | "1" | "TRUE" | "" => JournalCompress::Zstd,
            _ => JournalCompress::Zstd, // default
        }
    }

    fn from_u8(b: u8) -> Self {
        match b {
            0 => JournalCompress::None,
            1 => JournalCompress::Xz,
            2 => JournalCompress::Lz4,
            _ => JournalCompress::Zstd,
        }
    }

    /// Return the name as used by C systemd in `journalctl --verify` output.
    pub fn as_str(self) -> &'static str {
        match self {
            JournalCompress::None => "NONE",
            JournalCompress::Xz => "XZ",
            JournalCompress::Lz4 => "LZ4",
            JournalCompress::Zstd => "ZSTD",
        }
    }
}

/// On-disk file header (64 bytes).
#[derive(Debug, Clone)]
struct FileHeader {
    /// Magic bytes (`JRNL_RS\0`).
    magic: [u8; 8],
    /// Format version.
    version: u32,
    /// Unique file ID (random, for cursor addressing).
    file_id: u128,
    /// Sequence number of the first entry in this file.
    head_seqnum: u64,
    /// Sequence number of the last entry in this file.
    tail_seqnum: u64,
    /// Number of entries in this file.
    entry_count: u64,
    /// Total file size in bytes (updated after each append).
    file_size: u64,
    /// Compression algorithm (byte 60).
    compress: JournalCompress,
}

impl FileHeader {
    fn new(compress: JournalCompress) -> Self {
        // Generate a random file ID from /dev/urandom or fallback
        let file_id = generate_random_u128();
        FileHeader {
            magic: *JOURNAL_MAGIC,
            version: FORMAT_VERSION,
            file_id,
            head_seqnum: 0,
            tail_seqnum: 0,
            entry_count: 0,
            file_size: HEADER_SIZE,
            compress,
        }
    }

    fn serialize(&self) -> [u8; 64] {
        let mut buf = [0u8; 64];
        buf[0..8].copy_from_slice(&self.magic);
        buf[8..12].copy_from_slice(&self.version.to_le_bytes());
        buf[12..28].copy_from_slice(&self.file_id.to_le_bytes());
        buf[28..36].copy_from_slice(&self.head_seqnum.to_le_bytes());
        buf[36..44].copy_from_slice(&self.tail_seqnum.to_le_bytes());
        buf[44..52].copy_from_slice(&self.entry_count.to_le_bytes());
        buf[52..60].copy_from_slice(&self.file_size.to_le_bytes());
        buf[60] = self.compress as u8;
        // bytes 61..64 reserved (zeros)
        buf
    }

    fn deserialize(buf: &[u8; 64]) -> io::Result<Self> {
        let mut magic = [0u8; 8];
        magic.copy_from_slice(&buf[0..8]);
        if &magic != JOURNAL_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid journal file magic",
            ));
        }

        let version = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        if version != FORMAT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported journal format version {} (expected {})",
                    version, FORMAT_VERSION
                ),
            ));
        }

        Ok(FileHeader {
            magic,
            version,
            file_id: u128::from_le_bytes(buf[12..28].try_into().unwrap()),
            head_seqnum: u64::from_le_bytes(buf[28..36].try_into().unwrap()),
            tail_seqnum: u64::from_le_bytes(buf[36..44].try_into().unwrap()),
            entry_count: u64::from_le_bytes(buf[44..52].try_into().unwrap()),
            file_size: u64::from_le_bytes(buf[52..60].try_into().unwrap()),
            compress: JournalCompress::from_u8(buf[60]),
        })
    }
}

// ---------------------------------------------------------------------------
// JournalFile — a single on-disk journal file
// ---------------------------------------------------------------------------

/// Handle to an open journal file.
#[derive(Debug)]
struct JournalFile {
    /// Path on disk.
    path: PathBuf,
    /// Cached header.
    header: FileHeader,
    /// Open file handle for appending.
    writer: Option<BufWriter<File>>,
}

impl JournalFile {
    /// Create a new journal file at `path`.
    fn create(path: &Path, compress: JournalCompress) -> io::Result<Self> {
        let header = FileHeader::new(compress);

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;

        let mut writer = BufWriter::new(file);
        writer.write_all(&header.serialize())?;
        writer.flush()?;

        Ok(JournalFile {
            path: path.to_path_buf(),
            header,
            writer: Some(writer),
        })
    }

    /// Open an existing journal file for reading (and optionally appending).
    fn open(path: &Path, writable: bool) -> io::Result<Self> {
        let mut file = OpenOptions::new().read(true).open(path)?;

        let mut header_buf = [0u8; 64];
        file.read_exact(&mut header_buf)?;
        let header = FileHeader::deserialize(&header_buf)?;

        let writer = if writable {
            let append_file = OpenOptions::new().append(true).open(path)?;
            Some(BufWriter::new(append_file))
        } else {
            None
        };

        Ok(JournalFile {
            path: path.to_path_buf(),
            header,
            writer,
        })
    }

    /// Append an entry to this file.  Returns the sequence number assigned.
    fn append(&mut self, entry: &JournalEntry, seqnum: u64) -> io::Result<u64> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| io::Error::other("journal file not writable"))?;

        let frame = serialize_entry(entry, seqnum);
        writer.write_all(&frame)?;
        writer.flush()?;

        // Update header
        if self.header.entry_count == 0 {
            self.header.head_seqnum = seqnum;
        }
        self.header.tail_seqnum = seqnum;
        self.header.entry_count += 1;
        self.header.file_size += frame.len() as u64;

        // Write the updated header to disk (seek to beginning).
        // We do this by opening the file again briefly for a pwrite-style
        // update, since BufWriter is in append mode.
        let header_bytes = self.header.serialize();
        let mut header_file = OpenOptions::new().write(true).open(&self.path)?;
        header_file.write_all(&header_bytes)?;
        header_file.flush()?;

        Ok(seqnum)
    }

    /// Read all entries from this file.
    fn read_all(&self) -> io::Result<Vec<JournalEntry>> {
        let file = File::open(&self.path)?;
        let mut reader = BufReader::new(file);

        // Skip header
        reader.seek(SeekFrom::Start(HEADER_SIZE))?;

        let mut entries = Vec::new();
        loop {
            match deserialize_entry(&mut reader) {
                Ok(entry) => entries.push(entry),
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => {
                    // Possibly a truncated entry at the end — stop reading
                    eprintln!(
                        "journald: Warning: truncated entry in {}: {}",
                        self.path.display(),
                        e
                    );
                    break;
                }
            }
        }

        Ok(entries)
    }

    /// Get the on-disk size of this file.
    fn size(&self) -> u64 {
        self.header.file_size
    }
}

// ---------------------------------------------------------------------------
// Entry frame serialisation
// ---------------------------------------------------------------------------

/// Serialise a journal entry into a frame (see module-level docs for format).
fn serialize_entry(entry: &JournalEntry, seqnum: u64) -> Vec<u8> {
    // Calculate the frame body size first
    let mut body_size: usize = 0;
    body_size += 8; // realtime_usec
    body_size += 8; // monotonic_usec
    body_size += 8; // seqnum
    body_size += 4; // field count

    for (key, value) in &entry.fields {
        body_size += 2; // key length (u16)
        body_size += 4; // value length (u32)
        body_size += key.len();
        body_size += value.len();
    }

    let frame_len = body_size as u32;
    let total_size = 4 + body_size + 4; // leading len + body + trailing len
    let mut buf = Vec::with_capacity(total_size);

    // Leading frame length
    buf.extend_from_slice(&frame_len.to_le_bytes());

    // Timestamps and seqnum
    buf.extend_from_slice(&entry.realtime_usec.to_le_bytes());
    buf.extend_from_slice(&entry.monotonic_usec.to_le_bytes());
    buf.extend_from_slice(&seqnum.to_le_bytes());

    // Field count
    let field_count = entry.fields.len() as u32;
    buf.extend_from_slice(&field_count.to_le_bytes());

    // Fields
    for (key, value) in &entry.fields {
        let key_bytes = key.as_bytes();
        buf.extend_from_slice(&(key_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(&(value.len() as u32).to_le_bytes());
        buf.extend_from_slice(key_bytes);
        buf.extend_from_slice(value);
    }

    // Trailing frame length
    buf.extend_from_slice(&frame_len.to_le_bytes());

    debug_assert_eq!(buf.len(), total_size);
    buf
}

/// Deserialise a single entry frame from a reader.
fn deserialize_entry<R: Read>(reader: &mut R) -> io::Result<JournalEntry> {
    // Read leading frame length
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let frame_len = u32::from_le_bytes(len_buf) as usize;

    if frame_len < 28 {
        // Minimum: 8+8+8+4 = 28 bytes for timestamps + seqnum + field count
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too small: {} bytes", frame_len),
        ));
    }

    // Sanity cap: no single journal entry should exceed 64 MiB
    const MAX_FRAME_LEN: usize = 64 * 1024 * 1024;
    if frame_len > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too large: {} bytes", frame_len),
        ));
    }

    // Read the entire frame body
    let mut body = vec![0u8; frame_len];
    reader.read_exact(&mut body)?;

    // Read trailing frame length
    let mut trail_buf = [0u8; 4];
    reader.read_exact(&mut trail_buf)?;
    let trail_len = u32::from_le_bytes(trail_buf);
    if trail_len != frame_len as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "frame length mismatch: header={}, trailer={}",
                frame_len, trail_len
            ),
        ));
    }

    // Parse body
    let mut pos = 0;

    let realtime_usec = u64::from_le_bytes(body[pos..pos + 8].try_into().unwrap());
    pos += 8;
    let monotonic_usec = u64::from_le_bytes(body[pos..pos + 8].try_into().unwrap());
    pos += 8;
    let seqnum = u64::from_le_bytes(body[pos..pos + 8].try_into().unwrap());
    pos += 8;
    let field_count = u32::from_le_bytes(body[pos..pos + 4].try_into().unwrap()) as usize;
    pos += 4;

    let mut fields = BTreeMap::new();

    for _ in 0..field_count {
        if pos + 6 > body.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "truncated field header",
            ));
        }

        let key_len = u16::from_le_bytes(body[pos..pos + 2].try_into().unwrap()) as usize;
        pos += 2;
        let value_len = u32::from_le_bytes(body[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;

        if pos + key_len + value_len > body.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "truncated field data",
            ));
        }

        let key = String::from_utf8_lossy(&body[pos..pos + key_len]).into_owned();
        pos += key_len;
        let value = body[pos..pos + value_len].to_vec();
        pos += value_len;

        fields.insert(key, value);
    }

    let mut entry = JournalEntry::with_timestamp(realtime_usec, monotonic_usec);
    entry.seqnum = seqnum;
    entry.fields = fields;

    Ok(entry)
}

// ---------------------------------------------------------------------------
// JournalStorage — multi-file journal storage manager
// ---------------------------------------------------------------------------

/// Configuration for journal storage.
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// Base directory for journal files (e.g. `/var/log/journal/<machine-id>`
    /// or `/run/log/journal/<machine-id>`).
    pub directory: PathBuf,

    /// Maximum size of a single journal file in bytes.
    pub max_file_size: u64,

    /// Maximum total disk usage for all journal files.
    pub max_disk_usage: u64,

    /// Maximum number of journal files to keep.
    pub max_files: usize,

    /// Whether to use persistent storage (`/var/log/journal`) or volatile
    /// (`/run/log/journal`).
    pub persistent: bool,

    /// Minimum free disk space to keep on the file system (bytes).
    /// When the available space drops below this threshold, the oldest
    /// journal files are vacuumed.  Defaults to 15% of the file system
    /// or 4 GiB, whichever is smaller (set to 0 to disable).
    pub keep_free: u64,

    /// When true, `directory` is used as-is without appending the machine ID.
    /// This is used when `--directory` is passed explicitly by the user (the
    /// path already points to the journal directory).
    pub direct_directory: bool,

    /// Compression algorithm to record in new journal file headers.
    /// Defaults to `Zstd`.  Read from the `SYSTEMD_JOURNAL_COMPRESS`
    /// environment variable by journald.
    pub compress: JournalCompress,

    /// When non-empty, restrict reads to only these specific files.
    /// Used by `journalctl --file` to read only the specified journal files
    /// instead of all files in the directory.
    pub file_filter: Vec<PathBuf>,
}

/// Default keep-free value: 4 GiB (capped at 15% of filesystem in vacuum()).
const DEFAULT_KEEP_FREE: u64 = 4 * 1024 * 1024 * 1024;

impl Default for StorageConfig {
    fn default() -> Self {
        StorageConfig {
            directory: PathBuf::from("/run/log/journal"),
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            max_disk_usage: DEFAULT_MAX_DISK_USAGE,
            max_files: DEFAULT_MAX_FILES,
            persistent: false,
            keep_free: DEFAULT_KEEP_FREE,
            direct_directory: false,
            compress: JournalCompress::Zstd,
            file_filter: Vec::new(),
        }
    }
}

/// Manages journal storage across multiple files with rotation and vacuuming.
pub struct JournalStorage {
    config: StorageConfig,
    /// The currently active (writable) journal file.
    active_file: Option<JournalFile>,
    /// Next sequence number to assign.
    next_seqnum: u64,
    /// Cached machine ID for directory naming.
    machine_id: String,
}

impl JournalStorage {
    /// Create a new journal storage manager.
    ///
    /// This creates the storage directory if it does not exist and opens
    /// (or creates) the active journal file.
    pub fn new(config: StorageConfig) -> io::Result<Self> {
        let machine_id = read_machine_id();
        let journal_dir = if config.direct_directory {
            config.directory.clone()
        } else {
            config.directory.join(&machine_id)
        };
        fs::create_dir_all(&journal_dir)?;

        let mut storage = JournalStorage {
            config,
            active_file: None,
            next_seqnum: 1,
            machine_id,
        };

        // Try to resume from the newest existing journal file, otherwise
        // create a new one.
        storage.open_or_create_active_file()?;

        Ok(storage)
    }

    /// Open journal storage in read-only mode.  Does not create directories
    /// or files and does not open any write handles.
    pub fn open_read_only(config: StorageConfig) -> io::Result<Self> {
        let machine_id = read_machine_id();

        Ok(JournalStorage {
            config,
            active_file: None,
            next_seqnum: 0,
            machine_id,
        })
    }

    /// Append a journal entry.  Handles rotation if the active file is
    /// too large.  Returns the assigned sequence number.
    pub fn append(&mut self, entry: &JournalEntry) -> io::Result<u64> {
        // Check if we need to rotate
        if let Some(ref file) = self.active_file
            && file.size() >= self.config.max_file_size
        {
            self.rotate()?;
        }

        // Ensure we have an active file
        if self.active_file.is_none() {
            self.open_or_create_active_file()?;
        }

        let seqnum = self.next_seqnum;
        self.next_seqnum += 1;

        if let Some(ref mut file) = self.active_file {
            file.append(entry, seqnum)?;
        }

        Ok(seqnum)
    }

    /// Read all entries from all journal files, in chronological order.
    ///
    /// Supports both JRNL_RS and C systemd (LPKSHHRH) journal files.
    /// When `config.file_filter` is non-empty, only those files are read.
    pub fn read_all(&self) -> io::Result<Vec<JournalEntry>> {
        if !self.config.file_filter.is_empty() {
            read_all_from_files(&self.config.file_filter)
        } else {
            let journal_dir = self.journal_dir();
            read_all_from_directory(&journal_dir)
        }
    }

    /// Generate a cursor string for an entry.  The cursor encodes enough
    /// information to uniquely identify an entry across files.
    pub fn make_cursor(&self, entry: &JournalEntry) -> String {
        let file_id = self
            .active_file
            .as_ref()
            .map(|f| f.header.file_id)
            .unwrap_or(0);
        format!(
            "s={:032x};i={:x};b={};m={:x};t={:x};x={:x}",
            file_id,
            entry.seqnum,
            entry.boot_id().unwrap_or_default(),
            entry.monotonic_usec,
            entry.realtime_usec,
            0u64, // xor hash, unused for now
        )
    }

    /// Rotate the active journal file: close the current file and start
    /// a new one.
    pub fn rotate(&mut self) -> io::Result<()> {
        // Rename the active file from system.journal to system@<seqid>.journal
        // before closing it, matching C journald's archive naming convention.
        if let Some(ref file) = self.active_file {
            let journal_dir = self.journal_dir();
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros();
            let random_part = generate_random_u128() & 0xFFFF_FFFF;
            let archived_name = format!("system@{:016x}-{:08x}.journal", timestamp, random_part);
            let archived_path = journal_dir.join(archived_name);
            let _ = fs::rename(&file.path, &archived_path);
        }

        // Drop the current active file (closes the writer)
        self.active_file = None;

        // Vacuum old files if over limits
        self.vacuum()?;

        // Create a new file
        self.create_new_active_file()?;

        Ok(())
    }

    /// Flush any buffered writes to disk.
    pub fn flush(&mut self) -> io::Result<()> {
        if let Some(ref mut file) = self.active_file
            && let Some(ref mut writer) = file.writer
        {
            writer.flush()?;
            // fsync to ensure data is visible to other processes
            writer.get_ref().sync_all()?;
        }
        Ok(())
    }

    /// Update the storage configuration (e.g. after config reload).
    /// This updates limits like max_disk_usage, max_files, max_file_size
    /// without closing or moving files.
    pub fn update_config(&mut self, config: StorageConfig) {
        self.config = config;
    }

    /// Return the total disk usage of all journal files.
    pub fn disk_usage(&self) -> io::Result<u64> {
        let journal_dir = self.journal_dir();
        let files = list_journal_files(&journal_dir)?;
        let mut total = 0u64;
        for path in &files {
            if let Ok(meta) = fs::metadata(path) {
                total += meta.len();
            }
        }
        Ok(total)
    }

    /// Return the number of journal files.
    pub fn file_count(&self) -> io::Result<usize> {
        let journal_dir = self.journal_dir();
        let files = list_journal_files(&journal_dir)?;
        Ok(files.len())
    }

    /// Get the storage directory path.
    pub fn directory(&self) -> PathBuf {
        self.journal_dir()
    }

    /// Get the compression algorithm of the active (newest) journal file,
    /// or the configured default if no file is open.
    pub fn compress(&self) -> JournalCompress {
        self.active_file
            .as_ref()
            .map(|f| f.header.compress)
            .unwrap_or(self.config.compress)
    }

    /// Resolve the actual journal directory, respecting `direct_directory`.
    pub fn journal_dir(&self) -> PathBuf {
        if self.config.direct_directory {
            self.config.directory.clone()
        } else {
            self.config.directory.join(&self.machine_id)
        }
    }

    /// Create a [`JournalReader`] for reading entries from this storage
    /// with optional filtering and seeking.
    pub fn reader(&self) -> JournalReader {
        JournalReader {
            directory: self.directory(),
            filters: Vec::new(),
            seek: SeekPosition::Head,
        }
    }

    /// Read entries matching the given filters.
    ///
    /// This is a convenience wrapper around [`JournalReader`].
    pub fn read_filtered(&self, filters: &[FieldMatch]) -> io::Result<Vec<JournalEntry>> {
        let mut reader = self.reader();
        reader.filters = filters.to_vec();
        reader.collect()
    }

    /// Read entries starting from (or after) the given cursor string.
    ///
    /// If `after` is `true`, the entry at the cursor itself is excluded.
    pub fn read_from_cursor(&self, cursor: &str, after: bool) -> io::Result<Vec<JournalEntry>> {
        let mut reader = self.reader();
        reader.seek = if after {
            SeekPosition::AfterCursor(cursor.to_string())
        } else {
            SeekPosition::Cursor(cursor.to_string())
        };
        reader.collect()
    }

    /// Read entries starting from the given realtime timestamp (µs since epoch).
    pub fn read_from_realtime(&self, realtime_usec: u64) -> io::Result<Vec<JournalEntry>> {
        let mut reader = self.reader();
        reader.seek = SeekPosition::RealtimeTimestamp(realtime_usec);
        reader.collect()
    }

    /// Write all entries to a writer in the journal export format.
    pub fn export_all<W: Write>(&self, writer: &mut W) -> io::Result<u64> {
        let entries = self.read_all()?;
        let mut count = 0u64;
        for entry in &entries {
            let cursor = self.make_cursor(entry);
            let data = entry.to_export_format(&cursor);
            writer.write_all(&data)?;
            count += 1;
        }
        Ok(count)
    }

    // ---------------------------------------------------------------
    // Internal helpers
    // ---------------------------------------------------------------

    fn open_or_create_active_file(&mut self) -> io::Result<()> {
        let journal_dir = self.journal_dir();
        let mut files = list_journal_files(&journal_dir)?;
        files.sort();

        // Scan ALL files to find the highest tail_seqnum, ensuring the
        // counter never goes backwards even if files sort out of order.
        for path in &files {
            if let Ok(jf) = JournalFile::open(path, false)
                && jf.header.tail_seqnum >= self.next_seqnum
            {
                self.next_seqnum = jf.header.tail_seqnum + 1;
            }
        }

        // Try to open system.journal (C journald active file convention)
        let active_path = journal_dir.join("system.journal");
        if active_path.exists() {
            if let Ok(jf) = JournalFile::open(&active_path, true) {
                if jf.header.compress != self.config.compress {
                    // Compression setting changed (e.g. SYSTEMD_JOURNAL_COMPRESS
                    // env var). Archive the old file and create a new one.
                    drop(jf);
                } else if jf.size() < self.config.max_file_size {
                    self.active_file = Some(jf);
                    return Ok(());
                }
            }
        } else if let Some(newest) = files.last() {
            // Fallback: try the newest existing file (e.g. from an older version)
            if let Ok(jf) = JournalFile::open(newest, true) {
                if jf.size() < self.config.max_file_size {
                    self.active_file = Some(jf);
                    return Ok(());
                }
            }
        }

        self.create_new_active_file()
    }

    fn create_new_active_file(&mut self) -> io::Result<()> {
        let journal_dir = self.journal_dir();
        let path = journal_dir.join("system.journal");

        // If system.journal already exists (e.g. unclean shutdown), archive it first
        if path.exists() {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros();
            let random_part = generate_random_u128() & 0xFFFF_FFFF;
            let archived_name = format!("system@{:016x}-{:08x}.journal", timestamp, random_part);
            let _ = fs::rename(&path, journal_dir.join(archived_name));
        }

        let jf = JournalFile::create(&path, self.config.compress)?;
        self.active_file = Some(jf);
        Ok(())
    }

    /// Run vacuum to enforce file count, disk usage, and keep-free limits.
    ///
    /// This is also exposed publicly so the daemon can trigger periodic
    /// vacuum from its maintenance thread without a full rotation.
    pub fn vacuum(&mut self) -> io::Result<()> {
        let journal_dir = self.journal_dir();
        let mut files = list_journal_files(&journal_dir)?;
        files.sort();

        // Never vacuum the active journal file — it's currently being written to.
        let active_path = self.active_file.as_ref().map(|f| f.path.clone());
        if let Some(ref active) = active_path {
            files.retain(|f| f != active);
        }

        // Remove files until we're under the file count limit.
        // Always reserve one slot for the active file — even if it doesn't
        // currently exist, the daemon will create one when writing.
        while files.len() + 1 > self.config.max_files {
            if let Some(oldest) = files.first() {
                eprintln!(
                    "journald: Vacuuming {} (file count limit)",
                    oldest.display()
                );
                let _ = fs::remove_file(oldest);
                files.remove(0);
            } else {
                break;
            }
        }

        // Check total disk usage (include active file in the total)
        loop {
            let mut total: u64 = 0;
            for f in &files {
                if let Ok(meta) = fs::metadata(f) {
                    total += meta.len();
                }
            }
            if let Some(ref active) = active_path
                && let Ok(meta) = fs::metadata(active)
            {
                total += meta.len();
            }
            if total <= self.config.max_disk_usage || files.is_empty() {
                break;
            }
            if let Some(oldest) = files.first() {
                eprintln!(
                    "journald: Vacuuming {} (disk usage limit: {} > {})",
                    oldest.display(),
                    total,
                    self.config.max_disk_usage
                );
                let _ = fs::remove_file(oldest);
                files.remove(0);
            } else {
                break;
            }
        }

        // Enforce keep-free: ensure the filesystem has at least `keep_free`
        // bytes available. Cap keep_free at 15% of total filesystem size
        // to prevent vacuuming everything on small disks.
        if self.config.keep_free > 0 {
            let effective_keep_free = match total_disk_space(&journal_dir) {
                Some(total) => self.config.keep_free.min(total * 15 / 100),
                None => self.config.keep_free,
            };
            loop {
                if files.is_empty() {
                    break;
                }
                let free = available_disk_space(&journal_dir).unwrap_or(u64::MAX);
                if free >= effective_keep_free {
                    break;
                }
                if let Some(oldest) = files.first() {
                    eprintln!(
                        "journald: Vacuuming {} (keep free limit: {} free < {} required)",
                        oldest.display(),
                        free,
                        effective_keep_free
                    );
                    let _ = fs::remove_file(oldest);
                    files.remove(0);
                } else {
                    break;
                }
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Free helper functions
// ---------------------------------------------------------------------------

/// List all `.journal` files in a directory, sorted by name.
/// Return the available (non-privileged) disk space on the filesystem
/// containing `path`, or `None` on error.
fn available_disk_space(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
            Some(stat.f_bavail * stat.f_frsize)
        } else {
            None
        }
    }
}

fn total_disk_space(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
            Some(stat.f_blocks * stat.f_frsize)
        } else {
            None
        }
    }
}

fn list_journal_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "journal") && path.is_file() {
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

/// Read the machine ID from `/etc/machine-id`.
fn read_machine_id() -> String {
    fs::read_to_string("/etc/machine-id")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "0".repeat(32))
}

/// Generate a random u128 using `/dev/urandom`.
fn generate_random_u128() -> u128 {
    let mut buf = [0u8; 16];
    if let Ok(mut f) = File::open("/dev/urandom")
        && f.read_exact(&mut buf).is_ok()
    {
        return u128::from_le_bytes(buf);
    }
    // Fallback: use the current time as a poor man's random
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let lo = now.as_nanos() as u64;
    let hi = std::process::id() as u64;
    ((hi as u128) << 64) | (lo as u128)
}

// ---------------------------------------------------------------------------
// JournalReader — filtered, seekable journal reading
// ---------------------------------------------------------------------------

/// Where to start reading entries.
#[derive(Debug, Clone)]
pub enum SeekPosition {
    /// Start from the first (oldest) entry.
    Head,
    /// Start from the last (newest) entry.  When used with
    /// [`JournalReader::collect`], returns entries in reverse
    /// chronological order.
    Tail,
    /// Start from the entry identified by this cursor string (inclusive).
    Cursor(String),
    /// Start just after the entry identified by this cursor (exclusive).
    AfterCursor(String),
    /// Start from the first entry whose realtime timestamp ≥ this value.
    RealtimeTimestamp(u64),
    /// Start from the first entry whose sequence number ≥ this value.
    SeqNum(u64),
}

/// A configurable reader for journal entries with filtering and seeking.
///
/// # Example
///
/// ```rust,no_run
/// use libsystemd::journal::storage::{JournalStorage, StorageConfig, SeekPosition};
/// use libsystemd::journal::entry::FieldMatch;
///
/// let storage = JournalStorage::new(StorageConfig::default()).unwrap();
/// let mut reader = storage.reader();
/// reader.add_match(FieldMatch::Exact {
///     field: "_SYSTEMD_UNIT".into(),
///     value: b"sshd.service".to_vec(),
/// });
/// reader.add_match(FieldMatch::PriorityAtMost(4));
/// reader.seek = SeekPosition::RealtimeTimestamp(1_700_000_000_000_000);
///
/// let entries = reader.collect().unwrap();
/// for entry in &entries {
///     println!("{}", entry);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct JournalReader {
    /// Journal directory (e.g. `/var/log/journal/<machine-id>`).
    directory: PathBuf,
    /// Filters to apply to each entry.  Only entries matching **all**
    /// filters are returned.
    pub filters: Vec<FieldMatch>,
    /// Where to start reading.
    pub seek: SeekPosition,
}

impl JournalReader {
    /// Create a reader directly from a directory path.
    pub fn open(directory: PathBuf) -> Self {
        JournalReader {
            directory,
            filters: Vec::new(),
            seek: SeekPosition::Head,
        }
    }

    /// Add a filter.  Entries must match **all** added filters.
    pub fn add_match(&mut self, m: FieldMatch) {
        self.filters.push(m);
    }

    /// Set the seek position.
    pub fn set_seek(&mut self, pos: SeekPosition) {
        self.seek = pos;
    }

    /// Convenience: add an exact field match.
    pub fn match_field(&mut self, field: &str, value: &[u8]) {
        self.filters.push(FieldMatch::Exact {
            field: field.to_string(),
            value: value.to_vec(),
        });
    }

    /// Convenience: filter by systemd unit name.
    pub fn match_unit(&mut self, unit: &str) {
        self.match_field("_SYSTEMD_UNIT", unit.as_bytes());
    }

    /// Convenience: filter by syslog identifier.
    pub fn match_identifier(&mut self, ident: &str) {
        self.match_field("SYSLOG_IDENTIFIER", ident.as_bytes());
    }

    /// Convenience: filter by maximum priority level.
    pub fn match_priority(&mut self, max_priority: u8) {
        self.filters.push(FieldMatch::PriorityAtMost(max_priority));
    }

    /// Collect all matching entries into a `Vec`.
    ///
    /// Entries are returned in chronological order (oldest first) unless
    /// [`SeekPosition::Tail`] is used, in which case they are reversed.
    pub fn collect(&self) -> io::Result<Vec<JournalEntry>> {
        // Read all entries from all files in chronological order.
        let mut all_entries = read_all_from_directory(&self.directory)?;

        // Apply seek position.
        let reverse = matches!(self.seek, SeekPosition::Tail);
        all_entries = self.apply_seek(all_entries);

        // Apply filters.
        if !self.filters.is_empty() {
            all_entries.retain(|e| e.matches_all(&self.filters));
        }

        if reverse {
            all_entries.reverse();
        }

        Ok(all_entries)
    }

    /// Collect at most `n` matching entries.
    ///
    /// For [`SeekPosition::Tail`], this returns the last `n` entries
    /// (newest first).  For all other seek positions it returns the
    /// first `n` matching entries (oldest first).
    pub fn collect_n(&self, n: usize) -> io::Result<Vec<JournalEntry>> {
        let mut entries = self.collect()?;
        entries.truncate(n);
        Ok(entries)
    }

    /// Count matching entries without collecting them into a `Vec`.
    pub fn count(&self) -> io::Result<u64> {
        Ok(self.collect()?.len() as u64)
    }

    /// Collect all unique values of the given field across matching entries.
    pub fn unique_field_values(&self, field: &str) -> io::Result<Vec<String>> {
        let entries = self.collect()?;
        let mut seen = std::collections::BTreeSet::new();
        for entry in &entries {
            if let Some(val) = entry.field(field) {
                seen.insert(val);
            }
        }
        Ok(seen.into_iter().collect())
    }

    // ---------------------------------------------------------------
    // Internal helpers
    // ---------------------------------------------------------------

    fn apply_seek(&self, mut entries: Vec<JournalEntry>) -> Vec<JournalEntry> {
        match &self.seek {
            SeekPosition::Head => entries,
            SeekPosition::Tail => entries,
            SeekPosition::Cursor(cursor) => {
                if let Some(target_seqnum) = parse_cursor_seqnum(cursor) {
                    entries.retain(|e| e.seqnum >= target_seqnum);
                }
                entries
            }
            SeekPosition::AfterCursor(cursor) => {
                if let Some(target_seqnum) = parse_cursor_seqnum(cursor) {
                    entries.retain(|e| e.seqnum > target_seqnum);
                }
                entries
            }
            SeekPosition::RealtimeTimestamp(ts) => {
                entries.retain(|e| e.realtime_usec >= *ts);
                entries
            }
            SeekPosition::SeqNum(seq) => {
                entries.retain(|e| e.seqnum >= *seq);
                entries
            }
        }
    }
}

/// Read all entries from a specific list of journal files, sorted chronologically.
///
/// Supports both the native JRNL_RS format and C systemd's LPKSHHRH format.
fn read_all_from_files(files: &[PathBuf]) -> io::Result<Vec<JournalEntry>> {
    let mut all_entries = Vec::new();

    for file_path in files {
        match JournalFile::open(file_path, false) {
            Ok(jf) => match jf.read_all() {
                Ok(entries) => all_entries.extend(entries),
                Err(e) => {
                    eprintln!(
                        "journald: Warning: could not read {}: {}",
                        file_path.display(),
                        e
                    );
                }
            },
            Err(_) => {
                // JRNL_RS open failed — try C journal format (LPKSHHRH)
                match c_journal::read_c_journal(file_path) {
                    Ok(entries) => all_entries.extend(entries),
                    Err(e) => {
                        eprintln!(
                            "journald: Warning: could not open {}: {}",
                            file_path.display(),
                            e
                        );
                    }
                }
            }
        }
    }

    // Sort by realtime timestamp, then by sequence number for stability
    all_entries.sort_by(|a, b| {
        a.realtime_usec
            .cmp(&b.realtime_usec)
            .then_with(|| a.seqnum.cmp(&b.seqnum))
    });

    Ok(all_entries)
}

/// Read all entries from all journal files in a directory, sorted chronologically.
///
/// Supports both the native JRNL_RS format and C systemd's LPKSHHRH format.
/// Files are tried as JRNL_RS first; on failure, they are retried as C journal
/// files before being skipped with a warning.
fn read_all_from_directory(directory: &Path) -> io::Result<Vec<JournalEntry>> {
    let mut all_entries = Vec::new();
    // Include both .journal and .journal~ (dirty/unclean) files for reading.
    // list_journal_files() only returns .journal files (used by the write path),
    // so we also scan for .journal~ files here.
    let mut files = list_journal_files(directory)?;
    if directory.exists() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "journal~") && path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();

    for file_path in &files {
        match JournalFile::open(file_path, false) {
            Ok(jf) => match jf.read_all() {
                Ok(entries) => all_entries.extend(entries),
                Err(e) => {
                    eprintln!(
                        "journald: Warning: could not read {}: {}",
                        file_path.display(),
                        e
                    );
                }
            },
            Err(_) => {
                // JRNL_RS open failed — try C journal format (LPKSHHRH)
                match c_journal::read_c_journal(file_path) {
                    Ok(entries) => all_entries.extend(entries),
                    Err(e) => {
                        eprintln!(
                            "journald: Warning: could not open {}: {}",
                            file_path.display(),
                            e
                        );
                    }
                }
            }
        }
    }

    // Sort by realtime timestamp, then by sequence number for stability
    all_entries.sort_by(|a, b| {
        a.realtime_usec
            .cmp(&b.realtime_usec)
            .then_with(|| a.seqnum.cmp(&b.seqnum))
    });

    Ok(all_entries)
}

/// Parse the sequence number (`i=HEX`) from a cursor string.
///
/// Cursor format:
/// `s=<file_id>;i=<seqnum>;b=<boot_id>;m=<monotonic>;t=<realtime>;x=<xor_hash>`
fn parse_cursor_seqnum(cursor: &str) -> Option<u64> {
    for part in cursor.split(';') {
        if let Some(hex) = part.strip_prefix("i=") {
            return u64::from_str_radix(hex.trim(), 16).ok();
        }
    }
    None
}

/// C journal header incompatible flags for compression algorithms.
const C_HEADER_INCOMPATIBLE_COMPRESSED_XZ: u32 = 1 << 0;
const C_HEADER_INCOMPATIBLE_COMPRESSED_LZ4: u32 = 1 << 1;
// bit 2 = KEYED_HASH (not compression)
const C_HEADER_INCOMPATIBLE_COMPRESSED_ZSTD: u32 = 1 << 3;

/// C journal magic signature.
const C_JOURNAL_MAGIC: &[u8; 8] = b"LPKSHHRH";

/// Read the compression algorithm from a journal file's header.
///
/// Supports both JRNL_RS format (compress byte at offset 60) and
/// C systemd's LPKSHHRH format (incompatible_flags at offset 12).
pub fn read_file_compress(path: &Path) -> io::Result<JournalCompress> {
    let mut file = File::open(path)?;
    let mut header_buf = [0u8; 64];
    file.read_exact(&mut header_buf)?;

    let mut magic = [0u8; 8];
    magic.copy_from_slice(&header_buf[0..8]);

    if &magic == JOURNAL_MAGIC {
        let header = FileHeader::deserialize(&header_buf)?;
        Ok(header.compress)
    } else if &magic == C_JOURNAL_MAGIC {
        // C journal: compression is encoded in incompatible_flags at offset 12
        let incompatible_flags = u32::from_le_bytes(header_buf[12..16].try_into().unwrap());
        if incompatible_flags & C_HEADER_INCOMPATIBLE_COMPRESSED_ZSTD != 0 {
            Ok(JournalCompress::Zstd)
        } else if incompatible_flags & C_HEADER_INCOMPATIBLE_COMPRESSED_LZ4 != 0 {
            Ok(JournalCompress::Lz4)
        } else if incompatible_flags & C_HEADER_INCOMPATIBLE_COMPRESSED_XZ != 0 {
            Ok(JournalCompress::Xz)
        } else {
            Ok(JournalCompress::None)
        }
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unrecognized journal file magic",
        ))
    }
}

/// Parse the realtime timestamp (`t=HEX`) from a cursor string.
pub fn parse_cursor_realtime(cursor: &str) -> Option<u64> {
    for part in cursor.split(';') {
        if let Some(hex) = part.strip_prefix("t=") {
            return u64::from_str_radix(hex.trim(), 16).ok();
        }
    }
    None
}

/// Parse the boot ID (`b=HEX`) from a cursor string.
pub fn parse_cursor_boot_id(cursor: &str) -> Option<String> {
    for part in cursor.split(';') {
        if let Some(id) = part.strip_prefix("b=") {
            let trimmed = id.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Read JRNL_RS entries from a journal file starting at a given byte offset.
///
/// Returns the entries read and the new byte offset (end of last entry read).
/// This is used by follow mode for incremental reads — on each poll cycle,
/// only new entries appended since the last offset are read.
pub fn read_entries_from_offset(path: &Path, offset: u64) -> io::Result<(Vec<JournalEntry>, u64)> {
    let file = File::open(path)?;
    let file_len = file.metadata()?.len();
    if offset >= file_len {
        return Ok((Vec::new(), offset));
    }
    let mut reader = BufReader::new(file);
    let start = if offset < HEADER_SIZE {
        HEADER_SIZE
    } else {
        offset
    };
    reader.seek(SeekFrom::Start(start))?;

    let mut entries = Vec::new();
    loop {
        match deserialize_entry(&mut reader) {
            Ok(entry) => entries.push(entry),
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(_) => break,
        }
    }

    let end_offset = reader.stream_position().unwrap_or(file_len);
    Ok((entries, end_offset))
}

/// List journal files (`.journal` and `.journal~`) in a directory.
pub fn list_all_journal_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = list_journal_files(dir)?;
    if dir.exists() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "journal~") && path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_entry(msg: &str, priority: u8, seqnum: u64) -> JournalEntry {
        let mut entry =
            JournalEntry::with_timestamp(1_700_000_000_000_000 + seqnum * 1_000_000, seqnum * 1000);
        entry.seqnum = seqnum;
        entry.set_field("MESSAGE", msg);
        entry.set_field("PRIORITY", priority.to_string());
        entry.set_field("SYSLOG_IDENTIFIER", "test");
        entry.set_field("_PID", "42");
        entry
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("journald_test_{}", name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_header_roundtrip() {
        let header = FileHeader::new();
        let bytes = header.serialize();
        let header2 = FileHeader::deserialize(&bytes).unwrap();

        assert_eq!(header.magic, header2.magic);
        assert_eq!(header.version, header2.version);
        assert_eq!(header.file_id, header2.file_id);
        assert_eq!(header.head_seqnum, header2.head_seqnum);
        assert_eq!(header.tail_seqnum, header2.tail_seqnum);
        assert_eq!(header.entry_count, header2.entry_count);
        assert_eq!(header.file_size, header2.file_size);
    }

    #[test]
    fn test_header_bad_magic() {
        let mut buf = [0u8; 64];
        buf[0..8].copy_from_slice(b"INVALID\0");
        assert!(FileHeader::deserialize(&buf).is_err());
    }

    #[test]
    fn test_entry_serialize_deserialize() {
        let entry = make_test_entry("hello world", 6, 1);
        let frame = serialize_entry(&entry, 1);

        let mut reader = io::Cursor::new(&frame);
        let entry2 = deserialize_entry(&mut reader).unwrap();

        assert_eq!(entry.realtime_usec, entry2.realtime_usec);
        assert_eq!(entry.monotonic_usec, entry2.monotonic_usec);
        assert_eq!(entry2.seqnum, 1);
        assert_eq!(entry2.message(), Some("hello world".to_string()));
        assert_eq!(entry2.priority(), Some(6));
        assert_eq!(entry2.field("SYSLOG_IDENTIFIER"), Some("test".to_string()));
        assert_eq!(entry2.pid(), Some(42));
    }

    #[test]
    fn test_entry_serialize_empty() {
        let entry = JournalEntry::with_timestamp(100, 200);
        let frame = serialize_entry(&entry, 0);

        let mut reader = io::Cursor::new(&frame);
        let entry2 = deserialize_entry(&mut reader).unwrap();

        assert_eq!(entry2.realtime_usec, 100);
        assert_eq!(entry2.monotonic_usec, 200);
        assert_eq!(entry2.seqnum, 0);
        assert!(entry2.fields.is_empty());
    }

    #[test]
    fn test_entry_serialize_binary_value() {
        let mut entry = JournalEntry::with_timestamp(1000, 2000);
        entry.set_field_bytes("BINARY", vec![0x00, 0x01, 0x0a, 0xff, 0x80]);
        entry.set_field("TEXT", "normal text");

        let frame = serialize_entry(&entry, 5);
        let mut reader = io::Cursor::new(&frame);
        let entry2 = deserialize_entry(&mut reader).unwrap();

        assert_eq!(
            entry2.field_bytes("BINARY"),
            Some(&[0x00, 0x01, 0x0a, 0xff, 0x80][..])
        );
        assert_eq!(entry2.field("TEXT"), Some("normal text".to_string()));
    }

    #[test]
    fn test_entry_serialize_many_fields() {
        let mut entry = JournalEntry::with_timestamp(42, 43);
        for i in 0..100 {
            entry.set_field(format!("FIELD_{}", i), format!("value_{}", i));
        }

        let frame = serialize_entry(&entry, 99);
        let mut reader = io::Cursor::new(&frame);
        let entry2 = deserialize_entry(&mut reader).unwrap();

        assert_eq!(entry2.fields.len(), 100);
        for i in 0..100 {
            assert_eq!(
                entry2.field(&format!("FIELD_{}", i)),
                Some(format!("value_{}", i))
            );
        }
    }

    #[test]
    fn test_multiple_entries_in_sequence() {
        let entries: Vec<JournalEntry> = (1..=5)
            .map(|i| make_test_entry(&format!("msg {}", i), 6, i))
            .collect();

        let mut buf = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            buf.extend_from_slice(&serialize_entry(entry, (i + 1) as u64));
        }

        let mut reader = io::Cursor::new(&buf);
        let mut deserialized = Vec::new();
        while let Ok(entry) = deserialize_entry(&mut reader) {
            deserialized.push(entry);
        }

        assert_eq!(deserialized.len(), 5);
        for (i, entry) in deserialized.iter().enumerate() {
            assert_eq!(entry.message(), Some(format!("msg {}", i + 1)));
            assert_eq!(entry.seqnum, (i + 1) as u64);
        }
    }

    #[test]
    fn test_frame_length_mismatch_detected() {
        let entry = make_test_entry("test", 6, 1);
        let mut frame = serialize_entry(&entry, 1);

        // Corrupt the trailing length
        let len = frame.len();
        frame[len - 1] = 0xFF;
        frame[len - 2] = 0xFF;

        let mut reader = io::Cursor::new(&frame);
        let result = deserialize_entry(&mut reader);
        assert!(result.is_err());
    }

    #[test]
    fn test_journal_file_create_and_read() {
        let dir = temp_dir("file_create");
        let path = dir.join("test.journal");

        let mut jf = JournalFile::create(&path).unwrap();
        assert_eq!(jf.header.entry_count, 0);

        let entry1 = make_test_entry("first entry", 6, 1);
        jf.append(&entry1, 1).unwrap();

        let entry2 = make_test_entry("second entry", 4, 2);
        jf.append(&entry2, 2).unwrap();

        assert_eq!(jf.header.entry_count, 2);
        assert_eq!(jf.header.head_seqnum, 1);
        assert_eq!(jf.header.tail_seqnum, 2);

        // Read back
        let entries = jf.read_all().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message(), Some("first entry".to_string()));
        assert_eq!(entries[1].message(), Some("second entry".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_journal_file_reopen_and_read() {
        let dir = temp_dir("file_reopen");
        let path = dir.join("test.journal");

        // Create and write
        {
            let mut jf = JournalFile::create(&path).unwrap();
            let entry = make_test_entry("persisted entry", 6, 1);
            jf.append(&entry, 1).unwrap();
        }

        // Reopen and verify
        {
            let jf = JournalFile::open(&path, false).unwrap();
            assert_eq!(jf.header.entry_count, 1);
            assert_eq!(jf.header.head_seqnum, 1);
            assert_eq!(jf.header.tail_seqnum, 1);

            let entries = jf.read_all().unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].message(), Some("persisted entry".to_string()));
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_journal_file_append_after_reopen() {
        let dir = temp_dir("file_append_reopen");
        let path = dir.join("test.journal");

        // Create and write one entry
        {
            let mut jf = JournalFile::create(&path).unwrap();
            jf.append(&make_test_entry("entry 1", 6, 1), 1).unwrap();
        }

        // Reopen for appending and add another entry
        {
            let mut jf = JournalFile::open(&path, true).unwrap();
            jf.append(&make_test_entry("entry 2", 4, 2), 2).unwrap();
        }

        // Read all
        {
            let jf = JournalFile::open(&path, false).unwrap();
            let entries = jf.read_all().unwrap();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].message(), Some("entry 1".to_string()));
            assert_eq!(entries[1].message(), Some("entry 2".to_string()));
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_storage_basic() {
        let dir = temp_dir("storage_basic");
        let config = StorageConfig {
            directory: dir.clone(),
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            max_disk_usage: DEFAULT_MAX_DISK_USAGE,
            max_files: DEFAULT_MAX_FILES,
            persistent: false,
            keep_free: 0,
            direct_directory: false,
        };

        let mut storage = JournalStorage::new(config).unwrap();

        // Write some entries
        for i in 1..=10 {
            let entry = make_test_entry(&format!("log message {}", i), 6, i);
            let seqnum = storage.append(&entry).unwrap();
            assert_eq!(seqnum, i);
        }

        // Read all entries back
        let entries = storage.read_all().unwrap();
        assert_eq!(entries.len(), 10);
        for (i, entry) in entries.iter().enumerate() {
            assert_eq!(entry.message(), Some(format!("log message {}", i + 1)));
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_storage_rotation() {
        let dir = temp_dir("storage_rotation");
        let config = StorageConfig {
            directory: dir.clone(),
            // Very small max file size to force rotation
            max_file_size: 256,
            max_disk_usage: DEFAULT_MAX_DISK_USAGE,
            max_files: DEFAULT_MAX_FILES,
            persistent: false,
            keep_free: 0,
            direct_directory: false,
        };

        let mut storage = JournalStorage::new(config).unwrap();

        // Write enough entries to trigger rotation
        for i in 1..=20 {
            let entry = make_test_entry(&format!("rotation test {}", i), 6, i);
            storage.append(&entry).unwrap();
        }

        // Should have multiple files now
        let file_count = storage.file_count().unwrap();
        assert!(
            file_count > 1,
            "expected more than 1 file after rotation, got {}",
            file_count
        );

        // All entries should still be readable
        let entries = storage.read_all().unwrap();
        assert_eq!(entries.len(), 20);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_storage_vacuum_file_count() {
        let dir = temp_dir("storage_vacuum_count");
        let config = StorageConfig {
            directory: dir.clone(),
            max_file_size: 256,
            max_disk_usage: DEFAULT_MAX_DISK_USAGE,
            max_files: 3,
            persistent: false,
            keep_free: 0,
            direct_directory: false,
        };

        let mut storage = JournalStorage::new(config).unwrap();

        // Write many entries to create many files
        for i in 1..=50 {
            let entry = make_test_entry(&format!("vacuum test {}", i), 6, i);
            storage.append(&entry).unwrap();
        }

        // File count should be limited
        let file_count = storage.file_count().unwrap();
        assert!(
            file_count <= 4, // 3 limit + 1 active being written
            "expected <= 4 files, got {}",
            file_count
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_storage_disk_usage() {
        let dir = temp_dir("storage_disk_usage");
        let config = StorageConfig {
            directory: dir.clone(),
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            max_disk_usage: DEFAULT_MAX_DISK_USAGE,
            max_files: DEFAULT_MAX_FILES,
            persistent: false,
            keep_free: 0,
            direct_directory: false,
        };

        let mut storage = JournalStorage::new(config).unwrap();

        // Write some entries and check disk usage
        let entry = make_test_entry("disk usage test", 6, 1);
        storage.append(&entry).unwrap();

        let usage = storage.disk_usage().unwrap();
        assert!(usage > 0, "disk usage should be > 0");
        // At minimum, we have the header + one entry frame
        assert!(usage >= HEADER_SIZE);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_storage_make_cursor() {
        let dir = temp_dir("storage_cursor");
        let config = StorageConfig {
            directory: dir.clone(),
            ..StorageConfig::default()
        };

        let mut storage = JournalStorage::new(config).unwrap();
        let mut entry = make_test_entry("cursor test", 6, 1);
        entry.set_field("_BOOT_ID", "abcdef0123456789abcdef0123456789");
        storage.append(&entry).unwrap();

        let cursor = storage.make_cursor(&entry);
        assert!(cursor.contains("s="));
        assert!(cursor.contains("i="));
        assert!(cursor.contains("b="));
        assert!(cursor.contains("m="));
        assert!(cursor.contains("t="));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_storage_flush() {
        let dir = temp_dir("storage_flush");
        let config = StorageConfig {
            directory: dir.clone(),
            ..StorageConfig::default()
        };

        let mut storage = JournalStorage::new(config).unwrap();
        let entry = make_test_entry("flush test", 6, 1);
        storage.append(&entry).unwrap();
        // flush should not panic or error
        storage.flush().unwrap();

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_storage_explicit_rotation() {
        let dir = temp_dir("storage_explicit_rotate");
        let config = StorageConfig {
            directory: dir.clone(),
            ..StorageConfig::default()
        };

        let mut storage = JournalStorage::new(config).unwrap();

        let entry1 = make_test_entry("before rotation", 6, 1);
        storage.append(&entry1).unwrap();

        storage.rotate().unwrap();

        let entry2 = make_test_entry("after rotation", 6, 2);
        storage.append(&entry2).unwrap();

        // Both entries should be readable
        let entries = storage.read_all().unwrap();
        assert_eq!(entries.len(), 2);

        let file_count = storage.file_count().unwrap();
        assert_eq!(file_count, 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_list_journal_files_empty_dir() {
        let dir = temp_dir("list_empty");
        let files = list_journal_files(&dir).unwrap();
        assert!(files.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_list_journal_files_nonexistent_dir() {
        let dir = PathBuf::from("/tmp/journald_test_nonexistent_dir_xyz");
        let files = list_journal_files(&dir).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_list_journal_files_filters_non_journal() {
        let dir = temp_dir("list_filter");
        fs::write(dir.join("test.journal"), "data").unwrap();
        fs::write(dir.join("test.txt"), "not a journal").unwrap();
        fs::write(dir.join("other.log"), "also not").unwrap();

        let files = list_journal_files(&dir).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("test.journal"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_generate_random_u128() {
        let a = generate_random_u128();
        let b = generate_random_u128();
        // They should almost certainly be different
        assert_ne!(a, b, "two random u128 values should differ");
    }

    // ---------------------------------------------------------------
    // Cursor parsing tests
    // ---------------------------------------------------------------

    #[test]
    fn test_parse_cursor_seqnum() {
        let cursor = "s=00000000000000000000000000000000;i=a;b=abc123;m=64;t=60b6a7c0e4240;x=0";
        assert_eq!(parse_cursor_seqnum(cursor), Some(0xa));
    }

    #[test]
    fn test_parse_cursor_seqnum_missing() {
        assert_eq!(parse_cursor_seqnum("s=0;b=abc;m=0;t=0;x=0"), None);
    }

    #[test]
    fn test_parse_cursor_realtime() {
        let cursor = "s=0;i=1;b=abc;m=0;t=60b6a7c0e4240;x=0";
        assert_eq!(parse_cursor_realtime(cursor), Some(0x60b6a7c0e4240));
    }

    #[test]
    fn test_parse_cursor_boot_id() {
        let cursor = "s=0;i=1;b=abc123def;m=0;t=0;x=0";
        assert_eq!(parse_cursor_boot_id(cursor), Some("abc123def".to_string()));
    }

    #[test]
    fn test_parse_cursor_boot_id_empty() {
        let cursor = "s=0;i=1;b=;m=0;t=0;x=0";
        assert_eq!(parse_cursor_boot_id(cursor), None);
    }

    // ---------------------------------------------------------------
    // JournalReader tests
    // ---------------------------------------------------------------

    fn make_reader_storage(name: &str) -> (PathBuf, JournalStorage) {
        let dir = temp_dir(name);
        let config = StorageConfig {
            directory: dir.clone(),
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            max_disk_usage: DEFAULT_MAX_DISK_USAGE,
            max_files: DEFAULT_MAX_FILES,
            persistent: false,
            keep_free: 0,
            direct_directory: false,
        };
        let storage = JournalStorage::new(config).unwrap();
        (dir, storage)
    }

    fn populate_storage(storage: &mut JournalStorage) {
        // Write 10 entries with different priorities and units
        for i in 1..=10 {
            let mut entry =
                JournalEntry::with_timestamp(1_700_000_000_000_000 + i * 1_000_000, i * 1000);
            entry.set_field("MESSAGE", format!("message {}", i));
            entry.set_field("PRIORITY", (i % 8).to_string());
            if i <= 5 {
                entry.set_field("_SYSTEMD_UNIT", "foo.service");
            } else {
                entry.set_field("_SYSTEMD_UNIT", "bar.service");
            }
            entry.set_field("SYSLOG_IDENTIFIER", format!("app{}", i % 3));
            storage.append(&entry).unwrap();
        }
    }

    #[test]
    fn test_reader_collect_all() {
        let (dir, mut storage) = make_reader_storage("reader_all");
        populate_storage(&mut storage);

        let reader = storage.reader();
        let entries = reader.collect().unwrap();
        assert_eq!(entries.len(), 10);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reader_filter_by_unit() {
        let (dir, mut storage) = make_reader_storage("reader_unit");
        populate_storage(&mut storage);

        let mut reader = storage.reader();
        reader.match_unit("foo.service");
        let entries = reader.collect().unwrap();
        assert_eq!(entries.len(), 5);
        for e in &entries {
            assert_eq!(e.systemd_unit(), Some("foo.service".to_string()));
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reader_filter_by_priority() {
        let (dir, mut storage) = make_reader_storage("reader_prio");
        populate_storage(&mut storage);

        let mut reader = storage.reader();
        reader.match_priority(3); // emerg(0), alert(1), crit(2), err(3)
        let entries = reader.collect().unwrap();
        // Priorities 1..=10 mod 8 = 1,2,3,4,5,6,7,0,1,2
        // ≤3: indices with priority 1,2,3,0 → entries 1,2,3,8,9,10
        assert_eq!(entries.len(), 6);
        for e in &entries {
            assert!(e.priority().unwrap() <= 3);
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reader_filter_combined() {
        let (dir, mut storage) = make_reader_storage("reader_combined");
        populate_storage(&mut storage);

        let mut reader = storage.reader();
        reader.match_unit("foo.service");
        reader.match_priority(3);
        let entries = reader.collect().unwrap();
        // foo.service = entries 1-5, priorities 1,2,3,4,5
        // priority <= 3: entries 1,2,3
        assert_eq!(entries.len(), 3);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reader_seek_realtime() {
        let (dir, mut storage) = make_reader_storage("reader_seek_rt");
        populate_storage(&mut storage);

        let mut reader = storage.reader();
        // Seek to entry 6 (timestamp = base + 6_000_000)
        reader.seek = SeekPosition::RealtimeTimestamp(1_700_000_000_000_000 + 6_000_000);
        let entries = reader.collect().unwrap();
        assert_eq!(entries.len(), 5); // entries 6..=10
        assert_eq!(entries[0].message(), Some("message 6".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reader_seek_seqnum() {
        let (dir, mut storage) = make_reader_storage("reader_seek_seq");
        populate_storage(&mut storage);

        let mut reader = storage.reader();
        reader.seek = SeekPosition::SeqNum(8);
        let entries = reader.collect().unwrap();
        assert_eq!(entries.len(), 3); // entries 8,9,10
        assert_eq!(entries[0].message(), Some("message 8".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reader_seek_tail() {
        let (dir, mut storage) = make_reader_storage("reader_seek_tail");
        populate_storage(&mut storage);

        let mut reader = storage.reader();
        reader.seek = SeekPosition::Tail;
        let entries = reader.collect().unwrap();
        assert_eq!(entries.len(), 10);
        // Tail seek returns entries in reverse order
        assert_eq!(entries[0].message(), Some("message 10".to_string()));
        assert_eq!(entries[9].message(), Some("message 1".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reader_collect_n() {
        let (dir, mut storage) = make_reader_storage("reader_collect_n");
        populate_storage(&mut storage);

        let reader = storage.reader();
        let entries = reader.collect_n(3).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].message(), Some("message 1".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reader_collect_n_tail() {
        let (dir, mut storage) = make_reader_storage("reader_collect_n_tail");
        populate_storage(&mut storage);

        let mut reader = storage.reader();
        reader.seek = SeekPosition::Tail;
        let entries = reader.collect_n(3).unwrap();
        assert_eq!(entries.len(), 3);
        // Newest first
        assert_eq!(entries[0].message(), Some("message 10".to_string()));
        assert_eq!(entries[2].message(), Some("message 8".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reader_count() {
        let (dir, mut storage) = make_reader_storage("reader_count");
        populate_storage(&mut storage);

        let mut reader = storage.reader();
        reader.match_unit("bar.service");
        assert_eq!(reader.count().unwrap(), 5);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reader_unique_field_values() {
        let (dir, mut storage) = make_reader_storage("reader_unique");
        populate_storage(&mut storage);

        let reader = storage.reader();
        let units = reader.unique_field_values("_SYSTEMD_UNIT").unwrap();
        assert_eq!(units.len(), 2);
        assert!(units.contains(&"foo.service".to_string()));
        assert!(units.contains(&"bar.service".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reader_match_identifier() {
        let (dir, mut storage) = make_reader_storage("reader_ident");
        populate_storage(&mut storage);

        let mut reader = storage.reader();
        reader.match_identifier("app0");
        let entries = reader.collect().unwrap();
        // app0 for i%3==0: entries 3,6,9
        assert_eq!(entries.len(), 3);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reader_empty_storage() {
        let (dir, storage) = make_reader_storage("reader_empty");

        let reader = storage.reader();
        let entries = reader.collect().unwrap();
        assert!(entries.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------
    // Storage convenience method tests
    // ---------------------------------------------------------------

    #[test]
    fn test_storage_read_filtered() {
        let (dir, mut storage) = make_reader_storage("storage_filtered");
        populate_storage(&mut storage);

        let filters = vec![FieldMatch::Exact {
            field: "_SYSTEMD_UNIT".to_string(),
            value: b"bar.service".to_vec(),
        }];
        let entries = storage.read_filtered(&filters).unwrap();
        assert_eq!(entries.len(), 5);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_storage_read_from_cursor() {
        let (dir, mut storage) = make_reader_storage("storage_read_cursor");
        populate_storage(&mut storage);

        // Read all to get a cursor for entry 5
        let all = storage.read_all().unwrap();
        let cursor = storage.make_cursor(&all[4]); // entry 5 (0-indexed: 4)

        let entries = storage.read_from_cursor(&cursor, false).unwrap();
        // Should include entry 5 and everything after
        assert!(entries.len() >= 6); // entries 5..=10

        let entries_after = storage.read_from_cursor(&cursor, true).unwrap();
        // Should exclude entry 5 itself
        assert!(entries_after.len() < entries.len());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_storage_read_from_realtime() {
        let (dir, mut storage) = make_reader_storage("storage_realtime");
        populate_storage(&mut storage);

        let entries = storage
            .read_from_realtime(1_700_000_000_000_000 + 8_000_000)
            .unwrap();
        assert_eq!(entries.len(), 3); // entries 8,9,10

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_storage_export_all() {
        let (dir, mut storage) = make_reader_storage("storage_export");
        populate_storage(&mut storage);

        let mut buf = Vec::new();
        let count = storage.export_all(&mut buf).unwrap();
        assert_eq!(count, 10);

        let export_str = String::from_utf8_lossy(&buf);
        assert!(export_str.contains("__REALTIME_TIMESTAMP="));
        assert!(export_str.contains("MESSAGE=message 1"));
        assert!(export_str.contains("MESSAGE=message 10"));

        // Verify we can parse the export format back
        let mut cursor = io::Cursor::new(&buf);
        let parsed =
            super::super::entry::parse_export_entries(&mut io::BufReader::new(&mut cursor))
                .unwrap();
        assert_eq!(parsed.len(), 10);
        assert_eq!(parsed[0].message(), Some("message 1".to_string()));
        assert_eq!(parsed[9].message(), Some("message 10".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------
    // JournalReader::open tests
    // ---------------------------------------------------------------

    #[test]
    fn test_reader_open_directly() {
        let (dir, mut storage) = make_reader_storage("reader_open");
        populate_storage(&mut storage);

        let journal_dir = storage.directory();
        let mut reader = JournalReader::open(journal_dir);
        reader.match_priority(2); // emerg(0), alert(1), crit(2)
        let entries = reader.collect().unwrap();
        // Priorities: 1,2,3,4,5,6,7,0,1,2 → ≤2: entries with pri 1,2,0,1,2 → 5 entries
        assert_eq!(entries.len(), 5);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reader_add_match() {
        let (dir, mut storage) = make_reader_storage("reader_add_match");
        populate_storage(&mut storage);

        let mut reader = storage.reader();
        reader.add_match(FieldMatch::Exact {
            field: "_SYSTEMD_UNIT".to_string(),
            value: b"foo.service".to_vec(),
        });
        reader.add_match(FieldMatch::SinceRealtime(1_700_000_000_000_000 + 3_000_000));
        let entries = reader.collect().unwrap();
        // foo.service = entries 1-5, since entry 3 → entries 3,4,5
        assert_eq!(entries.len(), 3);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_reader_match_field_bytes() {
        let (dir, mut storage) = make_reader_storage("reader_match_bytes");
        populate_storage(&mut storage);

        let mut reader = storage.reader();
        reader.match_field("MESSAGE", b"message 7");
        let entries = reader.collect().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message(), Some("message 7".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }
}
