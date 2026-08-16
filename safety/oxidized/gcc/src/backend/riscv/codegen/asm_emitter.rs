//! RISC-V InlineAsmEmitter implementation: constraint classification, scratch
//! register allocation, operand loading/storing, and template substitution.

use super::emit::RiscvCodegen;
use super::inline_asm::{classify_rv_constraint, RvConstraintKind};
use crate::backend::inline_asm::{AsmOperand, AsmOperandKind, InlineAsmEmitter};
use crate::backend::state::CodegenState;
use crate::common::types::IrType;
use crate::ir::reexports::{BlockId, IrConst, Operand, Value};

/// RISC-V scratch registers for inline asm operand allocation.
/// The order matters: t-registers first (least likely to conflict), then a-registers.
/// a0/a1 are included as the last resort before overflowing to callee-saved registers,
/// since inline asm with many operands (e.g., 8 outputs + 8 inputs = 16 total) can
/// exhaust the first 13 entries. Without a0/a1, the allocator would overflow to
/// callee-saved registers (s2-s11) which may already be in use by the register
/// allocator for other live values, causing silent clobbering.
const RISCV_GP_SCRATCH: &[&str] = &[
    "t0", "t1", "t2", "t3", "t4", "t5", "t6", "a2", "a3", "a4", "a5", "a6", "a7", "a0", "a1",
];
const RISCV_FP_SCRATCH: &[&str] = &["ft0", "ft1", "ft2", "ft3", "ft4", "ft5", "ft6", "ft7"];

/// Select the correct RISC-V store instruction based on the operand's IR type.
/// On RV64, GP registers are always 64-bit, but sub-64-bit types must use
/// narrower store instructions (sb/sh/sw) to avoid corrupting adjacent memory.
fn gp_store_for_type(ty: IrType) -> &'static str {
    match ty {
        IrType::I8 | IrType::U8 => "sb",
        IrType::I16 | IrType::U16 => "sh",
        IrType::I32 | IrType::U32 => "sw",
        // I64, U64, Ptr, and anything else uses sd (full 64-bit store)
        _ => "sd",
    }
}

/// Find a GP scratch register for storing inline asm output values that doesn't
/// conflict with any output register or the current output being stored.
///
/// Uses a candidate list (t0-t6, a0, a1) to find a register not in the output
/// set. Since outputs are assigned from RISCV_GP_SCRATCH starting at t0, the
/// first 8 outputs use t0-t6 and a2. a0/a1 are at the end of RISCV_GP_SCRATCH
/// so they're unlikely to be output registers unless there are 14+ operands.
///
/// If no safe candidate is found, returns a fallback ("t0" or "t1"). The caller
/// must check whether the returned register is in all_output_regs and use a
/// stack-based spill if so.
fn find_gp_scratch_for_output(current_output_reg: &str, all_output_regs: &[&str]) -> String {
    const CANDIDATES: &[&str] = &["t0", "t1", "t2", "t3", "t4", "t5", "t6", "a0", "a1"];
    for &c in CANDIDATES {
        if !all_output_regs.contains(&c) && c != current_output_reg {
            return c.to_string();
        }
    }
    // Fallback: return the first candidate not equal to current_output_reg.
    // Caller must handle the case where this register is an output (stack spill).
    let fallback = if current_output_reg != "t0" {
        "t0"
    } else {
        "t1"
    };
    fallback.to_string()
}

impl InlineAsmEmitter for RiscvCodegen {
    fn asm_state(&mut self) -> &mut CodegenState {
        &mut self.state
    }

