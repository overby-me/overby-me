//! X86Codegen: i128 arithmetic and comparison operations.

use super::emit::X86Codegen;
use crate::backend::state::StackSlot;
use crate::common::types::IrType;
use crate::ir::reexports::{IrCmpOp, Operand, Value};

impl X86Codegen {
    pub(super) fn emit_i128_prep_binop_impl(&mut self, lhs: &Operand, rhs: &Operand) {
        self.prep_i128_binop(lhs, rhs);
    }

    pub(super) fn emit_i128_add_impl(&mut self) {
        self.state.emit("    addq %rcx, %rax");
        self.state.emit("    adcq %rsi, %rdx");
    }

    pub(super) fn emit_i128_sub_impl(&mut self) {
        self.state.emit("    subq %rcx, %rax");
        self.state.emit("    sbbq %rsi, %rdx");
    }

    pub(super) fn emit_i128_mul_impl(&mut self) {
        self.state.emit("    pushq %rdx");
        self.state.emit("    pushq %rax");
        self.state.emit("    movq %rcx, %r8");
        self.state.emit("    movq %rsi, %r9");
        self.state.emit("    popq %rax");
        self.state.emit("    popq %rdi");
        self.state.emit("    movq %rdi, %rcx");
        self.state.emit("    imulq %r8, %rcx");
        self.state.emit("    movq %rax, %rsi");
        self.state.emit("    imulq %r9, %rsi");
        self.state.emit("    mulq %r8");
        self.state.emit("    addq %rcx, %rdx");
        self.state.emit("    addq %rsi, %rdx");
    }

    pub(super) fn emit_i128_and_impl(&mut self) {
        self.state.emit("    andq %rcx, %rax");
        self.state.emit("    andq %rsi, %rdx");
    }

    pub(super) fn emit_i128_or_impl(&mut self) {
        self.state.emit("    orq %rcx, %rax");
        self.state.emit("    orq %rsi, %rdx");
    }

    pub(super) fn emit_i128_xor_impl(&mut self) {
        self.state.emit("    xorq %rcx, %rax");
        self.state.emit("    xorq %rsi, %rdx");
    }

    pub(super) fn emit_i128_shl_impl(&mut self) {
        self.state.emit("    shldq %cl, %rax, %rdx");
        self.state.emit("    shlq %cl, %rax");
        self.state.emit("    testb $64, %cl");
        self.state.emit("    je 1f");
        self.state.emit("    movq %rax, %rdx");
        self.state.emit("    xorl %eax, %eax");
        self.state.emit("1:");
    }

    pub(super) fn emit_i128_lshr_impl(&mut self) {
        self.state.emit("    shrdq %cl, %rdx, %rax");
        self.state.emit("    shrq %cl, %rdx");
        self.state.emit("    testb $64, %cl");
        self.state.emit("    je 1f");
        self.state.emit("    movq %rdx, %rax");
        self.state.emit("    xorl %edx, %edx");
        self.state.emit("1:");
    }

    pub(super) fn emit_i128_ashr_impl(&mut self) {
        self.state.emit("    shrdq %cl, %rdx, %rax");
        self.state.emit("    sarq %cl, %rdx");
        self.state.emit("    testb $64, %cl");
        self.state.emit("    je 1f");
        self.state.emit("    movq %rdx, %rax");
        self.state.emit("    sarq $63, %rdx");
        self.state.emit("1:");
    }

    pub(super) fn emit_i128_prep_shift_lhs_impl(&mut self, lhs: &Operand) {
        self.operand_to_rax_rdx(lhs);
    }

    pub(super) fn emit_i128_shl_const_impl(&mut self, amount: u32) {
        let amount = amount & 127;
        if amount == 0 {
            // no-op
        } else if amount == 64 {
            self.state.emit("    movq %rax, %rdx");
            self.state.emit("    xorl %eax, %eax");
        } else if amount > 64 {
            self.state
                .out
                .emit_instr_imm_reg("    shlq", (amount - 64) as i64, "rax");
            self.state.emit("    movq %rax, %rdx");
            self.state.emit("    xorl %eax, %eax");
        } else {
            self.state
                .emit_fmt(format_args!("    shldq ${}, %rax, %rdx", amount));
            self.state
                .out
                .emit_instr_imm_reg("    shlq", amount as i64, "rax");
        }
    }

