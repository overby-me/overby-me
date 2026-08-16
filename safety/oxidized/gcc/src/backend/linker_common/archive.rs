//! Archive and file loading for ELF64 linkers.
//!
//! Provides iterative archive member resolution (the `--start-group` algorithm),
//! regular and thin archive loading, and a generic file dispatch function that
//! handles archives, linker scripts, shared libraries, and object files.

use std::collections::HashMap;
use std::path::Path;

use super::dynamic::register_symbols_elf64;
use super::parse_object::parse_elf64_object;
use super::resolve_lib::resolve_lib;
use super::symbols::GlobalSymbolOps;
use super::types::Elf64Object;
use crate::backend::elf::{
    is_thin_archive, parse_archive_members, parse_linker_script_entries,
    parse_thin_archive_members, read_u16, LinkerScriptEntry, ELF_MAGIC, ET_DYN, STT_FILE,
    STT_SECTION,
};

/// Check if an archive member defines any currently-undefined, non-dynamic symbol.
fn member_resolves_undefined_generic<G: GlobalSymbolOps>(
    obj: &Elf64Object,
    globals: &HashMap<String, G>,
) -> bool {
    for sym in &obj.symbols {
        if sym.is_undefined() || sym.is_local() {
            continue;
        }
        if sym.sym_type() == STT_SECTION || sym.sym_type() == STT_FILE {
            continue;
        }
        if sym.name.is_empty() {
            continue;
        }
        if let Some(existing) = globals.get(&sym.name) {
            if !existing.is_defined() && !existing.is_dynamic() {
                return true;
            }
        }
    }
    false
}

/// Iterative archive member resolution (the --start-group algorithm).
///
/// Given a list of parsed archive member objects, pull in members that define
/// any currently-undefined global symbol. Repeat until no more progress.
fn resolve_archive_members<G: GlobalSymbolOps>(
    member_objects: &mut Vec<Elf64Object>,
    objects: &mut Vec<Elf64Object>,
    globals: &mut HashMap<String, G>,
    should_replace_extra: fn(&G) -> bool,
) {
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i < member_objects.len() {
            if member_resolves_undefined_generic(&member_objects[i], globals) {
                let obj = member_objects.remove(i);
                let obj_idx = objects.len();
                register_symbols_elf64(obj_idx, &obj, globals, should_replace_extra);
                objects.push(obj);
                changed = true;
            } else {
                i += 1;
            }
        }
    }
}

/// Load a regular archive (.a), parsing members and pulling in those that
/// resolve undefined symbols.
///
/// When `whole_archive` is true, all members are unconditionally included
/// (equivalent to GNU ld's `--whole-archive` flag). This is essential for
/// shared library creation from convenience archives (e.g., libtool).
pub fn load_archive_elf64<G: GlobalSymbolOps>(
    data: &[u8],
    archive_path: &str,
    objects: &mut Vec<Elf64Object>,
    globals: &mut HashMap<String, G>,
    expected_machine: u16,
    should_replace_extra: fn(&G) -> bool,
    whole_archive: bool,
) -> Result<(), String> {
    let members = parse_archive_members(data)?;
    let mut member_objects: Vec<Elf64Object> = Vec::new();
    for (name, offset, size) in &members {
        let member_data = &data[*offset..*offset + *size];
        if member_data.len() < 4 || member_data[0..4] != ELF_MAGIC {
            continue;
        }
        if expected_machine != 0 && member_data.len() >= 20 {
            let e_machine = read_u16(member_data, 18);
            if e_machine != expected_machine {
                continue;
            }
        }
        let full_name = format!("{}({})", archive_path, name);
        if let Ok(obj) = parse_elf64_object(member_data, &full_name, expected_machine) {
            member_objects.push(obj);
        }
    }
    if whole_archive {
        // --whole-archive: include ALL members unconditionally
        for obj in member_objects.drain(..) {
            let obj_idx = objects.len();
            register_symbols_elf64(obj_idx, &obj, globals, should_replace_extra);
            objects.push(obj);
        }
    } else {
        resolve_archive_members(&mut member_objects, objects, globals, should_replace_extra);
    }
    Ok(())
}

