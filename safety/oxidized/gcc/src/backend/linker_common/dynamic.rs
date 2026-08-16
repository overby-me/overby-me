//! Shared dynamic linking: symbol matching, library loading, and symbol registration.
//!
//! Extracts the duplicated shared-library symbol matching logic from x86 and ARM
//! linkers into a single generic implementation. Also provides `register_symbols_elf64()`
//! for populating the global symbol table from object files.

use std::collections::HashMap;
use std::path::Path;

use super::parse_shared::{parse_shared_library_symbols, parse_soname};
use super::resolve_lib::resolve_lib;
use super::symbols::{is_linker_defined_symbol, GlobalSymbolOps};
use super::types::{DynSymbol, Elf64Object};
use crate::backend::elf::{
    parse_linker_script_entries, LinkerScriptEntry, ELF_MAGIC, SHN_COMMON, STB_WEAK, STT_FILE,
    STT_OBJECT, STT_SECTION,
};

/// Match dynamic symbols from a shared library against undefined globals.
///
/// For each undefined, non-dynamic global that matches a library export:
/// 1. Replace it with a dynamic symbol entry (via `GlobalSymbolOps::new_dynamic`)
/// 2. Track WEAK STT_OBJECT matches for alias registration
///
/// After the first pass, a second pass registers any STT_OBJECT aliases at the
/// same (value, size) as matched WEAK symbols. This ensures COPY relocations
/// work correctly (e.g., `environ` is WEAK, `__environ` is GLOBAL in libc).
///
/// Returns `true` if at least one symbol was matched (i.e., this library is needed).
pub fn match_shared_library_dynsyms<G: GlobalSymbolOps>(
    dyn_syms: &[DynSymbol],
    soname: &str,
    globals: &mut HashMap<String, G>,
) -> bool {
    let mut lib_needed = false;
    let mut matched_weak_objects: Vec<(u64, u64)> = Vec::new();

    // First pass: match undefined symbols against library exports
    for dsym in dyn_syms {
        if let Some(existing) = globals.get(&dsym.name) {
            if !existing.is_defined() && !existing.is_dynamic() {
                lib_needed = true;
                globals.insert(dsym.name.clone(), G::new_dynamic(dsym, soname));
                // Track WEAK STT_OBJECT for alias detection
                let bind = dsym.info >> 4;
                let stype = dsym.info & 0xf;
                if bind == STB_WEAK
                    && stype == STT_OBJECT
                    && !matched_weak_objects.contains(&(dsym.value, dsym.size))
                {
                    matched_weak_objects.push((dsym.value, dsym.size));
                }
            }
        }
    }

    // Second pass: register aliases for matched WEAK STT_OBJECT symbols
    if !matched_weak_objects.is_empty() {
        for dsym in dyn_syms {
            let stype = dsym.info & 0xf;
            if stype == STT_OBJECT
                && matched_weak_objects.contains(&(dsym.value, dsym.size))
                && !globals.contains_key(&dsym.name)
            {
                lib_needed = true;
                globals.insert(dsym.name.clone(), G::new_dynamic(dsym, soname));
            }
        }
    }

    lib_needed
}

