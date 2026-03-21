//! Shared linker argument parsing.
//!
//! Extracts linker flags from the `user_args` passed through `-Wl,` and
//! direct `-L`/`-l` flags. Used by x86, ARM, and RISC-V linkers.

use std::path::Path;

/// Parsed linker arguments from user_args.
///
/// Contains all the flags that are common across backends. Not all backends
/// use every field; unused fields are simply ignored.
#[derive(Debug, Default)]
pub struct LinkerArgs {
    /// Extra library search paths from `-L` flags.
    pub extra_lib_paths: Vec<String>,
    /// Library names from `-l` flags (without the `lib` prefix or `.a`/`.so` suffix).
    pub libs_to_load: Vec<String>,
    /// Bare file paths (`.o`, `.a` files) passed as arguments.
    pub extra_object_files: Vec<String>,
    /// Whether `--export-dynamic` / `-rdynamic` was passed.
    pub export_dynamic: bool,
    /// RPATH entries from `-Wl,-rpath=` or `-Wl,-rpath,`.
    pub rpath_entries: Vec<String>,
    /// Use DT_RUNPATH instead of DT_RPATH (from `--enable-new-dtags`).
    pub use_runpath: bool,
    /// Symbol definitions from `--defsym=SYM=VAL`.
    /// TODO: only supports symbol-to-symbol aliasing, not arbitrary expressions.
    pub defsym_defs: Vec<(String, String)>,
    /// Enable garbage collection of unused sections (from `--gc-sections`).
    pub gc_sections: bool,
    /// Whether `-static` was passed.
    pub is_static: bool,
}

/// Parse user linker arguments into a structured `LinkerArgs`.
///
/// Handles `-L`, `-l`, `-Wl,` (with nested flags like `--defsym`, `--export-dynamic`,
/// `-rpath`, `--gc-sections`), `-rdynamic`, `-static`, and bare file paths.
pub fn parse_linker_args(user_args: &[String]) -> LinkerArgs {
    let mut result = LinkerArgs::default();
    let args: Vec<&str> = user_args.iter().map(|s| s.as_str()).collect();
    let mut pending_rpath = false; // for -Wl,-rpath -Wl,/path two-arg form
    let mut i = 0;
    while i < args.len() {
        let arg = args[i];
        if arg == "-rdynamic" {
            result.export_dynamic = true;
        } else if arg == "-static" {
            result.is_static = true;
        } else if let Some(path) = arg.strip_prefix("-L") {
            let p = if path.is_empty() && i + 1 < args.len() {
                i += 1;
                args[i]
            } else {
                path
            };
            result.extra_lib_paths.push(p.to_string());
        } else if let Some(lib) = arg.strip_prefix("-l") {
            let l = if lib.is_empty() && i + 1 < args.len() {
                i += 1;
                args[i]
            } else {
                lib
            };
            result.libs_to_load.push(l.to_string());
        } else if let Some(wl_arg) = arg.strip_prefix("-Wl,") {
            let parts: Vec<&str> = wl_arg.split(',').collect();
            // Handle -Wl,-rpath -Wl,/path two-arg form
            if pending_rpath && !parts.is_empty() {
                result.rpath_entries.push(parts[0].to_string());
                pending_rpath = false;
                i += 1;
                continue;
            }
            let mut j = 0;
            while j < parts.len() {
                let part = parts[j];
                if part == "--export-dynamic" || part == "-export-dynamic" || part == "-E" {
                    result.export_dynamic = true;
                } else if let Some(rp) = part.strip_prefix("-rpath=") {
                    result.rpath_entries.push(rp.to_string());
                } else if part == "-rpath" && j + 1 < parts.len() {
                    j += 1;
                    result.rpath_entries.push(parts[j].to_string());
                } else if part == "-rpath" {
                    // -rpath without following value in this -Wl, group;
                    // the path comes in the next -Wl, argument
                    pending_rpath = true;
                } else if part == "--enable-new-dtags" {
                    result.use_runpath = true;
                } else if part == "--disable-new-dtags" {
                    result.use_runpath = false;
                } else if let Some(lpath) = part.strip_prefix("-L") {
                    result.extra_lib_paths.push(lpath.to_string());
                } else if let Some(lib) = part.strip_prefix("-l") {
                    result.libs_to_load.push(lib.to_string());
                } else if let Some(defsym_arg) = part.strip_prefix("--defsym=") {
                    if let Some(eq_pos) = defsym_arg.find('=') {
                        result.defsym_defs.push((
                            defsym_arg[..eq_pos].to_string(),
                            defsym_arg[eq_pos + 1..].to_string(),
                        ));
                    }
                } else if part == "--defsym" && j + 1 < parts.len() {
                    j += 1;
                    let defsym_arg = parts[j];
                    if let Some(eq_pos) = defsym_arg.find('=') {
                        result.defsym_defs.push((
                            defsym_arg[..eq_pos].to_string(),
                            defsym_arg[eq_pos + 1..].to_string(),
                        ));
                    }
                } else if part == "--gc-sections" {
                    result.gc_sections = true;
                } else if part == "--no-gc-sections" {
                    result.gc_sections = false;
                } else if part == "-static" {
                    result.is_static = true;
                }
                // TODO: --whole-archive / --no-whole-archive are positional flags
                // that need per-file tracking; currently handled in link_shared's
                // custom parser (x86). Add here when link_builtin needs it.
                j += 1;
            }
        } else if !arg.starts_with('-') && Path::new(arg).exists() {
            result.extra_object_files.push(arg.to_string());
        }
        i += 1;
    }
    result
}
