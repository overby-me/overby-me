//! Input processing for the i686 linker.
//!
//! Phases 1-4 of the linking pipeline: argument parsing, input file collection,
//! library resolution, object parsing, and demand-driven archive extraction.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::dynsym::*;
use super::parse::*;
use super::types::*;

// Phase 1: Argument parsing
// ══════════════════════════════════════════════════════════════════════════════

pub(super) fn parse_user_args(
    user_args: &[String],
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<(String, String)>,
) {
    let mut extra_libs = Vec::new();
    let mut extra_lib_files = Vec::new();
    let mut extra_lib_paths = Vec::new();
    let mut extra_objects = Vec::new();
    let mut defsym_defs: Vec<(String, String)> = Vec::new();

    for arg in user_args {
        if arg == "-nostdlib" || arg == "-shared" || arg == "-static" || arg == "-r" {
            continue;
        } else if let Some(libarg) = arg.strip_prefix("-l") {
            if let Some(rest) = libarg.strip_prefix(':') {
                extra_lib_files.push(rest.to_string());
            } else {
                extra_libs.push(libarg.to_string());
            }
        } else if let Some(rest) = arg.strip_prefix("-L") {
            extra_lib_paths.push(rest.to_string());
        } else if arg == "-rdynamic" || arg == "--export-dynamic" {
            // Accepted but not currently used
        } else if let Some(wl_args) = arg.strip_prefix("-Wl,") {
            let parts: Vec<&str> = wl_args.split(',').collect();
            let mut j = 0;
            while j < parts.len() {
                let part = parts[j];
                if let Some(libarg) = part.strip_prefix("-l") {
                    if let Some(rest) = libarg.strip_prefix(':') {
                        extra_lib_files.push(rest.to_string());
                    } else {
                        extra_libs.push(libarg.to_string());
                    }
                } else if let Some(rest) = part.strip_prefix("-L") {
                    extra_lib_paths.push(rest.to_string());
                } else if let Some(defsym_arg) = part.strip_prefix("--defsym=") {
                    // --defsym=SYMBOL=EXPR: define a symbol alias
                    // TODO: only supports symbol-to-symbol aliasing, not arbitrary expressions
                    if let Some(eq_pos) = defsym_arg.find('=') {
                        defsym_defs.push((
                            defsym_arg[..eq_pos].to_string(),
                            defsym_arg[eq_pos + 1..].to_string(),
                        ));
                    }
                } else if part == "--defsym" && j + 1 < parts.len() {
                    // Two-argument form: --defsym SYM=VAL
                    j += 1;
                    let defsym_arg = parts[j];
                    if let Some(eq_pos) = defsym_arg.find('=') {
                        defsym_defs.push((
                            defsym_arg[..eq_pos].to_string(),
                            defsym_arg[eq_pos + 1..].to_string(),
                        ));
                    }
                }
                j += 1;
            }
        } else if !arg.starts_with('-') && Path::new(arg.as_str()).exists() {
            extra_objects.push(arg.clone());
        }
    }

    (
        extra_libs,
        extra_lib_files,
        extra_lib_paths,
        extra_objects,
        defsym_defs,
    )
}

// ══════════════════════════════════════════════════════════════════════════════
// Phase 2: Collect input files
// ══════════════════════════════════════════════════════════════════════════════

pub(super) fn collect_input_files(
    object_files: &[&str],
    extra_objects: &[String],
    crt_before: &[&str],
    crt_after: &[&str],
    is_nostdlib: bool,
    is_static: bool,
    lib_paths: &[&str],
) -> Vec<String> {
    let mut all_objects = Vec::new();

    for path in crt_before {
        if Path::new(path).exists() {
            all_objects.push(path.to_string());
        }
    }
    for obj in object_files {
        all_objects.push(obj.to_string());
    }
    for obj in extra_objects {
        all_objects.push(obj.clone());
    }
    for path in crt_after {
        if Path::new(path).exists() {
            all_objects.push(path.to_string());
        }
    }

    // Add libc_nonshared.a for dynamic linking
    if !is_nostdlib && !is_static {
        for dir in lib_paths {
            let path = format!("{}/libc_nonshared.a", dir);
            if Path::new(&path).exists() {
                all_objects.push(path);
                break;
            }
        }
    }

    all_objects
}

// ══════════════════════════════════════════════════════════════════════════════
// Phase 3: Library resolution
// ══════════════════════════════════════════════════════════════════════════════

