//! Static executable emission for the AArch64 linker.
//!
//! Emits a statically-linked ELF64 executable with two LOAD segments (RX+RW),
//! GOT for position-dependent code, IPLT/IRELATIVE for ifuncs, and TLS support.

use std::collections::HashMap;

use super::elf::*;
use super::reloc;
use super::types::{GlobalSymbol, BASE_ADDR, PAGE_SIZE};
use crate::backend::linker_common;
use linker_common::OutputSection;

// ── Static ELF emission ───────────────────────────────────────────────

pub(super) fn emit_executable(
    objects: &[ElfObject],
    globals: &mut HashMap<String, GlobalSymbol>,
    output_sections: &mut [OutputSection],
    section_map: &HashMap<(usize, usize), (usize, u64)>,
    output_path: &str,
) -> Result<(), String> {
    if std::env::var("LINKER_DEBUG").is_ok() {
        eprintln!("output sections:");
        for (i, sec) in output_sections.iter().enumerate() {
            eprintln!(
                "  [{}]: {} type={} flags=0x{:x} size={} align={}",
                i, sec.name, sec.sh_type, sec.flags, sec.mem_size, sec.alignment
            );
        }
    }

    // Layout: Single RX LOAD segment from file offset 0 (ELF hdr + phdrs + text + rodata),
    // followed by a RW LOAD segment for data + bss, plus TLS and GNU_STACK phdrs.
    let has_tls = output_sections
        .iter()
        .any(|s| s.flags & SHF_TLS != 0 && s.mem_size > 0);
    let phdr_count: u64 = 2 + if has_tls { 1 } else { 0 } + 1 + 1; // 2 LOAD + optional TLS + GNU_STACK + GNU_EH_FRAME
    let phdr_total_size = phdr_count * 56;
    let debug_layout = std::env::var("LINKER_DEBUG_LAYOUT").is_ok();

    // === Layout: RX segment (starts at file offset 0, vaddr BASE_ADDR) ===
    let mut offset = 64 + phdr_total_size; // After ELF header + phdrs

    // Text sections (executable)
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_EXECINSTR != 0 && sec.flags & SHF_ALLOC != 0 {
            let a = sec.alignment.max(4);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = BASE_ADDR + offset;
            sec.file_offset = offset;
            offset += sec.mem_size;
        }
    }

    // Rodata sections (read-only, in same RX segment)
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
            if debug_layout {
                eprintln!(
                    "  LAYOUT RO: {} addr=0x{:x} foff=0x{:x} sz=0x{:x} flags=0x{:x}",
                    sec.name, sec.addr, sec.file_offset, sec.mem_size, sec.flags
                );
            }
            offset += sec.mem_size;
        }
    }

    // Build .eh_frame_hdr: find .eh_frame, count FDEs, reserve space
    let mut eh_frame_hdr_vaddr = 0u64;
    let mut eh_frame_hdr_offset = 0u64;
    let mut eh_frame_hdr_size = 0u64;
    let mut eh_frame_vaddr = 0u64;
    let mut eh_frame_file_offset = 0u64;
    let mut eh_frame_size = 0u64;
    for sec in output_sections.iter() {
        if sec.name == ".eh_frame" && sec.mem_size > 0 {
            eh_frame_vaddr = sec.addr;
            eh_frame_file_offset = sec.file_offset;
            eh_frame_size = sec.mem_size;
            break;
        }
    }
    if eh_frame_size > 0 {
        // Count FDEs from individual input .eh_frame sections (data not merged yet)
        let mut fde_count = 0usize;
        if let Some(ef_sec) = output_sections
            .iter()
            .find(|s| s.name == ".eh_frame" && s.mem_size > 0)
        {
            for input in &ef_sec.inputs {
                let sd = &objects[input.object_idx].section_data[input.section_idx];
                fde_count += crate::backend::linker_common::count_eh_frame_fdes(sd);
            }
        }
        eh_frame_hdr_size = (12 + 8 * fde_count) as u64;
        // Align to 4 bytes
        offset = (offset + 3) & !3;
        eh_frame_hdr_offset = offset;
        eh_frame_hdr_vaddr = BASE_ADDR + offset;
        offset += eh_frame_hdr_size;
        if debug_layout {
            eprintln!(
                "  LAYOUT EH_FRAME_HDR: addr=0x{:x} foff=0x{:x} sz=0x{:x} fde_count={}",
                eh_frame_hdr_vaddr, eh_frame_hdr_offset, eh_frame_hdr_size, fde_count
            );
        }
    }

    let rx_filesz = offset; // RX segment: [0, rx_filesz)
    let _rx_memsz = rx_filesz;

    // Pre-count IFUNC symbols so we can reserve space for IPLT stubs in the RX gap.
    // Each IPLT stub is 16 bytes (ADRP + LDR + BR + NOP), placed in the gap between
    // the RX segment end and the page-aligned RW segment start.
    let pre_iplt_count = globals
        .iter()
        .filter(|(_, gsym)| gsym.info & 0xf == STT_GNU_IFUNC && gsym.defined_in.is_some())
        .count() as u64;
    let iplt_stubs_needed = pre_iplt_count * 16;
    if iplt_stubs_needed > 0 {
        // Ensure the gap after rx_filesz is large enough for IPLT stubs.
        // The stubs will be placed at 16-byte aligned offset after rx_filesz.
        let stub_start = (offset + 15) & !15;
        let stub_end = stub_start + iplt_stubs_needed;
        // Make sure offset is at least stub_end so page-alignment leaves enough room
        if offset < stub_end {
            offset = stub_end;
        }
    }

    // === Layout: RW segment (page-aligned) ===
    offset = (offset + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let rw_page_offset = offset;
    let rw_page_addr = BASE_ADDR + offset;

    // TLS data (.tdata) first in RW
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
            if debug_layout {
                eprintln!(
                    "  LAYOUT TLS: {} addr=0x{:x} foff=0x{:x} sz=0x{:x} align={} flags=0x{:x}",
                    sec.name, sec.addr, sec.file_offset, sec.mem_size, a, sec.flags
                );
            }
            offset += sec.mem_size;
        }
    }
    // If only .tbss (NOBITS TLS) exists with no .tdata, we still need a TLS segment.
    // Set tls_addr/tls_file_offset to the current position so TPOFF calculations work.
    if tls_addr == 0 && has_tls {
        tls_addr = BASE_ADDR + offset;
        tls_file_offset = offset;
    }
    // TLS BSS (.tbss) - doesn't consume file space
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_TLS != 0 && sec.sh_type == SHT_NOBITS {
            let a = sec.alignment.max(1);
            let aligned = (tls_mem_size + a - 1) & !(a - 1);
            sec.addr = tls_addr + aligned;
            sec.file_offset = offset;
            if debug_layout {
                eprintln!("  LAYOUT TBSS: {} addr=0x{:x} aligned_off=0x{:x} sz=0x{:x} align={} tls_mem_size=0x{:x}",
                    sec.name, sec.addr, aligned, sec.mem_size, a, tls_mem_size);
            }
            tls_mem_size = aligned + sec.mem_size;
            if a > tls_align {
                tls_align = a;
            }
        }
    }
    if tls_mem_size > 0 {
        tls_mem_size = (tls_mem_size + tls_align - 1) & !(tls_align - 1);
    }

    // init_array
    for sec in output_sections.iter_mut() {
        if sec.name == ".init_array" {
            let a = sec.alignment.max(8);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = BASE_ADDR + offset;
            sec.file_offset = offset;
            offset += sec.mem_size;
            break;
        }
    }
    // fini_array
    for sec in output_sections.iter_mut() {
        if sec.name == ".fini_array" {
            let a = sec.alignment.max(8);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = BASE_ADDR + offset;
            sec.file_offset = offset;
            offset += sec.mem_size;
            break;
        }
    }

    // .data.rel.ro (relocated read-only data) - must come before .data
    for sec in output_sections.iter_mut() {
        if sec.name == ".data.rel.ro" {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = BASE_ADDR + offset;
            sec.file_offset = offset;
            if debug_layout {
                eprintln!(
                    "  LAYOUT RW: {} addr=0x{:x} foff=0x{:x} sz=0x{:x} flags=0x{:x}",
                    sec.name, sec.addr, sec.file_offset, sec.mem_size, sec.flags
                );
            }
            offset += sec.mem_size;
        }
    }
    // Remaining data sections (writable, non-BSS, non-TLS)
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
            if debug_layout {
                eprintln!(
                    "  LAYOUT RW: {} addr=0x{:x} foff=0x{:x} sz=0x{:x} flags=0x{:x}",
                    sec.name, sec.addr, sec.file_offset, sec.mem_size, sec.flags
                );
            }
            offset += sec.mem_size;
        }
    }

    // GOT (Global Offset Table) - needed for R_AARCH64_ADR_GOT_PAGE / LD64_GOT_LO12_NC
    // and TLS IE relocations (which store TP offsets in GOT entries)
    let got_syms = reloc::collect_got_symbols(objects);
    let got_size = got_syms.len() as u64 * 8;
    offset = (offset + 7) & !7; // 8-byte align
    let got_offset = offset;
    let got_addr = BASE_ADDR + offset;
    let mut got_entries = HashMap::new();
    for (idx, (key, _kind)) in got_syms.iter().enumerate() {
        got_entries.insert(key.clone(), idx);
    }
    offset += got_size;

    // Collect IFUNC symbols before address resolution - we need them for layout.
    // Identify them by STT_GNU_IFUNC type in the symbol table.
    let mut ifunc_names: Vec<String> = Vec::new();
    for (name, gsym) in globals.iter() {
        if gsym.info & 0xf == STT_GNU_IFUNC && gsym.defined_in.is_some() {
            ifunc_names.push(name.clone());
        }
    }
    ifunc_names.sort(); // deterministic order

    // IPLT GOT slots for IFUNC symbols (one 8-byte slot per IFUNC)
    let iplt_got_count = ifunc_names.len();
    let iplt_got_size = iplt_got_count as u64 * 8;
    offset = (offset + 7) & !7;
    let iplt_got_offset = offset;
    let iplt_got_addr = BASE_ADDR + offset;
    offset += iplt_got_size;

    // IRELATIVE relocation entries (.rela.iplt) in the RW segment
    // Format: Elf64_Rela { r_offset: u64, r_info: u64, r_addend: i64 } = 24 bytes each
    let rela_iplt_size = iplt_got_count as u64 * 24;
    offset = (offset + 7) & !7;
    let rela_iplt_offset = offset;
    let rela_iplt_addr = BASE_ADDR + offset;
    let rela_iplt_end_addr = rela_iplt_addr + rela_iplt_size;
    offset += rela_iplt_size;

    let rw_filesz = offset - rw_page_offset;

    // BSS (nobits, non-TLS)
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
    let rw_memsz = if bss_size > 0 {
        (bss_addr + bss_size) - rw_page_addr
    } else {
        rw_filesz
    };

    // IPLT stubs go in the RX padding between rx_filesz and rw_page_offset.
    // Each stub: 16 bytes (ADRP + LDR + BR + NOP)
    let iplt_stub_size = iplt_got_count as u64 * 16;
    let iplt_stub_file_off = (rx_filesz + 15) & !15; // 16-byte aligned
    let iplt_stub_addr = BASE_ADDR + iplt_stub_file_off;
    if iplt_stub_size > 0 && iplt_stub_file_off + iplt_stub_size > rw_page_offset {
        return Err(format!(
            "IPLT stubs ({} bytes) don't fit in RX padding (gap={})",
            iplt_stub_size,
            rw_page_offset - iplt_stub_file_off
        ));
    }

    if std::env::var("LINKER_DEBUG").is_ok() {
        eprintln!("section layout:");
        for sec in output_sections.iter() {
            eprintln!(
                "  {} addr=0x{:x} foff=0x{:x} size=0x{:x}",
                sec.name, sec.addr, sec.file_offset, sec.mem_size
            );
        }
        eprintln!(
            "  GOT addr=0x{:x} foff=0x{:x} size=0x{:x} entries={}",
            got_addr,
            got_offset,
            got_size,
            got_entries.len()
        );
        if iplt_got_count > 0 {
            eprintln!(
                "  IPLT GOT addr=0x{:x} entries={}",
                iplt_got_addr, iplt_got_count
            );
            eprintln!(
                "  RELA.IPLT addr=0x{:x}..0x{:x}",
                rela_iplt_addr, rela_iplt_end_addr
            );
            eprintln!("  IPLT stubs addr=0x{:x}", iplt_stub_addr);
        }
        eprintln!("  BSS addr=0x{:x} size=0x{:x}", bss_addr, bss_size);
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

    // Update global symbol addresses
    for (name, gsym) in globals.iter_mut() {
        if let Some(obj_idx) = gsym.defined_in {
            if obj_idx == usize::MAX {
                continue;
            } // linker-defined
            if gsym.section_idx == SHN_COMMON || gsym.section_idx == 0xffff {
                if let Some(bss_sec) = output_sections.iter().find(|s| s.name == ".bss") {
                    gsym.value += bss_sec.addr;
                }
            } else if gsym.section_idx != SHN_UNDEF && gsym.section_idx != SHN_ABS {
                let si = gsym.section_idx as usize;
                if let Some(&(oi, so)) = section_map.get(&(obj_idx, si)) {
                    let old_val = gsym.value;
                    gsym.value += output_sections[oi].addr + so;
                    if std::env::var("LINKER_DEBUG").is_ok() && gsym.info & 0xf == STT_TLS {
                        eprintln!("  TLS sym '{}': old=0x{:x} -> new=0x{:x} (sec={} addr=0x{:x} off=0x{:x})",
                                  name, old_val, gsym.value, output_sections[oi].name, output_sections[oi].addr, so);
                    }
                } else if std::env::var("LINKER_DEBUG").is_ok() && gsym.info & 0xf == STT_TLS {
                    eprintln!("  TLS sym '{}': NO MAPPING for ({}, {})", name, obj_idx, si);
                }
            }
        }
    }

    // Build IFUNC resolver address map (now that addresses are resolved)
    let ifunc_syms: Vec<(String, u64)> = ifunc_names
        .iter()
        .map(|name| {
            let resolver_addr = globals.get(name).map(|g| g.value).unwrap_or(0);
            (name.clone(), resolver_addr)
        })
        .collect();

    // Redirect IFUNC symbols to their PLT stubs
    for (i, (name, _resolver_addr)) in ifunc_syms.iter().enumerate() {
        let plt_addr = iplt_stub_addr + i as u64 * 16;
        if let Some(gsym) = globals.get_mut(name) {
            gsym.value = plt_addr;
            // Change type from IFUNC to FUNC so relocations treat it normally
            gsym.info = (gsym.info & 0xf0) | STT_FUNC;
        }
    }

    if std::env::var("LINKER_DEBUG").is_ok() && !ifunc_syms.is_empty() {
        for (i, (name, resolver)) in ifunc_syms.iter().enumerate() {
            eprintln!(
                "  IFUNC[{}]: {} resolver=0x{:x} plt=0x{:x} got=0x{:x}",
                i,
                name,
                resolver,
                iplt_stub_addr + i as u64 * 16,
                iplt_got_addr + i as u64 * 8
            );
        }
    }

    // Compute init/fini array boundaries
    let init_array_start = output_sections
        .iter()
        .find(|s| s.name == ".init_array")
        .map(|s| s.addr)
        .unwrap_or(0);
    let init_array_end = output_sections
        .iter()
        .find(|s| s.name == ".init_array")
        .map(|s| s.addr + s.mem_size)
        .unwrap_or(0);
    let fini_array_start = output_sections
        .iter()
        .find(|s| s.name == ".fini_array")
        .map(|s| s.addr)
        .unwrap_or(0);
    let fini_array_end = output_sections
        .iter()
        .find(|s| s.name == ".fini_array")
        .map(|s| s.addr + s.mem_size)
        .unwrap_or(0);
    let init_addr = output_sections
        .iter()
        .find(|s| s.name == ".init")
        .map(|s| s.addr)
        .unwrap_or(0);
    let fini_addr = output_sections
        .iter()
        .find(|s| s.name == ".fini")
        .map(|s| s.addr)
        .unwrap_or(0);

    // Define linker-provided symbols using shared infrastructure (consistent
    // with x86-64/i686/RISC-V via get_standard_linker_symbols)
    let text_seg_end = BASE_ADDR + rx_filesz;
    let linker_addrs = LinkerSymbolAddresses {
        base_addr: BASE_ADDR,
        got_addr,
        dynamic_addr: 0, // No .dynamic in static mode (dynamic executables use emit_dynamic_executable)
        bss_addr,
        bss_size,
        text_end: text_seg_end,
        data_start: rw_page_addr,
        init_array_start,
        init_array_size: init_array_end - init_array_start,
        fini_array_start,
        fini_array_size: fini_array_end - fini_array_start,
        preinit_array_start: 0,
        preinit_array_size: 0,
        rela_iplt_start: rela_iplt_addr,
        rela_iplt_size,
    };
    for sym in &get_standard_linker_symbols(&linker_addrs) {
        if globals
            .get(sym.name)
            .map(|g| g.defined_in.is_none())
            .unwrap_or(true)
        {
            globals.insert(
                sym.name.to_string(),
                GlobalSymbol {
                    value: sym.value,
                    size: 0,
                    info: (sym.binding << 4) | STT_OBJECT,
                    defined_in: Some(usize::MAX),
                    section_idx: SHN_ABS,
                    from_lib: None,
                    plt_idx: None,
                    got_idx: None,
                    is_dynamic: false,
                    copy_reloc: false,
                    lib_sym_value: 0,
                },
            );
        }
    }

    // Auto-generate __start_<section> / __stop_<section> symbols (GNU ld feature)
    for (name, addr) in linker_common::resolve_start_stop_symbols(output_sections) {
        if globals
            .get(&name)
            .map(|g| g.defined_in.is_none())
            .unwrap_or(false)
        {
            globals.insert(
                name,
                GlobalSymbol {
                    value: addr,
                    size: 0,
                    info: (STB_GLOBAL << 4),
                    defined_in: Some(usize::MAX),
                    section_idx: SHN_ABS,
                    from_lib: None,
                    plt_idx: None,
                    got_idx: None,
                    is_dynamic: false,
                    copy_reloc: false,
                    lib_sym_value: 0,
                },
            );
        }
    }
    // ARM-specific linker symbols not in the shared list
    let arm_extra_syms: [(&str, u64); 3] = [
        ("__GNU_EH_FRAME_HDR", eh_frame_hdr_vaddr),
        ("_init", init_addr),
        ("_fini", fini_addr),
    ];
    for (name, val) in &arm_extra_syms {
        if globals
            .get(*name)
            .map(|g| g.defined_in.is_none())
            .unwrap_or(true)
        {
            globals.insert(
                name.to_string(),
                GlobalSymbol {
                    value: *val,
                    size: 0,
                    info: (STB_GLOBAL << 4) | STT_OBJECT,
                    defined_in: Some(usize::MAX),
                    section_idx: SHN_ABS,
                    from_lib: None,
                    plt_idx: None,
                    got_idx: None,
                    is_dynamic: false,
                    copy_reloc: false,
                    lib_sym_value: 0,
                },
            );
        }
    }

    let entry_addr = globals.get("_start").map(|s| s.value).unwrap_or(BASE_ADDR);

    if std::env::var("LINKER_DEBUG").is_ok() {
        if let Some(g) = globals.get("main") {
            eprintln!("  main resolved to 0x{:x}", g.value);
        }
        if let Some(g) = globals.get("_start") {
            eprintln!("  _start resolved to 0x{:x}", g.value);
        }
        if let Some(g) = globals.get("__libc_start_main") {
            eprintln!("  __libc_start_main resolved to 0x{:x}", g.value);
        }
        eprintln!("  entry_addr = 0x{:x}", entry_addr);
    }

    // Build output buffer
    let file_size = offset as usize;
    let mut out = vec![0u8; file_size];

    // Write section data
    for sec in output_sections.iter() {
        if sec.sh_type == SHT_NOBITS || sec.data.is_empty() {
            continue;
        }
        write_bytes(&mut out, sec.file_offset as usize, &sec.data);
    }

    // Populate GOT entries with resolved symbol addresses or TP offsets.
    // We re-walk the relocations to resolve each symbol (including locals)
    // rather than only looking up global symbol names.
    let globals_snap = globals.clone();
    let got_info = reloc::GotInfo {
        got_addr,
        entries: got_entries,
    };
    let got_kind_map: HashMap<String, reloc::GotEntryKind> = got_syms
        .iter()
        .map(|(k, kind)| (k.clone(), *kind))
        .collect();
    // Build a resolved address map for GOT entries by walking relocations
    let mut got_resolved: HashMap<String, u64> = HashMap::new();
    for obj_idx in 0..objects.len() {
        for sec_idx in 0..objects[obj_idx].sections.len() {
            for rela in &objects[obj_idx].relocations[sec_idx] {
                match rela.rela_type {
                    R_AARCH64_ADR_GOT_PAGE
                    | R_AARCH64_LD64_GOT_LO12_NC
                    | reloc::R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21
                    | reloc::R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC => {
                        let si = rela.sym_idx as usize;
                        if si < objects[obj_idx].symbols.len() {
                            let sym = &objects[obj_idx].symbols[si];
                            let key = reloc::got_key(obj_idx, sym);
                            got_resolved.entry(key).or_insert_with(|| {
                                reloc::resolve_sym(
                                    obj_idx,
                                    sym,
                                    &globals_snap,
                                    section_map,
                                    output_sections,
                                )
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    for (key, &idx) in &got_info.entries {
        let sym_addr = got_resolved.get(key).copied().unwrap_or(0);
        let val = match got_kind_map.get(key) {
            Some(&reloc::GotEntryKind::TlsIE) => {
                // GOT entry holds the TP-relative offset for this TLS variable
                // AArch64 variant 1: tp_offset = (sym_addr - tls_base) + 16
                if tls_addr != 0 {
                    let offset = (sym_addr as i64) - (tls_addr as i64) + 16;
                    if std::env::var("LINKER_DEBUG_TLS").is_ok() {
                        eprintln!("  GOT TLS IE: key='{}' sym_addr=0x{:x} tls_addr=0x{:x} -> got_val=0x{:x}",
                            key, sym_addr, tls_addr, offset as u64);
                    }
                    offset as u64
                } else {
                    sym_addr
                }
            }
            _ => sym_addr,
        };
        let entry_off = got_offset as usize + idx * 8;
        w64(&mut out, entry_off, val);
        if std::env::var("LINKER_DEBUG").is_ok() && val == 0 {
            eprintln!("  GOT[{}] = 0 for symbol '{}'", idx, key);
        }
    }

    // Populate IPLT GOT slots (initially with resolver addresses), RELA entries, and PLT stubs
    for (i, (_name, resolver_addr)) in ifunc_syms.iter().enumerate() {
        // IPLT GOT slot: initially contains resolver address (will be overwritten at startup)
        let got_slot_off = iplt_got_offset as usize + i * 8;
        if got_slot_off + 8 <= out.len() {
            w64(&mut out, got_slot_off, *resolver_addr);
        }

        // RELA.IPLT entry: { r_offset, r_info, r_addend }
        let rela_off = rela_iplt_offset as usize + i * 24;
        let got_slot_addr = iplt_got_addr + i as u64 * 8;
        if rela_off + 24 <= out.len() {
            w64(&mut out, rela_off, got_slot_addr); // r_offset: GOT slot VA
            w64(&mut out, rela_off + 8, 0x408); // r_info: R_AARCH64_IRELATIVE
            w64(&mut out, rela_off + 16, *resolver_addr); // r_addend: resolver VA
        }

        // PLT stub: ADRP x16, got_page; LDR x17, [x16, #got_lo]; BR x17; NOP
        let plt_off = iplt_stub_file_off as usize + i * 16;
        let plt_addr = iplt_stub_addr + i as u64 * 16;
        if plt_off + 16 <= out.len() {
            // ADRP x16, page_of(got_slot)
            let page_g = got_slot_addr & !0xFFF;
            let page_p = plt_addr & !0xFFF;
            let page_diff = ((page_g as i64 - page_p as i64) >> 12) as i32;
            let immlo = (page_diff & 3) as u32;
            let immhi = ((page_diff >> 2) & 0x7ffff) as u32;
            let adrp = 0x90000010u32 | (immlo << 29) | (immhi << 5); // ADRP x16
            w32(&mut out, plt_off, adrp);

            // LDR x17, [x16, #lo12(got_slot)]
            let lo12 = (got_slot_addr & 0xFFF) as u32;
            let ldr = 0xf9400211u32 | ((lo12 / 8) << 10); // LDR x17, [x16, #imm]
            w32(&mut out, plt_off + 4, ldr);

            // BR x17
            w32(&mut out, plt_off + 8, 0xd61f0220u32);

            // NOP
            w32(&mut out, plt_off + 12, 0xd503201fu32);
        }
    }

    // Apply relocations
    let tls_info = reloc::TlsInfo {
        tls_addr,
        tls_size: tls_mem_size,
    };
    reloc::apply_relocations(
        objects,
        &globals_snap,
        output_sections,
        section_map,
        &mut out,
        &tls_info,
        &got_info,
    )?;

    // Build .eh_frame_hdr from relocated .eh_frame data and write it
    if eh_frame_hdr_size > 0 && eh_frame_size > 0 {
        let ef_start = eh_frame_file_offset as usize;
        let ef_end = ef_start + eh_frame_size as usize;
        if ef_end <= out.len() {
            let eh_frame_relocated = out[ef_start..ef_end].to_vec();
            let hdr_data = crate::backend::linker_common::build_eh_frame_hdr(
                &eh_frame_relocated,
                eh_frame_vaddr,
                eh_frame_hdr_vaddr,
                true, // 64-bit
            );
            let hdr_off = eh_frame_hdr_offset as usize;
            if !hdr_data.is_empty() && hdr_off + hdr_data.len() <= out.len() {
                write_bytes(&mut out, hdr_off, &hdr_data);
            }
        }
    }

    // === ELF header ===
    out[0..4].copy_from_slice(&ELF_MAGIC);
    out[4] = ELFCLASS64;
    out[5] = ELFDATA2LSB;
    out[6] = 1;
    out[7] = 3; // ELFOSABI_GNU (matches ld output for static exes)
    w16(&mut out, 16, ET_EXEC);
    w16(&mut out, 18, EM_AARCH64);
    w32(&mut out, 20, 1);
    w64(&mut out, 24, entry_addr);
    w64(&mut out, 32, 64); // e_phoff
    w64(&mut out, 40, 0); // e_shoff
    w32(&mut out, 48, 0); // e_flags
    w16(&mut out, 52, 64); // e_ehsize
    w16(&mut out, 54, 56); // e_phentsize
    w16(&mut out, 56, phdr_count as u16);
    w16(&mut out, 58, 64); // e_shentsize
    w16(&mut out, 60, 0); // e_shnum
    w16(&mut out, 62, 0); // e_shstrndx

    // === Program headers ===
    let mut ph = 64usize;

    // LOAD: RX segment starting from file offset 0 (includes ELF header + PLT stubs)
    let rx_actual_filesz = if iplt_stub_size > 0 {
        iplt_stub_file_off + iplt_stub_size
    } else {
        rx_filesz
    };
    let rx_actual_memsz = rx_actual_filesz;
    wphdr(
        &mut out,
        ph,
        PT_LOAD,
        PF_R | PF_X,
        0,
        BASE_ADDR,
        rx_actual_filesz,
        rx_actual_memsz,
        PAGE_SIZE,
    );
    ph += 56;

    // LOAD: RW segment
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

    // TLS segment
    if has_tls && tls_addr != 0 {
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
        ph += 56;
    }

    // GNU_STACK
    wphdr(&mut out, ph, PT_GNU_STACK, PF_R | PF_W, 0, 0, 0, 0, 0x10);
    ph += 56;

    // PT_GNU_EH_FRAME: points to .eh_frame_hdr for stack unwinding
    wphdr(
        &mut out,
        ph,
        PT_GNU_EH_FRAME,
        PF_R,
        eh_frame_hdr_offset,
        eh_frame_hdr_vaddr,
        eh_frame_hdr_size,
        eh_frame_hdr_size,
        4,
    );

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
