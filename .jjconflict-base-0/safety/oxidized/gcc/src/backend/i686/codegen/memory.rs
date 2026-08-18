//! I686Codegen: memory operations (load, store, memcpy, GEP, stack).

use super::emit::{phys_reg_name, I686Codegen};
use crate::backend::state::{SlotAddr, StackSlot};
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{Operand, Value};

impl I686Codegen {
    // ---- Store/Load overrides ----

    pub(super) fn emit_store_impl(&mut self, val: &Operand, ptr: &Value, ty: IrType) {
        if ty == IrType::F128 {
            self.emit_f128_load_to_x87(val);
            let addr = self.state.resolve_slot_addr(ptr.0);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.emit_alloca_aligned_addr(slot, id);
                        self.state.emit("    fstpt (%ecx)");
                    }
                    SlotAddr::Direct(slot) => {
                        let sr = self.slot_ref(slot);
                        emit!(self.state, "    fstpt {}", sr);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.emit_load_ptr_from_slot(slot, ptr.0);
                        self.state.emit("    fstpt (%ecx)");
                    }
                }
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        if ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64 {
            let addr = self.state.resolve_slot_addr(ptr.0);
            self.emit_load_acc_pair(val);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.state.emit("    pushl %edx");
                        self.esp_adjust += 4;
                        self.state.emit("    pushl %eax");
                        self.esp_adjust += 4;
                        self.emit_alloca_aligned_addr(slot, id);
                        self.state.emit("    popl %eax");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %eax, (%ecx)");
                        self.state.emit("    popl %edx");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %edx, 4(%ecx)");
                    }
                    SlotAddr::Direct(slot) => {
                        let sr0 = self.slot_ref(slot);
                        let sr4 = self.slot_ref_offset(slot, 4);
                        emit!(self.state, "    movl %eax, {}", sr0);
                        emit!(self.state, "    movl %edx, {}", sr4);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.state.emit("    pushl %edx");
                        self.esp_adjust += 4;
                        self.state.emit("    pushl %eax");
                        self.esp_adjust += 4;
                        self.emit_load_ptr_from_slot(slot, ptr.0);
                        self.state.emit("    popl %eax");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %eax, (%ecx)");
                        self.state.emit("    popl %edx");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %edx, 4(%ecx)");
                    }
                }
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        crate::backend::traits::emit_store_default(self, val, ptr, ty);
    }

    pub(super) fn emit_load_impl(&mut self, dest: &Value, ptr: &Value, ty: IrType) {
        if ty == IrType::F128 {
            let addr = self.state.resolve_slot_addr(ptr.0);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.emit_alloca_aligned_addr(slot, id);
                        self.state.emit("    fldt (%ecx)");
                    }
                    SlotAddr::Direct(slot) => {
                        let sr = self.slot_ref(slot);
                        emit!(self.state, "    fldt {}", sr);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.emit_load_ptr_from_slot(slot, ptr.0);
                        self.state.emit("    fldt (%ecx)");
                    }
                }
                if let Some(dest_slot) = self.state.get_slot(dest.0) {
                    let sr = self.slot_ref(dest_slot);
                    emit!(self.state, "    fstpt {}", sr);
                    self.state.f128_direct_slots.insert(dest.0);
                }
            }
            return;
        }
        if ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64 {
            let addr = self.state.resolve_slot_addr(ptr.0);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.emit_alloca_aligned_addr(slot, id);
                        self.state.emit("    movl (%ecx), %eax");
                        self.state.emit("    movl 4(%ecx), %edx");
                    }
                    SlotAddr::Direct(slot) => {
                        let sr0 = self.slot_ref(slot);
                        let sr4 = self.slot_ref_offset(slot, 4);
                        emit!(self.state, "    movl {}, %eax", sr0);
                        emit!(self.state, "    movl {}, %edx", sr4);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.emit_load_ptr_from_slot(slot, ptr.0);
                        self.state.emit("    movl (%ecx), %eax");
                        self.state.emit("    movl 4(%ecx), %edx");
                    }
                }
                self.emit_store_acc_pair(dest);
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        crate::backend::traits::emit_load_default(self, dest, ptr, ty);
    }

    pub(super) fn emit_store_with_const_offset_impl(
        &mut self,
        val: &Operand,
        base: &Value,
        offset: i64,
        ty: IrType,
    ) {
        if ty == IrType::F128 {
            self.emit_f128_load_to_x87(val);
            let addr = self.state.resolve_slot_addr(base.0);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.emit_alloca_aligned_addr(slot, id);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    fstpt (%ecx)");
                    }
                    SlotAddr::Direct(slot) => {
                        let folded_slot = StackSlot(slot.0 + offset);
                        let sr = self.slot_ref(folded_slot);
                        emit!(self.state, "    fstpt {}", sr);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.emit_load_ptr_from_slot(slot, base.0);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    fstpt (%ecx)");
                    }
                }
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        if ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64 {
            self.emit_load_acc_pair(val);
            let addr = self.state.resolve_slot_addr(base.0);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.state.emit("    pushl %edx");
                        self.esp_adjust += 4;
                        self.state.emit("    pushl %eax");
                        self.esp_adjust += 4;
                        self.emit_alloca_aligned_addr(slot, id);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    popl %eax");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %eax, (%ecx)");
                        self.state.emit("    popl %edx");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %edx, 4(%ecx)");
                    }
                    SlotAddr::Direct(slot) => {
                        let folded_slot = StackSlot(slot.0 + offset);
                        let sr0 = self.slot_ref(folded_slot);
                        let sr4 = self.slot_ref_offset(folded_slot, 4);
                        emit!(self.state, "    movl %eax, {}", sr0);
                        emit!(self.state, "    movl %edx, {}", sr4);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.state.emit("    pushl %edx");
                        self.esp_adjust += 4;
                        self.state.emit("    pushl %eax");
                        self.esp_adjust += 4;
                        self.emit_load_ptr_from_slot(slot, base.0);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    popl %eax");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %eax, (%ecx)");
                        self.state.emit("    popl %edx");
                        self.esp_adjust -= 4;
                        self.state.emit("    movl %edx, 4(%ecx)");
                    }
                }
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        // Delegate to default for other types
        self.operand_to_eax(val);
        let addr = self.state.resolve_slot_addr(base.0);
        if let Some(addr) = addr {
            let store_instr = self.store_instr_for_type(ty);
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    self.emit_save_acc();
                    self.emit_alloca_aligned_addr(slot, id);
                    self.emit_add_offset_to_addr_reg(offset);
                    self.emit_typed_store_indirect(store_instr, ty);
                }
                SlotAddr::Direct(slot) => {
                    let folded_slot = StackSlot(slot.0 + offset);
                    self.emit_typed_store_to_slot(store_instr, ty, folded_slot);
                }
                SlotAddr::Indirect(slot) => {
                    self.emit_save_acc();
                    self.emit_load_ptr_from_slot(slot, base.0);
                    if offset != 0 {
                        self.emit_add_offset_to_addr_reg(offset);
                    }
                    self.emit_typed_store_indirect(store_instr, ty);
                }
            }
        }
    }

    pub(super) fn emit_load_with_const_offset_impl(
        &mut self,
        dest: &Value,
        base: &Value,
        offset: i64,
        ty: IrType,
    ) {
        if ty == IrType::F128 {
            let addr = self.state.resolve_slot_addr(base.0);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.emit_alloca_aligned_addr(slot, id);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    fldt (%ecx)");
                    }
                    SlotAddr::Direct(slot) => {
                        let folded_slot = StackSlot(slot.0 + offset);
                        let sr = self.slot_ref(folded_slot);
                        emit!(self.state, "    fldt {}", sr);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.emit_load_ptr_from_slot(slot, base.0);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    fldt (%ecx)");
                    }
                }
                if let Some(dest_slot) = self.state.get_slot(dest.0) {
                    let sr = self.slot_ref(dest_slot);
                    emit!(self.state, "    fstpt {}", sr);
                    self.state.f128_direct_slots.insert(dest.0);
                }
            }
            return;
        }
        if ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64 {
            let addr = self.state.resolve_slot_addr(base.0);
            if let Some(addr) = addr {
                match addr {
                    SlotAddr::OverAligned(slot, id) => {
                        self.emit_alloca_aligned_addr(slot, id);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    movl (%ecx), %eax");
                        self.state.emit("    movl 4(%ecx), %edx");
                    }
                    SlotAddr::Direct(slot) => {
                        let folded_slot = StackSlot(slot.0 + offset);
                        let sr0 = self.slot_ref(folded_slot);
                        let sr4 = self.slot_ref_offset(folded_slot, 4);
                        emit!(self.state, "    movl {}, %eax", sr0);
                        emit!(self.state, "    movl {}, %edx", sr4);
                    }
                    SlotAddr::Indirect(slot) => {
                        self.emit_load_ptr_from_slot(slot, base.0);
                        if offset != 0 {
                            self.emit_add_offset_to_addr_reg(offset);
                        }
                        self.state.emit("    movl (%ecx), %eax");
                        self.state.emit("    movl 4(%ecx), %edx");
                    }
                }
                self.emit_store_acc_pair(dest);
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        // Delegate to default for other types
        let addr = self.state.resolve_slot_addr(base.0);
        if let Some(addr) = addr {
            let load_instr = self.load_instr_for_type(ty);
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    self.emit_alloca_aligned_addr(slot, id);
                    self.emit_add_offset_to_addr_reg(offset);
                    self.emit_typed_load_indirect(load_instr);
                }
                SlotAddr::Direct(slot) => {
                    let folded_slot = StackSlot(slot.0 + offset);
                    self.emit_typed_load_from_slot(load_instr, folded_slot);
                }
                SlotAddr::Indirect(slot) => {
                    self.emit_load_ptr_from_slot(slot, base.0);
                    if offset != 0 {
                        self.emit_add_offset_to_addr_reg(offset);
                    }
                    self.emit_typed_load_indirect(load_instr);
                }
            }
            self.emit_store_result(dest);
        }
    }

    // ---- Typed store/load helpers ----

    pub(super) fn emit_typed_store_to_slot_impl(
        &mut self,
        instr: &'static str,
        ty: IrType,
        slot: StackSlot,
    ) {
        let reg = self.eax_for_type(ty);
        let sr = self.slot_ref(slot);
        emit!(self.state, "    {} {}, {}", instr, reg, sr);
    }

    pub(super) fn emit_typed_load_from_slot_impl(&mut self, instr: &'static str, slot: StackSlot) {
        let sr = self.slot_ref(slot);
        emit!(self.state, "    {} {}, %eax", instr, sr);
    }

    pub(super) fn emit_load_ptr_from_slot_impl(&mut self, slot: StackSlot, val_id: u32) {
        if let Some(phys) = self.reg_assignments.get(&val_id).copied() {
            let reg = phys_reg_name(phys);
            emit!(self.state, "    movl %{}, %ecx", reg);
        } else {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    movl {}, %ecx", sr);
        }
    }

    pub(super) fn emit_typed_store_indirect_impl(&mut self, instr: &'static str, ty: IrType) {
        let reg = match ty {
            IrType::I8 | IrType::U8 => "%dl",
            IrType::I16 | IrType::U16 => "%dx",
            _ => "%edx",
        };
        emit!(self.state, "    {} {}, (%ecx)", instr, reg);
    }

    pub(super) fn emit_typed_load_indirect_impl(&mut self, instr: &'static str) {
        emit!(self.state, "    {} (%ecx), %eax", instr);
    }

    pub(super) fn emit_add_offset_to_addr_reg_impl(&mut self, offset: i64) {
        if offset != 0 {
            emit!(self.state, "    addl ${}, %ecx", offset as i32);
        }
    }

    // ---- GEP primitives ----

    /// Compute the address of an alloca into `reg`, handling over-aligned allocas.
    pub(super) fn emit_alloca_addr_to(&mut self, reg: &str, val_id: u32, slot: StackSlot) {
        let sr = self.slot_ref(slot);
        if let Some(align) = self.state.alloca_over_align(val_id) {
            emit!(self.state, "    leal {}, %{}", sr, reg);
            emit!(self.state, "    addl ${}, %{}", align - 1, reg);
            emit!(self.state, "    andl ${}, %{}", -(align as i32), reg);
        } else {
            emit!(self.state, "    leal {}, %{}", sr, reg);
        }
    }

    pub(super) fn emit_slot_addr_to_secondary_impl(
        &mut self,
        slot: StackSlot,
        is_alloca: bool,
        val_id: u32,
    ) {
        if is_alloca {
            self.emit_alloca_addr_to("ecx", val_id, slot);
        } else if let Some(phys) = self.reg_assignments.get(&val_id).copied() {
            let reg = phys_reg_name(phys);
            emit!(self.state, "    movl %{}, %ecx", reg);
        } else {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    movl {}, %ecx", sr);
        }
    }

    pub(super) fn emit_gep_direct_const_impl(&mut self, slot: StackSlot, offset: i64) {
        let folded_slot = StackSlot(slot.0 + offset);
        let sr = self.slot_ref(folded_slot);
        emit!(self.state, "    leal {}, %eax", sr);
    }

    pub(super) fn emit_gep_indirect_const_impl(
        &mut self,
        slot: StackSlot,
        offset: i64,
        val_id: u32,
    ) {
        if let Some(phys) = self.reg_assignments.get(&val_id).copied() {
            let reg = phys_reg_name(phys);
            if offset == 0 {
                emit!(self.state, "    movl %{}, %eax", reg);
            } else {
                emit!(self.state, "    leal {}(%{}), %eax", offset, reg);
            }
        } else {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    movl {}, %eax", sr);
            if offset != 0 {
                emit!(self.state, "    addl ${}, %eax", offset as i32);
            }
        }
    }

    // ---- Dynamic alloca ----

    pub(super) fn emit_add_imm_to_acc_impl(&mut self, imm: i64) {
        emit!(self.state, "    addl ${}, %eax", imm as i32);
    }

    pub(super) fn emit_round_up_acc_to_16_impl(&mut self) {
        self.state.emit("    addl $15, %eax");
        self.state.emit("    andl $-16, %eax");
    }

    pub(super) fn emit_sub_sp_by_acc_impl(&mut self) {
        self.state.emit("    subl %eax, %esp");
    }

    pub(super) fn emit_mov_sp_to_acc_impl(&mut self) {
        self.state.emit("    movl %esp, %eax");
        self.state.reg_cache.invalidate_acc();
    }

    pub(super) fn emit_mov_acc_to_sp_impl(&mut self) {
        self.state.emit("    movl %eax, %esp");
    }

    pub(super) fn emit_align_acc_impl(&mut self, align: usize) {
        emit!(self.state, "    addl ${}, %eax", align - 1);
        emit!(self.state, "    andl ${}, %eax", -(align as i32));
    }

    // ---- Alloca aligned addr ----

    pub(super) fn emit_alloca_aligned_addr_impl(&mut self, slot: StackSlot, val_id: u32) {
        let align = self
            .state
            .alloca_over_align(val_id)
            .expect("alloca must have over-alignment for aligned addr emission");
        let sr = self.slot_ref(slot);
        emit!(self.state, "    leal {}, %ecx", sr);
        emit!(self.state, "    addl ${}, %ecx", align - 1);
        emit!(self.state, "    andl ${}, %ecx", -(align as i32));
    }

    pub(super) fn emit_alloca_aligned_addr_to_acc_impl(&mut self, slot: StackSlot, val_id: u32) {
        let align = self
            .state
            .alloca_over_align(val_id)
            .expect("alloca must have over-alignment for aligned addr emission");
        let sr = self.slot_ref(slot);
        emit!(self.state, "    leal {}, %eax", sr);
        emit!(self.state, "    addl ${}, %eax", align - 1);
        emit!(self.state, "    andl ${}, %eax", -(align as i32));
        self.state.reg_cache.invalidate_acc();
    }

    // ---- Memcpy ----

    pub(super) fn emit_memcpy_load_dest_addr_impl(
        &mut self,
        slot: StackSlot,
        is_alloca: bool,
        val_id: u32,
    ) {
        if is_alloca {
            self.emit_alloca_addr_to("edi", val_id, slot);
        } else if let Some(phys) = self.reg_assignments.get(&val_id).copied() {
            let reg = phys_reg_name(phys);
            emit!(self.state, "    movl %{}, %edi", reg);
        } else {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    movl {}, %edi", sr);
        }
    }

    pub(super) fn emit_memcpy_load_src_addr_impl(
        &mut self,
        slot: StackSlot,
        is_alloca: bool,
        val_id: u32,
    ) {
        if is_alloca {
            self.emit_alloca_addr_to("esi", val_id, slot);
        } else if let Some(phys) = self.reg_assignments.get(&val_id).copied() {
            let reg = phys_reg_name(phys);
            emit!(self.state, "    movl %{}, %esi", reg);
        } else {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    movl {}, %esi", sr);
        }
    }

    pub(super) fn emit_memcpy_impl_impl(&mut self, size: usize) {
        emit!(self.state, "    movl ${}, %ecx", size);
        self.state.emit("    rep movsb");
    }
}
