//! RiscvCodegen: 128-bit integer operations.

use super::emit::RiscvCodegen;
use crate::backend::state::StackSlot;
use crate::common::types::IrType;
use crate::ir::reexports::{IrCmpOp, Operand, Value};

impl RiscvCodegen {
    // ---- i128 acc pair primitives ----

    pub(super) fn emit_load_acc_pair_impl(&mut self, op: &Operand) {
        self.operand_to_t0_t1(op);
    }

    pub(super) fn emit_store_acc_pair_impl(&mut self, dest: &Value) {
        self.store_t0_t1_to(dest);
    }

    pub(super) fn emit_store_pair_to_slot_impl(&mut self, slot: StackSlot) {
        self.emit_store_to_s0("t0", slot.0, "sd");
        self.emit_store_to_s0("t1", slot.0 + 8, "sd");
    }

    pub(super) fn emit_load_pair_from_slot_impl(&mut self, slot: StackSlot) {
        self.emit_load_from_s0("t0", slot.0, "ld");
        self.emit_load_from_s0("t1", slot.0 + 8, "ld");
    }

    pub(super) fn emit_save_acc_pair_impl(&mut self) {
        self.state.emit("    mv t3, t0");
        self.state.emit("    mv t4, t1");
    }

    pub(super) fn emit_store_pair_indirect_impl(&mut self) {
        self.state.emit("    sd t3, 0(t5)");
        self.state.emit("    sd t4, 8(t5)");
    }

    pub(super) fn emit_load_pair_indirect_impl(&mut self) {
        self.state.emit("    ld t0, 0(t5)");
        self.state.emit("    ld t1, 8(t5)");
    }

    pub(super) fn emit_i128_neg_impl(&mut self) {
        self.state.emit("    not t0, t0");
        self.state.emit("    not t1, t1");
        self.state.emit("    addi t0, t0, 1");
        self.state.emit("    seqz t2, t0");
        self.state.emit("    add t1, t1, t2");
    }

    pub(super) fn emit_i128_not_impl(&mut self) {
        self.state.emit("    not t0, t0");
        self.state.emit("    not t1, t1");
    }

    pub(super) fn emit_sign_extend_acc_high_impl(&mut self) {
        self.state.emit("    srai t1, t0, 63");
    }

    pub(super) fn emit_zero_acc_high_impl(&mut self) {
        self.state.emit("    li t1, 0");
    }

    // ---- i128 binop primitives ----

    pub(super) fn emit_i128_prep_binop_impl(&mut self, lhs: &Operand, rhs: &Operand) {
        self.prep_i128_binop(lhs, rhs);
    }

    pub(super) fn emit_i128_add_impl(&mut self) {
        self.state.emit("    add t0, t3, t5");
        self.state.emit("    sltu t2, t0, t3");
        self.state.emit("    add t1, t4, t6");
        self.state.emit("    add t1, t1, t2");
    }

    pub(super) fn emit_i128_sub_impl(&mut self) {
        self.state.emit("    sltu t2, t3, t5");
        self.state.emit("    sub t0, t3, t5");
        self.state.emit("    sub t1, t4, t6");
        self.state.emit("    sub t1, t1, t2");
    }

    pub(super) fn emit_i128_mul_impl(&mut self) {
        self.state.emit("    mul t0, t3, t5");
        self.state.emit("    mulhu t1, t3, t5");
        self.state.emit("    mul t2, t4, t5");
        self.state.emit("    add t1, t1, t2");
        self.state.emit("    mul t2, t3, t6");
        self.state.emit("    add t1, t1, t2");
    }

    pub(super) fn emit_i128_and_impl(&mut self) {
        self.state.emit("    and t0, t3, t5");
        self.state.emit("    and t1, t4, t6");
    }

    pub(super) fn emit_i128_or_impl(&mut self) {
        self.state.emit("    or t0, t3, t5");
        self.state.emit("    or t1, t4, t6");
    }

    pub(super) fn emit_i128_xor_impl(&mut self) {
        self.state.emit("    xor t0, t3, t5");
        self.state.emit("    xor t1, t4, t6");
    }

