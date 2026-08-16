//! i686 (32-bit x86) code generator. Implements the ArchCodegen trait.
//!
//! Uses the cdecl calling convention (System V i386 ABI):
//! - All arguments passed on the stack, pushed right-to-left
//! - Return values: eax (32-bit), eax:edx (64-bit), st(0) for float/double/long double
//! - Callee-saved: ebx, esi, edi, ebp
//! - Caller-saved: eax, ecx, edx
//! - No register-based argument passing (unlike x86-64 SysV ABI)
//! - Stack aligned to 16 bytes at call sites (modern i386 ABI)

use crate::backend::call_abi;
use crate::backend::common::PtrDirective;
use crate::backend::generation::is_i128_type;
use crate::backend::regalloc::PhysReg;
use crate::backend::state::{CodegenState, StackSlot};
use crate::backend::traits::ArchCodegen;
use crate::common::fx_hash::FxHashMap;
use crate::common::types::{AddressSpace, IrType};
use crate::delegate_to_impl;
use crate::emit;
use crate::ir::reexports::{
    AtomicOrdering, AtomicRmwOp, BlockId, IntrinsicOp, IrBinOp, IrCmpOp, IrConst, IrFunction,
    IrUnaryOp, Operand, Value,
};

/// i686 code generator. Implements the ArchCodegen trait for the shared framework.
/// Uses cdecl calling convention with no register allocation (accumulator-based).
pub struct I686Codegen {
    pub(crate) state: CodegenState,
    pub(super) current_return_type: IrType,
    /// Whether the current function is variadic
    pub(super) is_variadic: bool,
    /// Register allocation results (callee-saved registers: ebx, esi, edi)
    pub(super) reg_assignments: FxHashMap<u32, PhysReg>,
    /// Which callee-saved registers are used and need save/restore
    pub(super) used_callee_saved: Vec<PhysReg>,
    /// Total stack bytes consumed by named parameters (for va_start computation).
    pub(super) va_named_stack_bytes: usize,
    /// Scratch register allocation index for inline asm GP registers.
    pub(super) asm_scratch_idx: usize,
    /// Scratch register allocation index for inline asm XMM registers.
    pub(super) asm_xmm_scratch_idx: usize,
    /// Whether the current function uses the fastcall calling convention.
    pub(super) is_fastcall: bool,
    /// For fastcall functions, the number of bytes of stack args the callee must pop on return.
    pub(super) fastcall_stack_cleanup: usize,
    /// For fastcall functions, how many leading params are passed in registers (0, 1, or 2).
    pub(super) fastcall_reg_param_count: usize,
    /// Whether the __x86.get_pc_thunk.bx helper needs to be emitted.
    pub(super) needs_pc_thunk_bx: bool,
    /// Number of integer arguments to pass in registers (-mregparm=N).
    /// 0 = standard cdecl, 1-3 = pass first N int args in EAX, EDX, ECX.
    pub(super) regparm: u8,
    /// Whether to omit the frame pointer (-fomit-frame-pointer).
    pub(super) omit_frame_pointer: bool,
    /// When omit_frame_pointer is true, this holds the offset from ESP (at its
    /// base position after prologue) to where EBP would have pointed.
    /// slot_ref() adds this to convert EBP-relative slot offsets to ESP-relative.
    /// Value: frame_size + callee_saved_bytes (without the pushed EBP).
    pub(super) frame_base_offset: i64,
    /// Tracks temporary ESP adjustments (e.g., subl $N,%esp for call args,
    /// subl $4,%esp for f32 conversion). Incremented on subl, decremented on addl.
    /// Added to frame_base_offset in slot_ref() to get the correct ESP offset.
    pub(super) esp_adjust: i64,
}

// Callee-saved physical register indices for i686
// PhysReg(0) = ebx, PhysReg(1) = esi, PhysReg(2) = edi, PhysReg(3) = ebp
pub(super) const I686_CALLEE_SAVED: &[PhysReg] = &[PhysReg(0), PhysReg(1), PhysReg(2)];
// Extended callee-saved list including ebp (used when -fomit-frame-pointer)
pub(super) const I686_CALLEE_SAVED_WITH_EBP: &[PhysReg] =
    &[PhysReg(0), PhysReg(1), PhysReg(2), PhysReg(3)];
// No caller-saved registers available for allocation (eax/ecx/edx are scratch)
pub(super) const I686_CALLER_SAVED: &[PhysReg] = &[];

pub(super) fn phys_reg_name(reg: PhysReg) -> &'static str {
    match reg.0 {
        0 => "ebx",
        1 => "esi",
        2 => "edi",
        3 => "ebp",
        _ => panic!("invalid i686 phys reg: {:?}", reg),
    }
}

/// Map inline asm constraint register names to callee-saved PhysReg indices.
pub(super) fn i686_constraint_to_phys(constraint: &str) -> Option<PhysReg> {
    match constraint {
        "b" | "{ebx}" | "ebx" => Some(PhysReg(0)),
        "S" | "{esi}" | "esi" => Some(PhysReg(1)),
        "D" | "{edi}" | "edi" => Some(PhysReg(2)),
        _ => None,
    }
}

/// Map inline asm clobber register names to callee-saved PhysReg indices.
pub(super) fn i686_clobber_to_phys(clobber: &str) -> Option<PhysReg> {
    match clobber {
        "ebx" | "~{ebx}" => Some(PhysReg(0)),
        "esi" | "~{esi}" => Some(PhysReg(1)),
        "edi" | "~{edi}" => Some(PhysReg(2)),
        _ => None,
    }
}

impl I686Codegen {
    pub fn new() -> Self {
        Self {
            state: CodegenState::new(),
            current_return_type: IrType::I32,
            is_variadic: false,
            reg_assignments: FxHashMap::default(),
            used_callee_saved: Vec::new(),
            va_named_stack_bytes: 0,
            asm_scratch_idx: 0,
            asm_xmm_scratch_idx: 0,
            is_fastcall: false,
            fastcall_stack_cleanup: 0,
            fastcall_reg_param_count: 0,
            needs_pc_thunk_bx: false,
            regparm: 0,
            omit_frame_pointer: false,
            frame_base_offset: 0,
            esp_adjust: 0,
        }
    }

    pub fn set_pic(&mut self, pic: bool) {
        self.state.pic_mode = pic;
    }

    pub fn set_no_jump_tables(&mut self, enabled: bool) {
        self.state.no_jump_tables = enabled;
    }

    /// Apply all relevant options from a `CodegenOptions` struct.
    pub fn apply_options(&mut self, opts: &crate::backend::CodegenOptions) {
        self.set_pic(opts.pic);
        self.set_no_jump_tables(opts.no_jump_tables);
        self.regparm = opts.regparm;
        self.omit_frame_pointer = opts.omit_frame_pointer;
        self.state.emit_cfi = opts.emit_cfi;
    }

    // --- i686 helper methods ---

    /// Format a stack slot reference as either `offset(%ebp)` or `offset(%esp)`.
    /// When frame pointer is omitted, converts EBP-relative offsets to ESP-relative
    /// by adding frame_base_offset + esp_adjust.
    pub(super) fn slot_ref(&self, slot: StackSlot) -> String {
        if self.omit_frame_pointer {
            let esp_off = slot.0 + self.frame_base_offset + self.esp_adjust;
            format!("{}(%esp)", esp_off)
        } else {
            format!("{}(%ebp)", slot.0)
        }
    }

    /// Format a stack slot reference with an additional byte offset.
    /// Used for accessing sub-fields of multi-byte slots (e.g., upper 4 bytes of i64).
    pub(super) fn slot_ref_offset(&self, slot: StackSlot, extra: i64) -> String {
        if self.omit_frame_pointer {
            let esp_off = slot.0 + extra + self.frame_base_offset + self.esp_adjust;
            format!("{}(%esp)", esp_off)
        } else {
            format!("{}(%ebp)", slot.0 + extra)
        }
    }

    /// Format a parameter reference from the caller's stack frame.
    /// With frame pointer: offset(%ebp) where offset is positive (above EBP).
    /// Without frame pointer: params are at ESP + frame_base_offset + esp_adjust + offset,
    /// but we need to subtract 4 because there's no pushed EBP taking up space.
    /// The param_offset is the EBP-relative offset (e.g., 8 for first param).
    pub(super) fn param_ref(&self, ebp_offset: i64) -> String {
        if self.omit_frame_pointer {
            // Without pushed EBP, params are 4 bytes closer to the frame.
            // EBP would have been at original_esp - 4 (after push ebp).
            // Without push ebp, the return address is at original_esp.
            // So param at ebp_offset(%ebp) = (ebp_offset - 4) relative to original_esp.
            // And relative to current ESP: ebp_offset - 4 + frame_base_offset + esp_adjust.
            let esp_off = ebp_offset - 4 + self.frame_base_offset + self.esp_adjust;
            format!("{}(%esp)", esp_off)
        } else {
            format!("{}(%ebp)", ebp_offset)
        }
    }

    pub(super) fn dest_reg(&self, dest: &Value) -> Option<PhysReg> {
        self.reg_assignments.get(&dest.0).copied()
    }

