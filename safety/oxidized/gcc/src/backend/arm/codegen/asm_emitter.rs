//! AArch64 InlineAsmEmitter implementation: constraint classification, scratch
//! register allocation, operand loading/storing, and template substitution.

use super::emit::{is_arm_fp_reg, ArmCodegen};
use crate::backend::inline_asm::{AsmOperand, AsmOperandKind, InlineAsmEmitter};
use crate::backend::regalloc::PhysReg;
use crate::backend::state::CodegenState;
use crate::common::types::IrType;
use crate::ir::reexports::{BlockId, IrConst, Operand, Value};

/// AArch64 scratch registers for inline asm (caller-saved temporaries).
pub(super) const ARM_GP_SCRATCH: &[&str] = &[
    "x9", "x10", "x11", "x12", "x13", "x14", "x15", "x19", "x20", "x21",
];
/// AArch64 FP/SIMD scratch registers for inline asm (d8-d15 are callee-saved,
/// d16-d31 are caller-saved; we use v16+ as scratch to avoid save/restore).
/// We use the 'v' prefix so that unmodified %0 in templates like `eor %0.16b, %1.16b, %2.16b`
/// correctly produces `v16.16b` (GCC behavior). Modifiers (%d0, %s0, etc.) convert
/// to the appropriate scalar view in format_reg_static.
const ARM_FP_SCRATCH: &[&str] = &[
    "v16", "v17", "v18", "v19", "v20", "v21", "v22", "v23", "v24", "v25",
];

/// Convert an IR constant to a 64-bit value appropriate for an inline asm operand.
/// On AArch64, 32-bit values in X registers must have upper 32 bits zeroed (zero-extended).
/// For 64-bit operand types, the constant is sign-extended to preserve its semantic value
/// (e.g., int64_t a = -128 stored as IrConst::I32(-128) must become 0xFFFFFFFF_FFFFFF80).
fn const_for_asm_operand(c: &IrConst, operand_type: &IrType) -> i64 {
    let sext = c.to_i64().unwrap_or(0);
    match operand_type.size() {
        // 32-bit operand: zero-extend the 32-bit bit pattern.
        // This ensures e.g. 0xF0000000u stays 0x00000000_F0000000 in the X register,
        // not 0xFFFFFFFF_F0000000 (which would be sign-extended).
        1 => sext & 0xFF,
        2 => sext & 0xFFFF,
        4 => sext & 0xFFFF_FFFF,
        // 64-bit or larger operand: sign-extend to preserve int64_t semantics.
        _ => sext,
    }
}

impl InlineAsmEmitter for ArmCodegen {
    fn asm_state(&mut self) -> &mut CodegenState {
        &mut self.state
    }

    // Multi-alternative constraint parsing (e.g., "rm", "ri") matching x86 behavior.
    // Priority: specific register > GP register > FP register > memory > immediate.
    // Registers are preferred over memory for performance.
    fn classify_constraint(&self, constraint: &str) -> AsmOperandKind {
        let c = constraint.trim_start_matches(['=', '+', '&', '%']);
        // Explicit register constraint from register variable: {regname}
        if c.starts_with('{') && c.ends_with('}') {
            let reg_name = &c[1..c.len() - 1];
            // On AArch64, GCC treats r0-r30 as aliases for x0-x30.
            // The Linux kernel uses `register ... asm("r0")` extensively
            // (e.g., arm-smccc.h). Normalize to the canonical x-register name.
            let normalized = normalize_aarch64_register(reg_name);
            return AsmOperandKind::Specific(normalized);
        }
        // TODO: ARM =@cc not fully implemented — needs CSET/CSINC in store_output_from_reg.
        // Currently stores incorrect results (just a GP register value, no condition capture).
        if let Some(cond) = c.strip_prefix("@cc") {
            return AsmOperandKind::ConditionCode(cond.to_string());
        }
        if !c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit()) {
            if let Ok(n) = c.parse::<usize>() {
                return AsmOperandKind::Tied(n);
            }
        }

        // Parse multi-alternative constraints character by character.
        let mut has_gp = false;
        let mut has_fp = false;
        let mut has_mem = false;
        let mut has_imm = false;

        for ch in c.chars() {
            match ch {
                'r' => has_gp = true,
                'g' => {
                    has_gp = true;
                    has_mem = true;
                    has_imm = true;
                }
                'w' => has_fp = true,
                'm' | 'Q' | 'o' | 'V' | 'p' => has_mem = true,
                'i' | 'n' | 'I' | 'K' | 'L' => has_imm = true,
                _ => {}
            }
        }

