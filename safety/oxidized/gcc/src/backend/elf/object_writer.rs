//! Shared relocatable ELF object (.o) writer.
//!
//! Provides `write_relocatable_object` which serializes an ELF .o file from
//! architecture-independent section/symbol/reloc data. Handles ELF64+RELA
//! (x86-64, AArch64, RISC-V), ELF32+RELA (RISC-V RV32), and ELF32+REL (i686).
//!
//! Each backend's ElfWriter builds its own section/symbol/reloc data through
//! arch-specific logic (instruction encoding, branch resolution, etc.), then
//! calls `write_relocatable_object` for the final ELF serialization step.

use super::constants::*;
use super::io::*;
use super::linker_symbols::section_index;
use super::string_table::StringTable;
use super::symbol_table::ObjSymbol;
use std::collections::HashMap;

/// Configuration for ELF object file emission. Parameterizes the format
/// differences between architectures (machine type, ELF class, flags, etc).
pub struct ElfConfig {
    /// ELF machine type (e.g., EM_X86_64, EM_AARCH64, EM_RISCV, EM_386)
    pub e_machine: u16,
    /// ELF flags (e.g., 0 for most, EF_RISCV_RVC | EF_RISCV_FLOAT_ABI_DOUBLE for RISC-V)
    pub e_flags: u32,
    /// ELF class: ELFCLASS64 or ELFCLASS32
    pub elf_class: u8,
    /// Force RELA relocations even for ELF32 (needed by RISC-V which always uses RELA).
    /// When false (default), ELF32 uses REL and ELF64 uses RELA.
    pub force_rela: bool,
}

/// A section in a relocatable object file being built by the assembler.
pub struct ObjSection {
    pub name: String,
    pub sh_type: u32,
    pub sh_flags: u64,
    pub data: Vec<u8>,
    pub sh_addralign: u64,
    /// Relocations targeting this section.
    pub relocs: Vec<ObjReloc>,
    /// If this section is part of a COMDAT group, the group signature symbol name.
    pub comdat_group: Option<String>,
}

/// A relocation entry in a relocatable object file.
///
/// Uses 64-bit offset/addend for all targets; the writer truncates to 32-bit
/// for ELF32/REL when needed.
#[derive(Clone)]
pub struct ObjReloc {
    pub offset: u64,
    pub reloc_type: u32,
    pub symbol_name: String,
    pub addend: i64,
}

/// Internal symbol table entry used during serialization.
struct SymEntry {
    st_name: u32,
    st_info: u8,
    st_other: u8,
    st_shndx: u16,
    st_value: u64,
    st_size: u64,
}

