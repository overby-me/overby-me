//! Stack layout: slot assignment, alloca coalescing, and regalloc helpers.
//!
//! ## Architecture
//!
//! Stack space calculation uses a three-tier allocation scheme:
//!
//! - **Tier 1**: Allocas get permanent, non-shared slots (addressable memory).
//!   Exception: non-escaping single-block allocas use Tier 3 sharing.
//!
//! - **Tier 2**: Multi-block non-alloca SSA temporaries use liveness-based packing.
//!   Values with non-overlapping live intervals share the same stack slot,
//!   using a greedy interval coloring algorithm via a min-heap.
//!
//! - **Tier 3**: Single-block non-alloca values use block-local coalescing with
//!   intra-block greedy slot reuse. Each block has its own pool; pools from
//!   different blocks overlap since only one block executes at a time.
//!
//! ## Submodules
//!
//! - `analysis`: value use-block maps, used-value collection, dead param detection
//! - `alloca_coalescing`: escape analysis for alloca coalescability
//! - `copy_coalescing`: copy alias maps and immediately-consumed value analysis
//! - `slot_assignment`: tier classification and slot assignment (Phases 2-7)
//! - `inline_asm`: shared ASM clobber scan
//! - `regalloc_helpers`: register allocator + clobber merge, callee-saved filtering
//!
//! ## Key functions
//!
//! - `calculate_stack_space_common`: orchestrates the three-tier allocation
//! - `compute_coalescable_allocas`: escape analysis for alloca coalescing
//! - `collect_inline_asm_callee_saved`: shared ASM clobber scan
//! - `run_regalloc_and_merge_clobbers`: register allocator + clobber merge
//! - `filter_available_regs`: callee-saved register filtering
//! - `find_param_alloca`: parameter alloca lookup

mod alloca_coalescing;
mod analysis;
mod copy_coalescing;
mod inline_asm;
mod regalloc_helpers;
mod slot_assignment;

// Re-export submodule public APIs at the stack_layout:: level
pub use inline_asm::{
    collect_inline_asm_callee_saved, collect_inline_asm_callee_saved_with_generic,
    collect_inline_asm_callee_saved_with_overflow,
};
pub use regalloc_helpers::{
    filter_available_regs, find_param_alloca, run_regalloc_and_merge_clobbers,
};

use super::regalloc::PhysReg;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrFunction};

use alloca_coalescing::CoalescableAllocas;

// ── Helper structs for slot allocation ────────────────────────────────────

/// A block-local slot whose final offset is deferred until after all tiers
/// have computed their space requirements. The final offset is:
/// `non_local_space + block_offset`.
struct DeferredSlot {
    dest_id: u32,
    size: i64,
    align: i64,
    block_offset: i64,
}

/// Multi-block value pending Tier 2 liveness-based packing.
struct MultiBlockValue {
    dest_id: u32,
    slot_size: i64,
}

/// Block-local non-alloca value pending Tier 3 intra-block reuse.
struct BlockLocalValue {
    dest_id: u32,
    slot_size: i64,
    block_idx: usize,
}

/// Intermediate state passed between phases of `calculate_stack_space_common`.
struct StackLayoutContext {
    /// Whether coalescing and multi-tier allocation is enabled (num_blocks >= 2).
    coalesce: bool,
    /// Per-value use-block map: value_id -> list of block indices where used.
    use_blocks_map: FxHashMap<u32, Vec<usize>>,
    /// Value ID -> defining block index.
    def_block: FxHashMap<u32, usize>,
    /// Values defined in multiple blocks (from phi elimination).
    multi_def_values: FxHashSet<u32>,
    /// Copy alias map: dest_id -> root_id (values sharing the same stack slot).
    copy_alias: FxHashMap<u32, u32>,
    /// All value IDs referenced as operands in the function body.
    used_values: FxHashSet<u32>,
    /// Dead parameter allocas (unused params, skip slot allocation).
    dead_param_allocas: FxHashSet<u32>,
    /// Alloca coalescing analysis results.
    coalescable_allocas: CoalescableAllocas,
    /// Values that are produced and immediately consumed by the next instruction,
    /// as the first operand loaded into the accumulator. These values don't need
    /// stack slots — the accumulator register cache keeps them alive.
    immediately_consumed: FxHashSet<u32>,
}

// ── Main stack space calculation ──────────────────────────────────────────

