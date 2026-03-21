//! Native i686 (32-bit x86) ELF linker.
//!
//! Links ELF32 relocatable objects (.o) and archives (.a) into a dynamically-
//! linked or static ELF32 executable. Supports PLT/GOT for dynamic symbols,
//! TLS (all i386 models), GNU hash tables, GLIBC version tables, copy
//! relocations, COMDAT group deduplication, and IFUNC (IRELATIVE) for static.
//!
//! ## Module structure
//!
//! - `types` - ELF32 constants, structures, and linker state types
//! - `parse` - ELF32 object file parsing
//! - `dynsym` - Dynamic symbol reading from shared libraries
//! - `reloc` - i386 relocation application
//! - `gnu_hash` - GNU hash table building
//! - `input` - Phases 1-4: argument parsing, file loading, archive resolution
//! - `sections` - Phase 5: section merging and COMDAT deduplication
//! - `symbols` - Phases 6-9: symbol resolution, PLT/GOT marking, IFUNC collection
//! - `shared` - Shared library (.so) emission
//! - `emit` - Phase 10: executable layout and ELF32 emission
//! - `link` - Orchestration: `link_builtin` and `link_shared` entry points

mod dynsym;
mod emit;
mod gnu_hash;
mod input;
mod link;
mod parse;
mod reloc;
mod sections;
mod shared;
mod symbols;
#[allow(dead_code)] // ELF constants defined for completeness; not all used yet
mod types;

use crate::backend::linker_common;

// ── DynStrTab using linker_common ─────────────────────────────────────────
// Wraps linker_common::DynStrTab (usize offsets) for i686's u32 needs.

struct DynStrTab(linker_common::DynStrTab);

impl DynStrTab {
    fn new() -> Self {
        Self(linker_common::DynStrTab::new())
    }
    fn add(&mut self, s: &str) -> u32 {
        self.0.add(s) as u32
    }
    fn get_offset(&self, s: &str) -> u32 {
        self.0.get_offset(s) as u32
    }
    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

#[cfg(not(feature = "gcc_linker"))]
pub use link::link_builtin;
#[cfg(not(feature = "gcc_linker"))]
pub use link::link_shared;
