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

use crate::attribute::{Attribute, AttributeSet, EnumAttr, IntAttr, TypeAttr};
use crate::constant::{CastOp, ConstExpr, ConstId, Constant};
use crate::function::Function;
use crate::global::{DllStorageClass, GlobalQualifiers, Linkage, RuntimePreemption, Visibility};
use crate::instruction::{
    AtomicOrdering, AtomicRmwOp, BinOp, CallingConv, InstKind, IntFlags, NamedCallingConv, TailKind,
};
use crate::intrinsic_table::Parameter;
use crate::metadata::{MdAttachment, MdField, MdOperand, MdRef, Metadata, SpecializedArgs};
use crate::module::Module;
use crate::summary::SummaryValue;
use crate::types::TypeKind;
use crate::value::{BlockId, GlobalRef, InstId, MdId, Name, Value};
use crate::{FunctionId, TypeId};
use llvm_support::ApInt;

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

    fn metadata_exists(&mut self, id: MdId, what: &str) {
        if self.module.metadata_node(id).is_none() {
            self.report(format!("{what} refers to undefined metadata !{}", id.0));
        }
    }

    // ----------------------------------------------------------- module rules

    fn module_level(&mut self) {
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
            if global.comdat.is_some() {
                self.comdat_member(&name, declaration);
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
        let base = crate::intrinsic_table::base_name(name);
        let element = |verifier: &Self, ty: TypeId| verifier.innermost_element(ty);
        match base {
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
        matches!(
            self.module.ctx.constant(callee).as_global(),
            Some(GlobalRef::Function(id))
                if matches!(&self.module.function(id).name, Name::Named(name) if name == wanted)
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

    /// An alias needs something to alias. A declaration is not a definition,
    /// and neither is an `available_externally` body, which the linker is
    /// entitled to drop.
    fn alias_targets(&mut self) {
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

    /// Every metadata node a named list, a global or an instruction reaches,
    /// directly or through another node.
    fn reachable_metadata(&self) -> Vec<MdId> {
        let mut roots: Vec<MdId> = self
            .module
            .named_metadata
            .iter()
            .flat_map(|named| named.operands.clone())
            .collect();
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
            let Some(node) = self.module.metadata_node(id) else {
                continue;
            };
            match node {
                Metadata::Tuple { operands, .. } => {
                    roots.extend(operands.iter().filter_map(|operand| match operand {
                        MdOperand::Ref(id) => Some(*id),
                        _ => None,
                    }));
                }
                Metadata::Specialized { args, .. } => {
                    let fields: Vec<&MdField> = match args {
                        SpecializedArgs::Named(fields) => {
                            fields.iter().map(|(_, value)| value).collect()
                        }
                        SpecializedArgs::Positional(values) => values.iter().collect(),
                    };
                    roots.extend(fields.iter().filter_map(|field| match field {
                        MdField::Ref(id) => Some(*id),
                        _ => None,
                    }));
                }
                Metadata::String(_) => {}
            }
        }
        order
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

        self.calling_convention(function);
        self.function_attributes(function);
        self.attribute_set(
            &function.return_attrs,
            function.return_type,
            "the return value",
        );
        for (index, param) in function.params.iter().enumerate() {
            self.attribute_set(&param.attrs, param.ty, &format!("parameter {index}"));
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
                "alias.scope" | "noalias" => self.scope_list(&attachment.node, &where_),
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
            }
            InstKind::Store {
                value_type,
                value,
                atomic,
                ..
            } => {
                if atomic.is_some() {
                    self.check(
                        matches!(
                            self.module.ctx.type_kind(*value_type),
                            TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Pointer { .. }
                        ),
                        format!("{where_} stores a type an atomic cannot move"),
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
            }
            InstKind::CmpXchg {
                success, failure, ..
            } => {
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
                for (position, arg) in call.args.iter().enumerate() {
                    let attrs = arg.attrs.clone();
                    self.attribute_set(&attrs, arg.ty, &format!("argument {position} of {where_}"));
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
                if call.tail == TailKind::MustTail && call.calling_conv != function.calling_conv {
                    self.report(format!(
                        "{where_} is a musttail call whose convention differs from its caller's"
                    ));
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
                // `speculatable` promises something about a function, not
                // about one call to it.
                if call.fn_attrs.has(EnumAttr::Speculatable) {
                    self.report(format!(
                        "{where_} carries speculatable, which a call site may not"
                    ));
                }
                // An indirect call has no declaration to name an opaque type
                // in, so it may not produce one.
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
                    // LangRef documents what each intrinsic takes, and the
                    // positions whose type is the same in every documented
                    // instantiation are the ones a call has to get right.
                    if is_intrinsic
                        && let Name::Named(intrinsic) = callee.name.clone()
                        && let Some(documented) = crate::intrinsic_table::signature(&intrinsic)
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
            CastOp::AddrSpaceCast => pointer(self, from) && pointer(self, to),
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
        match self.module.ctx.type_kind(ty) {
            TypeKind::Void
            | TypeKind::Label
            | TypeKind::Metadata
            | TypeKind::Token
            | TypeKind::X86Amx
            | TypeKind::Function { .. } => false,
            // A struct is as sized as its fields, and a body it has not been
            // given yet has no layout at all.
            TypeKind::Struct { fields, .. } => self.struct_is_sized(&fields.clone()),
            TypeKind::NamedStruct(id) => match &self.module.ctx.struct_def(*id).fields {
                Some(fields) => self.struct_is_sized(&fields.clone()),
                None => false,
            },
            TypeKind::Array { element, .. } => self.has_a_fixed_size(*element),
            _ => true,
        }
    }

    /// A struct holds scalable vectors only when it holds nothing else.
    /// Mixing them with fixed-size fields leaves no offset for whatever
    /// follows the scalable one, which is why upstream refuses `{i32,
    /// <vscale x 1 x i32>}` and reads `{<vscale x 1 x i32>, <vscale x 1 x
    /// i32>}`.
    fn struct_is_sized(&self, fields: &[TypeId]) -> bool {
        if !fields.iter().all(|field| self.is_sized(*field)) {
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
        match self.module.ctx.type_kind(ty) {
            TypeKind::Void
            | TypeKind::Label
            | TypeKind::Metadata
            | TypeKind::Token
            | TypeKind::X86Amx
            | TypeKind::Function { .. } => false,
            TypeKind::Vector { scalable, .. } => !scalable,
            // Not target extension types: whether one can be a global is a
            // property of the target rather than of the IR, and upstream
            // reads `target("spirv.DeviceEvent")` while refusing
            // `target("opaque")`. Without that property we would refuse
            // both.
            TypeKind::Struct { fields, .. } => fields
                .clone()
                .iter()
                .all(|field| self.fits_in_a_global(*field)),
            TypeKind::NamedStruct(id) => match &self.module.ctx.struct_def(*id).fields {
                Some(fields) => fields
                    .clone()
                    .iter()
                    .all(|field| self.fits_in_a_global(*field)),
                None => true,
            },
            TypeKind::Array { element, .. } => self.fits_in_a_global(*element),
            _ => true,
        }
    }

    /// Whether a type has a size the layout can state as a number. A scalable
    /// vector has a size, but not one a global can use.
    fn has_a_fixed_size(&self, ty: TypeId) -> bool {
        if !self.is_sized(ty) {
            return false;
        }
        match self.module.ctx.type_kind(ty) {
            TypeKind::Vector { scalable, .. } => !scalable,
            TypeKind::Struct { fields, .. } => fields
                .clone()
                .iter()
                .all(|field| self.has_a_fixed_size(*field)),
            TypeKind::NamedStruct(id) => match &self.module.ctx.struct_def(*id).fields {
                Some(fields) => fields
                    .clone()
                    .iter()
                    .all(|field| self.has_a_fixed_size(*field)),
                None => false,
            },
            TypeKind::Array { element, .. } => self.has_a_fixed_size(*element),
            _ => true,
        }
    }

    /// The debug-info rules that are the verifier's rather than the
    /// parser's: the grammar is checked when the node is read, and this is
    /// what needs the node's neighbours to make sense of.
    fn debug_info_node(&mut self, node: &Metadata) {
        let Metadata::Specialized { tag, args, .. } = node else {
            return;
        };
        let SpecializedArgs::Named(fields) = args else {
            return;
        };
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
            let array = matches!(
                field_of(fields, "tag"),
                Some(MdField::Words(words)) if words.iter().any(|w| w == "DW_TAG_array_type")
            );
            let variant_part = matches!(
                field_of(fields, "tag"),
                Some(MdField::Words(words)) if words.iter().any(|w| w == "DW_TAG_variant_part")
            );
            // These four describe an array's shape, and a discriminator picks
            // between the arms of a variant.
            for shape in ["rank", "allocated", "associated", "dataLocation"] {
                if field_of(fields, shape).is_some() && !array {
                    self.report(format!("{shape} appears on a type that is not an array"));
                }
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
            let pointer = matches!(
                field_of(fields, "tag"),
                Some(MdField::Words(words))
                    if words.iter().any(|word| matches!(
                        word.as_str(),
                        "DW_TAG_pointer_type"
                            | "DW_TAG_reference_type"
                            | "DW_TAG_rvalue_reference_type"
                    ))
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
                Attribute::Enum(EnumAttr::SignExt | EnumAttr::ZeroExt | EnumAttr::NoExt) => {
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

    /// Rules about a function attribute's own value, which have nothing to do
    /// with the type it is attached to.
    fn function_attributes(&mut self, function: &Function) {
        let unnamed = function.qualifiers.unnamed_addr.is_some();
        let mut attributes: Vec<Attribute> = function.attrs.attributes.clone();
        for group in &function.attrs.groups {
            if let Some(contents) = self.module.attribute_group(*group) {
                attributes.extend(contents.iter().cloned());
            }
        }
        let has = |wanted: EnumAttr| {
            attributes
                .iter()
                .any(|a| matches!(a, Attribute::Enum(kind) if *kind == wanted))
        };
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