/// Shared stack space calculation: iterates over all instructions, assigns stack
/// slots for allocas and value results. Arch-specific offset direction is handled
/// by the `assign_slot` closure.
///
/// `initial_offset`: starting offset (e.g., 0 for x86, 16 for ARM/RISC-V to skip saved regs)
/// `assign_slot`: maps (current_space, raw_alloca_size, alignment) -> (slot_offset, new_space)
pub fn calculate_stack_space_common(
    state: &mut super::state::CodegenState,
    func: &IrFunction,
    initial_offset: i64,
    assign_slot: impl Fn(i64, i64, i64) -> (i64, i64),
    reg_assigned: &FxHashMap<u32, PhysReg>,
    callee_saved_regs: &[PhysReg],
    cached_liveness: Option<super::liveness::LivenessResult>,
    lhs_first_binop: bool,
) -> i64 {
    let num_blocks = func.blocks.len();

    // Enable coalescing and multi-tier allocation for any multi-block function.
    // Even small functions benefit: a 3-block function with 20 intermediates can
    // save 100+ bytes. Critical for recursive functions (PostgreSQL plpgsql) and
    // kernel functions with macro-expanded short-lived intermediates.
    let coalesce = num_blocks >= 2;

    // Phase 1: Build analysis context (use-blocks, def-blocks, used values,
    //          dead param allocas, alloca coalescability, copy aliases).
    let ctx = build_layout_context(
        func,
        coalesce,
        reg_assigned,
        callee_saved_regs,
        lhs_first_binop,
    );

    // Tell CodegenState which values are register-assigned so that
    // resolve_slot_addr can return a dummy Indirect slot for them.
    state.reg_assigned_values = reg_assigned.keys().copied().collect();

    // Phase 2: Classify all instructions into the three tiers.
    let mut non_local_space = initial_offset;
    let mut deferred_slots: Vec<DeferredSlot> = Vec::new();
    let mut multi_block_values: Vec<MultiBlockValue> = Vec::new();
    let mut block_local_values: Vec<BlockLocalValue> = Vec::new();
    let mut block_space: FxHashMap<usize, i64> = FxHashMap::default();
    let mut max_block_local_space: i64 = 0;

    slot_assignment::classify_instructions(
        state,
        func,
        &ctx,
        &assign_slot,
        reg_assigned,
        &mut non_local_space,
        &mut deferred_slots,
        &mut multi_block_values,
        &mut block_local_values,
        &mut block_space,
        &mut max_block_local_space,
    );

    // Phase 3: Tier 3 — block-local greedy slot reuse.
    slot_assignment::assign_tier3_block_local_slots(
        func,
        &ctx,
        coalesce,
        &block_local_values,
        &mut deferred_slots,
        &mut block_space,
        &mut max_block_local_space,
        &assign_slot,
    );

    // Phase 4: Tier 2 — liveness-based packing for multi-block values.
    slot_assignment::assign_tier2_liveness_packed_slots(
        state,
        coalesce,
        cached_liveness,
        func,
        &multi_block_values,
        &mut non_local_space,
        &assign_slot,
    );

    // Phase 5: Finalize deferred block-local slots by adding the global base offset.
    let total_space = slot_assignment::finalize_deferred_slots(
        state,
        &deferred_slots,
        non_local_space,
        max_block_local_space,
        &assign_slot,
    );

    // Phase 6: Resolve copy aliases (propagate slots from root to aliased values).
    slot_assignment::resolve_copy_aliases(state, &ctx.copy_alias);

    // Phase 7: Propagate wide-value status through Copy chains (32-bit targets only).
    slot_assignment::propagate_wide_values(state, func, &ctx.copy_alias);

    total_space
}

// ── Phase 1: Build analysis context ───────────────────────────────────────

