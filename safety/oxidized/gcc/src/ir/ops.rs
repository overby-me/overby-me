//! IR operation enums: binary, unary, comparison, and atomic operations.
//!
//! Each enum carries its own evaluation methods (eval_i64, eval_i128, eval_f64)
//! for use by constant folding and simplification passes.

/// Atomic read-modify-write operations.
#[derive(Debug, Clone, Copy)]
pub enum AtomicRmwOp {
    /// Add: *ptr += val
    Add,
    /// Sub: *ptr -= val
    Sub,
    /// And: *ptr &= val
    And,
    /// Or: *ptr |= val
    Or,
    /// Xor: *ptr ^= val
    Xor,
    /// Nand: *ptr = ~(*ptr & val)
    Nand,
    /// Exchange: *ptr = val (returns old value)
    Xchg,
    /// Test and set: *ptr = 1 (returns old value)
    TestAndSet,
}

/// Memory ordering for atomic operations.
#[derive(Debug, Clone, Copy)]
pub enum AtomicOrdering {
    Relaxed,
    Acquire,
    Release,
    AcqRel,
    SeqCst,
}

/// Binary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IrBinOp {
    Add,
    Sub,
    Mul,
    SDiv,
    UDiv,
    SRem,
    URem,
    And,
    Or,
    Xor,
    Shl,
    AShr,
    LShr,
}

impl IrBinOp {
    /// Returns true if this operation is commutative (a op b == b op a).
    pub fn is_commutative(self) -> bool {
        matches!(
            self,
            IrBinOp::Add | IrBinOp::Mul | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor
        )
    }

    /// Returns true if this operation can trap at runtime (e.g., divide by zero causes SIGFPE).
    /// Such operations must not be speculatively executed by if-conversion.
    pub fn can_trap(self) -> bool {
        matches!(
            self,
            IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem
        )
    }

    /// Evaluate this binary operation on two i64 operands using wrapping arithmetic.
    ///
    /// Signed operations use Rust's native i64 arithmetic.
    /// Unsigned operations (UDiv, URem, LShr) reinterpret the bits as u64.
    /// Returns None for division/remainder by zero.
    pub fn eval_i64(self, lhs: i64, rhs: i64) -> Option<i64> {
        Some(match self {
            IrBinOp::Add => lhs.wrapping_add(rhs),
            IrBinOp::Sub => lhs.wrapping_sub(rhs),
            IrBinOp::Mul => lhs.wrapping_mul(rhs),
            IrBinOp::And => lhs & rhs,
            IrBinOp::Or => lhs | rhs,
            IrBinOp::Xor => lhs ^ rhs,
            IrBinOp::Shl => lhs.wrapping_shl(rhs as u32),
            IrBinOp::AShr => lhs.wrapping_shr(rhs as u32),
            IrBinOp::LShr => (lhs as u64).wrapping_shr(rhs as u32) as i64,
            IrBinOp::SDiv => {
                if rhs == 0 {
                    return None;
                }
                lhs.wrapping_div(rhs)
            }
            IrBinOp::UDiv => {
                if rhs == 0 {
                    return None;
                }
                ((lhs as u64).wrapping_div(rhs as u64)) as i64
            }
            IrBinOp::SRem => {
                if rhs == 0 {
                    return None;
                }
                lhs.wrapping_rem(rhs)
            }
            IrBinOp::URem => {
                if rhs == 0 {
                    return None;
                }
                ((lhs as u64).wrapping_rem(rhs as u64)) as i64
            }
        })
    }

