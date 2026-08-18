//! AArch64 linker types and constants.
//!
//! Defines the `GlobalSymbol` type used by all linker phases, plus
//! architecture-specific constants (base address, page size, interpreter path).

use super::elf::SHN_COMMON;
use super::elf::SHN_UNDEF;
use crate::backend::linker_common;
use linker_common::{Elf64Symbol, GlobalSymbolOps};

/// Dynamic linker path for AArch64
pub const INTERP: &[u8] = b"/lib/ld-linux-aarch64.so.1\0";

/// Base virtual address for the executable
pub const BASE_ADDR: u64 = 0x400000;
/// Page size for alignment
pub const PAGE_SIZE: u64 = 0x10000; // AArch64 uses 64KB pages for linker alignment

/// A resolved global symbol
#[derive(Clone)]
pub struct GlobalSymbol {
    pub value: u64,
    pub size: u64,
    pub info: u8,
    pub defined_in: Option<usize>,
    pub section_idx: u16,
    /// SONAME of the shared library this symbol was resolved from
    pub from_lib: Option<String>,
    /// PLT entry index (for dynamic function symbols)
    pub plt_idx: Option<usize>,
    /// GOT entry index (for dynamic symbols needing GOT slots)
    pub got_idx: Option<usize>,
    /// Whether this symbol is resolved from a shared library
    pub is_dynamic: bool,
    /// Whether this symbol needs a copy relocation
    pub copy_reloc: bool,
    /// Symbol's value in the source shared library (for alias detection)
    pub lib_sym_value: u64,
}

impl GlobalSymbolOps for GlobalSymbol {
    fn is_defined(&self) -> bool {
        self.defined_in.is_some()
    }
    fn is_dynamic(&self) -> bool {
        self.is_dynamic
    }
    fn info(&self) -> u8 {
        self.info
    }
    fn section_idx(&self) -> u16 {
        self.section_idx
    }
    fn value(&self) -> u64 {
        self.value
    }
    fn size(&self) -> u64 {
        self.size
    }
    fn new_defined(obj_idx: usize, sym: &Elf64Symbol) -> Self {
        GlobalSymbol {
            value: sym.value,
            size: sym.size,
            info: sym.info,
            defined_in: Some(obj_idx),
            from_lib: None,
            plt_idx: None,
            got_idx: None,
            section_idx: sym.shndx,
            is_dynamic: false,
            copy_reloc: false,
            lib_sym_value: 0,
        }
    }
    fn new_common(obj_idx: usize, sym: &Elf64Symbol) -> Self {
        GlobalSymbol {
            value: sym.value,
            size: sym.size,
            info: sym.info,
            defined_in: Some(obj_idx),
            from_lib: None,
            plt_idx: None,
            got_idx: None,
            section_idx: SHN_COMMON,
            is_dynamic: false,
            copy_reloc: false,
            lib_sym_value: 0,
        }
    }
    fn new_undefined(sym: &Elf64Symbol) -> Self {
        GlobalSymbol {
            value: 0,
            size: 0,
            info: sym.info,
            defined_in: None,
            from_lib: None,
            plt_idx: None,
            got_idx: None,
            section_idx: SHN_UNDEF,
            is_dynamic: false,
            copy_reloc: false,
            lib_sym_value: 0,
        }
    }
    fn set_common_bss(&mut self, bss_offset: u64) {
        self.value = bss_offset;
        self.section_idx = 0xffff;
    }
    fn new_dynamic(dsym: &linker_common::DynSymbol, soname: &str) -> Self {
        GlobalSymbol {
            value: 0,
            size: dsym.size,
            info: dsym.info,
            defined_in: None,
            from_lib: Some(soname.to_string()),
            plt_idx: None,
            got_idx: None,
            section_idx: SHN_UNDEF,
            is_dynamic: true,
            copy_reloc: false,
            lib_sym_value: dsym.value,
        }
    }
}

/// ARM-specific replacement policy: also replace dynamic symbols with local definitions.
pub fn arm_should_replace_extra(existing: &GlobalSymbol) -> bool {
    existing.is_dynamic
}
