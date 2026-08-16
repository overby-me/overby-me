//! Which identified structs a module still needs, and in what order.
//!
//! Upstream prints `%name = type ...` for the identified structs its type
//! finder reaches from the module, not for every one the parser made. A name
//! nothing refers to is dropped, and the order is the order the walk meets
//! them rather than the order they were written: a module defining `%A` then
//! `%B = type { %A }` and using only `%B` prints `%B` first, because `%B` is
//! what the global led to.
//!
//! The walk is the one upstream does: globals, then aliases and ifuncs, then
//! each function's signature followed by its instructions in order, and every
//! type reached is expanded into whatever it holds.

use std::collections::HashSet;

use llvm_ir::attribute::{Attribute, AttributeSet};
use llvm_ir::constant::{ConstId, Constant};
use llvm_ir::instruction::InstKind;
use llvm_ir::value::Value;
use llvm_ir::{Module, StructId, TypeId, TypeKind};

pub(crate) fn reachable_named_structs(module: &Module) -> Vec<StructId> {
    let mut walk = Walk {
        module,
        seen_types: HashSet::new(),
        seen_constants: HashSet::new(),
        found: Vec::new(),
    };

    for global in &module.globals {
        walk.ty(global.value_type);
        if let Some(initializer) = global.initializer {
            walk.constant(initializer);
        }
    }
    for alias in &module.aliases {
        walk.ty(alias.value_type);
        walk.constant(alias.aliasee);
    }
    for ifunc in &module.ifuncs {
        walk.ty(ifunc.value_type);
        walk.constant(ifunc.resolver);
    }
    // In the order they print rather than the order they were read, and
    // without the ones that do not print at all: upstream walks the module it
    // is about to write, so a declaration it erased in an upgrade takes its
    // types with it, and one it moved to the end meets them later.
    //
    // Measured on a module declaring `%late @llvm.ssa.copy(%late)` above a
    // function taking `%early`: upstream writes `%early` first, the renamed
    // declaration having moved past it. Nothing here reaches that yet, a
    // named struct being a mangling this does not build, so no file moves;
    // the walk is what upstream does either way.
    for index in module.function_print_order() {
        let function = &module.functions[index];
        walk.ty(function.return_type);
        walk.attributes(&function.return_attrs);
        for param in &function.params {
            walk.ty(param.ty);
            walk.attributes(&param.attrs);
        }
        for (id, _) in function.blocks() {
            for (_, instruction) in function.block_instructions(id) {
                walk.ty(instruction.ty);
                walk.instruction(&instruction.kind);
            }
        }
    }

    walk.found
}

struct Walk<'m> {
    module: &'m Module,
    seen_types: HashSet<TypeId>,
    seen_constants: HashSet<ConstId>,
    found: Vec<StructId>,
}

impl Walk<'_> {
    fn ty(&mut self, id: TypeId) {
        if !self.seen_types.insert(id) {
            return;
        }
        match self.module.ctx.type_kind(id).clone() {
            TypeKind::NamedStruct(struct_id) => {
                self.found.push(struct_id);
                if let Some(fields) = self.module.ctx.struct_def(struct_id).fields.clone() {
                    for field in fields {
                        self.ty(field);
                    }
                }
            }
            TypeKind::Struct { fields, .. } => {
                for field in fields {
                    self.ty(field);
                }
            }
            TypeKind::Array { element, .. } | TypeKind::Vector { element, .. } => self.ty(element),
            TypeKind::Function { result, params, .. } => {
                self.ty(result);
                for param in params {
                    self.ty(param);
                }
            }
            TypeKind::Target { types, .. } => {
                for held in types {
                    self.ty(held);
                }
            }
            _ => {}
        }
    }

    fn constant(&mut self, id: ConstId) {
        if !self.seen_constants.insert(id) {
            return;
        }
        let constant = self.module.ctx.constant(id).clone();
        self.ty(constant.ty());
        match constant {
            Constant::Struct { fields, .. } => {
                for field in fields {
                    self.constant(field);
                }
            }
            Constant::Array { elements, .. } | Constant::Vector { elements, .. } => {
                for element in elements {
                    self.constant(element);
                }
            }
            Constant::Splat { element, .. } => self.constant(element),
            Constant::Expression(expr) => {
                let (operands, types) = expr.parts();
                for operand in operands {
                    self.constant(operand);
                }
                for ty in types {
                    self.ty(ty);
                }
            }
            _ => {}
        }
    }

    /// A type named by an attribute rather than by an operand: `sret(%pair)`
    /// is the only place some structs are mentioned.
    fn attributes(&mut self, set: &AttributeSet) {
        for attribute in &set.attributes {
            match attribute {
                Attribute::Type { ty, .. } | Attribute::Range { ty, .. } => self.ty(*ty),
                _ => {}
            }
        }
    }

    fn instruction(&mut self, kind: &InstKind) {
        if let InstKind::Call(call)
        | InstKind::Invoke { call, .. }
        | InstKind::CallBr { call, .. } = kind
        {
            self.attributes(&call.return_attrs);
            self.attributes(&call.fn_attrs);
            for arg in &call.args {
                self.attributes(&arg.attrs);
            }
        }
        for ty in kind.named_types() {
            self.ty(ty);
        }
        for value in kind.operand_values() {
            if let Value::Constant(id) = value {
                self.constant(id);
            }
        }
    }
}