/// Build all the analysis data needed by the three-tier slot allocator.
/// This includes use-block maps, definition tracking, copy coalescing analysis,
/// dead param detection, and alloca coalescability.
fn build_layout_context(
    func: &IrFunction,
    coalesce: bool,
    reg_assigned: &FxHashMap<u32, PhysReg>,
    callee_saved_regs: &[PhysReg],
    lhs_first_binop: bool,
) -> StackLayoutContext {
    // Build use-block map
    let mut use_blocks_map = if coalesce {
        analysis::compute_value_use_blocks(func)
    } else {
        FxHashMap::default()
    };

    // Build def-block map and identify multi-definition values (phi elimination).
    let mut def_block: FxHashMap<u32, usize> = FxHashMap::default();
    let mut multi_def_values: FxHashSet<u32> = FxHashSet::default();
    if coalesce {
        for (block_idx, block) in func.blocks.iter().enumerate() {
            for inst in &block.instructions {
                if let Some(dest) = inst.dest() {
                    if let Some(&prev_blk) = def_block.get(&dest.0) {
                        if prev_blk != block_idx {
                            multi_def_values.insert(dest.0);
                        }
                    }
                    def_block.insert(dest.0, block_idx);
                }
            }
        }
    }

    // Collect all Value IDs referenced as operands (for dead value/param detection).
    let used_values = analysis::collect_used_values(func);

    // Detect dead parameter allocas.
    let dead_param_allocas =
        analysis::find_dead_param_allocas(func, &used_values, reg_assigned, callee_saved_regs);

    // Alloca coalescability analysis.
    let coalescable_allocas = if coalesce {
        alloca_coalescing::compute_coalescable_allocas(
            func,
            &dead_param_allocas,
            &func.param_alloca_values,
        )
    } else {
        CoalescableAllocas {
            single_block: FxHashMap::default(),
            dead: FxHashSet::default(),
        }
    };

    // Copy coalescing analysis.
    let copy_alias = copy_coalescing::build_copy_alias_map(
        func,
        &def_block,
        &multi_def_values,
        reg_assigned,
        &use_blocks_map,
    );

    // Immediately-consumed value analysis: identify values that can skip stack slots.
    let immediately_consumed = copy_coalescing::compute_immediately_consumed(func, lhs_first_binop);

    // Propagate copy-alias uses into use_blocks_map so that root values account
    // for their aliases' use sites when deciding block-local vs. multi-block.
    if coalesce && !copy_alias.is_empty() {
        for (&dest_id, &root_id) in &copy_alias {
            if let Some(dest_blocks) = use_blocks_map.get(&dest_id).cloned() {
                let root_blocks = use_blocks_map.entry(root_id).or_insert_with(Vec::new);
                for blk in dest_blocks {
                    if root_blocks.last() != Some(&blk) {
                        root_blocks.push(blk);
                    }
                }
            }
        }
    }

    // F128 load pointer promotion: when an F128 Load uses a non-alloca pointer,
    // the codegen records that pointer as the reload source for the full-precision
    // 128-bit value. If the loaded F128 dest is used in other blocks, the pointer
    // must remain accessible during those blocks' codegen. Without this, the
    // pointer stays block-local (Tier 3) and its slot gets reused by other
    // blocks' local values, causing the F128 reload to dereference garbage.
    //
    // Fix: propagate the F128 dest's use-blocks into the pointer's use-blocks,
    // forcing the pointer to Tier 2 (multi-block) when the dest crosses blocks.
    if coalesce {
        // Collect alloca value IDs to distinguish direct vs. indirect sources.
        let alloca_set: FxHashSet<u32> = func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter_map(|inst| {
                if let Instruction::Alloca { dest, .. } = inst {
                    Some(dest.0)
                } else {
                    None
                }
            })
            .collect();

        // Collect (ptr_id, dest_id) pairs for F128 loads from non-alloca pointers.
        let f128_loads: Vec<(u32, u32)> = func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter_map(|inst| {
                if let Instruction::Load { dest, ptr, ty, .. } = inst {
                    if *ty == IrType::F128 && !alloca_set.contains(&ptr.0) {
                        return Some((ptr.0, dest.0));
                    }
                }
                None
            })
            .collect();

        for (ptr_id, dest_id) in f128_loads {
            if let Some(dest_blocks) = use_blocks_map.get(&dest_id).cloned() {
                let ptr_blocks = use_blocks_map.entry(ptr_id).or_insert_with(Vec::new);
                for blk in dest_blocks {
                    if !ptr_blocks.contains(&blk) {
                        ptr_blocks.push(blk);
                    }
                }
            }
        }
    }

    StackLayoutContext {
        coalesce,
        use_blocks_map,
        def_block,
        multi_def_values,
        copy_alias,
        used_values,
        dead_param_allocas,
        coalescable_allocas,
        immediately_consumed,
    }
}
