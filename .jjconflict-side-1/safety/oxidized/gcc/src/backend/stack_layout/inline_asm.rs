//! Inline asm callee-saved register scanning.
//!
//! Scan functions for inline asm instructions and identify which callee-saved
//! registers are clobbered. Three variants handle different architectures:
//! - basic: x86 and RISC-V (explicit register constraints)
//! - with_overflow: x86 (overflow into callee-saved scratch pool)
//! - with_generic: i686 (conservative marking for generic constraints)

use super::super::regalloc::PhysReg;
use crate::common::fx_hash::FxHashSet;
use crate::ir::reexports::{Instruction, IrFunction};

// ── Inline asm callee-saved scanning ──────────────────────────────────────

/// Scan inline-asm instructions for callee-saved register usage.
///
/// Iterates over all inline-asm instructions in `func`, checking output/input
/// constraints and clobber lists for callee-saved registers.  Uses two callbacks:
/// - `constraint_to_phys`: maps an output/input constraint string to a PhysReg
/// - `clobber_to_phys`: maps a clobber register name to a PhysReg
///
/// Any discovered callee-saved PhysRegs are appended to `used` (deduplicated).
/// This shared helper eliminates duplicated scan loops in x86 and RISC-V.
///
/// When a generic register class constraint (e.g. "r", "q", "g") is found that
/// doesn't map to a specific register, ALL callee-saved registers from
/// `all_callee_saved` are conservatively marked as clobbered. This prevents the
/// register allocator from assigning callee-saved registers to values whose live
/// ranges span the inline asm block, since the scratch register allocator may
/// pick any callee-saved register at codegen time. This is especially important
/// on i686 where there are only 3 callee-saved GP registers (ebx, esi, edi) and
/// the scratch pool includes all of them.
pub fn collect_inline_asm_callee_saved(
    func: &IrFunction,
    used: &mut Vec<PhysReg>,
    constraint_to_phys: impl Fn(&str) -> Option<PhysReg>,
    clobber_to_phys: impl Fn(&str) -> Option<PhysReg>,
) {
    collect_inline_asm_callee_saved_inner(func, used, constraint_to_phys, clobber_to_phys, &[])
}

/// Like `collect_inline_asm_callee_saved`, but triggers conservative marking
/// of all callee-saved registers when any single inline asm has more generic GP
/// register operands than the caller-saved scratch pool can hold. This avoids
/// the overly conservative behavior of `_with_generic` (which marks all callee-saved
/// for ANY generic "r" constraint, even if only 1 register is needed).
pub fn collect_inline_asm_callee_saved_with_overflow(
    func: &IrFunction,
    used: &mut Vec<PhysReg>,
    constraint_to_phys: impl Fn(&str) -> Option<PhysReg>,
    clobber_to_phys: impl Fn(&str) -> Option<PhysReg>,
    all_callee_saved: &[PhysReg],
    caller_saved_scratch_count: usize,
) {
    let mut already: FxHashSet<u8> = used.iter().map(|r| r.0).collect();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::InlineAsm {
                outputs,
                inputs,
                clobbers,
                ..
            } = inst
            {
                // Count how many generic GP register operands this inline asm has.
                // This approximates how many scratch registers will be needed.
                // Note: "+" outputs create synthetic inputs too, but those are handled
                // by finalize (not scratch alloc), so we only count non-plus outputs
                // and non-synthetic inputs for the overflow check.
                let mut generic_gp_count = 0usize;
                let mut specific_claimed = 0usize;
                let mut clobber_claimed = 0usize;
                let num_plus = outputs.iter().filter(|(c, _, _)| c.contains('+')).count();

                for (constraint, _, _) in outputs {
                    let c = constraint.trim_start_matches(['=', '+', '&', '%']);
                    if let Some(phys) = constraint_to_phys(c) {
                        if already.insert(phys.0) {
                            used.push(phys);
                        }
                        specific_claimed += 1;
                    } else if is_generic_gp_constraint(c) {
                        generic_gp_count += 1;
                    }
                }
                for (idx, (constraint, _, _)) in inputs.iter().enumerate() {
                    let c = constraint.trim_start_matches(['=', '+', '&', '%']);
                    if let Some(phys) = constraint_to_phys(c) {
                        if already.insert(phys.0) {
                            used.push(phys);
                        }
                        specific_claimed += 1;
                    } else if idx >= num_plus && is_generic_gp_constraint(c) {
                        // Only count non-synthetic inputs (synthetic "+" inputs
                        // don't consume scratch registers)
                        generic_gp_count += 1;
                    }
                }
                for clobber in clobbers {
                    if let Some(phys) = clobber_to_phys(clobber.as_str()) {
                        if already.insert(phys.0) {
                            used.push(phys);
                        }
                        clobber_claimed += 1;
                    }
                }
                // If generic GP operands exceed the available caller-saved scratch
                // pool (after accounting for specific/clobber claims), the scratch
                // allocator will overflow into callee-saved registers.
                let available_scratch =
                    caller_saved_scratch_count.saturating_sub(specific_claimed + clobber_claimed);
                if generic_gp_count > available_scratch {
                    for &phys in all_callee_saved {
                        if already.insert(phys.0) {
                            used.push(phys);
                        }
                    }
                }
            }
        }
    }
}

