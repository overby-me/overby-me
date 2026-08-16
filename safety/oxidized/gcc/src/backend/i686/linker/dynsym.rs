//! Dynamic symbol reading from ELF32 shared libraries.
//!
//! Reads dynamic symbols from .so files for the i686 linker, including
//! GNU version info (GLIBC versioning). Also handles GNU linker scripts
//! that reference actual .so files via GROUP/INPUT directives.
//!
//! This is ELF32-specific because shared libraries on i686 use 32-bit
//! ELF format with different field sizes than the ELF64 shared library
//! parser in `linker_common`.

use super::types::*;

/// Read dynamic symbol info, with library search paths for resolving linker script entries.
pub(super) fn read_dynsyms_with_search(
    path: &str,
    lib_search_paths: &[&str],
) -> Result<Vec<DynSymInfo>, String> {
    const LOCAL_SHT_GNU_VERSYM: u32 = 0x6fffffff;
    const SHT_GNU_VERDEF: u32 = 0x6ffffffd;

    let data = std::fs::read(path).map_err(|e| format!("cannot read {}: {}", path, e))?;
    if data.len() < 52 || data[0..4] != ELF_MAGIC || data[4] != ELFCLASS32 {
        // Check if this is a linker script
        if let Ok(text) = std::str::from_utf8(&data) {
            if let Some(entries) = parse_linker_script_entries(text) {
                return resolve_linker_script_syms(path, &entries, lib_search_paths);
            }
        }
        return Err(format!("{}: not a valid ELF32 file", path));
    }
    let e_type = read_u16(&data, 16);
    if e_type != ET_DYN {
        return Err(format!("{}: not a shared library (type={})", path, e_type));
    }

    let e_shoff = read_u32(&data, 32) as usize;
    let e_shentsize = read_u16(&data, 46) as usize;
    let e_shnum = read_u16(&data, 48) as usize;

    // First pass: find dynsym, versym, verdef sections
    let mut dynsym_idx = None;
    let mut versym_shdr: Option<(usize, usize)> = None;
    let mut verdef_shdr: Option<(usize, usize, usize)> = None;

    for i in 0..e_shnum {
        let off = e_shoff + i * e_shentsize;
        if off + 40 > data.len() {
            break;
        }
        let sh_type = read_u32(&data, off + 4);
        match sh_type {
            SHT_DYNSYM => {
                dynsym_idx = Some(i);
            }
            LOCAL_SHT_GNU_VERSYM => {
                let sh_offset = read_u32(&data, off + 16) as usize;
                let sh_size = read_u32(&data, off + 20) as usize;
                versym_shdr = Some((sh_offset, sh_size));
            }
            SHT_GNU_VERDEF => {
                let sh_offset = read_u32(&data, off + 16) as usize;
                let sh_size = read_u32(&data, off + 20) as usize;
                let sh_link = read_u32(&data, off + 24) as usize;
                verdef_shdr = Some((sh_offset, sh_size, sh_link));
            }
            _ => {}
        }
    }

    // Parse version definitions to build index -> version string mapping
    let ver_names = parse_verdef(&data, verdef_shdr, e_shoff, e_shentsize);

    let dynsym_i = match dynsym_idx {
        Some(i) => i,
        None => return Ok(Vec::new()),
    };

    // Read dynsym section
    let off = e_shoff + dynsym_i * e_shentsize;
    if off + 40 > data.len() {
        return Ok(Vec::new());
    }

    let sh_offset = read_u32(&data, off + 16) as usize;
    let sh_size = read_u32(&data, off + 20) as usize;
    let sh_link = read_u32(&data, off + 24) as usize;
    let sh_entsize = read_u32(&data, off + 36) as usize;
    if sh_entsize == 0 {
        return Ok(Vec::new());
    }

    // Get the string table
    let str_off = e_shoff + sh_link * e_shentsize;
    if str_off + 40 > data.len() {
        return Ok(Vec::new());
    }
    let str_sh_offset = read_u32(&data, str_off + 16) as usize;
    let str_sh_size = read_u32(&data, str_off + 20) as usize;
    if str_sh_offset + str_sh_size > data.len() {
        return Ok(Vec::new());
    }
    let strtab = &data[str_sh_offset..str_sh_offset + str_sh_size];

    let count = sh_size / sh_entsize;
    let mut syms = Vec::new();
    for j in 0..count {
        let sym_off = sh_offset + j * sh_entsize;
        if sym_off + 16 > data.len() {
            break;
        }
        let st_name = read_u32(&data, sym_off) as usize;
        let st_size = read_u32(&data, sym_off + 8);
        let st_info = data[sym_off + 12];
        let st_shndx = read_u16(&data, sym_off + 14);

        if st_shndx == SHN_UNDEF {
            continue;
        }
        let binding = st_info >> 4;
        if binding != STB_GLOBAL && binding != STB_WEAK {
            continue;
        }

        // Look up version for this symbol
        let (version, is_default_ver) = lookup_version(j, versym_shdr, &ver_names, &data);

        if st_name < strtab.len() {
            let end = strtab[st_name..]
                .iter()
                .position(|&b| b == 0)
                .map(|p| st_name + p)
                .unwrap_or(strtab.len());
            let name = String::from_utf8_lossy(&strtab[st_name..end]).into_owned();
            if !name.is_empty() {
                syms.push(DynSymInfo {
                    name,
                    sym_type: st_info & 0xf,
                    size: st_size,
                    binding,
                    version,
                    is_default_ver,
                });
            }
        }
    }

    Ok(syms)
}

