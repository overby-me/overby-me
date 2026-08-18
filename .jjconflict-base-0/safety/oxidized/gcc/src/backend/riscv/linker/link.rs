//! RISC-V 64-bit ELF linker: orchestration of linking phases.
//!
//! Contains the two public entry points (`link_builtin` and `link_shared`) that
//! orchestrate the linking pipeline: load inputs, resolve symbols, merge sections,
//! and dispatch to the appropriate ELF emission path.
//!
//! All heavy ELF writing is delegated to:
//! - `emit_exec`: executable emission (static and dynamic)
//! - `emit_shared`: shared library (.so) emission

use super::elf_read::*;
use super::relocations::section_order;
use super::{input, sections, symbols};
use crate::backend::linker_common;
use std::collections::{HashMap, HashSet};

// ── Public entry point: executable linking ───────────────────────────────

/// Link object files into a RISC-V ELF executable (pre-resolved CRT/library variant).
///
/// This is the primary entry point, matching the pattern used by x86-64 and i686 linkers.
/// CRT objects and library paths are resolved by common.rs before being passed in.
///
/// `object_files`: paths to user .o files and .a archives
/// `output_path`: path for the output executable
/// `user_args`: additional linker flags from the user
/// `lib_paths`: pre-resolved library search paths (user -L first, then system)
/// `needed_libs`: pre-resolved default libraries (e.g., ["gcc", "gcc_s", "c", "m"])
/// `crt_objects_before`: CRT objects to link before user objects (e.g., crt1.o, crti.o, crtbegin.o)
/// `crt_objects_after`: CRT objects to link after user objects (e.g., crtend.o, crtn.o)
pub fn link_builtin(
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
    lib_paths: &[&str],
    needed_libs: &[&str],
    crt_objects_before: &[&str],
    crt_objects_after: &[&str],
) -> Result<(), String> {
    // Parse user arguments using shared infrastructure
    let parsed_args = linker_common::parse_linker_args(user_args);
    let is_static = parsed_args.is_static;
    let defsym_defs = parsed_args.defsym_defs;

    // Collect all input files: CRT before + user objects + bare files from args + CRT after
    let mut all_inputs: Vec<String> = Vec::new();
    for crt in crt_objects_before {
        all_inputs.push(crt.to_string());
    }
    for obj in object_files {
        all_inputs.push(obj.to_string());
    }
    all_inputs.extend(parsed_args.extra_object_files);
    for crt in crt_objects_after {
        all_inputs.push(crt.to_string());
    }

    // Merge pre-resolved library search paths with any extra -L paths from args
    let mut lib_search_paths: Vec<String> = lib_paths.iter().map(|s| s.to_string()).collect();
    for p in parsed_args.extra_lib_paths {
        if !lib_search_paths.contains(&p) {
            lib_search_paths.push(p);
        }
    }
    let mut needed_libs: Vec<String> = needed_libs.iter().map(|s| s.to_string()).collect();
    needed_libs.extend(parsed_args.libs_to_load);

    // ── Phase 1: Load inputs, discover shared libs, resolve archives ────

    let mut input_objs: Vec<(String, ElfObject)> = Vec::new();
    let mut inline_archive_paths: Vec<String> = Vec::new();
    input::load_input_files(&all_inputs, &mut input_objs, &mut inline_archive_paths)?;

    let mut defined_syms: HashSet<String> = HashSet::new();
    let mut undefined_syms: HashSet<String> = HashSet::new();
    input::collect_initial_symbols(&input_objs, &mut defined_syms, &mut undefined_syms);

    let mut shared_lib_syms: HashMap<String, DynSymbol> = HashMap::new();
    let mut actual_needed_libs: Vec<String> = Vec::new();
    if !is_static {
        input::discover_shared_lib_symbols(
            &needed_libs,
            &lib_search_paths,
            &mut input_objs,
            &mut defined_syms,
            &mut undefined_syms,
            &mut shared_lib_syms,
            &mut actual_needed_libs,
        );
    }

    // Treat symbols available from shared libs as "defined" for archive resolution
    let shared_defined: HashSet<String> = shared_lib_syms
        .keys()
        .filter(|s| undefined_syms.contains(*s))
        .cloned()
        .collect();
    for s in &shared_defined {
        undefined_syms.remove(s);
    }

    input::resolve_archives(
        &inline_archive_paths,
        &needed_libs,
        &lib_search_paths,
        &mut input_objs,
        &mut defined_syms,
        &mut undefined_syms,
        &shared_lib_syms,
    );

    // Restore shared-lib-defined symbols back into undefined_syms
    for s in shared_defined {
        if !defined_syms.contains(&s) {
            undefined_syms.insert(s);
        }
    }

    // ── Phase 2: Merge sections ─────────────────────────────────────────

    let (mut merged_sections, mut merged_map, input_sec_refs) =
        sections::merge_sections(&input_objs);

    // ── Phase 3: Build global symbol table ──────────────────────────────

    let mut sec_mapping: HashMap<(usize, usize), (usize, u64)> = HashMap::new();
    for r in &input_sec_refs {
        sec_mapping.insert(
            (r.obj_idx, r.sec_idx),
            (r.merged_sec_idx, r.offset_in_merged),
        );
    }

    let mut global_syms = symbols::build_global_symbols(
        &input_objs,
        &sec_mapping,
        &mut merged_sections,
        &mut merged_map,
    );

    let (plt_symbols, copy_symbols) = if !is_static {
        symbols::mark_plt_and_copy_symbols(&mut global_syms, &shared_lib_syms)
    } else {
        (Vec::new(), Vec::new())
    };

    // Apply --defsym definitions: alias one symbol to another
    for (alias, target) in &defsym_defs {
        if let Some(target_sym) = global_syms.get(target).cloned() {
            global_syms.insert(alias.clone(), target_sym);
        }
    }

    if !is_static {
        symbols::check_undefined_symbols(&global_syms, &shared_lib_syms)?;
    } else {
        symbols::check_undefined_symbols(&global_syms, &HashMap::new())?;
    }

    let (got_symbols, tls_got_symbols, local_got_sym_info) =
        symbols::collect_got_entries(&input_objs);

    // Sort sections by canonical order
    let mut sec_indices: Vec<usize> = (0..merged_sections.len()).collect();
    sec_indices.sort_by_key(|&i| {
        let ms = &merged_sections[i];
        section_order(&ms.name, ms.sh_flags)
    });

    // ── Phase 4+: Emit executable ───────────────────────────────────────

    super::emit_exec::emit_executable(
        &input_objs,
        &mut merged_sections,
        &mut merged_map,
        &sec_mapping,
        &mut global_syms,
        &got_symbols,
        &tls_got_symbols,
        &local_got_sym_info,
        &plt_symbols,
        &copy_symbols,
        &sec_indices,
        &actual_needed_libs,
        is_static,
        output_path,
    )
}

