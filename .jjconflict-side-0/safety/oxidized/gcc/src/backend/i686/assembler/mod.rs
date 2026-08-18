//! Native i686 (32-bit x86) assembler: parses AT&T syntax assembly and produces
//! 32-bit ELF .o files.
//!
//! Reuses the x86-64 parser since AT&T syntax is identical, but provides its own
//! instruction encoder (no REX prefixes, 32-bit default operand size) and 32-bit
//! ELF writer (ELFCLASS32, EM_386, Elf32_Sym/Elf32_Rel).
//!
//! Architecture:
//! - Parser: reused from `super::super::x86::assembler::parser`
//! - `encoder.rs` – Encode i686 instructions into machine code bytes (no REX)
//! - `elf_writer.rs` – Write 32-bit ELF object files

pub mod elf_writer;
pub mod encoder;

// Re-export the x86 parser – AT&T syntax is the same for both architectures
pub use crate::backend::x86::assembler::parser::parse_asm;

use elf_writer::ElfWriter;

/// Assemble AT&T syntax i686 assembly text into a 32-bit ELF object file.
///
/// This is the default assembler (used when the `gcc_assembler` feature is disabled).
pub fn assemble(asm_text: &str, output_path: &str) -> Result<(), String> {
    let items = parse_asm(asm_text)?;
    let obj = ElfWriter::new();
    let elf_bytes = obj.build(&items)?;
    std::fs::write(output_path, &elf_bytes)
        .map_err(|e| format!("Failed to write object file: {}", e))?;
    Ok(())
}
