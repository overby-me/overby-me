//! Read-only support for the C systemd journal file format (LPKSHHRH).
//!
//! This module parses journal files written by the original C `systemd-journald`
//! daemon, allowing `journalctl` (and other Rust consumers) to read entries
//! regardless of which journald implementation wrote them.
//!
//! The C journal format uses a complex on-disk layout with hash tables,
//! object headers, and entry arrays.  We only implement the *read path* —
//! just enough to iterate entries and extract their fields.

use super::entry::JournalEntry;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Magic signature for C systemd journal files.
const C_JOURNAL_MAGIC: &[u8; 8] = b"LPKSHHRH";

/// Minimum header size (fields through tail_entry_monotonic, offset 208).
const MIN_HEADER_SIZE: usize = 208;

// Header incompatible flags
const HEADER_INCOMPATIBLE_COMPACT: u32 = 1 << 4;

// Object types
const OBJECT_DATA: u8 = 1;
const OBJECT_ENTRY: u8 = 3;
const OBJECT_ENTRY_ARRAY: u8 = 6;

// Object compression flags
const OBJECT_COMPRESSED_XZ: u8 = 1 << 0;
const OBJECT_COMPRESSED_LZ4: u8 = 1 << 1;
const OBJECT_COMPRESSED_ZSTD: u8 = 1 << 2;
const OBJECT_COMPRESSION_MASK: u8 =
    OBJECT_COMPRESSED_XZ | OBJECT_COMPRESSED_LZ4 | OBJECT_COMPRESSED_ZSTD;

// Object header size (type + flags + reserved + size = 1+1+6+8 = 16)
const OBJECT_HEADER_SIZE: usize = 16;

// Entry object: fixed fields after object header
// seqnum(8) + realtime(8) + monotonic(8) + boot_id(16) + xor_hash(8) = 48
const ENTRY_FIXED_SIZE: usize = 48;

// Data object: fixed fields after object header
// hash(8) + next_hash(8) + next_field(8) + entry_offset(8) + entry_array(8) + n_entries(8) = 48
const DATA_FIXED_SIZE: usize = 48;

// Entry array: fixed fields after object header (next_entry_array_offset = 8)
const ENTRY_ARRAY_FIXED_SIZE: usize = 8;

// ---------------------------------------------------------------------------
// Header parsing
// ---------------------------------------------------------------------------

struct CJournalHeader {
    incompatible_flags: u32,
    entry_array_offset: u64,
}

impl CJournalHeader {
    fn is_compact(&self) -> bool {
        self.incompatible_flags & HEADER_INCOMPATIBLE_COMPACT != 0
    }
}

fn r32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

fn r64(data: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(data[off..off + 8].try_into().unwrap())
}

fn parse_header(data: &[u8]) -> io::Result<CJournalHeader> {
    if data.len() < MIN_HEADER_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file too small for C journal header",
        ));
    }
    if &data[0..8] != C_JOURNAL_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a C journal file",
        ));
    }

    Ok(CJournalHeader {
        incompatible_flags: r32(data, 12),
        entry_array_offset: r64(data, 176),
    })
}

// ---------------------------------------------------------------------------
// Object reading helpers
// ---------------------------------------------------------------------------

/// Read an object header at the given file offset.
/// Returns (type, flags, size).
fn read_object_header(data: &[u8], offset: u64) -> io::Result<(u8, u8, u64)> {
    let off = offset as usize;
    if off + OBJECT_HEADER_SIZE > data.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("object header at {offset} extends past end of file"),
        ));
    }
    let obj_type = data[off];
    let flags = data[off + 1];
    let size = r64(data, off + 8);
    Ok((obj_type, flags, size))
}

/// Read the entry array starting at `offset`, collecting entry offsets.
/// Follows the linked list of entry array objects.
fn collect_entry_offsets(data: &[u8], mut ea_offset: u64, compact: bool) -> io::Result<Vec<u64>> {
    let mut offsets = Vec::new();

    while ea_offset != 0 {
        let (obj_type, _flags, size) = read_object_header(data, ea_offset)?;
        if obj_type != OBJECT_ENTRY_ARRAY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected entry array object at {ea_offset}, got type {obj_type}"),
            ));
        }

        let base = ea_offset as usize + OBJECT_HEADER_SIZE;
        let payload_size = size as usize - OBJECT_HEADER_SIZE;

        if base + ENTRY_ARRAY_FIXED_SIZE > data.len() {
            break;
        }

        let next_ea = r64(data, base);
        let items_start = base + ENTRY_ARRAY_FIXED_SIZE;
        let items_bytes = payload_size.saturating_sub(ENTRY_ARRAY_FIXED_SIZE);

        if compact {
            let item_count = items_bytes / 4;
            for i in 0..item_count {
                let item_off = items_start + i * 4;
                if item_off + 4 > data.len() {
                    break;
                }
                let entry_offset = r32(data, item_off) as u64;
                if entry_offset != 0 {
                    offsets.push(entry_offset);
                }
            }
        } else {
            let item_count = items_bytes / 8;
            for i in 0..item_count {
                let item_off = items_start + i * 8;
                if item_off + 8 > data.len() {
                    break;
                }
                let entry_offset = r64(data, item_off);
                if entry_offset != 0 {
                    offsets.push(entry_offset);
                }
            }
        }

        ea_offset = next_ea;
    }

    Ok(offsets)
}

