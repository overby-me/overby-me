//! ArmCodegen: floating-point binary operations.

use super::emit::ArmCodegen;
use crate::backend::cast::FloatOp;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{Operand, Value};

impl ArmCodegen {
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
        // Non-F128: use default path.
        let mnemonic = self.emit_float_binop_mnemonic(op);
        self.operand_to_x0(lhs);
        self.state.emit("    mov x1, x0");
        self.operand_to_x0(rhs);
        self.emit_float_binop_body(mnemonic, ty);
        self.store_x0_to(dest);
    }

    pub(super) fn emit_float_binop_body(&mut self, mnemonic: &str, ty: IrType) {
        self.state.emit("    mov x2, x0");
        if ty == IrType::F32 {
            self.state.emit("    fmov s0, w1");
            self.state.emit("    fmov s1, w2");
            self.state
                .emit_fmt(format_args!("    {} s0, s0, s1", mnemonic));
            self.state.emit("    fmov w0, s0");
            self.state.emit("    mov w0, w0"); // zero-extend
        } else {
            self.state.emit("    fmov d0, x1");
            self.state.emit("    fmov d1, x2");
            self.state
                .emit_fmt(format_args!("    {} d0, d0, d1", mnemonic));
            self.state.emit("    fmov x0, d0");
        }
    }

    pub(super) fn emit_f128_neg_impl(&mut self, dest: &Value, src: &Operand) {
        self.emit_f128_neg_full(dest, src);
    }
}
