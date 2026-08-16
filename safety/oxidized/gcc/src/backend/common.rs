//! Shared backend utilities for assembler, linker, and data emission.
//!
//! All four backends (x86-64, i686, AArch64, RISC-V 64) share identical logic for:
//! - Assembling via an external toolchain (gcc/cross-gcc)
//! - Linking via an external toolchain
//! - Emitting assembly data directives (.data, .bss, .rodata, string literals, constants)
//!
//! This module extracts that shared logic, parameterized only by:
//! - The toolchain command name (e.g., "gcc" vs "aarch64-linux-gnu-gcc")
//! - The 64-bit data directive (`.quad` vs `.xword` vs `.dword`)
//! - Extra assembler/linker flags

use crate::backend::elf::{EM_386, EM_AARCH64, EM_RISCV, EM_X86_64};
use crate::common::types::IrType;
use crate::ir::reexports::{GlobalInit, IrConst, IrGlobal, IrModule};
#[cfg(any(feature = "gcc_assembler", feature = "gcc_linker"))]
use std::process::Command;
#[cfg(any(feature = "gcc_assembler", feature = "gcc_linker"))]
use std::sync::Once;

/// Print a one-time warning when using a GCC-backed assembler.
///
/// This fires when the `gcc_assembler` feature is enabled and GCC is
/// being used as the assembler. The warning is printed at most once per
/// process to avoid flooding stderr on large builds.
#[cfg(feature = "gcc_assembler")]
fn warn_gcc_assembler(command: &str) {
    static WARN_ONCE: Once = Once::new();
    WARN_ONCE.call_once(|| {
        eprintln!(
            "WARNING: Using GCC-backed assembler ({}) [gcc_assembler feature enabled]",
            command
        );
    });
}

/// Print a one-time warning when using GCC as the linker driver.
///
/// This fires when the `gcc_linker` feature is enabled and GCC is
/// being used as the linker. The warning is printed at most once per process.
#[cfg(feature = "gcc_linker")]
fn warn_gcc_linker(command: &str) {
    static WARN_ONCE: Once = Once::new();
    WARN_ONCE.call_once(|| {
        eprintln!(
            "WARNING: Using GCC-backed linker ({}) [gcc_linker feature enabled]",
            command
        );
    });
}

/// Configuration for an external assembler.
#[cfg_attr(not(feature = "gcc_assembler"), allow(dead_code))] // Only constructed/used when gcc_assembler enabled
pub struct AssemblerConfig {
    /// The assembler command (e.g., "gcc", "aarch64-linux-gnu-gcc")
    pub command: &'static str,
    /// Extra flags to pass (e.g., ["-march=rv64gc", "-mabi=lp64d"] for RISC-V)
    pub extra_args: &'static [&'static str],
}

/// Configuration for an external linker.
///
/// The `command` and `extra_args` fields are only used when linking via GCC
/// (`gcc_linker` feature). The built-in linker dispatches by `expected_elf_machine`.
#[allow(dead_code)] // `command`/`extra_args` fields only read under gcc_linker feature
pub struct LinkerConfig {
    /// The linker command (e.g., "gcc", "aarch64-linux-gnu-gcc")
    pub command: &'static str,
    /// Extra flags (e.g., ["-static"] for cross-compiled targets, ["-no-pie"] for x86)
    pub extra_args: &'static [&'static str],
    /// Expected ELF e_machine value for this target (e.g., EM_X86_64=62, EM_RISCV=243).
    /// Used to validate input .o files before linking and produce clear error messages
    /// when stale/wrong-arch objects are accidentally passed to the linker.
    pub expected_elf_machine: u16,
    /// Human-readable architecture name for error messages (e.g., "RISC-V", "x86-64").
    pub arch_name: &'static str,
}

/// Assemble text to an object file using GCC as the assembler.
///
/// Only available when the `gcc_assembler` Cargo feature is enabled.
/// The `extra_dynamic_args` are appended after the config's static extra_args,
/// allowing runtime overrides (e.g., -mabi=lp64 from CLI flags).
#[cfg(feature = "gcc_assembler")]
pub fn assemble_with_extra(
    config: &AssemblerConfig,
    asm_text: &str,
    output_path: &str,
    extra_dynamic_args: &[String],
) -> Result<(), String> {
    use crate::common::temp_files::TempFile;

    warn_gcc_assembler(config.command);

    let keep_asm = std::env::var("CCC_KEEP_ASM").is_ok();

    let asm_file = if keep_asm {
        let mut f = TempFile::with_path(format!("{}.s", output_path).into());
        f.set_keep(true);
        f
    } else {
        let stem = std::path::Path::new(output_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("asm");
        TempFile::new("gcc_asm", stem, "s")
    };
    std::fs::write(asm_file.path(), asm_text)
        .map_err(|e| format!("Failed to write assembly: {}", e))?;

    let mut cmd = Command::new(config.command);
    cmd.args(config.extra_args);
    cmd.args(extra_dynamic_args);
    cmd.args(["-c", "-o", output_path, asm_file.to_str()]);

    let result = cmd
        .output()
        .map_err(|e| format!("Failed to run assembler ({}): {}", config.command, e))?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(format!("Assembly failed ({}): {}", config.command, stderr));
    }

    Ok(())
}

/// Map an ELF e_machine value to a human-readable architecture name.
fn elf_machine_name(em: u16) -> &'static str {
    match em {
        EM_386 => "i386",
        40 => "ARM",
        EM_X86_64 => "x86-64",
        EM_AARCH64 => "aarch64",
        EM_RISCV => "RISC-V",
        _ => "unknown",
    }
}

/// Validate that all .o files in a list match the expected ELF e_machine.
/// Returns Ok(()) if all files match or are not ELF objects (archives, shared libs, etc.).
/// Returns Err with a diagnostic listing the mismatched files.
fn validate_object_architectures(
    files: impl Iterator<Item: AsRef<str>>,
    expected_machine: u16,
    arch_name: &str,
) -> Result<(), String> {
    use std::io::Read;
    let mut mismatched = Vec::new();

    for path_ref in files {
        let path = path_ref.as_ref();
        // Only check .o files (not .a, .so, -l flags, -Wl, flags, etc.)
        if !path.ends_with(".o") {
            continue;
        }
        // Read the ELF header: first 20 bytes contain e_ident (16) + e_type (2) + e_machine (2)
        let mut buf = [0u8; 20];
        let Ok(mut f) = std::fs::File::open(path) else {
            continue;
        };
        let Ok(n) = f.read(&mut buf) else { continue };
        if n < 20 {
            continue;
        }
        // Verify ELF magic
        if &buf[0..4] != b"\x7fELF" {
            continue;
        }
        // e_machine is at offset 18, always 2 bytes.
        // Determine endianness from EI_DATA (byte 5): 1=LE, 2=BE
        let is_le = buf[5] == 1;
        let em = if is_le {
            u16::from_le_bytes([buf[18], buf[19]])
        } else {
            u16::from_be_bytes([buf[18], buf[19]])
        };
        if em != expected_machine {
            mismatched.push((path.to_string(), em));
        }
    }

    if mismatched.is_empty() {
        return Ok(());
    }

    let mut msg = format!(
        "Object file architecture mismatch: target is {} (ELF e_machine={}) but these files are for a different architecture:\n",
        arch_name, expected_machine
    );
    for (path, em) in &mismatched {
        msg.push_str(&format!(
            "  {} ({}; e_machine={})\n",
            path,
            elf_machine_name(*em),
            em
        ));
    }
    msg.push_str("Hint: these look like stale objects from a previous build. Try running 'make clean' before rebuilding.");
    Err(msg)
}

/// Link object files into an executable (or shared library), with additional user-provided linker args.
///
/// When the `gcc_linker` Cargo feature is enabled, uses GCC as the linker
/// driver (with a warning). When disabled (default), uses the built-in native
/// linker for all supported architectures.
pub fn link_with_args(
    config: &LinkerConfig,
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
) -> Result<(), String> {
    // Validate that all input .o files match the target architecture.
    validate_object_architectures(
        object_files
            .iter()
            .copied()
            .chain(user_args.iter().map(|s| s.as_str())),
        config.expected_elf_machine,
        config.arch_name,
    )?;

    let is_shared = user_args.iter().any(|a| a == "-shared");
    let is_nostdlib = user_args.iter().any(|a| a == "-nostdlib");
    let is_relocatable = user_args.iter().any(|a| a == "-r");
    #[allow(unused_variables)] // Only used in the non-gcc_linker path below
    let is_static = user_args.iter().any(|a| a == "-static");

    // When gcc_linker feature is enabled, use GCC for ALL linking
    #[cfg(feature = "gcc_linker")]
    {
        link_with_gcc(
            config,
            object_files,
            output_path,
            user_args,
            is_shared,
            is_nostdlib,
            is_relocatable,
        )
    }

    // Default (gcc_linker disabled): use the built-in native linker
    #[cfg(not(feature = "gcc_linker"))]
    {
        if is_relocatable {
            return Err("Relocatable linking (-r) requires the gcc_linker feature. \
                       Rebuild with: cargo build --features gcc_linker"
                .to_string());
        }

        // Look up the architecture config by ELF machine number
        let arch = match config.expected_elf_machine {
            EM_X86_64 => &DIRECT_LD_X86_64,
            EM_AARCH64 => &DIRECT_LD_AARCH64,
            EM_RISCV => &DIRECT_LD_RISCV64,
            EM_386 => &DIRECT_LD_I686,
            _ => {
                return Err(format!(
                    "No built-in linker for ELF machine {} ({}). \
                     Rebuild with: cargo build --features gcc_linker",
                    config.expected_elf_machine, config.arch_name
                ));
            }
        };

        link_builtin_native(
            arch,
            object_files,
            output_path,
            user_args,
            is_nostdlib,
            is_static,
            is_shared,
        )
    }
}

