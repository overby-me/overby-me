//! ELF64 object file types shared across linker backends.
//!
//! These types are used by x86, ARM, and RISC-V linkers. The i686 linker uses
//! its own ELF32 types since field widths differ (u32 vs u64).

use crate::backend::elf::SHN_UNDEF;
use crate::backend::elf::{STB_GLOBAL, STB_LOCAL, STB_WEAK};

/// Parsed ELF64 section header.
#[derive(Debug, Clone)]
#[allow(dead_code)] // All fields populated during parsing; not every backend reads every field
pub struct Elf64Section {
    pub name_idx: u32,
    pub name: String,
    pub sh_type: u32,
    pub flags: u64,
    pub addr: u64,
    pub offset: u64,
    pub size: u64,
    pub link: u32,
    pub info: u32,
    pub addralign: u64,
    pub entsize: u64,
}

/// Parsed ELF64 symbol.
#[derive(Debug, Clone)]
#[allow(dead_code)] // All fields populated during parsing; not every backend reads every field
pub struct Elf64Symbol {
    pub name_idx: u32,
    pub name: String,
    pub info: u8,
    pub other: u8,
    pub shndx: u16,
    pub value: u64,
    pub size: u64,
}

#[allow(dead_code)] // Convenience accessors; not all used by every backend yet
impl Elf64Symbol {
    pub fn binding(&self) -> u8 {
        self.info >> 4
    }
    pub fn sym_type(&self) -> u8 {
        self.info & 0xf
    }
    pub fn visibility(&self) -> u8 {
        self.other & 0x3
    }
    pub fn is_undefined(&self) -> bool {
        self.shndx == SHN_UNDEF
    }
    pub fn is_global(&self) -> bool {
        self.binding() == STB_GLOBAL
    }
    pub fn is_weak(&self) -> bool {
        self.binding() == STB_WEAK
    }
    pub fn is_local(&self) -> bool {
        self.binding() == STB_LOCAL
    }
}

/// Parsed ELF64 relocation with addend (RELA).
#[derive(Debug, Clone)]
pub struct Elf64Rela {
    pub offset: u64,
    pub sym_idx: u32,
    pub rela_type: u32,
    pub addend: i64,
}

/// Parsed ELF64 object file (.o).
#[derive(Debug)]
pub struct Elf64Object {
    pub sections: Vec<Elf64Section>,
    pub symbols: Vec<Elf64Symbol>,
    pub section_data: Vec<Vec<u8>>,
    /// Relocations indexed by the section they apply to.
    pub relocations: Vec<Vec<Elf64Rela>>,
    pub source_name: String,
}

/// Dynamic symbol from a shared library (.so).
#[derive(Debug, Clone)]
pub struct DynSymbol {
    pub name: String,
    pub info: u8,
    pub value: u64,
    pub size: u64,
    /// GLIBC version string for this symbol (e.g. "GLIBC_2.3"), if any.
    pub version: Option<String>,
    /// Whether this is the default version (@@GLIBC_x.y vs @GLIBC_x.y).
    #[allow(dead_code)]
    // Populated during .so parsing; used by i686 linker's version preference logic
    pub is_default_ver: bool,
}

#[allow(dead_code)] // Convenience accessor; used by x86/ARM linkers via type alias
impl DynSymbol {
    pub fn sym_type(&self) -> u8 {
        self.info & 0xf
    }
}
