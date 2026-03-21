//! Shared library (.so) symbol parsing.
//!
//! Extracts dynamic symbols from shared libraries by reading `.dynsym` via section
//! headers, or falling back to program headers (`PT_DYNAMIC`) for stripped libraries.
//! Also provides SONAME extraction via `parse_soname()`.

use super::types::DynSymbol;
use crate::backend::elf::{
    read_cstr, read_i64, read_u16, read_u32, read_u64, DT_GNU_HASH, DT_NULL, DT_SONAME, DT_STRSZ,
    DT_STRTAB, DT_SYMTAB, DT_VERSYM, ELFCLASS64, ELFDATA2LSB, ELF_MAGIC, ET_DYN, PT_DYNAMIC,
    SHN_UNDEF, SHT_DYNAMIC, SHT_DYNSYM, SHT_GNU_VERDEF, SHT_GNU_VERSYM,
};

/// Extract dynamic symbols from a shared library (.so) file.
///
/// Reads the .dynsym section to find exported symbols. Used by x86 and RISC-V
/// linkers for dynamic linking resolution.
pub fn parse_shared_library_symbols(data: &[u8], lib_name: &str) -> Result<Vec<DynSymbol>, String> {
    if data.len() < 64 {
        return Err(format!("{}: file too small for ELF header", lib_name));
    }
    if data[0..4] != ELF_MAGIC {
        return Err(format!("{}: not an ELF file", lib_name));
    }
    if data[4] != ELFCLASS64 || data[5] != ELFDATA2LSB {
        return Err(format!("{}: not 64-bit little-endian ELF", lib_name));
    }

    let e_type = read_u16(data, 16);
    if e_type != ET_DYN {
        return Err(format!(
            "{}: not a shared library (type={})",
            lib_name, e_type
        ));
    }

    let e_shoff = read_u64(data, 40) as usize;
    let e_shentsize = read_u16(data, 58) as usize;
    let e_shnum = read_u16(data, 60) as usize;

    // Try section headers first (the standard approach)
    if e_shoff != 0 && e_shnum != 0 {
        let mut sections = Vec::with_capacity(e_shnum);
        for i in 0..e_shnum {
            let off = e_shoff + i * e_shentsize;
            if off + e_shentsize > data.len() {
                break;
            }
            sections.push((
                read_u32(data, off + 4),  // sh_type
                read_u64(data, off + 24), // offset
                read_u64(data, off + 32), // size
                read_u32(data, off + 40), // link
            ));
        }

        // Locate .gnu.version (SHT_GNU_VERSYM) and .gnu.verdef (SHT_GNU_VERDEF) sections
        let mut versym_shdr: Option<(usize, usize)> = None; // (offset, size)
        let mut verdef_shdr: Option<(usize, usize, usize)> = None; // (offset, size, link)
        for &(sh_type, offset, size, link) in &sections {
            if sh_type == SHT_GNU_VERSYM {
                versym_shdr = Some((offset as usize, size as usize));
            } else if sh_type == SHT_GNU_VERDEF {
                verdef_shdr = Some((offset as usize, size as usize, link as usize));
            }
        }

        // Parse version definitions to build index -> version string mapping
        let mut ver_names: std::collections::HashMap<u16, String> =
            std::collections::HashMap::new();
        if let Some((vd_off, vd_size, vd_link)) = verdef_shdr {
            // Get the string table for verdef (typically the dynstr)
            let vd_strtab = if vd_link < sections.len() {
                let (_, s_off, s_sz, _) = sections[vd_link];
                let s_off = s_off as usize;
                let s_sz = s_sz as usize;
                if s_off + s_sz <= data.len() {
                    &data[s_off..s_off + s_sz]
                } else {
                    &[] as &[u8]
                }
            } else {
                &[] as &[u8]
            };

            let mut pos = vd_off;
            let end = vd_off + vd_size;
            while pos < end && pos + 20 <= data.len() {
                let vd_ndx = read_u16(data, pos + 4);
                let vd_cnt = read_u16(data, pos + 6);
                let vd_aux = read_u32(data, pos + 12) as usize;
                let vd_next = read_u32(data, pos + 16) as usize;

                // First verdaux entry has the version name
                if vd_cnt > 0 {
                    let aux_pos = pos + vd_aux;
                    if aux_pos + 8 <= data.len() {
                        let vda_name = read_u32(data, aux_pos) as usize;
                        if vda_name < vd_strtab.len() {
                            let name = read_cstr(vd_strtab, vda_name);
                            ver_names.insert(vd_ndx, name);
                        }
                    }
                }

                if vd_next == 0 {
                    break;
                }
                pos += vd_next;
            }
        }

        // Find .dynsym and its string table
        for i in 0..sections.len() {
            let (sh_type, offset, size, link) = sections[i];
            if sh_type == SHT_DYNSYM {
                let strtab_idx = link as usize;
                if strtab_idx >= sections.len() {
                    continue;
                }
                let (_, str_off, str_size, _) = sections[strtab_idx];
                let str_off = str_off as usize;
                let str_size = str_size as usize;
                if str_off + str_size > data.len() {
                    continue;
                }
                let strtab = &data[str_off..str_off + str_size];

                let sym_off = offset as usize;
                let sym_size = size as usize;
                if sym_off + sym_size > data.len() {
                    continue;
                }
                let sym_data = &data[sym_off..sym_off + sym_size];
                let sym_count = sym_data.len() / 24;

                let mut symbols = Vec::new();
                for j in 1..sym_count {
                    let off = j * 24;
                    if off + 24 > sym_data.len() {
                        break;
                    }
                    let name_idx = read_u32(sym_data, off) as usize;
                    let info = sym_data[off + 4];
                    let shndx = read_u16(sym_data, off + 6);
                    let value = read_u64(sym_data, off + 8);
                    let size = read_u64(sym_data, off + 16);

                    if shndx == SHN_UNDEF {
                        continue;
                    }

                    // Check .gnu.version: if the hidden bit (0x8000) is set and
                    // the version index is >= 2, this is a non-default version
                    // (symbol@VERSION, not symbol@@VERSION). Such symbols should
                    // not be available for linking, matching GNU ld behavior.
                    if let Some((vs_off, vs_size)) = versym_shdr {
                        if vs_size >= sym_count * 2 && vs_off + vs_size <= data.len() {
                            let ver_entry = vs_off + j * 2;
                            let raw_ver = read_u16(data, ver_entry);
                            let hidden = raw_ver & 0x8000 != 0;
                            let ver_idx = raw_ver & 0x7fff;
                            if hidden && ver_idx >= 2 {
                                continue;
                            }
                        }
                    }

                    let name = read_cstr(strtab, name_idx);
                    if name.is_empty() {
                        continue;
                    }

                    // Look up version for this symbol from .gnu.version table
                    let (version, is_default_ver) = if let Some((vs_off, _vs_size)) = versym_shdr {
                        let vs_entry = vs_off + j * 2;
                        if vs_entry + 2 <= data.len() {
                            let raw_ver = read_u16(data, vs_entry);
                            let hidden = raw_ver & 0x8000 != 0;
                            let ver_idx = raw_ver & 0x7fff;
                            if ver_idx >= 2 {
                                (ver_names.get(&ver_idx).cloned(), !hidden)
                            } else {
                                (None, !hidden)
                            }
                        } else {
                            (None, true)
                        }
                    } else {
                        (None, true)
                    };

                    symbols.push(DynSymbol {
                        name,
                        info,
                        value,
                        size,
                        version,
                        is_default_ver,
                    });
                }
                return Ok(symbols);
            }
        }
    }

    // Fallback: use PT_DYNAMIC program header to find DT_SYMTAB/DT_STRTAB.
    // This handles shared libraries without section headers (e.g., our own
    // emitted .so files, or stripped libraries).
    parse_shared_library_symbols_from_phdrs(data, lib_name)
}

