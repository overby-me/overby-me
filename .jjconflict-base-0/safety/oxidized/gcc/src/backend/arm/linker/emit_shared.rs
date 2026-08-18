//! Shared library (.so) emission for the AArch64 linker.
//!
//! Emits an ELF64 shared library (ET_DYN) with PIC relocations, PLT stubs
//! for external function calls, and a `.dynamic` section for the dynamic linker.

use std::collections::HashMap;

use super::elf::*;
use super::reloc;
use super::types::{GlobalSymbol, PAGE_SIZE};
use crate::backend::linker_common;
use linker_common::{DynStrTab, OutputSection};

/// Emit a shared library (.so) ELF file for AArch64.
pub(super) fn emit_shared_library(
    objects: &[ElfObject],
    globals: &mut HashMap<String, GlobalSymbol>,
    output_sections: &mut [OutputSection],
    section_map: &HashMap<(usize, usize), (usize, u64)>,
    needed_sonames: &[String],
    output_path: &str,
    soname: Option<String>,
) -> Result<(), String> {
    let base_addr: u64 = 0;

    let mut dynstr = DynStrTab::new();
    for lib in needed_sonames {
        dynstr.add(lib);
    }
    if let Some(ref sn) = soname {
        dynstr.add(sn);
    }

    // Export all defined global symbols
    let mut dyn_sym_names: Vec<String> = Vec::new();
    let mut exported: Vec<String> = globals
        .iter()
        .filter(|(_, g)| {
            g.defined_in.is_some()
                && !g.is_dynamic
                && (g.info >> 4) != 0
                && g.section_idx != SHN_UNDEF
        })
        .map(|(n, _)| n.clone())
        .collect();
    exported.sort();
    for name in exported {
        if !dyn_sym_names.contains(&name) {
            dyn_sym_names.push(name);
        }
    }
    for (name, gsym) in globals.iter() {
        if gsym.is_dynamic && !dyn_sym_names.contains(name) {
            dyn_sym_names.push(name.clone());
        }
    }

    // Collect PLT symbols: external functions referenced by CALL26/JUMP26
    let mut so_plt_names: Vec<String> = Vec::new();
    for obj in objects.iter() {
        for sec_relas in &obj.relocations {
            for rela in sec_relas {
                let si = rela.sym_idx as usize;
                if si >= obj.symbols.len() {
                    continue;
                }
                let sym = &obj.symbols[si];
                if sym.name.is_empty() {
                    continue;
                }
                match rela.rela_type {
                    R_AARCH64_CALL26 | R_AARCH64_JUMP26 => {
                        let needs_plt = if let Some(g) = globals.get(&sym.name) {
                            g.is_dynamic || (g.defined_in.is_none() && g.section_idx == SHN_UNDEF)
                        } else {
                            sym.is_undefined() && !sym.is_local()
                        };
                        if needs_plt && !so_plt_names.contains(&sym.name) {
                            so_plt_names.push(sym.name.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // Add PLT symbols to dyn_sym_names if not already present
    for name in &so_plt_names {
        if !dyn_sym_names.contains(name) {
            dyn_sym_names.push(name.clone());
        }
    }
    // Assign PLT indices
    for (i, name) in so_plt_names.iter().enumerate() {
        if let Some(g) = globals.get_mut(name) {
            g.plt_idx = Some(i);
        }
    }

    // Add undefined/dynamic symbols referenced by ADRP/ADD/LDST to dyn_sym_names
    // so they get GLOB_DAT relocations and the dynamic linker can resolve them.
    for obj in objects.iter() {
        for sec_relas in &obj.relocations {
            for rela in sec_relas {
                let si = rela.sym_idx as usize;
                if si >= obj.symbols.len() {
                    continue;
                }
                let sym = &obj.symbols[si];
                if sym.name.is_empty() {
                    continue;
                }
                match rela.rela_type {
                    R_AARCH64_ADR_PREL_PG_HI21
                    | R_AARCH64_ADD_ABS_LO12_NC
                    | R_AARCH64_LDST64_ABS_LO12_NC
                    | R_AARCH64_LDST32_ABS_LO12_NC
                    | R_AARCH64_LDST8_ABS_LO12_NC
                    | R_AARCH64_LDST16_ABS_LO12_NC => {
                        let sym_needs_got = if let Some(g) = globals.get(&sym.name) {
                            g.is_dynamic || g.defined_in.is_none()
                        } else {
                            sym.is_undefined() && !sym.is_local()
                        };
                        if sym_needs_got && !dyn_sym_names.contains(&sym.name) {
                            dyn_sym_names.push(sym.name.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Reorder dyn_sym_names: undefined/import symbols first, then defined/export symbols.
    // The .gnu.hash table only covers defined symbols (those after symoffset).
    // Undefined symbols must be placed before symoffset so the dynamic linker
    // doesn't incorrectly find them during symbol lookup in this library.
    let mut undef_names: Vec<String> = Vec::new();
    let mut def_names: Vec<String> = Vec::new();
    for name in &dyn_sym_names {
        let is_undef = if let Some(g) = globals.get(name) {
            g.is_dynamic || g.defined_in.is_none() || g.section_idx == SHN_UNDEF
        } else {
            true
        };
        if is_undef {
            undef_names.push(name.clone());
        } else {
            def_names.push(name.clone());
        }
    }
    dyn_sym_names = Vec::new();
    dyn_sym_names.extend(undef_names.iter().cloned());
    let so_undef_count = dyn_sym_names.len();
    dyn_sym_names.extend(def_names.iter().cloned());

    for name in &dyn_sym_names {
        dynstr.add(name);
    }

    let dynsym_count = 1 + dyn_sym_names.len();
    let dynsym_size = dynsym_count as u64 * 24;
    let dynstr_size = dynstr.as_bytes().len() as u64;

    // .gnu.hash - only covers defined symbols (after the undefined ones)
    let gnu_hash_symoffset: usize = 1 + so_undef_count;
    let num_hashed = dyn_sym_names.len() - so_undef_count;
    let gnu_hash_nbuckets = if num_hashed == 0 {
        1
    } else {
        num_hashed.next_power_of_two().max(1)
    } as u32;
    let gnu_hash_bloom_size: u32 = 1;
    let gnu_hash_bloom_shift: u32 = 6;

    let hashed_sym_hashes: Vec<u32> = dyn_sym_names[so_undef_count..]
        .iter()
        .map(|name| linker_common::gnu_hash(name.as_bytes()))
        .collect();

    let mut bloom_word: u64 = 0;
    for &h in &hashed_sym_hashes {
        bloom_word |= 1u64 << (h as u64 % 64);
        bloom_word |= 1u64 << ((h >> gnu_hash_bloom_shift) as u64 % 64);
    }

    if num_hashed > 0 {
        let mut hashed_with_hash: Vec<(String, u32)> = dyn_sym_names[so_undef_count..]
            .iter()
            .zip(hashed_sym_hashes.iter())
            .map(|(n, &h)| (n.clone(), h))
            .collect();
        hashed_with_hash.sort_by_key(|(_, h)| h % gnu_hash_nbuckets);
        for (i, (name, _)) in hashed_with_hash.iter().enumerate() {
            dyn_sym_names[so_undef_count + i] = name.clone();
        }
    }

    let hashed_sym_hashes: Vec<u32> = dyn_sym_names[so_undef_count..]
        .iter()
        .map(|name| linker_common::gnu_hash(name.as_bytes()))
        .collect();

    let mut gnu_hash_buckets = vec![0u32; gnu_hash_nbuckets as usize];
    let mut gnu_hash_chains = vec![0u32; num_hashed];
    for (i, &h) in hashed_sym_hashes.iter().enumerate() {
        let bucket = (h % gnu_hash_nbuckets) as usize;
        if gnu_hash_buckets[bucket] == 0 {
            gnu_hash_buckets[bucket] = (gnu_hash_symoffset + i) as u32;
        }
        gnu_hash_chains[i] = h & !1;
    }
    for bucket_idx in 0..gnu_hash_nbuckets as usize {
        if gnu_hash_buckets[bucket_idx] == 0 {
            continue;
        }
        let mut last_in_bucket = 0;
        for (i, &h) in hashed_sym_hashes.iter().enumerate() {
            if (h % gnu_hash_nbuckets) as usize == bucket_idx {
                last_in_bucket = i;
            }
        }
        gnu_hash_chains[last_in_bucket] |= 1;
    }

    let gnu_hash_size: u64 = 16
        + (gnu_hash_bloom_size as u64 * 8)
        + (gnu_hash_nbuckets as u64 * 4)
        + (num_hashed as u64 * 4);

    let has_init_array = output_sections
        .iter()
        .any(|s| s.name == ".init_array" && s.mem_size > 0);
    let has_fini_array = output_sections
        .iter()
        .any(|s| s.name == ".fini_array" && s.mem_size > 0);
    let so_plt_size: u64 = if so_plt_names.is_empty() {
        0
    } else {
        32 + 16 * so_plt_names.len() as u64
    };
    let so_got_plt_count: u64 = if so_plt_names.is_empty() {
        0
    } else {
        3 + so_plt_names.len() as u64
    };
    let so_got_plt_size: u64 = so_got_plt_count * 8;
    let so_rela_plt_size: u64 = so_plt_names.len() as u64 * 24;

    let mut dyn_count = needed_sonames.len() as u64 + 12; // base 10 + 2 for FLAGS/FLAGS_1
    if soname.is_some() {
        dyn_count += 1;
    }
    if has_init_array {
        dyn_count += 2;
    }
    if has_fini_array {
        dyn_count += 2;
    }
    if !so_plt_names.is_empty() {
        dyn_count += 4;
    } // PLTGOT, PLTRELSZ, PLTREL, JMPREL
    let dynamic_size = dyn_count * 16;

    let has_tls_sections = output_sections
        .iter()
        .any(|s| s.flags & SHF_TLS != 0 && s.flags & SHF_ALLOC != 0);

    // Promote read-only sections that have R_AARCH64_ABS64 relocations to writable.
    // These sections contain embedded pointers that become R_AARCH64_RELATIVE dynamic
    // relocations, so the dynamic linker must be able to write to them at load time.
    {
        // Build a set of output section indices that have ABS64 relocs targeting them
        let mut needs_write: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for obj_idx in 0..objects.len() {
            for sec_idx in 0..objects[obj_idx].sections.len() {
                let relas = &objects[obj_idx].relocations[sec_idx];
                let has_abs64 = relas.iter().any(|r| r.rela_type == R_AARCH64_ABS64);
                if has_abs64 {
                    if let Some(&(out_idx, _)) = section_map.get(&(obj_idx, sec_idx)) {
                        needs_write.insert(out_idx);
                    }
                }
            }
        }
        for idx in needs_write {
            if output_sections[idx].flags & SHF_WRITE == 0
                && output_sections[idx].flags & SHF_EXECINSTR == 0
            {
                output_sections[idx].flags |= SHF_WRITE;
            }
        }
    }

    // Check if there are any pure-rodata (non-writable, non-executable) sections remaining
    let has_rodata = output_sections.iter().any(|s| {
        s.flags & SHF_ALLOC != 0
            && s.flags & SHF_EXECINSTR == 0
            && s.flags & SHF_WRITE == 0
            && s.sh_type != SHT_NOBITS
    });
    // PHDR, LOAD R, LOAD RX, [LOAD R(rodata)], LOAD RW, DYNAMIC, GNU_STACK, [TLS]
    let mut phdr_count: u64 = 6; // base: PHDR + LOAD R + LOAD RX + LOAD RW + DYNAMIC + GNU_STACK
    if has_rodata {
        phdr_count += 1;
    }
    if has_tls_sections {
        phdr_count += 1;
    }
    let phdr_total_size = phdr_count * 56;

    // === Layout ===
    let mut offset = 64 + phdr_total_size;
    offset = (offset + 7) & !7;
    let gnu_hash_offset = offset;
    let gnu_hash_addr = base_addr + offset;
    offset += gnu_hash_size;
    offset = (offset + 7) & !7;
    let dynsym_offset = offset;
    let dynsym_addr = base_addr + offset;
    offset += dynsym_size;
    let dynstr_offset = offset;
    let dynstr_addr = base_addr + offset;
    offset += dynstr_size;

    // Text segment
    offset = (offset + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let text_page_offset = offset;
    let text_page_addr = base_addr + offset;
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_EXECINSTR != 0 && sec.flags & SHF_ALLOC != 0 {
            let a = sec.alignment.max(4);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = base_addr + offset;
            sec.file_offset = offset;
            offset += sec.mem_size;
        }
    }
    // PLT stubs (in text segment, after .text sections)
    let so_plt_offset: u64;
    let so_plt_addr: u64;
    if so_plt_size > 0 {
        offset = (offset + 15) & !15; // align to 16 bytes
        so_plt_offset = offset;
        so_plt_addr = base_addr + offset;
        offset += so_plt_size;
    } else {
        so_plt_offset = 0;
        so_plt_addr = 0;
    }
    let text_total_size = offset - text_page_offset;

    // Rodata
    offset = (offset + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let rodata_page_offset = offset;
    let rodata_page_addr = base_addr + offset;
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_ALLOC != 0
            && sec.flags & SHF_EXECINSTR == 0
            && sec.flags & SHF_WRITE == 0
            && sec.sh_type != SHT_NOBITS
        {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = base_addr + offset;
            sec.file_offset = offset;
            offset += sec.mem_size;
        }
    }
    let rodata_total_size = offset - rodata_page_offset;

    // RW segment
    offset = (offset + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let rw_page_offset = offset;
    let rw_page_addr = base_addr + offset;

    let mut init_array_addr_so = 0u64;
    let mut init_array_size_so = 0u64;
    let mut fini_array_addr_so = 0u64;
    let mut fini_array_size_so = 0u64;

    for sec in output_sections.iter_mut() {
        if sec.name == ".init_array" {
            let a = sec.alignment.max(8);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = base_addr + offset;
            sec.file_offset = offset;
            init_array_addr_so = sec.addr;
            init_array_size_so = sec.mem_size;
            offset += sec.mem_size;
            break;
        }
    }
    for sec in output_sections.iter_mut() {
        if sec.name == ".fini_array" {
            let a = sec.alignment.max(8);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = base_addr + offset;
            sec.file_offset = offset;
            fini_array_addr_so = sec.addr;
            fini_array_size_so = sec.mem_size;
            offset += sec.mem_size;
            break;
        }
    }

    // Reserve space for .rela.dyn (R_AARCH64_RELATIVE + R_AARCH64_GLOB_DAT entries)
    offset = (offset + 7) & !7;
    let rela_dyn_offset = offset;
    let rela_dyn_addr = base_addr + offset;
    let mut max_rela_count: usize = 0;
    // Count ABS64 relocations (become RELATIVE)
    for obj in objects.iter() {
        for sec_relas in &obj.relocations {
            for rela in sec_relas {
                if rela.rela_type == R_AARCH64_ABS64 {
                    max_rela_count += 1;
                }
            }
        }
    }
    for sec in output_sections.iter() {
        if sec.name == ".init_array" || sec.name == ".fini_array" {
            max_rela_count += (sec.mem_size / 8) as usize;
        }
    }
    // Pre-count GOT entries that will need dynamic relocations (RELATIVE or GLOB_DAT).
    // This must be done before layout to correctly reserve .rela.dyn space.
    {
        let mut got_pre_count: usize = 0;
        let mut got_pre_names: Vec<String> = Vec::new();
        for obj in objects.iter() {
            for sec_relas in &obj.relocations {
                for rela in sec_relas {
                    let si = rela.sym_idx as usize;
                    if si >= obj.symbols.len() {
                        continue;
                    }
                    let sym_name = &obj.symbols[si].name;
                    if sym_name.is_empty() {
                        continue;
                    }
                    let needs_got = match rela.rela_type {
                        R_AARCH64_ADR_GOT_PAGE | R_AARCH64_LD64_GOT_LO12_NC => true,
                        R_AARCH64_ADR_PREL_PG_HI21
                        | R_AARCH64_ADD_ABS_LO12_NC
                        | R_AARCH64_LDST64_ABS_LO12_NC
                        | R_AARCH64_LDST32_ABS_LO12_NC
                        | R_AARCH64_LDST8_ABS_LO12_NC
                        | R_AARCH64_LDST16_ABS_LO12_NC => {
                            if let Some(g) = globals.get(sym_name.as_str()) {
                                g.is_dynamic || g.defined_in.is_none()
                            } else {
                                obj.symbols[si].is_undefined() && !obj.symbols[si].is_local()
                            }
                        }
                        _ => false,
                    };
                    if needs_got && !got_pre_names.contains(sym_name) {
                        got_pre_names.push(sym_name.clone());
                        got_pre_count += 1;
                    }
                }
            }
        }
        max_rela_count += got_pre_count;
    }
    let rela_dyn_max_size = max_rela_count as u64 * 24;
    offset += rela_dyn_max_size;

    offset = (offset + 7) & !7;
    let dynamic_offset = offset;
    let dynamic_addr_so = base_addr + offset;
    offset += dynamic_size;

    // GOT.PLT for PLT symbols
    let so_got_plt_offset: u64;
    let so_got_plt_addr: u64;
    if so_got_plt_size > 0 {
        offset = (offset + 7) & !7;
        so_got_plt_offset = offset;
        so_got_plt_addr = base_addr + offset;
        offset += so_got_plt_size;
    } else {
        so_got_plt_offset = 0;
        so_got_plt_addr = 0;
    }

    // RELA.PLT for JUMP_SLOT relocations
    let so_rela_plt_offset: u64;
    let so_rela_plt_addr: u64;
    if so_rela_plt_size > 0 {
        offset = (offset + 7) & !7;
        so_rela_plt_offset = offset;
        so_rela_plt_addr = base_addr + offset;
        offset += so_rela_plt_size;
    } else {
        so_rela_plt_offset = 0;
        so_rela_plt_addr = 0;
    }

    // GOT for locally-resolved symbols AND undefined/dynamic symbols referenced
    // by ADRP/ADD pairs that need GOT indirection in shared libraries.
    let got_offset = offset;
    let got_addr = base_addr + offset;
    let mut got_needed: Vec<String> = Vec::new();
    for obj in objects.iter() {
        for sec_relas in &obj.relocations {
            for rela in sec_relas {
                let si = rela.sym_idx as usize;
                if si >= obj.symbols.len() {
                    continue;
                }
                let sym = &obj.symbols[si];
                if sym.name.is_empty() {
                    continue;
                }
                // Skip local symbols - they don't need GOT entries in dynsym
                // (e.g. static _Thread_local variables)
                if sym.is_local() {
                    continue;
                }
                match rela.rela_type {
                    R_AARCH64_ADR_GOT_PAGE | R_AARCH64_LD64_GOT_LO12_NC => {
                        if !got_needed.contains(&sym.name) {
                            got_needed.push(sym.name.clone());
                        }
                    }
                    // In shared libraries, ADRP/ADD for undefined/dynamic symbols must
                    // go through the GOT since the symbol address is only known at runtime.
                    R_AARCH64_ADR_PREL_PG_HI21
                    | R_AARCH64_ADD_ABS_LO12_NC
                    | R_AARCH64_LDST64_ABS_LO12_NC
                    | R_AARCH64_LDST32_ABS_LO12_NC
                    | R_AARCH64_LDST8_ABS_LO12_NC
                    | R_AARCH64_LDST16_ABS_LO12_NC => {
                        let sym_needs_got = if let Some(g) = globals.get(&sym.name) {
                            g.is_dynamic || g.defined_in.is_none()
                        } else {
                            sym.is_undefined() && !sym.is_local()
                        };
                        if sym_needs_got && !got_needed.contains(&sym.name) {
                            got_needed.push(sym.name.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let got_size = got_needed.len() as u64 * 8;
    offset += got_size;

    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_ALLOC != 0
            && sec.flags & SHF_WRITE != 0
            && sec.sh_type != SHT_NOBITS
            && sec.name != ".init_array"
            && sec.name != ".fini_array"
            && sec.flags & SHF_TLS == 0
        {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = base_addr + offset;
            sec.file_offset = offset;
            offset += sec.mem_size;
        }
    }

    // TLS
    let mut tls_addr = 0u64;
    let mut tls_file_offset_so = 0u64;
    let mut tls_file_size = 0u64;
    let mut tls_mem_size = 0u64;
    let mut tls_align = 1u64;
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_TLS != 0 && sec.flags & SHF_ALLOC != 0 && sec.sh_type != SHT_NOBITS {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = base_addr + offset;
            sec.file_offset = offset;
            if tls_addr == 0 {
                tls_addr = sec.addr;
                tls_file_offset_so = offset;
                tls_align = a;
            }
            tls_file_size += sec.mem_size;
            tls_mem_size += sec.mem_size;
            offset += sec.mem_size;
        }
    }
    if tls_addr == 0 && has_tls_sections {
        tls_addr = base_addr + offset;
        tls_file_offset_so = offset;
    }
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_TLS != 0 && sec.sh_type == SHT_NOBITS {
            let a = sec.alignment.max(1);
            let aligned = (tls_mem_size + a - 1) & !(a - 1);
            sec.addr = tls_addr + aligned;
            sec.file_offset = offset;
            tls_mem_size = aligned + sec.mem_size;
            if a > tls_align {
                tls_align = a;
            }
        }
    }
    tls_mem_size = (tls_mem_size + tls_align - 1) & !(tls_align - 1);
    let has_tls = tls_addr != 0;

    let bss_addr = base_addr + offset;
    let mut bss_size = 0u64;
    for sec in output_sections.iter_mut() {
        if sec.sh_type == SHT_NOBITS && sec.flags & SHF_ALLOC != 0 && sec.flags & SHF_TLS == 0 {
            let a = sec.alignment.max(1);
            let aligned = (bss_addr + bss_size + a - 1) & !(a - 1);
            bss_size = aligned - bss_addr + sec.mem_size;
            sec.addr = aligned;
            sec.file_offset = offset;
        }
    }

    // Merge section data
    for sec in output_sections.iter_mut() {
        if sec.sh_type == SHT_NOBITS {
            continue;
        }
        let mut data = vec![0u8; sec.mem_size as usize];
        for input in &sec.inputs {
            let sd = &objects[input.object_idx].section_data[input.section_idx];
            let s = input.output_offset as usize;
            let e = s + sd.len();
            if e <= data.len() && !sd.is_empty() {
                data[s..e].copy_from_slice(sd);
            }
        }
        sec.data = data;
    }

    // Update globals
    for (_, gsym) in globals.iter_mut() {
        if let Some(obj_idx) = gsym.defined_in {
            if gsym.section_idx == SHN_COMMON || gsym.section_idx == 0xffff {
                if let Some(bss_sec) = output_sections.iter().find(|s| s.name == ".bss") {
                    gsym.value += bss_sec.addr;
                }
            } else if gsym.section_idx != SHN_UNDEF && gsym.section_idx != SHN_ABS {
                let si = gsym.section_idx as usize;
                if let Some(&(oi, so)) = section_map.get(&(obj_idx, si)) {
                    gsym.value += output_sections[oi].addr + so;
                }
            }
        }
    }

    // Linker-provided symbols for shared library
    let linker_addrs = LinkerSymbolAddresses {
        base_addr,
        got_addr,
        dynamic_addr: dynamic_addr_so,
        bss_addr,
        bss_size,
        text_end: text_page_addr + text_total_size,
        data_start: rw_page_addr,
        init_array_start: init_array_addr_so,
        init_array_size: init_array_size_so,
        fini_array_start: fini_array_addr_so,
        fini_array_size: fini_array_size_so,
        preinit_array_start: 0,
        preinit_array_size: 0,
        rela_iplt_start: 0,
        rela_iplt_size: 0,
    };
    for sym in &get_standard_linker_symbols(&linker_addrs) {
        let entry = globals.entry(sym.name.to_string()).or_insert(GlobalSymbol {
            value: 0,
            size: 0,
            info: (sym.binding << 4),
            defined_in: None,
            from_lib: None,
            plt_idx: None,
            got_idx: None,
            section_idx: SHN_ABS,
            is_dynamic: false,
            copy_reloc: false,
            lib_sym_value: 0,
        });
        if entry.defined_in.is_none() && !entry.is_dynamic {
            entry.value = sym.value;
            entry.defined_in = Some(usize::MAX);
            entry.section_idx = SHN_ABS;
        }
    }

    // Auto-generate __start_<section> / __stop_<section> symbols (GNU ld feature)
    for (name, addr) in linker_common::resolve_start_stop_symbols(output_sections) {
        if let Some(entry) = globals.get_mut(&name) {
            if entry.defined_in.is_none() && !entry.is_dynamic {
                entry.value = addr;
                entry.defined_in = Some(usize::MAX);
                entry.section_idx = SHN_ABS;
            }
        }
    }

    // Save RW segment file size before appending section headers
    let rw_end_offset = offset;

    // Section headers: null, .dynsym, .dynstr, .gnu.hash, .dynamic, .rela.dyn, .shstrtab
    // Build .shstrtab
    let mut shstrtab_data: Vec<u8> = vec![0]; // null byte at offset 0
    let shname_dynsym = shstrtab_data.len() as u32;
    shstrtab_data.extend_from_slice(b".dynsym\0");
    let shname_dynstr = shstrtab_data.len() as u32;
    shstrtab_data.extend_from_slice(b".dynstr\0");
    let shname_gnu_hash = shstrtab_data.len() as u32;
    shstrtab_data.extend_from_slice(b".gnu.hash\0");
    let shname_dynamic = shstrtab_data.len() as u32;
    shstrtab_data.extend_from_slice(b".dynamic\0");
    let shname_rela_dyn = shstrtab_data.len() as u32;
    shstrtab_data.extend_from_slice(b".rela.dyn\0");
    let shname_shstrtab = shstrtab_data.len() as u32;
    shstrtab_data.extend_from_slice(b".shstrtab\0");

    // Append .shstrtab data and section headers after file content
    offset = (offset + 7) & !7;
    let shstrtab_offset = offset;
    let shstrtab_size = shstrtab_data.len() as u64;
    offset += shstrtab_size;
    offset = (offset + 7) & !7;
    let shdr_offset = offset;
    let sh_count: u16 = 7; // null + .dynsym + .dynstr + .gnu.hash + .dynamic + .rela.dyn + .shstrtab
    let shdr_total = sh_count as u64 * 64;
    offset += shdr_total;

    // Build output buffer
    let file_size = offset as usize;
    let mut out = vec![0u8; file_size];

    // ELF header
    out[0..4].copy_from_slice(&ELF_MAGIC);
    out[4] = ELFCLASS64;
    out[5] = ELFDATA2LSB;
    out[6] = 1;
    w16(&mut out, 16, ET_DYN);
    w16(&mut out, 18, EM_AARCH64);
    w32(&mut out, 20, 1);
    w64(&mut out, 24, 0); // e_entry = 0
    w64(&mut out, 32, 64); // e_phoff
    w64(&mut out, 40, shdr_offset); // e_shoff
    w32(&mut out, 48, 0);
    w16(&mut out, 52, 64);
    w16(&mut out, 54, 56);
    w16(&mut out, 56, phdr_count as u16);
    w16(&mut out, 58, 64); // e_shentsize
    w16(&mut out, 60, sh_count); // e_shnum
    w16(&mut out, 62, sh_count - 1); // e_shstrndx (last section)

    // Program headers
    let mut ph = 64usize;
    wphdr(
        &mut out,
        ph,
        PT_PHDR,
        PF_R,
        64,
        base_addr + 64,
        phdr_total_size,
        phdr_total_size,
        8,
    );
    ph += 56;
    let ro_seg_end = dynstr_offset + dynstr_size;
    wphdr(
        &mut out, ph, PT_LOAD, PF_R, 0, base_addr, ro_seg_end, ro_seg_end, PAGE_SIZE,
    );
    ph += 56;
    if text_total_size > 0 {
        wphdr(
            &mut out,
            ph,
            PT_LOAD,
            PF_R | PF_X,
            text_page_offset,
            text_page_addr,
            text_total_size,
            text_total_size,
            PAGE_SIZE,
        );
        ph += 56;
    } else {
        wphdr(
            &mut out,
            ph,
            PT_LOAD,
            PF_R | PF_X,
            text_page_offset,
            text_page_addr,
            0,
            0,
            PAGE_SIZE,
        );
        ph += 56;
    }
    if has_rodata {
        wphdr(
            &mut out,
            ph,
            PT_LOAD,
            PF_R,
            rodata_page_offset,
            rodata_page_addr,
            rodata_total_size,
            rodata_total_size,
            PAGE_SIZE,
        );
        ph += 56;
    }
    let rw_filesz = rw_end_offset - rw_page_offset;
    let rw_memsz = if bss_size > 0 {
        (bss_addr + bss_size) - rw_page_addr
    } else {
        rw_filesz
    };
    wphdr(
        &mut out,
        ph,
        PT_LOAD,
        PF_R | PF_W,
        rw_page_offset,
        rw_page_addr,
        rw_filesz,
        rw_memsz,
        PAGE_SIZE,
    );
    ph += 56;
    wphdr(
        &mut out,
        ph,
        PT_DYNAMIC,
        PF_R | PF_W,
        dynamic_offset,
        dynamic_addr_so,
        dynamic_size,
        dynamic_size,
        8,
    );
    ph += 56;
    wphdr(&mut out, ph, PT_GNU_STACK, PF_R | PF_W, 0, 0, 0, 0, 0x10);
    ph += 56;
    if has_tls {
        wphdr(
            &mut out,
            ph,
            PT_TLS,
            PF_R,
            tls_file_offset_so,
            tls_addr,
            tls_file_size,
            tls_mem_size,
            tls_align,
        );
    }

    // .gnu.hash
    let gh = gnu_hash_offset as usize;
    w32(&mut out, gh, gnu_hash_nbuckets);
    w32(&mut out, gh + 4, gnu_hash_symoffset as u32);
    w32(&mut out, gh + 8, gnu_hash_bloom_size);
    w32(&mut out, gh + 12, gnu_hash_bloom_shift);
    w64(&mut out, gh + 16, bloom_word);
    let buckets_off = gh + 16 + (gnu_hash_bloom_size as usize * 8);
    for (i, &b) in gnu_hash_buckets.iter().enumerate() {
        w32(&mut out, buckets_off + i * 4, b);
    }
    let chains_off = buckets_off + (gnu_hash_nbuckets as usize * 4);
    for (i, &c) in gnu_hash_chains.iter().enumerate() {
        w32(&mut out, chains_off + i * 4, c);
    }

    // .dynsym
    let mut ds = dynsym_offset as usize + 24;
    for name in &dyn_sym_names {
        let no = dynstr.get_offset(name) as u32;
        w32(&mut out, ds, no);
        if let Some(gsym) = globals.get(name) {
            if gsym.defined_in.is_some() && !gsym.is_dynamic && gsym.section_idx != SHN_UNDEF {
                if ds + 5 < out.len() {
                    out[ds + 4] = gsym.info;
                    out[ds + 5] = 0;
                }
                w16(&mut out, ds + 6, 1);
                w64(&mut out, ds + 8, gsym.value);
                w64(&mut out, ds + 16, gsym.size);
            } else {
                // Undefined/dynamic: preserve original binding (STB_WEAK vs STB_GLOBAL)
                let bind = gsym.info >> 4;
                let orig_type = gsym.info & 0xf;
                let stype = if so_plt_names.contains(name) {
                    STT_FUNC
                } else if orig_type != 0 {
                    orig_type
                } else {
                    0u8
                };
                if ds + 5 < out.len() {
                    out[ds + 4] = (bind << 4) | stype;
                    out[ds + 5] = 0;
                }
                w16(&mut out, ds + 6, 0);
                w64(&mut out, ds + 8, 0);
                w64(&mut out, ds + 16, 0);
            }
        } else {
            let stype = if so_plt_names.contains(name) {
                STT_FUNC
            } else {
                0u8 /* STT_NOTYPE */
            };
            if ds + 5 < out.len() {
                out[ds + 4] = (STB_GLOBAL << 4) | stype;
                out[ds + 5] = 0;
            }
            w16(&mut out, ds + 6, 0);
            w64(&mut out, ds + 8, 0);
            w64(&mut out, ds + 16, 0);
        }
        ds += 24;
    }

    // .dynstr
    write_bytes(&mut out, dynstr_offset as usize, dynstr.as_bytes());

    // Section data
    for sec in output_sections.iter() {
        if sec.sh_type == SHT_NOBITS || sec.data.is_empty() {
            continue;
        }
        write_bytes(&mut out, sec.file_offset as usize, &sec.data);
    }

    // GOT entries
    let mut got_sym_addrs: HashMap<String, u64> = HashMap::new();
    for (i, name) in got_needed.iter().enumerate() {
        let gea = got_addr + i as u64 * 8;
        got_sym_addrs.insert(name.clone(), gea);
        if let Some(gsym) = globals.get(name) {
            if gsym.defined_in.is_some() && !gsym.is_dynamic {
                w64(&mut out, (got_offset + i as u64 * 8) as usize, gsym.value);
            }
        }
    }
    // For PLT symbols referenced via GOT (ADR_GOT_PAGE/LD64_GOT_LO12_NC),
    // point to the GOT.PLT entry so the dynamic linker resolves them
    for (i, name) in so_plt_names.iter().enumerate() {
        let gea = so_got_plt_addr + 24 + i as u64 * 8;
        if !got_sym_addrs.contains_key(name) {
            got_sym_addrs.insert(name.clone(), gea);
        }
    }

    // Apply relocations and collect dynamic relocation entries
    let globals_snap: HashMap<String, GlobalSymbol> = globals.clone();
    // Each entry: (offset, r_info, addend)
    let mut rela_dyn_entries: Vec<(u64, u64, u64)> = Vec::new();
    const R_AARCH64_RELATIVE_DYN: u64 = 1027;
    const R_AARCH64_GLOB_DAT_DYN: u64 = 1025;

    // RELATIVE for locally-defined GOT entries, GLOB_DAT for undefined/dynamic GOT entries
    for (i, name) in got_needed.iter().enumerate() {
        let gea = got_addr + i as u64 * 8;
        if let Some(gsym) = globals_snap.get(name) {
            if gsym.defined_in.is_some() && !gsym.is_dynamic {
                rela_dyn_entries.push((gea, R_AARCH64_RELATIVE_DYN, gsym.value));
            } else {
                // Dynamic/undefined symbol: emit GLOB_DAT with the dynsym index
                let si = dyn_sym_names
                    .iter()
                    .position(|n| n == name)
                    .map(|p| p + 1)
                    .unwrap_or(0) as u64;
                rela_dyn_entries.push((gea, (si << 32) | R_AARCH64_GLOB_DAT_DYN, 0));
            }
        } else {
            // Symbol not in globals - try to find in dynsym for GLOB_DAT
            let si = dyn_sym_names
                .iter()
                .position(|n| n == name)
                .map(|p| p + 1)
                .unwrap_or(0) as u64;
            if si != 0 {
                rela_dyn_entries.push((gea, (si << 32) | R_AARCH64_GLOB_DAT_DYN, 0));
            }
        }
    }

    for obj_idx in 0..objects.len() {
        for sec_idx in 0..objects[obj_idx].sections.len() {
            let relas = &objects[obj_idx].relocations[sec_idx];
            if relas.is_empty() {
                continue;
            }
            let (out_idx, sec_off) = match section_map.get(&(obj_idx, sec_idx)) {
                Some(&v) => v,
                None => continue,
            };
            let sa = output_sections[out_idx].addr;
            let sfo = output_sections[out_idx].file_offset;

            for rela in relas {
                let si = rela.sym_idx as usize;
                if si >= objects[obj_idx].symbols.len() {
                    continue;
                }
                let sym = &objects[obj_idx].symbols[si];
                let p = sa + sec_off + rela.offset;
                let fp = (sfo + sec_off + rela.offset) as usize;
                let a = rela.addend;
                let s =
                    reloc::resolve_sym(obj_idx, sym, &globals_snap, section_map, output_sections);

                match rela.rela_type {
                    R_AARCH64_ABS64 => {
                        let val = (s as i64 + a) as u64;
                        w64(&mut out, fp, val);
                        if s != 0 {
                            rela_dyn_entries.push((p, R_AARCH64_RELATIVE_DYN, val));
                        }
                    }
                    R_AARCH64_ABS32 => {
                        w32(&mut out, fp, (s as i64 + a) as u32);
                    }
                    R_AARCH64_PREL64 => {
                        w64(&mut out, fp, (s as i64 + a - p as i64) as u64);
                    }
                    R_AARCH64_PREL32 | R_AARCH64_PREL16 => {
                        let val = (s as i64 + a - p as i64) as u32;
                        if rela.rela_type == R_AARCH64_PREL32 {
                            w32(&mut out, fp, val);
                        } else {
                            w16(&mut out, fp, val as u16);
                        }
                    }
                    R_AARCH64_ADR_PREL_PG_HI21 => {
                        // For undefined/dynamic symbols in shared libs, redirect through GOT
                        if let Some(&gea) = got_sym_addrs.get(&sym.name) {
                            if s == 0
                                || globals_snap
                                    .get(&sym.name)
                                    .is_some_and(|g| g.is_dynamic || g.defined_in.is_none())
                            {
                                let page_g = gea & !0xFFF;
                                let page_p = p & !0xFFF;
                                let imm = (page_g as i64 - page_p as i64) >> 12;
                                reloc::encode_adrp(&mut out, fp, imm);
                            } else {
                                let sa_val = (s as i64 + a) as u64;
                                let page_s = sa_val & !0xFFF;
                                let page_p = p & !0xFFF;
                                let imm = (page_s as i64 - page_p as i64) >> 12;
                                reloc::encode_adrp(&mut out, fp, imm);
                            }
                        } else {
                            let sa_val = (s as i64 + a) as u64;
                            let page_s = sa_val & !0xFFF;
                            let page_p = p & !0xFFF;
                            let imm = (page_s as i64 - page_p as i64) >> 12;
                            reloc::encode_adrp(&mut out, fp, imm);
                        }
                    }
                    R_AARCH64_ADD_ABS_LO12_NC => {
                        // For undefined/dynamic symbols in shared libs, convert ADD to LDR from GOT
                        if let Some(&gea) = got_sym_addrs.get(&sym.name) {
                            if s == 0
                                || globals_snap
                                    .get(&sym.name)
                                    .is_some_and(|g| g.is_dynamic || g.defined_in.is_none())
                            {
                                // Convert: ADD Xd, Xn, #imm -> LDR Xd, [Xn, #imm]
                                // The ADD instruction loaded address = base + lo12
                                // We need LDR to dereference the GOT entry instead
                                let lo12 = (gea & 0xFFF) as u32;
                                if fp + 4 <= out.len() {
                                    let insn = read_u32(&out, fp);
                                    let rd = insn & 0x1f;
                                    let rn = (insn >> 5) & 0x1f;
                                    // LDR Xd, [Xn, #imm] = 0xF9400000 | (imm/8 << 10) | (Rn << 5) | Rd
                                    let ldr = 0xf9400000u32 | ((lo12 / 8) << 10) | (rn << 5) | rd;
                                    w32(&mut out, fp, ldr);
                                }
                            } else {
                                let sa_val = (s as i64 + a) as u64;
                                reloc::encode_add_imm12(&mut out, fp, (sa_val & 0xFFF) as u32);
                            }
                        } else {
                            let sa_val = (s as i64 + a) as u64;
                            reloc::encode_add_imm12(&mut out, fp, (sa_val & 0xFFF) as u32);
                        }
                    }
                    R_AARCH64_CALL26 | R_AARCH64_JUMP26 => {
                        if fp + 4 > out.len() {
                            continue;
                        }
                        let mut target = (s as i64 + a) as u64;
                        // If the symbol has a PLT entry, redirect to it
                        if target == 0 && !sym.name.is_empty() {
                            if let Some(g) = globals_snap.get(&sym.name) {
                                if let Some(pi) = g.plt_idx {
                                    target = so_plt_addr + 32 + pi as u64 * 16;
                                }
                            }
                        }
                        if target == 0 {
                            // Weak undefined with no PLT - NOP it
                            w32(&mut out, fp, 0xd503201f);
                        } else {
                            let offset_val = target as i64 - p as i64;
                            let mut insn = read_u32(&out, fp);
                            let imm26 = ((offset_val >> 2) as u32) & 0x3ffffff;
                            insn = (insn & 0xfc000000) | imm26;
                            w32(&mut out, fp, insn);
                        }
                    }
                    R_AARCH64_LDST8_ABS_LO12_NC => {
                        let sa_val = (s as i64 + a) as u64;
                        reloc::encode_ldst_imm12(&mut out, fp, (sa_val & 0xFFF) as u32, 0);
                    }
                    R_AARCH64_LDST16_ABS_LO12_NC => {
                        let sa_val = (s as i64 + a) as u64;
                        reloc::encode_ldst_imm12(&mut out, fp, (sa_val & 0xFFF) as u32, 1);
                    }
                    R_AARCH64_LDST32_ABS_LO12_NC => {
                        let sa_val = (s as i64 + a) as u64;
                        reloc::encode_ldst_imm12(&mut out, fp, (sa_val & 0xFFF) as u32, 2);
                    }
                    R_AARCH64_LDST64_ABS_LO12_NC => {
                        let sa_val = (s as i64 + a) as u64;
                        reloc::encode_ldst_imm12(&mut out, fp, (sa_val & 0xFFF) as u32, 3);
                    }
                    R_AARCH64_LDST128_ABS_LO12_NC => {
                        let sa_val = (s as i64 + a) as u64;
                        reloc::encode_ldst_imm12(&mut out, fp, (sa_val & 0xFFF) as u32, 4);
                    }
                    R_AARCH64_ADR_GOT_PAGE => {
                        if let Some(&gea) = got_sym_addrs.get(&sym.name) {
                            let page_g = gea & !0xFFF;
                            let page_p = p & !0xFFF;
                            let imm = (page_g as i64 - page_p as i64) >> 12;
                            reloc::encode_adrp(&mut out, fp, imm);
                        } else {
                            let sa_val = (s as i64 + a) as u64;
                            let page_s = sa_val & !0xFFF;
                            let page_p = p & !0xFFF;
                            let imm = (page_s as i64 - page_p as i64) >> 12;
                            reloc::encode_adrp(&mut out, fp, imm);
                        }
                    }
                    R_AARCH64_LD64_GOT_LO12_NC => {
                        if let Some(&gea) = got_sym_addrs.get(&sym.name) {
                            reloc::encode_ldst_imm12(&mut out, fp, (gea & 0xFFF) as u32, 3);
                        } else {
                            let sa_val = (s as i64 + a) as u64;
                            reloc::encode_ldst_imm12(&mut out, fp, (sa_val & 0xFFF) as u32, 3);
                        }
                    }
                    R_AARCH64_MOVW_UABS_G0 | R_AARCH64_MOVW_UABS_G0_NC => {
                        let sa_val = (s as i64 + a) as u64;
                        reloc::encode_movw(&mut out, fp, (sa_val & 0xffff) as u32);
                    }
                    R_AARCH64_MOVW_UABS_G1_NC => {
                        let sa_val = (s as i64 + a) as u64;
                        reloc::encode_movw(&mut out, fp, ((sa_val >> 16) & 0xffff) as u32);
                    }
                    R_AARCH64_MOVW_UABS_G2_NC => {
                        let sa_val = (s as i64 + a) as u64;
                        reloc::encode_movw(&mut out, fp, ((sa_val >> 32) & 0xffff) as u32);
                    }
                    R_AARCH64_MOVW_UABS_G3 => {
                        let sa_val = (s as i64 + a) as u64;
                        reloc::encode_movw(&mut out, fp, ((sa_val >> 48) & 0xffff) as u32);
                    }
                    R_AARCH64_NONE => {}
                    other => {
                        eprintln!(
                            "warning: unsupported relocation type {} for '{}' in shared library",
                            other, sym.name
                        );
                    }
                }
            }
        }
    }

    // Write .rela.dyn (RELATIVE + GLOB_DAT entries)
    // Sort: put RELATIVE entries first (for DT_RELACOUNT), then GLOB_DAT
    let relative_count = rela_dyn_entries
        .iter()
        .filter(|(_, info, _)| *info == R_AARCH64_RELATIVE_DYN)
        .count();
    rela_dyn_entries.sort_by_key(|(_, info, _)| {
        if *info == R_AARCH64_RELATIVE_DYN {
            0u8
        } else {
            1u8
        }
    });
    let actual_rela_count = rela_dyn_entries.len();
    let rela_dyn_size = actual_rela_count as u64 * 24;
    let mut rd = rela_dyn_offset as usize;
    for (rel_offset, rel_info, rel_addend) in &rela_dyn_entries {
        if rd + 24 <= out.len() {
            w64(&mut out, rd, *rel_offset);
            w64(&mut out, rd + 8, *rel_info);
            w64(&mut out, rd + 16, *rel_addend);
            rd += 24;
        }
    }

    // .plt stubs (AArch64 PLT stubs for shared library)
    if so_plt_size > 0 {
        let po = so_plt_offset as usize;
        // PLT header (32 bytes)
        let got2_addr = so_got_plt_addr + 16;
        let page_g = got2_addr & !0xFFF;
        let page_p = so_plt_addr & !0xFFF;
        let page_diff = ((page_g as i64 - page_p as i64) >> 12) as i32;
        let immlo = (page_diff & 3) as u32;
        let immhi = ((page_diff >> 2) & 0x7ffff) as u32;
        w32(&mut out, po, 0xa9bf7bf0u32); // stp x16, x30, [sp, #-16]!
        w32(&mut out, po + 4, 0x90000010 | (immlo << 29) | (immhi << 5)); // adrp x16, GOT+16
        let lo12 = (got2_addr & 0xFFF) as u32;
        w32(&mut out, po + 8, 0xf9400211 | ((lo12 / 8) << 10)); // ldr x17, [x16, #lo12]
        w32(&mut out, po + 12, 0x91000210 | ((lo12 & 0xFFF) << 10)); // add x16, x16, #lo12
        w32(&mut out, po + 16, 0xd61f0220u32); // br x17
        w32(&mut out, po + 20, 0xd503201fu32); // nop
        w32(&mut out, po + 24, 0xd503201fu32); // nop
        w32(&mut out, po + 28, 0xd503201fu32); // nop

        // Individual PLT entries (16 bytes each)
        for (i, _) in so_plt_names.iter().enumerate() {
            let ep = po + 32 + i * 16;
            let pea = so_plt_addr + 32 + i as u64 * 16;
            let gea = so_got_plt_addr + 24 + i as u64 * 8;
            let page_g = gea & !0xFFF;
            let page_p = pea & !0xFFF;
            let page_diff = ((page_g as i64 - page_p as i64) >> 12) as i32;
            let immlo = (page_diff & 3) as u32;
            let immhi = ((page_diff >> 2) & 0x7ffff) as u32;
            w32(&mut out, ep, 0x90000010 | (immlo << 29) | (immhi << 5)); // adrp x16
            let lo12 = (gea & 0xFFF) as u32;
            w32(&mut out, ep + 4, 0xf9400211 | ((lo12 / 8) << 10)); // ldr x17, [x16, #lo12]
            w32(&mut out, ep + 8, 0x91000210 | ((lo12 & 0xFFF) << 10)); // add x16, x16, #lo12
            w32(&mut out, ep + 12, 0xd61f0220u32); // br x17
        }
    }

    // GOT.PLT entries
    if so_got_plt_size > 0 {
        let gp = so_got_plt_offset as usize;
        w64(&mut out, gp, dynamic_addr_so); // GOT[0] = _DYNAMIC
                                            // GOT[1] and GOT[2] are filled by the dynamic linker
                                            // GOT[3..] are PLT GOT entries: initialized to PLT[0] (resolved eagerly via DF_BIND_NOW)
        for i in 0..so_plt_names.len() {
            w64(&mut out, gp + 24 + i * 8, so_plt_addr);
        }
    }

    // .rela.plt (R_AARCH64_JUMP_SLOT)
    const R_AARCH64_JUMP_SLOT: u64 = 1026;
    if so_rela_plt_size > 0 {
        let mut rp = so_rela_plt_offset as usize;
        for (i, name) in so_plt_names.iter().enumerate() {
            let gea = so_got_plt_addr + 24 + i as u64 * 8;
            // Find the dynsym index for this symbol
            let si = dyn_sym_names
                .iter()
                .position(|n| n == name)
                .map(|p| p + 1)
                .unwrap_or(0) as u64;
            w64(&mut out, rp, gea);
            w64(&mut out, rp + 8, (si << 32) | R_AARCH64_JUMP_SLOT);
            w64(&mut out, rp + 16, 0);
            rp += 24;
        }
    }

    // .dynamic
    let mut dd = dynamic_offset as usize;
    for lib in needed_sonames {
        let so = dynstr.get_offset(lib);
        w64(&mut out, dd, DT_NEEDED as u64);
        w64(&mut out, dd + 8, so as u64);
        dd += 16;
    }
    if let Some(ref sn) = soname {
        let so = dynstr.get_offset(sn);
        w64(&mut out, dd, DT_SONAME as u64);
        w64(&mut out, dd + 8, so as u64);
        dd += 16;
    }
    for &(tag, val) in &[
        (DT_STRTAB, dynstr_addr),
        (DT_SYMTAB, dynsym_addr),
        (DT_STRSZ, dynstr_size),
        (DT_SYMENT, 24),
        (DT_RELA, rela_dyn_addr),
        (DT_RELASZ, rela_dyn_size),
        (DT_RELAENT, 24),
        (DT_RELACOUNT, relative_count as u64),
        (DT_GNU_HASH, gnu_hash_addr),
    ] {
        w64(&mut out, dd, tag as u64);
        w64(&mut out, dd + 8, val);
        dd += 16;
    }
    if !so_plt_names.is_empty() {
        for &(tag, val) in &[
            (DT_PLTGOT, so_got_plt_addr),
            (DT_PLTRELSZ, so_rela_plt_size),
            (DT_PLTREL, 7u64),
            (DT_JMPREL, so_rela_plt_addr),
        ] {
            w64(&mut out, dd, tag as u64);
            w64(&mut out, dd + 8, val);
            dd += 16;
        }
    }
    if has_init_array {
        w64(&mut out, dd, DT_INIT_ARRAY as u64);
        w64(&mut out, dd + 8, init_array_addr_so);
        dd += 16;
        w64(&mut out, dd, DT_INIT_ARRAYSZ as u64);
        w64(&mut out, dd + 8, init_array_size_so);
        dd += 16;
    }
    if has_fini_array {
        w64(&mut out, dd, DT_FINI_ARRAY as u64);
        w64(&mut out, dd + 8, fini_array_addr_so);
        dd += 16;
        w64(&mut out, dd, DT_FINI_ARRAYSZ as u64);
        w64(&mut out, dd + 8, fini_array_size_so);
        dd += 16;
    }
    // Force eager binding so GOT.PLT entries are resolved before execution
    w64(&mut out, dd, DT_FLAGS as u64);
    w64(&mut out, dd + 8, DF_BIND_NOW as u64);
    dd += 16;
    w64(&mut out, dd, DT_FLAGS_1 as u64);
    w64(&mut out, dd + 8, DF_1_NOW as u64);
    dd += 16;
    w64(&mut out, dd, DT_NULL as u64);
    w64(&mut out, dd + 8, 0);

    // Write .shstrtab
    write_bytes(&mut out, shstrtab_offset as usize, &shstrtab_data);

    // Write section headers (64 bytes each for ELF64)
    // Helper to write one section header
    let mut sh = shdr_offset as usize;
    // [0] SHT_NULL
    // (already zeroed)
    sh += 64;
    // [1] .dynsym (SHT_DYNSYM = 11)
    w32(&mut out, sh, shname_dynsym);
    w32(&mut out, sh + 4, 11); // SHT_DYNSYM
    w64(&mut out, sh + 8, 0x2); // SHF_ALLOC
    w64(&mut out, sh + 16, dynsym_addr); // sh_addr
    w64(&mut out, sh + 24, dynsym_offset); // sh_offset
    w64(&mut out, sh + 32, dynsym_size); // sh_size
    w32(&mut out, sh + 40, 2); // sh_link = .dynstr index
    w32(&mut out, sh + 44, 1); // sh_info = 1 (one local sym: null)
    w64(&mut out, sh + 48, 8); // sh_addralign
    w64(&mut out, sh + 56, 24); // sh_entsize
    sh += 64;
    // [2] .dynstr (SHT_STRTAB = 3)
    w32(&mut out, sh, shname_dynstr);
    w32(&mut out, sh + 4, 3); // SHT_STRTAB
    w64(&mut out, sh + 8, 0x2); // SHF_ALLOC
    w64(&mut out, sh + 16, dynstr_addr);
    w64(&mut out, sh + 24, dynstr_offset);
    w64(&mut out, sh + 32, dynstr_size);
    w64(&mut out, sh + 48, 1); // sh_addralign
    sh += 64;
    // [3] .gnu.hash (SHT_GNU_HASH = 0x6ffffff6)
    w32(&mut out, sh, shname_gnu_hash);
    w32(&mut out, sh + 4, 0x6ffffff6u32); // SHT_GNU_HASH
    w64(&mut out, sh + 8, 0x2); // SHF_ALLOC
    w64(&mut out, sh + 16, gnu_hash_addr);
    w64(&mut out, sh + 24, gnu_hash_offset);
    w64(&mut out, sh + 32, gnu_hash_size);
    w32(&mut out, sh + 40, 1); // sh_link = .dynsym index
    w64(&mut out, sh + 48, 8);
    sh += 64;
    // [4] .dynamic (SHT_DYNAMIC = 6)
    w32(&mut out, sh, shname_dynamic);
    w32(&mut out, sh + 4, 6); // SHT_DYNAMIC
    w64(&mut out, sh + 8, 0x3); // SHF_WRITE | SHF_ALLOC
    w64(&mut out, sh + 16, dynamic_addr_so);
    w64(&mut out, sh + 24, dynamic_offset);
    w64(&mut out, sh + 32, dynamic_size);
    w32(&mut out, sh + 40, 2); // sh_link = .dynstr index
    w64(&mut out, sh + 48, 8);
    w64(&mut out, sh + 56, 16); // sh_entsize
    sh += 64;
    // [5] .rela.dyn (SHT_RELA = 4)
    w32(&mut out, sh, shname_rela_dyn);
    w32(&mut out, sh + 4, 4); // SHT_RELA
    w64(&mut out, sh + 8, 0x2); // SHF_ALLOC
    w64(&mut out, sh + 16, rela_dyn_addr);
    w64(&mut out, sh + 24, rela_dyn_offset);
    w64(&mut out, sh + 32, rela_dyn_size);
    w32(&mut out, sh + 40, 1); // sh_link = .dynsym index
    w64(&mut out, sh + 48, 8);
    w64(&mut out, sh + 56, 24); // sh_entsize
    sh += 64;
    // [6] .shstrtab (SHT_STRTAB = 3)
    w32(&mut out, sh, shname_shstrtab);
    w32(&mut out, sh + 4, 3); // SHT_STRTAB
    w64(&mut out, sh + 24, shstrtab_offset);
    w64(&mut out, sh + 32, shstrtab_size);
    w64(&mut out, sh + 48, 1);

    std::fs::write(output_path, &out)
        .map_err(|e| format!("failed to write '{}': {}", output_path, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(output_path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}
