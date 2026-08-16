//! ArmCodegen: comparison operations.

use super::emit::{arm_int_cond_code, arm_invert_cond_code, ArmCodegen};
use crate::common::types::IrType;
use crate::ir::reexports::{IrCmpOp, Operand, Value};

impl ArmCodegen {
    pub(super) fn emit_float_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        self.operand_to_x0(lhs);
        self.state.emit("    mov x1, x0");
        self.operand_to_x0(rhs);
        if ty == IrType::F32 {
            self.state.emit("    fmov s0, w1");
            self.state.emit("    fmov s1, w0");
            self.state.emit("    fcmp s0, s1");
        } else {
            self.state.emit("    fmov d0, x1");
            self.state.emit("    fmov d1, x0");
            self.state.emit("    fcmp d0, d1");
        }
        let cond = match op {
            IrCmpOp::Eq => "eq",
            IrCmpOp::Ne => "ne",
            IrCmpOp::Slt | IrCmpOp::Ult => "mi",
            IrCmpOp::Sle | IrCmpOp::Ule => "ls",
            IrCmpOp::Sgt | IrCmpOp::Ugt => "gt",
            IrCmpOp::Sge | IrCmpOp::Uge => "ge",
        };
        self.state.emit_fmt(format_args!("    cset x0, {}", cond));
        self.store_x0_to(dest);
    }

    pub(super) fn emit_f128_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        crate::backend::f128_softfloat::f128_cmp(self, dest, op, lhs, rhs);
    }

    pub(super) fn emit_int_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        self.emit_int_cmp_insn(lhs, rhs, ty);
        let cond = arm_int_cond_code(op);
        self.state.emit_fmt(format_args!("    cset x0, {}", cond));
        self.store_x0_to(dest);
    }

    pub(super) fn emit_fused_cmp_branch_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
        true_label: &str,
        false_label: &str,
    ) {
        self.emit_int_cmp_insn(lhs, rhs, ty);
        let cc = arm_int_cond_code(op);
        let inv_cc = arm_invert_cond_code(cc);
        let skip = self.state.fresh_label("skip");
        self.state
            .emit_fmt(format_args!("    b.{} {}", inv_cc, skip));
        self.state.emit_fmt(format_args!("    b {}", true_label));
        self.state.emit_fmt(format_args!("{}:", skip));
        self.state.emit_fmt(format_args!("    b {}", false_label));
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_select_impl(
        &mut self,
        dest: &Value,
        cond: &Operand,
        true_val: &Operand,
        false_val: &Operand,
        _ty: IrType,
    ) {
        self.operand_to_x0(false_val);
        self.state.emit("    mov x1, x0");
        self.operand_to_x0(true_val);
        self.state.emit("    mov x2, x0");
        self.operand_to_x0(cond);
        self.state.emit("    cmp x0, #0");
        self.state.emit("    csel x0, x2, x1, ne");
        self.state.reg_cache.invalidate_acc();
        self.store_x0_to(dest);
    }
}
