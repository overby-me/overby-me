#![allow(dead_code)]
//! Minimal GVariant serializer for producing OSTree and Flatpak binary formats.
//!
//! Supports the subset of GVariant types needed for:
//! - Commit objects: `(a{sv}aya(say)sstayay)`
//! - Dirtree objects: `(a(say)a(sayay))`
//! - Dirmeta objects: `(uuua(ayay))`
//! - Summary files and bundle metadata

/// A GVariant value that can be serialized.
#[derive(Debug, Clone)]
pub enum GVariant {
    /// Boolean.
    Bool(bool),
    /// Byte.
    Byte(u8),
    /// Unsigned 32-bit integer (little-endian).
    Uint32(u32),
    /// Unsigned 64-bit integer (little-endian).
    Uint64(u64),
    /// String (NUL-terminated).
    Str(String),
    /// Byte array.
    ByteArray(Vec<u8>),
    /// Array of values (all same type).
    Array(Vec<GVariant>),
    /// Tuple of values (heterogeneous).
    Tuple(Vec<GVariant>),
    /// Dictionary entry (key, value).
    DictEntry(Box<GVariant>, Box<GVariant>),
    /// Variant (type string + value).
    Variant(Box<GVariant>),
}

impl GVariant {
    /// Serialize this value to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        match self {
            GVariant::Bool(v) => vec![if *v { 1 } else { 0 }],
            GVariant::Byte(v) => vec![*v],
            GVariant::Uint32(v) => v.to_le_bytes().to_vec(),
            GVariant::Uint64(v) => v.to_le_bytes().to_vec(),
            GVariant::Str(s) => {
                let mut data = s.as_bytes().to_vec();
                data.push(0); // NUL terminator
                data
            }
            GVariant::ByteArray(v) => v.clone(),
            GVariant::Array(items) => serialize_array(items),
            GVariant::Tuple(items) => serialize_tuple(items),
            GVariant::DictEntry(k, v) => {
                let mut data = k.serialize();
                let v_align = v.alignment();
                pad_to(&mut data, v_align);
                data.extend_from_slice(&v.serialize());
                data
            }
            GVariant::Variant(inner) => {
                let inner_data = inner.serialize();
                let type_str = inner.type_string();
                let mut data = inner_data;
                data.push(0); // separator
                data.extend_from_slice(type_str.as_bytes());
                data
            }
        }
    }

    /// Get the fixed size of this type, or None if variable-size.
    pub fn fixed_size(&self) -> Option<usize> {
        match self {
            GVariant::Bool(_) | GVariant::Byte(_) => Some(1),
            GVariant::Uint32(_) => Some(4),
            GVariant::Uint64(_) => Some(8),
            _ => None,
        }
    }

    /// Get the alignment requirement.
    pub fn alignment(&self) -> usize {
        match self {
            GVariant::Bool(_) | GVariant::Byte(_) | GVariant::Str(_) | GVariant::ByteArray(_) => 1,
            GVariant::Uint32(_) => 4,
            GVariant::Uint64(_) => 8,
            GVariant::Array(items) => items.first().map(|i| i.alignment()).unwrap_or(1),
            GVariant::Tuple(items) => items.iter().map(|i| i.alignment()).max().unwrap_or(1),
            GVariant::DictEntry(k, v) => k.alignment().max(v.alignment()),
            GVariant::Variant(_) => 8,
        }
    }

    /// Get the GVariant type string.
    pub fn type_string(&self) -> String {
        match self {
            GVariant::Bool(_) => "b".into(),
            GVariant::Byte(_) => "y".into(),
            GVariant::Uint32(_) => "u".into(),
            GVariant::Uint64(_) => "t".into(),
            GVariant::Str(_) => "s".into(),
            GVariant::ByteArray(_) => "ay".into(),
            GVariant::Array(items) => {
                let inner = items
                    .first()
                    .map(|i| i.type_string())
                    .unwrap_or_else(|| "y".into());
                format!("a{inner}")
            }
            GVariant::Tuple(items) => {
                let inner: String = items.iter().map(|i| i.type_string()).collect();
                format!("({inner})")
            }
            GVariant::DictEntry(k, v) => format!("{{{}{}}}", k.type_string(), v.type_string()),
            GVariant::Variant(_) => "v".into(),
        }
    }

    /// Check if this type is variable-size.
    pub fn is_variable_size(&self) -> bool {
        self.fixed_size().is_none()
    }
}

/// Pad a buffer to a given alignment.
fn pad_to(buf: &mut Vec<u8>, alignment: usize) {
    let remainder = buf.len() % alignment;
    if remainder != 0 {
        let padding = alignment - remainder;
        buf.extend(std::iter::repeat_n(0u8, padding));
    }
}

