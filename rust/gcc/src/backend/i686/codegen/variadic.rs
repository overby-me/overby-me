//! I686Codegen: variadic argument operations (va_arg, va_start, va_copy).

use super::emit::I686Codegen;
use crate::backend::generation::is_i128_type;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::Value;

impl I686Codegen {
    pub(super) fn emit_va_arg_impl(
        &mut self,
        dest: &Value,
        va_list_ptr: &Value,
        result_ty: IrType,
    ) {
        self.load_va_list_addr_to_edx(va_list_ptr);
        self.state.emit("    movl (%edx), %ecx");

        if is_i128_type(result_ty) {
            if let Some(dest_slot) = self.state.get_slot(dest.0) {
                for i in (0..16).step_by(4) {
                    emit!(self.state, "    movl {}(%ecx), %eax", i);
                    let sr = self.slot_ref_offset(dest_slot, i as i64);
                    emit!(self.state, "    movl %eax, {}", sr);
                }
            }
            self.state.emit("    addl $16, %ecx");
        } else if result_ty == IrType::F128 {
            if let Some(dest_slot) = self.state.get_slot(dest.0) {
                self.state.emit("    fldt (%ecx)");
                let sr = self.slot_ref(dest_slot);
                emit!(self.state, "    fstpt {}", sr);
                self.state.f128_direct_slots.insert(dest.0);
            }
            self.state.emit("    addl $12, %ecx");
        } else if result_ty == IrType::F64 || result_ty == IrType::I64 || result_ty == IrType::U64 {
            if let Some(dest_slot) = self.state.get_slot(dest.0) {
                let sr0 = self.slot_ref(dest_slot);
                let sr4 = self.slot_ref_offset(dest_slot, 4);
                self.state.emit("    movl (%ecx), %eax");
                emit!(self.state, "    movl %eax, {}", sr0);
                self.state.emit("    movl 4(%ecx), %eax");
                emit!(self.state, "    movl %eax, {}", sr4);
            }
            self.state.emit("    addl $8, %ecx");
        } else {
            let load_instr = self.mov_load_for_type(result_ty);
            emit!(self.state, "    {} (%ecx), %eax", load_instr);
            self.store_eax_to(dest);
            let advance = result_ty.size().max(4);
            emit!(self.state, "    addl ${}, %ecx", advance);
        }
        self.load_va_list_addr_to_edx(va_list_ptr);
        self.state.emit("    movl %ecx, (%edx)");
    }

    pub(super) fn emit_va_start_impl(&mut self, va_list_ptr: &Value) {
        let vararg_offset = 8 + self.va_named_stack_bytes as i64;
        self.load_va_list_addr_to_edx(va_list_ptr);
        // vararg_offset is an EBP-relative param offset
        let pr = self.param_ref(vararg_offset);
        emit!(self.state, "    leal {}, %eax", pr);
        self.state.emit("    movl %eax, (%edx)");
    }

    pub(super) fn emit_va_copy_impl(&mut self, dest_ptr: &Value, src_ptr: &Value) {
        self.load_va_list_addr_to_edx(src_ptr);
        self.state.emit("    movl (%edx), %eax");
        self.load_va_list_addr_to_edx(dest_ptr);
        self.state.emit("    movl %eax, (%edx)");
    }

    pub(super) fn emit_va_arg_struct_impl(
        &mut self,
        _dest_ptr: &Value,
        _va_list_ptr: &Value,
        _size: usize,
    ) {
        panic!("VaArgStruct should not be emitted for i686 target");
    }
}
