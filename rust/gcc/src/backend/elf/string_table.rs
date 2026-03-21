//! ELF string table builder for .strtab, .shstrtab, and .dynstr sections.

use std::collections::HashMap;

/// ELF string table builder. Used for .strtab, .shstrtab, and .dynstr sections.
///
/// Strings are stored as null-terminated entries. The table always starts with
/// a null byte (index 0 = empty string), matching ELF convention.
pub struct StringTable {
    data: Vec<u8>,
    offsets: HashMap<String, u32>,
}

impl StringTable {
    /// Create a new string table with the initial null byte.
    pub fn new() -> Self {
        Self {
            data: vec![0],
            offsets: HashMap::new(),
        }
    }

    /// Add a string to the table and return its offset.
    /// Returns 0 for empty strings. Deduplicates repeated insertions.
    pub fn add(&mut self, s: &str) -> u32 {
        if s.is_empty() {
            return 0;
        }
        if let Some(&offset) = self.offsets.get(s) {
            return offset;
        }
        let offset = self.data.len() as u32;
        self.data.extend_from_slice(s.as_bytes());
        self.data.push(0);
        self.offsets.insert(s.to_string(), offset);
        offset
    }

    /// Look up the offset of a previously-added string. Returns 0 if not found.
    pub fn offset_of(&self, s: &str) -> u32 {
        self.offsets.get(s).copied().unwrap_or(0)
    }

    /// Return the raw table bytes (including the leading null byte).
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Return the size of the table in bytes.
    pub fn len(&self) -> usize {
        self.data.len()
    }
}
