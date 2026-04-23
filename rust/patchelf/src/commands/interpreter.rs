use crate::grow::ElfBits;
use goblin::elf::Elf;
use goblin::elf::program_header::PT_INTERP;
use std::process;

const PT_LOAD: u32 = 1;

pub fn set_interpreter(elf: &Elf, data: &mut Vec<u8>, new_interp: &str) {
    let bits = ElfBits::from(elf);
    let new_bytes = new_interp.as_bytes();

    let interp_idx = elf
        .program_headers
        .iter()
        .position(|ph| ph.p_type == PT_INTERP)
        .unwrap_or_else(|| {
            eprintln!("patchelf: no PT_INTERP segment found");
            process::exit(1);
        });
    let interp_phdr_off =
        elf.header.e_phoff as usize + interp_idx * elf.header.e_phentsize as usize;
    let interp_phdr = &elf.program_headers[interp_idx];
    let offset = interp_phdr.p_offset as usize;
    let seg_size = interp_phdr.p_filesz as usize;

    if new_bytes.len() + 1 <= seg_size {
        // Fits in the existing PT_INTERP segment.
        data[offset..offset + new_bytes.len()].copy_from_slice(new_bytes);
        for b in &mut data[offset + new_bytes.len()..offset + seg_size] {
            *b = 0;
        }
        return;
    }

    // Strategy 1: park in LOAD #1 slack so the new interp is still
    // covered by a LOAD segment (some loaders may want this even though
    // the kernel does not strictly require it).
    let interp_vaddr = interp_phdr.p_vaddr;
    if let Some((load_phdr_off, load_off, load_filesz, load_vaddr, next_load_off)) =
        find_load_slack(elf, interp_vaddr)
    {
        let load_end = load_off + load_filesz;
        let new_off = (load_end + 7) & !7u64;
        let new_size = (new_bytes.len() + 1) as u64;
        if new_off + new_size <= next_load_off {
            data[new_off as usize..new_off as usize + new_bytes.len()]
                .copy_from_slice(new_bytes);
            data[new_off as usize + new_bytes.len()] = 0;
            let new_vaddr = load_vaddr + (new_off - load_off);
            let pf = bits.phdr_fields(interp_phdr_off);
            bits.write_word(data, pf.p_offset, new_off);
            bits.write_word(data, pf.p_vaddr, new_vaddr);
            bits.write_word(data, pf.p_paddr, new_vaddr);
            bits.write_word(data, pf.p_filesz, new_size);
            bits.write_word(data, pf.p_memsz, new_size);
            let lp = bits.phdr_fields(load_phdr_off);
            let new_load_filesz = (new_off + new_size) - load_off;
            let cur_filesz = bits.read_word(data, lp.p_filesz);
            if new_load_filesz > cur_filesz {
                bits.write_word(data, lp.p_filesz, new_load_filesz);
            }
            let cur_memsz = bits.read_word(data, lp.p_memsz);
            if new_load_filesz > cur_memsz {
                bits.write_word(data, lp.p_memsz, new_load_filesz);
            }
            return;
        }
    }

    // Strategy 2: append at file EOF and repoint PT_INTERP. The kernel
    // reads PT_INTERP straight from the file via p_offset, no LOAD
    // coverage required.
    let pad = ((data.len() as u64 + 7) & !7u64) - data.len() as u64;
    for _ in 0..pad { data.push(0); }
    let new_off = data.len() as u64;
    data.extend_from_slice(new_bytes);
    data.push(0);
    let new_size = (new_bytes.len() + 1) as u64;
    let pf = bits.phdr_fields(interp_phdr_off);
    bits.write_word(data, pf.p_offset, new_off);
    bits.write_word(data, pf.p_vaddr, 0);
    bits.write_word(data, pf.p_paddr, 0);
    bits.write_word(data, pf.p_filesz, new_size);
    bits.write_word(data, pf.p_memsz, new_size);
}

fn find_load_slack(elf: &Elf, target_vaddr: u64) -> Option<(usize, u64, u64, u64, u64)> {
    let mut load_idx = None;
    let mut load_off = 0u64;
    let mut load_filesz = 0u64;
    let mut load_vaddr = 0u64;
    for (i, ph) in elf.program_headers.iter().enumerate() {
        if ph.p_type == PT_LOAD
            && target_vaddr >= ph.p_vaddr
            && target_vaddr < ph.p_vaddr + ph.p_memsz
        {
            load_idx = Some(i);
            load_off = ph.p_offset;
            load_filesz = ph.p_filesz;
            load_vaddr = ph.p_vaddr;
            break;
        }
    }
    let load_idx = load_idx?;
    let load_phdr_off = elf.header.e_phoff as usize
        + load_idx * elf.header.e_phentsize as usize;
    let mut next_load_off = u64::MAX;
    for ph in &elf.program_headers {
        if ph.p_type == PT_LOAD && ph.p_offset > load_off && ph.p_offset < next_load_off {
            next_load_off = ph.p_offset;
        }
    }
    if next_load_off == u64::MAX { return None; }
    if (load_off + load_filesz + 8) >= next_load_off { return None; }
    Some((load_phdr_off, load_off, load_filesz, load_vaddr, next_load_off))
}
