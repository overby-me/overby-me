//! ELF relocatable object file writer for x86-64.
//!
//! Thin wrapper around `ElfWriterCore` that provides x86-64-specific
//! instruction encoding and relocation types. All shared logic (section
//! management, label tracking, jump relaxation, ELF emission) lives in
//! `backend::elf_writer_common`.

use super::encoder::*;
use crate::backend::elf::{ELFCLASS64, EM_X86_64};
use crate::backend::elf_writer_common::{
    ElfWriterCore, EncodeResult, EncoderReloc, JumpDetection, X86Arch,
};

/// x86-64 architecture implementation for the shared ELF writer.
pub struct X86_64Arch;

impl X86Arch for X86_64Arch {
    fn encode_instruction(
        instr: &Instruction,
        section_data_len: u64,
    ) -> Result<EncodeResult, String> {
        let mut encoder = InstructionEncoder::new();
        encoder.offset = section_data_len;
        encoder.encode(instr)?;

        let instr_len = encoder.bytes.len();

        // Detect jump instructions for relaxation
        let jump = {
            let mnem = &instr.mnemonic;
            let is_jump = mnem.starts_with('j') && mnem.len() >= 2;
            if is_jump && instr.operands.len() == 1 {
                if let Operand::Label(_) = &instr.operands[0] {
                    let is_conditional = mnem != "jmp";
                    let expected_len = if is_conditional { 6 } else { 5 };
                    if instr_len == expected_len {
                        Some(JumpDetection {
                            is_conditional,
                            already_short: false,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };

        let relocations = encoder
            .relocations
            .into_iter()
            .map(|r| EncoderReloc {
                offset: r.offset,
                symbol: r.symbol,
                reloc_type: r.reloc_type,
                addend: r.addend,
                diff_symbol: None,
            })
            .collect();

        Ok(EncodeResult {
            bytes: encoder.bytes,
            relocations,
            jump,
        })
    }

    fn elf_machine() -> u16 {
        EM_X86_64
    }
    fn elf_class() -> u8 {
        ELFCLASS64
    }

    fn reloc_abs(size: usize) -> u32 {
        match size {
            2 => R_X86_64_16,
            4 => R_X86_64_32,
            _ => R_X86_64_64,
        }
    }
    fn reloc_abs64() -> u32 {
        R_X86_64_64
    }
    fn reloc_pc32() -> u32 {
        R_X86_64_PC32
    }
    fn reloc_plt32() -> u32 {
        R_X86_64_PLT32
    }

    fn uses_rel_format() -> bool {
        false
    }

    fn reloc_pc8_internal() -> Option<u32> {
        Some(R_X86_64_PC8_INTERNAL)
    }
    fn reloc_abs32_for_internal() -> Option<u32> {
        Some(R_X86_64_32)
    }
    fn supports_deferred_skips() -> bool {
        true
    }
    fn resolve_set_aliases_in_data() -> bool {
        true
    }
}

/// Builds an ELF relocatable object file from parsed assembly items.
pub type ElfWriter = ElfWriterCore<X86_64Arch>;
