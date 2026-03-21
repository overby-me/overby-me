//! Shared utilities for x86 and i686 backends.
//!
//! Both the x86-64 and i686 backends use the same x86 ISA family and share
//! register naming conventions, condition code mappings, and inline assembly
//! template parsing logic. This module provides shared free functions to
//! avoid duplicating these across both backends.

use crate::common::types::IrType;
use crate::ir::reexports::BlockId;
use std::borrow::Cow;
use std::fmt::Write;

/// Resolve GCC inline asm dialect alternatives in a template string.
///
/// GCC inline asm supports `{alt0|alt1}` syntax where `alt0` is the AT&T
/// dialect version and `alt1` is the Intel dialect version. Since we always
/// emit AT&T syntax, we select the first alternative and strip the braces.
///
/// Examples:
///   `"rep; bsr{q %1, %0| %0, %1}"` -> `"rep; bsrq %1, %0"`
///   `"bt{l} %[Bit],%[Base]"`       -> `"btl %[Bit],%[Base]"`
///   `"{lock }cmpxchg %1, %2"`      -> `"lock cmpxchg %1, %2"`
///
/// The `%{` escape (literal brace) is not used in practice, so we don't
/// handle it specially here.
fn resolve_dialect_alternatives(line: &str) -> Cow<'_, str> {
    if !line.contains('{') {
        return Cow::Borrowed(line);
    }
    let mut result = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            // Collect the first alternative (AT&T) until '|' or '}'
            i += 1; // skip '{'
            while i < chars.len() && chars[i] != '|' && chars[i] != '}' {
                result.push(chars[i]);
                i += 1;
            }
            // Skip the second alternative (Intel) and closing '}'
            if i < chars.len() && chars[i] == '|' {
                i += 1; // skip '|'
                while i < chars.len() && chars[i] != '}' {
                    i += 1;
                }
            }
            if i < chars.len() && chars[i] == '}' {
                i += 1; // skip '}'
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    Cow::Owned(result)
}

/// Map GCC inline asm condition code suffix to x86 SETcc/Jcc suffix.
///
/// GCC's `=@cc<cond>` output constraint maps directly to x86 condition codes.
/// This mapping is identical for both x86-64 and i686.
pub(crate) fn gcc_cc_to_x86(cond: &str) -> &'static str {
    match cond {
        "e" | "z" => "e",    // equal / zero
        "ne" | "nz" => "ne", // not equal / not zero
        "s" => "s",          // sign (negative)
        "ns" => "ns",        // not sign (non-negative)
        "o" => "o",          // overflow
        "no" => "no",        // no overflow
        "c" => "c",          // carry
        "nc" => "nc",        // no carry
        "a" | "nbe" => "a",  // above (unsigned >)
        "ae" | "nb" => "ae", // above or equal (unsigned >=)
        "b" | "nae" => "b",  // below (unsigned <)
        "be" | "na" => "be", // below or equal (unsigned <=)
        "g" | "nle" => "g",  // greater (signed >)
        "ge" | "nl" => "ge", // greater or equal (signed >=)
        "l" | "nge" => "l",  // less (signed <)
        "le" | "ng" => "le", // less or equal (signed <=)
        "p" | "pe" => "p",   // parity even
        "np" | "po" => "np", // parity odd / no parity
        // TODO: emit a warning/diagnostic for unrecognized condition code suffixes
        _ => "e", // fallback to equal
    }
}

/// Convert any x86 register name to its 32-bit variant.
///
/// Accepts both 64-bit (x86-64) and 32/16/8-bit (i686) register names.
/// For `r8`-`r15` (x86-64 only), appends `d` suffix (e.g., `r8` -> `r8d`).
pub(crate) fn reg_to_32<'a>(reg: &'a str) -> Cow<'a, str> {
    match reg {
        "rax" | "eax" | "ax" | "al" | "ah" => Cow::Borrowed("eax"),
        "rbx" | "ebx" | "bx" | "bl" | "bh" => Cow::Borrowed("ebx"),
        "rcx" | "ecx" | "cx" | "cl" | "ch" => Cow::Borrowed("ecx"),
        "rdx" | "edx" | "dx" | "dl" | "dh" => Cow::Borrowed("edx"),
        "rsi" | "esi" | "si" => Cow::Borrowed("esi"),
        "rdi" | "edi" | "di" => Cow::Borrowed("edi"),
        "rbp" | "ebp" | "bp" => Cow::Borrowed("ebp"),
        "rsp" | "esp" | "sp" => Cow::Borrowed("esp"),
        _ if reg.starts_with('r') => Cow::Owned(format!("{}d", reg)), // r8 -> r8d
        _ => Cow::Borrowed(reg),
    }
}

