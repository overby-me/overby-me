//! I686Codegen: comparison operations (float, int, fused branches, select).

use super::emit::I686Codegen;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{IrCmpOp, Operand, Value};

impl I686Codegen {
    pub(super) fn emit_float_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        if ty == IrType::F64 {
            let swap = matches!(
                op,
                IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
            );
            let (first, second) = if swap { (lhs, rhs) } else { (rhs, lhs) };
            self.emit_f64_load_to_x87(first);
            self.emit_f64_load_to_x87(second);
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
            self.state.emit("    movzbl %al, %eax");
            self.state.reg_cache.invalidate_acc();
            self.store_eax_to(dest);
            return;
        }
        // F32: Use SSE for float comparisons
        let swap_operands = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
        );
        let (first, second) = if swap_operands {
            (rhs, lhs)
        } else {
            (lhs, rhs)
        };

        self.operand_to_eax(first);
        self.state.emit("    movd %eax, %xmm0");
        self.operand_to_ecx(second);
        self.state.emit("    movd %ecx, %xmm1");
        self.state.emit("    ucomiss %xmm1, %xmm0");

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
        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    pub(super) fn emit_f128_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        let swap = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
        );
        let (first, second) = if swap { (lhs, rhs) } else { (rhs, lhs) };
        self.emit_f128_load_to_x87(first);
        self.emit_f128_load_to_x87(second);
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
        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    pub(super) fn emit_int_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        _ty: IrType,
    ) {
        self.operand_to_eax(lhs);
        self.operand_to_ecx(rhs);
        self.state.emit("    cmpl %ecx, %eax");

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
        emit!(self.state, "    {} %al", set_instr);
        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    pub(super) fn emit_fused_cmp_branch_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        _ty: IrType,
        true_label: &str,
        false_label: &str,
    ) {
        self.operand_to_eax(lhs);
        self.operand_to_ecx(rhs);
        self.state.emit("    cmpl %ecx, %eax");

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
        emit!(self.state, "    {} {}", jcc, true_label);
        emit!(self.state, "    jmp {}", false_label);
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_select_impl(
        &mut self,
        dest: &Value,
        cond: &Operand,
        true_val: &Operand,
        false_val: &Operand,
        ty: IrType,
    ) {
        use crate::ir::reexports::IrConst;
        // Constant-fold wide conditions at compile time
        match cond {
            Operand::Const(IrConst::I64(v)) => {
                self.emit_copy_value(dest, if *v != 0 { true_val } else { false_val });
                return;
            }
            Operand::Const(IrConst::F64(fval)) => {
                self.emit_copy_value(dest, if *fval != 0.0 { true_val } else { false_val });
                return;
            }
            _ => {}
        }

        let cond_is_wide = matches!(cond, Operand::Value(v) if self.state.is_wide_value(v.0));
        let result_is_wide = matches!(ty, IrType::F64 | IrType::I64 | IrType::U64);

        if !cond_is_wide && !result_is_wide {
            let label_id = self.state.next_label_id();
            let true_label = format!(".Lsel_true_{}", label_id);
            let end_label = format!(".Lsel_end_{}", label_id);
            self.emit_load_operand(cond);
            self.emit_branch_nonzero(&true_label);
            self.emit_load_operand(false_val);
            self.emit_store_result(dest);
            self.emit_branch(&end_label);
            self.state.emit_fmt(format_args!("{}:", true_label));
            self.emit_load_operand(true_val);
            self.emit_store_result(dest);
            self.state.emit_fmt(format_args!("{}:", end_label));
            return;
        }

        let label_id = self.state.next_label_id();
        let true_label = format!(".Lsel_true_{}", label_id);
        let end_label = format!(".Lsel_end_{}", label_id);

        if cond_is_wide {
            if let Operand::Value(v) = cond {
                self.emit_wide_value_to_eax_ored(v.0);
                self.state.reg_cache.invalidate_acc();
            }
        } else {
            self.operand_to_eax(cond);
        }

        self.emit_branch_nonzero(&true_label);

        self.emit_copy_value(dest, false_val);
        self.emit_branch(&end_label);

        self.state.emit_fmt(format_args!("{}:", true_label));
        self.emit_copy_value(dest, true_val);

        self.state.emit_fmt(format_args!("{}:", end_label));
    }
}