    fn classify_constraint(&self, constraint: &str) -> AsmOperandKind {
        // TODO: RISC-V =@cc not fully implemented — needs SLTU/SEQZ/etc. in store_output_from_reg.
        // Currently stores incorrect results (just a GP register value, no condition capture).
        let c = constraint.trim_start_matches(['=', '+', '&', '%']);
        // Explicit register constraint from register variable: {regname}
        if c.starts_with('{') && c.ends_with('}') {
            let reg_name = &c[1..c.len() - 1];
            return AsmOperandKind::Specific(reg_name.to_string());
        }
        if let Some(cond) = c.strip_prefix("@cc") {
            return AsmOperandKind::ConditionCode(cond.to_string());
        }
        // Map RISC-V's richer constraint types into the shared AsmOperandKind.
        let rv_kind = classify_rv_constraint(constraint);
        match rv_kind {
            RvConstraintKind::GpReg => AsmOperandKind::GpReg,
            RvConstraintKind::FpReg => AsmOperandKind::FpReg,
            RvConstraintKind::Memory => AsmOperandKind::Memory,
            RvConstraintKind::Address => AsmOperandKind::Address,
            RvConstraintKind::Immediate => AsmOperandKind::Immediate,
            RvConstraintKind::ZeroOrReg => AsmOperandKind::ZeroOrReg,
            RvConstraintKind::Specific(reg) => AsmOperandKind::Specific(reg),
            RvConstraintKind::Tied(n) => AsmOperandKind::Tied(n),
        }
    }

    fn setup_operand_metadata(&self, op: &mut AsmOperand, val: &Operand, _is_output: bool) {
        match &op.kind {
            AsmOperandKind::Memory | AsmOperandKind::Address => {
                if let Operand::Value(v) = val {
                    if let Some(slot) = self.state.get_slot(v.0) {
                        if self.state.is_alloca(v.0) {
                            // Alloca: stack slot IS the memory location
                            op.mem_offset = slot.0;
                        } else {
                            // Non-alloca: slot holds a pointer that needs indirection.
                            // Mark with empty mem_addr; resolve_memory_operand will handle it.
                            op.mem_addr = String::new();
                            op.mem_offset = 0;
                        }
                    }
                }
            }
            AsmOperandKind::Immediate => {
                if let Operand::Const(c) = val {
                    op.imm_value = Some(c.to_i64().unwrap_or(0));
                } else {
                    // Value operand for immediate constraint: fall back to GP register
                    op.kind = AsmOperandKind::GpReg;
                }
            }
            AsmOperandKind::ZeroOrReg => {
                // If the value is a constant 0, use "zero" register
                if let Operand::Const(c) = val {
                    if c.to_i64() == Some(0) {
                        op.reg = "zero".to_string();
                    }
                }
            }
            _ => {}
        }
    }

    fn resolve_memory_operand(
        &mut self,
        op: &mut AsmOperand,
        val: &Operand,
        excluded: &[String],
    ) -> bool {
        // For alloca memory operands with large offsets (>2047), the direct
        // `{offset}(s0)` format doesn't fit RISC-V's 12-bit signed immediate.
        // Compute the address into a scratch register instead.
        if op.mem_offset != 0 && !(op.mem_offset >= -2048 && op.mem_offset <= 2047) {
            let tmp_reg = self.assign_scratch_reg(&AsmOperandKind::GpReg, excluded);
            self.emit_addi_s0(&tmp_reg, op.mem_offset);
            op.mem_addr = format!("0({})", tmp_reg);
            op.mem_offset = 0;
            return true;
        }
        // If mem_addr is set or mem_offset is non-zero (alloca case), nothing to do
        if !op.mem_addr.is_empty() || op.mem_offset != 0 {
            return false;
        }
        // Each memory operand gets its own unique register via assign_scratch_reg,
        // so multiple "=m" outputs don't overwrite each other's addresses.
        match val {
            Operand::Value(v) => {
                // Check register allocation first: if the pointer value lives
                // in a callee-saved register, copy from there instead of the
                // stack (which may be stale for register-allocated values).
                if let Some(&phys) = self.reg_assignments.get(&v.0) {
                    let tmp_reg = self.assign_scratch_reg(&AsmOperandKind::GpReg, excluded);
                    let src_name = super::emit::callee_saved_name(phys);
                    self.state
                        .emit_fmt(format_args!("    mv {}, {}", tmp_reg, src_name));
                    op.mem_addr = format!("0({})", tmp_reg);
                    return true;
                }
                if let Some(slot) = self.state.get_slot(v.0) {
                    let tmp_reg = self.assign_scratch_reg(&AsmOperandKind::GpReg, excluded);
                    // Use emit_load_from_s0 to handle large stack offsets (>2047)
                    // that don't fit in RISC-V's 12-bit signed immediate range.
                    self.emit_load_from_s0(&tmp_reg, slot.0, "ld");
                    op.mem_addr = format!("0({})", tmp_reg);
                    return true;
                }
            }
            Operand::Const(c) => {
                // Constant address (e.g., from MMIO reads at compile-time constant addresses).
                // Load the constant into a scratch register for indirect addressing.
                if let Some(addr) = c.to_i64() {
                    let tmp_reg = self.assign_scratch_reg(&AsmOperandKind::GpReg, excluded);
                    self.state
                        .emit_fmt(format_args!("    li {}, {}", tmp_reg, addr));
                    op.mem_addr = format!("0({})", tmp_reg);
                    return true;
                }
            }
        }
        false
    }

