//! I686Codegen: ALU operations (integer arithmetic, bitwise, shifts).

use super::emit::{alu_mnemonic, shift_mnemonic, I686Codegen};
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{IrBinOp, Operand, Value};

impl I686Codegen {
    pub(super) fn emit_float_neg_impl(&mut self, ty: IrType) {
        if ty == IrType::F32 {
            self.state.emit("    movd %eax, %xmm0");
            self.state.emit("    movl $0x80000000, %ecx");
            self.state.emit("    movd %ecx, %xmm1");
            self.state.emit("    xorps %xmm1, %xmm0");
            self.state.emit("    movd %xmm0, %eax");
        } else {
            self.state.emit("    xorl $0x80000000, %eax");
        }
    }

    pub(super) fn emit_int_neg_impl(&mut self, _ty: IrType) {
        self.state.emit("    negl %eax");
    }

    pub(super) fn emit_int_not_impl(&mut self, _ty: IrType) {
        self.state.emit("    notl %eax");
    }

    pub(super) fn emit_int_clz_impl(&mut self, ty: IrType) {
        if matches!(ty, IrType::I32 | IrType::U32 | IrType::Ptr) {
            self.state.emit("    lzcntl %eax, %eax");
        } else if matches!(ty, IrType::I16 | IrType::U16) {
            self.state.emit("    lzcntw %ax, %ax");
        } else {
            self.state.emit("    lzcntl %eax, %eax");
        }
    }

    pub(super) fn emit_int_ctz_impl(&mut self, _ty: IrType) {
        // tzcntl works for all integer widths on i686: the value is in %eax
        // and trailing zero count is the same regardless of nominal width.
        self.state.emit("    tzcntl %eax, %eax");
    }

    pub(super) fn emit_int_bswap_impl(&mut self, ty: IrType) {
        match ty {
            IrType::I16 | IrType::U16 => self.state.emit("    rolw $8, %ax"),
            IrType::I32 | IrType::U32 | IrType::Ptr => self.state.emit("    bswapl %eax"),
            _ => self.state.emit("    bswapl %eax"),
        }
    }

    pub(super) fn emit_int_popcount_impl(&mut self, _ty: IrType) {
        self.state.emit("    popcntl %eax, %eax");
    }

    pub(super) fn emit_int_binop_impl(
        &mut self,
        dest: &Value,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
        _ty: IrType,
    ) {
        // Immediate optimization for ALU ops
        if matches!(
            op,
            IrBinOp::Add | IrBinOp::Sub | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor
        ) {
            if let Some(imm) = Self::const_as_imm32(rhs) {
                self.operand_to_eax(lhs);
                let mnem = alu_mnemonic(op);
                emit!(self.state, "    {}l ${}, %eax", mnem, imm);
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
                return;
            }
        }

        // Immediate multiply
        if op == IrBinOp::Mul {
            if let Some(imm) = Self::const_as_imm32(rhs) {
                self.operand_to_eax(lhs);
                match imm {
                    3 => emit!(self.state, "    leal (%eax, %eax, 2), %eax"),
                    5 => emit!(self.state, "    leal (%eax, %eax, 4), %eax"),
                    9 => emit!(self.state, "    leal (%eax, %eax, 8), %eax"),
                    _ => emit!(self.state, "    imull ${}, %eax, %eax", imm),
                }
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
                return;
            }
        }

        // Immediate shift
        if matches!(op, IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr) {
            if let Some(imm) = Self::const_as_imm32(rhs) {
                self.operand_to_eax(lhs);
                let mnem = shift_mnemonic(op);
                let shift_amount = (imm as u32) & 31;
                emit!(self.state, "    {} ${}, %eax", mnem, shift_amount);
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
                return;
            }
        }

        // General case: load lhs to eax, rhs to ecx
        self.operand_to_eax(lhs);
        self.operand_to_ecx(rhs);

        match op {
            IrBinOp::Add => self.state.emit("    addl %ecx, %eax"),
            IrBinOp::Sub => self.state.emit("    subl %ecx, %eax"),
            IrBinOp::Mul => self.state.emit("    imull %ecx, %eax"),
            IrBinOp::And => self.state.emit("    andl %ecx, %eax"),
            IrBinOp::Or => self.state.emit("    orl %ecx, %eax"),
            IrBinOp::Xor => self.state.emit("    xorl %ecx, %eax"),
            IrBinOp::Shl => self.state.emit("    shll %cl, %eax"),
            IrBinOp::AShr => self.state.emit("    sarl %cl, %eax"),
            IrBinOp::LShr => self.state.emit("    shrl %cl, %eax"),
            IrBinOp::SDiv => {
                self.state.emit("    cltd");
                self.state.emit("    idivl %ecx");
            }
            IrBinOp::UDiv => {
                self.state.emit("    xorl %edx, %edx");
                self.state.emit("    divl %ecx");
            }
            IrBinOp::SRem => {
                self.state.emit("    cltd");
                self.state.emit("    idivl %ecx");
                self.state.emit("    movl %edx, %eax");
            }
            IrBinOp::URem => {
                self.state.emit("    xorl %edx, %edx");
                self.state.emit("    divl %ecx");
                self.state.emit("    movl %edx, %eax");
            }
        }
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }
}