    /// Evaluate this binary operation on two i128 operands using wrapping arithmetic.
    ///
    /// Unsigned operations (UDiv, URem, LShr) reinterpret the bits as u128.
    /// Returns None for division/remainder by zero.
    pub fn eval_i128(self, lhs: i128, rhs: i128) -> Option<i128> {
        Some(match self {
            IrBinOp::Add => lhs.wrapping_add(rhs),
            IrBinOp::Sub => lhs.wrapping_sub(rhs),
            IrBinOp::Mul => lhs.wrapping_mul(rhs),
            IrBinOp::And => lhs & rhs,
            IrBinOp::Or => lhs | rhs,
            IrBinOp::Xor => lhs ^ rhs,
            IrBinOp::Shl => lhs.wrapping_shl(rhs as u32),
            IrBinOp::AShr => lhs.wrapping_shr(rhs as u32),
            IrBinOp::LShr => (lhs as u128).wrapping_shr(rhs as u32) as i128,
            IrBinOp::SDiv => {
                if rhs == 0 {
                    return None;
                }
                lhs.wrapping_div(rhs)
            }
            IrBinOp::UDiv => {
                if rhs == 0 {
                    return None;
                }
                (lhs as u128).wrapping_div(rhs as u128) as i128
            }
            IrBinOp::SRem => {
                if rhs == 0 {
                    return None;
                }
                lhs.wrapping_rem(rhs)
            }
            IrBinOp::URem => {
                if rhs == 0 {
                    return None;
                }
                (lhs as u128).wrapping_rem(rhs as u128) as i128
            }
        })
    }
}

/// Unary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IrUnaryOp {
    Neg,
    Not,
    Clz,
    Ctz,
    Bswap,
    Popcount,
    /// __builtin_constant_p: returns 1 if operand is a compile-time constant, 0 otherwise.
    /// Lowered as an IR instruction so it can be resolved after inlining and constant propagation.
    IsConstant,
}

/// Comparison operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IrCmpOp {
    Eq,
    Ne,
    Slt,
    Sle,
    Sgt,
    Sge,
    Ult,
    Ule,
    Ugt,
    Uge,
}

impl IrCmpOp {
    /// Evaluate this comparison on two i64 operands.
    ///
    /// Signed comparisons use Rust's native i64 ordering.
    /// Unsigned comparisons reinterpret the bits as u64.
    pub fn eval_i64(self, lhs: i64, rhs: i64) -> bool {
        match self {
            IrCmpOp::Eq => lhs == rhs,
            IrCmpOp::Ne => lhs != rhs,
            IrCmpOp::Slt => lhs < rhs,
            IrCmpOp::Sle => lhs <= rhs,
            IrCmpOp::Sgt => lhs > rhs,
            IrCmpOp::Sge => lhs >= rhs,
            IrCmpOp::Ult => (lhs as u64) < (rhs as u64),
            IrCmpOp::Ule => (lhs as u64) <= (rhs as u64),
            IrCmpOp::Ugt => (lhs as u64) > (rhs as u64),
            IrCmpOp::Uge => (lhs as u64) >= (rhs as u64),
        }
    }

    /// Evaluate this comparison on two i128 operands.
    ///
    /// Signed comparisons use Rust's native i128 ordering.
    /// Unsigned comparisons reinterpret the bits as u128.
    pub fn eval_i128(self, lhs: i128, rhs: i128) -> bool {
        match self {
            IrCmpOp::Eq => lhs == rhs,
            IrCmpOp::Ne => lhs != rhs,
            IrCmpOp::Slt => lhs < rhs,
            IrCmpOp::Sle => lhs <= rhs,
            IrCmpOp::Sgt => lhs > rhs,
            IrCmpOp::Sge => lhs >= rhs,
            IrCmpOp::Ult => (lhs as u128) < (rhs as u128),
            IrCmpOp::Ule => (lhs as u128) <= (rhs as u128),
            IrCmpOp::Ugt => (lhs as u128) > (rhs as u128),
            IrCmpOp::Uge => (lhs as u128) >= (rhs as u128),
        }
    }

    /// Evaluate this comparison on two f64 operands using IEEE 754 semantics.
    ///
    /// For floats, signed and unsigned comparison variants are equivalent since
    /// IEEE 754 defines a total ordering (NaN comparisons return false for
    /// ordered ops, true for Ne).
    pub fn eval_f64(self, lhs: f64, rhs: f64) -> bool {
        match self {
            IrCmpOp::Eq => lhs == rhs,
            IrCmpOp::Ne => lhs != rhs,
            IrCmpOp::Slt | IrCmpOp::Ult => lhs < rhs,
            IrCmpOp::Sle | IrCmpOp::Ule => lhs <= rhs,
            IrCmpOp::Sgt | IrCmpOp::Ugt => lhs > rhs,
            IrCmpOp::Sge | IrCmpOp::Uge => lhs >= rhs,
        }
    }
}
