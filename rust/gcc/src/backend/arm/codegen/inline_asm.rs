//! AArch64 inline assembly template substitution and register formatting.
//!
//! This module handles operand substitution in inline assembly templates
//! (e.g., `%0`, `%[name]`, `%w0`, `%x0`) and register formatting with
//! w/x/s/d/q modifiers for ARM targets. It also contains helpers for
//! atomic exclusive access instructions (ldxr/stxr) and atomic RMW operations.

use super::emit::ArmCodegen;
use crate::backend::state::CodegenState;
use crate::common::types::IrType;
use crate::ir::reexports::{AtomicOrdering, AtomicRmwOp};
use std::fmt::Write;

impl ArmCodegen {
    pub(super) fn substitute_asm_operands_static(
        line: &str,
        op_regs: &[String],
        op_names: &[Option<String>],
        gcc_to_internal: &[usize],
        op_imm_values: &[Option<i64>],
        op_imm_symbols: &[Option<String>],
    ) -> String {
        let mut result = String::new();
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '%' && i + 1 < chars.len() {
                i += 1;
                // Check for %% (literal %)
                if chars[i] == '%' {
                    result.push('%');
                    i += 1;
                    continue;
                }
                // Check for modifier: w, x, h, b, s, d, q, c, a
                // 'c' = raw constant (no # prefix), used in ARM inline asm
                // 'a' = memory address reference [reg], used for prfm/prefetch
                let mut modifier = None;
                if chars[i] == 'w'
                    || chars[i] == 'x'
                    || chars[i] == 'h'
                    || chars[i] == 'b'
                    || chars[i] == 's'
                    || chars[i] == 'd'
                    || chars[i] == 'q'
                    || chars[i] == 'c'
                    || chars[i] == 'a'
                {
                    // Check if next char is digit or [, meaning this is a modifier
                    if i + 1 < chars.len() && (chars[i + 1].is_ascii_digit() || chars[i + 1] == '[')
                    {
                        modifier = Some(chars[i]);
                        i += 1;
                    }
                }

                if chars[i] == '[' {
                    // Named operand: %[name] or %w[name]
                    i += 1;
                    let name_start = i;
                    while i < chars.len() && chars[i] != ']' {
                        i += 1;
                    }
                    let name: String = chars[name_start..i].iter().collect();
                    if i < chars.len() {
                        i += 1;
                    } // skip ]

                    // Look up by name in operands
                    let mut found = false;
                    for (idx, op_name) in op_names.iter().enumerate() {
                        if let Some(ref n) = op_name {
                            if n == &name {
                                result.push_str(&Self::format_operand_static(
                                    idx,
                                    modifier,
                                    op_regs,
                                    op_imm_values,
                                    op_imm_symbols,
                                ));
                                found = true;
                                break;
                            }
                        }
                    }
                    if !found {
                        // Fallback: emit raw
                        result.push('%');
                        if let Some(m) = modifier {
                            result.push(m);
                        }
                        result.push('[');
                        result.push_str(&name);
                        result.push(']');
                    }
                } else if chars[i].is_ascii_digit() {
                    // Positional operand: %0, %1, %w2, etc.
                    // The number is a GCC operand number (outputs numbered first,
                    // then real inputs, skipping synthetic "+" inputs).
                    // Map through gcc_to_internal to get the internal operand index.
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
                        result.push_str(&Self::format_operand_static(
                            internal_idx,
                            modifier,
                            op_regs,
                            op_imm_values,
                            op_imm_symbols,
                        ));
                    } else {
                        let _ = write!(result, "x{}", num);
                    }
                } else {
                    // Not a recognized pattern, emit as-is
                    result.push('%');
                    if let Some(m) = modifier {
                        result.push(m);
                    }
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

    /// Format an operand for ARM inline assembly substitution.
    /// For immediate operands (from "i"/"n" constraints), emit the raw value.
    /// GCC on AArch64 emits raw integers for "i" constraints (no '#' prefix),
    /// both in instruction and data directive contexts.
    /// For register operands, apply the modifier and emit the register name.
    fn format_operand_static(
        idx: usize,
        modifier: Option<char>,
        op_regs: &[String],
        op_imm_values: &[Option<i64>],
        op_imm_symbols: &[Option<String>],
    ) -> String {
        // Check for immediate symbol first (e.g., function/variable name)
        if let Some(Some(ref sym)) = op_imm_symbols.get(idx) {
            return sym.clone();
        }
        // Check for immediate value - emit raw (GCC AArch64 behavior)
        if let Some(Some(imm)) = op_imm_values.get(idx) {
            return imm.to_string();
        }
        // Regular register operand
        Self::format_reg_static(&op_regs[idx], modifier)
    }

    pub(super) fn format_reg_static(reg: &str, modifier: Option<char>) -> String {
        // Normalize r-prefix registers (GCC AArch64 aliases: r0-r30 = x0-x30)
        // before processing modifiers so %w0/%x0 work correctly on r-prefixed operands.
        let normalized;
        let reg = if reg.starts_with('r') {
            if let Some(suffix) = reg.strip_prefix('r') {
                if suffix.chars().all(|c| c.is_ascii_digit()) {
                    if let Ok(n) = suffix.parse::<u32>() {
                        if n <= 30 {
                            normalized = format!("x{}", n);
                            &normalized
                        } else {
                            reg
                        }
                    } else {
                        reg
                    }
                } else {
                    reg
                }
            } else {
                reg
            }
        } else {
            reg
        };
        // Extract the register number from any register form (x, w, d, s, q, v)
        let reg_num = || -> Option<&str> {
            if reg.starts_with('x')
                || reg.starts_with('w')
                || reg.starts_with('d')
                || reg.starts_with('s')
                || reg.starts_with('q')
                || reg.starts_with('v')
            {
                Some(&reg[1..])
            } else {
                None
            }
        };
        // Check if this is a FP/SIMD register (d, s, q, v prefix)
        let is_fp_reg = reg.starts_with('d')
            || reg.starts_with('s')
            || reg.starts_with('q')
            || reg.starts_with('v');
        match modifier {
            Some('w') => {
                // Convert to w-register (32-bit GP)
                if let Some(num) = reg_num() {
                    if reg.starts_with('x') || reg.starts_with('w') {
                        return format!("w{}", num);
                    }
                }
                reg.to_string()
            }
            Some('x') => {
                // Convert to x-register (64-bit GP)
                if let Some(num) = reg_num() {
                    if reg.starts_with('w') || reg.starts_with('x') {
                        return format!("x{}", num);
                    }
                }
                reg.to_string()
            }
            Some('d') => {
                // Convert to d-register (64-bit FP/double)
                if let Some(num) = reg_num() {
                    if is_fp_reg {
                        return format!("d{}", num);
                    }
                }
                reg.to_string()
            }
            Some('s') => {
                // Convert to s-register (32-bit FP/float)
                if let Some(num) = reg_num() {
                    if is_fp_reg {
                        return format!("s{}", num);
                    }
                }
                reg.to_string()
            }
            Some('q') => {
                // Convert to q-register (128-bit SIMD)
                if let Some(num) = reg_num() {
                    if is_fp_reg {
                        return format!("q{}", num);
                    }
                }
                reg.to_string()
            }
            Some('h') => {
                // Convert to h-register (16-bit FP/half)
                if let Some(num) = reg_num() {
                    if is_fp_reg {
                        return format!("h{}", num);
                    }
                }
                reg.to_string()
            }
            Some('b') => {
                // Convert to b-register (8-bit)
                if let Some(num) = reg_num() {
                    if is_fp_reg {
                        return format!("b{}", num);
                    }
                }
                reg.to_string()
            }
            Some('a') => {
                // Memory address reference: wrap register in square brackets [reg]
                // Used by prfm/prefetch instructions: "prfm pldl1keep, %a0"
                // If the operand is already a memory reference (e.g., "[sp, #N]"),
                // return it as-is. Otherwise, wrap the register in brackets.
                if reg.starts_with('[') {
                    reg.to_string()
                } else {
                    format!("[{}]", reg)
                }
            }
            None => {
                // No modifier: for FP/SIMD registers, emit as vN (GCC behavior).
                // This ensures `%0.16b` produces `v16.16b` not `d16.16b`.
                if is_fp_reg {
                    if let Some(num) = reg_num() {
                        return format!("v{}", num);
                    }
                }
                reg.to_string()
            }
            _ => reg.to_string(),
        }
    }

    /// Convert a FP/SIMD register name to its s-register counterpart (same register number).
    /// e.g., "d16" -> "s16", "v16" -> "s16"
    pub(super) fn fp_to_s_reg(reg: &str) -> String {
        if let Some(rest) = reg
            .strip_prefix('d')
            .or_else(|| reg.strip_prefix('v'))
            .or_else(|| reg.strip_prefix('s'))
            .or_else(|| reg.strip_prefix('q'))
        {
            format!("s{rest}")
        } else {
            reg.to_string()
        }
    }

    /// Convert a FP/SIMD register name to its d-register counterpart (same register number).
    /// e.g., "v16" -> "d16", "s16" -> "d16"
    pub(super) fn fp_to_d_reg(reg: &str) -> String {
        if let Some(rest) = reg
            .strip_prefix('v')
            .or_else(|| reg.strip_prefix('d'))
            .or_else(|| reg.strip_prefix('s'))
            .or_else(|| reg.strip_prefix('q'))
        {
            format!("d{rest}")
        } else {
            reg.to_string()
        }
    }

    /// Convert a FP/SIMD register name to its q-register counterpart (same register number).
    /// e.g., "v16" -> "q16", "d16" -> "q16"
    pub(super) fn fp_to_q_reg(reg: &str) -> String {
        if let Some(rest) = reg
            .strip_prefix('v')
            .or_else(|| reg.strip_prefix('d'))
            .or_else(|| reg.strip_prefix('s'))
            .or_else(|| reg.strip_prefix('q'))
        {
            format!("q{rest}")
        } else {
            reg.to_string()
        }
    }

    /// Get the exclusive load/store instructions and register prefix for a type,
    /// with appropriate acquire/release semantics based on ordering.
    /// - Relaxed: ldxr/stxr (no ordering)
    /// - Acquire: ldaxr/stxr (acquire on load)
    /// - Release: ldxr/stlxr (release on store)
    /// - AcqRel/SeqCst: ldaxr/stlxr (acquire on load, release on store)
    pub(super) fn exclusive_instrs(
        ty: IrType,
        ordering: AtomicOrdering,
    ) -> (&'static str, &'static str, &'static str) {
        let need_acquire = matches!(
            ordering,
            AtomicOrdering::Acquire | AtomicOrdering::AcqRel | AtomicOrdering::SeqCst
        );
        let need_release = matches!(
            ordering,
            AtomicOrdering::Release | AtomicOrdering::AcqRel | AtomicOrdering::SeqCst
        );
        match ty {
            IrType::I8 | IrType::U8 => (
                if need_acquire { "ldaxrb" } else { "ldxrb" },
                if need_release { "stlxrb" } else { "stxrb" },
                "w",
            ),
            IrType::I16 | IrType::U16 => (
                if need_acquire { "ldaxrh" } else { "ldxrh" },
                if need_release { "stlxrh" } else { "stxrh" },
                "w",
            ),
            IrType::I32 | IrType::U32 => (
                if need_acquire { "ldaxr" } else { "ldxr" },
                if need_release { "stlxr" } else { "stxr" },
                "w",
            ),
            _ => (
                if need_acquire { "ldaxr" } else { "ldxr" },
                if need_release { "stlxr" } else { "stxr" },
                "x",
            ),
        }
    }

    /// Emit the arithmetic operation for an atomic RMW.
    pub(super) fn emit_atomic_op_arm(
        state: &mut CodegenState,
        op: AtomicRmwOp,
        dest_reg: &str,
        old_reg: &str,
        val_reg: &str,
    ) {
        match op {
            AtomicRmwOp::Add => state.emit_fmt(format_args!(
                "    add {}, {}, {}",
                dest_reg, old_reg, val_reg
            )),
            AtomicRmwOp::Sub => state.emit_fmt(format_args!(
                "    sub {}, {}, {}",
                dest_reg, old_reg, val_reg
            )),
            AtomicRmwOp::And => state.emit_fmt(format_args!(
                "    and {}, {}, {}",
                dest_reg, old_reg, val_reg
            )),
            AtomicRmwOp::Or => state.emit_fmt(format_args!(
                "    orr {}, {}, {}",
                dest_reg, old_reg, val_reg
            )),
            AtomicRmwOp::Xor => state.emit_fmt(format_args!(
                "    eor {}, {}, {}",
                dest_reg, old_reg, val_reg
            )),
            AtomicRmwOp::Nand => {
                state.emit_fmt(format_args!(
                    "    and {}, {}, {}",
                    dest_reg, old_reg, val_reg
                ));
                state.emit_fmt(format_args!("    mvn {}, {}", dest_reg, dest_reg));
            }
            AtomicRmwOp::Xchg | AtomicRmwOp::TestAndSet => {
                // Handled separately in emit_atomic_rmw
                state.emit_fmt(format_args!("    mov {}, {}", dest_reg, val_reg));
            }
        }
    }
}
