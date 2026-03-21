//! Shared library (.so) emission for the i686 linker.
//!
//! Produces ELF32 shared libraries (ET_DYN) with PLT/GOT, PIC relocations,
//! `.dynamic` section, GNU hash tables, and GLIBC version tables.

use std::collections::HashMap;
use std::path::Path;

use super::emit::{build_plt, layout_custom_sections, layout_section, layout_tls};
use super::gnu_hash::build_gnu_hash_32;
use super::reloc::{resolve_got_reloc, resolve_tls_gotie, resolve_tls_ie, RelocContext};
use super::types::*;
use super::DynStrTab;
use crate::backend::linker_common;

/// Discover NEEDED shared library dependencies for a shared library build.
pub(super) fn resolve_dynamic_symbols_for_shared(
    inputs: &[InputObject],
    global_symbols: &HashMap<String, LinkerSymbol>,
    needed_sonames: &mut Vec<String>,
    lib_paths: &[String],
) {
    // Collect undefined symbol names
    let mut undefined: Vec<String> = Vec::new();
    for obj in inputs.iter() {
        for sym in &obj.symbols {
            if sym.binding == STB_LOCAL {
                continue;
            }
            if sym.section_index == SHN_UNDEF
                && !sym.name.is_empty()
                && !global_symbols
                    .get(&sym.name)
                    .map(|gs| gs.is_defined)
                    .unwrap_or(false)
                && !undefined.contains(&sym.name)
            {
                undefined.push(sym.name.clone());
            }
        }
    }
    if undefined.is_empty() {
        return;
    }

    // Search system libraries for these symbols
    let lib_names = [
        "libc.so.6",
        "libm.so.6",
        "libpthread.so.0",
        "libdl.so.2",
        "librt.so.1",
        "libgcc_s.so.1",
        "ld-linux.so.2",
    ];
    let mut libs: Vec<String> = Vec::new();
    for lib_name in &lib_names {
        for dir in lib_paths {
            let candidate = format!("{}/{}", dir, lib_name);
            if Path::new(&candidate).exists() {
                libs.push(candidate);
                break;
            }
        }
    }
    // Also check i686-specific paths
    let extra_dirs = [
        "/lib/i386-linux-gnu",
        "/usr/lib/i386-linux-gnu",
        "/lib32",
        "/usr/lib32",
    ];
    for lib_name in &lib_names {
        for dir in &extra_dirs {
            let candidate = format!("{}/{}", dir, lib_name);
            if Path::new(&candidate).exists() && !libs.contains(&candidate) {
                libs.push(candidate);
                break;
            }
        }
    }

    for lib_path in &libs {
        let data = match std::fs::read(lib_path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let soname_val = linker_common::parse_soname(&data).unwrap_or_else(|| {
            Path::new(lib_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        });
        if needed_sonames.contains(&soname_val) {
            continue;
        }
        let dyn_syms = match linker_common::parse_shared_library_symbols(&data, lib_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let provides_any = undefined
            .iter()
            .any(|name| dyn_syms.iter().any(|ds| ds.name == *name));
        if provides_any {
            needed_sonames.push(soname_val);
        }
    }
}

/// Emit an ELF32 shared library (.so) file.
///
/// Key differences from emit_executable:
/// - ELF type is ET_DYN (not ET_EXEC)
/// - Base address is 0 (position-independent)
/// - No PT_INTERP segment
/// - All defined global symbols exported to .dynsym
/// - R_386_RELATIVE relocations for internal absolute addresses
pub(super) fn emit_shared_library_32(
    inputs: &[InputObject],
    global_symbols: &mut HashMap<String, LinkerSymbol>,
    output_sections: &mut Vec<OutputSection>,
    section_name_to_idx: &HashMap<String, usize>,
    section_map: &SectionMap,
    needed_sonames: &[String],
    output_path: &str,
    soname: Option<String>,
) -> Result<(), String> {
    let base_addr: u32 = 0;

    // ── Build dynamic string table ────────────────────────────────────────
    let mut dynstr = DynStrTab::new();
    let _ = dynstr.add("");
    for lib in needed_sonames {
        dynstr.add(lib);
    }
    if let Some(ref sn) = soname {
        dynstr.add(sn);
    }

    // ── Identify PLT symbols (undefined function calls) ───────────────────
    let mut plt_names: Vec<String> = Vec::new();
    for obj in inputs.iter() {
        for sec in &obj.sections {
            for &(_, rel_type, sym_idx, _) in &sec.relocations {
                let si = sym_idx as usize;
                if si >= obj.symbols.len() {
                    continue;
                }
                let sym = &obj.symbols[si];
                if sym.name.is_empty() || sym.binding == STB_LOCAL {
                    continue;
                }
                match rel_type {
                    R_386_PC32 | R_386_PLT32 => {
                        if let Some(gs) = global_symbols.get(&sym.name) {
                            if !gs.is_defined && !plt_names.contains(&sym.name) {
                                plt_names.push(sym.name.clone());
                            }
                        } else if sym.section_index == SHN_UNDEF && !plt_names.contains(&sym.name) {
                            plt_names.push(sym.name.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Ensure PLT symbols are in global_symbols
    for name in &plt_names {
        global_symbols.entry(name.clone()).or_insert(LinkerSymbol {
            address: 0,
            size: 0,
            sym_type: STT_FUNC,
            binding: STB_GLOBAL,
            visibility: STV_DEFAULT,
            is_defined: false,
            needs_plt: true,
            needs_got: true,
            output_section: usize::MAX,
            section_offset: 0,
            plt_index: 0,
            got_index: 0,
            is_dynamic: true,
            dynlib: String::new(),
            needs_copy: false,
            copy_addr: 0,
            version: None,
            uses_textrel: false,
        });
        if let Some(gs) = global_symbols.get_mut(name) {
            gs.needs_plt = true;
            gs.is_dynamic = true;
        }
    }

    // Assign PLT indices
    for (i, name) in plt_names.iter().enumerate() {
        if let Some(gs) = global_symbols.get_mut(name) {
            gs.plt_index = i;
        }
    }
    let num_plt = plt_names.len();

    // ── Identify GOT symbols ──────────────────────────────────────────────
    let mut got_names: Vec<String> = Vec::new();
    for obj in inputs.iter() {
        for sec in &obj.sections {
            for &(_, rel_type, sym_idx, _) in &sec.relocations {
                let si = sym_idx as usize;
                if si >= obj.symbols.len() {
                    continue;
                }
                let sym = &obj.symbols[si];
                if sym.name.is_empty() {
                    continue;
                }
                match rel_type {
                    R_386_GOT32 | R_386_GOT32X | R_386_TLS_IE | R_386_TLS_GOTIE | R_386_TLS_GD => {
                        if !got_names.contains(&sym.name) && !plt_names.contains(&sym.name) {
                            got_names.push(sym.name.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Assign GOT indices (PLT symbols first, then GOT-only symbols)
    for (i, name) in got_names.iter().enumerate() {
        if let Some(gs) = global_symbols.get_mut(name) {
            gs.needs_got = true;
            gs.got_index = num_plt + i;
        }
    }
    let _num_got = num_plt + got_names.len();

    // ── Collect all exported symbols ──────────────────────────────────────
    let mut exported_names: Vec<String> = Vec::new();
    {
        let mut sorted: Vec<&String> = global_symbols.keys().collect();
        sorted.sort();
        for name in sorted {
            let gs = &global_symbols[name];
            if gs.is_defined && gs.binding != STB_LOCAL {
                exported_names.push(name.clone());
            }
        }
    }

    // Build dynsym: null entry + undefined PLT imports + defined exports
    let mut dynsym_names: Vec<String> = Vec::new();
    let mut dynsym_entries: Vec<Elf32Sym> = Vec::new();
    dynsym_entries.push(Elf32Sym {
        name: 0,
        value: 0,
        size: 0,
        info: 0,
        other: 0,
        shndx: 0,
    });

    // Undefined symbols first (PLT imports + GOT imports that are undefined)
    let mut undef_names: Vec<String> = Vec::new();
    for name in &plt_names {
        undef_names.push(name.clone());
    }
    for name in &got_names {
        if let Some(gs) = global_symbols.get(name) {
            if !gs.is_defined && !undef_names.contains(name) {
                undef_names.push(name.clone());
            }
        }
    }

    for name in &undef_names {
        let name_off = dynstr.add(name);
        let (bind, stype) = if let Some(gs) = global_symbols.get(name) {
            (
                gs.binding,
                if gs.sym_type != 0 {
                    gs.sym_type
                } else {
                    STT_FUNC
                },
            )
        } else {
            (STB_GLOBAL, STT_FUNC)
        };
        dynsym_entries.push(Elf32Sym {
            name: name_off,
            value: 0,
            size: 0,
            info: (bind << 4) | stype,
            other: 0,
            shndx: SHN_UNDEF,
        });
        dynsym_names.push(name.clone());
    }

    let gnu_hash_symoffset = dynsym_entries.len();

    // Defined/exported symbols (hashed)
    for name in &exported_names {
        if undef_names.contains(name) {
            continue;
        }
        let name_off = dynstr.add(name);
        let gs = &global_symbols[name];
        // Section index will be filled in after layout
        dynsym_entries.push(Elf32Sym {
            name: name_off,
            value: 0,
            size: gs.size,
            info: (gs.binding << 4) | gs.sym_type,
            other: 0,
            shndx: 1, // placeholder, will be fixed
        });
        dynsym_names.push(name.clone());
    }

    // Build .gnu.hash for the defined symbols
    let defined_for_hash: Vec<String> = dynsym_names[gnu_hash_symoffset - 1..].to_vec();
    let (gnu_hash_data, sorted_indices) =
        build_gnu_hash_32(&defined_for_hash, gnu_hash_symoffset as u32);

    // Reorder hashed entries
    if !sorted_indices.is_empty() {
        let hashed_start = gnu_hash_symoffset;
        let names_start = hashed_start - 1;
        let orig_entries: Vec<Elf32Sym> = (0..sorted_indices.len())
            .map(|i| dynsym_entries[hashed_start + i].clone())
            .collect();
        let orig_names: Vec<String> = (0..sorted_indices.len())
            .map(|i| dynsym_names[names_start + i].clone())
            .collect();
        for (new_pos, &orig_idx) in sorted_indices.iter().enumerate() {
            dynsym_entries[hashed_start + new_pos] = orig_entries[orig_idx].clone();
            dynsym_names[names_start + new_pos] = orig_names[orig_idx].clone();
        }
    }

    let dynsym_map: HashMap<String, usize> = dynsym_names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.clone(), i + 1))
        .collect();

    // Rebuild dynstr with soname
    let dynstr_data = dynstr.as_bytes().to_vec();

    // ── Pre-scan: count R_386_RELATIVE relocations needed ────────────────
    // In shared libraries, R_386_32 against defined symbols becomes R_386_RELATIVE
    let mut num_relative = 0usize;
    for (obj_idx, obj) in inputs.iter().enumerate() {
        for sec in &obj.sections {
            for &(_, rel_type, sym_idx, _) in &sec.relocations {
                if rel_type == R_386_32 {
                    let si = sym_idx as usize;
                    if si >= obj.symbols.len() {
                        continue;
                    }
                    let sym = &obj.symbols[si];
                    if sym.sym_type == STT_SECTION {
                        if section_map
                            .get(&(obj_idx, sym.section_index as usize))
                            .is_some()
                        {
                            num_relative += 1;
                        }
                    } else if !sym.name.is_empty() {
                        let is_defined = global_symbols
                            .get(&sym.name)
                            .map(|gs| gs.is_defined)
                            .unwrap_or(false);
                        if is_defined {
                            num_relative += 1;
                        }
                    }
                }
            }
        }
    }

    // GOT entries for undefined symbols need GLOB_DAT
    let mut num_glob_dat = 0usize;
    for name in &got_names {
        if let Some(gs) = global_symbols.get(name) {
            if !gs.is_defined {
                num_glob_dat += 1;
            }
        }
    }

    let num_rel_dyn = num_relative + num_glob_dat;
    let num_rel_plt = num_plt;

    // ── Layout ────────────────────────────────────────────────────────────
    let ehdr_size: u32 = 52;
    let phdr_size: u32 = 32;

    // Program headers: PHDR, LOAD(ro headers), LOAD(text), LOAD(rodata), LOAD(data), DYNAMIC, GNU_STACK
    let mut num_phdrs: u32 = 7;
    let has_tls = output_sections
        .iter()
        .any(|s| s.flags & SHF_TLS != 0 && s.flags & SHF_ALLOC != 0);
    if has_tls {
        num_phdrs += 1;
    }

    let phdrs_total_size = num_phdrs * phdr_size;

    let mut file_offset: u32 = ehdr_size;
    let mut vaddr: u32 = base_addr + ehdr_size;

    let phdr_offset = file_offset;
    let phdr_vaddr = vaddr;
    file_offset += phdrs_total_size;
    vaddr += phdrs_total_size;

    // .gnu.hash
    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let gnu_hash_offset = file_offset;
    let gnu_hash_vaddr = vaddr;
    let gnu_hash_size = gnu_hash_data.len() as u32;
    file_offset += gnu_hash_size;
    vaddr += gnu_hash_size;

    // .dynsym
    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let dynsym_offset = file_offset;
    let dynsym_vaddr = vaddr;
    let dynsym_entsize: u32 = 16;
    let dynsym_size = (dynsym_entries.len() as u32) * dynsym_entsize;
    file_offset += dynsym_size;
    vaddr += dynsym_size;

    // .dynstr
    let dynstr_offset = file_offset;
    let dynstr_vaddr = vaddr;
    let dynstr_size = dynstr_data.len() as u32;
    file_offset += dynstr_size;
    vaddr += dynstr_size;

    // .rel.dyn
    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let rel_dyn_offset = file_offset;
    let rel_dyn_vaddr = vaddr;
    let rel_dyn_size = (num_rel_dyn as u32) * 8;
    file_offset += rel_dyn_size;
    vaddr += rel_dyn_size;

    // .rel.plt
    let rel_plt_offset = file_offset;
    let rel_plt_vaddr = vaddr;
    let rel_plt_size = (num_rel_plt as u32) * 8;
    file_offset += rel_plt_size;
    vaddr += rel_plt_size;

    let ro_headers_end = file_offset;

    // ── Segment 1 (RX): .text + .plt ──
    file_offset = align_up(file_offset, PAGE_SIZE);
    vaddr = align_up(vaddr, PAGE_SIZE);
    // Ensure congruent file_offset and vaddr (mod PAGE_SIZE)
    vaddr = (vaddr & !0xfff) | (file_offset & 0xfff);

    let text_seg_file_start = file_offset;
    let text_seg_vaddr_start = vaddr;

    // .init
    let (init_vaddr, init_size) = layout_section(
        ".init",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );

    // .plt
    let plt_entry_size: u32 = 16;
    let plt_header_size: u32 = if num_plt > 0 { 16 } else { 0 };
    let plt_total_size = plt_header_size + (num_plt as u32) * plt_entry_size;
    file_offset = align_up(file_offset, 16);
    vaddr = align_up(vaddr, 16);
    let plt_offset = file_offset;
    let plt_vaddr = vaddr;
    file_offset += plt_total_size;
    vaddr += plt_total_size;

    // .text
    let (text_vaddr, text_size) = layout_section(
        ".text",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        16,
    );
    let _ = (text_vaddr, text_size);

    // .fini
    let (fini_vaddr, fini_size) = layout_section(
        ".fini",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );

    // Layout custom executable sections (for __start_/__stop_ symbol auto-generation)
    layout_custom_sections(
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        SHF_EXECINSTR,
    );

    let text_seg_file_end = file_offset;
    let text_seg_vaddr_end = vaddr;

    // ── Segment 2 (R): .rodata + .eh_frame ──
    file_offset = align_up(file_offset, PAGE_SIZE);
    vaddr = align_up(vaddr, PAGE_SIZE);
    vaddr = (vaddr & !0xfff) | (file_offset & 0xfff);

    let rodata_seg_file_start = file_offset;
    let rodata_seg_vaddr_start = vaddr;

    let (_, _) = layout_section(
        ".rodata",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        16,
    );
    let (_, _) = layout_section(
        ".eh_frame",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );

    // Layout custom read-only sections (for __start_/__stop_ symbol auto-generation)
    layout_custom_sections(
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        0,
    );

    let rodata_seg_file_end = file_offset;
    let rodata_seg_vaddr_end = vaddr;

    // ── Segment 3 (RW): .data + .got + .got.plt + .dynamic + .bss ──
    file_offset = align_up(file_offset, PAGE_SIZE);
    vaddr = align_up(vaddr, PAGE_SIZE);
    vaddr = (vaddr & !0xfff) | (file_offset & 0xfff);

    let data_seg_file_start = file_offset;
    let data_seg_vaddr_start = vaddr;

    // TLS sections
    let (tls_addr, _tls_file_offset, _tls_file_size, tls_mem_size, _tls_align) = layout_tls(
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
    );

    // .init_array
    let (init_array_vaddr, init_array_size) = layout_section(
        ".init_array",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );

    // .fini_array
    let (fini_array_vaddr, fini_array_size) = layout_section(
        ".fini_array",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );

    // Layout custom writable sections (for __start_/__stop_ symbol auto-generation)
    layout_custom_sections(
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        SHF_WRITE,
    );

    // .data
    let (_, _) = layout_section(
        ".data",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        16,
    );

    // .got (combined GOT for both data and PLT)
    let got_reserved: usize = 1; // GOT[0] = dynamic addr
    let gotplt_reserved: u32 = 3; // GOT.PLT[0..2] = dynamic/link_map/dl_resolve
    let got_total_entries = got_reserved + got_names.len();
    let gotplt_entries = gotplt_reserved as usize + num_plt;
    let got_size = (got_total_entries as u32) * 4;
    let gotplt_size = (gotplt_entries as u32) * 4;

    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let got_offset = file_offset;
    let got_vaddr = vaddr;
    let got_base = got_vaddr; // _GLOBAL_OFFSET_TABLE_ points here
    file_offset += got_size;
    vaddr += got_size;

    let gotplt_offset = file_offset;
    let gotplt_vaddr = vaddr;
    file_offset += gotplt_size;
    vaddr += gotplt_size;

    // .dynamic
    let dynamic_entry_size: u32 = 8;
    let mut num_dynamic: u32 = 0;
    num_dynamic += needed_sonames.len() as u32; // DT_NEEDED
    if soname.is_some() {
        num_dynamic += 1;
    } // DT_SONAME
    num_dynamic += 5; // GNU_HASH, STRTAB, SYMTAB, STRSZ, SYMENT
    if init_vaddr != 0 && init_size > 0 {
        num_dynamic += 1;
    } // DT_INIT
    if fini_vaddr != 0 && fini_size > 0 {
        num_dynamic += 1;
    } // DT_FINI
    if init_array_size > 0 {
        num_dynamic += 2;
    }
    if fini_array_size > 0 {
        num_dynamic += 2;
    }
    if num_rel_plt > 0 {
        num_dynamic += 4;
    } // PLTGOT, PLTRELSZ, PLTREL, JMPREL
    if num_rel_dyn > 0 {
        num_dynamic += 3;
    } // REL, RELSZ, RELENT
    num_dynamic += 1; // DT_NULL

    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let dynamic_offset = file_offset;
    let dynamic_vaddr = vaddr;
    let dynamic_size = num_dynamic * dynamic_entry_size;
    file_offset += dynamic_size;
    vaddr += dynamic_size;

    // .bss
    let bss_vaddr = vaddr;
    if let Some(&idx) = section_name_to_idx.get(".bss") {
        let a = output_sections[idx].align.max(4);
        vaddr = align_up(vaddr, a);
        output_sections[idx].addr = vaddr;
        output_sections[idx].file_offset = file_offset; // BSS doesn't occupy file space
        let bss_size = output_sections[idx].data.len() as u32;
        vaddr += bss_size;
    }
    let _ = bss_vaddr;

    let data_seg_file_end = file_offset;
    let data_seg_vaddr_end = vaddr;

    // ── Assign symbol addresses ───────────────────────────────────────────
    // Set _GLOBAL_OFFSET_TABLE_
    global_symbols
        .entry("_GLOBAL_OFFSET_TABLE_".to_string())
        .or_insert(LinkerSymbol {
            address: got_base,
            size: 0,
            sym_type: STT_OBJECT,
            binding: STB_LOCAL,
            visibility: STV_DEFAULT,
            is_defined: true,
            needs_plt: false,
            needs_got: false,
            output_section: usize::MAX,
            section_offset: 0,
            plt_index: 0,
            got_index: 0,
            is_dynamic: false,
            dynlib: String::new(),
            needs_copy: false,
            copy_addr: 0,
            version: None,
            uses_textrel: false,
        });
    if let Some(gs) = global_symbols.get_mut("_GLOBAL_OFFSET_TABLE_") {
        gs.address = got_base;
        gs.is_defined = true;
    }

    for (name, sym) in global_symbols.iter_mut() {
        if sym.needs_plt && !sym.is_defined {
            sym.address = plt_vaddr + plt_header_size + (sym.plt_index as u32) * plt_entry_size;
            continue;
        }
        if sym.output_section < output_sections.len() {
            sym.address = output_sections[sym.output_section].addr + sym.section_offset;
        }
        // Standard linker symbols
        match name.as_str() {
            "_edata" | "edata" => {
                sym.address = data_seg_vaddr_start + (data_seg_file_end - data_seg_file_start)
            }
            "_end" | "end" => sym.address = data_seg_vaddr_end,
            "__bss_start" | "__bss_start__" => {
                sym.address = if let Some(&idx) = section_name_to_idx.get(".bss") {
                    output_sections[idx].addr
                } else {
                    data_seg_vaddr_end
                };
            }
            _ => {}
        }
    }

    // ── Apply relocations ─────────────────────────────────────────────────
    // For shared libraries, we need to handle relocations differently:
    // R_386_32 -> write resolved value, emit R_386_RELATIVE
    // R_386_PC32 -> resolved normally (PC-relative, no dynamic reloc needed)
    let mut relative_relocs: Vec<u32> = Vec::new(); // addresses needing R_386_RELATIVE
    let mut glob_dat_relocs: Vec<(u32, usize)> = Vec::new(); // (got_addr, dynsym_idx)

    {
        let reloc_ctx = RelocContext {
            global_symbols: &*global_symbols,
            output_sections,
            section_map,
            got_base,
            got_vaddr,
            gotplt_vaddr,
            got_reserved,
            gotplt_reserved,
            plt_vaddr,
            plt_header_size,
            plt_entry_size,
            num_plt,
            tls_addr,
            tls_mem_size,
            has_tls,
        };

        for (obj_idx, obj) in inputs.iter().enumerate() {
            for sec in &obj.sections {
                if sec.relocations.is_empty() {
                    continue;
                }

                let (out_sec_idx, sec_base_offset) =
                    match section_map.get(&(obj_idx, sec.input_index)) {
                        Some(&v) => v,
                        None => continue,
                    };

                for &(rel_offset, rel_type, sym_idx, addend) in &sec.relocations {
                    let patch_offset = sec_base_offset + rel_offset;
                    let patch_addr = reloc_ctx.output_sections[out_sec_idx].addr + patch_offset;

                    let si = sym_idx as usize;
                    if si >= obj.symbols.len() {
                        continue;
                    }
                    let sym = &obj.symbols[si];

                    let sym_addr = resolve_sym_addr_shared(obj_idx, sym, &reloc_ctx);

                    match rel_type {
                        R_386_NONE => {}
                        R_386_32 => {
                            let value = (sym_addr as i32 + addend) as u32;
                            let off = patch_offset as usize;
                            let sec_data = &mut reloc_ctx.output_sections[out_sec_idx].data;
                            if off + 4 <= sec_data.len() {
                                sec_data[off..off + 4].copy_from_slice(&value.to_le_bytes());
                            }
                            // Determine if this needs a RELATIVE dynamic relocation
                            let needs_relative = if sym.sym_type == STT_SECTION {
                                section_map.contains_key(&(obj_idx, sym.section_index as usize))
                            } else if !sym.name.is_empty() {
                                global_symbols
                                    .get(&sym.name)
                                    .map(|gs| gs.is_defined)
                                    .unwrap_or(false)
                            } else {
                                false
                            };
                            if needs_relative {
                                relative_relocs.push(patch_addr);
                            }
                        }
                        R_386_PC32 | R_386_PLT32 => {
                            let target = if !sym.name.is_empty() {
                                if let Some(gs) = global_symbols.get(&sym.name) {
                                    if gs.needs_plt && !gs.is_defined {
                                        gs.address // PLT address
                                    } else {
                                        sym_addr
                                    }
                                } else {
                                    sym_addr
                                }
                            } else {
                                sym_addr
                            };
                            let value = (target as i32 + addend - patch_addr as i32) as u32;
                            let off = patch_offset as usize;
                            let sec_data = &mut reloc_ctx.output_sections[out_sec_idx].data;
                            if off + 4 <= sec_data.len() {
                                sec_data[off..off + 4].copy_from_slice(&value.to_le_bytes());
                            }
                        }
                        R_386_GOTPC => {
                            let value = (got_base as i32 + addend - patch_addr as i32) as u32;
                            let off = patch_offset as usize;
                            let sec_data = &mut reloc_ctx.output_sections[out_sec_idx].data;
                            if off + 4 <= sec_data.len() {
                                sec_data[off..off + 4].copy_from_slice(&value.to_le_bytes());
                            }
                        }
                        R_386_GOTOFF => {
                            let value = (sym_addr as i32 + addend - got_base as i32) as u32;
                            let off = patch_offset as usize;
                            let sec_data = &mut reloc_ctx.output_sections[out_sec_idx].data;
                            if off + 4 <= sec_data.len() {
                                sec_data[off..off + 4].copy_from_slice(&value.to_le_bytes());
                            }
                        }
                        R_386_GOT32 | R_386_GOT32X => {
                            // In shared libraries, GOT relocations work via GOT entries
                            let mut relax = false;
                            let value = resolve_got_reloc(
                                sym, sym_addr, addend, rel_type, &reloc_ctx, &mut relax,
                            );
                            let off = patch_offset as usize;
                            let sec_data = &mut reloc_ctx.output_sections[out_sec_idx].data;
                            if off + 4 <= sec_data.len() {
                                if relax && off >= 2 && sec_data[off - 2] == 0x8b {
                                    sec_data[off - 2] = 0x8d;
                                }
                                sec_data[off..off + 4].copy_from_slice(&value.to_le_bytes());
                            }
                        }
                        R_386_TLS_TPOFF | R_386_TLS_LE => {
                            if has_tls {
                                let tpoff = sym_addr as i32 - tls_addr as i32 - tls_mem_size as i32;
                                let value = (tpoff + addend) as u32;
                                let off = patch_offset as usize;
                                let sec_data = &mut reloc_ctx.output_sections[out_sec_idx].data;
                                if off + 4 <= sec_data.len() {
                                    sec_data[off..off + 4].copy_from_slice(&value.to_le_bytes());
                                }
                            }
                        }
                        R_386_TLS_LE_32 | R_386_TLS_TPOFF32 => {
                            if has_tls {
                                // rust-gcc emits `add` with TLS_TPOFF32, so use negative offset
                                let value =
                                    (sym_addr as i32 - tls_addr as i32 - tls_mem_size as i32
                                        + addend) as u32;
                                let off = patch_offset as usize;
                                let sec_data = &mut reloc_ctx.output_sections[out_sec_idx].data;
                                if off + 4 <= sec_data.len() {
                                    sec_data[off..off + 4].copy_from_slice(&value.to_le_bytes());
                                }
                            }
                        }
                        R_386_TLS_IE => {
                            let value = resolve_tls_ie(sym, sym_addr, addend, &reloc_ctx);
                            let off = patch_offset as usize;
                            let sec_data = &mut reloc_ctx.output_sections[out_sec_idx].data;
                            if off + 4 <= sec_data.len() {
                                sec_data[off..off + 4].copy_from_slice(&value.to_le_bytes());
                            }
                        }
                        R_386_TLS_GOTIE => {
                            let value = resolve_tls_gotie(sym, sym_addr, addend, &reloc_ctx);
                            let off = patch_offset as usize;
                            let sec_data = &mut reloc_ctx.output_sections[out_sec_idx].data;
                            if off + 4 <= sec_data.len() {
                                sec_data[off..off + 4].copy_from_slice(&value.to_le_bytes());
                            }
                        }
                        _ => {
                            // Silently skip unsupported relocations in shared libraries
                            eprintln!("warning: unsupported relocation type {} for '{}' in shared library", rel_type, sym.name);
                        }
                    }
                }
            }
        }
    }

    // Build GOT entries for undefined symbols -> GLOB_DAT
    for name in &got_names {
        if let Some(gs) = global_symbols.get(name) {
            if !gs.is_defined {
                let got_entry_addr =
                    got_vaddr + (got_reserved as u32 + (gs.got_index - num_plt) as u32) * 4;
                let dynsym_idx = dynsym_map.get(name).copied().unwrap_or(0);
                glob_dat_relocs.push((got_entry_addr, dynsym_idx));
            }
        }
    }

    // ── Build PLT ────────────────────────────────────────────────────────
    let plt_data = build_plt(
        num_plt,
        plt_vaddr,
        plt_header_size,
        plt_entry_size,
        gotplt_vaddr,
        gotplt_reserved,
    );

    // ── Build GOT data ───────────────────────────────────────────────────
    let mut got_data: Vec<u8> = Vec::new();
    // GOT[0] = address of .dynamic (filled by dynamic linker)
    got_data.extend_from_slice(&dynamic_vaddr.to_le_bytes());
    // GOT entries for data symbols
    for name in &got_names {
        if let Some(gs) = global_symbols.get(name) {
            if gs.is_defined {
                got_data.extend_from_slice(&gs.address.to_le_bytes());
            } else {
                got_data.extend_from_slice(&0u32.to_le_bytes());
            }
        } else {
            got_data.extend_from_slice(&0u32.to_le_bytes());
        }
    }

    // .got.plt
    let mut gotplt_data: Vec<u8> = Vec::new();
    gotplt_data.extend_from_slice(&dynamic_vaddr.to_le_bytes()); // GOT.PLT[0] = .dynamic
    gotplt_data.extend_from_slice(&0u32.to_le_bytes()); // GOT.PLT[1] = link_map (filled by ld.so)
    gotplt_data.extend_from_slice(&0u32.to_le_bytes()); // GOT.PLT[2] = dl_resolve (filled by ld.so)
                                                        // GOT.PLT[3..] = PLT lazy stubs (point back to PLT[N]+6)
    for i in 0..num_plt {
        let plt_stub_addr = plt_vaddr + plt_header_size + (i as u32) * plt_entry_size + 6;
        gotplt_data.extend_from_slice(&plt_stub_addr.to_le_bytes());
    }

    // ── Build .rel.dyn ───────────────────────────────────────────────────
    let mut rel_dyn_data: Vec<u8> = Vec::new();
    // R_386_RELATIVE entries
    for &addr in &relative_relocs {
        rel_dyn_data.extend_from_slice(&addr.to_le_bytes());
        rel_dyn_data.extend_from_slice(&R_386_RELATIVE.to_le_bytes());
    }
    // R_386_GLOB_DAT entries
    for &(addr, dynsym_idx) in &glob_dat_relocs {
        rel_dyn_data.extend_from_slice(&addr.to_le_bytes());
        let r_info = ((dynsym_idx as u32) << 8) | R_386_GLOB_DAT;
        rel_dyn_data.extend_from_slice(&r_info.to_le_bytes());
    }

    // ── Build .rel.plt ───────────────────────────────────────────────────
    let mut rel_plt_data: Vec<u8> = Vec::new();
    for (i, name) in plt_names.iter().enumerate() {
        let gotplt_entry = gotplt_vaddr + (gotplt_reserved + i as u32) * 4;
        let dynsym_idx = dynsym_map.get(name).copied().unwrap_or(0) as u32;
        rel_plt_data.extend_from_slice(&gotplt_entry.to_le_bytes());
        let r_info = (dynsym_idx << 8) | R_386_JMP_SLOT;
        rel_plt_data.extend_from_slice(&r_info.to_le_bytes());
    }

    // ── Build .dynamic section ───────────────────────────────────────────
    let mut dynamic_data: Vec<u8> = Vec::new();
    for lib in needed_sonames {
        push_dyn(&mut dynamic_data, DT_NEEDED, dynstr.get_offset(lib));
    }
    if let Some(ref sn) = soname {
        push_dyn(&mut dynamic_data, DT_SONAME, dynstr.get_offset(sn));
    }
    push_dyn(&mut dynamic_data, DT_GNU_HASH_TAG, gnu_hash_vaddr);
    push_dyn(&mut dynamic_data, DT_STRTAB, dynstr_vaddr);
    push_dyn(&mut dynamic_data, DT_SYMTAB, dynsym_vaddr);
    push_dyn(&mut dynamic_data, DT_STRSZ, dynstr_size);
    push_dyn(&mut dynamic_data, DT_SYMENT, dynsym_entsize);
    if init_vaddr != 0 && init_size > 0 {
        push_dyn(&mut dynamic_data, DT_INIT, init_vaddr);
    }
    if fini_vaddr != 0 && fini_size > 0 {
        push_dyn(&mut dynamic_data, DT_FINI, fini_vaddr);
    }
    if init_array_size > 0 {
        push_dyn(&mut dynamic_data, DT_INIT_ARRAY, init_array_vaddr);
        push_dyn(&mut dynamic_data, DT_INIT_ARRAYSZ, init_array_size);
    }
    if fini_array_size > 0 {
        push_dyn(&mut dynamic_data, DT_FINI_ARRAY, fini_array_vaddr);
        push_dyn(&mut dynamic_data, DT_FINI_ARRAYSZ, fini_array_size);
    }
    if num_rel_plt > 0 {
        push_dyn(&mut dynamic_data, DT_PLTGOT, gotplt_vaddr);
        push_dyn(&mut dynamic_data, DT_PLTRELSZ, rel_plt_size);
        push_dyn(&mut dynamic_data, DT_PLTREL, DT_REL as u32);
        push_dyn(&mut dynamic_data, DT_JMPREL, rel_plt_vaddr);
    }
    if num_rel_dyn > 0 {
        push_dyn(&mut dynamic_data, DT_REL, rel_dyn_vaddr);
        push_dyn(&mut dynamic_data, DT_RELSZ, rel_dyn_size);
        push_dyn(&mut dynamic_data, DT_RELENT, 8);
    }
    push_dyn(&mut dynamic_data, DT_NULL, 0);

    // Update dynsym entries with resolved addresses
    for (i, name) in dynsym_names.iter().enumerate() {
        if let Some(gs) = global_symbols.get(name) {
            if gs.is_defined {
                dynsym_entries[i + 1].value = gs.address;
                // Determine section index for dynsym
                if gs.output_section < output_sections.len() {
                    // Find the section number. For simplicity, mark as SHN_ABS=0xfff1
                    // Real implementations map to proper section indices but
                    // dynamic symbols usually don't need exact shndx
                    dynsym_entries[i + 1].shndx = SHN_ABS;
                }
            }
        }
    }

    // ── Write output file ─────────────────────────────────────────────────
    let total_file_size = data_seg_file_end as usize;
    let mut output = vec![0u8; total_file_size];

    // ELF header (ET_DYN, e_entry = 0)
    output[0..4].copy_from_slice(&ELF_MAGIC);
    output[4] = ELFCLASS32;
    output[5] = ELFDATA2LSB;
    output[6] = EV_CURRENT;
    output[7] = 0; // ELFOSABI_NONE
    output[16..18].copy_from_slice(&ET_DYN.to_le_bytes());
    output[18..20].copy_from_slice(&EM_386.to_le_bytes());
    output[20..24].copy_from_slice(&1u32.to_le_bytes()); // e_version
    output[24..28].copy_from_slice(&0u32.to_le_bytes()); // e_entry = 0 for .so
    output[28..32].copy_from_slice(&ehdr_size.to_le_bytes()); // e_phoff
    output[32..36].copy_from_slice(&0u32.to_le_bytes()); // e_shoff = 0 (no section headers)
    output[36..40].copy_from_slice(&0u32.to_le_bytes()); // e_flags
    output[40..42].copy_from_slice(&(ehdr_size as u16).to_le_bytes()); // e_ehsize
    output[42..44].copy_from_slice(&32u16.to_le_bytes()); // e_phentsize
    output[44..46].copy_from_slice(&(num_phdrs as u16).to_le_bytes()); // e_phnum
    output[46..48].copy_from_slice(&40u16.to_le_bytes()); // e_shentsize
    output[48..50].copy_from_slice(&0u16.to_le_bytes()); // e_shnum
    output[50..52].copy_from_slice(&0u16.to_le_bytes()); // e_shstrndx

    // Write program headers
    let mut ph_off = phdr_offset as usize;

    // Helper to write a PHDR
    let write_phdr = |out: &mut [u8],
                      off: usize,
                      p_type: u32,
                      flags: u32,
                      f_off: u32,
                      va: u32,
                      f_sz: u32,
                      m_sz: u32,
                      align: u32| {
        out[off..off + 4].copy_from_slice(&p_type.to_le_bytes());
        out[off + 4..off + 8].copy_from_slice(&f_off.to_le_bytes());
        out[off + 8..off + 12].copy_from_slice(&va.to_le_bytes());
        out[off + 12..off + 16].copy_from_slice(&va.to_le_bytes()); // paddr = vaddr
        out[off + 16..off + 20].copy_from_slice(&f_sz.to_le_bytes());
        out[off + 20..off + 24].copy_from_slice(&m_sz.to_le_bytes());
        out[off + 24..off + 28].copy_from_slice(&flags.to_le_bytes());
        out[off + 28..off + 32].copy_from_slice(&align.to_le_bytes());
    };

    // PHDR
    write_phdr(
        &mut output,
        ph_off,
        PT_PHDR,
        PF_R,
        phdr_offset,
        phdr_vaddr,
        phdrs_total_size,
        phdrs_total_size,
        4,
    );
    ph_off += 32;

    // LOAD: RO headers (ELF header + phdrs + .gnu.hash + .dynsym + .dynstr + .rel.*)
    write_phdr(
        &mut output,
        ph_off,
        PT_LOAD,
        PF_R,
        0,
        base_addr,
        ro_headers_end,
        ro_headers_end,
        PAGE_SIZE,
    );
    ph_off += 32;

    // LOAD: RX (text segment)
    let text_file_sz = text_seg_file_end - text_seg_file_start;
    let text_mem_sz = text_seg_vaddr_end - text_seg_vaddr_start;
    write_phdr(
        &mut output,
        ph_off,
        PT_LOAD,
        PF_R | PF_X,
        text_seg_file_start,
        text_seg_vaddr_start,
        text_file_sz,
        text_mem_sz,
        PAGE_SIZE,
    );
    ph_off += 32;

    // LOAD: RO (rodata segment)
    let rodata_file_sz = rodata_seg_file_end - rodata_seg_file_start;
    let rodata_mem_sz = rodata_seg_vaddr_end - rodata_seg_vaddr_start;
    if rodata_file_sz > 0 {
        write_phdr(
            &mut output,
            ph_off,
            PT_LOAD,
            PF_R,
            rodata_seg_file_start,
            rodata_seg_vaddr_start,
            rodata_file_sz,
            rodata_mem_sz,
            PAGE_SIZE,
        );
    } else {
        // Empty rodata segment
        write_phdr(
            &mut output,
            ph_off,
            PT_LOAD,
            PF_R,
            rodata_seg_file_start,
            rodata_seg_vaddr_start,
            0,
            0,
            PAGE_SIZE,
        );
    }
    ph_off += 32;

    // LOAD: RW (data segment)
    let data_file_sz = data_seg_file_end - data_seg_file_start;
    let data_mem_sz = data_seg_vaddr_end - data_seg_vaddr_start;
    write_phdr(
        &mut output,
        ph_off,
        PT_LOAD,
        PF_R | PF_W,
        data_seg_file_start,
        data_seg_vaddr_start,
        data_file_sz,
        data_mem_sz,
        PAGE_SIZE,
    );
    ph_off += 32;

    // DYNAMIC
    write_phdr(
        &mut output,
        ph_off,
        PT_DYNAMIC,
        PF_R | PF_W,
        dynamic_offset,
        dynamic_vaddr,
        dynamic_size,
        dynamic_size,
        4,
    );
    ph_off += 32;

    // GNU_STACK
    write_phdr(
        &mut output,
        ph_off,
        PT_GNU_STACK,
        PF_R | PF_W,
        0,
        0,
        0,
        0,
        0,
    );
    ph_off += 32;

    // TLS
    if has_tls {
        let tls_f_size = if let Some(&idx) = section_name_to_idx.get(".tdata") {
            output_sections[idx].data.len() as u32
        } else {
            0
        };
        write_phdr(
            &mut output,
            ph_off,
            PT_TLS,
            PF_R,
            if tls_addr > 0 {
                output_sections[section_name_to_idx[".tdata"]].file_offset
            } else {
                0
            },
            tls_addr,
            tls_f_size,
            tls_mem_size,
            _tls_align,
        );
        let _ = ph_off; // suppress unused warning
    }

    // Write .gnu.hash
    let off = gnu_hash_offset as usize;
    if off + gnu_hash_data.len() <= output.len() {
        output[off..off + gnu_hash_data.len()].copy_from_slice(&gnu_hash_data);
    }

    // Write .dynsym
    for (i, entry) in dynsym_entries.iter().enumerate() {
        let off = dynsym_offset as usize + i * 16;
        if off + 16 > output.len() {
            break;
        }
        output[off..off + 4].copy_from_slice(&entry.name.to_le_bytes());
        output[off + 4..off + 8].copy_from_slice(&entry.value.to_le_bytes());
        output[off + 8..off + 12].copy_from_slice(&entry.size.to_le_bytes());
        output[off + 12] = entry.info;
        output[off + 13] = entry.other;
        output[off + 14..off + 16].copy_from_slice(&entry.shndx.to_le_bytes());
    }

    // Write .dynstr
    let off = dynstr_offset as usize;
    if off + dynstr_data.len() <= output.len() {
        output[off..off + dynstr_data.len()].copy_from_slice(&dynstr_data);
    }

    // Write .rel.dyn
    let off = rel_dyn_offset as usize;
    if off + rel_dyn_data.len() <= output.len() {
        output[off..off + rel_dyn_data.len()].copy_from_slice(&rel_dyn_data);
    }

    // Write .rel.plt
    let off = rel_plt_offset as usize;
    if off + rel_plt_data.len() <= output.len() {
        output[off..off + rel_plt_data.len()].copy_from_slice(&rel_plt_data);
    }

    // Write .plt
    let off = plt_offset as usize;
    if off + plt_data.len() <= output.len() {
        output[off..off + plt_data.len()].copy_from_slice(&plt_data);
    }

    // Write output sections (text, rodata, data, etc.)
    for sec in output_sections.iter() {
        if sec.sh_type == SHT_NOBITS {
            continue;
        }
        let off = sec.file_offset as usize;
        if off + sec.data.len() <= output.len() && !sec.data.is_empty() {
            output[off..off + sec.data.len()].copy_from_slice(&sec.data);
        }
    }

    // Write .got
    let off = got_offset as usize;
    if off + got_data.len() <= output.len() {
        output[off..off + got_data.len()].copy_from_slice(&got_data);
    }

    // Write .got.plt
    let off = gotplt_offset as usize;
    if off + gotplt_data.len() <= output.len() {
        output[off..off + gotplt_data.len()].copy_from_slice(&gotplt_data);
    }

    // Write .dynamic
    let off = dynamic_offset as usize;
    if off + dynamic_data.len() <= output.len() {
        output[off..off + dynamic_data.len()].copy_from_slice(&dynamic_data);
    }

    // Write to file
    std::fs::write(output_path, &output)
        .map_err(|e| format!("failed to write {}: {}", output_path, e))?;

    // Set executable permission
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(output_path, std::fs::Permissions::from_mode(0o755));
    }

    Ok(())
}

/// Resolve symbol address in shared library context.
pub(super) fn resolve_sym_addr_shared(
    obj_idx: usize,
    sym: &InputSymbol,
    ctx: &RelocContext,
) -> u32 {
    if sym.sym_type == STT_SECTION {
        if sym.section_index != SHN_UNDEF && sym.section_index != SHN_ABS {
            match ctx.section_map.get(&(obj_idx, sym.section_index as usize)) {
                Some(&(sec_out_idx, sec_out_offset)) => {
                    ctx.output_sections[sec_out_idx].addr + sec_out_offset
                }
                None => 0,
            }
        } else {
            0
        }
    } else if sym.name.is_empty() {
        0
    } else if sym.binding == STB_LOCAL {
        // Local symbols - resolve via section map
        if sym.section_index != SHN_UNDEF && sym.section_index != SHN_ABS {
            match ctx.section_map.get(&(obj_idx, sym.section_index as usize)) {
                Some(&(sec_out_idx, sec_out_offset)) => {
                    ctx.output_sections[sec_out_idx].addr + sec_out_offset + sym.value
                }
                None => sym.value,
            }
        } else if sym.section_index == SHN_ABS {
            sym.value
        } else {
            0
        }
    } else {
        match ctx.global_symbols.get(&sym.name) {
            Some(gs) => gs.address,
            None => {
                if sym.section_index != SHN_UNDEF && sym.section_index != SHN_ABS {
                    match ctx.section_map.get(&(obj_idx, sym.section_index as usize)) {
                        Some(&(sec_out_idx, sec_out_offset)) => {
                            ctx.output_sections[sec_out_idx].addr + sec_out_offset + sym.value
                        }
                        None => sym.value,
                    }
                } else {
                    0
                }
            }
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
