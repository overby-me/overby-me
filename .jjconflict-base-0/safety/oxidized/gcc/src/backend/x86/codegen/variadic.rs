//! X86Codegen: variadic argument handling (va_arg, va_start, va_copy).

use super::emit::{phys_reg_name, X86Codegen};
use crate::common::types::IrType;
use crate::ir::reexports::Value;

impl X86Codegen {
    pub(super) fn emit_va_arg_impl(
        &mut self,
        dest: &Value,
        va_list_ptr: &Value,
        result_ty: IrType,
    ) {
        let is_fp = result_ty.is_float();
        let is_f128 = result_ty.is_long_double();
        let label_reg = self.state.fresh_label("va_arg_reg");
        let label_mem = self.state.fresh_label("va_arg_mem");
        let label_end = self.state.fresh_label("va_arg_end");

        // Load va_list pointer into %rcx
        if let Some(&reg) = self.reg_assignments.get(&va_list_ptr.0) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rcx");
        } else if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
            if self.state.is_alloca(va_list_ptr.0) {
                self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rcx");
            } else {
                self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rcx");
            }
        }

        if is_f128 {
            self.state.emit("    movq 8(%rcx), %rdx");
            self.state.emit("    addq $15, %rdx");
            self.state.emit("    andq $-16, %rdx");
            self.state.emit("    fldt (%rdx)");
            self.state.emit("    subq $8, %rsp");
            self.state.emit("    fstpl (%rsp)");
            self.state.emit("    movq (%rsp), %rax");
            self.state.emit("    addq $8, %rsp");
            self.state.emit("    addq $16, %rdx");
            self.state.emit("    movq %rdx, 8(%rcx)");
            self.store_rax_to(dest);
            self.state.reg_cache.invalidate_all();
            return;
        } else if is_fp {
            self.state.emit("    movl 4(%rcx), %eax");
            self.state.emit("    cmpl $176, %eax");
            self.state.out.emit_jcc_label("    jb", &label_reg);
            self.state.out.emit_jmp_label(&label_mem);

            self.state.out.emit_named_label(&label_reg);
            self.state.emit("    movl 4(%rcx), %eax");
            self.state.emit("    movslq %eax, %rdx");
            self.state.emit("    movq 16(%rcx), %rsi");
            if result_ty == IrType::F32 {
                self.state.emit("    movss (%rsi,%rdx), %xmm0");
                self.state.emit("    movd %xmm0, %eax");
            } else {
                self.state.emit("    movsd (%rsi,%rdx), %xmm0");
                self.state.emit("    movq %xmm0, %rax");
            }
            self.state.emit("    addl $16, 4(%rcx)");
            self.state.out.emit_jmp_label(&label_end);
        } else {
            self.state.emit("    movl (%rcx), %eax");
            self.state.emit("    cmpl $48, %eax");
            self.state.out.emit_jcc_label("    jb", &label_reg);
            self.state.out.emit_jmp_label(&label_mem);

            self.state.out.emit_named_label(&label_reg);
            self.state.emit("    movl (%rcx), %eax");
            self.state.emit("    movslq %eax, %rdx");
            self.state.emit("    movq 16(%rcx), %rsi");
            self.state.emit("    movq (%rsi,%rdx), %rax");
            self.state.emit("    addl $8, (%rcx)");
            self.state.out.emit_jmp_label(&label_end);
        }

        // Memory (overflow) path
        self.state.out.emit_named_label(&label_mem);
        self.state.emit("    movq 8(%rcx), %rdx");
        if is_fp && result_ty == IrType::F32 {
            self.state.emit("    movss (%rdx), %xmm0");
            self.state.emit("    movd %xmm0, %eax");
        } else if is_fp {
            self.state.emit("    movsd (%rdx), %xmm0");
            self.state.emit("    movq %xmm0, %rax");
        } else {
            self.state.emit("    movq (%rdx), %rax");
        }
        self.state.emit("    addq $8, 8(%rcx)");

        // End
        self.state.out.emit_named_label(&label_end);
        self.store_rax_to(dest);
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_va_arg_struct_impl(
        &mut self,
        dest_ptr: &Value,
        va_list_ptr: &Value,
        size: usize,
    ) {
        // Load va_list pointer into %rcx
        if let Some(&reg) = self.reg_assignments.get(&va_list_ptr.0) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rcx");
        } else if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
            if self.state.is_alloca(va_list_ptr.0) {
                self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rcx");
            } else {
                self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rcx");
            }
        }

        self.state.emit("    movq 8(%rcx), %rsi");

        if let Some(&reg) = self.reg_assignments.get(&dest_ptr.0) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rdi");
        } else if let Some(slot) = self.state.get_slot(dest_ptr.0) {
            if self.state.is_alloca(dest_ptr.0) {
                self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rdi");
            } else {
                self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rdi");
            }
        }

        let num_qwords = size.div_ceil(8);
        for i in 0..num_qwords {
            let offset = (i * 8) as i64;
            if offset + 8 <= size as i64 {
                self.state
                    .out
                    .emit_instr_mem_reg("    movq", offset, "rsi", "rax");
                self.state
                    .out
                    .emit_instr_reg_mem("    movq", "rax", offset, "rdi");
            } else {
                let remaining = size - i * 8;
                self.emit_partial_copy(offset, remaining);
            }
        }

        // Advance overflow_arg_area past the struct (8-byte aligned)
        if let Some(&reg) = self.reg_assignments.get(&va_list_ptr.0) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rcx");
        } else if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
            if self.state.is_alloca(va_list_ptr.0) {
                self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rcx");
            } else {
                self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rcx");
            }
        }
        let advance = size.div_ceil(8) * 8;
        self.state
            .out
            .emit_instr_imm_mem("    addq", advance as i64, 8, "rcx");
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_va_arg_struct_ex_impl(
        &mut self,
        dest_ptr: &Value,
        va_list_ptr: &Value,
        size: usize,
        eightbyte_classes: &[crate::common::types::EightbyteClass],
    ) {
        if eightbyte_classes.is_empty() {
            self.emit_va_arg_struct_impl(dest_ptr, va_list_ptr, size);
            return;
        }

        use crate::common::types::EightbyteClass;

        let gp_needed: usize = eightbyte_classes
            .iter()
            .filter(|c| **c == EightbyteClass::Integer)
            .count();
        let fp_needed: usize = eightbyte_classes
            .iter()
            .filter(|c| **c == EightbyteClass::Sse)
            .count();

        let label_reg = self.state.fresh_label("va_struct_reg");
        let label_mem = self.state.fresh_label("va_struct_mem");
        let label_end = self.state.fresh_label("va_struct_end");

        self.load_va_list_ptr_to_rcx(va_list_ptr);

        if let Some(&reg) = self.reg_assignments.get(&dest_ptr.0) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rdi");
        } else if let Some(slot) = self.state.get_slot(dest_ptr.0) {
            if self.state.is_alloca(dest_ptr.0) {
                self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rdi");
            } else {
                self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rdi");
            }
        }

        let need_gp_check = gp_needed > 0;
        let mut need_fp_check = fp_needed > 0;

        if need_gp_check {
            let gp_threshold = 48i64 - (gp_needed as i64 * 8);
            if gp_threshold < 0 {
                self.state.out.emit_jmp_label(&label_mem);
                need_fp_check = false;
            } else {
                self.state.emit("    movl (%rcx), %eax");
                self.state
                    .emit_fmt(format_args!("    cmpl ${}, %eax", gp_threshold));
                self.state.out.emit_jcc_label("    ja", &label_mem);
            }
        }
        if need_fp_check {
            let fp_threshold = 176i64 - (fp_needed as i64 * 16);
            if fp_threshold < 48 {
                self.state.out.emit_jmp_label(&label_mem);
            } else {
                self.state.emit("    movl 4(%rcx), %eax");
                self.state
                    .emit_fmt(format_args!("    cmpl ${}, %eax", fp_threshold));
                self.state.out.emit_jcc_label("    ja", &label_mem);
            }
        }

        // ==== Register path ====
        self.state.out.emit_named_label(&label_reg);
        {
            self.state.emit("    movq 16(%rcx), %rsi");

            for (i, class) in eightbyte_classes.iter().enumerate() {
                let dest_offset = (i * 8) as i64;
                match class {
                    EightbyteClass::Integer => {
                        self.state.emit("    movl (%rcx), %eax");
                        self.state.emit("    movslq %eax, %rdx");
                        self.state.emit("    movq (%rsi,%rdx), %rax");
                        self.state
                            .out
                            .emit_instr_reg_mem("    movq", "rax", dest_offset, "rdi");
                        self.state.emit("    addl $8, (%rcx)");
                    }
                    EightbyteClass::Sse => {
                        self.state.emit("    movl 4(%rcx), %eax");
                        self.state.emit("    movslq %eax, %rdx");
                        self.state.emit("    movsd (%rsi,%rdx), %xmm0");
                        self.state
                            .emit_fmt(format_args!("    movsd %xmm0, {}(%rdi)", dest_offset));
                        self.state.emit("    addl $16, 4(%rcx)");
                    }
                    EightbyteClass::NoClass => {
                        self.state.emit("    movl (%rcx), %eax");
                        self.state.emit("    movslq %eax, %rdx");
                        self.state.emit("    movq (%rsi,%rdx), %rax");
                        self.state
                            .out
                            .emit_instr_reg_mem("    movq", "rax", dest_offset, "rdi");
                        self.state.emit("    addl $8, (%rcx)");
                    }
                }
            }
        }
        self.state.out.emit_jmp_label(&label_end);

        // ==== Memory (overflow) path ====
        self.state.out.emit_named_label(&label_mem);
        {
            self.load_va_list_ptr_to_rcx(va_list_ptr);

            if let Some(&reg) = self.reg_assignments.get(&dest_ptr.0) {
                let reg_name = phys_reg_name(reg);
                self.state
                    .out
                    .emit_instr_reg_reg("    movq", reg_name, "rdi");
            } else if let Some(slot) = self.state.get_slot(dest_ptr.0) {
                if self.state.is_alloca(dest_ptr.0) {
                    self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rdi");
                } else {
                    self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rdi");
                }
            }

            self.state.emit("    movq 8(%rcx), %rsi");

            let num_qwords = size.div_ceil(8);
            for i in 0..num_qwords {
                let offset = (i * 8) as i64;
                if offset + 8 <= size as i64 {
                    self.state
                        .out
                        .emit_instr_mem_reg("    movq", offset, "rsi", "rax");
                    self.state
                        .out
                        .emit_instr_reg_mem("    movq", "rax", offset, "rdi");
                } else {
                    let remaining = size - i * 8;
                    self.emit_partial_copy(offset, remaining);
                }
            }

            let advance = size.div_ceil(8) * 8;
            self.load_va_list_ptr_to_rcx(va_list_ptr);
            self.state
                .out
                .emit_instr_imm_mem("    addq", advance as i64, 8, "rcx");
        }

        // ==== End ====
        self.state.out.emit_named_label(&label_end);
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_va_start_impl(&mut self, va_list_ptr: &Value) {
        if let Some(&reg) = self.reg_assignments.get(&va_list_ptr.0) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rax");
        } else if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
            if self.state.is_alloca(va_list_ptr.0) {
                self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rax");
            } else {
                self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rax");
            }
        }
        let gp_offset = self.num_named_int_params.min(6) * 8;
        self.state
            .out
            .emit_instr_imm_mem("    movl", gp_offset as i64, 0, "rax");
        let fp_offset = if self.no_sse {
            176
        } else {
            48 + self.num_named_fp_params.min(8) * 16
        };
        self.state
            .out
            .emit_instr_imm_mem("    movl", fp_offset as i64, 4, "rax");
        let overflow_offset = 16 + self.num_named_stack_bytes;
        self.state
            .out
            .emit_instr_rbp_reg("    leaq", overflow_offset as i64, "rcx");
        self.state.emit("    movq %rcx, 8(%rax)");
        let reg_save = self.reg_save_area_offset;
        self.state
            .out
            .emit_instr_rbp_reg("    leaq", reg_save, "rcx");
        self.state.emit("    movq %rcx, 16(%rax)");
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_va_copy_impl(&mut self, dest_ptr: &Value, src_ptr: &Value) {
        if let Some(&reg) = self.reg_assignments.get(&src_ptr.0) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rsi");
        } else if let Some(src_slot) = self.state.get_slot(src_ptr.0) {
            if self.state.is_alloca(src_ptr.0) {
                self.state
                    .out
                    .emit_instr_rbp_reg("    leaq", src_slot.0, "rsi");
            } else {
                self.state
                    .out
                    .emit_instr_rbp_reg("    movq", src_slot.0, "rsi");
            }
        }
        if let Some(&reg) = self.reg_assignments.get(&dest_ptr.0) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rdi");
        } else if let Some(dest_slot) = self.state.get_slot(dest_ptr.0) {
            if self.state.is_alloca(dest_ptr.0) {
                self.state
                    .out
                    .emit_instr_rbp_reg("    leaq", dest_slot.0, "rdi");
            } else {
                self.state
                    .out
                    .emit_instr_rbp_reg("    movq", dest_slot.0, "rdi");
            }
        }
        self.state.emit("    movq (%rsi), %rax");
        self.state.emit("    movq %rax, (%rdi)");
        self.state.emit("    movq 8(%rsi), %rax");
        self.state.emit("    movq %rax, 8(%rdi)");
        self.state.emit("    movq 16(%rsi), %rax");
        self.state.emit("    movq %rax, 16(%rdi)");
        self.state.reg_cache.invalidate_all();
    }

    /// Helper to emit partial struct copy for the last partial qword.
    fn emit_partial_copy(&mut self, offset: i64, remaining: usize) {
        if remaining >= 4 {
            self.state
                .out
                .emit_instr_mem_reg("    movl", offset, "rsi", "eax");
            self.state
                .out
                .emit_instr_reg_mem("    movl", "eax", offset, "rdi");
            if remaining > 4 {
                let off4 = offset + 4;
                if remaining >= 6 {
                    self.state
                        .out
                        .emit_instr_mem_reg("    movzwl", off4, "rsi", "eax");
                    self.state
                        .out
                        .emit_instr_reg_mem("    movw", "ax", off4, "rdi");
                    if remaining == 7 {
                        let off6 = offset + 6;
                        self.state
                            .out
                            .emit_instr_mem_reg("    movzbl", off6, "rsi", "eax");
                        self.state
                            .out
                            .emit_instr_reg_mem("    movb", "al", off6, "rdi");
                    }
                } else {
                    // remaining == 5
                    self.state
                        .out
                        .emit_instr_mem_reg("    movzbl", off4, "rsi", "eax");
                    self.state
                        .out
                        .emit_instr_reg_mem("    movb", "al", off4, "rdi");
                }
            }
        } else if remaining >= 2 {
            self.state
                .out
                .emit_instr_mem_reg("    movzwl", offset, "rsi", "eax");
            self.state
                .out
                .emit_instr_reg_mem("    movw", "ax", offset, "rdi");
            if remaining == 3 {
                let off2 = offset + 2;
                self.state
                    .out
                    .emit_instr_mem_reg("    movzbl", off2, "rsi", "eax");
                self.state
                    .out
                    .emit_instr_reg_mem("    movb", "al", off2, "rdi");
            }
        } else {
            self.state
                .out
                .emit_instr_mem_reg("    movzbl", offset, "rsi", "eax");
            self.state
                .out
                .emit_instr_reg_mem("    movb", "al", offset, "rdi");
        }
    }
}