/// Like `collect_inline_asm_callee_saved`, but with an additional
/// `all_callee_saved` list. When a constraint is a generic GP register class
/// (like "r", "q", "g") that doesn't map to a specific callee-saved register,
/// all registers in `all_callee_saved` are conservatively marked as clobbered.
pub fn collect_inline_asm_callee_saved_with_generic(
    func: &IrFunction,
    used: &mut Vec<PhysReg>,
    constraint_to_phys: impl Fn(&str) -> Option<PhysReg>,
    clobber_to_phys: impl Fn(&str) -> Option<PhysReg>,
    all_callee_saved: &[PhysReg],
) {
    collect_inline_asm_callee_saved_inner(
        func,
        used,
        constraint_to_phys,
        clobber_to_phys,
        all_callee_saved,
    )
}

/// Returns true if the constraint string (after stripping `=`, `+`, `&`)
/// contains a generic GP register class character that could cause the scratch
/// allocator to pick any GP register, including callee-saved ones.
fn is_generic_gp_constraint(constraint: &str) -> bool {
    // Skip explicit register constraints like {eax}
    if constraint.starts_with('{') {
        return false;
    }
    // Skip tied operands (all digits)
    if !constraint.is_empty() && constraint.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    // Skip condition code constraints (@cc...)
    if constraint.starts_with("@cc") {
        return false;
    }
    // Check for generic GP register class characters
    for ch in constraint.chars() {
        match ch {
            // 'r', 'q', 'R', 'Q', 'l' = any GP register
            // 'g' = general operand (GP reg, memory, or immediate)
            'r' | 'q' | 'R' | 'Q' | 'l' | 'g' => return true,
            // Specific register letters are not generic
            'a' | 'b' | 'c' | 'd' | 'S' | 'D' => return false,
            _ => {}
        }
    }
    false
}

fn collect_inline_asm_callee_saved_inner(
    func: &IrFunction,
    used: &mut Vec<PhysReg>,
    constraint_to_phys: impl Fn(&str) -> Option<PhysReg>,
    clobber_to_phys: impl Fn(&str) -> Option<PhysReg>,
    all_callee_saved: &[PhysReg],
) {
    let mut already: FxHashSet<u8> = used.iter().map(|r| r.0).collect();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::InlineAsm {
                outputs,
                inputs,
                clobbers,
                ..
            } = inst
            {
                for (constraint, _, _) in outputs {
                    let c = constraint.trim_start_matches(['=', '+', '&', '%']);
                    if let Some(phys) = constraint_to_phys(c) {
                        if already.insert(phys.0) {
                            used.push(phys);
                        }
                    } else if !all_callee_saved.is_empty() && is_generic_gp_constraint(c) {
                        // Generic register class: conservatively mark all
                        // callee-saved registers as clobbered since the
                        // scratch allocator may pick any of them.
                        for &phys in all_callee_saved {
                            if already.insert(phys.0) {
                                used.push(phys);
                            }
                        }
                    }
                }
                for (constraint, _, _) in inputs {
                    let c = constraint.trim_start_matches(['=', '+', '&', '%']);
                    if let Some(phys) = constraint_to_phys(c) {
                        if already.insert(phys.0) {
                            used.push(phys);
                        }
                    } else if !all_callee_saved.is_empty() && is_generic_gp_constraint(c) {
                        for &phys in all_callee_saved {
                            if already.insert(phys.0) {
                                used.push(phys);
                            }
                        }
                    }
                }
                for clobber in clobbers {
                    if let Some(phys) = clobber_to_phys(clobber.as_str()) {
                        if already.insert(phys.0) {
                            used.push(phys);
                        }
                    }
                }
            }
        }
    }
}
