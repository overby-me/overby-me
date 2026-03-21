//! Input file loading for the x86-64 linker.
//!
//! Handles loading of object files (.o), archives (.a), shared libraries (.so),
//! and linker scripts. Delegates to `linker_common` for ELF parsing.

use std::collections::HashMap;
use std::path::Path;

use super::elf::*;
use super::types::{x86_should_replace_extra, GlobalSymbol};
use crate::backend::linker_common;

pub(super) fn load_file(
    path: &str,
    objects: &mut Vec<ElfObject>,
    globals: &mut HashMap<String, GlobalSymbol>,
    needed_sonames: &mut Vec<String>,
    lib_paths: &[String],
    whole_archive: bool,
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
            EM_X86_64,
            x86_should_replace_extra,
            whole_archive,
        );
    }

    // Thin archive
    if is_thin_archive(&data) {
        return linker_common::load_thin_archive_elf64(
            &data,
            path,
            objects,
            globals,
            EM_X86_64,
            x86_should_replace_extra,
            whole_archive,
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
                                load_file(
                                    lib_path,
                                    objects,
                                    globals,
                                    needed_sonames,
                                    lib_paths,
                                    whole_archive,
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
                                        whole_archive,
                                    )?;
                                }
                            }
                        }
                        LinkerScriptEntry::Lib(lib_name) => {
                            if let Some(resolved_path) =
                                linker_common::resolve_lib(lib_name, lib_paths, false)
                            {
                                load_file(
                                    &resolved_path,
                                    objects,
                                    globals,
                                    needed_sonames,
                                    lib_paths,
                                    whole_archive,
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
            return linker_common::load_shared_library_elf64(
                path,
                globals,
                needed_sonames,
                lib_paths,
            );
        }
    }

    // Regular ELF object
    let obj = parse_object(&data, path)?;
    let obj_idx = objects.len();
    linker_common::register_symbols_elf64(obj_idx, &obj, globals, x86_should_replace_extra);
    objects.push(obj);
    Ok(())
}