/// Load a shared library file and match its exports against undefined globals.
///
/// Handles linker script indirection (e.g., libc.so may be a text file pointing
/// to the real .so). Uses as-needed semantics: only adds DT_NEEDED if at least
/// one symbol was actually resolved.
pub fn load_shared_library_elf64<G: GlobalSymbolOps>(
    path: &str,
    globals: &mut HashMap<String, G>,
    needed_sonames: &mut Vec<String>,
    lib_paths: &[String],
) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|e| format!("failed to read '{}': {}", path, e))?;

    // Handle linker scripts (e.g., libc.so is often a text file with GROUP/INPUT)
    if data.len() >= 4 && data[0..4] != ELF_MAGIC {
        if let Ok(text) = std::str::from_utf8(&data) {
            if let Some(entries) = parse_linker_script_entries(text) {
                let script_dir = Path::new(path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string());
                for entry in &entries {
                    let resolved_path = match entry {
                        LinkerScriptEntry::Path(lib_path) => {
                            if Path::new(lib_path).exists() {
                                Some(lib_path.clone())
                            } else if let Some(ref dir) = script_dir {
                                let p = format!("{}/{}", dir, lib_path);
                                if Path::new(&p).exists() {
                                    Some(p)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                        LinkerScriptEntry::Lib(lib_name) => resolve_lib(lib_name, lib_paths, false),
                    };
                    if let Some(resolved) = resolved_path {
                        let lib_data = std::fs::read(&resolved)
                            .map_err(|e| format!("failed to read '{}': {}", resolved, e))?;
                        if lib_data.len() >= 8 && &lib_data[0..8] == b"!<arch>\n" {
                            // Archives in linker scripts (like libc_nonshared.a)
                            // are silently skipped during shared lib loading
                            continue;
                        }
                        load_shared_library_elf64(&resolved, globals, needed_sonames, lib_paths)?;
                    }
                }
                return Ok(());
            }
        }
        return Err(format!("{}: not a valid ELF shared library", path));
    }

    let soname = parse_soname(&data).unwrap_or_else(|| {
        Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string())
    });

    let dyn_syms = parse_shared_library_symbols(&data, path)?;
    let lib_needed = match_shared_library_dynsyms(&dyn_syms, &soname, globals);

    if lib_needed && !needed_sonames.contains(&soname) {
        needed_sonames.push(soname);
    }
    Ok(())
}

/// Resolve remaining undefined symbols by searching default system libraries.
///
/// After all explicit -l libraries have been loaded, this function searches
/// the standard system libraries (libc, libm, libgcc_s) for any remaining
/// undefined, non-weak, non-linker-defined symbols.
///
/// `lib_search_paths` provides directories to search for the default libs.
/// `default_lib_names` lists the .so filenames to try (e.g., ["libc.so.6"]).
pub fn resolve_dynamic_symbols_elf64<G: GlobalSymbolOps>(
    globals: &mut HashMap<String, G>,
    needed_sonames: &mut Vec<String>,
    lib_search_paths: &[String],
    default_lib_names: &[&str],
) -> Result<(), String> {
    // Check if there are any truly undefined symbols worth resolving
    let has_undefined = globals.iter().any(|(name, sym)| {
        !sym.is_defined() && !sym.is_dynamic() && !is_linker_defined_symbol(name)
    });
    if !has_undefined {
        return Ok(());
    }

    // Find default libraries in the search paths
    for lib_name in default_lib_names {
        let lib_path = lib_search_paths
            .iter()
            .map(|dir| format!("{}/{}", dir, lib_name))
            .find(|candidate| Path::new(candidate).exists());

        if let Some(lib_path) = lib_path {
            let data = match std::fs::read(&lib_path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let soname = parse_soname(&data).unwrap_or_else(|| {
                Path::new(&lib_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            });
            let dyn_syms = match parse_shared_library_symbols(&data, &lib_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let lib_needed = match_shared_library_dynsyms(&dyn_syms, &soname, globals);
            if lib_needed && !needed_sonames.contains(&soname) {
                needed_sonames.push(soname);
            }
        }
    }
    Ok(())
}

/// Register symbols from an object file into the global symbol table.
///
/// Handles defined symbols, COMMON symbols, and undefined references.
/// For defined symbols, a GLOBAL definition replaces a WEAK one.
/// The `should_replace_extra` callback allows x86's linker to also check
/// `is_dynamic` when deciding whether to replace an existing symbol.
pub fn register_symbols_elf64<G: GlobalSymbolOps>(
    obj_idx: usize,
    obj: &Elf64Object,
    globals: &mut HashMap<String, G>,
    should_replace_extra: fn(existing: &G) -> bool,
) {
    for sym in &obj.symbols {
        if sym.sym_type() == STT_SECTION || sym.sym_type() == STT_FILE {
            continue;
        }
        if sym.name.is_empty() || sym.is_local() {
            continue;
        }

        let is_defined = !sym.is_undefined() && sym.shndx != SHN_COMMON;

        if is_defined {
            let should_replace = match globals.get(&sym.name) {
                None => true,
                Some(e) => {
                    !e.is_defined()
                        || should_replace_extra(e)
                        || (e.info() >> 4 == STB_WEAK && sym.is_global())
                }
            };
            if should_replace {
                globals.insert(sym.name.clone(), G::new_defined(obj_idx, sym));
            }
        } else if sym.shndx == SHN_COMMON {
            let should_insert = match globals.get(&sym.name) {
                None => true,
                Some(e) => !e.is_defined(),
            };
            if should_insert {
                globals.insert(sym.name.clone(), G::new_common(obj_idx, sym));
            }
        } else if !globals.contains_key(&sym.name) {
            globals.insert(sym.name.clone(), G::new_undefined(sym));
        }
    }
}
