//! ArmCodegen: 128-bit integer operations.

use super::emit::ArmCodegen;
use crate::backend::state::StackSlot;
use crate::common::types::IrType;
use crate::ir::reexports::{IrCmpOp, Operand, Value};

impl ArmCodegen {
    pub(super) fn emit_load_acc_pair_impl(&mut self, op: &Operand) {
        self.operand_to_x0_x1(op);
    }

    pub(super) fn emit_store_acc_pair_impl(&mut self, dest: &Value) {
        self.store_x0_x1_to(dest);
    }

    pub(super) fn emit_store_pair_to_slot_impl(&mut self, slot: StackSlot) {
        self.emit_store_to_sp("x0", slot.0, "str");
        self.emit_store_to_sp("x1", slot.0 + 8, "str");
    }

    pub(super) fn emit_load_pair_from_slot_impl(&mut self, slot: StackSlot) {
        self.emit_load_from_sp("x0", slot.0, "ldr");
        self.emit_load_from_sp("x1", slot.0 + 8, "ldr");
    }

    pub(super) fn emit_save_acc_pair_impl(&mut self) {
        self.state.emit("    mov x2, x0");
        self.state.emit("    mov x3, x1");
    }

    pub(super) fn emit_store_pair_indirect_impl(&mut self) {
        self.state.emit("    str x2, [x9]");
        self.state.emit("    str x3, [x9, #8]");
    }

    pub(super) fn emit_load_pair_indirect_impl(&mut self) {
        self.state.emit("    ldr x0, [x9]");
        self.state.emit("    ldr x1, [x9, #8]");
    }

    pub(super) fn emit_i128_neg_impl(&mut self) {
        self.state.emit("    mvn x0, x0");
        self.state.emit("    mvn x1, x1");
        self.state.emit("    adds x0, x0, #1");
        self.state.emit("    adc x1, x1, xzr");
    }

    pub(super) fn emit_i128_not_impl(&mut self) {
        self.state.emit("    mvn x0, x0");
        self.state.emit("    mvn x1, x1");
    }

    pub(super) fn emit_sign_extend_acc_high_impl(&mut self) {
        self.state.emit("    asr x1, x0, #63");
    }

    pub(super) fn emit_zero_acc_high_impl(&mut self) {
        self.state.emit("    mov x1, #0");
    }

    pub(super) fn emit_i128_prep_binop_impl(&mut self, lhs: &Operand, rhs: &Operand) {
        self.prep_i128_binop(lhs, rhs);
    }

    pub(super) fn emit_i128_add_impl(&mut self) {
        self.state.emit("    adds x0, x2, x4");
        self.state.emit("    adc x1, x3, x5");
    }

    pub(super) fn emit_i128_sub_impl(&mut self) {
        self.state.emit("    subs x0, x2, x4");
        self.state.emit("    sbc x1, x3, x5");
    }

    pub(super) fn emit_i128_mul_impl(&mut self) {
        self.state.emit("    mul x0, x2, x4");
        self.state.emit("    umulh x1, x2, x4");
        self.state.emit("    madd x1, x3, x4, x1");
        self.state.emit("    madd x1, x2, x5, x1");
    }

    pub(super) fn emit_i128_and_impl(&mut self) {
        self.state.emit("    and x0, x2, x4");
        self.state.emit("    and x1, x3, x5");
    }

    pub(super) fn emit_i128_or_impl(&mut self) {
        self.state.emit("    orr x0, x2, x4");
        self.state.emit("    orr x1, x3, x5");
    }

    pub(super) fn emit_i128_xor_impl(&mut self) {
        self.state.emit("    eor x0, x2, x4");
        self.state.emit("    eor x1, x3, x5");
    }

    pub(super) fn emit_i128_shl_impl(&mut self) {
        let lbl = self.state.fresh_label("shl128");
        let done = self.state.fresh_label("shl128_done");
        let noop = self.state.fresh_label("shl128_noop");
        self.state.emit("    and x4, x4, #127");
        self.state.emit_fmt(format_args!("    cbz x4, {}", noop));
        self.state.emit("    cmp x4, #64");
        self.state.emit_fmt(format_args!("    b.ge {}", lbl));
        self.state.emit("    lsl x1, x3, x4");
        self.state.emit("    mov x5, #64");
        self.state.emit("    sub x5, x5, x4");
        self.state.emit("    lsr x6, x2, x5");
        self.state.emit("    orr x1, x1, x6");
        self.state.emit("    lsl x0, x2, x4");
        self.state.emit_fmt(format_args!("    b {}", done));
        self.state.emit_fmt(format_args!("{}:", lbl));
        self.state.emit("    sub x4, x4, #64");
        self.state.emit("    lsl x1, x2, x4");
        self.state.emit("    mov x0, #0");
        self.state.emit_fmt(format_args!("    b {}", done));
        self.state.emit_fmt(format_args!("{}:", noop));
        self.state.emit("    mov x0, x2");
        self.state.emit("    mov x1, x3");
        self.state.emit_fmt(format_args!("{}:", done));
    }

