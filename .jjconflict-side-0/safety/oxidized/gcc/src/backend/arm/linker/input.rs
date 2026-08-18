//! Input file loading for the AArch64 linker.
//!
//! Handles loading of object files (.o), archives (.a), shared libraries (.so),
//! and linker scripts. Delegates to `linker_common` for ELF parsing.

use std::collections::HashMap;
use std::path::Path;

use super::elf::*;
use super::types::{arm_should_replace_extra, GlobalSymbol};
use crate::backend::linker_common;

pub fn load_file(
    path: &str,
    objects: &mut Vec<ElfObject>,
    globals: &mut HashMap<String, GlobalSymbol>,
    needed_sonames: &mut Vec<String>,
    lib_paths: &[String],
    is_static: bool,
) -> Result<(), String> {
    if std::env::var("LINKER_DEBUG").is_ok() {
        eprintln!("load_file: {}", path);
    }
    let data = std::fs::read(path).map_err(|e| format!("failed to read '{}': {}", path, e))?;

    // Regular archive
    if data.len() >= 8 && &data[0..8] == b"!<arch>\n" {
        return linker_common::load_archive_elf64(
            &data,
            path,
            objects,
            globals,
            EM_AARCH64,
            arm_should_replace_extra,
            false,
        );
    }

    // Thin archive
    if is_thin_archive(&data) {
        return linker_common::load_thin_archive_elf64(
            &data,
            path,
            objects,
            globals,
            EM_AARCH64,
            arm_should_replace_extra,
            false,
        );
    }

    // Not ELF? Try linker script (handles both GROUP and INPUT directives)
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
                                load_file(
                                    lib_path,
                                    objects,
                                    globals,
                                    needed_sonames,
                                    lib_paths,
                                    is_static,
                                )?;
                            } else if let Some(ref dir) = script_dir {
                                let resolved = format!("{}/{}", dir, lib_path);
                                if Path::new(&resolved).exists() {
                                    load_file(
                                        &resolved,
                                        objects,
                                        globals,
                                        needed_sonames,
                                        lib_paths,
                                        is_static,
                                    )?;
                                }
                            }
                        }
                        LinkerScriptEntry::Lib(lib_name) => {
                            if let Some(resolved) = resolve_lib(lib_name, lib_paths) {
                                load_file(
                                    &resolved,
                                    objects,
                                    globals,
                                    needed_sonames,
                                    lib_paths,
                                    is_static,
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

    // Shared library?
    if data.len() >= 18 {
        let e_type = read_u16(&data, 16);
        if e_type == ET_DYN {
            if is_static {
                return Ok(()); // Skip .so in static linking
            }
            return linker_common::load_shared_library_elf64(
                path,
                globals,
                needed_sonames,
                lib_paths,
            );
        }
    }

    let obj = parse_object(&data, path)?;
    let obj_idx = objects.len();
    linker_common::register_symbols_elf64(obj_idx, &obj, globals, arm_should_replace_extra);
    objects.push(obj);
    Ok(())
}

pub fn resolve_lib(name: &str, paths: &[String]) -> Option<String> {
    crate::backend::linker_common::resolve_lib(name, paths, true)
}

pub fn resolve_lib_prefer_shared(name: &str, paths: &[String]) -> Option<String> {
    // For dynamic linking, prefer .so over .a
    crate::backend::linker_common::resolve_lib(name, paths, false)
}