/// Link using GCC as the linker driver (fallback path).
///
/// Only compiled when the `gcc_linker` Cargo feature is enabled.
#[cfg(feature = "gcc_linker")]
fn link_with_gcc(
    config: &LinkerConfig,
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
    is_shared: bool,
    is_nostdlib: bool,
    is_relocatable: bool,
) -> Result<(), String> {
    warn_gcc_linker(config.command);
    let ld_command = config.command;

    let mut cmd = Command::new(ld_command);
    let skip_extra = is_shared || is_relocatable;
    for arg in config.extra_args {
        if skip_extra && (*arg == "-no-pie" || *arg == "-pie" || *arg == "-static") {
            continue;
        }
        cmd.arg(arg);
    }
    cmd.arg("-o").arg(output_path);
    if !is_relocatable {
        cmd.arg("-Wl,-z,noexecstack");
    }

    for obj in object_files {
        cmd.arg(obj);
    }

    for arg in user_args {
        cmd.arg(arg);
    }

    if !is_nostdlib && !is_shared {
        cmd.arg("-lc");
        cmd.arg("-lm");
    }

    let result = cmd
        .output()
        .map_err(|e| format!("Failed to run linker ({}): {}", ld_command, e))?;

    if !result.stdout.is_empty() {
        use std::io::Write;
        let _ = std::io::stdout().write_all(&result.stdout);
    }

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(format!("Linking failed ({}): {}", ld_command, stderr));
    }

    Ok(())
}

/// Per-architecture configuration for direct ld invocation and built-in linker
/// CRT/library discovery.
///
/// Each architecture has different CRT/GCC library paths, emulation mode,
/// dynamic linker path, etc. This struct captures all those differences
/// so a single generic function can handle all backends.
#[cfg(not(feature = "gcc_linker"))]
#[allow(dead_code)] // Some fields (emulation, dynamic_linker, etc.) are stored for documentation/future use
struct DirectLdArchConfig {
    /// Human-readable architecture name for error messages (e.g., "x86-64", "RISC-V")
    arch_name: &'static str,
    /// ELF e_machine value (e.g., EM_X86_64=62, EM_RISCV=243).
    /// Used to dispatch to the correct backend linker.
    elf_machine: u16,
    /// ld emulation mode (e.g., "elf_x86_64", "elf64lriscv", "elf_i386", "aarch64linux")
    emulation: &'static str,
    /// Dynamic linker path (e.g., "/lib64/ld-linux-x86-64.so.2")
    dynamic_linker: &'static str,
    /// Base paths to search for GCC lib dir (containing crtbegin.o)
    gcc_lib_base_paths: &'static [&'static str],
    /// GCC versions to probe (newest first)
    gcc_versions: &'static [&'static str],
    /// Candidate directories for system CRT objects (crt1.o)
    crt_dir_candidates: &'static [&'static str],
    /// Standard system library directories for -L paths
    system_lib_dirs: &'static [&'static str],
    /// Extra ld flags specific to this architecture (e.g., AArch64 erratum workarounds)
    extra_ld_flags: &'static [&'static str],
    /// Extra GCC flags to skip when converting user args (e.g., "-m32" for i686)
    extra_skip_flags: &'static [&'static str],
    /// If true, crti.o and crtn.o are found in the GCC lib dir rather than the CRT dir.
    /// This is the case for RISC-V cross-compilation where the CRT dir only has crt1.o.
    crti_from_gcc_dir: bool,
    /// Package hint for CRT not-found error messages
    crt_package_hint: &'static str,
    /// Package hint for GCC lib not-found error messages
    gcc_package_hint: &'static str,
}

/// Standard GCC versions to probe (newest to oldest), shared across most architectures.
#[cfg(not(feature = "gcc_linker"))]
const GCC_VERSIONS_FULL: &[&str] = &["14", "13", "12", "11", "10", "9", "8", "7", "6", "5", "4.9"];
/// Shorter version list for architectures that don't have very old GCC support.
#[cfg(not(feature = "gcc_linker"))]
const GCC_VERSIONS_SHORT: &[&str] = &["14", "13", "12", "11", "10", "9", "8", "7"];

#[cfg(not(feature = "gcc_linker"))]
const DIRECT_LD_X86_64: DirectLdArchConfig = DirectLdArchConfig {
    arch_name: "x86-64",
    elf_machine: EM_X86_64,
    emulation: "elf_x86_64",
    dynamic_linker: "/lib64/ld-linux-x86-64.so.2",
    gcc_lib_base_paths: &[
        "/usr/lib/gcc/x86_64-linux-gnu",
        "/usr/lib/gcc/x86_64-redhat-linux",
        "/usr/lib/gcc/x86_64-pc-linux-gnu",
        "/usr/lib64/gcc/x86_64-linux-gnu",
        "/usr/lib64/gcc/x86_64-redhat-linux",
    ],
    gcc_versions: GCC_VERSIONS_FULL,
    crt_dir_candidates: &[
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib64",
        "/lib/x86_64-linux-gnu",
        "/lib64",
    ],
    system_lib_dirs: &[
        "/lib/x86_64-linux-gnu",
        "/lib/../lib",
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/../lib",
    ],
    extra_ld_flags: &[],
    extra_skip_flags: &[],
    crti_from_gcc_dir: false,
    crt_package_hint: "Is the libc development package installed?",
    gcc_package_hint: "Is the GCC development package installed?",
};

#[cfg(not(feature = "gcc_linker"))]
const DIRECT_LD_RISCV64: DirectLdArchConfig = DirectLdArchConfig {
    arch_name: "RISC-V",
    elf_machine: EM_RISCV,
    emulation: "elf64lriscv",
    dynamic_linker: "/lib/ld-linux-riscv64-lp64d.so.1",
    gcc_lib_base_paths: &[
        "/usr/lib/gcc-cross/riscv64-linux-gnu",
        "/usr/lib/gcc/riscv64-linux-gnu",
        "/usr/lib/gcc/riscv64-redhat-linux",
        "/usr/lib64/gcc/riscv64-linux-gnu",
    ],
    gcc_versions: GCC_VERSIONS_SHORT,
    crt_dir_candidates: &[
        "/usr/riscv64-linux-gnu/lib",
        "/usr/lib/riscv64-linux-gnu",
        "/lib/riscv64-linux-gnu",
    ],
    system_lib_dirs: &["/lib/riscv64-linux-gnu", "/usr/lib/riscv64-linux-gnu"],
    extra_ld_flags: &[],
    extra_skip_flags: &[],
    crti_from_gcc_dir: true,
    crt_package_hint: "Is the riscv64-linux-gnu libc development package installed? \
        (e.g., libc6-dev-riscv64-cross)",
    gcc_package_hint: "Is the riscv64-linux-gnu GCC cross-compiler installed? \
        (e.g., gcc-riscv64-linux-gnu)",
};

#[cfg(not(feature = "gcc_linker"))]
const DIRECT_LD_I686: DirectLdArchConfig = DirectLdArchConfig {
    arch_name: "i686",
    elf_machine: EM_386,
    emulation: "elf_i386",
    dynamic_linker: "/lib/ld-linux.so.2",
    gcc_lib_base_paths: &[
        "/usr/lib/gcc-cross/i686-linux-gnu",
        "/usr/lib/gcc/i686-linux-gnu",
        "/usr/lib/gcc/i686-redhat-linux",
        "/usr/lib/gcc/i686-pc-linux-gnu",
        "/usr/lib/gcc/i386-linux-gnu",
        "/usr/lib/gcc/i386-redhat-linux",
    ],
    gcc_versions: GCC_VERSIONS_FULL,
    crt_dir_candidates: &[
        "/usr/lib/i386-linux-gnu",
        "/usr/i686-linux-gnu/lib",
        "/usr/lib32",
        "/lib/i386-linux-gnu",
        "/lib32",
    ],
    system_lib_dirs: &[
        "/lib/i386-linux-gnu",
        "/lib/../lib",
        "/usr/lib/i386-linux-gnu",
        "/usr/lib/../lib",
        "/usr/i686-linux-gnu/lib",
    ],
    extra_ld_flags: &[],
    extra_skip_flags: &["-m32"],
    crti_from_gcc_dir: false,
    crt_package_hint: "Is the libc-dev-i386-cross or libc6-dev-i386 package installed?",
    gcc_package_hint: "Is the gcc-i686-linux-gnu package installed?",
};

