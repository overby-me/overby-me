//! The verifier.
//!
//! The contract every other crate develops against: a module that passes here
//! is well formed, and one that does not says why and where. The rules are
//! LangRef's, restricted to what this tier models, and each one exists because
//! something downstream would otherwise miscompile rather than crash.
//!
//! Verification collects every problem instead of stopping at the first, so a
//! broken pass reports its whole blast radius in one run.

use std::collections::{HashMap, HashSet};

use crate::constant::CastOp;
use crate::function::Function;
use crate::instruction::{BinOp, InstKind, IntFlags};
use crate::module::Module;
use crate::types::TypeKind;
use crate::value::{BlockId, GlobalRef, InstId, MdId, Name, Value};
use crate::{FunctionId, TypeId};

/// One thing wrong with a module.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VerifyError {
    /// The function it was found in, if it was found inside one.
    pub function: Option<String>,
    pub message: String,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.function {
            Some(function) => write!(f, "in @{function}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// Checks a module, returning every problem found.
pub fn verify_module(module: &Module) -> Vec<VerifyError> {
    let mut verifier = Verifier {
        module,
        errors: Vec::new(),
        function: None,
    };
    verifier.module_level();
    for index in 0..module.functions.len() {
        verifier.function(FunctionId(index as u32));
    }
    verifier.errors
}

struct Verifier<'m> {
    module: &'m Module,
    errors: Vec<VerifyError>,
    function: Option<String>,
}

impl Verifier<'_> {
    fn report(&mut self, message: impl Into<String>) {
        self.errors.push(VerifyError {
            function: self.function.clone(),
            message: message.into(),
        });
    }

    fn check(&mut self, condition: bool, message: impl Into<String>) {
        if !condition {
            self.report(message);
        }
    }

    fn metadata_exists(&mut self, id: MdId, what: &str) {
        if self.module.metadata_node(id).is_none() {
            self.report(format!("{what} refers to undefined metadata !{}", id.0));
        }
    }

    // ----------------------------------------------------------- module rules

    fn module_level(&mut self) {
        for index in 0..self.module.globals.len() {
            let global = &self.module.globals[index];
            let name = describe(&global.name);
            if let Some(initializer) = global.initializer {
                let actual = self.module.ctx.constant(initializer).ty();
                self.check(
                    actual == global.value_type,
                    format!("@{name} has an initialiser of the wrong type"),
                );
            }
            for attachment in &global.metadata {
                let node = attachment.node;
                self.metadata_exists(node, &format!("@{name}"));
            }
            for group in &global.attrs.groups {
                let group = *group;
                if self.module.attribute_group(group).is_none() {
                    self.report(format!(
                        "@{name} refers to undefined attribute group #{group}"
                    ));
                }
            }
        }

        for named in &self.module.named_metadata {
            let name = named.name.clone();
            for operand in named.operands.clone() {
                self.metadata_exists(operand, &format!("!{name}"));
            }
        }

        for (id, node) in self.module.metadata_nodes() {
            for referenced in referenced_metadata(node) {
                if self.module.metadata_node(referenced).is_none() {
                    self.report(format!(
                        "!{} refers to undefined metadata !{}",
                        id.0, referenced.0
                    ));
                }
            }
        }
    }

    // --------------------------------------------------------- function rules

    fn function(&mut self, id: FunctionId) {
        let function = self.module.function(id);
        self.function = Some(describe(&function.name));

        for attachment in &function.metadata {
            let node = attachment.node;
            self.metadata_exists(node, "the function");
        }
        for group in &function.attrs.groups {
            let group = *group;
            if self.module.attribute_group(group).is_none() {
                self.report(format!("refers to undefined attribute group #{group}"));
            }
        }

        if !function.is_definition() {
            self.function = None;
            return;
        }

        let blocks: Vec<BlockId> = function.block_order.clone();
        for block_id in &blocks {
            self.basic_block(function, *block_id);
        }
        self.dominance(function);
        self.function = None;
    }

    fn basic_block(&mut self, function: &Function, id: BlockId) {
        let block = function.block(id);
        let label = describe_block(function, id);
        if block.instructions.is_empty() {
            self.report(format!("block {label} is empty"));
            return;
        }

        let mut seen_non_phi = false;
        let count = block.instructions.len();
        for (position, inst_id) in block.instructions.clone().into_iter().enumerate() {
            let Some(instruction) = function.try_instruction(inst_id) else {
                self.report(format!(
                    "block {label} refers to an instruction that was removed"
                ));
                continue;
            };
            let last = position + 1 == count;
            if instruction.kind.is_terminator() != last {
                if last {
                    self.report(format!("block {label} does not end in a terminator"));
                } else {
                    self.report(format!(
                        "block {label} has a terminator ({}) before its end",
                        instruction.kind.opcode()
                    ));
                }
            }
            match &instruction.kind {
                InstKind::Phi { .. } => {
                    if seen_non_phi {
                        self.report(format!(
                            "block {label} has a phi after a non-phi instruction"
                        ));
                    }
                }
                InstKind::DebugRecord { .. } => {}
                _ => seen_non_phi = true,
            }
            for attachment in &instruction.metadata {
                let node = attachment.node;
                self.metadata_exists(node, &format!("an instruction in {label}"));
            }
            self.instruction(function, id, inst_id);
        }
    }

    fn instruction(&mut self, function: &Function, block: BlockId, id: InstId) {
        let instruction = function.instruction(id);
        let ty = instruction.ty;
        let where_ = format!(
            "{} in {}",
            instruction.kind.opcode(),
            describe_block(function, block)
        );

        for successor in instruction.kind.successors() {
            if function.try_block(successor).is_none() {
                self.report(format!("{where_} branches to a block outside the function"));
            }
        }

        match &instruction.kind {
            InstKind::Binary {
                op,
                flags,
                fast_math,
                lhs,
                rhs,
            } => {
                let (op, flags, fast_math) = (*op, *flags, *fast_math);
                let (lhs, rhs) = (*lhs, *rhs);
                self.same_type(function, ty, lhs, &where_);
                self.same_type(function, ty, rhs, &where_);
                if op.is_floating_point() {
                    self.check(
                        self.is_float_or_float_vector(ty),
                        format!("{where_} needs a floating-point type"),
                    );
                    self.check(
                        flags.is_empty(),
                        format!("{where_} cannot take integer flags"),
                    );
                } else {
                    self.check(
                        self.is_int_or_int_vector(ty),
                        format!("{where_} needs an integer type"),
                    );
                    self.check(
                        fast_math.is_empty(),
                        format!("{where_} cannot take fast-math flags"),
                    );
                    self.integer_flags(op, flags, &where_);
                }
            }
            InstKind::FNeg { operand, .. } => {
                let operand = *operand;
                self.same_type(function, ty, operand, &where_);
                self.check(
                    self.is_float_or_float_vector(ty),
                    format!("{where_} needs a floating-point type"),
                );
            }
            InstKind::Cast {
                op,
                flags,
                operand,
                source_type,
            } => {
                let (op, flags, source_type) = (*op, *flags, *source_type);
                let operand = *operand;
                self.type_is(function, source_type, operand, &where_);
                self.cast_flags(op, flags, &where_);
                self.cast_shape(op, source_type, ty, &where_);
            }
            InstKind::ICmp {
                operand_type,
                lhs,
                rhs,
                flags,
                ..
            } => {
                let (operand_type, flags) = (*operand_type, *flags);
                let (lhs, rhs) = (*lhs, *rhs);
                self.type_is(function, operand_type, lhs, &where_);
                self.type_is(function, operand_type, rhs, &where_);
                self.check(
                    self.is_int_or_int_vector(operand_type)
                        || self.is_pointer_or_pointer_vector(operand_type),
                    format!("{where_} compares something that is not an integer or a pointer"),
                );
                let allowed = IntFlags {
                    samesign: true,
                    ..IntFlags::default()
                };
                self.check(
                    subset_of(flags, allowed),
                    format!("{where_} takes only the samesign flag"),
                );
            }
            InstKind::FCmp {
                operand_type,
                lhs,
                rhs,
                ..
            } => {
                let operand_type = *operand_type;
                let (lhs, rhs) = (*lhs, *rhs);
                self.type_is(function, operand_type, lhs, &where_);
                self.type_is(function, operand_type, rhs, &where_);
                self.check(
                    self.is_float_or_float_vector(operand_type),
                    format!("{where_} compares something that is not floating point"),
                );
            }
            InstKind::Ret(returned) => match returned {
                None => self.check(
                    matches!(
                        self.module.ctx.type_kind(function.return_type),
                        TypeKind::Void
                    ),
                    format!("{where_} returns nothing from a function that returns a value"),
                ),
                Some((returned_type, value)) => {
                    let (returned_type, value) = (*returned_type, *value);
                    self.check(
                        returned_type == function.return_type,
                        format!("{where_} returns the wrong type"),
                    );
                    self.type_is(function, returned_type, value, &where_);
                }
            },
            InstKind::CondBr { condition, .. } => {
                let condition = *condition;
                let bool_type = self.bool_type();
                match bool_type {
                    Some(bool_type) => self.type_is(function, bool_type, condition, &where_),
                    None => self.check(false, format!("{where_} has no i1 type in the module")),
                }
            }
            InstKind::Switch {
                value_type,
                value,
                cases,
                ..
            } => {
                let value_type = *value_type;
                let value = *value;
                let cases = cases.clone();
                self.type_is(function, value_type, value, &where_);
                let mut seen = HashSet::new();
                for (case, _) in &cases {
                    self.type_is(function, value_type, *case, &where_);
                    if let Value::Constant(id) = case {
                        if !seen.insert(*id) {
                            self.report(format!("{where_} has a duplicated case"));
                        }
                    } else {
                        self.report(format!("{where_} has a case that is not a constant"));
                    }
                }
            }
            InstKind::Load {
                loaded_type, align, ..
            } => {
                let loaded_type = *loaded_type;
                self.check(
                    loaded_type == ty,
                    format!("{where_} produces something other than what it loads"),
                );
                self.check(align.is_some(), format!("{where_} has no alignment"));
            }
            InstKind::Store {
                value_type, value, ..
            } => {
                let (value_type, value) = (*value_type, *value);
                self.type_is(function, value_type, value, &where_);
            }
            InstKind::Alloca { .. } => self.check(
                matches!(self.module.ctx.type_kind(ty), TypeKind::Pointer { .. }),
                format!("{where_} does not produce a pointer"),
            ),
            InstKind::GetElementPtr {
                pointer_type,
                pointer,
                ..
            } => {
                let (pointer_type, pointer) = (*pointer_type, *pointer);
                self.type_is(function, pointer_type, pointer, &where_);
                self.check(
                    self.is_pointer_or_pointer_vector(pointer_type),
                    format!("{where_} indexes something that is not a pointer"),
                );
            }
            InstKind::Phi { incoming, .. } => {
                let incoming = incoming.clone();
                for (value, _) in &incoming {
                    self.type_is(function, ty, *value, &where_);
                }
                let expected: Vec<BlockId> = predecessors_of(function, block);
                let mut named: Vec<BlockId> = incoming.iter().map(|(_, b)| *b).collect();
                named.sort_by_key(|b| b.0);
                named.dedup();
                let mut wanted = expected.clone();
                wanted.sort_by_key(|b| b.0);
                wanted.dedup();
                self.check(
                    named == wanted,
                    format!("{where_} does not name exactly its block's predecessors"),
                );
            }
            InstKind::Select {
                condition_type,
                condition,
                if_true,
                if_false,
                ..
            } => {
                let (condition_type, condition) = (*condition_type, *condition);
                let (if_true, if_false) = (*if_true, *if_false);
                self.type_is(function, condition_type, condition, &where_);
                self.type_is(function, ty, if_true, &where_);
                self.type_is(function, ty, if_false, &where_);
            }
            InstKind::Freeze {
                operand_type,
                operand,
            } => {
                let (operand_type, operand) = (*operand_type, *operand);
                self.check(
                    operand_type == ty,
                    format!("{where_} changes the type of its operand"),
                );
                self.type_is(function, operand_type, operand, &where_);
            }
            InstKind::Call(call)
            | InstKind::Invoke { call, .. }
            | InstKind::CallBr { call, .. } => {
                let call = call.clone();
                let signature = self.module.ctx.type_kind(call.function_type).clone();
                if let TypeKind::Function {
                    params, is_var_arg, ..
                } = signature
                {
                    if is_var_arg {
                        self.check(
                            call.args.len() >= params.len(),
                            format!("{where_} passes too few arguments"),
                        );
                    } else {
                        self.check(
                            call.args.len() == params.len(),
                            format!("{where_} passes the wrong number of arguments"),
                        );
                    }
                    for (arg, param) in call.args.iter().zip(params.iter()) {
                        self.check(
                            arg.ty == *param,
                            format!("{where_} passes an argument of the wrong type"),
                        );
                        self.type_is(function, arg.ty, arg.value, &where_);
                    }
                } else {
                    self.report(format!("{where_} has a callee type that is not a function"));
                }
                for group in &call.fn_attrs.groups {
                    let group = *group;
                    if self.module.attribute_group(group).is_none() {
                        self.report(format!(
                            "{where_} refers to undefined attribute group #{group}"
                        ));
                    }
                }
                // Opaque pointers mean the callee carries no signature of its
                // own, so a call to a known function is the only place the two
                // can be compared. `call void @g()` against `declare void
                // @g(i32)` is well typed at the call site and still wrong.
                if let Value::Constant(id) = call.callee
                    && let Some(GlobalRef::Function(callee)) =
                        self.module.ctx.constant(id).as_global()
                {
                    let callee = self.module.function(callee);
                    let (result, params, is_var_arg) =
                        match self.module.ctx.type_kind(call.function_type) {
                            TypeKind::Function {
                                result,
                                params,
                                is_var_arg,
                            } => (*result, params.clone(), *is_var_arg),
                            _ => (call.function_type, Vec::new(), false),
                        };
                    let declared: Vec<TypeId> = callee.params.iter().map(|p| p.ty).collect();
                    let matches = result == callee.return_type
                        && params == declared
                        && is_var_arg == callee.is_var_arg;
                    self.check(
                        matches,
                        format!("{where_} does not match the signature of the function it calls"),
                    );
                }
            }
            _ => {}
        }
    }

    /// nuw and nsw belong to add, sub, mul and shl; exact to the divisions and
    /// the right shifts; disjoint to or. Anything else is a flag on an opcode
    /// that does not define it.
    fn integer_flags(&mut self, op: BinOp, flags: IntFlags, where_: &str) {
        let allowed = match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Shl => IntFlags {
                nuw: true,
                nsw: true,
                ..IntFlags::default()
            },
            BinOp::UDiv | BinOp::SDiv | BinOp::LShr | BinOp::AShr => IntFlags {
                exact: true,
                ..IntFlags::default()
            },
            BinOp::Or => IntFlags {
                disjoint: true,
                ..IntFlags::default()
            },
            _ => IntFlags::default(),
        };
        self.check(
            subset_of(flags, allowed),
            format!("{where_} carries a flag it does not define"),
        );
    }

    fn cast_flags(&mut self, op: CastOp, flags: IntFlags, where_: &str) {
        let allowed = match op {
            CastOp::Trunc => IntFlags {
                nuw: true,
                nsw: true,
                ..IntFlags::default()
            },
            CastOp::ZExt | CastOp::UiToFp => IntFlags {
                nneg: true,
                ..IntFlags::default()
            },
            _ => IntFlags::default(),
        };
        self.check(
            subset_of(flags, allowed),
            format!("{where_} carries a flag it does not define"),
        );
    }

    /// The shape rules that do not need the data layout: which kind of type
    /// each cast goes between, and which direction it has to move in.
    fn cast_shape(&mut self, op: CastOp, from: TypeId, to: TypeId, where_: &str) {
        let int = |v: &Self, ty| v.is_int_or_int_vector(ty);
        let float = |v: &Self, ty| v.is_float_or_float_vector(ty);
        let pointer = |v: &Self, ty| v.is_pointer_or_pointer_vector(ty);
        let ok = match op {
            CastOp::Trunc | CastOp::ZExt | CastOp::SExt => int(self, from) && int(self, to),
            CastOp::FpTrunc | CastOp::FpExt => float(self, from) && float(self, to),
            CastOp::FpToUi | CastOp::FpToSi => float(self, from) && int(self, to),
            CastOp::UiToFp | CastOp::SiToFp => int(self, from) && float(self, to),
            CastOp::PtrToInt | CastOp::PtrToAddr => pointer(self, from) && int(self, to),
            CastOp::IntToPtr => int(self, from) && pointer(self, to),
            CastOp::AddrSpaceCast => pointer(self, from) && pointer(self, to),
            CastOp::BitCast => true,
        };
        self.check(
            ok,
            format!("{where_} casts between the wrong kinds of type"),
        );

        if let (Some(from_bits), Some(to_bits)) = (self.scalar_bits(from), self.scalar_bits(to)) {
            let widening = matches!(op, CastOp::ZExt | CastOp::SExt | CastOp::FpExt);
            let narrowing = matches!(op, CastOp::Trunc | CastOp::FpTrunc);
            if widening {
                self.check(to_bits > from_bits, format!("{where_} does not widen"));
            }
            if narrowing {
                self.check(to_bits < from_bits, format!("{where_} does not narrow"));
            }
        }
    }

    // ------------------------------------------------------------- dominance

    /// Every use has to be dominated by its definition, except in a phi, where
    /// the incoming value has to dominate the end of the predecessor it comes
    /// from. This is the rule that makes SSA mean anything.
    fn dominance(&mut self, function: &Function) {
        let dominators = immediate_dominators(function);
        let mut defining_block: HashMap<InstId, BlockId> = HashMap::new();
        let mut position: HashMap<InstId, usize> = HashMap::new();
        for (block_id, block) in function.blocks() {
            for (index, inst) in block.instructions.iter().enumerate() {
                defining_block.insert(*inst, block_id);
                position.insert(*inst, index);
            }
        }

        for (block_id, block) in function.blocks() {
            for (index, inst) in block.instructions.iter().enumerate() {
                let Some(instruction) = function.try_instruction(*inst) else {
                    continue;
                };
                if let InstKind::Phi { incoming, .. } = &instruction.kind {
                    for (value, from) in incoming {
                        let Value::Instruction(used) = value else {
                            continue;
                        };
                        let Some(definition) = defining_block.get(used) else {
                            self.report("a phi names a value that is not in this function");
                            continue;
                        };
                        if !dominates(&dominators, *definition, *from) {
                            self.report(format!(
                                "a phi in {} takes a value that does not reach it from {}",
                                describe_block(function, block_id),
                                describe_block(function, *from)
                            ));
                        }
                    }
                    continue;
                }

                for used in operands(&instruction.kind) {
                    let Some(definition) = defining_block.get(&used) else {
                        self.report(format!(
                            "{} in {} uses a value that is not in this function",
                            instruction.kind.opcode(),
                            describe_block(function, block_id)
                        ));
                        continue;
                    };
                    let reaches = if *definition == block_id {
                        position[&used] < index
                    } else {
                        dominates(&dominators, *definition, block_id)
                    };
                    if !reaches {
                        self.report(format!(
                            "{} in {} uses a value defined where it cannot reach",
                            instruction.kind.opcode(),
                            describe_block(function, block_id)
                        ));
                    }
                }
            }
        }
    }

    // ----------------------------------------------------------- type helpers

    fn bool_type(&self) -> Option<TypeId> {
        (0..self.module.ctx.type_count())
            .map(|index| TypeId(index as u32))
            .find(|id| matches!(self.module.ctx.type_kind(*id), TypeKind::Integer(1)))
    }

    fn value_type(&self, function: &Function, value: Value) -> Option<TypeId> {
        match value {
            Value::Constant(id) => Some(self.module.ctx.constant(id).ty()),
            Value::Instruction(id) => function.try_instruction(id).map(|inst| inst.ty),
            Value::Argument(index) => function.params.get(index as usize).map(|p| p.ty),
            Value::Block(_) | Value::Metadata(_) => None,
        }
    }

    fn type_is(&mut self, function: &Function, expected: TypeId, value: Value, where_: &str) {
        if let Some(actual) = self.value_type(function, value)
            && actual != expected
        {
            self.report(format!("{where_} has an operand of the wrong type"));
        }
    }

    fn same_type(&mut self, function: &Function, expected: TypeId, value: Value, where_: &str) {
        self.type_is(function, expected, value, where_);
    }

    fn element_or_self(&self, ty: TypeId) -> TypeId {
        match self.module.ctx.type_kind(ty) {
            TypeKind::Vector { element, .. } => *element,
            _ => ty,
        }
    }

    fn is_int_or_int_vector(&self, ty: TypeId) -> bool {
        matches!(
            self.module.ctx.type_kind(self.element_or_self(ty)),
            TypeKind::Integer(_)
        )
    }

    fn is_float_or_float_vector(&self, ty: TypeId) -> bool {
        matches!(
            self.module.ctx.type_kind(self.element_or_self(ty)),
            TypeKind::Float(_)
        )
    }

    fn is_pointer_or_pointer_vector(&self, ty: TypeId) -> bool {
        matches!(
            self.module.ctx.type_kind(self.element_or_self(ty)),
            TypeKind::Pointer { .. }
        )
    }

    /// Width of a scalar or of a vector's element, for the widen and narrow
    /// rules. `None` when the type has no fixed width here.
    fn scalar_bits(&self, ty: TypeId) -> Option<u32> {
        match self.module.ctx.type_kind(self.element_or_self(ty)) {
            TypeKind::Integer(bits) => Some(*bits),
            TypeKind::Float(semantics) => Some(semantics.bit_width()),
            _ => None,
        }
    }
}

