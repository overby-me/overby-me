//! Building IR.
//!
//! `docs/surface-inventory.md` counts what the rustc backend reaches for:
//! 194 of its 363 entry points are IR construction, and the great majority of
//! those are `LLVMBuild*`. This is the shape they take here, as safe Rust
//! rather than as a mirror of the C API.
//!
//! Two things the builder does that a caller should not have to: it works out
//! each instruction's result type from its operands, and it fills in the
//! alignment upstream would have computed. IR built here therefore verifies
//! and prints the same as IR that was parsed, which is what makes the smoke
//! test in `tests/` a real comparison rather than a self-consistent one.
//!
//! ```
//! use llvm_ir::Module;
//! use llvm_ir::builder::Builder;
//! use llvm_ir::instruction::BinOp;
//!
//! let mut module = Module::new();
//! let i32 = module.ctx.int_type(32);
//! let function = module.declare_function("double_it", i32, vec![i32], false);
//!
//! let mut builder = Builder::new(&mut module, function);
//! builder.append_block(Some("entry"));
//! let argument = builder.argument(0);
//! let doubled = builder.binary(BinOp::Add, argument, argument);
//! builder.ret(Some(doubled));
//! ```

use crate::attribute::Attribute;
use crate::constant::{CastOp, ConstId, Constant, GepFlags};
use crate::function::{BasicBlock, Function, Param};
use crate::global::{GlobalQualifiers, GlobalVariable, Linkage, UnnamedAddr};
use crate::instruction::{
    AtomicOrdering, BinOp, CallArg, CallData, FastMathFlags, InstKind, Instruction, IntFlags,
    IntPredicate, LandingPadClause, SyncScope,
};
use crate::metadata::{MdAttachment, MdOperand, MdRef, Metadata, NamedMetadata};
use crate::module::Module;
use crate::types::{TypeId, TypeKind};
use crate::value::{BlockId, FunctionId, GlobalRef, GlobalVarId, InstId, MdId, Name, Value};
use llvm_support::ApInt;

impl Module {
    /// Adds a function with no body, which is a `declare` until a block is
    /// appended to it.
    pub fn declare_function(
        &mut self,
        name: &str,
        result: TypeId,
        params: Vec<TypeId>,
        is_var_arg: bool,
    ) -> FunctionId {
        let mut function = Function::new(Name::named(name), result);
        function.is_var_arg = is_var_arg;
        function.params = params
            .into_iter()
            .map(|ty| Param {
                ty,
                attrs: crate::attribute::AttributeSet::default(),
                name: None,
            })
            .collect();
        self.add_function(function)
    }

    /// A private constant, which is what a string literal or a vtable
    /// becomes. Private linkage and `unnamed_addr` are what rustc emits for
    /// anything the program only reads through.
    pub fn add_private_constant(
        &mut self,
        name: &str,
        value_type: TypeId,
        initializer: ConstId,
    ) -> GlobalVarId {
        let align = self.default_align(value_type, false);
        self.add_global(GlobalVariable {
            name: Name::named(name),
            qualifiers: GlobalQualifiers {
                linkage: Some(Linkage::Private),
                unnamed_addr: Some(UnnamedAddr::Global),
                ..GlobalQualifiers::default()
            },
            externally_initialized: false,
            is_constant: true,
            value_type,
            initializer: Some(initializer),
            section: None,
            partition: None,
            comdat: None,
            align,
            metadata: Vec::new(),
            attrs: crate::attribute::AttributeSet::default(),
            code_model: None,
            sanitizer: crate::global::Sanitizers::default(),
        })
    }

    /// A `c"..."` byte string and the array type that holds it.
    pub fn const_string(&mut self, bytes: Vec<u8>) -> (TypeId, ConstId) {
        let byte = self.ctx.int_type(8);
        let ty = self.ctx.array_type(byte, bytes.len() as u64);
        let constant = self.ctx.intern_constant(Constant::String { ty, bytes });
        (ty, constant)
    }

    /// Attributes on a function, its return value, and its parameters. The
    /// backend sets these constantly: `docs/surface-inventory.md` counts 52
    /// call sites across 23 attribute entry points.
    pub fn add_function_attribute(&mut self, id: FunctionId, attribute: Attribute) {
        self.function_mut(id).attrs.push(attribute);
    }

    pub fn add_return_attribute(&mut self, id: FunctionId, attribute: Attribute) {
        self.function_mut(id).return_attrs.push(attribute);
    }

    pub fn add_param_attribute(&mut self, id: FunctionId, index: usize, attribute: Attribute) {
        self.function_mut(id).params[index].attrs.push(attribute);
    }