#[cfg(not(feature = "gcc_linker"))]
const DIRECT_LD_AARCH64: DirectLdArchConfig = DirectLdArchConfig {
    arch_name: "AArch64",
    elf_machine: EM_AARCH64,
    emulation: "aarch64linux",
    dynamic_linker: "/lib/ld-linux-aarch64.so.1",
    gcc_lib_base_paths: &[
        "/usr/lib/gcc-cross/aarch64-linux-gnu",
        "/usr/lib/gcc/aarch64-linux-gnu",
        "/usr/lib/gcc/aarch64-redhat-linux",
        "/usr/lib/gcc/aarch64-unknown-linux-gnu",
        "/usr/lib64/gcc/aarch64-linux-gnu",
        "/usr/lib64/gcc/aarch64-redhat-linux",
    ],
    gcc_versions: GCC_VERSIONS_FULL,
    crt_dir_candidates: &[
        "/usr/aarch64-linux-gnu/lib",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/lib64",
        "/lib/aarch64-linux-gnu",
        "/lib64",
    ],
    system_lib_dirs: &[
        "/lib/aarch64-linux-gnu",
        "/lib/../lib",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/lib/../lib",
        "/usr/aarch64-linux-gnu/lib",
    ],
    extra_ld_flags: &["-EL", "-X", "--fix-cortex-a53-843419"],
    extra_skip_flags: &[],
    crti_from_gcc_dir: false,
    crt_package_hint: "Is the libc-dev-arm64-cross package installed?",
    gcc_package_hint: "Is the gcc-aarch64-linux-gnu package installed?",
};

/// Discover GCC's library directory by probing well-known paths.
/// Returns the path containing crtbegin.o (e.g., "/usr/lib/gcc/x86_64-linux-gnu/13").
#[cfg(not(feature = "gcc_linker"))]
fn find_gcc_lib_dir(arch: &DirectLdArchConfig) -> Option<String> {
    for base in arch.gcc_lib_base_paths {
        for ver in arch.gcc_versions {
            let dir = format!("{}/{}", base, ver);
            let crtbegin = format!("{}/crtbegin.o", dir);
            if std::path::Path::new(&crtbegin).exists() {
                return Some(dir);
            }
        }
    }
    None
}

/// Discover the system CRT directory containing crt1.o.
/// Returns the path (e.g., "/usr/lib/x86_64-linux-gnu").
#[cfg(not(feature = "gcc_linker"))]
fn find_crt_dir(arch: &DirectLdArchConfig) -> Option<String> {
    for dir in arch.crt_dir_candidates {
        let crt1 = format!("{}/crt1.o", dir);
        if std::path::Path::new(&crt1).exists() {
            return Some(dir.to_string());
        }
    }
    None
}

/// Resolve CRT objects and library paths for a built-in linker using DirectLdArchConfig.
///
/// This shared helper is used by all four built-in linker wrappers
/// (x86-64, i686, AArch64, RISC-V) to avoid duplicating CRT/library
/// discovery logic. Returns:
/// - `crt_before`: CRT objects to link before user objects
/// - `crt_after`: CRT objects to link after user objects
/// - `lib_paths`: Combined library search paths (user -L first, then system paths)
/// - `needed_libs`: Default libraries to link
#[cfg(not(feature = "gcc_linker"))]
struct BuiltinLinkSetup {
    crt_before: Vec<String>,
    crt_after: Vec<String>,
    lib_paths: Vec<String>,
    needed_libs: Vec<String>,
}

#[cfg(not(feature = "gcc_linker"))]
fn resolve_builtin_link_setup(
    arch: &DirectLdArchConfig,
    user_args: &[String],
    is_nostdlib: bool,
    is_static: bool,
) -> BuiltinLinkSetup {
    let gcc_lib_dir = find_gcc_lib_dir(arch);
    let crt_dir = find_crt_dir(arch);

    // System library paths
    let mut system_lib_paths: Vec<String> = Vec::new();
    if let Some(ref gcc) = gcc_lib_dir {
        system_lib_paths.push(gcc.clone());
    }
    if let Some(ref crt) = crt_dir {
        system_lib_paths.push(crt.clone());
    }
    for dir in arch.system_lib_dirs {
        if std::path::Path::new(dir).exists() {
            system_lib_paths.push(dir.to_string());
        }
    }

    // User-provided -L paths from args
    let mut user_lib_paths: Vec<String> = Vec::new();
    let mut i = 0;
    while i < user_args.len() {
        let arg = &user_args[i];
        if let Some(path) = arg.strip_prefix("-L") {
            if path.is_empty() {
                if i + 1 < user_args.len() {
                    i += 1;
                    user_lib_paths.push(user_args[i].clone());
                }
            } else {
                user_lib_paths.push(path.to_string());
            }
        } else if let Some(wl_arg) = arg.strip_prefix("-Wl,") {
            for part in wl_arg.split(',') {
                if let Some(lpath) = part.strip_prefix("-L") {
                    user_lib_paths.push(lpath.to_string());
                }
            }
        }
        i += 1;
    }

    // CRT objects
    let mut crt_before: Vec<String> = Vec::new();
    let mut crt_after: Vec<String> = Vec::new();

    if !is_nostdlib {
        // crt1.o comes from the CRT dir
        if let Some(ref crt) = crt_dir {
            crt_before.push(format!("{}/crt1.o", crt));
        }
        // crti.o: from GCC dir for cross-compilation (e.g., RISC-V), otherwise from CRT dir
        if arch.crti_from_gcc_dir {
            if let Some(ref gcc) = gcc_lib_dir {
                crt_before.push(format!("{}/crti.o", gcc));
            }
        } else if let Some(ref crt) = crt_dir {
            crt_before.push(format!("{}/crti.o", crt));
        }
        // crtbegin: use crtbeginT.o for static linking, crtbegin.o for dynamic
        if let Some(ref gcc) = gcc_lib_dir {
            if is_static {
                let crtbegin_t = format!("{}/crtbeginT.o", gcc);
                if std::path::Path::new(&crtbegin_t).exists() {
                    crt_before.push(crtbegin_t);
                } else {
                    crt_before.push(format!("{}/crtbegin.o", gcc));
                }
            } else {
                crt_before.push(format!("{}/crtbegin.o", gcc));
            }
        }
        if let Some(ref gcc) = gcc_lib_dir {
            crt_after.push(format!("{}/crtend.o", gcc));
        }
        // crtn.o: from GCC dir for cross-compilation, otherwise from CRT dir
        if arch.crti_from_gcc_dir {
            if let Some(ref gcc) = gcc_lib_dir {
                crt_after.push(format!("{}/crtn.o", gcc));
            }
        } else if let Some(ref crt) = crt_dir {
            crt_after.push(format!("{}/crtn.o", crt));
        }
    }

    // Default libraries
    let needed_libs: Vec<String> = if !is_nostdlib {
        vec!["gcc".to_string(), "c".to_string(), "m".to_string()]
    } else {
        vec![]
    };

    // Combined paths: user first, then system
    let mut lib_paths: Vec<String> = user_lib_paths;
    lib_paths.extend(system_lib_paths);

    BuiltinLinkSetup {
        crt_before,
        crt_after,
        lib_paths,
        needed_libs,
    }
}

/// Add architecture-specific extra libraries after "gcc" in the needed libs list.
///
/// Most architectures need libgcc_eh.a (static) or libgcc_s.so (dynamic) for
/// exception handling / stack unwinding, but the exact policy varies:
/// - x86-64: no extra libs needed (libgcc alone suffices)
/// - i686: always adds gcc_eh (needed for __divmoddi4, etc.)
/// - AArch64/RISC-V: gcc_eh for static, gcc_s for dynamic
#[cfg(not(feature = "gcc_linker"))]
fn add_arch_extra_libs(setup: &mut BuiltinLinkSetup, elf_machine: u16, is_static: bool) {
    // x86-64 doesn't need extra gcc libs
    if elf_machine == EM_X86_64 {
        return;
    }
    // Find the "gcc" entry and insert the extra lib after it
    if let Some(pos) = setup.needed_libs.iter().position(|l| l == "gcc") {
        // i686 always needs gcc_eh; others use gcc_eh for static, gcc_s for dynamic
        let extra = if elf_machine == EM_386 || is_static {
            "gcc_eh"
        } else {
            "gcc_s"
        };
        setup.needed_libs.insert(pos + 1, extra.to_string());
    }
}

/// Convert a `BuiltinLinkSetup` into borrowed slices for passing to backend linkers.
///
/// Avoids repeating the same 4-line `.iter().map(|s| s.as_str()).collect()` pattern.
#[cfg(not(feature = "gcc_linker"))]
struct LinkSetupRefs<'a> {
    lib_paths: Vec<&'a str>,
    needed_libs: Vec<&'a str>,
    crt_before: Vec<&'a str>,
    crt_after: Vec<&'a str>,
}