    pub(super) fn emit_i128_lshr_const_impl(&mut self, amount: u32) {
        let amount = amount & 127;
        if amount == 0 {
            // no-op
        } else if amount == 64 {
            self.state.emit("    movq %rdx, %rax");
            self.state.emit("    xorl %edx, %edx");
        } else if amount > 64 {
            self.state
                .out
                .emit_instr_imm_reg("    shrq", (amount - 64) as i64, "rdx");
            self.state.emit("    movq %rdx, %rax");
            self.state.emit("    xorl %edx, %edx");
        } else {
            self.state
                .emit_fmt(format_args!("    shrdq ${}, %rdx, %rax", amount));
            self.state
                .out
                .emit_instr_imm_reg("    shrq", amount as i64, "rdx");
        }
    }

    pub(super) fn emit_i128_ashr_const_impl(&mut self, amount: u32) {
        let amount = amount & 127;
        if amount == 0 {
            // no-op
        } else if amount == 64 {
            self.state.emit("    movq %rdx, %rax");
            self.state.emit("    sarq $63, %rdx");
        } else if amount > 64 {
            self.state
                .out
                .emit_instr_imm_reg("    sarq", (amount - 64) as i64, "rdx");
            self.state.emit("    movq %rdx, %rax");
            self.state.emit("    sarq $63, %rdx");
        } else {
            self.state
                .emit_fmt(format_args!("    shrdq ${}, %rdx, %rax", amount));
            self.state
                .out
                .emit_instr_imm_reg("    sarq", amount as i64, "rdx");
        }
    }

    pub(super) fn emit_i128_divrem_call_impl(
        &mut self,
        func_name: &str,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        self.operand_to_rax_rdx(rhs);
        self.state.emit("    pushq %rdx");
        self.state.emit("    pushq %rax");
        self.operand_to_rax_rdx(lhs);
        self.state.emit("    movq %rax, %rdi");
        self.state.emit("    movq %rdx, %rsi");
        self.state.emit("    popq %rdx");
        self.state.emit("    popq %rcx");
        self.state
            .emit_fmt(format_args!("    call {}@PLT", func_name));
    }

    pub(super) fn emit_i128_store_result_impl(&mut self, dest: &Value) {
        self.store_rax_rdx_to(dest);
    }

    pub(super) fn emit_i128_to_float_call_impl(
        &mut self,
        src: &Operand,
        from_signed: bool,
        to_ty: IrType,
    ) {
        self.operand_to_rax_rdx(src);
        self.state.emit("    movq %rax, %rdi");
        self.state.emit("    movq %rdx, %rsi");
        let func_name = match (from_signed, to_ty) {
            (true, IrType::F64) => "__floattidf",
            (true, IrType::F32) => "__floattisf",
            (false, IrType::F64) => "__floatuntidf",
            (false, IrType::F32) => "__floatuntisf",
            _ => panic!("unsupported i128-to-float conversion: {:?}", to_ty),
        };
        self.state
            .emit_fmt(format_args!("    call {}@PLT", func_name));
        self.state.reg_cache.invalidate_all();
        if to_ty == IrType::F32 {
            self.state.emit("    movd %xmm0, %eax");
        } else {
            self.state.emit("    movq %xmm0, %rax");
        }
    }

    pub(super) fn emit_float_to_i128_call_impl(
        &mut self,
        src: &Operand,
        to_signed: bool,
        from_ty: IrType,
    ) {
        self.operand_to_rax(src);
        if from_ty == IrType::F32 {
            self.state.emit("    movd %eax, %xmm0");
        } else {
            self.state.emit("    movq %rax, %xmm0");
        }
        let func_name = match (to_signed, from_ty) {
            (true, IrType::F64) => "__fixdfti",
            (true, IrType::F32) => "__fixsfti",
            (false, IrType::F64) => "__fixunsdfti",
            (false, IrType::F32) => "__fixunssfti",
            _ => panic!("unsupported float-to-i128 conversion: {:?}", from_ty),
        };
        self.state
            .emit_fmt(format_args!("    call {}@PLT", func_name));
        self.state.reg_cache.invalidate_all();
    }

    // ---- i128 cmp primitives ----

