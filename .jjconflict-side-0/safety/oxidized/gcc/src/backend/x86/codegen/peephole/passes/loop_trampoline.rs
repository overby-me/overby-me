//! Loop trampoline elimination pass.
//!
//! SSA codegen creates "trampoline" blocks for loop back-edges to resolve phi
//! nodes. Instead of updating loop variables in-place, it creates new SSA values
//! in fresh registers and uses a separate block to shuffle them back:
//!
//!   .LOOP:
//!       ; ... loop body using %r9, %r10, %r11 ...
//!       movq %r9, %r14           ; copy old dest to new reg
//!       addq $320, %r14          ; modify new dest
//!       movq %r10, %r15          ; copy old frac to new reg
//!       addl %r8d, %r15d         ; modify new frac
//!       ; ... loop condition ...
//!       jne .TRAMPOLINE
//!   .TRAMPOLINE:
//!       movq %r14, %r9           ; shuffle new dest back
//!       movq %r15, %r10          ; shuffle new frac back
//!       jmp .LOOP
//!
//! This pass detects trampoline blocks and coalesces the register copies:
//!   1. For each trampoline copy %src -> %dst, find where %src was created in
//!      the predecessor (as a copy from %dst followed by modifications).
//!   2. Rewrite those modifications to target %dst directly.
//!   3. NOP the initial copy and the trampoline copy.
//!   4. Redirect the branch directly to the loop header.

use super::super::types::*;
use super::helpers::*;

/// Look up a register family from a name string that does NOT have the '%' prefix.
/// This avoids allocating a `format!("%{}", name)` just to call `register_family_fast`.
/// Intentionally duplicates the core lookup logic from `register_family_fast` in types.rs
/// to avoid the allocation overhead on this hot path.
#[inline]
fn register_family_no_prefix(name: &str) -> RegId {
    let b = name.as_bytes();
    let len = b.len();
    if len < 2 {
        return REG_NONE;
    }
    // Dispatch on first character (same logic as register_family_fast but without '%' prefix)
    match b[0] {
        b'r' | b'e' => {
            if len < 3 {
                // len==2: only r8, r9 are valid
                return if b[0] == b'r' {
                    reg_digit_to_id(b[1])
                } else {
                    REG_NONE
                };
            }
            match (b[1], b[2]) {
                (b'a', b'x') => 0, // rax / eax
                (b'c', b'x') => 1, // rcx / ecx
                (b'd', b'x') => 2, // rdx / edx
                (b'd', b'i') => 7, // rdi / edi
                (b'b', b'x') => 3, // rbx / ebx
                (b'b', b'p') => 5, // rbp / ebp
                (b's', b'p') => 4, // rsp / esp
                (b's', b'i') => 6, // rsi / esi
                (b'8', _) => 8,    // r8d / r8w / r8b
                (b'9', _) => 9,    // r9d / r9w / r9b
                (b'1', b'0') => 10,
                (b'1', b'1') => 11,
                (b'1', b'2') => 12,
                (b'1', b'3') => 13,
                (b'1', b'4') => 14,
                (b'1', b'5') => 15,
                _ => REG_NONE,
            }
        }
        // 16-bit / 8-bit short forms: ax, al, ah, cx, cl, etc.
        b'a' => {
            if matches!(b[1], b'x' | b'l' | b'h') {
                0
            } else {
                REG_NONE
            }
        }
        b'c' => {
            if matches!(b[1], b'x' | b'l' | b'h') {
                1
            } else {
                REG_NONE
            }
        }
        b'd' => match b[1] {
            b'i' => 7,
            b'x' | b'l' | b'h' => 2,
            _ => REG_NONE,
        },
        b'b' => match b[1] {
            b'p' => 5,
            b'x' | b'l' | b'h' => 3,
            _ => REG_NONE,
        },
        b's' => match b[1] {
            b'p' => 4,
            b'i' => 6,
            _ => REG_NONE,
        },
        _ => REG_NONE,
    }
}

