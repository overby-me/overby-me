use crate::elf::{available_space_at, dynstr_info, read_str_at, write_str_inplace};
use crate::grow::{add_dynamic_entry, append_to_dynstr};
use goblin::elf::Elf;
use goblin::elf::dynamic::DT_NEEDED;
use std::process;

pub fn add_needed(elf: &Elf, data: &mut Vec<u8>, lib: &str) {
    let mut payload = lib.as_bytes().to_vec();
    payload.push(0);
    let (_v, new_off) = append_to_dynstr(elf, data, &payload);
    if !add_dynamic_entry(elf, data, DT_NEEDED, new_off as u64) {
        eprintln!("patchelf: --add-needed: no DT_NULL slot in .dynamic");
        process::exit(1);
    }
}

pub fn remove_needed(elf: &Elf, data: &mut [u8], lib: &str) {
    let is_64 = elf.is_64;
    let is_le = elf.little_endian;

    let (strtab_offset, _strtab_size) = dynstr_info(elf, data);

    if let Some(dynamic) = &elf.dynamic {
        let dyn_phdr = elf
            .program_headers
            .iter()
            .find(|ph| ph.p_type == goblin::elf::program_header::PT_DYNAMIC)
            .unwrap_or_else(|| {
                eprintln!("patchelf: no PT_DYNAMIC segment found");
                process::exit(1);
            });

        let dyn_offset = dyn_phdr.p_offset as usize;
        let entry_size = if is_64 { 16usize } else { 8usize };

        for (idx, dyn_entry) in dynamic.dyns.iter().enumerate() {
            if dyn_entry.d_tag == DT_NEEDED {
                let str_off = strtab_offset + dyn_entry.d_val as usize;
                let name = read_str_at(data, str_off);
                if name == lib {
                    let entry_offset = dyn_offset + idx * entry_size;
                    if is_64 {
                        if is_le {
                            data[entry_offset..entry_offset + 8]
                                .copy_from_slice(&0u64.to_le_bytes());
                            data[entry_offset + 8..entry_offset + 16]
                                .copy_from_slice(&0u64.to_le_bytes());
                        } else {
                            data[entry_offset..entry_offset + 8]
                                .copy_from_slice(&0u64.to_be_bytes());
                            data[entry_offset + 8..entry_offset + 16]
                                .copy_from_slice(&0u64.to_be_bytes());
                        }
                    } else if is_le {
                        data[entry_offset..entry_offset + 4]
                            .copy_from_slice(&0u32.to_le_bytes());
                        data[entry_offset + 4..entry_offset + 8]
                            .copy_from_slice(&0u32.to_le_bytes());
                    } else {
                        data[entry_offset..entry_offset + 4]
                            .copy_from_slice(&0u32.to_be_bytes());
                        data[entry_offset + 4..entry_offset + 8]
                            .copy_from_slice(&0u32.to_be_bytes());
                    }
                    return;
                }
            }
        }
    }

    eprintln!("patchelf: DT_NEEDED entry {lib} not found");
    process::exit(1);
}

pub fn replace_needed(elf: &Elf, data: &mut Vec<u8>, old: &str, new: &str) {
    let target_idx;
    {
        let (strtab_offset, strtab_size) = dynstr_info(elf, data);
        let strtab_end = strtab_offset + strtab_size;
        let dynamic = match &elf.dynamic {
            Some(d) => d,
            None => {
                eprintln!("patchelf: replace-needed: no .dynamic");
                process::exit(1);
            }
        };
        let mut found = None;
        for (idx, dyn_entry) in dynamic.dyns.iter().enumerate() {
            if dyn_entry.d_tag == DT_NEEDED {
                let str_offset = strtab_offset + dyn_entry.d_val as usize;
                let name = read_str_at(data, str_offset).to_string();
                if name == old {
                    let space = available_space_at(data, str_offset, strtab_end);
                    if new.len() + 1 <= space {
                        write_str_inplace(data, str_offset, space, new);
                        return;
                    }
                    found = Some(idx);
                    break;
                }
            }
        }
        target_idx = match found {
            Some(i) => i,
            None => {
                eprintln!("patchelf: DT_NEEDED entry {old} not found");
                process::exit(1);
            }
        };
    }
    // Need to grow dynstr.
    let mut payload = new.as_bytes().to_vec();
    payload.push(0);
    let (_v, new_off) = append_to_dynstr(elf, data, &payload);
    // Patch the d_val of the NEEDED entry at target_idx.
    let dyn_phdr = elf
        .program_headers
        .iter()
        .find(|ph| ph.p_type == goblin::elf::program_header::PT_DYNAMIC)
        .expect("PT_DYNAMIC");
    let dyn_off = dyn_phdr.p_offset as usize;
    let entry_size = if elf.is_64 { 16usize } else { 8usize };
    let entry_off = dyn_off + target_idx * entry_size;
    let new_val = new_off as u64;
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
}
