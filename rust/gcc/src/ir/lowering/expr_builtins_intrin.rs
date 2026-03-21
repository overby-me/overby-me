//! Integer bit-manipulation intrinsics for the lowerer.
//!
//! Handles __builtin_clz, __builtin_ctz, __builtin_ffs, __builtin_clrsb,
//! __builtin_bswap, __builtin_popcount, and __builtin_parity (plus their
//! l/ll suffix variants).

use super::lower::Lowerer;
use crate::common::types::{target_int_ir_type, IrType};
use crate::frontend::parser::ast::Expr;
use crate::ir::reexports::{Instruction, IrBinOp, IrCmpOp, IrConst, IrUnaryOp, Operand};

impl Lowerer {
    /// Determine the operand width for a suffix-encoded intrinsic (clz, ctz, popcount, etc.).
    /// `ll` suffix = long long = always I64.
    /// `l` suffix = long = I64 on LP64, I32 on ILP32.
    /// No suffix = int = I32.
    pub(super) fn intrinsic_type_from_suffix(name: &str) -> IrType {
        if name.ends_with("ll") {
            IrType::I64
        } else if name.ends_with('l') {
            target_int_ir_type() // long: I64 on LP64, I32 on ILP32
        } else {
            IrType::I32
        }
    }

    /// Lower a simple unary intrinsic (CLZ, CTZ, Popcount) that takes one integer arg.
    pub(super) fn lower_unary_intrinsic(
        &mut self,
        name: &str,
        args: &[Expr],
        ir_op: IrUnaryOp,
    ) -> Option<Operand> {
        if args.is_empty() {
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let ty = Self::intrinsic_type_from_suffix(name);
        // Cast the argument to the intrinsic's operand width (e.g. zero-extend
        // a 32-bit size_t to 64-bit for __builtin_clzll on i686).
        let arg = self.lower_expr_with_type(&args[0], ty);
        let dest = self.fresh_value();
        self.emit(Instruction::UnaryOp {
            dest,
            op: ir_op,
            src: arg,
            ty,
        });
        Some(Operand::Value(dest))
    }

    /// Lower __builtin_ffs/__builtin_ffsl/__builtin_ffsll.
    /// ffs(x) returns 0 if x == 0, otherwise (ctz(x) + 1).
    /// Synthesized as: select(x == 0, 0, ctz(x) + 1)
    pub(super) fn lower_ffs_intrinsic(&mut self, name: &str, args: &[Expr]) -> Option<Operand> {
        if args.is_empty() {
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let ty = Self::intrinsic_type_from_suffix(name);
        // Cast arg to the intrinsic's operand width (e.g. zero-extend for "ll" on i686).
        let arg = self.lower_expr_with_type(&args[0], ty);

        // ctz_val = ctz(x)
        let ctz_val = self.fresh_value();
        self.emit(Instruction::UnaryOp {
            dest: ctz_val,
            op: IrUnaryOp::Ctz,
            src: arg,
            ty,
        });

        // ctz_plus_1 = ctz_val + 1
        let ctz_plus_1 = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: ctz_plus_1,
            op: IrBinOp::Add,
            lhs: Operand::Value(ctz_val),
            rhs: Operand::Const(if ty == IrType::I64 {
                IrConst::I64(1)
            } else {
                IrConst::I32(1)
            }),
            ty,
        });

        // is_zero = (x == 0)
        let is_zero = self.fresh_value();
        self.emit(Instruction::Cmp {
            dest: is_zero,
            op: IrCmpOp::Eq,
            lhs: arg,
            rhs: Operand::Const(if ty == IrType::I64 {
                IrConst::I64(0)
            } else {
                IrConst::I32(0)
            }),
            ty,
        });

        // result = select(is_zero, 0, ctz_plus_1)
        let result = self.fresh_value();
        self.emit(Instruction::Select {
            dest: result,
            cond: Operand::Value(is_zero),
            true_val: Operand::Const(if ty == IrType::I64 {
                IrConst::I64(0)
            } else {
                IrConst::I32(0)
            }),
            false_val: Operand::Value(ctz_plus_1),
            ty,
        });

        Some(Operand::Value(result))
    }