/// Convert any x86 register name to its 16-bit variant.
///
/// Accepts both 64-bit and 32/16/8-bit register names.
/// For `r8`-`r15` (x86-64 only), appends `w` suffix (e.g., `r8` -> `r8w`).
pub(crate) fn reg_to_16<'a>(reg: &'a str) -> Cow<'a, str> {
    match reg {
        "rax" | "eax" | "ax" | "al" | "ah" => Cow::Borrowed("ax"),
        "rbx" | "ebx" | "bx" | "bl" | "bh" => Cow::Borrowed("bx"),
        "rcx" | "ecx" | "cx" | "cl" | "ch" => Cow::Borrowed("cx"),
        "rdx" | "edx" | "dx" | "dl" | "dh" => Cow::Borrowed("dx"),
        "rsi" | "esi" | "si" => Cow::Borrowed("si"),
        "rdi" | "edi" | "di" => Cow::Borrowed("di"),
        "rbp" | "ebp" | "bp" => Cow::Borrowed("bp"),
        "rsp" | "esp" | "sp" => Cow::Borrowed("sp"),
        _ if reg.starts_with('r') => Cow::Owned(format!("{}w", reg)), // r8 -> r8w
        _ => Cow::Borrowed(reg),
    }
}

/// Convert any x86 register name to its 8-bit low variant.
///
/// Accepts both 64-bit and 32/16/8-bit register names.
/// For `r8`-`r15` (x86-64 only), appends `b` suffix (e.g., `r8` -> `r8b`).
///
/// Note: `rsi`/`rdi` map to `sil`/`dil` (which exist only in 64-bit mode).
/// The 32-bit forms `esi`/`edi` are NOT mapped to `sil`/`dil` because those
/// 8-bit registers don't exist on i686. When i686 passes `"esi"`, it falls
/// through to the default arm and returns the register unchanged (correct
/// behavior: i686 `esi`/`edi` lack 8-bit low forms).
pub(crate) fn reg_to_8l<'a>(reg: &'a str) -> Cow<'a, str> {
    match reg {
        "rax" | "eax" | "ax" | "al" => Cow::Borrowed("al"),
        "rbx" | "ebx" | "bx" | "bl" => Cow::Borrowed("bl"),
        "rcx" | "ecx" | "cx" | "cl" => Cow::Borrowed("cl"),
        "rdx" | "edx" | "dx" | "dl" => Cow::Borrowed("dl"),
        "rsi" => Cow::Borrowed("sil"),
        "rdi" => Cow::Borrowed("dil"),
        _ if reg.starts_with('r') => Cow::Owned(format!("{}b", reg)), // r8 -> r8b
        _ => Cow::Borrowed(reg),
    }
}

/// Convert any x86 register name to its 8-bit high variant.
///
/// Only the four legacy registers (a/b/c/d) have high-byte forms.
/// Falls back to the low-byte variant for all other registers.
pub(crate) fn reg_to_8h<'a>(reg: &'a str) -> Cow<'a, str> {
    match reg {
        "rax" | "eax" | "ax" | "ah" => Cow::Borrowed("ah"),
        "rbx" | "ebx" | "bx" | "bh" => Cow::Borrowed("bh"),
        "rcx" | "ecx" | "cx" | "ch" => Cow::Borrowed("ch"),
        "rdx" | "edx" | "dx" | "dh" => Cow::Borrowed("dh"),
        _ => reg_to_8l(reg), // fallback to low byte
    }
}

