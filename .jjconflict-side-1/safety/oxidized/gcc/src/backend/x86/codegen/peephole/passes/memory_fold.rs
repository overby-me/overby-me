//! Memory operand folding pass.
//!
//! Folds a stack load followed by an ALU instruction that uses the loaded register
//! as a source operand into a single instruction with a memory source operand.
//!
//! Pattern:
//!   movq  -N(%rbp), %rcx       ; LoadRbp { reg: 1(rcx), offset: -N, size: Q }
//!   addq  %rcx, %rax           ; Other: rax = rax + rcx
//!
//! Transformed to:
//!   addq  -N(%rbp), %rax       ; rax = rax + mem[rbp-N]
//!
//! Supported ALU ops: add, sub, and, or, xor, cmp, test (with q/l suffixes).
//! The loaded register must be used as the first (source) operand in AT&T syntax.
//! We only fold when the loaded register is one of the scratch registers (rax=0,
//! rcx=1, rdx=2) to avoid breaking live register values.

use super::super::types::*;

/// Format a stack slot as an assembly memory operand string.
fn format_rbp_offset(offset: i32) -> String {
    format!("{}(%rbp)", offset)
}

/// Try to parse an ALU instruction of the form "OPsuffix %src, %dst"
/// where OP is add/sub/and/or/xor/cmp/test.
/// Returns (op_name_with_suffix, dst_reg_str, src_family, dst_family).
fn parse_alu_reg_reg(trimmed: &str) -> Option<(&str, &str, RegId, RegId)> {
    let b = trimmed.as_bytes();
    if b.len() < 6 {
        return None;
    }

    let op_len = if b.starts_with(b"add")
        || b.starts_with(b"sub")
        || b.starts_with(b"and")
        || b.starts_with(b"xor")
        || b.starts_with(b"cmp")
    {
        3
    } else if b.starts_with(b"test") {
        4
    } else if b.starts_with(b"or")
        && b.len() > 2
        && (b[2] == b'q' || b[2] == b'l' || b[2] == b'w' || b[2] == b'b')
    {
        2
    } else {
        return None;
    };

    let suffix = b[op_len];
    if suffix != b'q' && suffix != b'l' && suffix != b'w' && suffix != b'b' {
        return None;
    }
    let op_with_suffix = &trimmed[..op_len + 1];

    let rest = trimmed[op_len + 1..].trim();
    let (src_str, dst_str) = rest.split_once(',')?;
    let src_str = src_str.trim();
    let dst_str = dst_str.trim();

    if !src_str.starts_with('%') || !dst_str.starts_with('%') {
        return None;
    }

    let src_fam = register_family_fast(src_str);
    let dst_fam = register_family_fast(dst_str);
    if src_fam == REG_NONE || dst_fam == REG_NONE {
        return None;
    }

    Some((op_with_suffix, dst_str, src_fam, dst_fam))
}

/// Fold stack loads into subsequent ALU instructions as memory operands.
///
/// Safety: We only fold when the loaded register (the one being eliminated) is
/// a scratch register (rax=0, rcx=1, rdx=2) because the codegen guarantees
/// these are temporary and overwritten before the next use. We also verify
/// the loaded register is not the *destination* of the ALU instruction to avoid
/// creating a memory-destination instruction (which would write to the stack slot).
pub(super) fn fold_memory_operands(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    let mut changed = false;
    let mut i = 0;

    while i + 1 < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        if let LineKind::LoadRbp {
            reg: load_reg,
            offset,
            size: load_size,
        } = infos[i].kind
        {
            // Only fold loads into scratch registers (rax=0, rcx=1, rdx=2)
            if load_reg > 2 {
                i += 1;
                continue;
            }

            // Only fold Q and L loads (64-bit and 32-bit). SLQ (sign-extending)
            // loads have different semantics.
            if load_size != MoveSize::Q && load_size != MoveSize::L {
                i += 1;
                continue;
            }

            // Find the next non-NOP, non-empty instruction
            let mut j = i + 1;
            while j < len && (infos[j].is_nop() || infos[j].kind == LineKind::Empty) {
                j += 1;
            }
            if j >= len {
                i += 1;
                continue;
            }

            let is_foldable_target =
                matches!(infos[j].kind, LineKind::Other { .. } | LineKind::Cmp);
            if is_foldable_target {
                let trimmed_j = infos[j].trimmed(store.get(j));
                if let Some((op_suffix, dst_str, src_fam, dst_fam)) = parse_alu_reg_reg(trimmed_j) {
                    if src_fam == load_reg && dst_fam != load_reg {
                        // Check for intervening store to the same offset
                        let mut intervening_store = false;
                        for k in (i + 1)..j {
                            if let LineKind::StoreRbp { offset: so, .. } = infos[k].kind {
                                if so == offset {
                                    intervening_store = true;
                                    break;
                                }
                            }
                        }
                        if intervening_store {
                            i += 1;
                            continue;
                        }

                        let mem_op = format_rbp_offset(offset);
                        let new_inst = format!("    {} {}, {}", op_suffix, mem_op, dst_str);

                        mark_nop(&mut infos[i]);
                        replace_line(store, &mut infos[j], j, new_inst);
                        changed = true;
                        i = j + 1;
                        continue;
                    }
                }
            }
        }

        i += 1;
    }

    changed
}
