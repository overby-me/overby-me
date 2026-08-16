//! Phase 3: Global symbol table construction.
//!
//! Builds the global symbol table from input objects, handles COMMON symbols,
//! marks PLT/GOT needs for dynamic linking, and identifies GOT entries for
//! PC-relative GOT references.

use super::elf_read::*;
use super::relocations::{
    got_sym_key, GlobalSym, MergedSection, R_RISCV_GOT_HI20, R_RISCV_TLS_GD_HI20,
    R_RISCV_TLS_GOT_HI20,
};
use std::collections::{HashMap, HashSet};

/// Build the global symbol table from all input objects.
///
/// For each non-local symbol, creates or updates a `GlobalSym` entry.
/// Handles SHN_UNDEF (undefined), SHN_ABS (absolute), SHN_COMMON (common/BSS),
/// and defined symbols. Weak-to-strong overrides are applied.
pub fn build_global_symbols(
    input_objs: &[(String, ElfObject)],
    sec_mapping: &HashMap<(usize, usize), (usize, u64)>,
    merged_sections: &mut Vec<MergedSection>,
    merged_map: &mut HashMap<String, usize>,
) -> HashMap<String, GlobalSym> {
    let mut global_syms: HashMap<String, GlobalSym> = HashMap::new();

    for (obj_idx, (_, obj)) in input_objs.iter().enumerate() {
        for sym in &obj.symbols {
            if sym.name.is_empty() || sym.binding() == STB_LOCAL {
                continue;
            }

            if sym.shndx == SHN_UNDEF {
                global_syms
                    .entry(sym.name.clone())
                    .or_insert_with(|| GlobalSym {
                        value: 0,
                        size: 0,
                        binding: sym.binding(),
                        sym_type: sym.sym_type(),
                        visibility: sym.visibility(),
                        defined: false,
                        needs_plt: false,
                        plt_idx: 0,
                        got_offset: None,
                        section_idx: None,
                    });
                continue;
            }

            if sym.shndx == SHN_ABS {
                let entry = global_syms
                    .entry(sym.name.clone())
                    .or_insert_with(|| GlobalSym {
                        value: sym.value,
                        size: sym.size,
                        binding: sym.binding(),
                        sym_type: sym.sym_type(),
                        visibility: sym.visibility(),
                        defined: true,
                        needs_plt: false,
                        plt_idx: 0,
                        got_offset: None,
                        section_idx: None,
                    });
                if !entry.defined || (entry.binding == STB_WEAK && sym.binding() == STB_GLOBAL) {
                    entry.value = sym.value;
                    entry.size = sym.size;
                    entry.binding = sym.binding();
                    entry.sym_type = sym.sym_type();
                    entry.defined = true;
                }
                continue;
            }

            if sym.shndx == SHN_COMMON {
                allocate_common_symbol(sym, &mut global_syms, merged_sections, merged_map);
                continue;
            }

            let sec_idx = sym.shndx as usize;
            let (merged_idx, offset) = match sec_mapping.get(&(obj_idx, sec_idx)) {
                Some(&v) => v,
                None => continue,
            };

            let entry = global_syms
                .entry(sym.name.clone())
                .or_insert_with(|| GlobalSym {
                    value: 0,
                    size: sym.size,
                    binding: sym.binding(),
                    sym_type: sym.sym_type(),
                    visibility: sym.visibility(),
                    defined: false,
                    needs_plt: false,
                    plt_idx: 0,
                    got_offset: None,
                    section_idx: None,
                });

            if entry.defined && !(entry.binding == STB_WEAK && sym.binding() == STB_GLOBAL) {
                continue;
            }

            entry.value = offset + sym.value;
            entry.size = sym.size;
            entry.binding = sym.binding();
            entry.sym_type = sym.sym_type();
            entry.visibility = sym.visibility();
            entry.defined = true;
            entry.section_idx = Some(merged_idx);
        }
    }

    global_syms
}

/// Allocate a COMMON symbol in .bss.
fn allocate_common_symbol(
    sym: &Symbol,
    global_syms: &mut HashMap<String, GlobalSym>,
    merged_sections: &mut Vec<MergedSection>,
    merged_map: &mut HashMap<String, usize>,
) {
    let bss_idx = *merged_map.entry(".bss".into()).or_insert_with(|| {
        let idx = merged_sections.len();
        merged_sections.push(MergedSection {
            name: ".bss".into(),
            sh_type: SHT_NOBITS,
            sh_flags: SHF_ALLOC | SHF_WRITE,
            data: Vec::new(),
            vaddr: 0,
            align: 8,
        });
        idx
    });
    let ms = &mut merged_sections[bss_idx];
    let align = sym.value.max(1) as usize; // st_value is alignment for COMMON
    let cur = ms.data.len();
    let aligned = (cur + align - 1) & !(align - 1);
    ms.data.resize(aligned, 0);
    let off = ms.data.len() as u64;
    ms.data.resize(ms.data.len() + sym.size as usize, 0);
    ms.align = ms.align.max(align as u64);

    let entry = global_syms
        .entry(sym.name.clone())
        .or_insert_with(|| GlobalSym {
            value: off,
            size: sym.size,
            binding: sym.binding(),
            sym_type: STT_OBJECT,
            visibility: sym.visibility(),
            defined: true,
            needs_plt: false,
            plt_idx: 0,
            got_offset: None,
            section_idx: Some(bss_idx),
        });
    if !entry.defined || (entry.binding == STB_WEAK && sym.binding() == STB_GLOBAL) {
        entry.value = off;
        entry.size = sym.size.max(entry.size);
        entry.binding = sym.binding();
        entry.defined = true;
        entry.section_idx = Some(bss_idx);
    }
}

