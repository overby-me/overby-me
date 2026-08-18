use crate::elf::{available_space_at, dynstr_info, write_str_inplace};
use crate::grow::{add_dynamic_entry, append_to_dynstr};
use goblin::elf::Elf;
use goblin::elf::dynamic::DT_SONAME;

pub fn set_soname(elf: &Elf, data: &mut Vec<u8>, new_soname: &str) {
    {
        let (strtab_offset, strtab_size) = dynstr_info(elf, data);
        let strtab_end = strtab_offset + strtab_size;
        if let Some(dynamic) = &elf.dynamic {
            for dyn_entry in &dynamic.dyns {
                if dyn_entry.d_tag == DT_SONAME {
                    let str_offset = strtab_offset + dyn_entry.d_val as usize;
                    let space = available_space_at(data, str_offset, strtab_end);
                    if new_soname.len() + 1 <= space {
                        write_str_inplace(data, str_offset, space, new_soname);
                    } else {
                        let mut payload = new_soname.as_bytes().to_vec();
                        payload.push(0);
                        let (_v, new_off) = append_to_dynstr(elf, data, &payload);
                        patch_soname_val(elf, data, new_off as u64);
                    }
                    return;
                }
            }
        }
    }
    // No DT_SONAME yet — grow dynstr and insert one.
    let mut payload = new_soname.as_bytes().to_vec();
    payload.push(0);
    let (_v, new_off) = append_to_dynstr(elf, data, &payload);
    if !add_dynamic_entry(elf, data, DT_SONAME, new_off as u64) {
        eprintln!("patchelf: set-soname: no DT_NULL slot in .dynamic to add DT_SONAME");
        std::process::exit(1);
    }
}

fn patch_soname_val(elf: &Elf, data: &mut [u8], new_val: u64) {
    let dyn_phdr = elf
        .program_headers
        .iter()
        .find(|ph| ph.p_type == goblin::elf::program_header::PT_DYNAMIC)
        .expect("PT_DYNAMIC");
    let dyn_off = dyn_phdr.p_offset as usize;
    let entry_size = if elf.is_64 { 16usize } else { 8usize };
    let dynamic = elf.dynamic.as_ref().expect(".dynamic");
    for (idx, e) in dynamic.dyns.iter().enumerate() {
        if e.d_tag == DT_SONAME {
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
