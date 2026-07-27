//! The interning context.
//!
//! Types, identified structs and constants live here. A [`crate::Module`] owns
//! its context, so a module is self-contained and `Send + Sync` with no
//! shared-context bookkeeping; the price is that ids are only meaningful
//! together with the module that minted them, and merging two modules is an
//! explicit operation rather than something that happens by construction.

use std::collections::HashMap;

use crate::constant::{ConstId, Constant};
use crate::types::{StructDef, StructId, TypeId, TypeKind};
use llvm_support::{ApInt, FloatSemantics};

/// Interned types, structs and constants.
#[derive(Clone, Debug)]
pub struct Context {
    types: Vec<TypeKind>,
    type_ids: HashMap<TypeKind, TypeId>,
    structs: Vec<StructDef>,
    struct_ids: HashMap<String, StructId>,
    /// `%name = type [8 x i8]` names a type that is not a struct. Upstream
    /// expands those where they are used and never prints them, because only
    /// identified structs have identity; this table is how that expansion
    /// happens.
    type_aliases: HashMap<String, TypeId>,
    constants: Vec<Constant>,
    constant_ids: HashMap<Constant, ConstId>,
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    pub fn new() -> Context {
        Context {
            types: Vec::new(),
            type_ids: HashMap::new(),
            structs: Vec::new(),
            struct_ids: HashMap::new(),
            type_aliases: HashMap::new(),
            constants: Vec::new(),
            constant_ids: HashMap::new(),
        }
    }

    pub fn intern_type(&mut self, kind: TypeKind) -> TypeId {
        if let Some(id) = self.type_ids.get(&kind) {
            return *id;
        }
        let id = TypeId(self.types.len() as u32);
        self.types.push(kind.clone());
        self.type_ids.insert(kind, id);
        id
    }

    pub fn type_kind(&self, id: TypeId) -> &TypeKind {
        &self.types[id.0 as usize]
    }

    pub fn void_type(&mut self) -> TypeId {
        self.intern_type(TypeKind::Void)
    }

    pub fn label_type(&mut self) -> TypeId {
        self.intern_type(TypeKind::Label)
    }

    pub fn metadata_type(&mut self) -> TypeId {
        self.intern_type(TypeKind::Metadata)
    }

    pub fn token_type(&mut self) -> TypeId {
        self.intern_type(TypeKind::Token)
    }

    pub fn int_type(&mut self, bits: u32) -> TypeId {
        self.intern_type(TypeKind::Integer(bits))
    }

    pub fn float_type(&mut self, semantics: FloatSemantics) -> TypeId {
        self.intern_type(TypeKind::Float(semantics))
    }

    pub fn pointer_type(&mut self, address_space: u32) -> TypeId {
        self.intern_type(TypeKind::Pointer { address_space })
    }

    pub fn array_type(&mut self, element: TypeId, count: u64) -> TypeId {
        self.intern_type(TypeKind::Array { element, count })
    }

    pub fn vector_type(&mut self, element: TypeId, count: u64, scalable: bool) -> TypeId {
        self.intern_type(TypeKind::Vector {
            element,
            count,
            scalable,
        })
    }

    pub fn struct_type(&mut self, fields: Vec<TypeId>, packed: bool) -> TypeId {
        self.intern_type(TypeKind::Struct { fields, packed })
    }

    pub fn function_type(
        &mut self,
        result: TypeId,
        params: Vec<TypeId>,
        is_var_arg: bool,
    ) -> TypeId {
        self.intern_type(TypeKind::Function {
            result,
            params,
            is_var_arg,
        })
    }

    /// The identified struct with this name, created opaque if it is new.
    ///
    /// A forward reference and a genuine `type opaque` are the same thing
    /// until a body arrives, which is what makes recursive types expressible.
    pub fn named_struct(&mut self, name: &str) -> StructId {
        if let Some(id) = self.struct_ids.get(name) {
            return *id;
        }
        let id = StructId(self.structs.len() as u32);
        self.structs.push(StructDef {
            name: name.to_string(),
            fields: None,
            packed: false,
            numbered: false,
        });
        self.struct_ids.insert(name.to_string(), id);
        id
    }

    /// Creates an identified struct under a name nobody else has, appending a
    /// suffix the way upstream does when two structs want the same name.
    pub fn unique_named_struct(&mut self, preferred: &str) -> StructId {
        if !self.struct_ids.contains_key(preferred) {
            return self.named_struct(preferred);
        }
        let mut counter = 0u32;
        loop {
            let candidate = format!("{preferred}.{counter}");
            if !self.struct_ids.contains_key(&candidate) {
                return self.named_struct(&candidate);
            }
            counter += 1;
        }
    }

    /// Marks a struct as one named by number rather than by word.
    pub fn set_struct_numbered(&mut self, id: StructId) {
        self.structs[id.0 as usize].numbered = true;
    }

    pub fn set_struct_body(&mut self, id: StructId, fields: Vec<TypeId>, packed: bool) {
        let def = &mut self.structs[id.0 as usize];
        def.fields = Some(fields);
        def.packed = packed;
    }

    pub fn struct_def(&self, id: StructId) -> &StructDef {
        &self.structs[id.0 as usize]
    }

    pub fn named_struct_type(&mut self, id: StructId) -> TypeId {
        self.intern_type(TypeKind::NamedStruct(id))
    }

    /// Records `%name = type <something that is not a struct>`.
    pub fn set_type_alias(&mut self, name: &str, ty: TypeId) {
        self.type_aliases.insert(name.to_string(), ty);
    }

