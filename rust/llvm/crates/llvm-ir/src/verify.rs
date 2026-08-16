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

use crate::attribute::{Attribute, AttributeSet, EnumAttr, IntAttr, StructuredAttr, TypeAttr};
use crate::constant::{CastOp, ConstExpr, ConstId, Constant};
use crate::function::Function;
use crate::global::{DllStorageClass, GlobalQualifiers, Linkage, RuntimePreemption, Visibility};
use crate::instruction::{
    AtomicOrdering, AtomicRmwOp, BinOp, CallingConv, InstKind, IntFlags, NamedCallingConv, TailKind,
};
use crate::intrinsic::table::Parameter;
use crate::metadata::{MdAttachment, MdField, MdOperand, MdRef, Metadata, SpecializedArgs};
use crate::module::Module;
use crate::summary::SummaryValue;
use crate::types::{StructId, TypeKind};
use crate::value::{AliasId, BlockId, GlobalRef, GlobalVarId, InstId, MdId, Name, Value};
use crate::{FunctionId, TypeId};
use llvm_support::ApInt;

/// The largest alignment upstream's encoding holds, in bytes. The parser caps
/// a written one; this is for the alignments a type asks for by its size.
const MAXIMUM_ALIGNMENT: u64 = 1 << 32;

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

    /// An attachment either names a node that has to exist, or carries one.
    fn attachment_resolves(&mut self, node: &MdRef, what: &str) {
        match node {
            MdRef::Id(id) => self.metadata_exists(*id, what),
            MdRef::Inline(inline) => {
                for referenced in referenced_metadata(inline) {
                    self.metadata_exists(referenced, what);
                }
            }
        }
    }

    /// A `!DILocation` says where in the source something came from, so it
    /// belongs on the instruction rather than inside a node the instruction
    /// carries. Upstream refuses one an attachment reaches through a plain
    /// node, on an instruction or on a function, while an attachment that is
    /// a location itself is what `!dbg` is.
    ///
    /// `llvm.loop` is exempt, its whole subtree with it, that being where the
    /// locations of a loop's own boundaries are kept. A global's attachments
    /// and a named list are not asked at all, and neither is a specialized
    /// node's field, which is where debug info reaches its own locations.
    ///
    /// Two kinds abort llvm-as rather than answering, `!prof` and
    /// `!annotation`, so there is no verdict for either and this refuses them
    /// the way it refuses every other kind.
    fn attachment_keeps_locations_out(&mut self, attachment: &MdAttachment, what: &str) {
        if attachment.kind == "llvm.loop" {
            return;
        }
        let Some(node) = self.resolve(&attachment.node) else {
            return;
        };
        let mut pending = match node.as_tuple() {
            Some(operands) => operands.to_vec(),
            None => return,
        };
        let mut seen: HashSet<MdId> = HashSet::new();
        while let Some(operand) = pending.pop() {
            let node = match operand {
                MdOperand::Ref(id) if seen.insert(id) => self.module.metadata_node(id).cloned(),
                MdOperand::Inline(node) => Some(*node),
                _ => None,
            };
            let Some(node) = node else { continue };
            if matches!(&node, Metadata::Specialized { tag, .. } if tag == "DILocation") {
                self.report(format!("{what} reaches a !DILocation through a plain node"));
                return;
            }
            if let Some(operands) = node.as_tuple() {
                pending.extend(operands.iter().cloned());
            }
        }
    }

    fn metadata_exists(&mut self, id: MdId, what: &str) {
        if self.module.metadata_node(id).is_none() {
            self.report(format!("{what} refers to undefined metadata !{}", id.0));
        }
    }

    // ----------------------------------------------------------- module rules

    fn module_level(&mut self) {
        self.reserved_globals_are_unread();
        self.compile_units_are_listed();
        // A blockaddress names a label in a function that may not have been
        // read yet when the constant is built, so the check waits until the
        // whole module is here. Only named labels are checked: matching `%3`
        // needs the slot numbers, which the printer works out and this does
        // not have.
        for index in 0..self.module.ctx.constant_count() {
            let Constant::BlockAddress {
                function, block, ..
            } = self.module.ctx.constant(ConstId(index as u32)).clone()
            else {
                continue;
            };
            let Name::Named(label) = &block else { continue };
            let GlobalRef::Function(id) = function else {
                continue;
            };
            let target = self.module.function(id);
            if target.block_order.is_empty() {
                continue;
            }
            let defined = target.block_order.iter().any(|candidate| {
                target.block(*candidate).name.as_ref() == Some(&Name::Named(label.clone()))
            });
            if !defined {
                let name = describe(&target.name);
                self.report(format!(
                    "a blockaddress names %{label}, which @{name} does not define"
                ));
            }
        }
        for index in 0..self.module.ctx.struct_count() {
            let id = StructId(index as u32);
            if self.module.ctx.struct_def(id).fields.is_none() {
                continue;
            }
            let fields = self
                .module
                .ctx
                .struct_def(id)
                .fields
                .clone()
                .unwrap_or_default();
            if fields
                .iter()
                .any(|field| self.reaches_struct(*field, id, &mut Vec::new()))
            {
                let name = self.module.ctx.struct_def(id).name.clone();
                self.report(format!("%{name} contains itself, so it has no size"));
            }
        }
        let mut comdats: HashSet<String> = HashSet::new();
        for index in 0..self.module.comdats.len() {
            let name = self.module.comdats[index].name.clone();
            if !comdats.insert(name.clone()) {
                self.report(format!("${name} is declared more than once"));
            }
        }
        self.summary_index();
        self.alias_targets();
        // Constant expressions are interned rather than owned by whatever
        // mentions them, so checking every one once beats finding them again
        // at each use.
        for index in 0..self.module.ctx.constant_count() {
            let id = ConstId(index as u32);
            let Constant::Expression(expr) = self.module.ctx.constant(id) else {
                continue;
            };
            if let ConstExpr::GetElementPtr { base, indices, .. } = &**expr {
                let base_type = self.module.ctx.constant(*base).ty();
                let indices: Vec<(TypeId, Value)> = indices
                    .iter()
                    .map(|id| (self.module.ctx.constant(*id).ty(), Value::Constant(*id)))
                    .collect();
                self.gep_vector_widths(base_type, &indices, "a getelementptr expression");
            }
        }
        // Upstream verifies the debug info it can reach, and a node nothing
        // names is not reached. `set1.ll` leans on that: it has a composite
        // type with a null among its elements, which is an error where it is
        // seen and not an error where it is not.
        let reachable = self.reachable_metadata();
        for id in reachable {
            if let Some(node) = self.module.metadata_node(id).cloned() {
                self.debug_info_node(&node);
            }
        }
        for index in 0..self.module.functions.len() {
            let function = &self.module.functions[index];
            let (name, qualifiers) = (describe(&function.name), function.qualifiers.clone());
            self.linkage_and_visibility(&name, &qualifiers);
        }
        for index in 0..self.module.aliases.len() {
            let alias = &self.module.aliases[index];
            let (name, qualifiers) = (describe(&alias.name), alias.qualifiers.clone());
            self.linkage_and_visibility(&name, &qualifiers);
        }
        for index in 0..self.module.ifuncs.len() {
            let ifunc = &self.module.ifuncs[index];
            let (name, qualifiers) = (describe(&ifunc.name), ifunc.qualifiers.clone());
            self.linkage_and_visibility(&name, &qualifiers);
        }
        for index in 0..self.module.globals.len() {
            let global = &self.module.globals[index];
            let name = describe(&global.name);
            let qualifiers = global.qualifiers.clone();
            self.linkage_and_visibility(&name, &qualifiers);
            let value_type = global.value_type;
            // Two rules with different reach. What kind of type it is, and
            // that it holds nothing scalable, hold either way. Having a body
            // to lay out is only asked of a definition: upstream reads
            // `@g = external global %opaque` and refuses to define one.
            let defined = global.initializer.is_some();
            if !self.fits_in_a_global(value_type) || (defined && !self.is_sized(value_type)) {
                self.report(format!("@{name} has an invalid type for a global variable"));
            }
            let (linkage, in_comdat) = {
                let global = &self.module.globals[index];
                (global.qualifiers.linkage, global.comdat.is_some())
            };
            // A common symbol is merged by the linker by being common, and a
            // comdat is a different merging rule; upstream refuses to have
            // both apply to one symbol.
            if linkage == Some(Linkage::Common) && in_comdat {
                self.report(format!("@{name}: a common global may not be in a comdat"));
            }
            let initializer = self.module.globals[index].initializer;
            if let Some(initializer) = initializer
                && !name.starts_with("llvm.")
                && let Some(intrinsic) = self.mentions_an_intrinsic(initializer)
            {
                self.report(format!("@{name} takes the address of @{intrinsic}"));
            }
            self.intrinsic_global(&name, value_type, initializer);
            let global = &self.module.globals[index];
            if let Some(initializer) = global.initializer {
                let actual = self.module.ctx.constant(initializer).ty();
                self.check(
                    actual == global.value_type,
                    format!("@{name} has an initialiser of the wrong type"),
                );
            }
            for attachment in global.metadata.clone() {
                self.attachment_resolves(&attachment.node, &format!("@{name}"));
                self.global_attachment(&name, &attachment);
            }
            let global = &self.module.globals[index];
            let declaration = global.initializer.is_none()
                || global.qualifiers.linkage == Some(Linkage::AvailableExternally);
            if let Some(comdat) = &global.comdat {
                self.comdat_member(&name, declaration);
                let wanted = comdat.name.clone().unwrap_or_else(|| name.clone());
                if !self.module.comdats.iter().any(|c| c.name == wanted) {
                    self.report(format!(
                        "@{name} is in comdat ${wanted}, which does not exist"
                    ));
                }
            }
        }
        for index in 0..self.module.ifuncs.len() {
            let ifunc = &self.module.ifuncs[index];
            let (name, resolver) = (describe(&ifunc.name), ifunc.resolver);
            // The resolver is called at load time, so it has to name a
            // function. A cast or an alias in the way is fine, because the
            // linker sees through both; arithmetic is not.
            if !self.names_a_symbol(resolver) {
                self.report(format!("@{name} must have a function as its resolver"));
            }
            let linkage = self.module.ifuncs[index].qualifiers.linkage;
            if matches!(
                linkage,
                Some(
                    Linkage::ExternWeak
                        | Linkage::AvailableExternally
                        | Linkage::Common
                        | Linkage::Appending
                )
            ) {
                self.report(format!("@{name} has a linkage an ifunc may not have"));
            }
        }

        for named in &self.module.named_metadata {
            let name = named.name.clone();
            let operands = named.operands.clone();
            for operand in &operands {
                self.metadata_exists(*operand, &format!("!{name}"));
            }
            match name.as_str().unwrap_or_default() {
                "llvm.module.flags" => {
                    for operand in &operands {
                        self.module_flag(*operand);
                    }
                    self.pauth_abi_is_named_whole(&operands);
                }
                // Both of these are lists of one-string nodes, and both have
                // upstream tests that say so.
                "llvm.ident" | "llvm.commandline" => {
                    for operand in &operands {
                        self.string_node(&name.to_lossy(), *operand);
                    }
                }
                _ => {}
            }
        }

        // Constant expressions obey the same cast rules as instructions, and
        // a module keeps only the constants it uses, so checking the whole
        // table checks exactly what appears.
        for index in 0..self.module.ctx.constant_count() {
            let constant = self.module.ctx.constant(ConstId(index as u32)).clone();
            let Constant::Expression(expr) = constant else {
                continue;
            };
            if let crate::constant::ConstExpr::Cast { op, operand, ty } = *expr {
                let from = self.module.ctx.constant(operand).ty();
                self.cast_shape(op, from, ty, "a constant expression");
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
        // A `!DIExpression` is a flat list of numbers that has to read as an
        // opcode, its operands, the next opcode and so on.
        // `crates/llvm-ir/src/metadata/expression.rs` is what upstream takes,
        // measured a module at a time.
        let invalid: Vec<MdId> = self
            .module
            .metadata_nodes()
            .filter(|(_, node)| !expression_is_valid(node))
            .map(|(id, _)| id)
            .collect();
        for id in invalid {
            self.report(format!("invalid expression in !{}", id.0));
        }
    }

    /// The globals whose names upstream reserves, and the shapes it insists
    /// on for them. Getting one wrong is not a style problem: the linker and
    /// the runtime both read these by layout.
    /// The node a specialized field refers to, whether by number or in place.
    fn field_node(&self, field: &MdField) -> Option<Metadata> {
        match field {
            MdField::Ref(id) => self.module.metadata_node(*id).cloned(),
            MdField::Inline(node) => Some((**node).clone()),
            _ => None,
        }
    }

    /// The rules LangRef states in prose, keyed on the base name.
    fn intrinsic_rule(
        &mut self,
        name: &str,
        result: TypeId,
        arguments: &[TypeId],
        values: &[Value],
        attributes: &[AttributeSet],
        bundles: &[String],
        returns_next: bool,
        where_: &str,
    ) {
        let base = crate::intrinsic::base_name(name);
        let element = |verifier: &Self, ty: TypeId| verifier.innermost_element(ty);
        match base {
            // A three-way compare answers lane by lane, in a result wide
            // enough to hold the three answers.
            "llvm.scmp" | "llvm.ucmp" => {
                let lanes = |verifier: &Self, ty: TypeId| {
                    TypeKind::as_vector(verifier.module.ctx.type_kind(ty)).map(|(_, n, s)| (n, s))
                };
                if let Some(argument) = arguments.first()
                    && lanes(self, *argument) != lanes(self, result)
                {
                    self.report(format!(
                        "{where_} answers in a different number of lanes than it compares"
                    ));
                }
                if let TypeKind::Integer(bits) = self.module.ctx.type_kind(element(self, result))
                    && *bits < 2
                {
                    self.report(format!(
                        "{where_} answers three ways in {bits} bit, which holds two"
                    ));
                }
            }
            // A predicated cast casts lane by lane like any other.
            _ if base.starts_with("llvm.vp.")
                && matches!(
                    base.trim_start_matches("llvm.vp."),
                    "fptosi"
                        | "fptoui"
                        | "sitofp"
                        | "uitofp"
                        | "fptrunc"
                        | "fpext"
                        | "trunc"
                        | "zext"
                        | "sext"
                        | "ptrtoint"
                        | "inttoptr"
                ) =>
            {
                let lanes = |verifier: &Self, ty: TypeId| {
                    TypeKind::as_vector(verifier.module.ctx.type_kind(ty)).map(|(_, n, s)| (n, s))
                };
                if let Some(argument) = arguments.first()
                    && lanes(self, *argument) != lanes(self, result)
                {
                    self.report(format!(
                        "{where_} casts a different number of lanes than it produces"
                    ));
                }
            }
            // The address of a thread's copy is only a thing a thread-local
            // has, so the argument names one rather than being any pointer.
            "llvm.threadlocal.address" => {
                let names_one = values.first().is_some_and(|value| {
                    let Value::Constant(id) = value else {
                        return false;
                    };
                    let Constant::Global { target, .. } = self.module.ctx.constant(*id) else {
                        return false;
                    };
                    // An alias to one is one, which is what upstream's own
                    // threadlocal-pass.ll leans on.
                    match target {
                        GlobalRef::Variable(id) => {
                            self.module.global(*id).qualifiers.thread_local.is_some()
                        }
                        GlobalRef::Alias(id) => {
                            self.module.alias(*id).qualifiers.thread_local.is_some()
                        }
                        _ => false,
                    }
                });
                if !names_one {
                    self.report(format!(
                        "{where_} takes the address of something that is not thread-local"
                    ));
                }
            }
            // The declaration says where one scope begins, so it names one
            // scope: a list holding a single node, not a string and not two.
            "llvm.experimental.noalias.scope.decl" => {
                let single = values.first().and_then(|value| {
                    let Value::Constant(id) = value else {
                        return None;
                    };
                    let Constant::Metadata { operand, .. } = self.module.ctx.constant(*id).clone()
                    else {
                        return None;
                    };
                    let node = match *operand {
                        MdOperand::Ref(id) => self.module.metadata_node(id).cloned(),
                        MdOperand::Inline(inline) => Some(*inline),
                        _ => None,
                    }?;
                    Some(node.as_tuple().is_some_and(|operands| operands.len() == 1))
                });
                if single == Some(false) {
                    self.report(format!("{where_} declares something other than one scope"));
                }
            }
            // A reduction folds a vector down to one of its own lanes, so
            // what it produces is what the vector holds.
            _ if base.starts_with("llvm.vector.reduce.") => {
                if let Some(vector) = arguments.last()
                    && let Some((element, _, _)) =
                        TypeKind::as_vector(self.module.ctx.type_kind(*vector))
                    && element != result
                {
                    self.report(format!(
                        "{where_} reduces to a type the vector does not hold"
                    ));
                }
            }
            // A byte swap has bytes to swap in pairs.
            "llvm.bswap" => {
                if let TypeKind::Integer(bits) = *self.module.ctx.type_kind(element(self, result))
                    && bits % 16 != 0
                {
                    self.report(format!(
                        "{where_} swaps {bits} bits, which is not a whole number of byte pairs"
                    ));
                }
            }
            // The alignment operand of a masked access is an alignment.
            "llvm.masked.load"
            | "llvm.masked.store"
            | "llvm.masked.gather"
            | "llvm.masked.scatter" => {
                // The alignment follows the pointer, and a store writes its
                // value before the pointer.
                let position =
                    usize::from(matches!(base, "llvm.masked.store" | "llvm.masked.scatter")) + 1;
                if let Some(Value::Constant(id)) = values.get(position)
                    && let Some(alignment) = self
                        .module
                        .ctx
                        .constant(*id)
                        .as_integer()
                        .and_then(ApInt::to_u64)
                    && (alignment == 0 || !alignment.is_power_of_two())
                {
                    self.report(format!(
                        "{where_} passes an alignment of {alignment}, which is not a power of two"
                    ));
                }
            }
            // The mask it produces is one bit per lane.
            "llvm.get.active.lane.mask"
                if !matches!(
                    self.module.ctx.type_kind(element(self, result)),
                    TypeKind::Integer(1)
                ) =>
            {
                self.report(format!("{where_} produces a mask that is not made of i1"));
            }
            // It counts the lanes, so what it counts into is a lane wide
            // enough to hold a count: an integer, and one of at least eight
            // bits, which is what makes `<vscale x 16 x i1>` too narrow where
            // `<4 x i8>` is fine.
            "llvm.stepvector"
                if !matches!(
                    TypeKind::as_vector(self.module.ctx.type_kind(result))
                        .map(|(element, _, _)| self.module.ctx.type_kind(element)),
                    Some(TypeKind::Integer(bits)) if *bits >= 8
                ) =>
            {
                self.report(format!("{where_} steps through lanes narrower than an i8"));
            }
            // The pattern is written into memory however many times it
            // fits, so it has to be something with a size: `target("foo")`
            // is unsized where `target("spirv.Event")` is not.
            "llvm.experimental.memset.pattern"
                if arguments
                    .get(1)
                    .is_some_and(|pattern| !self.is_sized(*pattern)) =>
            {
                self.report(format!("{where_} sets memory to an unsized pattern"));
            }
            // It masks a pointer, so it takes one and returns one.
            "llvm.ptrmask" => {
                let pointer = |verifier: &Self, ty: TypeId| {
                    matches!(
                        verifier
                            .module
                            .ctx
                            .type_kind(verifier.innermost_element(ty)),
                        TypeKind::Pointer { .. }
                    )
                };
                if !pointer(self, result) || arguments.first().is_none_or(|ty| !pointer(self, *ty))
                {
                    self.report(format!("{where_} masks something that is not a pointer"));
                }
            }
            // The vector length it asks for is a length.
            "llvm.experimental.get.vector.length" => {
                if let Some(Value::Constant(id)) = values.get(1)
                    && self
                        .module
                        .ctx
                        .constant(*id)
                        .as_integer()
                        .and_then(ApInt::to_u64)
                        == Some(0)
                {
                    self.report(format!("{where_} asks for a vector factor of zero"));
                }
            }
            // The offset is a number of bytes.
            "llvm.get.dynamic.area.offset"
                if !matches!(self.module.ctx.type_kind(result), TypeKind::Integer(_)) =>
            {
                self.report(format!(
                    "{where_} produces something other than a scalar integer"
                ));
            }
            // Splicing takes an index into the concatenation of the two
            // halves, counted from either end.
            "llvm.vector.splice" => {
                if let Some((_, count, false)) = arguments
                    .first()
                    .map(|ty| self.module.ctx.type_kind(*ty))
                    .and_then(TypeKind::as_vector)
                    && let Some(Value::Constant(id)) = values.get(2)
                    && let Some(written) = self.module.ctx.constant(*id).as_integer()
                    && let Ok(index) = i64::try_from(
                        written.to_u64_truncating() as i128
                            - if written.is_negative() {
                                1i128 << written.bits()
                            } else {
                                0
                            },
                    )
                    && (index >= count as i64 || index < -(count as i64))
                {
                    self.report(format!(
                        "{where_} splices at {index}, which is outside a vector of {count}"
                    ));
                }
            }
            // A subvector starts at a multiple of its own length.
            "llvm.vector.extract" | "llvm.vector.insert" => {
                let subvector = if base == "llvm.vector.extract" {
                    Some(result)
                } else {
                    arguments.get(1).copied()
                };
                if let Some((_, lanes, false)) = subvector
                    .map(|ty| self.module.ctx.type_kind(ty))
                    .and_then(TypeKind::as_vector)
                    && let Some(Value::Constant(id)) = values.last()
                    && let Some(index) = self
                        .module
                        .ctx
                        .constant(*id)
                        .as_integer()
                        .and_then(ApInt::to_u64)
                    && lanes != 0
                    && index % lanes != 0
                {
                    self.report(format!(
                        "{where_} starts at {index}, which is not a multiple of {lanes}"
                    ));
                }
            }
            // These reach through a pointer whose pointee the opaque type
            // system no longer records, so the call has to say what it is.
            "llvm.experimental.gc.statepoint"
            | "llvm.aarch64.ldxr"
            | "llvm.aarch64.ldaxr"
            | "llvm.aarch64.stxr"
            | "llvm.aarch64.stlxr"
            | "llvm.arm.ldrex"
            | "llvm.arm.ldaex"
            | "llvm.arm.strex"
            | "llvm.arm.stlex" => {
                // The statepoint's callee is its third argument; everything
                // else here reaches through the pointer it is given, which is
                // the last one.
                let position = if base == "llvm.experimental.gc.statepoint" {
                    2
                } else {
                    attributes.len().saturating_sub(1)
                };
                if let Some(attrs) = attributes.get(position)
                    && !has_type_attribute(attrs, TypeAttr::ElementType)
                {
                    self.report(format!(
                        "{where_} reaches through argument {position} without an elementtype"
                    ));
                }
            }
            // A matrix is held in a flat vector, so its two dimensions
            // multiply out to the number of lanes there are.
            "llvm.matrix.transpose" => {
                if let Some((_, lanes, false)) =
                    TypeKind::as_vector(self.module.ctx.type_kind(result))
                    && let (Some(rows), Some(columns)) = (
                        constant_u64(self, values.get(1)),
                        constant_u64(self, values.get(2)),
                    )
                    && rows.saturating_mul(columns) != lanes
                {
                    self.report(format!(
                        "{where_} transposes a {rows} by {columns} matrix held in {lanes} lanes"
                    ));
                }
            }
            // A deoptimising call does not come back, so the only thing that
            // may follow it is the return it stands in for.
            "llvm.experimental.deoptimize" if !returns_next => {
                self.report(format!("{where_} is not followed by a return"));
            }
            // A guard carries the state to deoptimise into, once.
            "llvm.experimental.guard" => {
                let deopt = bundles.iter().filter(|tag| *tag == "deopt").count();
                if deopt != 1 {
                    self.report(format!("{where_} carries {deopt} deopt bundles, not one"));
                }
            }
            _ => {}
        }
    }

    /// Whether an instruction is a call to the named intrinsic.
    fn calls_intrinsic(&self, function: &Function, id: InstId, wanted: &str) -> bool {
        let InstKind::Call(call) = &function.instruction(id).kind else {
            return false;
        };
        let Value::Constant(callee) = call.callee else {
            return false;
        };
        // The name an instantiation reduces to, not the name itself: an
        // overloaded intrinsic carries its types, so `llvm.va_start.p0` is
        // `llvm.va_start` called on a flat pointer and the rule is the same
        // one. Comparing the whole name missed every module that wrote the
        // components out, which is most of them.
        matches!(
            self.module.ctx.constant(callee).as_global(),
            Some(GlobalRef::Function(id))
                if matches!(
                    &self.module.function(id).name,
                    Name::Named(name)
                        if crate::intrinsic::candidates(name).any(|candidate| candidate == wanted)
                )
        )
    }

    /// The intrinsic a constant names, if it names one. An intrinsic is
    /// lowered where it is called, so there is nothing for its address to
    /// point at and taking one is an error rather than a missing definition.
    fn mentions_an_intrinsic(&self, id: ConstId) -> Option<String> {
        let named = |target: &GlobalRef| match target {
            GlobalRef::Function(id) => match &self.module.function(*id).name {
                Name::Named(name) if name.starts_with("llvm.") => Some(name.clone()),
                _ => None,
            },
            _ => None,
        };
        match self.module.ctx.constant(id) {
            Constant::Global { target, .. } => named(target),
            Constant::Struct { fields, .. } => {
                fields.iter().find_map(|f| self.mentions_an_intrinsic(*f))
            }
            Constant::Array { elements, .. } | Constant::Vector { elements, .. } => {
                elements.iter().find_map(|e| self.mentions_an_intrinsic(*e))
            }
            Constant::Expression(expr) => match **expr {
                ConstExpr::Cast { operand, .. } => self.mentions_an_intrinsic(operand),
                _ => None,
            },
            _ => None,
        }
    }

    /// The conventions that describe a GPU entry point, which the hardware
    /// calls rather than another function: they return nothing, take a fixed
    /// argument list, and two of them cannot be called at all.
    fn calling_convention(&mut self, function: &Function) {
        let conv = match function.calling_conv {
            CallingConv::Named(named) => named,
            _ => return,
        };
        let entry_point = matches!(
            conv,
            NamedCallingConv::AmdgpuKernel
                | NamedCallingConv::AmdgpuVs
                | NamedCallingConv::AmdgpuLs
                | NamedCallingConv::AmdgpuHs
                | NamedCallingConv::AmdgpuEs
                | NamedCallingConv::AmdgpuGs
                | NamedCallingConv::AmdgpuPs
                | NamedCallingConv::AmdgpuCs
                | NamedCallingConv::SpirKernel
        );
        if !entry_point {
            return;
        }
        if function.is_var_arg {
            self.report(format!(
                "the {} convention does not take a variable argument list",
                conv.keyword()
            ));
        }
        // `amdgpu_ps` and its siblings return a value to the fixed-function
        // hardware; the two kernel conventions return to nobody.
        let kernel = matches!(
            conv,
            NamedCallingConv::AmdgpuKernel | NamedCallingConv::SpirKernel
        );
        if kernel
            && !matches!(
                self.module.ctx.type_kind(function.return_type),
                TypeKind::Void
            )
        {
            self.report(format!("the {} convention returns nothing", conv.keyword()));
        }
    }

    /// An x86 interrupt handler is called by the processor, which has already
    /// pushed the interrupt frame onto the stack before the handler starts. So
    /// the first parameter is the frame, passed by reference to the memory the
    /// processor wrote it to: a `ptr byval(T)` and nothing else. A handler for
    /// an interrupt that carries an error code takes it as a second parameter,
    /// and that one is an ordinary value.
    fn x86_interrupt(&mut self, function: &Function) {
        if !matches!(
            function.calling_conv,
            CallingConv::Named(NamedCallingConv::X86Intr)
        ) {
            return;
        }
        let Some(first) = function.params.first() else {
            return;
        };
        let pointer = matches!(
            self.module.ctx.type_kind(first.ty),
            TypeKind::Pointer { .. }
        );
        if !pointer || !has_type_attribute(&first.attrs, TypeAttr::ByVal) {
            let name = describe(&function.name);
            self.report(format!(
                "@{name}: calling convention parameter requires byval"
            ));
        }
    }

    /// What a function does to AArch64's streaming mode and its two matrix
    /// state registers, written as quoted attributes. Each register is either
    /// created fresh, read, written, both, preserved, or left to the caller to
    /// decide, and those are six answers to one question rather than six
    /// independent claims. Streaming mode is the same shape with two answers:
    /// `aarch64_pstate_sm_body` says what the body does and sits alongside
    /// either.
    fn sme_state(&mut self, attrs: &AttributeSet) {
        const ZA: [&str; 6] = [
            "aarch64_new_za",
            "aarch64_in_za",
            "aarch64_out_za",
            "aarch64_inout_za",
            "aarch64_preserves_za",
            "aarch64_za_state_agnostic",
        ];
        const ZT0: [&str; 6] = [
            "aarch64_new_zt0",
            "aarch64_in_zt0",
            "aarch64_out_zt0",
            "aarch64_inout_zt0",
            "aarch64_preserves_zt0",
            "aarch64_za_state_agnostic",
        ];
        let written = |key: &str| {
            attrs
                .attributes
                .iter()
                .any(|a| matches!(a, Attribute::String { key: k, .. } if k == key))
        };
        if written("aarch64_pstate_sm_enabled") && written("aarch64_pstate_sm_compatible") {
            self.report(
                "'aarch64_pstate_sm_enabled' and 'aarch64_pstate_sm_compatible' are incompatible",
            );
        }
        for (group, register) in [(ZA, "za"), (ZT0, "zt0")] {
            if group.iter().filter(|key| written(key)).count() > 1 {
                self.report(format!(
                    "the attributes describing {register} state are mutually exclusive"
                ));
            }
        }
        // Whether the register's contents matter after this one call is the
        // caller's knowledge, not something the callee can declare.
        if written("aarch64_zt0_undef") {
            self.report("'aarch64_zt0_undef' can only be applied to a callsite");
        }
    }

    /// An alias needs something to alias. A declaration is not a definition,
    /// and neither is an `available_externally` body, which the linker is
    /// entitled to drop.
    fn alias_targets(&mut self) {
        for index in 0..self.module.aliases.len() {
            // Following the chain has to end somewhere, and an alias that
            // reaches itself never does.
            let mut seen = vec![AliasId(index as u32)];
            let mut current = self.module.aliases[index].aliasee;
            while let Some(GlobalRef::Alias(next)) = self.resolve_symbol(current) {
                if seen.contains(&next) {
                    let name = describe(&self.module.aliases[index].name);
                    self.report(format!("@{name} aliases its way back to itself"));
                    break;
                }
                seen.push(next);
                current = self.module.aliases[next.0 as usize].aliasee;
            }
        }
        for index in 0..self.module.aliases.len() {
            let alias = &self.module.aliases[index];
            let (name, aliasee) = (describe(&alias.name), alias.aliasee);
            let Some(target) = self.resolve_symbol(aliasee) else {
                continue;
            };
            let defined = match target {
                GlobalRef::Function(id) => {
                    let function = self.module.function(id);
                    function.is_definition()
                        && function.qualifiers.linkage != Some(Linkage::AvailableExternally)
                }
                GlobalRef::Variable(id) => {
                    let global = &self.module.globals[id.0 as usize];
                    global.initializer.is_some()
                        && global.qualifiers.linkage != Some(Linkage::AvailableExternally)
                }
                GlobalRef::Alias(_) | GlobalRef::IFunc(_) => true,
            };
            if !defined {
                self.report(format!(
                    "@{name} aliases something this module does not define"
                ));
            }
            // A bare reference to a symbol has the symbol's own pointer
            // type, address space and all. Crossing address spaces is what
            // `addrspacecast` is for, so an expression aliasee is not asked.
            let bare = match self.module.ctx.constant(aliasee) {
                Constant::Global { target, ty } => Some((*target, *ty)),
                _ => None,
            };
            if let Some((target, ty)) = bare
                && let Some(written) = self.module.ctx.type_kind(ty).pointer_address_space()
            {
                let actual = match target {
                    GlobalRef::Function(id) => self
                        .module
                        .function(id)
                        .qualifiers
                        .address_space
                        .unwrap_or(0),
                    GlobalRef::Variable(id) => self.module.globals[id.0 as usize]
                        .qualifiers
                        .address_space
                        .unwrap_or(0),
                    _ => written,
                };
                if written != actual {
                    self.report(format!(
                        "@{name} names a symbol in address space {actual} through a pointer to \
                         address space {written}"
                    ));
                }
            }
        }
    }

    /// The symbol a constant names, through any number of casts.
    fn resolve_symbol(&self, id: ConstId) -> Option<GlobalRef> {
        match self.module.ctx.constant(id) {
            Constant::Global { target, .. } => Some(*target),
            Constant::Expression(expr) => match **expr {
                ConstExpr::Cast { operand, .. } => self.resolve_symbol(operand),
                _ => None,
            },
            _ => None,
        }
    }

    /// The attributes that say something about a result, as against the ones
    /// that say something about the function or about a place a caller put an
    /// argument. `define nounwind i8 @f()` writes a promise about the whole
    /// function where a promise about its result belongs.
    fn attributes_apply_to_a_result(&mut self, attrs: &AttributeSet) {
        for attribute in &attrs.attributes {
            let applies = match attribute {
                Attribute::Enum(kind) => matches!(
                    kind,
                    EnumAttr::InReg
                        | EnumAttr::NoAlias
                        | EnumAttr::NoExt
                        | EnumAttr::NonNull
                        | EnumAttr::NoUndef
                        | EnumAttr::SignExt
                        | EnumAttr::ZeroExt
                ),
                Attribute::Int { kind, .. } => matches!(
                    kind,
                    IntAttr::Align
                        | IntAttr::AlignStack
                        | IntAttr::Dereferenceable
                        | IntAttr::DereferenceableOrNull
                ),
                Attribute::Range { .. } => true,
                Attribute::Structured { kind, .. } => {
                    matches!(kind, StructuredAttr::NoFpClass)
                }
                // A quoted attribute is carried rather than read, and upstream
                // carries one here.
                Attribute::String { .. } => true,
                Attribute::Type { .. } => false,
            };
            if !applies {
                self.report(format!(
                    "{} does not apply to return values",
                    describe_attribute(attribute)
                ));
            }
        }
    }

    fn named_metadata_roots(&self) -> Vec<MdId> {
        self.module
            .named_metadata
            .iter()
            .flat_map(|named| named.operands.clone())
            .collect()
    }

    /// A compile unit describes a whole translation unit, and what a consumer
    /// reads to find them is `llvm.dbg.cu`. One a named list mentions and that
    /// list does not is a unit nothing will be told about.
    ///
    /// The reach is narrower than the debug-info rules': a unit an attachment
    /// leads to is not asked, only one a named list leads to. So a
    /// `DISubprogram` written into a `!named` list takes its `unit:` with it,
    /// while the same subprogram hung off a function does not.
    fn compile_units_are_listed(&mut self) {
        let listed: Vec<MdId> = self
            .module
            .named_metadata
            .iter()
            .filter(|named| named.name == "llvm.dbg.cu")
            .flat_map(|named| named.operands.clone())
            .collect();

        let mut seen: HashSet<MdId> = HashSet::new();
        let mut pending = self.named_metadata_roots();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(node) = self.module.metadata_node(id) else {
                continue;
            };
            if let Metadata::Specialized { tag, .. } = node
                && tag == "DICompileUnit"
                && !listed.contains(&id)
            {
                self.report(format!(
                    "!{}: DICompileUnit not listed in llvm.dbg.cu",
                    id.0
                ));
            }
            pending.extend(self.node_references(id));
        }
    }

    /// Every metadata node a named list, a global or an instruction reaches,
    /// directly or through another node.
    fn reachable_metadata(&self) -> Vec<MdId> {
        let mut roots: Vec<MdId> = self.named_metadata_roots();
        for global in &self.module.globals {
            roots.extend(attachment_ids(&global.metadata));
        }
        for function in &self.module.functions {
            roots.extend(attachment_ids(&function.metadata));
            for (_, block) in function.blocks() {
                for inst in &block.instructions {
                    if let Some(instruction) = function.try_instruction(*inst) {
                        roots.extend(attachment_ids(&instruction.metadata));
                    }
                }
            }
        }

        let mut seen: HashSet<MdId> = HashSet::new();
        let mut order = Vec::new();
        while let Some(id) = roots.pop() {
            if !seen.insert(id) {
                continue;
            }
            order.push(id);
            roots.extend(self.node_references(id));
        }
        order
    }

    /// The nodes one node names, whether it is a tuple or a specialized node.
    fn node_references(&self, id: MdId) -> Vec<MdId> {
        let Some(node) = self.module.metadata_node(id) else {
            return Vec::new();
        };
        match node {
            Metadata::Tuple { operands, .. } => operands
                .iter()
                .filter_map(|operand| match operand {
                    MdOperand::Ref(id) => Some(*id),
                    _ => None,
                })
                .collect(),
            Metadata::Specialized { args, .. } => {
                let fields: Vec<&MdField> = match args {
                    SpecializedArgs::Named(fields) => {
                        fields.iter().map(|(_, value)| value).collect()
                    }
                    SpecializedArgs::Positional(values) => values.iter().collect(),
                };
                fields
                    .iter()
                    .filter_map(|field| match field {
                        MdField::Ref(id) => Some(*id),
                        _ => None,
                    })
                    .collect()
            }
            Metadata::String(_) => Vec::new(),
        }
    }

    /// A `gv` entry that names a symbol has to name one this module has.
    /// Upstream reports it as a parse error; the effect is the same and the
    /// name table is easier to reach from here.
    fn summary_index(&mut self) {
        let named: HashSet<String> = self
            .module
            .globals
            .iter()
            .map(|global| &global.name)
            .chain(self.module.functions.iter().map(|f| &f.name))
            .chain(self.module.aliases.iter().map(|a| &a.name))
            .chain(self.module.ifuncs.iter().map(|i| &i.name))
            .filter_map(|name| match name {
                Name::Named(text) => Some(text.clone()),
                Name::Number(_) => None,
            })
            .collect();
        let missing: Vec<String> = self
            .module
            .summary
            .iter()
            .filter(|entry| entry.kind == "gv")
            .filter_map(|entry| match &entry.value {
                SummaryValue::Tuple(fields) => {
                    fields
                        .iter()
                        .find_map(|field| match (field.key.as_deref(), &field.value) {
                            (Some("name"), SummaryValue::String(text)) if !named.contains(text) => {
                                Some(text.clone())
                            }
                            _ => None,
                        })
                }
                _ => None,
            })
            .collect();
        for name in missing {
            self.report(format!(
                "the summary index names @{name}, which this module has not"
            ));
        }
    }

    /// Whether a constant names a symbol, through any number of casts.
    fn names_a_symbol(&self, id: ConstId) -> bool {
        match self.module.ctx.constant(id) {
            Constant::Global { .. } => true,
            Constant::Expression(expr) => match **expr {
                ConstExpr::Cast { operand, .. } => self.names_a_symbol(operand),
                _ => false,
            },
            _ => false,
        }
    }

    /// A comdat groups definitions that the linker picks one of, which needs
    /// a definition to pick.
    ///
    /// It also needs a name the linker can see, and private linkage does not
    /// give it one, but that rule is COFF's rather than the IR's: upstream
    /// reports it only for a Windows triple, and llvm-as reads the same
    /// module without one.
    fn comdat_member(&mut self, name: &str, declaration: bool) {
        // The linker matches comdat members by the name of the group, which
        // defaults to the symbol's own. A symbol with only a number has none
        // to give.
        if name.chars().all(|c| c.is_ascii_digit()) {
            self.report(format!("@{name} has no name to key a comdat on"));
        }
        if declaration {
            self.report(format!(
                "@{name} is a declaration and may not be in a comdat"
            ));
        }
    }

    /// The metadata kinds a global carries whose shape upstream checks.
    fn global_attachment(&mut self, name: &str, attachment: &MdAttachment) {
        let Some(node) = self.resolve(&attachment.node) else {
            return;
        };
        let Some(operands) = node.as_tuple() else {
            return;
        };
        // `!associated` names another symbol this one is kept alive by, and
        // `!absolute_symbol` gives the ranges the linker may place it in. One
        // range is two values, and there may be several.
        let pointer_valued = matches!(
            operands,
            [MdOperand::Value { ty, .. }]
                if matches!(self.module.ctx.type_kind(*ty), TypeKind::Pointer { .. })
        );
        match attachment.kind.as_str().unwrap_or_default() {
            "associated" if !pointer_valued => {
                self.report(format!(
                    "@{name}: !associated takes one pointer-typed value"
                ));
            }
            "absolute_symbol" if operands.is_empty() || operands.len() % 2 != 0 => {
                self.report(format!(
                    "@{name}: !absolute_symbol takes ranges of two values"
                ));
            }
            _ => {}
        }
    }

    fn intrinsic_global(&mut self, name: &str, value_type: TypeId, initializer: Option<ConstId>) {
        match name {
            "llvm.used" | "llvm.compiler.used" | "llvm.compiler_used" => {
                let element = match self.module.ctx.type_kind(value_type) {
                    TypeKind::Array { element, .. } => Some(*element),
                    _ => None,
                };
                let ok = element.is_some_and(|e| {
                    matches!(self.module.ctx.type_kind(e), TypeKind::Pointer { .. })
                });
                if !ok {
                    self.report(format!(
                        "@{name} has the wrong type for an intrinsic global"
                    ));
                    return;
                }
                // The list has to name its entries; a zeroed array names none
                // of them and is what an accidental declaration produces.
                let listed = initializer.is_some_and(|id| {
                    matches!(self.module.ctx.constant(id), Constant::Array { .. })
                });
                let empty = matches!(
                    self.module.ctx.type_kind(value_type),
                    TypeKind::Array { count: 0, .. }
                );
                if !listed && !empty {
                    self.report(format!(
                        "@{name} has the wrong initialiser for an intrinsic global"
                    ));
                }
                // The point of the list is to keep symbols alive, and a null
                // keeps nothing alive.
                if let Some(id) = initializer
                    && let Constant::Array { elements, .. } = self.module.ctx.constant(id).clone()
                    && elements
                        .iter()
                        .any(|element| self.resolve_symbol(*element).is_none())
                {
                    self.report(format!("@{name} has a member that names no symbol"));
                }
            }
            "llvm.global_ctors" | "llvm.global_dtors" => {
                let element = match self.module.ctx.type_kind(value_type) {
                    TypeKind::Array { element, .. } => *element,
                    _ => {
                        self.report(format!(
                            "@{name} has the wrong type for an intrinsic global"
                        ));
                        return;
                    }
                };
                let fields = match self.module.ctx.type_kind(element) {
                    TypeKind::Struct { fields, .. } => fields.clone(),
                    TypeKind::NamedStruct(id) => self
                        .module
                        .ctx
                        .struct_def(*id)
                        .fields
                        .clone()
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                // { priority, function, associated data }. The two-field form
                // was obsoleted, and a module still using it would silently
                // lose its associated-data column.
                if fields.len() != 3 {
                    self.report(format!(
                        "@{name}: the third field of the element type is mandatory"
                    ));
                }
            }
            _ => {}
        }
    }

    /// The four globals whose contents mean something to whoever consumes the
    /// module rather than to the module itself. `llvm.used` keeps a symbol
    /// alive and `llvm.global_ctors` names what runs before main; neither
    /// holds a pointer the program reads, so upstream refuses a module that
    /// reads one. `llvm.compiler_used`, spelled with an underscore, is not
    /// one of the four and may be read like any other global.
    fn reserved_globals_are_unread(&mut self) {
        let reserved: Vec<(GlobalVarId, String)> = self
            .module
            .globals
            .iter()
            .enumerate()
            .filter_map(|(index, global)| {
                let Name::Named(name) = &global.name else {
                    return None;
                };
                matches!(
                    name.as_str(),
                    "llvm.used" | "llvm.compiler.used" | "llvm.global_ctors" | "llvm.global_dtors"
                )
                .then(|| (GlobalVarId(index as u32), name.clone()))
            })
            .collect();
        if reserved.is_empty() {
            return;
        }

        let mut mentioned: HashSet<GlobalVarId> = HashSet::new();
        let mut seen: HashSet<ConstId> = HashSet::new();
        let mut pending: Vec<ConstId> = Vec::new();
        // A global's own initializer is where a reserved global lists what it
        // keeps, so it is not a site that reads one; every other constant is.
        for (index, global) in self.module.globals.iter().enumerate() {
            let here = GlobalVarId(index as u32);
            if let Some(initializer) = global.initializer
                && !reserved.iter().any(|(id, _)| *id == here)
            {
                pending.push(initializer);
            }
        }
        for alias in &self.module.aliases {
            pending.push(alias.aliasee);
        }
        for ifunc in &self.module.ifuncs {
            pending.push(ifunc.resolver);
        }
        for function in &self.module.functions {
            for (id, _) in function.blocks() {
                for (_, instruction) in function.block_instructions(id) {
                    for value in instruction.kind.operand_values() {
                        if let Value::Constant(id) = value {
                            pending.push(id);
                        }
                    }
                }
            }
        }

        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            match self.module.ctx.constant(id) {
                Constant::Global {
                    target: GlobalRef::Variable(id),
                    ..
                } => {
                    mentioned.insert(*id);
                }
                Constant::Struct { fields, .. } => pending.extend(fields.iter().copied()),
                Constant::Array { elements, .. } | Constant::Vector { elements, .. } => {
                    pending.extend(elements.iter().copied());
                }
                Constant::Splat { element, .. } => pending.push(*element),
                Constant::Expression(expr) => pending.extend(expr.parts().0),
                _ => {}
            }
        }

        for (id, name) in reserved {
            if mentioned.contains(&id) {
                self.report(format!("invalid uses of intrinsic global variable @{name}"));
            }
        }
    }

    /// A symbol nobody outside the module can name has nothing to be hidden
    /// from, so upstream rejects the combination rather than ignoring it.
    fn linkage_and_visibility(&mut self, name: &str, qualifiers: &GlobalQualifiers) {
        let local = matches!(
            qualifiers.linkage,
            Some(Linkage::Private | Linkage::Internal)
        );
        let non_default = matches!(
            qualifiers.visibility,
            Some(Visibility::Hidden | Visibility::Protected)
        );
        if local && non_default {
            self.report(format!(
                "@{name}: symbol with local linkage must have default visibility"
            ));
        }
        // Importing a symbol from another image and promising it is local
        // to this one say opposite things.
        if qualifiers.dll_storage == Some(DllStorageClass::Import)
            && qualifiers.preemption == Some(RuntimePreemption::DsoLocal)
        {
            self.report(format!("@{name} is both dllimport and dso_local"));
        }
        // An exported symbol has to be visible to be exported.
        if qualifiers.dll_storage == Some(DllStorageClass::Export)
            && qualifiers.visibility == Some(Visibility::Hidden)
        {
            self.report(format!(
                "@{name}: a dllexport symbol must have default or protected visibility"
            ));
        }
    }

    /// A module flag is `!{i32 behaviour, !"key", value}`.
    fn module_flag(&mut self, id: MdId) {
        let Some(node) = self.module.metadata_node(id) else {
            return;
        };
        let Some(operands) = node.as_tuple() else {
            self.report(format!("!{}: module flag must be a MDNode triple", id.0));
            return;
        };
        if operands.len() != 3 {
            self.report(format!("!{}: module flag must be a MDNode triple", id.0));
            return;
        }
        let behaviour = match &operands[0] {
            MdOperand::Value {
                value: Value::Constant(constant),
                ..
            } => self
                .module
                .ctx
                .constant(*constant)
                .as_integer()
                .and_then(ApInt::to_u64),
            _ => None,
        };
        match behaviour {
            // Error, Warning, Require, Override, Append, AppendUnique, Max,
            // Min, in that order.
            Some(1..=8) => {}
            _ => self.report(format!(
                "!{}: invalid behaviour operand in module flag",
                id.0
            )),
        }
        if !matches!(operands[1], MdOperand::String(_)) {
            self.report(format!("!{}: invalid ID operand in module flag", id.0));
        }
        // Two flags say something about the shape of their own value.
        let MdOperand::String(name) = &operands[1] else {
            return;
        };
        match name.as_str().unwrap_or_default() {
            // A yes-or-no answer, written as a number.
            "SemanticInterposition" => {
                let is_integer = matches!(&operands[2], MdOperand::Value {
                    value: Value::Constant(constant),
                    ..
                } if self.module.ctx.constant(*constant).as_integer().is_some());
                if !is_integer {
                    self.report(format!(
                        "!{}: SemanticInterposition is a number rather than a word",
                        id.0
                    ));
                }
            }
            // A list of edges, each naming a caller, a callee and a count.
            "CG Profile" => {
                let entries = match &operands[2] {
                    MdOperand::Ref(list) => self
                        .module
                        .metadata_node(*list)
                        .and_then(|node| node.as_tuple().map(<[MdOperand]>::to_vec)),
                    _ => None,
                };
                for entry in entries.unwrap_or_default() {
                    let MdOperand::Ref(entry) = entry else {
                        continue;
                    };
                    let holds_three = self
                        .module
                        .metadata_node(entry)
                        .and_then(Metadata::as_tuple)
                        .is_some_and(|edge| edge.len() == 3);
                    if !holds_three {
                        self.report(format!(
                            "!{}: a CG Profile edge names a caller, a callee and a count",
                            id.0
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    /// `!llvm.ident` and `!llvm.commandline` hold nodes of exactly one string.
    fn string_node(&mut self, list: &str, id: MdId) {
        let Some(node) = self.module.metadata_node(id) else {
            return;
        };
        let ok = node.as_tuple().is_some_and(|operands| {
            operands.len() == 1 && matches!(operands[0], MdOperand::String(_))
        });
        if !ok {
            self.report(format!(
                "!{list} operand !{} must be a node with one string",
                id.0
            ));
        }
    }

    // --------------------------------------------------------- function rules

    fn function(&mut self, id: FunctionId) {
        let function = self.module.function(id);
        self.function = Some(describe(&function.name));

        for attachment in function.metadata.clone() {
            self.attachment_resolves(&attachment.node, "the function");
            self.attachment_keeps_locations_out(&attachment, "the function");
        }

        let profiles = function
            .metadata
            .iter()
            .filter(|attachment| attachment.kind == "prof")
            .count();
        if profiles > 0 && !function.is_definition() {
            self.report("a function declaration may not have a !prof attachment");
        }
        if profiles > 1 {
            self.report("a function may have only one !prof attachment");
        }
        for attachment in function.metadata.clone() {
            if attachment.kind != "prof" {
                continue;
            }
            let named = self
                .resolve(&attachment.node)
                .and_then(|node| node.as_tuple().map(<[MdOperand]>::to_vec))
                .is_some_and(|operands| matches!(operands.first(), Some(MdOperand::String(_))));
            if !named {
                self.report("a !prof attachment on a function starts with the annotation's name");
            }
        }
        let kcfi = function
            .metadata
            .iter()
            .filter(|attachment| attachment.kind == "kcfi_type")
            .count();
        if kcfi > 1 {
            self.report("a function may have only one !kcfi_type attachment");
        }

        // token and x86_amx have no representation a caller could pass, so
        // only an intrinsic may name them in its signature.
        let intrinsic = matches!(&function.name, Name::Named(text) if text.starts_with("llvm."));
        if intrinsic && function.is_definition() {
            self.report("an intrinsic is provided by the compiler and cannot be defined");
        }
        if !intrinsic {
            let return_type = function.return_type;
            let params: Vec<TypeId> = function.params.iter().map(|p| p.ty).collect();
            if let Some(what) = self.opaque_value_type(return_type) {
                self.report(format!("function returns a {what} but is not an intrinsic"));
            }
            for param in params {
                if let Some(what) = self.opaque_value_type(param) {
                    self.report(format!("function takes a {what} but is not an intrinsic"));
                }
            }
        }
        // A parameter holds a value the caller passes, and a label is a
        // place in this function rather than a value at all. Nor can a caller
        // pass a struct whose body this module has never seen: there is no
        // knowing how much of it to copy. A return type may be one, the
        // caller having only to name the place it goes.
        for (index, param) in function.params.iter().enumerate() {
            let opaque = matches!(self.module.ctx.type_kind(param.ty),
                TypeKind::NamedStruct(id) if self.module.ctx.struct_def(*id).fields.is_none());
            if opaque
                || matches!(
                    self.module.ctx.type_kind(param.ty),
                    TypeKind::Label | TypeKind::Function { .. }
                )
            {
                self.report(format!("parameter {index} has a type no caller can pass"));
            }
        }

        self.calling_convention(function);
        self.x86_interrupt(function);
        self.sme_state(&function.attrs);
        self.function_attributes(function);
        if function.comdat.is_some() {
            let name = describe(&function.name);
            self.comdat_member(&name, !function.is_definition());
        }
        self.attribute_set(
            &function.return_attrs,
            function.return_type,
            "the return value",
        );
        // `immarg` says an argument is written as a literal, so a result
        // cannot be one, and `builtin` describes a call rather than a value.
        for kind in [EnumAttr::ImmArg, EnumAttr::Builtin, EnumAttr::SafeStack] {
            if function.return_attrs.has(kind) {
                self.report(format!(
                    "{} on the return value, which it does not describe",
                    kind.keyword()
                ));
            }
        }
        self.attributes_apply_to_a_result(&function.return_attrs);
        for (index, param) in function.params.iter().enumerate() {
            self.attribute_set(&param.attrs, param.ty, &format!("parameter {index}"));
            for kind in [
                EnumAttr::MustProgress,
                EnumAttr::NoUnwind,
                EnumAttr::WillReturn,
                EnumAttr::NoInline,
                EnumAttr::AlwaysInline,
                EnumAttr::OptNone,
                EnumAttr::Cold,
                EnumAttr::Hot,
            ] {
                if param.attrs.has(kind) {
                    self.report(format!(
                        "{} on parameter {index}, which describes a function",
                        kind.keyword()
                    ));
                }
            }
            if param.attrs.has(EnumAttr::ImmArg) {
                let placed = param.attrs.has(EnumAttr::InReg)
                    || param.attrs.has(EnumAttr::Nest)
                    || param.attrs.attributes.iter().any(|attribute| {
                        matches!(
                            attribute,
                            Attribute::Type {
                                kind: TypeAttr::ByVal
                                    | TypeAttr::ByRef
                                    | TypeAttr::InAlloca
                                    | TypeAttr::Preallocated
                                    | TypeAttr::StructRet,
                                ..
                            }
                        )
                    });
                if placed {
                    self.report(format!(
                        "immarg on parameter {index} alongside an attribute that places it"
                    ));
                }
            }
            if param.attrs.has(EnumAttr::ImmArg) && !intrinsic {
                self.report(format!(
                    "immarg on parameter {index} of a function that is not an intrinsic"
                ));
            }
            if param.attrs.has(EnumAttr::SafeStack) {
                self.report(format!(
                    "safestack on parameter {index}, which it does not describe"
                ));
            }
        }
        self.at_most_one_of(function);

        if !function.is_definition() {
            self.function = None;
            return;
        }

        let blocks: Vec<BlockId> = function.block_order.clone();
        for block_id in &blocks {
            self.basic_block(function, *block_id);
        }
        self.funclet_tokens(function);
        // Catching an exception needs a personality routine to decide what
        // was thrown, and a landing pad needs an edge that lands on it.
        let unwind_targets: HashSet<BlockId> = blocks
            .iter()
            .flat_map(|block| function.block(*block).instructions.clone())
            .filter_map(|inst| function.try_instruction(inst))
            .filter_map(|instruction| match instruction.kind {
                InstKind::Invoke { unwind, .. } => Some(unwind),
                _ => None,
            })
            .collect();
        for block_id in &blocks {
            for inst in function.block(*block_id).instructions.clone() {
                let Some(instruction) = function.try_instruction(inst) else {
                    continue;
                };
                let pad = matches!(
                    instruction.kind,
                    InstKind::LandingPad { .. }
                        | InstKind::CatchPad { .. }
                        | InstKind::CleanupPad { .. }
                        | InstKind::CatchSwitch { .. }
                );
                if pad && Some(*block_id) == function.block_order.first().copied() {
                    self.report(format!(
                        "{} opens the entry block, which is reached without unwinding",
                        instruction.kind.opcode()
                    ));
                }
                // A catchswitch names the blocks a throw may land in, and
                // landing in one means running its catchpad.
                if let InstKind::CatchSwitch { handlers, .. } = &instruction.kind {
                    for handler in handlers {
                        let opens_with_a_catchpad = function
                            .block(*handler)
                            .instructions
                            .iter()
                            .filter_map(|inst| function.try_instruction(*inst))
                            .any(|first| matches!(first.kind, InstKind::CatchPad { .. }));
                        if !opens_with_a_catchpad {
                            self.report(format!(
                                "a catchswitch in {} hands to {}, which has no catchpad",
                                describe_block(function, *block_id),
                                describe_block(function, *handler)
                            ));
                        }
                    }
                }
                let catching = pad || matches!(instruction.kind, InstKind::Resume { .. });
                if catching && function.personality.is_none() {
                    self.report(format!(
                        "{} in {} needs a personality routine on its function",
                        instruction.kind.opcode(),
                        describe_block(function, *block_id)
                    ));
                }
                if matches!(instruction.kind, InstKind::LandingPad { .. })
                    && !unwind_targets.contains(block_id)
                {
                    self.report(format!(
                        "a landingpad in {} sits in a block nothing unwinds to",
                        describe_block(function, *block_id)
                    ));
                }
            }
        }
        // Every block a terminator names is one this function writes. It
        // needs the slot-numbered blocks to resolve, which is why it could
        // not be turned on before they did.
        for block_id in &blocks {
            for inst in function.block(*block_id).instructions.clone() {
                let Some(instruction) = function.try_instruction(inst) else {
                    continue;
                };
                for target in instruction.kind.successors() {
                    if !function.block_order.contains(&target) {
                        self.report(format!(
                            "a terminator in {} names a block this function does not define",
                            describe_block(function, *block_id)
                        ));
                    }
                }
            }
        }
        self.dominance(function);
        // An invoke's unwind edge lands on a pad. Nothing else can receive
        // the exception, so nothing else may start that block.
        for block_id in &blocks {
            for inst in function.block(*block_id).instructions.clone() {
                let Some(instruction) = function.try_instruction(inst) else {
                    continue;
                };
                let InstKind::Invoke { unwind, .. } = instruction.kind else {
                    continue;
                };
                let lands_on_a_pad = function
                    .block(unwind)
                    .instructions
                    .first()
                    .and_then(|first| function.try_instruction(*first))
                    .is_some_and(|first| {
                        matches!(
                            first.kind,
                            InstKind::LandingPad { .. }
                                | InstKind::CatchSwitch { .. }
                                | InstKind::CleanupPad { .. }
                        )
                    });
                if !lands_on_a_pad {
                    self.report(format!(
                        "the unwind destination of an invoke in {} does not begin with a pad",
                        describe_block(function, *block_id)
                    ));
                }
            }
        }
        // `va_start` walks the arguments past the declared ones, and a
        // function without a `...` has none.
        if !function.is_var_arg {
            let starts = blocks.iter().any(|block| {
                function
                    .block(*block)
                    .instructions
                    .iter()
                    .any(|inst| self.calls_intrinsic(function, *inst, "llvm.va_start"))
            });
            if starts {
                self.report(
                    "llvm.va_start is called in a function that takes no variable arguments",
                );
            }
        }
        // A naked function has no prologue, so nothing put its arguments
        // anywhere the body could read them.
        if has_enum_attribute(&function.attrs, EnumAttr::Naked) {
            let reads_one = blocks.iter().any(|block| {
                function
                    .block(*block)
                    .instructions
                    .iter()
                    .filter_map(|inst| function.try_instruction(*inst))
                    .any(|instruction| {
                        instruction
                            .kind
                            .operand_values()
                            .iter()
                            .any(|value| matches!(value, Value::Argument(_)))
                    })
            });
            if reads_one {
                self.report("a naked function reads an argument it was never given");
            }
        }
        // A collector has to be named before its barriers mean anything.
        if function.gc.is_none() {
            let uses_gc = blocks.iter().any(|block| {
                function.block(*block).instructions.iter().any(|inst| {
                    ["llvm.gcroot", "llvm.gcread", "llvm.gcwrite"]
                        .iter()
                        .any(|name| self.calls_intrinsic(function, *inst, name))
                })
            });
            if uses_gc {
                self.report("a gc barrier is used in a function that names no collector");
            }
        }
        // A frame's escaped locals are described once, because the recover
        // side indexes into that one list.
        let escapes = blocks
            .iter()
            .flat_map(|block| function.block(*block).instructions.clone())
            .filter(|inst| self.calls_intrinsic(function, *inst, "llvm.localescape"))
            .count();
        if escapes > 1 {
            self.report(format!(
                "{escapes} calls to llvm.localescape in one function, which allows one"
            ));
        }
        self.function = None;
    }

    /// A call inside a funclet says which funclet it is in, when the
    /// intrinsic it calls is one upstream may lower into a real call.
    ///
    /// Windows exception handling runs a catch or a cleanup as a funclet of
    /// its own, and the unwinder needs to know which one a call belongs to.
    /// Most intrinsics need no bundle, an ordinary call needs none either,
    /// and `crates/llvm-ir/src/intrinsic/funclet.rs` is the measured set of
    /// the ones that do.
    ///
    /// Which blocks are inside a funclet is a colouring: every block reached
    /// from one a pad opens, stopping where a `catchret` or a `cleanupret`
    /// leaves it. A block reached both ways is a module upstream refuses for
    /// its colouring rather than for this.
    fn funclet_tokens(&mut self, function: &Function) {
        let opens_a_funclet = |block: BlockId| {
            function
                .block(block)
                .instructions
                .iter()
                .filter_map(|inst| function.try_instruction(*inst))
                .any(|instruction| {
                    matches!(
                        instruction.kind,
                        InstKind::CatchPad { .. } | InstKind::CleanupPad { .. }
                    )
                })
        };
        let mut inside: HashSet<BlockId> = HashSet::new();
        let mut pending: Vec<BlockId> = function
            .block_order
            .iter()
            .copied()
            .filter(|block| opens_a_funclet(*block))
            .collect();
        while let Some(block) = pending.pop() {
            if !inside.insert(block) {
                continue;
            }
            let Some(terminator) = function.block(block).terminator() else {
                continue;
            };
            let Some(instruction) = function.try_instruction(terminator) else {
                continue;
            };
            // Both of these hand control back out of the funclet, so what
            // they name is outside it.
            if matches!(
                instruction.kind,
                InstKind::CatchRet { .. } | InstKind::CleanupRet { .. }
            ) {
                continue;
            }
            pending.extend(instruction.kind.successors());
        }
        for block in inside {
            for inst in function.block(block).instructions.clone() {
                let Some(instruction) = function.try_instruction(inst) else {
                    continue;
                };
                let InstKind::Call(call) = &instruction.kind else {
                    continue;
                };
                if call.bundles.iter().any(|bundle| bundle.tag == "funclet") {
                    continue;
                }
                let Value::Constant(id) = call.callee else {
                    continue;
                };
                let Some(GlobalRef::Function(callee)) = self.module.ctx.constant(id).as_global()
                else {
                    continue;
                };
                let Name::Named(name) = &self.module.function(callee).name else {
                    continue;
                };
                if crate::intrinsic::funclet::needs_funclet_token(name) {
                    self.report(format!(
                        "a call to {name} in {} is missing its funclet token",
                        describe_block(function, block)
                    ));
                }
            }
        }
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
            for attachment in instruction.metadata.clone() {
                self.attachment_resolves(&attachment.node, &format!("an instruction in {label}"));
                self.attachment_keeps_locations_out(
                    &attachment,
                    &format!("an instruction in {label}"),
                );
            }
            self.instruction(function, id, inst_id);
        }
    }

    fn instruction(&mut self, function: &Function, block: BlockId, id: InstId) {
        let this_instruction = id;
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

        // Several well-known attachments only mean something on particular
        // instructions, and upstream says so rather than ignoring them where
        // they cannot apply.
        let is_load = matches!(instruction.kind, InstKind::Load { .. });
        let is_pointer = matches!(self.module.ctx.type_kind(ty), TypeKind::Pointer { .. });
        let takes_range = matches!(
            instruction.kind,
            InstKind::Load { .. } | InstKind::Call(_) | InstKind::Invoke { .. }
        );
        let is_inttoptr = matches!(
            instruction.kind,
            InstKind::Cast {
                op: CastOp::IntToPtr,
                ..
            }
        );
        for attachment in instruction.metadata.clone() {
            let kind = attachment.kind.clone();
            match kind.as_str().unwrap_or_default() {
                _ if {
                    let node = self.resolve(&attachment.node);
                    if let Some(node) = node {
                        self.metadata_shape(&node, &where_);
                    }
                    false
                } => {}
                "alias.scope" | "noalias" => self.scope_list(&attachment.node, &where_),
                // A type-based alias tag names a base type, an access type
                // and an offset, and may add a flag saying the access is to
                // constant memory. Nothing else is a tag.
                "tbaa" => {
                    let operands = self
                        .resolve(&attachment.node)
                        .and_then(|node| node.as_tuple().map(<[MdOperand]>::to_vec));
                    if let Some(operands) = operands {
                        if !matches!(operands.len(), 3 | 4) {
                            self.report(format!(
                                "{where_}: a tbaa tag has three operands or four, not {}",
                                operands.len()
                            ));
                        } else {
                            self.tbaa_types(&operands, &where_);
                        }
                    }
                }
                "fpmath" => {
                    let float = self
                        .value_type(function, Value::Instruction(id))
                        .is_some_and(|ty| {
                            matches!(
                                self.module.ctx.type_kind(self.innermost_element(ty)),
                                TypeKind::Float(_)
                            )
                        });
                    self.check(
                        float,
                        format!("{where_}: fpmath requires a floating-point result"),
                    );
                }
                // A memory-model annotation belongs on something that
                // touches memory.
                "mmra" => self.check(
                    matches!(
                        instruction.kind,
                        InstKind::Load { .. }
                            | InstKind::Store { .. }
                            | InstKind::AtomicRmw { .. }
                            | InstKind::CmpXchg { .. }
                            | InstKind::Fence { .. }
                            | InstKind::Call(_)
                            | InstKind::Invoke { .. }
                    ),
                    format!("{where_}: mmra is attached to an instruction that takes none"),
                ),
                "annotation" | "memprof" => {
                    let empty = self
                        .resolve(&attachment.node)
                        .and_then(|node| node.as_tuple().map(<[MdOperand]>::to_vec))
                        .is_some_and(|operands| operands.is_empty());
                    self.check(
                        !empty,
                        format!("{where_}: {kind} needs at least one operand"),
                    );
                }
                "noalias.addrspace" => {
                    let ranges = self
                        .resolve(&attachment.node)
                        .and_then(|node| node.as_tuple().map(<[MdOperand]>::to_vec));
                    if let Some(operands) = ranges
                        && (operands.is_empty() || operands.len() % 2 != 0)
                    {
                        self.report(format!(
                            "{where_}: noalias.addrspace takes ranges of two values"
                        ));
                    }
                }
                "invariant.group" => self.check(
                    matches!(
                        instruction.kind,
                        InstKind::Load { .. } | InstKind::Store { .. }
                    ),
                    format!("{where_}: invariant.group is only for loads and stores"),
                ),
                "llvm.access.group" => self.access_group(&attachment.node, &where_),
                "align" => self.check(
                    is_load && is_pointer,
                    format!("{where_}: align applies only to loads of a pointer"),
                ),
                "nonnull" => self.check(
                    is_load && is_pointer,
                    format!("{where_}: nonnull applies only to loads of a pointer"),
                ),
                "range" => self.check(
                    takes_range,
                    format!("{where_}: ranges are only for loads, calls and invokes"),
                ),
                "dereferenceable" | "dereferenceable_or_null" => self.check(
                    is_load || is_inttoptr,
                    format!(
                        "{where_}: dereferenceable applies only to load and inttoptr instructions"
                    ),
                ),
                _ => {}
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
                ..
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
                self.check(
                    self.is_sized(loaded_type),
                    format!("{where_} loads a type with no size"),
                );
                // A load reads, so it can acquire and cannot release.
                if let Some((_, ordering)) = instruction.kind.atomic_ordering() {
                    self.atomic_operand(loaded_type, &where_);
                    self.check(
                        !matches!(ordering, AtomicOrdering::Release | AtomicOrdering::AcqRel),
                        format!("{where_} has an ordering a load cannot have"),
                    );
                }
                // Unreachable from parsed text, where an unwritten alignment
                // is filled in from the data layout, but a module built by
                // hand can still omit one.
                // Only for a type whose alignment could have been computed;
                // a target extension type has no layout here and upstream
                // still accepts a load of one.
                self.check(
                    align.is_some() || self.module.default_align(loaded_type, false).is_none(),
                    format!("{where_} has no alignment"),
                );
            }
            InstKind::AtomicRmw { op, value_type, .. } => {
                let (op, value_type) = (*op, *value_type);
                let kind = self.module.ctx.type_kind(value_type).clone();
                let integer = matches!(kind, TypeKind::Integer(_));
                let float = matches!(kind, TypeKind::Float(_));
                let pointer = matches!(kind, TypeKind::Pointer { .. });
                // Arithmetic on the value needs a value the target can do
                // arithmetic on, and only an exchange takes anything else.
                let wanted = match op {
                    // An exchange moves the value without reading it, but
                    // still one register's worth.
                    AtomicRmwOp::Xchg => integer || float || pointer,
                    // The floating-point ones are the only ones a target
                    // does lane by lane.
                    AtomicRmwOp::FAdd
                    | AtomicRmwOp::FSub
                    | AtomicRmwOp::FMax
                    | AtomicRmwOp::FMin
                    | AtomicRmwOp::FMaximum
                    | AtomicRmwOp::FMinimum => self.is_float_or_float_vector(value_type),
                    _ => integer,
                };
                if !wanted {
                    self.report(format!(
                        "{where_} operates on a type its operation cannot take"
                    ));
                }
                self.atomic_size(value_type, &where_);
            }
            InstKind::Store {
                value_type,
                value,
                atomic,
                ..
            } => {
                if let Some((_, ordering)) = atomic {
                    self.check(
                        matches!(
                            self.module.ctx.type_kind(*value_type),
                            TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Pointer { .. }
                        ),
                        format!("{where_} stores a type an atomic cannot move"),
                    );
                    let value_type = *value_type;
                    self.atomic_operand(value_type, &where_);
                    // A store writes, so it can release and cannot acquire.
                    self.check(
                        !matches!(ordering, AtomicOrdering::Acquire | AtomicOrdering::AcqRel),
                        format!("{where_} has an ordering a store cannot have"),
                    );
                }
                let (value_type, value) = (*value_type, *value);
                self.type_is(function, value_type, value, &where_);
                self.check(
                    self.is_sized(value_type),
                    format!("{where_} stores a type with no size"),
                );
            }
            InstKind::ExtractValue {
                aggregate_type,
                aggregate,
                indices,
            } => {
                let (aggregate_type, aggregate) = (*aggregate_type, *aggregate);
                let indices = indices.clone();
                self.type_is(function, aggregate_type, aggregate, &where_);
                self.constant_indices(aggregate_type, &indices, "extractvalue", &where_);
            }
            InstKind::InsertValue {
                aggregate_type,
                aggregate,
                element_type,
                element,
                indices,
            } => {
                let (aggregate_type, aggregate) = (*aggregate_type, *aggregate);
                let (element_type, element) = (*element_type, *element);
                let indices = indices.clone();
                self.type_is(function, aggregate_type, aggregate, &where_);
                self.type_is(function, element_type, element, &where_);
                self.constant_indices(aggregate_type, &indices, "insertvalue", &where_);
                if let Some(field) = self.aggregate_field(aggregate_type, &indices)
                    && field != element_type
                {
                    self.report(format!("{where_} writes a value the field cannot hold"));
                }
            }
            InstKind::CmpXchg {
                success,
                failure,
                compare_type,
                new,
                ..
            } => {
                let (compare_type, new) = (*compare_type, *new);
                self.type_is(function, compare_type, new, &where_);
                // A compare-and-swap compares bit patterns, and two floats
                // that differ in their bits can still be equal, so upstream
                // takes an integer or a pointer and nothing else.
                self.check(
                    matches!(
                        self.module.ctx.type_kind(compare_type),
                        TypeKind::Integer(_) | TypeKind::Pointer { .. }
                    ),
                    format!("{where_} compares something other than an integer or a pointer"),
                );
                self.atomic_operand(compare_type, &where_);
                let (success, failure) = (*success, *failure);
                // Both orderings have to be at least monotonic, and a failure
                // ordering cannot release anything, because there is nothing
                // to release when the exchange did not happen.
                self.check(
                    success != AtomicOrdering::Unordered,
                    format!("{where_} has an invalid success ordering"),
                );
                self.check(
                    !matches!(
                        failure,
                        AtomicOrdering::Unordered
                            | AtomicOrdering::Release
                            | AtomicOrdering::AcqRel
                    ),
                    format!("{where_} has an invalid failure ordering"),
                );
            }
            InstKind::Alloca {
                allocated_type,
                address_space,
                ..
            } => {
                if let Some(space) = address_space
                    && *space >= 1 << 24
                {
                    self.report(format!(
                        "{where_} names address space {space}, which is too large"
                    ));
                }
                let allocated_type = *allocated_type;
                self.check(
                    matches!(self.module.ctx.type_kind(ty), TypeKind::Pointer { .. }),
                    format!("{where_} does not produce a pointer"),
                );
                self.check(
                    self.is_sized(allocated_type),
                    format!("{where_} has an invalid type for alloca"),
                );
            }
            InstKind::GetElementPtr {
                source_type,
                pointer_type,
                pointer,
                indices,
                ..
            } => {
                let (source_type, pointer_type, pointer) = (*source_type, *pointer_type, *pointer);
                let indices = indices.clone();
                self.type_is(function, pointer_type, pointer, &where_);
                self.check(
                    self.is_pointer_or_pointer_vector(pointer_type),
                    format!("{where_} indexes something that is not a pointer"),
                );
                self.gep_vector_widths(pointer_type, &indices, &where_);
                self.walk_indices(source_type, &indices, &where_);
                // A struct holding a scalable vector has no offset for
                // whatever follows it, so there is no arithmetic to do on one
                // even when the index picks the first field.
                if self.holds_a_scalable_vector(source_type, &mut Vec::new()) {
                    self.report(format!(
                        "{where_} cannot target a structure that contains a scalable vector"
                    ));
                }
            }
            InstKind::Fence { ordering, .. } => {
                self.check(
                    matches!(
                        ordering,
                        AtomicOrdering::Acquire
                            | AtomicOrdering::Release
                            | AtomicOrdering::AcqRel
                            | AtomicOrdering::SeqCst
                    ),
                    format!("{where_} has an ordering that orders nothing"),
                );
            }
            InstKind::InsertElement {
                vector_type,
                element_type,
                ..
            } => {
                let (vector_type, element_type) = (*vector_type, *element_type);
                if let Some((wanted, _, _)) =
                    TypeKind::as_vector(self.module.ctx.type_kind(vector_type))
                    && wanted != element_type
                {
                    self.report(format!("{where_} inserts a type the vector does not hold"));
                }
            }
            InstKind::ShuffleVector {
                vector_type, mask, ..
            } => {
                // A mask picks lanes out of the two operands laid end to end,
                // so an index past the second one picks nothing. `undef` and
                // `poison` mean the lane does not matter and are not indices.
                let vector_type = *vector_type;
                let mask = *mask;
                let lanes = TypeKind::as_vector(self.module.ctx.type_kind(vector_type))
                    .filter(|(_, _, scalable)| !scalable)
                    .map(|(_, lanes, _)| lanes);
                if let (Some(lanes), Value::Constant(id)) = (lanes, &mask)
                    && let Constant::Vector { elements, .. } = self.module.ctx.constant(*id).clone()
                {
                    let reach = lanes * 2;
                    for element in elements {
                        let constant = self.module.ctx.constant(element).clone();
                        if matches!(constant, Constant::Undef(_) | Constant::Poison(_)) {
                            continue;
                        }
                        let Some(index) = constant.as_integer() else {
                            continue;
                        };
                        if index.is_negative() || index.to_u64().is_none_or(|n| n >= reach) {
                            self.report(format!(
                                "{where_} picks a lane the two vectors together do not have"
                            ));
                        }
                    }
                }
            }
            InstKind::Phi { incoming, .. } if !self.module.ctx.type_kind(ty).is_first_class() => {
                let _ = incoming;
                self.report(format!("{where_} produces a type no register can hold"));
            }
            InstKind::Phi { incoming, .. } => {
                let incoming = incoming.clone();
                self.check(
                    !matches!(self.module.ctx.type_kind(ty), TypeKind::Token),
                    format!("{where_}: phi values cannot have token type"),
                );
                for (value, _) in &incoming {
                    self.type_is(function, ty, *value, &where_);
                }
                // One entry per edge, not per predecessor: `br i1 %c, label
                // %b, label %b` arrives twice and a switch may arrive many
                // times, and upstream counts each arrival.
                let mut named: Vec<BlockId> = incoming.iter().map(|(_, b)| *b).collect();
                named.sort_by_key(|b| b.0);
                let mut wanted = predecessor_edges(function, block);
                wanted.sort_by_key(|b| b.0);
                self.check(
                    named == wanted,
                    format!("{where_} does not name exactly its block's incoming edges"),
                );
                // Two edges from one block arrive at the same time, so they
                // cannot disagree about what the phi holds.
                for (index, (value, from)) in incoming.iter().enumerate() {
                    let disagrees = incoming[..index]
                        .iter()
                        .any(|(earlier, block)| block == from && earlier != value);
                    if disagrees {
                        self.report(format!(
                            "{where_} gives {} two values for one arrival",
                            describe_block(function, *from)
                        ));
                    }
                }
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
                // A select picks lane by lane, so the condition has as many
                // lanes as the values it picks between.
                let lanes = |verifier: &Self, ty: TypeId| {
                    TypeKind::as_vector(verifier.module.ctx.type_kind(ty)).map(|(_, n, s)| (n, s))
                };
                if lanes(self, condition_type) != lanes(self, ty) {
                    self.report(format!("{where_} picks with a condition of another width"));
                }
                self.type_is(function, ty, if_true, &where_);
                self.type_is(function, ty, if_false, &where_);
                self.check(
                    !matches!(self.module.ctx.type_kind(ty), TypeKind::Token),
                    format!("{where_}: select values cannot have token type"),
                );
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
                // What crosses a call boundary is laid out by the target,
                // and an alignment past what the encoding holds is one the
                // target cannot ask for. An intrinsic is exempt: it is
                // lowered rather than called, so nothing has to be placed.
                let lowered = match call.callee {
                    Value::Constant(id) => self.mentions_an_intrinsic(id).is_some(),
                    _ => false,
                };
                if !lowered {
                    let result = self.module.ctx.type_kind(call.function_type).clone();
                    if let TypeKind::Function { result, .. } = result
                        && self.wants_an_unrepresentable_alignment(result, &mut Vec::new())
                    {
                        self.report(format!("{where_} returns a type it cannot align"));
                    }
                    for arg in &call.args {
                        if self.wants_an_unrepresentable_alignment(arg.ty, &mut Vec::new()) {
                            self.report(format!("{where_} passes a type it cannot align"));
                        }
                    }
                }
                for arg in &call.args {
                    let Value::Constant(id) = arg.value else {
                        continue;
                    };
                    let Constant::Metadata { operand, .. } = self.module.ctx.constant(id).clone()
                    else {
                        continue;
                    };
                    let MdOperand::String(text) = *operand else {
                        continue;
                    };
                    let Some(word) = text.as_str() else {
                        continue;
                    };
                    let known = match word.split_once('.') {
                        Some(("round", _)) => matches!(
                            word,
                            "round.dynamic"
                                | "round.tonearest"
                                | "round.downward"
                                | "round.upward"
                                | "round.towardzero"
                                | "round.tonearestaway"
                        ),
                        Some(("fpexcept", _)) => {
                            matches!(
                                word,
                                "fpexcept.ignore" | "fpexcept.maytrap" | "fpexcept.strict"
                            )
                        }
                        _ => true,
                    };
                    if !known {
                        self.report(format!("{where_} names {word}, which is not one of them"));
                    }
                }
                // How many parameters the callee declares, which is where
                // the variadic part of the argument list starts.
                //
                // A statepoint is the exception, its variadic part being the
                // wrapped call's own arguments rather than arguments of its
                // own, so an `sret` there names a place the wrapped callee
                // did declare. Upstream reads its own statepoint.ll.
                let forwards_its_arguments = match call.callee {
                    Value::Constant(id) => self
                        .mentions_an_intrinsic(id)
                        .is_some_and(|name| name.starts_with("llvm.experimental.gc.statepoint")),
                    _ => false,
                };
                let declared = match self.module.ctx.type_kind(call.function_type) {
                    _ if forwards_its_arguments => call.args.len(),
                    TypeKind::Function { params, .. } => params.len(),
                    _ => call.args.len(),
                };
                for (position, arg) in call.args.iter().enumerate() {
                    let attrs = arg.attrs.clone();
                    self.attribute_set(&attrs, arg.ty, &format!("argument {position} of {where_}"));
                    // `inalloca` says the argument was pushed onto the stack
                    // where the callee expects to find it, which is what
                    // `alloca inalloca` does and what an ordinary alloca does
                    // not. A value that is not an alloca at all says nothing
                    // about where it came from, so nothing is asked of it.
                    if has_type_attribute(&attrs, TypeAttr::InAlloca)
                        && let Value::Instruction(id) = arg.value
                        && let Some(instruction) = function.try_instruction(id)
                        && let InstKind::Alloca { inalloca, .. } = instruction.kind
                        && !inalloca
                    {
                        self.report(format!(
                            "{where_} passes argument {position} inalloca from an alloca that is \
                             not one"
                        ));
                    }
                    // An argument past the declared parameters is one the
                    // callee has no name for, so nothing can be said about
                    // where it goes or what it comes back as.
                    if position >= declared {
                        if has_type_attribute(&attrs, TypeAttr::StructRet) {
                            self.report(format!(
                                "{where_} marks a variadic argument sret, which names a place the \
                                 callee did not declare"
                            ));
                        }
                        if has_enum_attribute(&attrs, EnumAttr::Returned) {
                            self.report(format!(
                                "{where_} marks a variadic argument returned, which promises \
                                 something the callee did not declare"
                            ));
                        }
                    }
                    // The inalloca argument is the one the callee finds on
                    // the stack, so it is the one pushed last.
                    if has_type_attribute(&attrs, TypeAttr::InAlloca)
                        && position + 1 != call.args.len()
                    {
                        self.report(format!(
                            "{where_} has inalloca on an argument that is not the last"
                        ));
                    }
                }
                if call.tail == TailKind::MustTail && ty != function.return_type {
                    self.report(format!(
                        "{where_} is a musttail call that returns something its caller does not"
                    ));
                }
                if call.tail == TailKind::MustTail && call.calling_conv != function.calling_conv {
                    self.report(format!(
                        "{where_} is a musttail call whose convention differs from its caller's"
                    ));
                }
                // The tail conventions hand the frame over whole, and an
                // argument in a register is not part of the frame, so there
                // is nowhere for it to go. Both ends are checked: the
                // caller's own parameters and the call's arguments.
                if call.tail == TailKind::MustTail
                    && matches!(
                        function.calling_conv,
                        CallingConv::Named(NamedCallingConv::Tail | NamedCallingConv::SwiftTail)
                    )
                {
                    let in_register = function
                        .params
                        .iter()
                        .any(|param| has_enum_attribute(&param.attrs, EnumAttr::InReg))
                        || call
                            .args
                            .iter()
                            .any(|arg| has_enum_attribute(&arg.attrs, EnumAttr::InReg));
                    if in_register {
                        self.report(format!(
                            "{where_} is a musttail call in a tail convention, where inreg has nowhere to go"
                        ));
                    }
                }
                // What the constraint string promises, the call has to
                // provide: one `!` for each label it can jump to, and an
                // `elementtype` on every operand an indirect constraint
                // reaches through.
                if let Value::Constant(id) = call.callee
                    && let Constant::InlineAsm(asm) = self.module.ctx.constant(id).clone()
                {
                    let entries: Vec<&str> = asm.constraints.split(',').collect();
                    let labels = entries
                        .iter()
                        .filter(|entry| entry.starts_with('!'))
                        .count();
                    if let InstKind::CallBr { indirect, .. } = &instruction.kind
                        && labels != indirect.len()
                    {
                        self.report(format!(
                            "{where_} has {labels} label constraints for {} indirect labels",
                            indirect.len()
                        ));
                    }
                    // An input constraint consumes an argument; an output one
                    // that is not `=*` does not, so walk them together.
                    let mut argument = 0;
                    for entry in entries {
                        let indirect_operand = entry.contains('*');
                        let consumes = indirect_operand || !entry.starts_with('=');
                        if !consumes {
                            continue;
                        }
                        if let Some(arg) = call.args.get(argument)
                            && indirect_operand
                            && !has_type_attribute(&arg.attrs, TypeAttr::ElementType)
                        {
                            self.report(format!(
                                "{where_} passes argument {argument} to an indirect constraint without elementtype"
                            ));
                        }
                        argument += 1;
                    }
                }
                // Some bundle tags say one thing about the call, so a second
                // one of the same tag has nothing left to say.
                for tag in ["kcfi", "ptrauth", "deopt", "funclet", "gc-transition"] {
                    let count = call
                        .bundles
                        .iter()
                        .filter(|bundle| bundle.tag == tag)
                        .count();
                    if count > 1 {
                        self.report(format!("{where_} carries {count} {tag} operand bundles"));
                    }
                }
                // What `llvm.assume` is told is written as a bundle whose tag
                // is the attribute being asserted, so the tag has to name one.
                // Two tags are its own rather than an attribute's: `ignore`
                // stands for an assertion that was dropped, and
                // `separate_storage` names two allocations that do not
                // overlap.
                let asserts = self
                    .resolve_symbol_of(call.callee)
                    .and_then(|target| match target {
                        GlobalRef::Function(id) => match &self.module.function(id).name {
                            Name::Named(name) => Some(name.clone()),
                            _ => None,
                        },
                        _ => None,
                    })
                    .is_some_and(|name| name == "llvm.assume");
                if asserts {
                    for bundle in call.bundles.clone() {
                        self.assume_bundle(&bundle, &where_);
                    }
                }
                let intrinsic = match call.callee {
                    Value::Constant(id) => self.mentions_an_intrinsic(id),
                    _ => None,
                };
                if let Some(name) = intrinsic {
                    // A tile is a register the hardware fills, so there is no
                    // constant of that type for a caller to hand over: a call
                    // takes one another call produced.
                    for (position, arg) in call.args.iter().enumerate() {
                        if matches!(self.module.ctx.type_kind(arg.ty), TypeKind::X86Amx)
                            && matches!(arg.value, Value::Constant(_))
                        {
                            self.report(format!(
                                "{where_}: argument {position} is a constant x86_amx"
                            ));
                        }
                    }
                    // An intrinsic is selected by its name, so its argument
                    // list is the one the name owns rather than one a caller
                    // chooses. Calling one through a variadic type says the
                    // caller chose, which is only true of the few intrinsics
                    // that are variadic. LangRef declares those with `...`,
                    // and `corpus/intrinsic-signatures.nu` drops exactly
                    // those, so having a signature at all is the evidence.
                    if crate::intrinsic::table::signature(&name).is_some()
                        && matches!(
                            self.module.ctx.type_kind(call.function_type),
                            TypeKind::Function {
                                is_var_arg: true,
                                ..
                            }
                        )
                    {
                        self.report(format!(
                            "{where_} calls @{name} through a variadic signature, which it does \
                             not have"
                        ));
                    }
                    // Relocating a pointer, or reading what a call returned,
                    // is asking about one safepoint, and a statepoint is what
                    // makes one. `token none` marks no point at all.
                    let asks_about_a_safepoint = name.starts_with("llvm.experimental.gc.relocate")
                        || name.starts_with("llvm.experimental.gc.result");
                    if asks_about_a_safepoint
                        && let Some(first) = call.args.first()
                        && !self.comes_from_a_statepoint(function, first.value)
                    {
                        self.report(format!(
                            "{where_} is incorrectly tied to the statepoint it names"
                        ));
                    }
                }
                for attribute in &call.fn_attrs.attributes {
                    // Not the type-valued ones: `preallocated(T)` is a
                    // call-site function attribute naming the setup it pairs
                    // with, and upstream reads it.
                    if matches!(
                        attribute,
                        Attribute::Int {
                            kind: IntAttr::Align
                                | IntAttr::Dereferenceable
                                | IntAttr::DereferenceableOrNull,
                            ..
                        }
                    ) {
                        self.report(format!(
                            "{where_} carries {}, which describes an argument rather than a call",
                            describe_attribute(attribute)
                        ));
                    }
                }
                // `speculatable` promises the call can be moved to somewhere
                // it might not have run, and that is a promise the callee
                // makes about itself. A call site may repeat it and only
                // repeat it: an indirect call has no declaration to have made
                // it, and an alias is a name rather than a body.
                if self
                    .resolved_attributes(&call.fn_attrs)
                    .iter()
                    .any(|a| matches!(a, Attribute::Enum(EnumAttr::Speculatable)))
                {
                    let promised = self
                        .resolve_symbol_of(call.callee)
                        .and_then(|target| match target {
                            GlobalRef::Function(id) => Some(self.module.function(id).attrs.clone()),
                            _ => None,
                        })
                        .is_some_and(|attrs| {
                            self.resolved_attributes(&attrs)
                                .iter()
                                .any(|a| matches!(a, Attribute::Enum(EnumAttr::Speculatable)))
                        });
                    if !promised {
                        self.report(format!(
                            "{where_} carries speculatable, which its callee does not"
                        ));
                    }
                }
                // An indirect call has no declaration to name an opaque type
                // in, so it may not produce one.
                // Only a genuinely indirect call: a named symbol carries its
                // own address space, and upstream reads a call to an ifunc in
                // address space zero from a program space that is not.
                let names_a_symbol = matches!(call.callee, Value::Constant(id)
                    if self.module.ctx.constant(id).as_global().is_some());
                if !names_a_symbol
                    && let Some(space) = self
                        .value_type(function, call.callee)
                        .and_then(|ty| self.module.ctx.type_kind(ty).pointer_address_space())
                {
                    // A call may write the space it goes through, and then
                    // that is what the callee has to be in. Written or not,
                    // the two have to agree; unwritten it is the program's.
                    // A module that writes no layout gets the default one,
                    // whose program space is zero, so there is always a
                    // space to compare against.
                    let wanted = match call.address_space {
                        Some(written) => written,
                        None => self
                            .module
                            .data_layout
                            .clone()
                            .unwrap_or_default()
                            .program_address_space(),
                    };
                    {
                        if space != wanted {
                            self.report(format!(
                                "{where_} calls through address space {space} rather than {wanted}"
                            ));
                        }
                    }
                }
                let direct = matches!(call.callee, Value::Constant(id)
                    if matches!(self.module.ctx.constant(id).as_global(), Some(GlobalRef::Function(_))));
                if !direct && let Some(what) = self.opaque_value_type(ty) {
                    self.report(format!("{where_} returns a {what} from an indirect call"));
                }
                let return_attrs = call.return_attrs.clone();
                self.attribute_set(&return_attrs, ty, &format!("the result of {where_}"));
                // A chain call replaces the current wave rather than
                // returning to it, so it is only ever a tail call.
                if matches!(
                    call.calling_conv,
                    CallingConv::Named(
                        NamedCallingConv::AmdgpuCsChain | NamedCallingConv::AmdgpuCsChainPreserve
                    )
                ) && call.tail != TailKind::MustTail
                {
                    self.report(format!(
                        "{where_} uses a convention that does not permit calls"
                    ));
                }
                // A call to an ordinary function is deliberately *not*
                // compared against that function's declared signature: opaque
                // pointers put the signature at the call site, and real
                // llvm-as accepts `call void @g()` against `declare void
                // @g(i32)`. An intrinsic is different, because it is selected
                // by its name and mangled suffix together, so a call with
                // another signature is a call to something that does not
                // exist.
                if let Value::Constant(id) = call.callee
                    && let Some(GlobalRef::Function(callee)) =
                        self.module.ctx.constant(id).as_global()
                {
                    let callee = self.module.function(callee);
                    let is_intrinsic =
                        matches!(&callee.name, Name::Named(name) if name.starts_with("llvm."));
                    let immediate: Vec<bool> = callee
                        .params
                        .iter()
                        .map(|param| param.attrs.has(EnumAttr::ImmArg))
                        .collect();
                    // An `immarg` parameter is written as a literal at the
                    // call, so a `range` on it is something the call itself
                    // can be held to rather than a promise about a value
                    // nobody here can see.
                    let ranges: Vec<Option<(ApInt, ApInt)>> =
                        callee
                            .params
                            .iter()
                            .map(|param| {
                                param.attrs.attributes.iter().find_map(
                                    |attribute| match attribute {
                                        Attribute::Range { lower, upper, .. } => {
                                            Some((lower.clone(), upper.clone()))
                                        }
                                        _ => None,
                                    },
                                )
                            })
                            .collect();
                    // The other half of the same reading: positions whose
                    // types vary together across LangRef's instantiations
                    // are one overloaded type, so a call giving two of them
                    // different types calls no instantiation there is.
                    // Upstream verifies this at the call rather than at the
                    // declaration, an unused declaration never being looked
                    // at, which is why this sits here.
                    if is_intrinsic
                        && let Name::Named(intrinsic) = &callee.name
                        && let Some((arity, classes)) = crate::intrinsic::overloads::tied(intrinsic)
                        && arity == call.args.len() + 1
                    {
                        let positions: Vec<TypeId> = std::iter::once(ty)
                            .chain(call.args.iter().map(|arg| arg.ty))
                            .collect();
                        for class in classes {
                            let mut wanted = None;
                            for position in *class {
                                let ty = positions[*position];
                                match wanted {
                                    None => wanted = Some(ty),
                                    Some(first) if first == ty => {}
                                    Some(_) => {
                                        self.report(format!(
                                            "{where_} calls an intrinsic with two types where it takes one"
                                        ));
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    // The same reading on lane counts rather than types: a
                    // mask is `<4 x i1>` where the value it masks is
                    // `<4 x double>`, so the two are one shape without being
                    // one type, and a call giving them different lengths
                    // names an instantiation there is not.
                    if is_intrinsic
                        && let Name::Named(intrinsic) = &callee.name
                        && let Some((arity, classes)) =
                            crate::intrinsic::overloads::tied_lanes(intrinsic)
                        && arity == call.args.len() + 1
                    {
                        let positions: Vec<TypeId> = std::iter::once(ty)
                            .chain(call.args.iter().map(|arg| arg.ty))
                            .collect();
                        for class in classes {
                            let mut wanted = None;
                            for position in *class {
                                let lanes = TypeKind::as_vector(
                                    self.module.ctx.type_kind(positions[*position]),
                                )
                                .map(|(_, count, scalable)| (count, scalable));
                                match wanted {
                                    None => wanted = Some(lanes),
                                    Some(first) if first == lanes => {}
                                    Some(_) => {
                                        self.report(format!(
                                            "{where_} calls an intrinsic with two lane counts where it takes one"
                                        ));
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    // LangRef documents what each intrinsic takes, and the
                    // positions whose type is the same in every documented
                    // instantiation are the ones a call has to get right.
                    if is_intrinsic
                        && let Name::Named(intrinsic) = callee.name.clone()
                        && let Some(documented) = crate::intrinsic::table::signature(&intrinsic)
                    {
                        // Not the argument count: upstream auto-upgrades the
                        // older spelling of an intrinsic, so a call with
                        // fewer arguments than LangRef documents is a module
                        // llvm-as reads and rewrites. Only the positions that
                        // are there get checked.
                        let arguments: Vec<TypeId> = call.args.iter().map(|arg| arg.ty).collect();
                        for (position, (wanted, actual)) in
                            documented.iter().zip(arguments.iter()).enumerate()
                        {
                            let kind = self.module.ctx.type_kind(*actual);
                            let fits = match wanted {
                                Parameter::Any => true,
                                Parameter::Int(bits) => {
                                    matches!(kind, TypeKind::Integer(width) if width == bits)
                                }
                                Parameter::Pointer => matches!(kind, TypeKind::Pointer { .. }),
                                Parameter::Metadata => matches!(kind, TypeKind::Metadata),
                                Parameter::Float => matches!(kind, TypeKind::Float(_)),
                            };
                            if !fits {
                                self.report(format!(
                                    "{where_} passes the wrong type in argument {position} of an intrinsic"
                                ));
                            }
                        }
                    }
                    if is_intrinsic && let Name::Named(intrinsic) = callee.name.clone() {
                        let arguments: Vec<TypeId> = call.args.iter().map(|a| a.ty).collect();
                        let values: Vec<Value> = call.args.iter().map(|a| a.value).collect();
                        let attributes: Vec<AttributeSet> =
                            call.args.iter().map(|a| a.attrs.clone()).collect();
                        let bundles: Vec<String> =
                            call.bundles.iter().map(|b| b.tag.clone()).collect();
                        let returns_next = function
                            .block(block)
                            .instructions
                            .iter()
                            .skip_while(|other| **other != this_instruction)
                            .nth(1)
                            .and_then(|next| function.try_instruction(*next))
                            .is_some_and(|next| matches!(next.kind, InstKind::Ret { .. }));
                        self.intrinsic_rule(
                            &intrinsic,
                            ty,
                            &arguments,
                            &values,
                            &attributes,
                            &bundles,
                            returns_next,
                            &where_,
                        );
                    }
                    if is_intrinsic {
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
                            format!("{where_} calls an intrinsic with an incompatible signature"),
                        );
                    }
                    for arg in &call.args {
                        let Value::Constant(id) = arg.value else {
                            continue;
                        };
                        let Constant::Metadata { operand, .. } =
                            self.module.ctx.constant(id).clone()
                        else {
                            continue;
                        };
                        let MdOperand::String(text) = *operand else {
                            continue;
                        };
                        let Some(word) = text.as_str() else {
                            continue;
                        };
                        let known = match word.split_once('.') {
                            Some(("round", _)) => matches!(
                                word,
                                "round.dynamic"
                                    | "round.tonearest"
                                    | "round.downward"
                                    | "round.upward"
                                    | "round.towardzero"
                                    | "round.tonearestaway"
                            ),
                            Some(("fpexcept", _)) => {
                                matches!(
                                    word,
                                    "fpexcept.ignore" | "fpexcept.maytrap" | "fpexcept.strict"
                                )
                            }
                            _ => true,
                        };
                        if !known {
                            self.report(format!("{where_} names {word}, which is not one of them"));
                        }
                    }
                    for (position, arg) in call.args.iter().enumerate() {
                        // A constant expression counts: upstream folds one
                        // before the verifier looks, so `add (i32 2, i32 3)`
                        // reaches it as the number 5.
                        let immediate_value = match arg.value {
                            Value::Constant(id) => matches!(
                                self.module.ctx.constant(id),
                                Constant::Integer { .. }
                                    | Constant::Float { .. }
                                    | Constant::ZeroInitializer(_)
                                    | Constant::Expression(_)
                            ),
                            _ => false,
                        };
                        if immediate.get(position) == Some(&true) && !immediate_value {
                            self.report(format!(
                                "{where_} passes a non-immediate to an immarg parameter"
                            ));
                        }
                        if immediate.get(position) == Some(&true)
                            && let Some(Some((lower, upper))) = ranges.get(position)
                            && let Value::Constant(id) = arg.value
                            && let Constant::Integer { value, .. } = self.module.ctx.constant(id)
                            && !range_holds(lower, upper, value)
                        {
                            self.report(format!(
                                "{where_} passes {} to an immarg parameter ranged [{}, {})",
                                value.to_string_signed(),
                                lower.to_string_signed(),
                                upper.to_string_signed()
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Walks a `getelementptr` index list through the type it indexes. The
    /// first index steps over the pointee and descends into nothing; a struct
    /// takes only a constant `i32`, since the field it selects decides the
    /// result type.
    /// A getelementptr that indexes lanewise does so with one width. Every
    /// vector among the base and the indices has to agree on how many lanes
    /// there are, and a scalable vector never indexes a struct.
    fn gep_vector_widths(
        &mut self,
        pointer_type: TypeId,
        indices: &[(TypeId, Value)],
        where_: &str,
    ) {
        let mut width: Option<(u64, bool)> = self
            .module
            .ctx
            .type_kind(pointer_type)
            .as_vector()
            .map(|(_, count, scalable)| (count, scalable));
        for (index_type, _) in indices {
            let Some((_, count, scalable)) = self.module.ctx.type_kind(*index_type).as_vector()
            else {
                continue;
            };
            match width {
                Some(seen) if seen != (count, scalable) => {
                    self.report(format!("{where_} has vector indices of different widths"));
                    return;
                }
                Some(_) => {}
                None => width = Some((count, scalable)),
            }
        }
    }

    fn walk_indices(&mut self, source_type: TypeId, indices: &[(TypeId, Value)], where_: &str) {
        let mut current = source_type;
        for (position, (index_type, index)) in indices.iter().enumerate() {
            if position == 0 {
                continue;
            }
            match self.module.ctx.type_kind(current).clone() {
                TypeKind::Array { element, .. } | TypeKind::Vector { element, .. } => {
                    current = element;
                }
                TypeKind::Struct { fields, .. } => {
                    match self.struct_index(*index_type, *index, fields.len(), where_) {
                        Some(field) => current = fields[field],
                        None => return,
                    }
                }
                TypeKind::NamedStruct(id) => {
                    let fields = match &self.module.ctx.struct_def(id).fields {
                        Some(fields) => fields.clone(),
                        None => {
                            self.report(format!("{where_} indexes an opaque struct"));
                            return;
                        }
                    };
                    match self.struct_index(*index_type, *index, fields.len(), where_) {
                        Some(field) => current = fields[field],
                        None => return,
                    }
                }
                _ => {
                    self.report(format!("{where_} has invalid indices"));
                    return;
                }
            }
        }
    }

    /// A struct index, or `None` after reporting why it is not one.
    /// The type at the end of a constant index path, when the path is one
    /// the aggregate actually has.
    fn aggregate_field(&self, aggregate: TypeId, indices: &[u32]) -> Option<TypeId> {
        let mut current = aggregate;
        for index in indices {
            current = match self.module.ctx.type_kind(current).clone() {
                TypeKind::Array { element, count } if u64::from(*index) < count => element,
                TypeKind::Struct { fields, .. } => *fields.get(*index as usize)?,
                TypeKind::NamedStruct(id) => *self
                    .module
                    .ctx
                    .struct_def(id)
                    .fields
                    .as_ref()?
                    .get(*index as usize)?,
                _ => return None,
            };
        }
        Some(current)
    }

    /// The one value a struct index carries, whether it is written as a
    /// number or as a vector every lane of which holds that number.
    fn uniform_index(&self, constant: ConstId) -> Option<u64> {
        match self.module.ctx.constant(constant) {
            Constant::ZeroInitializer(_) => Some(0),
            Constant::Vector { elements, .. } => {
                let first = self.uniform_index(*elements.first()?)?;
                elements
                    .iter()
                    .all(|element| self.uniform_index(*element) == Some(first))
                    .then_some(first)
            }
            other => other.as_integer().and_then(ApInt::to_u64),
        }
    }

    fn struct_index(
        &mut self,
        index_type: TypeId,
        index: Value,
        fields: usize,
        where_: &str,
    ) -> Option<usize> {
        // A gep over a vector of pointers indexes a struct with a vector of
        // i32, every lane of which has to pick the same field. A scalable
        // vector cannot: nothing says what its lanes hold.
        if matches!(
            self.module.ctx.type_kind(index_type),
            TypeKind::Vector { scalable: true, .. }
        ) {
            self.report(format!("{where_} has invalid indices"));
            return None;
        }
        let scalar = self.innermost_element(index_type);
        if !matches!(self.module.ctx.type_kind(scalar), TypeKind::Integer(32)) {
            self.report(format!("{where_} has invalid indices"));
            return None;
        }
        let Value::Constant(constant) = index else {
            self.report(format!("{where_} indexes a struct with a variable"));
            return None;
        };
        let Some(value) = self.uniform_index(constant) else {
            self.report(format!("{where_} has invalid indices"));
            return None;
        };
        if value as usize >= fields {
            self.report(format!("{where_} has invalid indices"));
            return None;
        }
        Some(value as usize)
    }

    /// The literal index list of `extractvalue` and `insertvalue`, which has
    /// to name a field that exists at every level and cannot be empty.
    fn constant_indices(&mut self, aggregate: TypeId, indices: &[u32], opcode: &str, where_: &str) {
        if indices.is_empty() {
            self.report(format!("{where_}: invalid indices for {opcode}"));
            return;
        }
        let mut current = aggregate;
        for index in indices {
            let count = match self.module.ctx.type_kind(current).clone() {
                TypeKind::Array { element, count } => {
                    current = element;
                    count
                }
                TypeKind::Struct { fields, .. } => {
                    let count = fields.len() as u64;
                    if (*index as u64) < count {
                        current = fields[*index as usize];
                    }
                    count
                }
                TypeKind::NamedStruct(id) => {
                    let Some(fields) = self.module.ctx.struct_def(id).fields.clone() else {
                        self.report(format!("{where_}: invalid indices for {opcode}"));
                        return;
                    };
                    let count = fields.len() as u64;
                    if (*index as u64) < count {
                        current = fields[*index as usize];
                    }
                    count
                }
                _ => {
                    self.report(format!("{where_}: invalid indices for {opcode}"));
                    return;
                }
            };
            if u64::from(*index) >= count {
                self.report(format!("{where_}: invalid indices for {opcode}"));
                return;
            }
        }
    }

    /// The node an attachment points at, whether it is named or written in
    /// place.
    fn resolve(&self, node: &MdRef) -> Option<Metadata> {
        match node {
            MdRef::Id(id) => self.module.metadata_node(*id).cloned(),
            MdRef::Inline(inline) => Some((**inline).clone()),
        }
    }

    /// `!alias.scope` and `!noalias` take a list of scopes, and a scope is a
    /// node of two or three operands: its own identity, its domain, and an
    /// optional description.
    fn scope_list(&mut self, node: &MdRef, where_: &str) {
        let Some(list) = self.resolve(node) else {
            return;
        };
        let Some(operands) = list.as_tuple().map(<[MdOperand]>::to_vec) else {
            self.report(format!("{where_}: scope list must consist of MDNodes"));
            return;
        };
        for operand in &operands {
            let scope = match operand {
                MdOperand::Ref(id) => self.module.metadata_node(*id).cloned(),
                MdOperand::Inline(inline) => Some((**inline).clone()),
                _ => {
                    self.report(format!("{where_}: scope list must consist of MDNodes"));
                    continue;
                }
            };
            let Some(scope) = scope else {
                continue;
            };
            match scope.as_tuple() {
                Some(fields) if (2..=3).contains(&fields.len()) => {}
                _ => self.report(format!("{where_}: scope must have two or three operands")),
            }
        }
    }

    /// `!llvm.access.group` is either one group, which is a distinct empty
    /// node, or a list of them.
    fn access_group(&mut self, node: &MdRef, where_: &str) {
        let Some(attached) = self.resolve(node) else {
            return;
        };
        let Some(operands) = attached.as_tuple().map(<[MdOperand]>::to_vec) else {
            self.report(format!(
                "{where_}: access scope list must consist of MDNodes"
            ));
            return;
        };
        // A group itself is distinct and empty; anything else at this level
        // has to be a list of groups.
        if attached.is_distinct() && operands.is_empty() {
            return;
        }
        for operand in &operands {
            let group = match operand {
                MdOperand::Ref(id) => self.module.metadata_node(*id).cloned(),
                MdOperand::Inline(inline) => Some((**inline).clone()),
                _ => {
                    self.report(format!(
                        "{where_}: access scope list must consist of MDNodes"
                    ));
                    continue;
                }
            };
            let valid = group.as_ref().is_some_and(|g| {
                g.is_distinct() && g.as_tuple().is_some_and(<[MdOperand]>::is_empty)
            });
            if !valid {
                self.report(format!(
                    "{where_}: access scope list contains an invalid scope"
                ));
            }
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
            // Crossing address spaces is the whole of what this cast does,
            // so one that stays in its own space is not this cast: upstream
            // calls it an invalid opcode rather than a no-op.
            CastOp::AddrSpaceCast => {
                pointer(self, from)
                    && pointer(self, to)
                    && self.address_space_of(from) != self.address_space_of(to)
            }
            // A bitcast reinterprets the same bits, so it may not cross
            // between a pointer and anything else, may not change an address
            // space (that is what addrspacecast is for), and may not apply to
            // an aggregate.
            CastOp::BitCast => {
                let from_pointer = pointer(self, from);
                let to_pointer = pointer(self, to);
                from_pointer == to_pointer
                    && (!from_pointer || self.address_space_of(from) == self.address_space_of(to))
                    && !self.module.ctx.type_kind(from).is_aggregate()
                    && !self.module.ctx.type_kind(to).is_aggregate()
            }
        };
        self.check(
            ok,
            format!("{where_} casts between the wrong kinds of type"),
        );

        // Every cast but a bitcast works lane by lane, so both sides are
        // vectors of the same width or neither is a vector at all.
        if op != CastOp::BitCast {
            let lanes = |verifier: &Self, ty: TypeId| {
                TypeKind::as_vector(verifier.module.ctx.type_kind(ty)).map(|(_, n, s)| (n, s))
            };
            if lanes(self, from) != lanes(self, to) {
                self.report(format!("{where_} casts between different vector shapes"));
            }
        }

        // Reinterpreting bits cannot change how many there are.
        if op == CastOp::BitCast
            && let (Some(from_bits), Some(to_bits)) =
                (self.size_in_bits(from), self.size_in_bits(to))
        {
            self.check(
                from_bits == to_bits,
                format!("{where_} changes the size of its operand"),
            );
        }

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
        // Dominance says nothing about a block the entry cannot reach, and
        // upstream agrees: `%x = add i32 %x, 1` in dead code is a module
        // llvm-as reads. Restricting the check to reachable blocks is the
        // rule, not a concession.
        let reachable: HashSet<BlockId> = match function.entry_block() {
            Some(entry) => reverse_postorder(function, entry).into_iter().collect(),
            None => HashSet::new(),
        };
        let mut defining_block: HashMap<InstId, BlockId> = HashMap::new();
        let mut position: HashMap<InstId, usize> = HashMap::new();
        for (block_id, block) in function.blocks() {
            for (index, inst) in block.instructions.iter().enumerate() {
                defining_block.insert(*inst, block_id);
                position.insert(*inst, index);
            }
        }

        for (block_id, block) in function.blocks() {
            if !reachable.contains(&block_id) {
                continue;
            }
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

    /// Whether a value of this type can be stored somewhere: allocated on the
    /// stack, held in a global, or placed in an aggregate.
    fn is_sized(&self, ty: TypeId) -> bool {
        self.is_sized_within(ty, &mut Vec::new())
    }

    /// A named struct may name itself, directly or through others, and a
    /// type that contains itself has no size to compute. Upstream refuses
    /// `%s = type { %s }` as a global for that reason, and walking it
    /// without the trail below does not return.
    fn is_sized_within(&self, ty: TypeId, trail: &mut Vec<TypeId>) -> bool {
        if trail.contains(&ty) {
            return false;
        }
        trail.push(ty);
        let sized = self.is_sized_step(ty, trail);
        trail.pop();
        sized
    }

    fn is_sized_step(&self, ty: TypeId, trail: &mut Vec<TypeId>) -> bool {
        match self.module.ctx.type_kind(ty) {
            TypeKind::Void
            | TypeKind::Label
            | TypeKind::Metadata
            | TypeKind::Token
            | TypeKind::Function { .. } => false,
            // A target extension type has whatever properties the target
            // registered it with, and one upstream does not know has none:
            // `target("spirv.Image")` is loaded and stored where
            // `target("foo")` is refused. The table is measured against the
            // assembler by `corpus/target-extension-types.nu`, LangRef
            // saying the properties exist and pointing at a header for the
            // list.
            TypeKind::Target { name, .. } => {
                crate::target_extension::properties(name.as_str()).sized
            }
            // A struct is as sized as its fields, and a body it has not been
            // given yet has no layout at all.
            TypeKind::Struct { fields, .. } => self.struct_is_sized(&fields.clone(), trail),
            TypeKind::NamedStruct(id) => match &self.module.ctx.struct_def(*id).fields {
                Some(fields) => self.struct_is_sized(&fields.clone(), trail),
                None => false,
            },
            TypeKind::Array { element, .. } => self.has_a_fixed_size_within(*element, trail),
            _ => true,
        }
    }

    /// Whether a type reaches itself by value, through struct fields and
    /// array elements. A pointer to it does not count, which is what makes a
    /// linked list legal and `%t = type { %t }` not.
    fn reaches_struct(&self, ty: TypeId, wanted: StructId, trail: &mut Vec<TypeId>) -> bool {
        if trail.contains(&ty) {
            return false;
        }
        trail.push(ty);
        let fields: Vec<TypeId> = match self.module.ctx.type_kind(ty) {
            TypeKind::NamedStruct(id) if *id == wanted => {
                trail.pop();
                return true;
            }
            TypeKind::Struct { fields, .. } => fields.clone(),
            TypeKind::NamedStruct(id) => self
                .module
                .ctx
                .struct_def(*id)
                .fields
                .clone()
                .unwrap_or_default(),
            TypeKind::Array { element, .. } => vec![*element],
            _ => Vec::new(),
        };
        let found = fields
            .iter()
            .any(|field| self.reaches_struct(*field, wanted, trail));
        trail.pop();
        found
    }

    /// A struct holds scalable vectors only when it holds nothing else.
    /// Mixing them with fixed-size fields leaves no offset for whatever
    /// follows the scalable one, which is why upstream refuses `{i32,
    /// <vscale x 1 x i32>}` and reads `{<vscale x 1 x i32>, <vscale x 1 x
    /// i32>}`.
    fn struct_is_sized(&self, fields: &[TypeId], trail: &mut Vec<TypeId>) -> bool {
        if !fields
            .iter()
            .all(|field| self.is_sized_within(*field, trail))
        {
            return false;
        }
        let scalable = |ty: TypeId| {
            matches!(
                self.module.ctx.type_kind(ty),
                TypeKind::Vector { scalable: true, .. }
            )
        };
        let any = fields.iter().any(|field| scalable(*field));
        let all = fields.iter().all(|field| scalable(*field));
        !any || all
    }

    /// What a global may hold whether or not it is defined here: a type with
    /// a representation, and nothing scalable.
    fn fits_in_a_global(&self, ty: TypeId) -> bool {
        self.fits_in_a_global_within(ty, &mut Vec::new())
    }

    fn fits_in_a_global_within(&self, ty: TypeId, trail: &mut Vec<TypeId>) -> bool {
        if trail.contains(&ty) {
            return false;
        }
        trail.push(ty);
        let fits = self.fits_in_a_global_step(ty, trail);
        trail.pop();
        fits
    }

    fn fits_in_a_global_step(&self, ty: TypeId, trail: &mut Vec<TypeId>) -> bool {
        match self.module.ctx.type_kind(ty) {
            TypeKind::Void
            | TypeKind::Label
            | TypeKind::Metadata
            | TypeKind::Token
            | TypeKind::X86Amx
            | TypeKind::Function { .. } => false,
            TypeKind::Vector { scalable, .. } => !scalable,
            // Whether a target extension type can be a global is a property
            // of the target rather than of the IR: upstream reads
            // `target("spirv.DeviceEvent")` while refusing `target("opaque")`
            // and `target("aarch64.svcount")`, the last of which is sized and
            // still not allowed here.
            TypeKind::Target { name, .. } => {
                crate::target_extension::properties(name.as_str()).global
            }
            TypeKind::Struct { fields, .. } => fields
                .clone()
                .iter()
                .all(|field| self.fits_in_a_global_within(*field, trail)),
            TypeKind::NamedStruct(id) => match &self.module.ctx.struct_def(*id).fields {
                Some(fields) => fields
                    .clone()
                    .iter()
                    .all(|field| self.fits_in_a_global_within(*field, trail)),
                None => true,
            },
            TypeKind::Array { element, .. } => self.fits_in_a_global_within(*element, trail),
            _ => true,
        }
    }

    /// Whether a type has a size the layout can state as a number. A scalable
    /// vector has a size, but not one a global can use.
    fn has_a_fixed_size_within(&self, ty: TypeId, trail: &mut Vec<TypeId>) -> bool {
        if trail.contains(&ty) {
            return false;
        }
        if !self.is_sized_within(ty, trail) {
            return false;
        }
        trail.push(ty);
        let fixed = self.has_a_fixed_size_step(ty, trail);
        trail.pop();
        fixed
    }

    fn has_a_fixed_size_step(&self, ty: TypeId, trail: &mut Vec<TypeId>) -> bool {
        match self.module.ctx.type_kind(ty) {
            TypeKind::Vector { scalable, .. } => !scalable,
            TypeKind::Struct { fields, .. } => fields
                .clone()
                .iter()
                .all(|field| self.has_a_fixed_size_within(*field, trail)),
            TypeKind::NamedStruct(id) => match &self.module.ctx.struct_def(*id).fields {
                Some(fields) => fields
                    .clone()
                    .iter()
                    .all(|field| self.has_a_fixed_size_within(*field, trail)),
                None => false,
            },
            TypeKind::Array { element, .. } => self.has_a_fixed_size_within(*element, trail),
            _ => true,
        }
    }

    /// The debug-info rules that are the verifier's rather than the
    /// parser's: the grammar is checked when the node is read, and this is
    /// what needs the node's neighbours to make sense of.
    fn metadata_shape(&mut self, node: &Metadata, what: &str) {
        let Metadata::Tuple { operands, .. } = node else {
            return;
        };
        if operands.iter().any(|operand| {
            matches!(operand, MdOperand::Value { ty, .. }
                if matches!(self.module.ctx.type_kind(*ty), TypeKind::Metadata))
        }) {
            self.report(format!("{what} holds metadata wrapped in a value"));
        }
    }

    fn debug_info_node(&mut self, node: &Metadata) {
        let Metadata::Specialized { tag, args, .. } = node else {
            return;
        };
        let SpecializedArgs::Named(fields) = args else {
            return;
        };
        // A field holds text, or a number, or a node. `!"text"` is none of
        // those: it is a reference to a metadata string, which is what a
        // module writes when it names a type it has not described. Upstream
        // refuses one nearly everywhere, and where it does not is a list with
        // no shape, so `corpus/md-string-fields.nu` measured it.
        for (name, value) in fields {
            let holds_a_string = match value {
                MdField::Ref(id) => {
                    matches!(self.module.metadata_node(*id), Some(Metadata::String(_)))
                }
                MdField::Inline(node) => matches!(**node, Metadata::String(_)),
                _ => false,
            };
            if holds_a_string
                && STRING_VALUED
                    .binary_search(&(tag.as_str(), name.as_str()))
                    .is_err()
            {
                self.report(format!("!{tag}: invalid {name}, expected a node"));
            }
        }
        if tag == "DISubrange" || tag == "DIGenericSubrange" {
            // A subrange is described from one end or the other, never both.
            if field_of(fields, "count").is_some() && field_of(fields, "upperBound").is_some() {
                self.report(format!("!{tag} has both a count and an upperBound"));
            }
            // Where a bound is a node rather than a number, it has to be
            // something that produces one at run time.
            for bound in ["lowerBound", "upperBound", "stride", "count"] {
                let Some(field) = field_of(fields, bound) else {
                    continue;
                };
                if let Some(node) = self.field_node(field)
                    && !matches!(
                        specialized_tag(&node),
                        Some("DIExpression" | "DILocalVariable" | "DIGlobalVariable")
                    )
                {
                    self.report(format!(
                        "!{tag} has a {bound} that is neither a constant, a variable nor an expression"
                    ));
                }
            }
        }
        if tag == "DIGenericSubrange" && field_of(fields, "stride").is_none() {
            self.report("!DIGenericSubrange has no stride");
        }
        if tag == "DICompositeType" {
            let array = tag_is(fields, &["DW_TAG_array_type"]);
            let variant_part = tag_is(fields, &["DW_TAG_variant_part"]);
            // These four describe an array's shape, and a discriminator picks
            // between the arms of a variant.
            for shape in ["rank", "allocated", "associated", "dataLocation"] {
                if field_of(fields, shape).is_some() && !array {
                    self.report(format!("{shape} appears on a type that is not an array"));
                }
            }
            // An array says what it is an array of. It is the only composite
            // tag that has to: a structure, a union and an enumeration are
            // each read without one.
            if array && field_of(fields, "baseType").is_none() {
                self.report("an array type has no baseType");
            }
            // A null among the elements names no member.
            if let Some(field) = field_of(fields, "elements")
                && let Some(node) = self.field_node(field)
                && let Some(operands) = node.as_tuple()
                && operands
                    .iter()
                    .any(|operand| matches!(operand, MdOperand::Null))
            {
                self.report("the elements of a composite type contain a null entry");
            }
            // A type is passed by reference or by value, not both.
            if let Some(MdField::Words(words)) = field_of(fields, "flags") {
                let by_reference = words.iter().any(|word| word == "DIFlagTypePassByReference");
                let by_value = words.iter().any(|word| word == "DIFlagTypePassByValue");
                if by_reference && by_value {
                    self.report("a type is marked as passed both by reference and by value");
                }
            }
            if field_of(fields, "discriminator").is_some() && !variant_part {
                self.report("a discriminator appears on a type that is not a variant part");
            }
            if let Some(field) = field_of(fields, "templateParams") {
                let parameters = match self.field_node(field) {
                    Some(node) => node.as_tuple().map(<[MdOperand]>::to_vec),
                    None => None,
                };
                let Some(parameters) = parameters else {
                    self.report("the template parameters of a composite type are not a tuple");
                    return;
                };
                for parameter in parameters {
                    let node = match &parameter {
                        MdOperand::Ref(id) => self.module.metadata_node(*id).cloned(),
                        MdOperand::Inline(node) => Some((**node).clone()),
                        _ => None,
                    };
                    let good = node.as_ref().is_some_and(|node| {
                        matches!(
                            specialized_tag(node),
                            Some("DITemplateTypeParameter" | "DITemplateValueParameter")
                        )
                    });
                    if !good {
                        self.report("a template parameter is not a template parameter node");
                    }
                }
            }
        }
        if tag == "DIDerivedType" && field_of(fields, "dwarfAddressSpace").is_some() {
            // The address space says where a pointer points. A typedef or a
            // qualifier has nowhere to put it.
            let pointer = tag_is(
                fields,
                &[
                    "DW_TAG_pointer_type",
                    "DW_TAG_reference_type",
                    "DW_TAG_rvalue_reference_type",
                ],
            );
            if !pointer {
                self.report("DWARF address space only applies to pointer or reference types");
            }
        }
    }

    /// Attributes that describe something only a pointer has. Upstream words
    /// every one of these the same way, so the check is one rule and not
    /// twenty.
    fn attribute_set(&mut self, attrs: &AttributeSet, ty: TypeId, where_: &str) {
        let pointer = matches!(self.module.ctx.type_kind(ty), TypeKind::Pointer { .. });
        for attribute in &attrs.attributes {
            let wants_a_pointer = match attribute {
                // `signext` and `zeroext` say how a narrow integer is widened
                // to fill a register, which nothing but an integer is narrow
                // in. `noext` says not to widen it and upstream reads one
                // anywhere, having nothing to do either way.
                Attribute::Enum(EnumAttr::SignExt | EnumAttr::ZeroExt) => {
                    if !matches!(
                        self.module.ctx.type_kind(self.innermost_element(ty)),
                        TypeKind::Integer(_)
                    ) {
                        self.report(format!(
                            "{} on {where_}, which is not an integer",
                            describe_attribute(attribute)
                        ));
                    }
                    false
                }
                Attribute::Enum(kind) => matches!(
                    kind,
                    EnumAttr::AllocPtr
                        | EnumAttr::DeadOnReturn
                        | EnumAttr::DeadOnUnwind
                        | EnumAttr::Nest
                        | EnumAttr::NoAlias
                        | EnumAttr::NoCapture
                        | EnumAttr::NonNull
                        | EnumAttr::SwiftAsync
                        | EnumAttr::SwiftError
                        | EnumAttr::SwiftSelf
                        | EnumAttr::Writable
                ),
                Attribute::Int {
                    kind: IntAttr::Align,
                    first,
                    ..
                } => {
                    if !first.is_power_of_two() {
                        self.report(format!(
                            "an alignment of {first} on {where_} is not a power of two"
                        ));
                    }
                    true
                }
                Attribute::Int { kind, .. } => matches!(
                    kind,
                    IntAttr::Align | IntAttr::Dereferenceable | IntAttr::DereferenceableOrNull
                ),
                Attribute::Type { kind, .. } => matches!(
                    kind,
                    TypeAttr::ByRef
                        | TypeAttr::ByVal
                        | TypeAttr::ElementType
                        | TypeAttr::InAlloca
                        | TypeAttr::Preallocated
                        | TypeAttr::StructRet
                ),
                _ => false,
            };
            // Passing something by value means copying it, which needs a
            // size, and a copy of four gigabytes is not something a caller
            // does. The default layout is enough to catch the huge ones, so
            // a module that states none is still held to this.
            if let Attribute::Type {
                kind: kind @ (TypeAttr::ByVal | TypeAttr::InAlloca | TypeAttr::Preallocated),
                ty: pointee,
            } = attribute
            {
                if !self.is_sized(*pointee) {
                    self.report(format!(
                        "{} on {where_} names a type with no size",
                        kind.keyword()
                    ));
                }
                let layout = self.module.data_layout.clone().unwrap_or_default();
                if let Ok(size) =
                    crate::layout::alloc_size_bytes(&self.module.ctx, &layout, *pointee)
                    && size >= 1 << 32
                {
                    self.report(format!(
                        "a {} of {size} bytes on {where_} is too large",
                        kind.keyword()
                    ));
                }
            }
            // `initializes((0, 4), (8, 12))` lists ranges that each run
            // forwards and do not overlap.
            if let Attribute::Structured {
                kind: crate::attribute::StructuredAttr::Initializes,
                arguments,
            } = attribute
            {
                let bounds: Vec<i64> = arguments
                    .split(|c: char| !c.is_ascii_digit() && c != '-')
                    .filter(|word| !word.is_empty())
                    .filter_map(|word| word.parse().ok())
                    .collect();
                let ordered = bounds
                    .chunks(2)
                    .all(|range| matches!(range, [low, high] if low < high))
                    && bounds.windows(2).all(|pair| pair[0] <= pair[1]);
                if !ordered {
                    self.report(format!(
                        "the initializes ranges on {where_} are unordered or overlapping"
                    ));
                }
            }
            if wants_a_pointer && !pointer {
                self.report(format!(
                    "{} on {where_}, which is not a pointer",
                    describe_attribute(attribute)
                ));
                continue;
            }
            match attribute {
                // `range(i8 1, 0)` says what it constrains, so the width has
                // to be the width of the thing it is attached to.
                Attribute::Range {
                    ty: range_ty,
                    lower,
                    upper,
                } if lower == upper => {
                    let _ = range_ty;
                    self.report(format!("an empty range on {where_} constrains nothing"));
                }
                Attribute::Range { ty: range_ty, .. } => {
                    let element = self.element_or_self(ty);
                    let TypeKind::Integer(bits) = *self.module.ctx.type_kind(element) else {
                        self.report(format!("range on {where_}, which is not an integer"));
                        continue;
                    };
                    if let TypeKind::Integer(range_bits) = *self.module.ctx.type_kind(*range_ty)
                        && range_bits != bits
                    {
                        self.report(format!(
                            "range of i{range_bits} on {where_}, which is i{bits}"
                        ));
                    }
                }
                Attribute::Structured { kind, .. }
                    if *kind == crate::attribute::StructuredAttr::NoFpClass =>
                {
                    // Unlike `range`, this one reaches through arrays as well
                    // as vectors: `[8 x [4 x float]]` may carry it.
                    let element = self.innermost_element(ty);
                    if !matches!(self.module.ctx.type_kind(element), TypeKind::Float(_)) {
                        self.report(format!(
                            "nofpclass on {where_}, which is not a floating-point type"
                        ));
                    }
                }
                Attribute::Enum(EnumAttr::NoUndef)
                    if matches!(self.module.ctx.type_kind(ty), TypeKind::Void) =>
                {
                    self.report(format!("noundef on {where_}, which is void"));
                }
                _ => {}
            }
        }
    }

    /// An attribute set with the groups it names folded in, `#0` standing for
    /// whatever `attributes #0 = { ... }` holds. A group nothing defines is an
    /// empty set rather than an error.
    fn resolved_attributes(&self, set: &AttributeSet) -> Vec<Attribute> {
        let mut attributes = set.attributes.clone();
        for group in &set.groups {
            if let Some(contents) = self.module.attribute_group(*group) {
                attributes.extend(contents.iter().cloned());
            }
        }
        attributes
    }

    /// The symbol a value names, reaching through the casts a constant
    /// expression may wrap it in.
    fn resolve_symbol_of(&self, value: Value) -> Option<GlobalRef> {
        match value {
            Value::Constant(id) => self.resolve_symbol(id),
            _ => None,
        }
    }

    /// Rules about a function attribute's own value, which have nothing to do
    /// with the type it is attached to.
    fn function_attributes(&mut self, function: &Function) {
        let unnamed = function.qualifiers.unnamed_addr.is_some();
        let attributes = self.resolved_attributes(&function.attrs);
        let has = |wanted: EnumAttr| {
            attributes
                .iter()
                .any(|a| matches!(a, Attribute::Enum(kind) if *kind == wanted))
        };
        let variants: Vec<String> = attributes
            .iter()
            .filter_map(|a| match a {
                Attribute::String { key, value } if key == "vector-function-abi-variant" => {
                    value.clone()
                }
                _ => None,
            })
            .collect();
        for variant in variants {
            for name in variant.split(',') {
                if !self.vfabi_variant(name, function.params.len()) {
                    self.report(format!("invalid name for a VFABI variant: {name}"));
                }
            }
        }
        // Pairs that ask for opposite things.
        for (first, second) in [
            (EnumAttr::AlwaysInline, EnumAttr::NoInline),
            (EnumAttr::AlwaysInline, EnumAttr::OptNone),
            (EnumAttr::ReadNone, EnumAttr::ReadOnly),
            (
                EnumAttr::SanitizeRealtime,
                EnumAttr::SanitizeRealtimeBlocking,
            ),
        ] {
            if has(first) && has(second) {
                self.report(format!(
                    "{} and {} are incompatible",
                    first.keyword(),
                    second.keyword()
                ));
            }
        }
        for attribute in &attributes {
            match attribute {
                Attribute::Enum(kind @ (EnumAttr::Builtin | EnumAttr::ImmArg)) => {
                    self.report(format!(
                        "{} describes a call site rather than a function",
                        kind.keyword()
                    ));
                }
                Attribute::Enum(EnumAttr::JumpTable) if !unnamed => {
                    self.report("jumptable requires unnamed_addr");
                }
                Attribute::Int {
                    kind: IntAttr::VScaleRange,
                    first,
                    second,
                } => {
                    if *first == 0 {
                        self.report("the vscale_range minimum must be greater than zero");
                    } else if !first.is_power_of_two() {
                        self.report("the vscale_range minimum must be a power of two");
                    }
                    if let Some(max) = second {
                        if *max != 0 && !max.is_power_of_two() {
                            self.report("the vscale_range maximum must be a power of two");
                        }
                        if first > max {
                            self.report(
                                "the vscale_range minimum may not be greater than the maximum",
                            );
                        }
                    }
                }
                // `allocsize(0, 1)` names parameters by position, so the
                // positions have to exist.
                Attribute::Int {
                    kind: IntAttr::AllocSize,
                    first,
                    second,
                } => {
                    let params = function.params.len() as u64;
                    for index in [Some(*first), *second].into_iter().flatten() {
                        if index >= params {
                            self.report(format!(
                                "allocsize names parameter {index} of a function with {params}"
                            ));
                        }
                    }
                    // The two are the element count and the element size, so
                    // naming one parameter twice says nothing.
                    if *second == Some(*first) {
                        self.report(format!("allocsize names parameter {first} twice"));
                    }
                }
                // `allockind("alloc,zeroed")` says which of the three things
                // this function does, and it does exactly one of them.
                Attribute::Structured {
                    kind: crate::attribute::StructuredAttr::UwTable,
                    arguments,
                } if !matches!(arguments.trim(), "" | "async" | "sync") => {
                    self.report(format!(
                        "uwtable names {arguments}, which is not a kind of unwind table"
                    ));
                }
                Attribute::Structured {
                    kind: crate::attribute::StructuredAttr::AllocKind,
                    arguments,
                } => {
                    let kinds = arguments
                        .split(',')
                        .map(|word| word.trim().trim_matches('"'))
                        .filter(|word| matches!(*word, "alloc" | "realloc" | "free"))
                        .count();
                    if kinds != 1 {
                        self.report("allockind names none or several of alloc, realloc and free");
                    }
                }
                Attribute::String { key, value } => self.string_attribute(key, value.as_deref()),
                _ => {}
            }
        }
    }

    /// The quoted attributes whose value upstream reads rather than carries.
    fn string_attribute(&mut self, key: &str, value: Option<&str>) {
        let value = value.unwrap_or("");
        match key {
            "frame-pointer" if !matches!(value, "all" | "non-leaf" | "none" | "reserved") => {
                self.report(format!("invalid value for 'frame-pointer': {value}"));
            }
            "denormal-fp-math" | "denormal-fp-math-f32" if !is_a_denormal_mode(value) => {
                self.report(format!("invalid value for '{key}': {value}"));
            }
            "patchable-function-entry" | "patchable-function-prefix" | "warn-stack-size"
                if value.parse::<u64>().is_err() =>
            {
                self.report(format!("'{key}' takes an unsigned integer: {value}"));
            }
            "sign-return-address" if !matches!(value, "none" | "non-leaf" | "all") => {
                self.report(format!("invalid value for '{key}' attribute: {value}"));
            }
            "sign-return-address-key" if !matches!(value, "a_key" | "b_key") => {
                self.report(format!("invalid value for '{key}' attribute: {value}"));
            }
            "no-jump-tables"
            | "less-precise-fpmad"
            | "no-infs-fp-math"
            | "no-nans-fp-math"
            | "no-signed-zeros-fp-math"
            | "unsafe-fp-math"
            | "use-soft-float"
                if !matches!(value, "true" | "false") =>
            {
                self.report(format!("invalid value for '{key}' attribute: {value}"));
            }
            "alloc-variant-zeroed" if value.is_empty() => {
                self.report("'alloc-variant-zeroed' must not be empty");
            }
            _ => {}
        }
    }

    /// The attributes a function may carry on only one of its parameters.
    fn at_most_one_of(&mut self, function: &Function) {
        const ONCE: &[(EnumAttr, &str)] = &[
            (EnumAttr::SwiftAsync, "swiftasync"),
            (EnumAttr::SwiftError, "swifterror"),
            (EnumAttr::SwiftSelf, "swiftself"),
        ];
        for (attribute, name) in ONCE {
            let count = function
                .params
                .iter()
                .filter(|param| param.attrs.has(*attribute))
                .count();
            if count > 1 {
                self.report(format!("{count} parameters are {name}, which allows one"));
            }
        }
        // `preallocated` is not on this list: upstream's preallocated-valid.ll
        // puts it on two parameters of one function and llvm-as reads it.
        const ONCE_TYPED: &[(TypeAttr, &str)] = &[
            (TypeAttr::InAlloca, "inalloca"),
            (TypeAttr::StructRet, "sret"),
        ];
        for (attribute, name) in ONCE_TYPED {
            let count = function
                .params
                .iter()
                .filter(|param| has_type_attribute(&param.attrs, *attribute))
                .count();
            if count > 1 {
                self.report(format!("{count} parameters are {name}, which allows one"));
            }
        }
        // swifterror describes where a callee writes an error, which a return
        // value has no room for.
        if function.return_attrs.has(EnumAttr::SwiftError) {
            self.report("swifterror on the return value, which it does not apply to");
        }
    }

    /// The scalar at the bottom of any nest of arrays and vectors.
    fn innermost_element(&self, ty: TypeId) -> TypeId {
        match self.module.ctx.type_kind(ty) {
            TypeKind::Array { element, .. } | TypeKind::Vector { element, .. } => {
                self.innermost_element(*element)
            }
            _ => ty,
        }
    }

    fn opaque_value_type(&self, ty: TypeId) -> Option<&'static str> {
        match self.module.ctx.type_kind(ty) {
            TypeKind::Token => Some("token"),
            TypeKind::X86Amx => Some("x86_amx"),
            _ => None,
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

    /// The address space of a pointer, or of a vector of them.
    fn address_space_of(&self, ty: TypeId) -> Option<u32> {
        match self.module.ctx.type_kind(self.element_or_self(ty)) {
            TypeKind::Pointer { address_space } => Some(*address_space),
            _ => None,
        }
    }

    /// The total width of a type, when the data layout can give one.
    /// What an atomic access may move: one scalar the target can load or
    /// store in a single instruction. That rules out vectors and aggregates
    /// whatever their size, and it rules out a scalar whose size is not a
    /// power of two, which is why `x86_fp80` cannot be moved atomically and
    /// `fp128` can.
    fn atomic_operand(&mut self, ty: TypeId, where_: &str) {
        let kind = self.module.ctx.type_kind(ty).clone();
        match kind {
            TypeKind::Integer(_) | TypeKind::Float(_) => self.atomic_size(ty, where_),
            TypeKind::Pointer { .. } => {}
            _ => self.report(format!("{where_} moves a type an atomic cannot move")),
        }
    }

    /// The size half of the same rule, for `atomicrmw`, whose own operand
    /// check already says which kinds its operation takes. A vector is one of
    /// them for the floating-point operations, and `<3 x half>` is refused
    /// for its 48 bits rather than for being a vector.
    fn atomic_size(&mut self, ty: TypeId, where_: &str) {
        let Some(bits) = self.size_in_bits(ty) else {
            return;
        };
        if bits < 8 || !bits.is_power_of_two() {
            self.report(format!(
                "{where_} moves {bits} bits, which is not a size an atomic comes in"
            ));
        }
    }

    /// Whether a token was made by a statepoint, which is what makes a
    /// safepoint there is anything to ask about. A statepoint written as an
    /// `invoke` makes one on both of its edges, so the `landingpad` the
    /// unwind edge opens with carries the same token.
    ///
    /// `poison` and `undef` are neither yes nor no: they stand for any value,
    /// so there is nothing to be wrong about. Nor is an argument of the
    /// enclosing function: `llvm-as` crashes on one, and a crash is not a
    /// verdict to copy.
    fn comes_from_a_statepoint(&self, function: &Function, value: Value) -> bool {
        match value {
            Value::Instruction(id) => match function.try_instruction(id) {
                Some(instruction) => match &instruction.kind {
                    InstKind::Call(call) | InstKind::Invoke { call, .. } => {
                        self.calls_a_statepoint(call)
                    }
                    InstKind::LandingPad { .. } => self.unwinds_from_a_statepoint(function, id),
                    _ => false,
                },
                None => true,
            },
            Value::Constant(id) => !matches!(self.module.ctx.constant(id), Constant::NoneToken(_)),
            _ => true,
        }
    }

    fn calls_a_statepoint(&self, call: &crate::instruction::CallData) -> bool {
        match call.callee {
            Value::Constant(id) => self
                .mentions_an_intrinsic(id)
                .is_some_and(|name| name.starts_with("llvm.experimental.gc.statepoint")),
            _ => false,
        }
    }

    /// Whether the block a `landingpad` opens is one a statepoint invoke
    /// unwinds to.
    fn unwinds_from_a_statepoint(&self, function: &Function, pad: InstId) -> bool {
        let Some(landing) = function
            .blocks()
            .find(|(id, _)| function.block_instructions(*id).any(|(at, _)| at == pad))
            .map(|(id, _)| id)
        else {
            return false;
        };
        function.blocks().any(|(id, _)| {
            function.block_instructions(id).any(|(_, instruction)| {
                matches!(&instruction.kind, InstKind::Invoke { call, unwind, .. }
                    if *unwind == landing && self.calls_a_statepoint(call))
            })
        })
    }

    /// Whether a name is a well-formed vector variant of this function.
    fn vfabi_variant(&self, name: &str, params: usize) -> bool {
        let Some(rest) = name.strip_prefix("_ZGV") else {
            return false;
        };
        let rest = match rest.strip_prefix("_LLVM_") {
            Some(rest) => rest,
            None => match rest.char_indices().nth(1) {
                Some((at, _)) => &rest[at..],
                None => return false,
            },
        };
        let mut chars = rest.chars().peekable();
        if !matches!(chars.next(), Some('M' | 'N')) {
            return false;
        }
        let mut lanes = String::new();
        while chars.peek().is_some_and(char::is_ascii_digit) {
            lanes.push(chars.next().unwrap_or_default());
        }
        if lanes.parse::<u64>().unwrap_or(0) == 0 {
            return false;
        }

        let mut described = 0usize;
        loop {
            match chars.next() {
                Some('_') => break,
                Some('v' | 'u') => {}
                // A linear parameter walks by a stride: a constant, one held
                // in another parameter (`s` and its position), or a negative
                // one (`n`).
                Some('l' | 'R' | 'L' | 'U') => match chars.peek() {
                    Some('n') => {
                        chars.next();
                        while chars.peek().is_some_and(char::is_ascii_digit) {
                            chars.next();
                        }
                    }
                    Some('s') => {
                        chars.next();
                        if !chars.peek().is_some_and(char::is_ascii_digit) {
                            return false;
                        }
                        while chars.peek().is_some_and(char::is_ascii_digit) {
                            chars.next();
                        }
                    }
                    _ => {
                        while chars.peek().is_some_and(char::is_ascii_digit) {
                            chars.next();
                        }
                    }
                },
                _ => return false,
            }
            // Any of them may say what the pointed-at memory is aligned to.
            if chars.peek() == Some(&'a') {
                chars.next();
                let mut alignment = String::new();
                while chars.peek().is_some_and(char::is_ascii_digit) {
                    alignment.push(chars.next().unwrap_or_default());
                }
                match alignment.parse::<u64>() {
                    Ok(bytes) if bytes.is_power_of_two() => {}
                    _ => return false,
                }
            }
            described += 1;
        }
        // One per parameter, and at least one either way: a variant of a
        // function that takes nothing has nothing to widen.
        if described == 0 || described != params {
            return false;
        }

        // What is left is the scalar name, and the variant's name in brackets
        // after it. Both have to be there to name anything.
        let names: String = chars.collect();
        let scalar = names.split('(').next().unwrap_or_default();
        if scalar.is_empty() {
            return false;
        }
        match names.split_once('(') {
            None => true,
            Some((_, variant)) => variant
                .rfind(')')
                .is_some_and(|close| !variant[..close].is_empty()),
        }
    }

    /// The two types a `!tbaa` tag names. Both are type nodes, and a type
    /// node reaches a root by way of its parent: `!{!"int", !{!"omnipotent
    /// char", !{!"root"}, i64 0}, i64 0}` is three deep. A chain that comes
    /// back round to itself reaches no root, so it describes nothing.
    ///
    /// The access type is the narrower of the two. A base may be a struct
    /// type node, which lists its fields and their offsets, but an access is
    /// what was actually read or written and so is a scalar.
    fn tbaa_types(&mut self, operands: &[MdOperand], where_: &str) {
        let base = self.tbaa_reaches_a_root(&operands[0], &mut Vec::new());
        if !base {
            self.report(format!("{where_}: base type node reaches no root"));
        }
        if !self.tbaa_is_scalar(&operands[1])
            || !self.tbaa_scalar_chain(&operands[1], &mut Vec::new())
        {
            self.report(format!(
                "{where_}: access type node must be a valid scalar type"
            ));
        }
    }

    /// An access type and everything it refines: scalars up to a root, with
    /// no struct in between, an access being one value rather than a place.
    fn tbaa_scalar_chain(&self, operand: &MdOperand, trail: &mut Vec<MdId>) -> bool {
        let MdOperand::Ref(id) = operand else {
            return false;
        };
        if trail.contains(id) {
            return false;
        }
        trail.push(*id);
        let answer = match self
            .module
            .metadata_node(*id)
            .and_then(|node| node.as_tuple().map(<[MdOperand]>::to_vec))
        {
            Some(fields) if fields.len() == 1 => matches!(fields[0], MdOperand::String(_)),
            Some(fields) if matches!(fields.len(), 2 | 3) => {
                matches!(fields[0], MdOperand::String(_))
                    && self.tbaa_scalar_chain(&fields[1], trail)
            }
            _ => false,
        };
        trail.pop();
        answer
    }

    /// Whether a type node is one at all, and whether following its parent
    /// ends. A node with one operand is a root; one with two or three is a
    /// scalar and has a parent; anything longer is a struct and each of its
    /// fields is a type node in turn.
    fn tbaa_reaches_a_root(&self, operand: &MdOperand, trail: &mut Vec<MdId>) -> bool {
        let MdOperand::Ref(id) = operand else {
            return false;
        };
        if trail.contains(id) {
            return false;
        }
        trail.push(*id);
        let answer = match self
            .module
            .metadata_node(*id)
            .and_then(|node| node.as_tuple().map(<[MdOperand]>::to_vec))
        {
            None => false,
            Some(fields) => match fields.len() {
                0 => false,
                1 => matches!(fields[0], MdOperand::String(_)),
                2 | 3 => {
                    matches!(fields[0], MdOperand::String(_))
                        && self.tbaa_reaches_a_root(&fields[1], trail)
                }
                _ => {
                    matches!(fields[0], MdOperand::String(_))
                        && fields
                            .iter()
                            .skip(1)
                            .step_by(2)
                            .all(|field| self.tbaa_reaches_a_root(field, trail))
                }
            },
        };
        trail.pop();
        answer
    }

    /// A scalar type node: a name and the parent it refines, with the offset
    /// into that parent optional.
    fn tbaa_is_scalar(&self, operand: &MdOperand) -> bool {
        let MdOperand::Ref(id) = operand else {
            return false;
        };
        self.module
            .metadata_node(*id)
            .and_then(|node| node.as_tuple().map(<[MdOperand]>::to_vec))
            .is_some_and(|fields| matches!(fields.len(), 2 | 3))
    }

    /// Which pointer-authentication ABI an AArch64 ELF object was built for
    /// is a platform and a version together, and a note holding one of the
    /// two says nothing a linker can act on. So a module writes both flags or
    /// neither.
    fn pauth_abi_is_named_whole(&mut self, flags: &[MdId]) {
        let named = |wanted: &str| {
            flags.iter().any(|id| {
                self.module
                    .metadata_node(*id)
                    .and_then(|node| node.as_tuple())
                    .and_then(|operands| operands.get(1).cloned())
                    .is_some_and(|operand| match operand {
                        MdOperand::String(text) => text.as_str() == Some(wanted),
                        _ => false,
                    })
            })
        };
        if named("aarch64-elf-pauthabi-platform") != named("aarch64-elf-pauthabi-version") {
            self.report(
                "either both or no 'aarch64-elf-pauthabi-platform' and \
                 'aarch64-elf-pauthabi-version' module flags must be present",
            );
        }
    }

    /// One bundle on a call to `llvm.assume`.
    fn assume_bundle(&mut self, bundle: &crate::instruction::OperandBundle, where_: &str) {
        let tag = bundle.tag.as_str();
        if matches!(tag, "ignore" | "separate_storage") {
            // Two allocations, and there is nothing to say about one.
            if tag == "separate_storage" && bundle.args.len() != 2 {
                self.report(format!(
                    "{where_}: a separate_storage assumption names two allocations"
                ));
            }
            return;
        }
        if !crate::attribute::names_an_attribute(tag) {
            self.report(format!(
                "{where_}: tags must be valid attribute names, and {tag} is not one"
            ));
            return;
        }
        // The one whose arguments upstream reads: the pointer being asserted
        // about and how many bytes behind it are there.
        if tag == "dereferenceable" {
            if bundle.args.len() != 2 {
                self.report(format!(
                    "{where_}: dereferenceable assumptions should have 2 arguments"
                ));
                return;
            }
            let (first, second) = (bundle.args[0].0, bundle.args[1].0);
            if !matches!(self.module.ctx.type_kind(first), TypeKind::Pointer { .. }) {
                self.report(format!("{where_}: first argument should be a pointer"));
            }
            if !matches!(self.module.ctx.type_kind(second), TypeKind::Integer(_)) {
                self.report(format!("{where_}: second argument should be an integer"));
            }
        }
    }

    /// Whether a struct holds a scalable vector, directly or through another
    /// struct. An array is not looked through: the question is whether this
    /// type is a struct with no fixed field offsets, and an array of such
    /// structs is one upstream indexes happily.
    fn holds_a_scalable_vector(&self, ty: TypeId, trail: &mut Vec<TypeId>) -> bool {
        if trail.contains(&ty) {
            return false;
        }
        trail.push(ty);
        let fields = match self.module.ctx.type_kind(ty).clone() {
            TypeKind::Struct { fields, .. } => fields,
            TypeKind::NamedStruct(id) => self
                .module
                .ctx
                .struct_def(id)
                .fields
                .clone()
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let answer = fields.iter().any(|field| {
            matches!(
                self.module.ctx.type_kind(*field),
                TypeKind::Vector { scalable: true, .. }
            ) || self.holds_a_scalable_vector(*field, trail)
        });
        trail.pop();
        answer
    }

    /// Whether anything inside a type asks to be aligned past what the
    /// encoding holds. A vector is the only thing that can: its alignment is
    /// its own size rounded up to a power of two, so `<2147483649 x i16>` is
    /// four gigabytes and two, which rounds to eight. An array or a struct
    /// inherits the answer from what it holds, and a type that reaches itself
    /// is left to the rule that says so.
    fn wants_an_unrepresentable_alignment(&self, ty: TypeId, trail: &mut Vec<TypeId>) -> bool {
        if trail.contains(&ty) {
            return false;
        }
        trail.push(ty);
        let answer = match self.module.ctx.type_kind(ty).clone() {
            TypeKind::Vector {
                scalable: false, ..
            } => self.size_in_bits(ty).is_some_and(|bits| {
                bits.checked_next_power_of_two()
                    .is_none_or(|rounded| rounded.div_ceil(8) > MAXIMUM_ALIGNMENT)
            }),
            TypeKind::Array { element, .. } => {
                self.wants_an_unrepresentable_alignment(element, trail)
            }
            TypeKind::Struct { fields, .. } => fields
                .iter()
                .any(|field| self.wants_an_unrepresentable_alignment(*field, trail)),
            TypeKind::NamedStruct(id) => self
                .module
                .ctx
                .struct_def(id)
                .fields
                .clone()
                .unwrap_or_default()
                .iter()
                .any(|field| self.wants_an_unrepresentable_alignment(*field, trail)),
            _ => false,
        };
        trail.pop();
        answer
    }

    fn size_in_bits(&self, ty: TypeId) -> Option<u64> {
        let layout = self.module.data_layout.clone().unwrap_or_default();
        crate::layout::size_in_bits(&self.module.ctx, &layout, ty).ok()
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

/// Every arrival at `target`, counted once per edge, so a terminator that
/// names it twice appears twice.
fn predecessor_edges(function: &Function, target: BlockId) -> Vec<BlockId> {
    let mut edges = Vec::new();
    for (id, _) in function.blocks() {
        for (_, instruction) in function.block_instructions(id) {
            for successor in instruction.kind.successors() {
                if successor == target {
                    edges.push(id);
                }
            }
        }
    }
    edges
}

/// Every value an instruction reads, flattened.
/// The instructions an instruction reads, which is what dominance needs.
fn operands(kind: &InstKind) -> Vec<InstId> {
    kind.operand_values()
        .into_iter()
        .filter_map(|value| match value {
            Value::Instruction(id) => Some(id),
            _ => None,
        })
        .collect()
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

fn has_enum_attribute(attrs: &AttributeSet, wanted: EnumAttr) -> bool {
    attrs
        .attributes
        .iter()
        .any(|a| matches!(a, Attribute::Enum(kind) if *kind == wanted))
}

fn has_type_attribute(attrs: &AttributeSet, wanted: TypeAttr) -> bool {
    attrs
        .attributes
        .iter()
        .any(|a| matches!(a, Attribute::Type { kind, .. } if *kind == wanted))
}

/// An attribute's keyword, without its argument.
fn describe_attribute(attribute: &Attribute) -> String {
    match attribute {
        Attribute::Enum(kind) => kind.keyword().to_string(),
        Attribute::Int { kind, .. } => kind.keyword().to_string(),
        Attribute::Type { kind, .. } => kind.keyword().to_string(),
        Attribute::Range { .. } => "range".to_string(),
        Attribute::Structured { kind, .. } => kind.keyword().to_string(),
        Attribute::String { key, .. } => format!("\"{key}\""),
    }
}

/// Whether a node's tag is one of these, whichever way the module wrote it.
/// A tag is read into the number its word stands for, so both spellings
/// arrive here as that number.
fn tag_is(fields: &[(String, MdField)], wanted: &[&str]) -> bool {
    match field_of(fields, "tag") {
        Some(MdField::Unsigned(number)) => wanted.iter().any(|word| {
            crate::metadata::number("", "tag", word)
                .is_some_and(|value| u128::from(value) == *number)
        }),
        // A word outside the vocabulary is kept as it was written.
        Some(MdField::Words(words)) => words.iter().any(|word| wanted.contains(&word.as_str())),
        _ => false,
    }
}

fn field_of<'a>(fields: &'a [(String, MdField)], wanted: &str) -> Option<&'a MdField> {
    fields
        .iter()
        .find(|(name, _)| name == wanted)
        .map(|(_, value)| value)
}

/// `denormal-fp-math` is one mode, or the input mode and the output mode.
fn is_a_denormal_mode(value: &str) -> bool {
    let modes: Vec<&str> = value.split(',').collect();
    modes.len() <= 2
        && modes.iter().all(|mode| {
            matches!(
                *mode,
                "ieee" | "preserve-sign" | "positive-zero" | "dynamic"
            )
        })
}

fn specialized_tag(node: &Metadata) -> Option<&str> {
    match node {
        Metadata::Specialized { tag, .. } => Some(tag.as_str()),
        _ => None,
    }
}

fn constant_u64(verifier: &Verifier<'_>, value: Option<&Value>) -> Option<u64> {
    let Some(Value::Constant(id)) = value else {
        return None;
    };
    verifier
        .module
        .ctx
        .constant(*id)
        .as_integer()
        .and_then(ApInt::to_u64)
}

fn attachment_ids(attachments: &[MdAttachment]) -> Vec<MdId> {
    attachments
        .iter()
        .filter_map(|attachment| match attachment.node {
            MdRef::Id(id) => Some(id),
            MdRef::Inline(_) => None,
        })
        .collect()
}

/// The fields that may hold a metadata string rather than a node. Sorted, so
/// the lookup can be a binary search. Generated by
/// `corpus/md-string-fields.nu`, which explains why this is a list rather
/// than a rule.
static STRING_VALUED: &[(&str, &str)] = &[
    ("DICompositeType", "annotations"),
    ("DICompositeType", "offset"),
    ("DIDerivedType", "annotations"),
    ("DIDerivedType", "extraData"),
    ("DIDerivedType", "offset"),
    ("DIImportedEntity", "elements"),
    ("DIImportedEntity", "file"),
    ("DILexicalBlock", "file"),
    ("DILocalVariable", "annotations"),
    ("DIModule", "file"),
    ("DIModule", "scope"),
    ("DIStringType", "size"),
    ("DIStringType", "stringLength"),
    ("DIStringType", "stringLengthExpression"),
    ("DIStringType", "stringLocationExpression"),
    ("DITemplateValueParameter", "value"),
];

/// Whether a `range(T lower, upper)` holds a value.
///
/// The interval is half-open and the comparison is unsigned, so a range whose
/// lower bound is above its upper one is the wrap round the end rather than an
/// empty one: `range(i32 -3, 4)` holds -3 and 3 and neither -4 nor 4, while
/// `range(i32 4, -3)` holds 5 and not nought. Both were measured, an empty
/// range being refused elsewhere.
fn range_holds(lower: &ApInt, upper: &ApInt, value: &ApInt) -> bool {
    if lower.bits() != value.bits() || upper.bits() != value.bits() {
        return true;
    }
    let above_lower = value.cmp_unsigned(lower).is_ge();
    let below_upper = value.cmp_unsigned(upper).is_lt();
    if lower.cmp_unsigned(upper).is_gt() {
        above_lower || below_upper
    } else {
        above_lower && below_upper
    }
}

/// Whether a node that is a `!DIExpression` holds something upstream reads.
///
/// Anything that is not one answers yes, there being nothing to say about it.
/// The elements are numbers by the time they are here, a word having been
/// read as the number it stands for, so `DW_OP_deref` and `6` are one node
/// and one question.
fn expression_is_valid(node: &Metadata) -> bool {
    let Metadata::Specialized { tag, args, .. } = node else {
        return true;
    };
    if tag != "DIExpression" {
        return true;
    }
    let SpecializedArgs::Positional(fields) = args else {
        return true;
    };
    // An element that is neither a number nor one is an expression upstream
    // never built, a word having been read into its number as the node was.
    let Some(elements) = crate::metadata::expression::elements(fields) else {
        return false;
    };
    crate::metadata::expression::is_valid(&elements)
}
