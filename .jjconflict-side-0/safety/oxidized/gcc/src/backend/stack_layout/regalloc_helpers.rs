//! Register allocation helpers and parameter alloca lookup.
//!
//! Shared utilities that eliminate duplicated regalloc setup boilerplate
//! across all four backends (x86-64, i686, AArch64, RISC-V 64).

use super::super::regalloc::PhysReg;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrFunction, Value};

// ── Register allocation helpers ───────────────────────────────────────────

/// Run register allocation and merge ASM-clobbered callee-saved registers.
///
/// This shared helper eliminates duplicated regalloc setup boilerplate across
/// all four backends (x86-64, i686, AArch64, RISC-V 64).  Each backend supplies its callee-saved
/// register list and pre-collected ASM clobber list; this function handles the
/// common steps: filtering available registers, running the allocator, storing
/// results, merging clobbers into `used_callee_saved`, and building the
/// `reg_assigned` set.
///
/// Returns `(reg_assigned, cached_liveness)` for use by `calculate_stack_space_common`.
pub fn run_regalloc_and_merge_clobbers(
    func: &IrFunction,
    available_regs: Vec<PhysReg>,
    caller_saved_regs: Vec<PhysReg>,
    asm_clobbered_regs: &[PhysReg],
    reg_assignments: &mut FxHashMap<u32, PhysReg>,
    used_callee_saved: &mut Vec<PhysReg>,
    allow_inline_asm_regalloc: bool,
) -> (
    FxHashMap<u32, PhysReg>,
    Option<super::super::liveness::LivenessResult>,
) {
    let config = super::super::regalloc::RegAllocConfig {
        available_regs,
        caller_saved_regs,
        allow_inline_asm_regalloc,
    };
    let alloc_result = super::super::regalloc::allocate_registers(func, &config);
    *reg_assignments = alloc_result.assignments;
    *used_callee_saved = alloc_result.used_regs;
    let cached_liveness = alloc_result.liveness;

    // Merge inline-asm clobbered callee-saved registers into the save/restore
    // list (they need to be preserved per the ABI even though we don't allocate
    // values to them).
    for phys in asm_clobbered_regs {
        if !used_callee_saved.iter().any(|r| r.0 == phys.0) {
            used_callee_saved.push(*phys);
        }
    }
    used_callee_saved.sort_by_key(|r| r.0);

    let reg_assigned: FxHashMap<u32, PhysReg> = reg_assignments.clone();
    (reg_assigned, cached_liveness)
}

/// Filter a callee-saved register list by removing ASM-clobbered entries.
/// Returns the filtered list suitable for passing to `run_regalloc_and_merge_clobbers`.
pub fn filter_available_regs(callee_saved: &[PhysReg], asm_clobbered: &[PhysReg]) -> Vec<PhysReg> {
    let mut available = callee_saved.to_vec();
    if !asm_clobbered.is_empty() {
        let clobbered_set: FxHashSet<u8> = asm_clobbered.iter().map(|r| r.0).collect();
        available.retain(|r| !clobbered_set.contains(&r.0));
    }
    available
}

// ── Utility ───────────────────────────────────────────────────────────────

/// Find the nth alloca instruction in the entry block (used for parameter storage).
pub fn find_param_alloca(func: &IrFunction, param_idx: usize) -> Option<(Value, IrType)> {
    func.blocks.first().and_then(|block| {
        block
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Alloca { .. }))
            .nth(param_idx)
            .and_then(|inst| {
                if let Instruction::Alloca { dest, ty, .. } = inst {
                    Some((*dest, *ty))
                } else {
                    None
                }
            })
    })
}
