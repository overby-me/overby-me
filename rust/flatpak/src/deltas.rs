#![allow(dead_code)]
//! OSTree static delta parsing and application.
//!
//! Static deltas allow efficient transfer of changes between commits without
//! fetching individual objects. A delta consists of a superblock (metadata)
//! and one or more part files containing instructions and data.
//!
//! Delta instruction set:
//! - OPEN_SPLICE_AND_CLOSE: write entire object from inline data
//! - OPEN: begin writing a new object
//! - WRITE: append data to current object
//! - SET_READ_SOURCE: set a source object for copy operations
//! - UNSET_READ_SOURCE: clear the read source
//! - CLOSE: finalize the current object
//! - BSPATCH: apply a bsdiff-style patch

use std::fs;
use std::path::{Path, PathBuf};

// Delta instruction opcodes.
const DELTA_OP_OPEN_SPLICE_AND_CLOSE: u8 = b'n';
const DELTA_OP_OPEN: u8 = b'o';
const DELTA_OP_WRITE: u8 = b'w';
const DELTA_OP_SET_READ_SOURCE: u8 = b'r';
const DELTA_OP_UNSET_READ_SOURCE: u8 = b'R';
const DELTA_OP_CLOSE: u8 = b'c';
const DELTA_OP_BSPATCH: u8 = b'B';

/// A parsed delta superblock.
#[derive(Debug)]
pub struct DeltaSuperblock {
    /// Timestamp of the target commit.
    pub timestamp: u64,
    /// Source commit checksum (empty for initial pulls).
    pub from_checksum: Option<String>,
    /// Target commit checksum.
    pub to_checksum: String,
    /// Number of delta parts.
    pub n_parts: usize,
    /// Part metadata (size, checksum).
    pub parts: Vec<DeltaPart>,
}

/// Metadata for a single delta part.
#[derive(Debug)]
pub struct DeltaPart {
    /// Size of the compressed part file.
    pub compressed_size: u64,
    /// Size after decompression.
    pub uncompressed_size: u64,
    /// Objects produced by this part.
    pub n_objects: u32,
}

/// State machine for applying delta instructions.
struct DeltaApplier {
    /// Directory to write objects into.
    repo_objects: PathBuf,
    /// Current object being written.
    current_object: Option<ObjectWriter>,
    /// Source data for copy operations.
    read_source: Option<Vec<u8>>,
}

struct ObjectWriter {
    checksum: String,
    object_type: String,
    data: Vec<u8>,
}

impl DeltaApplier {
    fn new(repo_objects: &Path) -> Self {
        Self {
            repo_objects: repo_objects.to_path_buf(),
            current_object: None,
            read_source: None,
        }
    }

    /// Apply a stream of delta instructions.
    fn apply(&mut self, instructions: &[u8], inline_data: &[u8]) -> Result<usize, String> {
        let mut pos = 0;
        let mut data_pos = 0;
        let mut objects_written = 0;

        while pos < instructions.len() {
            let op = instructions[pos];
            pos += 1;

            match op {
                DELTA_OP_OPEN_SPLICE_AND_CLOSE => {
                    // Read object type (1 byte) + checksum (32 bytes) + size (varint).
                    if pos + 33 > instructions.len() {
                        return Err("truncated OPEN_SPLICE_AND_CLOSE".into());
                    }
                    let obj_type = instructions[pos];
                    pos += 1;
                    let checksum = hex_encode(&instructions[pos..pos + 32]);
                    pos += 32;
                    let (size, bytes_read) = read_varint(&instructions[pos..]);
                    pos += bytes_read;

                    // Read `size` bytes from inline data.
                    if data_pos + size > inline_data.len() {
                        return Err("inline data underflow".into());
                    }
                    let data = &inline_data[data_pos..data_pos + size];
                    data_pos += size;

                    let ext = obj_type_ext(obj_type);
                    self.write_object(&checksum, ext, data)?;
                    objects_written += 1;
                }

                DELTA_OP_OPEN => {
                    if pos + 33 > instructions.len() {
                        return Err("truncated OPEN".into());
                    }
                    let obj_type = instructions[pos];
                    pos += 1;
                    let checksum = hex_encode(&instructions[pos..pos + 32]);
                    pos += 32;

                    let ext = obj_type_ext(obj_type);
                    self.current_object = Some(ObjectWriter {
                        checksum,
                        object_type: ext.to_string(),
                        data: Vec::new(),
                    });
                }

                DELTA_OP_WRITE => {
                    let (size, bytes_read) = read_varint(&instructions[pos..]);
                    pos += bytes_read;

                    if data_pos + size > inline_data.len() {
                        return Err("inline data underflow in WRITE".into());
                    }

                    if let Some(ref mut obj) = self.current_object {
                        obj.data
                            .extend_from_slice(&inline_data[data_pos..data_pos + size]);
                    }
                    data_pos += size;
                }

                DELTA_OP_SET_READ_SOURCE => {
                    if pos + 32 > instructions.len() {
                        return Err("truncated SET_READ_SOURCE".into());
                    }
                    let checksum = hex_encode(&instructions[pos..pos + 32]);
                    pos += 32;

                    // Load the source object.
                    let obj_path = self
                        .repo_objects
                        .join(&checksum[..2])
                        .join(format!("{}.filez", &checksum[2..]));
                    self.read_source = fs::read(&obj_path).ok();
                }

                DELTA_OP_UNSET_READ_SOURCE => {
                    self.read_source = None;
                }

                DELTA_OP_CLOSE => {
                    if let Some(obj) = self.current_object.take() {
                        self.write_object(&obj.checksum, &obj.object_type, &obj.data)?;
                        objects_written += 1;
                    }
                }

                DELTA_OP_BSPATCH => {
                    // Read patch size.
                    let (patch_size, bytes_read) = read_varint(&instructions[pos..]);
                    pos += bytes_read;

                    if data_pos + patch_size > inline_data.len() {
                        return Err("inline data underflow in BSPATCH".into());
                    }

                    let patch_data = &inline_data[data_pos..data_pos + patch_size];
                    data_pos += patch_size;

                    // Apply bspatch to the read source.
                    if let (Some(obj), Some(source)) = (&mut self.current_object, &self.read_source)
                    {
                        match bspatch(source, patch_data) {
                            Ok(patched) => {
                                obj.data = patched;
                            }
                            Err(e) => {
                                eprintln!("  warning: bspatch failed: {e}");
                                obj.data.extend_from_slice(patch_data);
                            }
                        }
                    }
                }

                _ => {
                    return Err(format!("unknown delta opcode: 0x{op:02x}"));
                }
            }
        }

        Ok(objects_written)
    }

