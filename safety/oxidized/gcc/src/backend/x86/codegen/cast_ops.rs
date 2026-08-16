//! X86Codegen: cast operations.

use super::emit::X86Codegen;
use crate::backend::generation::is_i128_type;
use crate::common::types::IrType;
use crate::ir::reexports::{IrConst, Operand, Value};

impl X86Codegen {
    pub(super) fn emit_cast_instrs_impl(&mut self, from_ty: IrType, to_ty: IrType) {
        self.emit_cast_instrs_x86(from_ty, to_ty);
    }

    pub(super) fn emit_cast_impl(
        &mut self,
        dest: &Value,
        src: &Operand,
        from_ty: IrType,
        to_ty: IrType,
    ) {
        // Intercept casts TO F128: produce full 80-bit x87 value in dest slot.
        if to_ty == IrType::F128 && from_ty != IrType::F128 && !is_i128_type(from_ty) {
            if let Some(dest_slot) = self.state.get_slot(dest.0) {
                if from_ty == IrType::F64 {
                    self.operand_to_rax(src);
                    self.state.emit("    subq $8, %rsp");
                    self.state.emit("    movq %rax, (%rsp)");
                    self.state.emit("    fldl (%rsp)");
                    self.state.emit("    addq $8, %rsp");
                } else if from_ty == IrType::F32 {
                    self.operand_to_rax(src);
                    self.state.emit("    subq $4, %rsp");
                    self.state.emit("    movl %eax, (%rsp)");
                    self.state.emit("    flds (%rsp)");
                    self.state.emit("    addq $4, %rsp");
                } else if from_ty.is_signed() || (!from_ty.is_float() && !from_ty.is_unsigned()) {
                    self.operand_to_rax(src);
                    if from_ty.size() < 8 {
                        self.emit_cast_instrs_x86(from_ty, IrType::I64);
                    }
                    self.state.emit("    subq $8, %rsp");
                    self.state.emit("    movq %rax, (%rsp)");
                    self.state.emit("    fildq (%rsp)");
                    self.state.emit("    addq $8, %rsp");
                } else {
                    self.operand_to_rax(src);
                    if from_ty.size() < 8 {
                        self.emit_cast_instrs_x86(from_ty, IrType::I64);
                    }
                    let big_label = self.state.fresh_label("u2f128_big");
                    let done_label = self.state.fresh_label("u2f128_done");
                    self.state.emit("    testq %rax, %rax");
                    self.state.out.emit_jcc_label("    js", &big_label);
                    self.state.emit("    subq $8, %rsp");
                    self.state.emit("    movq %rax, (%rsp)");
                    self.state.emit("    fildq (%rsp)");
                    self.state.emit("    addq $8, %rsp");
                    self.state.out.emit_jmp_label(&done_label);
                    self.state.out.emit_named_label(&big_label);
                    self.state.emit("    subq $8, %rsp");
                    self.state.emit("    movq %rax, (%rsp)");
                    self.state.emit("    fildq (%rsp)");
                    self.state.emit("    addq $8, %rsp");
                    self.state.emit("    subq $16, %rsp");
                    self.state.out.emit_instr_imm_reg(
                        "    movabsq",
                        -9223372036854775808i64,
                        "rax",
                    );
                    self.state.emit("    movq %rax, (%rsp)");
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", 0x403Fi64, "rax");
                    self.state.emit("    movq %rax, 8(%rsp)");
                    self.state.emit("    fldt (%rsp)");
                    self.state.emit("    addq $16, %rsp");
                    self.state.emit("    faddp %st, %st(1)");
                    self.state.out.emit_named_label(&done_label);
                }
                self.state.out.emit_instr_rbp("    fstpt", dest_slot.0);
                self.state.out.emit_instr_rbp("    fldt", dest_slot.0);
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    popq %rax");
                self.state.reg_cache.set_acc(dest.0, false);
                self.state.f128_direct_slots.insert(dest.0);
                return;
            }
        }

        // Intercept F128 -> F64/F32 casts
        if from_ty == IrType::F128 && (to_ty == IrType::F64 || to_ty == IrType::F32) {
            self.emit_f128_load_to_x87(src);
            if to_ty == IrType::F64 {
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    movq (%rsp), %rax");
                self.state.emit("    addq $8, %rsp");
            } else {
                self.state.emit("    subq $4, %rsp");
                self.state.emit("    fstps (%rsp)");
                self.state.emit("    movl (%rsp), %eax");
                self.state.emit("    addq $4, %rsp");
            }
            self.state.reg_cache.invalidate_acc();
            self.store_rax_to(dest);
            return;
        }

        // Intercept F128 -> integer casts when we know the source's memory location
        if from_ty == IrType::F128 && !to_ty.is_float() && !is_i128_type(to_ty) {
            if let Operand::Value(v) = src {
                if self.state.f128_direct_slots.contains(&v.0) {
                    if let Some(slot) = self.state.get_slot(v.0) {
                        let addr = crate::backend::state::SlotAddr::Direct(slot);
                        self.emit_f128_to_int_from_memory(&addr, to_ty);
                        self.store_rax_to(dest);
                        return;
                    }
                }
                if let Some((ptr_id, _offset, _is_indirect)) = self.state.get_f128_source(v.0) {
                    if let Some(addr) = self.state.resolve_slot_addr(ptr_id) {
                        self.emit_f128_to_int_from_memory(&addr, to_ty);
                        self.store_rax_to(dest);
                        return;
                    }
                }
            }
            if let Operand::Const(IrConst::LongDouble(_, f128_bytes)) = src {
                let x87 = crate::common::long_double::f128_bytes_to_x87_bytes(f128_bytes);
                self.state.emit("    subq $16, %rsp");
                let lo = u64::from_le_bytes(x87[0..8].try_into().unwrap());
                let hi = u16::from_le_bytes(x87[8..10].try_into().unwrap());
                self.state
                    .out
                    .emit_instr_imm_reg("    movabsq", lo as i64, "rax");
                self.state.emit("    movq %rax, (%rsp)");
                self.state
                    .out
                    .emit_instr_imm_reg("    movq", hi as i64, "rax");
                self.state.emit("    movq %rax, 8(%rsp)");
                self.state.emit("    fldt (%rsp)");
                self.state.emit("    addq $16, %rsp");
                self.emit_f128_st0_to_int(to_ty);
                self.store_rax_to(dest);
                return;
            }
        }
        // Fall through to default implementation for all other cases
        crate::backend::traits::emit_cast_default(self, dest, src, from_ty, to_ty);
    }
}
