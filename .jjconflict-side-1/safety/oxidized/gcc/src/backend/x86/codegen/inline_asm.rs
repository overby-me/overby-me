//! X86 inline assembly template substitution and register formatting.
//!
//! This module provides helper methods for processing x86-64 inline assembly
//! operands. Register conversion and condition code mapping delegate to the
//! shared `x86_common` module. Template substitution uses the shared parsing
//! logic with an x86-64-specific operand emission callback.

use super::emit::X86Codegen;
use crate::backend::x86_common;
use crate::common::types::IrType;
use crate::ir::reexports::BlockId;
use std::borrow::Cow;
use std::fmt::Write;

impl X86Codegen {
    /// Substitute %0, %1, %[name], %k0, %b1, %w2, %q3, %h4, %c0, %P0, %a0, %n0, %l[name] etc.
    /// in x86 asm template.
    ///
    /// Delegates to the shared x86 template parser with an x86-64-specific
    /// operand emission callback.
    pub(super) fn substitute_x86_asm_operands(
        line: &str,
        op_regs: &[String],
        op_names: &[Option<String>],
        op_is_memory: &[bool],
        op_mem_addrs: &[String],
        op_types: &[IrType],
        gcc_to_internal: &[usize],
        goto_labels: &[(String, BlockId)],
        op_imm_values: &[Option<i64>],
        op_imm_symbols: &[Option<String>],
    ) -> String {
        x86_common::substitute_x86_asm_operands(
            line,
            op_regs,
            op_names,
            op_is_memory,
            op_mem_addrs,
            op_types,
            gcc_to_internal,
            goto_labels,
            op_imm_values,
            op_imm_symbols,
            Self::emit_operand_with_modifier,
        )
    }

    /// Emit a single operand with the given modifier into the result string.
    /// Shared helper for both named and positional operand substitution.
    ///
    /// Handles x86-64-specific behavior:
    /// - `%a` with symbol emits `symbol(%rip)` (RIP-relative addressing)
    /// - Default register width is 64-bit (no modifier needed for `rax`)
    fn emit_operand_with_modifier(
        result: &mut String,
        idx: usize,
        modifier: Option<char>,
        op_regs: &[String],
        op_is_memory: &[bool],
        op_mem_addrs: &[String],
        op_types: &[IrType],
        op_imm_values: &[Option<i64>],
        op_imm_symbols: &[Option<String>],
    ) {
        // Try shared logic first (handles %n, %c/%P, memory, $symbol, $imm)
        if x86_common::emit_operand_common(
            result,
            idx,
            modifier,
            op_regs,
            op_is_memory,
            op_mem_addrs,
            op_imm_values,
            op_imm_symbols,
        ) {
            return;
        }

        let has_symbol = op_imm_symbols.get(idx).and_then(|s| s.as_ref());
        let has_imm = op_imm_values.get(idx).and_then(|v| v.as_ref());

        if modifier == Some('a') {
            // %a: emit as address reference (x86-64 uses RIP-relative)
            if let Some(sym) = has_symbol {
                let _ = write!(result, "{}(%rip)", sym);
            } else if let Some(imm) = has_imm {
                result.push_str(&imm.to_string());
            } else if op_is_memory[idx] {
                result.push_str(&op_mem_addrs[idx]);
            } else {
                let _ = write!(result, "(%{})", op_regs[idx]);
            }
        } else {
            // Register operand — apply size modifier, default is 64-bit
            let effective_mod =
                modifier.or_else(|| Self::default_modifier_for_type(op_types.get(idx).copied()));
            result.push('%');
            result.push_str(&Self::format_x86_reg(&op_regs[idx], effective_mod));
        }
    }