    fn assign_scratch_reg(&mut self, kind: &AsmOperandKind, excluded: &[String]) -> String {
        match kind {
            AsmOperandKind::FpReg => {
                let idx = self.asm_fp_scratch_idx;
                self.asm_fp_scratch_idx += 1;
                if idx < RISCV_FP_SCRATCH.len() {
                    RISCV_FP_SCRATCH[idx].to_string()
                } else {
                    format!("fs{}", idx - RISCV_FP_SCRATCH.len())
                }
            }
            _ => {
                // Collect callee-saved registers already allocated by the register
                // allocator. When the scratch pool overflows to s2, s3, ... we must
                // skip any that hold live values to avoid clobbering them.
                let allocated_callee_saved: Vec<String> = self
                    .reg_assignments
                    .values()
                    .map(|phys| super::emit::callee_saved_name(*phys).to_string())
                    .collect();
                loop {
                    let idx = self.asm_gp_scratch_idx;
                    self.asm_gp_scratch_idx += 1;
                    let reg = if idx < RISCV_GP_SCRATCH.len() {
                        RISCV_GP_SCRATCH[idx].to_string()
                    } else {
                        // Overflow into callee-saved registers s2-s11.
                        // First try unallocated callee-saved registers.
                        let overflow_idx = idx - RISCV_GP_SCRATCH.len();
                        let mut available = Vec::new();
                        for n in 2..=11u32 {
                            let name = format!("s{}", n);
                            if !allocated_callee_saved.contains(&name)
                                && !excluded.iter().any(|e| e == &name)
                            {
                                available.push(name);
                            }
                        }
                        if overflow_idx < available.len() {
                            return available.into_iter().nth(overflow_idx).unwrap();
                        }
                        // All unallocated callee-saved exhausted. Borrow one that IS
                        // allocated — we'll save/restore it around the inline asm.
                        // Pick from least-likely-to-conflict candidates.
                        let borrow_candidates = [
                            "s7", "s6", "s1", "s2", "s3", "s4", "s5", "s8", "s9", "s10", "s11",
                        ];
                        for &bc in &borrow_candidates {
                            if !excluded.iter().any(|e| e == bc)
                                && !self.asm_borrowed_callee_saved.iter().any(|e| e == bc)
                            {
                                self.asm_borrowed_callee_saved.push(bc.to_string());
                                return bc.to_string();
                            }
                        }
                        // TODO: all borrow candidates exhausted, this may generate
                        // incorrect code if s7 is in use. This requires 26+ operands
                        // to trigger (15 caller-saved + 11 callee-saved) so it's
                        // extremely unlikely in practice.
                        return "s7".to_string();
                    };
                    if !excluded.iter().any(|e| e == &reg) && !allocated_callee_saved.contains(&reg)
                    {
                        return reg;
                    }
                    // Safety bail: avoid infinite loops
                    if idx > 30 {
                        return reg;
                    }
                }
            }
        }
    }