/// Load a GNU thin archive. Members are external files referenced by name
/// relative to the archive's directory.
///
/// When `whole_archive` is true, all members are unconditionally included.
pub fn load_thin_archive_elf64<G: GlobalSymbolOps>(
    data: &[u8],
    archive_path: &str,
    objects: &mut Vec<Elf64Object>,
    globals: &mut HashMap<String, G>,
    expected_machine: u16,
    should_replace_extra: fn(&G) -> bool,
    whole_archive: bool,
) -> Result<(), String> {
    let member_names = parse_thin_archive_members(data)?;
    let archive_dir = Path::new(archive_path)
        .parent()
        .unwrap_or_else(|| Path::new("."));

    let mut member_objects: Vec<Elf64Object> = Vec::new();
    for name in &member_names {
        let member_path = archive_dir.join(name);
        let member_data = std::fs::read(&member_path).map_err(|e| {
            format!(
                "thin archive {}: failed to read member '{}': {}",
                archive_path,
                member_path.display(),
                e
            )
        })?;
        if member_data.len() < 4 || member_data[0..4] != ELF_MAGIC {
            continue;
        }
        let full_name = format!("{}({})", archive_path, name);
        if let Ok(obj) = parse_elf64_object(&member_data, &full_name, expected_machine) {
            member_objects.push(obj);
        }
    }
    if whole_archive {
        for obj in member_objects.drain(..) {
            let obj_idx = objects.len();
            register_symbols_elf64(obj_idx, &obj, globals, should_replace_extra);
            objects.push(obj);
        }
    } else {
        resolve_archive_members(&mut member_objects, objects, globals, should_replace_extra);
    }
    Ok(())
}

/// Load a file, dispatching by format (archive, thin archive, linker script,
/// shared library, or object file).
///
/// The `on_shared_lib` callback handles shared libraries (.so files). This allows
/// x86 and ARM to handle dynamic symbol extraction differently. Pass a no-op
/// closure for static-only linking.
///
/// Currently unused: x86 and ARM linkers have their own `load_file` implementations.
/// This generic version will be used as those linkers migrate to shared infrastructure.
#[allow(dead_code)] // Planned shared infrastructure; x86/ARM linkers will migrate to this
pub fn load_file_elf64<G: GlobalSymbolOps>(
    path: &str,
    objects: &mut Vec<Elf64Object>,
    globals: &mut HashMap<String, G>,
    expected_machine: u16,
    lib_paths: &[String],
    prefer_static: bool,
    should_replace_extra: fn(&G) -> bool,
    on_shared_lib: &mut dyn FnMut(&str, &[u8]) -> Result<(), String>,
) -> Result<(), String> {
    if std::env::var("LINKER_DEBUG").is_ok() {
        eprintln!("load_file: {}", path);
    }

    let data = std::fs::read(path).map_err(|e| format!("failed to read '{}': {}", path, e))?;

    // Regular archive
    if data.len() >= 8 && &data[0..8] == b"!<arch>\n" {
        return load_archive_elf64(
            &data,
            path,
            objects,
            globals,
            expected_machine,
            should_replace_extra,
            false,
        );
    }

    // Thin archive
    if is_thin_archive(&data) {
        return load_thin_archive_elf64(
            &data,
            path,
            objects,
            globals,
            expected_machine,
            should_replace_extra,
            false,
        );
    }

    // Not ELF? Try linker script (handles GROUP and INPUT directives)
    if data.len() >= 4 && data[0..4] != ELF_MAGIC {
        if let Ok(text) = std::str::from_utf8(&data) {
            if let Some(entries) = parse_linker_script_entries(text) {
                let script_dir = Path::new(path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string());
                for entry in &entries {
                    match entry {
                        LinkerScriptEntry::Path(lib_path) => {
                            if Path::new(lib_path).exists() {
                                load_file_elf64(
                                    lib_path,
                                    objects,
                                    globals,
                                    expected_machine,
                                    lib_paths,
                                    prefer_static,
                                    should_replace_extra,
                                    on_shared_lib,
                                )?;
                            } else if let Some(ref dir) = script_dir {
                                let resolved = format!("{}/{}", dir, lib_path);
                                if Path::new(&resolved).exists() {
                                    load_file_elf64(
                                        &resolved,
                                        objects,
                                        globals,
                                        expected_machine,
                                        lib_paths,
                                        prefer_static,
                                        should_replace_extra,
                                        on_shared_lib,
                                    )?;
                                }
                            }
                        }
                        LinkerScriptEntry::Lib(lib_name) => {
                            if let Some(resolved_path) =
                                resolve_lib(lib_name, lib_paths, prefer_static)
                            {
                                load_file_elf64(
                                    &resolved_path,
                                    objects,
                                    globals,
                                    expected_machine,
                                    lib_paths,
                                    prefer_static,
                                    should_replace_extra,
                                    on_shared_lib,
                                )?;
                            }
                        }
                    }
                }
                return Ok(());
            }
        }
        return Err(format!("{}: not a valid ELF object or archive", path));
    }

    // Shared library
    if data.len() >= 18 {
        let e_type = u16::from_le_bytes([data[16], data[17]]);
        if e_type == ET_DYN {
            return on_shared_lib(path, &data);
        }
    }

    // Regular ELF object
    let obj = parse_elf64_object(&data, path, expected_machine)?;
    let obj_idx = objects.len();
    register_symbols_elf64(obj_idx, &obj, globals, should_replace_extra);
    objects.push(obj);
    Ok(())
}