// ── Public entry point: shared library linking ──────────────────────────

/// Link object files into a RISC-V shared library (.so).
///
/// Produces an ET_DYN ELF with base address 0 (position-independent).
/// All defined global symbols are exported to .dynsym.
pub fn link_shared(
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
    lib_paths: &[&str],
) -> Result<(), String> {
    let lib_path_strings: Vec<String> = lib_paths.iter().map(|s| s.to_string()).collect();

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
            for j in 0..parts.len() {
                if let Some(sn) = parts[j].strip_prefix("-soname=") {
                    soname = Some(sn.to_string());
                } else if parts[j] == "-soname" && j + 1 < parts.len() {
                    soname = Some(parts[j + 1].to_string());
                } else if let Some(lpath) = parts[j].strip_prefix("-L") {
                    extra_lib_paths.push(lpath.to_string());
                } else if let Some(lib) = parts[j].strip_prefix("-l") {
                    libs_to_load.push(lib.to_string());
                }
            }
        } else if arg == "-shared" || arg == "-nostdlib" || arg == "-o" {
            if arg == "-o" {
                i += 1;
            }
        } else if !arg.starts_with('-') && std::path::Path::new(arg).exists() {
            extra_object_files.push(arg.to_string());
        }
        i += 1;
    }

    // ── Phase 1: Load input objects ──────────────────────────────────

    let mut input_objs: Vec<(String, ElfObject)> = Vec::new();
    let mut defined_syms: HashSet<String> = HashSet::new();
    let mut undefined_syms: HashSet<String> = HashSet::new();
    let mut needed_sonames: Vec<String> = Vec::new();

    let mut all_lib_paths: Vec<String> = extra_lib_paths;
    all_lib_paths.extend(lib_path_strings.iter().cloned());

    input::load_shared_lib_inputs(
        object_files,
        &extra_object_files,
        &mut input_objs,
        &mut defined_syms,
        &mut undefined_syms,
    )?;
    input::resolve_shared_lib_deps(
        &libs_to_load,
        &all_lib_paths,
        &mut input_objs,
        &mut defined_syms,
        &mut undefined_syms,
        &mut needed_sonames,
    )?;

    if input_objs.is_empty() {
        return Err("No input files for shared library".to_string());
    }

    // ── Phase 2: Merge sections ─────────────────────────────────────

    let (mut merged_sections, mut merged_map, input_sec_refs) =
        sections::merge_sections(&input_objs);

    // ── Phase 3: Build global symbol table ───────────────────────────

    let mut sec_mapping: HashMap<(usize, usize), (usize, u64)> = HashMap::new();
    for r in &input_sec_refs {
        sec_mapping.insert(
            (r.obj_idx, r.sec_idx),
            (r.merged_sec_idx, r.offset_in_merged),
        );
    }

    let mut global_syms = symbols::build_global_symbols(
        &input_objs,
        &sec_mapping,
        &mut merged_sections,
        &mut merged_map,
    );

    // Identify GOT entries needed
    let (got_symbols, tls_got_symbols, local_got_sym_info) =
        symbols::collect_got_entries(&input_objs);

    // ── Phase 4+: Emit shared library ───────────────────────────────

    super::emit_shared::emit_shared_library(
        &input_objs,
        &mut merged_sections,
        &mut merged_map,
        &sec_mapping,
        &mut global_syms,
        &got_symbols,
        &tls_got_symbols,
        &local_got_sym_info,
        &needed_sonames,
        soname,
        output_path,
    )
}