        if has_gp {
            AsmOperandKind::GpReg
        } else if has_fp {
            AsmOperandKind::FpReg
        } else if has_mem {
            AsmOperandKind::Memory
        } else if has_imm {
            AsmOperandKind::Immediate
        } else {
            AsmOperandKind::GpReg
        }
    }

    fn setup_operand_metadata(&self, op: &mut AsmOperand, val: &Operand, _is_output: bool) {
        if matches!(op.kind, AsmOperandKind::Memory) {
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
    }

    fn resolve_memory_operand(
        &mut self,
        op: &mut AsmOperand,
        val: &Operand,
        excluded: &[String],
    ) -> bool {
        if !op.mem_addr.is_empty() || op.mem_offset != 0 {
            return false;
        }
        // Each memory operand gets its own unique register via assign_scratch_reg,
        // so multiple "=m" outputs don't overwrite each other's addresses.
        match val {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    let tmp_reg = self.assign_scratch_reg(&AsmOperandKind::GpReg, excluded);
                    self.emit_load_from_sp(&tmp_reg, slot.0, "ldr");
                    op.mem_addr = format!("[{}]", tmp_reg);
                    return true;
                }
            }
            Operand::Const(c) => {
                // Constant address (e.g., from MMIO reads at compile-time constant addresses).
                // Copy propagation can replace Value operands with Const in inline asm inputs.
                // Load the constant into a scratch register for indirect addressing.
                if let Some(addr) = c.to_i64() {
                    let tmp_reg = self.assign_scratch_reg(&AsmOperandKind::GpReg, excluded);
                    self.emit_load_imm64(&tmp_reg, addr);
                    op.mem_addr = format!("[{}]", tmp_reg);
                    return true;
                }
            }
        }
        false
    }

    fn assign_scratch_reg(&mut self, kind: &AsmOperandKind, excluded: &[String]) -> String {
        if matches!(kind, AsmOperandKind::FpReg) {
            // Safety: limit iterations to avoid infinite loop if all regs are excluded
            for _ in 0..32 {
                let idx = self.asm_fp_scratch_idx;
                self.asm_fp_scratch_idx += 1;
                let reg = if idx < ARM_FP_SCRATCH.len() {
                    ARM_FP_SCRATCH[idx].to_string()
                } else {
                    format!("v{}", 16 + idx)
                };
                if !excluded.iter().any(|e| e == &reg) {
                    return reg;
                }
            }
            // Fallback: return next register even if excluded
            format!("v{}", 16 + self.asm_fp_scratch_idx)
        } else {
            loop {
                let idx = self.asm_scratch_idx;
                self.asm_scratch_idx += 1;
                let reg = if idx < ARM_GP_SCRATCH.len() {
                    ARM_GP_SCRATCH[idx].to_string()
                } else {
                    format!("x{}", 9 + idx)
                };
                if !excluded.iter().any(|e| e == &reg) {
                    // If this is a callee-saved register (x19-x28), ensure it is
                    // saved/restored in the prologue/epilogue.
                    let reg_num = reg.strip_prefix('x').and_then(|s| s.parse::<u8>().ok());
                    if let Some(n) = reg_num {
                        if (19..=28).contains(&n) {
                            let phys = PhysReg(n);
                            if !self.used_callee_saved.contains(&phys) {
                                self.used_callee_saved.push(phys);
                                self.used_callee_saved.sort_by_key(|r| r.0);
                            }
                        }
                    }
                    return reg;
                }
            }
        }
    }

    fn load_input_to_reg(&mut self, op: &AsmOperand, val: &Operand, _constraint: &str) {
        let reg = &op.reg;
        let is_fp = is_arm_fp_reg(reg);
        let is_sp = reg == "sp";
        match val {
            Operand::Const(c) => {
                if is_fp {
                    // Load FP constant: extract IEEE 754 bit pattern, move to GP reg,
                    // then fmov to FP reg. to_i64() returns None for floats, so we
                    // must use to_bits() to get the bit-level representation.
                    // fmov requires d/s register form, not v.
                    let bits = match c {
                        IrConst::F32(v) => v.to_bits() as i64,
                        IrConst::F64(v) => v.to_bits() as i64,
                        _ => c.to_i64().unwrap_or(0),
                    };
                    self.emit_load_imm64("x9", bits);
                    if op.operand_type == IrType::F32 {
                        let s_reg = Self::fp_to_s_reg(reg);
                        self.state.emit_fmt(format_args!("    fmov {}, w9", s_reg));
                    } else {
                        let d_reg = Self::fp_to_d_reg(reg);
                        self.state.emit_fmt(format_args!("    fmov {}, x9", d_reg));
                    }
                } else if is_sp {
                    // ARM64: can't use ldr/mov imm to sp directly in most cases.
                    // Load to scratch first, then mov to sp.
                    let val = const_for_asm_operand(c, &op.operand_type);
                    self.emit_load_imm64("x9", val);
                    self.state.emit("    mov sp, x9");
                } else {
                    let val = const_for_asm_operand(c, &op.operand_type);
                    self.emit_load_imm64(reg, val);
                }
            }
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    if is_fp {
                        // Load FP value from stack: use ldr with d/s register form.
                        // For SIMD vector types (>= 16 bytes), use ldr with q register.
                        let type_size = op.operand_type.size();
                        if type_size == 16 {
                            // 128-bit vector: load directly with ldr qN
                            let q_reg = Self::fp_to_q_reg(reg);
                            self.emit_load_from_sp(&q_reg, slot.0, "ldr");
                        } else if op.operand_type == IrType::F32 || type_size == 4 {
                            self.state
                                .emit_fmt(format_args!("    ldr w9, [sp, #{}]", slot.0));
                            let s_reg = Self::fp_to_s_reg(reg);
                            self.state.emit_fmt(format_args!("    fmov {}, w9", s_reg));
                        } else {
                            self.state
                                .emit_fmt(format_args!("    ldr x9, [sp, #{}]", slot.0));
                            let d_reg = Self::fp_to_d_reg(reg);
                            self.state.emit_fmt(format_args!("    fmov {}, x9", d_reg));
                        }
                    } else if is_sp {
                        // ARM64: can't use ldr to load directly into sp.
                        // Load to scratch first, then mov to sp.
                        self.emit_load_from_sp("x9", slot.0, "ldr");
                        self.state.emit("    mov sp, x9");
                    } else if self.state.is_alloca(v.0) {
                        // Alloca: the IR value represents the ADDRESS of the
                        // allocated memory. Compute its address instead of
                        // loading the contents.
                        self.emit_alloca_addr(reg, v.0, slot.0);
                    } else {
                        // Non-alloca: load the value from the stack slot.
                        self.emit_load_from_sp(reg, slot.0, "ldr");
                    }
                }
            }
        }
    }

    fn preload_readwrite_output(&mut self, op: &AsmOperand, ptr: &Value) {
        let reg = &op.reg;
        let is_fp = is_arm_fp_reg(reg);
        if let Some(slot) = self.state.get_slot(ptr.0) {
            if is_fp {
                // Load current FP value for read-write constraint.
                // fmov requires d/s register form, not v.
                let type_size = op.operand_type.size();
                if type_size == 16 {
                    let q_reg = Self::fp_to_q_reg(reg);
                    self.emit_load_from_sp(&q_reg, slot.0, "ldr");
                } else if op.operand_type == IrType::F32 || type_size == 4 {
                    self.state
                        .emit_fmt(format_args!("    ldr w9, [sp, #{}]", slot.0));
                    let s_reg = Self::fp_to_s_reg(reg);
                    self.state.emit_fmt(format_args!("    fmov {}, w9", s_reg));
                } else {
                    self.state
                        .emit_fmt(format_args!("    ldr x9, [sp, #{}]", slot.0));
                    let d_reg = Self::fp_to_d_reg(reg);
                    self.state.emit_fmt(format_args!("    fmov {}, x9", d_reg));
                }
            } else if reg == "sp" {
                // ARM64: can't use ldr to load directly into sp.
                self.emit_load_from_sp("x9", slot.0, "ldr");
                self.state.emit("    mov sp, x9");
            } else {
                self.emit_load_from_sp(reg, slot.0, "ldr");
            }
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
        // For memory operands (Q/m constraints), use mem_addr (e.g., "[x9]") or
        // format as [sp, #offset] for stack-based memory. For register operands,
        // use the register name directly.
        let op_regs: Vec<String> = operands
            .iter()
            .map(|o| {
                if matches!(o.kind, AsmOperandKind::Memory) {
                    if !o.mem_addr.is_empty() {
                        // Non-alloca pointer: mem_addr already formatted as "[xN]"
                        o.mem_addr.clone()
                    } else if o.mem_offset != 0 {
                        // Alloca: stack-relative address
                        format!("[sp, #{}]", o.mem_offset)
                    } else {
                        // Fallback: wrap register in brackets
                        if o.reg.is_empty() {
                            "[sp]".to_string()
                        } else {
                            format!("[{}]", o.reg)
                        }
                    }
                } else {
                    o.reg.clone()
                }
            })
            .collect();
        let op_names: Vec<Option<String>> = operands.iter().map(|o| o.name.clone()).collect();
        let op_imm_values: Vec<Option<i64>> = operands.iter().map(|o| o.imm_value).collect();
        let op_imm_symbols: Vec<Option<String>> =
            operands.iter().map(|o| o.imm_symbol.clone()).collect();
        let mut result = Self::substitute_asm_operands_static(
            line,
            &op_regs,
            &op_names,
            gcc_to_internal,
            &op_imm_values,
            &op_imm_symbols,
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
        if matches!(op.kind, AsmOperandKind::Memory) {
            return;
        }
        let reg = &op.reg;
        let is_fp = is_arm_fp_reg(reg);
        if let Some(slot) = self.state.get_slot(ptr.0) {
            if is_fp {
                // Store FP/SIMD register output. fmov requires d/s form, not v.
                let type_size = op.operand_type.size();
                if type_size == 16 {
                    // 128-bit vector: store directly with str qN
                    let q_reg = Self::fp_to_q_reg(reg);
                    self.emit_store_to_sp(&q_reg, slot.0, "str");
                } else if op.operand_type == IrType::F32 || type_size == 4 {
                    let s_reg = Self::fp_to_s_reg(reg);
                    self.state.emit_fmt(format_args!("    fmov w9, {}", s_reg));
                    self.state
                        .emit_fmt(format_args!("    str w9, [sp, #{}]", slot.0));
                } else {
                    let d_reg = Self::fp_to_d_reg(reg);
                    self.state.emit_fmt(format_args!("    fmov x9, {}", d_reg));
                    self.state
                        .emit_fmt(format_args!("    str x9, [sp, #{}]", slot.0));
                }
            } else if reg == "sp" {
                // ARM64: sp (register 31) can't be used as str source operand directly.
                // Move to a scratch register first, then store.
                self.state.emit("    mov x9, sp");
                self.emit_store_to_sp("x9", slot.0, "str");
            } else if self.state.is_direct_slot(ptr.0) {
                self.emit_store_to_sp(reg, slot.0, "str");
            } else {
                // Non-alloca: slot holds a pointer, store through it.
                // Pick a scratch register that doesn't conflict with ANY output register,
                // not just the current one. This prevents clobbering other outputs that
                // haven't been stored yet.
                let candidates = ["x9", "x10", "x11", "x12", "x13", "x14", "x15"];
                let scratch = candidates
                    .iter()
                    .find(|&&c| !all_output_regs.contains(&c))
                    .copied()
                    .unwrap_or(if reg != "x9" { "x9" } else { "x10" });
                self.emit_load_from_sp(scratch, slot.0, "ldr");
                self.state
                    .emit_fmt(format_args!("    str {}, [{}]", reg, scratch));
            }
        }
    }

    fn reset_scratch_state(&mut self) {
        self.asm_scratch_idx = 0;
        self.asm_fp_scratch_idx = 0;
    }

    /// Override the default (x86) immediate constraint validation with AArch64 semantics.
    ///
    /// On AArch64:
    ///   'K' - logical immediate: a bitmask value encodable in the N:immr:imms field
    ///         of AND/ORR/EOR/TST instructions. Excludes 0 and all-ones.
    ///   'I' - unsigned 12-bit immediate (0..4095) for add/sub instructions
    ///   'L' - logical immediate for 32-bit operations (32-bit bitmask pattern)
    fn constant_fits_immediate(&self, constraint: &str, value: i64) -> bool {
        let stripped = constraint.trim_start_matches(['=', '+', '&', '%']);
        // If constraint has 'i' or 'n', any constant value is accepted
        if stripped.contains('i') || stripped.contains('n') {
            return true;
        }
        // Check each constraint letter with AArch64-specific ranges
        for ch in stripped.chars() {
            let fits = match ch {
                // AArch64 'K': 64-bit logical immediate (bitmask encodable in N:immr:imms)
                // Used by AND/ORR/EOR/TST instructions. 0 and all-ones are NOT valid.
                'K' => is_valid_aarch64_logical_immediate(value as u64),
                // AArch64 'L': 32-bit logical immediate (validate in 32-bit context)
                'L' => is_valid_aarch64_logical_immediate_32(value as u32),
                // AArch64 'I': unsigned 12-bit add/sub immediate
                'I' => (0..=4095).contains(&value),
                _ => continue,
            };
            if fits {
                return true;
            }
        }
        false
    }
}

/// Normalize AArch64 register name aliases.
///
/// GCC treats `r0`-`r30` as aliases for `x0`-`x30` on AArch64. The Linux kernel
/// uses this convention extensively in inline assembly (e.g., `register unsigned long
/// r0 asm("r0")` in arm-smccc.h). This function maps these aliases to canonical
/// AArch64 register names so the assembler accepts them.
pub(super) fn normalize_aarch64_register(name: &str) -> String {
    if let Some(suffix) = name.strip_prefix('r') {
        if let Ok(n) = suffix.parse::<u32>() {
            if n <= 30 {
                return format!("x{}", n);
            }
        }
    }
    name.to_string()
}

/// Check whether a 32-bit value is a valid AArch64 32-bit logical immediate.
/// Used for the 'L' constraint (32-bit logical operations like AND w0, w1, #imm).
fn is_valid_aarch64_logical_immediate_32(value: u32) -> bool {
    if value == 0 || value == u32::MAX {
        return false;
    }
    // Check element sizes: 2, 4, 8, 16, 32 bits within a 32-bit value
    let mut size: u32 = 32;
    while size >= 2 {
        let mask = if size == 32 {
            u32::MAX
        } else {
            (1u32 << size) - 1
        };
        let element = value & mask;
        // Check if value is a repeating pattern of this element size
        let mut check = element;
        let mut s = size;
        while s < 32 {
            check |= check << s;
            s *= 2;
        }
        if check == value {
            let val = element & mask;
            if val != 0 && val != mask {
                let rotated = ((val >> 1) | ((val & 1) << (size - 1))) & mask;
                let transitions = val ^ rotated;
                if transitions.count_ones() == 2 {
                    return true;
                }
            }
        }
        size >>= 1;
    }
    false
}

/// Check whether a 64-bit value is a valid AArch64 logical immediate.
///
/// AArch64 logical immediates are bitmask patterns encodable in the 13-bit
/// N:immr:imms field of AND/ORR/EOR/TST instructions. Valid patterns consist
/// of a repeating element of size 2, 4, 8, 16, 32, or 64 bits, where each
/// element contains a contiguous (possibly rotated) run of set bits.
///
/// The values 0 and all-ones (0xFFFFFFFF_FFFFFFFF) are NOT valid logical immediates.
fn is_valid_aarch64_logical_immediate(value: u64) -> bool {
    // 0 and all-ones are never valid logical immediates
    if value == 0 || value == u64::MAX {
        return false;
    }

    // Try each possible element size: 2, 4, 8, 16, 32, 64 bits.
    // For each size, check if the value is a repeating pattern of that element,
    // and if the element contains a contiguous (possibly rotated) run of 1-bits.
    let mut size: u32 = 64;
    while size >= 2 {
        let mask = if size == 64 {
            u64::MAX
        } else {
            (1u64 << size) - 1
        };
        let element = value & mask;

        // Check if value is a repeating pattern of this element size
        if is_repeating_pattern(value, element, size) {
            // Check if the element has a contiguous run of 1-bits (possibly rotated)
            if has_contiguous_ones(element, size) {
                return true;
            }
        }
        size >>= 1;
    }
    false
}

/// Check if `value` is composed of `element` repeated to fill 64 bits.
fn is_repeating_pattern(value: u64, element: u64, size: u32) -> bool {
    let mut check = element;
    let mut s = size;
    while s < 64 {
        check |= check << s;
        s *= 2;
    }
    check == value
}

/// Check if the lowest `size` bits of `element` contain a contiguous run of
/// set bits (possibly rotated). A contiguous-rotated pattern means there's
/// at most one 0->1 transition and one 1->0 transition in the circular bit sequence.
fn has_contiguous_ones(element: u64, size: u32) -> bool {
    let mask = if size == 64 {
        u64::MAX
    } else {
        (1u64 << size) - 1
    };
    let val = element & mask;
    // All-zeros or all-ones within the element are not valid
    if val == 0 || val == mask {
        return false;
    }
    // A contiguous run of 1-bits (possibly rotated) has the property that
    // val ^ (val rotated by 1) has exactly 2 set bits (the two transitions).
    let rotated = ((val >> 1) | ((val & 1) << (size - 1))) & mask;
    let transitions = val ^ rotated;
    transitions.count_ones() == 2
}
