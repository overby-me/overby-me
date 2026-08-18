//! ELF32 types and constants for the i686 linker.
//!
//! Contains all i686/ELF32-specific constants, structures, and the linker symbol
//! types used throughout the linking process. Architecture-specific constants
//! (relocation types, dynamic tags with i32 types for ELF32) live here rather
//! than in the shared `elf` module which uses u64/i64 for ELF64.

use std::collections::HashMap;

// Re-export shared ELF constants used throughout the linker
pub(super) use crate::backend::elf::{
    get_standard_linker_symbols, is_thin_archive, parse_archive_members,
    parse_linker_script_entries, parse_thin_archive_members, read_cstr, read_i32, read_u16,
    read_u32, LinkerScriptEntry, LinkerSymbolAddresses, ELFCLASS32, ELFDATA2LSB, ELF_MAGIC, EM_386,
    ET_DYN, ET_EXEC, ET_REL, EV_CURRENT, PF_R, PF_W, PF_X, PT_DYNAMIC, PT_GNU_EH_FRAME,
    PT_GNU_STACK, PT_INTERP, PT_LOAD, PT_PHDR, PT_TLS, SHN_ABS, SHN_COMMON, SHN_UNDEF, SHT_DYNSYM,
    SHT_FINI_ARRAY, SHT_GROUP, SHT_INIT_ARRAY, SHT_NOBITS, SHT_NULL, SHT_PROGBITS, SHT_REL,
    SHT_RELA, SHT_STRTAB, SHT_SYMTAB, STB_GLOBAL, STB_LOCAL, STB_WEAK, STT_FILE, STT_FUNC,
    STT_GNU_IFUNC, STT_OBJECT, STT_SECTION, STT_TLS, STV_DEFAULT,
};

// ── ELF32-specific constants ──────────────────────────────────────────────────
// These either differ in type (i32 vs i64 for DT_*) or aren't in the shared module.

pub(super) const SHT_NOTE: u32 = 7;
#[allow(dead_code)] // ELF standard section type, defined for reference
pub(super) const SHT_GNU_HASH: u32 = 0x6ffffff6;
#[allow(dead_code)] // ELF standard section type, defined for reference
pub(super) const SHT_GNU_VERSYM_CONST: u32 = 0x6fffffff;
#[allow(dead_code)] // ELF standard section type, defined for reference
pub(super) const SHT_GNU_VERNEED: u32 = 0x6ffffffe;

// Section flags (i686 uses u32 instead of shared module's u64)
pub(super) const SHF_WRITE: u32 = 0x1;
pub(super) const SHF_ALLOC: u32 = 0x2;
pub(super) const SHF_EXECINSTR: u32 = 0x4;
#[allow(dead_code)] // ELF standard section flag, defined for reference
pub(super) const SHF_MERGE: u32 = 0x10;
#[allow(dead_code)] // ELF standard section flag, defined for reference
pub(super) const SHF_STRINGS: u32 = 0x20;
#[allow(dead_code)] // ELF standard section flag, defined for reference
pub(super) const SHF_INFO_LINK: u32 = 0x40;
pub(super) const SHF_GROUP: u32 = 0x200;
pub(super) const SHF_TLS: u32 = 0x400;

// i386 relocation types
pub(super) const R_386_NONE: u32 = 0;
pub(super) const R_386_32: u32 = 1;
pub(super) const R_386_PC32: u32 = 2;
pub(super) const R_386_GOT32: u32 = 3;
pub(super) const R_386_PLT32: u32 = 4;
pub(super) const R_386_GOTOFF: u32 = 9;
pub(super) const R_386_GOTPC: u32 = 10;
pub(super) const R_386_TLS_TPOFF: u32 = 14;
pub(super) const R_386_TLS_IE: u32 = 15;
pub(super) const R_386_TLS_GOTIE: u32 = 16;
pub(super) const R_386_TLS_LE: u32 = 17;
pub(super) const R_386_TLS_GD: u32 = 18;
pub(super) const R_386_TLS_LE_32: u32 = 34;
pub(super) const R_386_TLS_DTPMOD32: u32 = 35;
pub(super) const R_386_TLS_DTPOFF32: u32 = 36;
pub(super) const R_386_TLS_TPOFF32: u32 = 37;
pub(super) const R_386_COPY: u32 = 5;
pub(super) const R_386_GLOB_DAT: u32 = 6;
pub(super) const R_386_JMP_SLOT: u32 = 7;
pub(super) const R_386_RELATIVE: u32 = 8;
pub(super) const R_386_IRELATIVE: u32 = 42;
pub(super) const R_386_GOT32X: u32 = 43;

