//! RiscvCodegen: float binary operations.

use super::emit::RiscvCodegen;
use crate::backend::cast::FloatOp;
use crate::common::types::IrType;
use crate::ir::reexports::{Operand, Value};

impl RiscvCodegen {
    pub(super) fn emit_float_binop_impl(
        &mut self,
        dest: &Value,
        op: FloatOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        if ty == IrType::F128 {
            crate::backend::f128_softfloat::f128_emit_binop(self, dest, op, lhs, rhs);
            return;
        }

        // Non-F128: use the default implementation (loads f64/f32 via operand_to_t0)
        let mnemonic = self.emit_float_binop_mnemonic(op);
        self.operand_to_t0(lhs);
        self.state.emit("    mv t1, t0");
        self.operand_to_t0(rhs);
        self.emit_float_binop_body(mnemonic, ty);
        self.store_t0_to(dest);
    }

    pub(super) fn emit_float_binop_body(&mut self, mnemonic: &str, ty: IrType) {
        // After setup: t1 = lhs, t0 = rhs
        self.state.emit("    mv t2, t0"); // t2 = rhs
        if ty == IrType::F64 {
            self.state.emit("    fmv.d.x ft0, t1");
            self.state.emit("    fmv.d.x ft1, t2");
            self.state
                .emit_fmt(format_args!("    {}.d ft0, ft0, ft1", mnemonic));
            self.state.emit("    fmv.x.d t0, ft0");
        } else {
            self.state.emit("    fmv.w.x ft0, t1");
            self.state.emit("    fmv.w.x ft1, t2");
            self.state
                .emit_fmt(format_args!("    {}.s ft0, ft0, ft1", mnemonic));
            self.state.emit("    fmv.x.w t0, ft0");
        }
    }

    /// Map FloatOp to RISC-V float mnemonic prefix.
    pub(super) fn emit_float_binop_mnemonic(&self, op: FloatOp) -> &'static str {
        match op {
            FloatOp::Add => "fadd",
            FloatOp::Sub => "fsub",
            FloatOp::Mul => "fmul",
            FloatOp::Div => "fdiv",
        }
    }

    pub(super) fn emit_f128_neg_impl(&mut self, dest: &Value, src: &Operand) {
        self.emit_f128_neg_full(dest, src);
    }
}
