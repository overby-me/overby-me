//! Unused callee-saved register elimination pass.
//!
//! After peephole optimization, some callee-saved registers may no longer be
//! referenced in the function body (all uses were optimized away). This pass
//! detects such registers and removes their prologue save / epilogue restore
//! instructions. The stack frame is not shrunk (see rationale inside function).

use super::super::types::*;
use super::helpers::*;

pub(super) fn eliminate_unused_callee_saves(store: &LineStore, infos: &mut [LineInfo]) {
    let len = store.len();
    if len == 0 {
        return;
    }

    let mut i = 0;
    while i < len {
        // Look for the prologue: pushq %rbp
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        if !matches!(infos[i].kind, LineKind::Push { reg: 5 }) {
            i += 1;
            continue;
        }

        // Next non-nop should be "movq %rsp, %rbp"
        let mut j = next_non_nop(infos, i + 1, len);
        if j >= len {
            i = j;
            continue;
        }
        let mov_rbp_line = infos[j].trimmed(store.get(j));
        if mov_rbp_line != "movq %rsp, %rbp" {
            i = j + 1;
            continue;
        }
        j += 1;

        // Next non-nop should be "subq $N, %rsp"
        j = next_non_nop(infos, j, len);
        if j >= len {
            i = j;
            continue;
        }
        let subq_line = infos[j].trimmed(store.get(j));
        if let Some(rest) = subq_line.strip_prefix("subq $") {
            if let Some(val_str) = rest.strip_suffix(", %rsp") {
                if val_str.parse::<i64>().is_err() {
                    i = j + 1;
                    continue;
                }
            } else {
                i = j + 1;
                continue;
            }
        } else {
            i = j + 1;
            continue;
        }
        j += 1;

        // Collect callee-saved register saves immediately after subq.
        struct CalleeSave {
            reg: RegId,
            offset: i32,
            save_line_idx: usize,
        }
        let mut saves: Vec<CalleeSave> = Vec::new();
        j = next_non_nop(infos, j, len);
        while j < len {
            if infos[j].is_nop() {
                j += 1;
                continue;
            }
            if let LineKind::StoreRbp {
                reg,
                offset,
                size: MoveSize::Q,
            } = infos[j].kind
            {
                if is_callee_saved_reg(reg) && offset < 0 {
                    saves.push(CalleeSave {
                        reg,
                        offset,
                        save_line_idx: j,
                    });
                    j += 1;
                    continue;
                }
            }
            break;
        }

        if saves.is_empty() {
            i = j;
            continue;
        }

        // Find the end of this function by looking for the .size directive.
        let body_start = j;
        let mut func_end = len;
        for k in body_start..len {
            if infos[k].is_nop() {
                continue;
            }
            let line = infos[k].trimmed(store.get(k));
            if line.starts_with(".size ") {
                func_end = k + 1;
                break;
            }
        }

        // For each callee-saved register, check if it's referenced in the body.
        for save in &saves {
            let reg = save.reg;

            let mut restore_indices: Vec<usize> = Vec::new();
            let mut body_has_reference = false;

            for k in body_start..func_end {
                if infos[k].is_nop() {
                    continue;
                }

                if let LineKind::LoadRbp {
                    reg: load_reg,
                    offset: load_offset,
                    size: MoveSize::Q,
                } = infos[k].kind
                {
                    if load_reg == reg && load_offset == save.offset && is_near_epilogue(infos, k) {
                        restore_indices.push(k);
                        continue;
                    }
                }

                if line_references_reg_fast(&infos[k], reg) {
                    body_has_reference = true;
                    break;
                }
            }

            if !body_has_reference && !restore_indices.is_empty() {
                mark_nop(&mut infos[save.save_line_idx]);
                for &ri in &restore_indices {
                    mark_nop(&mut infos[ri]);
                }
            }
        }

        // Note: we intentionally do NOT shrink the stack frame (subq $N, %rsp)
        // even though some callee-saved saves were eliminated. The remaining saves
        // still reference their original rbp-relative offsets, which are below rsp
        // if we shrink the frame. Data below rsp can be corrupted by interrupts
        // or signal handlers. Keeping the original frame size ensures all saved
        // registers remain safely above rsp. The unused slots become dead space.
        // TODO: To also shrink the frame, we would need to rewrite the offsets of
        // all remaining callee-saved saves/restores to pack them tightly.

        i = func_end;
    }
}
