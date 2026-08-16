//! RiscvCodegen: integer/float arithmetic, unary ops, binop, copy.

use super::emit::RiscvCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{IrBinOp, Operand, Value};

impl RiscvCodegen {
    // ---- Unary ----

    pub(super) fn emit_float_neg_impl(&mut self, ty: IrType) {
        if ty == IrType::F64 {
            self.state.emit("    fmv.d.x ft0, t0");
            self.state.emit("    fneg.d ft0, ft0");
            self.state.emit("    fmv.x.d t0, ft0");
        } else {
            self.state.emit("    fmv.w.x ft0, t0");
            self.state.emit("    fneg.s ft0, ft0");
            self.state.emit("    fmv.x.w t0, ft0");
        }
    }

    pub(super) fn emit_int_neg_impl(&mut self, _ty: IrType) {
        self.state.emit("    neg t0, t0");
    }

    pub(super) fn emit_int_not_impl(&mut self, _ty: IrType) {
        self.state.emit("    not t0, t0");
    }

    // ---- Integer binop ----

    pub(super) fn emit_int_binop_impl(
        &mut self,
        dest: &Value,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        // Note: i128 dispatch is handled by the shared emit_binop default in traits.rs.
        self.operand_to_t0(lhs);
        self.state.emit("    mv t1, t0");
        self.operand_to_t0(rhs);
        self.state.emit("    mv t2, t0");

        let use_32bit = ty == IrType::I32 || ty == IrType::U32;

        let mnemonic = match (op, use_32bit) {
            (IrBinOp::Add, false) => "add",
            (IrBinOp::Add, true) => "addw",
            (IrBinOp::Sub, false) => "sub",
            (IrBinOp::Sub, true) => "subw",
            (IrBinOp::Mul, false) => "mul",
            (IrBinOp::Mul, true) => "mulw",
            (IrBinOp::SDiv, false) => "div",
            (IrBinOp::SDiv, true) => "divw",
            (IrBinOp::UDiv, false) => "divu",
            (IrBinOp::UDiv, true) => "divuw",
            (IrBinOp::SRem, false) => "rem",
            (IrBinOp::SRem, true) => "remw",
            (IrBinOp::URem, false) => "remu",
            (IrBinOp::URem, true) => "remuw",
            (IrBinOp::And, _) => "and",
            (IrBinOp::Or, _) => "or",
            (IrBinOp::Xor, _) => "xor",
            (IrBinOp::Shl, false) => "sll",
            (IrBinOp::Shl, true) => "sllw",
            (IrBinOp::AShr, false) => "sra",
            (IrBinOp::AShr, true) => "sraw",
            (IrBinOp::LShr, false) => "srl",
            (IrBinOp::LShr, true) => "srlw",
        };
        self.state
            .emit_fmt(format_args!("    {} t0, t1, t2", mnemonic));

        self.store_t0_to(dest);
    }

    // ---- Copy i128 ----

    pub(super) fn emit_copy_i128_impl(&mut self, dest: &Value, src: &Operand) {
        self.operand_to_t0_t1(src);
        self.store_t0_t1_to(dest);
    }
}
