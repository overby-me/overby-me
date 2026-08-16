//! Value use-block analysis, used-value collection, and dead parameter alloca detection.
//!
//! These functions build the foundational data structures that drive the
//! three-tier slot allocation: which blocks each value is used in, which
//! values are referenced at all, and which parameter allocas are dead.

use crate::backend::liveness::{
    for_each_operand_in_instruction, for_each_operand_in_terminator,
    for_each_value_use_in_instruction,
};
use crate::backend::regalloc::PhysReg;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{Instruction, IrFunction, Operand};

/// Compute the "use-block map" for all values in the function.
/// For each value, records the set of block indices where that value is referenced.
///
/// Phi node operands are attributed to their SOURCE block (not the Phi's block).
/// This is critical for coalescing in dispatch-style code (switch/computed goto):
/// a value defined in block A that flows through a Phi to block B is semantically
/// "used at the end of block A" (where the branch resolves the Phi). Attributing
/// Phi uses to the source block allows such values to be considered block-local
/// to A, enabling stack slot coalescing across case handlers.
pub(super) fn compute_value_use_blocks(func: &IrFunction) -> FxHashMap<u32, Vec<usize>> {
    let mut uses: FxHashMap<u32, Vec<usize>> = FxHashMap::default();

    let record_use = |id: u32, block_idx: usize, uses: &mut FxHashMap<u32, Vec<usize>>| {
        let blocks = uses.entry(id).or_default();
        if blocks.last() != Some(&block_idx) {
            blocks.push(block_idx);
        }
    };

    // Build BlockId -> block_index map for Phi source block resolution.
    let block_id_to_idx: FxHashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(idx, b)| (b.label.0, idx))
        .collect();

    for (block_idx, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            // Handle Phi nodes specially: attribute each operand's use to its
            // source block rather than the block containing the Phi. This reflects
            // SSA semantics where Phi inputs are "evaluated" at the end of the
            // predecessor block, not at the start of the Phi's block.
            if let Instruction::Phi { incoming, .. } = inst {
                for (op, src_block_id) in incoming {
                    if let Operand::Value(v) = op {
                        if let Some(&src_idx) = block_id_to_idx.get(&src_block_id.0) {
                            record_use(v.0, src_idx, &mut uses);
                        } else {
                            // Fallback: if source block not found, attribute to Phi's block
                            record_use(v.0, block_idx, &mut uses);
                        }
                    }
                }
            } else {
                for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        record_use(v.0, block_idx, &mut uses);
                    }
                });
            }
            for_each_value_use_in_instruction(inst, |v| {
                record_use(v.0, block_idx, &mut uses);
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                record_use(v.0, block_idx, &mut uses);
            }
        });
    }
    uses
}

/// Collect all Value IDs referenced as operands anywhere in the function body.
pub(super) fn collect_used_values(func: &IrFunction) -> FxHashSet<u32> {
    let mut used: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    used.insert(v.0);
                }
            });
            for_each_value_use_in_instruction(inst, |v| {
                used.insert(v.0);
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                used.insert(v.0);
            }
        });
    }
    used
}

/// Find param allocas that can safely skip stack slot allocation.
///
/// A param alloca is dead when BOTH conditions hold:
/// 1. No instruction references the alloca value (mem2reg promoted it away)
/// 2. The corresponding ParamRef dest IS register-assigned, so emit_store_params
///    can store the ABI arg register directly to the callee-saved register
///
/// Without condition 2, the param value would have nowhere to live: the alloca
/// slot was eliminated, the ABI register may be clobbered, and no callee-saved
/// register was assigned. This would cause emit_param_ref to fall back to
/// reading the (already-clobbered) ABI register.
pub(super) fn find_dead_param_allocas(
    func: &IrFunction,
    used_values: &FxHashSet<u32>,
    reg_assigned: &FxHashMap<u32, PhysReg>,
    callee_saved_regs: &[PhysReg],
) -> FxHashSet<u32> {
    let mut dead = FxHashSet::default();
    if func.param_alloca_values.is_empty() {
        return dead;
    }

    // Build param_idx -> ParamRef dest Value map
    let mut paramref_dests: Vec<Option<u32>> = vec![None; func.param_alloca_values.len()];
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::ParamRef {
                dest, param_idx, ..
            } = inst
            {
                if *param_idx < paramref_dests.len() {
                    paramref_dests[*param_idx] = Some(dest.0);
                }
            }
        }
    }

    for (idx, pv) in func.param_alloca_values.iter().enumerate() {
        if !used_values.contains(&pv.0) {
            // Only eliminate the alloca if the ParamRef dest is assigned to a
            // callee-saved register. Caller-saved registers overlap with ABI
            // argument registers, so using them would clobber other params'
            // values before they're saved.
            if let Some(dest_id) = paramref_dests.get(idx).copied().flatten() {
                let is_callee_saved = reg_assigned
                    .get(&dest_id)
                    .map(|phys| callee_saved_regs.contains(phys))
                    .unwrap_or(false);
                if is_callee_saved {
                    dead.insert(pv.0);
                }
            }
        }
    }
    dead
}