    /// Load the address of va_list storage into %edx.
    ///
    /// va_list_ptr is an IR value that holds a pointer to the va_list storage.
    /// - If va_list_ptr is an alloca (local va_list variable), we LEA the slot
    ///   address into %edx (the alloca IS the va_list storage).
    /// - If va_list_ptr is a regular value (e.g., loaded pointer from va_list*),
    ///   we load its value into %edx (the value IS the address of va_list storage).
    pub(super) fn load_va_list_addr_to_edx(&mut self, va_list_ptr: &Value) {
        let is_alloca = self.state.is_alloca(va_list_ptr.0);
        if let Some(phys) = self.reg_assignments.get(&va_list_ptr.0).copied() {
            // Value is in a callee-saved register (non-alloca pointer value)
            let reg = phys_reg_name(phys);
            emit!(self.state, "    movl %{}, %edx", reg);
        } else if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
            let sr = self.slot_ref(slot);
            if is_alloca {
                // Alloca: the slot IS the va_list; get the address of the slot
                emit!(self.state, "    leal {}, %edx", sr);
            } else {
                // Regular value: the slot holds a pointer to the va_list storage
                emit!(self.state, "    movl {}, %edx", sr);
            }
        }
    }

    /// Load an operand into %eax.
    pub(super) fn operand_to_eax(&mut self, op: &Operand) {
        // Check register cache - skip load if value is already in eax
        if let Operand::Value(v) = op {
            let is_alloca = self.state.is_alloca(v.0);
            if self.state.reg_cache.acc_has(v.0, is_alloca) {
                return;
            }
        }

        match op {
            Operand::Const(c) => {
                match c {
                    IrConst::I8(v) => emit!(self.state, "    movl ${}, %eax", *v as i32),
                    IrConst::I16(v) => emit!(self.state, "    movl ${}, %eax", *v as i32),
                    IrConst::I32(v) => {
                        if *v == 0 {
                            self.state.emit("    xorl %eax, %eax");
                        } else {
                            emit!(self.state, "    movl ${}, %eax", v);
                        }
                    }
                    IrConst::I64(v) => {
                        // On i686, we can only hold 32 bits in eax
                        // Truncate to low 32 bits
                        let low = *v as i32;
                        if low == 0 {
                            self.state.emit("    xorl %eax, %eax");
                        } else {
                            emit!(self.state, "    movl ${}, %eax", low);
                        }
                    }
                    IrConst::I128(v) => {
                        let low = *v as i32;
                        emit!(self.state, "    movl ${}, %eax", low);
                    }
                    IrConst::F32(fval) => {
                        emit!(self.state, "    movl ${}, %eax", fval.to_bits() as i32)
                    }
                    IrConst::F64(fval) => {
                        // Store low 32 bits of the f64 bit pattern
                        let low = fval.to_bits() as i32;
                        emit!(self.state, "    movl ${}, %eax", low);
                    }
                    IrConst::LongDouble(_, bytes) => {
                        // Load first 4 bytes of long double
                        let low = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                        emit!(self.state, "    movl ${}, %eax", low);
                    }
                    IrConst::Zero => {
                        self.state.emit("    xorl %eax, %eax");
                    }
                }
                self.state.reg_cache.invalidate_acc();
            }
            Operand::Value(v) => {
                let is_alloca = self.state.is_alloca(v.0);
                // Check if value is in a callee-saved register (allocas are never register-allocated)
                if let Some(phys) = self.reg_assignments.get(&v.0).copied() {
                    let reg = phys_reg_name(phys);
                    emit!(self.state, "    movl %{}, %eax", reg);
                    self.state.reg_cache.set_acc(v.0, false);
                } else if let Some(slot) = self.state.get_slot(v.0) {
                    let sr = self.slot_ref(slot);
                    if is_alloca {
                        // Alloca: the slot IS the data; load the address of the slot
                        if let Some(align) = self.state.alloca_over_align(v.0) {
                            // Over-aligned alloca: compute aligned address
                            emit!(self.state, "    leal {}, %eax", sr);
                            emit!(self.state, "    addl ${}, %eax", align - 1);
                            emit!(self.state, "    andl ${}, %eax", -(align as i32));
                        } else {
                            emit!(self.state, "    leal {}, %eax", sr);
                        }
                    } else {
                        // Regular value: load the value from the slot
                        emit!(self.state, "    movl {}, %eax", sr);
                    }
                    self.state.reg_cache.set_acc(v.0, is_alloca);
                }
            }
        }
    }

    /// Load a 64-bit value's slot into %eax by OR'ing both 32-bit halves.
    /// Used for truthiness testing of I64/U64/F64 values on i686, where a value
    /// is nonzero iff either half is nonzero.
    pub(super) fn emit_wide_value_to_eax_ored(&mut self, value_id: u32) {
        if let Some(slot) = self.state.get_slot(value_id) {
            let sr0 = self.slot_ref(slot);
            let sr4 = self.slot_ref_offset(slot, 4);
            emit!(self.state, "    movl {}, %eax", sr0);
            emit!(self.state, "    orl {}, %eax", sr4);
        } else {
            // Wide values (I64/F64) on i686 should always have stack slots since
            // they can't fit in a single 32-bit register. Fall back to loading
            // the low 32 bits only as a last resort.
            self.operand_to_eax(&Operand::Value(Value(value_id)));
        }
    }

    /// Load an operand into %ecx.
    pub(super) fn operand_to_ecx(&mut self, op: &Operand) {
        match op {
            Operand::Const(c) => match c {
                IrConst::I8(v) => emit!(self.state, "    movl ${}, %ecx", *v as i32),
                IrConst::I16(v) => emit!(self.state, "    movl ${}, %ecx", *v as i32),
                IrConst::I32(v) => {
                    if *v == 0 {
                        self.state.emit("    xorl %ecx, %ecx");
                    } else {
                        emit!(self.state, "    movl ${}, %ecx", v);
                    }
                }
                IrConst::I64(v) => {
                    let low = *v as i32;
                    if low == 0 {
                        self.state.emit("    xorl %ecx, %ecx");
                    } else {
                        emit!(self.state, "    movl ${}, %ecx", low);
                    }
                }
                IrConst::I128(v) => {
                    let low = *v as i32;
                    emit!(self.state, "    movl ${}, %ecx", low);
                }
                IrConst::F32(fval) => {
                    emit!(self.state, "    movl ${}, %ecx", fval.to_bits() as i32)
                }
                IrConst::F64(fval) => {
                    let low = fval.to_bits() as i32;
                    emit!(self.state, "    movl ${}, %ecx", low);
                }
                IrConst::LongDouble(_, bytes) => {
                    let low = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                    emit!(self.state, "    movl ${}, %ecx", low);
                }
                IrConst::Zero => {
                    self.state.emit("    xorl %ecx, %ecx");
                }
            },
            Operand::Value(v) => {
                let is_alloca = self.state.is_alloca(v.0);
                if let Some(phys) = self.reg_assignments.get(&v.0).copied() {
                    let reg = phys_reg_name(phys);
                    emit!(self.state, "    movl %{}, %ecx", reg);
                } else if let Some(slot) = self.state.get_slot(v.0) {
                    let sr = self.slot_ref(slot);
                    if is_alloca {
                        // Alloca: load the address of the slot
                        if let Some(align) = self.state.alloca_over_align(v.0) {
                            emit!(self.state, "    leal {}, %ecx", sr);
                            emit!(self.state, "    addl ${}, %ecx", align - 1);
                            emit!(self.state, "    andl ${}, %ecx", -(align as i32));
                        } else {
                            emit!(self.state, "    leal {}, %ecx", sr);
                        }
                    } else {
                        emit!(self.state, "    movl {}, %ecx", sr);
                    }
                } else if self.state.reg_cache.acc_has(v.0, false)
                    || self.state.reg_cache.acc_has(v.0, true)
                {
                    // Value is in accumulator (no stack slot) — move eax to ecx.
                    self.state.emit("    movl %eax, %ecx");
                } else {
                    self.state.emit("    xorl %ecx, %ecx");
                }
            }
        }
    }

    /// Store %eax to a value's destination (callee-saved register or stack slot).
    pub(super) fn store_eax_to(&mut self, dest: &Value) {
        if let Some(phys) = self.dest_reg(dest) {
            let reg = phys_reg_name(phys);
            emit!(self.state, "    movl %eax, %{}", reg);
            self.state.reg_cache.invalidate_acc();
        } else if let Some(slot) = self.state.get_slot(dest.0) {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    movl %eax, {}", sr);
            // If this dest is a wide value (I64/U64/F64), zero the upper 4 bytes.
            // Wide values occupy 8-byte slots, and other paths (e.g. Copy from
            // IrConst::I64) may write all 8 bytes. If we only write the low 4,
            // the upper half retains stack garbage, which corrupts truthiness
            // checks that OR both halves (emit_wide_value_to_eax_ored).
            if self.state.wide_values.contains(&dest.0) {
                let sr4 = self.slot_ref_offset(slot, 4);
                emit!(self.state, "    movl $0, {}", sr4);
            }
            self.state.reg_cache.set_acc(dest.0, false);
        }
    }

    /// Return the store mnemonic for a given type.
    pub(super) fn mov_store_for_type(&self, ty: IrType) -> &'static str {
        match ty {
            IrType::I8 | IrType::U8 => "movb",
            IrType::I16 | IrType::U16 => "movw",
            // On i686, pointer-sized types use movl (32-bit)
            _ => "movl",
        }
    }

    /// Return the load mnemonic for a given type.
    pub(super) fn mov_load_for_type(&self, ty: IrType) -> &'static str {
        match ty {
            IrType::I8 => "movsbl",  // sign-extend byte to 32-bit
            IrType::U8 => "movzbl",  // zero-extend byte to 32-bit
            IrType::I16 => "movswl", // sign-extend word to 32-bit
            IrType::U16 => "movzwl", // zero-extend word to 32-bit
            // Everything 32-bit or larger uses movl
            _ => "movl",
        }
    }

    /// Return the type suffix for an operation.
    pub(super) fn type_suffix(&self, ty: IrType) -> &'static str {
        match ty {
            IrType::I8 | IrType::U8 => "b",
            IrType::I16 | IrType::U16 => "w",
            // On i686, the default (pointer-sized) is "l" (32-bit)
            _ => "l",
        }
    }

    /// Return the register name for eax sub-register based on type size.
    pub(super) fn eax_for_type(&self, ty: IrType) -> &'static str {
        match ty {
            IrType::I8 | IrType::U8 => "%al",
            IrType::I16 | IrType::U16 => "%ax",
            _ => "%eax",
        }
    }

    /// Check if a param type is eligible for fastcall register passing.
    /// Only DWORD-sized or smaller integer/pointer types qualify.
    pub(super) fn is_fastcall_reg_eligible(&self, ty: IrType) -> bool {
        matches!(
            ty,
            IrType::I8
                | IrType::U8
                | IrType::I16
                | IrType::U16
                | IrType::I32
                | IrType::U32
                | IrType::Ptr
        )
    }

    /// Count how many leading params are passed in registers for fastcall (max 2).
    pub(super) fn count_fastcall_reg_params(&self, func: &IrFunction) -> usize {
        let mut count = 0;
        for param in &func.params {
            if count >= 2 {
                break;
            }
            let ty = param.ty;
            if self.is_fastcall_reg_eligible(ty) {
                count += 1;
            } else {
                break; // non-eligible param stops register assignment
            }
        }
        count
    }

    /// Check if an operand is a constant that fits in an i32 immediate.
    pub(super) fn const_as_imm32(op: &Operand) -> Option<i64> {
        match op {
            Operand::Const(c) => {
                match c {
                    IrConst::I8(v) => Some(*v as i64),
                    IrConst::I16(v) => Some(*v as i64),
                    IrConst::I32(v) => Some(*v as i64),
                    IrConst::I64(v) => {
                        // On i686, check if the value fits in 32 bits
                        if *v >= i32::MIN as i64 && *v <= i32::MAX as i64 {
                            Some(*v)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Extract an immediate integer value from an operand.
    /// Used for SSE/AES instructions that require compile-time immediate operands.
    pub(super) fn operand_to_imm_i64(op: &Operand) -> i64 {
        match op {
            Operand::Const(c) => match c {
                IrConst::I8(v) => *v as i64,
                IrConst::I16(v) => *v as i64,
                IrConst::I32(v) => *v as i64,
                IrConst::I64(v) => *v,
                _ => 0,
            },
            Operand::Value(_) => 0,
        }
    }

    /// Load an F128 (long double) operand onto the x87 FPU stack.
    pub(super) fn emit_f128_load_to_x87(&mut self, op: &Operand) {
        match op {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    let sr = self.slot_ref(slot);
                    emit!(self.state, "    fldt {}", sr);
                }
            }
            Operand::Const(IrConst::LongDouble(_, bytes)) => {
                // Convert f128 (IEEE binary128) bytes to x87 80-bit format for fldt
                let x87 = crate::common::long_double::f128_bytes_to_x87_bytes(bytes);
                let dword0 = i32::from_le_bytes([x87[0], x87[1], x87[2], x87[3]]);
                let dword1 = i32::from_le_bytes([x87[4], x87[5], x87[6], x87[7]]);
                let word2 = i16::from_le_bytes([x87[8], x87[9]]) as i32;
                self.state.emit("    subl $12, %esp");
                emit!(self.state, "    movl ${}, (%esp)", dword0);
                emit!(self.state, "    movl ${}, 4(%esp)", dword1);
                emit!(self.state, "    movw ${}, 8(%esp)", word2);
                self.state.emit("    fldt (%esp)");
                self.state.emit("    addl $12, %esp");
            }
            Operand::Const(IrConst::F64(fval)) => {
                // Convert f64 to x87: push to stack as f64, fld, convert
                let bits = fval.to_bits();
                let low = (bits & 0xFFFFFFFF) as i32;
                let high = ((bits >> 32) & 0xFFFFFFFF) as i32;
                self.state.emit("    subl $8, %esp");
                emit!(self.state, "    movl ${}, (%esp)", low);
                emit!(self.state, "    movl ${}, 4(%esp)", high);
                self.state.emit("    fldl (%esp)");
                self.state.emit("    addl $8, %esp");
            }
            Operand::Const(IrConst::F32(fval)) => {
                emit!(self.state, "    movl ${}, %eax", fval.to_bits() as i32);
                self.state.emit("    pushl %eax");
                self.state.emit("    flds (%esp)");
                self.state.emit("    addl $4, %esp");
            }
            _ => {
                self.operand_to_eax(op);
                // Fallback: treat as integer, push to stack
                self.state.emit("    pushl %eax");
                self.state.emit("    flds (%esp)");
                self.state.emit("    addl $4, %esp");
            }
        }
    }

    /// Load an F64 (double) operand onto the x87 FPU stack.
    /// F64 values occupy 8-byte stack slots on i686, so we use fldl to load
    /// them directly from memory rather than going through the 32-bit accumulator.
    pub(super) fn emit_f64_load_to_x87(&mut self, op: &Operand) {
        match op {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    let sr = self.slot_ref(slot);
                    emit!(self.state, "    fldl {}", sr);
                }
            }
            Operand::Const(IrConst::F64(fval)) => {
                let bits = fval.to_bits();
                let low = (bits & 0xFFFFFFFF) as i32;
                let high = ((bits >> 32) & 0xFFFFFFFF) as i32;
                self.state.emit("    subl $8, %esp");
                emit!(self.state, "    movl ${}, (%esp)", low);
                emit!(self.state, "    movl ${}, 4(%esp)", high);
                self.state.emit("    fldl (%esp)");
                self.state.emit("    addl $8, %esp");
            }
            Operand::Const(IrConst::F32(fval)) => {
                emit!(self.state, "    movl ${}, %eax", fval.to_bits() as i32);
                self.state.emit("    pushl %eax");
                self.state.emit("    flds (%esp)");
                self.state.emit("    addl $4, %esp");
            }
            Operand::Const(IrConst::Zero) => {
                self.state.emit("    fldz");
            }
            _ => {
                // Fallback: load integer bits and convert
                self.operand_to_eax(op);
                self.state.emit("    pushl %eax");
                self.state.emit("    fildl (%esp)");
                self.state.emit("    addl $4, %esp");
            }
        }
    }

    /// Store the x87 st(0) value as F64 into a destination stack slot.
    /// Pops st(0).
    pub(super) fn emit_f64_store_from_x87(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    fstpl {}", sr);
        } else {
            // No slot available, pop x87 stack to discard
            self.state.emit("    fstp %st(0)");
        }
    }

    // --- 64-bit atomic helpers using cmpxchg8b (for I64/U64/F64 on i686) ---

    /// Check if a type requires 64-bit atomic handling on i686 (needs cmpxchg8b).
    pub(super) fn is_atomic_wide(&self, ty: IrType) -> bool {
        matches!(ty, IrType::I64 | IrType::U64 | IrType::F64)
    }

    /// 64-bit atomic RMW using lock cmpxchg8b loop.
    ///
    /// cmpxchg8b compares edx:eax with 8 bytes at memory location.
    /// If equal, stores ecx:ebx to memory. If not, loads memory into edx:eax.
    /// We use a loop: load old value, compute new value, try cmpxchg8b.
    ///
    /// Register plan:
    ///   esi = pointer to atomic variable (saved/restored)
    ///   edx:eax = old (expected) value
    ///   ecx:ebx = new (desired) value
    ///   Stack: saved operand value (8 bytes)
    pub(super) fn emit_atomic_rmw_wide(
        &mut self,
        dest: &Value,
        op: AtomicRmwOp,
        ptr: &Operand,
        val: &Operand,
    ) {
        // Save callee-saved registers we need to clobber
        self.state.emit("    pushl %ebx");
        self.esp_adjust += 4;
        self.state.emit("    pushl %esi");
        self.esp_adjust += 4;

        // Load pointer into esi
        self.operand_to_eax(ptr);
        self.state.emit("    movl %eax, %esi");

        // Load 64-bit operand value onto stack (8 bytes)
        self.emit_load_acc_pair(val);
        self.state.emit("    pushl %edx"); // high word at 4(%esp)
        self.state.emit("    pushl %eax"); // low word at (%esp)

        // Load current value from memory into edx:eax
        self.state.emit("    movl (%esi), %eax");
        self.state.emit("    movl 4(%esi), %edx");

        match op {
            AtomicRmwOp::Xchg => {
                // For exchange, the desired value is the operand (constant across retries)
                self.state.emit("    movl (%esp), %ebx");
                self.state.emit("    movl 4(%esp), %ecx");
                let loop_label = format!(".Latomic_{}", self.state.next_label_id());
                emit!(self.state, "{}:", loop_label);
                self.state.emit("    lock cmpxchg8b (%esi)");
                emit!(self.state, "    jne {}", loop_label);
            }
            AtomicRmwOp::Add => {
                let loop_label = format!(".Latomic_{}", self.state.next_label_id());
                emit!(self.state, "{}:", loop_label);
                self.state.emit("    movl %eax, %ebx");
                self.state.emit("    movl %edx, %ecx");
                self.state.emit("    addl (%esp), %ebx");
                self.state.emit("    adcl 4(%esp), %ecx");
                self.state.emit("    lock cmpxchg8b (%esi)");
                emit!(self.state, "    jne {}", loop_label);
            }
            AtomicRmwOp::Sub => {
                let loop_label = format!(".Latomic_{}", self.state.next_label_id());
                emit!(self.state, "{}:", loop_label);
                self.state.emit("    movl %eax, %ebx");
                self.state.emit("    movl %edx, %ecx");
                self.state.emit("    subl (%esp), %ebx");
                self.state.emit("    sbbl 4(%esp), %ecx");
                self.state.emit("    lock cmpxchg8b (%esi)");
                emit!(self.state, "    jne {}", loop_label);
            }
            AtomicRmwOp::And => {
                let loop_label = format!(".Latomic_{}", self.state.next_label_id());
                emit!(self.state, "{}:", loop_label);
                self.state.emit("    movl %eax, %ebx");
                self.state.emit("    movl %edx, %ecx");
                self.state.emit("    andl (%esp), %ebx");
                self.state.emit("    andl 4(%esp), %ecx");
                self.state.emit("    lock cmpxchg8b (%esi)");
                emit!(self.state, "    jne {}", loop_label);
            }
            AtomicRmwOp::Or => {
                let loop_label = format!(".Latomic_{}", self.state.next_label_id());
                emit!(self.state, "{}:", loop_label);
                self.state.emit("    movl %eax, %ebx");
                self.state.emit("    movl %edx, %ecx");
                self.state.emit("    orl (%esp), %ebx");
                self.state.emit("    orl 4(%esp), %ecx");
                self.state.emit("    lock cmpxchg8b (%esi)");
                emit!(self.state, "    jne {}", loop_label);
            }
            AtomicRmwOp::Xor => {
                let loop_label = format!(".Latomic_{}", self.state.next_label_id());
                emit!(self.state, "{}:", loop_label);
                self.state.emit("    movl %eax, %ebx");
                self.state.emit("    movl %edx, %ecx");
                self.state.emit("    xorl (%esp), %ebx");
                self.state.emit("    xorl 4(%esp), %ecx");
                self.state.emit("    lock cmpxchg8b (%esi)");
                emit!(self.state, "    jne {}", loop_label);
            }
            AtomicRmwOp::Nand => {
                let loop_label = format!(".Latomic_{}", self.state.next_label_id());
                emit!(self.state, "{}:", loop_label);
                self.state.emit("    movl %eax, %ebx");
                self.state.emit("    movl %edx, %ecx");
                self.state.emit("    andl (%esp), %ebx");
                self.state.emit("    andl 4(%esp), %ecx");
                self.state.emit("    notl %ebx");
                self.state.emit("    notl %ecx");
                self.state.emit("    lock cmpxchg8b (%esi)");
                emit!(self.state, "    jne {}", loop_label);
            }
            AtomicRmwOp::TestAndSet => {
                // For 64-bit test-and-set, set the low byte to 1, rest to 0
                self.state.emit("    movl $1, %ebx");
                self.state.emit("    xorl %ecx, %ecx");
                let loop_label = format!(".Latomic_{}", self.state.next_label_id());
                emit!(self.state, "{}:", loop_label);
                self.state.emit("    lock cmpxchg8b (%esi)");
                emit!(self.state, "    jne {}", loop_label);
            }
        }

        // Clean up stack (remove 8-byte operand value)
        self.state.emit("    addl $8, %esp");
        // Restore callee-saved registers
        self.state.emit("    popl %esi");
        self.esp_adjust -= 4;
        self.state.emit("    popl %ebx");
        self.esp_adjust -= 4;

        // Result (old value) is in edx:eax — store to dest's 64-bit stack slot
        self.state.reg_cache.invalidate_acc();
        self.emit_store_acc_pair(dest);
    }

    /// 64-bit atomic compare-exchange using lock cmpxchg8b.
    ///
    /// cmpxchg8b: compares edx:eax with 8 bytes at memory.
    /// If equal, stores ecx:ebx to memory and sets ZF.
    /// If not equal, loads memory into edx:eax and clears ZF.
    pub(super) fn emit_atomic_cmpxchg_wide(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        expected: &Operand,
        desired: &Operand,
        returns_bool: bool,
    ) {
        // Save callee-saved registers
        self.state.emit("    pushl %ebx");
        self.esp_adjust += 4;
        self.state.emit("    pushl %esi");
        self.esp_adjust += 4;

        // Load pointer into esi
        self.operand_to_eax(ptr);
        self.state.emit("    movl %eax, %esi");

        // Load expected into edx:eax, save on stack temporarily
        self.emit_load_acc_pair(expected);
        self.state.emit("    pushl %edx");
        self.esp_adjust += 4;
        self.state.emit("    pushl %eax");
        self.esp_adjust += 4;

        // Load desired into ecx:ebx
        self.emit_load_acc_pair(desired);
        self.state.emit("    movl %eax, %ebx");
        self.state.emit("    movl %edx, %ecx");

        // Restore expected into edx:eax
        self.state.emit("    popl %eax");
        self.esp_adjust -= 4;
        self.state.emit("    popl %edx");
        self.esp_adjust -= 4;

        // Execute cmpxchg8b
        self.state.emit("    lock cmpxchg8b (%esi)");

        if returns_bool {
            self.state.emit("    sete %al");
            self.state.emit("    movzbl %al, %eax");
            // Restore callee-saved registers
            self.state.emit("    popl %esi");
            self.esp_adjust -= 4;
            self.state.emit("    popl %ebx");
            self.esp_adjust -= 4;
            self.state.reg_cache.invalidate_acc();
            self.store_eax_to(dest);
        } else {
            // Result (old value) is in edx:eax
            // Restore callee-saved registers
            self.state.emit("    popl %esi");
            self.esp_adjust -= 4;
            self.state.emit("    popl %ebx");
            self.esp_adjust -= 4;
            self.state.reg_cache.invalidate_acc();
            self.emit_store_acc_pair(dest);
        }
    }

    /// 64-bit atomic load using cmpxchg8b with expected == desired == 0.
    ///
    /// cmpxchg8b always loads the current memory value into edx:eax on failure,
    /// so we set edx:eax = ecx:ebx = 0 and execute cmpxchg8b. If the memory
    /// happens to be 0, the exchange writes 0 (no change). If non-zero,
    /// we get the current value in edx:eax without modifying memory.
    pub(super) fn emit_atomic_load_wide(&mut self, dest: &Value, ptr: &Operand) {
        self.state.emit("    pushl %ebx");
        self.esp_adjust += 4;
        self.state.emit("    pushl %esi");
        self.esp_adjust += 4;

        self.operand_to_eax(ptr);
        self.state.emit("    movl %eax, %esi");

        // Set all registers to zero: edx:eax = ecx:ebx = 0
        self.state.emit("    xorl %eax, %eax");
        self.state.emit("    xorl %edx, %edx");
        self.state.emit("    xorl %ebx, %ebx");
        self.state.emit("    xorl %ecx, %ecx");
        // lock cmpxchg8b: if (%esi) == 0 -> store 0 (no change), else load into edx:eax
        self.state.emit("    lock cmpxchg8b (%esi)");

        self.state.emit("    popl %esi");
        self.esp_adjust -= 4;
        self.state.emit("    popl %ebx");
        self.esp_adjust -= 4;

        self.state.reg_cache.invalidate_acc();
        self.emit_store_acc_pair(dest);
    }

    /// 64-bit atomic store using a cmpxchg8b loop.
    ///
    /// There is no single instruction for atomic 64-bit stores on i686, so we
    /// use a cmpxchg8b loop: read current value, try to replace with desired.
    pub(super) fn emit_atomic_store_wide(&mut self, ptr: &Operand, val: &Operand) {
        self.state.emit("    pushl %ebx");
        self.esp_adjust += 4;
        self.state.emit("    pushl %esi");
        self.esp_adjust += 4;

        // Load pointer into esi
        self.operand_to_eax(ptr);
        self.state.emit("    movl %eax, %esi");

        // Load desired value into ecx:ebx
        self.emit_load_acc_pair(val);
        self.state.emit("    movl %eax, %ebx");
        self.state.emit("    movl %edx, %ecx");

        // Load current value from memory into edx:eax (initial guess for cmpxchg8b)
        self.state.emit("    movl (%esi), %eax");
        self.state.emit("    movl 4(%esi), %edx");

        let loop_label = format!(".Latomic_{}", self.state.next_label_id());
        emit!(self.state, "{}:", loop_label);
        self.state.emit("    lock cmpxchg8b (%esi)");
        emit!(self.state, "    jne {}", loop_label);

        self.state.emit("    popl %esi");
        self.esp_adjust -= 4;
        self.state.emit("    popl %ebx");
        self.esp_adjust -= 4;
        self.state.reg_cache.invalidate_acc();
    }

    /// Emit a fastcall function call on i686.
    /// First two DWORD (int/ptr) args go in ECX, EDX.
    /// Remaining args go on the stack (right-to-left push order).
    /// The callee pops stack args, so caller does NOT adjust ESP after call.
    pub(super) fn emit_fastcall(
        &mut self,
        args: &[Operand],
        arg_types: &[IrType],
        direct_name: Option<&str>,
        func_ptr: Option<&Operand>,
        dest: Option<Value>,
        return_type: IrType,
    ) {
        let indirect = func_ptr.is_some() && direct_name.is_none();

        // Determine which args go in registers vs stack.
        let mut reg_count = 0usize;
        for ty in arg_types.iter() {
            if reg_count >= 2 {
                break;
            }
            if self.is_fastcall_reg_eligible(*ty) {
                reg_count += 1;
            } else {
                break;
            }
        }

        // Compute stack space for overflow args (args beyond the register ones).
        let mut stack_bytes = 0usize;
        for i in reg_count..args.len() {
            let ty = if i < arg_types.len() {
                arg_types[i]
            } else {
                IrType::I32
            };
            match ty {
                IrType::F64 | IrType::I64 | IrType::U64 => stack_bytes += 8,
                IrType::F128 => stack_bytes += 12,
                _ if is_i128_type(ty) => stack_bytes += 16,
                _ => stack_bytes += 4,
            }
        }
        // Align to 16 bytes
        let stack_arg_space = (stack_bytes + 15) & !15;

        // Spill indirect function pointer before stack manipulation.
        if indirect {
            self.emit_call_spill_fptr(func_ptr.expect("indirect call requires func_ptr"));
        }

        // Phase 1: Allocate stack space and write stack args.
        if stack_arg_space > 0 {
            emit!(self.state, "    subl ${}, %esp", stack_arg_space);
            self.esp_adjust += stack_arg_space as i64;
        }

        // Write stack args (skipping register args).
        let mut offset = 0i64;
        for i in reg_count..args.len() {
            let ty = if i < arg_types.len() {
                arg_types[i]
            } else {
                IrType::I32
            };
            let arg = &args[i];

            match ty {
                IrType::I64 | IrType::U64 | IrType::F64 => {
                    self.emit_load_acc_pair(arg);
                    emit!(self.state, "    movl %eax, {}(%esp)", offset);
                    emit!(self.state, "    movl %edx, {}(%esp)", offset + 4);
                    offset += 8;
                }
                IrType::F128 => {
                    // Load F128 value to x87 and store to stack
                    self.emit_f128_load_to_x87(arg);
                    emit!(self.state, "    fstpt {}(%esp)", offset);
                    offset += 12;
                }
                _ if is_i128_type(ty) => {
                    // Copy 16 bytes
                    if let Operand::Value(v) = arg {
                        if let Some(slot) = self.state.get_slot(v.0) {
                            for j in (0..16).step_by(4) {
                                let sr = self.slot_ref_offset(slot, j as i64);
                                emit!(self.state, "    movl {}, %eax", sr);
                                emit!(self.state, "    movl %eax, {}(%esp)", offset + j as i64);
                            }
                        }
                    }
                    offset += 16;
                }
                _ => {
                    self.emit_load_operand(arg);
                    emit!(self.state, "    movl %eax, {}(%esp)", offset);
                    offset += 4;
                }
            }
        }

        // Phase 2: Load register args into ECX and EDX.
        // Load EDX first (arg 1) then ECX (arg 0), because loading arg 0
        // may clobber EDX if it involves function calls.
        if reg_count >= 2 {
            self.emit_load_operand(&args[1]);
            self.state.emit("    movl %eax, %edx");
        }
        if reg_count >= 1 {
            self.emit_load_operand(&args[0]);
            self.state.emit("    movl %eax, %ecx");
        }

        // Phase 3: Emit the call.
        if indirect {
            // Reload function pointer from spill slot
            let fptr_offset = stack_arg_space as i64;
            emit!(self.state, "    movl {}(%esp), %eax", fptr_offset);
            self.state.emit("    call *%eax");
        } else if let Some(name) = direct_name {
            emit!(self.state, "    call {}", name);
        }

        // Phase 4: For indirect calls, pop the spilled function pointer.
        // Note: callee already cleaned up the stack args, so we only need
        // to handle the fptr spill and alignment padding.
        // After call: callee popped stack_bytes, so esp_adjust drops by that amount.
        self.esp_adjust -= stack_bytes as i64;
        if indirect {
            self.state.emit("    addl $4, %esp"); // pop fptr spill
            self.esp_adjust -= 4;
        }
        // Clean up alignment padding (the difference between actual stack bytes and aligned)
        let padding = stack_arg_space - stack_bytes;
        if padding > 0 {
            emit!(self.state, "    addl ${}, %esp", padding);
            self.esp_adjust -= padding as i64;
        }

        // Phase 5: Store return value.
        if let Some(dest) = dest {
            self.emit_call_store_result(&dest, return_type);
        }

        self.state.reg_cache.invalidate_acc();
    }
}

