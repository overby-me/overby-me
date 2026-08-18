//! Phase 1: Input file loading, archive resolution, and shared library symbol discovery.
//!
//! Handles reading ELF objects, static archives (both regular and thin), and shared
//! libraries. Resolves undefined symbols by demand-loading archive members, with
//! group iteration to handle circular dependencies between libraries.

use super::elf_read::*;
use super::relocations::resolve_archive_members;
use crate::backend::linker_common;
use std::collections::{HashMap, HashSet};

/// Load a single input file (ELF object, archive, or thin archive) into
/// `input_objs`. Archives are saved to `inline_archive_paths` for later
/// demand-driven extraction.
pub fn load_input_files(
    all_inputs: &[String],
    input_objs: &mut Vec<(String, ElfObject)>,
    inline_archive_paths: &mut Vec<String>,
) -> Result<(), String> {
    for path in all_inputs {
        if !std::path::Path::new(path).exists() {
            return Err(format!("linker input file not found: {}", path));
        }
        let data = std::fs::read(path).map_err(|e| format!("Cannot read {}: {}", path, e))?;

        if (data.len() >= 8 && &data[0..8] == b"!<arch>\n") || is_thin_archive(&data) {
            inline_archive_paths.push(path.clone());
        } else if data.len() >= 4 && &data[0..4] == b"\x7fELF" {
            let obj = parse_object(&data, path).map_err(|e| format!("{}: {}", path, e))?;
            input_objs.push((path.clone(), obj));
        }
        // Skip non-ELF/non-archive files (e.g. linker scripts)
    }
    Ok(())
}

/// Scan input objects to build initial defined/undefined symbol sets.
pub fn collect_initial_symbols(
    input_objs: &[(String, ElfObject)],
    defined_syms: &mut HashSet<String>,
    undefined_syms: &mut HashSet<String>,
) {
    for (_, obj) in input_objs {
        for sym in &obj.symbols {
            if sym.shndx != SHN_UNDEF && sym.binding() != STB_LOCAL && !sym.name.is_empty() {
                defined_syms.insert(sym.name.clone());
            }
        }
    }
    for (_, obj) in input_objs {
        for sym in &obj.symbols {
            if sym.shndx == SHN_UNDEF
                && !sym.name.is_empty()
                && sym.binding() != STB_LOCAL
                && !defined_syms.contains(&sym.name)
            {
                undefined_syms.insert(sym.name.clone());
            }
        }
    }
}