/// Check if trimmed instruction matches "movq <first_reg>, <second_reg>" exactly.
/// `first_reg` and `second_reg` should include the '%' prefix (e.g., "%rax").
/// Avoids allocating a format!() string for comparison.
#[inline]
fn is_movq_reg_reg(trimmed: &str, first_reg: &str, second_reg: &str) -> bool {
    // Expected: "movq %rXX, %rYY" (regs include '%' prefix from REG_NAMES)
    let b = trimmed.as_bytes();
    let expected_len = 5 + first_reg.len() + 2 + second_reg.len(); // "movq " + first + ", " + second
    if b.len() != expected_len {
        return false;
    }
    trimmed.starts_with("movq ")
        && trimmed[5..].starts_with(first_reg)
        && trimmed[5 + first_reg.len()..].starts_with(", ")
        && trimmed[5 + first_reg.len() + 2..] == *second_reg
}

/// Check if trimmed instruction matches "movslq <first_reg>, <second_reg>" exactly.
#[inline]
fn is_movslq_reg_reg(trimmed: &str, first_reg: &str, second_reg: &str) -> bool {
    let expected_len = 7 + first_reg.len() + 2 + second_reg.len(); // "movslq " + first + ", " + second
    if trimmed.len() != expected_len {
        return false;
    }
    trimmed.starts_with("movslq ")
        && trimmed[7..].starts_with(first_reg)
        && trimmed[7 + first_reg.len()..].starts_with(", ")
        && trimmed[7 + first_reg.len() + 2..] == *second_reg
}

