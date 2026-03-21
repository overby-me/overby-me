//! Linker-defined symbols and section name helpers.
//!
//! Provides the standard set of symbols that all backend linkers define
//! (e.g. `_edata`, `__bss_start`, `_end`), plus section name/flags lookup.

use super::constants::*;

// ── Linker-defined symbols ────────────────────────────────────────────────────
//
// All four backend linkers (x86-64, i686, ARM, RISC-V) need to define a standard
// set of symbols that programs expect the linker to provide. Previously each
// backend had its own list with inconsistent names and values. This shared
// infrastructure ensures all backends define the same symbols with consistent
// semantics.

/// Addresses that linker backends must provide for linker-defined symbol resolution.
///
/// Each backend computes these from its own layout, then passes them to
/// `get_standard_linker_symbols()` to get the canonical symbol list.
pub struct LinkerSymbolAddresses {
    /// Base address of the ELF executable (e.g., 0x400000 for x86-64).
    pub base_addr: u64,
    /// Address of the GOT or GOT.PLT section.
    pub got_addr: u64,
    /// Address of the .dynamic section (0 if static-only linking).
    pub dynamic_addr: u64,
    /// Start address of the BSS section.
    pub bss_addr: u64,
    /// Size of the BSS section in memory.
    pub bss_size: u64,
    /// End of the text (RX) segment.
    pub text_end: u64,
    /// Start of the data (RW) segment.
    pub data_start: u64,
    /// Start address of .init_array section (0 if absent).
    pub init_array_start: u64,
    /// Size of .init_array section in bytes.
    pub init_array_size: u64,
    /// Start address of .fini_array section (0 if absent).
    pub fini_array_start: u64,
    /// Size of .fini_array section in bytes.
    pub fini_array_size: u64,
    /// Start address of .preinit_array section (0 if absent).
    pub preinit_array_start: u64,
    /// Size of .preinit_array section in bytes.
    pub preinit_array_size: u64,
    /// Start address of .rela.iplt / .rel.iplt section (0 if absent).
    pub rela_iplt_start: u64,
    /// Size of .rela.iplt / .rel.iplt section in bytes.
    pub rela_iplt_size: u64,
}

/// A linker-defined symbol entry with name, value, and binding.
pub struct LinkerDefinedSym {
    pub name: &'static str,
    pub value: u64,
    pub binding: u8,
}