    pub fn lookup_type_alias(&self, name: &str) -> Option<TypeId> {
        self.type_aliases.get(name).copied()
    }

    pub fn lookup_named_struct(&self, name: &str) -> Option<StructId> {
        self.struct_ids.get(name).copied()
    }

    /// Every identified struct, in creation order, which is the order they
    /// print in.
    pub fn named_structs(&self) -> impl Iterator<Item = (StructId, &StructDef)> {
        self.structs
            .iter()
            .enumerate()
            .map(|(index, def)| (StructId(index as u32), def))
    }

    pub fn intern_constant(&mut self, constant: Constant) -> ConstId {
        if let Some(id) = self.constant_ids.get(&constant) {
            return *id;
        }
        let id = ConstId(self.constants.len() as u32);
        self.constants.push(constant.clone());
        self.constant_ids.insert(constant, id);
        id
    }

    pub fn constant(&self, id: ConstId) -> &Constant {
        &self.constants[id.0 as usize]
    }

    pub fn const_int(&mut self, ty: TypeId, value: ApInt) -> ConstId {
        self.intern_constant(Constant::Integer { ty, value })
    }

    /// `i<bits> <value>`, the shortest way to write a small literal.
    pub fn const_int_of(&mut self, bits: u32, value: i128) -> ConstId {
        let ty = self.int_type(bits);
        self.const_int(ty, ApInt::from_i128(bits, value))
    }

    pub fn const_bool(&mut self, value: bool) -> ConstId {
        self.const_int_of(1, i128::from(value))
    }

    pub fn const_null(&mut self, ty: TypeId) -> ConstId {
        self.intern_constant(Constant::Null(ty))
    }

    pub fn const_undef(&mut self, ty: TypeId) -> ConstId {
        self.intern_constant(Constant::Undef(ty))
    }

    pub fn const_poison(&mut self, ty: TypeId) -> ConstId {
        self.intern_constant(Constant::Poison(ty))
    }

    pub fn const_zero_initializer(&mut self, ty: TypeId) -> ConstId {
        self.intern_constant(Constant::ZeroInitializer(ty))
    }

    pub fn type_count(&self) -> usize {
        self.types.len()
    }

    pub fn constant_count(&self) -> usize {
        self.constants.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_are_interned_structurally() {
        let mut ctx = Context::new();
        let i32a = ctx.int_type(32);
        let i32b = ctx.int_type(32);
        assert_eq!(i32a, i32b);
        assert_ne!(i32a, ctx.int_type(64));

        let literal_a = ctx.struct_type(vec![i32a, i32a], false);
        let literal_b = ctx.struct_type(vec![i32a, i32a], false);
        assert_eq!(literal_a, literal_b, "literal structs unify");
        assert_ne!(
            literal_a,
            ctx.struct_type(vec![i32a, i32a], true),
            "packed is part of the type"
        );
    }

    #[test]
    fn named_structs_have_identity_rather_than_structure() {
        let mut ctx = Context::new();
        let i32 = ctx.int_type(32);
        let a = ctx.named_struct("a");
        let b = ctx.named_struct("b");
        ctx.set_struct_body(a, vec![i32], false);
        ctx.set_struct_body(b, vec![i32], false);
        let ta = ctx.named_struct_type(a);
        let tb = ctx.named_struct_type(b);
        assert_ne!(ta, tb, "identical bodies stay different types");
        assert_eq!(
            ctx.named_struct("a"),
            a,
            "the second lookup finds the first"
        );
    }

    #[test]
    fn a_named_struct_starts_opaque_and_can_be_recursive() {
        let mut ctx = Context::new();
        let node = ctx.named_struct("Node");
        assert!(ctx.struct_def(node).fields.is_none(), "opaque until bodied");
        let node_ty = ctx.named_struct_type(node);
        let ptr = ctx.pointer_type(0);
        ctx.set_struct_body(node, vec![ptr, node_ty], false);
        assert_eq!(
            ctx.struct_def(node).fields.as_deref(),
            Some([ptr, node_ty].as_slice())
        );
    }

    #[test]
    fn a_name_collision_gets_a_suffix() {
        let mut ctx = Context::new();
        let first = ctx.unique_named_struct("pair");
        let second = ctx.unique_named_struct("pair");
        let third = ctx.unique_named_struct("pair");
        assert_ne!(first, second);
        assert_eq!(ctx.struct_def(first).name, "pair");
        assert_eq!(ctx.struct_def(second).name, "pair.0");
        assert_eq!(ctx.struct_def(third).name, "pair.1");
    }

    #[test]
    fn constants_are_interned() {
        let mut ctx = Context::new();
        let a = ctx.const_int_of(32, 7);
        let b = ctx.const_int_of(32, 7);
        assert_eq!(a, b);
        assert_ne!(a, ctx.const_int_of(32, 8));
        assert_ne!(a, ctx.const_int_of(64, 7), "width is part of the constant");
        assert_eq!(ctx.constant(a).as_integer().unwrap().to_u64(), Some(7));
        assert_eq!(ctx.constant_count(), 3);
    }

    #[test]
    fn negative_constants_are_stored_two_s_complement() {
        let mut ctx = Context::new();
        let minus_one = ctx.const_int_of(8, -1);
        assert_eq!(
            ctx.constant(minus_one).as_integer().unwrap().to_u64(),
            Some(255)
        );
        assert_eq!(
            ctx.constant(minus_one)
                .as_integer()
                .unwrap()
                .to_string_signed(),
            "-1"
        );
    }
}