    /// Determine the default register size modifier based on the operand's IR type.
    /// In GCC inline asm, the default register size matches the C type:
    /// - 8-bit types -> %al (modifier 'b')
    /// - 16-bit types -> %ax (modifier 'w')
    /// - 32-bit types -> %eax (modifier 'k')
    /// - 64-bit types -> %rax (no modifier / 'q')
    pub(super) fn default_modifier_for_type(ty: Option<IrType>) -> Option<char> {
        match ty {
            Some(IrType::I8) | Some(IrType::U8) => Some('b'),
            Some(IrType::I16) | Some(IrType::U16) => Some('w'),
            Some(IrType::I32) | Some(IrType::U32) | Some(IrType::F32) => Some('k'),
            // I64, U64, Ptr, F64 all use 64-bit registers (default)
            _ => None,
        }
    }

    /// Format x86-64 register with size modifier.
    /// Modifiers: k (32-bit), w (16-bit), b (8-bit low), h (8-bit high), q (64-bit), l (32-bit alt)
    /// XMM registers (xmm0-xmm15) have no size variants and are returned as-is.
    pub(super) fn format_x86_reg<'a>(reg: &'a str, modifier: Option<char>) -> Cow<'a, str> {
        // XMM registers don't have size variants
        if reg.starts_with("xmm") {
            return Cow::Borrowed(reg);
        }
        // x87 FPU stack registers don't have size variants
        if reg.starts_with("st(") || reg == "st" {
            return Cow::Borrowed(reg);
        }
        match modifier {
            Some('k') | Some('l') => x86_common::reg_to_32(reg),
            Some('w') => x86_common::reg_to_16(reg),
            Some('b') => x86_common::reg_to_8l(reg),
            Some('h') => x86_common::reg_to_8h(reg),
            Some('q') | None => Cow::Borrowed(reg),
            _ => Cow::Borrowed(reg),
        }
    }

    /// Convert 64-bit register name to 32-bit variant.
    pub(super) fn reg_to_32<'a>(reg: &'a str) -> Cow<'a, str> {
        x86_common::reg_to_32(reg)
    }

    /// Convert any register name (8/16/32-bit) to its 64-bit equivalent.
    /// If the register is already 64-bit, returns it unchanged.
    pub(super) fn reg_to_64<'a>(reg: &'a str) -> Cow<'a, str> {
        match reg {
            "eax" | "ax" | "al" | "ah" => Cow::Borrowed("rax"),
            "ebx" | "bx" | "bl" | "bh" => Cow::Borrowed("rbx"),
            "ecx" | "cx" | "cl" | "ch" => Cow::Borrowed("rcx"),
            "edx" | "dx" | "dl" | "dh" => Cow::Borrowed("rdx"),
            "esi" | "si" | "sil" => Cow::Borrowed("rsi"),
            "edi" | "di" | "dil" => Cow::Borrowed("rdi"),
            "ebp" | "bp" | "bpl" => Cow::Borrowed("rbp"),
            "esp" | "sp" | "spl" => Cow::Borrowed("rsp"),
            "r8d" | "r8w" | "r8b" => Cow::Borrowed("r8"),
            "r9d" | "r9w" | "r9b" => Cow::Borrowed("r9"),
            "r10d" | "r10w" | "r10b" => Cow::Borrowed("r10"),
            "r11d" | "r11w" | "r11b" => Cow::Borrowed("r11"),
            "r12d" | "r12w" | "r12b" => Cow::Borrowed("r12"),
            "r13d" | "r13w" | "r13b" => Cow::Borrowed("r13"),
            "r14d" | "r14w" | "r14b" => Cow::Borrowed("r14"),
            "r15d" | "r15w" | "r15b" => Cow::Borrowed("r15"),
            _ => Cow::Borrowed(reg),
        }
    }

    /// Convert 64-bit register name to 16-bit variant.
    pub(super) fn reg_to_16<'a>(reg: &'a str) -> Cow<'a, str> {
        x86_common::reg_to_16(reg)
    }

    /// Convert 64-bit register name to 8-bit low variant.
    pub(super) fn reg_to_8l<'a>(reg: &'a str) -> Cow<'a, str> {
        x86_common::reg_to_8l(reg)
    }

    /// Map GCC inline asm condition code suffix to x86 SETcc suffix.
    pub(super) fn gcc_cc_to_x86(cond: &str) -> &'static str {
        x86_common::gcc_cc_to_x86(cond)
    }
}
