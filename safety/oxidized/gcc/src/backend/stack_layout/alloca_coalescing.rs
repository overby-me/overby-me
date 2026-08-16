//! Alloca escape analysis and coalescability classification.
//!
//! Determines which allocas can share stack slots via block-local coalescing
//! (Tier 3) by performing an escape analysis: allocas whose addresses never
//! leak (through calls, casts, phi nodes, terminators, etc.) and are used
//! in only a single block can safely share stack space with other block-local
//! values.

use crate::backend::liveness::for_each_operand_in_terminator;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{Instruction, IrFunction, Operand, Terminator, Value};

/// Result of alloca coalescability analysis.
pub(super) struct CoalescableAllocas {
    /// Single-block allocas: alloca ID -> the single block index where it's used.
    /// These can use block-local coalescing (Tier 3).
    pub(super) single_block: FxHashMap<u32, usize>,
    /// Dead non-param allocas: alloca IDs with no uses at all.
    /// These can be skipped entirely (no stack slot needed).
    pub(super) dead: FxHashSet<u32>,
}

/// Escape analysis state for alloca coalescability computation.
struct AllocaEscapeAnalysis {
    alloca_set: FxHashSet<u32>,
    gep_to_alloca: FxHashMap<u32, u32>,
    escaped: FxHashSet<u32>,
    use_blocks: FxHashMap<u32, Vec<usize>>,
}

