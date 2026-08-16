//! Local peephole pattern matching passes.
//!
//! Merges 7 simple local passes into a single linear scan (`combined_local_pass`)
//! to avoid redundant iteration over the lines array. Also includes
//! `fuse_movq_ext_truncation` which fuses movq + extension/truncation patterns.
//!
//! Merged passes in `combined_local_pass`:
//!   1. eliminate_redundant_movq_self: movq %reg, %reg (same src/dst)
//!   2. eliminate_reverse_move: movq %A,%B + movq %B,%A -> remove second
//!   3. eliminate_redundant_jumps: jmp to the immediately following label
//!   4. eliminate_cond_branch_inversion: jCC+jmp+label -> j!CC (inverted)
//!   5. eliminate_adjacent_store_load: store/load at same %rbp offset
//!   6. eliminate_redundant_zero_extend: redundant zero/sign extensions
//!   7. eliminate_redundant_xorl_zero: xorl %eax,%eax when %rax already zero

use super::super::types::*;
use super::helpers::is_valid_gp_reg;

pub(super) fn combined_local_pass(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();

    // Track whether %rax is known to be zero for redundant xorl elimination.
    // This is set to true after `xorl %eax, %eax` and stays true across
    // StoreRbp instructions (which don't modify register values), but is
    // invalidated by anything that writes %rax, or by control flow barriers.
    let mut rax_is_zero = false;

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // --- Pattern: redundant xorl %eax, %eax elimination ---
        // When %rax is already known to be zero (from a previous xorl %eax, %eax),
        // and only StoreRbp instructions intervene (which read but don't modify
        // registers), the repeated xorl is redundant.
        //
        // Common pattern from codegen zeroing multiple local variables:
        //   xorl %eax, %eax          # sets rax = 0
        //   movq %rax, -N(%rbp)      # stores 0, rax still 0
        //   xorl %eax, %eax          # REDUNDANT
        //   movq %rax, -M(%rbp)      # stores 0, rax still 0
        if rax_is_zero {
            if let LineKind::Other { dest_reg: 0 } = infos[i].kind {
                let trimmed = infos[i].trimmed(store.get(i));
                if trimmed == "xorl %eax, %eax" {
                    mark_nop(&mut infos[i]);
                    changed = true;
                    i += 1;
                    continue;
                }
            }
        }

        // Update rax_is_zero tracking based on current instruction.
        match infos[i].kind {
            LineKind::StoreRbp { .. } => {
                // Stores to stack don't modify registers, rax_is_zero unchanged.
            }
            LineKind::Other { dest_reg: 0 } => {
                // Something writes to %rax. Check if it's xorl %eax, %eax.
                let trimmed = infos[i].trimmed(store.get(i));
                rax_is_zero = trimmed == "xorl %eax, %eax";
            }
            LineKind::Other { dest_reg } if dest_reg != 0 => {
                // Writes to a non-rax register, rax_is_zero unchanged.
                // But check if it also reads/clobbers rax implicitly.
                // Most Other instructions only write their dest_reg.
                // Conservative: only keep rax_is_zero if the instruction
                // doesn't reference rax at all (via reg_refs).
                if infos[i].reg_refs & 1 != 0 {
                    // References rax - could be a read or write, invalidate
                    // But actually a read of rax is fine for rax_is_zero.
                    // Only a write to rax matters. Since dest_reg != 0,
                    // rax is not the destination, so it's a read - OK.
                    // Exception: instructions like div/idiv/mul/cqto that
                    // implicitly clobber rax through dest_reg rdx.
                    let trimmed = infos[i].trimmed(store.get(i));
                    if trimmed.starts_with("div")
                        || trimmed.starts_with("idiv")
                        || trimmed.starts_with("mul")
                        || trimmed.starts_with("imul")
                        || trimmed == "cqto"
                        || trimmed == "cqo"
                        || trimmed == "cdq"
                        || trimmed.starts_with("xchg")
                        || trimmed.starts_with("cmpxchg")
                    {
                        rax_is_zero = false;
                    }
                    // Otherwise rax is only read, not written - keep tracking.
                }
            }
            LineKind::LoadRbp { reg: 0, .. } => {
                // Load to rax - rax is no longer zero
                rax_is_zero = false;
            }
            LineKind::LoadRbp { .. } => {
                // Load to non-rax register, rax_is_zero unchanged.
            }
            LineKind::Label
            | LineKind::Jmp
            | LineKind::JmpIndirect
            | LineKind::CondJmp
            | LineKind::Ret
            | LineKind::Call => {
                // Control flow or label - invalidate tracking
                rax_is_zero = false;
            }
            LineKind::Pop { reg: 0 } | LineKind::SetCC { reg: 0 } => {
                rax_is_zero = false;
            }
            LineKind::Pop { .. }
            | LineKind::SetCC { .. }
            | LineKind::Push { .. }
            | LineKind::Cmp
            | LineKind::Directive => {
                // Don't affect rax
            }
            _ => {
                // Conservative: invalidate
                rax_is_zero = false;
            }
        }

        // --- Pattern: self-move elimination (movq %reg, %reg) ---
        // Pre-classified as SelfMove during classify_line, avoiding string parsing.
        if infos[i].kind == LineKind::SelfMove {
            mark_nop(&mut infos[i]);
            changed = true;
            i += 1;
            continue;
        }

        // --- Pattern: reverse-move elimination ---
        // Detects `movq %regA, %regB` followed by `movq %regB, %regA` and
        // eliminates the second mov (since %regA still holds the original value).
        //
        // Safety: We only skip NOPs and StoreRbp between the two instructions.
        // StoreRbp reads registers but never modifies any GP register value.
        // Any other instruction type causes the search to stop via `break`.
        if let LineKind::Other { dest_reg: dest_a } = infos[i].kind {
            if is_valid_gp_reg(dest_a) {
                let line_i = infos[i].trimmed(store.get(i));
                // Parse "movq %srcReg, %dstReg" pattern
                if let Some(rest) = line_i.strip_prefix("movq ") {
                    if let Some((src_str, dst_str)) = rest.split_once(',') {
                        let src = src_str.trim();
                        let dst = dst_str.trim();
                        let src_fam = register_family_fast(src);
                        let dst_fam = register_family_fast(dst);
                        // Both must be GP registers, different families, both register operands
                        if is_valid_gp_reg(src_fam)
                            && is_valid_gp_reg(dst_fam)
                            && src_fam != dst_fam
                            && src.starts_with('%')
                            && dst.starts_with('%')
                        {
                            // Find the next non-NOP, non-StoreRbp instruction.
                            // Limit search to 8 lines to avoid pathological scanning.
                            let mut j = i + 1;
                            let search_limit = (i + 8).min(len);
                            while j < search_limit {
                                if infos[j].is_nop() {
                                    j += 1;
                                    continue;
                                }
                                if matches!(infos[j].kind, LineKind::StoreRbp { .. }) {
                                    j += 1;
                                    continue;
                                }
                                break;
                            }
                            if j < search_limit {
                                // Check if line j is the reverse: movq %dstReg, %srcReg
                                if let LineKind::Other { dest_reg: dest_b } = infos[j].kind {
                                    if dest_b == src_fam {
                                        let line_j = infos[j].trimmed(store.get(j));
                                        if let Some(rest_j) = line_j.strip_prefix("movq ") {
                                            if let Some((src_j, dst_j)) = rest_j.split_once(',') {
                                                let src_j = src_j.trim();
                                                let dst_j = dst_j.trim();
                                                let src_j_fam = register_family_fast(src_j);
                                                let dst_j_fam = register_family_fast(dst_j);
                                                if src_j_fam == dst_fam && dst_j_fam == src_fam {
                                                    mark_nop(&mut infos[j]);
                                                    changed = true;
                                                    i += 1;
                                                    continue;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // --- Pattern: redundant jump to next label ---
        if infos[i].kind == LineKind::Jmp {
            let jmp_line = infos[i].trimmed(store.get(i));
            if let Some(target) = jmp_line.strip_prefix("jmp ") {
                let target = target.trim();
                // Find the next non-NOP, non-empty line
                let mut found_redundant = false;
                for j in (i + 1)..len {
                    if infos[j].is_nop() || infos[j].kind == LineKind::Empty {
                        continue;
                    }
                    if infos[j].kind == LineKind::Label {
                        let next = infos[j].trimmed(store.get(j));
                        if let Some(label) = next.strip_suffix(':') {
                            if label == target {
                                mark_nop(&mut infos[i]);
                                changed = true;
                                found_redundant = true;
                            }
                        }
                    }
                    break;
                }
                if found_redundant {
                    i += 1;
                    continue;
                }
            }
        }

        // --- Pattern: conditional branch inversion for fall-through ---
        // Detects:
        //   jCC .Ltrue        (conditional jump)
        //   jmp .Lfalse       (unconditional jump)
        //   .Ltrue:           (label matching the conditional target)
        //
        // Transforms to:
        //   j!CC .Lfalse      (inverted condition, jump to false target)
        //   .Ltrue:           (fall through naturally)
        if infos[i].kind == LineKind::CondJmp {
            let cond_line = infos[i].trimmed(store.get(i));
            // Parse: "jCC target" -> extract CC and target
            if let Some(space_pos) = cond_line.find(' ') {
                let cc = &cond_line[1..space_pos]; // e.g., "l", "ge", "ne"
                let cond_target = cond_line[space_pos + 1..].trim();
                // Find the next non-NOP line (should be jmp)
                let mut j = i + 1;
                while j < len && infos[j].is_nop() {
                    j += 1;
                }
                if j < len && infos[j].kind == LineKind::Jmp {
                    let jmp_line = infos[j].trimmed(store.get(j));
                    if let Some(jmp_target) = jmp_line.strip_prefix("jmp ") {
                        let jmp_target = jmp_target.trim();
                        // Find the next non-NOP/non-empty line after jmp (should be a label)
                        let mut k = j + 1;
                        while k < len && (infos[k].is_nop() || infos[k].kind == LineKind::Empty) {
                            k += 1;
                        }
                        if k < len && infos[k].kind == LineKind::Label {
                            let label_line = infos[k].trimmed(store.get(k));
                            if let Some(label_name) = label_line.strip_suffix(':') {
                                if label_name == cond_target {
                                    let inv_cc = invert_cc(cc);
                                    if inv_cc != cc {
                                        let new_line = format!("    j{} {}", inv_cc, jmp_target);
                                        replace_line(store, &mut infos[i], i, new_line);
                                        mark_nop(&mut infos[j]); // Remove the jmp
                                        changed = true;
                                        i += 1;
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // --- Pattern: adjacent store/load at same %rbp offset ---
        if let LineKind::StoreRbp {
            reg: sr,
            offset: so,
            size: ss,
        } = infos[i].kind
        {
            if i + 1 < len && !infos[i + 1].is_nop() {
                if let LineKind::LoadRbp {
                    reg: lr,
                    offset: lo,
                    size: ls,
                } = infos[i + 1].kind
                {
                    // Different register cases are handled by global_store_forwarding
                    if so == lo && ss == ls && sr == lr && sr != REG_NONE {
                        // Same register: load is redundant
                        mark_nop(&mut infos[i + 1]);
                        changed = true;
                        i += 1;
                        continue;
                    }
                }
            }
        }

        // --- Pattern: redundant zero/sign extension (including cltq) ---
        // Uses pre-classified ExtKind to avoid repeated starts_with/ends_with
        // string comparisons on every iteration.
        let mut ext_idx = i + 1;
        while ext_idx < len && ext_idx < i + 10 {
            if infos[ext_idx].is_nop() {
                ext_idx += 1;
                continue;
            }
            if matches!(infos[ext_idx].kind, LineKind::StoreRbp { .. }) {
                ext_idx += 1;
                continue;
            }
            break;
        }

        if ext_idx < len && !infos[ext_idx].is_nop() {
            let next_ext = infos[ext_idx].ext_kind;
            let prev_ext = infos[i].ext_kind;

            let is_redundant_ext = match next_ext {
                ExtKind::MovzbqAlRax => matches!(
                    prev_ext,
                    ExtKind::ProducerMovzbqToRax | ExtKind::MovzbqAlRax
                ),
                ExtKind::MovzwqAxRax => matches!(
                    prev_ext,
                    ExtKind::ProducerMovzwqToRax | ExtKind::MovzwqAxRax
                ),
                ExtKind::MovsbqAlRax => matches!(
                    prev_ext,
                    ExtKind::ProducerMovsbqToRax | ExtKind::MovsbqAlRax
                ),
                ExtKind::MovslqEaxRax => matches!(
                    prev_ext,
                    ExtKind::ProducerMovslqToRax | ExtKind::MovslqEaxRax
                ),
                ExtKind::Cltq => matches!(
                    prev_ext,
                    ExtKind::ProducerMovslqToRax
                        | ExtKind::ProducerMovqConstRax
                        | ExtKind::MovslqEaxRax
                        | ExtKind::Cltq
                ),
                ExtKind::MovlEaxEax => matches!(
                    prev_ext,
                    ExtKind::ProducerArith32
                        | ExtKind::ProducerMovlToEax
                        | ExtKind::ProducerMovzbToEax
                        | ExtKind::ProducerMovzbqToRax
                        | ExtKind::ProducerMovzwToEax
                        | ExtKind::ProducerMovzwqToRax
                        | ExtKind::ProducerDiv32
                        | ExtKind::MovlEaxEax
                ),
                _ => false,
            };

            if is_redundant_ext {
                mark_nop(&mut infos[ext_idx]);
                changed = true;
                i += 1;
                continue;
            }

            // --- Extended scan: cltq past non-rax-clobbering instructions ---
            if next_ext == ExtKind::Cltq && !is_redundant_ext {
                let i_writes_rax = match infos[i].kind {
                    LineKind::Other { dest_reg } => dest_reg == 0,
                    LineKind::LoadRbp { reg, .. } => reg == 0,
                    LineKind::StoreRbp { .. } => false,
                    LineKind::Nop | LineKind::Empty => false,
                    _ => true, // conservative: barriers, calls, etc. may write rax
                };

                if !i_writes_rax && i > 0 {
                    let mut found_producer = false;
                    let scan_limit = i.saturating_sub(6);
                    let mut k = i - 1;
                    while k >= scan_limit {
                        if infos[k].is_nop() {
                            if k == 0 {
                                break;
                            }
                            k -= 1;
                            continue;
                        }
                        if matches!(infos[k].kind, LineKind::StoreRbp { .. }) {
                            if k == 0 {
                                break;
                            }
                            k -= 1;
                            continue;
                        }
                        // Stop at barriers (labels, calls, jumps, ret)
                        if infos[k].is_barrier() {
                            break;
                        }
                        // Check if this instruction is a sign-extension producer for rax
                        let k_ext = infos[k].ext_kind;
                        if matches!(
                            k_ext,
                            ExtKind::ProducerMovslqToRax
                                | ExtKind::ProducerMovqConstRax
                                | ExtKind::MovslqEaxRax
                                | ExtKind::Cltq
                        ) {
                            found_producer = true;
                            break;
                        }
                        // Check if this instruction writes to %rax (family 0)
                        let writes_rax = match infos[k].kind {
                            LineKind::Other { dest_reg } => dest_reg == 0,
                            LineKind::LoadRbp { reg, .. } => reg == 0,
                            _ => true, // conservative: treat unknown as writing rax
                        };
                        if writes_rax {
                            break;
                        }
                        if k == 0 {
                            break;
                        }
                        k -= 1;
                    }
                    if found_producer {
                        mark_nop(&mut infos[ext_idx]);
                        changed = true;
                        i += 1;
                        continue;
                    }
                }
            }
        }

        i += 1;
    }
    changed
}

// ── Movq + extension/truncation fusion ───────────────────────────────────────
//
// Fuses `movq %REG, %rax` followed by a cast instruction into a single
// instruction. The two-instruction pattern arises from the accumulator-based
// codegen model: emit_load_operand loads a 64-bit value into %rax, then
// emit_cast_instrs emits an extension/truncation on %rax/%eax/%ax/%al.
//
// Fused patterns (all require REG != rax, no intervening non-NOP instructions):
//   movq %REG, %rax + movl %eax, %eax   -> movl %REGd, %eax    (truncate to u32)
//   movq %REG, %rax + movslq %eax, %rax -> movslq %REGd, %rax  (sign-extend i32->i64)
//   movq %REG, %rax + cltq              -> movslq %REGd, %rax   (sign-extend i32->i64)
//   movq %REG, %rax + movzbq %al, %rax  -> movzbl %REGb, %eax  (zero-extend u8->i64)
//   movq %REG, %rax + movzwq %ax, %rax  -> movzwl %REGw, %eax  (zero-extend u16->i64)
//   movq %REG, %rax + movsbq %al, %rax  -> movsbq %REGb, %rax  (sign-extend i8->i64)

pub(super) fn fuse_movq_ext_truncation(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();

    let mut i = 0;
    while i + 1 < len {
        // Look for ProducerMovqRegToRax
        if infos[i].ext_kind != ExtKind::ProducerMovqRegToRax {
            i += 1;
            continue;
        }

        // Find next non-NOP instruction (skip only NOPs, not stores)
        let mut j = i + 1;
        while j < len && infos[j].is_nop() {
            j += 1;
        }
        if j >= len {
            i += 1;
            continue;
        }

        // Check if next instruction is a fusable extension/truncation on %rax
        let next_ext = infos[j].ext_kind;
        let fusable = matches!(
            next_ext,
            ExtKind::MovlEaxEax
                | ExtKind::MovslqEaxRax
                | ExtKind::Cltq
                | ExtKind::MovzbqAlRax
                | ExtKind::MovzwqAxRax
                | ExtKind::MovsbqAlRax
        );
        if !fusable {
            i += 1;
            continue;
        }

        // Extract source register family from the movq instruction
        let movq_line = infos[i].trimmed(store.get(i));
        let src_family = if let Some(rest) = movq_line.strip_prefix("movq ") {
            if let Some((src, _dst)) = rest.split_once(',') {
                let src = src.trim();
                let fam = register_family_fast(src);
                if fam != REG_NONE && fam != 0 {
                    fam
                } else {
                    REG_NONE
                }
            } else {
                REG_NONE
            }
        } else {
            REG_NONE
        };

        if src_family == REG_NONE {
            i += 1;
            continue;
        }

        // Build the fused instruction based on the extension type
        let new_text = match next_ext {
            ExtKind::MovlEaxEax => {
                let src_32 = REG_NAMES[1][src_family as usize];
                format!("    movl {}, %eax", src_32)
            }
            ExtKind::MovslqEaxRax | ExtKind::Cltq => {
                let src_32 = REG_NAMES[1][src_family as usize];
                format!("    movslq {}, %rax", src_32)
            }
            ExtKind::MovzbqAlRax => {
                let src_8 = REG_NAMES[3][src_family as usize];
                format!("    movzbl {}, %eax", src_8)
            }
            ExtKind::MovzwqAxRax => {
                let src_16 = REG_NAMES[2][src_family as usize];
                format!("    movzwl {}, %eax", src_16)
            }
            ExtKind::MovsbqAlRax => {
                let src_8 = REG_NAMES[3][src_family as usize];
                format!("    movsbq {}, %rax", src_8)
            }
            _ => unreachable!("mov+ext fusion matched unexpected ExtKind"),
        };

        replace_line(store, &mut infos[i], i, new_text);
        mark_nop(&mut infos[j]);
        changed = true;
        i = j + 1;
        continue;
    }
    changed
}