fn subset_of(flags: IntFlags, allowed: IntFlags) -> bool {
    (!flags.nuw || allowed.nuw)
        && (!flags.nsw || allowed.nsw)
        && (!flags.exact || allowed.exact)
        && (!flags.disjoint || allowed.disjoint)
        && (!flags.nneg || allowed.nneg)
        && (!flags.samesign || allowed.samesign)
}

fn describe(name: &Name) -> String {
    match name {
        Name::Named(text) => text.clone(),
        Name::Number(number) => number.to_string(),
    }
}

fn describe_block(function: &Function, id: BlockId) -> String {
    match function.try_block(id).and_then(|block| block.name.as_ref()) {
        Some(name) => format!("%{}", describe(name)),
        None => format!("block {}", id.0),
    }
}

fn predecessors_of(function: &Function, target: BlockId) -> Vec<BlockId> {
    let mut preds = Vec::new();
    for (id, _) in function.blocks() {
        for (_, instruction) in function.block_instructions(id) {
            if instruction.kind.successors().contains(&target) {
                preds.push(id);
                break;
            }
        }
    }
    preds
}

/// Every value an instruction reads, flattened.
fn operands(kind: &InstKind) -> Vec<InstId> {
    let mut out = Vec::new();
    let mut push = |value: &Value| {
        if let Value::Instruction(id) = value {
            out.push(*id);
        }
    };
    match kind {
        InstKind::Ret(Some((_, value)))
        | InstKind::Resume { value, .. }
        | InstKind::FNeg { operand: value, .. }
        | InstKind::Cast { operand: value, .. }
        | InstKind::Freeze { operand: value, .. }
        | InstKind::CondBr {
            condition: value, ..
        }
        | InstKind::Switch { value, .. }
        | InstKind::IndirectBr { address: value, .. }
        | InstKind::CatchRet { pad: value, .. }
        | InstKind::CleanupRet { pad: value, .. }
        | InstKind::CatchSwitch { parent: value, .. }
        | InstKind::VaArg { list: value, .. } => push(value),
        InstKind::Binary { lhs, rhs, .. }
        | InstKind::ICmp { lhs, rhs, .. }
        | InstKind::FCmp { lhs, rhs, .. } => {
            push(lhs);
            push(rhs);
        }
        InstKind::Load { pointer, .. } => push(pointer),
        InstKind::Store { value, pointer, .. } => {
            push(value);
            push(pointer);
        }
        InstKind::Alloca { count, .. } => {
            if let Some((_, value)) = count {
                push(value);
            }
        }
        InstKind::CmpXchg {
            pointer,
            compare,
            new,
            ..
        } => {
            push(pointer);
            push(compare);
            push(new);
        }
        InstKind::AtomicRmw { pointer, value, .. } => {
            push(pointer);
            push(value);
        }
        InstKind::GetElementPtr {
            pointer, indices, ..
        } => {
            push(pointer);
            for (_, index) in indices {
                push(index);
            }
        }
        InstKind::ExtractElement { vector, index, .. } => {
            push(vector);
            push(index);
        }
        InstKind::InsertElement {
            vector,
            element,
            index,
            ..
        } => {
            push(vector);
            push(element);
            push(index);
        }
        InstKind::ShuffleVector {
            first,
            second,
            mask,
            ..
        } => {
            push(first);
            push(second);
            push(mask);
        }
        InstKind::ExtractValue { aggregate, .. } => push(aggregate),
        InstKind::InsertValue {
            aggregate, element, ..
        } => {
            push(aggregate);
            push(element);
        }
        InstKind::Select {
            condition,
            if_true,
            if_false,
            ..
        } => {
            push(condition);
            push(if_true);
            push(if_false);
        }
        InstKind::Call(call) | InstKind::Invoke { call, .. } | InstKind::CallBr { call, .. } => {
            push(&call.callee);
            for arg in &call.args {
                push(&arg.value);
            }
            for bundle in &call.bundles {
                for (_, value) in &bundle.args {
                    push(value);
                }
            }
        }
        InstKind::CatchPad { parent, args } | InstKind::CleanupPad { parent, args } => {
            push(parent);
            for (_, value) in args {
                push(value);
            }
        }
        InstKind::LandingPad { clauses, .. } => {
            for clause in clauses {
                match clause {
                    crate::instruction::LandingPadClause::Catch { value, .. }
                    | crate::instruction::LandingPadClause::Filter { value, .. } => push(value),
                }
            }
        }
        InstKind::Phi { .. }
        | InstKind::Ret(None)
        | InstKind::Br { .. }
        | InstKind::Fence { .. }
        | InstKind::Unreachable
        | InstKind::DebugRecord { .. } => {}
    }
    out
}