impl AllocaEscapeAnalysis {
    fn new(
        func: &IrFunction,
        dead_param_allocas: &FxHashSet<u32>,
        param_alloca_values: &[Value],
    ) -> Self {
        let param_set: FxHashSet<u32> = param_alloca_values.iter().map(|v| v.0).collect();
        let mut alloca_set: FxHashSet<u32> = FxHashSet::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Alloca { dest, .. } = inst {
                    if !dead_param_allocas.contains(&dest.0) && !param_set.contains(&dest.0) {
                        alloca_set.insert(dest.0);
                    }
                }
            }
        }
        Self {
            alloca_set,
            gep_to_alloca: FxHashMap::default(),
            escaped: FxHashSet::default(),
            use_blocks: FxHashMap::default(),
        }
    }

    /// Resolve a value to its root alloca, if any.
    fn resolve_root(&self, val_id: u32) -> Option<u32> {
        if self.alloca_set.contains(&val_id) {
            Some(val_id)
        } else {
            self.gep_to_alloca.get(&val_id).copied()
        }
    }

    /// Record a use of an alloca (or GEP-derived value) in a block.
    fn record_use(&mut self, val_id: u32, block_idx: usize) {
        if let Some(root) = self.resolve_root(val_id) {
            let blocks = self.use_blocks.entry(root).or_default();
            if blocks.last() != Some(&block_idx) {
                blocks.push(block_idx);
            }
        }
    }

    /// Mark an alloca (or GEP-derived value) as escaped and record the use.
    fn mark_escaped(&mut self, val_id: u32, block_idx: usize) {
        if let Some(root) = self.resolve_root(val_id) {
            self.escaped.insert(root);
            let blocks = self.use_blocks.entry(root).or_default();
            if blocks.last() != Some(&block_idx) {
                blocks.push(block_idx);
            }
        }
    }

    /// Mark an operand's value as escaped (no use-block recording).
    fn mark_operand_escaped(&mut self, op: &Operand) {
        if let Operand::Value(v) = op {
            if let Some(root) = self.resolve_root(v.0) {
                self.escaped.insert(root);
            }
        }
    }

    /// Mark a direct value as escaped (for va_list ptrs etc).
    fn mark_direct_escaped(&mut self, val_id: u32) {
        if self.alloca_set.contains(&val_id) {
            self.escaped.insert(val_id);
        }
    }

    /// Scan all instructions and terminators for escape conditions and use sites.
    fn scan_instructions(&mut self, func: &IrFunction) {
        for (block_idx, block) in func.blocks.iter().enumerate() {
            for inst in &block.instructions {
                self.scan_gep(inst, block_idx);
                self.scan_instruction(inst, block_idx);
            }
            self.scan_terminator(&block.terminator);
        }
    }

    fn scan_gep(&mut self, inst: &Instruction, block_idx: usize) {
        if let Instruction::GetElementPtr { dest, base, .. } = inst {
            if let Some(root_alloca) = self.resolve_root(base.0) {
                self.gep_to_alloca.insert(dest.0, root_alloca);
                self.record_use(dest.0, block_idx);
            }
        }
    }

    fn scan_instruction(&mut self, inst: &Instruction, block_idx: usize) {
        match inst {
            Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                for arg in &info.args {
                    if let Operand::Value(v) = arg {
                        self.mark_escaped(v.0, block_idx);
                    }
                }
            }
            Instruction::Store { val, ptr, .. } => {
                self.mark_operand_escaped(val);
                self.record_use(ptr.0, block_idx);
            }
            Instruction::Load { ptr, .. } => {
                self.record_use(ptr.0, block_idx);
            }
            Instruction::Memcpy { dest, src, .. } => {
                for v in [dest, src] {
                    self.record_use(v.0, block_idx);
                }
            }
            Instruction::InlineAsm {
                outputs, inputs, ..
            } => {
                for (_, v, _) in outputs {
                    self.mark_escaped(v.0, block_idx);
                }
                for (_, op, _) in inputs {
                    if let Operand::Value(v) = op {
                        self.mark_escaped(v.0, block_idx);
                    }
                }
            }
            Instruction::Intrinsic { dest_ptr, args, .. } => {
                if let Some(dp) = dest_ptr {
                    self.record_use(dp.0, block_idx);
                }
                for arg in args {
                    if let Operand::Value(v) = arg {
                        self.record_use(v.0, block_idx);
                    }
                }
            }
            Instruction::AtomicRmw { ptr, val, .. } => {
                self.mark_operand_escaped(val);
                if let Operand::Value(v) = ptr {
                    self.record_use(v.0, block_idx);
                }
            }
            Instruction::AtomicCmpxchg {
                ptr,
                expected,
                desired,
                ..
            } => {
                for op in [expected, desired] {
                    self.mark_operand_escaped(op);
                }
                if let Operand::Value(v) = ptr {
                    self.record_use(v.0, block_idx);
                }
            }
            Instruction::AtomicLoad {
                ptr: Operand::Value(v),
                ..
            } => {
                self.record_use(v.0, block_idx);
            }
            Instruction::AtomicStore { ptr, val, .. } => {
                self.mark_operand_escaped(val);
                if let Operand::Value(v) = ptr {
                    self.record_use(v.0, block_idx);
                }
            }
            Instruction::VaStart { va_list_ptr }
            | Instruction::VaEnd { va_list_ptr }
            | Instruction::VaArg { va_list_ptr, .. } => {
                self.mark_direct_escaped(va_list_ptr.0);
            }
            Instruction::VaCopy { dest_ptr, src_ptr } => {
                self.mark_direct_escaped(dest_ptr.0);
                self.mark_direct_escaped(src_ptr.0);
            }
            Instruction::VaArgStruct {
                dest_ptr,
                va_list_ptr,
                ..
            } => {
                self.mark_direct_escaped(dest_ptr.0);
                self.mark_direct_escaped(va_list_ptr.0);
            }
            Instruction::Cast { src, .. }
            | Instruction::Copy { src, .. }
            | Instruction::UnaryOp { src, .. } => {
                self.mark_operand_escaped(src);
            }
            Instruction::BinOp { lhs, rhs, .. } | Instruction::Cmp { lhs, rhs, .. } => {
                for op in [lhs, rhs] {
                    self.mark_operand_escaped(op);
                }
            }
            Instruction::Select {
                cond,
                true_val,
                false_val,
                ..
            } => {
                for op in [cond, true_val, false_val] {
                    self.mark_operand_escaped(op);
                }
            }
            Instruction::Phi { incoming, .. } => {
                for (op, _) in incoming {
                    self.mark_operand_escaped(op);
                }
            }
            _ => {}
        }
    }

    fn scan_terminator(&mut self, terminator: &Terminator) {
        for_each_operand_in_terminator(terminator, |op| {
            if let Operand::Value(v) = op {
                if let Some(root) = self.resolve_root(v.0) {
                    self.escaped.insert(root);
                }
            }
        });
    }

    /// Classify non-escaped allocas into single-block and dead.
    fn into_result(self) -> CoalescableAllocas {
        let mut single_block: FxHashMap<u32, usize> = FxHashMap::default();
        let mut dead: FxHashSet<u32> = FxHashSet::default();
        for &alloca_id in &self.alloca_set {
            if self.escaped.contains(&alloca_id) {
                continue;
            }
            if let Some(blocks) = self.use_blocks.get(&alloca_id) {
                let mut unique: Vec<usize> = blocks.clone();
                unique.sort_unstable();
                unique.dedup();
                if unique.len() == 1 {
                    single_block.insert(alloca_id, unique[0]);
                }
                // Multi-block allocas: not coalesced, get permanent slots.
            } else {
                dead.insert(alloca_id);
            }
        }
        CoalescableAllocas { single_block, dead }
    }
}

/// Compute which allocas can be coalesced (either block-locally or via liveness packing).
///
/// An alloca is coalescable if its address never "escapes" (stored as a value,
/// used in casts/binops/phi/select, passed to calls, or used in terminators).
/// GEP chains are tracked transitively.
pub(super) fn compute_coalescable_allocas(
    func: &IrFunction,
    dead_param_allocas: &FxHashSet<u32>,
    param_alloca_values: &[Value],
) -> CoalescableAllocas {
    let mut analysis = AllocaEscapeAnalysis::new(func, dead_param_allocas, param_alloca_values);
    if analysis.alloca_set.is_empty() {
        return CoalescableAllocas {
            single_block: FxHashMap::default(),
            dead: FxHashSet::default(),
        };
    }
    analysis.scan_instructions(func);
    analysis.into_result()
}