/// Parse dynamic symbols using program headers (PT_DYNAMIC) instead of section headers.
///
/// When a shared library has no section headers (e_shoff == 0), we can still find
/// the dynamic symbol table by:
/// 1. Locating PT_DYNAMIC in the program header table
/// 2. Reading DT_SYMTAB, DT_STRTAB, DT_STRSZ from the dynamic section
/// 3. Determining symtab size from DT_GNU_HASH (number of symbols) or by
///    scanning until we hit the strtab address
fn parse_shared_library_symbols_from_phdrs(
    data: &[u8],
    lib_name: &str,
) -> Result<Vec<DynSymbol>, String> {
    let e_phoff = read_u64(data, 32) as usize;
    let e_phentsize = read_u16(data, 54) as usize;
    let e_phnum = read_u16(data, 56) as usize;

    if e_phoff == 0 || e_phnum == 0 {
        return Err(format!(
            "{}: no program headers and no section headers",
            lib_name
        ));
    }

    // Find PT_DYNAMIC
    let mut dyn_offset = 0usize;
    let mut dyn_size = 0usize;
    for i in 0..e_phnum {
        let ph = e_phoff + i * e_phentsize;
        if ph + e_phentsize > data.len() {
            break;
        }
        let p_type = read_u32(data, ph);
        if p_type == PT_DYNAMIC {
            dyn_offset = read_u64(data, ph + 8) as usize;
            dyn_size = read_u64(data, ph + 32) as usize;
            break;
        }
    }

    if dyn_offset == 0 {
        return Err(format!("{}: no PT_DYNAMIC segment found", lib_name));
    }

    // Read dynamic entries to find DT_SYMTAB, DT_STRTAB, DT_STRSZ, DT_GNU_HASH, DT_VERSYM
    let mut symtab_addr: u64 = 0;
    let mut strtab_addr: u64 = 0;
    let mut strsz: u64 = 0;
    let mut gnu_hash_addr: u64 = 0;
    let mut versym_addr: u64 = 0;

    let mut pos = dyn_offset;
    let dyn_end = dyn_offset + dyn_size;
    while pos + 16 <= dyn_end && pos + 16 <= data.len() {
        let tag = read_i64(data, pos);
        let val = read_u64(data, pos + 8);
        match tag {
            x if x == DT_NULL => break,
            x if x == DT_SYMTAB => symtab_addr = val,
            x if x == DT_STRTAB => strtab_addr = val,
            x if x == DT_STRSZ => strsz = val,
            x if x == DT_GNU_HASH => gnu_hash_addr = val,
            x if x == DT_VERSYM => versym_addr = val,
            _ => {}
        }
        pos += 16;
    }

    if symtab_addr == 0 || strtab_addr == 0 {
        return Err(format!(
            "{}: missing DT_SYMTAB or DT_STRTAB in dynamic section",
            lib_name
        ));
    }

    // For shared libraries with base address 0 (PIC), the DT_ values are
    // virtual addresses. We need to convert them to file offsets.
    // For our emitted .so files, vaddr == file offset (base_addr = 0 and
    // segments are identity-mapped). For system .so files loaded at higher
    // addresses, we'd need to use the PT_LOAD mappings. Since we primarily
    // need this for our own .so output, use identity mapping and also try
    // PT_LOAD-based translation.
    let symtab_file_offset = vaddr_to_file_offset(data, e_phoff, e_phentsize, e_phnum, symtab_addr);
    let strtab_file_offset = vaddr_to_file_offset(data, e_phoff, e_phentsize, e_phnum, strtab_addr);

    if strtab_file_offset + strsz as usize > data.len() {
        return Err(format!("{}: strtab extends beyond file", lib_name));
    }
    let strtab = &data[strtab_file_offset..strtab_file_offset + strsz as usize];

    // Determine number of dynamic symbols. We can get this from .gnu.hash
    // (the symoffset + number of hashed symbols), or by scanning symbols
    // until we reach the strtab address.
    let sym_count = if gnu_hash_addr != 0 {
        let gnu_hash_file_offset =
            vaddr_to_file_offset(data, e_phoff, e_phentsize, e_phnum, gnu_hash_addr);
        count_dynsyms_from_gnu_hash(data, gnu_hash_file_offset)
    } else {
        // Fallback: symtab ends where strtab begins (they're typically adjacent)
        let sym_size = if strtab_file_offset > symtab_file_offset {
            strtab_file_offset - symtab_file_offset
        } else {
            // Can't determine size; try a reasonable max
            1024 * 24
        };
        sym_size / 24
    };

    // Resolve versym file offset if DT_VERSYM was found
    let versym_file_offset = if versym_addr != 0 {
        vaddr_to_file_offset(data, e_phoff, e_phentsize, e_phnum, versym_addr)
    } else {
        0
    };

    let mut symbols = Vec::new();
    for j in 1..sym_count {
        let off = symtab_file_offset + j * 24;
        if off + 24 > data.len() {
            break;
        }
        let name_idx = read_u32(data, off) as usize;
        let info = data[off + 4];
        let shndx = read_u16(data, off + 6);
        let value = read_u64(data, off + 8);
        let size = read_u64(data, off + 16);

        if shndx == SHN_UNDEF {
            continue;
        }

        // Check versym: skip non-default (hidden) versioned symbols
        if versym_addr != 0 {
            let ver_entry = versym_file_offset + j * 2;
            if ver_entry + 2 <= data.len() {
                let raw_ver = read_u16(data, ver_entry);
                let hidden = raw_ver & 0x8000 != 0;
                let ver_idx = raw_ver & 0x7fff;
                if hidden && ver_idx >= 2 {
                    continue;
                }
            }
        }

        let name = read_cstr(strtab, name_idx);
        if name.is_empty() {
            continue;
        }

        symbols.push(DynSymbol {
            name,
            info,
            value,
            size,
            version: None,
            is_default_ver: true,
        });
    }

    Ok(symbols)
}