/// Immediate dominators, by the iterative algorithm over reverse postorder.
///
/// The entry block dominates itself; a block nothing reaches has no entry, so
/// dominance queries about it answer false and its unreachable code is left
/// alone rather than reported as broken SSA.
fn immediate_dominators(function: &Function) -> HashMap<BlockId, BlockId> {
    let Some(entry) = function.entry_block() else {
        return HashMap::new();
    };
    let order = reverse_postorder(function, entry);
    let index_of: HashMap<BlockId, usize> = order
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect();
    let mut preds: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
    for (id, _) in function.blocks() {
        for (_, instruction) in function.block_instructions(id) {
            for successor in instruction.kind.successors() {
                preds.entry(successor).or_default().push(id);
            }
        }
    }

    let mut idom: HashMap<BlockId, BlockId> = HashMap::new();
    idom.insert(entry, entry);
    let mut changed = true;
    while changed {
        changed = false;
        for block in order.iter().skip(1) {
            let empty = Vec::new();
            let candidates = preds.get(block).unwrap_or(&empty);
            let mut new_idom = None;
            for pred in candidates {
                if !idom.contains_key(pred) {
                    continue;
                }
                new_idom = Some(match new_idom {
                    None => *pred,
                    Some(current) => intersect(&idom, &index_of, current, *pred),
                });
            }
            if let Some(new_idom) = new_idom
                && idom.get(block) != Some(&new_idom)
            {
                idom.insert(*block, new_idom);
                changed = true;
            }
        }
    }
    idom
}