    /// The personality routine a function's landing pads dispatch through.
    pub fn set_personality(&mut self, id: FunctionId, personality: FunctionId) {
        let ty = self.ctx.pointer_type(0);
        let value = self.ctx.intern_constant(Constant::Global {
            ty,
            target: GlobalRef::Function(personality),
        });
        self.function_mut(id).personality = Some((ty, value));
    }

    /// A metadata string.
    ///
    /// This is an operand rather than a node: upstream has no syntax for a
    /// numbered string, so `!0 = !"text"` does not parse and a string only
    /// ever appears inside a node.
    pub fn md_string(&mut self, text: &str) -> MdOperand {
        MdOperand::String(text.into())
    }

    pub fn md_tuple(&mut self, operands: Vec<MdOperand>, distinct: bool) -> MdId {
        self.add_metadata(Metadata::Tuple { distinct, operands })
    }

    /// A tuple of typed integers, which is the shape of `!range` and of most
    /// module flags.
    pub fn md_ints(&mut self, bits: u32, values: &[i128]) -> MdId {
        let ty = self.ctx.int_type(bits);
        let operands = values
            .iter()
            .map(|value| MdOperand::Value {
                ty,
                value: Value::Constant(self.ctx.const_int_of(bits, *value)),
            })
            .collect();
        self.md_tuple(operands, false)
    }

    pub fn add_named_metadata(&mut self, name: &str, operands: Vec<MdId>) {
        self.named_metadata.push(NamedMetadata {
            name: name.into(),
            operands,
        });
    }

    /// The function type of a declared function, which a call needs and
    /// opaque pointers do not carry.
    pub fn function_type_of(&mut self, id: FunctionId) -> TypeId {
        let function = self.function(id);
        let result = function.return_type;
        let params: Vec<TypeId> = function.params.iter().map(|p| p.ty).collect();
        let is_var_arg = function.is_var_arg;
        self.ctx.function_type(result, params, is_var_arg)
    }
}

/// Appends instructions to one block of one function.
pub struct Builder<'m> {
    module: &'m mut Module,
    function: FunctionId,
    block: Option<BlockId>,
}

