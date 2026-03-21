//! ELF64 parsing for the RISC-V linker.
//!
//! This module re-exports the shared ELF64 types and parser from `linker_common`,
//! plus provides RISC-V-specific constants. The actual parsing logic lives in the
//! shared module to avoid duplication with x86 and ARM.

pub use crate::backend::elf::{
    get_standard_linker_symbols, is_thin_archive, parse_archive_members,
    parse_thin_archive_members, LinkerSymbolAddresses, EM_RISCV, SHF_ALLOC, SHF_EXECINSTR, SHF_TLS,
    SHF_WRITE, SHN_ABS, SHN_COMMON, SHN_UNDEF, SHT_GROUP, SHT_NOBITS, SHT_PROGBITS, SHT_RELA,
    SHT_STRTAB, SHT_SYMTAB, STB_GLOBAL, STB_LOCAL, STB_WEAK, STT_FUNC, STT_NOTYPE, STT_OBJECT,
    STT_SECTION, STV_DEFAULT,
};

use crate::backend::linker_common;

// RISC-V specific constants
pub const SHT_RISCV_ATTRIBUTES: u32 = 0x70000003;

// ── Type aliases ─────────────────────────────────────────────────────────
// Re-export shared types under the names the RISC-V linker uses.

pub type Symbol = linker_common::Elf64Symbol;
pub type ElfObject = linker_common::Elf64Object;
pub type DynSymbol = linker_common::DynSymbol;

// ── Parsing functions ────────────────────────────────────────────────────
// Delegate to shared implementations.

pub fn parse_object(data: &[u8], source_name: &str) -> Result<ElfObject, String> {
    linker_common::parse_elf64_object(data, source_name, EM_RISCV)
}

/// Parse a .a static archive and return all ELF .o members.
pub fn parse_archive(data: &[u8]) -> Result<Vec<(String, ElfObject)>, String> {
    let members = parse_archive_members(data)?;
    let mut results = Vec::new();
    for (name, offset, size) in members {
        let member_data = &data[offset..offset + size];
        if member_data.len() >= 4 && &member_data[0..4] == b"\x7fELF" {
            let full_name = format!("archive({})", name);
            if let Ok(obj) = parse_object(member_data, &full_name) {
                results.push((name, obj));
            }
        }
    }
    Ok(results)
}

/// Parse a GNU thin archive and return all ELF .o members.
/// In thin archives, member data is stored in external files referenced by name.
pub fn parse_thin_archive(
    data: &[u8],
    archive_path: &str,
) -> Result<Vec<(String, ElfObject)>, String> {
    let member_names = parse_thin_archive_members(data)?;
    let archive_dir = std::path::Path::new(archive_path)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut results = Vec::new();
    for name in member_names {
        let member_path = archive_dir.join(&name);
        let member_data = std::fs::read(&member_path).map_err(|e| {
            format!(
                "thin archive {}: failed to read member '{}': {}",
                archive_path,
                member_path.display(),
                e
            )
        })?;
        if member_data.len() >= 4 && &member_data[0..4] == b"\x7fELF" {
            let full_name = format!("archive({})", name);
            if let Ok(obj) = parse_object(&member_data, &full_name) {
                results.push((name, obj));
            }
        }
    }
    Ok(results)
}

pub fn parse_shared_library_symbols(data: &[u8], lib_name: &str) -> Result<Vec<DynSymbol>, String> {
    linker_common::parse_shared_library_symbols(data, lib_name)
}

/// Read dynamic symbols from a shared library file path.
///
/// Convenience wrapper that reads the file and delegates to
/// `parse_shared_library_symbols`.
pub fn read_shared_lib_symbols(path: &str) -> Result<Vec<DynSymbol>, String> {
    let data = std::fs::read(path).map_err(|e| format!("Cannot read {}: {}", path, e))?;
    parse_shared_library_symbols(&data, path)
}
