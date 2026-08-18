use crate::backend::call_abi::{CallAbiConfig, CallArgClass};
use crate::backend::cast::FloatOp;
use crate::backend::common::PtrDirective;
use crate::backend::inline_asm::emit_inline_asm_common;
use crate::backend::regalloc::PhysReg;
use crate::backend::state::{CodegenState, StackSlot};
use crate::backend::traits::ArchCodegen;
use crate::common::fx_hash::FxHashMap;
use crate::common::types::IrType;
use crate::delegate_to_impl;
use crate::ir::reexports::{
    AtomicOrdering, AtomicRmwOp, BlockId, IntrinsicOp, IrBinOp, IrCmpOp, IrConst, IrFunction,
    Operand, Value,
};

/// RISC-V callee-saved registers always available for register allocation.
/// s0 is the frame pointer.
/// These 6 registers are always safe to allocate: s1, s7-s11.
/// PhysReg encoding: 1=s1, 7=s7, 8=s8, 9=s9, 10=s10, 11=s11.
pub(super) const RISCV_CALLEE_SAVED: [PhysReg; 6] = [
    PhysReg(1),
    PhysReg(7),
    PhysReg(8),
    PhysReg(9),
    PhysReg(10),
    PhysReg(11),
];

/// Additional callee-saved registers available for allocation.
/// s2-s6 were originally reserved for call argument staging, but the current
/// three-phase staging strategy uses only caller-saved t3/t4/t5, so all five
/// are now unconditionally available for the register allocator, giving up to
/// 11 callee-saved registers total (vs. 6 without these).
/// PhysReg(2)=s2, PhysReg(3)=s3, PhysReg(4)=s4, PhysReg(5)=s5, PhysReg(6)=s6.
pub(super) const CALL_TEMP_CALLEE_SAVED: [PhysReg; 5] =
    [PhysReg(2), PhysReg(3), PhysReg(4), PhysReg(5), PhysReg(6)];

/// Map a PhysReg index to its RISC-V register name.
pub(super) fn callee_saved_name(reg: PhysReg) -> &'static str {
    match reg.0 {
        1 => "s1",
        2 => "s2",
        3 => "s3",
        4 => "s4",
        5 => "s5",
        6 => "s6",
        7 => "s7",
        8 => "s8",
        9 => "s9",
        10 => "s10",
        11 => "s11",
        _ => unreachable!("invalid RISC-V callee-saved register index"),
    }
}

/// Scan inline asm instructions in a function and collect any callee-saved
/// registers that are used via specific constraints or listed in clobbers.
pub(super) fn collect_inline_asm_callee_saved_riscv(func: &IrFunction, used: &mut Vec<PhysReg>) {
    crate::backend::generation::collect_inline_asm_callee_saved(
        func,
        used,
        constraint_to_callee_saved_riscv,
        riscv_reg_to_callee_saved,
    );
}

/// Map a RISC-V register name to its PhysReg index, if it is callee-saved.
fn riscv_reg_to_callee_saved(name: &str) -> Option<PhysReg> {
    match name {
        "s1" | "x9" => Some(PhysReg(1)),
        "s2" | "x18" => Some(PhysReg(2)),
        "s3" | "x19" => Some(PhysReg(3)),
        "s4" | "x20" => Some(PhysReg(4)),
        "s5" | "x21" => Some(PhysReg(5)),
        "s6" | "x22" => Some(PhysReg(6)),
        "s7" | "x23" => Some(PhysReg(7)),
        "s8" | "x24" => Some(PhysReg(8)),
        "s9" | "x25" => Some(PhysReg(9)),
        "s10" | "x26" => Some(PhysReg(10)),
        "s11" | "x27" => Some(PhysReg(11)),
        _ => None,
    }
}

/// Check if a constraint string refers to a specific RISC-V callee-saved register.
fn constraint_to_callee_saved_riscv(constraint: &str) -> Option<PhysReg> {
    if constraint.starts_with('{') && constraint.ends_with('}') {
        let reg = &constraint[1..constraint.len() - 1];
        return riscv_reg_to_callee_saved(reg);
    }
    riscv_reg_to_callee_saved(constraint)
}

pub(super) const RISCV_ARG_REGS: [&str; 8] = ["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7"];

/// RISC-V 64 code generator. Implements the ArchCodegen trait for the shared framework.
/// Uses standard RISC-V calling convention with register allocation for hot values.
pub struct RiscvCodegen {
    pub(crate) state: CodegenState,
    pub(super) current_return_type: IrType,
    /// Number of named integer params for current variadic function.
    pub(super) va_named_gp_count: usize,
    /// Total bytes of named params that overflow to the caller's stack.
    pub(super) va_named_stack_bytes: usize,
    /// Current frame size (below s0, not including the register save area above s0).
    pub(super) current_frame_size: i64,
    /// Whether the current function is variadic.
    pub(super) is_variadic: bool,
    /// Scratch register indices for inline asm allocation.
    pub(super) asm_gp_scratch_idx: usize,
    pub(super) asm_fp_scratch_idx: usize,
    /// Callee-saved registers borrowed by inline asm scratch allocation that need
    /// save/restore. When all caller-saved registers are exhausted, the scratch
    /// allocator borrows a callee-saved register, saves it to the stack before
    /// input loading, and restores it after output storing.
    pub(super) asm_borrowed_callee_saved: Vec<String>,
    /// Register allocation results for the current function.
    pub(super) reg_assignments: FxHashMap<u32, PhysReg>,
    /// Which callee-saved registers are used and need save/restore.
    pub(super) used_callee_saved: Vec<PhysReg>,
    /// Whether to suppress linker relaxation (-mno-relax).
    pub(super) no_relax: bool,
}