/// Parse a single entry object at the given offset, resolving its data objects
/// into key-value fields.
fn parse_entry(data: &[u8], offset: u64, compact: bool) -> io::Result<JournalEntry> {
    let (obj_type, _flags, size) = read_object_header(data, offset)?;
    if obj_type != OBJECT_ENTRY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected entry object at {offset}, got type {obj_type}"),
        ));
    }

    let base = offset as usize + OBJECT_HEADER_SIZE;
    let payload_size = size as usize - OBJECT_HEADER_SIZE;

    if base + ENTRY_FIXED_SIZE > data.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "entry object truncated",
        ));
    }

    let seqnum = r64(data, base);
    let realtime = r64(data, base + 8);
    let monotonic = r64(data, base + 16);
    let boot_id = &data[base + 24..base + 40];
    // xor_hash at base + 40, not needed

    // Parse item references to data objects
    let items_start = base + ENTRY_FIXED_SIZE;
    let items_bytes = payload_size.saturating_sub(ENTRY_FIXED_SIZE);

    let mut fields = BTreeMap::new();

    if compact {
        // Compact mode: items are le32_t object offsets (4 bytes each)
        let item_count = items_bytes / 4;
        for i in 0..item_count {
            let item_off = items_start + i * 4;
            if item_off + 4 > data.len() {
                break;
            }
            let data_offset = r32(data, item_off) as u64;
            if data_offset != 0
                && let Ok((k, v)) = read_data_object(data, data_offset)
            {
                fields.insert(k, v);
            }
        }
    } else {
        // Regular mode: items are (le64 object_offset, le64 hash) = 16 bytes each
        let item_count = items_bytes / 16;
        for i in 0..item_count {
            let item_off = items_start + i * 16;
            if item_off + 8 > data.len() {
                break;
            }
            let data_offset = r64(data, item_off);
            if data_offset != 0
                && let Ok((k, v)) = read_data_object(data, data_offset)
            {
                fields.insert(k, v);
            }
        }
    }

    // Format boot ID as a hex string
    let boot_id_hex = format!(
        "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        boot_id[0],
        boot_id[1],
        boot_id[2],
        boot_id[3],
        boot_id[4],
        boot_id[5],
        boot_id[6],
        boot_id[7],
        boot_id[8],
        boot_id[9],
        boot_id[10],
        boot_id[11],
        boot_id[12],
        boot_id[13],
        boot_id[14],
        boot_id[15],
    );

    // Ensure _BOOT_ID is set (it's stored in the entry header, not always
    // as a separate data object)
    fields
        .entry("_BOOT_ID".to_string())
        .or_insert_with(|| boot_id_hex.into_bytes());

    let mut entry = JournalEntry::with_timestamp(realtime, monotonic);
    entry.seqnum = seqnum;
    entry.fields = fields;

    Ok(entry)
}

/// Decompress a compressed data object payload.
fn decompress_data_object(flags: u8, compressed: &[u8]) -> io::Result<Vec<u8>> {
    if flags & OBJECT_COMPRESSED_LZ4 != 0 {
        // C journal LZ4: first 8 bytes are LE64 uncompressed size, rest is LZ4 block
        if compressed.len() < 8 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "LZ4 compressed data too short for size prefix",
            ));
        }
        let uncompressed_size = u64::from_le_bytes(compressed[..8].try_into().unwrap()) as usize;
        let lz4_data = &compressed[8..];
        lz4_flex::decompress(lz4_data, uncompressed_size)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("LZ4 decompress: {e}")))
    } else if flags & OBJECT_COMPRESSED_ZSTD != 0 {
        zstd::stream::decode_all(compressed).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("zstd decompress: {e}"))
        })
    } else if flags & OBJECT_COMPRESSED_XZ != 0 {
        let mut output = Vec::new();
        lzma_rs::xz_decompress(&mut io::Cursor::new(compressed), &mut output).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("XZ decompress: {e}"))
        })?;
        Ok(output)
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unknown compression type",
        ))
    }
}