/// Parse GNU version definitions to build index -> version string mapping.
fn parse_verdef(
    data: &[u8],
    verdef_shdr: Option<(usize, usize, usize)>,
    e_shoff: usize,
    e_shentsize: usize,
) -> std::collections::HashMap<u16, String> {
    let mut ver_names = std::collections::HashMap::new();

    let (vd_off, vd_size, vd_link) = match verdef_shdr {
        Some(v) => v,
        None => return ver_names,
    };

    // Get the string table for verdef
    let vd_str_hdr = e_shoff + vd_link * e_shentsize;
    let vd_strtab = if vd_str_hdr + 40 <= data.len() {
        let s_off = read_u32(data, vd_str_hdr + 16) as usize;
        let s_sz = read_u32(data, vd_str_hdr + 20) as usize;
        if s_off + s_sz <= data.len() {
            &data[s_off..s_off + s_sz]
        } else {
            return ver_names;
        }
    } else {
        return ver_names;
    };

    let mut pos = vd_off;
    let end = vd_off + vd_size;
    while pos < end && pos + 20 <= data.len() {
        let vd_ndx = read_u16(data, pos + 4);
        let vd_cnt = read_u16(data, pos + 6);
        let vd_aux = read_u32(data, pos + 12) as usize;
        let vd_next = read_u32(data, pos + 16) as usize;

        if vd_cnt > 0 {
            let aux_pos = pos + vd_aux;
            if aux_pos + 8 <= data.len() {
                let vda_name = read_u32(data, aux_pos) as usize;
                if vda_name < vd_strtab.len() {
                    let name = read_cstr(vd_strtab, vda_name);
                    ver_names.insert(vd_ndx, name.to_string());
                }
            }
        }

        if vd_next == 0 {
            break;
        }
        pos += vd_next;
    }

    ver_names
}

/// Look up the GNU version info for a symbol at dynsym index `j`.
fn lookup_version(
    j: usize,
    versym_shdr: Option<(usize, usize)>,
    ver_names: &std::collections::HashMap<u16, String>,
    data: &[u8],
) -> (Option<String>, bool) {
    if let Some((vs_off, _vs_size)) = versym_shdr {
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
    }
}