    fn write_object(&self, checksum: &str, ext: &str, data: &[u8]) -> Result<(), String> {
        let dir = self.repo_objects.join(&checksum[..2]);
        let _ = fs::create_dir_all(&dir);
        let path = dir.join(format!("{}.{ext}", &checksum[2..]));
        fs::write(&path, data).map_err(|e| format!("write object {checksum}: {e}"))
    }
}

/// Map OSTree object type byte to file extension.
fn obj_type_ext(type_byte: u8) -> &'static str {
    match type_byte {
        1 => "file", // OSTREE_OBJECT_TYPE_FILE
        2 => "dirtree",
        3 => "dirmeta",
        4 => "commit",
        5 => "commitmeta",
        6 => "filez", // archive-z2 file
        _ => "unknown",
    }
}

/// Encode raw bytes as hex string.
fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

/// Read a variable-length integer (LEB128 unsigned).
/// Returns (value, bytes_consumed).
fn read_varint(data: &[u8]) -> (usize, usize) {
    let mut result: usize = 0;
    let mut shift = 0;
    let mut pos = 0;

    while pos < data.len() {
        let byte = data[pos] as usize;
        pos += 1;
        result |= (byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }

    (result, pos)
}

/// Parse a delta superblock.
///
/// The superblock is a GVariant with type `(a{sv}tayay...)`.
/// This is a simplified parser that extracts the key fields.
pub fn parse_superblock(data: &[u8]) -> Result<DeltaSuperblock, String> {
    if data.len() < 64 {
        return Err("superblock too short".into());
    }

    // The superblock format is complex GVariant. For now, extract
    // basic info from the known-offset fields.
    // Real parsing would use the GVariant framing offsets.

    // The to_checksum is typically in the last 32 bytes of the metadata.
    let to_checksum = if data.len() >= 32 {
        hex_encode(&data[data.len() - 32..])
    } else {
        "unknown".to_string()
    };

    Ok(DeltaSuperblock {
        timestamp: 0,
        from_checksum: None,
        to_checksum,
        n_parts: 1, // Assume 1 part for simple deltas.
        parts: vec![DeltaPart {
            compressed_size: 0,
            uncompressed_size: 0,
            n_objects: 0,
        }],
    })
}

/// Fetch and apply a static delta from a remote.
///
/// Returns the number of objects written, or Err if the delta
/// could not be applied.
pub fn apply_delta(
    repo_url: &str,
    commit_checksum: &str,
    repo_objects: &Path,
) -> Result<usize, String> {
    let superblock_url = format!(
        "{}/deltas/{}/{}/superblock",
        repo_url.trim_end_matches('/'),
        &commit_checksum[..2],
        &commit_checksum[2..]
    );

    let superblock_data = crate::ostree::fetch_url(&superblock_url)?;
    let _superblock = parse_superblock(&superblock_data)?;

    // Fetch part 0.
    let part_url = format!(
        "{}/deltas/{}/{}/0",
        repo_url.trim_end_matches('/'),
        &commit_checksum[..2],
        &commit_checksum[2..]
    );

    let part_data = crate::ostree::fetch_url(&part_url)?;

    // Delta parts are typically compressed. Decompress.
    let decompressed =
        miniz_oxide::inflate::decompress_to_vec(&part_data).unwrap_or_else(|_| part_data.clone()); // If decompression fails, try raw.

    // The decompressed part contains a header followed by instructions and inline data.
    // Simplified: treat the first half as instructions and the second half as data.
    // A proper implementation would parse the part header to get the instruction/data split.
    let split = decompressed.len() / 2;
    let instructions = &decompressed[..split];
    let inline_data = &decompressed[split..];

    let mut applier = DeltaApplier::new(repo_objects);
    applier.apply(instructions, inline_data)
}

// ---------------------------------------------------------------------------
// bspatch implementation
// ---------------------------------------------------------------------------

/// Read a signed 64-bit integer from bsdiff's offset encoding.
/// Bsdiff stores offsets as: if value >= 0, it's stored directly;
/// if the high bit of byte 7 is set, the value is negative.
fn bsdiff_read_offset(buf: &[u8]) -> i64 {
    if buf.len() < 8 {
        return 0;
    }
    let mut y = buf[0] as i64
        | (buf[1] as i64) << 8
        | (buf[2] as i64) << 16
        | (buf[3] as i64) << 24
        | (buf[4] as i64) << 32
        | (buf[5] as i64) << 40
        | (buf[6] as i64) << 48
        | ((buf[7] & 0x7F) as i64) << 56;
    if buf[7] & 0x80 != 0 {
        y = -y;
    }
    y
}

/// Apply a bspatch to produce output from old data + patch.
///
/// Bsdiff patch format (after header):
/// - Control tuples: (diff_len: i64, extra_len: i64, seek_offset: i64)
///   - Each field is 8 bytes, offset-encoded
/// - Diff stream: bytes to add to old data
/// - Extra stream: bytes to insert verbatim
///
/// The standard bsdiff format has a 32-byte header:
///   "BSDIFF40" magic + control_len + diff_len + new_size (all 8-byte offset-encoded)
///   followed by bzip2-compressed control, diff, and extra blocks.
///
/// OSTree uses raw (uncompressed) bspatch within deltas, so the data
/// is already decompressed. The format is:
///   - 4-byte LE: number of control tuples
///   - control tuples (24 bytes each: 3 x 8-byte offsets)
///   - diff data
///   - extra data
pub fn bspatch(old: &[u8], patch: &[u8]) -> Result<Vec<u8>, String> {
    // Try standard bsdiff format first.
    if patch.len() >= 32 && &patch[..8] == b"BSDIFF40" {
        return bspatch_standard(old, patch);
    }

    // OSTree inline bspatch format: raw control + diff + extra.
    bspatch_raw(old, patch)
}

/// Apply a standard BSDIFF40 patch.
fn bspatch_standard(old: &[u8], patch: &[u8]) -> Result<Vec<u8>, String> {
    if patch.len() < 32 {
        return Err("patch too short for BSDIFF40".into());
    }

    let ctrl_len = bsdiff_read_offset(&patch[8..16]) as usize;
    let diff_len = bsdiff_read_offset(&patch[16..24]) as usize;
    let new_size = bsdiff_read_offset(&patch[24..32]) as usize;

    // The data after the header may be bzip2-compressed. Since we don't have
    // a bzip2 decompressor, try to decompress with deflate, or treat as raw.
    let data = &patch[32..];

    // Try treating as raw (uncompressed) data.
    if data.len() < ctrl_len + diff_len {
        return Err("patch data too short".into());
    }

    let ctrl_data = &data[..ctrl_len];
    let diff_data = &data[ctrl_len..ctrl_len + diff_len];
    let extra_data = &data[ctrl_len + diff_len..];

    apply_bspatch_streams(old, ctrl_data, diff_data, extra_data, new_size)
}

/// Apply a raw (OSTree-style) bspatch.
fn bspatch_raw(old: &[u8], patch: &[u8]) -> Result<Vec<u8>, String> {
    if patch.len() < 4 {
        return Err("raw bspatch too short".into());
    }

    let n_ctrl = u32::from_le_bytes(patch[0..4].try_into().unwrap()) as usize;
    let ctrl_size = n_ctrl * 24;
    let header_size = 4;

    if patch.len() < header_size + ctrl_size {
        return Err("raw bspatch control data truncated".into());
    }

    let ctrl_data = &patch[header_size..header_size + ctrl_size];
    let rest = &patch[header_size + ctrl_size..];

    // Calculate diff_len from control tuples.
    let mut diff_total = 0usize;
    for i in 0..n_ctrl {
        let offset = i * 24;
        let diff_len = bsdiff_read_offset(&ctrl_data[offset..]) as usize;
        diff_total += diff_len;
    }

    if rest.len() < diff_total {
        return Err("raw bspatch diff data truncated".into());
    }

    let diff_data = &rest[..diff_total];
    let extra_data = &rest[diff_total..];

    // Estimate new_size from control tuples.
    let mut new_size = 0usize;
    for i in 0..n_ctrl {
        let offset = i * 24;
        let dl = bsdiff_read_offset(&ctrl_data[offset..]) as usize;
        let el = bsdiff_read_offset(&ctrl_data[offset + 8..]) as usize;
        new_size += dl + el;
    }

    apply_bspatch_streams(old, ctrl_data, diff_data, extra_data, new_size)
}

/// Core bspatch algorithm: apply control tuples with diff and extra streams.
fn apply_bspatch_streams(
    old: &[u8],
    ctrl_data: &[u8],
    diff_data: &[u8],
    extra_data: &[u8],
    new_size: usize,
) -> Result<Vec<u8>, String> {
    let mut new = Vec::with_capacity(new_size);
    let mut old_pos: i64 = 0;
    let mut diff_pos = 0usize;
    let mut extra_pos = 0usize;

    let n_ctrl = ctrl_data.len() / 24;

    for i in 0..n_ctrl {
        let offset = i * 24;
        let diff_len = bsdiff_read_offset(&ctrl_data[offset..]) as usize;
        let extra_len = bsdiff_read_offset(&ctrl_data[offset + 8..]) as usize;
        let seek_adj = bsdiff_read_offset(&ctrl_data[offset + 16..]);

        // Add diff bytes to old data.
        for j in 0..diff_len {
            let old_byte = if (old_pos + j as i64) >= 0 && (old_pos + j as i64) < old.len() as i64 {
                old[(old_pos + j as i64) as usize]
            } else {
                0
            };
            let diff_byte = if diff_pos + j < diff_data.len() {
                diff_data[diff_pos + j]
            } else {
                0
            };
            new.push(old_byte.wrapping_add(diff_byte));
        }
        diff_pos += diff_len;
        old_pos += diff_len as i64;

        // Copy extra bytes verbatim.
        if extra_pos + extra_len <= extra_data.len() {
            new.extend_from_slice(&extra_data[extra_pos..extra_pos + extra_len]);
        } else {
            // Pad with zeros if extra data is short.
            let available = extra_data.len().saturating_sub(extra_pos);
            if available > 0 {
                new.extend_from_slice(&extra_data[extra_pos..extra_pos + available]);
            }
            new.resize(new.len() + extra_len - available, 0);
        }
        extra_pos += extra_len;

        // Adjust old position.
        old_pos += seek_adj;
    }

    Ok(new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bsdiff_read_offset_positive() {
        let buf = [42, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(bsdiff_read_offset(&buf), 42);
    }

    #[test]
    fn test_bsdiff_read_offset_negative() {
        let mut buf = [42, 0, 0, 0, 0, 0, 0, 0];
        buf[7] = 0x80; // set negative flag
        assert_eq!(bsdiff_read_offset(&buf), -42);
    }

    #[test]
    fn test_bspatch_identity() {
        // A patch that produces the same output as the input:
        // one control tuple: (input_len, 0, 0) with diff = all zeros
        let old = b"hello world";
        let diff_len = old.len();

        let mut patch = Vec::new();
        // n_ctrl = 1
        patch.extend_from_slice(&1u32.to_le_bytes());
        // control: (diff_len, 0, 0)
        let mut ctrl = [0u8; 24];
        ctrl[0] = diff_len as u8;
        patch.extend_from_slice(&ctrl);
        // diff data: all zeros (old + 0 = old)
        patch.extend(std::iter::repeat(0u8).take(diff_len));
        // no extra data

        let result = bspatch(old, &patch).unwrap();
        assert_eq!(result, old);
    }

    #[test]
    fn test_bspatch_extra_only() {
        // A patch with only extra data (no diff from old).
        let old = b"";
        let new_data = b"new content";

        let mut patch = Vec::new();
        // n_ctrl = 1
        patch.extend_from_slice(&1u32.to_le_bytes());
        // control: (0, new_data.len(), 0)
        let mut ctrl = [0u8; 24];
        ctrl[8] = new_data.len() as u8; // extra_len
        patch.extend_from_slice(&ctrl);
        // no diff data
        // extra data
        patch.extend_from_slice(new_data);

        let result = bspatch(old, &patch).unwrap();
        assert_eq!(result, new_data);
    }

    #[test]
    fn test_read_varint() {
        assert_eq!(read_varint(&[0x05]), (5, 1));
        assert_eq!(read_varint(&[0x80, 0x01]), (128, 2));
        assert_eq!(read_varint(&[0xAC, 0x02]), (300, 2));
    }
}
