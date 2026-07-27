//! Constants and constant expressions.
//!
//! Constants are interned in the [`crate::Context`] alongside types, so
//! `ConstId` equality is value equality. Every variant carries its own type
//! rather than deriving it, because a constant is asked for its type far more
//! often than it is built, and a reference to a global would otherwise need
//! the module to answer.
//!
//! Constant *expressions* are a shrinking set upstream: the arithmetic ones
//! were removed over several releases, and what remains here is what LLVM 21
//! still accepts. Refusing the rest is the point.

use crate::attribute::AsmDialect;
use crate::metadata::MdOperand;
use crate::types::TypeId;
use crate::value::{GlobalRef, Name};
use llvm_support::{ApFloat, ApInt};

/// An interned constant.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ConstId(pub u32);

/// Cast opcodes, shared by cast instructions and cast constant expressions.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CastOp {
    Trunc,
    ZExt,
    SExt,
    FpTrunc,
    FpExt,
    FpToUi,
    FpToSi,
    UiToFp,
    SiToFp,
    PtrToInt,
    PtrToAddr,
    IntToPtr,
    BitCast,
    AddrSpaceCast,
}

impl CastOp {
    pub fn keyword(self) -> &'static str {
        match self {
            CastOp::Trunc => "trunc",
            CastOp::ZExt => "zext",
            CastOp::SExt => "sext",
            CastOp::FpTrunc => "fptrunc",
            CastOp::FpExt => "fpext",
            CastOp::FpToUi => "fptoui",
            CastOp::FpToSi => "fptosi",
            CastOp::UiToFp => "uitofp",
            CastOp::SiToFp => "sitofp",
            CastOp::PtrToInt => "ptrtoint",
            CastOp::PtrToAddr => "ptrtoaddr",
            CastOp::IntToPtr => "inttoptr",
            CastOp::BitCast => "bitcast",
            CastOp::AddrSpaceCast => "addrspacecast",
        }
    }

    pub fn from_keyword(word: &str) -> Option<CastOp> {
        Some(match word {
            "trunc" => CastOp::Trunc,
            "zext" => CastOp::ZExt,
            "sext" => CastOp::SExt,
            "fptrunc" => CastOp::FpTrunc,
            "fpext" => CastOp::FpExt,
            "fptoui" => CastOp::FpToUi,
            "fptosi" => CastOp::FpToSi,
            "uitofp" => CastOp::UiToFp,
            "sitofp" => CastOp::SiToFp,
            "ptrtoint" => CastOp::PtrToInt,
            "ptrtoaddr" => CastOp::PtrToAddr,
            "inttoptr" => CastOp::IntToPtr,
            "bitcast" => CastOp::BitCast,
            "addrspacecast" => CastOp::AddrSpaceCast,
            _ => return None,
        })
    }
}

/// The no-wrap flags a `getelementptr` can carry.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct GepFlags {
    pub inbounds: bool,
    /// `nusw`: no unsigned-signed wrap. Implied by `inbounds`, but written
    /// separately when it stands alone.
    pub nusw: bool,
    pub nuw: bool,
}

impl GepFlags {
    pub fn is_empty(self) -> bool {
        !self.inbounds && !self.nusw && !self.nuw
    }
}

/// A module-level inline assembly value, which is a callee rather than a
/// constant in upstream's hierarchy but behaves like one here.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct InlineAsm {
    /// The function type the assembly is called through.
    pub function_type: TypeId,
    pub assembly: String,
    pub constraints: String,
    pub has_side_effects: bool,
    pub align_stack: bool,
    pub dialect: AsmDialect,
    /// `unwind`: the assembly may throw.
    pub can_unwind: bool,
}

/// A constant.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Constant {
    Integer {
        ty: TypeId,
        value: ApInt,
    },
    Float {
        ty: TypeId,
        value: ApFloat,
    },
    /// `null`, only for pointers.
    Null(TypeId),
    /// `none`, only for tokens.
    NoneToken(TypeId),
    Undef(TypeId),
    Poison(TypeId),
    ZeroInitializer(TypeId),
    Struct {
        ty: TypeId,
        fields: Vec<ConstId>,
    },
    Array {
        ty: TypeId,
        elements: Vec<ConstId>,
    },
    /// The `c"..."` spelling of an `i8` array. Kept distinct from `Array` so
    /// that a string prints back as a string.
    String {
        ty: TypeId,
        bytes: Vec<u8>,
    },
    Vector {
        ty: TypeId,
        elements: Vec<ConstId>,
    },
    /// `splat (i32 7)`, a vector every lane of which holds one value.
    Splat {
        ty: TypeId,
        element: ConstId,
    },
    /// `ptrauth (ptr @f, i32 0, i64 1234, ptr @disc)`: a pointer signed with
    /// a key, a discriminator and an address discriminator, the last two of
    /// which are optional.
    PtrAuth {
        ty: TypeId,
        pointer: ConstId,
        key: ConstId,
        discriminator: Option<ConstId>,
        address_discriminator: Option<ConstId>,
    },
    /// A reference to something spelled with a leading `@`. The type is always
    /// a pointer, in the global's own address space.
    Global {
        ty: TypeId,
        target: GlobalRef,
    },
    /// `blockaddress(@f, %block)`. The block is held by name because it can
    /// refer forward into a function that has not been parsed yet.
    BlockAddress {
        ty: TypeId,
        function: GlobalRef,
        block: Name,
    },
    /// `dso_local_equivalent @f`.
    DsoLocalEquivalent {
        ty: TypeId,
        target: GlobalRef,
    },
    /// `no_cfi @f`.
    NoCfiValue {
        ty: TypeId,
        target: GlobalRef,
    },
    /// Metadata where a value is expected, as in a debug intrinsic's
    /// argument. It is a whole metadata operand rather than a reference,
    /// because `metadata ptr %s` and `metadata !DIExpression()` are both
    /// legal there.
    Metadata {
        ty: TypeId,
        operand: Box<MdOperand>,
    },
    InlineAsm(Box<InlineAsm>),
    Expression(Box<ConstExpr>),
}