/// Read a data object at the given offset and extract the KEY=VALUE pair.
fn read_data_object(data: &[u8], offset: u64) -> io::Result<(String, Vec<u8>)> {
    let (obj_type, flags, size) = read_object_header(data, offset)?;
    if obj_type != OBJECT_DATA {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected data object at {offset}, got type {obj_type}"),
        ));
    }

    let base = offset as usize + OBJECT_HEADER_SIZE;
    let payload_size = size as usize - OBJECT_HEADER_SIZE;

    if payload_size < DATA_FIXED_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "data object too small",
        ));
    }

    // Payload starts after the fixed data-object fields
    let kv_start = base + DATA_FIXED_SIZE;
    let kv_len = payload_size - DATA_FIXED_SIZE;

    if kv_start + kv_len > data.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "data payload extends past end of file",
        ));
    }

    let compressed_bytes = &data[kv_start..kv_start + kv_len];

    // Decompress if needed
    let kv_bytes: std::borrow::Cow<'_, [u8]> = if flags & OBJECT_COMPRESSION_MASK != 0 {
        let decompressed = decompress_data_object(flags, compressed_bytes)?;
        std::borrow::Cow::Owned(decompressed)
    } else {
        std::borrow::Cow::Borrowed(compressed_bytes)
    };

    // Find the '=' separator
    if let Some(eq_pos) = kv_bytes.iter().position(|&b| b == b'=') {
        let key = String::from_utf8_lossy(&kv_bytes[..eq_pos]).into_owned();
        let value = kv_bytes[eq_pos + 1..].to_vec();
        Ok((key, value))
    } else {
        // Some objects don't have '=' (field name objects), skip them
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "data object has no KEY=VALUE separator",
        ))
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Check if the file at `path` is a C systemd journal file (LPKSHHRH magic).
pub fn is_c_journal(path: &Path) -> bool {
    if let Ok(data) = std::fs::read(path) {
        data.len() >= 8 && &data[0..8] == C_JOURNAL_MAGIC
    } else {
        false
    }
}

/// Read all entries from a C systemd journal file.
///
/// Returns entries sorted by their stored sequence number.  Compressed
/// data objects (LZ4, ZSTD, XZ) are transparently decompressed.
pub fn read_c_journal(path: &Path) -> io::Result<Vec<JournalEntry>> {
    let data = std::fs::read(path)?;
    let header = parse_header(&data)?;

    if header.entry_array_offset == 0 {
        return Ok(Vec::new());
    }

    let compact = header.is_compact();
    let entry_offsets = collect_entry_offsets(&data, header.entry_array_offset, compact)?;

    let mut entries = Vec::with_capacity(entry_offsets.len());
    for &offset in &entry_offsets {
        match parse_entry(&data, offset, compact) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                eprintln!(
                    "journald: Warning: could not parse entry at offset {} in {}: {}",
                    offset,
                    path.display(),
                    e
                );
            }
        }
    }

    Ok(entries)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_magic_check() {
        assert_eq!(C_JOURNAL_MAGIC, b"LPKSHHRH");
    }

    #[test]
    fn test_parse_header_too_small() {
        let data = vec![0u8; 100];
        assert!(parse_header(&data).is_err());
    }

    #[test]
    fn test_parse_header_wrong_magic() {
        let mut data = vec![0u8; MIN_HEADER_SIZE];
        data[0..8].copy_from_slice(b"JRNL_RS\0");
        assert!(parse_header(&data).is_err());
    }

    #[test]
    fn test_parse_header_valid() {
        let mut data = vec![0u8; MIN_HEADER_SIZE];
        data[0..8].copy_from_slice(b"LPKSHHRH");
        // Set header_size
        data[88..96].copy_from_slice(&(MIN_HEADER_SIZE as u64).to_le_bytes());
        let header = parse_header(&data).unwrap();
        assert!(!header.is_compact());
        assert_eq!(header.entry_array_offset, 0);
    }

    #[test]
    fn test_parse_header_compact() {
        let mut data = vec![0u8; MIN_HEADER_SIZE];
        data[0..8].copy_from_slice(b"LPKSHHRH");
        // Set incompatible flags to include COMPACT
        data[12..16].copy_from_slice(&HEADER_INCOMPATIBLE_COMPACT.to_le_bytes());
        data[88..96].copy_from_slice(&(MIN_HEADER_SIZE as u64).to_le_bytes());
        let header = parse_header(&data).unwrap();
        assert!(header.is_compact());
    }
}
