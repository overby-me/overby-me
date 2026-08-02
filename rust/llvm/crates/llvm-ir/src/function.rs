//! Functions and basic blocks.
//!
//! A function owns two arenas, one of instructions and one of blocks, and a
//! block holds the ids of its instructions in order. PLAN.md section 4.2 asks
//! for intrusive use-lists, which is a different structure: def-use chains
//! have no consumer before the first analysis pass, and the property passes
//! actually need from an arena, that an id survives insertion and removal
//! elsewhere, holds either way.

use crate::attribute::AttributeSet;
use crate::constant::ConstId;
use crate::global::{ComdatRef, GlobalQualifiers};
use crate::instruction::{CallingConv, Instruction};
use crate::metadata::MdAttachment;
use crate::types::TypeId;
use crate::value::{BlockId, InstId, Name, Value};
use llvm_support::Align;

/// One formal parameter.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Param {
    pub ty: TypeId,
    pub attrs: AttributeSet,
    /// `None` on a declaration, or when the parameter is unnamed and prints
    /// as its slot number.
    pub name: Option<Name>,
}

/// A basic block: a label and the instructions under it, in order.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct BasicBlock {
    /// `None` when the block is unnamed and prints as its slot number.
    pub name: Option<Name>,
    pub instructions: Vec<InstId>,
}

impl BasicBlock {
    pub fn terminator(&self) -> Option<InstId> {
        self.instructions.last().copied()
    }
}

/// A function definition or declaration.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Function {
    pub name: Name,
    pub qualifiers: GlobalQualifiers,
    pub calling_conv: CallingConv,
    pub return_attrs: AttributeSet,
    pub return_type: TypeId,
    pub params: Vec<Param>,
    pub is_var_arg: bool,
    pub attrs: AttributeSet,
    pub section: Option<String>,
    pub partition: Option<String>,
    pub comdat: Option<ComdatRef>,
    pub align: Option<Align>,
    pub gc: Option<String>,
    pub prefix: Option<(TypeId, ConstId)>,
    pub prologue: Option<(TypeId, ConstId)>,
    pub personality: Option<(TypeId, ConstId)>,
    pub metadata: Vec<MdAttachment>,

    /// Instruction arena. A slot is `None` once its instruction is removed, so
    /// that every other `InstId` keeps meaning what it meant.
    instructions: Vec<Option<Instruction>>,
    blocks: Vec<BasicBlock>,
    /// Blocks in the order they print, which is the order they were parsed.
    pub block_order: Vec<BlockId>,
}

impl Function {
    pub fn new(name: Name, return_type: TypeId) -> Function {
        Function {
            name,
            qualifiers: GlobalQualifiers::default(),
            calling_conv: CallingConv::default(),
            return_attrs: AttributeSet::default(),
            return_type,
            params: Vec::new(),
            is_var_arg: false,
            attrs: AttributeSet::default(),
            section: None,
            partition: None,
            comdat: None,
            align: None,
            gc: None,
            prefix: None,
            prologue: None,
            personality: None,
            metadata: Vec::new(),
            instructions: Vec::new(),
            blocks: Vec::new(),
            block_order: Vec::new(),
        }
    }

    /// True when the function has a body.
    pub fn is_definition(&self) -> bool {
        !self.block_order.is_empty()
    }

    pub fn add_instruction(&mut self, instruction: Instruction) -> InstId {
        let id = InstId(self.instructions.len() as u32);
        self.instructions.push(Some(instruction));
        id
    }