/// Determine the offset size for a container of the given byte length.
fn offset_size(total_len: usize) -> usize {
    if total_len <= 0xFF {
        1
    } else if total_len <= 0xFFFF {
        2
    } else {
        4
    }
}

/// Write an offset value.
fn write_offset(buf: &mut Vec<u8>, offset: usize, size: usize) {
    match size {
        1 => buf.push(offset as u8),
        2 => buf.extend_from_slice(&(offset as u16).to_le_bytes()),
        4 => buf.extend_from_slice(&(offset as u32).to_le_bytes()),
        _ => {}
    }
}

/// Serialize a GVariant array.
fn serialize_array(items: &[GVariant]) -> Vec<u8> {
    if items.is_empty() {
        return Vec::new();
    }

    let element_fixed = items[0].fixed_size();

    if element_fixed.is_some() {
        // Fixed-size elements: just concatenate.
        let mut data = Vec::new();
        for item in items {
            data.extend_from_slice(&item.serialize());
        }
        data
    } else {
        // Variable-size elements: serialize each, then append framing offsets.
        let mut body = Vec::new();
        let mut offsets = Vec::new();
        let alignment = items[0].alignment();

        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                pad_to(&mut body, alignment);
            }
            body.extend_from_slice(&item.serialize());
            offsets.push(body.len());
        }

        // Determine offset size.
        let total = body.len() + offsets.len() * 4; // approximate
        let osz = offset_size(total);

        // Write offsets.
        for &off in &offsets {
            write_offset(&mut body, off, osz);
        }

        body
    }
}

/// Serialize a GVariant tuple.
fn serialize_tuple(items: &[GVariant]) -> Vec<u8> {
    if items.is_empty() {
        return Vec::new();
    }

    let mut body = Vec::new();
    let mut offsets = Vec::new(); // offsets for variable-size non-last elements

    for (i, item) in items.iter().enumerate() {
        // Align to element's alignment.
        pad_to(&mut body, item.alignment());

        body.extend_from_slice(&item.serialize());

        // Record offset for variable-size elements (except the last).
        if i < items.len() - 1 && item.is_variable_size() {
            offsets.push(body.len());
        }
    }

    if !offsets.is_empty() {
        let total_estimate = body.len() + offsets.len() * 4;
        let osz = offset_size(total_estimate);

        for &off in &offsets {
            write_offset(&mut body, off, osz);
        }
    }

    body
}

// ---------------------------------------------------------------------------
// Convenience constructors
// ---------------------------------------------------------------------------

/// Create an empty dict `a{sv}`.
pub fn empty_metadata() -> GVariant {
    GVariant::Array(Vec::new())
}

/// Create a byte array from raw bytes.
pub fn byte_array(data: &[u8]) -> GVariant {
    GVariant::ByteArray(data.to_vec())
}

/// Create a string value.
pub fn string(s: &str) -> GVariant {
    GVariant::Str(s.to_string())
}

/// Create an OSTree commit object.
pub fn commit(
    subject: &str,
    body: &str,
    timestamp_be: u64,
    root_dirtree: &[u8],
    root_dirmeta: &[u8],
    parent: Option<&[u8]>,
) -> GVariant {
    GVariant::Tuple(vec![
        empty_metadata(),                  // a{sv}
        byte_array(parent.unwrap_or(&[])), // ay (parent)
        GVariant::Array(Vec::new()),       // a(say) (related)
        string(subject),                   // s
        string(body),                      // s
        GVariant::Uint64(timestamp_be),    // t (BE in value)
        byte_array(root_dirtree),          // ay
        byte_array(root_dirmeta),          // ay
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_string() {
        let v = GVariant::Str("hello".into());
        assert_eq!(v.serialize(), b"hello\0");
    }

    #[test]
    fn serialize_uint32() {
        let v = GVariant::Uint32(42);
        assert_eq!(v.serialize(), 42u32.to_le_bytes());
    }

    #[test]
    fn serialize_tuple_basic() {
        let v = GVariant::Tuple(vec![GVariant::Uint32(1), GVariant::Uint32(2)]);
        let data = v.serialize();
        assert_eq!(data.len(), 8); // two u32s, no framing needed (both fixed)
    }

    #[test]
    fn serialize_byte_array() {
        let v = byte_array(&[1, 2, 3]);
        assert_eq!(v.serialize(), vec![1, 2, 3]);
    }

    #[test]
    fn type_string_commit() {
        let c = commit("test", "", 0, &[0; 32], &[0; 32], None);
        // Empty arrays lose inner type info, so this doesn't exactly match
        // the OSTree format string. For serialization the bytes are what matter.
        let ts = c.type_string();
        assert!(ts.starts_with('('));
        assert!(ts.ends_with(')'));
        assert!(ts.contains("ss")); // subject + body strings
        assert!(ts.contains("ay")); // byte arrays
    }
}
