//! ArmCodegen: return operations.

use super::emit::ArmCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{IrConst, Operand, Value};

impl ArmCodegen {
    pub(super) fn emit_return_impl(&mut self, val: Option<&Operand>, frame_size: i64) {
        if let Some(val) = val {
            let ret_ty = self.current_return_type;
            if ret_ty.is_long_double() {
                self.emit_f128_operand_to_q0_full(val);
                self.emit_epilogue_and_ret_impl(frame_size);
                return;
            }
        }
        crate::backend::traits::emit_return_default(self, val, frame_size);
    }

    pub(super) fn emit_return_i128_to_regs_impl(&mut self) {
        // x0:x1 already hold the i128 return value per AAPCS64 -- noop
    }

    pub(super) fn emit_return_f128_to_reg_impl(&mut self) {
        self.state.emit("    fmov d0, x0");
        self.state.emit("    bl __extenddftf2");
    }

    pub(super) fn emit_return_f32_to_reg_impl(&mut self) {
        self.state.emit("    fmov s0, w0");
    }

    pub(super) fn emit_return_f64_to_reg_impl(&mut self) {
        self.state.emit("    fmov d0, x0");
    }

    pub(super) fn emit_return_int_to_reg_impl(&mut self) {
        // x0 already holds the return value per AAPCS64 -- noop
    }

    pub(super) fn current_return_type_impl(&self) -> IrType {
        self.current_return_type
    }

    pub(super) fn emit_get_return_f64_second_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_sp("d1", slot.0, "str");
        }
    }

    pub(super) fn emit_set_return_f64_second_impl(&mut self, src: &Operand) {
        match src {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    self.emit_load_from_sp("d1", slot.0, "ldr");
                }
            }
            Operand::Const(IrConst::F64(f)) => {
                let bits = f.to_bits();
                self.emit_load_imm64("x0", bits as i64);
                self.state.emit("    fmov d1, x0");
            }
            _ => {
                self.operand_to_x0(src);
                self.state.emit("    fmov d1, x0");
            }
        }
    }

    pub(super) fn emit_get_return_f32_second_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_sp("s1", slot.0, "str");
        }
    }

    pub(super) fn emit_set_return_f32_second_impl(&mut self, src: &Operand) {
        match src {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    self.emit_load_from_sp("s1", slot.0, "ldr");
                }
            }
            Operand::Const(IrConst::F32(f)) => {
                let bits = f.to_bits();
                self.emit_load_imm64("x0", bits as i64);
                self.state.emit("    fmov s1, w0");
            }
            _ => {
                self.operand_to_x0(src);
                self.state.emit("    fmov s1, w0");
            }
        }
    }

    pub(super) fn emit_get_return_f128_second_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_sp("q1", slot.0, "str");
            self.state.track_f128_self(dest.0);
        }
    }

    pub(super) fn emit_set_return_f128_second_impl(&mut self, src: &Operand) {
        self.emit_f128_operand_to_q0_full(src);
        self.state.emit("    mov v1.16b, v0.16b");
    }
}
