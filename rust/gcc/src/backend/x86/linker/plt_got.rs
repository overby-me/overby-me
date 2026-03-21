//! PLT/GOT construction and IFUNC collection for the x86-64 linker.
//!
//! Scans object file relocations to determine which symbols need PLT stubs
//! or GOT entries, and collects IFUNC symbols for IRELATIVE relocations.

use std::collections::HashMap;

use super::elf::*;
use super::types::GlobalSymbol;

pub(super) fn collect_ifunc_symbols(
    globals: &HashMap<String, GlobalSymbol>,
    is_static: bool,
) -> Vec<String> {
    if !is_static {
        return Vec::new();
    }
    let mut ifunc_symbols: Vec<String> = globals
        .iter()
        .filter(|(_, g)| g.defined_in.is_some() && (g.info & 0xf) == STT_GNU_IFUNC)
        .map(|(n, _)| n.clone())
        .collect();
    ifunc_symbols.sort();
    ifunc_symbols
}

pub(super) fn create_plt_got(
    objects: &[ElfObject],
    globals: &mut HashMap<String, GlobalSymbol>,
) -> (Vec<String>, Vec<(String, bool)>) {
    let mut plt_names: Vec<String> = Vec::new();
    let mut got_only_names: Vec<String> = Vec::new();
    let mut copy_reloc_names: Vec<String> = Vec::new();

    for obj in objects {
        for sec_idx in 0..obj.sections.len() {
            for rela in &obj.relocations[sec_idx] {
                let si = rela.sym_idx as usize;
                if si >= obj.symbols.len() {
                    continue;
                }
                let sym = &obj.symbols[si];
                if sym.name.is_empty() || sym.is_local() {
                    continue;
                }
                let gsym_info = globals.get(&sym.name).map(|g| (g.is_dynamic, g.info & 0xf));

                match rela.rela_type {
                    R_X86_64_PLT32 | R_X86_64_PC32 if gsym_info.map(|g| g.0).unwrap_or(false) => {
                        let sym_type = gsym_info.map(|g| g.1).unwrap_or(0);
                        if sym_type == STT_OBJECT {
                            // Dynamic data symbol - needs copy relocation
                            if !copy_reloc_names.contains(&sym.name) {
                                copy_reloc_names.push(sym.name.clone());
                            }
                        } else {
                            // Dynamic function symbol - needs PLT
                            if !plt_names.contains(&sym.name) {
                                plt_names.push(sym.name.clone());
                            }
                        }
                    }
                    R_X86_64_GOTPCREL | R_X86_64_GOTPCRELX | R_X86_64_REX_GOTPCRELX => {
                        // GOTPCREL always needs a dedicated GOT entry, even if the
                        // symbol also has a PLT entry. The PLT's GOT.PLT slot uses
                        // JUMP_SLOT (lazy binding, initially PLT+6) which is wrong
                        // for address-of. For symbols with PLT, the GOT entry is
                        // statically filled with the PLT address (no GLOB_DAT);
                        // for other dynamic symbols, GLOB_DAT is used.
                        if !got_only_names.contains(&sym.name) {
                            got_only_names.push(sym.name.clone());
                        }
                    }
                    R_X86_64_GOTTPOFF => {
                        if !got_only_names.contains(&sym.name) && !plt_names.contains(&sym.name) {
                            got_only_names.push(sym.name.clone());
                        }
                    }
                    _ if gsym_info.map(|g| g.0).unwrap_or(false) => {
                        let sym_type = gsym_info.map(|g| g.1).unwrap_or(0);
                        if sym_type != STT_OBJECT && rela.rela_type == R_X86_64_64 {
                            // R_X86_64_64 for dynamic function (e.g. function pointer init) needs PLT
                            if !plt_names.contains(&sym.name) {
                                plt_names.push(sym.name.clone());
                            }
                        } else if !plt_names.contains(&sym.name)
                            && !got_only_names.contains(&sym.name)
                        {
                            got_only_names.push(sym.name.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Mark copy relocation symbols and their aliases.
    // When a symbol like `environ` (WEAK) needs a COPY relocation, we must also
    // mark aliases like `__environ` (GLOBAL) at the same shared library address.
    // This ensures the dynamic linker redirects all references to our BSS copy.
    let mut copy_reloc_lib_addrs: Vec<(String, u64)> = Vec::new(); // (from_lib, lib_sym_value)
    for name in &copy_reloc_names {
        if let Some(gsym) = globals.get_mut(name) {
            gsym.copy_reloc = true;
            if let Some(ref lib) = gsym.from_lib {
                if (gsym.info & 0xf) == STT_OBJECT && gsym.lib_sym_value != 0 {
                    let key = (lib.clone(), gsym.lib_sym_value);
                    if !copy_reloc_lib_addrs.contains(&key) {
                        copy_reloc_lib_addrs.push(key);
                    }
                }
            }
        }
    }
    // Also mark aliases (other dynamic STT_OBJECT symbols at the same library address)
    if !copy_reloc_lib_addrs.is_empty() {
        let alias_names: Vec<String> = globals
            .iter()
            .filter(|(name, g)| {
                g.is_dynamic
                    && !g.copy_reloc
                    && (g.info & 0xf) == STT_OBJECT
                    && !copy_reloc_names.contains(name)
                    && g.from_lib.is_some()
                    && g.lib_sym_value != 0
                    && copy_reloc_lib_addrs
                        .contains(&(g.from_lib.as_ref().unwrap().clone(), g.lib_sym_value))
            })
            .map(|(n, _)| n.clone())
            .collect();
        for name in alias_names {
            if let Some(gsym) = globals.get_mut(&name) {
                gsym.copy_reloc = true;
            }
        }
    }

    let mut got_entries: Vec<(String, bool)> = Vec::new();
    got_entries.push((String::new(), false)); // GOT[0]
    got_entries.push((String::new(), false)); // GOT[1]
    got_entries.push((String::new(), false)); // GOT[2]

    for (plt_idx, name) in plt_names.iter().enumerate() {
        let got_idx = got_entries.len();
        got_entries.push((name.clone(), true));
        if let Some(gsym) = globals.get_mut(name) {
            gsym.plt_idx = Some(plt_idx);
            gsym.got_idx = Some(got_idx);
        }
    }

    for name in &got_only_names {
        let got_idx = got_entries.len();
        got_entries.push((name.clone(), false));
        if let Some(gsym) = globals.get_mut(name) {
            gsym.got_idx = Some(got_idx);
        }
    }

    (plt_names, got_entries)
}
