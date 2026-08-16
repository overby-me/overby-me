//! RiscvCodegen: return value handling.

use super::emit::RiscvCodegen;
use crate::ir::reexports::{IrConst, Operand, Value};

impl RiscvCodegen {
    pub(super) fn emit_return_impl(&mut self, val: Option<&Operand>, frame_size: i64) {
        if let Some(val) = val {
            let ret_ty = self.current_return_type;
            if ret_ty.is_long_double() {
                // Full-precision F128 return: load the full 128-bit value into a0:a1
                self.emit_f128_operand_to_a0_a1(val);
                self.emit_epilogue_and_ret_impl(frame_size);
                return;
            }
        }
        crate::backend::traits::emit_return_default(self, val, frame_size);
    }

    pub(super) fn emit_return_i128_to_regs_impl(&mut self) {
        self.state.emit("    mv a0, t0");
        self.state.emit("    mv a1, t1");
    }

    pub(super) fn emit_return_f128_to_reg_impl(&mut self) {
        self.state.emit("    fmv.d.x fa0, t0");
        self.state.emit("    call __extenddftf2");
    }

    pub(super) fn emit_return_f32_to_reg_impl(&mut self) {
        self.state.emit("    fmv.w.x fa0, t0");
    }

    pub(super) fn emit_return_f64_to_reg_impl(&mut self) {
        self.state.emit("    fmv.d.x fa0, t0");
    }

    pub(super) fn emit_return_int_to_reg_impl(&mut self) {
        self.state.emit("    mv a0, t0");
    }

    pub(super) fn emit_get_return_f64_second_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_s0("fa1", slot.0, "fsd");
        }
    }

    pub(super) fn emit_set_return_f64_second_impl(&mut self, src: &Operand) {
        match src {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    self.emit_load_from_s0("fa1", slot.0, "fld");
                }
            }
            Operand::Const(IrConst::F64(f)) => {
                let bits = f.to_bits() as i64;
                self.state.emit_fmt(format_args!("    li t0, {}", bits));
                self.state.emit("    fmv.d.x fa1, t0");
            }
            _ => {
                self.operand_to_t0(src);
                self.state.emit("    fmv.d.x fa1, t0");
            }
        }
    }

    pub(super) fn emit_get_return_f32_second_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_s0("fa1", slot.0, "fsw");
        }
    }

    pub(super) fn emit_set_return_f32_second_impl(&mut self, src: &Operand) {
        match src {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    self.emit_load_from_s0("fa1", slot.0, "flw");
                }
            }
            Operand::Const(IrConst::F32(f)) => {
                let bits = f.to_bits() as i64;
                self.state.emit_fmt(format_args!("    li t0, {}", bits));
                self.state.emit("    fmv.w.x fa1, t0");
            }
            _ => {
                self.operand_to_t0(src);
                self.state.emit("    fmv.w.x fa1, t0");
            }
        }
    }
}
