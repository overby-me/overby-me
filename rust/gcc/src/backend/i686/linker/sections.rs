//! Section merging for the i686 linker.
//!
//! Phase 5 of the linking pipeline: merges input sections from all objects
//! into output sections, handling COMDAT group deduplication and section
//! type/flag assignment.

use std::collections::{HashMap, HashSet};

use super::types::*;

pub(super) fn merge_sections(
    inputs: &[InputObject],
) -> (Vec<OutputSection>, HashMap<String, usize>, SectionMap) {
    let mut output_sections: Vec<OutputSection> = Vec::new();
    let mut section_name_to_idx: HashMap<String, usize> = HashMap::new();
    let mut section_map: SectionMap = HashMap::new();
    let mut included_comdat_sections: HashSet<String> = HashSet::new();

    // COMDAT group deduplication
    let comdat_skip = compute_comdat_skip(inputs);

    for (obj_idx, obj) in inputs.iter().enumerate() {
        for sec in obj.sections.iter() {
            if comdat_skip.contains(&(obj_idx, sec.input_index)) {
                continue;
            }
            let out_name = match output_section_name(&sec.name, sec.flags, sec.sh_type) {
                Some(n) => n,
                None => continue,
            };

            // COMDAT deduplication by section name
            if sec.flags & SHF_GROUP != 0 && !included_comdat_sections.insert(sec.name.clone()) {
                continue;
            }

            let out_idx = if let Some(&idx) = section_name_to_idx.get(&out_name) {
                idx
            } else {
                let idx = output_sections.len();
                let (sh_type, flags) = section_type_and_flags(&out_name, sec);
                section_name_to_idx.insert(out_name.clone(), idx);
                output_sections.push(OutputSection {
                    name: out_name,
                    sh_type,
                    flags,
                    data: Vec::new(),
                    align: 1,
                    addr: 0,
                    file_offset: 0,
                });
                idx
            };

            let out_sec = &mut output_sections[out_idx];
            // .init and .fini must be concatenated without padding
            let align = if out_sec.name == ".init" || out_sec.name == ".fini" {
                1
            } else {
                sec.align.max(1)
            };
            if align > out_sec.align {
                out_sec.align = align;
            }
            let padding = (align - (out_sec.data.len() as u32 % align)) % align;
            out_sec
                .data
                .extend(std::iter::repeat_n(0u8, padding as usize));
            let offset = out_sec.data.len() as u32;

            section_map.insert((obj_idx, sec.input_index), (out_idx, offset));

            if sec.sh_type != SHT_NOBITS {
                out_sec.data.extend_from_slice(&sec.data);
            } else {
                out_sec
                    .data
                    .extend(std::iter::repeat_n(0u8, sec.data.len()));
            }
        }
    }

    (output_sections, section_name_to_idx, section_map)
}

pub(super) fn compute_comdat_skip(inputs: &[InputObject]) -> HashSet<(usize, usize)> {
    let mut comdat_skip = HashSet::new();
    let mut seen_groups: HashSet<String> = HashSet::new();

    for (obj_idx, obj) in inputs.iter().enumerate() {
        for sec in obj.sections.iter() {
            if sec.sh_type != SHT_GROUP {
                continue;
            }
            if sec.data.len() < 4 {
                continue;
            }
            let flags = read_u32(&sec.data, 0);
            if flags & 1 == 0 {
                continue;
            }
            let sig_name = if (sec.info as usize) < obj.symbols.len() {
                obj.symbols[sec.info as usize].name.clone()
            } else {
                continue;
            };
            if !seen_groups.insert(sig_name) {
                let mut off = 4;
                while off + 4 <= sec.data.len() {
                    let member_idx = read_u32(&sec.data, off) as usize;
                    comdat_skip.insert((obj_idx, member_idx));
                    off += 4;
                }
            }
        }
    }

    comdat_skip
}

pub(super) fn section_type_and_flags(out_name: &str, sec: &InputSection) -> (u32, u32) {
    match out_name {
        ".text" => (SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR),
        ".rodata" => (SHT_PROGBITS, SHF_ALLOC),
        ".data" => (SHT_PROGBITS, SHF_ALLOC | SHF_WRITE),
        ".bss" => (SHT_NOBITS, SHF_ALLOC | SHF_WRITE),
        ".tdata" => (SHT_PROGBITS, SHF_ALLOC | SHF_WRITE | SHF_TLS),
        ".tbss" => (SHT_NOBITS, SHF_ALLOC | SHF_WRITE | SHF_TLS),
        ".init" | ".fini" => (SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR),
        ".init_array" => (SHT_INIT_ARRAY, SHF_ALLOC | SHF_WRITE),
        ".fini_array" => (SHT_FINI_ARRAY, SHF_ALLOC | SHF_WRITE),
        ".eh_frame" => (SHT_PROGBITS, SHF_ALLOC),
        ".note" => (SHT_NOTE, SHF_ALLOC),
        _ => (
            sec.sh_type,
            sec.flags & (SHF_ALLOC | SHF_WRITE | SHF_EXECINSTR),
        ),
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Phase 6: Symbol resolution
// ══════════════════════════════════════════════════════════════════════════════