// Helper functions for ALU mnemonics
pub(super) fn alu_mnemonic(op: IrBinOp) -> &'static str {
    match op {
        IrBinOp::Add => "add",
        IrBinOp::Sub => "sub",
        IrBinOp::And => "and",
        IrBinOp::Or => "or",
        IrBinOp::Xor => "xor",
        _ => panic!("not a simple ALU op: {:?}", op),
    }
}

pub(super) fn shift_mnemonic(op: IrBinOp) -> &'static str {
    match op {
        IrBinOp::Shl => "shll",
        IrBinOp::AShr => "sarl",
        IrBinOp::LShr => "shrl",
        _ => panic!("not a shift op: {:?}", op),
    }
}

// --- 64-bit bit-manipulation helpers ---
// On i686, 64-bit values are in the eax:edx register pair (eax=low, edx=high).
// The result of clz/ctz/popcount is a small integer (0-64) that fits in eax,
// so we zero edx to produce a proper I64 result.
impl I686Codegen {
    /// clzll(x): Count leading zeros of 64-bit value in eax:edx.
    /// If high half (edx) != 0, result = lzcnt(edx).
    /// Otherwise, result = 32 + lzcnt(eax).
    pub(super) fn emit_i64_clz(&mut self) {
        let done = self.state.fresh_label("clz64_done");
        let hi_zero = self.state.fresh_label("clz64_hi_zero");
        // Test high half
        self.state.emit("    testl %edx, %edx");
        emit!(self.state, "    je {}", hi_zero);
        // High half is non-zero: result = lzcnt(edx)
        self.state.emit("    lzcntl %edx, %eax");
        self.state.emit("    xorl %edx, %edx");
        emit!(self.state, "    jmp {}", done);
        // High half is zero: result = 32 + lzcnt(eax)
        emit!(self.state, "{}:", hi_zero);
        self.state.emit("    lzcntl %eax, %eax");
        self.state.emit("    addl $32, %eax");
        self.state.emit("    xorl %edx, %edx");
        emit!(self.state, "{}:", done);
    }