impl Constant {
    pub fn ty(&self) -> TypeId {
        match self {
            Constant::Integer { ty, .. }
            | Constant::Float { ty, .. }
            | Constant::Null(ty)
            | Constant::NoneToken(ty)
            | Constant::Undef(ty)
            | Constant::Poison(ty)
            | Constant::ZeroInitializer(ty)
            | Constant::Struct { ty, .. }
            | Constant::Array { ty, .. }
            | Constant::String { ty, .. }
            | Constant::Vector { ty, .. }
            | Constant::Splat { ty, .. }
            | Constant::PtrAuth { ty, .. }
            | Constant::Global { ty, .. }
            | Constant::BlockAddress { ty, .. }
            | Constant::DsoLocalEquivalent { ty, .. }
            | Constant::NoCfiValue { ty, .. }
            | Constant::Metadata { ty, .. } => *ty,
            Constant::InlineAsm(asm) => asm.function_type,
            Constant::Expression(expr) => expr.ty(),
        }
    }

    pub fn as_integer(&self) -> Option<&ApInt> {
        match self {
            Constant::Integer { value, .. } => Some(value),
            _ => None,
        }
    }

    pub fn as_global(&self) -> Option<GlobalRef> {
        match self {
            Constant::Global { target, .. } => Some(*target),
            _ => None,
        }
    }
}

/// The constant expressions LLVM 21 still has.
///
/// Everything else (`add`, `sub`, `mul`, `select`, `icmp`, the removed casts)
/// is rejected by the parser rather than accepted and lowered, because a
/// module that still uses them is written for an older dialect.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ConstExpr {
    /// `add`, `sub` and `xor` outlived the other arithmetic expressions.
    Binary {
        op: crate::instruction::BinOp,
        /// `add nuw nsw (...)`: the wrapping flags a constant expression can
        /// carry, exactly as the instruction can.
        flags: crate::instruction::IntFlags,
        lhs: ConstId,
        rhs: ConstId,
        ty: TypeId,
    },
    Cast {
        op: CastOp,
        operand: ConstId,
        ty: TypeId,
    },
    GetElementPtr {
        /// The type being indexed, which opaque pointers force to be written
        /// out rather than recovered from the pointer.
        source_type: TypeId,
        base: ConstId,
        indices: Vec<ConstId>,
        flags: GepFlags,
        /// `inrange(lower, upper)`, a byte range the result may not leave.
        inrange: Option<(i64, i64)>,
        ty: TypeId,
    },
    ExtractElement {
        vector: ConstId,
        index: ConstId,
        ty: TypeId,
    },
    InsertElement {
        vector: ConstId,
        element: ConstId,
        index: ConstId,
        ty: TypeId,
    },
    ShuffleVector {
        first: ConstId,
        second: ConstId,
        mask: ConstId,
        ty: TypeId,
    },
}

impl ConstExpr {
    /// The constants and types this expression names, which is what a type
    /// finder walks.
    pub fn parts(&self) -> (Vec<ConstId>, Vec<TypeId>) {
        match self {
            ConstExpr::Binary { lhs, rhs, ty, .. } => (vec![*lhs, *rhs], vec![*ty]),
            ConstExpr::Cast { operand, ty, .. } => (vec![*operand], vec![*ty]),
            ConstExpr::GetElementPtr {
                source_type,
                base,
                indices,
                ty,
                ..
            } => {
                let mut operands = vec![*base];
                operands.extend(indices.iter().copied());
                (operands, vec![*source_type, *ty])
            }
            ConstExpr::ExtractElement { vector, index, ty } => (vec![*vector, *index], vec![*ty]),
            ConstExpr::InsertElement {
                vector,
                element,
                index,
                ty,
            } => (vec![*vector, *element, *index], vec![*ty]),
            ConstExpr::ShuffleVector {
                first,
                second,
                mask,
                ty,
            } => (vec![*first, *second, *mask], vec![*ty]),
        }
    }

    pub fn ty(&self) -> TypeId {
        match self {
            ConstExpr::Binary { ty, .. }
            | ConstExpr::Cast { ty, .. }
            | ConstExpr::GetElementPtr { ty, .. }
            | ConstExpr::ExtractElement { ty, .. }
            | ConstExpr::InsertElement { ty, .. }
            | ConstExpr::ShuffleVector { ty, .. } => *ty,
        }
    }

    pub fn keyword(&self) -> &'static str {
        match self {
            ConstExpr::Binary { op, .. } => op.keyword(),
            ConstExpr::Cast { op, .. } => op.keyword(),
            ConstExpr::GetElementPtr { .. } => "getelementptr",
            ConstExpr::ExtractElement { .. } => "extractelement",
            ConstExpr::InsertElement { .. } => "insertelement",
            ConstExpr::ShuffleVector { .. } => "shufflevector",
        }
    }
}