#[cfg(not(feature = "gcc_linker"))]
impl BuiltinLinkSetup {
    fn as_refs(&self) -> LinkSetupRefs<'_> {
        LinkSetupRefs {
            lib_paths: self.lib_paths.iter().map(|s| s.as_str()).collect(),
            needed_libs: self.needed_libs.iter().map(|s| s.as_str()).collect(),
            crt_before: self.crt_before.iter().map(|s| s.as_str()).collect(),
            crt_after: self.crt_after.iter().map(|s| s.as_str()).collect(),
        }
    }
}

/// Link using the built-in native ELF linker for any supported architecture.
///
/// This is the fully native path: no external ld binary is needed. The linker
/// reads ELF .o files and .a archives, resolves symbols against system shared
/// libraries (libc.so.6), handles relocations, and produces a dynamically-linked
/// ELF executable. Dispatches to the correct per-architecture backend based on
/// the `arch.elf_machine` value.
///
/// For shared library output (-shared), delegates to the per-arch `link_shared`
/// entry point with library paths only (no CRT objects).
#[cfg(not(feature = "gcc_linker"))]
fn link_builtin_native(
    arch: &DirectLdArchConfig,
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
    is_nostdlib: bool,
    is_static: bool,
    is_shared: bool,
) -> Result<(), String> {
    use crate::backend::{arm, i686, riscv, x86};

    if is_shared {
        // Shared libraries: no CRT objects, lib paths only
        let setup = resolve_builtin_link_setup(arch, user_args, true, false);
        let refs = setup.as_refs();
        return match arch.elf_machine {
            EM_X86_64 => {
                // x86-64 shared linker also takes implicit libs (gcc for runtime helpers)
                let implicit_libs: Vec<&str> = if is_nostdlib { vec![] } else { vec!["gcc"] };
                x86::linker::link_shared(
                    object_files,
                    output_path,
                    user_args,
                    &refs.lib_paths,
                    &implicit_libs,
                )
            }
            EM_AARCH64 => {
                arm::linker::link_shared(object_files, output_path, user_args, &refs.lib_paths)
            }
            EM_RISCV => {
                riscv::linker::link_shared(object_files, output_path, user_args, &refs.lib_paths)
            }
            EM_386 => {
                i686::linker::link_shared(object_files, output_path, user_args, &refs.lib_paths)
            }
            _ => Err(format!(
                "No shared library linker for {} (elf_machine={})",
                arch.arch_name, arch.elf_machine
            )),
        };
    }

    let mut setup = resolve_builtin_link_setup(arch, user_args, is_nostdlib, is_static);
    add_arch_extra_libs(&mut setup, arch.elf_machine, is_static);
    let refs = setup.as_refs();

    match arch.elf_machine {
        EM_X86_64 => x86::linker::link_builtin(
            object_files,
            output_path,
            user_args,
            &refs.lib_paths,
            &refs.needed_libs,
            &refs.crt_before,
            &refs.crt_after,
        ),
        EM_386 => i686::linker::link_builtin(
            object_files,
            output_path,
            user_args,
            &refs.lib_paths,
            &refs.needed_libs,
            &refs.crt_before,
            &refs.crt_after,
        ),
        EM_AARCH64 => arm::linker::link_builtin(
            object_files,
            output_path,
            user_args,
            &refs.lib_paths,
            &refs.needed_libs,
            &refs.crt_before,
            &refs.crt_after,
            is_static,
        ),
        EM_RISCV => riscv::linker::link_builtin(
            object_files,
            output_path,
            user_args,
            &refs.lib_paths,
            &refs.needed_libs,
            &refs.crt_before,
            &refs.crt_after,
        ),
        _ => Err(format!(
            "No built-in linker for {} (elf_machine={})",
            arch.arch_name, arch.elf_machine
        )),
    }
}

/// Assembly output buffer with helpers for emitting text.
///
/// Besides the generic `emit` and `emit_fmt` methods, this provides specialized
/// fast-path emitters for common patterns that avoid `core::fmt` overhead.
/// The fast integer writer (`write_i64`) uses direct digit extraction instead
/// of going through `Display`/`write_fmt` machinery.
pub struct AsmOutput {
    pub buf: String,
}