    fn load_input_to_reg(&mut self, op: &AsmOperand, val: &Operand, _constraint: &str) {
        let reg = &op.reg;
        let is_fp = matches!(op.kind, AsmOperandKind::FpReg);

        // If this register was borrowed from the register allocator (it's a
        // callee-saved register already holding a live value), save that value
        // to the stack before we overwrite it. The restore happens after
        // emit_inline_asm_common returns (in emit_inline_asm).
        // TODO: if a borrowed register is allocated during resolve_memory_operand
        // (not for an input), this save won't be reached. Currently only input
        // operands trigger overflow in practice.
        if let Some(pos) = self.asm_borrowed_callee_saved.iter().position(|r| r == reg) {
            self.state.emit("    addi sp, sp, -16");
            self.state.emit_fmt(format_args!("    sd {}, 0(sp)", reg));
            // Move from "pending save" to "saved" by marking with a prefix
            // so we don't save again but still know to restore.
            self.asm_borrowed_callee_saved[pos] = format!("saved:{}", reg);
        }
        let is_addr = matches!(op.kind, AsmOperandKind::Address);

        match val {
            Operand::Const(c) => {
                if is_fp {
                    // Extract IEEE 754 bit pattern for float constants.
                    // to_i64() returns None for F32/F64, so use to_bits() instead.
                    let imm = match c {
                        IrConst::F32(v) => v.to_bits() as i64,
                        IrConst::F64(v) => v.to_bits() as i64,
                        _ => c.to_i64().unwrap_or(0),
                    };
                    self.state.emit_fmt(format_args!("    li t5, {}", imm));
                    // Use fmv.w.x for 32-bit floats, fmv.d.x for 64-bit doubles.
                    // The decision must be based on the actual operand type, not the
                    // constraint string (which is just "f" for both float and double).
                    let is_f32 = matches!(c, IrConst::F32(_)) || op.operand_type == IrType::F32;
                    if is_f32 {
                        self.state.emit_fmt(format_args!("    fmv.w.x {}, t5", reg));
                    } else {
                        self.state.emit_fmt(format_args!("    fmv.d.x {}, t5", reg));
                    }
                } else {
                    let imm = c.to_i64().unwrap_or(0);
                    self.state.emit_fmt(format_args!("    li {}, {}", reg, imm));
                }
            }
            Operand::Value(v) => {
                // Check register allocation first: if the value lives in a
                // callee-saved register, use it directly instead of loading
                // from the stack. This is critical for correctness when inline
                // asm changes page tables (e.g., csrw satp) — any stack access
                // between CSR writes would fault because the new page table
                // doesn't map the stack.
                // This also handles Address operands whose pointer value was
                // register-allocated (e.g., "+A" for AMO instructions).
                if !self.state.is_alloca(v.0) && !is_fp {
                    if let Some(&phys) = self.reg_assignments.get(&v.0) {
                        let src_name = super::emit::callee_saved_name(phys);
                        self.state
                            .emit_fmt(format_args!("    mv {}, {}", reg, src_name));
                        return;
                    }
                }
                if let Some(slot) = self.state.get_slot(v.0) {
                    if is_addr && self.state.is_alloca(v.0) {
                        // Alloca: stack slot IS the variable's memory, compute its address
                        self.emit_alloca_addr(reg, v.0, slot.0);
                    } else if is_addr {
                        // Non-alloca: stack slot holds a pointer value, load it
                        self.emit_load_from_s0(reg, slot.0, "ld");
                    } else if self.state.is_alloca(v.0) {
                        // Non-address alloca: compute address for "r"/"=r" constraint.
                        // The IR value of an alloca represents the address of the
                        // allocated memory, not the contents.
                        self.emit_alloca_addr(reg, v.0, slot.0);
                    } else if is_fp {
                        // Use flw for F32, fld for F64/other
                        let load_op = if op.operand_type == IrType::F32 {
                            "flw"
                        } else {
                            "fld"
                        };
                        self.emit_load_from_s0(reg, slot.0, load_op);
                    } else {
                        self.emit_load_from_s0(reg, slot.0, "ld");
                    }
                }
            }
        }
    }

