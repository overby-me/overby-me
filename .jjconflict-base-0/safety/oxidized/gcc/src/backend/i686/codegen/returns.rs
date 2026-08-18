//! I686Codegen: return value operations.

use super::emit::I686Codegen;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{Operand, Value};

impl I686Codegen {
    pub(super) fn emit_return_impl(&mut self, val: Option<&Operand>, frame_size: i64) {
        if let Some(val) = val {
            let ret_ty = self.current_return_type;
            if ret_ty == IrType::I64 || ret_ty == IrType::U64 {
                self.emit_load_acc_pair(val);
                self.emit_epilogue_and_ret(frame_size);
                return;
            }
            if ret_ty == IrType::F64 {
                self.emit_f64_load_to_x87(val);
                self.emit_epilogue_and_ret(frame_size);
                return;
            }
            if ret_ty.is_long_double() {
                self.emit_f128_load_to_x87(val);
                self.emit_epilogue_and_ret(frame_size);
                return;
            }
        }
        // Delegate all other cases (I128, F32, scalar int) to default
        crate::backend::traits::emit_return_default(self, val, frame_size);
    }

    pub(super) fn emit_return_i128_to_regs_impl(&mut self) {
        // eax:edx already holds the low 64 bits
    }

    pub(super) fn emit_return_f128_to_reg_impl(&mut self) {
        self.state.emit("    pushl %eax");
        self.state.emit("    fildl (%esp)");
        self.state.emit("    addl $4, %esp");
    }

    pub(super) fn emit_return_f32_to_reg_impl(&mut self) {
        self.state.emit("    pushl %eax");
        self.state.emit("    flds (%esp)");
        self.state.emit("    addl $4, %esp");
    }

    pub(super) fn emit_return_f64_to_reg_impl(&mut self) {
        self.state.emit("    pushl %edx");
        self.state.emit("    pushl %eax");
        self.state.emit("    fldl (%esp)");
        self.state.emit("    addl $8, %esp");
    }

    pub(super) fn emit_return_int_to_reg_impl(&mut self) {
        // eax already holds the return value
    }

    pub(super) fn emit_get_return_f64_second_impl(&mut self, dest: &Value) {
        self.store_eax_to(dest);
    }

    pub(super) fn emit_set_return_f64_second_impl(&mut self, src: &Operand) {
        self.operand_to_eax(src);
    }

    pub(super) fn emit_get_return_f32_second_impl(&mut self, dest: &Value) {
        self.store_eax_to(dest);
    }

    pub(super) fn emit_set_return_f32_second_impl(&mut self, src: &Operand) {
        self.operand_to_eax(src);
    }

    pub(super) fn emit_get_return_f128_second_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    fstpt {}", sr);
            self.state.f128_direct_slots.insert(dest.0);
        }
    }

    pub(super) fn emit_set_return_f128_second_impl(&mut self, src: &Operand) {
        self.emit_f128_load_to_x87(src);
    }
}