/// Mark symbols that need PLT entries (undefined functions found in shared libs)
/// and collect symbols that need COPY relocations (undefined data objects).
pub fn mark_plt_and_copy_symbols(
    global_syms: &mut HashMap<String, GlobalSym>,
    shared_lib_syms: &HashMap<String, DynSymbol>,
) -> (Vec<String>, Vec<(String, u64)>) {
    let mut plt_symbols: Vec<String> = Vec::new();
    let mut copy_symbols: Vec<(String, u64)> = Vec::new();

    for (name, sym) in global_syms.iter_mut() {
        if !sym.defined {
            if let Some(shlib_sym) = shared_lib_syms.get(name) {
                if shlib_sym.sym_type() == STT_OBJECT {
                    copy_symbols.push((name.clone(), shlib_sym.size));
                } else {
                    sym.needs_plt = true;
                    sym.plt_idx = plt_symbols.len();
                    plt_symbols.push(name.clone());
                }
            }
        }
    }

    (plt_symbols, copy_symbols)
}

/// Identify GOT entries needed by scanning for GOT_HI20 and TLS GOT relocations.
///
/// Returns the ordered list of GOT symbol keys, the set of TLS GOT symbols,
/// and a map of local GOT symbol info for resolving local GOT entries.
pub fn collect_got_entries(
    input_objs: &[(String, ElfObject)],
) -> (
    Vec<String>,
    HashSet<String>,
    HashMap<String, (usize, usize, i64)>,
) {
    let mut got_symbols: Vec<String> = Vec::new();
    let mut tls_got_symbols: HashSet<String> = HashSet::new();
    let mut local_got_sym_info: HashMap<String, (usize, usize, i64)> = HashMap::new();

    let mut got_set: HashSet<String> = HashSet::new();
    for (obj_idx, (_, obj)) in input_objs.iter().enumerate() {
        for relocs in &obj.relocations {
            for reloc in relocs {
                if reloc.rela_type == R_RISCV_GOT_HI20
                    || reloc.rela_type == R_RISCV_TLS_GOT_HI20
                    || reloc.rela_type == R_RISCV_TLS_GD_HI20
                {
                    let sym = &obj.symbols[reloc.sym_idx as usize];
                    let (name, is_local) = got_sym_key(obj_idx, sym, reloc.addend);
                    if !name.is_empty() && !got_set.contains(&name) {
                        got_set.insert(name.clone());
                        got_symbols.push(name.clone());
                        if is_local {
                            local_got_sym_info.insert(
                                name.clone(),
                                (obj_idx, reloc.sym_idx as usize, reloc.addend),
                            );
                        }
                    }
                    if reloc.rela_type == R_RISCV_TLS_GOT_HI20
                        || reloc.rela_type == R_RISCV_TLS_GD_HI20
                    {
                        tls_got_symbols.insert(name);
                    }
                }
            }
        }
    }

    (got_symbols, tls_got_symbols, local_got_sym_info)
}

/// Build local symbol virtual address table for relocation resolution.
pub fn build_local_sym_vaddrs(
    input_objs: &[(String, ElfObject)],
    sec_mapping: &HashMap<(usize, usize), (usize, u64)>,
    section_vaddrs: &[u64],
    global_syms: &HashMap<String, GlobalSym>,
) -> Vec<Vec<u64>> {
    let mut local_sym_vaddrs: Vec<Vec<u64>> = Vec::new();
    for (obj_idx, (_, obj)) in input_objs.iter().enumerate() {
        let mut sym_vaddrs = vec![0u64; obj.symbols.len()];
        for (si, sym) in obj.symbols.iter().enumerate() {
            if sym.shndx == SHN_UNDEF || sym.shndx == SHN_ABS {
                if sym.shndx == SHN_ABS {
                    sym_vaddrs[si] = sym.value;
                }
                continue;
            }
            if sym.shndx == SHN_COMMON {
                if let Some(gs) = global_syms.get(&sym.name) {
                    sym_vaddrs[si] = gs.value;
                }
                continue;
            }
            let sec_idx = sym.shndx as usize;
            if let Some(&(merged_idx, offset)) = sec_mapping.get(&(obj_idx, sec_idx)) {
                sym_vaddrs[si] = section_vaddrs[merged_idx] + offset + sym.value;
            }
        }
        local_sym_vaddrs.push(sym_vaddrs);
    }
    local_sym_vaddrs
}

/// Check for truly undefined symbols (not dynamic, not weak, not linker-defined).
pub fn check_undefined_symbols(
    global_syms: &HashMap<String, GlobalSym>,
    shared_lib_syms: &HashMap<String, DynSymbol>,
) -> Result<(), String> {
    let mut truly_undefined: Vec<&String> = global_syms
        .iter()
        .filter(|(name, sym)| {
            !sym.defined
                && !sym.needs_plt
                && sym.binding != STB_WEAK
                && !crate::backend::linker_common::is_linker_defined_symbol(name)
                && !shared_lib_syms.contains_key(name.as_str())
        })
        .map(|(name, _)| name)
        .collect();

    if !truly_undefined.is_empty() {
        truly_undefined.sort();
        truly_undefined.truncate(20);
        return Err(format!(
            "undefined symbols: {}",
            truly_undefined
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(())
}