    pub(super) fn emit_i128_lshr_impl(&mut self) {
        let lbl = self.state.fresh_label("lshr128");
        let done = self.state.fresh_label("lshr128_done");
        let noop = self.state.fresh_label("lshr128_noop");
        self.state.emit("    and x4, x4, #127");
        self.state.emit_fmt(format_args!("    cbz x4, {}", noop));
        self.state.emit("    cmp x4, #64");
        self.state.emit_fmt(format_args!("    b.ge {}", lbl));
        self.state.emit("    lsr x0, x2, x4");
        self.state.emit("    mov x5, #64");
        self.state.emit("    sub x5, x5, x4");
        self.state.emit("    lsl x6, x3, x5");
        self.state.emit("    orr x0, x0, x6");
        self.state.emit("    lsr x1, x3, x4");
        self.state.emit_fmt(format_args!("    b {}", done));
        self.state.emit_fmt(format_args!("{}:", lbl));
        self.state.emit("    sub x4, x4, #64");
        self.state.emit("    lsr x0, x3, x4");
        self.state.emit("    mov x1, #0");
        self.state.emit_fmt(format_args!("    b {}", done));
        self.state.emit_fmt(format_args!("{}:", noop));
        self.state.emit("    mov x0, x2");
        self.state.emit("    mov x1, x3");
        self.state.emit_fmt(format_args!("{}:", done));
    }

    pub(super) fn emit_i128_ashr_impl(&mut self) {
        let lbl = self.state.fresh_label("ashr128");
        let done = self.state.fresh_label("ashr128_done");
        let noop = self.state.fresh_label("ashr128_noop");
        self.state.emit("    and x4, x4, #127");
        self.state.emit_fmt(format_args!("    cbz x4, {}", noop));
        self.state.emit("    cmp x4, #64");
        self.state.emit_fmt(format_args!("    b.ge {}", lbl));
        self.state.emit("    lsr x0, x2, x4");
        self.state.emit("    mov x5, #64");
        self.state.emit("    sub x5, x5, x4");
        self.state.emit("    lsl x6, x3, x5");
        self.state.emit("    orr x0, x0, x6");
        self.state.emit("    asr x1, x3, x4");
        self.state.emit_fmt(format_args!("    b {}", done));
        self.state.emit_fmt(format_args!("{}:", lbl));
        self.state.emit("    sub x4, x4, #64");
        self.state.emit("    asr x0, x3, x4");
        self.state.emit("    asr x1, x3, #63");
        self.state.emit_fmt(format_args!("    b {}", done));
        self.state.emit_fmt(format_args!("{}:", noop));
        self.state.emit("    mov x0, x2");
        self.state.emit("    mov x1, x3");
        self.state.emit_fmt(format_args!("{}:", done));
    }

    pub(super) fn emit_i128_prep_shift_lhs_impl(&mut self, lhs: &Operand) {
        self.operand_to_x0_x1(lhs);
        self.state.emit("    mov x2, x0");
        self.state.emit("    mov x3, x1");
    }

    pub(super) fn emit_i128_shl_const_impl(&mut self, amount: u32) {
        let amount = amount & 127;
        if amount == 0 {
            self.state.emit("    mov x0, x2");
            self.state.emit("    mov x1, x3");
        } else if amount == 64 {
            self.state.emit("    mov x1, x2");
            self.state.emit("    mov x0, #0");
        } else if amount > 64 {
            self.state
                .emit_fmt(format_args!("    lsl x1, x2, #{}", amount - 64));
            self.state.emit("    mov x0, #0");
        } else {
            self.state
                .emit_fmt(format_args!("    lsl x1, x3, #{}", amount));
            self.state
                .emit_fmt(format_args!("    orr x1, x1, x2, lsr #{}", 64 - amount));
            self.state
                .emit_fmt(format_args!("    lsl x0, x2, #{}", amount));
        }
    }

    pub(super) fn emit_i128_lshr_const_impl(&mut self, amount: u32) {
        let amount = amount & 127;
        if amount == 0 {
            self.state.emit("    mov x0, x2");
            self.state.emit("    mov x1, x3");
        } else if amount == 64 {
            self.state.emit("    mov x0, x3");
            self.state.emit("    mov x1, #0");
        } else if amount > 64 {
            self.state
                .emit_fmt(format_args!("    lsr x0, x3, #{}", amount - 64));
            self.state.emit("    mov x1, #0");
        } else {
            self.state
                .emit_fmt(format_args!("    lsr x0, x2, #{}", amount));
            self.state
                .emit_fmt(format_args!("    orr x0, x0, x3, lsl #{}", 64 - amount));
            self.state
                .emit_fmt(format_args!("    lsr x1, x3, #{}", amount));
        }
    }

