//! Dynamic string table builder for `.dynstr` section emission.
//!
//! Used by linkers that produce dynamically-linked executables (x86, i686, RISC-V).
//! Deduplicates strings and tracks offsets.

use std::collections::HashMap;

/// Dynamic string table builder.
///
/// Used by linkers that produce dynamically-linked executables (x86, i686, RISC-V).
/// Deduplicates strings and tracks offsets for .dynstr section emission.
pub struct DynStrTab {
    data: Vec<u8>,
    offsets: HashMap<String, usize>,
}

impl DynStrTab {
    pub fn new() -> Self {
        Self {
            data: vec![0],
            offsets: HashMap::new(),
        }
    }

    pub fn add(&mut self, s: &str) -> usize {
        if s.is_empty() {
            return 0;
        }
        if let Some(&off) = self.offsets.get(s) {
            return off;
        }
        let off = self.data.len();
        self.data.extend_from_slice(s.as_bytes());
        self.data.push(0);
        self.offsets.insert(s.to_string(), off);
        off
    }

    pub fn get_offset(&self, s: &str) -> usize {
        if s.is_empty() {
            0
        } else {
            self.offsets.get(s).copied().unwrap_or(0)
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }
}