impl RiscvCodegen {
    pub fn new() -> Self {
        Self {
            state: CodegenState::new(),
            current_return_type: IrType::I64,
            va_named_gp_count: 0,
            va_named_stack_bytes: 0,
            current_frame_size: 0,
            is_variadic: false,
            asm_gp_scratch_idx: 0,
            asm_fp_scratch_idx: 0,
            asm_borrowed_callee_saved: Vec::new(),
            reg_assignments: FxHashMap::default(),
            used_callee_saved: Vec::new(),
            no_relax: false,
        }
    }

    /// Disable jump table emission (-fno-jump-tables).
    pub fn set_no_jump_tables(&mut self, enabled: bool) {
        self.state.no_jump_tables = enabled;
    }

    /// Enable position-independent code generation (-fPIC/-fpie).
    pub fn set_pic(&mut self, pic: bool) {
        self.state.pic_mode = pic;
    }

    /// Suppress linker relaxation (-mno-relax).
    pub fn set_no_relax(&mut self, enabled: bool) {
        self.no_relax = enabled;
    }

    /// Apply all relevant options from a `CodegenOptions` struct.
    pub fn apply_options(&mut self, opts: &crate::backend::CodegenOptions) {
        self.set_pic(opts.pic);
        self.set_no_jump_tables(opts.no_jump_tables);
        self.set_no_relax(opts.no_relax);
        self.state.emit_cfi = opts.emit_cfi;
    }

    /// Emit `.option norelax` if -mno-relax is set.
    pub fn emit_pre_directives(&mut self) {
        if self.no_relax {
            self.state.emit(".option norelax");
        }
    }

    /// Load comparison operands into t1 and t2, then sign/zero-extend
    /// sub-64-bit types. Shared by emit_cmp and emit_fused_cmp_branch.
    pub(super) fn emit_cmp_operand_load(&mut self, lhs: &Operand, rhs: &Operand, ty: IrType) {
        self.operand_to_t0(lhs);
        self.state.emit("    mv t1, t0");
        self.operand_to_t0(rhs);
        self.state.emit("    mv t2, t0");

        match ty {
            IrType::U8 => {
                self.state.emit("    andi t1, t1, 0xff");
                self.state.emit("    andi t2, t2, 0xff");
            }
            IrType::U16 => {
                self.state.emit("    slli t1, t1, 48");
                self.state.emit("    srli t1, t1, 48");
                self.state.emit("    slli t2, t2, 48");
                self.state.emit("    srli t2, t2, 48");
            }
            IrType::U32 => {
                self.state.emit("    slli t1, t1, 32");
                self.state.emit("    srli t1, t1, 32");
                self.state.emit("    slli t2, t2, 32");
                self.state.emit("    srli t2, t2, 32");
            }
            IrType::I8 => {
                self.state.emit("    slli t1, t1, 56");
                self.state.emit("    srai t1, t1, 56");
                self.state.emit("    slli t2, t2, 56");
                self.state.emit("    srai t2, t2, 56");
            }
            IrType::I16 => {
                self.state.emit("    slli t1, t1, 48");
                self.state.emit("    srai t1, t1, 48");
                self.state.emit("    slli t2, t2, 48");
                self.state.emit("    srai t2, t2, 48");
            }
            IrType::I32 => {
                self.state.emit("    sext.w t1, t1");
                self.state.emit("    sext.w t2, t2");
            }
            _ => {} // I64/U64/Ptr: no extension needed
        }
    }

    // --- RISC-V helpers ---

    /// Check if an immediate fits in a 12-bit signed field.
    pub(super) fn fits_imm12(val: i64) -> bool {
        (-2048..=2047).contains(&val)
    }

    /// Emit: store `reg` to `offset(s0)`, handling large offsets via t6.
    pub(super) fn emit_store_to_s0(&mut self, reg: &str, offset: i64, store_instr: &str) {
        if Self::fits_imm12(offset) {
            self.state
                .emit_fmt(format_args!("    {} {}, {}(s0)", store_instr, reg, offset));
        } else {
            self.state.emit_fmt(format_args!("    li t6, {}", offset));
            self.state.emit("    add t6, s0, t6");
            self.state
                .emit_fmt(format_args!("    {} {}, 0(t6)", store_instr, reg));
        }
    }

    /// Emit: load from `offset(base)` into `dest`, handling large offsets via t6.
    pub(super) fn emit_load_from_reg(
        state: &mut crate::backend::state::CodegenState,
        dest: &str,
        base: &str,
        offset: i64,
        load_instr: &str,
    ) {
        if Self::fits_imm12(offset) {
            state.emit_fmt(format_args!(
                "    {} {}, {}({})",
                load_instr, dest, offset, base
            ));
        } else {
            state.emit_fmt(format_args!("    li t6, {}", offset));
            state.emit_fmt(format_args!("    add t6, {}, t6", base));
            state.emit_fmt(format_args!("    {} {}, 0(t6)", load_instr, dest));
        }
    }