impl<'m> Builder<'m> {
    pub fn new(module: &'m mut Module, function: FunctionId) -> Builder<'m> {
        Builder {
            module,
            function,
            block: None,
        }
    }

    pub fn module(&mut self) -> &mut Module {
        self.module
    }

    /// Adds a block at the end of the function and moves the insertion point
    /// to it.
    pub fn append_block(&mut self, name: Option<&str>) -> BlockId {
        let block = BasicBlock {
            name: name.map(Name::named),
            instructions: Vec::new(),
        };
        let id = self.module.function_mut(self.function).add_block(block);
        self.block = Some(id);
        id
    }

    pub fn position_at_end(&mut self, block: BlockId) {
        self.block = Some(block);
    }

    pub fn current_block(&self) -> Option<BlockId> {
        self.block
    }

    pub fn argument(&self, index: u32) -> Value {
        Value::Argument(index)
    }

    /// The type of any value, which the builder needs to give an instruction
    /// its result type.
    pub fn type_of(&mut self, value: Value) -> TypeId {
        match value {
            Value::Constant(id) => self.module.ctx.constant(id).ty(),
            Value::Instruction(id) => self.module.function(self.function).instruction(id).ty,
            Value::Argument(index) => self.module.function(self.function).params[index as usize].ty,
            Value::Block(_) => self.module.ctx.label_type(),
            Value::Metadata(_) => self.module.ctx.metadata_type(),
        }
    }

    // ------------------------------------------------------------- constants

    pub fn int(&mut self, bits: u32, value: i128) -> Value {
        Value::Constant(self.module.ctx.const_int_of(bits, value))
    }

    pub fn bool(&mut self, value: bool) -> Value {
        Value::Constant(self.module.ctx.const_bool(value))
    }

    pub fn null(&mut self, ty: TypeId) -> Value {
        Value::Constant(self.module.ctx.const_null(ty))
    }

    pub fn undef(&mut self, ty: TypeId) -> Value {
        Value::Constant(self.module.ctx.const_undef(ty))
    }

    pub fn poison(&mut self, ty: TypeId) -> Value {
        Value::Constant(self.module.ctx.const_poison(ty))
    }

    /// A reference to a global-scope symbol, as a pointer.
    pub fn global_ref(&mut self, target: GlobalRef) -> Value {
        let address_space = self.module.global_address_space(target);
        let ty = self.module.ctx.pointer_type(address_space);
        Value::Constant(
            self.module
                .ctx
                .intern_constant(Constant::Global { ty, target }),
        )
    }

    // ---------------------------------------------------------- instructions

    /// Appends an instruction and gives back a reference to its result.
    fn append(&mut self, ty: TypeId, kind: InstKind) -> Value {
        let block = self
            .block
            .expect("the builder has no block; call append_block first");
        let function = self.module.function_mut(self.function);
        let id = function.add_instruction(Instruction::new(ty, kind));
        function.block_mut(block).instructions.push(id);
        Value::Instruction(id)
    }

    /// Names the value an instruction produced, so it prints as `%name`
    /// rather than as a slot number.
    pub fn name(&mut self, value: Value, name: &str) -> Value {
        if let Value::Instruction(id) = value {
            self.module
                .function_mut(self.function)
                .instruction_mut(id)
                .name = Some(Name::named(name));
        }
        value
    }

    pub fn binary(&mut self, op: BinOp, lhs: Value, rhs: Value) -> Value {
        let ty = self.type_of(lhs);
        self.append(
            ty,
            InstKind::Binary {
                op,
                flags: IntFlags::default(),
                fast_math: FastMathFlags::default(),
                lhs,
                rhs,
            },
        )
    }

    /// The same, with the no-wrap flags an optimizer would want.
    pub fn binary_with_flags(
        &mut self,
        op: BinOp,
        flags: IntFlags,
        lhs: Value,
        rhs: Value,
    ) -> Value {
        let ty = self.type_of(lhs);
        self.append(
            ty,
            InstKind::Binary {
                op,
                flags,
                fast_math: FastMathFlags::default(),
                lhs,
                rhs,
            },
        )
    }

    pub fn icmp(&mut self, predicate: IntPredicate, lhs: Value, rhs: Value) -> Value {
        let operand_type = self.type_of(lhs);
        let ty = match self.module.ctx.type_kind(operand_type) {
            TypeKind::Vector {
                count, scalable, ..
            } => {
                let (count, scalable) = (*count, *scalable);
                let bool_type = self.module.ctx.int_type(1);
                self.module.ctx.vector_type(bool_type, count, scalable)
            }
            _ => self.module.ctx.int_type(1),
        };
        self.append(
            ty,
            InstKind::ICmp {
                predicate,
                flags: IntFlags::default(),
                operand_type,
                lhs,
                rhs,
            },
        )
    }

    pub fn cast(&mut self, op: CastOp, value: Value, to: TypeId) -> Value {
        let source_type = self.type_of(value);
        self.append(
            to,
            InstKind::Cast {
                op,
                flags: IntFlags::default(),
                fast_math: FastMathFlags::default(),
                operand: value,
                source_type,
            },
        )
    }

    pub fn alloca(&mut self, allocated_type: TypeId) -> Value {
        let ty = self.module.ctx.pointer_type(0);
        let align = self.module.default_align(allocated_type, true);
        self.append(
            ty,
            InstKind::Alloca {
                allocated_type,
                count: None,
                align,
                address_space: None,
                inalloca: false,
                swifterror: false,
            },
        )
    }

    pub fn load(&mut self, loaded_type: TypeId, pointer: Value) -> Value {
        let align = self.module.default_align(loaded_type, false);
        self.append(
            loaded_type,
            InstKind::Load {
                loaded_type,
                pointer,
                volatile: false,
                atomic: None,
                align,
            },
        )
    }

    pub fn store(&mut self, value: Value, pointer: Value) {
        let value_type = self.type_of(value);
        let align = self.module.default_align(value_type, false);
        let void = self.module.ctx.void_type();
        self.append(
            void,
            InstKind::Store {
                value_type,
                value,
                pointer,
                volatile: false,
                atomic: None,
                align,
            },
        );
    }

    pub fn atomic_rmw(
        &mut self,
        op: crate::instruction::AtomicRmwOp,
        pointer: Value,
        value: Value,
        ordering: AtomicOrdering,
    ) -> Value {
        let value_type = self.type_of(value);
        let align = self.module.default_align(value_type, false);
        self.append(
            value_type,
            InstKind::AtomicRmw {
                op,
                pointer,
                value_type,
                value,
                volatile: false,
                scope: SyncScope::system(),
                ordering,
                align,
            },
        )
    }

    /// `getelementptr inbounds`, which is what a field or element access is.
    pub fn gep(&mut self, source_type: TypeId, pointer: Value, indices: Vec<Value>) -> Value {
        let pointer_type = self.type_of(pointer);
        let indices = indices
            .into_iter()
            .map(|index| (self.type_of(index), index))
            .collect();
        self.append(
            pointer_type,
            InstKind::GetElementPtr {
                source_type,
                pointer_type,
                pointer,
                indices,
                flags: GepFlags {
                    inbounds: true,
                    ..GepFlags::default()
                },
                inrange: None,
            },
        )
    }

    pub fn call(&mut self, callee: FunctionId, args: Vec<Value>) -> Value {
        let (result, call) = self.call_data(callee, args);
        self.append(result, InstKind::Call(Box::new(call)))
    }

    /// Everything `call` and `invoke` share: the callee's signature, the
    /// argument types, and the defaults for the rest.
    fn call_data(&mut self, callee: FunctionId, args: Vec<Value>) -> (TypeId, CallData) {
        let function_type = self.module.function_type_of(callee);
        let result = self.module.function(callee).return_type;
        let callee_value = self.global_ref(GlobalRef::Function(callee));
        let args = args
            .into_iter()
            .map(|value| CallArg {
                ty: self.type_of(value),
                attrs: crate::attribute::AttributeSet::default(),
                value,
            })
            .collect();
        (
            result,
            CallData {
                tail: crate::instruction::TailKind::None,
                fast_math: FastMathFlags::default(),
                calling_conv: crate::instruction::CallingConv::C,
                return_attrs: crate::attribute::AttributeSet::default(),
                function_type,
                address_space: None,
                callee: callee_value,
                args,
                fn_attrs: crate::attribute::AttributeSet::default(),
                bundles: Vec::new(),
            },
        )
    }

    pub fn phi(&mut self, ty: TypeId, incoming: Vec<(Value, BlockId)>) -> Value {
        self.append(
            ty,
            InstKind::Phi {
                fast_math: FastMathFlags::default(),
                incoming,
            },
        )
    }

    /// Adds an edge to a phi after the fact, which a loop header needs: the
    /// incoming value comes from a block built later.
    pub fn add_incoming(&mut self, phi: Value, value: Value, from: BlockId) {
        let Value::Instruction(id) = phi else {
            return;
        };
        if let InstKind::Phi { incoming, .. } = &mut self
            .module
            .function_mut(self.function)
            .instruction_mut(id)
            .kind
        {
            incoming.push((value, from));
        }
    }

    pub fn select(&mut self, condition: Value, if_true: Value, if_false: Value) -> Value {
        let condition_type = self.type_of(condition);
        let ty = self.type_of(if_true);
        self.append(
            ty,
            InstKind::Select {
                fast_math: FastMathFlags::default(),
                condition_type,
                condition,
                if_true,
                if_false,
            },
        )
    }

    pub fn ret(&mut self, value: Option<Value>) {
        let void = self.module.ctx.void_type();
        let returned = value.map(|value| {
            let ty = self.module.function(self.function).return_type;
            (ty, value)
        });
        self.append(void, InstKind::Ret(returned));
    }

    pub fn br(&mut self, target: BlockId) {
        let void = self.module.ctx.void_type();
        self.append(void, InstKind::Br { target });
    }

    pub fn cond_br(&mut self, condition: Value, if_true: BlockId, if_false: BlockId) {
        let void = self.module.ctx.void_type();
        self.append(
            void,
            InstKind::CondBr {
                condition,
                if_true,
                if_false,
            },
        );
    }

    /// A call that can unwind, which is what every call in a function with
    /// destructors becomes.
    pub fn invoke(
        &mut self,
        callee: FunctionId,
        args: Vec<Value>,
        normal: BlockId,
        unwind: BlockId,
    ) -> Value {
        let (result, call) = self.call_data(callee, args);
        self.append(
            result,
            InstKind::Invoke {
                call: Box::new(call),
                normal,
                unwind,
            },
        )
    }

    /// The landing pad an unwind edge arrives at. The type is the exception
    /// pair the personality routine hands back, `{ ptr, i32 }` for Rust.
    pub fn landing_pad(
        &mut self,
        ty: TypeId,
        cleanup: bool,
        clauses: Vec<LandingPadClause>,
    ) -> Value {
        self.append(ty, InstKind::LandingPad { cleanup, clauses })
    }

    pub fn resume(&mut self, value: Value) {
        let ty = self.type_of(value);
        let void = self.module.ctx.void_type();
        self.append(void, InstKind::Resume { ty, value });
    }

    pub fn switch(&mut self, value: Value, default: BlockId, cases: Vec<(Value, BlockId)>) {
        let value_type = self.type_of(value);
        let void = self.module.ctx.void_type();
        self.append(
            void,
            InstKind::Switch {
                value_type,
                value,
                default,
                cases,
            },
        );
    }

    pub fn extract_value(&mut self, aggregate: Value, indices: Vec<u32>) -> Value {
        let aggregate_type = self.type_of(aggregate);
        let ty = self.element_at(aggregate_type, &indices);
        self.append(
            ty,
            InstKind::ExtractValue {
                aggregate_type,
                aggregate,
                indices,
            },
        )
    }

    pub fn insert_value(&mut self, aggregate: Value, element: Value, indices: Vec<u32>) -> Value {
        let aggregate_type = self.type_of(aggregate);
        let element_type = self.type_of(element);
        self.append(
            aggregate_type,
            InstKind::InsertValue {
                aggregate_type,
                aggregate,
                element_type,
                element,
                indices,
            },
        )
    }

    pub fn freeze(&mut self, operand: Value) -> Value {
        let operand_type = self.type_of(operand);
        self.append(
            operand_type,
            InstKind::Freeze {
                operand_type,
                operand,
            },
        )
    }

    /// Attaches metadata to the instruction a value came from, which is how
    /// `!dbg`, `!range` and `!noalias` reach the IR.
    pub fn attach(&mut self, value: Value, kind: &str, node: MdId) {
        let Value::Instruction(id) = value else {
            return;
        };
        self.module
            .function_mut(self.function)
            .instruction_mut(id)
            .metadata
            .push(MdAttachment {
                kind: kind.into(),
                node: MdRef::Id(node),
            });
    }

    /// The type reached by walking an aggregate's index list.
    fn element_at(&mut self, aggregate: TypeId, indices: &[u32]) -> TypeId {
        let mut current = aggregate;
        for index in indices {
            current = match self.module.ctx.type_kind(current).clone() {
                TypeKind::Struct { fields, .. } => fields[*index as usize],
                TypeKind::NamedStruct(id) => {
                    let fields = self.module.ctx.struct_def(id).fields.clone();
                    fields.expect("an opaque struct has no elements")[*index as usize]
                }
                TypeKind::Array { element, .. } => element,
                _ => current,
            };
        }
        current
    }

    pub fn unreachable(&mut self) {
        let void = self.module.ctx.void_type();
        self.append(void, InstKind::Unreachable);
    }

    /// An integer constant of an arbitrary width, for the cases `int` cannot
    /// express.
    pub fn wide_int(&mut self, ty: TypeId, value: ApInt) -> Value {
        Value::Constant(self.module.ctx.const_int(ty, value))
    }

    /// The instruction an id refers to, for a caller inspecting what it built.
    pub fn instruction(&self, id: InstId) -> &Instruction {
        self.module.function(self.function).instruction(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_types_come_from_the_operands() {
        let mut module = Module::new();
        let i32 = module.ctx.int_type(32);
        let i64 = module.ctx.int_type(64);
        let function = module.declare_function("f", i32, vec![i32], false);

        let mut builder = Builder::new(&mut module, function);
        builder.append_block(Some("entry"));
        let argument = builder.argument(0);
        let sum = builder.binary(BinOp::Add, argument, argument);
        assert_eq!(builder.type_of(sum), i32, "add takes its operand's type");
        let widened = builder.cast(CastOp::SExt, sum, i64);
        assert_eq!(builder.type_of(widened), i64, "a cast takes its target");
        let compared = builder.icmp(IntPredicate::Slt, argument, argument);
        let bool_type = builder.module().ctx.int_type(1);
        assert_eq!(builder.type_of(compared), bool_type, "icmp produces i1");
        let slot = builder.alloca(i64);
        let pointer = builder.module().ctx.pointer_type(0);
        assert_eq!(builder.type_of(slot), pointer, "alloca produces a pointer");
    }

    #[test]
    fn alignment_is_filled_in_the_way_parsing_fills_it() {
        let mut module = Module::new();
        module.data_layout = Some(
            llvm_support::DataLayout::parse("e-m:e-i64:64-i128:128-n8:16:32:64-S128").unwrap(),
        );
        let i64 = module.ctx.int_type(64);
        let void = module.ctx.void_type();
        let function = module.declare_function("f", void, Vec::new(), false);

        let mut builder = Builder::new(&mut module, function);
        builder.append_block(Some("entry"));
        let slot = builder.alloca(i64);
        let loaded = builder.load(i64, slot);
        builder.ret(None);

        let Value::Instruction(id) = loaded else {
            panic!("load produces an instruction");
        };
        let InstKind::Load { align, .. } = builder.instruction(id).kind else {
            panic!("that instruction is a load");
        };
        assert_eq!(
            align.map(llvm_support::Align::bytes),
            Some(8),
            "an i64 load takes the ABI alignment the layout gives it"
        );
    }
}