/// Write a relocatable ELF object file (.o) from assembled sections and symbols.
///
/// This is the shared serialization pipeline used by all four backend assemblers.
/// The caller is responsible for:
/// - Building sections with encoded instructions and data
/// - Resolving local branches and patching instruction bytes
/// - Building the symbol list (defined labels, COMMON, aliases, undefined)
/// - Providing the correct `ElfConfig` for the target architecture
///
/// The function handles the complete ELF layout and serialization:
/// 1. Build shstrtab/strtab string tables
/// 2. Build symbol table entries (NULL, section symbols, local, global)
/// 3. Compute section/rela/symtab/strtab layout offsets
/// 4. Write ELF header, section data, relocations, symtab, strtab, section headers
///
/// Returns the serialized ELF bytes on success.
pub fn write_relocatable_object(
    config: &ElfConfig,
    section_order: &[String],
    sections: &HashMap<String, ObjSection>,
    symbols: &[ObjSymbol],
) -> Result<Vec<u8>, String> {
    let is_32bit = config.elf_class == ELFCLASS32;
    // ELF64 always uses RELA; ELF32 defaults to REL but some architectures
    // (e.g., RISC-V) always use RELA even in 32-bit mode.
    let use_rela = !is_32bit || config.force_rela;

    let ehdr_size = if is_32bit {
        ELF32_EHDR_SIZE
    } else {
        ELF64_EHDR_SIZE
    };
    let shdr_size = if is_32bit {
        ELF32_SHDR_SIZE
    } else {
        ELF64_SHDR_SIZE
    };
    let sym_entry_size = if is_32bit {
        ELF32_SYM_SIZE
    } else {
        ELF64_SYM_SIZE
    };
    let reloc_entry_size = if use_rela {
        if is_32bit {
            ELF32_RELA_SIZE
        } else {
            ELF64_RELA_SIZE
        }
    } else {
        ELF32_REL_SIZE
    };
    let reloc_prefix = if use_rela { ".rela" } else { ".rel" };
    let reloc_sh_type = if use_rela { SHT_RELA } else { SHT_REL };
    let alignment_mask = if is_32bit { 3usize } else { 7usize }; // 4 or 8 byte alignment

    // ── Collect COMDAT groups ──
    // Map: group_name -> list of member content section names
    let mut comdat_groups: Vec<(String, Vec<String>)> = Vec::new();
    {
        let mut group_map: HashMap<String, Vec<String>> = HashMap::new();
        let mut group_order: Vec<String> = Vec::new();
        for sec_name in section_order {
            if let Some(section) = sections.get(sec_name) {
                if let Some(ref group_name) = section.comdat_group {
                    group_map
                        .entry(group_name.clone())
                        .or_insert_with(|| {
                            group_order.push(group_name.clone());
                            Vec::new()
                        })
                        .push(sec_name.clone());
                }
            }
        }
        for gname in group_order {
            if let Some(members) = group_map.remove(&gname) {
                comdat_groups.push((gname, members));
            }
        }
    }
    let num_groups = comdat_groups.len();

    // ── Build string tables ──
    let mut shstrtab = StringTable::new();
    let mut strtab = StringTable::new();

    let content_sections: &[String] = section_order;

    // Add group section names to shstrtab
    for _ in &comdat_groups {
        shstrtab.add(".group");
    }

    // Add section names to shstrtab
    for sec_name in content_sections {
        shstrtab.add(sec_name);
    }
    shstrtab.add(".symtab");
    shstrtab.add(".strtab");
    shstrtab.add(".shstrtab");

    // Build reloc section names
    let mut reloc_section_names: Vec<String> = Vec::new();
    for sec_name in content_sections {
        if let Some(section) = sections.get(sec_name) {
            if !section.relocs.is_empty() {
                let reloc_name = format!("{}{}", reloc_prefix, sec_name);
                shstrtab.add(&reloc_name);
                reloc_section_names.push(reloc_name);
            }
        }
    }

    // ── Build symbol table entries ──
    let mut sym_entries: Vec<SymEntry> = Vec::new();
    // Content sections start at shdr index: NULL + num_groups + content_index
    let content_shndx_offset = (num_groups + 1) as u16;

    // NULL symbol (index 0)
    sym_entries.push(SymEntry {
        st_name: 0,
        st_info: 0,
        st_other: 0,
        st_shndx: 0,
        st_value: 0,
        st_size: 0,
    });

    // Section symbols (one per content section)
    // Per ELF convention, section symbols have st_name=0 (unnamed).
    // Tools like the Linux kernel's modpost derive the name from the section
    // header and expect st_name=0 so these don't appear in symbol searches.
    for (i, sec_name) in content_sections.iter().enumerate() {
        strtab.add(sec_name);
        sym_entries.push(SymEntry {
            st_name: 0,
            st_info: (STB_LOCAL << 4) | STT_SECTION,
            st_other: 0,
            st_shndx: content_shndx_offset + i as u16,
            st_value: 0,
            st_size: 0,
        });
    }

    // Separate local and global symbols
    let mut local_syms: Vec<&ObjSymbol> = Vec::new();
    let mut global_syms: Vec<&ObjSymbol> = Vec::new();
    for sym in symbols {
        if sym.binding == STB_LOCAL {
            local_syms.push(sym);
        } else {
            global_syms.push(sym);
        }
    }

    let first_global_idx = sym_entries.len() + local_syms.len();

    for sym in &local_syms {
        let name_offset = strtab.add(&sym.name);
        let shndx = section_index(&sym.section_name, content_sections, content_shndx_offset);
        sym_entries.push(SymEntry {
            st_name: name_offset,
            st_info: (sym.binding << 4) | sym.sym_type,
            st_other: sym.visibility,
            st_shndx: shndx,
            st_value: sym.value,
            st_size: sym.size,
        });
    }

    for sym in &global_syms {
        let name_offset = strtab.add(&sym.name);
        let shndx = section_index(&sym.section_name, content_sections, content_shndx_offset);
        sym_entries.push(SymEntry {
            st_name: name_offset,
            st_info: (sym.binding << 4) | sym.sym_type,
            st_other: sym.visibility,
            st_shndx: shndx,
            st_value: sym.value,
            st_size: sym.size,
        });
    }

    // ── Build COMDAT group section data ──
    // Each group section contains: GRP_COMDAT flag (u32) + member section indices (u32 each)
    let mut group_section_data: Vec<Vec<u8>> = Vec::new();
    for (_group_name, members) in &comdat_groups {
        let mut data = Vec::with_capacity(4 + 4 * members.len());
        data.extend_from_slice(&GRP_COMDAT.to_le_bytes());
        for member_name in members {
            // Find the section header index of this member
            let member_idx = content_sections
                .iter()
                .position(|s| s == member_name)
                .map(|i| content_shndx_offset as u32 + i as u32)
                .unwrap_or(0);
            data.extend_from_slice(&member_idx.to_le_bytes());
        }
        group_section_data.push(data);
    }

    // ── Calculate layout ──
    let mut offset = ehdr_size;

    // Group section offsets (come first, before content sections)
    let mut group_offsets: Vec<usize> = Vec::new();
    for gdata in &group_section_data {
        offset = (offset + 3) & !3; // align to 4 bytes
        group_offsets.push(offset);
        offset += gdata.len();
    }

    // Content section offsets
    let mut section_offsets: Vec<usize> = Vec::new();
    for sec_name in content_sections {
        let section = sections.get(sec_name).unwrap();
        let align = section.sh_addralign.max(1) as usize;
        offset = (offset + align - 1) & !(align - 1);
        section_offsets.push(offset);
        if section.sh_type != SHT_NOBITS {
            offset += section.data.len();
        }
    }

    // Reloc section offsets
    let mut reloc_offsets: Vec<usize> = Vec::new();
    for sec_name in content_sections {
        if let Some(section) = sections.get(sec_name) {
            if !section.relocs.is_empty() {
                offset = (offset + alignment_mask) & !alignment_mask;
                reloc_offsets.push(offset);
                offset += section.relocs.len() * reloc_entry_size;
            }
        }
    }

    // Symtab offset
    offset = (offset + alignment_mask) & !alignment_mask;
    let symtab_offset = offset;
    let symtab_size = sym_entries.len() * sym_entry_size;
    offset += symtab_size;

    // Strtab offset
    let strtab_offset = offset;
    let strtab_data = strtab.as_bytes().to_vec();
    offset += strtab_data.len();

    // Shstrtab offset
    let shstrtab_offset = offset;
    let shstrtab_data = shstrtab.as_bytes().to_vec();
    offset += shstrtab_data.len();

    // Section headers offset
    offset = (offset + alignment_mask) & !alignment_mask;
    let shdr_offset = offset;

    // Total section count: NULL + groups + content + relocs + symtab + strtab + shstrtab
    let num_sections = 1 + num_groups + content_sections.len() + reloc_section_names.len() + 3;
    let shstrtab_idx = num_sections - 1;
    let symtab_shndx = 1 + num_groups + content_sections.len() + reloc_section_names.len();

    // ── Write ELF ──
    let total_size = shdr_offset + num_sections * shdr_size;
    let mut elf = Vec::with_capacity(total_size);

    // ELF header (e_ident)
    elf.extend_from_slice(&ELF_MAGIC);
    elf.push(config.elf_class);
    elf.push(ELFDATA2LSB);
    elf.push(EV_CURRENT);
    elf.push(ELFOSABI_NONE);
    elf.extend_from_slice(&[0u8; 8]); // padding

    if is_32bit {
        // ELF32 header
        elf.extend_from_slice(&ET_REL.to_le_bytes());
        elf.extend_from_slice(&config.e_machine.to_le_bytes());
        elf.extend_from_slice(&1u32.to_le_bytes()); // e_version
        elf.extend_from_slice(&0u32.to_le_bytes()); // e_entry
        elf.extend_from_slice(&0u32.to_le_bytes()); // e_phoff
        elf.extend_from_slice(&(shdr_offset as u32).to_le_bytes());
        elf.extend_from_slice(&config.e_flags.to_le_bytes());
        elf.extend_from_slice(&(ehdr_size as u16).to_le_bytes());
        elf.extend_from_slice(&0u16.to_le_bytes()); // e_phentsize
        elf.extend_from_slice(&0u16.to_le_bytes()); // e_phnum
        elf.extend_from_slice(&(shdr_size as u16).to_le_bytes());
        elf.extend_from_slice(&(num_sections as u16).to_le_bytes());
        elf.extend_from_slice(&(shstrtab_idx as u16).to_le_bytes());
    } else {
        // ELF64 header
        elf.extend_from_slice(&ET_REL.to_le_bytes());
        elf.extend_from_slice(&config.e_machine.to_le_bytes());
        elf.extend_from_slice(&1u32.to_le_bytes()); // e_version
        elf.extend_from_slice(&0u64.to_le_bytes()); // e_entry
        elf.extend_from_slice(&0u64.to_le_bytes()); // e_phoff
        elf.extend_from_slice(&(shdr_offset as u64).to_le_bytes());
        elf.extend_from_slice(&config.e_flags.to_le_bytes());
        elf.extend_from_slice(&(ehdr_size as u16).to_le_bytes());
        elf.extend_from_slice(&0u16.to_le_bytes()); // e_phentsize
        elf.extend_from_slice(&0u16.to_le_bytes()); // e_phnum
        elf.extend_from_slice(&(shdr_size as u16).to_le_bytes());
        elf.extend_from_slice(&(num_sections as u16).to_le_bytes());
        elf.extend_from_slice(&(shstrtab_idx as u16).to_le_bytes());
    }

    debug_assert_eq!(elf.len(), ehdr_size);

    // ── Write group section data ──
    for (gi, gdata) in group_section_data.iter().enumerate() {
        while elf.len() < group_offsets[gi] {
            elf.push(0);
        }
        elf.extend_from_slice(gdata);
    }

    // ── Write content section data ──
    for (i, sec_name) in content_sections.iter().enumerate() {
        let section = sections.get(sec_name).unwrap();
        while elf.len() < section_offsets[i] {
            elf.push(0);
        }
        if section.sh_type != SHT_NOBITS {
            elf.extend_from_slice(&section.data);
        }
    }

    // ── Write relocation section data ──
    let mut reloc_idx = 0;
    for sec_name in content_sections {
        if let Some(section) = sections.get(sec_name) {
            if !section.relocs.is_empty() {
                while elf.len() < reloc_offsets[reloc_idx] {
                    elf.push(0);
                }
                for reloc in &section.relocs {
                    let sym_idx = find_symbol_index_shared(
                        &reloc.symbol_name,
                        &sym_entries,
                        &strtab,
                        content_sections,
                    );
                    if use_rela && !is_32bit {
                        write_rela64(
                            &mut elf,
                            reloc.offset,
                            sym_idx,
                            reloc.reloc_type,
                            reloc.addend,
                        );
                    } else if use_rela && is_32bit {
                        debug_assert!(reloc.reloc_type <= 255, "ELF32 reloc type must fit in u8");
                        debug_assert!(
                            reloc.addend >= i32::MIN as i64 && reloc.addend <= i32::MAX as i64,
                            "ELF32 RELA addend must fit in i32"
                        );
                        write_rela32(
                            &mut elf,
                            reloc.offset as u32,
                            sym_idx,
                            reloc.reloc_type as u8,
                            reloc.addend as i32,
                        );
                    } else {
                        debug_assert!(reloc.reloc_type <= 255, "ELF32 reloc type must fit in u8");
                        write_rel32(
                            &mut elf,
                            reloc.offset as u32,
                            sym_idx,
                            reloc.reloc_type as u8,
                        );
                    }
                }
                reloc_idx += 1;
            }
        }
    }

    // ── Write symtab ──
    while elf.len() < symtab_offset {
        elf.push(0);
    }
    for sym in &sym_entries {
        if is_32bit {
            write_sym32(
                &mut elf,
                sym.st_name,
                sym.st_value as u32,
                sym.st_size as u32,
                sym.st_info,
                sym.st_other,
                sym.st_shndx,
            );
        } else {
            write_sym64(
                &mut elf,
                sym.st_name,
                sym.st_info,
                sym.st_other,
                sym.st_shndx,
                sym.st_value,
                sym.st_size,
            );
        }
    }

    // ── Write strtab ──
    debug_assert_eq!(elf.len(), strtab_offset);
    elf.extend_from_slice(&strtab_data);

    // ── Write shstrtab ──
    debug_assert_eq!(elf.len(), shstrtab_offset);
    elf.extend_from_slice(&shstrtab_data);

    // ── Write section headers ──
    while elf.len() < shdr_offset {
        elf.push(0);
    }

    let strtab_shndx = symtab_shndx + 1;

    if is_32bit {
        // NULL
        write_shdr32(&mut elf, 0, SHT_NULL, 0, 0, 0, 0, 0, 0, 0, 0);
        // Group sections (COMDAT)
        for (gi, (group_name, _members)) in comdat_groups.iter().enumerate() {
            let sh_name = shstrtab.offset_of(".group");
            // sh_link = symtab index, sh_info = symbol index of group signature
            let sig_sym_idx =
                find_symbol_index_shared(group_name, &sym_entries, &strtab, content_sections);
            write_shdr32(
                &mut elf,
                sh_name,
                SHT_GROUP,
                0,
                0,
                group_offsets[gi] as u32,
                group_section_data[gi].len() as u32,
                symtab_shndx as u32,
                sig_sym_idx,
                4,
                4,
            );
        }
        // Content sections
        for (i, sec_name) in content_sections.iter().enumerate() {
            let section = sections.get(sec_name).unwrap();
            let sh_name = shstrtab.offset_of(sec_name);
            let sh_offset = if section.sh_type == SHT_NOBITS {
                0
            } else {
                section_offsets[i] as u32
            };
            write_shdr32(
                &mut elf,
                sh_name,
                section.sh_type,
                section.sh_flags as u32,
                0,
                sh_offset,
                section.data.len() as u32,
                0,
                0,
                section.sh_addralign as u32,
                0,
            );
        }
        // Reloc sections
        reloc_idx = 0;
        for (i, sec_name) in content_sections.iter().enumerate() {
            if let Some(section) = sections.get(sec_name) {
                if !section.relocs.is_empty() {
                    let reloc_name = format!("{}{}", reloc_prefix, sec_name);
                    let sh_name = shstrtab.offset_of(&reloc_name);
                    let sh_offset = reloc_offsets[reloc_idx] as u32;
                    let sh_size = (section.relocs.len() * reloc_entry_size) as u32;
                    write_shdr32(
                        &mut elf,
                        sh_name,
                        reloc_sh_type,
                        0,
                        0,
                        sh_offset,
                        sh_size,
                        symtab_shndx as u32,
                        content_shndx_offset as u32 + i as u32,
                        4,
                        reloc_entry_size as u32,
                    );
                    reloc_idx += 1;
                }
            }
        }
        // .symtab
        write_shdr32(
            &mut elf,
            shstrtab.offset_of(".symtab"),
            SHT_SYMTAB,
            0,
            0,
            symtab_offset as u32,
            symtab_size as u32,
            strtab_shndx as u32,
            first_global_idx as u32,
            4,
            sym_entry_size as u32,
        );
        // .strtab
        write_shdr32(
            &mut elf,
            shstrtab.offset_of(".strtab"),
            SHT_STRTAB,
            0,
            0,
            strtab_offset as u32,
            strtab_data.len() as u32,
            0,
            0,
            1,
            0,
        );
        // .shstrtab
        write_shdr32(
            &mut elf,
            shstrtab.offset_of(".shstrtab"),
            SHT_STRTAB,
            0,
            0,
            shstrtab_offset as u32,
            shstrtab_data.len() as u32,
            0,
            0,
            1,
            0,
        );
    } else {
        // NULL
        write_shdr64(&mut elf, 0, SHT_NULL, 0, 0, 0, 0, 0, 0, 0, 0);
        // Group sections (COMDAT)
        for (gi, (group_name, _members)) in comdat_groups.iter().enumerate() {
            let sh_name = shstrtab.offset_of(".group");
            let sig_sym_idx =
                find_symbol_index_shared(group_name, &sym_entries, &strtab, content_sections);
            write_shdr64(
                &mut elf,
                sh_name,
                SHT_GROUP,
                0,
                0,
                group_offsets[gi] as u64,
                group_section_data[gi].len() as u64,
                symtab_shndx as u32,
                sig_sym_idx,
                4,
                4,
            );
        }
        // Content sections
        for (i, sec_name) in content_sections.iter().enumerate() {
            let section = sections.get(sec_name).unwrap();
            let sh_name = shstrtab.offset_of(sec_name);
            let sh_offset = if section.sh_type == SHT_NOBITS {
                0
            } else {
                section_offsets[i] as u64
            };
            write_shdr64(
                &mut elf,
                sh_name,
                section.sh_type,
                section.sh_flags,
                0,
                sh_offset,
                section.data.len() as u64,
                0,
                0,
                section.sh_addralign,
                0,
            );
        }
        // Reloc sections
        reloc_idx = 0;
        for (i, sec_name) in content_sections.iter().enumerate() {
            if let Some(section) = sections.get(sec_name) {
                if !section.relocs.is_empty() {
                    let reloc_name = format!("{}{}", reloc_prefix, sec_name);
                    let sh_name = shstrtab.offset_of(&reloc_name);
                    let sh_offset = reloc_offsets[reloc_idx] as u64;
                    let sh_size = (section.relocs.len() * reloc_entry_size) as u64;
                    write_shdr64(
                        &mut elf,
                        sh_name,
                        reloc_sh_type,
                        SHF_INFO_LINK,
                        0,
                        sh_offset,
                        sh_size,
                        symtab_shndx as u32,
                        content_shndx_offset as u32 + i as u32,
                        8,
                        reloc_entry_size as u64,
                    );
                    reloc_idx += 1;
                }
            }
        }
        // .symtab
        write_shdr64(
            &mut elf,
            shstrtab.offset_of(".symtab"),
            SHT_SYMTAB,
            0,
            0,
            symtab_offset as u64,
            symtab_size as u64,
            strtab_shndx as u32,
            first_global_idx as u32,
            8,
            sym_entry_size as u64,
        );
        // .strtab
        write_shdr64(
            &mut elf,
            shstrtab.offset_of(".strtab"),
            SHT_STRTAB,
            0,
            0,
            strtab_offset as u64,
            strtab_data.len() as u64,
            0,
            0,
            1,
            0,
        );
        // .shstrtab
        write_shdr64(
            &mut elf,
            shstrtab.offset_of(".shstrtab"),
            SHT_STRTAB,
            0,
            0,
            shstrtab_offset as u64,
            shstrtab_data.len() as u64,
            0,
            0,
            1,
            0,
        );
    }

    Ok(elf)
}

/// Find a symbol's index in the ELF symbol table.
///
/// Checks section names first (returns section symbol index), then searches
/// by string table offset for named symbols (excluding section symbols).
fn find_symbol_index_shared(
    name: &str,
    sym_entries: &[SymEntry],
    strtab: &StringTable,
    content_sections: &[String],
) -> u32 {
    // Check if it's a section symbol
    for (i, sec_name) in content_sections.iter().enumerate() {
        if sec_name == name {
            return (i + 1) as u32; // +1 for NULL entry
        }
    }
    // Search named symbols
    let name_offset = strtab.offset_of(name);
    for (i, entry) in sym_entries.iter().enumerate() {
        if entry.st_name == name_offset && entry.st_info & 0xF != STT_SECTION {
            return i as u32;
        }
    }
    0 // undefined
}