    pub fn add_block(&mut self, block: BasicBlock) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(block);
        self.block_order.push(id);
        id
    }

    /// Allocates an instruction id whose instruction has not been read yet,
    /// which a parser needs for the value a loop-header phi names before the
    /// body defines it. Program order comes from the block, not the arena.
    pub fn reserve_instruction(&mut self) -> InstId {
        let id = InstId(self.instructions.len() as u32);
        self.instructions.push(None);
        id
    }

    /// Fills a slot from [`Function::reserve_instruction`].
    pub fn define_instruction(&mut self, id: InstId, instruction: Instruction) {
        self.instructions[id.0 as usize] = Some(instruction);
    }

    /// Allocates a block that is not in the printing order yet, for the same
    /// reason: `br label %later` names a block that comes later in the text.
    pub fn reserve_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(BasicBlock::default());
        id
    }

    /// Puts a reserved block into the printing order, at the end.
    pub fn place_block(&mut self, id: BlockId) {
        self.block_order.push(id);
    }

    pub fn instruction(&self, id: InstId) -> &Instruction {
        self.instructions[id.0 as usize]
            .as_ref()
            .expect("instruction was removed")
    }

    pub fn instruction_mut(&mut self, id: InstId) -> &mut Instruction {
        self.instructions[id.0 as usize]
            .as_mut()
            .expect("instruction was removed")
    }

    pub fn try_instruction(&self, id: InstId) -> Option<&Instruction> {
        self.instructions.get(id.0 as usize)?.as_ref()
    }

    pub fn block(&self, id: BlockId) -> &BasicBlock {
        &self.blocks[id.0 as usize]
    }

    pub fn block_mut(&mut self, id: BlockId) -> &mut BasicBlock {
        &mut self.blocks[id.0 as usize]
    }

    pub fn try_block(&self, id: BlockId) -> Option<&BasicBlock> {
        self.blocks.get(id.0 as usize)
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// The entry block, which is the first one in program order.
    pub fn entry_block(&self) -> Option<BlockId> {
        self.block_order.first().copied()
    }

    /// Detaches an instruction from its block and empties its arena slot. The
    /// id stays allocated so that no later instruction inherits it.
    pub fn remove_instruction(&mut self, block: BlockId, id: InstId) {
        self.block_mut(block).instructions.retain(|i| *i != id);
        self.instructions[id.0 as usize] = None;
    }

    /// Blocks in printing order.
    pub fn blocks(&self) -> impl Iterator<Item = (BlockId, &BasicBlock)> {
        self.block_order
            .iter()
            .map(move |id| (*id, &self.blocks[id.0 as usize]))
    }

    /// Every live instruction of a block, in order.
    pub fn block_instructions(&self, id: BlockId) -> impl Iterator<Item = (InstId, &Instruction)> {
        self.block(id)
            .instructions
            .iter()
            .filter_map(move |inst| Some((*inst, self.try_instruction(*inst)?)))
    }

    /// Every live instruction of the function, in printing order.
    pub fn instructions(&self) -> impl Iterator<Item = (InstId, &Instruction)> {
        self.blocks()
            .flat_map(move |(id, _)| self.block_instructions(id))
    }

    /// How many operand slots in this function read a value.
    ///
    /// One per slot rather than one per instruction, which is what upstream
    /// counts.
    pub fn value_uses(&self, target: Value) -> usize {
        self.instructions()
            .flat_map(|(_, instruction)| instruction.kind.use_count_values())
            .filter(|value| *value == target)
            .count()
    }

    /// How many terminator slots in this function name a block.
    ///
    /// A block's use list is not a value's. It holds the terminator operands
    /// that can transfer control to the block, one per slot, so
    /// `br i1 %c, label %b, label %b` uses `%b` twice. A phi's incoming
    /// blocks are not in it: upstream stores those beside the operand list
    /// rather than in it, so naming a block as an incoming edge is not a use
    /// of it. The other half of the list, the `blockaddress` constant that
    /// names the block, is a module-wide question and is counted there.
    pub fn block_uses(&self, block: BlockId) -> usize {
        self.instructions()
            .flat_map(|(_, instruction)| instruction.kind.successors())
            .filter(|successor| *successor == block)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruction::InstKind;
    use crate::types::TypeId;

    fn function() -> Function {
        Function::new(Name::named("f"), TypeId(0))
    }

    #[test]
    fn a_fresh_function_is_a_declaration() {
        let f = function();
        assert!(!f.is_definition());
        assert_eq!(f.entry_block(), None);
        assert_eq!(f.block_count(), 0);
    }

    #[test]
    fn blocks_and_instructions_keep_their_ids() {
        let mut f = function();
        let entry = f.add_block(BasicBlock::default());
        let first = f.add_instruction(Instruction::new(TypeId(0), InstKind::Unreachable));
        let second = f.add_instruction(Instruction::new(TypeId(0), InstKind::Unreachable));
        f.block_mut(entry).instructions = vec![first, second];

        assert!(f.is_definition());
        assert_eq!(f.entry_block(), Some(entry));
        assert_eq!(f.block(entry).terminator(), Some(second));

        // Removing the first instruction must not renumber the second.
        f.remove_instruction(entry, first);
        assert_eq!(f.block(entry).instructions, vec![second]);
        assert!(f.try_instruction(first).is_none());
        assert!(f.try_instruction(second).is_some());
        assert_eq!(f.block_instructions(entry).count(), 1);

        // A later instruction gets a fresh id rather than the freed one.
        let third = f.add_instruction(Instruction::new(TypeId(0), InstKind::Unreachable));
        assert_ne!(third, first);
    }

    /// Both arms of one branch are two entries, not one, and a phi naming
    /// the block as an incoming edge is not an entry at all. Measured
    /// against `llvm-as`, which takes a directive only when its index count
    /// matches the list.
    #[test]
    fn a_block_is_used_once_per_terminator_slot_that_names_it() {
        let mut f = function();
        let entry = f.add_block(BasicBlock::default());
        let target = f.add_block(BasicBlock::default());

        let branch = f.add_instruction(Instruction::new(
            TypeId(0),
            InstKind::CondBr {
                condition: Value::Argument(0),
                if_true: target,
                if_false: target,
            },
        ));
        f.block_mut(entry).instructions = vec![branch];

        let phi = f.add_instruction(Instruction::new(
            TypeId(0),
            InstKind::Phi {
                fast_math: Default::default(),
                incoming: vec![(Value::Argument(0), entry), (Value::Argument(1), target)],
            },
        ));
        f.block_mut(target).instructions = vec![phi];

        assert_eq!(f.block_uses(target), 2);
        // Nothing branches to the entry block; being entered is not a use.
        assert_eq!(f.block_uses(entry), 0);
        // The phi's own operands are values, and they are counted as such.
        assert_eq!(f.value_uses(Value::Argument(0)), 2);
        assert_eq!(f.value_uses(Value::Argument(1)), 1);
    }
}
