//! Expression operator lowering: binary ops, unary ops, conditional/ternary,
//! short-circuit logical operators, and increment/decrement.
//!
//! Extracted from expr.rs to keep expression lowering manageable.

use super::lower::Lowerer;
use crate::common::types::{widened_op_type, AddressSpace, CType, IrType};
use crate::frontend::parser::ast::{BinOp, Expr, PostfixOp, UnaryOp};
use crate::ir::reexports::{
    Instruction, IrBinOp, IrCmpOp, IrConst, IrUnaryOp, Operand, Terminator, Value,
};

impl Lowerer {
    // -----------------------------------------------------------------------
    // Binary operations
    // -----------------------------------------------------------------------

    pub(super) fn lower_binary_op(&mut self, op: &BinOp, lhs: &Expr, rhs: &Expr) -> Operand {
        match op {
            BinOp::LogicalAnd => return self.lower_short_circuit(lhs, rhs, true),
            BinOp::LogicalOr => return self.lower_short_circuit(lhs, rhs, false),
            _ => {}
        }

        // Vector arithmetic (element-wise) -- check before constant folding since
        // vectors can't be constant-folded and get_expr_type doesn't handle them.
        // Per GCC vector extensions, if either operand is a vector, the operation
        // is element-wise. A scalar operand is splatted (broadcast) to all lanes.
        if matches!(
            op,
            BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::Div
                | BinOp::Mod
                | BinOp::BitAnd
                | BinOp::BitOr
                | BinOp::BitXor
                | BinOp::Shl
                | BinOp::Shr
        ) {
            let lhs_ct = self.expr_ctype(lhs);
            if lhs_ct.is_vector() {
                return self.lower_vector_binary_op(op, lhs, rhs, &lhs_ct);
            }
            let rhs_ct = self.expr_ctype(rhs);
            if rhs_ct.is_vector() {
                return self.lower_vector_binary_op(op, lhs, rhs, &rhs_ct);
            }
        }

        // Fast path: constant-fold pure integer arithmetic at lowering time.
        // This ensures correct C type semantics (e.g., 32-bit int width for
        // expressions like (1 << 31) / N) which the IR-level fold pass may lose
        // since it operates on 64-bit values without C type information.
        // Only apply to integer-only operations (shifts, bitwise, int arithmetic).
        // Skip float-involving expressions since eval_const_binop_float doesn't
        // correctly handle mixed int/float type promotion (e.g., int - float).
        if matches!(
            op,
            BinOp::Shl
                | BinOp::Shr
                | BinOp::BitAnd
                | BinOp::BitOr
                | BinOp::BitXor
                | BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::Div
                | BinOp::Mod
        ) {
            let lhs_ty = self.get_expr_type(lhs);
            let rhs_ty = self.get_expr_type(rhs);
            if !lhs_ty.is_float() && !rhs_ty.is_float() {
                if let Some(val) = self.eval_const_expr_from_parts(op, lhs, rhs) {
                    return Operand::Const(val);
                }
            }
        }

        // Complex arithmetic
        if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div) {
            let lhs_ct = self.expr_ctype(lhs);
            let rhs_ct = self.expr_ctype(rhs);
            if lhs_ct.is_complex() || rhs_ct.is_complex() {
                return self.lower_complex_binary_op(op, lhs, rhs, &lhs_ct, &rhs_ct);
            }
        }

        // Pointer arithmetic
        if matches!(op, BinOp::Add | BinOp::Sub) {
            if let Some(result) = self.try_lower_pointer_arithmetic(op, lhs, rhs) {
                return result;
            }
        }