/// Substitute `%0`, `%1`, `%[name]`, `%k0`, `%b1`, `%w2`, `%q3`, `%h4`,
/// `%c0`, `%P0`, `%a0`, `%n0`, `%l[name]` etc. in an x86 asm template line.
///
/// This is the template parsing logic shared between x86-64 and i686. The
/// actual operand emission is delegated to the `emit_operand` callback, which
/// handles architecture-specific differences (e.g., RIP-relative addressing on
/// x86-64 vs. absolute addressing on i686, and different default register widths).
pub(crate) fn substitute_x86_asm_operands(
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
    emit_operand: fn(
        result: &mut String,
        idx: usize,
        modifier: Option<char>,
        op_regs: &[String],
        op_is_memory: &[bool],
        op_mem_addrs: &[String],
        op_types: &[IrType],
        op_imm_values: &[Option<i64>],
        op_imm_symbols: &[Option<String>],
    ),
) -> String {
    // Pre-process GCC dialect alternatives: {att_syntax|intel_syntax}
    // We always target AT&T syntax, so select the first alternative.
    let line = resolve_dialect_alternatives(line);
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
            // Check for x86 size/format modifiers:
            //   k (32), w (16), b (8-low), h (8-high), q (64), l (32-alt),
            //   c (raw constant), P (raw symbol), a (address)
            // But 'l' followed by '[' may be a goto label reference %l[name]
            let mut modifier = None;
            if chars[i] == 'l'
                && i + 1 < chars.len()
                && chars[i + 1] == '['
                && !goto_labels.is_empty()
            {
                // This could be %l[name] goto label reference
                // Parse the name and check if it's a goto label first
                let saved_i = i;
                i += 1; // skip 'l'
                i += 1; // skip '['
                let name_start = i;
                while i < chars.len() && chars[i] != ']' {
                    i += 1;
                }
                let name: String = chars[name_start..i].iter().collect();
                if i < chars.len() {
                    i += 1;
                } // skip ']'

                if let Some((_, block_id)) = goto_labels.iter().find(|(n, _)| n == &name) {
                    // It's a goto label — emit the assembly label
                    result.push_str(&block_id.to_string());
                    continue;
                }
                // Not a goto label — backtrack and treat as %l (32-bit modifier) + [name]
                i = saved_i;
                modifier = Some('l');
                i += 1; // skip 'l', will parse [name] below
            } else if chars[i] == 'l'
                && i + 1 < chars.len()
                && chars[i + 1].is_ascii_digit()
                && !goto_labels.is_empty()
            {
                // %l<N> could be a goto label positional reference
                let saved_i = i;
                i += 1; // skip 'l'
                let mut num = 0usize;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    num = num * 10 + (chars[i] as usize - '0' as usize);
                    i += 1;
                }
                // GCC numbers goto labels after all operands.
                // %l<N> where N >= num_operands refers to label (N - num_operands).
                let label_idx = num.wrapping_sub(op_regs.len());
                if label_idx < goto_labels.len() {
                    result.push_str(&goto_labels[label_idx].1.to_string());
                    continue;
                }
                // Not a valid label index — backtrack
                i = saved_i;
                modifier = Some('l');
                i += 1;
            } else if chars[i] == 'P' {
                // %P: raw symbol/value modifier (uppercase — always a modifier, never a register)
                if i + 1 < chars.len() && (chars[i + 1].is_ascii_digit() || chars[i + 1] == '[') {
                    modifier = Some('P');
                    i += 1;
                }
            } else if matches!(
                chars[i],
                'k' | 'w' | 'b' | 'h' | 'q' | 'l' | 'c' | 'a' | 'n'
            ) && i + 1 < chars.len()
                && (chars[i + 1].is_ascii_digit() || chars[i + 1] == '[')
            {
                modifier = Some(chars[i]);
                i += 1;
            }

            if chars[i] == '[' {
                // Named operand: %[name] or %k[name]
                i += 1;
                let name_start = i;
                while i < chars.len() && chars[i] != ']' {
                    i += 1;
                }
                let name: String = chars[name_start..i].iter().collect();
                if i < chars.len() {
                    i += 1;
                } // skip ]

                let mut found = false;
                for (idx, op_name) in op_names.iter().enumerate() {
                    if let Some(ref n) = op_name {
                        if n == &name {
                            emit_operand(
                                &mut result,
                                idx,
                                modifier,
                                op_regs,
                                op_is_memory,
                                op_mem_addrs,
                                op_types,
                                op_imm_values,
                                op_imm_symbols,
                            );
                            found = true;
                            break;
                        }
                    }
                }
                if !found {
                    result.push('%');
                    if let Some(m) = modifier {
                        result.push(m);
                    }
                    result.push('[');
                    result.push_str(&name);
                    result.push(']');
                }
            } else if chars[i].is_ascii_digit() {
                // Positional operand: %0, %1, %k2, etc.
                // GCC operand numbers skip synthetic "+" inputs, so map through gcc_to_internal
                let mut num = 0usize;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    num = num * 10 + (chars[i] as usize - '0' as usize);
                    i += 1;
                }
                let internal_idx = if num < gcc_to_internal.len() {
                    gcc_to_internal[num]
                } else {
                    num // fallback: direct mapping
                };
                if internal_idx < op_regs.len() {
                    emit_operand(
                        &mut result,
                        internal_idx,
                        modifier,
                        op_regs,
                        op_is_memory,
                        op_mem_addrs,
                        op_types,
                        op_imm_values,
                        op_imm_symbols,
                    );
                } else {
                    result.push('%');
                    if let Some(m) = modifier {
                        result.push(m);
                    }
                    result.push_str(&num.to_string());
                }
            } else {
                // Not a recognized pattern, emit as-is (e.g. %rax, %eax, etc.)
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

/// Emit a single operand with modifier — shared logic for `%n`, `%c`/`%P`,
/// memory operands, symbol immediates, and normal immediates.
///
/// Returns `true` if the operand was fully handled by the shared code.
/// Returns `false` if the caller needs to handle the final register
/// formatting case (which differs between x86-64 and i686 due to different
/// default register widths and addressing modes for `%a` + symbol).
pub(crate) fn emit_operand_common(
    result: &mut String,
    idx: usize,
    modifier: Option<char>,
    op_regs: &[String],
    op_is_memory: &[bool],
    op_mem_addrs: &[String],
    op_imm_values: &[Option<i64>],
    op_imm_symbols: &[Option<String>],
) -> bool {
    let is_raw = matches!(modifier, Some('c') | Some('P'));
    let is_neg = modifier == Some('n');
    let has_symbol = op_imm_symbols.get(idx).and_then(|s| s.as_ref());
    let has_imm = op_imm_values.get(idx).and_then(|v| v.as_ref());

    if is_neg {
        // %n: emit the negated constant value (no $ prefix)
        if let Some(&imm) = has_imm {
            result.push_str(&imm.wrapping_neg().to_string());
        } else {
            // Fallback: emit as-is if not an immediate (shouldn't happen with correct usage)
            result.push_str(&op_regs[idx]);
        }
        return true;
    }

    if is_raw {
        // %c / %P: emit raw value without $ or % prefix
        if let Some(sym) = has_symbol {
            result.push_str(sym);
        } else if let Some(imm) = has_imm {
            result.push_str(&imm.to_string());
        } else if op_is_memory[idx] {
            result.push_str(&op_mem_addrs[idx]);
        } else {
            // Register: emit without % prefix
            result.push_str(&op_regs[idx]);
        }
        return true;
    }

    if op_is_memory[idx] {
        // Memory operand — emit the pre-computed mem_addr string
        result.push_str(&op_mem_addrs[idx]);
        return true;
    }

    if modifier != Some('a') {
        // Not %a modifier — handle symbol/immediate cases
        if let Some(sym) = has_symbol {
            let _ = write!(result, "${}", sym);
            return true;
        }
        if let Some(imm) = has_imm {
            let _ = write!(result, "${}", imm);
            return true;
        }
    }

    // Not handled: %a modifier, or register operand needing arch-specific formatting
    false
}