    /// ctzll(x): Count trailing zeros of 64-bit value in eax:edx.
    /// If low half (eax) != 0, result = tzcnt(eax).
    /// Otherwise, result = 32 + tzcnt(edx).
    pub(super) fn emit_i64_ctz(&mut self) {
        let done = self.state.fresh_label("ctz64_done");
        let lo_zero = self.state.fresh_label("ctz64_lo_zero");
        // Test low half
        self.state.emit("    testl %eax, %eax");
        emit!(self.state, "    je {}", lo_zero);
        // Low half is non-zero: result = tzcnt(eax)
        self.state.emit("    tzcntl %eax, %eax");
        self.state.emit("    xorl %edx, %edx");
        emit!(self.state, "    jmp {}", done);
        // Low half is zero: result = 32 + tzcnt(edx)
        emit!(self.state, "{}:", lo_zero);
        self.state.emit("    tzcntl %edx, %eax");
        self.state.emit("    addl $32, %eax");
        self.state.emit("    xorl %edx, %edx");
        emit!(self.state, "{}:", done);
    }

    /// popcountll(x): Population count of 64-bit value in eax:edx.
    /// result = popcount(eax) + popcount(edx)
    pub(super) fn emit_i64_popcount(&mut self) {
        self.state.emit("    popcntl %edx, %ecx");
        self.state.emit("    popcntl %eax, %eax");
        self.state.emit("    addl %ecx, %eax");
        self.state.emit("    xorl %edx, %edx");
    }

    /// bswap64(x): Byte-swap 64-bit value in eax:edx.
    /// result_lo = bswap(high), result_hi = bswap(low)
    pub(super) fn emit_i64_bswap(&mut self) {
        // eax=low, edx=high
        // bswap each half, then swap: new_eax = bswap(edx), new_edx = bswap(eax)
        self.state.emit("    bswapl %eax");
        self.state.emit("    bswapl %edx");
        self.state.emit("    xchgl %eax, %edx");
    }

    /// Copy `n_bytes` from stack slot to call stack area, 4 bytes at a time.
    pub(super) fn emit_copy_slot_to_stack(
        &mut self,
        slot: StackSlot,
        stack_offset: usize,
        n_bytes: usize,
    ) {
        let mut copied = 0usize;
        while copied + 4 <= n_bytes {
            let sr = self.slot_ref_offset(slot, copied as i64);
            emit!(self.state, "    movl {}, %eax", sr);
            emit!(self.state, "    movl %eax, {}(%esp)", stack_offset + copied);
            copied += 4;
        }
        while copied < n_bytes {
            let sr = self.slot_ref_offset(slot, copied as i64);
            emit!(self.state, "    movb {}, %al", sr);
            emit!(self.state, "    movb %al, {}(%esp)", stack_offset + copied);
            copied += 1;
        }
        self.state.reg_cache.invalidate_acc();
    }

    /// Fallback: store eax to stack, zero-fill remaining bytes.
    pub(super) fn emit_eax_to_stack_zeroed(
        &mut self,
        arg: &Operand,
        stack_offset: usize,
        total_bytes: usize,
    ) {
        self.operand_to_eax(arg);
        emit!(self.state, "    movl %eax, {}(%esp)", stack_offset);
        for j in (4..total_bytes).step_by(4) {
            emit!(self.state, "    movl $0, {}(%esp)", stack_offset + j);
        }
    }

    /// Emit I128 argument to call stack (16 bytes).
    pub(super) fn emit_call_i128_stack_arg(&mut self, arg: &Operand, stack_offset: usize) {
        if let Operand::Value(v) = arg {
            if let Some(slot) = self.state.get_slot(v.0) {
                self.emit_copy_slot_to_stack(slot, stack_offset, 16);
            } else {
                self.emit_eax_to_stack_zeroed(arg, stack_offset, 16);
            }
        }
    }

    /// Emit F128 (long double) argument to call stack (12 bytes).
    pub(super) fn emit_call_f128_stack_arg(&mut self, arg: &Operand, stack_offset: usize) {
        match arg {
            Operand::Value(v) => {
                if self.state.f128_direct_slots.contains(&v.0) {
                    if let Some(slot) = self.state.get_slot(v.0) {
                        let sr = self.slot_ref(slot);
                        emit!(self.state, "    fldt {}", sr);
                        emit!(self.state, "    fstpt {}(%esp)", stack_offset);
                    }
                } else if let Some(slot) = self.state.get_slot(v.0) {
                    self.emit_copy_slot_to_stack(slot, stack_offset, 12);
                } else {
                    self.emit_eax_to_stack_zeroed(arg, stack_offset, 12);
                }
            }
            Operand::Const(IrConst::LongDouble(_, bytes)) => {
                let x87 = crate::common::long_double::f128_bytes_to_x87_bytes(bytes);
                let dword0 = i32::from_le_bytes([x87[0], x87[1], x87[2], x87[3]]);
                let dword1 = i32::from_le_bytes([x87[4], x87[5], x87[6], x87[7]]);
                let word2 = i16::from_le_bytes([x87[8], x87[9]]) as i32;
                emit!(self.state, "    movl ${}, {}(%esp)", dword0, stack_offset);
                emit!(
                    self.state,
                    "    movl ${}, {}(%esp)",
                    dword1,
                    stack_offset + 4
                );
                emit!(
                    self.state,
                    "    movw ${}, {}(%esp)",
                    word2,
                    stack_offset + 8
                );
            }
            Operand::Const(IrConst::F64(fval)) => {
                let bits = fval.to_bits();
                let low = (bits & 0xFFFFFFFF) as i32;
                let high = ((bits >> 32) & 0xFFFFFFFF) as i32;
                self.state.emit("    subl $8, %esp");
                emit!(self.state, "    movl ${}, (%esp)", low);
                emit!(self.state, "    movl ${}, 4(%esp)", high);
                self.state.emit("    fldl (%esp)");
                self.state.emit("    addl $8, %esp");
                emit!(self.state, "    fstpt {}(%esp)", stack_offset);
            }
            _ => {
                self.emit_eax_to_stack_zeroed(arg, stack_offset, 12);
            }
        }
    }

    /// Emit struct-by-value argument to call stack.
    pub(super) fn emit_call_struct_stack_arg(
        &mut self,
        arg: &Operand,
        stack_offset: usize,
        size: usize,
    ) {
        if let Operand::Value(v) = arg {
            if self.state.is_alloca(v.0) {
                if let Some(slot) = self.state.get_slot(v.0) {
                    self.emit_copy_slot_to_stack(slot, stack_offset, size);
                }
            } else {
                // Non-alloca: value is a pointer to struct data.
                self.operand_to_eax(arg);
                self.state.emit("    movl %eax, %ecx");
                let mut copied = 0usize;
                while copied + 4 <= size {
                    emit!(self.state, "    movl {}(%ecx), %eax", copied);
                    emit!(self.state, "    movl %eax, {}(%esp)", stack_offset + copied);
                    copied += 4;
                }
                while copied < size {
                    emit!(self.state, "    movb {}(%ecx), %al", copied);
                    emit!(self.state, "    movb %al, {}(%esp)", stack_offset + copied);
                    copied += 1;
                }
                self.state.reg_cache.invalidate_acc();
            }
        }
    }

    /// Emit 8-byte scalar (F64/I64/U64) to call stack.
    pub(super) fn emit_call_8byte_stack_arg(
        &mut self,
        arg: &Operand,
        ty: IrType,
        stack_offset: usize,
    ) {
        if let Operand::Value(v) = arg {
            if let Some(slot) = self.state.get_slot(v.0) {
                let sr0 = self.slot_ref(slot);
                let sr4 = self.slot_ref_offset(slot, 4);
                emit!(self.state, "    movl {}, %eax", sr0);
                emit!(self.state, "    movl %eax, {}(%esp)", stack_offset);
                emit!(self.state, "    movl {}, %eax", sr4);
                emit!(self.state, "    movl %eax, {}(%esp)", stack_offset + 4);
                self.state.reg_cache.invalidate_acc();
            } else {
                self.operand_to_eax(arg);
                emit!(self.state, "    movl %eax, {}(%esp)", stack_offset);
                emit!(self.state, "    movl $0, {}(%esp)", stack_offset + 4);
            }
        } else if ty == IrType::F64 {
            if let Operand::Const(IrConst::F64(f)) = arg {
                let bits = f.to_bits();
                let lo = (bits & 0xFFFF_FFFF) as u32;
                let hi = (bits >> 32) as u32;
                emit!(
                    self.state,
                    "    movl ${}, {}(%esp)",
                    lo as i32,
                    stack_offset
                );
                emit!(
                    self.state,
                    "    movl ${}, {}(%esp)",
                    hi as i32,
                    stack_offset + 4
                );
            } else {
                self.operand_to_eax(arg);
                emit!(self.state, "    movl %eax, {}(%esp)", stack_offset);
                emit!(self.state, "    movl $0, {}(%esp)", stack_offset + 4);
            }
        } else {
            // I64/U64 constant
            self.operand_to_eax(arg);
            emit!(self.state, "    movl %eax, {}(%esp)", stack_offset);
            if let Operand::Const(IrConst::I64(v)) = arg {
                let hi = ((*v as u64) >> 32) as i32;
                emit!(self.state, "    movl ${}, {}(%esp)", hi, stack_offset + 4);
            } else {
                emit!(self.state, "    movl $0, {}(%esp)", stack_offset + 4);
            }
        }
    }
}

