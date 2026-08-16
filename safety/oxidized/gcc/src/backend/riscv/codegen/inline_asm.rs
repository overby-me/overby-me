//! RISC-V inline assembly operand handling and template substitution.
//!
//! This module contains the constraint classification types and functions for
//! RISC-V inline assembly, as well as the operand formatting and template
//! substitution logic used by `emit_inline_asm` in the main codegen module.

use super::emit::RiscvCodegen;
use std::fmt::Write;

/// Constraint classification for RISC-V inline asm operands.
#[derive(Clone, PartialEq)]
pub(super) enum RvConstraintKind {
    GpReg,            // "r" - general purpose register
    FpReg,            // "f" - floating point register
    Memory,           // "m" - memory (offset(s0))
    Address,          // "A" - address for AMO instructions (produces (reg) format)
    Immediate,        // "I", "i" - immediate value
    ZeroOrReg,        // "J" in "rJ" - zero register or GP reg
    Specific(String), // specific register name
    Tied(usize),      // tied to output operand N
}

/// Classify a RISC-V inline asm constraint string into its kind.
// TODO: Support multi-alternative constraint parsing (e.g., "rm", "Ir") like x86.
// Currently only single-alternative constraints are recognized.
pub(super) fn classify_rv_constraint(constraint: &str) -> RvConstraintKind {
    let c = constraint.trim_start_matches(['=', '+', '&', '%']);
    // Check for tied operand (all digits)
    if !c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit()) {
        if let Ok(n) = c.parse::<usize>() {
            return RvConstraintKind::Tied(n);
        }
    }
    match c {
        "m" => RvConstraintKind::Memory,
        "A" => RvConstraintKind::Address,
        "f" => RvConstraintKind::FpReg,
        "I" | "i" | "n" => RvConstraintKind::Immediate,
        "J" => RvConstraintKind::ZeroOrReg,
        "rJ" => RvConstraintKind::ZeroOrReg,
        "a0" | "a1" | "a2" | "a3" | "a4" | "a5" | "a6" | "a7" | "ra" | "t0" | "t1" | "t2" => {
            RvConstraintKind::Specific(c.to_string())
        }
        _ if c.starts_with("ft") || c.starts_with("fa") || c.starts_with("fs") => {
            RvConstraintKind::Specific(c.to_string())
        }
        _ => RvConstraintKind::GpReg,
    }
}

impl RiscvCodegen {
    /// Format an operand for substitution based on its constraint kind.
    pub(super) fn format_operand(
        idx: usize,
        op_regs: &[String],
        op_kinds: &[RvConstraintKind],
        op_mem_offsets: &[i64],
        op_mem_addrs: &[String],
        op_imm_values: &[Option<i64>],
        op_imm_symbols: &[Option<String>],
        use_addr_format: bool, // true for Address kind
    ) -> String {
        if idx >= op_kinds.len() {
            return String::new();
        }
        match &op_kinds[idx] {
            RvConstraintKind::Memory => {
                // If mem_addr is set (non-alloca pointer dereference), use it directly
                if idx < op_mem_addrs.len() && !op_mem_addrs[idx].is_empty() {
                    op_mem_addrs[idx].clone()
                } else {
                    format!("{}(s0)", op_mem_offsets[idx])
                }
            }
            RvConstraintKind::Address => {
                // Address operands produce (register) format for AMO/LR/SC
                if use_addr_format {
                    format!("({})", &op_regs[idx])
                } else {
                    op_regs[idx].clone()
                }
            }
            RvConstraintKind::Immediate => {
                // Check for immediate symbol first (e.g., string literal, function/variable name)
                if let Some(Some(ref sym)) = op_imm_symbols.get(idx) {
                    return sym.clone();
                }
                // Emit the immediate value directly
                if let Some(imm) = op_imm_values[idx] {
                    imm.to_string()
                } else {
                    "0".to_string()
                }
            }
            _ => op_regs[idx].clone(),
        }
    }

