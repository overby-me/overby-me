//! i686 linker orchestration.
//!
//! Contains the two public entry points (`link_builtin` and `link_shared`) that
//! orchestrate the linking pipeline: parse arguments, load inputs, merge sections,
//! resolve symbols, build PLT/GOT, and emit the ELF32 executable or shared library.

use std::collections::HashMap;
use std::path::Path;

use super::emit::emit_executable;
use super::input::*;
use super::sections::merge_sections;
use super::shared::{emit_shared_library_32, resolve_dynamic_symbols_for_shared};
use super::symbols::*;
use super::types::*;

/// Built-in linker entry point with pre-resolved CRT objects and library paths.
pub fn link_builtin(
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
    lib_paths: &[&str],
    needed_libs_param: &[&str],
    crt_objects_before: &[&str],
    crt_objects_after: &[&str],
) -> Result<(), String> {
    let is_nostdlib = user_args.iter().any(|a| a == "-nostdlib");
    let is_static = user_args.iter().any(|a| a == "-static");

    // Phase 1: Parse arguments and collect file lists
    let (extra_libs, extra_lib_files, extra_lib_paths, extra_objects, defsym_defs) =
        parse_user_args(user_args);

    let all_lib_dirs: Vec<String> = extra_lib_paths
        .into_iter()
        .chain(lib_paths.iter().map(|s| s.to_string()))
        .collect();

    // Phase 2: Collect all input objects in link order
    let all_objects = collect_input_files(
        object_files,
        &extra_objects,
        crt_objects_before,
        crt_objects_after,
        is_nostdlib,
        is_static,
        lib_paths,
    );

    // Phase 3: Load dynamic library symbols and resolve static libs from -l flags
    let (dynlib_syms, static_lib_objects) = load_libraries(
        is_static,
        is_nostdlib,
        needed_libs_param,
        &extra_libs,
        &extra_lib_files,
        &all_lib_dirs,
    );

    // Phase 4: Parse all input objects and archives
    let mut all_objs = all_objects;
    for lib_path in &static_lib_objects {
        all_objs.push(lib_path.clone());
    }

    let (inputs, _archive_pool) = load_and_parse_objects(&all_objs, &defsym_defs)?;

    // Phase 5: Merge sections
    let (mut output_sections, mut section_name_to_idx, section_map) = merge_sections(&inputs);

    // Phase 6: Resolve symbols
    let (mut global_symbols, sym_resolution) =
        resolve_symbols(&inputs, &output_sections, &section_map, &dynlib_syms);

    // Phase 6b: Allocate COMMON symbols in .bss
    allocate_common_symbols(
        &inputs,
        &mut output_sections,
        &mut section_name_to_idx,
        &mut global_symbols,
    );

    // Phase 7: Mark PLT/GOT needs and check undefined
    mark_plt_got_needs(&inputs, &mut global_symbols, is_static);

    // Apply --defsym definitions: alias one symbol to another
    for (alias, target) in &defsym_defs {
        if let Some(target_sym) = global_symbols.get(target).cloned() {
            global_symbols.insert(alias.clone(), target_sym);
        }
    }

    check_undefined_symbols(&global_symbols)?;

    // Phase 8: Build PLT/GOT structures
    let (plt_symbols, got_dyn_symbols, got_local_symbols, num_plt, num_got_total) =
        build_plt_got_lists(&mut global_symbols);

    // Phase 8b: Mark WEAK dynamic data symbols for text relocations instead of COPY
    if !is_static {
        let weak_data_syms: Vec<String> = global_symbols
            .iter()
            .filter(|(_, s)| {
                s.is_dynamic
                    && s.needs_copy
                    && s.binding == STB_WEAK
                    && s.sym_type != STT_FUNC
                    && s.sym_type != STT_GNU_IFUNC
            })
            .map(|(n, _)| n.clone())
            .collect();
        for name in &weak_data_syms {
            if let Some(sym) = global_symbols.get_mut(name) {
                sym.needs_copy = false;
                sym.uses_textrel = true;
            }
        }
    }

    // Phase 9: Collect IFUNC symbols for static linking
    let ifunc_symbols = collect_ifunc_symbols(&global_symbols, is_static);

    // Phase 10: Layout + emit
    emit_executable(
        &inputs,
        &mut output_sections,
        &section_name_to_idx,
        &section_map,
        &mut global_symbols,
        &sym_resolution,
        &dynlib_syms,
        &plt_symbols,
        &got_dyn_symbols,
        &got_local_symbols,
        num_plt,
        num_got_total,
        &ifunc_symbols,
        is_static,
        is_nostdlib,
        needed_libs_param,
        output_path,
    )
}

