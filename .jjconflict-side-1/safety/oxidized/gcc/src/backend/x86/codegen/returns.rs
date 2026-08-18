//! X86Codegen: return value operations.

use super::emit::X86Codegen;
use crate::common::types::IrType;
use crate::ir::reexports::{IrConst, Operand, Value};

impl X86Codegen {
    pub(super) fn emit_return_impl(&mut self, val: Option<&Operand>, frame_size: i64) {
        use crate::backend::state::SlotAddr;
        if let Some(val) = val {
            let ret_ty = self.current_return_type;
            if ret_ty.is_long_double() {
                if let Operand::Value(v) = val {
                    if self.state.f128_direct_slots.contains(&v.0) {
                        if let Some(slot) = self.state.get_slot(v.0) {
                            self.state.out.emit_instr_rbp("    fldt", slot.0);
                            self.emit_epilogue_and_ret_impl(frame_size);
                            return;
                        }
                    }
                    if let Some((ptr_id, _offset, _is_indirect)) = self.state.get_f128_source(v.0) {
                        if let Some(addr) = self.state.resolve_slot_addr(ptr_id) {
                            match addr {
                                SlotAddr::Direct(slot) => {
                                    self.state.out.emit_instr_rbp("    fldt", slot.0);
                                }
                                SlotAddr::OverAligned(slot, id) => {
                                    self.emit_alloca_aligned_addr_impl(slot, id);
                                    self.state.emit("    fldt (%rcx)");
                                }
                                SlotAddr::Indirect(slot) => {
                                    self.emit_load_ptr_from_slot_impl(slot, ptr_id);
                                    self.state.emit("    fldt (%rcx)");
                                }
                            }
                            self.emit_epilogue_and_ret_impl(frame_size);
                            return;
                        }
                    }
                }
                if let Operand::Const(IrConst::LongDouble(_, f128_bytes)) = val {
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
                    self.emit_epilogue_and_ret_impl(frame_size);
                    return;
                }
            }
        }
        crate::backend::traits::emit_return_default(self, val, frame_size);
    }

    pub(super) fn current_return_type_impl(&self) -> IrType {
        self.current_return_type
    }

    pub(super) fn emit_return_f32_to_reg_impl(&mut self) {
        self.state.emit("    movd %eax, %xmm0");
    }

    pub(super) fn emit_return_f64_to_reg_impl(&mut self) {
        self.state.emit("    movq %rax, %xmm0");
    }

    pub(super) fn emit_return_i128_to_regs_impl(&mut self) {
        // For pure INTEGER+INTEGER returns, rax:rdx already correct — noop.
        // For mixed INTEGER+SSE or SSE+INTEGER returns, move the SSE eightbyte
        // from the GP register to xmm0 per SysV AMD64 ABI.
        // Use func_ret_classes (set once in prologue) so that intervening call
        // instructions cannot clobber the function's own return classification.
        use crate::common::types::EightbyteClass;
        if self.func_ret_classes.len() == 2 {
            let (c0, c1) = (self.func_ret_classes[0], self.func_ret_classes[1]);
            match (c0, c1) {
                (EightbyteClass::Integer, EightbyteClass::Sse) => {
                    // Second eightbyte is SSE: move rdx -> xmm0
                    self.state.emit("    movq %rdx, %xmm0");
                }
                (EightbyteClass::Sse, EightbyteClass::Integer) => {
                    // First eightbyte is SSE: rax -> xmm0, rdx -> rax
                    self.state.emit("    movq %rax, %xmm0");
                    self.state.emit("    movq %rdx, %rax");
                }
                (EightbyteClass::Sse, EightbyteClass::Sse) => {
                    // Both SSE: rax -> xmm0, rdx -> xmm1
                    self.state.emit("    movq %rax, %xmm0");
                    self.state.emit("    movq %rdx, %xmm1");
                }
                _ => {} // INTEGER+INTEGER: already correct
            }
        }
    }

    pub(super) fn emit_return_f128_to_reg_impl(&mut self) {
        // F128 (long double) must be returned in x87 st(0) per SysV ABI.
        // rax has f64 bit pattern; push to stack, load with fldl.
        self.state.emit("    pushq %rax");
        self.state.emit("    fldl (%rsp)");
        self.state.emit("    addq $8, %rsp");
    }

    pub(super) fn emit_return_int_to_reg_impl(&mut self) {
        // rax already holds the return value per SysV ABI — noop
    }

    pub(super) fn emit_get_return_f64_second_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.state
                .out
                .emit_instr_reg_rbp("    movsd", "xmm1", slot.0);
        }
    }

    pub(super) fn emit_set_return_f64_second_impl(&mut self, src: &Operand) {
        match src {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    self.state
                        .out
                        .emit_instr_rbp_reg("    movsd", slot.0, "xmm1");
                }
            }
            Operand::Const(IrConst::F64(f)) => {
                let bits = f.to_bits();
                self.state
                    .out
                    .emit_instr_imm_reg("    movabsq", bits as i64, "rax");
                self.state.emit("    movq %rax, %xmm1");
                self.state.reg_cache.invalidate_all();
            }
            _ => {
                self.operand_to_rax(src);
                self.state.emit("    movq %rax, %xmm1");
                self.state.reg_cache.invalidate_all();
            }
        }
    }

    pub(super) fn emit_get_return_f32_second_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.state
                .out
                .emit_instr_reg_rbp("    movss", "xmm1", slot.0);
        }
    }

    pub(super) fn emit_set_return_f32_second_impl(&mut self, src: &Operand) {
        match src {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    self.state
                        .out
                        .emit_instr_rbp_reg("    movss", slot.0, "xmm1");
                }
            }
            Operand::Const(IrConst::F32(f)) => {
                let bits = f.to_bits();
                self.state
                    .out
                    .emit_instr_imm_reg("    movl", bits as i64, "eax");
                self.state.emit("    movd %eax, %xmm1");
                self.state.reg_cache.invalidate_all();
            }
            _ => {
                self.operand_to_rax(src);
                self.state.emit("    movd %eax, %xmm1");
                self.state.reg_cache.invalidate_all();
            }
        }
    }

    pub(super) fn emit_get_return_f128_second_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.state.out.emit_instr_rbp("    fstpt", slot.0);
            self.state.out.emit_instr_rbp("    fldt", slot.0);
            self.state.emit("    subq $8, %rsp");
            self.state.emit("    fstpl (%rsp)");
            self.state.emit("    popq %rax");
            self.state.reg_cache.set_acc(dest.0, false);
            self.state.f128_direct_slots.insert(dest.0);
        } else {
            self.state.emit("    fstp %st(0)");
        }
    }

    pub(super) fn emit_set_return_f128_second_impl(&mut self, src: &Operand) {
        match src {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    self.state.out.emit_instr_rbp("    fldt", slot.0);
                }
            }
            _ => {
                self.operand_to_rax(src);
                self.state.emit("    pushq %rax");
                self.state.emit("    fildq (%rsp)");
                self.state.emit("    addq $8, %rsp");
            }
        }
    }
}