/// Discover symbols exported by shared libraries referenced via `-l` flags.
///
/// For each needed library, tries to find `.so` files in the search paths and
/// reads their dynamic symbol tables. Handles GNU linker scripts that reference
/// other `.so` files or `-l` flags. Returns the actual NEEDED soname strings.
///
/// Also loads archive members from `_nonshared.a` files referenced in linker scripts.
pub fn discover_shared_lib_symbols(
    needed_libs: &[String],
    lib_search_paths: &[String],
    input_objs: &mut Vec<(String, ElfObject)>,
    defined_syms: &mut HashSet<String>,
    undefined_syms: &mut HashSet<String>,
    shared_lib_syms: &mut HashMap<String, DynSymbol>,
    actual_needed_libs: &mut Vec<String>,
) {
    use super::relocations::find_versioned_soname;

    for libname in needed_libs {
        let so_name = if let Some(exact) = libname.strip_prefix(':') {
            exact.to_string()
        } else {
            format!("lib{}.so", libname)
        };
        for dir in lib_search_paths {
            let path = format!("{}/{}", dir, so_name);
            if !std::path::Path::new(&path).exists() {
                continue;
            }
            let data = match std::fs::read(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            if data.starts_with(b"/* GNU ld script")
                || data.starts_with(b"OUTPUT_FORMAT")
                || data.starts_with(b"GROUP")
                || data.starts_with(b"INPUT")
            {
                process_linker_script(
                    &data,
                    dir,
                    lib_search_paths,
                    input_objs,
                    defined_syms,
                    undefined_syms,
                    shared_lib_syms,
                );
            } else if data.len() >= 4 && &data[0..4] == b"\x7fELF" {
                if let Ok(syms) = read_shared_lib_symbols(&path) {
                    for si in syms {
                        shared_lib_syms.insert(si.name.clone(), si);
                    }
                }
            }
            let versioned = find_versioned_soname(dir, libname);
            if let Some(soname) = versioned {
                if !actual_needed_libs.contains(&soname) {
                    actual_needed_libs.push(soname);
                }
            }
            break;
        }
    }
}

/// Process a GNU linker script, extracting `.so` references and `-l` library flags.
fn process_linker_script(
    data: &[u8],
    dir: &str,
    lib_search_paths: &[String],
    input_objs: &mut Vec<(String, ElfObject)>,
    defined_syms: &mut HashSet<String>,
    undefined_syms: &mut HashSet<String>,
    shared_lib_syms: &mut HashMap<String, DynSymbol>,
) {
    let text = String::from_utf8_lossy(data);
    for token in text.split_whitespace() {
        let token = token.trim_matches(|c: char| c == '(' || c == ')' || c == ',');
        if let Some(lib_name) = token.strip_prefix("-l") {
            if !lib_name.is_empty() {
                let so_name = format!("lib{}.so", lib_name);
                for search_dir in lib_search_paths {
                    let candidate = format!("{}/{}", search_dir, so_name);
                    if std::path::Path::new(&candidate).exists() {
                        if let Ok(syms) = read_shared_lib_symbols(&candidate) {
                            for si in syms {
                                shared_lib_syms.insert(si.name.clone(), si);
                            }
                        }
                        break;
                    }
                }
            }
            continue;
        }
        if token.contains(".so") && (token.starts_with('/') || token.starts_with("lib")) {
            let actual_path = if token.starts_with('/') {
                token.to_string()
            } else {
                let mut found = format!("{}/{}", dir, token);
                if !std::path::Path::new(&found).exists() {
                    for search_dir in lib_search_paths {
                        let candidate = format!("{}/{}", search_dir, token);
                        if std::path::Path::new(&candidate).exists() {
                            found = candidate;
                            break;
                        }
                    }
                }
                found
            };
            if let Ok(syms) = read_shared_lib_symbols(&actual_path) {
                for si in syms {
                    shared_lib_syms.insert(si.name.clone(), si);
                }
            }
            // Also try to pull in _nonshared.a from linker script
            if actual_path.ends_with("_nonshared.a") {
                load_nonshared_archive(&actual_path, input_objs, defined_syms, undefined_syms);
            }
        }
    }
}

/// Load members from a `_nonshared.a` archive (referenced from linker scripts).
fn load_nonshared_archive(
    path: &str,
    input_objs: &mut Vec<(String, ElfObject)>,
    defined_syms: &mut HashSet<String>,
    undefined_syms: &mut HashSet<String>,
) {
    if let Ok(archive_data) = std::fs::read(path) {
        if archive_data.len() >= 8 && &archive_data[0..8] == b"!<arch>\n" {
            if let Ok(members) = parse_archive(&archive_data) {
                resolve_archive_members(members, input_objs, defined_syms, undefined_syms);
            }
        } else if is_thin_archive(&archive_data) {
            if let Ok(members) = parse_thin_archive(&archive_data, path) {
                resolve_archive_members(members, input_objs, defined_syms, undefined_syms);
            }
        }
    }
}

/// Resolve undefined symbols from `.a` archives using group iteration.
///
/// Iterates over all archives repeatedly until no new symbols are resolved,
/// handling circular dependencies (e.g., libm -> libgcc -> libc -> libgcc).
pub fn resolve_archives(
    inline_archive_paths: &[String],
    needed_libs: &[String],
    lib_search_paths: &[String],
    input_objs: &mut Vec<(String, ElfObject)>,
    defined_syms: &mut HashSet<String>,
    undefined_syms: &mut HashSet<String>,
    shared_lib_syms: &HashMap<String, DynSymbol>,
) {
    let mut archive_paths: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Add inline archives first
    for path in inline_archive_paths {
        if !seen.contains(path) {
            seen.insert(path.clone());
            archive_paths.push(path.clone());
        }
    }
    // Then -l library archives
    for libname in needed_libs {
        let archive_name = if let Some(exact) = libname.strip_prefix(':') {
            exact.to_string()
        } else {
            format!("lib{}.a", libname)
        };
        for dir in lib_search_paths {
            let path = format!("{}/{}", dir, archive_name);
            if std::path::Path::new(&path).exists() {
                if !seen.contains(&path) {
                    seen.insert(path.clone());
                    archive_paths.push(path);
                }
                break;
            }
        }
    }

    // Group iteration until stable
    let mut group_changed = true;
    while group_changed {
        group_changed = false;
        let prev_count = input_objs.len();

        // Remove symbols available from shared libraries from undefined_syms.
        // This prevents archive members from being extracted just because they
        // define symbols already available via dynamic linking (e.g., stdio.o
        // from libc.a defining stderr/stdout when libc.so provides them).
        // Must be done each iteration since newly-extracted members may add
        // shared-lib-provided symbols back into undefined_syms.
        for sym_name in shared_lib_syms.keys() {
            if !defined_syms.contains(sym_name) {
                undefined_syms.remove(sym_name);
            }
        }

        for path in &archive_paths {
            let data = match std::fs::read(path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            if data.len() >= 8 && &data[0..8] == b"!<arch>\n" {
                if let Ok(members) = parse_archive(&data) {
                    resolve_archive_members(members, input_objs, defined_syms, undefined_syms);
                }
            } else if is_thin_archive(&data) {
                if let Ok(members) = parse_thin_archive(&data, path) {
                    resolve_archive_members(members, input_objs, defined_syms, undefined_syms);
                }
            }
        }
        if input_objs.len() != prev_count {
            group_changed = true;
        }
    }
}

/// Load objects for shared library linking. Loads all archive members eagerly
/// (shared libs export everything, so we need all symbols).
pub fn load_shared_lib_inputs(
    object_files: &[&str],
    extra_object_files: &[String],
    input_objs: &mut Vec<(String, ElfObject)>,
    defined_syms: &mut HashSet<String>,
    undefined_syms: &mut HashSet<String>,
) -> Result<(), String> {
    let load = |path: &str,
                objs: &mut Vec<(String, ElfObject)>,
                defs: &mut HashSet<String>,
                undefs: &mut HashSet<String>|
     -> Result<(), String> {
        let data = std::fs::read(path).map_err(|e| format!("failed to read '{}': {}", path, e))?;
        if data.len() < 4 {
            return Ok(());
        }
        if data.starts_with(b"!<arch>") {
            let members = parse_archive(&data)?;
            for (name, obj) in members {
                register_obj_symbols(&obj, defs, undefs);
                objs.push((format!("{}({})", path, name), obj));
            }
        } else if data.starts_with(&[0x7f, b'E', b'L', b'F']) {
            let obj = parse_object(&data, path)?;
            register_obj_symbols(&obj, defs, undefs);
            objs.push((path.to_string(), obj));
        }
        Ok(())
    };

    for path in object_files {
        load(path, input_objs, defined_syms, undefined_syms)?;
    }
    for path in extra_object_files {
        load(path, input_objs, defined_syms, undefined_syms)?;
    }
    Ok(())
}

/// Register an object's symbols in the defined/undefined sets.
fn register_obj_symbols(
    obj: &ElfObject,
    defined_syms: &mut HashSet<String>,
    undefined_syms: &mut HashSet<String>,
) {
    for sym in &obj.symbols {
        if sym.shndx != SHN_UNDEF && sym.binding() != STB_LOCAL && !sym.name.is_empty() {
            defined_syms.insert(sym.name.clone());
            undefined_syms.remove(&sym.name);
        }
    }
    for sym in &obj.symbols {
        if sym.shndx == SHN_UNDEF
            && !sym.name.is_empty()
            && sym.binding() != STB_LOCAL
            && !defined_syms.contains(&sym.name)
        {
            undefined_syms.insert(sym.name.clone());
        }
    }
}

/// Resolve `-l` libraries for shared library linking, recording NEEDED sonames.
pub fn resolve_shared_lib_deps(
    libs_to_load: &[String],
    all_lib_paths: &[String],
    input_objs: &mut Vec<(String, ElfObject)>,
    defined_syms: &mut HashSet<String>,
    undefined_syms: &mut HashSet<String>,
    needed_sonames: &mut Vec<String>,
) -> Result<(), String> {
    for lib_name in libs_to_load {
        if let Some(lib_path) = linker_common::resolve_lib(lib_name, all_lib_paths, false) {
            let data = match std::fs::read(&lib_path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            if data.starts_with(b"!<arch>") {
                if let Ok(members) = parse_archive(&data) {
                    resolve_archive_members(members, input_objs, defined_syms, undefined_syms);
                }
            } else if data.starts_with(&[0x7f, b'E', b'L', b'F']) {
                if data.len() >= 18 {
                    let e_type = u16::from_le_bytes([data[16], data[17]]);
                    if e_type == 3 {
                        // ET_DYN - shared library
                        if let Some(sn) = linker_common::parse_soname(&data) {
                            if !needed_sonames.contains(&sn) {
                                needed_sonames.push(sn);
                            }
                        } else {
                            let base = std::path::Path::new(&lib_path)
                                .file_name()
                                .map(|f| f.to_string_lossy().into_owned())
                                .unwrap_or_else(|| lib_name.clone());
                            if !needed_sonames.contains(&base) {
                                needed_sonames.push(base);
                            }
                        }
                        continue;
                    }
                }
                // Relocatable object
                let load_fn = |path: &str,
                               objs: &mut Vec<(String, ElfObject)>,
                               defs: &mut HashSet<String>,
                               undefs: &mut HashSet<String>|
                 -> Result<(), String> {
                    let data = std::fs::read(path)
                        .map_err(|e| format!("failed to read '{}': {}", path, e))?;
                    if data.starts_with(&[0x7f, b'E', b'L', b'F']) {
                        let obj = parse_object(&data, path)?;
                        register_obj_symbols(&obj, defs, undefs);
                        objs.push((path.to_string(), obj));
                    }
                    Ok(())
                };
                load_fn(&lib_path, input_objs, defined_syms, undefined_syms)?;
            } else if data.starts_with(b"/* GNU ld script")
                || data.starts_with(b"GROUP")
                || data.starts_with(b"INPUT")
            {
                continue;
            }
        }
    }
    Ok(())
}