// ══════════════════════════════════════════════════════════════════════════════
// Shared library linker (-shared)
// ══════════════════════════════════════════════════════════════════════════════

/// Create a shared library (.so) from ELF32 object files.
///
/// Produces an ELF32 `ET_DYN` file with base address 0, exporting all defined
/// global symbols. Used when the compiler is invoked with `-shared`.
pub fn link_shared(
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
    lib_paths: &[&str],
) -> Result<(), String> {
    // Parse user args for -L, -l, -Wl,-soname=, bare .o/.a files
    let mut extra_lib_paths: Vec<String> = Vec::new();
    let mut libs_to_load: Vec<String> = Vec::new();
    let mut extra_object_files: Vec<String> = Vec::new();
    let mut soname: Option<String> = None;
    let mut i = 0;
    let args: Vec<&str> = user_args.iter().map(|s| s.as_str()).collect();
    while i < args.len() {
        let arg = args[i];
        if let Some(path) = arg.strip_prefix("-L") {
            let p = if path.is_empty() && i + 1 < args.len() {
                i += 1;
                args[i]
            } else {
                path
            };
            extra_lib_paths.push(p.to_string());
        } else if let Some(lib) = arg.strip_prefix("-l") {
            let l = if lib.is_empty() && i + 1 < args.len() {
                i += 1;
                args[i]
            } else {
                lib
            };
            libs_to_load.push(l.to_string());
        } else if let Some(wl_arg) = arg.strip_prefix("-Wl,") {
            let parts: Vec<&str> = wl_arg.split(',').collect();
            let mut j = 0;
            while j < parts.len() {
                let part = parts[j];
                if let Some(sn) = part.strip_prefix("-soname=") {
                    soname = Some(sn.to_string());
                } else if part == "-soname" && j + 1 < parts.len() {
                    j += 1;
                    soname = Some(parts[j].to_string());
                } else if let Some(lpath) = part.strip_prefix("-L") {
                    extra_lib_paths.push(lpath.to_string());
                } else if let Some(lib) = part.strip_prefix("-l") {
                    libs_to_load.push(lib.to_string());
                }
                j += 1;
            }
        } else if arg == "-shared" || arg == "-nostdlib" || arg == "-o" {
            if arg == "-o" {
                i += 1;
            }
        } else if !arg.starts_with('-') && Path::new(arg).exists() {
            extra_object_files.push(arg.to_string());
        }
        i += 1;
    }

    // Collect all objects to parse
    let mut all_objs: Vec<String> = object_files.iter().map(|s| s.to_string()).collect();
    all_objs.extend(extra_object_files);

    // Parse all input objects
    let defsym_defs: Vec<(String, String)> = Vec::new();
    let (inputs, _archive_pool) = load_and_parse_objects(&all_objs, &defsym_defs)?;

    // Merge sections
    let (mut output_sections, section_name_to_idx, section_map) = merge_sections(&inputs);

    // Resolve symbols (no dynamic library symbols for shared lib output)
    let dynlib_syms: HashMap<String, (String, u8, u32, Option<String>, bool, u8)> = HashMap::new();
    let (mut global_symbols, _sym_resolution) =
        resolve_symbols(&inputs, &output_sections, &section_map, &dynlib_syms);

    // Load -l libraries (resolve into archives and load them)
    let lib_path_strings: Vec<String> = lib_paths.iter().map(|s| s.to_string()).collect();
    let mut all_lib_paths: Vec<String> = extra_lib_paths;
    all_lib_paths.extend(lib_path_strings.iter().cloned());

    if !libs_to_load.is_empty() {
        for lib_name in &libs_to_load {
            // Search for static archive only in shared library mode
            for dir in &all_lib_paths {
                let cand = format!("{}/lib{}.a", dir, lib_name);
                if Path::new(&cand).exists() {
                    let objs = vec![cand];
                    let (extra_inputs, _) = load_and_parse_objects(&objs, &defsym_defs)?;
                    // Add symbols from these archives
                    for _inp in &extra_inputs {
                        // TODO: properly merge archive objects
                    }
                    break;
                }
            }
        }
    }

    // Discover NEEDED dependencies by scanning for undefined symbols
    let mut needed_sonames: Vec<String> = Vec::new();
    resolve_dynamic_symbols_for_shared(
        &inputs,
        &global_symbols,
        &mut needed_sonames,
        &all_lib_paths,
    );

    // Emit shared library
    emit_shared_library_32(
        &inputs,
        &mut global_symbols,
        &mut output_sections,
        &section_name_to_idx,
        &section_map,
        &needed_sonames,
        output_path,
        soname,
    )
}