    /// Substitute %0, %1, %[name], %z0, etc. in RISC-V asm template.
    pub(super) fn substitute_riscv_asm_operands(
        line: &str,
        op_regs: &[String],
        op_names: &[Option<String>],
        op_kinds: &[RvConstraintKind],
        op_mem_offsets: &[i64],
        op_mem_addrs: &[String],
        op_imm_values: &[Option<i64>],
        op_imm_symbols: &[Option<String>],
        gcc_to_internal: &[usize],
    ) -> String {
        let mut result = String::new();
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '%' && i + 1 < chars.len() {
                i += 1;
                // %% -> literal %
                if chars[i] == '%' {
                    result.push('%');
                    i += 1;
                    continue;
                }

                // Check for modifiers: %z (zero-if-zero), %lo, %hi
                if chars[i] == 'z' && i + 1 < chars.len() {
                    // %z modifier: emit "zero" if operand value is 0, else register name
                    i += 1;
                    if chars[i] == '[' {
                        // %z[name]
                        i += 1;
                        let name_start = i;
                        while i < chars.len() && chars[i] != ']' {
                            i += 1;
                        }
                        let name: String = chars[name_start..i].iter().collect();
                        if i < chars.len() {
                            i += 1;
                        }
                        let mut found = false;
                        for (idx, op_name) in op_names.iter().enumerate() {
                            if let Some(ref n) = op_name {
                                if n == &name {
                                    // Check if operand is zero
                                    if let Some(imm) = op_imm_values[idx] {
                                        if imm == 0 {
                                            result.push_str("zero");
                                        } else {
                                            result.push_str(&op_regs[idx]);
                                        }
                                    } else {
                                        result.push_str(&op_regs[idx]);
                                    }
                                    found = true;
                                    break;
                                }
                            }
                        }
                        if !found {
                            result.push_str("%z[");
                            result.push_str(&name);
                            result.push(']');
                        }
                    } else if chars[i].is_ascii_digit() {
                        // %z0, %z1, etc.
                        let mut num = 0usize;
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            num = num * 10 + (chars[i] as usize - '0' as usize);
                            i += 1;
                        }
                        let internal_idx = if num < gcc_to_internal.len() {
                            gcc_to_internal[num]
                        } else {
                            num
                        };
                        if internal_idx < op_regs.len() {
                            // Check if the operand is a constant zero (rJ constraint with zero value)
                            if op_regs[internal_idx] == "zero" {
                                result.push_str("zero");
                            } else if let Some(imm) =
                                op_imm_values.get(internal_idx).and_then(|v| *v)
                            {
                                if imm == 0 {
                                    result.push_str("zero");
                                } else {
                                    result.push_str(&op_regs[internal_idx]);
                                }
                            } else {
                                result.push_str(&op_regs[internal_idx]);
                            }
                        } else {
                            let _ = write!(result, "%z{}", num);
                        }
                    } else {
                        // Not a valid %z pattern, emit as-is
                        result.push('%');
                        result.push('z');
                    }
                    continue;
                }

                // %lo and %hi modifiers (pass through as assembler directives)
                if chars[i] == 'l' && i + 1 < chars.len() && chars[i + 1] == 'o' {
                    result.push_str("%lo");
                    i += 2;
                    continue;
                }
                if chars[i] == 'h' && i + 1 < chars.len() && chars[i + 1] == 'i' {
                    result.push_str("%hi");
                    i += 2;
                    continue;
                }

                if chars[i] == '[' {
                    // Named operand: %[name]
                    i += 1;
                    let name_start = i;
                    while i < chars.len() && chars[i] != ']' {
                        i += 1;
                    }
                    let name: String = chars[name_start..i].iter().collect();
                    if i < chars.len() {
                        i += 1;
                    }

                    let mut found = false;
                    for (idx, op_name) in op_names.iter().enumerate() {
                        if let Some(ref n) = op_name {
                            if n == &name {
                                result.push_str(&Self::format_operand(
                                    idx,
                                    op_regs,
                                    op_kinds,
                                    op_mem_offsets,
                                    op_mem_addrs,
                                    op_imm_values,
                                    op_imm_symbols,
                                    true,
                                ));
                                found = true;
                                break;
                            }
                        }
                    }
                    if !found {
                        result.push('%');
                        result.push('[');
                        result.push_str(&name);
                        result.push(']');
                    }
                } else if chars[i].is_ascii_digit() {
                    // Positional operand: %0, %1, etc.
                    let mut num = 0usize;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        num = num * 10 + (chars[i] as usize - '0' as usize);
                        i += 1;
                    }
                    let internal_idx = if num < gcc_to_internal.len() {
                        gcc_to_internal[num]
                    } else {
                        num
                    };
                    if internal_idx < op_regs.len() {
                        result.push_str(&Self::format_operand(
                            internal_idx,
                            op_regs,
                            op_kinds,
                            op_mem_offsets,
                            op_mem_addrs,
                            op_imm_values,
                            op_imm_symbols,
                            true,
                        ));
                    } else {
                        let _ = write!(result, "%{}", num);
                    }
                } else {
                    // Not recognized, emit as-is (e.g., %pcrel_lo, %pcrel_hi, etc.)
                    result.push('%');
                    result.push(chars[i]);
                    i += 1;
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }
}