fn intersect(
    idom: &HashMap<BlockId, BlockId>,
    index_of: &HashMap<BlockId, usize>,
    mut a: BlockId,
    mut b: BlockId,
) -> BlockId {
    while a != b {
        let (ai, bi) = (index_of[&a], index_of[&b]);
        if ai > bi {
            match idom.get(&a) {
                Some(next) if *next != a => a = *next,
                _ => return b,
            }
        } else {
            match idom.get(&b) {
                Some(next) if *next != b => b = *next,
                _ => return a,
            }
        }
    }
    a
}

fn reverse_postorder(function: &Function, entry: BlockId) -> Vec<BlockId> {
    let mut visited = HashSet::new();
    let mut order = Vec::new();
    let mut stack = vec![(entry, false)];
    while let Some((block, expanded)) = stack.pop() {
        if expanded {
            order.push(block);
            continue;
        }
        if !visited.insert(block) {
            continue;
        }
        stack.push((block, true));
        let mut successors = Vec::new();
        for (_, instruction) in function.block_instructions(block) {
            successors.extend(instruction.kind.successors());
        }
        for successor in successors.into_iter().rev() {
            if !visited.contains(&successor) {
                stack.push((successor, false));
            }
        }
    }
    order.reverse();
    order
}

fn dominates(idom: &HashMap<BlockId, BlockId>, definition: BlockId, use_site: BlockId) -> bool {
    if !idom.contains_key(&use_site) {
        // Unreachable code cannot be reached, so nothing about it is wrong.
        return true;
    }
    let mut current = use_site;
    loop {
        if current == definition {
            return true;
        }
        match idom.get(&current) {
            Some(next) if *next != current => current = *next,
            _ => return false,
        }
    }
}

/// Metadata a node points at, so the module check can find dangling numbers.
fn referenced_metadata(node: &crate::metadata::Metadata) -> Vec<MdId> {
    use crate::metadata::{MdField, MdOperand, Metadata, SpecializedArgs};
    let mut out = Vec::new();
    match node {
        Metadata::String(_) => {}
        Metadata::Tuple { operands, .. } => {
            for operand in operands {
                match operand {
                    MdOperand::Ref(id) => out.push(*id),
                    MdOperand::Inline(inner) => out.extend(referenced_metadata(inner)),
                    _ => {}
                }
            }
        }
        Metadata::Specialized { args, .. } => {
            let fields: Vec<&MdField> = match args {
                SpecializedArgs::Named(fields) => fields.iter().map(|(_, f)| f).collect(),
                SpecializedArgs::Positional(fields) => fields.iter().collect(),
            };
            for field in fields {
                match field {
                    MdField::Ref(id) => out.push(*id),
                    MdField::Inline(inner) => out.extend(referenced_metadata(inner)),
                    _ => {}
                }
            }
        }
    }
    out
}
