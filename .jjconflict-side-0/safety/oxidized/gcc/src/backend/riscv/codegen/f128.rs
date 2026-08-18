//! RISC-V F128 (quad-precision / long double) soft-float helpers.
//!
//! IEEE 754 binary128 operations via compiler-rt/libgcc soft-float libcalls.
//! RISC-V LP64D ABI: f128 passed in GP register pairs (a0:a1, a2:a3).
//!
//! This file implements the `F128SoftFloat` trait for RISC-V, providing the
//! arch-specific primitives (GP register pair representation, instruction
//! mnemonics, S0-relative addressing). The shared orchestration logic lives
//! in `backend/f128_softfloat.rs`.

use super::emit::{callee_saved_name, RiscvCodegen};
use crate::backend::f128_softfloat::F128SoftFloat;
use crate::backend::state::{SlotAddr, StackSlot};
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{Operand, Value};

impl F128SoftFloat for RiscvCodegen {
    fn state(&mut self) -> &mut crate::backend::state::CodegenState {
        &mut self.state
    }

    fn f128_get_slot(&self, val_id: u32) -> Option<StackSlot> {
        self.state.get_slot(val_id)
    }

    fn f128_get_source(&self, val_id: u32) -> Option<(u32, i64, bool)> {
        self.state.get_f128_source(val_id)
    }

    fn f128_resolve_slot_addr(&self, val_id: u32) -> Option<SlotAddr> {
        self.state.resolve_slot_addr(val_id)
    }

    fn f128_load_const_to_arg1(&mut self, lo: u64, hi: u64) {
        self.state
            .emit_fmt(format_args!("    li a0, {}", lo as i64));
        self.state
            .emit_fmt(format_args!("    li a1, {}", hi as i64));
    }

    fn f128_load_16b_from_addr_reg_to_arg1(&mut self) {
        // t5 holds the address; load 16 bytes into a0:a1
        self.state.emit("    ld a0, 0(t5)");
        self.state.emit("    ld a1, 8(t5)");
    }

    fn f128_load_from_frame_offset_to_arg1(&mut self, offset: i64) {
        self.emit_load_from_s0("a0", offset, "ld");
        self.emit_load_from_s0("a1", offset + 8, "ld");
    }

    fn f128_load_ptr_to_addr_reg(&mut self, slot: StackSlot, val_id: u32) {
        self.emit_load_ptr_from_slot(slot, val_id);
    }

    fn f128_add_offset_to_addr_reg(&mut self, offset: i64) {
        self.emit_add_offset_to_addr_reg(offset);
    }

    fn f128_alloca_aligned_addr(&mut self, slot: StackSlot, val_id: u32) {
        self.emit_alloca_aligned_addr(slot, val_id);
    }

    fn f128_load_operand_and_extend(&mut self, op: &Operand) {
        self.operand_to_t0(op);
        self.state.emit("    fmv.d.x fa0, t0");
        self.state.emit("    call __extenddftf2");
        // __extenddftf2 is a function call that clobbers caller-saved regs
        // (including t0). Invalidate the cache so subsequent operand loads
        // for the same value won't skip the reload.
        self.state.reg_cache.invalidate_all();
    }

    fn f128_move_arg1_to_arg2(&mut self) {
        // Move a0:a1 -> a2:a3 (GP register pair move)
        self.state.emit("    mv a2, a0");
        self.state.emit("    mv a3, a1");
    }

    fn f128_save_arg1_to_sp(&mut self) {
        self.state.emit("    sd a0, 0(sp)");
        self.state.emit("    sd a1, 8(sp)");
    }

    fn f128_reload_arg1_from_sp(&mut self) {
        self.state.emit("    ld a0, 0(sp)");
        self.state.emit("    ld a1, 8(sp)");
    }

    fn f128_alloc_temp_16(&mut self) {
        self.emit_addi_sp(-16);
    }

    fn f128_free_temp_16(&mut self) {
        self.emit_addi_sp(16);
    }

    fn f128_call(&mut self, name: &str) {
        self.state.emit_fmt(format_args!("    call {}", name));
    }

    fn f128_truncate_result_to_acc(&mut self) {
        self.state.emit("    call __trunctfdf2");
        self.state.emit("    fmv.x.d t0, fa0");
    }

    fn f128_store_const_halves_to_slot(&mut self, lo: u64, hi: u64, slot: StackSlot) {
        self.state
            .emit_fmt(format_args!("    li t0, {}", lo as i64));
        self.emit_store_to_s0("t0", slot.0, "sd");
        self.state
            .emit_fmt(format_args!("    li t0, {}", hi as i64));
        self.emit_store_to_s0("t0", slot.0 + 8, "sd");
    }

