//! X86Codegen: floating-point binary operations and F128 negation.

use super::emit::X86Codegen;
use crate::backend::cast::FloatOp;
use crate::common::types::IrType;
use crate::ir::reexports::{IrUnaryOp, Operand, Value};

impl X86Codegen {
    pub(super) fn emit_float_binop_impl(
        &mut self,
        dest: &Value,
        op: FloatOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        if ty == IrType::F128 {
            let x87_op = match op {
                FloatOp::Add => "faddp",
                FloatOp::Sub => "fsubrp",
                FloatOp::Mul => "fmulp",
                FloatOp::Div => "fdivrp",
            };
            self.emit_f128_load_to_x87(lhs);
            self.emit_f128_load_to_x87(rhs);
            self.state
                .emit_fmt(format_args!("    {} %st, %st(1)", x87_op));
            if let Some(dest_slot) = self.state.get_slot(dest.0) {
                self.state.out.emit_instr_rbp("    fstpt", dest_slot.0);
                self.state.out.emit_instr_rbp("    fldt", dest_slot.0);
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    popq %rax");
                self.state.reg_cache.set_acc(dest.0, false);
                self.state.f128_direct_slots.insert(dest.0);
            } else {
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    popq %rax");
                self.state.reg_cache.invalidate_acc();
                self.store_rax_to(dest);
            }
            return;
        }
        let mnemonic = self.emit_float_binop_mnemonic_impl(op);
        let (mov_rax_to_xmm0, mov_rcx_to_xmm1, mov_xmm0_to_rax) = if ty == IrType::F32 {
            ("movd %eax, %xmm0", "movd %ecx, %xmm1", "movd %xmm0, %eax")
        } else {
            ("movq %rax, %xmm0", "movq %rcx, %xmm1", "movq %xmm0, %rax")
        };
        self.operand_to_rax(lhs);
        self.state.emit_fmt(format_args!("    {}", mov_rax_to_xmm0));
        self.operand_to_rcx(rhs);
        self.state.emit_fmt(format_args!("    {}", mov_rcx_to_xmm1));
        let suffix = if ty == IrType::F64 { "sd" } else { "ss" };
        self.state
            .emit_fmt(format_args!("    {}{} %xmm1, %xmm0", mnemonic, suffix));
        self.state.emit_fmt(format_args!("    {}", mov_xmm0_to_rax));
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }

    pub(super) fn emit_float_binop_impl_impl(&mut self, _mnemonic: &str, _ty: IrType) {
        unreachable!("x86 emit_float_binop_impl should not be called directly");
    }

    pub(super) fn emit_float_binop_mnemonic_impl(&self, op: FloatOp) -> &'static str {
        match op {
            FloatOp::Add => "add",
            FloatOp::Sub => "sub",
            FloatOp::Mul => "mul",
            FloatOp::Div => "div",
        }
    }

    pub(super) fn emit_unaryop_impl(
        &mut self,
        dest: &Value,
        op: IrUnaryOp,
        src: &Operand,
        ty: IrType,
    ) {
        if ty == IrType::F128 && op == IrUnaryOp::Neg {
            self.emit_f128_load_to_x87(src);
            self.state.emit("    fchs");
            if let Some(dest_slot) = self.state.get_slot(dest.0) {
                self.state.out.emit_instr_rbp("    fstpt", dest_slot.0);
                self.state.out.emit_instr_rbp("    fldt", dest_slot.0);
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    popq %rax");
                self.state.reg_cache.set_acc(dest.0, false);
                self.state.f128_direct_slots.insert(dest.0);
            } else {
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    popq %rax");
                self.state.reg_cache.invalidate_acc();
                self.store_rax_to(dest);
            }
            return;
        }
        crate::backend::traits::emit_unaryop_default(self, dest, op, src, ty);
    }
}