// Dynamic tags (i32 for ELF32, vs i64 in the shared module)
pub(super) const DT_NULL: i32 = 0;
pub(super) const DT_NEEDED: i32 = 1;
pub(super) const DT_PLTRELSZ: i32 = 2;
pub(super) const DT_PLTGOT: i32 = 3;
pub(super) const DT_STRTAB: i32 = 5;
pub(super) const DT_SYMTAB: i32 = 6;
pub(super) const DT_STRSZ: i32 = 10;
pub(super) const DT_SYMENT: i32 = 11;
pub(super) const DT_INIT: i32 = 12;
pub(super) const DT_FINI: i32 = 13;
pub(super) const DT_REL: i32 = 17;
pub(super) const DT_RELSZ: i32 = 18;
pub(super) const DT_RELENT: i32 = 19;
pub(super) const DT_PLTREL: i32 = 20;
pub(super) const DT_DEBUG: i32 = 21;
pub(super) const DT_TEXTREL: i32 = 22;
pub(super) const DT_JMPREL: i32 = 23;
pub(super) const DT_INIT_ARRAY: i32 = 25;
pub(super) const DT_FINI_ARRAY: i32 = 26;
pub(super) const DT_INIT_ARRAYSZ: i32 = 27;
pub(super) const DT_FINI_ARRAYSZ: i32 = 28;
pub(super) const DT_SONAME: i32 = 14;
pub(super) const DT_FLAGS: i32 = 30;
pub(super) const DT_GNU_HASH_TAG: i32 = 0x6ffffef5u32 as i32;
pub(super) const DT_VERNEED: i32 = 0x6ffffffe_u32 as i32;
pub(super) const DT_VERNEEDNUM: i32 = 0x6fffffff_u32 as i32;
pub(super) const DT_VERSYM: i32 = 0x6ffffff0_u32 as i32;

pub(super) const PAGE_SIZE: u32 = 0x1000;
pub(super) const BASE_ADDR: u32 = 0x08048000;
pub(super) const INTERP: &[u8] = b"/lib/ld-linux.so.2\0";

// ── ELF32 structures ────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub(super) struct Elf32Sym {
    pub name: u32,
    pub value: u32,
    pub size: u32,
    pub info: u8,
    pub other: u8,
    pub shndx: u16,
}

#[allow(dead_code)] // Convenience accessors; not all used by every code path
impl Elf32Sym {
    pub fn binding(&self) -> u8 {
        self.info >> 4
    }
    pub fn sym_type(&self) -> u8 {
        self.info & 0xf
    }
}

#[derive(Clone, Debug)]
pub(super) struct Elf32Shdr {
    pub name: u32,
    pub sh_type: u32,
    pub flags: u32,
    #[allow(dead_code)] // Populated during ELF parsing; not yet read by linker
    pub addr: u32,
    pub offset: u32,
    pub size: u32,
    pub link: u32,
    pub info: u32,
    pub addralign: u32,
    pub entsize: u32,
}

// ── Input object types ────────────────────────────────────────────────────────

/// A parsed section from an input .o file.
#[derive(Clone)]
pub(super) struct InputSection {
    pub name: String,
    pub sh_type: u32,
    pub flags: u32,
    pub data: Vec<u8>,
    pub align: u32,
    /// Relocations: (offset, rel_type, sym_idx_in_input, addend)
    pub relocations: Vec<(u32, u32, u32, i32)>,
    /// Index in the input file's section header table.
    pub input_index: usize,
    #[allow(dead_code)] // Populated during ELF parsing; preserved for future SHF_MERGE support
    pub entsize: u32,
    #[allow(dead_code)] // Populated during ELF parsing; preserved for section linking
    pub link: u32,
    pub info: u32,
}

/// A parsed symbol from an input .o file.
#[derive(Clone, Debug)]
pub(super) struct InputSymbol {
    pub name: String,
    pub value: u32,
    pub size: u32,
    pub binding: u8,
    pub sym_type: u8,
    #[allow(dead_code)] // Parsed from ELF; needed for future STV_HIDDEN/STV_PROTECTED handling
    pub visibility: u8,
    pub section_index: u16,
}

/// A parsed input file.
pub(super) struct InputObject {
    pub sections: Vec<InputSection>,
    pub symbols: Vec<InputSymbol>,
    pub filename: String,
}

// ── Linker state types ────────────────────────────────────────────────────────

/// Resolved symbol info in the linker.
#[derive(Clone, Debug)]
pub(super) struct LinkerSymbol {
    pub address: u32,
    pub size: u32,
    pub sym_type: u8,
    pub binding: u8,
    #[allow(dead_code)] // Tracked for future STV_HIDDEN/STV_PROTECTED handling
    pub visibility: u8,
    pub is_defined: bool,
    pub needs_plt: bool,
    pub needs_got: bool,
    pub output_section: usize,
    pub section_offset: u32,
    pub plt_index: usize,
    pub got_index: usize,
    pub is_dynamic: bool,
    pub dynlib: String,
    pub needs_copy: bool,
    pub copy_addr: u32,
    pub version: Option<String>,
    /// Whether this dynamic data symbol uses text relocations instead of COPY.
    pub uses_textrel: bool,
}