    fn preload_readwrite_output(&mut self, op: &AsmOperand, ptr: &Value) {
        let reg = op.reg.clone();
        if let Some(slot) = self.state.get_slot(ptr.0) {
            match &op.kind {
                AsmOperandKind::Address => {
                    if self.state.is_alloca(ptr.0) {
                        // Alloca: stack slot IS the variable, compute its address
                        self.emit_alloca_addr(&reg, ptr.0, slot.0);
                    } else {
                        // Non-alloca: pointer may live in a register
                        if let Some(&phys) = self.reg_assignments.get(&ptr.0) {
                            let src = super::emit::callee_saved_name(phys);
                            self.state.emit_fmt(format_args!("    mv {}, {}", reg, src));
                        } else {
                            self.emit_load_from_s0(&reg, slot.0, "ld");
                        }
                    }
                }
                AsmOperandKind::FpReg => {
                    // Use flw for F32, fld for F64/other
                    let load_op = if op.operand_type == IrType::F32 {
                        "flw"
                    } else {
                        "fld"
                    };
                    self.emit_load_from_s0(&reg, slot.0, load_op);
                }
                AsmOperandKind::Memory => {} // No preload for memory
                _ => {
                    self.emit_load_from_s0(&reg, slot.0, "ld");
                }
            }
        } else if let Some(&phys) = self.reg_assignments.get(&ptr.0) {
            // No stack slot (copy-alias eliminated), but value lives in a
            // callee-saved register.  Load it into the operand register.
            let src = super::emit::callee_saved_name(phys);
            self.state.emit_fmt(format_args!("    mv {}, {}", reg, src));
        }
    }

    fn substitute_template_line(
        &self,
        line: &str,
        operands: &[AsmOperand],
        gcc_to_internal: &[usize],
        _operand_types: &[IrType],
        goto_labels: &[(String, BlockId)],
    ) -> String {
        // Build parallel arrays for the RISC-V substitution function
        let op_regs: Vec<String> = operands.iter().map(|o| o.reg.clone()).collect();
        let op_names: Vec<Option<String>> = operands.iter().map(|o| o.name.clone()).collect();
        let op_mem_offsets: Vec<i64> = operands.iter().map(|o| o.mem_offset).collect();
        let op_mem_addrs: Vec<String> = operands.iter().map(|o| o.mem_addr.clone()).collect();
        let op_imm_values: Vec<Option<i64>> = operands.iter().map(|o| o.imm_value).collect();
        let op_imm_symbols: Vec<Option<String>> =
            operands.iter().map(|o| o.imm_symbol.clone()).collect();

        // Convert AsmOperandKind back to RvConstraintKind for the substitution function
        let op_kinds: Vec<RvConstraintKind> = operands
            .iter()
            .map(|o| match &o.kind {
                AsmOperandKind::GpReg => RvConstraintKind::GpReg,
                AsmOperandKind::FpReg => RvConstraintKind::FpReg,
                AsmOperandKind::Memory => RvConstraintKind::Memory,
                AsmOperandKind::Address => RvConstraintKind::Address,
                AsmOperandKind::Immediate => RvConstraintKind::Immediate,
                AsmOperandKind::ZeroOrReg => RvConstraintKind::ZeroOrReg,
                AsmOperandKind::Specific(r) => RvConstraintKind::Specific(r.clone()),
                AsmOperandKind::Tied(n) => RvConstraintKind::Tied(*n),
                // TODO: RISC-V =@cc not fully implemented — needs SLTU/SEQZ/etc. instead of SETcc.
                // Currently maps to GpReg which will store incorrect results.
                AsmOperandKind::ConditionCode(_) => RvConstraintKind::GpReg,
                // x86-only constraint kinds; map to GpReg as fallback (should never occur on RISC-V)
                AsmOperandKind::X87St0 | AsmOperandKind::X87St1 | AsmOperandKind::QReg => {
                    RvConstraintKind::GpReg
                }
            })
            .collect();

        let mut result = Self::substitute_riscv_asm_operands(
            line,
            &op_regs,
            &op_names,
            &op_kinds,
            &op_mem_offsets,
            &op_mem_addrs,
            &op_imm_values,
            &op_imm_symbols,
            gcc_to_internal,
        );
        // Substitute %l[name] goto label references
        result = crate::backend::inline_asm::substitute_goto_labels(
            &result,
            goto_labels,
            operands.len(),
        );
        result
    }

