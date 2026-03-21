//! Push/pop pair elimination passes.
//!
//! Eliminates redundant push/pop pairs and push/binop/move/pop patterns
//! that arise from the stack-based codegen model.

use super::super::types::*;

pub(super) fn eliminate_push_pop_pairs(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();

    for i in 0..len.saturating_sub(2) {
        let push_reg_id = match infos[i].kind {
            LineKind::Push { reg } if reg != REG_NONE => reg,
            _ => continue,
        };

        for j in (i + 1)..std::cmp::min(i + 4, len) {
            if infos[j].is_nop() {
                continue;
            }
            if let LineKind::Pop { reg: pop_reg_id } = infos[j].kind {
                if pop_reg_id == push_reg_id {
                    let mut safe = true;
                    for k in (i + 1)..j {
                        if infos[k].is_nop() {
                            continue;
                        }
                        if instruction_modifies_reg_id(&infos[k], push_reg_id) {
                            safe = false;
                            break;
                        }
                        // Check for instructions that implicitly modify the stack
                        // (pushfq, popfq, pushfl, popfl, etc.) -- these read/write
                        // the stack slot that our push placed data on, so eliminating
                        // the push/pop pair would be incorrect.
                        if instruction_modifies_stack(store.get(k), &infos[k]) {
                            safe = false;
                            break;
                        }
                    }
                    if safe {
                        mark_nop(&mut infos[i]);
                        mark_nop(&mut infos[j]);
                        changed = true;
                    }
                }
                break;
            }
            if infos[j].is_push() {
                break;
            }
            if matches!(
                infos[j].kind,
                LineKind::Call | LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret
            ) {
                break;
            }
        }
    }
    changed
}

/// Check if an instruction implicitly modifies the stack pointer or accesses
/// the stack in ways that would make push/pop elimination incorrect.
/// This catches instructions like pushfq, popfq, pushfl, popfl, etc. that
/// read/write stack memory without being classified as Push/Pop.
fn instruction_modifies_stack(line: &str, info: &LineInfo) -> bool {
    if info.is_nop() {
        return false;
    }
    let s = info.trimmed(line);
    let b = s.as_bytes();
    if b.is_empty() {
        return false;
    }
    // pushfq, pushfl, pushf, popfq, popfl, popf -- these modify %rsp and
    // read/write the stack, making it unsafe to eliminate surrounding push/pop
    if b[0] == b'p' && (s.starts_with("pushf") || s.starts_with("popf")) {
        return true;
    }
    // subq/addq to %rsp
    if (s.starts_with("subq ") || s.starts_with("addq ")) && s.ends_with("%rsp") {
        return true;
    }
    false
}

/// Fast check whether a line's instruction modifies a register identified by RegId.
/// Uses pre-parsed `LineInfo` fields to avoid string parsing in the hot path.
#[inline]
fn instruction_modifies_reg_id(info: &LineInfo, reg_id: RegId) -> bool {
    match info.kind {
        LineKind::StoreRbp { .. }
        | LineKind::Cmp
        | LineKind::Nop
        | LineKind::Empty
        | LineKind::Label
        | LineKind::Directive
        | LineKind::Jmp
        | LineKind::JmpIndirect
        | LineKind::CondJmp
        | LineKind::SelfMove => false,
        LineKind::LoadRbp { reg, .. } => reg == reg_id,
        LineKind::Pop { reg } => reg == reg_id,
        LineKind::Push { .. } => false, // push reads, doesn't modify the source reg
        LineKind::SetCC { reg } => reg_id == reg, // setCC writes to the byte register's family
        LineKind::Call => matches!(reg_id, 0 | 1 | 2 | 6 | 7 | 8 | 9 | 10 | 11),
        LineKind::Ret => false,
        LineKind::Other { dest_reg } => {
            if dest_reg == reg_id {
                return true;
            }
            // div/idiv also clobber rdx (family 2), and rax (family 0)
            // mul also clobbers rdx
            if dest_reg == 0 && reg_id == 2 {
                return true; // TODO: could be more precise
            }
            false
        }
    }
}

pub(super) fn eliminate_binop_push_pop_pattern(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    let mut changed = false;
    let len = store.len();

    let mut i = 0;
    while i + 3 < len {
        let push_reg_id = match infos[i].kind {
            LineKind::Push { reg } if reg != REG_NONE => reg,
            _ => {
                i += 1;
                continue;
            }
        };

        let push_line = infos[i].trimmed(store.get(i));
        let push_reg = match push_line.strip_prefix("pushq ") {
            Some(r) => r.trim(),
            None => {
                i += 1;
                continue;
            }
        };

        // Find next 3 non-NOP lines
        let mut real_indices = [0usize; 3];
        let count = collect_non_nop_indices(infos, i, len, &mut real_indices);

        if count == 3 {
            let load_idx = real_indices[0];
            let move_idx = real_indices[1];
            let pop_idx = real_indices[2];

            if let LineKind::Pop { reg: pop_reg_id } = infos[pop_idx].kind {
                if pop_reg_id == push_reg_id {
                    // Check no stack-modifying instructions between push and pop
                    let mut stack_safe = true;
                    for k in (i + 1)..=pop_idx {
                        if !infos[k].is_nop() && instruction_modifies_stack(store.get(k), &infos[k])
                        {
                            stack_safe = false;
                            break;
                        }
                    }
                    if !stack_safe {
                        i += 1;
                        continue;
                    }

                    let load_line = infos[load_idx].trimmed(store.get(load_idx));
                    let move_line = infos[move_idx].trimmed(store.get(move_idx));

                    if let Some(move_target) = parse_reg_to_reg_move(move_line, push_reg) {
                        if instruction_writes_to(load_line, push_reg)
                            && can_redirect_instruction(load_line)
                        {
                            if let Some(new_load) =
                                replace_dest_register(load_line, push_reg, move_target)
                            {
                                mark_nop(&mut infos[i]);
                                let new_text = format!("    {}", new_load);
                                replace_line(store, &mut infos[load_idx], load_idx, new_text);
                                mark_nop(&mut infos[move_idx]);
                                mark_nop(&mut infos[pop_idx]);
                                changed = true;
                                i = pop_idx + 1;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        i += 1;
    }
    changed
}
