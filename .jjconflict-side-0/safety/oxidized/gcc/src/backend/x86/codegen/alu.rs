//! X86Codegen: integer/float arithmetic, unary ops, binop, copy.

use super::emit::{shift_mnemonic, X86Codegen};
use crate::common::types::IrType;
use crate::ir::reexports::{IrBinOp, Operand, Value};

impl X86Codegen {
    // ---- Unary ----

    pub(super) fn emit_float_neg_impl(&mut self, ty: IrType) {
        if ty == IrType::F32 {
            self.state.emit("    movd %eax, %xmm0");
            self.state.emit("    movl $0x80000000, %ecx");
            self.state.emit("    movd %ecx, %xmm1");
            self.state.emit("    xorps %xmm1, %xmm0");
            self.state.emit("    movd %xmm0, %eax");
        } else {
            self.state.emit("    movq %rax, %xmm0");
            self.state.emit("    movabsq $-9223372036854775808, %rcx");
            self.state.emit("    movq %rcx, %xmm1");
            self.state.emit("    xorpd %xmm1, %xmm0");
            self.state.emit("    movq %xmm0, %rax");
        }
    }

    pub(super) fn emit_int_neg_impl(&mut self, _ty: IrType) {
        self.state.emit("    negq %rax");
    }

    pub(super) fn emit_int_not_impl(&mut self, _ty: IrType) {
        self.state.emit("    notq %rax");
    }

    pub(super) fn emit_int_clz_impl(&mut self, ty: IrType) {
        if ty == IrType::I32 || ty == IrType::U32 {
            self.state.emit("    lzcntl %eax, %eax");
        } else {
            self.state.emit("    lzcntq %rax, %rax");
        }
    }

    pub(super) fn emit_int_ctz_impl(&mut self, ty: IrType) {
        if ty == IrType::I32 || ty == IrType::U32 {
            self.state.emit("    tzcntl %eax, %eax");
        } else {
            self.state.emit("    tzcntq %rax, %rax");
        }
    }

    pub(super) fn emit_int_bswap_impl(&mut self, ty: IrType) {
        if ty == IrType::I16 || ty == IrType::U16 {
            self.state.emit("    rolw $8, %ax");
        } else if ty == IrType::I32 || ty == IrType::U32 {
            self.state.emit("    bswapl %eax");
        } else {
            self.state.emit("    bswapq %rax");
        }
    }

    pub(super) fn emit_int_popcount_impl(&mut self, ty: IrType) {
        if ty == IrType::I32 || ty == IrType::U32 {
            self.state.emit("    popcntl %eax, %eax");
        } else {
            self.state.emit("    popcntq %rax, %rax");
        }
    }

    // ---- Binop ----

    pub(super) fn emit_int_binop_impl(
        &mut self,
        dest: &Value,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        let use_32bit = ty == IrType::I32 || ty == IrType::U32;
        let is_unsigned = ty.is_unsigned();

        // Register-direct path
        if let Some(dest_phys) = self.dest_reg(dest) {
            let is_simple_alu = matches!(
                op,
                IrBinOp::Add
                    | IrBinOp::Sub
                    | IrBinOp::And
                    | IrBinOp::Or
                    | IrBinOp::Xor
                    | IrBinOp::Mul
            );
            if is_simple_alu {
                self.emit_alu_reg_direct(op, lhs, rhs, dest_phys, use_32bit, is_unsigned);
                return;
            }
            if matches!(op, IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr) {
                self.emit_shift_reg_direct(op, lhs, rhs, dest_phys, use_32bit, is_unsigned);
                return;
            }
        }

        // Accumulator-based fallback: try immediate optimizations first
        if self.try_emit_acc_immediate(dest, op, lhs, rhs, use_32bit, is_unsigned) {
            return;
        }

        // General case: load lhs to rax, rhs to rcx
        self.operand_to_rax(lhs);
        self.operand_to_rcx(rhs);

        match op {
            IrBinOp::Add | IrBinOp::Sub | IrBinOp::Mul => {
                let mnem = match op {
                    IrBinOp::Add => "add",
                    IrBinOp::Sub => "sub",
                    IrBinOp::Mul => "imul",
                    _ => unreachable!("unexpected i64 binop: {:?}", op),
                };
                if use_32bit {
                    self.state
                        .emit_fmt(format_args!("    {}l %ecx, %eax", mnem));
                    if !is_unsigned {
                        self.state.emit("    cltq");
                    }
                } else {
                    self.state
                        .emit_fmt(format_args!("    {}q %rcx, %rax", mnem));
                }
            }
            IrBinOp::SDiv => {
                if use_32bit {
                    self.state.emit("    cltd");
                    self.state.emit("    idivl %ecx");
                    self.state.emit("    cltq");
                } else {
                    self.state.emit("    cqto");
                    self.state.emit("    idivq %rcx");
                }
            }
            IrBinOp::UDiv => {
                self.state.emit("    xorl %edx, %edx");
                if use_32bit {
                    self.state.emit("    divl %ecx");
                } else {
                    self.state.emit("    divq %rcx");
                }
            }
            IrBinOp::SRem => {
                if use_32bit {
                    self.state.emit("    cltd");
                    self.state.emit("    idivl %ecx");
                    self.state.emit("    movl %edx, %eax");
                    self.state.emit("    cltq");
                } else {
                    self.state.emit("    cqto");
                    self.state.emit("    idivq %rcx");
                    self.state.emit("    movq %rdx, %rax");
                }
            }
            IrBinOp::URem => {
                self.state.emit("    xorl %edx, %edx");
                if use_32bit {
                    self.state.emit("    divl %ecx");
                    self.state.emit("    movl %edx, %eax");
                } else {
                    self.state.emit("    divq %rcx");
                    self.state.emit("    movq %rdx, %rax");
                }
            }
            IrBinOp::And => self.state.emit("    andq %rcx, %rax"),
            IrBinOp::Or => self.state.emit("    orq %rcx, %rax"),
            IrBinOp::Xor => self.state.emit("    xorq %rcx, %rax"),
            IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr => {
                let (mnem32, mnem64) = shift_mnemonic(op);
                if use_32bit {
                    self.state
                        .emit_fmt(format_args!("    {} %cl, %eax", mnem32));
                    if !is_unsigned && op != IrBinOp::LShr {
                        self.state.emit("    cltq");
                    }
                } else {
                    self.state
                        .emit_fmt(format_args!("    {} %cl, %rax", mnem64));
                }
            }
        }

        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }

    pub(super) fn emit_copy_i128_impl(&mut self, dest: &Value, src: &Operand) {
        self.operand_to_rax_rdx(src);
        self.store_rax_rdx_to(dest);
    }
}