/// Write an i64 directly into a String buffer using manual digit extraction.
/// This is ~3-4x faster than `write!(buf, "{}", val)` for the common case
/// because it avoids the `core::fmt` vtable dispatch and `pad_integral` overhead.
#[inline]
fn write_i64_fast(buf: &mut String, val: i64) {
    if val == 0 {
        buf.push('0');
        return;
    }
    let mut tmp = [0u8; 20]; // i64 max is 19 digits + sign
    let negative = val < 0;
    // Work with absolute value using wrapping to handle i64::MIN correctly
    let mut v = if negative {
        (val as u64).wrapping_neg()
    } else {
        val as u64
    };
    let mut pos = 20;
    while v > 0 {
        pos -= 1;
        tmp[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    if negative {
        pos -= 1;
        tmp[pos] = b'-';
    }
    // All bytes are ASCII digits and optionally '-', which is always valid UTF-8.
    let s = std::str::from_utf8(&tmp[pos..20]).expect("integer formatting produced non-UTF8");
    buf.push_str(s);
}

/// Write a u64 directly into a String buffer.
#[inline]
fn write_u64_fast(buf: &mut String, val: u64) {
    if val == 0 {
        buf.push('0');
        return;
    }
    let mut tmp = [0u8; 20]; // u64 max is 20 digits
    let mut v = val;
    let mut pos = 20;
    while v > 0 {
        pos -= 1;
        tmp[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    let s = std::str::from_utf8(&tmp[pos..20]).expect("integer formatting produced non-UTF8");
    buf.push_str(s);
}

impl AsmOutput {
    pub fn new() -> Self {
        // Pre-allocate 256KB to avoid repeated reallocations during codegen.
        Self {
            buf: String::with_capacity(256 * 1024),
        }
    }

    /// Emit a line of assembly.
    #[inline]
    pub fn emit(&mut self, s: &str) {
        self.buf.push_str(s);
        self.buf.push('\n');
    }

    /// Emit formatted assembly directly into the buffer (no temporary String).
    #[inline]
    pub fn emit_fmt(&mut self, args: std::fmt::Arguments<'_>) {
        std::fmt::Write::write_fmt(&mut self.buf, args).unwrap();
        self.buf.push('\n');
    }

    // ── Fast-path emitters ──────────────────────────────────────────────
    //
    // These avoid the overhead of `format_args!` + `core::fmt::write` for
    // the most common codegen patterns. Each one directly pushes bytes into
    // the buffer using `push_str` and our fast integer writer.

    /// Emit: `    {mnemonic} ${imm}, %{reg}`
    /// Used for movq/movl/movabsq with immediate to register.
    #[inline]
    pub fn emit_instr_imm_reg(&mut self, mnemonic: &str, imm: i64, reg: &str) {
        self.buf.push_str(mnemonic);
        self.buf.push_str(" $");
        write_i64_fast(&mut self.buf, imm);
        self.buf.push_str(", %");
        self.buf.push_str(reg);
        self.buf.push('\n');
    }

    /// Emit: `    {mnemonic} %{src}, %{dst}`
    /// Used for movq/movl/xorq register-to-register.
    #[inline]
    pub fn emit_instr_reg_reg(&mut self, mnemonic: &str, src: &str, dst: &str) {
        self.buf.push_str(mnemonic);
        self.buf.push_str(" %");
        self.buf.push_str(src);
        self.buf.push_str(", %");
        self.buf.push_str(dst);
        self.buf.push('\n');
    }

    /// Emit: `    {mnemonic} {offset}(%rbp), %{reg}`
    /// Used for loads from stack slots.
    #[inline]
    pub fn emit_instr_rbp_reg(&mut self, mnemonic: &str, offset: i64, reg: &str) {
        self.buf.push_str(mnemonic);
        self.buf.push(' ');
        write_i64_fast(&mut self.buf, offset);
        self.buf.push_str("(%rbp), %");
        self.buf.push_str(reg);
        self.buf.push('\n');
    }

    /// Emit: `    {mnemonic} %{reg}, {offset}(%rbp)`
    /// Used for stores to stack slots.
    #[inline]
    pub fn emit_instr_reg_rbp(&mut self, mnemonic: &str, reg: &str, offset: i64) {
        self.buf.push_str(mnemonic);
        self.buf.push_str(" %");
        self.buf.push_str(reg);
        self.buf.push_str(", ");
        write_i64_fast(&mut self.buf, offset);
        self.buf.push_str("(%rbp)");
        self.buf.push('\n');
    }

    /// Emit a block label line: `.LBB{id}:`
    #[inline]
    pub fn emit_block_label(&mut self, block_id: u32) {
        self.buf.push_str(".LBB");
        write_u64_fast(&mut self.buf, block_id as u64);
        self.buf.push(':');
        self.buf.push('\n');
    }

    /// Emit: `    jmp .LBB{block_id}`
    #[inline]
    pub fn emit_jmp_block(&mut self, block_id: u32) {
        self.buf.push_str("    jmp .LBB");
        write_u64_fast(&mut self.buf, block_id as u64);
        self.buf.push('\n');
    }

    /// Emit: `    {jcc} .LBB{block_id}` (conditional jump to block label)
    #[inline]
    pub fn emit_jcc_block(&mut self, jcc: &str, block_id: u32) {
        self.buf.push_str(jcc);
        self.buf.push_str(" .LBB");
        write_u64_fast(&mut self.buf, block_id as u64);
        self.buf.push('\n');
    }

    /// Emit: `    {mnemonic} {reg}`  (single-register instruction like push/pop)
    #[inline]
    pub fn emit_instr_reg(&mut self, mnemonic: &str, reg: &str) {
        self.buf.push_str(mnemonic);
        self.buf.push_str(" %");
        self.buf.push_str(reg);
        self.buf.push('\n');
    }

    /// Emit: `    {mnemonic} ${imm}`  (single-immediate instruction like push)
    #[inline]
    pub fn emit_instr_imm(&mut self, mnemonic: &str, imm: i64) {
        self.buf.push_str(mnemonic);
        self.buf.push_str(" $");
        write_i64_fast(&mut self.buf, imm);
        self.buf.push('\n');
    }

    /// Write an i64 into the buffer without newline. Useful for building
    /// custom format patterns that include integers.
    #[inline]
    pub fn write_i64(&mut self, val: i64) {
        write_i64_fast(&mut self.buf, val);
    }

    /// Write a u64 into the buffer without newline.
    #[inline]
    pub fn write_u64(&mut self, val: u64) {
        write_u64_fast(&mut self.buf, val);
    }

    /// Emit: `    {mnemonic} {offset}(%rbp)` (single rbp-offset operand, e.g. fldt/fstpt)
    #[inline]
    pub fn emit_instr_rbp(&mut self, mnemonic: &str, offset: i64) {
        self.buf.push_str(mnemonic);
        self.buf.push(' ');
        write_i64_fast(&mut self.buf, offset);
        self.buf.push_str("(%rbp)");
        self.buf.push('\n');
    }

    /// Emit a named label definition: `{label}:`
    #[inline]
    pub fn emit_named_label(&mut self, label: &str) {
        self.buf.push_str(label);
        self.buf.push(':');
        self.buf.push('\n');
    }

    /// Emit: `    jmp {label}` (jump to named label)
    #[inline]
    pub fn emit_jmp_label(&mut self, label: &str) {
        self.buf.push_str("    jmp ");
        self.buf.push_str(label);
        self.buf.push('\n');
    }

    /// Emit: `    {jcc} {label}` (conditional jump to named label)
    #[inline]
    pub fn emit_jcc_label(&mut self, jcc: &str, label: &str) {
        self.buf.push_str(jcc);
        self.buf.push(' ');
        self.buf.push_str(label);
        self.buf.push('\n');
    }

    /// Emit: `    call {target}` (direct call to named function/label)
    #[inline]
    pub fn emit_call(&mut self, target: &str) {
        self.buf.push_str("    call ");
        self.buf.push_str(target);
        self.buf.push('\n');
    }

    /// Emit: `    {mnemonic} {offset}(%{base}), %{reg}` (memory to register with arbitrary base)
    #[inline]
    pub fn emit_instr_mem_reg(&mut self, mnemonic: &str, offset: i64, base: &str, reg: &str) {
        self.buf.push_str(mnemonic);
        self.buf.push(' ');
        if offset != 0 {
            write_i64_fast(&mut self.buf, offset);
        }
        self.buf.push_str("(%");
        self.buf.push_str(base);
        self.buf.push_str("), %");
        self.buf.push_str(reg);
        self.buf.push('\n');
    }

    /// Emit: `    {mnemonic} %{reg}, {offset}(%{base})` (register to memory with arbitrary base)
    #[inline]
    pub fn emit_instr_reg_mem(&mut self, mnemonic: &str, reg: &str, offset: i64, base: &str) {
        self.buf.push_str(mnemonic);
        self.buf.push_str(" %");
        self.buf.push_str(reg);
        self.buf.push_str(", ");
        if offset != 0 {
            write_i64_fast(&mut self.buf, offset);
        }
        self.buf.push_str("(%");
        self.buf.push_str(base);
        self.buf.push(')');
        self.buf.push('\n');
    }

    /// Emit: `    {mnemonic} ${imm}, {offset}(%{base})` (immediate to memory with arbitrary base)
    #[inline]
    pub fn emit_instr_imm_mem(&mut self, mnemonic: &str, imm: i64, offset: i64, base: &str) {
        self.buf.push_str(mnemonic);
        self.buf.push_str(" $");
        write_i64_fast(&mut self.buf, imm);
        self.buf.push_str(", ");
        if offset != 0 {
            write_i64_fast(&mut self.buf, offset);
        }
        self.buf.push_str("(%");
        self.buf.push_str(base);
        self.buf.push(')');
        self.buf.push('\n');
    }

    /// Emit: `    {mnemonic} {symbol}(%{base}), %{reg}` (symbol-relative addressing)
    /// Used for RIP-relative loads like `leaq table_label(%rip), %rcx`.
    #[inline]
    pub fn emit_instr_sym_base_reg(&mut self, mnemonic: &str, symbol: &str, base: &str, reg: &str) {
        self.buf.push_str(mnemonic);
        self.buf.push(' ');
        self.buf.push_str(symbol);
        self.buf.push_str("(%");
        self.buf.push_str(base);
        self.buf.push_str("), %");
        self.buf.push_str(reg);
        self.buf.push('\n');
    }

    /// Emit: `    {mnemonic} ${symbol}, %{reg}` (symbol as immediate)
    /// Used for absolute symbol addressing like `movq $name, %rax`.
    #[inline]
    pub fn emit_instr_sym_imm_reg(&mut self, mnemonic: &str, symbol: &str, reg: &str) {
        self.buf.push_str(mnemonic);
        self.buf.push_str(" $");
        self.buf.push_str(symbol);
        self.buf.push_str(", %");
        self.buf.push_str(reg);
        self.buf.push('\n');
    }

    /// Push a string slice without newline.
    #[inline]
    pub fn write_str(&mut self, s: &str) {
        self.buf.push_str(s);
    }

    /// Push a newline to end the current line.
    #[inline]
    pub fn newline(&mut self) {
        self.buf.push('\n');
    }
}

/// Emit formatted assembly directly into the output buffer, avoiding temporary
/// String allocations from `format!()`. Usage: `emit!(state, "    mov {}, {}", src, dst)`
#[macro_export]
macro_rules! emit {
    ($state:expr, $($arg:tt)*) => {
        $state.out.emit_fmt(format_args!($($arg)*))
    };
}

/// The only arch-specific difference in data emission: the name of the 64-bit pointer directive.
/// x86 uses `.quad`, AArch64 uses `.xword`, RISC-V uses `.dword`.
#[derive(Clone, Copy)]
pub enum PtrDirective {
    Quad,  // x86-64
    Long,  // i686 (32-bit)
    Xword, // AArch64
    Dword, // RISC-V 64
}

impl PtrDirective {
    pub fn as_str(self) -> &'static str {
        match self {
            PtrDirective::Quad => ".quad",
            PtrDirective::Long => ".long",
            PtrDirective::Xword => ".xword",
            PtrDirective::Dword => ".dword",
        }
    }

    /// Returns true if this is an x86 target directive (x86-64 or i686).
    /// Used to select x87 80-bit extended precision format for long double constants.
    pub fn is_x86(self) -> bool {
        matches!(self, PtrDirective::Quad | PtrDirective::Long)
    }

    /// Returns true if this is a 32-bit pointer directive.
    pub fn is_32bit(self) -> bool {
        matches!(self, PtrDirective::Long)
    }

    /// Returns true if this is the RISC-V target directive.
    /// RISC-V stores full IEEE binary128 long doubles in memory (allocas and globals).
    pub fn is_riscv(self) -> bool {
        matches!(self, PtrDirective::Dword)
    }

    /// Returns true if this is the AArch64 target directive.
    /// AArch64 stores full IEEE binary128 long doubles in memory (allocas and globals).
    pub fn is_arm(self) -> bool {
        matches!(self, PtrDirective::Xword)
    }

    /// Convert a byte alignment value to the correct `.align` argument for this target.
    /// On x86-64, `.align N` means N bytes. On ARM and RISC-V, `.align N` means 2^N bytes,
    /// so we must emit log2(N) instead.
    pub fn align_arg(self, bytes: usize) -> usize {
        debug_assert!(
            bytes == 0 || bytes.is_power_of_two(),
            "alignment must be power of 2"
        );
        match self {
            PtrDirective::Quad | PtrDirective::Long => bytes,
            PtrDirective::Xword | PtrDirective::Dword => {
                if bytes <= 1 {
                    0
                } else {
                    bytes.trailing_zeros() as usize
                }
            }
        }
    }
}

/// Emit all data sections (rodata for string literals, .data and .bss for globals).
pub fn emit_data_sections(out: &mut AsmOutput, module: &IrModule, ptr_dir: PtrDirective) {
    // String literals in .rodata
    if !module.string_literals.is_empty()
        || !module.wide_string_literals.is_empty()
        || !module.char16_string_literals.is_empty()
    {
        out.emit(".section .rodata");
        for (label, value) in &module.string_literals {
            out.emit_fmt(format_args!("{}:", label));
            emit_string_bytes(out, value);
        }
        // Wide string literals (L"..."): each char is a 4-byte wchar_t value
        for (label, chars) in &module.wide_string_literals {
            out.emit_fmt(format_args!(".align {}", ptr_dir.align_arg(4)));
            out.emit_fmt(format_args!("{}:", label));
            for &ch in chars {
                out.emit_fmt(format_args!("  .long {}", ch));
            }
        }
        // char16_t string literals (u"..."): each char is a 2-byte char16_t value
        for (label, chars) in &module.char16_string_literals {
            out.emit_fmt(format_args!(".align {}", ptr_dir.align_arg(2)));
            out.emit_fmt(format_args!("{}:", label));
            for &ch in chars {
                out.emit_fmt(format_args!("  .short {}", ch));
            }
        }
        out.emit("");
    }

    // Global variables
    emit_globals(out, &module.globals, ptr_dir);
}

/// Compute effective alignment for a global, promoting to 16 when size >= 16.
/// This matches GCC/Clang behavior on x86-64 and aarch64, enabling aligned SSE/NEON access.
/// Globals placed in custom sections are excluded from promotion because they may
/// form contiguous arrays (e.g. the kernel's __param or .init.setup sections) where
/// the linker expects elements at their natural stride with no extra padding.
/// Additionally, when the user explicitly specified an alignment via __attribute__((aligned(N)))
/// or _Alignas, we respect their choice and don't auto-promote. GCC behaves the same way:
/// explicit aligned(8) on a 24-byte struct gives 8-byte alignment, not 16.
fn effective_align(g: &IrGlobal) -> usize {
    if g.section.is_some() || g.has_explicit_align {
        return g.align;
    }
    if g.size >= 16 && g.align < 16 {
        16
    } else {
        g.align
    }
}

/// Emit a zero-initialized global variable (used in .bss, .tbss, and custom section zero-init).
fn emit_zero_global(out: &mut AsmOutput, g: &IrGlobal, obj_type: &str, ptr_dir: PtrDirective) {
    emit_symbol_directives(out, g);
    out.emit_fmt(format_args!(
        ".align {}",
        ptr_dir.align_arg(effective_align(g))
    ));
    out.emit_fmt(format_args!(".type {}, {}", g.name, obj_type));
    out.emit_fmt(format_args!(".size {}, {}", g.name, g.size));
    out.emit_fmt(format_args!("{}:", g.name));
    out.emit_fmt(format_args!("    .zero {}", g.size));
}

/// Target section classification for a global variable.
///
/// Each global is classified exactly once into one of these categories,
/// which determines which assembly section it belongs to.
#[derive(PartialEq, Eq)]
enum GlobalSection {
    /// Extern (undefined) symbol -- only needs visibility directive, no storage.
    Extern,
    /// Has `__attribute__((section(...)))` -- emitted in its custom section.
    Custom,
    /// Const-qualified, non-TLS, initialized, non-zero-size -> `.rodata`.
    Rodata,
    /// Thread-local, initialized, non-zero-size -> `.tdata`.
    Tdata,
    /// Non-const, non-TLS, initialized, non-zero-size -> `.data`.
    Data,
    /// Zero-initialized, `is_common` flag set -> `.comm` directive.
    Common,
    /// Thread-local, zero-initialized (or zero-size) -> `.tbss`.
    Tbss,
    /// Non-TLS, zero-initialized (or zero-size with init) -> `.bss`.
    Bss,
}

/// Classify a global variable into the section it should be emitted to.
///
/// The classification priority matches GCC behavior:
/// 1. Extern symbols get no storage (just visibility directives).
/// 2. Custom section overrides all other placement.
/// 3. TLS globals go to .tdata (initialized) or .tbss (zero-init).
/// 4. Const globals go to .rodata.
/// 5. Non-zero initialized non-const globals go to .data.
/// 6. Zero-initialized common globals go to .comm.
/// 7. Zero-initialized non-common globals go to .bss.
fn classify_global(g: &IrGlobal) -> GlobalSection {
    if g.is_extern {
        return GlobalSection::Extern;
    }
    if g.section.is_some() {
        return GlobalSection::Custom;
    }
    let is_zero = matches!(g.init, GlobalInit::Zero);
    let has_nonzero_init = !is_zero && g.size > 0;
    if g.is_thread_local {
        return if has_nonzero_init {
            GlobalSection::Tdata
        } else {
            GlobalSection::Tbss
        };
    }
    if has_nonzero_init {
        return if g.is_const {
            GlobalSection::Rodata
        } else {
            GlobalSection::Data
        };
    }
    // Zero-initialized (or zero-size with init)
    if g.is_common && is_zero {
        return GlobalSection::Common;
    }
    GlobalSection::Bss
}

/// Emit global variable definitions, grouped by target section.
///
/// Classifies each global once via `classify_global`, then emits all globals
/// for each section in a fixed order: extern visibility, custom sections,
/// .rodata, .tdata, .data, .comm, .tbss, .bss.
fn emit_globals(out: &mut AsmOutput, globals: &[IrGlobal], ptr_dir: PtrDirective) {
    // Phase 1: classify every global into its target section.
    let classified: Vec<GlobalSection> = globals.iter().map(classify_global).collect();

    // Phase 2: emit each section group in order.

    // Extern visibility directives (needed for PIC code so the assembler/linker knows
    // these symbols are resolved within the link unit).
    for (g, sect) in globals.iter().zip(&classified) {
        if matches!(sect, GlobalSection::Extern) {
            emit_visibility_directive(out, &g.name, &g.visibility);
            // For extern TLS variables, emit .type @tls_object so the assembler
            // creates a TLS-typed undefined symbol. Without this, the linker
            // reports "TLS definition mismatches non-TLS reference" when the
            // defining TU has the symbol in .tdata but this TU's reference
            // lacks TLS type information (defaults to STT_NOTYPE).
            if g.is_thread_local {
                out.emit_fmt(format_args!(".type {}, @tls_object", g.name));
            }
        }
    }

    // Custom section globals: each gets its own .section directive since they
    // may target different sections.
    for (g, sect) in globals.iter().zip(&classified) {
        if !matches!(sect, GlobalSection::Custom) {
            continue;
        }
        let section_name = g.section.as_ref().expect("custom section must have a name");
        // Use "a" (read-only) for const-qualified globals or rodata sections,
        // "aw" (writable) otherwise. GCC uses the const qualification of the
        // variable to determine section flags, not just the section name.
        // This matters for kernel sections like .modinfo which contain const data.
        let flags = if g.is_const || section_name.contains("rodata") {
            "a"
        } else {
            "aw"
        };
        // Sections starting with ".bss" are NOBITS (no file space, BSS semantics)
        let section_type = if section_name.starts_with(".bss") {
            "@nobits"
        } else {
            "@progbits"
        };
        out.emit_fmt(format_args!(
            ".section {},\"{}\",{}",
            section_name, flags, section_type
        ));
        if matches!(g.init, GlobalInit::Zero) || g.size == 0 {
            emit_zero_global(out, g, "@object", ptr_dir);
        } else {
            emit_global_def(out, g, ptr_dir);
        }
        out.emit("");
    }

    // .rodata: const-qualified initialized globals (matches GCC -fno-PIE behavior;
    // the linker handles relocations in .rodata fine, and kernel linker scripts
    // don't recognize .data.rel.ro).
    emit_section_group(
        out,
        globals,
        &classified,
        &GlobalSection::Rodata,
        ".section .rodata",
        false,
        ptr_dir,
    );

    // .tdata: thread-local initialized globals
    emit_section_group(
        out,
        globals,
        &classified,
        &GlobalSection::Tdata,
        ".section .tdata,\"awT\",@progbits",
        false,
        ptr_dir,
    );

    // .data: non-const initialized globals
    emit_section_group(
        out,
        globals,
        &classified,
        &GlobalSection::Data,
        ".section .data",
        false,
        ptr_dir,
    );

    // .comm: zero-initialized common globals (weak linkage, linker merges duplicates).
    // .comm alignment is always in bytes on all platforms, unlike .align.
    for (g, sect) in globals.iter().zip(&classified) {
        if matches!(sect, GlobalSection::Common) {
            out.emit_fmt(format_args!(
                ".comm {},{},{}",
                g.name,
                g.size,
                effective_align(g)
            ));
        }
    }

    // .tbss: thread-local zero-initialized globals
    emit_section_group(
        out,
        globals,
        &classified,
        &GlobalSection::Tbss,
        ".section .tbss,\"awT\",@nobits",
        true,
        ptr_dir,
    );

    // .bss: non-TLS zero-initialized globals (includes zero-size globals with
    // empty initializers like `Type arr[0] = {}` to avoid address overlap).
    emit_section_group(
        out,
        globals,
        &classified,
        &GlobalSection::Bss,
        ".section .bss",
        true,
        ptr_dir,
    );
}

/// Emit all globals matching `target` section, with a section header on first match.
/// If `is_zero` is true, emits as zero-initialized; otherwise as initialized data.
fn emit_section_group(
    out: &mut AsmOutput,
    globals: &[IrGlobal],
    classified: &[GlobalSection],
    target: &GlobalSection,
    section_header: &str,
    is_zero: bool,
    ptr_dir: PtrDirective,
) {
    let mut emitted_header = false;
    for (g, sect) in globals.iter().zip(classified) {
        if sect != target {
            continue;
        }
        if !emitted_header {
            out.emit(section_header);
            emitted_header = true;
        }
        if is_zero {
            let obj_type = if g.is_thread_local {
                "@tls_object"
            } else {
                "@object"
            };
            emit_zero_global(out, g, obj_type, ptr_dir);
        } else {
            emit_global_def(out, g, ptr_dir);
        }
    }
    if emitted_header {
        out.emit("");
    }
}

/// Emit a visibility directive (.hidden, .protected, .internal) for a symbol if applicable.
fn emit_visibility_directive(out: &mut AsmOutput, name: &str, visibility: &Option<String>) {
    if let Some(ref vis) = visibility {
        match vis.as_str() {
            "hidden" => out.emit_fmt(format_args!(".hidden {}", name)),
            "protected" => out.emit_fmt(format_args!(".protected {}", name)),
            "internal" => out.emit_fmt(format_args!(".internal {}", name)),
            _ => {} // "default" or unknown: no directive needed
        }
    }
}

/// Emit linkage directives (.globl or .weak) for a non-static symbol.
fn emit_linkage_directive(out: &mut AsmOutput, name: &str, is_static: bool, is_weak: bool) {
    if !is_static {
        if is_weak {
            out.emit_fmt(format_args!(".weak {}", name));
        } else {
            out.emit_fmt(format_args!(".globl {}", name));
        }
    }
}

/// Emit both linkage (.globl/.weak) and visibility (.hidden/.protected/.internal) directives.
fn emit_symbol_directives(out: &mut AsmOutput, g: &IrGlobal) {
    emit_linkage_directive(out, &g.name, g.is_static, g.is_weak);
    emit_visibility_directive(out, &g.name, &g.visibility);
}

/// Emit a single global variable definition.
fn emit_global_def(out: &mut AsmOutput, g: &IrGlobal, ptr_dir: PtrDirective) {
    emit_symbol_directives(out, g);
    out.emit_fmt(format_args!(
        ".align {}",
        ptr_dir.align_arg(effective_align(g))
    ));
    let obj_type = if g.is_thread_local {
        "@tls_object"
    } else {
        "@object"
    };
    out.emit_fmt(format_args!(".type {}, {}", g.name, obj_type));
    out.emit_fmt(format_args!(".size {}, {}", g.name, g.size));
    out.emit_fmt(format_args!("{}:", g.name));

    emit_init_data(out, &g.init, g.ty, g.size, ptr_dir);
}

/// Emit the data for a single GlobalInit element.
///
/// Handles all init variants: scalars, arrays, strings, global addresses, label diffs,
/// and compound initializers (which recurse into this function for each element).
/// `fallback_ty` is the declared element type of the enclosing global/array, used to
/// widen narrow constants (e.g., IrConst::I32(0) in a pointer array emits .quad 0).
/// `total_size` is the declared size of the enclosing global for padding calculations.
fn emit_init_data(
    out: &mut AsmOutput,
    init: &GlobalInit,
    fallback_ty: IrType,
    total_size: usize,
    ptr_dir: PtrDirective,
) {
    match init {
        GlobalInit::Zero => {
            out.emit_fmt(format_args!("    .zero {}", total_size));
        }
        GlobalInit::Scalar(c) => {
            emit_const_data(out, c, fallback_ty, ptr_dir);
        }
        GlobalInit::Array(values) => {
            // Coalesce consecutive zero-valued elements into .zero directives
            // to avoid emitting millions of individual `.byte 0` lines for
            // large partially-initialized arrays like `char x[500000]={'a'}`.
            let mut i = 0;
            while i < values.len() {
                let val = &values[i];
                let const_ty = const_natural_type(val, fallback_ty);
                // Only widen integer constants to fallback_ty (e.g., I32(0) in a pointer
                // array should emit .quad 0). Float constants (F32, F64, LongDouble) must
                // keep their natural size -- complex arrays store F32 pairs where each zero
                // imaginary slot is exactly 4 bytes, not pointer-sized.
                let elem_ty = if fallback_ty.size() > const_ty.size() && const_ty.is_integer() {
                    fallback_ty
                } else {
                    const_ty
                };

                if val.is_zero() {
                    // Count consecutive zero elements and emit as a single .zero
                    let elem_size = elem_ty.size();
                    let mut zero_count = 1usize;
                    while i + zero_count < values.len() && values[i + zero_count].is_zero() {
                        zero_count += 1;
                    }
                    let zero_bytes = zero_count * elem_size;
                    if zero_bytes > 0 {
                        out.emit_fmt(format_args!("    .zero {}", zero_bytes));
                    }
                    i += zero_count;
                } else {
                    emit_const_data(out, val, elem_ty, ptr_dir);
                    i += 1;
                }
            }
        }
        GlobalInit::String(s) => {
            let string_chars = s.chars().count();
            let string_bytes_with_nul = string_chars + 1;
            if string_bytes_with_nul <= total_size {
                // NUL terminator fits: use .asciz (emits string + NUL)
                out.emit_fmt(format_args!("    .asciz \"{}\"", escape_string(s)));
                if total_size > string_bytes_with_nul {
                    out.emit_fmt(format_args!(
                        "    .zero {}",
                        total_size - string_bytes_with_nul
                    ));
                }
            } else {
                // NUL terminator doesn't fit (C11 6.7.9 p14): truncate to array size.
                // Use .ascii (no implicit NUL) with the string truncated to total_size chars.
                let truncated: String = s.chars().take(total_size).collect();
                out.emit_fmt(format_args!("    .ascii \"{}\"", escape_string(&truncated)));
            }
        }
        GlobalInit::WideString(chars) => {
            emit_wide_string(out, chars);
            let wide_bytes = (chars.len() + 1) * 4;
            if total_size > wide_bytes {
                out.emit_fmt(format_args!("    .zero {}", total_size - wide_bytes));
            }
        }
        GlobalInit::Char16String(chars) => {
            emit_char16_string(out, chars);
            let char16_bytes = (chars.len() + 1) * 2;
            if total_size > char16_bytes {
                out.emit_fmt(format_args!("    .zero {}", total_size - char16_bytes));
            }
        }
        GlobalInit::GlobalAddr(label) => {
            out.emit_fmt(format_args!("    {} {}", ptr_dir.as_str(), label));
        }
        GlobalInit::GlobalAddrOffset(label, offset) => {
            if *offset >= 0 {
                out.emit_fmt(format_args!(
                    "    {} {}+{}",
                    ptr_dir.as_str(),
                    label,
                    offset
                ));
            } else {
                out.emit_fmt(format_args!("    {} {}{}", ptr_dir.as_str(), label, offset));
            }
        }
        GlobalInit::GlobalLabelDiff(lab1, lab2, byte_size) => {
            emit_label_diff(out, lab1, lab2, *byte_size);
        }
        GlobalInit::Compound(elements) => {
            for elem in elements {
                // Compound elements are self-typed: each element knows its own size.
                // For Scalar elements, use the constant's natural type (falling back
                // to the enclosing global's type for I64/wider constants).
                emit_compound_element(out, elem, fallback_ty, ptr_dir);
            }
        }
    }
}

/// Emit a single element within a Compound initializer.
///
/// Most variants delegate to the shared emit_init_data. Scalar elements use the
/// constant's natural type rather than the enclosing global's type, since compound
/// elements may have heterogeneous types (e.g., struct with int and pointer fields).
fn emit_compound_element(
    out: &mut AsmOutput,
    elem: &GlobalInit,
    fallback_ty: IrType,
    ptr_dir: PtrDirective,
) {
    match elem {
        GlobalInit::Scalar(c) => {
            // In compound initializers, each element may have a different type.
            // Use the constant's own type, falling back to fallback_ty for I64 and wider.
            let elem_ty = const_natural_type(c, fallback_ty);
            emit_const_data(out, c, elem_ty, ptr_dir);
        }
        GlobalInit::Zero => {
            // Zero element in compound: emit a single pointer-sized zero
            out.emit_fmt(format_args!("    {} 0", ptr_dir.as_str()));
        }
        GlobalInit::Compound(elements) => {
            // Nested compound: recurse into each element
            for inner in elements {
                emit_compound_element(out, inner, fallback_ty, ptr_dir);
            }
        }
        // All other variants (GlobalAddr, GlobalAddrOffset, WideString, etc.)
        // delegate to the shared handler with zero total_size (no padding).
        other => emit_init_data(out, other, fallback_ty, 0, ptr_dir),
    }
}

/// Get the natural IR type of a constant, falling back to `default_ty` for
/// types that don't have a narrower representation (I64, I128, etc.).
fn const_natural_type(c: &IrConst, default_ty: IrType) -> IrType {
    match c {
        IrConst::I8(_) => IrType::I8,
        IrConst::I16(_) => IrType::I16,
        IrConst::I32(_) => IrType::I32,
        IrConst::F32(_) => IrType::F32,
        IrConst::F64(_) => IrType::F64,
        IrConst::LongDouble(..) => IrType::F128,
        _ => default_ty,
    }
}

/// Emit a wide string (wchar_t) as .long directives with null terminator.
fn emit_wide_string(out: &mut AsmOutput, chars: &[u32]) {
    for &ch in chars {
        out.emit_fmt(format_args!("    .long {}", ch));
    }
    out.emit("    .long 0"); // null terminator
}

/// Emit a char16_t string as .short directives with null terminator.
fn emit_char16_string(out: &mut AsmOutput, chars: &[u16]) {
    for &ch in chars {
        out.emit_fmt(format_args!("    .short {}", ch));
    }
    out.emit("    .short 0"); // null terminator
}

/// Emit a label difference as a sized assembly directive (`.long lab1-lab2`, etc.).
fn emit_label_diff(out: &mut AsmOutput, lab1: &str, lab2: &str, byte_size: usize) {
    let dir = match byte_size {
        1 => ".byte",
        2 => ".short",
        4 => ".long",
        _ => ".quad",
    };
    out.emit_fmt(format_args!("    {} {}-{}", dir, lab1, lab2));
}

/// Emit a 64-bit value as two `.long` directives in little-endian order.
/// Used on i686 (32-bit) targets where 64-bit values must be split.
#[inline]
fn emit_u64_as_long_pair(out: &mut AsmOutput, bits: u64) {
    out.emit_fmt(format_args!("    .long {}", bits as u32));
    out.emit_fmt(format_args!("    .long {}", (bits >> 32) as u32));
}

pub fn emit_const_data(out: &mut AsmOutput, c: &IrConst, ty: IrType, ptr_dir: PtrDirective) {
    match c {
        // Integer constants: all share the same widening/narrowing logic.
        // The value is sign-extended to i64, then emitted at the target type's width.
        IrConst::I8(v) => emit_int_data(out, *v as i64, ty, ptr_dir),
        IrConst::I16(v) => emit_int_data(out, *v as i64, ty, ptr_dir),
        IrConst::I32(v) => emit_int_data(out, *v as i64, ty, ptr_dir),
        IrConst::I64(v) => emit_int_data(out, *v, ty, ptr_dir),
        IrConst::F32(v) => {
            out.emit_fmt(format_args!("    .long {}", v.to_bits()));
        }
        IrConst::F64(v) => {
            let bits = v.to_bits();
            if ptr_dir.is_32bit() {
                emit_u64_as_long_pair(out, bits);
            } else {
                out.emit_fmt(format_args!("    {} {}", ptr_dir.as_str(), bits));
            }
        }
        IrConst::LongDouble(f64_val, f128_bytes) => {
            if ptr_dir.is_x86() {
                // x86: convert f128 bytes to x87 80-bit extended precision for emission.
                // x87 80-bit format = 10 bytes: 8 bytes (significand+exp low) + 2 bytes (exp high+sign)
                let x87 = crate::common::long_double::f128_bytes_to_x87_bytes(f128_bytes);
                let lo = u64::from_le_bytes(x87[0..8].try_into().unwrap());
                let hi = u64::from_le_bytes([x87[8], x87[9], 0, 0, 0, 0, 0, 0]);
                if ptr_dir.is_32bit() {
                    emit_u64_as_long_pair(out, lo);
                    // x87 80-bit: third .long holds the upper 2 bytes
                    out.emit_fmt(format_args!("    .long {}", hi as u32));
                } else {
                    out.emit_fmt(format_args!("    {} {}", ptr_dir.as_str(), lo as i64));
                    out.emit_fmt(format_args!("    {} {}", ptr_dir.as_str(), hi as i64));
                }
            } else if ptr_dir.is_riscv() || ptr_dir.is_arm() {
                // RISC-V and ARM64: f128 bytes are already in IEEE 754 binary128 format.
                let lo = u64::from_le_bytes(f128_bytes[0..8].try_into().unwrap());
                let hi = u64::from_le_bytes(f128_bytes[8..16].try_into().unwrap());
                out.emit_fmt(format_args!("    {} {}", ptr_dir.as_str(), lo as i64));
                out.emit_fmt(format_args!("    {} {}", ptr_dir.as_str(), hi as i64));
            } else {
                // Fallback: store f64 approximation (should not normally be reached).
                let f64_bits = f64_val.to_bits();
                out.emit_fmt(format_args!("    {} {}", ptr_dir.as_str(), f64_bits as i64));
                out.emit_fmt(format_args!("    {} 0", ptr_dir.as_str()));
            }
        }
        IrConst::I128(v) => {
            let lo = *v as u64;
            let hi = (*v >> 64) as u64;
            if ptr_dir.is_32bit() {
                emit_u64_as_long_pair(out, lo);
                emit_u64_as_long_pair(out, hi);
            } else {
                // 64-bit targets: emit as two 64-bit values (little-endian: low quad first)
                out.emit_fmt(format_args!("    {} {}", ptr_dir.as_str(), lo as i64));
                out.emit_fmt(format_args!("    {} {}", ptr_dir.as_str(), hi as i64));
            }
        }
        IrConst::Zero => {
            let size = ty.size();
            out.emit_fmt(format_args!(
                "    .zero {}",
                if size == 0 { 4 } else { size }
            ));
        }
    }
}

/// Emit an integer constant at the width specified by `ty`.
/// Truncates or sign-extends `val` (an i64) as needed to match the target width.
fn emit_int_data(out: &mut AsmOutput, val: i64, ty: IrType, ptr_dir: PtrDirective) {
    match ty {
        IrType::I8 | IrType::U8 => out.emit_fmt(format_args!("    .byte {}", val as u8)),
        IrType::I16 | IrType::U16 => out.emit_fmt(format_args!("    .short {}", val as u16)),
        IrType::I32 | IrType::U32 => out.emit_fmt(format_args!("    .long {}", val as u32)),
        // On i686 (32-bit), pointers are 4 bytes -- emit a single .long, not two.
        IrType::Ptr if ptr_dir.is_32bit() => {
            out.emit_fmt(format_args!("    .long {}", val as u32));
        }
        _ => {
            if ptr_dir.is_32bit() {
                emit_u64_as_long_pair(out, val as u64);
            } else {
                out.emit_fmt(format_args!("    {} {}", ptr_dir.as_str(), val));
            }
        }
    }
}

/// Emit string literal as .byte directives with null terminator.
/// Each char in the string is treated as a raw byte value (0-255),
/// not as a UTF-8 encoded character. This is correct for C narrow
/// string literals where \xNN escapes produce single bytes.
///
/// Writes directly into the output buffer without any intermediate
/// heap allocations (no per-byte String, no Vec, no join). Uses
/// a pre-computed lookup table to convert bytes to decimal strings
/// without fmt::Write overhead.
pub fn emit_string_bytes(out: &mut AsmOutput, s: &str) {
    // Chunk output into lines of at most 32 bytes each to avoid
    // extremely long lines that can cause parser performance issues.
    let mut count = 0;
    for c in s.chars() {
        if count % 32 == 0 {
            if count > 0 {
                out.buf.push('\n');
            }
            out.buf.push_str("    .byte ");
        } else {
            out.buf.push_str(", ");
        }
        push_u8_decimal(&mut out.buf, c as u8);
        count += 1;
    }
    // Null terminator
    if count % 32 == 0 {
        if count > 0 {
            out.buf.push('\n');
        }
        out.buf.push_str("    .byte 0\n");
    } else {
        out.buf.push_str(", 0\n");
    }
}

/// Append a u8 value as a decimal string directly into the buffer.
/// Avoids fmt::Write overhead by using direct digit extraction.
#[inline]
fn push_u8_decimal(buf: &mut String, v: u8) {
    if v >= 100 {
        buf.push((b'0' + v / 100) as char);
        buf.push((b'0' + (v / 10) % 10) as char);
        buf.push((b'0' + v % 10) as char);
    } else if v >= 10 {
        buf.push((b'0' + v / 10) as char);
        buf.push((b'0' + v % 10) as char);
    } else {
        buf.push((b'0' + v) as char);
    }
}

/// Escape a string for use in assembly .asciz directives.
pub fn escape_string(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        match c {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\n' => result.push_str("\\n"),
            '\t' => result.push_str("\\t"),
            '\r' => result.push_str("\\r"),
            '\0' => result.push_str("\\000"),
            c if c.is_ascii_graphic() || c == ' ' => result.push(c),
            c => {
                // Emit the raw byte value (char as u8), not UTF-8 encoding
                use std::fmt::Write;
                let _ = write!(result, "\\{:03o}", c as u8);
            }
        }
    }
    result
}