    pub(super) fn emit_i128_shl_impl(&mut self) {
        let lbl = self.state.fresh_label("shl128");
        let done = self.state.fresh_label("shl128_done");
        let noop = self.state.fresh_label("shl128_noop");
        self.state.emit("    andi t5, t5, 127");
        self.state.emit_fmt(format_args!("    beqz t5, {}", noop));
        self.state.emit("    li t2, 64");
        self.state.emit_fmt(format_args!("    bge t5, t2, {}", lbl));
        self.state.emit("    sll t1, t4, t5");
        self.state.emit("    sub t2, t2, t5");
        self.state.emit("    srl t6, t3, t2");
        self.state.emit("    or t1, t1, t6");
        self.state.emit("    sll t0, t3, t5");
        self.state.emit_fmt(format_args!("    j {}", done));
        self.state.emit_fmt(format_args!("{}:", lbl));
        self.state.emit("    li t2, 64");
        self.state.emit("    sub t5, t5, t2");
        self.state.emit("    sll t1, t3, t5");
        self.state.emit("    li t0, 0");
        self.state.emit_fmt(format_args!("    j {}", done));
        self.state.emit_fmt(format_args!("{}:", noop));
        self.state.emit("    mv t0, t3");
        self.state.emit("    mv t1, t4");
        self.state.emit_fmt(format_args!("{}:", done));
    }

    pub(super) fn emit_i128_lshr_impl(&mut self) {
        let lbl = self.state.fresh_label("lshr128");
        let done = self.state.fresh_label("lshr128_done");
        let noop = self.state.fresh_label("lshr128_noop");
        self.state.emit("    andi t5, t5, 127");
        self.state.emit_fmt(format_args!("    beqz t5, {}", noop));
        self.state.emit("    li t2, 64");
        self.state.emit_fmt(format_args!("    bge t5, t2, {}", lbl));
        self.state.emit("    srl t0, t3, t5");
        self.state.emit("    sub t2, t2, t5");
        self.state.emit("    sll t6, t4, t2");
        self.state.emit("    or t0, t0, t6");
        self.state.emit("    srl t1, t4, t5");
        self.state.emit_fmt(format_args!("    j {}", done));
        self.state.emit_fmt(format_args!("{}:", lbl));
        self.state.emit("    li t2, 64");
        self.state.emit("    sub t5, t5, t2");
        self.state.emit("    srl t0, t4, t5");
        self.state.emit("    li t1, 0");
        self.state.emit_fmt(format_args!("    j {}", done));
        self.state.emit_fmt(format_args!("{}:", noop));
        self.state.emit("    mv t0, t3");
        self.state.emit("    mv t1, t4");
        self.state.emit_fmt(format_args!("{}:", done));
    }

    pub(super) fn emit_i128_ashr_impl(&mut self) {
        let lbl = self.state.fresh_label("ashr128");
        let done = self.state.fresh_label("ashr128_done");
        let noop = self.state.fresh_label("ashr128_noop");
        self.state.emit("    andi t5, t5, 127");
        self.state.emit_fmt(format_args!("    beqz t5, {}", noop));
        self.state.emit("    li t2, 64");
        self.state.emit_fmt(format_args!("    bge t5, t2, {}", lbl));
        self.state.emit("    srl t0, t3, t5");
        self.state.emit("    sub t2, t2, t5");
        self.state.emit("    sll t6, t4, t2");
        self.state.emit("    or t0, t0, t6");
        self.state.emit("    sra t1, t4, t5");
        self.state.emit_fmt(format_args!("    j {}", done));
        self.state.emit_fmt(format_args!("{}:", lbl));
        self.state.emit("    li t2, 64");
        self.state.emit("    sub t5, t5, t2");
        self.state.emit("    sra t0, t4, t5");
        self.state.emit("    srai t1, t4, 63");
        self.state.emit_fmt(format_args!("    j {}", done));
        self.state.emit_fmt(format_args!("{}:", noop));
        self.state.emit("    mv t0, t3");
        self.state.emit("    mv t1, t4");
        self.state.emit_fmt(format_args!("{}:", done));
    }