/// Convert a virtual address to a file offset using PT_LOAD program headers.
pub(crate) fn vaddr_to_file_offset(
    data: &[u8],
    e_phoff: usize,
    e_phentsize: usize,
    e_phnum: usize,
    vaddr: u64,
) -> usize {
    use crate::backend::elf::PT_LOAD;
    for i in 0..e_phnum {
        let ph = e_phoff + i * e_phentsize;
        if ph + e_phentsize > data.len() {
            break;
        }
        let p_type = read_u32(data, ph);
        if p_type != PT_LOAD {
            continue;
        }
        let p_offset = read_u64(data, ph + 8);
        let p_vaddr = read_u64(data, ph + 16);
        let p_filesz = read_u64(data, ph + 32);
        if vaddr >= p_vaddr && vaddr < p_vaddr + p_filesz {
            return (p_offset + (vaddr - p_vaddr)) as usize;
        }
    }
    // If no PT_LOAD matches, assume identity mapping (vaddr == file offset)
    vaddr as usize
}

/// Count the total number of dynamic symbols from a .gnu.hash section.
///
/// The .gnu.hash header contains symoffset (first hashed symbol index).
/// We scan the hash chains to find the highest symbol index, then add 1.
fn count_dynsyms_from_gnu_hash(data: &[u8], offset: usize) -> usize {
    if offset + 16 > data.len() {
        return 0;
    }
    let nbuckets = read_u32(data, offset) as usize;
    let symoffset = read_u32(data, offset + 4) as usize;
    let bloom_size = read_u32(data, offset + 8) as usize;

    let buckets_off = offset + 16 + bloom_size * 8;
    let chains_off = buckets_off + nbuckets * 4;

    if buckets_off + nbuckets * 4 > data.len() {
        return symoffset;
    }

    // Find the maximum bucket value (highest starting symbol index)
    let mut max_sym = symoffset;
    for i in 0..nbuckets {
        let bucket_val = read_u32(data, buckets_off + i * 4) as usize;
        if bucket_val >= max_sym {
            // Walk the chain from this bucket to find the last symbol
            let mut idx = bucket_val;
            loop {
                let chain_pos = chains_off + (idx - symoffset) * 4;
                if chain_pos + 4 > data.len() {
                    break;
                }
                let chain_val = read_u32(data, chain_pos);
                if idx + 1 > max_sym {
                    max_sym = idx + 1;
                }
                if chain_val & 1 != 0 {
                    break;
                } // end of chain
                idx += 1;
            }
        }
    }

    max_sym
}

