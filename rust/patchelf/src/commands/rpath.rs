use crate::elf::{available_space_at, dynstr_info, read_str_at, write_str_inplace};
use crate::grow::{add_dynamic_entry, append_to_dynstr};
use goblin::elf::Elf;
use goblin::elf::dynamic::{DT_RPATH, DT_RUNPATH};
use std::path::Path;

pub fn set_rpath(elf: &Elf, data: &mut Vec<u8>, new_rpath: &str, force_rpath: bool) {
    {
        let (strtab_offset, strtab_size) = dynstr_info(elf, data);
        let strtab_end = strtab_offset + strtab_size;
        if let Some(dynamic) = &elf.dynamic {
            let preferred = if force_rpath { DT_RPATH } else { DT_RUNPATH };
            for tag in &[DT_RUNPATH, DT_RPATH] {
                for dyn_entry in &dynamic.dyns {
                    if dyn_entry.d_tag == *tag {
                        let str_offset = strtab_offset + dyn_entry.d_val as usize;
                        let space = available_space_at(data, str_offset, strtab_end);
                        if new_rpath.len() + 1 <= space {
                            write_str_inplace(data, str_offset, space, new_rpath);
                        } else {
                            let mut payload = new_rpath.as_bytes().to_vec();
                            payload.push(0);
                            let (_v, new_off) = append_to_dynstr(elf, data, &payload);
                            patch_dyn_entry_val(elf, data, *tag, new_off as u64);
                        }
                        if *tag != preferred {
                            patch_dyn_entry_tag(elf, data, *tag, preferred);
                        }
                        return;
                    }
                }
            }
        }
    }
    let mut payload = new_rpath.as_bytes().to_vec();
    payload.push(0);
    let (_v, new_off) = append_to_dynstr(elf, data, &payload);
    let new_tag = if force_rpath { DT_RPATH } else { DT_RUNPATH };
    if !add_dynamic_entry(elf, data, new_tag, new_off as u64) {
        eprintln!("patchelf: set-rpath: no DT_NULL slot in .dynamic to add DT_RUNPATH");
        std::process::exit(1);
    }
}

fn patch_dyn_entry_tag(elf: &Elf, data: &mut [u8], old_tag: u64, new_tag: u64) {
    let dyn_phdr = elf
        .program_headers
        .iter()
        .find(|ph| ph.p_type == goblin::elf::program_header::PT_DYNAMIC)
        .expect("PT_DYNAMIC");
    let dyn_off = dyn_phdr.p_offset as usize;
    let entry_size = if elf.is_64 { 16usize } else { 8usize };
    let dynamic = elf.dynamic.as_ref().expect(".dynamic");
    for (idx, e) in dynamic.dyns.iter().enumerate() {
        if e.d_tag == old_tag {
            let entry_off = dyn_off + idx * entry_size;
            if elf.is_64 {
                if elf.little_endian {
                    data[entry_off..entry_off + 8].copy_from_slice(&new_tag.to_le_bytes());
                } else {
                    data[entry_off..entry_off + 8].copy_from_slice(&new_tag.to_be_bytes());
                }
            } else if elf.little_endian {
                data[entry_off..entry_off + 4].copy_from_slice(&(new_tag as u32).to_le_bytes());
            } else {
                data[entry_off..entry_off + 4].copy_from_slice(&(new_tag as u32).to_be_bytes());
            }
            return;
        }
    }
}

fn patch_dyn_entry_val(elf: &Elf, data: &mut [u8], tag: u64, new_val: u64) {
    let dyn_phdr = elf
        .program_headers
        .iter()
        .find(|ph| ph.p_type == goblin::elf::program_header::PT_DYNAMIC)
        .expect("PT_DYNAMIC");
    let dyn_off = dyn_phdr.p_offset as usize;
    let entry_size = if elf.is_64 { 16usize } else { 8usize };
    let dynamic = elf.dynamic.as_ref().expect(".dynamic");
    for (idx, e) in dynamic.dyns.iter().enumerate() {
        if e.d_tag == tag {
            let entry_off = dyn_off + idx * entry_size;
            if elf.is_64 {
                if elf.little_endian {
                    data[entry_off + 8..entry_off + 16].copy_from_slice(&new_val.to_le_bytes());
                } else {
                    data[entry_off + 8..entry_off + 16].copy_from_slice(&new_val.to_be_bytes());
                }
            } else if elf.little_endian {
                data[entry_off + 4..entry_off + 8].copy_from_slice(&(new_val as u32).to_le_bytes());
            } else {
                data[entry_off + 4..entry_off + 8].copy_from_slice(&(new_val as u32).to_be_bytes());
            }
            return;
        }
    }
}