/// A merged output section.
pub(super) struct OutputSection {
    pub name: String,
    pub sh_type: u32,
    pub flags: u32,
    pub data: Vec<u8>,
    pub align: u32,
    pub addr: u32,
    pub file_offset: u32,
}

/// Maps (object_index, section_index) -> (output_section_index, offset_in_output).
pub(super) type SectionMap = HashMap<(usize, usize), (usize, u32)>;

/// Info about a dynamic symbol from a shared library.
pub(super) struct DynSymInfo {
    pub name: String,
    pub sym_type: u8,
    pub size: u32,
    #[allow(dead_code)] // Parsed from .so; needed for future weak-vs-global symbol preference
    pub binding: u8,
    pub version: Option<String>,
    pub is_default_ver: bool,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

pub(super) fn align_up(value: u32, align: u32) -> u32 {
    if align == 0 {
        return value;
    }
    (value + align - 1) & !(align - 1)
}

/// Append an ELF32 dynamic entry (8 bytes: tag + value).
pub(super) fn push_dyn(data: &mut Vec<u8>, tag: i32, val: u32) {
    data.extend_from_slice(&tag.to_le_bytes());
    data.extend_from_slice(&val.to_le_bytes());
}

/// Determine the output section name for an input section.
///
/// Returns `None` for sections that should not be included in the output
/// (metadata sections, non-allocated sections, etc.).
pub(super) fn output_section_name(name: &str, flags: u32, sh_type: u32) -> Option<String> {
    // Skip non-allocatable sections, symbol tables, relocation sections, etc.
    if sh_type == SHT_NULL
        || sh_type == SHT_SYMTAB
        || sh_type == SHT_STRTAB
        || sh_type == SHT_REL
        || sh_type == SHT_RELA
        || sh_type == SHT_GROUP
    {
        return None;
    }
    if name == ".note.GNU-stack" || name == ".comment" {
        return None;
    }

    // Group by canonical output section name
    if name.starts_with(".text") || name == ".init" || name == ".fini" {
        if name == ".init" {
            return Some(".init".to_string());
        }
        if name == ".fini" {
            return Some(".fini".to_string());
        }
        return Some(".text".to_string());
    }
    if name.starts_with(".rodata") {
        return Some(".rodata".to_string());
    }
    if name == ".eh_frame" {
        return Some(".eh_frame".to_string());
    }
    if name == ".tbss" || name.starts_with(".tbss.") {
        return Some(".tbss".to_string());
    }
    if name == ".tdata" || name.starts_with(".tdata.") {
        return Some(".tdata".to_string());
    }
    if flags & SHF_TLS != 0 {
        return if sh_type == SHT_NOBITS {
            Some(".tbss".to_string())
        } else {
            Some(".tdata".to_string())
        };
    }
    if name.starts_with(".data") {
        return Some(".data".to_string());
    }
    if name.starts_with(".bss") || sh_type == SHT_NOBITS {
        return Some(".bss".to_string());
    }
    if name == ".init_array" || name.starts_with(".init_array.") {
        return Some(".init_array".to_string());
    }
    if name == ".fini_array" || name.starts_with(".fini_array.") {
        return Some(".fini_array".to_string());
    }
    if name.starts_with(".note.") && sh_type == SHT_NOTE {
        return Some(".note".to_string());
    }
    if name.starts_with(".tm_clone_table") {
        return Some(".data".to_string());
    }

    // For alloc sections whose names are valid C identifiers, preserve the
    // original name. This is needed for __start_<section> / __stop_<section>
    // symbol auto-generation (GNU ld feature). Without this, custom sections
    // like "my_cbs" would be merged into .text/.data/.rodata and the linker
    // could not resolve __start_my_cbs / __stop_my_cbs symbols.
    if flags & SHF_ALLOC != 0 {
        if crate::backend::linker_common::is_valid_c_identifier_for_section(name) {
            return Some(name.to_string());
        }
        // Fall back to flag-based grouping for other alloc sections
        // (e.g. .gcc_except_table, .stapsdt.base, .gnu.warning.*)
        if flags & SHF_EXECINSTR != 0 {
            return Some(".text".to_string());
        }
        if flags & SHF_WRITE != 0 {
            return if sh_type == SHT_NOBITS {
                Some(".bss".to_string())
            } else {
                Some(".data".to_string())
            };
        }
        return Some(".rodata".to_string());
    }

    None
}
