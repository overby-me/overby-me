//! Shared cast and float operation classification, plus F128 soft-float libcall mapping.
//!
//! All four backends use the same decision logic to determine what kind of cast
//! to emit — only the actual machine instructions differ. By classifying the cast
//! once in shared code, we eliminate duplicated Ptr-normalization and F128-reduction
//! logic from each backend. This module also provides the shared mnemonic-to-libcall
//! mapping for F128 soft-float arithmetic and comparisons (ARM, RISC-V).

use crate::common::types::IrType;
use crate::ir::reexports::{IrBinOp, IrCmpOp, IrConst, Operand};

/// Classification of type casts. All four backends use the same control flow
/// to decide which kind of cast to emit; only the actual instructions differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastKind {
    /// No conversion needed (same type, or Ptr <-> I64/U64, or F128 <-> F64).
    Noop,
    /// Float to signed integer (from_ty is F32 or F64).
    FloatToSigned { from_f64: bool },
    /// Float to unsigned integer (from_ty is F32 or F64).
    FloatToUnsigned { from_f64: bool, to_u64: bool },
    /// Signed integer to float (to_ty is F32 or F64).
    /// `from_ty` is the source integer type, needed to sign-extend sub-64-bit values
    /// before conversion (e.g., I32 in rax must be sign-extended to 64 bits).
    SignedToFloat { to_f64: bool, from_ty: IrType },
    /// Unsigned integer to float. `from_ty` is the source unsigned integer type,
    /// needed for proper zero-extension on RISC-V (where W-suffix instructions
    /// sign-extend) and for U64 overflow handling on x86.
    UnsignedToFloat { to_f64: bool, from_ty: IrType },
    /// Float-to-float conversion (F32 <-> F64).
    FloatToFloat { widen: bool },
    /// Integer widening: sign- or zero-extend a smaller type to a larger one.
    IntWiden { from_ty: IrType, to_ty: IrType },
    /// Integer narrowing: truncate a larger type to a smaller one.
    IntNarrow { to_ty: IrType },
    /// Same-size signed-to-unsigned (need to mask/clear upper bits).
    SignedToUnsignedSameSize { to_ty: IrType },
    /// Same-size unsigned-to-signed. On most architectures this is a noop,
    /// but on RISC-V 64-bit, U32->I32 needs sign-extension because the ABI
    /// requires all 32-bit values to be sign-extended in 64-bit registers.
    UnsignedToSignedSameSize { to_ty: IrType },
    /// Signed integer -> F128 via softfloat (__floatsitf / __floatditf).
    /// Used on ARM/RISC-V where long double is IEEE binary128.
    SignedToF128 { from_ty: IrType },
    /// Unsigned integer -> F128 via softfloat (__floatunsitf / __floatunditf).
    UnsignedToF128 { from_ty: IrType },
    /// F128 -> signed integer via softfloat (__fixtfsi / __fixtfdi).
    F128ToSigned { to_ty: IrType },
    /// F128 -> unsigned integer via softfloat (__fixunstfsi / __fixunstfdi).
    F128ToUnsigned { to_ty: IrType },
    /// F32/F64 -> F128 widening via softfloat (__extendsftf2 / __extenddftf2).
    FloatToF128 { from_f32: bool },
    /// F128 -> F32/F64 narrowing via softfloat (__trunctfsf2 / __trunctfdf2).
    F128ToFloat { to_f32: bool },
}