// ─── ArchCodegen trait implementation ────────────────────────────────────────

impl ArchCodegen for I686Codegen {
    fn state(&mut self) -> &mut CodegenState {
        &mut self.state
    }
    fn state_ref(&self) -> &CodegenState {
        &self.state
    }

    fn ptr_directive(&self) -> PtrDirective {
        PtrDirective::Long
    }

    fn get_phys_reg_for_value(&self, val_id: u32) -> Option<PhysReg> {
        self.reg_assignments.get(&val_id).copied()
    }

    fn emit_reg_to_reg_move(&mut self, src: PhysReg, dest: PhysReg) {
        let src_name = phys_reg_name(src);
        let dest_name = phys_reg_name(dest);
        emit!(self.state, "    movl %{}, %{}", src_name, dest_name);
    }

    fn emit_acc_to_phys_reg(&mut self, dest: PhysReg) {
        let dest_name = phys_reg_name(dest);
        emit!(self.state, "    movl %eax, %{}", dest_name);
    }

    // ---- Standard trait methods (kept inline - arch-specific) ----
    fn emit_load_operand(&mut self, op: &Operand) {
        self.operand_to_eax(op);
    }
    fn emit_store_result(&mut self, dest: &Value) {
        self.store_eax_to(dest);
    }
    fn emit_save_acc(&mut self) {
        self.state.emit("    movl %eax, %edx");
    }
    fn emit_add_secondary_to_acc(&mut self) {
        self.state.emit("    addl %ecx, %eax");
    }
    fn emit_acc_to_secondary(&mut self) {
        self.state.emit("    movl %eax, %ecx");
    }
    fn emit_memcpy_store_dest_from_acc(&mut self) {
        self.state.emit("    movl %eax, %edi");
    }
    fn emit_memcpy_store_src_from_acc(&mut self) {
        self.state.emit("    movl %eax, %esi");
    }
    fn current_return_type(&self) -> IrType {
        self.current_return_type
    }
    fn emit_gep_add_const_to_acc(&mut self, offset: i64) {
        if offset != 0 {
            emit!(self.state, "    addl ${}, %eax", offset as i32);
        }
    }