pub fn shrink_rpath(elf: &Elf, data: &mut [u8], _file: &Path, allowed_prefixes: Option<&str>) {
    let (strtab_offset, strtab_size) = dynstr_info(elf, data);
    let strtab_end = strtab_offset + strtab_size;

    // Mirror upstream behaviour: each needed library is satisfied by
    // the *first* rpath entry that provides it. Subsequent entries that
    // would only provide an already-satisfied library are dropped.
    let needed_libs: Vec<&str> = elf.libraries.iter().map(|s| s.as_ref()).collect();

    if let Some(dynamic) = &elf.dynamic {
        for tag in &[DT_RUNPATH, DT_RPATH] {
            for dyn_entry in &dynamic.dyns {
                if dyn_entry.d_tag == *tag {
                    let str_offset = strtab_offset + dyn_entry.d_val as usize;
                    let old_rpath = read_str_at(data, str_offset).to_string();
                    let space = available_space_at(data, str_offset, strtab_end);

                    let prefixes: Vec<&str> = allowed_prefixes
                        .map(|p| p.split(':').filter(|s| !s.is_empty()).collect())
                        .unwrap_or_default();

                    let mut found = vec![false; needed_libs.len()];
                    let mut kept: Vec<&str> = Vec::new();
                    for dir in old_rpath.split(':') {
                        if dir.is_empty() {
                            continue;
                        }
                        // Non-absolute paths (e.g. $ORIGIN) always pass.
                        if !dir.starts_with('/') {
                            kept.push(dir);
                            continue;
                        }
                        if !prefixes.is_empty()
                            && !prefixes.iter().any(|pref| dir.starts_with(pref))
                        {
                            continue;
                        }
                        let dir_path = Path::new(dir);
                        let mut keep = false;
                        for (j, lib) in needed_libs.iter().enumerate() {
                            if !found[j] && dir_path.join(lib).exists() {
                                found[j] = true;
                                keep = true;
                            }
                        }
                        if keep {
                            kept.push(dir);
                        }
                    }

                    let new_rpath = kept.join(":");
                    write_str_inplace(data, str_offset, space, &new_rpath);
                    return;
                }
            }
        }
    }
}

pub fn remove_rpath(elf: &Elf, data: &mut [u8]) {
    // Re-tag any DT_RPATH/DT_RUNPATH entry to DT_DEBUG so the loader and
    // objdump no longer see it as an rpath. We can't simply DT_NULL it
    // because DT_NULL terminates the dynamic array.
    const DT_DEBUG: u64 = 21;
    let dynamic = match &elf.dynamic {
        Some(d) => d,
        None => return,
    };
    let dyn_phdr = match elf
        .program_headers
        .iter()
        .find(|ph| ph.p_type == goblin::elf::program_header::PT_DYNAMIC)
    {
        Some(p) => p,
        None => return,
    };
    let dyn_off = dyn_phdr.p_offset as usize;
    let entry_size = if elf.is_64 { 16usize } else { 8usize };
    for (idx, e) in dynamic.dyns.iter().enumerate() {
        if e.d_tag == DT_RPATH || e.d_tag == DT_RUNPATH {
            let entry_off = dyn_off + idx * entry_size;
            if elf.is_64 {
                if elf.little_endian {
                    data[entry_off..entry_off + 8].copy_from_slice(&DT_DEBUG.to_le_bytes());
                    data[entry_off + 8..entry_off + 16].copy_from_slice(&0u64.to_le_bytes());
                } else {
                    data[entry_off..entry_off + 8].copy_from_slice(&DT_DEBUG.to_be_bytes());
                    data[entry_off + 8..entry_off + 16].copy_from_slice(&0u64.to_be_bytes());
                }
            } else if elf.little_endian {
                data[entry_off..entry_off + 4]
                    .copy_from_slice(&(DT_DEBUG as u32).to_le_bytes());
                data[entry_off + 4..entry_off + 8].copy_from_slice(&0u32.to_le_bytes());
            } else {
                data[entry_off..entry_off + 4]
                    .copy_from_slice(&(DT_DEBUG as u32).to_be_bytes());
                data[entry_off + 4..entry_off + 8].copy_from_slice(&0u32.to_be_bytes());
            }
        }
    }
}

pub fn add_rpath(elf: &Elf, data: &mut Vec<u8>, extra: &str, force_rpath: bool) {
    {
        let (strtab_offset, strtab_size) = dynstr_info(elf, data);
        let strtab_end = strtab_offset + strtab_size;
        if let Some(dynamic) = &elf.dynamic {
            for tag in &[DT_RUNPATH, DT_RPATH] {
                for dyn_entry in &dynamic.dyns {
                    if dyn_entry.d_tag == *tag {
                        let str_offset = strtab_offset + dyn_entry.d_val as usize;
                        let space = available_space_at(data, str_offset, strtab_end);
                        let old = read_str_at(data, str_offset).to_string();
                        let combined = if old.is_empty() {
                            extra.to_string()
                        } else {
                            format!("{old}:{extra}")
                        };
                        if combined.len() + 1 <= space {
                            write_str_inplace(data, str_offset, space, &combined);
                        } else {
                            let mut payload = combined.as_bytes().to_vec();
                            payload.push(0);
                            let (_v, new_off) = append_to_dynstr(elf, data, &payload);
                            patch_dyn_entry_val(elf, data, *tag, new_off as u64);
                        }
                        let preferred = if force_rpath { DT_RPATH } else { DT_RUNPATH };
                        if *tag != preferred {
                            patch_dyn_entry_tag(elf, data, *tag, preferred);
                        }
                        return;
                    }
                }
            }
        }
    }
    set_rpath(elf, data, extra, force_rpath);
}