    fn f128_store_arg1_to_slot(&mut self, slot: StackSlot) {
        // Store a0:a1 (f128 in GP pair) to slot
        self.emit_store_to_s0("a0", slot.0, "sd");
        self.emit_store_to_s0("a1", slot.0 + 8, "sd");
    }

    fn f128_copy_slot_to_slot(&mut self, src_offset: i64, dest_slot: StackSlot) {
        // Load 16 bytes from source into t0 (two loads), store to dest
        self.emit_load_from_s0("t0", src_offset, "ld");
        self.emit_store_to_s0("t0", dest_slot.0, "sd");
        self.emit_load_from_s0("t0", src_offset + 8, "ld");
        self.emit_store_to_s0("t0", dest_slot.0 + 8, "sd");
    }

    fn f128_copy_addr_reg_to_slot(&mut self, dest_slot: StackSlot) {
        // Load from t5 (addr reg), store to slot
        self.state.emit("    ld t0, 0(t5)");
        self.emit_store_to_s0("t0", dest_slot.0, "sd");
        self.state.emit("    ld t0, 8(t5)");
        self.emit_store_to_s0("t0", dest_slot.0 + 8, "sd");
    }

    fn f128_store_const_halves_to_addr(&mut self, lo: u64, hi: u64) {
        // t5 holds dest address
        self.state
            .emit_fmt(format_args!("    li t0, {}", lo as i64));
        self.state.emit("    sd t0, 0(t5)");
        self.state
            .emit_fmt(format_args!("    li t0, {}", hi as i64));
        self.state.emit("    sd t0, 8(t5)");
    }

    fn f128_save_addr_reg(&mut self) {
        // Save t5 to t3
        self.state.emit("    mv t3, t5");
    }

    fn f128_copy_slot_to_saved_addr(&mut self, src_offset: i64) {
        // Load 16 bytes from source slot, store to saved addr (t3)
        self.emit_load_from_s0("t0", src_offset, "ld");
        self.state.emit("    sd t0, 0(t3)");
        self.emit_load_from_s0("t0", src_offset + 8, "ld");
        self.state.emit("    sd t0, 8(t3)");
    }

    fn f128_copy_addr_reg_to_saved_addr(&mut self) {
        // Load 16 bytes from t5, store to t3
        self.state.emit("    ld t0, 0(t5)");
        self.state.emit("    sd t0, 0(t3)");
        self.state.emit("    ld t0, 8(t5)");
        self.state.emit("    sd t0, 8(t3)");
    }

    fn f128_store_arg1_to_saved_addr(&mut self) {
        // Store a0:a1 (f128 in arg1) to saved addr (t3)
        self.state.emit("    sd a0, 0(t3)");
        self.state.emit("    sd a1, 8(t3)");
    }

    fn f128_flip_sign_bit(&mut self) {
        // Flip bit 63 of a1 (which is bit 127 of the IEEE f128 representation)
        self.state.emit("    li t0, 1");
        self.state.emit("    slli t0, t0, 63");
        self.state.emit("    xor a1, a1, t0");
    }

    fn f128_cmp_result_to_bool(&mut self, kind: crate::backend::cast::F128CmpKind) {
        use crate::backend::cast::F128CmpKind;
        match kind {
            F128CmpKind::Eq => self.state.emit("    seqz t0, a0"),
            F128CmpKind::Ne => self.state.emit("    snez t0, a0"),
            F128CmpKind::Lt => self.state.emit("    slti t0, a0, 0"),
            F128CmpKind::Le => {
                // t0 = (a0 <= 0) = (a0 < 1)
                self.state.emit("    slti t0, a0, 1");
            }
            F128CmpKind::Gt => {
                // t0 = (a0 > 0): 0 < a0
                self.state.emit("    li t0, 0");
                self.state.emit("    slt t0, t0, a0");
            }
            F128CmpKind::Ge => {
                // t0 = (a0 >= 0) = !(a0 < 0)
                self.state.emit("    slti t0, a0, 0");
                self.state.emit("    xori t0, t0, 1");
            }
        }
    }

    fn f128_store_acc_to_dest(&mut self, dest: &Value) {
        self.store_t0_to(dest);
    }

    fn f128_track_self(&mut self, dest_id: u32) {
        self.state.track_f128_self(dest_id);
    }

    fn f128_set_acc_cache(&mut self, dest_id: u32) {
        self.state.reg_cache.set_acc(dest_id, false);
    }

    fn f128_set_dyn_alloca(&mut self, _val: bool) -> bool {
        // RISC-V uses s0 (frame pointer) for all slot addressing, so the
        // dyn_alloca flag doesn't affect addressing. Return false as no-op.
        false
    }

    fn f128_move_callee_reg_to_addr_reg(&mut self, val_id: u32) -> bool {
        if let Some(&reg) = self.reg_assignments.get(&val_id) {
            let reg_name = callee_saved_name(reg);
            self.state.emit_fmt(format_args!("    mv t5, {}", reg_name));
            true
        } else {
            false
        }
    }