        // Complex equality/inequality
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            let lhs_ct = self.expr_ctype(lhs);
            let rhs_ct = self.expr_ctype(rhs);
            if lhs_ct.is_complex() || rhs_ct.is_complex() {
                return self.lower_complex_comparison(op, lhs, rhs, &lhs_ct, &rhs_ct);
            }
        }

        // Pointer comparison
        if matches!(
            op,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        ) && (self.expr_is_pointer(lhs) || self.expr_is_pointer(rhs))
        {
            let lhs_val = self.lower_expr(lhs);
            let rhs_val = self.lower_expr(rhs);
            let cmp_op = Self::binop_to_cmp(*op, true);
            let ptr_ty = crate::common::types::target_int_ir_type();
            let dest = self.emit_cmp_val(cmp_op, lhs_val, rhs_val, ptr_ty);
            return Operand::Value(dest);
        }

        self.lower_arithmetic_binop(op, lhs, rhs)
    }

    fn lower_complex_binary_op(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
        lhs_ct: &CType,
        rhs_ct: &CType,
    ) -> Operand {
        let result_ct = self.common_complex_type(lhs_ct, rhs_ct);

        // Special case: real - complex uses negation for imag part to preserve -0.0
        if *op == BinOp::Sub && !lhs_ct.is_complex() && rhs_ct.is_complex() {
            let lhs_val = self.lower_expr(lhs);
            let rhs_val = self.lower_expr(rhs);
            let rhs_complex = self.convert_to_complex(rhs_val, rhs_ct, &result_ct);
            let rhs_ptr = self.operand_to_value(rhs_complex);
            return self.lower_real_minus_complex(lhs_val, lhs_ct, rhs_ptr, &result_ct);
        }

        let lhs_val = self.lower_expr(lhs);
        let rhs_val = self.lower_expr(rhs);
        let lhs_complex = self.convert_to_complex(lhs_val, lhs_ct, &result_ct);
        let rhs_complex = self.convert_to_complex(rhs_val, rhs_ct, &result_ct);
        let lhs_ptr = self.operand_to_value(lhs_complex);
        let rhs_ptr = self.operand_to_value(rhs_complex);

        match op {
            BinOp::Add => self.lower_complex_add(lhs_ptr, rhs_ptr, &result_ct),
            BinOp::Sub => self.lower_complex_sub(lhs_ptr, rhs_ptr, &result_ct),
            BinOp::Mul => self.lower_complex_mul(lhs_ptr, rhs_ptr, &result_ct),
            BinOp::Div => self.lower_complex_div(lhs_ptr, rhs_ptr, &result_ct),
            _ => unreachable!("unsupported complex binary op: {:?}", op),
        }
    }

    fn try_lower_pointer_arithmetic(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
    ) -> Option<Operand> {
        let lhs_is_ptr = self.expr_is_pointer(lhs);
        let rhs_is_ptr = self.expr_is_pointer(rhs);
        // Use target-appropriate pointer-width type: I32 on ILP32, I64 on LP64
        let ptr_int_ty = crate::common::types::target_int_ir_type();

        if lhs_is_ptr && !rhs_is_ptr {
            let elem_size = self.get_pointer_elem_size_from_expr(lhs);
            let rhs_ty = self.get_expr_type(rhs);
            let lhs_val = self.lower_expr(lhs);
            let rhs_val = self.lower_expr(rhs);
            // Widen the integer index to pointer width (sign-extend for signed types,
            // zero-extend for unsigned) before scaling and adding to the pointer.
            let rhs_val = self.emit_implicit_cast(rhs_val, rhs_ty, ptr_int_ty);
            let scaled_rhs = self.scale_index(rhs_val, elem_size);
            let ir_op = if *op == BinOp::Add {
                IrBinOp::Add
            } else {
                IrBinOp::Sub
            };
            let dest = self.emit_binop_val(ir_op, lhs_val, scaled_rhs, ptr_int_ty);
            Some(Operand::Value(dest))
        } else if rhs_is_ptr && !lhs_is_ptr && *op == BinOp::Add {
            let elem_size = self.get_pointer_elem_size_from_expr(rhs);
            let lhs_ty = self.get_expr_type(lhs);
            let lhs_val = self.lower_expr(lhs);
            // Widen the integer index to pointer width before scaling and adding to the pointer.
            let lhs_val = self.emit_implicit_cast(lhs_val, lhs_ty, ptr_int_ty);
            let rhs_val = self.lower_expr(rhs);
            let scaled_lhs = self.scale_index(lhs_val, elem_size);
            let dest = self.emit_binop_val(IrBinOp::Add, scaled_lhs, rhs_val, ptr_int_ty);
            Some(Operand::Value(dest))
        } else if lhs_is_ptr && rhs_is_ptr && *op == BinOp::Sub {
            let elem_size = self.get_pointer_elem_size_from_expr(lhs);
            let lhs_val = self.lower_expr(lhs);
            let rhs_val = self.lower_expr(rhs);
            let diff = self.emit_binop_val(IrBinOp::Sub, lhs_val, rhs_val, ptr_int_ty);
            if elem_size > 1 {
                let scale = Operand::Const(IrConst::ptr_int(elem_size as i64));
                let dest =
                    self.emit_binop_val(IrBinOp::SDiv, Operand::Value(diff), scale, ptr_int_ty);
                Some(Operand::Value(dest))
            } else {
                Some(Operand::Value(diff))
            }
        } else {
            None
        }
    }

    /// Multiply an index value by a scale factor (for pointer arithmetic).
    pub(super) fn scale_index(&mut self, index: Operand, scale: usize) -> Operand {
        if scale <= 1 {
            return index;
        }
        let ptr_int_ty = crate::common::types::target_int_ir_type();
        let scaled = self.emit_binop_val(
            IrBinOp::Mul,
            index,
            Operand::Const(IrConst::ptr_int(scale as i64)),
            ptr_int_ty,
        );
        Operand::Value(scaled)
    }

    fn lower_arithmetic_binop(&mut self, op: &BinOp, lhs: &Expr, rhs: &Expr) -> Operand {
        let lhs_ty = self.infer_expr_type(lhs);
        let rhs_ty = self.infer_expr_type(rhs);
        let lhs_expr_ty = self.get_expr_type(lhs);
        let rhs_expr_ty = self.get_expr_type(rhs);

        let is_shift = matches!(op, BinOp::Shl | BinOp::Shr);

        let (op_ty, is_unsigned, common_ty) = if lhs_expr_ty.is_float() || rhs_expr_ty.is_float() {
            let ft = if lhs_expr_ty == IrType::F128 || rhs_expr_ty == IrType::F128 {
                IrType::F128
            } else if lhs_expr_ty == IrType::F64 || rhs_expr_ty == IrType::F64 {
                IrType::F64
            } else {
                IrType::F32
            };
            (ft, false, ft)
        } else if is_shift {
            let promoted_lhs = Self::integer_promote(lhs_ty);
            let shift_op_ty = widened_op_type(promoted_lhs);
            (shift_op_ty, promoted_lhs.is_unsigned(), promoted_lhs)
        } else {
            let ct = Self::common_type(lhs_ty, rhs_ty);
            let ot = widened_op_type(ct);
            (ot, ct.is_unsigned(), ct)
        };

        let (lhs_val, rhs_val) = if is_shift {
            let shift_lhs_ty = widened_op_type(op_ty);
            let lhs_val = self.lower_expr_with_type(lhs, shift_lhs_ty);
            // Shift amount is always small; use widened type for consistency
            let shift_rhs_ty = widened_op_type(IrType::I32);
            let rhs_val = self.lower_expr_with_type(rhs, shift_rhs_ty);
            (lhs_val, rhs_val)
        } else {
            let mut lhs_val = self.lower_expr_with_type(lhs, op_ty);
            let mut rhs_val = self.lower_expr_with_type(rhs, op_ty);
            if common_ty == IrType::U32 && !crate::common::types::target_is_32bit() {
                // On 64-bit: zero-extend both operands to ensure correct 64-bit unsigned
                // semantics. All operand types (signed or unsigned, any width <= 32) need
                // truncation to U32 so that 64-bit operations (like divq) see the correct
                // 32-bit unsigned values.
                // On 32-bit: not needed since U32 operations are native width.
                if lhs_ty.size() <= 4 {
                    let masked = self.emit_cast_val(lhs_val, IrType::I64, IrType::U32);
                    lhs_val = Operand::Value(masked);
                }
                if rhs_ty.size() <= 4 {
                    let masked = self.emit_cast_val(rhs_val, IrType::I64, IrType::U32);
                    rhs_val = Operand::Value(masked);
                }
            }
            (lhs_val, rhs_val)
        };
        let dest = self.fresh_value();

        match op {
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let cmp_op = Self::binop_to_cmp(*op, is_unsigned);
                self.emit(Instruction::Cmp {
                    dest,
                    op: cmp_op,
                    lhs: lhs_val,
                    rhs: rhs_val,
                    ty: common_ty,
                });
            }
            _ => {
                let ir_op = Self::binop_to_ir(*op, is_unsigned);
                self.emit(Instruction::BinOp {
                    dest,
                    op: ir_op,
                    lhs: lhs_val,
                    rhs: rhs_val,
                    ty: op_ty,
                });
            }
        }

        self.maybe_narrow_binop_result(dest, op, common_ty)
    }

    /// Integer promotion: types narrower than int are promoted to int.
    pub(super) fn integer_promote(ty: IrType) -> IrType {
        match ty {
            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 => IrType::I32,
            _ => ty,
        }
    }

    /// Convert a comparison BinOp to the corresponding IrCmpOp.
    pub(super) fn binop_to_cmp(op: BinOp, is_unsigned: bool) -> IrCmpOp {
        match (op, is_unsigned) {
            (BinOp::Eq, _) => IrCmpOp::Eq,
            (BinOp::Ne, _) => IrCmpOp::Ne,
            (BinOp::Lt, false) => IrCmpOp::Slt,
            (BinOp::Lt, true) => IrCmpOp::Ult,
            (BinOp::Le, false) => IrCmpOp::Sle,
            (BinOp::Le, true) => IrCmpOp::Ule,
            (BinOp::Gt, false) => IrCmpOp::Sgt,
            (BinOp::Gt, true) => IrCmpOp::Ugt,
            (BinOp::Ge, false) => IrCmpOp::Sge,
            (BinOp::Ge, true) => IrCmpOp::Uge,
            _ => unreachable!("non-comparison BinOp in binop_to_cmp: {:?}", op),
        }
    }

    /// Convert an arithmetic/bitwise BinOp to the corresponding IrBinOp.
    pub(super) fn binop_to_ir(op: BinOp, is_unsigned: bool) -> IrBinOp {
        match (op, is_unsigned) {
            (BinOp::Add, _) => IrBinOp::Add,
            (BinOp::Sub, _) => IrBinOp::Sub,
            (BinOp::Mul, _) => IrBinOp::Mul,
            (BinOp::Div, false) => IrBinOp::SDiv,
            (BinOp::Div, true) => IrBinOp::UDiv,
            (BinOp::Mod, false) => IrBinOp::SRem,
            (BinOp::Mod, true) => IrBinOp::URem,
            (BinOp::BitAnd, _) => IrBinOp::And,
            (BinOp::BitOr, _) => IrBinOp::Or,
            (BinOp::BitXor, _) => IrBinOp::Xor,
            (BinOp::Shl, _) => IrBinOp::Shl,
            (BinOp::Shr, false) => IrBinOp::AShr,
            (BinOp::Shr, true) => IrBinOp::LShr,
            _ => unreachable!("non-arithmetic BinOp in binop_to_ir: {:?}", op),
        }
    }

    fn maybe_narrow_binop_result(&mut self, dest: Value, op: &BinOp, common_ty: IrType) -> Operand {
        // Only narrow integer results; float operations use their exact type.
        if common_ty.is_float() || common_ty.is_128bit() || op.is_comparison() {
            return Operand::Value(dest);
        }
        let wt = widened_op_type(common_ty);
        if common_ty != wt {
            let narrowed = self.emit_cast_val(Operand::Value(dest), wt, common_ty);
            Operand::Value(narrowed)
        } else {
            Operand::Value(dest)
        }
    }

    // -----------------------------------------------------------------------
    // Unary operations
    // -----------------------------------------------------------------------

    pub(super) fn lower_unary_op(&mut self, op: UnaryOp, inner: &Expr) -> Operand {
        match op {
            UnaryOp::Plus => {
                let inner_ct = self.expr_ctype(inner);
                if inner_ct.is_complex() {
                    return self.lower_expr(inner);
                }
                let val = self.lower_expr(inner);
                let inner_ty = self.infer_expr_type(inner);
                let promoted_ty = Self::integer_promote(inner_ty);
                if promoted_ty != inner_ty && inner_ty.is_integer() {
                    let dest = self.emit_cast_val(val, inner_ty, promoted_ty);
                    Operand::Value(dest)
                } else {
                    val
                }
            }
            UnaryOp::Neg => {
                let inner_ct = self.expr_ctype(inner);
                if inner_ct.is_complex() {
                    let val = self.lower_expr(inner);
                    let ptr = self.operand_to_value(val);
                    return self.lower_complex_neg(ptr, &inner_ct);
                }
                let ty = self.get_expr_type(inner);
                let inner_ty = self.infer_expr_type(inner);
                let neg_ty = if ty.is_float() || ty.is_128bit() {
                    ty
                } else {
                    widened_op_type(ty)
                };
                let val = self.lower_expr(inner);
                // C integer promotion (C11 6.3.1.1): sign-extend (or zero-extend
                // for unsigned) sub-int types to the operation width before negating.
                // Without this, signed char -13 (0xF3) is zero-extended to 243
                // instead of sign-extended to -13, producing wrong results.
                let val = if !neg_ty.is_float() && inner_ty.size() < neg_ty.size() {
                    Operand::Value(self.emit_cast_val(val, inner_ty, neg_ty))
                } else {
                    val
                };
                let dest = self.fresh_value();
                self.emit(Instruction::UnaryOp {
                    dest,
                    op: IrUnaryOp::Neg,
                    src: val,
                    ty: neg_ty,
                });
                if !neg_ty.is_float() {
                    let promoted_ty = Self::integer_promote(inner_ty);
                    self.maybe_narrow(dest, promoted_ty)
                } else {
                    Operand::Value(dest)
                }
            }
            UnaryOp::BitNot => {
                let inner_ct = self.expr_ctype(inner);
                if inner_ct.is_complex() {
                    return self.lower_complex_conj(inner);
                }
                let inner_ty = self.infer_expr_type(inner);
                let ty = self.get_expr_type(inner);
                let not_ty = if ty.is_128bit() {
                    ty
                } else {
                    widened_op_type(ty)
                };
                let val = self.lower_expr(inner);
                // C integer promotion: widen sub-int types before bitwise NOT,
                // same as for Neg above. E.g. ~(signed char -13) must sign-extend
                // -13 to int before complementing, giving 12 (not -244).
                let val = if inner_ty.size() < not_ty.size() {
                    Operand::Value(self.emit_cast_val(val, inner_ty, not_ty))
                } else {
                    val
                };
                let dest = self.fresh_value();
                self.emit(Instruction::UnaryOp {
                    dest,
                    op: IrUnaryOp::Not,
                    src: val,
                    ty: not_ty,
                });
                let promoted_ty = Self::integer_promote(inner_ty);
                self.maybe_narrow(dest, promoted_ty)
            }
            UnaryOp::LogicalNot => {
                let int_ty = crate::common::types::target_int_ir_type();
                let zero = if int_ty == IrType::I32 {
                    IrConst::I32(0)
                } else {
                    IrConst::I64(0)
                };
                let inner_ct = self.expr_ctype(inner);
                if inner_ct.is_complex() {
                    // !complex_val => (real == 0) && (imag == 0)
                    let val = self.lower_expr(inner);
                    let ptr = self.operand_to_value(val);
                    let bool_val = self.lower_complex_to_bool(ptr, &inner_ct);
                    // Negate: bool_val is 1 if nonzero, so !complex is (bool_val == 0)
                    let dest =
                        self.emit_cmp_val(IrCmpOp::Eq, bool_val, Operand::Const(zero), int_ty);
                    Operand::Value(dest)
                } else {
                    let inner_ty = self.infer_expr_type(inner);
                    let val = self.lower_expr(inner);
                    let cmp_val = self.mask_float_sign_for_truthiness(val, inner_ty);
                    // Use the inner expression's type for comparison when it is
                    // a wide integer (I64/U64) on a 32-bit target. Otherwise a
                    // 64-bit value like 0x1_0000_0000 would be truncated to 32
                    // bits and incorrectly compare equal to zero. Float types
                    // are excluded because mask_float_sign_for_truthiness already
                    // reduces them to an I32 boolean on 32-bit targets.
                    let cmp_ty = if !inner_ty.is_float() && inner_ty.size() > int_ty.size() {
                        inner_ty
                    } else {
                        int_ty
                    };
                    let cmp_zero = match cmp_ty {
                        IrType::I64 | IrType::U64 => IrConst::I64(0),
                        _ => {
                            if int_ty == IrType::I32 {
                                IrConst::I32(0)
                            } else {
                                IrConst::I64(0)
                            }
                        }
                    };
                    let dest =
                        self.emit_cmp_val(IrCmpOp::Eq, cmp_val, Operand::Const(cmp_zero), cmp_ty);
                    Operand::Value(dest)
                }
            }
            UnaryOp::PreInc | UnaryOp::PreDec => self.lower_pre_inc_dec(inner, op),
            UnaryOp::RealPart => self.lower_complex_real_part(inner),
            UnaryOp::ImagPart => self.lower_complex_imag_part(inner),
        }
    }

    // -----------------------------------------------------------------------
    // Conditional (ternary) operator
    // -----------------------------------------------------------------------

    pub(super) fn lower_conditional(
        &mut self,
        cond: &Expr,
        then_expr: &Expr,
        else_expr: &Expr,
    ) -> Operand {
        // Complex types are represented as pointers to stack-allocated {real, imag}
        // pairs. Both lower_expr() for complex variables and lower_function_call()
        // for complex-returning functions produce Ptr operands. However,
        // get_expr_type() returns the function signature's IR return type (e.g., F64
        // for packed _Complex float on x86-64), creating a mismatch. Force Ptr as
        // the common type when either branch is complex to avoid corrupting pointer
        // values with float casts.
        let then_ct = self.expr_ctype(then_expr);
        let else_ct = self.expr_ctype(else_expr);
        let result_is_complex = then_ct.is_complex() || else_ct.is_complex();

        // Constant-fold the condition at lowering time. If the condition is a
        // compile-time constant (e.g., sizeof(x)==4, __builtin_constant_p(v)),
        // skip generating code for the dead branch entirely. This is critical
        // for kernel patterns like hweight_long() where __builtin_constant_p
        // and sizeof checks gate large macro expansions — without folding, the
        // dead branches bloat the IR beyond inlining limits and cause linker
        // errors when always_inline functions can't be inlined.
        if let Some(const_val) = self.eval_const_expr(cond) {
            let is_true = const_val.is_nonzero();
            if is_true {
                let then_val = self.lower_expr(then_expr);
                if result_is_complex {
                    // Convert to the common complex type if the branch types differ
                    let common_ct = self.common_complex_type(&then_ct, &else_ct);
                    return self.convert_to_complex(then_val, &then_ct, &common_ct);
                }
                let then_ty = if self.expr_is_pointer(then_expr) {
                    IrType::Ptr
                } else {
                    self.get_expr_type(then_expr)
                };
                let else_ty = if self.expr_is_pointer(else_expr) {
                    IrType::Ptr
                } else {
                    self.get_expr_type(else_expr)
                };
                let common_ty = Self::common_type(then_ty, else_ty);
                return self.emit_implicit_cast(then_val, then_ty, common_ty);
            } else {
                let else_val = self.lower_expr(else_expr);
                if result_is_complex {
                    // Convert to the common complex type if the branch types differ
                    let common_ct = self.common_complex_type(&then_ct, &else_ct);
                    return self.convert_to_complex(else_val, &else_ct, &common_ct);
                }
                let then_ty = if self.expr_is_pointer(then_expr) {
                    IrType::Ptr
                } else {
                    self.get_expr_type(then_expr)
                };
                let else_ty = if self.expr_is_pointer(else_expr) {
                    IrType::Ptr
                } else {
                    self.get_expr_type(else_expr)
                };
                let common_ty = Self::common_type(then_ty, else_ty);
                return self.emit_implicit_cast(else_val, else_ty, common_ty);
            }
        }

        let cond_val = self.lower_condition_expr(cond);

        // For complex types, both branches produce Ptr values; use Ptr as the
        // alloca/store/load type so we don't emit bogus int-to-float casts.
        // When the branch types differ, convert each to the common complex type.
        if result_is_complex {
            let common_ct = self.common_complex_type(&then_ct, &else_ct);
            return self.emit_ternary_branch(
                cond_val,
                IrType::Ptr,
                |s| {
                    let v = s.lower_expr(then_expr);
                    s.convert_to_complex(v, &then_ct, &common_ct)
                },
                |s| {
                    let v = s.lower_expr(else_expr);
                    s.convert_to_complex(v, &else_ct, &common_ct)
                },
            );
        }

        let mut then_ty = self.get_expr_type(then_expr);
        let mut else_ty = self.get_expr_type(else_expr);
        if self.expr_is_pointer(then_expr) {
            then_ty = IrType::Ptr;
        }
        if self.expr_is_pointer(else_expr) {
            else_ty = IrType::Ptr;
        }
        let common_ty = Self::common_type(then_ty, else_ty);

        // Detect struct-typed ternary where branches produce inconsistent
        // representations: compound literals and struct derefs produce alloca
        // addresses, while function calls returning small (≤8 byte) structs
        // produce packed data in registers.  When one branch produces packed
        // data and the other produces an address, normalize by loading packed
        // data from address-producing branches so both branches produce data
        // values.
        //
        // Only check for struct-typed expressions (both branches struct/union).
        // expr_produces_packed_struct_data() is not valid for non-struct types
        // (it can return true for non-struct function calls).
        let result_is_struct = then_ct.is_struct_or_union() || else_ct.is_struct_or_union();
        let (needs_struct_load, then_produces_packed) = if result_is_struct {
            let tp = self.expr_produces_packed_struct_data(then_expr);
            let ep = self.expr_produces_packed_struct_data(else_expr);
            (tp != ep, tp)
        } else {
            (false, false)
        };
        // Use target int type for loading packed small-struct data (≤8 bytes)
        let effective_ty = if needs_struct_load {
            crate::common::types::target_int_ir_type()
        } else {
            common_ty
        };

        self.emit_ternary_branch(
            cond_val,
            effective_ty,
            |s| {
                let then_val = s.lower_expr(then_expr);
                if needs_struct_load && !then_produces_packed {
                    // Address-producing branch (compound literal, deref, etc.);
                    // load the packed data to match the other packed-data branch
                    if let Operand::Value(ptr) = then_val {
                        let loaded = s.fresh_value();
                        s.emit(Instruction::Load {
                            dest: loaded,
                            ptr,
                            ty: effective_ty,
                            seg_override: AddressSpace::Default,
                        });
                        Operand::Value(loaded)
                    } else {
                        then_val
                    }
                } else {
                    s.emit_implicit_cast(then_val, then_ty, effective_ty)
                }
            },
            |s| {
                let else_val = s.lower_expr(else_expr);
                if needs_struct_load && then_produces_packed {
                    // Address-producing branch (compound literal, deref, etc.);
                    // load the packed data to match the other packed-data branch
                    if let Operand::Value(ptr) = else_val {
                        let loaded = s.fresh_value();
                        s.emit(Instruction::Load {
                            dest: loaded,
                            ptr,
                            ty: effective_ty,
                            seg_override: AddressSpace::Default,
                        });
                        Operand::Value(loaded)
                    } else {
                        else_val
                    }
                } else {
                    s.emit_implicit_cast(else_val, else_ty, effective_ty)
                }
            },
        )
    }

    /// Lower GNU conditional expression: `cond ? : else_expr`
    /// The "then" value is the condition value itself (evaluated once).
    pub(super) fn lower_gnu_conditional(&mut self, cond: &Expr, else_expr: &Expr) -> Operand {
        let cond_val = self.lower_expr(cond);
        let cond_ty = self.get_expr_type(cond);
        let else_ty = self.get_expr_type(else_expr);

        // Complex types always lower to Ptr; avoid type mismatch with function
        // call return types (e.g., F64 for packed _Complex float).
        let cond_ct = self.expr_ctype(cond);
        let else_ct = self.expr_ctype(else_expr);
        let result_is_complex = cond_ct.is_complex() || else_ct.is_complex();

        if result_is_complex {
            // Complex condition: convert to bool via (real != 0) || (imag != 0)
            let common_ct = self.common_complex_type(&cond_ct, &else_ct);
            let cond_ptr = self.operand_to_value(cond_val);
            let cond_bool_op = self.lower_complex_to_bool(cond_ptr, &cond_ct);
            let cond_bool_val = self.operand_to_value(cond_bool_op);
            return self.emit_ternary_branch(
                Operand::Value(cond_bool_val),
                IrType::Ptr,
                |s| s.convert_to_complex(cond_val, &cond_ct, &common_ct),
                |s| {
                    let v = s.lower_expr(else_expr);
                    s.convert_to_complex(v, &else_ct, &common_ct)
                },
            );
        }

        let common_ty = Self::common_type(cond_ty, else_ty);

        // Convert condition to boolean for branching
        let int_ty = crate::common::types::target_int_ir_type();
        let zero = if int_ty == IrType::I32 {
            IrConst::I32(0)
        } else {
            IrConst::I64(0)
        };
        let cond_bool = self.fresh_value();
        self.emit(Instruction::Cmp {
            dest: cond_bool,
            op: IrCmpOp::Ne,
            lhs: cond_val,
            rhs: Operand::Const(zero),
            ty: int_ty,
        });

        self.emit_ternary_branch(
            Operand::Value(cond_bool),
            common_ty,
            |s| s.emit_implicit_cast(cond_val, cond_ty, common_ty),
            |s| {
                let else_val = s.lower_expr(else_expr);
                s.emit_implicit_cast(else_val, else_ty, common_ty)
            },
        )
    }

    /// Shared helper for ternary branch patterns (conditional and GNU conditional).
    /// Evaluates `then_fn` in the true branch and `else_fn` in the false branch,
    /// stores both results to an alloca, and returns the loaded result.
    fn emit_ternary_branch(
        &mut self,
        cond: Operand,
        result_ty: IrType,
        then_fn: impl FnOnce(&mut Self) -> Operand,
        else_fn: impl FnOnce(&mut Self) -> Operand,
    ) -> Operand {
        // Use emit_entry_alloca so the alloca is in the entry block, ensuring
        // mem2reg can promote it to SSA/Phi form. Previously this used self.emit()
        // which placed the alloca in the current block — if the expression was inside
        // nested control flow (e.g., inside a loop or if body), the alloca would
        // be in a non-entry block and mem2reg would refuse to promote it.
        //
        // Use the actual result type for the alloca size so that wider types like
        // long double (F128, 16 bytes) and __int128 are stored correctly.
        let int_ty = crate::common::types::target_int_ir_type();
        let min_alloca_size = int_ty.size();
        let alloca_size = result_ty.size().max(min_alloca_size);
        let alloca_ty = if result_ty.size() > min_alloca_size {
            result_ty
        } else {
            int_ty
        };
        let result_alloca = self.emit_entry_alloca(alloca_ty, alloca_size, 0, false);

        let then_label = self.fresh_label();
        let else_label = self.fresh_label();
        let end_label = self.fresh_label();

        self.terminate(Terminator::CondBranch {
            cond,
            true_label: then_label,
            false_label: else_label,
        });

        self.start_block(then_label);
        let then_val = then_fn(self);
        self.emit(Instruction::Store {
            val: then_val,
            ptr: result_alloca,
            ty: alloca_ty,
            seg_override: AddressSpace::Default,
        });
        self.terminate(Terminator::Branch(end_label));

        self.start_block(else_label);
        let else_val = else_fn(self);
        self.emit(Instruction::Store {
            val: else_val,
            ptr: result_alloca,
            ty: alloca_ty,
            seg_override: AddressSpace::Default,
        });
        self.terminate(Terminator::Branch(end_label));

        self.start_block(end_label);
        let result = self.fresh_value();
        self.emit(Instruction::Load {
            dest: result,
            ptr: result_alloca,
            ty: alloca_ty,
            seg_override: AddressSpace::Default,
        });
        Operand::Value(result)
    }

    // -----------------------------------------------------------------------
    // Short-circuit logical operators
    // -----------------------------------------------------------------------

    fn lower_short_circuit(&mut self, lhs: &Expr, rhs: &Expr, is_and: bool) -> Operand {
        // Use target-appropriate int type: I32 on ILP32 (i686), I64 on LP64
        let int_ty = crate::common::types::target_int_ir_type();
        let int_size = if int_ty == IrType::I32 { 4 } else { 8 };
        let make_int_const = |v: i64| -> IrConst {
            if int_ty == IrType::I32 {
                IrConst::I32(v as i32)
            } else {
                IrConst::I64(v)
            }
        };

        // Constant-fold the LHS to eliminate dead code at lowering time.
        // This is critical for constructs like IS_ENABLED(CONFIG_X) && func()
        // where CONFIG_X is not set: without this, the compiler would emit a
        // reference to func() even though it's never called, causing link errors.
        if let Some(lhs_const) = self.eval_const_expr(lhs) {
            let lhs_is_true = lhs_const.is_nonzero();
            if is_and {
                if !lhs_is_true {
                    // 0 && rhs => always 0, skip RHS entirely
                    return Operand::Const(make_int_const(0));
                }
                // nonzero && rhs => result is bool(rhs)
                let rhs_val = self.lower_condition_expr(rhs);
                let rhs_bool = self.emit_cmp_val(
                    IrCmpOp::Ne,
                    rhs_val,
                    Operand::Const(make_int_const(0)),
                    int_ty,
                );
                return Operand::Value(rhs_bool);
            } else {
                if lhs_is_true {
                    // nonzero || rhs => always 1, skip RHS entirely
                    return Operand::Const(make_int_const(1));
                }
                // 0 || rhs => result is bool(rhs)
                let rhs_val = self.lower_condition_expr(rhs);
                let rhs_bool = self.emit_cmp_val(
                    IrCmpOp::Ne,
                    rhs_val,
                    Operand::Const(make_int_const(0)),
                    int_ty,
                );
                return Operand::Value(rhs_bool);
            }
        }

        // Check if the RHS is a compile-time constant. When it is, we can
        // simplify the short-circuit result without emitting the RHS branch.
        // This helps patterns like `expr && 0` or `expr || 1` where the RHS
        // is a literal or compile-time constant expression.
        if let Some(rhs_const) = self.eval_const_expr(rhs) {
            let rhs_is_true = rhs_const.is_nonzero();
            if is_and && !rhs_is_true {
                // LHS && false => always false. Evaluate LHS for side effects, return 0.
                let _ = self.lower_condition_expr(lhs);
                return Operand::Const(make_int_const(0));
            } else if !is_and && rhs_is_true {
                // LHS || true => always true. Evaluate LHS for side effects, return 1.
                let _ = self.lower_condition_expr(lhs);
                return Operand::Const(make_int_const(1));
            }
            // For "LHS && true" or "LHS || false", fall through to normal lowering
            // since the result depends on LHS.
        }

        // Use emit_entry_alloca so the alloca is in the entry block, ensuring
        // mem2reg can promote it to SSA/Phi form. Previously this used self.emit()
        // which placed the alloca in the current block — if the expression was inside
        // nested control flow (e.g., inside a loop or if body), the alloca would
        // be in a non-entry block and mem2reg would refuse to promote it.
        let result_alloca = self.emit_entry_alloca(int_ty, int_size, 0, false);

        let rhs_label = self.fresh_label();
        let end_label = self.fresh_label();

        let lhs_val = self.lower_condition_expr(lhs);

        let default_val = if is_and { 0 } else { 1 };
        self.emit(Instruction::Store {
            val: Operand::Const(make_int_const(default_val)),
            ptr: result_alloca,
            ty: int_ty,
            seg_override: AddressSpace::Default,
        });

        let (true_label, false_label) = if is_and {
            (rhs_label, end_label)
        } else {
            (end_label, rhs_label)
        };
        self.terminate(Terminator::CondBranch {
            cond: lhs_val,
            true_label,
            false_label,
        });

        self.start_block(rhs_label);
        let rhs_val = self.lower_condition_expr(rhs);
        let rhs_bool = self.emit_cmp_val(
            IrCmpOp::Ne,
            rhs_val,
            Operand::Const(make_int_const(0)),
            int_ty,
        );
        self.emit(Instruction::Store {
            val: Operand::Value(rhs_bool),
            ptr: result_alloca,
            ty: int_ty,
            seg_override: AddressSpace::Default,
        });
        self.terminate(Terminator::Branch(end_label));

        self.start_block(end_label);
        let result = self.fresh_value();
        self.emit(Instruction::Load {
            dest: result,
            ptr: result_alloca,
            ty: int_ty,
            seg_override: AddressSpace::Default,
        });
        Operand::Value(result)
    }

    // -----------------------------------------------------------------------
    // Increment/decrement operators
    // -----------------------------------------------------------------------

    pub(super) fn lower_pre_inc_dec(&mut self, inner: &Expr, op: UnaryOp) -> Operand {
        let is_inc = op == UnaryOp::PreInc;
        self.lower_inc_dec_impl(inner, is_inc, true)
    }

    pub(super) fn lower_post_inc_dec(&mut self, inner: &Expr, op: PostfixOp) -> Operand {
        let is_inc = op == PostfixOp::PostInc;
        self.lower_inc_dec_impl(inner, is_inc, false)
    }

    fn lower_inc_dec_impl(&mut self, inner: &Expr, is_inc: bool, return_new: bool) -> Operand {
        if let Some(result) = self.try_lower_bitfield_inc_dec(inner, is_inc, return_new) {
            return result;
        }

        let ty = self.get_expr_type(inner);
        if let Some(lv) = self.lower_lvalue(inner) {
            let loaded = self.load_lvalue_typed(&lv, ty);
            let loaded_val = self.operand_to_value(loaded);
            let (step, binop_ty) = self.inc_dec_step_and_type(ty, inner);
            let ir_op = if is_inc { IrBinOp::Add } else { IrBinOp::Sub };
            let result = self.emit_binop_val(ir_op, Operand::Value(loaded_val), step, binop_ty);
            let store_op = if self.is_bool_lvalue(inner) {
                self.emit_bool_normalize_typed(Operand::Value(result), binop_ty)
            } else if ty != binop_ty && ty.is_integer() && binop_ty.is_integer() {
                // When the add/sub was done in a wider type than the variable's type
                // (e.g. U32 variable incremented via I64 add), truncate back to the
                // variable's type so wrapping semantics are correct. Without this,
                // 0xFFFFFFFF + 1 would be 0x100000000 instead of wrapping to 0.
                self.emit_implicit_cast(Operand::Value(result), binop_ty, ty)
            } else {
                Operand::Value(result)
            };
            self.store_lvalue_typed(&lv, store_op, ty);
            return if return_new { store_op } else { loaded };
        }
        self.lower_expr(inner)
    }

    fn try_lower_bitfield_inc_dec(
        &mut self,
        inner: &Expr,
        is_inc: bool,
        return_new: bool,
    ) -> Option<Operand> {
        let (field_addr, storage_ty, bit_offset, bit_width) =
            self.resolve_bitfield_lvalue(inner)?;

        let current_val =
            self.extract_bitfield_from_addr(field_addr, storage_ty, bit_offset, bit_width);

        let ir_op = if is_inc { IrBinOp::Add } else { IrBinOp::Sub };
        let wt = widened_op_type(IrType::I32);
        let one = if wt == IrType::I32 {
            IrConst::I32(1)
        } else {
            IrConst::I64(1)
        };
        let result = self.emit_binop_val(ir_op, current_val, Operand::Const(one), wt);

        self.store_bitfield(
            field_addr,
            storage_ty,
            bit_offset,
            bit_width,
            Operand::Value(result),
        );

        let ret_val = if return_new {
            Operand::Value(result)
        } else {
            current_val
        };
        Some(self.truncate_to_bitfield_value(ret_val, bit_width, storage_ty.is_signed()))
    }

    fn inc_dec_step_and_type(&self, ty: IrType, expr: &Expr) -> (Operand, IrType) {
        if ty == IrType::Ptr {
            let ptr_int_ty = crate::common::types::target_int_ir_type();
            let elem_size = self.get_pointer_elem_size_from_expr(expr);
            (
                Operand::Const(IrConst::ptr_int(elem_size as i64)),
                ptr_int_ty,
            )
        } else if ty == IrType::F64 {
            (Operand::Const(IrConst::F64(1.0)), IrType::F64)
        } else if ty == IrType::F32 {
            (Operand::Const(IrConst::F32(1.0)), IrType::F32)
        } else if ty == IrType::F128 {
            (Operand::Const(IrConst::long_double(1.0)), IrType::F128)
        } else if ty == IrType::I128 || ty == IrType::U128 {
            (Operand::Const(IrConst::I128(1)), ty)
        } else {
            // For regular integer types, use the widened operation type for arithmetic.
            // On 64-bit, this is I64; on 32-bit, this is I32 (or I64 for I64/U64).
            // The caller truncates back to the variable's type after the operation.
            let wt = widened_op_type(ty);
            let one = if wt == IrType::I32 {
                IrConst::I32(1)
            } else {
                IrConst::I64(1)
            };
            (Operand::Const(one), wt)
        }
    }
}
