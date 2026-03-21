//! I686Codegen: 128-bit (i128) and 64-bit pair operations.
//!
//! On i686, "i128" operations actually operate on 64-bit values using eax:edx pairs.
//! This module also contains the i64 bit-manipulation helpers.

use super::emit::I686Codegen;
use crate::backend::state::StackSlot;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{IrCmpOp, IrConst, Operand, Value};

impl I686Codegen {
    pub(super) fn emit_sign_extend_acc_high_impl(&mut self) {
        self.state.emit("    cltd");
    }

    pub(super) fn emit_zero_acc_high_impl(&mut self) {
        self.state.emit("    xorl %edx, %edx");
    }

    pub(super) fn emit_load_acc_pair_impl(&mut self, op: &Operand) {
        match op {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    let sr0 = self.slot_ref(slot);
                    let sr4 = self.slot_ref_offset(slot, 4);
                    emit!(self.state, "    movl {}, %eax", sr0);
                    emit!(self.state, "    movl {}, %edx", sr4);
                } else if let Some(phys) = self.reg_assignments.get(&v.0).copied() {
                    let reg = super::emit::phys_reg_name(phys);
                    emit!(self.state, "    movl %{}, %eax", reg);
                    self.state.emit("    xorl %edx, %edx");
                }
            }
            Operand::Const(IrConst::I128(v)) => {
                let low = (*v & 0xFFFFFFFF) as i32;
                let high = ((*v >> 32) & 0xFFFFFFFF) as i32;
                emit!(self.state, "    movl ${}, %eax", low);
                emit!(self.state, "    movl ${}, %edx", high);
            }
            Operand::Const(IrConst::I64(v)) => {
                let low = (*v & 0xFFFFFFFF) as i32;
                let high = ((*v >> 32) & 0xFFFFFFFF) as i32;
                emit!(self.state, "    movl ${}, %eax", low);
                emit!(self.state, "    movl ${}, %edx", high);
            }
            Operand::Const(IrConst::F64(f)) => {
                let bits = f.to_bits();
                let low = (bits & 0xFFFFFFFF) as i32;
                let high = (bits >> 32) as i32;
                emit!(self.state, "    movl ${}, %eax", low);
                emit!(self.state, "    movl ${}, %edx", high);
            }
            Operand::Const(IrConst::Zero) => {
                self.state.emit("    xorl %eax, %eax");
                self.state.emit("    xorl %edx, %edx");
            }
            Operand::Const(c)
                if matches!(c, IrConst::I8(_) | IrConst::I16(_) | IrConst::I32(_)) =>
            {
                if let Some(ext) = c.to_i64() {
                    let low = (ext & 0xFFFFFFFF) as i32;
                    let high = ((ext >> 32) & 0xFFFFFFFF) as i32;
                    emit!(self.state, "    movl ${}, %eax", low);
                    emit!(self.state, "    movl ${}, %edx", high);
                }
            }
            _ => {
                self.operand_to_eax(op);
                self.state.emit("    xorl %edx, %edx");
            }
        }
    }

    pub(super) fn emit_store_acc_pair_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr0 = self.slot_ref(slot);
            let sr4 = self.slot_ref_offset(slot, 4);
            emit!(self.state, "    movl %eax, {}", sr0);
            emit!(self.state, "    movl %edx, {}", sr4);
        }
    }

    pub(super) fn emit_store_pair_to_slot_impl(&mut self, slot: StackSlot) {
        let sr0 = self.slot_ref(slot);
        let sr4 = self.slot_ref_offset(slot, 4);
        emit!(self.state, "    movl %eax, {}", sr0);
        emit!(self.state, "    movl %edx, {}", sr4);
    }

    pub(super) fn emit_load_pair_from_slot_impl(&mut self, slot: StackSlot) {
        let sr0 = self.slot_ref(slot);
        let sr4 = self.slot_ref_offset(slot, 4);
        emit!(self.state, "    movl {}, %eax", sr0);
        emit!(self.state, "    movl {}, %edx", sr4);
    }

    pub(super) fn emit_save_acc_pair_impl(&mut self) {
        self.state.emit("    movl %eax, %esi");
        self.state.emit("    movl %edx, %edi");
    }

    pub(super) fn emit_store_pair_indirect_impl(&mut self) {
        self.state.emit("    movl %esi, (%ecx)");
        self.state.emit("    movl %edi, 4(%ecx)");
    }

    pub(super) fn emit_load_pair_indirect_impl(&mut self) {
        self.state.emit("    movl (%ecx), %eax");
        self.state.emit("    movl 4(%ecx), %edx");
    }

    pub(super) fn emit_i128_neg_impl(&mut self) {
        self.state.emit("    notl %eax");
        self.state.emit("    notl %edx");
        self.state.emit("    addl $1, %eax");
        self.state.emit("    adcl $0, %edx");
    }

    pub(super) fn emit_i128_not_impl(&mut self) {
        self.state.emit("    notl %eax");
        self.state.emit("    notl %edx");
    }

    pub(super) fn emit_i128_to_float_call_impl(
        &mut self,
        src: &Operand,
        from_signed: bool,
        to_ty: IrType,
    ) {
        self.emit_load_acc_pair(src);
        if from_signed {
            self.state.emit("    pushl %edx");
            self.state.emit("    pushl %eax");
            self.state.emit("    fildq (%esp)");
            if to_ty == IrType::F32 {
                self.state.emit("    fstps (%esp)");
                self.state.emit("    movl (%esp), %eax");
            } else {
                self.state.emit("    fstpl (%esp)");
                self.state.emit("    movl (%esp), %eax");
            }
            self.state.emit("    addl $8, %esp");
        } else {
            let label_id = self.state.next_label_id();
            let big_label = format!(".Lu64_to_f_big_{}", label_id);
            let done_label = format!(".Lu64_to_f_done_{}", label_id);

            self.state.emit("    pushl %edx");
            self.state.emit("    pushl %eax");
            self.state.emit("    testl %edx, %edx");
            emit!(self.state, "    js {}", big_label);
            self.state.emit("    fildq (%esp)");
            emit!(self.state, "    jmp {}", done_label);
            emit!(self.state, "{}:", big_label);
            self.state.emit("    movl (%esp), %eax");
            self.state.emit("    movl 4(%esp), %edx");
            self.state.emit("    shrl $1, %eax");
            self.state.emit("    movl %edx, %ecx");
            self.state.emit("    shrl $1, %edx");
            self.state.emit("    andl $1, %ecx");
            self.state.emit("    shll $31, %ecx");
            self.state.emit("    orl %ecx, %eax");
            self.state.emit("    movl %eax, (%esp)");
            self.state.emit("    movl %edx, 4(%esp)");
            self.state.emit("    fildq (%esp)");
            self.state.emit("    fadd %st(0), %st(0)");
            emit!(self.state, "{}:", done_label);
            if to_ty == IrType::F32 {
                self.state.emit("    fstps (%esp)");
                self.state.emit("    movl (%esp), %eax");
            } else {
                self.state.emit("    fstpl (%esp)");
                self.state.emit("    movl (%esp), %eax");
            }
            self.state.emit("    addl $8, %esp");
        }
    }

    pub(super) fn emit_float_to_i128_call_impl(
        &mut self,
        src: &Operand,
        _to_signed: bool,
        _from_ty: IrType,
    ) {
        // TODO: F64 should use fldl instead of flds, and unsigned conversion
        // may need different handling for values exceeding i64 range.
        self.operand_to_eax(src);
        self.state.emit("    subl $8, %esp");
        self.state.emit("    movl %eax, (%esp)");
        self.state.emit("    flds (%esp)");
        self.state.emit("    fisttpq (%esp)");
        self.state.emit("    movl (%esp), %eax");
        self.state.emit("    movl 4(%esp), %edx");
        self.state.emit("    addl $8, %esp");
    }

    pub(super) fn emit_i128_prep_binop_impl(&mut self, lhs: &Operand, rhs: &Operand) {
        self.emit_load_acc_pair(rhs);
        self.state.emit("    pushl %edx");
        self.esp_adjust += 4;
        self.state.emit("    pushl %eax");
        self.esp_adjust += 4;
        self.emit_load_acc_pair(lhs);
    }

    pub(super) fn emit_i128_prep_shift_lhs_impl(&mut self, lhs: &Operand) {
        self.emit_load_acc_pair(lhs);
    }

    pub(super) fn emit_i128_add_impl(&mut self) {
        self.state.emit("    addl (%esp), %eax");
        self.state.emit("    adcl 4(%esp), %edx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_sub_impl(&mut self) {
        self.state.emit("    subl (%esp), %eax");
        self.state.emit("    sbbl 4(%esp), %edx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_mul_impl(&mut self) {
        self.state.emit("    movl %edx, %ecx");
        self.state.emit("    imull (%esp), %ecx");
        self.state.emit("    movl %eax, %edx");
        self.state.emit("    imull 4(%esp), %edx");
        self.state.emit("    addl %edx, %ecx");
        self.state.emit("    mull (%esp)");
        self.state.emit("    addl %ecx, %edx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_and_impl(&mut self) {
        self.state.emit("    andl (%esp), %eax");
        self.state.emit("    andl 4(%esp), %edx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_or_impl(&mut self) {
        self.state.emit("    orl (%esp), %eax");
        self.state.emit("    orl 4(%esp), %edx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_xor_impl(&mut self) {
        self.state.emit("    xorl (%esp), %eax");
        self.state.emit("    xorl 4(%esp), %edx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_shl_impl(&mut self) {
        let label_id = self.state.next_label_id();
        let done_label = format!(".Lshl64_done_{}", label_id);
        self.state.emit("    movl (%esp), %ecx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
        self.state.emit("    shldl %cl, %eax, %edx");
        self.state.emit("    shll %cl, %eax");
        self.state.emit("    testb $32, %cl");
        emit!(self.state, "    je {}", done_label);
        self.state.emit("    movl %eax, %edx");
        self.state.emit("    xorl %eax, %eax");
        emit!(self.state, "{}:", done_label);
    }

    pub(super) fn emit_i128_lshr_impl(&mut self) {
        let label_id = self.state.next_label_id();
        let done_label = format!(".Llshr64_done_{}", label_id);
        self.state.emit("    movl (%esp), %ecx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
        self.state.emit("    shrdl %cl, %edx, %eax");
        self.state.emit("    shrl %cl, %edx");
        self.state.emit("    testb $32, %cl");
        emit!(self.state, "    je {}", done_label);
        self.state.emit("    movl %edx, %eax");
        self.state.emit("    xorl %edx, %edx");
        emit!(self.state, "{}:", done_label);
    }

    pub(super) fn emit_i128_ashr_impl(&mut self) {
        let label_id = self.state.next_label_id();
        let done_label = format!(".Lashr64_done_{}", label_id);
        self.state.emit("    movl (%esp), %ecx");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
        self.state.emit("    shrdl %cl, %edx, %eax");
        self.state.emit("    sarl %cl, %edx");
        self.state.emit("    testb $32, %cl");
        emit!(self.state, "    je {}", done_label);
        self.state.emit("    movl %edx, %eax");
        self.state.emit("    sarl $31, %edx");
        emit!(self.state, "{}:", done_label);
    }

    pub(super) fn emit_i128_divrem_call_impl(
        &mut self,
        func_name: &str,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        let di_func = match func_name {
            "__divti3" => "__divdi3",
            "__udivti3" => "__udivdi3",
            "__modti3" => "__moddi3",
            "__umodti3" => "__umoddi3",
            _ => func_name,
        };

        self.state.needs_divdi3_helpers = true;

        self.emit_load_acc_pair(rhs);
        self.state.emit("    pushl %edx");
        self.esp_adjust += 4;
        self.state.emit("    pushl %eax");
        self.esp_adjust += 4;
        self.emit_load_acc_pair(lhs);
        self.state.emit("    pushl %edx");
        self.state.emit("    pushl %eax");
        if self.state.needs_plt(di_func) {
            emit!(self.state, "    call {}@PLT", di_func);
        } else {
            emit!(self.state, "    call {}", di_func);
        }
        self.state.emit("    addl $16, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_store_result_impl(&mut self, dest: &Value) {
        self.emit_store_acc_pair(dest);
    }

    pub(super) fn emit_i128_shl_const_impl(&mut self, amount: u32) {
        if amount == 0 {
            return;
        }
        if amount >= 64 {
            self.state.emit("    xorl %eax, %eax");
            self.state.emit("    xorl %edx, %edx");
        } else if amount >= 32 {
            self.state.emit("    movl %eax, %edx");
            self.state.emit("    xorl %eax, %eax");
            if amount > 32 {
                emit!(self.state, "    shll ${}, %edx", amount - 32);
            }
        } else {
            emit!(self.state, "    shldl ${}, %eax, %edx", amount);
            emit!(self.state, "    shll ${}, %eax", amount);
        }
    }

    pub(super) fn emit_i128_lshr_const_impl(&mut self, amount: u32) {
        if amount == 0 {
            return;
        }
        if amount >= 64 {
            self.state.emit("    xorl %eax, %eax");
            self.state.emit("    xorl %edx, %edx");
        } else if amount >= 32 {
            self.state.emit("    movl %edx, %eax");
            self.state.emit("    xorl %edx, %edx");
            if amount > 32 {
                emit!(self.state, "    shrl ${}, %eax", amount - 32);
            }
        } else {
            emit!(self.state, "    shrdl ${}, %edx, %eax", amount);
            emit!(self.state, "    shrl ${}, %edx", amount);
        }
    }

    pub(super) fn emit_i128_ashr_const_impl(&mut self, amount: u32) {
        if amount == 0 {
            return;
        }
        if amount >= 64 {
            self.state.emit("    sarl $31, %edx");
            self.state.emit("    movl %edx, %eax");
        } else if amount >= 32 {
            self.state.emit("    movl %edx, %eax");
            self.state.emit("    sarl $31, %edx");
            if amount > 32 {
                emit!(self.state, "    sarl ${}, %eax", amount - 32);
            }
        } else {
            emit!(self.state, "    shrdl ${}, %edx, %eax", amount);
            emit!(self.state, "    sarl ${}, %edx", amount);
        }
    }

    pub(super) fn emit_i128_cmp_eq_impl(&mut self, is_ne: bool) {
        self.state.emit("    cmpl (%esp), %eax");
        self.state.emit("    sete %al");
        self.state.emit("    cmpl 4(%esp), %edx");
        self.state.emit("    sete %cl");
        self.state.emit("    andb %cl, %al");
        if is_ne {
            self.state.emit("    xorb $1, %al");
        }
        self.state.emit("    movzbl %al, %eax");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_cmp_ordered_impl(&mut self, op: IrCmpOp) {
        let is_signed = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge
        );

        if is_signed {
            let label_id = self.state.next_label_id();
            let label_hi_decided = format!(".Li128_hidec_{}", label_id);
            let label_done = format!(".Li128_done_{}", label_id);

            self.state.emit("    cmpl 4(%esp), %edx");
            emit!(self.state, "    jne {}", label_hi_decided);

            self.state.emit("    cmpl (%esp), %eax");
            let low_set = match op {
                IrCmpOp::Slt => "setb",
                IrCmpOp::Sle => "setbe",
                IrCmpOp::Sgt => "seta",
                IrCmpOp::Sge => "setae",
                _ => unreachable!("signed i64 low-word cmp got non-signed op: {:?}", op),
            };
            emit!(self.state, "    {} %al", low_set);
            emit!(self.state, "    jmp {}", label_done);

            emit!(self.state, "{}:", label_hi_decided);
            let high_set = match op {
                IrCmpOp::Slt => "setl",
                IrCmpOp::Sle => "setl",
                IrCmpOp::Sgt => "setg",
                IrCmpOp::Sge => "setg",
                _ => unreachable!("signed i64 high-word cmp got non-signed op: {:?}", op),
            };
            emit!(self.state, "    {} %al", high_set);

            emit!(self.state, "{}:", label_done);
        } else {
            let label_id = self.state.next_label_id();
            let high_decided = format!(".Li128_high_{}", label_id);

            self.state.emit("    cmpl 4(%esp), %edx");
            emit!(self.state, "    jne {}", high_decided);
            self.state.emit("    cmpl (%esp), %eax");
            emit!(self.state, "{}:", high_decided);

            let set_instr = match op {
                IrCmpOp::Ult => "setb",
                IrCmpOp::Ule => "setbe",
                IrCmpOp::Ugt => "seta",
                IrCmpOp::Uge => "setae",
                _ => unreachable!("unsigned i64 cmp got non-unsigned op: {:?}", op),
            };
            emit!(self.state, "    {} %al", set_instr);
        }
        self.state.emit("    movzbl %al, %eax");
        self.state.emit("    addl $8, %esp");
        self.esp_adjust -= 8;
    }

    pub(super) fn emit_i128_cmp_store_result_impl(&mut self, dest: &Value) {
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }
}