    pub(super) fn emit_i128_ashr_const_impl(&mut self, amount: u32) {
        let amount = amount & 127;
        if amount == 0 {
            self.state.emit("    mov x0, x2");
            self.state.emit("    mov x1, x3");
        } else if amount == 64 {
            self.state.emit("    mov x0, x3");
            self.state.emit("    asr x1, x3, #63");
        } else if amount > 64 {
            self.state
                .emit_fmt(format_args!("    asr x0, x3, #{}", amount - 64));
            self.state.emit("    asr x1, x3, #63");
        } else {
            self.state
                .emit_fmt(format_args!("    lsr x0, x2, #{}", amount));
            self.state
                .emit_fmt(format_args!("    orr x0, x0, x3, lsl #{}", 64 - amount));
            self.state
                .emit_fmt(format_args!("    asr x1, x3, #{}", amount));
        }
    }

    pub(super) fn emit_i128_divrem_call_impl(
        &mut self,
        func_name: &str,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        self.operand_to_x0_x1(lhs);
        self.state.emit("    mov x2, x0");
        self.state.emit("    mov x3, x1");
        self.operand_to_x0_x1(rhs);
        self.state.emit("    mov x4, x0");
        self.state.emit("    mov x5, x1");
        self.state.emit("    mov x0, x2");
        self.state.emit("    mov x1, x3");
        self.state.emit("    mov x2, x4");
        self.state.emit("    mov x3, x5");
        self.state.emit_fmt(format_args!("    bl {}", func_name));
    }

    pub(super) fn emit_i128_store_result_impl(&mut self, dest: &Value) {
        self.store_x0_x1_to(dest);
    }

    pub(super) fn emit_float_to_i128_call_impl(
        &mut self,
        src: &Operand,
        to_signed: bool,
        from_ty: IrType,
    ) {
        self.operand_to_x0(src);
        if from_ty == IrType::F32 {
            self.state.emit("    fmov s0, w0");
        } else {
            self.state.emit("    fmov d0, x0");
        }
        let func_name = match (to_signed, from_ty) {
            (true, IrType::F64) => "__fixdfti",
            (true, IrType::F32) => "__fixsfti",
            (false, IrType::F64) => "__fixunsdfti",
            (false, IrType::F32) => "__fixunssfti",
            _ => panic!("unsupported float-to-i128 conversion: {:?}", from_ty),
        };
        self.state.emit_fmt(format_args!("    bl {}", func_name));
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_i128_to_float_call_impl(
        &mut self,
        src: &Operand,
        from_signed: bool,
        to_ty: IrType,
    ) {
        self.operand_to_x0_x1(src);
        let func_name = match (from_signed, to_ty) {
            (true, IrType::F64) => "__floattidf",
            (true, IrType::F32) => "__floattisf",
            (false, IrType::F64) => "__floatuntidf",
            (false, IrType::F32) => "__floatuntisf",
            _ => panic!("unsupported i128-to-float conversion: {:?}", to_ty),
        };
        self.state.emit_fmt(format_args!("    bl {}", func_name));
        self.state.reg_cache.invalidate_all();
        if to_ty == IrType::F32 {
            self.state.emit("    fmov w0, s0");
        } else {
            self.state.emit("    fmov x0, d0");
        }
    }

    pub(super) fn emit_i128_cmp_eq_impl(&mut self, is_ne: bool) {
        self.state.emit("    eor x0, x2, x4");
        self.state.emit("    eor x1, x3, x5");
        self.state.emit("    orr x0, x0, x1");
        self.state.emit("    cmp x0, #0");
        if is_ne {
            self.state.emit("    cset x0, ne");
        } else {
            self.state.emit("    cset x0, eq");
        }
    }

    pub(super) fn emit_i128_cmp_ordered_impl(&mut self, op: IrCmpOp) {
        let done = self.state.fresh_label("cmp128_done");
        self.state.emit("    cmp x3, x5");
        let (hi_cond, lo_cond) = match op {
            IrCmpOp::Slt | IrCmpOp::Sle => ("lt", if op == IrCmpOp::Slt { "lo" } else { "ls" }),
            IrCmpOp::Sgt | IrCmpOp::Sge => ("gt", if op == IrCmpOp::Sgt { "hi" } else { "hs" }),
            IrCmpOp::Ult | IrCmpOp::Ule => ("lo", if op == IrCmpOp::Ult { "lo" } else { "ls" }),
            IrCmpOp::Ugt | IrCmpOp::Uge => ("hi", if op == IrCmpOp::Ugt { "hi" } else { "hs" }),
            _ => unreachable!("i128 ordered cmp got equality op: {:?}", op),
        };
        self.state
            .emit_fmt(format_args!("    cset x0, {}", hi_cond));
        self.state.emit_fmt(format_args!("    b.ne {}", done));
        self.state.emit("    cmp x2, x4");
        self.state
            .emit_fmt(format_args!("    cset x0, {}", lo_cond));
        self.state.emit_fmt(format_args!("{}:", done));
    }

    pub(super) fn emit_i128_cmp_store_result_impl(&mut self, dest: &Value) {
        self.store_x0_to(dest);
    }
}