    /// Override emit_memcpy for i686: uses rep movsb with esi/edi.
    fn emit_memcpy(&mut self, dest: &Value, src: &Value, size: usize) {
        use crate::backend::state::SlotAddr;
        // Always save esi and edi around rep movsb.
        // These are callee-saved registers in the System V i386 ABI, so we must
        // preserve them even if the register allocator didn't assign any values
        // to them in this function. A caller may be relying on their preservation
        // across a call to this function.
        self.state.emit("    pushl %esi");
        self.esp_adjust += 4;
        self.state.emit("    pushl %edi");
        self.esp_adjust += 4;

        // Load dest address into edi
        if let Some(addr) = self.state.resolve_slot_addr(dest.0) {
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    self.emit_alloca_aligned_addr_to_acc(slot, id);
                    self.state.emit("    movl %eax, %edi");
                }
                SlotAddr::Direct(slot) => self.emit_memcpy_load_dest_addr(slot, true, dest.0),
                SlotAddr::Indirect(slot) => self.emit_memcpy_load_dest_addr(slot, false, dest.0),
            }
        }
        // Load src address into esi
        if let Some(addr) = self.state.resolve_slot_addr(src.0) {
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    self.emit_alloca_aligned_addr_to_acc(slot, id);
                    self.state.emit("    movl %eax, %esi");
                }
                SlotAddr::Direct(slot) => self.emit_memcpy_load_src_addr(slot, true, src.0),
                SlotAddr::Indirect(slot) => self.emit_memcpy_load_src_addr(slot, false, src.0),
            }
        }
        // Perform the copy
        self.emit_memcpy_impl(size);

        // Restore edi and esi (reverse order of push)
        self.state.emit("    popl %edi");
        self.esp_adjust -= 4;
        self.state.emit("    popl %esi");
        self.esp_adjust -= 4;
    }

    /// Override emit_binop to route I64/U64 through register-pair (eax:edx) arithmetic.
    fn emit_binop(&mut self, dest: &Value, op: IrBinOp, lhs: &Operand, rhs: &Operand, ty: IrType) {
        if matches!(ty, IrType::I64 | IrType::U64) {
            self.emit_i128_binop(dest, op, lhs, rhs);
            self.state.reg_cache.invalidate_all();
            return;
        }
        if crate::backend::generation::is_i128_type(ty) {
            self.emit_i128_binop(dest, op, lhs, rhs);
            return;
        }
        if ty.is_float() {
            let float_op = crate::backend::cast::classify_float_binop(op)
                .unwrap_or_else(|| panic!("unsupported float binop: {:?} on type {:?}", op, ty));
            self.emit_float_binop(dest, float_op, lhs, rhs, ty);
            return;
        }
        self.emit_int_binop(dest, op, lhs, rhs, ty);
    }

    /// Override emit_cmp to route F64 and I64/U64 comparisons correctly on i686.
    fn emit_cmp(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) {
        if ty == IrType::F128 {
            self.emit_f128_cmp(dest, op, lhs, rhs);
            return;
        }
        if ty == IrType::F64 || ty == IrType::F32 {
            self.emit_float_cmp(dest, op, lhs, rhs, ty);
            return;
        }
        if matches!(ty, IrType::I64 | IrType::U64) || crate::backend::generation::is_i128_type(ty) {
            self.emit_i128_cmp(dest, op, lhs, rhs);
            return;
        }
        self.emit_int_cmp(dest, op, lhs, rhs, ty);
    }

    fn emit_unaryop(&mut self, dest: &Value, op: IrUnaryOp, src: &Operand, ty: IrType) {
        if op == IrUnaryOp::IsConstant {
            self.emit_load_operand(&Operand::Const(IrConst::I32(0)));
            self.emit_store_result(dest);
            return;
        }
        if ty == IrType::F128 && matches!(op, IrUnaryOp::Neg) {
            self.emit_f128_neg(dest, src);
            return;
        }
        if ty == IrType::F64 && op == IrUnaryOp::Neg {
            self.emit_f64_load_to_x87(src);
            self.state.emit("    fchs");
            self.emit_f64_store_from_x87(dest);
            self.state.reg_cache.invalidate_acc();
            return;
        }
        if matches!(ty, IrType::I64 | IrType::U64) || crate::backend::generation::is_i128_type(ty) {
            self.emit_load_acc_pair(src);
            match op {
                IrUnaryOp::Neg => self.emit_i128_neg(),
                IrUnaryOp::Not => self.emit_i128_not(),
                IrUnaryOp::Clz => self.emit_i64_clz(),
                IrUnaryOp::Ctz => self.emit_i64_ctz(),
                IrUnaryOp::Popcount => self.emit_i64_popcount(),
                IrUnaryOp::Bswap => self.emit_i64_bswap(),
                IrUnaryOp::IsConstant => unreachable!("handled above"),
            }
            self.emit_store_acc_pair(dest);
            self.state.reg_cache.invalidate_all();
            return;
        }
        self.operand_to_eax(src);
        match op {
            IrUnaryOp::Neg => {
                if ty.is_float() {
                    self.emit_float_neg(ty);
                } else {
                    self.emit_int_neg(ty);
                }
            }
            IrUnaryOp::Not => self.emit_int_not(ty),
            IrUnaryOp::Clz => self.emit_int_clz(ty),
            IrUnaryOp::Ctz => self.emit_int_ctz(ty),
            IrUnaryOp::Popcount => self.emit_int_popcount(ty),
            IrUnaryOp::Bswap => self.emit_int_bswap(ty),
            IrUnaryOp::IsConstant => unreachable!("handled above"),
        }
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    /// Override emit_call to handle fastcall calling convention.
    fn emit_call(
        &mut self,
        args: &[Operand],
        arg_types: &[IrType],
        direct_name: Option<&str>,
        func_ptr: Option<&Operand>,
        dest: Option<Value>,
        return_type: IrType,
        is_variadic: bool,
        _num_fixed_args: usize,
        struct_arg_sizes: &[Option<usize>],
        struct_arg_aligns: &[Option<usize>],
        struct_arg_classes: &[Vec<crate::common::types::EightbyteClass>],
        struct_arg_riscv_float_classes: &[Option<crate::common::types::RiscvFloatClass>],
        is_sret: bool,
        is_fastcall: bool,
        ret_eightbyte_classes: &[crate::common::types::EightbyteClass],
    ) {
        if is_fastcall {
            self.emit_fastcall(args, arg_types, direct_name, func_ptr, dest, return_type);
            return;
        }
        use crate::backend::call_abi::*;
        let config = self.call_abi_config();
        let arg_classes_vec = classify_call_args(
            args,
            arg_types,
            struct_arg_sizes,
            struct_arg_aligns,
            struct_arg_classes,
            struct_arg_riscv_float_classes,
            is_variadic,
            &config,
        );
        let indirect = func_ptr.is_some() && direct_name.is_none();
        if indirect {
            self.emit_call_spill_fptr(func_ptr.expect("indirect call requires func_ptr"));
        }
        let stack_arg_space = self.emit_call_compute_stack_space(&arg_classes_vec, arg_types);
        let f128_temp_space =
            self.emit_call_f128_pre_convert(args, &arg_classes_vec, arg_types, stack_arg_space);
        self.state().reg_cache.invalidate_acc();
        let total_sp_adjust = self.emit_call_stack_args(
            args,
            &arg_classes_vec,
            arg_types,
            stack_arg_space,
            if indirect {
                self.emit_call_fptr_spill_size()
            } else {
                0
            },
            f128_temp_space,
        );
        self.state().reg_cache.invalidate_acc();
        self.emit_call_reg_args(
            args,
            &arg_classes_vec,
            arg_types,
            total_sp_adjust,
            f128_temp_space,
            stack_arg_space,
            &[],
        );
        self.emit_call_instruction(direct_name, func_ptr, indirect, stack_arg_space);
        let callee_pops = self.callee_pops_bytes_for_sret(is_sret);
        // Account for bytes the callee pops via `ret $N` (sret pointer on i686).
        if callee_pops > 0 {
            self.esp_adjust -= callee_pops as i64;
        }
        let effective_stack_cleanup = stack_arg_space.saturating_sub(callee_pops);
        self.emit_call_cleanup(effective_stack_cleanup, f128_temp_space, indirect);
        if let Some(dest) = dest {
            self.set_call_ret_eightbyte_classes(ret_eightbyte_classes);
            self.emit_call_store_result(&dest, return_type);
        }
    }

    fn callee_pops_bytes_for_sret(&self, is_sret: bool) -> usize {
        if is_sret {
            4
        } else {
            0
        }
    }

    // ---- Control flow ----

    fn jump_mnemonic(&self) -> &'static str {
        "jmp"
    }
    fn trap_instruction(&self) -> &'static str {
        "ud2"
    }

    fn emit_branch_nonzero(&mut self, label: &str) {
        self.state.emit("    testl %eax, %eax");
        emit!(self.state, "    jne {}", label);
    }

    /// On i686, 64-bit conditions need both 32-bit halves tested.
    fn emit_cond_branch_blocks(
        &mut self,
        cond: &Operand,
        true_block: BlockId,
        false_block: BlockId,
    ) {
        match cond {
            Operand::Const(IrConst::I64(v)) => {
                if *v != 0 {
                    self.emit_branch_to_block(true_block);
                } else {
                    self.emit_branch_to_block(false_block);
                }
                return;
            }
            Operand::Const(IrConst::F64(fval)) => {
                if *fval != 0.0 {
                    self.emit_branch_to_block(true_block);
                } else {
                    self.emit_branch_to_block(false_block);
                }
                return;
            }
            _ => {}
        }
        if let Operand::Value(v) = cond {
            if self.state.is_wide_value(v.0) {
                self.emit_wide_value_to_eax_ored(v.0);
                self.state.reg_cache.invalidate_acc();
                let true_label = true_block.as_label();
                self.emit_branch_nonzero(&true_label);
                self.emit_branch_to_block(false_block);
                return;
            }
        }
        self.operand_to_eax(cond);
        let true_label = true_block.as_label();
        self.emit_branch_nonzero(&true_label);
        self.emit_branch_to_block(false_block);
    }

    fn emit_jump_indirect(&mut self) {
        self.state.emit("    jmp *%eax");
    }

    /// Override emit_switch to handle 64-bit switch values on i686.
    fn emit_switch(
        &mut self,
        val: &Operand,
        cases: &[(i64, BlockId)],
        default: &BlockId,
        ty: IrType,
    ) {
        let is_wide = match val {
            Operand::Value(v) => self.state.is_wide_value(v.0),
            Operand::Const(IrConst::I64(_)) => true,
            _ => false,
        };

        if !is_wide {
            use crate::backend::traits::{
                MAX_JUMP_TABLE_RANGE, MIN_JUMP_TABLE_CASES, MIN_JUMP_TABLE_DENSITY_PERCENT,
            };
            let use_jump_table = if self.state.no_jump_tables {
                false
            } else if cases.len() >= MIN_JUMP_TABLE_CASES {
                let min_val = cases
                    .iter()
                    .map(|&(v, _)| v)
                    .min()
                    .expect("switch must have cases");
                let max_val = cases
                    .iter()
                    .map(|&(v, _)| v)
                    .max()
                    .expect("switch must have cases");
                let range = (max_val - min_val + 1) as usize;
                range <= MAX_JUMP_TABLE_RANGE
                    && cases.len() * 100 / range >= MIN_JUMP_TABLE_DENSITY_PERCENT
            } else {
                false
            };
            if use_jump_table {
                self.emit_switch_jump_table(val, cases, default, ty);
            } else {
                self.emit_load_operand(val);
                for &(case_val, target) in cases {
                    let label = target.as_label();
                    self.emit_switch_case_branch(case_val, &label, ty);
                }
                self.emit_branch_to_block(*default);
            }
            return;
        }

        // 64-bit switch: compare both 32-bit halves
        self.emit_load_acc_pair(val);
        for &(case_val, target) in cases {
            let case_low = case_val as i32;
            let case_high = (case_val >> 32) as i32;
            let label = target.as_label();
            let skip_label = format!(".Lswskip_{}", self.state.next_label_id());

            if case_low == 0 {
                self.state.emit("    testl %eax, %eax");
            } else {
                emit!(self.state, "    cmpl ${}, %eax", case_low);
            }
            emit!(self.state, "    jne {}", skip_label);

            if case_high == 0 {
                self.state.emit("    testl %edx, %edx");
            } else {
                emit!(self.state, "    cmpl ${}, %edx", case_high);
            }
            emit!(self.state, "    je {}", label);

            emit!(self.state, "{}:", skip_label);
        }
        self.emit_branch_to_block(*default);
        self.state.reg_cache.invalidate_all();
    }

    fn emit_switch_case_branch(&mut self, case_val: i64, label: &str, _ty: IrType) {
        let val = case_val as i32;
        if val == 0 {
            self.state.emit("    testl %eax, %eax");
        } else {
            emit!(self.state, "    cmpl ${}, %eax", val);
        }
        emit!(self.state, "    je {}", label);
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

        self.operand_to_eax(val);
        if min_val != 0 {
            emit!(self.state, "    subl ${}, %eax", min_val as i32);
        }
        emit!(self.state, "    cmpl ${}, %eax", range);
        emit!(self.state, "    jae {}", default_label);

        if self.state.pic_mode {
            emit!(self.state, "    leal {}@GOTOFF(%ebx), %ecx", table_label);
            self.state.emit("    movl (%ecx, %eax, 4), %eax");
            self.state.emit("    addl %ecx, %eax");
            self.state.emit("    jmp *%eax");
        } else {
            emit!(self.state, "    jmp *{}(, %eax, 4)", table_label);
        }

        self.state.emit(".section .rodata");
        self.state.emit(".align 4");
        self.state.emit_fmt(format_args!("{}:", table_label));
        for target in &table {
            let target_label = target.as_label();
            if self.state.pic_mode {
                self.state
                    .emit_fmt(format_args!("    .long {} - {}", target_label, table_label));
            } else {
                self.state
                    .emit_fmt(format_args!("    .long {}", target_label));
            }
        }
        let sect = self.state.current_text_section.clone();
        self.state
            .emit_fmt(format_args!(".section {},\"ax\",@progbits", sect));
        self.state.reg_cache.invalidate_all();
    }

    fn emit_float_binop(
        &mut self,
        dest: &Value,
        op: crate::backend::cast::FloatOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        if ty == IrType::F64 {
            let mnemonic = self.emit_float_binop_mnemonic(op);
            self.emit_f64_load_to_x87(lhs);
            self.emit_f64_load_to_x87(rhs);
            emit!(self.state, "    f{}p %st, %st(1)", mnemonic);
            self.emit_f64_store_from_x87(dest);
            self.state.reg_cache.invalidate_acc();
            return;
        }
        if ty == IrType::F128 {
            let mnemonic = self.emit_float_binop_mnemonic(op);
            self.emit_f128_load_to_x87(lhs);
            self.emit_f128_load_to_x87(rhs);
            emit!(self.state, "    f{}p %st, %st(1)", mnemonic);
            if let Some(slot) = self.state.get_slot(dest.0) {
                let sr = self.slot_ref(slot);
                emit!(self.state, "    fstpt {}", sr);
                self.state.f128_direct_slots.insert(dest.0);
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        let mnemonic = match op {
            crate::backend::cast::FloatOp::Add => "add",
            crate::backend::cast::FloatOp::Sub => "sub",
            crate::backend::cast::FloatOp::Mul => "mul",
            crate::backend::cast::FloatOp::Div => "div",
        };
        self.emit_load_operand(lhs);
        self.emit_acc_to_secondary();
        self.emit_load_operand(rhs);
        self.emit_float_binop_impl(mnemonic, ty);
        self.emit_store_result(dest);
    }

    fn emit_float_binop_mnemonic(&self, op: crate::backend::cast::FloatOp) -> &'static str {
        match op {
            crate::backend::cast::FloatOp::Add => "add",
            crate::backend::cast::FloatOp::Sub => "subr",
            crate::backend::cast::FloatOp::Mul => "mul",
            crate::backend::cast::FloatOp::Div => "divr",
        }
    }

    fn emit_float_binop_impl(&mut self, mnemonic: &str, ty: IrType) {
        if ty == IrType::F32 {
            self.state.emit("    movd %ecx, %xmm0");
            self.state.emit("    movd %eax, %xmm1");
            emit!(self.state, "    {}ss %xmm1, %xmm0", mnemonic);
            self.state.emit("    movd %xmm0, %eax");
        }
        self.state.reg_cache.invalidate_acc();
    }

    /// emit_copy_value: handles F128, wide (F64/I64/U64), and 32-bit copies.
    fn emit_copy_value(&mut self, dest: &Value, src: &Operand) {
        if let Operand::Const(IrConst::LongDouble(..)) = src {
            if let Some(dest_slot) = self.state.get_slot(dest.0) {
                self.emit_f128_load_to_x87(src);
                let sr = self.slot_ref(dest_slot);
                emit!(self.state, "    fstpt {}", sr);
                self.state.f128_direct_slots.insert(dest.0);
                return;
            }
        }

        if let Operand::Value(v) = src {
            if self.state.f128_direct_slots.contains(&v.0) {
                if let (Some(src_slot), Some(dest_slot)) =
                    (self.state.get_slot(v.0), self.state.get_slot(dest.0))
                {
                    let ssr = self.slot_ref(src_slot);
                    let dsr = self.slot_ref(dest_slot);
                    emit!(self.state, "    fldt {}", ssr);
                    emit!(self.state, "    fstpt {}", dsr);
                    self.state.f128_direct_slots.insert(dest.0);
                    return;
                }
            }
            if let Some(&alloca_ty) = self.state.alloca_types.get(&v.0) {
                if alloca_ty == IrType::F128 {
                    if let (Some(src_slot), Some(dest_slot)) =
                        (self.state.get_slot(v.0), self.state.get_slot(dest.0))
                    {
                        let ssr = self.slot_ref(src_slot);
                        let dsr = self.slot_ref(dest_slot);
                        emit!(self.state, "    fldt {}", ssr);
                        emit!(self.state, "    fstpt {}", dsr);
                        self.state.f128_direct_slots.insert(dest.0);
                        return;
                    }
                }
            }
        }

        let is_wide = match src {
            Operand::Value(v) => self.state.is_wide_value(v.0),
            Operand::Const(IrConst::F64(_)) => true,
            Operand::Const(IrConst::I64(_)) => true,
            _ => false,
        };
        if is_wide {
            if let Some(dest_slot) = self.state.get_slot(dest.0) {
                match src {
                    Operand::Value(v) => {
                        if let Some(src_slot) = self.state.get_slot(v.0) {
                            let ssr0 = self.slot_ref(src_slot);
                            let dsr0 = self.slot_ref(dest_slot);
                            let ssr4 = self.slot_ref_offset(src_slot, 4);
                            let dsr4 = self.slot_ref_offset(dest_slot, 4);
                            emit!(self.state, "    movl {}, %eax", ssr0);
                            emit!(self.state, "    movl %eax, {}", dsr0);
                            emit!(self.state, "    movl {}, %eax", ssr4);
                            emit!(self.state, "    movl %eax, {}", dsr4);
                        }
                    }
                    Operand::Const(IrConst::F64(val)) => {
                        let bits = val.to_bits();
                        let lo = bits as u32;
                        let hi = (bits >> 32) as u32;
                        let dsr0 = self.slot_ref(dest_slot);
                        let dsr4 = self.slot_ref_offset(dest_slot, 4);
                        emit!(self.state, "    movl ${}, {}", lo as i32, dsr0);
                        emit!(self.state, "    movl ${}, {}", hi as i32, dsr4);
                    }
                    Operand::Const(IrConst::I64(val)) => {
                        let lo = *val as u32;
                        let hi = (*val >> 32) as u32;
                        let dsr0 = self.slot_ref(dest_slot);
                        let dsr4 = self.slot_ref_offset(dest_slot, 4);
                        emit!(self.state, "    movl ${}, {}", lo as i32, dsr0);
                        emit!(self.state, "    movl ${}, {}", hi as i32, dsr4);
                    }
                    _ => unreachable!("unexpected wide constant type in i686 emit_copy"),
                }
                self.state.reg_cache.invalidate_all();
                return;
            }
        }

        self.emit_load_operand(src);
        self.emit_store_result(dest);
    }

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
        crate::backend::inline_asm::emit_inline_asm_common(
            self,
            template,
            outputs,
            inputs,
            clobbers,
            operand_types,
            goto_labels,
            input_symbols,
        );
    }

    fn emit_intrinsic(
        &mut self,
        dest: &Option<Value>,
        op: &IntrinsicOp,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
        self.emit_intrinsic_impl(dest, op, dest_ptr, args);
    }

    // ---- Delegated methods (via macro) ----

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
        fn emit_typed_store_indirect(&mut self, instr: &'static str, ty: IrType) => emit_typed_store_indirect_impl;
        fn emit_typed_load_indirect(&mut self, instr: &'static str) => emit_typed_load_indirect_impl;
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
        fn emit_alloca_aligned_addr(&mut self, slot: StackSlot, val_id: u32) => emit_alloca_aligned_addr_impl;
        fn emit_alloca_aligned_addr_to_acc(&mut self, slot: StackSlot, val_id: u32) => emit_alloca_aligned_addr_to_acc_impl;
        fn emit_memcpy_load_dest_addr(&mut self, slot: StackSlot, is_alloca: bool, val_id: u32) => emit_memcpy_load_dest_addr_impl;
        fn emit_memcpy_load_src_addr(&mut self, slot: StackSlot, is_alloca: bool, val_id: u32) => emit_memcpy_load_src_addr_impl;
        fn emit_memcpy_impl(&mut self, size: usize) => emit_memcpy_impl_impl;
        // alu
        fn emit_float_neg(&mut self, ty: IrType) => emit_float_neg_impl;
        fn emit_int_neg(&mut self, ty: IrType) => emit_int_neg_impl;
        fn emit_int_not(&mut self, ty: IrType) => emit_int_not_impl;
        fn emit_int_clz(&mut self, ty: IrType) => emit_int_clz_impl;
        fn emit_int_ctz(&mut self, ty: IrType) => emit_int_ctz_impl;
        fn emit_int_bswap(&mut self, ty: IrType) => emit_int_bswap_impl;
        fn emit_int_popcount(&mut self, ty: IrType) => emit_int_popcount_impl;
        fn emit_int_binop(&mut self, dest: &Value, op: IrBinOp, lhs: &Operand, rhs: &Operand, ty: IrType) => emit_int_binop_impl;
        // comparison
        fn emit_float_cmp(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) => emit_float_cmp_impl;
        fn emit_f128_cmp(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand) => emit_f128_cmp_impl;
        fn emit_int_cmp(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) => emit_int_cmp_impl;
        fn emit_fused_cmp_branch(&mut self, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType, true_label: &str, false_label: &str) => emit_fused_cmp_branch_impl;
        fn emit_select(&mut self, dest: &Value, cond: &Operand, true_val: &Operand, false_val: &Operand, ty: IrType) => emit_select_impl;
        fn emit_f128_neg(&mut self, dest: &Value, src: &Operand) => emit_f128_neg_impl;
        // calls
        fn call_abi_config(&self) -> call_abi::CallAbiConfig => call_abi_config_impl;
        fn emit_call_f128_pre_convert(&mut self, args: &[Operand], arg_classes: &[call_abi::CallArgClass], arg_types: &[IrType], stack_arg_space: usize) -> usize => emit_call_f128_pre_convert_impl;
        fn emit_call_compute_stack_space(&self, arg_classes: &[call_abi::CallArgClass], arg_types: &[IrType]) -> usize => emit_call_compute_stack_space_impl;
        fn emit_call_stack_args(&mut self, args: &[Operand], arg_classes: &[call_abi::CallArgClass], arg_types: &[IrType], stack_arg_space: usize, fptr_spill: usize, f128_temp_space: usize) -> i64 => emit_call_stack_args_impl;
        fn emit_call_reg_args(&mut self, args: &[Operand], arg_classes: &[call_abi::CallArgClass], arg_types: &[IrType], total_sp_adjust: i64, f128_temp_space: usize, stack_arg_space: usize, struct_arg_riscv_float_classes: &[Option<crate::common::types::RiscvFloatClass>]) => emit_call_reg_args_impl;
        fn emit_call_instruction(&mut self, direct_name: Option<&str>, func_ptr: Option<&Operand>, indirect: bool, stack_arg_space: usize) => emit_call_instruction_impl;
        fn emit_call_cleanup(&mut self, stack_arg_space: usize, f128_temp_space: usize, indirect: bool) => emit_call_cleanup_impl;
        fn emit_call_store_result(&mut self, dest: &Value, return_type: IrType) => emit_call_store_result_impl;
        fn emit_call_store_i128_result(&mut self, dest: &Value) => emit_call_store_i128_result_impl;
        fn emit_call_store_f128_result(&mut self, dest: &Value) => emit_call_store_f128_result_impl;
        fn emit_call_move_f32_to_acc(&mut self) => emit_call_move_f32_to_acc_impl;
        fn emit_call_move_f64_to_acc(&mut self) => emit_call_move_f64_to_acc_impl;
        // globals
        fn emit_global_addr(&mut self, dest: &Value, name: &str) => emit_global_addr_impl;
        fn emit_label_addr(&mut self, dest: &Value, label: &str) => emit_label_addr_impl;
        fn emit_tls_global_addr(&mut self, dest: &Value, name: &str) => emit_tls_global_addr_impl;
        // cast
        fn emit_cast(&mut self, dest: &Value, src: &Operand, from_ty: IrType, to_ty: IrType) => emit_cast_impl;
        fn emit_cast_instrs(&mut self, from_ty: IrType, to_ty: IrType) => emit_cast_instrs_impl;
        // variadic
        fn emit_va_arg(&mut self, dest: &Value, va_list_ptr: &Value, result_ty: IrType) => emit_va_arg_impl;
        fn emit_va_start(&mut self, va_list_ptr: &Value) => emit_va_start_impl;
        fn emit_va_copy(&mut self, dest_ptr: &Value, src_ptr: &Value) => emit_va_copy_impl;
        fn emit_va_arg_struct(&mut self, dest_ptr: &Value, va_list_ptr: &Value, size: usize) => emit_va_arg_struct_impl;
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
        fn emit_get_return_f128_second(&mut self, dest: &Value) => emit_get_return_f128_second_impl;
        fn emit_set_return_f128_second(&mut self, src: &Operand) => emit_set_return_f128_second_impl;
        // atomics
        fn emit_atomic_rmw(&mut self, dest: &Value, op: AtomicRmwOp, ptr: &Operand, val: &Operand, ty: IrType, ordering: AtomicOrdering) => emit_atomic_rmw_impl;
        fn emit_atomic_cmpxchg(&mut self, dest: &Value, ptr: &Operand, expected: &Operand, desired: &Operand, ty: IrType, success_ordering: AtomicOrdering, failure_ordering: AtomicOrdering, returns_bool: bool) => emit_atomic_cmpxchg_impl;
        fn emit_atomic_load(&mut self, dest: &Value, ptr: &Operand, ty: IrType, ordering: AtomicOrdering) => emit_atomic_load_impl;
        fn emit_atomic_store(&mut self, ptr: &Operand, val: &Operand, ty: IrType, ordering: AtomicOrdering) => emit_atomic_store_impl;
        fn emit_fence(&mut self, ordering: AtomicOrdering) => emit_fence_impl;
        // i128 ops
        fn emit_sign_extend_acc_high(&mut self) => emit_sign_extend_acc_high_impl;
        fn emit_zero_acc_high(&mut self) => emit_zero_acc_high_impl;
        fn emit_load_acc_pair(&mut self, op: &Operand) => emit_load_acc_pair_impl;
        fn emit_store_acc_pair(&mut self, dest: &Value) => emit_store_acc_pair_impl;
        fn emit_store_pair_to_slot(&mut self, slot: StackSlot) => emit_store_pair_to_slot_impl;
        fn emit_load_pair_from_slot(&mut self, slot: StackSlot) => emit_load_pair_from_slot_impl;
        fn emit_save_acc_pair(&mut self) => emit_save_acc_pair_impl;
        fn emit_store_pair_indirect(&mut self) => emit_store_pair_indirect_impl;
        fn emit_load_pair_indirect(&mut self) => emit_load_pair_indirect_impl;
        fn emit_i128_neg(&mut self) => emit_i128_neg_impl;
        fn emit_i128_not(&mut self) => emit_i128_not_impl;
        fn emit_i128_to_float_call(&mut self, src: &Operand, from_signed: bool, to_ty: IrType) => emit_i128_to_float_call_impl;
        fn emit_float_to_i128_call(&mut self, src: &Operand, to_signed: bool, from_ty: IrType) => emit_float_to_i128_call_impl;
        fn emit_i128_prep_binop(&mut self, lhs: &Operand, rhs: &Operand) => emit_i128_prep_binop_impl;
        fn emit_i128_prep_shift_lhs(&mut self, lhs: &Operand) => emit_i128_prep_shift_lhs_impl;
        fn emit_i128_add(&mut self) => emit_i128_add_impl;
        fn emit_i128_sub(&mut self) => emit_i128_sub_impl;
        fn emit_i128_mul(&mut self) => emit_i128_mul_impl;
        fn emit_i128_and(&mut self) => emit_i128_and_impl;
        fn emit_i128_or(&mut self) => emit_i128_or_impl;
        fn emit_i128_xor(&mut self) => emit_i128_xor_impl;
        fn emit_i128_shl(&mut self) => emit_i128_shl_impl;
        fn emit_i128_lshr(&mut self) => emit_i128_lshr_impl;
        fn emit_i128_ashr(&mut self) => emit_i128_ashr_impl;
        fn emit_i128_shl_const(&mut self, amount: u32) => emit_i128_shl_const_impl;
        fn emit_i128_lshr_const(&mut self, amount: u32) => emit_i128_lshr_const_impl;
        fn emit_i128_ashr_const(&mut self, amount: u32) => emit_i128_ashr_const_impl;
        fn emit_i128_divrem_call(&mut self, func_name: &str, lhs: &Operand, rhs: &Operand) => emit_i128_divrem_call_impl;
        fn emit_i128_store_result(&mut self, dest: &Value) => emit_i128_store_result_impl;
        fn emit_i128_cmp_eq(&mut self, is_ne: bool) => emit_i128_cmp_eq_impl;
        fn emit_i128_cmp_ordered(&mut self, op: IrCmpOp) => emit_i128_cmp_ordered_impl;
        fn emit_i128_cmp_store_result(&mut self, dest: &Value) => emit_i128_cmp_store_result_impl;
    }

    // ---- Segment overrides (x86-specific) ----

    fn emit_seg_load(&mut self, dest: &Value, ptr: &Value, ty: IrType, seg: AddressSpace) {
        self.operand_to_eax(&Operand::Value(*ptr));
        self.state.emit("    movl %eax, %ecx");
        let seg_prefix = match seg {
            AddressSpace::SegGs => "%gs:",
            AddressSpace::SegFs => "%fs:",
            AddressSpace::Default => {
                unreachable!("segment-prefixed op called with default address space")
            }
        };
        let load_instr = self.mov_load_for_type(ty);
        emit!(self.state, "    {} {}(%ecx), %eax", load_instr, seg_prefix);
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    fn emit_seg_store(&mut self, val: &Operand, ptr: &Value, ty: IrType, seg: AddressSpace) {
        self.operand_to_eax(val);
        self.state.emit("    movl %eax, %edx");
        self.operand_to_eax(&Operand::Value(*ptr));
        self.state.emit("    movl %eax, %ecx");
        let seg_prefix = match seg {
            AddressSpace::SegGs => "%gs:",
            AddressSpace::SegFs => "%fs:",
            AddressSpace::Default => {
                unreachable!("segment-prefixed op called with default address space")
            }
        };
        let store_instr = self.mov_store_for_type(ty);
        let reg = match ty {
            IrType::I8 | IrType::U8 => "%dl",
            IrType::I16 | IrType::U16 => "%dx",
            _ => "%edx",
        };
        emit!(
            self.state,
            "    {} {}, {}(%ecx)",
            store_instr,
            reg,
            seg_prefix
        );
    }

    fn emit_seg_load_symbol(&mut self, dest: &Value, sym: &str, ty: IrType, seg: AddressSpace) {
        let seg_prefix = match seg {
            AddressSpace::SegGs => "%gs:",
            AddressSpace::SegFs => "%fs:",
            AddressSpace::Default => {
                unreachable!("segment-prefixed op called with default address space")
            }
        };
        let load_instr = self.mov_load_for_type(ty);
        // i686 uses absolute addressing (no RIP-relative)
        emit!(self.state, "    {} {}{}, %eax", load_instr, seg_prefix, sym);
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    fn emit_seg_store_symbol(&mut self, val: &Operand, sym: &str, ty: IrType, seg: AddressSpace) {
        self.operand_to_eax(val);
        let seg_prefix = match seg {
            AddressSpace::SegGs => "%gs:",
            AddressSpace::SegFs => "%fs:",
            AddressSpace::Default => {
                unreachable!("segment-prefixed op called with default address space")
            }
        };
        let store_instr = self.mov_store_for_type(ty);
        let reg = match ty {
            IrType::I8 | IrType::U8 => "%al",
            IrType::I16 | IrType::U16 => "%ax",
            _ => "%eax",
        };
        emit!(
            self.state,
            "    {} {}, {}{}",
            store_instr,
            reg,
            seg_prefix,
            sym
        );
    }

    fn emit_runtime_stubs(&mut self) {
        if self.needs_pc_thunk_bx {
            let s = &mut self.state;
            s.emit("");
            s.emit(".section .text.__x86.get_pc_thunk.bx,\"axG\",@progbits,__x86.get_pc_thunk.bx,comdat");
            s.emit(".globl __x86.get_pc_thunk.bx");
            s.emit(".hidden __x86.get_pc_thunk.bx");
            s.emit(".type __x86.get_pc_thunk.bx, @function");
            s.emit("__x86.get_pc_thunk.bx:");
            s.emit("    movl (%esp), %ebx");
            s.emit("    ret");
            s.emit(".size __x86.get_pc_thunk.bx, .-__x86.get_pc_thunk.bx");
            s.emit(".text");
        }

        if !self.state.needs_divdi3_helpers {
            return;
        }

        // Switch to .text explicitly: the last function in the module may have been
        // in a custom section (e.g. .init.text), and without this directive the
        // helper stubs would inherit that section.  The Linux kernel's modpost
        // check rejects .text -> .init.text cross-references, so these must live
        // in .text.
        self.state.emit(".text");

        self.emit_udivdi3_stub();
        self.emit_umoddi3_stub();
        self.emit_divdi3_stub();
        self.emit_moddi3_stub();
    }
}

// ─── 64-bit division runtime stubs for i686 ──────────────────────────────────
//
// On 32-bit x86, 64-bit division/modulo requires runtime helpers normally
// provided by libgcc (__divdi3, __udivdi3, __moddi3, __umoddi3).  Standalone
// builds (e.g. musl libc) that don't link libgcc need the compiler to provide
// these.  We emit them as .weak symbols so that if libgcc IS linked, its
// versions take precedence.
//
// Calling convention (cdecl, stack-based):
//   4(%esp)  = dividend low  (A_lo)
//   8(%esp)  = dividend high (A_hi)
//   12(%esp) = divisor low   (B_lo)
//   16(%esp) = divisor high  (B_hi)
//   Return: edx:eax = 64-bit result

impl I686Codegen {
    /// Emit __udivdi3: unsigned 64-bit division, returns quotient in edx:eax.
    /// Algorithm based on compiler-rt's i386/udivdi3.S (Stephen Canon, 2008).
    /// Uses normalized-divisor estimation with remainder-based adjustment.
    fn emit_udivdi3_stub(&mut self) {
        let s = &mut self.state;
        s.emit("");
        s.emit(".weak __udivdi3");
        s.emit(".type __udivdi3, @function");
        s.emit("__udivdi3:");
        // Stack: ret(0), A_lo(4), A_hi(8), B_lo(12), B_hi(16)
        s.emit("    pushl %ebx");
        // Stack: ebx(0), ret(4), A_lo(8), A_hi(12), B_lo(16), B_hi(20)
        s.emit("    movl 20(%esp), %ebx"); // B_hi
        s.emit("    bsrl %ebx, %ecx"); // ecx = i = index of leading bit of B_hi
        s.emit("    jz .Ludiv_b_hi_zero"); // B_hi == 0 -> special case

        // --- B_hi != 0: quotient fits in 32 bits ---
        // Construct bhi = bits [1+i : 32+i] of B (top 32 bits of B, normalized).
        //   bhi = (B_lo >> (1+i)) | (B_hi << (31-i))
        s.emit("    movl 16(%esp), %eax"); // B_lo
        s.emit("    shrl %cl, %eax"); // B_lo >> i
        s.emit("    shrl %eax"); // B_lo >> (1+i)
        s.emit("    notl %ecx"); // cl = 31-i (mod 32)
        s.emit("    shll %cl, %ebx"); // B_hi << (31-i)
        s.emit("    orl %eax, %ebx"); // ebx = bhi
        s.emit("    movl 12(%esp), %edx"); // A_hi
        s.emit("    movl 8(%esp), %eax"); // A_lo
        s.emit("    cmpl %ebx, %edx"); // if A_hi >= bhi, need overflow path
        s.emit("    jae .Ludiv_big_overflow");

        // A_hi < bhi: divide edx:eax by bhi directly (no overflow)
        s.emit("    divl %ebx"); // eax = qs
        s.emit("    pushl %edi");
        // Stack: edi(0), ebx(4), ret(8), A_lo(12), A_hi(16), B_lo(20), B_hi(24)
        s.emit("    notl %ecx"); // cl = i again
        s.emit("    shrl %eax");
        s.emit("    shrl %cl, %eax"); // q = qs >> (1+i)
        s.emit("    movl %eax, %edi"); // edi = q
                                       // Verify: compute a - q*b, adjust if negative
        s.emit("    mull 20(%esp)"); // edx:eax = q * B_lo
        s.emit("    movl 12(%esp), %ebx");
        s.emit("    movl 16(%esp), %ecx"); // ecx:ebx = a
        s.emit("    subl %eax, %ebx");
        s.emit("    sbbl %edx, %ecx"); // ecx:ebx = a - q*B_lo
        s.emit("    movl 24(%esp), %eax"); // B_hi
        s.emit("    imull %edi, %eax"); // q * B_hi (low 32 bits)
        s.emit("    subl %eax, %ecx"); // ecx:ebx = a - q*b
        s.emit("    sbbl $0, %edi"); // if remainder was negative, decrement q
        s.emit("    xorl %edx, %edx");
        s.emit("    movl %edi, %eax");
        s.emit("    popl %edi");
        s.emit("    popl %ebx");
        s.emit("    ret");

        // A_hi >= bhi: subtract bhi first to avoid divl overflow
        s.emit(".Ludiv_big_overflow:");
        s.emit("    subl %ebx, %edx"); // edx = A_hi - bhi
        s.emit("    divl %ebx"); // eax = qs (for quotient 1:qs)
        s.emit("    pushl %edi");
        s.emit("    notl %ecx"); // cl = i
        s.emit("    shrl %eax");
        s.emit("    orl $0x80000000, %eax"); // set high bit (the '1' prefix)
        s.emit("    shrl %cl, %eax"); // q = (1:qs) >> (1+i)
        s.emit("    movl %eax, %edi");
        s.emit("    mull 20(%esp)"); // q * B_lo
        s.emit("    movl 12(%esp), %ebx");
        s.emit("    movl 16(%esp), %ecx");
        s.emit("    subl %eax, %ebx");
        s.emit("    sbbl %edx, %ecx");
        s.emit("    movl 24(%esp), %eax");
        s.emit("    imull %edi, %eax");
        s.emit("    subl %eax, %ecx");
        s.emit("    sbbl $0, %edi");
        s.emit("    xorl %edx, %edx");
        s.emit("    movl %edi, %eax");
        s.emit("    popl %edi");
        s.emit("    popl %ebx");
        s.emit("    ret");

        // --- B_hi == 0: two-step divide ---
        s.emit(".Ludiv_b_hi_zero:");
        s.emit("    movl 12(%esp), %eax"); // A_hi
        s.emit("    movl 16(%esp), %ecx"); // B_lo
        s.emit("    xorl %edx, %edx");
        s.emit("    divl %ecx"); // eax = Q_hi, edx = rem
        s.emit("    movl %eax, %ebx"); // save Q_hi
        s.emit("    movl 8(%esp), %eax"); // A_lo
        s.emit("    divl %ecx"); // eax = Q_lo
        s.emit("    movl %ebx, %edx"); // edx = Q_hi
        s.emit("    popl %ebx");
        s.emit("    ret");
        s.emit(".size __udivdi3, .-__udivdi3");
    }

    /// Emit __umoddi3: unsigned 64-bit modulo, returns remainder in edx:eax.
    /// Computes a % b = a - (a / b) * b, delegating division to __udivdi3.
    fn emit_umoddi3_stub(&mut self) {
        let s = &mut self.state;
        s.emit("");
        s.emit(".weak __umoddi3");
        s.emit(".type __umoddi3, @function");
        s.emit("__umoddi3:");
        // Stack: ret(0), A_lo(4), A_hi(8), B_lo(12), B_hi(16)
        s.emit("    pushl %ebx");
        s.emit("    pushl %esi");
        s.emit("    pushl %edi");
        s.emit("    pushl %ebp");
        // Stack: ebp(0), edi(4), esi(8), ebx(12), ret(16), A_lo(20), A_hi(24), B_lo(28), B_hi(32)

        // Call __udivdi3(A, B) to get quotient
        s.emit("    pushl 32(%esp)"); // B_hi
        s.emit("    pushl 32(%esp)"); // B_lo (28+4=32 after push)
        s.emit("    pushl 32(%esp)"); // A_hi (24+8=32 after two pushes)
        s.emit("    pushl 32(%esp)"); // A_lo (20+12=32 after three pushes)
        s.emit("    call __udivdi3");
        s.emit("    addl $16, %esp");
        // edx:eax = quotient (Q_hi:Q_lo)

        // Compute q * B (64-bit), result in ecx:ebx
        // q * B = Q_lo * B_lo + (Q_lo * B_hi + Q_hi * B_lo) << 32
        // We only need the low 64 bits.
        s.emit("    movl %eax, %ebx"); // save Q_lo
        s.emit("    movl %edx, %ecx"); // save Q_hi
        s.emit("    imull 28(%esp), %ecx"); // ecx = Q_hi * B_lo (low 32)
        s.emit("    movl 32(%esp), %ebp"); // B_hi
        s.emit("    imull %ebx, %ebp"); // ebp = Q_lo * B_hi (low 32)
        s.emit("    addl %ebp, %ecx"); // ecx = cross terms sum
        s.emit("    movl %ebx, %eax"); // eax = Q_lo
        s.emit("    mull 28(%esp)"); // edx:eax = Q_lo * B_lo
        s.emit("    addl %ecx, %edx"); // edx:eax = q * B (low 64 bits)

        // remainder = A - q*B
        s.emit("    movl 20(%esp), %ebx"); // A_lo
        s.emit("    movl 24(%esp), %ecx"); // A_hi
        s.emit("    subl %eax, %ebx");
        s.emit("    sbbl %edx, %ecx");
        s.emit("    movl %ebx, %eax");
        s.emit("    movl %ecx, %edx");
        s.emit("    popl %ebp");
        s.emit("    popl %edi");
        s.emit("    popl %esi");
        s.emit("    popl %ebx");
        s.emit("    ret");
        s.emit(".size __umoddi3, .-__umoddi3");
    }

    /// Emit __divdi3: signed 64-bit division.
    /// Negates operands to unsigned, calls __udivdi3, negates result if needed.
    fn emit_divdi3_stub(&mut self) {
        let s = &mut self.state;
        s.emit("");
        s.emit(".weak __divdi3");
        s.emit(".type __divdi3, @function");
        s.emit("__divdi3:");
        // Stack: ret, A_lo(4), A_hi(8), B_lo(12), B_hi(16)
        s.emit("    pushl %ebx");
        s.emit("    pushl %esi");
        s.emit("    pushl %edi");
        // Stack: edi, esi, ebx, ret, A_lo(16), A_hi(20), B_lo(24), B_hi(28)
        s.emit("    movl 20(%esp), %edx"); // A_hi
        s.emit("    movl 28(%esp), %ecx"); // B_hi
        s.emit("    movl %edx, %edi"); // save A_hi for sign
        s.emit("    xorl %ecx, %edi"); // edi = sign of result (bit 31)
        s.emit("    movl 16(%esp), %eax"); // A_lo
        s.emit("    movl 24(%esp), %ebx"); // B_lo
                                           // Negate A if negative
        s.emit("    testl %edx, %edx");
        s.emit("    jns .Ldiv_a_pos");
        s.emit("    negl %eax");
        s.emit("    adcl $0, %edx");
        s.emit("    negl %edx");
        s.emit(".Ldiv_a_pos:");
        // Negate B if negative
        s.emit("    testl %ecx, %ecx");
        s.emit("    jns .Ldiv_b_pos");
        s.emit("    negl %ebx");
        s.emit("    adcl $0, %ecx");
        s.emit("    negl %ecx");
        s.emit(".Ldiv_b_pos:");
        // Push unsigned args and call __udivdi3
        s.emit("    pushl %ecx"); // B_hi (unsigned)
        s.emit("    pushl %ebx"); // B_lo (unsigned)
        s.emit("    pushl %edx"); // A_hi (unsigned)
        s.emit("    pushl %eax"); // A_lo (unsigned)
        s.emit("    call __udivdi3");
        s.emit("    addl $16, %esp");
        // Result in edx:eax. Negate if sign differs.
        s.emit("    testl %edi, %edi");
        s.emit("    jns .Ldiv_done");
        s.emit("    negl %eax");
        s.emit("    adcl $0, %edx");
        s.emit("    negl %edx");
        s.emit(".Ldiv_done:");
        s.emit("    popl %edi");
        s.emit("    popl %esi");
        s.emit("    popl %ebx");
        s.emit("    ret");
        s.emit(".size __divdi3, .-__divdi3");
    }

    /// Emit __moddi3: signed 64-bit modulo.
    /// Negates operands to unsigned, calls __umoddi3, negates result if dividend was negative.
    fn emit_moddi3_stub(&mut self) {
        let s = &mut self.state;
        s.emit("");
        s.emit(".weak __moddi3");
        s.emit(".type __moddi3, @function");
        s.emit("__moddi3:");
        // Stack: ret, A_lo(4), A_hi(8), B_lo(12), B_hi(16)
        s.emit("    pushl %ebx");
        s.emit("    pushl %esi");
        s.emit("    pushl %edi");
        // Stack: edi, esi, ebx, ret, A_lo(16), A_hi(20), B_lo(24), B_hi(28)
        s.emit("    movl 20(%esp), %edx"); // A_hi
        s.emit("    movl 28(%esp), %ecx"); // B_hi
        s.emit("    movl %edx, %edi"); // save A_hi sign (remainder sign = dividend sign)
        s.emit("    movl 16(%esp), %eax"); // A_lo
        s.emit("    movl 24(%esp), %ebx"); // B_lo
                                           // Negate A if negative
        s.emit("    testl %edx, %edx");
        s.emit("    jns .Lmod_a_pos");
        s.emit("    negl %eax");
        s.emit("    adcl $0, %edx");
        s.emit("    negl %edx");
        s.emit(".Lmod_a_pos:");
        // Negate B if negative
        s.emit("    testl %ecx, %ecx");
        s.emit("    jns .Lmod_b_pos");
        s.emit("    negl %ebx");
        s.emit("    adcl $0, %ecx");
        s.emit("    negl %ecx");
        s.emit(".Lmod_b_pos:");
        // Push unsigned args and call __umoddi3
        s.emit("    pushl %ecx");
        s.emit("    pushl %ebx");
        s.emit("    pushl %edx");
        s.emit("    pushl %eax");
        s.emit("    call __umoddi3");
        s.emit("    addl $16, %esp");
        // Negate result if dividend was negative
        s.emit("    testl %edi, %edi");
        s.emit("    jns .Lmod_done");
        s.emit("    negl %eax");
        s.emit("    adcl $0, %edx");
        s.emit("    negl %edx");
        s.emit(".Lmod_done:");
        s.emit("    popl %edi");
        s.emit("    popl %esi");
        s.emit("    popl %ebx");
        s.emit("    ret");
        s.emit(".size __moddi3, .-__moddi3");
    }
}
