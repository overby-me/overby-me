//! ArmCodegen: variadic function operations (va_arg, va_start, va_copy).

use super::emit::{callee_saved_name, ArmCodegen};
use crate::common::types::IrType;
use crate::ir::reexports::Value;

impl ArmCodegen {
    pub(super) fn emit_va_arg_impl(
        &mut self,
        dest: &Value,
        va_list_ptr: &Value,
        result_ty: IrType,
    ) {
        let is_fp = result_ty.is_float();
        let is_f128 = result_ty.is_long_double();

        if self.state.is_alloca(va_list_ptr.0) {
            if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
                self.emit_add_fp_offset("x1", slot.0);
            }
        } else if let Some(&reg) = self.reg_assignments.get(&va_list_ptr.0) {
            let reg_name = callee_saved_name(reg);
            self.state
                .emit_fmt(format_args!("    mov x1, {}", reg_name));
        } else if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
            self.emit_load_from_sp("x1", slot.0, "ldr");
        }

        if is_f128 {
            let label_id = self.state.next_label_id();
            let label_stack = format!(".Lva_stack_{}", label_id);
            let label_done = format!(".Lva_done_{}", label_id);

            self.state.emit("    ldrsw x2, [x1, #28]");
            self.state
                .emit_fmt(format_args!("    tbz x2, #63, {}", label_stack));
            self.state.emit("    ldr x3, [x1, #16]");
            self.state.emit("    add x3, x3, x2");
            self.state.emit("    add w2, w2, #16");
            self.state.emit("    str w2, [x1, #28]");
            self.state.emit("    ldr q0, [x3]");
            self.state.emit("    bl __trunctfdf2");
            self.state.emit("    fmov x0, d0");
            self.state.emit_fmt(format_args!("    b {}", label_done));

            self.state.emit_fmt(format_args!("{}:", label_stack));
            self.state.emit("    ldr x3, [x1]");
            self.state.emit("    add x3, x3, #15");
            self.state.emit("    and x3, x3, #-16");
            self.state.emit("    mov x4, x1");
            self.state.emit("    ldr q0, [x3]");
            self.state.emit("    add x3, x3, #16");
            self.state.emit("    str x3, [x4]");
            self.state.emit("    bl __trunctfdf2");
            self.state.emit("    fmov x0, d0");

            self.state.emit_fmt(format_args!("{}:", label_done));
            self.state.reg_cache.invalidate_all();
        } else if is_fp {
            let label_id = self.state.next_label_id();
            let label_stack = format!(".Lva_stack_{}", label_id);
            let label_done = format!(".Lva_done_{}", label_id);

            self.state.emit("    ldrsw x2, [x1, #28]");
            self.state
                .emit_fmt(format_args!("    tbz x2, #63, {}", label_stack));
            self.state.emit("    ldr x3, [x1, #16]");
            self.state.emit("    add x3, x3, x2");
            self.state.emit("    add w2, w2, #16");
            self.state.emit("    str w2, [x1, #28]");
            if result_ty == IrType::F32 {
                self.state.emit("    ldr w0, [x3]");
            } else {
                self.state.emit("    ldr x0, [x3]");
            }
            self.state.emit_fmt(format_args!("    b {}", label_done));

            self.state.emit_fmt(format_args!("{}:", label_stack));
            self.state.emit("    ldr x3, [x1]");
            if result_ty == IrType::F32 {
                self.state.emit("    ldr w0, [x3]");
            } else {
                self.state.emit("    ldr x0, [x3]");
            }
            self.state.emit("    add x3, x3, #8");
            self.state.emit("    str x3, [x1]");

            self.state.emit_fmt(format_args!("{}:", label_done));
        } else {
            let label_id = self.state.next_label_id();
            let label_stack = format!(".Lva_stack_{}", label_id);
            let label_done = format!(".Lva_done_{}", label_id);

            self.state.emit("    ldrsw x2, [x1, #24]");
            self.state
                .emit_fmt(format_args!("    tbz x2, #63, {}", label_stack));
            self.state.emit("    ldr x3, [x1, #8]");
            self.state.emit("    add x3, x3, x2");
            self.state.emit("    add w2, w2, #8");
            self.state.emit("    str w2, [x1, #24]");
            self.state.emit("    ldr x0, [x3]");
            self.state.emit_fmt(format_args!("    b {}", label_done));

            self.state.emit_fmt(format_args!("{}:", label_stack));
            self.state.emit("    ldr x3, [x1]");
            self.state.emit("    ldr x0, [x3]");
            self.state.emit("    add x3, x3, #8");
            self.state.emit("    str x3, [x1]");

            self.state.emit_fmt(format_args!("{}:", label_done));
        }

        if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_sp("x0", slot.0, "str");
        }
    }

    pub(super) fn emit_va_start_impl(&mut self, va_list_ptr: &Value) {
        if self.state.is_alloca(va_list_ptr.0) {
            if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
                self.emit_add_fp_offset("x0", slot.0);
            }
        } else if let Some(&reg) = self.reg_assignments.get(&va_list_ptr.0) {
            let reg_name = callee_saved_name(reg);
            self.state
                .emit_fmt(format_args!("    mov x0, {}", reg_name));
        } else if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
            self.emit_load_from_sp("x0", slot.0, "ldr");
        }

        let stack_offset = self.current_frame_size + self.va_named_stack_bytes as i64;
        if stack_offset <= 4095 {
            self.state
                .emit_fmt(format_args!("    add x1, x29, #{}", stack_offset));
        } else {
            self.load_large_imm("x1", stack_offset);
            self.state.emit("    add x1, x29, x1");
        }
        self.state.emit("    str x1, [x0]");

        let gr_top_offset = self.va_gp_save_offset + 64;
        self.emit_add_sp_offset("x1", gr_top_offset);
        self.state.emit("    str x1, [x0, #8]");

        if self.general_regs_only {
            self.state.emit("    str xzr, [x0, #16]");
        } else {
            let vr_top_offset = self.va_fp_save_offset + 128;
            self.emit_add_sp_offset("x1", vr_top_offset);
            self.state.emit("    str x1, [x0, #16]");
        }

        let gr_offs: i32 = -((8 - self.va_named_gp_count as i32) * 8);
        self.state
            .emit_fmt(format_args!("    mov w1, #{}", gr_offs));
        self.state.emit("    str w1, [x0, #24]");

        let vr_offs: i32 = if self.general_regs_only {
            0
        } else {
            -((8 - self.va_named_fp_count as i32) * 16)
        };
        self.state
            .emit_fmt(format_args!("    mov w1, #{}", vr_offs));
        self.state.emit("    str w1, [x0, #28]");
    }

    pub(super) fn emit_va_copy_impl(&mut self, dest_ptr: &Value, src_ptr: &Value) {
        if self.state.is_alloca(src_ptr.0) {
            if let Some(src_slot) = self.state.get_slot(src_ptr.0) {
                self.emit_add_fp_offset("x1", src_slot.0);
            }
        } else if let Some(&reg) = self.reg_assignments.get(&src_ptr.0) {
            let reg_name = callee_saved_name(reg);
            self.state
                .emit_fmt(format_args!("    mov x1, {}", reg_name));
        } else if let Some(src_slot) = self.state.get_slot(src_ptr.0) {
            self.emit_load_from_sp("x1", src_slot.0, "ldr");
        }
        if self.state.is_alloca(dest_ptr.0) {
            if let Some(dest_slot) = self.state.get_slot(dest_ptr.0) {
                self.emit_add_fp_offset("x0", dest_slot.0);
            }
        } else if let Some(&reg) = self.reg_assignments.get(&dest_ptr.0) {
            let reg_name = callee_saved_name(reg);
            self.state
                .emit_fmt(format_args!("    mov x0, {}", reg_name));
        } else if let Some(dest_slot) = self.state.get_slot(dest_ptr.0) {
            self.emit_load_from_sp("x0", dest_slot.0, "ldr");
        }
        self.state.emit("    ldp x2, x3, [x1]");
        self.state.emit("    stp x2, x3, [x0]");
        self.state.emit("    ldp x2, x3, [x1, #16]");
        self.state.emit("    stp x2, x3, [x0, #16]");
    }

    /// Emit va_arg for struct types on AArch64 (AAPCS64).
    ///
    /// Per AAPCS64, composite types in variadic args are passed via GP registers.
    /// A struct requiring N register slots (N = ceil(size/8)) must fit ENTIRELY
    /// in the remaining GP register save area, or be read ENTIRELY from the stack
    /// overflow area. It must never be split across the boundary.
    ///
    /// AAPCS64 va_list layout (32 bytes):
    ///   [+0]  __stack   : pointer to next stack (overflow) arg
    ///   [+8]  __gr_top  : pointer to top of GP register save area
    ///   [+16] __vr_top  : pointer to top of FP register save area
    ///   [+24] __gr_offs : negative offset from __gr_top (i32)
    ///   [+28] __vr_offs : negative offset from __vr_top (i32)
    ///
    /// __gr_offs starts negative and advances toward 0. When it would become >= 0
    /// for the struct, we use the stack path instead.
    pub(super) fn emit_va_arg_struct_impl(
        &mut self,
        dest_ptr: &Value,
        va_list_ptr: &Value,
        size: usize,
    ) {
        let num_slots = size.div_ceil(8);
        let total_reg_bytes = num_slots * 8;

        let label_id = self.state.next_label_id();
        let label_stack = format!(".Lva_struct_stack_{}", label_id);
        let label_done = format!(".Lva_struct_done_{}", label_id);

        // Load va_list pointer into x1
        self.load_va_list_ptr(va_list_ptr, "x1");

        // Load dest_ptr into x4 (scratch register)
        self.load_dest_ptr(dest_ptr, "x4");

        // Check if enough GP register slots remain for the entire struct.
        // __gr_offs is a negative i32 at [va_list + 24].
        // We need: __gr_offs + total_reg_bytes <= 0
        // Equivalently: __gr_offs <= -total_reg_bytes
        // Or: __gr_offs + total_reg_bytes is still negative (bit 63 set after sign-extend + add)
        self.state.emit("    ldrsw x2, [x1, #24]"); // x2 = sign-extended __gr_offs
        if total_reg_bytes <= 4095 {
            self.state
                .emit_fmt(format_args!("    adds x3, x2, #{}", total_reg_bytes));
        } else {
            self.load_large_imm("x3", total_reg_bytes as i64);
            self.state.emit("    adds x3, x2, x3");
        }
        // If x3 > 0 (not enough register slots for entire struct), use stack path.
        self.state
            .emit_fmt(format_args!("    b.gt {}", label_stack));

        // ==== Register path ====
        // Read all slots from the GP register save area.
        // Base address: __gr_top + __gr_offs
        self.state.emit("    ldr x5, [x1, #8]"); // x5 = __gr_top
        self.state.emit("    add x5, x5, x2"); // x5 = __gr_top + __gr_offs (source addr)

        // Copy struct data from register save area to dest
        for i in 0..num_slots {
            let offset = (i * 8) as i64;
            if offset + 8 <= size as i64 {
                // Full 8-byte slot
                if offset == 0 {
                    self.state.emit("    ldr x6, [x5]");
                    self.state.emit("    str x6, [x4]");
                } else {
                    self.state
                        .emit_fmt(format_args!("    ldr x6, [x5, #{}]", offset));
                    self.state
                        .emit_fmt(format_args!("    str x6, [x4, #{}]", offset));
                }
            } else {
                // Partial last slot: copy remaining bytes
                let remaining = size - i * 8;
                self.emit_partial_struct_copy(offset, remaining, "x5", "x4");
            }
        }

        // Advance __gr_offs by total_reg_bytes
        // x3 already holds __gr_offs + total_reg_bytes from the adds above
        self.state.emit("    str w3, [x1, #24]");
        self.state.emit_fmt(format_args!("    b {}", label_done));

        // ==== Stack path ====
        self.state.emit_fmt(format_args!("{}:", label_stack));
        // When falling through to stack, we must also set __gr_offs to 0
        // to indicate no more GP registers available (per AAPCS64).
        self.state.emit("    str wzr, [x1, #24]");

        // Read from __stack
        self.state.emit("    ldr x5, [x1]"); // x5 = __stack (source addr)

        // Align __stack to 8 bytes (structs on stack are 8-byte aligned per AAPCS64)
        self.state.emit("    add x5, x5, #7");
        self.state.emit("    and x5, x5, #-8");

        // Copy struct data from stack to dest
        for i in 0..num_slots {
            let offset = (i * 8) as i64;
            if offset + 8 <= size as i64 {
                if offset == 0 {
                    self.state.emit("    ldr x6, [x5]");
                    self.state.emit("    str x6, [x4]");
                } else {
                    self.state
                        .emit_fmt(format_args!("    ldr x6, [x5, #{}]", offset));
                    self.state
                        .emit_fmt(format_args!("    str x6, [x4, #{}]", offset));
                }
            } else {
                let remaining = size - i * 8;
                self.emit_partial_struct_copy(offset, remaining, "x5", "x4");
            }
        }

        // Advance __stack past the struct (8-byte aligned)
        let advance = num_slots * 8;
        if advance <= 4095 {
            self.state
                .emit_fmt(format_args!("    add x5, x5, #{}", advance));
        } else {
            self.load_large_imm("x6", advance as i64);
            self.state.emit("    add x5, x5, x6");
        }
        self.state.emit("    str x5, [x1]");

        // ==== Done ====
        self.state.emit_fmt(format_args!("{}:", label_done));
        self.state.reg_cache.invalidate_all();
    }

    /// Load the va_list pointer into the specified register.
    fn load_va_list_ptr(&mut self, va_list_ptr: &Value, dest_reg: &str) {
        if self.state.is_alloca(va_list_ptr.0) {
            if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
                self.emit_add_fp_offset(dest_reg, slot.0);
            }
        } else if let Some(&reg) = self.reg_assignments.get(&va_list_ptr.0) {
            let reg_name = callee_saved_name(reg);
            self.state
                .emit_fmt(format_args!("    mov {}, {}", dest_reg, reg_name));
        } else if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
            self.emit_load_from_sp(dest_reg, slot.0, "ldr");
        }
    }

    /// Load the destination pointer into the specified register.
    fn load_dest_ptr(&mut self, dest_ptr: &Value, dest_reg: &str) {
        if self.state.is_alloca(dest_ptr.0) {
            if let Some(slot) = self.state.get_slot(dest_ptr.0) {
                self.emit_add_fp_offset(dest_reg, slot.0);
            }
        } else if let Some(&reg) = self.reg_assignments.get(&dest_ptr.0) {
            let reg_name = callee_saved_name(reg);
            self.state
                .emit_fmt(format_args!("    mov {}, {}", dest_reg, reg_name));
        } else if let Some(slot) = self.state.get_slot(dest_ptr.0) {
            self.emit_load_from_sp(dest_reg, slot.0, "ldr");
        }
    }

    /// Emit byte-by-byte copy for a partial struct slot (last slot with < 8 bytes).
    fn emit_partial_struct_copy(
        &mut self,
        base_offset: i64,
        remaining: usize,
        src_reg: &str,
        dst_reg: &str,
    ) {
        let mut copied = 0usize;
        // Copy 4 bytes if possible
        if remaining >= 4 {
            let off = base_offset + copied as i64;
            if off == 0 {
                self.state
                    .emit_fmt(format_args!("    ldr w6, [{}]", src_reg));
                self.state
                    .emit_fmt(format_args!("    str w6, [{}]", dst_reg));
            } else {
                self.state
                    .emit_fmt(format_args!("    ldr w6, [{}, #{}]", src_reg, off));
                self.state
                    .emit_fmt(format_args!("    str w6, [{}, #{}]", dst_reg, off));
            }
            copied += 4;
        }
        // Copy 2 bytes if possible
        if remaining - copied >= 2 {
            let off = base_offset + copied as i64;
            self.state
                .emit_fmt(format_args!("    ldrh w6, [{}, #{}]", src_reg, off));
            self.state
                .emit_fmt(format_args!("    strh w6, [{}, #{}]", dst_reg, off));
            copied += 2;
        }
        // Copy 1 byte if remaining
        if remaining - copied >= 1 {
            let off = base_offset + copied as i64;
            self.state
                .emit_fmt(format_args!("    ldrb w6, [{}, #{}]", src_reg, off));
            self.state
                .emit_fmt(format_args!("    strb w6, [{}, #{}]", dst_reg, off));
        }
    }
}