/// Classify a cast between two IR types. This captures the shared decision logic
/// that all four backends use identically. Backends then match on the returned
/// `CastKind` to emit architecture-specific instructions.
///
/// Handles Ptr normalization (Ptr treated as U64) and F128 reduction (F128 treated
/// as F64 for computation purposes on x86) before classification.
///
/// `f128_is_native`: true on ARM/RISC-V where F128 is IEEE binary128 and requires
/// softfloat library calls for conversions. false on x86 where F128 is x87 80-bit
/// and is approximated as F64.
pub fn classify_cast_with_f128(from_ty: IrType, to_ty: IrType, f128_is_native: bool) -> CastKind {
    if from_ty == to_ty {
        return CastKind::Noop;
    }

    // F128 (long double) handling depends on architecture.
    if from_ty == IrType::F128 || to_ty == IrType::F128 {
        if f128_is_native {
            // ARM/RISC-V: F128 is true IEEE binary128. Use softfloat library calls.
            return classify_f128_cast_native(from_ty, to_ty);
        }
        // x86: F128 (x87 80-bit) is computed as F64. Treat F128 <-> F64 as noop,
        // and F128 <-> other as F64 <-> other.
        let effective_from = if from_ty == IrType::F128 {
            IrType::F64
        } else {
            from_ty
        };
        let effective_to = if to_ty == IrType::F128 {
            IrType::F64
        } else {
            to_ty
        };
        if effective_from == effective_to {
            return CastKind::Noop;
        }
        return classify_cast(effective_from, effective_to);
    }

    // Ptr is equivalent to U64 on LP64 targets, U32 on ILP32 targets.
    if (from_ty == IrType::Ptr || to_ty == IrType::Ptr) && !from_ty.is_float() && !to_ty.is_float()
    {
        let ptr_int_ty = if crate::common::types::target_is_32bit() {
            IrType::U32
        } else {
            IrType::U64
        };
        let effective_from = if from_ty == IrType::Ptr {
            ptr_int_ty
        } else {
            from_ty
        };
        let effective_to = if to_ty == IrType::Ptr {
            ptr_int_ty
        } else {
            to_ty
        };
        let ptr_sz = crate::common::types::target_ptr_size();
        if effective_from == effective_to
            || (effective_from.size() == ptr_sz && effective_to.size() == ptr_sz)
        {
            return CastKind::Noop;
        }
        return classify_cast(effective_from, effective_to);
    }

    // Float-to-int
    if from_ty.is_float() && !to_ty.is_float() {
        let is_unsigned_dest = to_ty.is_unsigned() || to_ty == IrType::Ptr;
        let from_f64 = from_ty == IrType::F64;
        if is_unsigned_dest {
            let to_u64 = to_ty == IrType::U64 || to_ty == IrType::Ptr;
            return CastKind::FloatToUnsigned { from_f64, to_u64 };
        } else {
            return CastKind::FloatToSigned { from_f64 };
        }
    }

    // Int-to-float
    if !from_ty.is_float() && to_ty.is_float() {
        let is_unsigned_src = from_ty.is_unsigned();
        let to_f64 = to_ty == IrType::F64;
        if is_unsigned_src {
            return CastKind::UnsignedToFloat { to_f64, from_ty };
        } else {
            return CastKind::SignedToFloat { to_f64, from_ty };
        }
    }

    // Float-to-float
    if from_ty.is_float() && to_ty.is_float() {
        let widen = from_ty == IrType::F32 && to_ty == IrType::F64;
        return CastKind::FloatToFloat { widen };
    }

    // Integer-to-integer
    let from_size = from_ty.size();
    let to_size = to_ty.size();

    if from_size == to_size {
        if from_ty.is_signed() && to_ty.is_unsigned() {
            return CastKind::SignedToUnsignedSameSize { to_ty };
        }
        if from_ty.is_unsigned() && to_ty.is_signed() {
            return CastKind::UnsignedToSignedSameSize { to_ty };
        }
        return CastKind::Noop;
    }

    if to_size > from_size {
        return CastKind::IntWiden { from_ty, to_ty };
    }

    CastKind::IntNarrow { to_ty }
}

/// Backward-compatible wrapper: classifies casts with x86 F128 semantics
/// (F128 treated as F64 approximation).
pub fn classify_cast(from_ty: IrType, to_ty: IrType) -> CastKind {
    classify_cast_with_f128(from_ty, to_ty, false)
}

