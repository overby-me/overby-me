//! Slot numbering.
//!
//! Values without a name print as a number. Upstream assigns those numbers
//! from one counter per function, walking unnamed arguments first and then
//! each block and each value-producing instruction in program order, so a
//! block and an instruction can never share a number. Reproducing that
//! walk exactly is what makes `%13` in our output mean the same thing as
//! `%13` in upstream's.
//!
//! Module-scope symbols need the same treatment for a different reason. The
//! number a global is written with is an identity for the references to it
//! and not what it prints as: upstream renumbers every unnamed module symbol
//! from zero, so `@5` and `@7` come back as `@0` and `@1`. The order is by
//! kind rather than by definition, which was measured rather than assumed.

use std::collections::HashMap;

use llvm_ir::function::Function;
use llvm_ir::value::{AliasId, FunctionId, GlobalRef, GlobalVarId, IFuncId, Name};
use llvm_ir::{BlockId, InstId, Module, TypeKind};

/// Numbers for everything unnamed at module scope.
#[derive(Debug, Default)]
pub struct ModuleSlots {
    numbers: HashMap<GlobalRef, u32>,
}

impl ModuleSlots {
    pub fn compute(module: &Module) -> ModuleSlots {
        let mut slots = ModuleSlots::default();
        let mut next = 0u32;
        // Every unnamed global first, then the aliases, the ifuncs and the
        // functions, each in module order. A function written before the
        // globals still numbers after them.
        let kinds: [Box<dyn Iterator<Item = (GlobalRef, &Name)>>; 4] =
            [
                Box::new(module.globals.iter().enumerate().map(|(index, global)| {
                    (GlobalRef::Variable(GlobalVarId(index as u32)), &global.name)
                })),
                Box::new(
                    module.aliases.iter().enumerate().map(|(index, alias)| {
                        (GlobalRef::Alias(AliasId(index as u32)), &alias.name)
                    }),
                ),
                Box::new(
                    module.ifuncs.iter().enumerate().map(|(index, ifunc)| {
                        (GlobalRef::IFunc(IFuncId(index as u32)), &ifunc.name)
                    }),
                ),
                Box::new(
                    module
                        .functions
                        .iter()
                        .enumerate()
                        .map(|(index, function)| {
                            (
                                GlobalRef::Function(FunctionId(index as u32)),
                                &function.name,
                            )
                        }),
                ),
            ];
        for kind in kinds {
            for (id, name) in kind {
                if matches!(name, Name::Number(_)) {
                    slots.numbers.insert(id, next);
                    next += 1;
                }
            }
        }
        slots
    }

    pub fn get(&self, id: GlobalRef) -> Option<u32> {
        self.numbers.get(&id).copied()
    }
}

/// Numbers for everything unnamed in one function.
#[derive(Debug, Default)]
pub struct FunctionSlots {
    arguments: HashMap<u32, u32>,
    blocks: HashMap<BlockId, u32>,
    instructions: HashMap<InstId, u32>,
    /// What a named value prints as, for the one place that has to write a
    /// local without a function in hand: a value operand inside a debug
    /// record.
    argument_names: HashMap<u32, String>,
    instruction_names: HashMap<InstId, String>,
}

impl FunctionSlots {
    pub fn compute(module: &Module, function: &Function) -> FunctionSlots {
        let mut slots = FunctionSlots::default();
        let mut next = 0u32;

        for (index, param) in function.params.iter().enumerate() {
            match &param.name {
                None => {
                    slots.arguments.insert(index as u32, next);
                    next += 1;
                }
                Some(name) => {
                    slots
                        .argument_names
                        .insert(index as u32, crate::printer::name_text(name));
                }
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
                match &instruction.name {
                    None if produces_value => {
                        slots.instructions.insert(inst_id, next);
                        next += 1;
                    }
                    Some(name) => {
                        slots
                            .instruction_names
                            .insert(inst_id, crate::printer::name_text(name));
                    }
                    None => {}
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

    pub fn argument_name(&self, index: u32) -> Option<&str> {
        self.argument_names.get(&index).map(String::as_str)
    }

    pub fn instruction_name(&self, id: InstId) -> Option<&str> {
        self.instruction_names.get(&id).map(String::as_str)
    }
}
