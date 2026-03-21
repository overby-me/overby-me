//! Phase 2: Section merging.
//!
//! Collects all allocatable sections from input objects, groups them by output
//! section name, and merges their data with proper alignment. Used by both
//! executable and shared library linking.

use super::elf_read::*;
use super::relocations::{output_section_name, InputSecRef, MergedSection};
use std::collections::HashMap;

/// Merge sections from all input objects into output sections.
///
/// Groups input sections by their canonical output name (e.g., `.text.foo` → `.text`),
/// concatenates their data with proper alignment, and returns the merged sections
/// along with a mapping of input section indices to merged section positions.
pub fn merge_sections(
    input_objs: &[(String, ElfObject)],
) -> (Vec<MergedSection>, HashMap<String, usize>, Vec<InputSecRef>) {
    let mut merged_sections: Vec<MergedSection> = Vec::new();
    let mut merged_map: HashMap<String, usize> = HashMap::new();
    let mut input_sec_refs: Vec<InputSecRef> = Vec::new();

    for (obj_idx, (_, obj)) in input_objs.iter().enumerate() {
        for (sec_idx, sec) in obj.sections.iter().enumerate() {
            if sec.name.is_empty()
                || sec.sh_type == SHT_SYMTAB
                || sec.sh_type == SHT_STRTAB
                || sec.sh_type == SHT_RELA
                || sec.sh_type == SHT_GROUP
            {
                continue;
            }

            let out_name = match output_section_name(&sec.name, sec.sh_type, sec.flags) {
                Some(n) => n,
                None => continue,
            };

            let sec_data = &obj.section_data[sec_idx];

            // Skip .note.GNU-stack (we generate our own)
            if out_name == ".note.GNU-stack" {
                continue;
            }

            // .riscv.attributes and .note.* sections: keep only the first occurrence
            if out_name == ".riscv.attributes" || out_name.starts_with(".note.") {
                if !merged_map.contains_key(&out_name) {
                    let idx = merged_sections.len();
                    merged_map.insert(out_name.clone(), idx);
                    merged_sections.push(MergedSection {
                        name: out_name.clone(),
                        sh_type: sec.sh_type,
                        sh_flags: sec.flags,
                        data: sec_data.clone(),
                        vaddr: 0,
                        align: sec.addralign.max(1),
                    });
                    input_sec_refs.push(InputSecRef {
                        obj_idx,
                        sec_idx,
                        merged_sec_idx: idx,
                        offset_in_merged: 0,
                    });
                }
                continue;
            }

            let merged_idx = if let Some(&idx) = merged_map.get(&out_name) {
                idx
            } else {
                let idx = merged_sections.len();
                let is_bss = sec.sh_type == SHT_NOBITS || out_name == ".bss" || out_name == ".sbss";
                merged_map.insert(out_name.clone(), idx);
                merged_sections.push(MergedSection {
                    name: out_name.clone(),
                    sh_type: if is_bss { SHT_NOBITS } else { sec.sh_type },
                    sh_flags: sec.flags,
                    data: Vec::new(),
                    vaddr: 0,
                    align: sec.addralign.max(1),
                });
                idx
            };

            let ms = &mut merged_sections[merged_idx];
            // Union flags from all input sections
            ms.sh_flags |= sec.flags & (SHF_WRITE | SHF_ALLOC | SHF_EXECINSTR | SHF_TLS);
            ms.align = ms.align.max(sec.addralign.max(1));

            // Align within merged section data
            let align = sec.addralign.max(1) as usize;
            let cur_len = ms.data.len();
            let aligned = (cur_len + align - 1) & !(align - 1);
            if aligned > cur_len {
                ms.data.resize(aligned, 0);
            }
            let offset_in_merged = ms.data.len() as u64;

            // Append section data
            if sec.sh_type == SHT_NOBITS {
                ms.data.resize(ms.data.len() + sec.size as usize, 0);
            } else {
                ms.data.extend_from_slice(sec_data);
            }

            input_sec_refs.push(InputSecRef {
                obj_idx,
                sec_idx,
                merged_sec_idx: merged_idx,
                offset_in_merged,
            });
        }
    }

    (merged_sections, merged_map, input_sec_refs)
}