pub(super) fn load_libraries(
    is_static: bool,
    _is_nostdlib: bool,
    needed_libs: &[&str],
    extra_libs: &[String],
    extra_lib_files: &[String],
    all_lib_dirs: &[String],
) -> (
    HashMap<String, (String, u8, u32, Option<String>, bool, u8)>,
    Vec<String>,
) {
    let mut dynlib_syms: HashMap<String, (String, u8, u32, Option<String>, bool, u8)> =
        HashMap::new();
    let mut static_lib_objects: Vec<String> = Vec::new();
    let all_lib_refs: Vec<&str> = all_lib_dirs.iter().map(|s| s.as_str()).collect();

    if !is_static {
        let mut libs_to_scan: Vec<String> = needed_libs.iter().map(|s| s.to_string()).collect();
        libs_to_scan.extend(extra_libs.iter().cloned());

        for lib in &libs_to_scan {
            if !scan_shared_lib(lib, &all_lib_refs, &mut dynlib_syms) {
                // No .so found, try static archive
                let ar_filename = format!("lib{}.a", lib);
                for dir in &all_lib_refs {
                    let path = format!("{}/{}", dir, ar_filename);
                    if Path::new(&path).exists() {
                        static_lib_objects.push(path);
                        break;
                    }
                }
            }
        }
    }

    // Handle -l flags in static linking mode
    if is_static {
        let mut libs_to_scan: Vec<String> = needed_libs.iter().map(|s| s.to_string()).collect();
        libs_to_scan.extend(extra_libs.iter().cloned());
        for lib in &libs_to_scan {
            let ar_filename = format!("lib{}.a", lib);
            for dir in &all_lib_refs {
                let path = format!("{}/{}", dir, ar_filename);
                if Path::new(&path).exists() {
                    static_lib_objects.push(path);
                    break;
                }
            }
        }
    }

    // Handle -l:filename
    for filename in extra_lib_files {
        for dir in &all_lib_refs {
            let path = format!("{}/{}", dir, filename);
            if Path::new(&path).exists() {
                if filename.ends_with(".a") || filename.ends_with(".o") {
                    static_lib_objects.push(path);
                } else if !is_static {
                    let real_path = std::fs::canonicalize(&path).ok();
                    let check_path = real_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or(path.clone());
                    if let Ok(syms) = read_dynsyms_with_search(&check_path, &all_lib_refs) {
                        let lib_soname = filename.clone();
                        for sym in syms {
                            insert_dynsym(&mut dynlib_syms, sym, &lib_soname);
                        }
                    }
                    static_lib_objects.push(path);
                } else {
                    static_lib_objects.push(path);
                }
                break;
            }
        }
    }

    (dynlib_syms, static_lib_objects)
}

/// Try to find and scan a shared library, returning true if found.
pub(super) fn scan_shared_lib(
    lib: &str,
    lib_refs: &[&str],
    dynlib_syms: &mut HashMap<String, (String, u8, u32, Option<String>, bool, u8)>,
) -> bool {
    let so_base = format!("lib{}.so", lib);
    for dir in lib_refs {
        let mut candidates: Vec<String> = vec![format!("{}/{}", dir, so_base)];
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let fname = entry.file_name().to_string_lossy().into_owned();
                if fname.starts_with(&so_base)
                    && fname.len() > so_base.len()
                    && fname.as_bytes()[so_base.len()] == b'.'
                {
                    candidates.push(format!("{}/{}", dir, fname));
                }
            }
        }
        for cand in &candidates {
            let real_path = std::fs::canonicalize(cand).ok();
            let check_path = real_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(cand.clone());
            if let Ok(syms) = read_dynsyms_with_search(&check_path, lib_refs) {
                // Read the actual SONAME from the ELF file; fall back to hardcoded defaults
                let lib_soname = parse_soname_elf32(&check_path).unwrap_or_else(|| {
                    if lib == "c" {
                        "libc.so.6".to_string()
                    } else if lib == "m" {
                        "libm.so.6".to_string()
                    } else {
                        format!("lib{}.so", lib)
                    }
                });
                for sym in syms {
                    insert_dynsym(dynlib_syms, sym, &lib_soname);
                }
                return true;
            }
        }
    }
    false
}