pub(super) fn eliminate_loop_trampolines(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    if len < 4 {
        return false;
    }

    let mut changed = false;

    // Build a map of label_name -> line_index for all labels.
    let mut label_positions: Vec<(u32, usize)> = Vec::new();
    let mut max_label_num: u32 = 0;

    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        if infos[i].kind == LineKind::Label {
            let trimmed = infos[i].trimmed(store.get(i));
            if let Some(n) = parse_label_number(trimmed) {
                label_positions.push((n, i));
                if n > max_label_num {
                    max_label_num = n;
                }
            }
        }
    }

    if label_positions.is_empty() {
        return false;
    }

    let table_size = (max_label_num + 1) as usize;

    // Build label_num -> line_index lookup
    let mut label_line: Vec<usize> = vec![usize::MAX; table_size];
    for &(num, idx) in &label_positions {
        label_line[num as usize] = idx;
    }

    // Count branch references to each label AND build reverse index from
    // label_num -> first conditional branch line targeting it.
    // This eliminates the O(n) scan per trampoline candidate that was the
    // dominant bottleneck (previously ~20% of total compile time on large files).
    let mut label_branch_count: Vec<u32> = vec![0; table_size];
    let mut cond_branch_for_label: Vec<usize> = vec![usize::MAX; table_size];

    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        match infos[i].kind {
            LineKind::Jmp | LineKind::CondJmp => {
                let trimmed = infos[i].trimmed(store.get(i));
                if let Some(target) = extract_jump_target(trimmed) {
                    if let Some(n) = parse_dotl_number(target) {
                        if (n as usize) < table_size {
                            label_branch_count[n as usize] += 1;
                            // Record the first conditional branch targeting this label
                            if infos[i].kind == LineKind::CondJmp
                                && cond_branch_for_label[n as usize] == usize::MAX
                            {
                                cond_branch_for_label[n as usize] = i;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Find trampoline blocks
    for &(tramp_num, tramp_label_idx) in &label_positions {
        if label_branch_count[tramp_num as usize] != 1 {
            continue;
        }

        // Parse the trampoline block contents
        let mut tramp_moves: Vec<(RegId, RegId)> = Vec::new();
        let mut tramp_jmp_target: Option<u32> = None;
        let mut has_stack_load = false;
        let mut tramp_stack_loads: Vec<(i32, RegId, usize, usize)> = Vec::new();
        let mut tramp_all_lines: Vec<usize> = Vec::new();

        let mut j = tramp_label_idx + 1;
        while j < len {
            if infos[j].is_nop() || infos[j].kind == LineKind::Empty {
                j += 1;
                continue;
            }
            let trimmed = infos[j].trimmed(store.get(j));

            // Check for movq %regA, %regB
            if let Some(rest) = trimmed.strip_prefix("movq %") {
                if let Some((src_str, dst_str)) = rest.split_once(", %") {
                    let src_fam = register_family_no_prefix(src_str);
                    let dst_fam = register_family_no_prefix(dst_str.trim());
                    if src_fam != REG_NONE && dst_fam != REG_NONE && src_fam != dst_fam {
                        tramp_moves.push((src_fam, dst_fam));
                        tramp_all_lines.push(j);
                        j += 1;
                        continue;
                    }
                }
            }

            // Check for movslq %regA, %regB
            if let Some(rest) = trimmed.strip_prefix("movslq %") {
                if let Some((src_str, dst_str)) = rest.split_once(", %") {
                    let src_fam = register_family_no_prefix(src_str);
                    let dst_fam = register_family_no_prefix(dst_str.trim());
                    if src_fam != REG_NONE && dst_fam != REG_NONE && src_fam != dst_fam {
                        tramp_moves.push((src_fam, dst_fam));
                        tramp_all_lines.push(j);
                        j += 1;
                        continue;
                    }
                }
            }

            // Check for stack load pattern: movq -N(%rbp), %rax
            if let LineKind::LoadRbp { reg: 0, offset, .. } = infos[j].kind {
                let mut k = j + 1;
                while k < len && (infos[k].is_nop() || infos[k].kind == LineKind::Empty) {
                    k += 1;
                }
                if k < len {
                    let next_trimmed = infos[k].trimmed(store.get(k));
                    if let Some(rest) = next_trimmed.strip_prefix("movq %rax, %") {
                        let dst_fam = register_family_no_prefix(rest.trim());
                        if dst_fam != REG_NONE && dst_fam != 0 {
                            has_stack_load = true;
                            tramp_stack_loads.push((offset, dst_fam, j, k));
                            tramp_all_lines.push(j);
                            tramp_all_lines.push(k);
                            j = k + 1;
                            continue;
                        }
                    }
                }
                break;
            }

            // Check for jmp .LBB_N (final instruction)
            if infos[j].kind == LineKind::Jmp {
                if let Some(target) = extract_jump_target(trimmed) {
                    if let Some(n) = parse_dotl_number(target) {
                        tramp_jmp_target = Some(n);
                        tramp_all_lines.push(j);
                    }
                }
                break;
            }

            break;
        }

        let target_num = match tramp_jmp_target {
            Some(n) => n,
            None => continue,
        };

        if tramp_moves.is_empty() && !has_stack_load {
            continue;
        }

        // Find the conditional branch that targets this trampoline using
        // the pre-built reverse index (O(1) instead of O(n) scan).
        let branch_idx = cond_branch_for_label[tramp_num as usize];
        if branch_idx == usize::MAX {
            continue;
        }

        // Per-move coalescing
        let mut move_coalesced: Vec<bool> = Vec::with_capacity(tramp_moves.len());
        let mut coalesce_actions: Vec<(usize, RegId, RegId)> = Vec::new();
        let mut copy_nop_lines: Vec<usize> = Vec::new();
        for &(src_fam, dst_fam) in &tramp_moves {
            let src_64 = REG_NAMES[0][src_fam as usize];
            let dst_64 = REG_NAMES[0][dst_fam as usize];

            let mut copy_idx = None;
            let mut modifications: Vec<usize> = Vec::new();
            let mut scan_ok = true;

            let mut k = branch_idx;
            while k > 0 {
                k -= 1;
                if infos[k].is_nop() || infos[k].kind == LineKind::Empty {
                    continue;
                }
                if infos[k].kind == LineKind::Label {
                    break;
                }
                if matches!(
                    infos[k].kind,
                    LineKind::Call | LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret
                ) {
                    break;
                }

                let trimmed = infos[k].trimmed(store.get(k));

                let modifies_src = match infos[k].kind {
                    LineKind::Other { dest_reg } => dest_reg == src_fam,
                    LineKind::StoreRbp { .. } => false,
                    LineKind::LoadRbp { reg, .. } => reg == src_fam,
                    LineKind::SetCC { reg } => {
                        // SetCC is a partial write (only 1 byte) — it cannot be
                        // safely rewritten to a different register family because
                        // it does not clear the upper bytes. Bail out of coalescing
                        // entirely when SetCC modifies src_fam.
                        if reg == src_fam {
                            scan_ok = false;
                            break;
                        }
                        false
                    }
                    LineKind::Pop { reg } => reg == src_fam,
                    _ => false,
                };

                if modifies_src {
                    // Check if this is the initial copy: "movq <dst_64>, <src_64>"
                    if is_movq_reg_reg(trimmed, dst_64, src_64) {
                        copy_idx = Some(k);
                        break;
                    }
                    // Check for movslq variant
                    let dst_32 = REG_NAMES[1][dst_fam as usize];
                    if is_movslq_reg_reg(trimmed, dst_32, src_64) {
                        scan_ok = false;
                        break;
                    }
                    modifications.push(k);
                    continue;
                }

                if infos[k].reg_refs & (1u16 << src_fam) != 0 {
                    if infos[k].reg_refs & (1u16 << dst_fam) != 0 {
                        scan_ok = false;
                        break;
                    }
                    modifications.push(k);
                    continue;
                }

                if infos[k].reg_refs & (1u16 << dst_fam) != 0 {
                    scan_ok = false;
                    break;
                }
            }

            if !scan_ok || copy_idx.is_none() {
                move_coalesced.push(false);
                continue;
            }

            let copy_idx = copy_idx.unwrap();

            // Verify fall-through safety
            let check_regs = [dst_fam, src_fam];
            let mut fall_through_safe = true;
            let mut m = branch_idx + 1;
            let mut killed = [false; 2];
            let mut jumps_followed = 0u32;
            'ft_scan: while m < len {
                if infos[m].is_nop()
                    || infos[m].kind == LineKind::Empty
                    || infos[m].kind == LineKind::Label
                {
                    m += 1;
                    continue;
                }
                if infos[m].kind == LineKind::Jmp {
                    if jumps_followed < 2 && (!killed[0] || !killed[1]) {
                        let trimmed = infos[m].trimmed(store.get(m));
                        if let Some(target) = extract_jump_target(trimmed) {
                            if let Some(n) = parse_dotl_number(target) {
                                if (n as usize) < label_line.len()
                                    && label_line[n as usize] != usize::MAX
                                {
                                    m = label_line[n as usize] + 1;
                                    jumps_followed += 1;
                                    continue 'ft_scan;
                                }
                            }
                        }
                    }
                    break;
                }
                if matches!(infos[m].kind, LineKind::JmpIndirect | LineKind::Ret) {
                    break;
                }
                if infos[m].kind == LineKind::CondJmp {
                    fall_through_safe = false;
                    break;
                }
                for i in 0..2 {
                    if killed[i] {
                        continue;
                    }
                    let reg = check_regs[i];
                    if infos[m].reg_refs & (1u16 << reg) != 0 {
                        fall_through_safe = false;
                        break;
                    }
                    let writes_reg = match infos[m].kind {
                        LineKind::Other { dest_reg } => dest_reg == reg,
                        LineKind::LoadRbp { reg: r, .. } => r == reg,
                        LineKind::SetCC { reg: r } => r == reg,
                        LineKind::Pop { reg: r } => r == reg,
                        _ => false,
                    };
                    if writes_reg {
                        killed[i] = true;
                    }
                }
                if !fall_through_safe {
                    break;
                }
                if killed[0] && killed[1] {
                    break;
                }
                m += 1;
            }
            if !fall_through_safe {
                move_coalesced.push(false);
                continue;
            }

            copy_nop_lines.push(copy_idx);

            for &mod_idx in &modifications {
                coalesce_actions.push((mod_idx, src_fam, dst_fam));
            }

            move_coalesced.push(true);
        }

        // Stack-load coalescing is not attempted (see comment in original code).
        let stack_coalesced: Vec<bool> = vec![false; tramp_stack_loads.len()];
        let stack_nop_lines: Vec<usize> = Vec::new();
        let stack_store_rewrites: Vec<(usize, String)> = Vec::new();

        let num_moves_coalesced = move_coalesced.iter().filter(|&&c| c).count();
        let num_stack_coalesced = stack_coalesced.iter().filter(|&&c| c).count();
        let total_coalesced = num_moves_coalesced + num_stack_coalesced;

        if total_coalesced == 0 {
            continue;
        }

        let all_coalesced = num_moves_coalesced == tramp_moves.len()
            && num_stack_coalesced == tramp_stack_loads.len();

        // Apply the register-register coalescing actions
        for &nop_idx in &copy_nop_lines {
            mark_nop(&mut infos[nop_idx]);
        }

        for &(mod_idx, old_fam, new_fam) in &coalesce_actions {
            let old_line = infos[mod_idx].trimmed(store.get(mod_idx)).to_string();
            if let Some(new_line) = rewrite_instruction_register(&old_line, old_fam, new_fam) {
                replace_line(
                    store,
                    &mut infos[mod_idx],
                    mod_idx,
                    format!("    {}", new_line),
                );
            }
        }

        for &(store_idx, ref new_line) in &stack_store_rewrites {
            replace_line(store, &mut infos[store_idx], store_idx, new_line.clone());
        }
        for &nop_idx in &stack_nop_lines {
            mark_nop(&mut infos[nop_idx]);
        }

        if all_coalesced {
            for &line_idx in &tramp_all_lines {
                mark_nop(&mut infos[line_idx]);
            }
            mark_nop(&mut infos[tramp_label_idx]);

            let branch_trimmed = infos[branch_idx].trimmed(store.get(branch_idx)).to_string();
            if let Some(space_pos) = branch_trimmed.find(' ') {
                let cc = &branch_trimmed[..space_pos];
                let target_label = format!(".LBB{}", target_num);
                let new_branch = format!("    {} {}", cc, target_label);
                replace_line(store, &mut infos[branch_idx], branch_idx, new_branch);
            }
        } else {
            for (idx, &(src_fam, dst_fam)) in tramp_moves.iter().enumerate() {
                if !move_coalesced[idx] {
                    continue;
                }
                let src_64 = REG_NAMES[0][src_fam as usize];
                let dst_64 = REG_NAMES[0][dst_fam as usize];
                for &line_idx in &tramp_all_lines {
                    if infos[line_idx].is_nop() {
                        continue;
                    }
                    let trimmed = infos[line_idx].trimmed(store.get(line_idx));
                    if is_movq_reg_reg(trimmed, src_64, dst_64) {
                        mark_nop(&mut infos[line_idx]);
                        break;
                    }
                    let src_32 = REG_NAMES[1][src_fam as usize];
                    if is_movslq_reg_reg(trimmed, src_32, dst_64) {
                        mark_nop(&mut infos[line_idx]);
                        break;
                    }
                }
            }
        }

        changed = true;
    }

    changed
}

/// Rewrite an instruction to use a different register family.
fn rewrite_instruction_register(inst: &str, old_fam: RegId, new_fam: RegId) -> Option<String> {
    let result = replace_reg_family(inst, old_fam, new_fam);
    if result == inst {
        None
    } else {
        Some(result)
    }
}