    /// Emit: load from `offset(s0)` into `reg`, handling large offsets via t6.
    pub(super) fn emit_load_from_s0(&mut self, reg: &str, offset: i64, load_instr: &str) {
        if Self::fits_imm12(offset) {
            self.state
                .emit_fmt(format_args!("    {} {}, {}(s0)", load_instr, reg, offset));
        } else {
            self.state.emit_fmt(format_args!("    li t6, {}", offset));
            self.state.emit("    add t6, s0, t6");
            self.state
                .emit_fmt(format_args!("    {} {}, 0(t6)", load_instr, reg));
        }
    }

    /// Emit: `dest_reg = s0 + offset`, handling large offsets.
    pub(super) fn emit_addi_s0(&mut self, dest_reg: &str, offset: i64) {
        if Self::fits_imm12(offset) {
            self.state
                .emit_fmt(format_args!("    addi {}, s0, {}", dest_reg, offset));
        } else {
            self.state
                .emit_fmt(format_args!("    li {}, {}", dest_reg, offset));
            self.state
                .emit_fmt(format_args!("    add {}, s0, {}", dest_reg, dest_reg));
        }
    }

    /// Compute the address of an alloca into `dest`, handling over-aligned allocas.
    /// For normal allocas: `dest = s0 + offset`.
    /// For over-aligned allocas: `dest = (s0 + offset + align-1) & -align`.
    pub(super) fn emit_alloca_addr(&mut self, dest: &str, val_id: u32, offset: i64) {
        if let Some(align) = self.state.alloca_over_align(val_id) {
            self.emit_addi_s0(dest, offset);
            self.state
                .emit_fmt(format_args!("    li t6, {}", align - 1));
            self.state
                .emit_fmt(format_args!("    add {}, {}, t6", dest, dest));
            self.state.emit_fmt(format_args!("    li t6, -{}", align));
            self.state
                .emit_fmt(format_args!("    and {}, {}, t6", dest, dest));
        } else {
            self.emit_addi_s0(dest, offset);
        }
    }

    /// Emit: store `reg` to `offset(sp)`, handling large offsets via t6.
    pub(super) fn emit_store_to_sp(&mut self, reg: &str, offset: i64, store_instr: &str) {
        if Self::fits_imm12(offset) {
            self.state
                .emit_fmt(format_args!("    {} {}, {}(sp)", store_instr, reg, offset));
        } else {
            self.state.emit_fmt(format_args!("    li t6, {}", offset));
            self.state.emit("    add t6, sp, t6");
            self.state
                .emit_fmt(format_args!("    {} {}, 0(t6)", store_instr, reg));
        }
    }

    /// Emit: load from `offset(sp)` into `reg`, handling large offsets via t6.
    pub(super) fn emit_load_from_sp(&mut self, reg: &str, offset: i64, load_instr: &str) {
        if Self::fits_imm12(offset) {
            self.state
                .emit_fmt(format_args!("    {} {}, {}(sp)", load_instr, reg, offset));
        } else {
            self.state.emit_fmt(format_args!("    li t6, {}", offset));
            self.state.emit("    add t6, sp, t6");
            self.state
                .emit_fmt(format_args!("    {} {}, 0(t6)", load_instr, reg));
        }
    }

    /// Emit: `sp = sp + imm`, handling large immediates via t6.
    pub(super) fn emit_addi_sp(&mut self, imm: i64) {
        if Self::fits_imm12(imm) {
            self.state
                .emit_fmt(format_args!("    addi sp, sp, {}", imm));
        } else if imm > 0 {
            self.state.emit_fmt(format_args!("    li t6, {}", imm));
            self.state.emit("    add sp, sp, t6");
        } else {
            self.state.emit_fmt(format_args!("    li t6, {}", -imm));
            self.state.emit("    sub sp, sp, t6");
        }
    }

