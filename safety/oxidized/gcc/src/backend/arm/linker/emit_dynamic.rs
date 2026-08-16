//! Dynamic executable emission for the AArch64 linker.
//!
//! Emits a dynamically-linked ELF64 executable with PLT/GOT, `.dynamic` section,
//! `.dynsym`/`.dynstr` tables, `.rela.dyn`/`.rela.plt`, and copy relocations.
//! This is the code path used when shared library symbols are present.

use std::collections::HashMap;

use super::elf::*;
use super::reloc;
use super::types::{GlobalSymbol, BASE_ADDR, INTERP, PAGE_SIZE};
use crate::backend::linker_common;
use linker_common::{DynStrTab, OutputSection};

// ── Dynamic executable emission ─────────────────────────────────────────

/// Emit a dynamically-linked AArch64 ELF executable with PLT/GOT/.dynamic support.
pub(super) fn emit_dynamic_executable(
    objects: &[ElfObject],
    globals: &mut HashMap<String, GlobalSymbol>,
    output_sections: &mut [OutputSection],
    section_map: &HashMap<(usize, usize), (usize, u64)>,
    plt_names: &[String],
    got_entries: &[(String, bool)],
    needed_sonames: &[String],
    output_path: &str,
    export_dynamic: bool,
) -> Result<(), String> {
    let mut dynstr = DynStrTab::new();
    for lib in needed_sonames {
        dynstr.add(lib);
    }

    // Build dynamic symbol name list
    let mut dyn_sym_names: Vec<String> = Vec::new();
    for name in plt_names {
        if !dyn_sym_names.contains(name) {
            dyn_sym_names.push(name.clone());
        }
    }
    for (name, is_plt) in got_entries {
        if !name.is_empty() && !*is_plt && !dyn_sym_names.contains(name) {
            if let Some(gsym) = globals.get(name) {
                if gsym.is_dynamic && !gsym.copy_reloc {
                    dyn_sym_names.push(name.clone());
                }
            }
        }
    }
    let gnu_hash_symoffset = 1 + dyn_sym_names.len();

    // Collect copy relocation symbols
    let copy_reloc_syms: Vec<(String, u64)> = globals
        .iter()
        .filter(|(_, g)| g.copy_reloc)
        .map(|(n, g)| (n.clone(), g.size))
        .collect();
    for (name, _) in &copy_reloc_syms {
        if !dyn_sym_names.contains(name) {
            dyn_sym_names.push(name.clone());
        }
    }

    if export_dynamic {
        let mut exported: Vec<String> = globals
            .iter()
            .filter(|(_, g)| {
                g.section_idx != SHN_UNDEF && !g.is_dynamic && !g.copy_reloc && (g.info >> 4) != 0
            })
            .map(|(n, _)| n.clone())
            .collect();
        exported.sort();
        for name in exported {
            if !dyn_sym_names.contains(&name) {
                dyn_sym_names.push(name);
            }
        }
    }

    for name in &dyn_sym_names {
        dynstr.add(name);
    }

    let dynsym_count = 1 + dyn_sym_names.len();
    let dynsym_size = dynsym_count as u64 * 24;
    let dynstr_size = dynstr.as_bytes().len() as u64;
    let rela_plt_size = plt_names.len() as u64 * 24;
    let rela_dyn_glob_count = got_entries
        .iter()
        .filter(|(n, p)| {
            !n.is_empty()
                && !*p
                && globals
                    .get(n)
                    .map(|g| g.is_dynamic && !g.copy_reloc)
                    .unwrap_or(false)
        })
        .count();
    let rela_dyn_count = rela_dyn_glob_count + copy_reloc_syms.len();
    let rela_dyn_size = rela_dyn_count as u64 * 24;

    // Build .gnu.hash
    let num_hashed = dyn_sym_names.len() - (gnu_hash_symoffset - 1);
    let gnu_hash_nbuckets = if num_hashed == 0 {
        1
    } else {
        num_hashed.next_power_of_two().max(1)
    } as u32;
    let gnu_hash_bloom_size: u32 = 1;
    let gnu_hash_bloom_shift: u32 = 6;

    let hashed_sym_hashes: Vec<u32> = dyn_sym_names[gnu_hash_symoffset - 1..]
        .iter()
        .map(|name| linker_common::gnu_hash(name.as_bytes()))
        .collect();

    let mut bloom_word: u64 = 0;
    for &h in &hashed_sym_hashes {
        bloom_word |= 1u64 << (h as u64 % 64);
        bloom_word |= 1u64 << ((h >> gnu_hash_bloom_shift) as u64 % 64);
    }

    if num_hashed > 0 {
        let hashed_start = gnu_hash_symoffset - 1;
        let mut hashed_with_hash: Vec<(String, u32)> = dyn_sym_names[hashed_start..]
            .iter()
            .zip(hashed_sym_hashes.iter())
            .map(|(n, &h)| (n.clone(), h))
            .collect();
        hashed_with_hash.sort_by_key(|(_, h)| h % gnu_hash_nbuckets);
        for (i, (name, _)) in hashed_with_hash.iter().enumerate() {
            dyn_sym_names[hashed_start + i] = name.clone();
        }
    }

    let hashed_sym_hashes: Vec<u32> = dyn_sym_names[gnu_hash_symoffset - 1..]
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
    // PLT: 32 bytes header + 16 bytes per entry
    let plt_size = if plt_names.is_empty() {
        0u64
    } else {
        32 + 16 * plt_names.len() as u64
    };
    let got_plt_count = 3 + plt_names.len();
    let got_plt_size = got_plt_count as u64 * 8;
    let got_globdat_count = got_entries
        .iter()
        .filter(|(n, p)| !n.is_empty() && !*p)
        .count();
    let got_size = got_globdat_count as u64 * 8;

    let has_init_array = output_sections
        .iter()
        .any(|s| s.name == ".init_array" && s.mem_size > 0);
    let has_fini_array = output_sections
        .iter()
        .any(|s| s.name == ".fini_array" && s.mem_size > 0);
    let mut dyn_count = needed_sonames.len() as u64 + 16; // fixed entries + DT_FLAGS + DT_FLAGS_1 + NULL
    if has_init_array {
        dyn_count += 2;
    }
    if has_fini_array {
        dyn_count += 2;
    }
    let dynamic_size = dyn_count * 16;

    let has_tls_sections = output_sections
        .iter()
        .any(|s| s.flags & SHF_TLS != 0 && s.flags & SHF_ALLOC != 0);
    // phdrs: PHDR, INTERP, LOAD(ro), LOAD(text), LOAD(rodata), LOAD(rw), DYNAMIC, GNU_STACK, [TLS]
    let phdr_count: u64 = if has_tls_sections { 9 } else { 8 };
    let phdr_total_size = phdr_count * 56;

    // === Layout ===
    let mut offset = 64 + phdr_total_size;
    let interp_offset = offset;
    let interp_addr = BASE_ADDR + offset;
    offset += INTERP.len() as u64;

    offset = (offset + 7) & !7;
    let gnu_hash_offset = offset;
    let gnu_hash_addr = BASE_ADDR + offset;
    offset += gnu_hash_size;
    offset = (offset + 7) & !7;
    let dynsym_offset = offset;
    let dynsym_addr = BASE_ADDR + offset;
    offset += dynsym_size;
    let dynstr_offset = offset;
    let dynstr_addr = BASE_ADDR + offset;
    offset += dynstr_size;
    offset = (offset + 7) & !7;
    let rela_dyn_offset = offset;
    let rela_dyn_addr = BASE_ADDR + offset;
    offset += rela_dyn_size;
    offset = (offset + 7) & !7;
    let rela_plt_offset = offset;
    let rela_plt_addr = BASE_ADDR + offset;
    offset += rela_plt_size;

    // Text segment
    offset = (offset + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let text_page_offset = offset;
    let text_page_addr = BASE_ADDR + offset;
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_EXECINSTR != 0 && sec.flags & SHF_ALLOC != 0 {
            let a = sec.alignment.max(4);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = BASE_ADDR + offset;
            sec.file_offset = offset;
            offset += sec.mem_size;
        }
    }
    // PLT in text segment
    let (plt_addr, plt_offset) = if plt_size > 0 {
        offset = (offset + 15) & !15;
        let a = BASE_ADDR + offset;
        let o = offset;
        offset += plt_size;
        (a, o)
    } else {
        (0u64, 0u64)
    };
    let text_total_size = offset - text_page_offset;

    // Rodata segment (separate LOAD R)
    offset = (offset + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let rodata_page_offset = offset;
    let rodata_page_addr = BASE_ADDR + offset;
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_ALLOC != 0
            && sec.flags & SHF_EXECINSTR == 0
            && sec.flags & SHF_WRITE == 0
            && sec.sh_type != SHT_NOBITS
            && sec.flags & SHF_TLS == 0
        {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = BASE_ADDR + offset;
            sec.file_offset = offset;
            offset += sec.mem_size;
        }
    }
    let rodata_total_size = offset - rodata_page_offset;

    // RW segment
    offset = (offset + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let rw_page_offset = offset;
    let rw_page_addr = BASE_ADDR + offset;

    let mut init_array_addr = 0u64;
    let mut init_array_size = 0u64;
    let mut fini_array_addr = 0u64;
    let mut fini_array_size = 0u64;

    for sec in output_sections.iter_mut() {
        if sec.name == ".init_array" {
            let a = sec.alignment.max(8);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = BASE_ADDR + offset;
            sec.file_offset = offset;
            init_array_addr = sec.addr;
            init_array_size = sec.mem_size;
            offset += sec.mem_size;
            break;
        }
    }
    for sec in output_sections.iter_mut() {
        if sec.name == ".fini_array" {
            let a = sec.alignment.max(8);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = BASE_ADDR + offset;
            sec.file_offset = offset;
            fini_array_addr = sec.addr;
            fini_array_size = sec.mem_size;
            offset += sec.mem_size;
            break;
        }
    }

    offset = (offset + 7) & !7;
    let dynamic_offset = offset;
    let dynamic_addr = BASE_ADDR + offset;
    offset += dynamic_size;
    offset = (offset + 7) & !7;
    let got_offset = offset;
    let got_addr = BASE_ADDR + offset;
    offset += got_size;
    offset = (offset + 7) & !7;
    let got_plt_offset = offset;
    let got_plt_addr = BASE_ADDR + offset;
    offset += got_plt_size;

    // Data.rel.ro
    for sec in output_sections.iter_mut() {
        if sec.name == ".data.rel.ro" {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = BASE_ADDR + offset;
            sec.file_offset = offset;
            offset += sec.mem_size;
        }
    }

    // Remaining data sections
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_ALLOC != 0
            && sec.flags & SHF_WRITE != 0
            && sec.sh_type != SHT_NOBITS
            && sec.flags & SHF_TLS == 0
            && sec.name != ".init_array"
            && sec.name != ".fini_array"
            && sec.name != ".data.rel.ro"
        {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = BASE_ADDR + offset;
            sec.file_offset = offset;
            offset += sec.mem_size;
        }
    }

    // TLS sections
    let mut tls_addr = 0u64;
    let mut tls_file_offset = 0u64;
    let mut tls_file_size = 0u64;
    let mut tls_mem_size = 0u64;
    let mut tls_align = 1u64;
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_TLS != 0 && sec.flags & SHF_ALLOC != 0 && sec.sh_type != SHT_NOBITS {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = BASE_ADDR + offset;
            sec.file_offset = offset;
            if tls_addr == 0 {
                tls_addr = sec.addr;
                tls_file_offset = offset;
                tls_align = a;
            }
            tls_file_size += sec.mem_size;
            tls_mem_size += sec.mem_size;
            offset += sec.mem_size;
        }
    }
    if tls_addr == 0 && has_tls_sections {
        tls_addr = BASE_ADDR + offset;
        tls_file_offset = offset;
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

    let bss_addr = BASE_ADDR + offset;
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

    // BSS space for copy relocations
    let mut copy_reloc_addr_map: HashMap<(String, u64), u64> = HashMap::new();
    for (name, size) in &copy_reloc_syms {
        let gsym = globals.get(name).cloned();
        let key = gsym.as_ref().and_then(|g| {
            g.from_lib
                .as_ref()
                .map(|lib| (lib.clone(), g.lib_sym_value))
        });
        let addr = if let Some(ref k) = key {
            if let Some(&existing_addr) = copy_reloc_addr_map.get(k) {
                existing_addr
            } else {
                let aligned = (bss_addr + bss_size + 7) & !7;
                bss_size = aligned - bss_addr + size;
                copy_reloc_addr_map.insert(k.clone(), aligned);
                aligned
            }
        } else {
            let aligned = (bss_addr + bss_size + 7) & !7;
            bss_size = aligned - bss_addr + size;
            aligned
        };
        if let Some(gsym) = globals.get_mut(name) {
            gsym.value = addr;
            gsym.defined_in = Some(usize::MAX);
        }
    }

    let rw_filesz = offset - rw_page_offset;
    let rw_memsz = if bss_size > 0 {
        (bss_addr + bss_size) - rw_page_addr
    } else {
        rw_filesz
    };

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

    // Update global symbol addresses
    for (_, gsym) in globals.iter_mut() {
        if let Some(obj_idx) = gsym.defined_in {
            if obj_idx == usize::MAX {
                continue;
            } // linker-defined or copy-reloc
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

    // Define linker-provided symbols
    let text_seg_end = text_page_addr + text_total_size;
    let linker_addrs = LinkerSymbolAddresses {
        base_addr: BASE_ADDR,
        got_addr: got_plt_addr,
        dynamic_addr,
        bss_addr,
        bss_size,
        text_end: text_seg_end,
        data_start: rw_page_addr,
        init_array_start: init_array_addr,
        init_array_size,
        fini_array_start: fini_array_addr,
        fini_array_size,
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

    let entry_addr = globals
        .get("_start")
        .map(|s| s.value)
        .unwrap_or(text_page_addr);

    // === Build output buffer ===
    let file_size = offset as usize;
    let mut out = vec![0u8; file_size];

    // ELF header
    out[0..4].copy_from_slice(&ELF_MAGIC);
    out[4] = ELFCLASS64;
    out[5] = ELFDATA2LSB;
    out[6] = 1;
    out[7] = 0; // ELFOSABI_NONE for dynamic executables
    w16(&mut out, 16, ET_EXEC);
    w16(&mut out, 18, EM_AARCH64);
    w32(&mut out, 20, 1);
    w64(&mut out, 24, entry_addr);
    w64(&mut out, 32, 64);
    w64(&mut out, 40, 0);
    w32(&mut out, 48, 0);
    w16(&mut out, 52, 64);
    w16(&mut out, 54, 56);
    w16(&mut out, 56, phdr_count as u16);
    w16(&mut out, 58, 64);
    w16(&mut out, 60, 0);
    w16(&mut out, 62, 0);

    // Program headers
    let mut ph = 64usize;
    wphdr(
        &mut out,
        ph,
        PT_PHDR,
        PF_R,
        64,
        BASE_ADDR + 64,
        phdr_total_size,
        phdr_total_size,
        8,
    );
    ph += 56;
    wphdr(
        &mut out,
        ph,
        PT_INTERP,
        PF_R,
        interp_offset,
        interp_addr,
        INTERP.len() as u64,
        INTERP.len() as u64,
        1,
    );
    ph += 56;
    let ro_seg_end = rela_plt_offset + rela_plt_size;
    wphdr(
        &mut out, ph, PT_LOAD, PF_R, 0, BASE_ADDR, ro_seg_end, ro_seg_end, PAGE_SIZE,
    );
    ph += 56;
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
    if rodata_total_size > 0 {
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
    } else {
        // Empty placeholder segment to keep phdr count consistent
        wphdr(
            &mut out,
            ph,
            PT_LOAD,
            PF_R,
            rodata_page_offset,
            rodata_page_addr,
            0,
            0,
            PAGE_SIZE,
        );
        ph += 56;
    }
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
        dynamic_addr,
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
            tls_file_offset,
            tls_addr,
            tls_file_size,
            tls_mem_size,
            tls_align,
        );
    }

    // .interp
    write_bytes(&mut out, interp_offset as usize, INTERP);

    // .gnu.hash
    let gh = gnu_hash_offset as usize;
    w32(&mut out, gh, gnu_hash_nbuckets);
    w32(&mut out, gh + 4, gnu_hash_symoffset as u32);
    w32(&mut out, gh + 8, gnu_hash_bloom_size);
    w32(&mut out, gh + 12, gnu_hash_bloom_shift);
    let bloom_off = gh + 16;
    w64(&mut out, bloom_off, bloom_word);
    let buckets_off = bloom_off + (gnu_hash_bloom_size as usize * 8);
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
            if gsym.copy_reloc {
                if ds + 5 < out.len() {
                    out[ds + 4] = (STB_GLOBAL << 4) | STT_OBJECT;
                    out[ds + 5] = 0;
                }
                w16(&mut out, ds + 6, 1);
                w64(&mut out, ds + 8, gsym.value);
                w64(&mut out, ds + 16, gsym.size);
            } else if !gsym.is_dynamic && gsym.section_idx != SHN_UNDEF && gsym.value != 0 {
                let stt = gsym.info & 0xf;
                let stb = gsym.info >> 4;
                if ds + 5 < out.len() {
                    out[ds + 4] = (stb << 4) | stt;
                    out[ds + 5] = 0;
                }
                w16(&mut out, ds + 6, 1);
                w64(&mut out, ds + 8, gsym.value);
                w64(&mut out, ds + 16, gsym.size);
            } else {
                // Preserve original binding (STB_WEAK vs STB_GLOBAL) and type
                let bind = gsym.info >> 4;
                let stype = gsym.info & 0xf;
                let st_info = (bind << 4) | if stype != 0 { stype } else { STT_FUNC };
                if ds + 5 < out.len() {
                    out[ds + 4] = st_info;
                    out[ds + 5] = 0;
                }
                w16(&mut out, ds + 6, 0);
                w64(&mut out, ds + 8, 0);
                w64(&mut out, ds + 16, 0);
            }
        } else {
            if ds + 5 < out.len() {
                out[ds + 4] = (STB_GLOBAL << 4) | STT_FUNC;
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

    // .rela.dyn (GLOB_DAT + COPY)
    // AArch64: R_AARCH64_GLOB_DAT = 1025, R_AARCH64_COPY = 1024
    const R_AARCH64_GLOB_DAT: u64 = 1025;
    const R_AARCH64_COPY: u64 = 1024;
    let mut rd = rela_dyn_offset as usize;
    let mut gd_a = got_addr;
    for (name, is_plt) in got_entries {
        if name.is_empty() || *is_plt {
            continue;
        }
        let is_dynamic = globals
            .get(name)
            .map(|g| g.is_dynamic && !g.copy_reloc)
            .unwrap_or(false);
        if is_dynamic {
            let si = dyn_sym_names
                .iter()
                .position(|n| n == name)
                .map(|i| i + 1)
                .unwrap_or(0) as u64;
            w64(&mut out, rd, gd_a);
            w64(&mut out, rd + 8, (si << 32) | R_AARCH64_GLOB_DAT);
            w64(&mut out, rd + 16, 0);
            rd += 24;
        }
        gd_a += 8;
    }
    for (name, _) in &copy_reloc_syms {
        if let Some(gsym) = globals.get(name) {
            let si = dyn_sym_names
                .iter()
                .position(|n| n == name)
                .map(|i| i + 1)
                .unwrap_or(0) as u64;
            let copy_addr = gsym.value;
            w64(&mut out, rd, copy_addr);
            w64(&mut out, rd + 8, (si << 32) | R_AARCH64_COPY);
            w64(&mut out, rd + 16, 0);
            rd += 24;
        }
    }

    // .rela.plt (R_AARCH64_JUMP_SLOT = 1026)
    const R_AARCH64_JUMP_SLOT: u64 = 1026;
    let mut rp = rela_plt_offset as usize;
    let gpb = got_plt_addr + 24;
    for (i, name) in plt_names.iter().enumerate() {
        let gea = gpb + i as u64 * 8;
        let si = dyn_sym_names
            .iter()
            .position(|n| n == name)
            .map(|j| j + 1)
            .unwrap_or(0) as u64;
        w64(&mut out, rp, gea);
        w64(&mut out, rp + 8, (si << 32) | R_AARCH64_JUMP_SLOT);
        w64(&mut out, rp + 16, 0);
        rp += 24;
    }

    // Section data
    for sec in output_sections.iter() {
        if sec.sh_type == SHT_NOBITS || sec.data.is_empty() {
            continue;
        }
        write_bytes(&mut out, sec.file_offset as usize, &sec.data);
    }

    // .plt (AArch64 PLT stubs)
    if plt_size > 0 {
        let po = plt_offset as usize;
        // PLT header (32 bytes):
        // stp x16, x30, [sp, #-16]!   ; save registers
        // adrp x16, GOT+16            ; load page of GOT[2]
        // ldr x17, [x16, #lo12(GOT+16)] ; load GOT[2] (resolver)
        // add x16, x16, #lo12(GOT+16)   ; compute address
        // br x17                        ; jump to resolver
        // nop; nop; nop                  ; padding to 32 bytes

        let got2_addr = got_plt_addr + 16;
        let page_g = got2_addr & !0xFFF;
        let page_p = plt_addr & !0xFFF;
        let page_diff = ((page_g as i64 - page_p as i64) >> 12) as i32;
        let immlo = (page_diff & 3) as u32;
        let immhi = ((page_diff >> 2) & 0x7ffff) as u32;

        // STP x16, x30, [sp, #-16]!
        w32(&mut out, po, 0xa9bf7bf0);
        // ADRP x16, page_of(GOT+16)
        w32(&mut out, po + 4, 0x90000010 | (immlo << 29) | (immhi << 5));
        // LDR x17, [x16, #lo12(GOT+16)]
        let lo12 = (got2_addr & 0xFFF) as u32;
        w32(&mut out, po + 8, 0xf9400211 | ((lo12 / 8) << 10));
        // ADD x16, x16, #lo12(GOT+16)
        w32(&mut out, po + 12, 0x91000210 | ((lo12 & 0xFFF) << 10));
        // BR x17
        w32(&mut out, po + 16, 0xd61f0220);
        // NOP padding
        w32(&mut out, po + 20, 0xd503201f);
        w32(&mut out, po + 24, 0xd503201f);
        w32(&mut out, po + 28, 0xd503201f);

        // Individual PLT entries (16 bytes each):
        // adrp x16, GOT_entry_page
        // ldr x17, [x16, #lo12(GOT_entry)]
        // add x16, x16, #lo12(GOT_entry)
        // br x17
        for (i, _) in plt_names.iter().enumerate() {
            let ep = po + 32 + i * 16;
            let pea = plt_addr + 32 + i as u64 * 16;
            let gea = got_plt_addr + 24 + i as u64 * 8;

            let page_g = gea & !0xFFF;
            let page_p = pea & !0xFFF;
            let page_diff = ((page_g as i64 - page_p as i64) >> 12) as i32;
            let immlo = (page_diff & 3) as u32;
            let immhi = ((page_diff >> 2) & 0x7ffff) as u32;

            // ADRP x16, page_of(GOT entry)
            w32(&mut out, ep, 0x90000010 | (immlo << 29) | (immhi << 5));
            // LDR x17, [x16, #lo12(GOT entry)]
            let lo12 = (gea & 0xFFF) as u32;
            w32(&mut out, ep + 4, 0xf9400211 | ((lo12 / 8) << 10));
            // ADD x16, x16, #lo12(GOT entry)
            w32(&mut out, ep + 8, 0x91000210 | ((lo12 & 0xFFF) << 10));
            // BR x17
            w32(&mut out, ep + 12, 0xd61f0220);
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
    for &(tag, val) in &[
        (DT_STRTAB, dynstr_addr),
        (DT_SYMTAB, dynsym_addr),
        (DT_STRSZ, dynstr_size),
        (DT_SYMENT, 24),
        (DT_DEBUG, 0),
        (DT_PLTGOT, got_plt_addr),
        (DT_PLTRELSZ, rela_plt_size),
        (DT_PLTREL, 7u64),
        (DT_JMPREL, rela_plt_addr),
        (DT_RELA, rela_dyn_addr),
        (DT_RELASZ, rela_dyn_size),
        (DT_RELAENT, 24),
        (DT_GNU_HASH, gnu_hash_addr),
        (DT_FLAGS, DF_BIND_NOW as u64),
        (DT_FLAGS_1, DF_1_NOW as u64),
    ] {
        w64(&mut out, dd, tag as u64);
        w64(&mut out, dd + 8, val);
        dd += 16;
    }
    if has_init_array {
        w64(&mut out, dd, DT_INIT_ARRAY as u64);
        w64(&mut out, dd + 8, init_array_addr);
        dd += 16;
        w64(&mut out, dd, DT_INIT_ARRAYSZ as u64);
        w64(&mut out, dd + 8, init_array_size);
        dd += 16;
    }
    if has_fini_array {
        w64(&mut out, dd, DT_FINI_ARRAY as u64);
        w64(&mut out, dd + 8, fini_array_addr);
        dd += 16;
        w64(&mut out, dd, DT_FINI_ARRAYSZ as u64);
        w64(&mut out, dd + 8, fini_array_size);
        dd += 16;
    }
    w64(&mut out, dd, DT_NULL as u64);
    w64(&mut out, dd + 8, 0);

    // .got (GLOB_DAT entries)
    let mut go = got_offset as usize;
    for (name, is_plt) in got_entries {
        if name.is_empty() || *is_plt {
            continue;
        }
        if let Some(gsym) = globals.get(name) {
            if gsym.defined_in.is_some() && !gsym.is_dynamic {
                let sym_val = gsym.value;
                if has_tls && (gsym.info & 0xf) == STT_TLS {
                    let tpoff = (sym_val as i64 - tls_addr as i64) + 16;
                    w64(&mut out, go, tpoff as u64);
                } else {
                    w64(&mut out, go, sym_val);
                }
            } else if gsym.copy_reloc && gsym.value != 0 {
                w64(&mut out, go, gsym.value);
            }
        }
        go += 8;
    }

    // .got.plt
    let gp = got_plt_offset as usize;
    w64(&mut out, gp, dynamic_addr);
    w64(&mut out, gp + 8, 0);
    w64(&mut out, gp + 16, 0);
    for (i, _) in plt_names.iter().enumerate() {
        // Initialize GOT.plt entries to PLT[0] (resolved eagerly via DF_BIND_NOW)
        w64(&mut out, gp + 24 + i * 8, plt_addr);
    }

    // Apply relocations
    let globals_snap: HashMap<String, GlobalSymbol> = globals.clone();

    // Build GotInfo for GOT-only entries (non-PLT symbols accessed via ADR_GOT_PAGE/LD64_GOT_LO12_NC)
    let mut dyn_got_entries: HashMap<String, usize> = HashMap::new();
    {
        let mut got_only_idx = 0usize;
        for (name, is_plt) in got_entries.iter() {
            if name.is_empty() || *is_plt {
                continue;
            }
            // got_key for global symbols is just the name
            dyn_got_entries.insert(name.clone(), got_only_idx);
            got_only_idx += 1;
        }
    }
    let dyn_got_info = reloc::GotInfo {
        got_addr,
        entries: dyn_got_entries,
    };

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
                let s = resolve_sym_dynamic(
                    obj_idx,
                    sym,
                    &globals_snap,
                    section_map,
                    output_sections,
                    plt_addr,
                );

                match rela.rela_type {
                    R_AARCH64_ABS64 => {
                        let t = if !sym.name.is_empty() {
                            if let Some(g) = globals_snap.get(&sym.name) {
                                if g.is_dynamic && !g.copy_reloc {
                                    if let Some(pi) = g.plt_idx {
                                        plt_addr + 32 + pi as u64 * 16
                                    } else {
                                        s
                                    }
                                } else {
                                    s
                                }
                            } else {
                                s
                            }
                        } else {
                            s
                        };
                        w64(&mut out, fp, (t as i64 + a) as u64);
                    }
                    R_AARCH64_ABS32 => {
                        w32(&mut out, fp, (s as i64 + a) as u32);
                    }
                    R_AARCH64_ABS16 => {
                        w16(&mut out, fp, (s as i64 + a) as u16);
                    }
                    R_AARCH64_PREL64 => {
                        w64(&mut out, fp, (s as i64 + a - p as i64) as u64);
                    }
                    R_AARCH64_PREL32 => {
                        w32(&mut out, fp, (s as i64 + a - p as i64) as u32);
                    }
                    R_AARCH64_PREL16 => {
                        w16(&mut out, fp, (s as i64 + a - p as i64) as u16);
                    }
                    R_AARCH64_ADR_PREL_PG_HI21 => {
                        let sa_val = (s as i64 + a) as u64;
                        let page_s = sa_val & !0xFFF;
                        let page_p = p & !0xFFF;
                        let imm = (page_s as i64 - page_p as i64) >> 12;
                        reloc::encode_adrp(&mut out, fp, imm);
                    }
                    R_AARCH64_ADR_PREL_LO21 => {
                        let offset_val = (s as i64 + a) - (p as i64);
                        reloc::encode_adr(&mut out, fp, offset_val);
                    }
                    R_AARCH64_ADD_ABS_LO12_NC => {
                        let sa_val = (s as i64 + a) as u64;
                        reloc::encode_add_imm12(&mut out, fp, (sa_val & 0xFFF) as u32);
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
                    R_AARCH64_CALL26 | R_AARCH64_JUMP26 => {
                        if fp + 4 > out.len() {
                            continue;
                        }
                        let t = if !sym.name.is_empty() {
                            if let Some(g) = globals_snap.get(&sym.name) {
                                if let Some(pi) = g.plt_idx {
                                    plt_addr + 32 + pi as u64 * 16
                                } else {
                                    s
                                }
                            } else {
                                s
                            }
                        } else {
                            s
                        };
                        let sa_val = (t as i64 + a) as u64;
                        if sa_val == 0 {
                            w32(&mut out, fp, 0xd503201f); // NOP for weak undef
                        } else {
                            let offset_val = (sa_val as i64) - (p as i64);
                            let mut insn = read_u32(&out, fp);
                            let imm26 = ((offset_val >> 2) as u32) & 0x3ffffff;
                            insn = (insn & 0xfc000000) | imm26;
                            w32(&mut out, fp, insn);
                        }
                    }
                    R_AARCH64_CONDBR19 => {
                        let offset_val = (s as i64 + a) - (p as i64);
                        if fp + 4 > out.len() {
                            continue;
                        }
                        let mut insn = read_u32(&out, fp);
                        let imm19 = ((offset_val >> 2) as u32) & 0x7ffff;
                        insn = (insn & 0xff00001f) | (imm19 << 5);
                        w32(&mut out, fp, insn);
                    }
                    R_AARCH64_TSTBR14 => {
                        let offset_val = (s as i64 + a) - (p as i64);
                        if fp + 4 > out.len() {
                            continue;
                        }
                        let mut insn = read_u32(&out, fp);
                        let imm14 = ((offset_val >> 2) as u32) & 0x3fff;
                        insn = (insn & 0xfff8001f) | (imm14 << 5);
                        w32(&mut out, fp, insn);
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
                    R_AARCH64_ADR_GOT_PAGE | R_AARCH64_LD64_GOT_LO12_NC => {
                        // Use GOT entry address for symbols with GOT-only entries
                        let gkey = sym.name.clone();
                        let tls_info = reloc::TlsInfo {
                            tls_addr,
                            tls_size: tls_mem_size,
                        };
                        reloc::apply_one_reloc(
                            &mut out,
                            fp,
                            rela.rela_type,
                            s,
                            a,
                            p,
                            &sym.name,
                            &objects[obj_idx].source_name,
                            &tls_info,
                            &dyn_got_info,
                            &gkey,
                        )?;
                    }
                    _ => {
                        // Delegate to the standard reloc handler for TLS etc.
                        let gkey = reloc::got_key(obj_idx, sym);
                        let tls_info = reloc::TlsInfo {
                            tls_addr,
                            tls_size: tls_mem_size,
                        };
                        reloc::apply_one_reloc(
                            &mut out,
                            fp,
                            rela.rela_type,
                            s,
                            a,
                            p,
                            &sym.name,
                            &objects[obj_idx].source_name,
                            &tls_info,
                            &dyn_got_info,
                            &gkey,
                        )?;
                    }
                }
            }
        }
    }

    // Write output
    std::fs::write(output_path, &out)
        .map_err(|e| format!("failed to write '{}': {}", output_path, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(output_path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// Resolve a symbol address for dynamic linking. Dynamic symbols go through PLT.
fn resolve_sym_dynamic(
    obj_idx: usize,
    sym: &Symbol,
    globals: &HashMap<String, GlobalSymbol>,
    section_map: &HashMap<(usize, usize), (usize, u64)>,
    output_sections: &[OutputSection],
    plt_addr: u64,
) -> u64 {
    if sym.sym_type() == STT_SECTION {
        let si = sym.shndx as usize;
        return section_map
            .get(&(obj_idx, si))
            .map(|&(oi, so)| output_sections[oi].addr + so)
            .unwrap_or(0);
    }
    if !sym.name.is_empty() && !sym.is_local() {
        if let Some(g) = globals.get(&sym.name) {
            if g.defined_in.is_some() {
                return g.value;
            }
            if g.is_dynamic {
                if let Some(pi) = g.plt_idx {
                    return plt_addr + 32 + pi as u64 * 16;
                }
                if g.copy_reloc {
                    return g.value;
                }
            }
        }
        if sym.is_weak() {
            return 0;
        }
    }
    if sym.is_undefined() {
        return 0;
    }
    if sym.shndx == SHN_ABS {
        return sym.value;
    }
    section_map
        .get(&(obj_idx, sym.shndx as usize))
        .map(|&(oi, so)| output_sections[oi].addr + so + sym.value)
        .unwrap_or(sym.value)
}