    /// Lower __builtin_bswap{16,32,64} - type determined by numeric suffix.
    pub(super) fn lower_bswap_intrinsic(&mut self, name: &str, args: &[Expr]) -> Option<Operand> {
        if args.is_empty() {
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let ty = if name.contains("64") {
            IrType::I64
        } else if name.contains("16") {
            IrType::I16
        } else {
            IrType::I32
        };
        // Cast arg to the intrinsic's operand width.
        let arg = self.lower_expr_with_type(&args[0], ty);
        let dest = self.fresh_value();
        self.emit(Instruction::UnaryOp {
            dest,
            op: IrUnaryOp::Bswap,
            src: arg,
            ty,
        });
        // bswap16 returns uint16_t which is promoted to int in C.
        // Zero-extend the 16-bit result to 32-bit to match the C return type
        // and prevent sign-extension of values with the high bit set (e.g. 0xBBAA).
        if ty == IrType::I16 {
            let zext = self.emit_cast_val(Operand::Value(dest), IrType::U16, IrType::I32);
            return Some(Operand::Value(zext));
        }
        Some(Operand::Value(dest))
    }

    /// Lower __builtin_parity{,l,ll} - implemented as popcount & 1.
    pub(super) fn lower_parity_intrinsic(&mut self, name: &str, args: &[Expr]) -> Option<Operand> {
        if args.is_empty() {
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let ty = Self::intrinsic_type_from_suffix(name);
        // Cast arg to the intrinsic's operand width.
        let arg = self.lower_expr_with_type(&args[0], ty);
        let pop = self.fresh_value();
        self.emit(Instruction::UnaryOp {
            dest: pop,
            op: IrUnaryOp::Popcount,
            src: arg,
            ty,
        });
        let one = if ty == IrType::I64 {
            IrConst::I64(1)
        } else {
            IrConst::I32(1)
        };
        let dest = self.emit_binop_val(IrBinOp::And, Operand::Value(pop), Operand::Const(one), ty);
        Some(Operand::Value(dest))
    }

    /// Lower __builtin_clrsb{,l,ll}(x) - count leading redundant sign bits.
    /// Computes: clz(x ^ negate(x >>> (bits-1))) - 1
    /// Uses logical shift + negate to create sign mask (avoids AShr 32-bit backend issues).
    pub(super) fn lower_clrsb_intrinsic(&mut self, name: &str, args: &[Expr]) -> Option<Operand> {
        if args.is_empty() {
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let ty = Self::intrinsic_type_from_suffix(name);
        // Cast arg to the intrinsic's operand width.
        let arg = self.lower_expr_with_type(&args[0], ty);
        let bits = if ty == IrType::I64 { 63i64 } else { 31i64 };

        // Extract sign bit using logical shift right: sign_bit = x >>> (bits)
        // This gives 0 for positive, 1 for negative
        let sign_bit = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: sign_bit,
            op: IrBinOp::LShr,
            lhs: arg,
            rhs: Operand::Const(if ty == IrType::I64 {
                IrConst::I64(bits)
            } else {
                IrConst::I32(bits as i32)
            }),
            ty,
        });

        // Negate sign bit to get mask: 0 -> 0, 1 -> -1 (all ones)
        let sign_mask = self.fresh_value();
        self.emit(Instruction::UnaryOp {
            dest: sign_mask,
            op: IrUnaryOp::Neg,
            src: Operand::Value(sign_bit),
            ty,
        });

        // XOR x with sign_mask: positive unchanged, negative gets ~x
        let xored = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: xored,
            op: IrBinOp::Xor,
            lhs: arg,
            rhs: Operand::Value(sign_mask),
            ty,
        });

        // CLZ of the XOR result
        let clz_val = self.fresh_value();
        self.emit(Instruction::UnaryOp {
            dest: clz_val,
            op: IrUnaryOp::Clz,
            src: Operand::Value(xored),
            ty,
        });

        // Subtract 1 (the sign bit itself is not counted)
        let result = self.emit_binop_val(
            IrBinOp::Sub,
            Operand::Value(clz_val),
            Operand::Const(IrConst::I32(1)),
            IrType::I32,
        );

        Some(Operand::Value(result))
    }
}
