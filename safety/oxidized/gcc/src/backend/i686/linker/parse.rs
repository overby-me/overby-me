//! ELF32 object file parsing for the i686 linker.
//!
//! Handles parsing of relocatable ELF32 .o files, regular archives (.a),
//! and thin archives. This is separate from the ELF64 parser in `linker_common`
//! because ELF32 has different field widths (u32 vs u64 for addresses/offsets).

use std::collections::HashMap;

use super::types::*;

/// Parse an ELF32 relocatable object file.
pub(super) fn parse_elf32(data: &[u8], filename: &str) -> Result<InputObject, String> {
    if data.len() < 52 {
        return Err(format!("{}: too small for ELF header", filename));
    }
    if data[0..4] != ELF_MAGIC {
        return Err(format!("{}: not an ELF file", filename));
    }
    if data[4] != ELFCLASS32 {
        return Err(format!("{}: not ELF32", filename));
    }
    if data[5] != ELFDATA2LSB {
        return Err(format!("{}: not little-endian", filename));
    }
    let e_type = read_u16(data, 16);
    if e_type != ET_REL {
        return Err(format!(
            "{}: not a relocatable object (type={})",
            filename, e_type
        ));
    }
    let e_machine = read_u16(data, 18);
    if e_machine != EM_386 {
        return Err(format!("{}: not i386 (machine={})", filename, e_machine));
    }

    let e_shoff = read_u32(data, 32) as usize;
    let e_shentsize = read_u16(data, 46) as usize;
    let e_shnum = read_u16(data, 48) as usize;
    let e_shstrndx = read_u16(data, 50) as usize;

    // Parse section headers
    let mut shdrs = Vec::with_capacity(e_shnum);
    for i in 0..e_shnum {
        let off = e_shoff + i * e_shentsize;
        shdrs.push(Elf32Shdr {
            name: read_u32(data, off),
            sh_type: read_u32(data, off + 4),
            flags: read_u32(data, off + 8),
            addr: read_u32(data, off + 12),
            offset: read_u32(data, off + 16),
            size: read_u32(data, off + 20),
            link: read_u32(data, off + 24),
            info: read_u32(data, off + 28),
            addralign: read_u32(data, off + 32),
            entsize: read_u32(data, off + 36),
        });
    }

    // Read section name string table
    let shstrtab = &shdrs[e_shstrndx];
    let shstrtab_data = &data[shstrtab.offset as usize..(shstrtab.offset + shstrtab.size) as usize];

    // Find symtab and strtab
    let mut symtab_idx = None;
    let mut strtab_data: &[u8] = &[];
    for (i, shdr) in shdrs.iter().enumerate() {
        if shdr.sh_type == SHT_SYMTAB {
            symtab_idx = Some(i);
            let str_idx = shdr.link as usize;
            let str_shdr = &shdrs[str_idx];
            strtab_data =
                &data[str_shdr.offset as usize..(str_shdr.offset + str_shdr.size) as usize];
        }
    }

    // Parse symbols
    let mut symbols = Vec::new();
    if let Some(si) = symtab_idx {
        let sym_shdr = &shdrs[si];
        let sym_count = sym_shdr.size / sym_shdr.entsize;
        for j in 0..sym_count {
            let off = (sym_shdr.offset + j * sym_shdr.entsize) as usize;
            let st_name = read_u32(data, off);
            let st_value = read_u32(data, off + 4);
            let st_size = read_u32(data, off + 8);
            let st_info = data[off + 12];
            let st_other = data[off + 13];
            let st_shndx = read_u16(data, off + 14);
            let mut sym_name = read_cstr(strtab_data, st_name as usize);
            // Defense-in-depth: strip @PLT suffix from symbol names.
            if sym_name.ends_with("@PLT") {
                sym_name.truncate(sym_name.len() - 4);
            }
            symbols.push(InputSymbol {
                name: sym_name,
                value: st_value,
                size: st_size,
                binding: st_info >> 4,
                sym_type: st_info & 0xf,
                visibility: st_other & 3,
                section_index: st_shndx,
            });
        }
    }

    // Build relocation map: section index -> list of REL section indices
    let mut rel_map: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, shdr) in shdrs.iter().enumerate() {
        if shdr.sh_type == SHT_REL {
            rel_map.entry(shdr.info as usize).or_default().push(i);
        }
    }

    // Parse sections with their relocations
    let mut sections = Vec::with_capacity(e_shnum);
    for (i, shdr) in shdrs.iter().enumerate() {
        let sec_name = read_cstr(shstrtab_data, shdr.name as usize);
        let sec_data = if shdr.sh_type != SHT_NOBITS && shdr.size > 0 {
            data[shdr.offset as usize..(shdr.offset + shdr.size) as usize].to_vec()
        } else {
            vec![0u8; shdr.size as usize]
        };

        let mut relocs = Vec::new();
        if let Some(rel_indices) = rel_map.get(&i) {
            for &ri in rel_indices {
                let rel_shdr = &shdrs[ri];
                let count = rel_shdr.size / rel_shdr.entsize.max(8);
                for j in 0..count {
                    let roff = (rel_shdr.offset + j * rel_shdr.entsize.max(8)) as usize;
                    let r_offset = read_u32(data, roff);
                    let r_info = read_u32(data, roff + 4);
                    let sym_idx = r_info >> 8;
                    let rel_type = r_info & 0xff;
                    // For REL (not RELA), the addend is implicit in the section data
                    let addend =
                        if rel_type != R_386_NONE && (r_offset as usize + 4) <= sec_data.len() {
                            read_i32(&sec_data, r_offset as usize)
                        } else {
                            0
                        };
                    relocs.push((r_offset, rel_type, sym_idx, addend));
                }
            }
        }

        sections.push(InputSection {
            name: sec_name,
            sh_type: shdr.sh_type,
            flags: shdr.flags,
            data: sec_data,
            align: shdr.addralign.max(1),
            relocations: relocs,
            input_index: i,
            entsize: shdr.entsize,
            link: shdr.link,
            info: shdr.info,
        });
    }

    Ok(InputObject {
        sections,
        symbols,
        filename: filename.to_string(),
    })
}

/// Parse a regular (.a) archive, returning ELF32 members.
pub(super) fn parse_archive(
    data: &[u8],
    _filename: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let raw_members = parse_archive_members(data)?;
    let mut members = Vec::new();
    for (name, offset, size) in raw_members {
        let content = &data[offset..offset + size];
        if content.len() >= 4 && content[0..4] == ELF_MAGIC {
            members.push((name, content.to_vec()));
        }
    }
    Ok(members)
}

/// Parse a GNU thin archive, reading member data from external files.
pub(super) fn parse_thin_archive_i686(
    data: &[u8],
    archive_path: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let member_names = parse_thin_archive_members(data)?;
    let archive_dir = std::path::Path::new(archive_path)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut members = Vec::new();
    for name in member_names {
        let member_path = archive_dir.join(&name);
        let content = std::fs::read(&member_path).map_err(|e| {
            format!(
                "thin archive {}: failed to read member '{}': {}",
                archive_path,
                member_path.display(),
                e
            )
        })?;
        if content.len() >= 4 && content[0..4] == ELF_MAGIC {
            members.push((name, content));
        }
    }
    Ok(members)
}