    pub(super) fn emit_i128_prep_shift_lhs_impl(&mut self, lhs: &Operand) {
        self.operand_to_t0_t1(lhs);
        self.state.emit("    mv t3, t0");
        self.state.emit("    mv t4, t1");
    }

    pub(super) fn emit_i128_shl_const_impl(&mut self, amount: u32) {
        let amount = amount & 127;
        if amount == 0 {
            self.state.emit("    mv t0, t3");
            self.state.emit("    mv t1, t4");
        } else if amount == 64 {
            self.state.emit("    mv t1, t3");
            self.state.emit("    li t0, 0");
        } else if amount > 64 {
            self.state
                .emit_fmt(format_args!("    slli t1, t3, {}", amount - 64));
            self.state.emit("    li t0, 0");
        } else {
            self.state
                .emit_fmt(format_args!("    slli t1, t4, {}", amount));
            self.state
                .emit_fmt(format_args!("    srli t2, t3, {}", 64 - amount));
            self.state.emit("    or t1, t1, t2");
            self.state
                .emit_fmt(format_args!("    slli t0, t3, {}", amount));
        }
    }

    pub(super) fn emit_i128_lshr_const_impl(&mut self, amount: u32) {
        let amount = amount & 127;
        if amount == 0 {
            self.state.emit("    mv t0, t3");
            self.state.emit("    mv t1, t4");
        } else if amount == 64 {
            self.state.emit("    mv t0, t4");
            self.state.emit("    li t1, 0");
        } else if amount > 64 {
            self.state
                .emit_fmt(format_args!("    srli t0, t4, {}", amount - 64));
            self.state.emit("    li t1, 0");
        } else {
            self.state
                .emit_fmt(format_args!("    srli t0, t3, {}", amount));
            self.state
                .emit_fmt(format_args!("    slli t2, t4, {}", 64 - amount));
            self.state.emit("    or t0, t0, t2");
            self.state
                .emit_fmt(format_args!("    srli t1, t4, {}", amount));
        }
    }

    pub(super) fn emit_i128_ashr_const_impl(&mut self, amount: u32) {
        let amount = amount & 127;
        if amount == 0 {
            self.state.emit("    mv t0, t3");
            self.state.emit("    mv t1, t4");
        } else if amount == 64 {
            self.state.emit("    mv t0, t4");
            self.state.emit("    srai t1, t4, 63");
        } else if amount > 64 {
            self.state
                .emit_fmt(format_args!("    srai t0, t4, {}", amount - 64));
            self.state.emit("    srai t1, t4, 63");
        } else {
            self.state
                .emit_fmt(format_args!("    srli t0, t3, {}", amount));
            self.state
                .emit_fmt(format_args!("    slli t2, t4, {}", 64 - amount));
            self.state.emit("    or t0, t0, t2");
            self.state
                .emit_fmt(format_args!("    srai t1, t4, {}", amount));
        }
    }

    pub(super) fn emit_i128_divrem_call_impl(
        &mut self,
        func_name: &str,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        self.operand_to_t0_t1(lhs);
        self.state.emit("    mv a0, t0");
        self.state.emit("    mv a1, t1");
        self.operand_to_t0_t1(rhs);
        self.state.emit("    mv a2, t0");
        self.state.emit("    mv a3, t1");
        self.state.emit_fmt(format_args!("    call {}", func_name));
        self.state.emit("    mv t0, a0");
        self.state.emit("    mv t1, a1");
    }

    pub(super) fn emit_i128_store_result_impl(&mut self, dest: &Value) {
        self.store_t0_t1_to(dest);
    }