    /// Load an operand into t0.
    pub(super) fn operand_to_t0(&mut self, op: &Operand) {
        match op {
            Operand::Const(c) => {
                self.state.reg_cache.invalidate_acc();
                match c {
                    IrConst::I8(v) => self.state.emit_fmt(format_args!("    li t0, {}", v)),
                    IrConst::I16(v) => self.state.emit_fmt(format_args!("    li t0, {}", v)),
                    IrConst::I32(v) => self.state.emit_fmt(format_args!("    li t0, {}", v)),
                    IrConst::I64(v) => self.state.emit_fmt(format_args!("    li t0, {}", v)),
                    IrConst::F32(v) => {
                        let bits = v.to_bits() as u64;
                        self.state
                            .emit_fmt(format_args!("    li t0, {}", bits as i64));
                    }
                    IrConst::F64(v) => {
                        let bits = v.to_bits();
                        self.state
                            .emit_fmt(format_args!("    li t0, {}", bits as i64));
                    }
                    IrConst::LongDouble(v, _) => {
                        let bits = v.to_bits();
                        self.state
                            .emit_fmt(format_args!("    li t0, {}", bits as i64));
                    }
                    IrConst::I128(v) => self
                        .state
                        .emit_fmt(format_args!("    li t0, {}", *v as i64)),
                    IrConst::Zero => self.state.emit("    li t0, 0"),
                }
            }
            Operand::Value(v) => {
                let is_alloca = self.state.is_alloca(v.0);
                if self.state.reg_cache.acc_has(v.0, is_alloca) {
                    return;
                }
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    let reg_name = callee_saved_name(reg);
                    self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
                    self.state.reg_cache.set_acc(v.0, false);
                } else if let Some(slot) = self.state.get_slot(v.0) {
                    if is_alloca {
                        self.emit_alloca_addr("t0", v.0, slot.0);
                    } else {
                        self.emit_load_from_s0("t0", slot.0, "ld");
                    }
                    self.state.reg_cache.set_acc(v.0, is_alloca);
                } else if self.state.reg_cache.acc_has(v.0, false)
                    || self.state.reg_cache.acc_has(v.0, true)
                {
                } else {
                    self.state.emit("    li t0, 0");
                    self.state.reg_cache.invalidate_acc();
                }
            }
        }
    }

    /// Store t0 to a value's location (register or stack slot).
    pub(super) fn store_t0_to(&mut self, dest: &Value) {
        if let Some(&reg) = self.reg_assignments.get(&dest.0) {
            let reg_name = callee_saved_name(reg);
            self.state.emit_fmt(format_args!("    mv {}, t0", reg_name));
        } else if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_s0("t0", slot.0, "sd");
        }
        self.state.reg_cache.set_acc(dest.0, false);
    }

    // --- 128-bit integer helpers ---

    /// Load a 128-bit operand into t0 (low) : t1 (high).
    pub(super) fn operand_to_t0_t1(&mut self, op: &Operand) {
        match op {
            Operand::Const(c) => match c {
                IrConst::I128(v) => {
                    let low = *v as u64 as i64;
                    let high = (*v >> 64) as u64 as i64;
                    self.state.emit_fmt(format_args!("    li t0, {}", low));
                    self.state.emit_fmt(format_args!("    li t1, {}", high));
                }
                IrConst::Zero => {
                    self.state.emit("    li t0, 0");
                    self.state.emit("    li t1, 0");
                }
                _ => {
                    self.operand_to_t0(op);
                    self.state.emit("    li t1, 0");
                }
            },
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    if self.state.is_alloca(v.0) {
                        self.emit_alloca_addr("t0", v.0, slot.0);
                        self.state.emit("    li t1, 0");
                    } else if self.state.is_i128_value(v.0) {
                        self.emit_load_from_s0("t0", slot.0, "ld");
                        self.emit_load_from_s0("t1", slot.0 + 8, "ld");
                    } else {
                        if let Some(&reg) = self.reg_assignments.get(&v.0) {
                            let reg_name = callee_saved_name(reg);
                            self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
                        } else {
                            self.emit_load_from_s0("t0", slot.0, "ld");
                        }
                        self.state.emit("    li t1, 0");
                    }
                } else if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    let reg_name = callee_saved_name(reg);
                    self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
                    self.state.emit("    li t1, 0");
                } else {
                    self.state.emit("    li t0, 0");
                    self.state.emit("    li t1, 0");
                }
            }
        }
    }

    /// Store t0 (low) : t1 (high) to a 128-bit value's stack slot.
    pub(super) fn store_t0_t1_to(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_s0("t0", slot.0, "sd");
            self.emit_store_to_s0("t1", slot.0 + 8, "sd");
        }
    }

    /// Prepare a 128-bit binary operation: load lhs into t3:t4, rhs into t5:t6.
    pub(super) fn prep_i128_binop(&mut self, lhs: &Operand, rhs: &Operand) {
        self.operand_to_t0_t1(lhs);
        self.state.emit("    mv t3, t0");
        self.state.emit("    mv t4, t1");
        self.operand_to_t0_t1(rhs);
        self.state.emit("    mv t5, t0");
        self.state.emit("    mv t6, t1");
    }

    pub(super) fn store_for_type(ty: IrType) -> &'static str {
        match ty {
            IrType::I8 | IrType::U8 => "sb",
            IrType::I16 | IrType::U16 => "sh",
            IrType::I32 | IrType::U32 | IrType::F32 => "sw",
            _ => "sd",
        }
    }

    pub(super) fn load_for_type(ty: IrType) -> &'static str {
        match ty {
            IrType::I8 => "lb",
            IrType::U8 => "lbu",
            IrType::I16 => "lh",
            IrType::U16 => "lhu",
            IrType::I32 => "lw",
            IrType::U32 | IrType::F32 => "lwu",
            _ => "ld",
        }
    }

    /// Load the address of a pointer Value into the given register.
    pub(super) fn load_ptr_to_reg_rv(&mut self, ptr: &Value, reg: &str) {
        if let Some(slot) = self.state.get_slot(ptr.0) {
            if self.state.is_alloca(ptr.0) {
                self.emit_alloca_addr(reg, ptr.0, slot.0);
            } else {
                self.emit_load_from_s0(reg, slot.0, "ld");
            }
        }
    }
}

impl ArchCodegen for RiscvCodegen {
    fn state(&mut self) -> &mut CodegenState {
        &mut self.state
    }
    fn state_ref(&self) -> &CodegenState {
        &self.state
    }
    fn ptr_directive(&self) -> PtrDirective {
        PtrDirective::Dword
    }

    fn get_phys_reg_for_value(&self, val_id: u32) -> Option<PhysReg> {
        self.reg_assignments.get(&val_id).copied()
    }

    fn emit_reg_to_reg_move(&mut self, src: PhysReg, dest: PhysReg) {
        let s_name = callee_saved_name(src);
        let d_name = callee_saved_name(dest);
        self.state
            .emit_fmt(format_args!("    mv {}, {}", d_name, s_name));
    }

    fn emit_acc_to_phys_reg(&mut self, dest: PhysReg) {
        let d_name = callee_saved_name(dest);
        self.state.emit_fmt(format_args!("    mv {}, t0", d_name));
    }