    fn store_output_from_reg(
        &mut self,
        op: &AsmOperand,
        ptr: &Value,
        _constraint: &str,
        all_output_regs: &[&str],
    ) {
        match &op.kind {
            AsmOperandKind::Memory => (),
            AsmOperandKind::Address => (), // AMO/LR/SC wrote through the pointer
            AsmOperandKind::FpReg => {
                let reg = op.reg.clone();
                // Use fsw for F32, fsd for F64/other
                let store_op = if op.operand_type == IrType::F32 {
                    "fsw"
                } else {
                    "fsd"
                };
                let is_direct = self.state.is_direct_slot(ptr.0);
                let slot = self.state.get_slot(ptr.0);
                if is_direct {
                    if let Some(slot) = slot {
                        self.emit_store_to_s0(&reg, slot.0, store_op);
                    }
                } else if let Some(&phys) = self.reg_assignments.get(&ptr.0) {
                    let scratch = find_gp_scratch_for_output("", all_output_regs);
                    let src_name = super::emit::callee_saved_name(phys);
                    if all_output_regs.contains(&scratch.as_str()) {
                        self.state.emit_fmt(format_args!("    addi sp, sp, -16"));
                        self.state
                            .emit_fmt(format_args!("    sd {}, 0(sp)", scratch));
                        self.state
                            .emit_fmt(format_args!("    mv {}, {}", scratch, src_name));
                        self.state
                            .emit_fmt(format_args!("    {} {}, 0({})", store_op, reg, scratch));
                        self.state
                            .emit_fmt(format_args!("    ld {}, 0(sp)", scratch));
                        self.state.emit_fmt(format_args!("    addi sp, sp, 16"));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    mv {}, {}", scratch, src_name));
                        self.state
                            .emit_fmt(format_args!("    {} {}, 0({})", store_op, reg, scratch));
                    }
                } else if let Some(slot) = slot {
                    let scratch = find_gp_scratch_for_output("", all_output_regs);
                    if all_output_regs.contains(&scratch.as_str()) {
                        self.state.emit_fmt(format_args!("    addi sp, sp, -16"));
                        self.state
                            .emit_fmt(format_args!("    sd {}, 0(sp)", scratch));
                        self.emit_load_from_s0(&scratch, slot.0, "ld");
                        self.state
                            .emit_fmt(format_args!("    {} {}, 0({})", store_op, reg, &scratch));
                        self.state
                            .emit_fmt(format_args!("    ld {}, 0(sp)", scratch));
                        self.state.emit_fmt(format_args!("    addi sp, sp, 16"));
                    } else {
                        self.emit_load_from_s0(&scratch, slot.0, "ld");
                        self.state
                            .emit_fmt(format_args!("    {} {}, 0({})", store_op, reg, &scratch));
                    }
                }
            }
            _ => {
                let reg = op.reg.clone();
                // For stores through pointers (indirect), use the type-appropriate
                // store instruction to avoid corrupting adjacent memory with an
                // oversized store. For direct slots (alloca IS the variable), always
                // use sd because alloca slots are padded to 8 bytes on RV64 and
                // the preload path uses ld unconditionally.
                let is_direct = self.state.is_direct_slot(ptr.0);
                let store_op = if is_direct {
                    "sd"
                } else {
                    gp_store_for_type(op.operand_type)
                };
                let slot = self.state.get_slot(ptr.0);
                if is_direct {
                    if let Some(slot) = slot {
                        self.emit_store_to_s0(&reg, slot.0, "sd");
                    }
                } else if let Some(&phys) = self.reg_assignments.get(&ptr.0) {
                    // Pointer lives in a callee-saved register. Store the asm
                    // output through the pointer. This avoids stack access
                    // after CSR writes (e.g., csrw satp changing page tables).
                    let scratch = find_gp_scratch_for_output(&reg, all_output_regs);
                    if all_output_regs.contains(&scratch.as_str()) {
                        // All candidates are output registers. Use stack-based spill:
                        // save the scratch output on stack, use it as address, then restore.
                        let src_name = super::emit::callee_saved_name(phys);
                        self.state.emit_fmt(format_args!("    addi sp, sp, -16"));
                        self.state
                            .emit_fmt(format_args!("    sd {}, 0(sp)", scratch));
                        self.state
                            .emit_fmt(format_args!("    mv {}, {}", scratch, src_name));
                        self.state
                            .emit_fmt(format_args!("    {} {}, 0({})", store_op, reg, scratch));
                        self.state
                            .emit_fmt(format_args!("    ld {}, 0(sp)", scratch));
                        self.state.emit_fmt(format_args!("    addi sp, sp, 16"));
                    } else {
                        let src_name = super::emit::callee_saved_name(phys);
                        self.state
                            .emit_fmt(format_args!("    mv {}, {}", scratch, src_name));
                        self.state
                            .emit_fmt(format_args!("    {} {}, 0({})", store_op, reg, scratch));
                    }
                } else if let Some(slot) = slot {
                    // Non-alloca: slot holds a pointer, store through it.
                    let scratch = find_gp_scratch_for_output(&reg, all_output_regs);
                    if all_output_regs.contains(&scratch.as_str()) {
                        // All candidates are output registers. Use stack-based spill:
                        // save the scratch output on stack, load address into it,
                        // store through it, then restore the scratch output.
                        self.state.emit_fmt(format_args!("    addi sp, sp, -16"));
                        self.state
                            .emit_fmt(format_args!("    sd {}, 0(sp)", scratch));
                        self.emit_load_from_s0(&scratch, slot.0, "ld");
                        self.state
                            .emit_fmt(format_args!("    {} {}, 0({})", store_op, reg, scratch));
                        self.state
                            .emit_fmt(format_args!("    ld {}, 0(sp)", scratch));
                        self.state.emit_fmt(format_args!("    addi sp, sp, 16"));
                    } else {
                        self.emit_load_from_s0(&scratch, slot.0, "ld");
                        self.state
                            .emit_fmt(format_args!("    {} {}, 0({})", store_op, reg, scratch));
                    }
                }
            }
        }
    }

