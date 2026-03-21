//! ELF64 parsing for the AArch64 linker.
//!
//! This module re-exports the shared ELF64 types and parser from `linker_common`,
//! plus provides AArch64-specific relocation constants. The actual parsing logic
//! lives in the shared module to avoid duplication with x86 and RISC-V.

// Re-export shared ELF constants so existing callers (mod.rs, reloc.rs)
// continue to work via `use super::elf::*`.
pub use crate::backend::elf::{
    get_standard_linker_symbols, is_thin_archive, parse_linker_script_entries, read_u16, read_u32,
    w16, w32, w64, wphdr, write_bytes, LinkerScriptEntry, LinkerSymbolAddresses, DF_1_NOW,
    DF_BIND_NOW, DT_DEBUG, DT_FINI_ARRAY, DT_FINI_ARRAYSZ, DT_FLAGS, DT_FLAGS_1, DT_GNU_HASH,
    DT_INIT_ARRAY, DT_INIT_ARRAYSZ, DT_JMPREL, DT_NEEDED, DT_NULL, DT_PLTGOT, DT_PLTREL,
    DT_PLTRELSZ, DT_RELA, DT_RELACOUNT, DT_RELAENT, DT_RELASZ, DT_SONAME, DT_STRSZ, DT_STRTAB,
    DT_SYMENT, DT_SYMTAB, ELFCLASS64, ELFDATA2LSB, ELF_MAGIC, EM_AARCH64, ET_DYN, ET_EXEC, PF_R,
    PF_W, PF_X, PT_DYNAMIC, PT_GNU_EH_FRAME, PT_GNU_STACK, PT_INTERP, PT_LOAD, PT_PHDR, PT_TLS,
    SHF_ALLOC, SHF_EXECINSTR, SHF_TLS, SHF_WRITE, SHN_ABS, SHN_COMMON, SHN_UNDEF, SHT_NOBITS,
    STB_GLOBAL, STB_WEAK, STT_FUNC, STT_GNU_IFUNC, STT_OBJECT, STT_SECTION, STT_TLS,
};

use crate::backend::linker_common;

// ── AArch64 relocation types ───────────────────────────────────────────

pub const R_AARCH64_NONE: u32 = 0;
pub const R_AARCH64_ABS64: u32 = 257; // S + A
pub const R_AARCH64_ABS32: u32 = 258; // S + A (32-bit)
pub const R_AARCH64_ABS16: u32 = 259; // S + A (16-bit)
pub const R_AARCH64_PREL64: u32 = 260; // S + A - P
pub const R_AARCH64_PREL32: u32 = 261; // S + A - P
pub const R_AARCH64_PREL16: u32 = 262; // S + A - P
pub const R_AARCH64_ADR_PREL_PG_HI21: u32 = 275; // Page(S+A) - Page(P)
pub const R_AARCH64_ADR_PREL_LO21: u32 = 274; // S + A - P
pub const R_AARCH64_ADD_ABS_LO12_NC: u32 = 277; // (S + A) & 0xFFF
pub const R_AARCH64_LDST8_ABS_LO12_NC: u32 = 278;
pub const R_AARCH64_LDST16_ABS_LO12_NC: u32 = 284;
pub const R_AARCH64_LDST32_ABS_LO12_NC: u32 = 285;
pub const R_AARCH64_LDST64_ABS_LO12_NC: u32 = 286;
pub const R_AARCH64_LDST128_ABS_LO12_NC: u32 = 299;
pub const R_AARCH64_JUMP26: u32 = 282; // S + A - P (26-bit B)
pub const R_AARCH64_CALL26: u32 = 283; // S + A - P (26-bit BL)
pub const R_AARCH64_MOVW_UABS_G0_NC: u32 = 264;
pub const R_AARCH64_MOVW_UABS_G1_NC: u32 = 265;
pub const R_AARCH64_MOVW_UABS_G2_NC: u32 = 266;
pub const R_AARCH64_MOVW_UABS_G3: u32 = 267;
pub const R_AARCH64_MOVW_UABS_G0: u32 = 263;
pub const R_AARCH64_ADR_GOT_PAGE: u32 = 311;
pub const R_AARCH64_LD64_GOT_LO12_NC: u32 = 312;
pub const R_AARCH64_CONDBR19: u32 = 280;
pub const R_AARCH64_TSTBR14: u32 = 279;

// ── Type aliases ─────────────────────────────────────────────────────────
// Re-export shared types under the names the ARM linker already uses.

pub type SectionHeader = linker_common::Elf64Section;
pub type Symbol = linker_common::Elf64Symbol;
pub type Rela = linker_common::Elf64Rela;
pub type ElfObject = linker_common::Elf64Object;

// ── Parsing functions ────────────────────────────────────────────────────
// Delegate to shared implementations.

pub fn parse_object(data: &[u8], source_name: &str) -> Result<ElfObject, String> {
    linker_common::parse_elf64_object(data, source_name, EM_AARCH64)
}