/// Return the standard set of linker-defined symbols that all backends should provide.
///
/// This ensures consistent symbol definitions across x86-64, i686, ARM, and RISC-V.
/// Each backend may also define additional architecture-specific symbols (e.g.,
/// `__global_pointer$` for RISC-V) after calling this function.
///
/// The returned list uses the same semantics as GNU ld:
/// - `_edata` / `__bss_start` = start of BSS (end of initialized data)
/// - `_end` / `__end` = end of BSS (end of all data)
/// - `_etext` / `etext` = end of text segment
/// - `__dso_handle` = start of data segment
/// - `_DYNAMIC` = address of .dynamic section
/// - `data_start` is weak (can be overridden by object files)
pub fn get_standard_linker_symbols(addrs: &LinkerSymbolAddresses) -> Vec<LinkerDefinedSym> {
    let end_addr = addrs.bss_addr + addrs.bss_size;
    vec![
        // GOT / dynamic
        LinkerDefinedSym {
            name: "_GLOBAL_OFFSET_TABLE_",
            value: addrs.got_addr,
            binding: STB_GLOBAL,
        },
        LinkerDefinedSym {
            name: "_DYNAMIC",
            value: addrs.dynamic_addr,
            binding: STB_GLOBAL,
        },
        // BSS / data boundaries
        LinkerDefinedSym {
            name: "__bss_start",
            value: addrs.bss_addr,
            binding: STB_GLOBAL,
        },
        LinkerDefinedSym {
            name: "_edata",
            value: addrs.bss_addr,
            binding: STB_GLOBAL,
        },
        LinkerDefinedSym {
            name: "_end",
            value: end_addr,
            binding: STB_GLOBAL,
        },
        LinkerDefinedSym {
            name: "__end",
            value: end_addr,
            binding: STB_GLOBAL,
        },
        // Text boundaries
        LinkerDefinedSym {
            name: "_etext",
            value: addrs.text_end,
            binding: STB_GLOBAL,
        },
        LinkerDefinedSym {
            name: "etext",
            value: addrs.text_end,
            binding: STB_GLOBAL,
        },
        // ELF header / executable start
        LinkerDefinedSym {
            name: "__ehdr_start",
            value: addrs.base_addr,
            binding: STB_GLOBAL,
        },
        LinkerDefinedSym {
            name: "__executable_start",
            value: addrs.base_addr,
            binding: STB_GLOBAL,
        },
        // Data segment
        LinkerDefinedSym {
            name: "__dso_handle",
            value: addrs.data_start,
            binding: STB_GLOBAL,
        },
        LinkerDefinedSym {
            name: "__data_start",
            value: addrs.data_start,
            binding: STB_GLOBAL,
        },
        LinkerDefinedSym {
            name: "data_start",
            value: addrs.data_start,
            binding: STB_WEAK,
        },
        // Init/fini/preinit arrays
        LinkerDefinedSym {
            name: "__init_array_start",
            value: addrs.init_array_start,
            binding: STB_GLOBAL,
        },
        LinkerDefinedSym {
            name: "__init_array_end",
            value: addrs.init_array_start + addrs.init_array_size,
            binding: STB_GLOBAL,
        },
        LinkerDefinedSym {
            name: "__fini_array_start",
            value: addrs.fini_array_start,
            binding: STB_GLOBAL,
        },
        LinkerDefinedSym {
            name: "__fini_array_end",
            value: addrs.fini_array_start + addrs.fini_array_size,
            binding: STB_GLOBAL,
        },
        LinkerDefinedSym {
            name: "__preinit_array_start",
            value: addrs.preinit_array_start,
            binding: STB_GLOBAL,
        },
        LinkerDefinedSym {
            name: "__preinit_array_end",
            value: addrs.preinit_array_start + addrs.preinit_array_size,
            binding: STB_GLOBAL,
        },
        // IPLT relocation boundaries
        LinkerDefinedSym {
            name: "__rela_iplt_start",
            value: addrs.rela_iplt_start,
            binding: STB_GLOBAL,
        },
        LinkerDefinedSym {
            name: "__rela_iplt_end",
            value: addrs.rela_iplt_start + addrs.rela_iplt_size,
            binding: STB_GLOBAL,
        },
    ]
}

// ── Section name mapping ─────────────────────────────────────────────────────

/// Map a symbol's section name to its index in the section header table.
///
/// Handles special pseudo-sections used during assembly:
/// - `*COM*` → `SHN_COMMON` (0xFFF2) for COMMON symbols
/// - `*UND*` or empty → `SHN_UNDEF` (0) for undefined symbols
/// - Otherwise, looks up the section in the content section list (1-based index)
///
/// `shndx_offset` is the number of section headers before the content sections
/// (typically 1 for NULL, or 1 + num_groups when COMDAT groups are present).
pub fn section_index(section_name: &str, content_sections: &[String], shndx_offset: u16) -> u16 {
    if section_name == "*COM*" {
        SHN_COMMON
    } else if section_name == "*UND*" || section_name.is_empty() {
        SHN_UNDEF
    } else {
        content_sections
            .iter()
            .position(|s| s == section_name)
            .map(|i| (i as u16) + shndx_offset)
            .unwrap_or(SHN_UNDEF)
    }
}

/// Return default ELF section flags based on section name conventions.
///
/// These are the standard mappings: `.text.*` → alloc+exec, `.data.*` → alloc+write,
/// `.rodata.*` → alloc, `.bss.*` → alloc+write, `.tdata`/`.tbss` → alloc+write+TLS, etc.
pub fn default_section_flags(name: &str) -> u64 {
    if name == ".text" || name.starts_with(".text.") {
        SHF_ALLOC | SHF_EXECINSTR
    } else if name == ".data"
        || name.starts_with(".data.")
        || name == ".bss"
        || name.starts_with(".bss.")
    {
        SHF_ALLOC | SHF_WRITE
    } else if name == ".rodata" || name.starts_with(".rodata.") {
        SHF_ALLOC
    } else if name == ".note.GNU-stack" {
        0 // Non-executable stack marker, no flags
    } else if name.starts_with(".note") {
        SHF_ALLOC
    } else if name.starts_with(".tdata") || name.starts_with(".tbss") {
        SHF_ALLOC | SHF_WRITE | SHF_TLS
    } else if name.starts_with(".init") || name.starts_with(".fini") {
        SHF_ALLOC | SHF_EXECINSTR
    } else {
        0
    }
}
