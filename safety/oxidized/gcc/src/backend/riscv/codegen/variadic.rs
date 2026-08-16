//! RiscvCodegen: va_arg, va_start, va_copy operations.

use super::emit::{callee_saved_name, RiscvCodegen};
use crate::common::types::IrType;
use crate::ir::reexports::Value;

impl RiscvCodegen {
    pub(super) fn emit_va_arg_impl(
        &mut self,
        dest: &Value,
        va_list_ptr: &Value,
        result_ty: IrType,
    ) {
        // RISC-V LP64D: va_list is just a void* (pointer to the next arg on stack).
        if self.state.is_alloca(va_list_ptr.0) {
            if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
                self.emit_addi_s0("t1", slot.0);
            }
        } else if let Some(&reg) = self.reg_assignments.get(&va_list_ptr.0) {
            let reg_name = callee_saved_name(reg);
            self.state.emit_fmt(format_args!("    mv t1, {}", reg_name));
        } else if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
            self.emit_load_from_s0("t1", slot.0, "ld");
        }
        // Load the current va_list pointer value (points to next arg)
        self.state.emit("    ld t2, 0(t1)");

        if result_ty.is_long_double() {
            // F128 (long double): 16 bytes, 16-byte aligned.
            self.state.emit("    addi t2, t2, 15");
            self.state.emit("    andi t2, t2, -16");
            self.state.emit("    ld a0, 0(t2)");
            self.state.emit("    ld a1, 8(t2)");
            self.state.emit("    addi t2, t2, 16");
            self.state.emit("    sd t2, 0(t1)");
            self.state.emit("    call __trunctfdf2");
            self.state.emit("    fmv.x.d t0, fa0");
        } else {
            // Standard 8-byte arg
            self.state.emit("    ld t0, 0(t2)");
            self.state.emit("    addi t2, t2, 8");
            self.state.emit("    sd t2, 0(t1)");
        }
        self.store_t0_to(dest);
    }

    pub(super) fn emit_va_start_impl(&mut self, va_list_ptr: &Value) {
        if self.state.is_alloca(va_list_ptr.0) {
            if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
                self.emit_addi_s0("t0", slot.0);
            }
        } else if let Some(&reg) = self.reg_assignments.get(&va_list_ptr.0) {
            let reg_name = callee_saved_name(reg);
            self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
        } else if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
            self.emit_load_from_s0("t0", slot.0, "ld");
        }

        let vararg_offset = if self.va_named_gp_count >= 8 {
            64 + self.va_named_stack_bytes as i64
        } else {
            (self.va_named_gp_count as i64) * 8
        };
        self.emit_addi_s0("t1", vararg_offset);
        self.state.emit("    sd t1, 0(t0)");
    }

    pub(super) fn emit_va_copy_impl(&mut self, dest_ptr: &Value, src_ptr: &Value) {
        if self.state.is_alloca(src_ptr.0) {
            if let Some(src_slot) = self.state.get_slot(src_ptr.0) {
                self.emit_addi_s0("t1", src_slot.0);
            }
        } else if let Some(&reg) = self.reg_assignments.get(&src_ptr.0) {
            let reg_name = callee_saved_name(reg);
            self.state.emit_fmt(format_args!("    mv t1, {}", reg_name));
        } else if let Some(src_slot) = self.state.get_slot(src_ptr.0) {
            self.emit_load_from_s0("t1", src_slot.0, "ld");
        }
        self.state.emit("    ld t2, 0(t1)");
        if self.state.is_alloca(dest_ptr.0) {
            if let Some(dest_slot) = self.state.get_slot(dest_ptr.0) {
                self.emit_addi_s0("t0", dest_slot.0);
            }
        } else if let Some(&reg) = self.reg_assignments.get(&dest_ptr.0) {
            let reg_name = callee_saved_name(reg);
            self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
        } else if let Some(dest_slot) = self.state.get_slot(dest_ptr.0) {
            self.emit_load_from_s0("t0", dest_slot.0, "ld");
        }
        self.state.emit("    sd t2, 0(t0)");
    }
}
