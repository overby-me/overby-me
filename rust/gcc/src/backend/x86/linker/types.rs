//! x86-64 linker types and constants.
//!
//! Defines the `GlobalSymbol` type used by all linker phases, plus
//! architecture-specific constants (base address, page size, interpreter path).

use super::elf::{SHN_COMMON, SHN_UNDEF};
use crate::backend::linker_common::{self, Elf64Symbol, GlobalSymbolOps};

/// Base virtual address for the executable (standard non-PIE x86-64 address)
pub const BASE_ADDR: u64 = 0x400000;
/// Page size for alignment
pub const PAGE_SIZE: u64 = 0x1000;
/// Dynamic linker path
pub const INTERP: &[u8] = b"/lib64/ld-linux-x86-64.so.2\0";

/// A resolved global symbol.
///
/// This struct has x86-specific dynamic linking fields (plt_idx, got_idx,
/// copy_reloc, from_lib, version, lib_sym_value) in addition to the common
/// fields needed by the shared linker infrastructure.
#[derive(Clone)]
pub struct GlobalSymbol {
    pub value: u64,
    pub size: u64,
    pub info: u8,
    pub defined_in: Option<usize>,
    pub from_lib: Option<String>,
    pub plt_idx: Option<usize>,
    pub got_idx: Option<usize>,
    pub section_idx: u16,
    pub is_dynamic: bool,
    pub copy_reloc: bool,
    pub lib_sym_value: u64,
    pub version: Option<String>,
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
            version: None,
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
            version: None,
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
            version: None,
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
            version: dsym.version.clone(),
        }
    }
}

/// For x86, a dynamic definition should be replaced by a static definition.
pub fn x86_should_replace_extra(existing: &GlobalSymbol) -> bool {
    existing.is_dynamic
}