/// Get the SONAME from a shared library's .dynamic section.
///
/// Tries section headers first, then falls back to program headers (PT_DYNAMIC)
/// for shared libraries that lack section headers (e.g., our own emitted .so files).
pub fn parse_soname(data: &[u8]) -> Option<String> {
    if data.len() < 64 || data[0..4] != ELF_MAGIC {
        return None;
    }

    let e_shoff = read_u64(data, 40) as usize;
    let e_shentsize = read_u16(data, 58) as usize;
    let e_shnum = read_u16(data, 60) as usize;

    // Try section headers first
    if e_shoff != 0 && e_shnum != 0 {
        for i in 0..e_shnum {
            let off = e_shoff + i * e_shentsize;
            if off + 64 > data.len() {
                break;
            }
            let sh_type = read_u32(data, off + 4);
            if sh_type == SHT_DYNAMIC {
                let dyn_off = read_u64(data, off + 24) as usize;
                let dyn_size = read_u64(data, off + 32) as usize;
                let link = read_u32(data, off + 40) as usize;

                let str_sec_off = e_shoff + link * e_shentsize;
                if str_sec_off + 64 > data.len() {
                    return None;
                }
                let str_off = read_u64(data, str_sec_off + 24) as usize;
                let str_size = read_u64(data, str_sec_off + 32) as usize;
                if str_off + str_size > data.len() {
                    return None;
                }
                let strtab = &data[str_off..str_off + str_size];

                let mut pos = dyn_off;
                while pos + 16 <= dyn_off + dyn_size && pos + 16 <= data.len() {
                    let tag = read_i64(data, pos);
                    let val = read_u64(data, pos + 8);
                    if tag == DT_NULL {
                        break;
                    }
                    if tag == DT_SONAME {
                        return Some(read_cstr(strtab, val as usize));
                    }
                    pos += 16;
                }
                return None;
            }
        }
        return None;
    }

    // Fallback: use program headers (PT_DYNAMIC) to find the dynamic section
    let e_phoff = read_u64(data, 32) as usize;
    let e_phentsize = read_u16(data, 54) as usize;
    let e_phnum = read_u16(data, 56) as usize;

    if e_phoff == 0 || e_phnum == 0 {
        return None;
    }

    // Find PT_DYNAMIC
    let mut dyn_file_offset = 0usize;
    let mut dyn_filesz = 0usize;
    for i in 0..e_phnum {
        let ph = e_phoff + i * e_phentsize;
        if ph + e_phentsize > data.len() {
            break;
        }
        let p_type = read_u32(data, ph);
        if p_type == PT_DYNAMIC {
            dyn_file_offset = read_u64(data, ph + 8) as usize;
            dyn_filesz = read_u64(data, ph + 32) as usize;
            break;
        }
    }

    if dyn_file_offset == 0 {
        return None;
    }

    // First pass: find DT_STRTAB and DT_SONAME offset
    let mut strtab_addr: u64 = 0;
    let mut strsz: u64 = 0;
    let mut soname_offset: Option<u64> = None;

    let mut pos = dyn_file_offset;
    let dyn_end = dyn_file_offset + dyn_filesz;
    while pos + 16 <= dyn_end && pos + 16 <= data.len() {
        let tag = read_i64(data, pos);
        let val = read_u64(data, pos + 8);
        match tag {
            x if x == DT_NULL => break,
            x if x == DT_STRTAB => strtab_addr = val,
            x if x == DT_STRSZ => strsz = val,
            x if x == DT_SONAME => soname_offset = Some(val),
            _ => {}
        }
        pos += 16;
    }

    if strtab_addr == 0 || soname_offset.is_none() {
        return None;
    }

    let strtab_file_off = vaddr_to_file_offset(data, e_phoff, e_phentsize, e_phnum, strtab_addr);
    let name_off = soname_offset.unwrap() as usize;
    if strtab_file_off + name_off >= data.len() {
        return None;
    }
    if strsz > 0 && strtab_file_off + strsz as usize <= data.len() {
        let strtab = &data[strtab_file_off..strtab_file_off + strsz as usize];
        Some(read_cstr(strtab, name_off))
    } else {
        // Best effort: read from strtab_file_off + name_off
        Some(read_cstr(&data[strtab_file_off..], name_off))
    }
}
