//! Section merging and common symbol allocation for ELF64 linkers.
//!
//! Groups input sections by mapped name, computes output offsets with proper
//! alignment, sorts output sections by permission profile, and allocates
//! SHN_COMMON symbols into `.bss`.

use std::collections::{HashMap, HashSet};

use super::section_map::map_section_name;
use super::symbols::{GlobalSymbolOps, InputSection, OutputSection};
use super::types::Elf64Object;
use crate::backend::elf::{
    SHF_ALLOC, SHF_EXCLUDE, SHF_EXECINSTR, SHF_TLS, SHF_WRITE, SHN_COMMON, SHT_GROUP, SHT_NOBITS,
    SHT_NULL, SHT_PROGBITS, SHT_REL, SHT_RELA, SHT_STRTAB, SHT_SYMTAB,
};

/// Merge input sections from all objects into output sections.
///
/// Groups input sections by mapped name (e.g., `.text.foo` -> `.text`),
/// computes output offsets with proper alignment, and sorts output sections
/// by permission profile: RO -> Exec -> RW(progbits) -> RW(nobits).
pub fn merge_sections_elf64(
    objects: &[Elf64Object],
    output_sections: &mut Vec<OutputSection>,
    section_map: &mut HashMap<(usize, usize), (usize, u64)>,
) {
    let no_dead = HashSet::new();
    merge_sections_elf64_gc(objects, output_sections, section_map, &no_dead);
}

/// Merge input sections into output sections, optionally skipping dead sections.
///
/// When `dead_sections` is non-empty (from --gc-sections), sections in the set
/// are excluded from the output, effectively garbage-collecting unreferenced code.
pub fn merge_sections_elf64_gc(
    objects: &[Elf64Object],
    output_sections: &mut Vec<OutputSection>,
    section_map: &mut HashMap<(usize, usize), (usize, u64)>,
    dead_sections: &HashSet<(usize, usize)>,
) {
    let mut output_map: HashMap<String, usize> = HashMap::new();

    for obj_idx in 0..objects.len() {
        for sec_idx in 0..objects[obj_idx].sections.len() {
            let sec = &objects[obj_idx].sections[sec_idx];
            if sec.flags & SHF_ALLOC == 0 {
                continue;
            }
            if matches!(
                sec.sh_type,
                SHT_NULL | SHT_STRTAB | SHT_SYMTAB | SHT_RELA | SHT_REL | SHT_GROUP
            ) {
                continue;
            }
            if sec.flags & SHF_EXCLUDE != 0 {
                continue;
            }
            if !dead_sections.is_empty() && dead_sections.contains(&(obj_idx, sec_idx)) {
                continue;
            }

            let output_name = map_section_name(&sec.name).to_string();
            let alignment = sec.addralign.max(1);

            let out_idx = if let Some(&idx) = output_map.get(&output_name) {
                if alignment > output_sections[idx].alignment {
                    output_sections[idx].alignment = alignment;
                }
                idx
            } else {
                let idx = output_sections.len();
                output_map.insert(output_name.clone(), idx);
                output_sections.push(OutputSection {
                    name: output_name,
                    sh_type: sec.sh_type,
                    flags: sec.flags,
                    alignment,
                    inputs: Vec::new(),
                    data: Vec::new(),
                    addr: 0,
                    file_offset: 0,
                    mem_size: 0,
                });
                idx
            };

            if sec.sh_type == SHT_PROGBITS {
                output_sections[out_idx].sh_type = SHT_PROGBITS;
            }
            output_sections[out_idx].flags |=
                sec.flags & (SHF_WRITE | SHF_EXECINSTR | SHF_ALLOC | SHF_TLS);
            output_sections[out_idx].inputs.push(InputSection {
                object_idx: obj_idx,
                section_idx: sec_idx,
                output_offset: 0,
                size: sec.size,
            });
        }
    }

    for out_sec in output_sections.iter_mut() {
        let mut off: u64 = 0;
        for input in &mut out_sec.inputs {
            let a = objects[input.object_idx].sections[input.section_idx]
                .addralign
                .max(1);
            off = (off + a - 1) & !(a - 1);
            input.output_offset = off;
            off += input.size;
        }
        out_sec.mem_size = off;
    }

    for (out_idx, out_sec) in output_sections.iter().enumerate() {
        for input in &out_sec.inputs {
            section_map.insert(
                (input.object_idx, input.section_idx),
                (out_idx, input.output_offset),
            );
        }
    }

    // Sort: RO -> Exec -> RW(progbits) -> RW(nobits)
    let len = output_sections.len();
    let mut opts: Vec<Option<OutputSection>> = output_sections.drain(..).map(Some).collect();
    let mut sort_indices: Vec<usize> = (0..len).collect();
    sort_indices.sort_by_key(|&i| {
        let sec = opts[i].as_ref().unwrap();
        let is_exec = sec.flags & SHF_EXECINSTR != 0;
        let is_write = sec.flags & SHF_WRITE != 0;
        let is_nobits = sec.sh_type == SHT_NOBITS;
        if is_exec {
            (1u32, is_nobits as u32)
        } else if !is_write {
            (0, is_nobits as u32)
        } else {
            (2, is_nobits as u32)
        }
    });

    let mut index_remap: HashMap<usize, usize> = HashMap::new();
    for (new_idx, &old_idx) in sort_indices.iter().enumerate() {
        index_remap.insert(old_idx, new_idx);
    }
    for &old_idx in &sort_indices {
        output_sections.push(opts[old_idx].take().unwrap());
    }

    let old_map: Vec<_> = section_map.drain().collect();
    for ((obj_idx, sec_idx), (old_out_idx, off)) in old_map {
        if let Some(&new_out_idx) = index_remap.get(&old_out_idx) {
            section_map.insert((obj_idx, sec_idx), (new_out_idx, off));
        }
    }
}

/// Allocate SHN_COMMON symbols into the .bss output section.
pub fn allocate_common_symbols_elf64<G: GlobalSymbolOps>(
    globals: &mut HashMap<String, G>,
    output_sections: &mut Vec<OutputSection>,
) {
    let common_syms: Vec<(String, u64, u64)> = globals
        .iter()
        .filter(|(_, sym)| sym.section_idx() == SHN_COMMON && sym.is_defined())
        .map(|(name, sym)| (name.clone(), sym.value().max(1), sym.size()))
        .collect();
    if common_syms.is_empty() {
        return;
    }

    let bss_idx = output_sections
        .iter()
        .position(|s| s.name == ".bss")
        .unwrap_or_else(|| {
            let idx = output_sections.len();
            output_sections.push(OutputSection {
                name: ".bss".to_string(),
                sh_type: SHT_NOBITS,
                flags: SHF_ALLOC | SHF_WRITE,
                alignment: 1,
                inputs: Vec::new(),
                data: Vec::new(),
                addr: 0,
                file_offset: 0,
                mem_size: 0,
            });
            idx
        });

    let mut bss_off = output_sections[bss_idx].mem_size;
    for (name, alignment, size) in &common_syms {
        let a = (*alignment).max(1);
        bss_off = (bss_off + a - 1) & !(a - 1);
        if let Some(sym) = globals.get_mut(name) {
            sym.set_common_bss(bss_off);
        }
        if *alignment > output_sections[bss_idx].alignment {
            output_sections[bss_idx].alignment = *alignment;
        }
        bss_off += size;
    }
    output_sections[bss_idx].mem_size = bss_off;
}