/// Classify F128 casts on targets where F128 is true IEEE binary128 (ARM/RISC-V).
/// These require softfloat library calls for full precision.
fn classify_f128_cast_native(from_ty: IrType, to_ty: IrType) -> CastKind {
    debug_assert!(from_ty == IrType::F128 || to_ty == IrType::F128);

    if to_ty == IrType::F128 {
        // Something -> F128
        if from_ty == IrType::F64 {
            return CastKind::FloatToF128 { from_f32: false };
        }
        if from_ty == IrType::F32 {
            return CastKind::FloatToF128 { from_f32: true };
        }
        // Integer -> F128
        if from_ty.is_float() {
            // F128 -> F128: should not happen (handled by from_ty == to_ty check)
            return CastKind::Noop;
        }
        if from_ty.is_unsigned() {
            return CastKind::UnsignedToF128 { from_ty };
        }
        return CastKind::SignedToF128 { from_ty };
    }

    // F128 -> something
    if to_ty == IrType::F64 {
        return CastKind::F128ToFloat { to_f32: false };
    }
    if to_ty == IrType::F32 {
        return CastKind::F128ToFloat { to_f32: true };
    }
    // F128 -> integer
    if to_ty.is_float() {
        return CastKind::Noop;
    }
    if to_ty.is_unsigned() || to_ty == IrType::Ptr {
        return CastKind::F128ToUnsigned { to_ty };
    }
    CastKind::F128ToSigned { to_ty }
}

/// Float arithmetic operations that all four backends support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Classify a binary operation on floats. Returns None if the operation is not
/// meaningful on floats (e.g., bitwise And, Or, Xor, shifts, integer remainder).
pub fn classify_float_binop(op: IrBinOp) -> Option<FloatOp> {
    match op {
        IrBinOp::Add => Some(FloatOp::Add),
        IrBinOp::Sub => Some(FloatOp::Sub),
        IrBinOp::Mul => Some(FloatOp::Mul),
        IrBinOp::SDiv | IrBinOp::UDiv => Some(FloatOp::Div),
        _ => None,
    }
}

/// Map a float binop mnemonic (fadd/fsub/fmul/fdiv) to the corresponding F128
/// soft-float libcall. Used by ARM and RISC-V backends (x86 uses x87 for F128).
/// Returns None for unrecognized mnemonics (caller should fall back to f64 hardware).
/// How to interpret an F128 comparison libcall result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum F128CmpKind {
    /// Result == 0 means true (equality)
    Eq,
    /// Result != 0 means true (inequality)
    Ne,
    /// Result < 0 means true (less than)
    Lt,
    /// Result <= 0 means true (less or equal)
    Le,
    /// Result > 0 means true (greater than)
    Gt,
    /// Result >= 0 means true (greater or equal)
    Ge,
}

/// Map a comparison operation to the F128 soft-float libcall and result interpretation.
pub fn f128_cmp_libcall(op: IrCmpOp) -> (&'static str, F128CmpKind) {
    match op {
        IrCmpOp::Eq => ("__eqtf2", F128CmpKind::Eq),
        IrCmpOp::Ne => ("__eqtf2", F128CmpKind::Ne),
        IrCmpOp::Slt | IrCmpOp::Ult => ("__lttf2", F128CmpKind::Lt),
        IrCmpOp::Sle | IrCmpOp::Ule => ("__letf2", F128CmpKind::Le),
        IrCmpOp::Sgt | IrCmpOp::Ugt => ("__gttf2", F128CmpKind::Gt),
        IrCmpOp::Sge | IrCmpOp::Uge => ("__getf2", F128CmpKind::Ge),
    }
}

/// Extract the IEEE f128 low/high u64 halves from an F128 constant operand.
/// The f128 bytes are already in IEEE binary128 format.
/// Returns None for non-constant operands (caller must use runtime conversion).
pub fn f128_const_halves(op: &Operand) -> Option<(u64, u64)> {
    if let Operand::Const(IrConst::LongDouble(_, f128_bytes)) = op {
        let lo = u64::from_le_bytes(f128_bytes[0..8].try_into().unwrap());
        let hi = u64::from_le_bytes(f128_bytes[8..16].try_into().unwrap());
        Some((lo, hi))
    } else {
        None
    }
}
