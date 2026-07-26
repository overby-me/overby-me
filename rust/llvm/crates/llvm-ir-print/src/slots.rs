//! Slot numbering.
//!
//! Values without a name print as a number. Upstream assigns those numbers
//! from one counter per function, walking unnamed arguments first and then
//! each block and each value-producing instruction in program order, so a
//! block and an instruction can never share a number. Reproducing that
//! walk exactly is what makes `%13` in our output mean the same thing as
//! `%13` in upstream's.
//!
//! Module-scope symbols need none of this: a global written `@0` parses to a
//! numbered name and prints back as the number it came with.

use std::collections::HashMap;

use llvm_ir::function::Function;
use llvm_ir::{BlockId, InstId, Module, TypeKind};

/// Numbers for everything unnamed in one function.
#[derive(Debug, Default)]
pub struct FunctionSlots {
    arguments: HashMap<u32, u32>,
    blocks: HashMap<BlockId, u32>,
    instructions: HashMap<InstId, u32>,
}

impl FunctionSlots {
    pub fn compute(module: &Module, function: &Function) -> FunctionSlots {
        let mut slots = FunctionSlots::default();
        let mut next = 0u32;

        for (index, param) in function.params.iter().enumerate() {
            if param.name.is_none() {
                slots.arguments.insert(index as u32, next);
                next += 1;
            }
        }

        for (id, block) in function.blocks() {
            if block.name.is_none() {
                slots.blocks.insert(id, next);
                next += 1;
            }
            for (inst_id, instruction) in function.block_instructions(id) {
                let produces_value =
                    !matches!(module.ctx.type_kind(instruction.ty), TypeKind::Void);
                if produces_value && instruction.name.is_none() {
                    slots.instructions.insert(inst_id, next);
                    next += 1;
                }
            }
        }

        slots
    }

    pub fn argument(&self, index: u32) -> Option<u32> {
        self.arguments.get(&index).copied()
    }

    pub fn block(&self, id: BlockId) -> Option<u32> {
        self.blocks.get(&id).copied()
    }

    pub fn instruction(&self, id: InstId) -> Option<u32> {
        self.instructions.get(&id).copied()
    }
}
