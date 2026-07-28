//! Printing values and instructions.

use std::fmt::Write as _;

use llvm_ir::function::Function;
use llvm_ir::{TypeId, TypeKind, Value};

use crate::{CONTINUATION, Printer, attribute_list, escape_string, name_text};
use llvm_ir::BlockId;
use llvm_ir::instruction::{
    CallData, CallingConv, FastMathFlags, InstKind, Instruction, IntFlags, LandingPadClause,
    NamedCallingConv, SyncScope, UnwindTarget,
};

use crate::align_text;

impl Printer<'_> {
    // ---------------------------------------------------------------- values

    pub(crate) fn value(&mut self, function: &Function, value: Value) {
        match value {
            Value::Constant(id) => self.constant(id),
            Value::Instruction(id) => {
                let text = match &function.instruction(id).name {
                    Some(name) => format!("%{}", name_text(name)),
                    None => match self.slots.instruction(id) {
                        Some(slot) => format!("%{slot}"),
                        None => "%<badref>".to_string(),
                    },
                };
                self.push(&text);
            }
            Value::Argument(index) => {
                let text = match &function.params[index as usize].name {
                    Some(name) => format!("%{}", name_text(name)),
                    None => match self.slots.argument(index) {
                        Some(slot) => format!("%{slot}"),
                        None => "%<badref>".to_string(),
                    },
                };
                self.push(&text);
            }
            Value::Block(id) => {
                let text = self.block_label(function, id);
                self.push(&text);
            }
            Value::Metadata(id) => {
                let _ = write!(self.out, "!{}", id.0);
            }
        }
    }

    /// A pointer operand, written with the address space it points through
    /// rather than a bare `ptr`. Most are address space zero and print the
    /// same either way, but a load through `ptr addrspace(42)` says so.
    pub(crate) fn pointer_operand(&mut self, function: &Function, value: Value) {
        let ty = match value {
            Value::Constant(id) => Some(self.module.ctx.constant(id).ty()),
            Value::Instruction(id) => function.try_instruction(id).map(|inst| inst.ty),
            Value::Argument(index) => function.params.get(index as usize).map(|param| param.ty),
            Value::Block(_) | Value::Metadata(_) => None,
        };
        match ty {
            Some(ty) => self.ty(ty),
            None => self.push("ptr"),
        }
        self.push(" ");
        self.value(function, value);
    }

    pub(crate) fn typed_value(&mut self, function: &Function, ty: TypeId, value: Value) {
        self.ty(ty);
        self.push(" ");
        self.value(function, value);
    }

    pub(crate) fn block_operand(&mut self, function: &Function, id: BlockId) {
        let text = self.block_label(function, id);
        let _ = write!(self.out, "label {text}");
    }

    // ---------------------------------------------------------- instructions

    pub(crate) fn instruction(&mut self, function: &Function, id: llvm_ir::InstId) {
        let instruction = function.instruction(id).clone();
        // A debug record is indented past the instructions it sits between,
        // the way upstream writes it: it is a record attached to the block
        // rather than an instruction in it.
        let debug_record = matches!(instruction.kind, InstKind::DebugRecord { .. });
        self.push(if debug_record { "    " } else { "  " });
        let produces_value =
            !matches!(self.module.ctx.type_kind(instruction.ty), TypeKind::Void) && !debug_record;
        if produces_value {
            match &instruction.name {
                Some(name) => {
                    let _ = write!(self.out, "%{} = ", name_text(name));
                }
                None => {
                    let slot = self.slots.instruction(id);
                    match slot {
                        Some(slot) => {
                            let _ = write!(self.out, "%{slot} = ");
                        }
                        None => self.push("%<badref> = "),
                    }
                }
            }
        }
        self.instruction_body(function, &instruction);
        self.metadata_attachments(&instruction.metadata, ", ");
        self.push("\n");
    }

    pub(crate) fn instruction_body(&mut self, function: &Function, instruction: &Instruction) {
        match &instruction.kind {
            InstKind::Ret(None) => self.push("ret void"),
            InstKind::Ret(Some((ty, value))) => {
                self.push("ret ");
                self.typed_value(function, *ty, *value);
            }
            InstKind::Br { target } => {
                self.push("br ");
                self.block_operand(function, *target);
            }
            InstKind::CondBr {
                condition,
                if_true,
                if_false,
            } => {
                self.push("br i1 ");
                self.value(function, *condition);
                self.push(", ");
                self.block_operand(function, *if_true);
                self.push(", ");
                self.block_operand(function, *if_false);
            }
            InstKind::Switch {
                value_type,
                value,
                default,
                cases,
            } => {
                self.push("switch ");
                self.typed_value(function, *value_type, *value);
                self.push(", ");
                self.block_operand(function, *default);
                self.push(" [\n");
                for (case, block) in cases {
                    self.push("    ");
                    self.typed_value(function, *value_type, *case);
                    self.push(", ");
                    self.block_operand(function, *block);
                    self.push("\n");
                }
                self.push("  ]");
            }
            InstKind::IndirectBr {
                address,
                destinations,
            } => {
                self.push("indirectbr ");
                self.value_with_inferred_type(function, *address);
                self.push(", [");
                for (index, block) in destinations.iter().enumerate() {
                    if index > 0 {
                        self.push(", ");
                    }
                    self.block_operand(function, *block);
                }
                self.push("]");
            }
            InstKind::Invoke {
                call,
                normal,
                unwind,
            } => {
                self.push("invoke ");
                self.call_body(function, call, false);
                let _ = write!(self.out, "\n{CONTINUATION}to ");
                self.block_operand(function, *normal);
                self.push(" unwind ");
                self.block_operand(function, *unwind);
            }
            InstKind::CallBr {
                call,
                fallthrough,
                indirect,
            } => {
                self.push("callbr ");
                self.call_body(function, call, false);
                let _ = write!(self.out, "\n{CONTINUATION}to ");
                self.block_operand(function, *fallthrough);
                self.push(" [");
                for (index, block) in indirect.iter().enumerate() {
                    if index > 0 {
                        self.push(", ");
                    }
                    self.block_operand(function, *block);
                }
                self.push("]");
            }
            InstKind::Resume { ty, value } => {
                self.push("resume ");
                self.typed_value(function, *ty, *value);
            }
            InstKind::CatchSwitch {
                parent,
                handlers,
                unwind,
            } => {
                self.push("catchswitch within ");
                self.value(function, *parent);
                self.push(" [");
                for (index, block) in handlers.iter().enumerate() {
                    if index > 0 {
                        self.push(", ");
                    }
                    self.block_operand(function, *block);
                }
                self.push("] unwind ");
                self.unwind_target(function, *unwind);
            }
            InstKind::CatchRet { pad, target } => {
                self.push("catchret from ");
                self.value(function, *pad);
                self.push(" to ");
                self.block_operand(function, *target);
            }
            InstKind::CleanupRet { pad, unwind } => {
                self.push("cleanupret from ");
                self.value(function, *pad);
                self.push(" unwind ");
                self.unwind_target(function, *unwind);
            }
            InstKind::Unreachable => self.push("unreachable"),
            InstKind::Binary {
                op,
                flags,
                fast_math,
                lhs,
                rhs,
            } => {
                self.push(op.keyword());
                self.int_flags(*flags);
                self.fast_math(*fast_math);
                self.push(" ");
                self.ty(instruction.ty);
                self.push(" ");
                self.value(function, *lhs);
                self.push(", ");
                self.value(function, *rhs);
            }
            InstKind::FNeg { fast_math, operand } => {
                self.push("fneg");
                self.fast_math(*fast_math);
                self.push(" ");
                self.ty(instruction.ty);
                self.push(" ");
                self.value(function, *operand);
            }
            InstKind::Cast {
                op,
                flags,
                fast_math,
                operand,
                source_type,
            } => {
                self.push(op.keyword());
                self.int_flags(*flags);
                self.fast_math(*fast_math);
                self.push(" ");
                self.typed_value(function, *source_type, *operand);
                self.push(" to ");
                self.ty(instruction.ty);
            }
            InstKind::ICmp {
                predicate,
                flags,
                operand_type,
                lhs,
                rhs,
            } => {
                self.push("icmp");
                self.int_flags(*flags);
                let _ = write!(self.out, " {} ", predicate.keyword());
                self.typed_value(function, *operand_type, *lhs);
                self.push(", ");
                self.value(function, *rhs);
            }
            InstKind::FCmp {
                predicate,
                fast_math,
                operand_type,
                lhs,
                rhs,
            } => {
                self.push("fcmp");
                self.fast_math(*fast_math);
                let _ = write!(self.out, " {} ", predicate.keyword());
                self.typed_value(function, *operand_type, *lhs);
                self.push(", ");
                self.value(function, *rhs);
            }
            InstKind::Alloca {
                allocated_type,
                count,
                align,
                address_space,
                inalloca,
                swifterror,
            } => {
                self.push("alloca ");
                if *inalloca {
                    self.push("inalloca ");
                }
                if *swifterror {
                    self.push("swifterror ");
                }
                self.ty(*allocated_type);
                // A count of exactly one is not an array allocation, and
                // upstream leaves it out. Only when it is written in the
                // width a count defaults to, though: `alloca i1, i64 1` keeps
                // its count because dropping it would change the width back
                // to i32.
                if let Some((ty, value)) = count
                    && !(self.is_constant_one(*value) && self.is_default_count_width(*ty))
                {
                    self.push(", ");
                    self.typed_value(function, *ty, *value);
                }
                self.push(&align_text(*align));
                // The default address space is not written, here as anywhere.
                if let Some(address_space) = address_space
                    && *address_space != 0
                {
                    let _ = write!(self.out, ", addrspace({address_space})");
                }
            }
            InstKind::Load {
                loaded_type,
                pointer,
                volatile,
                atomic,
                align,
            } => {
                self.push("load ");
                if atomic.is_some() {
                    self.push("atomic ");
                }
                if *volatile {
                    self.push("volatile ");
                }
                self.ty(*loaded_type);
                self.push(", ");
                self.pointer_operand(function, *pointer);
                if let Some((scope, ordering)) = atomic {
                    self.sync_scope(scope);
                    let _ = write!(self.out, " {}", ordering.keyword());
                }
                self.push(&align_text(*align));
            }
            InstKind::Store {
                value_type,
                value,
                pointer,
                volatile,
                atomic,
                align,
            } => {
                self.push("store ");
                if atomic.is_some() {
                    self.push("atomic ");
                }
                if *volatile {
                    self.push("volatile ");
                }
                self.typed_value(function, *value_type, *value);
                self.push(", ");
                self.pointer_operand(function, *pointer);
                if let Some((scope, ordering)) = atomic {
                    self.sync_scope(scope);
                    let _ = write!(self.out, " {}", ordering.keyword());
                }
                self.push(&align_text(*align));
            }
            InstKind::Fence { scope, ordering } => {
                self.push("fence");
                self.sync_scope(scope);
                let _ = write!(self.out, " {}", ordering.keyword());
            }
            InstKind::CmpXchg {
                pointer,
                compare_type,
                compare,
                new,
                weak,
                volatile,
                scope,
                success,
                failure,
                align,
            } => {
                self.push("cmpxchg ");
                if *weak {
                    self.push("weak ");
                }
                if *volatile {
                    self.push("volatile ");
                }
                self.pointer_operand(function, *pointer);
                self.push(", ");
                self.typed_value(function, *compare_type, *compare);
                self.push(", ");
                self.typed_value(function, *compare_type, *new);
                self.sync_scope(scope);
                let _ = write!(self.out, " {} {}", success.keyword(), failure.keyword());
                self.push(&align_text(*align));
            }
            InstKind::AtomicRmw {
                op,
                pointer,
                value_type,
                value,
                volatile,
                scope,
                ordering,
                align,
            } => {
                self.push("atomicrmw ");
                if *volatile {
                    self.push("volatile ");
                }
                let _ = write!(self.out, "{} ptr ", op.keyword());
                self.value(function, *pointer);
                self.push(", ");
                self.typed_value(function, *value_type, *value);
                self.sync_scope(scope);
                let _ = write!(self.out, " {}", ordering.keyword());
                self.push(&align_text(*align));
            }
            InstKind::GetElementPtr {
                source_type,
                pointer_type,
                pointer,
                indices,
                flags,
                inrange,
            } => {
                self.push("getelementptr ");
                self.gep_flags(*flags);
                if let Some((low, high)) = inrange {
                    let _ = write!(self.out, "inrange({low}, {high}) ");
                }
                self.ty(*source_type);
                self.push(", ");
                self.typed_value(function, *pointer_type, *pointer);
                for (ty, value) in indices {
                    self.push(", ");
                    self.typed_value(function, *ty, *value);
                }
            }
            InstKind::ExtractElement {
                vector_type,
                vector,
                index_type,
                index,
            } => {
                self.push("extractelement ");
                self.typed_value(function, *vector_type, *vector);
                self.push(", ");
                self.typed_value(function, *index_type, *index);
            }
            InstKind::InsertElement {
                vector_type,
                vector,
                element_type,
                element,
                index_type,
                index,
            } => {
                self.push("insertelement ");
                self.typed_value(function, *vector_type, *vector);
                self.push(", ");
                self.typed_value(function, *element_type, *element);
                self.push(", ");
                self.typed_value(function, *index_type, *index);
            }
            InstKind::ShuffleVector {
                vector_type,
                first,
                second,
                mask_type,
                mask,
            } => {
                self.push("shufflevector ");
                self.typed_value(function, *vector_type, *first);
                self.push(", ");
                self.typed_value(function, *vector_type, *second);
                self.push(", ");
                self.typed_value(function, *mask_type, *mask);
            }
            InstKind::ExtractValue {
                aggregate_type,
                aggregate,
                indices,
            } => {
                self.push("extractvalue ");
                self.typed_value(function, *aggregate_type, *aggregate);
                for index in indices {
                    let _ = write!(self.out, ", {index}");
                }
            }
            InstKind::InsertValue {
                aggregate_type,
                aggregate,
                element_type,
                element,
                indices,
            } => {
                self.push("insertvalue ");
                self.typed_value(function, *aggregate_type, *aggregate);
                self.push(", ");
                self.typed_value(function, *element_type, *element);
                for index in indices {
                    let _ = write!(self.out, ", {index}");
                }
            }
            InstKind::Phi {
                fast_math,
                incoming,
            } => {
                self.push("phi");
                self.fast_math(*fast_math);
                self.push(" ");
                self.ty(instruction.ty);
                self.push(" ");
                for (index, (value, block)) in incoming.iter().enumerate() {
                    if index > 0 {
                        self.push(", ");
                    }
                    self.push("[ ");
                    self.value(function, *value);
                    self.push(", ");
                    let label = self.block_label(function, *block);
                    self.push(&label);
                    self.push(" ]");
                }
            }
            InstKind::Select {
                fast_math,
                condition_type,
                condition,
                if_true,
                if_false,
            } => {
                self.push("select");
                self.fast_math(*fast_math);
                self.push(" ");
                self.typed_value(function, *condition_type, *condition);
                self.push(", ");
                self.typed_value(function, instruction.ty, *if_true);
                self.push(", ");
                self.typed_value(function, instruction.ty, *if_false);
            }
            InstKind::Freeze {
                operand_type,
                operand,
            } => {
                self.push("freeze ");
                self.typed_value(function, *operand_type, *operand);
            }
            InstKind::Call(call) => {
                self.call_body(function, call, true);
            }
            InstKind::VaArg { list_type, list } => {
                self.push("va_arg ");
                self.typed_value(function, *list_type, *list);
                self.push(", ");
                self.ty(instruction.ty);
            }
            InstKind::LandingPad { cleanup, clauses } => {
                self.push("landingpad ");
                self.ty(instruction.ty);
                if *cleanup {
                    let _ = write!(self.out, "\n{CONTINUATION}cleanup");
                }
                for clause in clauses {
                    match clause {
                        LandingPadClause::Catch { ty, value } => {
                            let _ = write!(self.out, "\n{CONTINUATION}catch ");
                            self.typed_value(function, *ty, *value);
                        }
                        LandingPadClause::Filter { ty, value } => {
                            let _ = write!(self.out, "\n{CONTINUATION}filter ");
                            self.typed_value(function, *ty, *value);
                        }
                    }
                }
            }
            InstKind::CatchPad { parent, args } => {
                self.push("catchpad within ");
                self.value(function, *parent);
                self.push(" [");
                for (index, (ty, value)) in args.iter().enumerate() {
                    if index > 0 {
                        self.push(", ");
                    }
                    self.typed_value(function, *ty, *value);
                }
                self.push("]");
            }
            InstKind::CleanupPad { parent, args } => {
                self.push("cleanuppad within ");
                self.value(function, *parent);
                self.push(" [");
                for (index, (ty, value)) in args.iter().enumerate() {
                    if index > 0 {
                        self.push(", ");
                    }
                    self.typed_value(function, *ty, *value);
                }
                self.push("]");
            }
            InstKind::DebugRecord { name, operands } => {
                let _ = write!(self.out, "#{name}(");
                for (index, operand) in operands.iter().enumerate() {
                    if index > 0 {
                        self.push(", ");
                    }
                    self.metadata_operand(operand);
                }
                self.push(")");
            }
        }
    }

    /// A value whose type the instruction does not spell out separately, which
    /// only happens where the operand is a constant carrying its own type or
    /// an instruction whose result type we can look up.
    pub(crate) fn value_with_inferred_type(&mut self, function: &Function, value: Value) {
        match value {
            Value::Constant(id) => self.constant_with_type(id),
            Value::Instruction(id) => {
                let ty = function.instruction(id).ty;
                self.typed_value(function, ty, value);
            }
            Value::Argument(index) => {
                let ty = function.params[index as usize].ty;
                self.typed_value(function, ty, value);
            }
            _ => self.value(function, value),
        }
    }

    pub(crate) fn call_body(&mut self, function: &Function, call: &CallData, print_tail: bool) {
        if print_tail && let Some(keyword) = call.tail.keyword() {
            let _ = write!(self.out, "{keyword} ");
        }
        if print_tail {
            self.push("call");
            self.fast_math(call.fast_math);
            self.push(" ");
        } else {
            self.fast_math(call.fast_math);
            if !call.fast_math.is_empty() {
                self.push(" ");
            }
        }
        self.calling_conv(call.calling_conv);
        if !call.return_attrs.is_empty() {
            let _ = write!(
                self.out,
                "{} ",
                attribute_list(self.module, &call.return_attrs, false)
            );
        }
        // Upstream writes the whole function type only when it has to: a
        // variadic callee, or a callee whose result type alone would be
        // ambiguous. Otherwise it writes the result type.
        let (result, params, is_var_arg) = match self.module.ctx.type_kind(call.function_type) {
            TypeKind::Function {
                result,
                params,
                is_var_arg,
            } => (*result, params.clone(), *is_var_arg),
            _ => (call.function_type, Vec::new(), false),
        };
        if is_var_arg {
            self.ty(result);
            self.push(" (");
            for (index, param) in params.iter().enumerate() {
                if index > 0 {
                    self.push(", ");
                }
                self.ty(*param);
            }
            if params.is_empty() {
                self.push("...");
            } else {
                self.push(", ...");
            }
            self.push(")");
        } else {
            self.ty(result);
        }
        if let Some(address_space) = call.address_space {
            let _ = write!(self.out, " addrspace({address_space})");
        }
        self.push(" ");
        self.value(function, call.callee);
        self.push("(");
        for (index, arg) in call.args.iter().enumerate() {
            if index > 0 {
                self.push(", ");
            }
            self.ty(arg.ty);
            if !arg.attrs.is_empty() {
                let _ = write!(
                    self.out,
                    " {}",
                    attribute_list(self.module, &arg.attrs, false)
                );
            }
            self.push(" ");
            self.value(function, arg.value);
        }
        self.push(")");
        if let Some(group) = self.group_for(&call.fn_attrs) {
            let _ = write!(self.out, " #{group}");
        }
        if !call.bundles.is_empty() {
            self.push(" [ ");
            for (index, bundle) in call.bundles.iter().enumerate() {
                if index > 0 {
                    self.push(", ");
                }
                let _ = write!(self.out, "\"{}\"(", escape_string(&bundle.tag));
                for (position, (ty, value)) in bundle.args.iter().enumerate() {
                    if position > 0 {
                        self.push(", ");
                    }
                    self.typed_value(function, *ty, *value);
                }
                self.push(")");
            }
            self.push(" ]");
        }
    }

    pub(crate) fn unwind_target(&mut self, function: &Function, target: UnwindTarget) {
        match target {
            UnwindTarget::Caller => self.push("to caller"),
            UnwindTarget::Block(block) => self.block_operand(function, block),
        }
    }

    /// Whether a type is the width an alloca count defaults to, which is
    /// what makes a count of one droppable.
    fn is_default_count_width(&self, ty: llvm_ir::TypeId) -> bool {
        matches!(
            self.module.ctx.type_kind(ty),
            llvm_ir::TypeKind::Integer(32)
        )
    }

    pub(crate) fn calling_conv(&mut self, conv: CallingConv) {
        match conv {
            CallingConv::C => {}
            CallingConv::Named(named) => {
                // Upstream's own spelling of these two ends in a space, so it
                // prints two where every other convention prints one. It is a
                // quirk rather than a rule, and reproducing it is what makes
                // the printed text match.
                let padding = match named {
                    NamedCallingConv::AvrIntr | NamedCallingConv::AvrSignal => "  ",
                    _ => " ",
                };
                let _ = write!(self.out, "{}{padding}", named.keyword());
            }
            CallingConv::Numbered(number) => {
                let _ = write!(self.out, "cc{number} ");
            }
            CallingConv::RiscvVls(length) => {
                let _ = write!(self.out, "riscv_vls_cc({length}) ");
            }
        }
    }

    pub(crate) fn int_flags(&mut self, flags: IntFlags) {
        if flags.nuw {
            self.push(" nuw");
        }
        if flags.nsw {
            self.push(" nsw");
        }
        if flags.exact {
            self.push(" exact");
        }
        if flags.disjoint {
            self.push(" disjoint");
        }
        if flags.nneg {
            self.push(" nneg");
        }
        if flags.samesign {
            self.push(" samesign");
        }
    }

    pub(crate) fn fast_math(&mut self, flags: FastMathFlags) {
        if flags.is_fast() {
            self.push(" fast");
            return;
        }
        // Upstream's order, which is not the order the flags are declared in:
        // reassoc comes first and afn last.
        if flags.reassoc {
            self.push(" reassoc");
        }
        if flags.nnan {
            self.push(" nnan");
        }
        if flags.ninf {
            self.push(" ninf");
        }
        if flags.nsz {
            self.push(" nsz");
        }
        if flags.arcp {
            self.push(" arcp");
        }
        if flags.contract {
            self.push(" contract");
        }
        if flags.afn {
            self.push(" afn");
        }
    }

    pub(crate) fn sync_scope(&mut self, scope: &SyncScope) {
        if let Some(name) = &scope.0 {
            let _ = write!(self.out, " syncscope(\"{}\")", escape_string(name));
        }
    }

    /// Whether a value is the integer constant one.
    fn is_constant_one(&self, value: Value) -> bool {
        match value {
            Value::Constant(id) => self
                .module
                .ctx
                .constant(id)
                .as_integer()
                .is_some_and(llvm_support::ApInt::is_one),
            _ => false,
        }
    }
}