    // f128_move_aligned_to_addr_reg: RISC-V uses t5 for both alloca-aligned
    // addr and F128 addr register, so the default no-op is correct.

    fn f128_load_indirect_ptr_to_addr_reg(&mut self, slot: StackSlot, val_id: u32) {
        self.emit_load_ptr_from_slot(slot, val_id);
    }

    fn f128_load_from_addr_reg_to_acc(&mut self, dest: &Value) {
        // Load 16 bytes from t5, convert to f64 via __trunctfdf2, store to dest
        self.state.emit("    ld a0, 0(t5)");
        self.state.emit("    ld a1, 8(t5)");
        self.state.emit("    call __trunctfdf2");
        self.state.emit("    fmv.x.d t0, fa0");
        self.state.reg_cache.invalidate_all();
        self.store_t0_to(dest);
    }

    fn f128_load_from_direct_slot_to_acc(&mut self, slot: StackSlot) {
        self.emit_load_from_s0("a0", slot.0, "ld");
        self.emit_load_from_s0("a1", slot.0 + 8, "ld");
    }

    fn f128_store_result_and_truncate(&mut self, dest: &Value) {
        // Store full f128 from arg1 (a0:a1) to dest slot
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_s0("a0", slot.0, "sd");
            self.emit_store_to_s0("a1", slot.0 + 8, "sd");
        }
        // Produce f64 approximation
        self.state.emit("    call __trunctfdf2");
        self.state.emit("    fmv.x.d t0, fa0");
        self.state.reg_cache.invalidate_all();
        self.state.track_f128_self(dest.0);
        // Store f64 approx only to register (if register-allocated), not to slot
        if let Some(&reg) = self.reg_assignments.get(&dest.0) {
            let reg_name = callee_saved_name(reg);
            self.state.emit_fmt(format_args!("    mv {}, t0", reg_name));
        }
        self.state.reg_cache.set_acc(dest.0, false);
    }

    fn f128_move_acc_to_arg0(&mut self) {
        self.state.emit("    mv a0, t0");
    }

    fn f128_move_arg0_to_acc(&mut self) {
        self.state.emit("    mv t0, a0");
    }

    fn f128_load_operand_to_acc(&mut self, op: &Operand) {
        self.operand_to_t0(op);
    }

    fn f128_sign_extend_acc(&mut self, from_size: usize) {
        match from_size {
            1 => {
                self.state.emit("    slli t0, t0, 56");
                self.state.emit("    srai t0, t0, 56");
            }
            2 => {
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srai t0, t0, 48");
            }
            4 => self.state.emit("    sext.w t0, t0"),
            _ => {}
        }
    }

    fn f128_zero_extend_acc(&mut self, from_size: usize) {
        match from_size {
            1 => self.state.emit("    andi t0, t0, 0xff"),
            2 => {
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srli t0, t0, 48");
            }
            4 => {
                self.state.emit("    slli t0, t0, 32");
                self.state.emit("    srli t0, t0, 32");
            }
            _ => {}
        }
    }

    fn f128_narrow_acc(&mut self, to_ty: IrType) {
        self.emit_cast_instrs(IrType::I64, to_ty);
    }

    fn f128_extend_float_to_f128(&mut self, from_ty: IrType) {
        if from_ty == IrType::F32 {
            self.state.emit("    fmv.w.x fa0, t0");
            self.state.emit("    call __extendsftf2");
        } else {
            self.state.emit("    fmv.d.x fa0, t0");
            self.state.emit("    call __extenddftf2");
        }
    }

    fn f128_truncate_to_float_acc(&mut self, to_ty: IrType) {
        if to_ty == IrType::F32 {
            self.state.emit("    call __trunctfsf2");
            self.state.emit("    fmv.x.w t0, fa0");
        } else {
            self.state.emit("    call __trunctfdf2");
            self.state.emit("    fmv.x.d t0, fa0");
        }
    }

    fn f128_is_alloca(&self, val_id: u32) -> bool {
        self.state.is_alloca(val_id)
    }
}

// =============================================================================
// Public helpers that delegate to shared orchestration
// =============================================================================

impl RiscvCodegen {
    /// Load an F128 operand into a0:a1 with full precision.
    pub(super) fn emit_f128_operand_to_a0_a1(&mut self, op: &Operand) {
        crate::backend::f128_softfloat::f128_operand_to_arg1(self, op);
    }

    /// Negate an F128 value with full precision.
    pub(super) fn emit_f128_neg_full(&mut self, dest: &Value, src: &Operand) {
        crate::backend::f128_softfloat::f128_neg(self, dest, src);
    }
}