pub(super) fn insert_dynsym(
    dynlib_syms: &mut HashMap<String, (String, u8, u32, Option<String>, bool, u8)>,
    sym: DynSymInfo,
    lib_soname: &str,
) {
    let entry = dynlib_syms.entry(sym.name.clone());
    match entry {
        std::collections::hash_map::Entry::Vacant(e) => {
            e.insert((
                lib_soname.to_string(),
                sym.sym_type,
                sym.size,
                sym.version,
                sym.is_default_ver,
                sym.binding,
            ));
        }
        std::collections::hash_map::Entry::Occupied(mut e) => {
            if sym.is_default_ver && !e.get().4 {
                e.insert((
                    lib_soname.to_string(),
                    sym.sym_type,
                    sym.size,
                    sym.version,
                    sym.is_default_ver,
                    sym.binding,
                ));
            }
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Phase 4: Parse objects with demand-driven archive extraction
// ══════════════════════════════════════════════════════════════════════════════

pub(super) fn load_and_parse_objects(
    all_objects: &[String],
    defsym_defs: &[(String, String)],
) -> Result<(Vec<InputObject>, Vec<InputObject>), String> {
    let mut inputs: Vec<InputObject> = Vec::new();
    let mut archive_pool: Vec<InputObject> = Vec::new();

    for obj_path in all_objects {
        let data =
            std::fs::read(obj_path).map_err(|e| format!("cannot read {}: {}", obj_path, e))?;
        if data.len() >= 8 && &data[0..8] == b"!<arch>\n" {
            let members = parse_archive(&data, obj_path)?;
            for (name, mdata) in members {
                let member_name = format!("{}({})", obj_path, name);
                if let Ok(obj) = parse_elf32(&mdata, &member_name) {
                    archive_pool.push(obj);
                }
            }
        } else if is_thin_archive(&data) {
            let members = parse_thin_archive_i686(&data, obj_path)?;
            for (name, mdata) in members {
                let member_name = format!("{}({})", obj_path, name);
                if let Ok(obj) = parse_elf32(&mdata, &member_name) {
                    archive_pool.push(obj);
                }
            }
        } else {
            inputs.push(parse_elf32(&data, obj_path)?);
        }
    }

    // Demand-driven archive member extraction
    resolve_archive_members(&mut inputs, &mut archive_pool, defsym_defs);

    Ok((inputs, archive_pool))
}

/// Pull in archive members that satisfy undefined symbols, iterating until stable.
pub(super) fn resolve_archive_members(
    inputs: &mut Vec<InputObject>,
    archive_pool: &mut Vec<InputObject>,
    defsym_defs: &[(String, String)],
) {
    let mut defined: HashSet<String> = HashSet::new();
    let mut undefined: HashSet<String> = HashSet::new();

    for obj in inputs.iter() {
        for sym in &obj.symbols {
            if sym.name.is_empty() || sym.sym_type == STT_FILE || sym.sym_type == STT_SECTION {
                continue;
            }
            if sym.section_index != SHN_UNDEF {
                defined.insert(sym.name.clone());
            } else {
                undefined.insert(sym.name.clone());
            }
        }
    }
    undefined.retain(|s| !defined.contains(s));

    // For --defsym aliases (e.g. fmod=__ieee754_fmod), if the alias is
    // undefined we also need the target symbol to be pulled from archives.
    for (alias, target) in defsym_defs {
        if undefined.contains(alias) && !defined.contains(target) {
            undefined.insert(target.clone());
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i < archive_pool.len() {
            let resolves = archive_pool[i].symbols.iter().any(|sym| {
                !sym.name.is_empty()
                    && sym.sym_type != STT_FILE
                    && sym.sym_type != STT_SECTION
                    && sym.section_index != SHN_UNDEF
                    && undefined.contains(&sym.name)
            });
            if resolves {
                let obj = archive_pool.remove(i);
                for sym in &obj.symbols {
                    if sym.name.is_empty()
                        || sym.sym_type == STT_FILE
                        || sym.sym_type == STT_SECTION
                    {
                        continue;
                    }
                    if sym.section_index != SHN_UNDEF {
                        defined.insert(sym.name.clone());
                        undefined.remove(&sym.name);
                    } else if !defined.contains(&sym.name) {
                        undefined.insert(sym.name.clone());
                    }
                }
                inputs.push(obj);
                changed = true;
            } else {
                i += 1;
            }
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Phase 5: Section merging
// ══════════════════════════════════════════════════════════════════════════════
