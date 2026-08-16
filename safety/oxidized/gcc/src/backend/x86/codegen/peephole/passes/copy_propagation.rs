//! Register copy propagation pass.
//!
//! Propagates register-to-register copies across entire basic blocks to
//! eliminate intermediate moves. The accumulator-based codegen routes many
//! operations through %rax as an intermediary, producing chains like:
//!
//!   movq %rax, %rcx             # copy rax -> rcx
//!   movq %rcx, %rdx             # copy rcx -> rdx (really rax -> rdx)
//!   addq %rcx, %r8              # uses rcx (really rax)
//!
//! After propagation, this becomes:
//!
//!   movq %rax, %rcx             # potentially dead
//!   movq %rax, %rdx             # uses rax directly
//!   addq %rax, %r8              # uses rax directly
//!
//! The dead movq instructions are cleaned up by subsequent passes.

use super::super::types::*;
use super::helpers::*;

/// Try to replace uses of `dst_id` with `src_id` in instruction at index `j`.
/// Returns true if a replacement was made.
fn try_propagate_into(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    j: usize,
    src_id: RegId,
    dst_id: RegId,
) -> bool {
    // Never propagate into opaque lines (inline asm, multi-instruction sequences)
    if infos[j].has_indirect_mem {
        return false;
    }

    let trimmed = infos[j].trimmed(store.get(j));

    // The instruction must reference the destination register
    if infos[j].reg_refs & (1u16 << dst_id) == 0 {
        return false;
    }

    // Skip instructions with implicit register usage
    if has_implicit_reg_usage(trimmed) {
        return false;
    }

    // Skip shift/rotate when propagating into %rcx (they need %cl)
    if dst_id == 1 && is_shift_or_rotate(trimmed) {
        return false;
    }

    let next_dest = get_dest_reg(&infos[j]);

    let src_name = REG_NAMES[0][src_id as usize];
    let dst_name = REG_NAMES[0][dst_id as usize];

    // Case 1: next instruction writes to src_id
    if next_dest == src_id {
        // Source register is being written by this instruction.
        // Only safe if dst appears ONLY in a memory base position like (%dst).
        let dst_paren = format!("({})", dst_name);
        if !trimmed.contains(dst_paren.as_str()) {
            return false;
        }
        let src_paren = format!("({})", src_name);
        let src_direct_count = trimmed.matches(src_name).count();
        let src_paren_count = trimmed.matches(src_paren.as_str()).count();
        let is_dest_only = if let Some((_before, after_comma)) = trimmed.rsplit_once(',') {
            after_comma.trim() == src_name
        } else {
            false
        };
        let src_as_source = src_direct_count - src_paren_count - if is_dest_only { 1 } else { 0 };
        if src_as_source > 0 {
            return false;
        }
        let new_text = format!("    {}", replace_reg_family(trimmed, dst_id, src_id));
        replace_line(store, &mut infos[j], j, new_text);
        return true;
    }

    // Case 2: dst is not the destination - replace all occurrences
    if next_dest != dst_id {
        let new_content = replace_reg_family(trimmed, dst_id, src_id);
        if new_content != trimmed {
            let new_text = format!("    {}", new_content);
            replace_line(store, &mut infos[j], j, new_text);
            return true;
        }
        return false;
    }

    // Case 3: dst is the destination AND a source (e.g., addq %rcx, %rcx)
    // For single-operand read-modify-write instructions (negq, notq, incq, decq),
    // the single operand is both source and destination. Don't replace.
    if !trimmed.contains(',') {
        return false;
    }
    // Only replace the source-position occurrences.
    let new_content = replace_reg_family_in_source(trimmed, dst_id, src_id);
    if new_content != trimmed {
        let new_text = format!("    {}", new_content);
        replace_line(store, &mut infos[j], j, new_text);
        return true;
    }
    false
}

pub(super) fn propagate_register_copies(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();

    // copy_src[dst] = src means "dst currently holds the same value as src"
    let mut copy_src: [RegId; 16] = [REG_NONE; 16];

    let mut i = 0;
    while i < len {
        // At basic block boundaries, clear all copies
        if infos[i].is_barrier() {
            copy_src = [REG_NONE; 16];
            i += 1;
            continue;
        }

        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Lines with indirect memory access (including semicolon-separated
        // inline asm like "pushf ; pop %rcx") are opaque: don't propagate
        // into them and invalidate all copies, since we can't determine which
        // registers the multi-instruction sequence reads or writes.
        if infos[i].has_indirect_mem {
            copy_src = [REG_NONE; 16];
            i += 1;
            continue;
        }

        // Check if this is a reg-to-reg movq that defines a new copy
        let trimmed = infos[i].trimmed(store.get(i));
        if let Some((src_id, dst_id)) = parse_reg_to_reg_movq(&infos[i], trimmed) {
            // Resolve transitive copies
            let ultimate_src = if copy_src[src_id as usize] != REG_NONE {
                copy_src[src_id as usize]
            } else {
                src_id
            };

            if ultimate_src != src_id && ultimate_src != dst_id {
                let new_src_name = REG_NAMES[0][ultimate_src as usize];
                let dst_name = REG_NAMES[0][dst_id as usize];
                let new_text = format!("    movq {}, {}", new_src_name, dst_name);
                replace_line(store, &mut infos[i], i, new_text);
                changed = true;
            } else if ultimate_src == dst_id {
                mark_nop(&mut infos[i]);
                changed = true;
                i += 1;
                continue;
            }

            // Before recording: invalidate any copies that have dst as their source.
            for k in 0..16u8 {
                if copy_src[k as usize] == dst_id {
                    copy_src[k as usize] = REG_NONE;
                }
            }

            // Record the copy
            copy_src[dst_id as usize] = ultimate_src;

            i += 1;
            continue;
        }

        // Not a copy instruction. Try to propagate active copies into this instruction.
        let dest_reg = get_dest_reg(&infos[i]);

        let mut did_propagate = false;
        for reg in 0..16u8 {
            let src = copy_src[reg as usize];
            if src == REG_NONE {
                continue;
            }
            if infos[i].reg_refs & (1u16 << reg) == 0 {
                continue;
            }
            let cur_trimmed = infos[i].trimmed(store.get(i));
            if has_implicit_reg_usage(cur_trimmed) {
                break;
            }

            if try_propagate_into(store, infos, i, src, reg) {
                changed = true;
                did_propagate = true;
                break;
            }
        }

        // If we propagated, don't increment i - re-process.
        // But we still need to do invalidation below.
        let _ = did_propagate;

        // Invalidate copies affected by this instruction's writes.
        if dest_reg != REG_NONE && dest_reg <= REG_GP_MAX {
            copy_src[dest_reg as usize] = REG_NONE;
            for k in 0..16u8 {
                if copy_src[k as usize] == dest_reg {
                    copy_src[k as usize] = REG_NONE;
                }
            }
        }

        // Instructions with implicit register usage conservatively invalidate all.
        {
            let cur_trimmed = infos[i].trimmed(store.get(i));
            if has_implicit_reg_usage(cur_trimmed) {
                copy_src = [REG_NONE; 16];
            }
        }

        i += 1;
    }
    changed
}
