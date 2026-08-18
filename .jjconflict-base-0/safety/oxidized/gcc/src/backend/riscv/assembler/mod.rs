//! Native RISC-V assembler.
//!
//! Parses `.s` assembly text (as emitted by the RISC-V codegen) and produces
//! ELF `.o` object files, removing the dependency on `riscv64-linux-gnu-gcc`
//! for assembly.
//!
//! Architecture:
//! - `parser.rs`     – Tokenize + parse assembly text into `AsmStatement` items
//! - `encoder.rs`    – Encode RISC-V instructions into 32-bit machine words
//! - `compress.rs`   – RV64C compressed instruction support (32-bit → 16-bit)
//! - `elf_writer.rs` – Write ELF object files with sections, symbols, and relocations

pub mod compress;
pub mod elf_writer;
pub mod encoder;
pub mod parser;

use crate::backend::elf::{ELFCLASS32, ELFCLASS64};
use elf_writer::{
    ElfWriter, EF_RISCV_FLOAT_ABI_DOUBLE, EF_RISCV_FLOAT_ABI_QUAD, EF_RISCV_FLOAT_ABI_SINGLE,
    EF_RISCV_RVC,
};
use parser::parse_asm;

/// Assemble RISC-V assembly text into an ELF object file, with extra args.
///
/// Supports `-mabi=` to control ELF float ABI flags and ELF class (32/64-bit),
/// and `-march=` to control ELF class (rv32 vs rv64) and RVC flag.
pub fn assemble_with_args(
    asm_text: &str,
    output_path: &str,
    extra_args: &[String],
) -> Result<(), String> {
    let statements = parse_asm(asm_text)?;
    let mut writer = ElfWriter::new();

    // Collect ABI and arch info from extra args (last value wins, matching GCC behavior)
    let mut abi_name: Option<String> = None;
    let mut march_name: Option<String> = None;
    for arg in extra_args {
        if let Some(abi) = arg.strip_prefix("-mabi=") {
            abi_name = Some(abi.to_string());
        }
        if let Some(march) = arg.strip_prefix("-march=") {
            march_name = Some(march.to_string());
        }
    }

    // Determine RVC from -march= (check for 'c' extension in the arch string)
    let has_rvc = match &march_name {
        Some(march) => march_has_c_extension(march),
        None => true, // default: assume RVC (matches rv64gc default)
    };

    // Set ELF flags based on ABI + RVC
    if let Some(ref abi) = abi_name {
        writer.set_elf_flags(elf_flags_for_abi(abi, has_rvc));
        if abi.starts_with("ilp32") {
            writer.set_elf_class(ELFCLASS32);
        } else {
            writer.set_elf_class(ELFCLASS64);
        }
    } else if !has_rvc {
        // No -mabi= but -march= without 'c': clear RVC from default flags
        let default_flags = EF_RISCV_FLOAT_ABI_DOUBLE;
        writer.set_elf_flags(default_flags);
    }

    // -march= overrides ELF class (takes precedence, processed after -mabi=)
    if let Some(ref march) = march_name {
        if march.starts_with("rv32") {
            writer.set_elf_class(ELFCLASS32);
        } else if march.starts_with("rv64") {
            writer.set_elf_class(ELFCLASS64);
        }
    }

    writer.process_statements(&statements)?;
    writer.write_elf(output_path)?;
    Ok(())
}

/// Map an ABI name to ELF e_flags, with optional RVC flag.
fn elf_flags_for_abi(abi: &str, has_rvc: bool) -> u32 {
    let float_abi = match abi {
        "lp64" | "ilp32" => 0x0, // soft-float
        "lp64f" | "ilp32f" => EF_RISCV_FLOAT_ABI_SINGLE,
        "lp64d" | "ilp32d" => EF_RISCV_FLOAT_ABI_DOUBLE,
        "lp64q" | "ilp32q" => EF_RISCV_FLOAT_ABI_QUAD,
        _ => EF_RISCV_FLOAT_ABI_DOUBLE, // default
    };
    if has_rvc {
        float_abi | EF_RISCV_RVC
    } else {
        float_abi
    }
}

/// Check if a -march= string includes the 'c' (compressed) extension.
/// Handles both shorthand (rv64gc) and explicit (rv64imafdc_zicsr) formats.
fn march_has_c_extension(march: &str) -> bool {
    // Strip the rv32/rv64 prefix
    let rest = if march.starts_with("rv32") || march.starts_with("rv64") {
        &march[4..]
    } else {
        march
    };
    // The base ISA letters come before the first '_' (extension separator)
    let base = rest.split('_').next().unwrap_or(rest);
    // 'g' expands to 'imafd' (no 'c'), so only check for explicit 'c'
    base.contains('c')
}