/// Resolve symbols from a GNU linker script (GROUP/INPUT directives).
fn resolve_linker_script_syms(
    path: &str,
    entries: &[LinkerScriptEntry],
    lib_search_paths: &[&str],
) -> Result<Vec<DynSymInfo>, String> {
    let script_dir = std::path::Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().to_string());
    let mut all_syms = Vec::new();
    for entry in entries {
        let resolved = match entry {
            LinkerScriptEntry::Path(lib_path) => {
                resolve_script_path(lib_path, script_dir.as_deref(), lib_search_paths)
            }
            LinkerScriptEntry::Lib(lib_name) => lib_search_paths
                .iter()
                .map(|dir| format!("{}/lib{}.so", dir, lib_name))
                .find(|p| std::path::Path::new(p).exists()),
        };
        if let Some(resolved_path) = resolved {
            if let Ok(syms) = read_dynsyms_with_search(&resolved_path, lib_search_paths) {
                all_syms.extend(syms);
            }
        }
    }
    if !all_syms.is_empty() {
        Ok(all_syms)
    } else {
        Err(format!(
            "{}: linker script but no resolvable libraries found",
            path
        ))
    }
}

/// Extract the SONAME from an ELF32 shared library's .dynamic section.
///
/// Returns the SONAME string if present, or None if not found.
pub(super) fn parse_soname_elf32(path: &str) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 52 || data[0..4] != ELF_MAGIC || data[4] != ELFCLASS32 {
        return None;
    }
    let e_type = read_u16(&data, 16);
    if e_type != ET_DYN {
        return None;
    }

    let e_shoff = read_u32(&data, 32) as usize;
    let e_shentsize = read_u16(&data, 46) as usize;
    let e_shnum = read_u16(&data, 48) as usize;

    if e_shoff == 0 || e_shnum == 0 {
        return None;
    }

    // Find .dynamic section
    for i in 0..e_shnum {
        let off = e_shoff + i * e_shentsize;
        if off + 40 > data.len() {
            break;
        }
        let sh_type = read_u32(&data, off + 4);
        if sh_type == 6 {
            // SHT_DYNAMIC
            let dyn_off = read_u32(&data, off + 16) as usize;
            let dyn_size = read_u32(&data, off + 20) as usize;
            let link = read_u32(&data, off + 24) as usize;

            // Get linked string table
            let str_sec_off = e_shoff + link * e_shentsize;
            if str_sec_off + 40 > data.len() {
                return None;
            }
            let str_off = read_u32(&data, str_sec_off + 16) as usize;
            let str_size = read_u32(&data, str_sec_off + 20) as usize;
            if str_off + str_size > data.len() {
                return None;
            }
            let strtab = &data[str_off..str_off + str_size];

            // Scan .dynamic entries for DT_SONAME (tag = 14)
            let mut pos = dyn_off;
            while pos + 8 <= dyn_off + dyn_size && pos + 8 <= data.len() {
                let tag = read_i32(&data, pos);
                let val = read_u32(&data, pos + 4) as usize;
                if tag == 0 {
                    break;
                } // DT_NULL
                if tag == 14 {
                    // DT_SONAME
                    return Some(read_cstr(strtab, val));
                }
                pos += 8;
            }
            return None;
        }
    }
    None
}

/// Resolve a path from a linker script entry.
fn resolve_script_path(
    lib_path: &str,
    script_dir: Option<&str>,
    lib_search_paths: &[&str],
) -> Option<String> {
    if std::path::Path::new(lib_path).exists() {
        return Some(lib_path.to_string());
    }
    if let Some(dir) = script_dir {
        let p = format!("{}/{}", dir, lib_path);
        if std::path::Path::new(&p).exists() {
            return Some(p);
        }
    }
    for search_dir in lib_search_paths {
        let p = format!("{}/{}", search_dir, lib_path);
        if std::path::Path::new(&p).exists() {
            return Some(p);
        }
    }
    None
}
