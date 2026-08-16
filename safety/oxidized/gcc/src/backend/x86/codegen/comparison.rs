//! X86Codegen: comparison and select operations.

use super::emit::{phys_reg_name, X86Codegen};
use crate::common::types::IrType;
use crate::ir::reexports::{BlockId, IrCmpOp, IrConst, Operand, Value};

impl X86Codegen {
    pub(super) fn emit_float_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        let (mov_to_xmm0, mov_to_xmm1_from_rcx) = if ty == IrType::F32 {
            ("movd %eax, %xmm0", "movd %ecx, %xmm1")
        } else {
            ("movq %rax, %xmm0", "movq %rcx, %xmm1")
        };
        let swap_operands = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
        );
        let (first, second) = if swap_operands {
            (rhs, lhs)
        } else {
            (lhs, rhs)
        };
        self.operand_to_rax(first);
        self.state.emit_fmt(format_args!("    {}", mov_to_xmm0));
        self.operand_to_rcx(second);
        self.state
            .emit_fmt(format_args!("    {}", mov_to_xmm1_from_rcx));
        if ty == IrType::F64 {
            self.state.emit("    ucomisd %xmm1, %xmm0");
        } else {
            self.state.emit("    ucomiss %xmm1, %xmm0");
        }
        match op {
            IrCmpOp::Eq => {
                self.state.emit("    setnp %al");
                self.state.emit("    sete %cl");
                self.state.emit("    andb %cl, %al");
            }
            IrCmpOp::Ne => {
                self.state.emit("    setp %al");
                self.state.emit("    setne %cl");
                self.state.emit("    orb %cl, %al");
            }
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sgt | IrCmpOp::Ugt => {
                self.state.emit("    seta %al");
            }
            IrCmpOp::Sle | IrCmpOp::Ule | IrCmpOp::Sge | IrCmpOp::Uge => {
                self.state.emit("    setae %al");
            }
        }
        self.state.emit("    movzbq %al, %rax");
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }

    pub(super) fn emit_f128_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        let swap_x87 = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
        );
        let (first_x87, second_x87) = if swap_x87 { (lhs, rhs) } else { (rhs, lhs) };
        self.emit_f128_load_to_x87(first_x87);
        self.emit_f128_load_to_x87(second_x87);
        self.state.emit("    fucomip %st(1), %st");
        self.state.emit("    fstp %st(0)");
        match op {
            IrCmpOp::Eq => {
                self.state.emit("    setnp %al");
                self.state.emit("    sete %cl");
                self.state.emit("    andb %cl, %al");
            }
            IrCmpOp::Ne => {
                self.state.emit("    setp %al");
                self.state.emit("    setne %cl");
                self.state.emit("    orb %cl, %al");
            }
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sgt | IrCmpOp::Ugt => {
                self.state.emit("    seta %al");
            }
            IrCmpOp::Sle | IrCmpOp::Ule | IrCmpOp::Sge | IrCmpOp::Uge => {
                self.state.emit("    setae %al");
            }
        }
        self.state.emit("    movzbq %al, %rax");
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }

    pub(super) fn emit_int_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        let use_32bit = ty == IrType::I32 || ty == IrType::U32;
        self.emit_int_cmp_insn_typed(lhs, rhs, use_32bit);

        let set_instr = match op {
            IrCmpOp::Eq => "sete",
            IrCmpOp::Ne => "setne",
            IrCmpOp::Slt => "setl",
            IrCmpOp::Sle => "setle",
            IrCmpOp::Sgt => "setg",
            IrCmpOp::Sge => "setge",
            IrCmpOp::Ult => "setb",
            IrCmpOp::Ule => "setbe",
            IrCmpOp::Ugt => "seta",
            IrCmpOp::Uge => "setae",
        };
        self.state.emit_fmt(format_args!("    {} %al", set_instr));
        self.state.emit("    movzbq %al, %rax");
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
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
        let use_32bit = ty == IrType::I32 || ty == IrType::U32;
        self.emit_int_cmp_insn_typed(lhs, rhs, use_32bit);

        let jcc = match op {
            IrCmpOp::Eq => "je",
            IrCmpOp::Ne => "jne",
            IrCmpOp::Slt => "jl",
            IrCmpOp::Sle => "jle",
            IrCmpOp::Sgt => "jg",
            IrCmpOp::Sge => "jge",
            IrCmpOp::Ult => "jb",
            IrCmpOp::Ule => "jbe",
            IrCmpOp::Ugt => "ja",
            IrCmpOp::Uge => "jae",
        };
        self.state
            .emit_fmt(format_args!("    {} {}", jcc, true_label));
        self.state.out.emit_jmp_label(false_label);
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_fused_cmp_branch_blocks_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
        true_block: BlockId,
        false_block: BlockId,
    ) {
        let use_32bit = ty == IrType::I32 || ty == IrType::U32;
        self.emit_int_cmp_insn_typed(lhs, rhs, use_32bit);

        let jcc = match op {
            IrCmpOp::Eq => "    je",
            IrCmpOp::Ne => "    jne",
            IrCmpOp::Slt => "    jl",
            IrCmpOp::Sle => "    jle",
            IrCmpOp::Sgt => "    jg",
            IrCmpOp::Sge => "    jge",
            IrCmpOp::Ult => "    jb",
            IrCmpOp::Ule => "    jbe",
            IrCmpOp::Ugt => "    ja",
            IrCmpOp::Uge => "    jae",
        };
        self.state.out.emit_jcc_block(jcc, true_block.0);
        self.state.out.emit_jmp_block(false_block.0);
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_cond_branch_blocks_impl(
        &mut self,
        cond: &Operand,
        true_block: BlockId,
        false_block: BlockId,
    ) {
        self.operand_to_rax(cond);
        self.state.emit("    testq %rax, %rax");
        self.state.out.emit_jcc_block("    jne", true_block.0);
        self.state.out.emit_jmp_block(false_block.0);
    }

    pub(super) fn emit_select_impl(
        &mut self,
        dest: &Value,
        cond: &Operand,
        true_val: &Operand,
        false_val: &Operand,
        _ty: IrType,
    ) {
        self.operand_to_rax(false_val);
        self.operand_to_rcx(true_val);

        match cond {
            Operand::Const(c) => {
                let val = match c {
                    IrConst::I8(v) => *v as i64,
                    IrConst::I16(v) => *v as i64,
                    IrConst::I32(v) => *v as i64,
                    IrConst::I64(v) => *v,
                    IrConst::Zero => 0,
                    _ => 0,
                };
                if val == 0 {
                    self.state.emit("    xorl %edx, %edx");
                } else if val >= i32::MIN as i64 && val <= i32::MAX as i64 {
                    self.state.out.emit_instr_imm_reg("    movq", val, "rdx");
                } else {
                    self.state.out.emit_instr_imm_reg("    movabsq", val, "rdx");
                }
            }
            Operand::Value(v) => {
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    let reg_name = phys_reg_name(reg);
                    self.state
                        .out
                        .emit_instr_reg_reg("    movq", reg_name, "rdx");
                } else if self.state.get_slot(v.0).is_some() {
                    self.value_to_reg(v, "rdx");
                } else {
                    self.state.emit("    xorl %edx, %edx");
                }
            }
        }
        self.state.emit("    testq %rdx, %rdx");
        self.state.emit("    cmovneq %rcx, %rax");
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }
}
