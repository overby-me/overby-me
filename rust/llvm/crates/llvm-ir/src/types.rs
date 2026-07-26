//! The type system.
//!
//! Types are interned in a [`crate::Context`], so a `TypeId` is a `u32` and
//! type equality is id equality. Two flavours of struct exist for a reason
//! LangRef spells out: a literal struct is structural, so `{i32, i32}` written
//! twice is one type, while an identified struct has a name and an identity,
//! so `%a = type {i32}` and `%b = type {i32}` are different types that happen
//! to look alike. Only identified structs can be recursive or opaque.

use llvm_support::FloatSemantics;

/// An interned type.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TypeId(pub u32);

/// An identified (named) struct, which has identity rather than structure.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct StructId(pub u32);

/// The shape of a type.
///
/// `Pointer` carries no pointee: this dialect is opaque-pointer only, which
/// PLAN.md section 1.2 makes a non-goal to revisit.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum TypeKind {
    Void,
    Label,
    Metadata,
    Token,
    /// The x86 AMX tile type, which rustc never emits but upstream tests do.
    X86Amx,
    Integer(u32),
    Float(FloatSemantics),
    Pointer {
        address_space: u32,
    },
    Array {
        element: TypeId,
        count: u64,
    },
    Vector {
        element: TypeId,
        count: u64,
        scalable: bool,
    },
    /// A literal struct: `{i32, ptr}` or `<{i8, i32}>`.
    Struct {
        fields: Vec<TypeId>,
        packed: bool,
    },
    /// An identified struct: `%Foo`.
    NamedStruct(StructId),
    Function {
        result: TypeId,
        params: Vec<TypeId>,
        is_var_arg: bool,
    },
    /// A target extension type, `target("name", types..., ints...)`.
    Target {
        name: String,
        types: Vec<TypeId>,
        ints: Vec<u32>,
    },
}

/// The body of an identified struct. `fields` is `None` while the struct is
/// opaque, which is how a forward reference or a genuinely opaque type looks.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StructDef {
    pub name: String,
    pub fields: Option<Vec<TypeId>>,
    pub packed: bool,
}

impl TypeKind {
    pub fn is_first_class(&self) -> bool {
        !matches!(self, TypeKind::Void | TypeKind::Function { .. })
    }

    /// True for the types that can be the element of a vector or the operand
    /// of most arithmetic: integers, floats and pointers.
    pub fn is_single_value(&self) -> bool {
        matches!(
            self,
            TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Pointer { .. }
        )
    }

    pub fn is_aggregate(&self) -> bool {
        matches!(
            self,
            TypeKind::Array { .. } | TypeKind::Struct { .. } | TypeKind::NamedStruct(_)
        )
    }

    pub fn as_integer(&self) -> Option<u32> {
        match self {
            TypeKind::Integer(bits) => Some(*bits),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<FloatSemantics> {
        match self {
            TypeKind::Float(semantics) => Some(*semantics),
            _ => None,
        }
    }

    pub fn as_vector(&self) -> Option<(TypeId, u64, bool)> {
        match self {
            TypeKind::Vector {
                element,
                count,
                scalable,
            } => Some((*element, *count, *scalable)),
            _ => None,
        }
    }

    pub fn as_function(&self) -> Option<(TypeId, &[TypeId], bool)> {
        match self {
            TypeKind::Function {
                result,
                params,
                is_var_arg,
            } => Some((*result, params, *is_var_arg)),
            _ => None,
        }
    }

    pub fn pointer_address_space(&self) -> Option<u32> {
        match self {
            TypeKind::Pointer { address_space } => Some(*address_space),
            _ => None,
        }
    }
}