    pub(super) fn emit_i128_cmp_eq_impl(&mut self, is_ne: bool) {
        self.state.emit("    xorq %rcx, %rax");
        self.state.emit("    xorq %rsi, %rdx");
        self.state.emit("    orq %rdx, %rax");
        if is_ne {
            self.state.emit("    setne %al");
        } else {
            self.state.emit("    sete %al");
        }
        self.state.emit("    movzbq %al, %rax");
    }

    pub(super) fn emit_i128_cmp_ordered_impl(&mut self, op: IrCmpOp) {
        self.state.emit("    cmpq %rsi, %rdx");
        let set_hi = match op {
            IrCmpOp::Slt | IrCmpOp::Sle => "setl",
            IrCmpOp::Sgt | IrCmpOp::Sge => "setg",
            IrCmpOp::Ult | IrCmpOp::Ule => "setb",
            IrCmpOp::Ugt | IrCmpOp::Uge => "seta",
            _ => unreachable!("i128 ordered cmp got equality op: {:?}", op),
        };
        self.state.emit_fmt(format_args!("    {} %r8b", set_hi));
        self.state.emit("    jne 1f");
        self.state.emit("    cmpq %rcx, %rax");
        let set_lo = match op {
            IrCmpOp::Slt | IrCmpOp::Ult => "setb",
            IrCmpOp::Sle | IrCmpOp::Ule => "setbe",
            IrCmpOp::Sgt | IrCmpOp::Ugt => "seta",
            IrCmpOp::Sge | IrCmpOp::Uge => "setae",
            _ => unreachable!("i128 ordered cmp (low word) got equality op: {:?}", op),
        };
        self.state.emit_fmt(format_args!("    {} %r8b", set_lo));
        self.state.emit("1:");
        self.state.emit("    movzbq %r8b, %rax");
    }

    pub(super) fn emit_i128_cmp_store_result_impl(&mut self, dest: &Value) {
        self.store_rax_to(dest);
    }

    // ---- i128 load/store acc pair ----

    pub(super) fn emit_load_acc_pair_impl(&mut self, op: &Operand) {
        self.operand_to_rax_rdx(op);
    }

    pub(super) fn emit_store_acc_pair_impl(&mut self, dest: &Value) {
        self.store_rax_rdx_to(dest);
    }

    pub(super) fn emit_sign_extend_acc_high_impl(&mut self) {
        self.state.emit("    cqto"); // sign-extend rax into rdx:rax
    }

    pub(super) fn emit_zero_acc_high_impl(&mut self) {
        self.state.emit("    xorl %edx, %edx");
    }

    pub(super) fn emit_store_pair_to_slot_impl(&mut self, slot: StackSlot) {
        self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0);
        self.state
            .out
            .emit_instr_reg_rbp("    movq", "rdx", slot.0 + 8);
    }

    pub(super) fn emit_load_pair_from_slot_impl(&mut self, slot: StackSlot) {
        self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rax");
        self.state
            .out
            .emit_instr_rbp_reg("    movq", slot.0 + 8, "rdx");
    }

    pub(super) fn emit_save_acc_pair_impl(&mut self) {
        self.state.emit("    movq %rax, %rsi");
        self.state.emit("    movq %rdx, %rdi");
    }

    pub(super) fn emit_store_pair_indirect_impl(&mut self) {
        // pair saved to rsi:rdi by emit_save_acc_pair, ptr in rcx via emit_load_ptr_from_slot
        self.state.emit("    movq %rsi, (%rcx)");
        self.state.emit("    movq %rdi, 8(%rcx)");
    }

    pub(super) fn emit_load_pair_indirect_impl(&mut self) {
        // ptr in rcx via emit_load_ptr_from_slot, load pair from it
        self.state.emit("    movq (%rcx), %rax");
        self.state.emit("    movq 8(%rcx), %rdx");
    }

    pub(super) fn emit_i128_neg_impl(&mut self) {
        self.state.emit("    notq %rax");
        self.state.emit("    notq %rdx");
        self.state.emit("    addq $1, %rax");
        self.state.emit("    adcq $0, %rdx");
    }

    pub(super) fn emit_i128_not_impl(&mut self) {
        self.state.emit("    notq %rax");
        self.state.emit("    notq %rdx");
    }
}
