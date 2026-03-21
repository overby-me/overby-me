/// ELF file parsing for the x86-64 linker.
///
/// This module re-exports the shared ELF64 types and parser from `linker_common`,
/// plus provides x86-64-specific relocation constants. The actual parsing logic
/// lives in the shared module to avoid duplication with ARM and RISC-V.
// Re-export shared ELF constants for mod.rs and the emitter functions.
// Archive/linker-script functions are now called via linker_common.
pub use crate::backend::elf::{
    get_standard_linker_symbols, is_thin_archive, parse_linker_script_entries, w16, w32, w64,
    wphdr, write_bytes, LinkerScriptEntry, LinkerSymbolAddresses, DT_GNU_HASH, DT_JMPREL,
    DT_NEEDED, DT_NULL, DT_PLTGOT, DT_PLTREL, DT_PLTRELSZ, DT_RELA, DT_RELAENT, DT_RELASZ,
    DT_STRSZ, DT_STRTAB, DT_SYMENT, DT_SYMTAB, ELFCLASS64, ELFDATA2LSB, ELF_MAGIC, EM_X86_64,
    ET_DYN, ET_EXEC, PF_R, PF_W, PF_X, PT_DYNAMIC, PT_GNU_RELRO, PT_GNU_STACK, PT_INTERP, PT_LOAD,
    PT_PHDR, PT_TLS, SHF_ALLOC, SHF_EXECINSTR, SHF_TLS, SHF_WRITE, SHN_ABS, SHN_COMMON, SHN_UNDEF,
    SHT_DYNAMIC, SHT_DYNSYM, SHT_FINI_ARRAY, SHT_GNU_HASH, SHT_GNU_VERNEED, SHT_GNU_VERSYM,
    SHT_INIT_ARRAY, SHT_NOBITS, SHT_PROGBITS, SHT_RELA, SHT_STRTAB, STB_GLOBAL, STB_WEAK, STT_FUNC,
    STT_GNU_IFUNC, STT_OBJECT, STT_SECTION, STT_TLS,
};

use crate::backend::linker_common;

// x86-64 relocation types
pub const R_X86_64_NONE: u32 = 0;
pub const R_X86_64_64: u32 = 1;
pub const R_X86_64_PC32: u32 = 2;
pub const R_X86_64_GOT32: u32 = 3;
pub const R_X86_64_PLT32: u32 = 4;
pub const R_X86_64_GLOB_DAT: u32 = 6;
pub const R_X86_64_JUMP_SLOT: u32 = 7;
pub const R_X86_64_RELATIVE: u32 = 8;
pub const R_X86_64_GOTPCREL: u32 = 9;
pub const R_X86_64_32: u32 = 10;
pub const R_X86_64_32S: u32 = 11;
pub const R_X86_64_DTPMOD64: u32 = 16; // GD TLS model (not yet implemented; IE model used)
pub const R_X86_64_DTPOFF64: u32 = 17; // GD TLS model (not yet implemented; IE model used)
pub const R_X86_64_TPOFF64: u32 = 18;
pub const R_X86_64_GOTTPOFF: u32 = 22;
pub const R_X86_64_TPOFF32: u32 = 23;
pub const R_X86_64_PC64: u32 = 24;
pub const R_X86_64_GOTPCRELX: u32 = 41;
pub const R_X86_64_REX_GOTPCRELX: u32 = 42;
pub const R_X86_64_IRELATIVE: u32 = 37;

// DT_* constants now in shared module - re-export them
pub use crate::backend::elf::{
    DT_DEBUG, DT_FINI_ARRAY, DT_FINI_ARRAYSZ, DT_INIT_ARRAY, DT_INIT_ARRAYSZ, DT_RELACOUNT,
    DT_RPATH, DT_RUNPATH, DT_SONAME, DT_VERNEED, DT_VERNEEDNUM, DT_VERSYM,
};

pub const DF_BIND_NOW: i64 = 0x8;

// ── Type aliases ─────────────────────────────────────────────────────────
// Re-export shared types under the names the x86 linker already uses.

pub type SectionHeader = linker_common::Elf64Section;
pub type Symbol = linker_common::Elf64Symbol;
pub type Rela = linker_common::Elf64Rela;
pub type ElfObject = linker_common::Elf64Object;
pub type DynSymbol = linker_common::DynSymbol;

// ── Parsing functions ────────────────────────────────────────────────────
// Delegate to shared implementations.

pub fn parse_object(data: &[u8], source_name: &str) -> Result<ElfObject, String> {
    linker_common::parse_elf64_object(data, source_name, EM_X86_64)
}

pub fn parse_shared_library_symbols(data: &[u8], lib_name: &str) -> Result<Vec<DynSymbol>, String> {
    linker_common::parse_shared_library_symbols(data, lib_name)
}

pub fn parse_soname(data: &[u8]) -> Option<String> {
    linker_common::parse_soname(data)
}