    pub(super) fn emit_i128_to_float_call_impl(
        &mut self,
        src: &Operand,
        from_signed: bool,
        to_ty: IrType,
    ) {
        self.operand_to_t0_t1(src);
        self.state.emit("    mv a0, t0");
        self.state.emit("    mv a1, t1");
        let func_name = match (from_signed, to_ty) {
            (true, IrType::F64) => "__floattidf",
            (true, IrType::F32) => "__floattisf",
            (false, IrType::F64) => "__floatuntidf",
            (false, IrType::F32) => "__floatuntisf",
            _ => panic!("unsupported i128-to-float conversion: {:?}", to_ty),
        };
        self.state.emit_fmt(format_args!("    call {}", func_name));
        self.state.reg_cache.invalidate_all();
        if to_ty == IrType::F32 {
            self.state.emit("    fmv.x.w t0, fa0");
        } else {
            self.state.emit("    fmv.x.d t0, fa0");
        }
    }

    pub(super) fn emit_float_to_i128_call_impl(
        &mut self,
        src: &Operand,
        to_signed: bool,
        from_ty: IrType,
    ) {
        self.operand_to_t0(src);
        if from_ty == IrType::F32 {
            self.state.emit("    fmv.w.x fa0, t0");
        } else {
            self.state.emit("    fmv.d.x fa0, t0");
        }
        let func_name = match (to_signed, from_ty) {
            (true, IrType::F64) => "__fixdfti",
            (true, IrType::F32) => "__fixsfti",
            (false, IrType::F64) => "__fixunsdfti",
            (false, IrType::F32) => "__fixunssfti",
            _ => panic!("unsupported float-to-i128 conversion: {:?}", from_ty),
        };
        self.state.emit_fmt(format_args!("    call {}", func_name));
        self.state.reg_cache.invalidate_all();
        self.state.emit("    mv t0, a0");
        self.state.emit("    mv t1, a1");
    }

    // ---- i128 cmp primitives ----

    pub(super) fn emit_i128_cmp_eq_impl(&mut self, is_ne: bool) {
        self.state.emit("    xor t0, t3, t5");
        self.state.emit("    xor t1, t4, t6");
        self.state.emit("    or t0, t0, t1");
        if is_ne {
            self.state.emit("    snez t0, t0");
        } else {
            self.state.emit("    seqz t0, t0");
        }
    }

    pub(super) fn emit_i128_cmp_ordered_impl(&mut self, op: IrCmpOp) {
        let hi_differ = self.state.fresh_label("cmp128_hi_diff");
        let hi_equal = self.state.fresh_label("cmp128_hi_eq");
        let done = self.state.fresh_label("cmp128_done");
        self.state
            .emit_fmt(format_args!("    bne t4, t6, {}", hi_differ));
        self.state.emit_fmt(format_args!("    j {}", hi_equal));
        self.state.emit_fmt(format_args!("{}:", hi_differ));
        match op {
            IrCmpOp::Slt | IrCmpOp::Sle => self.state.emit("    slt t0, t4, t6"),
            IrCmpOp::Sgt | IrCmpOp::Sge => self.state.emit("    slt t0, t6, t4"),
            IrCmpOp::Ult | IrCmpOp::Ule => self.state.emit("    sltu t0, t4, t6"),
            IrCmpOp::Ugt | IrCmpOp::Uge => self.state.emit("    sltu t0, t6, t4"),
            _ => unreachable!("i128 ordered cmp (high word) got equality op: {:?}", op),
        }
        self.state.emit_fmt(format_args!("    j {}", done));
        self.state.emit_fmt(format_args!("{}:", hi_equal));
        match op {
            IrCmpOp::Slt | IrCmpOp::Ult => self.state.emit("    sltu t0, t3, t5"),
            IrCmpOp::Sle | IrCmpOp::Ule => {
                self.state.emit("    sltu t0, t5, t3");
                self.state.emit("    xori t0, t0, 1");
            }
            IrCmpOp::Sgt | IrCmpOp::Ugt => self.state.emit("    sltu t0, t5, t3"),
            IrCmpOp::Sge | IrCmpOp::Uge => {
                self.state.emit("    sltu t0, t3, t5");
                self.state.emit("    xori t0, t0, 1");
            }
            _ => unreachable!("i128 ordered cmp (low word) got equality op: {:?}", op),
        }
        self.state.emit_fmt(format_args!("{}:", done));
    }

    pub(super) fn emit_i128_cmp_store_result_impl(&mut self, dest: &Value) {
        self.store_t0_to(dest);
    }
}