    fn jump_mnemonic(&self) -> &'static str {
        "j"
    }
    fn trap_instruction(&self) -> &'static str {
        "ebreak"
    }

    fn emit_branch(&mut self, label: &str) {
        self.state.emit_fmt(format_args!("    jump {}, t6", label));
    }

    fn emit_branch_to_block(&mut self, block: BlockId) {
        let out = &mut self.state.out;
        out.write_str("    jump .LBB");
        out.write_u64(block.0 as u64);
        out.write_str(", t6");
        out.newline();
    }

    fn emit_branch_nonzero(&mut self, label: &str) {
        let skip = self.state.fresh_label("skip");
        self.state.emit_fmt(format_args!("    beqz t0, {}", skip));
        self.state.emit_fmt(format_args!("    jump {}, t6", label));
        self.state.emit_fmt(format_args!("{}:", skip));
    }

    fn emit_jump_indirect(&mut self) {
        self.state.emit("    jr t0");
    }

    fn emit_switch_case_branch(&mut self, case_val: i64, label: &str, ty: IrType) {
        let skip = self.state.fresh_label("sw_skip");
        let use_32bit = matches!(
            ty,
            IrType::I32 | IrType::U32 | IrType::I16 | IrType::U16 | IrType::I8 | IrType::U8
        );
        if use_32bit {
            // Sign-extend both values to match; li already sign-extends on RV64
            self.state
                .emit_fmt(format_args!("    li t1, {}", case_val as i32 as i64));
            self.state.emit("    sext.w t2, t0");
            self.state
                .emit_fmt(format_args!("    bne t2, t1, {}", skip));
        } else {
            self.state.emit_fmt(format_args!("    li t1, {}", case_val));
            self.state
                .emit_fmt(format_args!("    bne t0, t1, {}", skip));
        }
        self.state.emit_fmt(format_args!("    jump {}, t6", label));
        self.state.emit_fmt(format_args!("{}:", skip));
    }

    fn emit_switch_jump_table(
        &mut self,
        val: &Operand,
        cases: &[(i64, BlockId)],
        default: &BlockId,
        _ty: IrType,
    ) {
        use crate::backend::traits::build_jump_table;
        let (table, min_val, range) = build_jump_table(cases, default);
        let table_label = self.state.fresh_label("jt");
        let default_label = default.as_label();

        self.operand_to_t0(val);

        if min_val != 0 {
            let neg_min = -min_val;
            if (-2048..=2047).contains(&neg_min) {
                self.state
                    .emit_fmt(format_args!("    addi t0, t0, {}", neg_min));
            } else {
                self.state.emit_fmt(format_args!("    li t1, {}", neg_min));
                self.state.emit("    add t0, t0, t1");
            }
        }
        let range_ok = self.state.fresh_label("range_ok");
        self.state.emit_fmt(format_args!("    li t1, {}", range));
        self.state
            .emit_fmt(format_args!("    bltu t0, t1, {}", range_ok));
        self.state
            .emit_fmt(format_args!("    jump {}, t6", default_label));
        self.state.emit_fmt(format_args!("{}:", range_ok));

        self.state
            .emit_fmt(format_args!("    lla t1, {}", table_label));
        self.state.emit("    slli t0, t0, 2");
        self.state.emit("    add t1, t1, t0");
        self.state.emit("    lw t0, 0(t1)");
        self.state
            .emit_fmt(format_args!("    lla t1, {}", table_label));
        self.state.emit("    add t1, t1, t0");
        self.state.emit("    jr t1");

        self.state.emit(".section .rodata");
        self.state.emit(".align 2");
        self.state.emit_fmt(format_args!("{}:", table_label));
        for target in &table {
            let target_label = target.as_label();
            self.state
                .emit_fmt(format_args!("    .word {} - {}", target_label, table_label));
        }
        let sect = self.state.current_text_section.clone();
        self.state
            .emit_fmt(format_args!(".section {},\"ax\",@progbits", sect));
        self.state.reg_cache.invalidate_all();
    }

    // ---- Standard trait methods (kept inline - arch-specific) ----
    fn emit_load_operand(&mut self, op: &Operand) {
        self.operand_to_t0(op)
    }
    fn emit_store_result(&mut self, dest: &Value) {
        self.store_t0_to(dest)
    }
    fn emit_save_acc(&mut self) {
        self.state.emit("    mv t3, t0");
    }
    fn emit_typed_store_indirect(&mut self, instr: &'static str, _ty: IrType) {
        self.state.emit_fmt(format_args!("    {} t3, 0(t5)", instr));
    }
    fn emit_typed_load_indirect(&mut self, instr: &'static str) {
        self.state.emit_fmt(format_args!("    {} t0, 0(t5)", instr));
    }
    fn emit_add_secondary_to_acc(&mut self) {
        self.state.emit("    add t0, t1, t0");
    }
    fn emit_gep_add_const_to_acc(&mut self, offset: i64) {
        if offset != 0 {
            self.emit_add_imm_to_acc_impl(offset);
        }
    }
    fn emit_acc_to_secondary(&mut self) {
        self.state.emit("    mv t1, t0");
    }
    fn emit_memcpy_store_dest_from_acc(&mut self) {
        self.state.emit("    mv t1, t5");
    }
    fn emit_memcpy_store_src_from_acc(&mut self) {
        self.state.emit("    mv t2, t5");
    }
    fn emit_call_store_i128_result(&mut self, _dest: &Value) {
        unreachable!("RISC-V uses custom emit_call_store_result");
    }
    fn emit_call_store_f128_result(&mut self, _dest: &Value) {
        unreachable!("RISC-V uses custom emit_call_store_result");
    }
    fn emit_call_move_f32_to_acc(&mut self) {
        self.state.emit("    fmv.x.w t0, fa0");
    }
    fn emit_call_move_f64_to_acc(&mut self) {
        self.state.emit("    fmv.x.d t0, fa0");
    }
    fn current_return_type(&self) -> IrType {
        self.current_return_type
    }
    fn emit_get_return_f128_second(&mut self, _dest: &Value) {}
    fn emit_set_return_f128_second(&mut self, _src: &Operand) {}
    fn emit_va_arg_struct(&mut self, _dest_ptr: &Value, _va_list_ptr: &Value, _size: usize) {
        panic!("VaArgStruct should not be emitted for RISC-V target");
    }

    // ---- ALU with different impl names ----
    fn emit_int_clz(&mut self, ty: IrType) {
        self.emit_clz(ty)
    }
    fn emit_int_ctz(&mut self, ty: IrType) {
        self.emit_ctz(ty)
    }
    fn emit_int_bswap(&mut self, ty: IrType) {
        self.emit_bswap(ty)
    }
    fn emit_int_popcount(&mut self, ty: IrType) {
        self.emit_popcount(ty)
    }

    // ---- Float binop body uses different name ----
    fn emit_float_binop_impl(&mut self, mnemonic: &str, ty: IrType) {
        self.emit_float_binop_body(mnemonic, ty)
    }

    // ---- Inline asm / intrinsics ----
    fn emit_inline_asm(
        &mut self,
        template: &str,
        outputs: &[(String, Value, Option<String>)],
        inputs: &[(String, Operand, Option<String>)],
        clobbers: &[String],
        operand_types: &[IrType],
        goto_labels: &[(String, BlockId)],
        input_symbols: &[Option<String>],
    ) {
        emit_inline_asm_common(
            self,
            template,
            outputs,
            inputs,
            clobbers,
            operand_types,
            goto_labels,
            input_symbols,
        );
        // Restore any callee-saved registers that were borrowed by the scratch
        // allocator. These were saved to the stack before input loading and need
        // to be restored now that all output stores are complete.
        // Entries are "saved:<reg>" after the save was emitted in load_input_to_reg.
        let borrowed = std::mem::take(&mut self.asm_borrowed_callee_saved);
        if !borrowed.is_empty() {
            for entry in borrowed.iter().rev() {
                if let Some(reg) = entry.strip_prefix("saved:") {
                    self.state.emit_fmt(format_args!("    ld {}, 0(sp)", reg));
                    self.state.emit("    addi sp, sp, 16");
                }
            }
        }
    }
    fn emit_intrinsic(
        &mut self,
        dest: &Option<Value>,
        op: &IntrinsicOp,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
        self.emit_intrinsic_rv(dest, op, dest_ptr, args);
    }

    // All remaining methods delegate to self.method_name_impl(args...)
    delegate_to_impl! {
        // prologue
        fn calculate_stack_space(&mut self, func: &IrFunction) -> i64 => calculate_stack_space_impl;
        fn aligned_frame_size(&self, raw_space: i64) -> i64 => aligned_frame_size_impl;
        fn emit_prologue(&mut self, func: &IrFunction, frame_size: i64) => emit_prologue_impl;
        fn emit_epilogue(&mut self, frame_size: i64) => emit_epilogue_impl;
        fn emit_store_params(&mut self, func: &IrFunction) => emit_store_params_impl;
        fn emit_param_ref(&mut self, dest: &Value, param_idx: usize, ty: IrType) => emit_param_ref_impl;
        fn emit_epilogue_and_ret(&mut self, frame_size: i64) => emit_epilogue_and_ret_impl;
        fn store_instr_for_type(&self, ty: IrType) -> &'static str => store_instr_for_type_impl;
        fn load_instr_for_type(&self, ty: IrType) -> &'static str => load_instr_for_type_impl;
        // memory
        fn emit_store(&mut self, val: &Operand, ptr: &Value, ty: IrType) => emit_store_impl;
        fn emit_load(&mut self, dest: &Value, ptr: &Value, ty: IrType) => emit_load_impl;
        fn emit_store_with_const_offset(&mut self, val: &Operand, base: &Value, offset: i64, ty: IrType) => emit_store_with_const_offset_impl;
        fn emit_load_with_const_offset(&mut self, dest: &Value, base: &Value, offset: i64, ty: IrType) => emit_load_with_const_offset_impl;
        fn emit_typed_store_to_slot(&mut self, instr: &'static str, ty: IrType, slot: StackSlot) => emit_typed_store_to_slot_impl;
        fn emit_typed_load_from_slot(&mut self, instr: &'static str, slot: StackSlot) => emit_typed_load_from_slot_impl;
        fn emit_load_ptr_from_slot(&mut self, slot: StackSlot, val_id: u32) => emit_load_ptr_from_slot_impl;
        fn emit_add_offset_to_addr_reg(&mut self, offset: i64) => emit_add_offset_to_addr_reg_impl;
        fn emit_slot_addr_to_secondary(&mut self, slot: StackSlot, is_alloca: bool, val_id: u32) => emit_slot_addr_to_secondary_impl;
        fn emit_gep_direct_const(&mut self, slot: StackSlot, offset: i64) => emit_gep_direct_const_impl;
        fn emit_gep_indirect_const(&mut self, slot: StackSlot, offset: i64, val_id: u32) => emit_gep_indirect_const_impl;
        fn emit_add_imm_to_acc(&mut self, imm: i64) => emit_add_imm_to_acc_impl;
        fn emit_round_up_acc_to_16(&mut self) => emit_round_up_acc_to_16_impl;
        fn emit_sub_sp_by_acc(&mut self) => emit_sub_sp_by_acc_impl;
        fn emit_mov_sp_to_acc(&mut self) => emit_mov_sp_to_acc_impl;
        fn emit_mov_acc_to_sp(&mut self) => emit_mov_acc_to_sp_impl;
        fn emit_align_acc(&mut self, align: usize) => emit_align_acc_impl;
        fn emit_memcpy_load_dest_addr(&mut self, slot: StackSlot, is_alloca: bool, val_id: u32) => emit_memcpy_load_dest_addr_impl;
        fn emit_memcpy_load_src_addr(&mut self, slot: StackSlot, is_alloca: bool, val_id: u32) => emit_memcpy_load_src_addr_impl;
        fn emit_alloca_aligned_addr(&mut self, slot: StackSlot, val_id: u32) => emit_alloca_aligned_addr_impl;
        fn emit_alloca_aligned_addr_to_acc(&mut self, slot: StackSlot, val_id: u32) => emit_alloca_aligned_addr_to_acc_impl;
        fn emit_memcpy_impl(&mut self, size: usize) => emit_memcpy_impl_impl;
        // alu
        fn emit_float_neg(&mut self, ty: IrType) => emit_float_neg_impl;
        fn emit_int_neg(&mut self, ty: IrType) => emit_int_neg_impl;
        fn emit_int_not(&mut self, ty: IrType) => emit_int_not_impl;
        fn emit_int_binop(&mut self, dest: &Value, op: IrBinOp, lhs: &Operand, rhs: &Operand, ty: IrType) => emit_int_binop_impl;
        fn emit_copy_i128(&mut self, dest: &Value, src: &Operand) => emit_copy_i128_impl;
        // comparison
        fn emit_float_cmp(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) => emit_float_cmp_impl;
        fn emit_f128_cmp(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand) => emit_f128_cmp_impl;
        fn emit_int_cmp(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) => emit_int_cmp_impl;
        fn emit_fused_cmp_branch(&mut self, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType, true_label: &str, false_label: &str) => emit_fused_cmp_branch_impl;
        fn emit_select(&mut self, dest: &Value, cond: &Operand, true_val: &Operand, false_val: &Operand, ty: IrType) => emit_select_impl;
        // calls
        fn call_abi_config(&self) -> CallAbiConfig => call_abi_config_impl;
        fn emit_call_compute_stack_space(&self, arg_classes: &[CallArgClass], arg_types: &[IrType]) -> usize => emit_call_compute_stack_space_impl;
        fn emit_call_f128_pre_convert(&mut self, args: &[Operand], arg_classes: &[CallArgClass], arg_types: &[IrType], stack_arg_space: usize) -> usize => emit_call_f128_pre_convert_impl;
        fn emit_call_stack_args(&mut self, args: &[Operand], arg_classes: &[CallArgClass], arg_types: &[IrType], stack_arg_space: usize, fptr_spill: usize, f128_temp_space: usize) -> i64 => emit_call_stack_args_impl;
        fn emit_call_reg_args(&mut self, args: &[Operand], arg_classes: &[CallArgClass], arg_types: &[IrType], total_sp_adjust: i64, f128_temp_space: usize, stack_arg_space: usize, struct_arg_riscv_float_classes: &[Option<crate::common::types::RiscvFloatClass>]) => emit_call_reg_args_impl;
        fn emit_call_instruction(&mut self, direct_name: Option<&str>, func_ptr: Option<&Operand>, indirect: bool, stack_arg_space: usize) => emit_call_instruction_impl;
        fn emit_call_cleanup(&mut self, stack_arg_space: usize, f128_temp_space: usize, indirect: bool) => emit_call_cleanup_impl;
        fn emit_call_store_result(&mut self, dest: &Value, return_type: IrType) => emit_call_store_result_impl;
        // globals
        fn emit_global_addr(&mut self, dest: &Value, name: &str) => emit_global_addr_impl;
        fn emit_label_addr(&mut self, dest: &Value, label: &str) => emit_label_addr_impl;
        fn emit_tls_global_addr(&mut self, dest: &Value, name: &str) => emit_tls_global_addr_impl;
        // cast
        fn emit_cast_instrs(&mut self, from_ty: IrType, to_ty: IrType) => emit_cast_instrs_impl;
        fn emit_cast(&mut self, dest: &Value, src: &Operand, from_ty: IrType, to_ty: IrType) => emit_cast_impl;
        // variadic
        fn emit_va_arg(&mut self, dest: &Value, va_list_ptr: &Value, result_ty: IrType) => emit_va_arg_impl;
        fn emit_va_start(&mut self, va_list_ptr: &Value) => emit_va_start_impl;
        fn emit_va_copy(&mut self, dest_ptr: &Value, src_ptr: &Value) => emit_va_copy_impl;
        // returns
        fn emit_return(&mut self, val: Option<&Operand>, frame_size: i64) => emit_return_impl;
        fn emit_return_i128_to_regs(&mut self) => emit_return_i128_to_regs_impl;
        fn emit_return_f128_to_reg(&mut self) => emit_return_f128_to_reg_impl;
        fn emit_return_f32_to_reg(&mut self) => emit_return_f32_to_reg_impl;
        fn emit_return_f64_to_reg(&mut self) => emit_return_f64_to_reg_impl;
        fn emit_return_int_to_reg(&mut self) => emit_return_int_to_reg_impl;
        fn emit_get_return_f64_second(&mut self, dest: &Value) => emit_get_return_f64_second_impl;
        fn emit_set_return_f64_second(&mut self, src: &Operand) => emit_set_return_f64_second_impl;
        fn emit_get_return_f32_second(&mut self, dest: &Value) => emit_get_return_f32_second_impl;
        fn emit_set_return_f32_second(&mut self, src: &Operand) => emit_set_return_f32_second_impl;
        // atomics
        fn emit_atomic_rmw(&mut self, dest: &Value, op: AtomicRmwOp, ptr: &Operand, val: &Operand, ty: IrType, ordering: AtomicOrdering) => emit_atomic_rmw_impl;
        fn emit_atomic_cmpxchg(&mut self, dest: &Value, ptr: &Operand, expected: &Operand, desired: &Operand, ty: IrType, ordering: AtomicOrdering, failure_ordering: AtomicOrdering, returns_bool: bool) => emit_atomic_cmpxchg_impl;
        fn emit_atomic_load(&mut self, dest: &Value, ptr: &Operand, ty: IrType, ordering: AtomicOrdering) => emit_atomic_load_impl;
        fn emit_atomic_store(&mut self, ptr: &Operand, val: &Operand, ty: IrType, ordering: AtomicOrdering) => emit_atomic_store_impl;
        fn emit_fence(&mut self, ordering: AtomicOrdering) => emit_fence_impl;
        // float ops
        fn emit_float_binop(&mut self, dest: &Value, op: FloatOp, lhs: &Operand, rhs: &Operand, ty: IrType) => emit_float_binop_impl;
        fn emit_f128_neg(&mut self, dest: &Value, src: &Operand) => emit_f128_neg_impl;
        // i128 ops
        fn emit_load_acc_pair(&mut self, op: &Operand) => emit_load_acc_pair_impl;
        fn emit_store_acc_pair(&mut self, dest: &Value) => emit_store_acc_pair_impl;
        fn emit_store_pair_to_slot(&mut self, slot: StackSlot) => emit_store_pair_to_slot_impl;
        fn emit_load_pair_from_slot(&mut self, slot: StackSlot) => emit_load_pair_from_slot_impl;
        fn emit_save_acc_pair(&mut self) => emit_save_acc_pair_impl;
        fn emit_store_pair_indirect(&mut self) => emit_store_pair_indirect_impl;
        fn emit_load_pair_indirect(&mut self) => emit_load_pair_indirect_impl;
        fn emit_i128_neg(&mut self) => emit_i128_neg_impl;
        fn emit_i128_not(&mut self) => emit_i128_not_impl;
        fn emit_sign_extend_acc_high(&mut self) => emit_sign_extend_acc_high_impl;
        fn emit_zero_acc_high(&mut self) => emit_zero_acc_high_impl;
        fn emit_i128_prep_binop(&mut self, lhs: &Operand, rhs: &Operand) => emit_i128_prep_binop_impl;
        fn emit_i128_add(&mut self) => emit_i128_add_impl;
        fn emit_i128_sub(&mut self) => emit_i128_sub_impl;
        fn emit_i128_mul(&mut self) => emit_i128_mul_impl;
        fn emit_i128_and(&mut self) => emit_i128_and_impl;
        fn emit_i128_or(&mut self) => emit_i128_or_impl;
        fn emit_i128_xor(&mut self) => emit_i128_xor_impl;
        fn emit_i128_shl(&mut self) => emit_i128_shl_impl;
        fn emit_i128_lshr(&mut self) => emit_i128_lshr_impl;
        fn emit_i128_ashr(&mut self) => emit_i128_ashr_impl;
        fn emit_i128_prep_shift_lhs(&mut self, lhs: &Operand) => emit_i128_prep_shift_lhs_impl;
        fn emit_i128_shl_const(&mut self, amount: u32) => emit_i128_shl_const_impl;
        fn emit_i128_lshr_const(&mut self, amount: u32) => emit_i128_lshr_const_impl;
        fn emit_i128_ashr_const(&mut self, amount: u32) => emit_i128_ashr_const_impl;
        fn emit_i128_divrem_call(&mut self, func_name: &str, lhs: &Operand, rhs: &Operand) => emit_i128_divrem_call_impl;
        fn emit_i128_store_result(&mut self, dest: &Value) => emit_i128_store_result_impl;
        fn emit_i128_to_float_call(&mut self, src: &Operand, from_signed: bool, to_ty: IrType) => emit_i128_to_float_call_impl;
        fn emit_float_to_i128_call(&mut self, src: &Operand, to_signed: bool, from_ty: IrType) => emit_float_to_i128_call_impl;
        fn emit_i128_cmp_eq(&mut self, is_ne: bool) => emit_i128_cmp_eq_impl;
        fn emit_i128_cmp_ordered(&mut self, op: IrCmpOp) => emit_i128_cmp_ordered_impl;
        fn emit_i128_cmp_store_result(&mut self, dest: &Value) => emit_i128_cmp_store_result_impl;
    }
}

impl Default for RiscvCodegen {
    fn default() -> Self {
        Self::new()
    }
}
