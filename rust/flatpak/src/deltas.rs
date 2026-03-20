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
                        // Simplified bspatch: for now, just append the patch data.
                        // A full bspatch implementation would decompose the
                        // control/diff/extra streams and apply them to the source.
                        // This is a placeholder that at least doesn't crash.
                        obj.data.extend_from_slice(patch_data);
                        eprintln!(
                            "  warning: bspatch not fully implemented, {} bytes from source ignored",
                            source.len()
                        );
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