    fn reset_scratch_state(&mut self) {
        self.asm_gp_scratch_idx = 0;
        self.asm_fp_scratch_idx = 0;
        self.asm_borrowed_callee_saved.clear();
    }

    /// RISC-V-specific immediate constraint ranges.
    ///
    /// On RISC-V, 'K' means a 5-bit unsigned CSR immediate (0-31), used by
    /// csrs/csrc/csrw instructions. This differs from x86 where 'K' means 0-255.
    /// Without this override, values like 128 would be incorrectly emitted as
    /// immediates in CSR instructions, causing assembler errors.
    fn constant_fits_immediate(&self, constraint: &str, value: i64) -> bool {
        let stripped = constraint.trim_start_matches(['=', '+', '&', '%']);
        // If constraint has 'i' or 'n', any constant value is accepted
        if stripped.contains('i') || stripped.contains('n') {
            return true;
        }
        // Check each constraint letter with RISC-V-specific ranges
        for ch in stripped.chars() {
            let fits = match ch {
                // RISC-V 'I': 12-bit signed immediate (-2048..2047)
                'I' => (-2048..=2047).contains(&value),
                // RISC-V 'K': 5-bit unsigned CSR immediate (0..31) for csrs/csrc/csrw
                'K' => (0..=31).contains(&value),
                _ => continue,
            };
            if fits {
                return true;
            }
        }
        false
    }
}
