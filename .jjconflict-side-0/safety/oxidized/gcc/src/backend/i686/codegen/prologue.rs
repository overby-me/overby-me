//! I686Codegen: prologue/epilogue and stack frame operations.

use super::emit::{
    i686_clobber_to_phys, i686_constraint_to_phys, phys_reg_name, I686Codegen, I686_CALLEE_SAVED,
    I686_CALLEE_SAVED_WITH_EBP, I686_CALLER_SAVED,
};
use crate::backend::call_abi::{classify_params, ParamClass};
use crate::backend::generation::{
    calculate_stack_space_common, collect_inline_asm_callee_saved_with_generic,
    filter_available_regs, find_param_alloca, is_i128_type, run_regalloc_and_merge_clobbers,
};
use crate::backend::regalloc::PhysReg;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{Instruction, IrFunction, Value};

impl I686Codegen {
    // ---- calculate_stack_space ----

    pub(super) fn calculate_stack_space_impl(&mut self, func: &IrFunction) -> i64 {
        self.is_variadic = func.is_variadic;
        self.is_fastcall = func.is_fastcall;
        self.current_return_type = func.return_type;

        // Dynamic alloca (VLAs) requires the frame pointer to track the stack,
        // since ESP changes by runtime-computed amounts.
        if self.state.has_dyn_alloca {
            self.omit_frame_pointer = false;
        }

        // Compute named parameter stack bytes for va_start (variadic functions).
        if func.is_variadic {
            let config = self.call_abi_config();
            let classification = crate::backend::call_abi::classify_params_full(func, &config);
            self.va_named_stack_bytes = classification.total_stack_bytes;
        }

        // Run register allocator before stack space computation.
        // Use the _with_generic variant to conservatively mark all callee-saved
        // registers as clobbered when generic register constraints (r, q, g) are
        // present. On i686, the scratch allocator may pick esi/edi/ebx for generic
        // constraints, which would clobber values the register allocator placed there.
        let mut asm_clobbered_regs: Vec<PhysReg> = Vec::new();

        // When omitting the frame pointer, EBP is available as a callee-saved
        // register, so use the extended set that includes EBP.
        let callee_saved_set = if self.omit_frame_pointer {
            I686_CALLEE_SAVED_WITH_EBP
        } else {
            I686_CALLEE_SAVED
        };

        collect_inline_asm_callee_saved_with_generic(
            func,
            &mut asm_clobbered_regs,
            i686_constraint_to_phys,
            i686_clobber_to_phys,
            callee_saved_set,
        );
        // In PIC mode, %ebx (PhysReg(0)) is reserved as the GOT base pointer.
        if self.state.pic_mode && !asm_clobbered_regs.contains(&PhysReg(0)) {
            asm_clobbered_regs.push(PhysReg(0));
        }
        let available_regs = filter_available_regs(callee_saved_set, &asm_clobbered_regs);

        let caller_saved_regs = I686_CALLER_SAVED.to_vec();

        let (reg_assigned, cached_liveness) = run_regalloc_and_merge_clobbers(
            func,
            available_regs,
            caller_saved_regs,
            &asm_clobbered_regs,
            &mut self.reg_assignments,
            &mut self.used_callee_saved,
            false,
        );

        // In PIC mode, %ebx must be saved/restored as a callee-saved register.
        if self.state.pic_mode && !self.used_callee_saved.contains(&PhysReg(0)) {
            self.used_callee_saved.insert(0, PhysReg(0));
        }

        let callee_saved_bytes = self.used_callee_saved.len() as i64 * 4;

        // The bias ensures that slots requiring >= 16-byte alignment land on
        // 16-byte boundaries at runtime. The correct value depends on the
        // stack overhead between the 16-byte-aligned call-site ESP and the
        // reference point for slot addressing:
        //
        //   With frame pointer:   return addr (4) + saved ebp (4) = 8
        //     Address of slot -X = EBP - X = (16n - 8) - X, aligned when X ≡ 8 mod 16
        //
        //   Without frame pointer: return addr (4) only
        //     Address of slot = 16n - 4 - space, aligned when space ≡ 12 mod 16
        let omit_fp = self.omit_frame_pointer;
        let alignment_bias: i64 = if omit_fp { 12 } else { 8 };

        calculate_stack_space_common(
            &mut self.state,
            func,
            callee_saved_bytes,
            |space, alloc_size, align| {
                let effective_align = if align > 0 { align.max(4) } else { 4 };
                let alloc = (alloc_size + 3) & !3;
                let required = space + alloc;
                let new_space = if effective_align >= 16 {
                    let bias = alignment_bias;
                    let a = effective_align;
                    let rem = ((required % a) + a) % a;
                    let needed = if rem <= bias {
                        bias - rem
                    } else {
                        a - rem + bias
                    };
                    required + needed
                } else {
                    ((required + effective_align - 1) / effective_align) * effective_align
                };
                (-new_space, new_space)
            },
            &reg_assigned,
            callee_saved_set,
            cached_liveness,
            false,
        )
    }

    // ---- aligned_frame_size ----

    pub(super) fn aligned_frame_size_impl(&self, raw_space: i64) -> i64 {
        let callee_saved_bytes = self.used_callee_saved.len() as i64 * 4;
        let raw_locals = raw_space - callee_saved_bytes;
        // With frame pointer: overhead = callee_saved + 8 (saved ebp + return addr)
        // Without frame pointer: overhead = callee_saved + 4 (return addr only)
        let fixed_overhead = if self.omit_frame_pointer {
            callee_saved_bytes + 4
        } else {
            callee_saved_bytes + 8
        };
        let needed = raw_locals + fixed_overhead;
        let aligned = (needed + 15) & !15;
        aligned - fixed_overhead
    }

    // ---- emit_prologue ----

    pub(super) fn emit_prologue_impl(&mut self, _func: &IrFunction, frame_size: i64) {
        if self.omit_frame_pointer {
            // No frame pointer setup; use ESP-relative addressing.
            // frame_base_offset and esp_adjust will be set after callee-saved pushes.
            // TODO: Emit ESP-relative CFI directives (.cfi_def_cfa_offset after each
            // push/sub) for proper unwinding when frame pointer is omitted. Currently
            // the default .cfi_startproc CFA (ESP+4) is used, which is only valid at
            // function entry. This is acceptable for now since -fomit-frame-pointer on
            // i686 is primarily used by the Linux kernel boot code, which disables
            // unwind tables via -fno-asynchronous-unwind-tables.
        } else {
            self.state.emit("    pushl %ebp");
            if self.state.emit_cfi {
                self.state.emit("    .cfi_def_cfa_offset 8");
                self.state.emit("    .cfi_offset %ebp, -8");
            }
            self.state.emit("    movl %esp, %ebp");
            if self.state.emit_cfi {
                self.state.emit("    .cfi_def_cfa_register %ebp");
            }
        }

        for &reg in self.used_callee_saved.iter() {
            let name = phys_reg_name(reg);
            emit!(self.state, "    pushl %{}", name);
        }

        if self.state.pic_mode {
            debug_assert!(
                self.used_callee_saved.contains(&PhysReg(0)),
                "PIC mode requires ebx in used_callee_saved"
            );
            self.state.emit("    call __x86.get_pc_thunk.bx");
            self.state.emit("    addl $_GLOBAL_OFFSET_TABLE_, %ebx");
            self.needs_pc_thunk_bx = true;
        }

        if frame_size > 0 {
            emit!(self.state, "    subl ${}, %esp", frame_size);
        }

        if self.omit_frame_pointer {
            let callee_saved_bytes = self.used_callee_saved.len() as i64 * 4;
            self.frame_base_offset = callee_saved_bytes + frame_size;
            self.esp_adjust = 0;
        }
    }

    // ---- emit_epilogue ----

    pub(super) fn emit_epilogue_impl(&mut self, _frame_size: i64) {
        if self.omit_frame_pointer {
            let callee_saved_bytes = self.used_callee_saved.len() as i64 * 4;
            let total = self.frame_base_offset - callee_saved_bytes;
            if total > 0 {
                emit!(self.state, "    addl ${}, %esp", total);
            }
        } else {
            let callee_saved_bytes = self.used_callee_saved.len() as i64 * 4;
            if callee_saved_bytes > 0 {
                emit!(self.state, "    leal -{}(%ebp), %esp", callee_saved_bytes);
            } else {
                self.state.emit("    movl %ebp, %esp");
            }
        }

        for &reg in self.used_callee_saved.iter().rev() {
            let name = phys_reg_name(reg);
            emit!(self.state, "    popl %{}", name);
        }

        if !self.omit_frame_pointer {
            self.state.emit("    popl %ebp");
        }
    }

    // ---- emit_store_params ----

    pub(super) fn emit_store_params_impl(&mut self, func: &IrFunction) {
        let config = self.call_abi_config();
        let param_classes = classify_params(func, &config);
        self.state.param_classes = param_classes.clone();
        self.state.num_params = func.params.len();
        self.state.func_is_variadic = func.is_variadic;

        self.state.param_alloca_slots = (0..func.params.len())
            .map(|i| {
                find_param_alloca(func, i)
                    .and_then(|(dest, ty)| self.state.get_slot(dest.0).map(|slot| (slot, ty)))
            })
            .collect();

        let fastcall_reg_count = if self.is_fastcall {
            self.count_fastcall_reg_params(func)
        } else {
            0
        };
        self.fastcall_reg_param_count = fastcall_reg_count;

        if self.is_fastcall {
            let mut total_stack_bytes: usize = 0;
            for (i, _p) in func.params.iter().enumerate() {
                if i < fastcall_reg_count {
                    continue;
                }
                let ty = func.params[i].ty;
                let size = match ty {
                    IrType::I64 | IrType::U64 | IrType::F64 => 8,
                    IrType::F128 => 12,
                    _ if is_i128_type(ty) => 16,
                    _ => 4,
                };
                total_stack_bytes += size;
            }
            self.fastcall_stack_cleanup = total_stack_bytes;
        } else {
            self.fastcall_stack_cleanup = 0;
        }

        // Build a map of param_idx -> ParamRef dest Value for fast lookup.
        // Used to handle the case where param alloca was eliminated by mem2reg
        // but the register allocator assigned a callee-saved register.
        let mut paramref_dests: Vec<Option<Value>> = vec![None; func.params.len()];
        if self.is_fastcall {
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let Instruction::ParamRef {
                        dest, param_idx, ..
                    } = inst
                    {
                        if *param_idx < paramref_dests.len() {
                            paramref_dests[*param_idx] = Some(*dest);
                        }
                    }
                }
            }
        }

        let stack_base: i64 = 8;
        let mut fastcall_reg_idx = 0usize;

        // Build a map from physical register -> list of param indices that use it,
        // so we can detect when two params share the same callee-saved register.
        let mut reg_to_params: crate::common::fx_hash::FxHashMap<u8, Vec<usize>> =
            crate::common::fx_hash::FxHashMap::default();
        if self.is_fastcall {
            for (i, _) in func.params.iter().enumerate() {
                if let Some(paramref_dest) = paramref_dests[i] {
                    if let Some(&phys_reg) = self.reg_assignments.get(&paramref_dest.0) {
                        reg_to_params.entry(phys_reg.0).or_default().push(i);
                    }
                }
            }
        }

        for (i, _param) in func.params.iter().enumerate() {
            let class = param_classes[i];

            // Pre-store optimization for fastcall register params: when the param's
            // alloca was eliminated (by mem2reg) but the ParamRef dest is register-
            // allocated to a callee-saved register, store the fastcall ABI register
            // (%ecx/%edx) directly to the assigned physical register. This is critical
            // because:
            // 1. Dead alloca means no stack slot exists for this param
            // 2. %ecx/%edx are caller-saved and will be clobbered
            // 3. We must save the value NOW, before any other code runs
            // 4. emit_param_ref will see param_pre_stored and skip code generation
            if self.is_fastcall && fastcall_reg_idx < fastcall_reg_count {
                let param_ty = func.params[i].ty;
                if self.is_fastcall_reg_eligible(param_ty) {
                    let has_alloca_slot = find_param_alloca(func, i)
                        .and_then(|(dest, _)| self.state.get_slot(dest.0))
                        .is_some();
                    if !has_alloca_slot {
                        let src_reg = if fastcall_reg_idx == 0 {
                            "%ecx"
                        } else {
                            "%edx"
                        };
                        if let Some(paramref_dest) = paramref_dests[i] {
                            if let Some(&phys_reg) = self.reg_assignments.get(&paramref_dest.0) {
                                // Safety check: if another param's dest is also assigned
                                // to this register, skip pre-store to avoid conflicts.
                                let shared = reg_to_params
                                    .get(&phys_reg.0)
                                    .is_some_and(|users| users.len() > 1);
                                if !shared {
                                    // Store directly to the callee-saved register
                                    let dest_reg = phys_reg_name(phys_reg);
                                    emit!(self.state, "    movl {}, %{}", src_reg, dest_reg);
                                    self.state.param_pre_stored.insert(i);
                                }
                            } else if let Some(slot) = self.state.get_slot(paramref_dest.0) {
                                // Value was spilled to a stack slot - no register conflict
                                let slot_ref = self.slot_ref(slot);
                                emit!(self.state, "    movl {}, {}", src_reg, slot_ref);
                                self.state.param_pre_stored.insert(i);
                            }
                        }
                        fastcall_reg_idx += 1;
                        continue;
                    }
                }
            }

            let (slot, ty, dest_id) = if let Some((dest, ty)) = find_param_alloca(func, i) {
                if let Some(slot) = self.state.get_slot(dest.0) {
                    (slot, ty, dest.0)
                } else {
                    if self.is_fastcall
                        && fastcall_reg_idx < fastcall_reg_count
                        && i < func.params.len()
                        && self.is_fastcall_reg_eligible(ty)
                    {
                        fastcall_reg_idx += 1;
                    }
                    continue;
                }
            } else {
                if self.is_fastcall
                    && fastcall_reg_idx < fastcall_reg_count
                    && i < func.params.len()
                {
                    let param_ty = func.params[i].ty;
                    if self.is_fastcall_reg_eligible(param_ty) {
                        fastcall_reg_idx += 1;
                    }
                }
                continue;
            };

            if self.is_fastcall
                && fastcall_reg_idx < fastcall_reg_count
                && self.is_fastcall_reg_eligible(ty)
            {
                let src_reg_full = if fastcall_reg_idx == 0 {
                    "%ecx"
                } else {
                    "%edx"
                };
                let slot_ref = self.slot_ref(slot);
                // For sub-int types, sign/zero-extend to full 32-bit before
                // storing to the 4-byte SSA slot (avoids partial-write issues).
                match ty {
                    IrType::I8 => {
                        let src_byte = if fastcall_reg_idx == 0 { "%cl" } else { "%dl" };
                        emit!(self.state, "    movsbl {}, {}", src_byte, src_reg_full);
                        emit!(self.state, "    movl {}, {}", src_reg_full, slot_ref);
                    }
                    IrType::U8 => {
                        let src_byte = if fastcall_reg_idx == 0 { "%cl" } else { "%dl" };
                        emit!(self.state, "    movzbl {}, {}", src_byte, src_reg_full);
                        emit!(self.state, "    movl {}, {}", src_reg_full, slot_ref);
                    }
                    IrType::I16 => {
                        let src_word = if fastcall_reg_idx == 0 { "%cx" } else { "%dx" };
                        emit!(self.state, "    movswl {}, {}", src_word, src_reg_full);
                        emit!(self.state, "    movl {}, {}", src_reg_full, slot_ref);
                    }
                    IrType::U16 => {
                        let src_word = if fastcall_reg_idx == 0 { "%cx" } else { "%dx" };
                        emit!(self.state, "    movzwl {}, {}", src_word, src_reg_full);
                        emit!(self.state, "    movl {}, {}", src_reg_full, slot_ref);
                    }
                    _ => {
                        emit!(self.state, "    movl {}, {}", src_reg_full, slot_ref);
                    }
                }
                fastcall_reg_idx += 1;
                continue;
            }

            let stack_offset_adjust = if self.is_fastcall {
                fastcall_reg_count as i64 * 4
            } else {
                0
            };

            match class {
                ParamClass::StackScalar { offset } => {
                    let src_offset = stack_base + offset - stack_offset_adjust;
                    if ty == IrType::F64 || ty == IrType::I64 || ty == IrType::U64 {
                        let src_ref = self.param_ref(src_offset);
                        let dst_ref = self.slot_ref(slot);
                        emit!(self.state, "    movl {}, %eax", src_ref);
                        emit!(self.state, "    movl %eax, {}", dst_ref);
                        let src_ref_hi = self.param_ref(src_offset + 4);
                        let dst_ref_hi = self.slot_ref_offset(slot, 4);
                        emit!(self.state, "    movl {}, %eax", src_ref_hi);
                        emit!(self.state, "    movl %eax, {}", dst_ref_hi);
                    } else {
                        let load_instr = self.mov_load_for_type(ty);
                        let src_ref = self.param_ref(src_offset);
                        let dst_ref = self.slot_ref(slot);
                        emit!(self.state, "    {} {}, %eax", load_instr, src_ref);
                        // Always store full 32-bit value to SSA slot. The load
                        // instruction above already sign/zero-extended sub-int
                        // types into the full eax register. Using movb/movw here
                        // would leave garbage in the upper bytes of the 4-byte
                        // slot, which gets read back later by movl.
                        emit!(self.state, "    movl %eax, {}", dst_ref);
                    }
                }
                ParamClass::StructStack { offset, size } => {
                    let src = stack_base + offset - stack_offset_adjust;
                    let mut copied = 0usize;
                    while copied + 4 <= size {
                        let src_ref = self.param_ref(src + copied as i64);
                        let dst_ref = self.slot_ref_offset(slot, copied as i64);
                        emit!(self.state, "    movl {}, %eax", src_ref);
                        emit!(self.state, "    movl %eax, {}", dst_ref);
                        copied += 4;
                    }
                    while copied < size {
                        let src_ref = self.param_ref(src + copied as i64);
                        let dst_ref = self.slot_ref_offset(slot, copied as i64);
                        emit!(self.state, "    movb {}, %al", src_ref);
                        emit!(self.state, "    movb %al, {}", dst_ref);
                        copied += 1;
                    }
                }
                ParamClass::LargeStructStack { offset, size } => {
                    let src = stack_base + offset - stack_offset_adjust;
                    let mut copied = 0usize;
                    while copied + 4 <= size {
                        let src_ref = self.param_ref(src + copied as i64);
                        let dst_ref = self.slot_ref_offset(slot, copied as i64);
                        emit!(self.state, "    movl {}, %eax", src_ref);
                        emit!(self.state, "    movl %eax, {}", dst_ref);
                        copied += 4;
                    }
                    while copied < size {
                        let src_ref = self.param_ref(src + copied as i64);
                        let dst_ref = self.slot_ref_offset(slot, copied as i64);
                        emit!(self.state, "    movb {}, %al", src_ref);
                        emit!(self.state, "    movb %al, {}", dst_ref);
                        copied += 1;
                    }
                }
                ParamClass::F128AlwaysStack { offset } => {
                    let src = stack_base + offset - stack_offset_adjust;
                    let src_ref = self.param_ref(src);
                    let dst_ref = self.slot_ref(slot);
                    emit!(self.state, "    fldt {}", src_ref);
                    emit!(self.state, "    fstpt {}", dst_ref);
                    self.state.f128_direct_slots.insert(dest_id);
                }
                ParamClass::I128Stack { offset } => {
                    let src = stack_base + offset - stack_offset_adjust;
                    for j in (0..16).step_by(4) {
                        let src_ref = self.param_ref(src + j as i64);
                        let dst_ref = self.slot_ref_offset(slot, j as i64);
                        emit!(self.state, "    movl {}, %eax", src_ref);
                        emit!(self.state, "    movl %eax, {}", dst_ref);
                    }
                }
                ParamClass::F128Stack { offset } => {
                    let src = stack_base + offset - stack_offset_adjust;
                    let src_ref = self.param_ref(src);
                    let dst_ref = self.slot_ref(slot);
                    emit!(self.state, "    fldt {}", src_ref);
                    emit!(self.state, "    fstpt {}", dst_ref);
                    self.state.f128_direct_slots.insert(dest_id);
                }
                ParamClass::IntReg { reg_idx } => {
                    // regparm: param arrives in EAX/EDX/ECX (reg_idx 0/1/2)
                    let regparm_regs_full = ["%eax", "%edx", "%ecx"];
                    let regparm_regs_byte = ["%al", "%dl", "%cl"];
                    let regparm_regs_word = ["%ax", "%dx", "%cx"];
                    let src_full = regparm_regs_full[reg_idx];
                    let slot_ref = self.slot_ref(slot);
                    match ty {
                        IrType::I8 => {
                            let src_byte = regparm_regs_byte[reg_idx];
                            emit!(self.state, "    movsbl {}, {}", src_byte, src_full);
                            emit!(self.state, "    movl {}, {}", src_full, slot_ref);
                        }
                        IrType::U8 => {
                            let src_byte = regparm_regs_byte[reg_idx];
                            emit!(self.state, "    movzbl {}, {}", src_byte, src_full);
                            emit!(self.state, "    movl {}, {}", src_full, slot_ref);
                        }
                        IrType::I16 => {
                            let src_word = regparm_regs_word[reg_idx];
                            emit!(self.state, "    movswl {}, {}", src_word, src_full);
                            emit!(self.state, "    movl {}, {}", src_full, slot_ref);
                        }
                        IrType::U16 => {
                            let src_word = regparm_regs_word[reg_idx];
                            emit!(self.state, "    movzwl {}, {}", src_word, src_full);
                            emit!(self.state, "    movl {}, {}", src_full, slot_ref);
                        }
                        _ => {
                            emit!(self.state, "    movl {}, {}", src_full, slot_ref);
                        }
                    }
                }
                _ => {
                    // Remaining register classes (FloatReg, StructByValReg, etc.)
                    // don't apply to i686's ABI classification.
                }
            }
        }
    }

    // ---- emit_param_ref ----

    pub(super) fn emit_param_ref_impl(&mut self, dest: &Value, param_idx: usize, ty: IrType) {
        use crate::backend::call_abi::ParamClass;

        // If this param was pre-stored in the prologue (fastcall register param
        // with eliminated alloca), the value is already in the correct physical
        // register or stack slot. No code generation needed.
        if self.state.param_pre_stored.contains(&param_idx) {
            return;
        }

        if param_idx < self.state.param_alloca_slots.len() {
            if let Some((alloca_slot, _alloca_ty)) = self.state.param_alloca_slots[param_idx] {
                if let Some(dest_slot) = self.state.get_slot(dest.0) {
                    if dest_slot.0 == alloca_slot.0 {
                        // The param value is already in the alloca slot (stored by
                        // emit_store_params). If dest also has a register assignment,
                        // we must initialize the register from the slot — otherwise
                        // the register contains garbage from the caller, and any
                        // subsequent read via operand_to_eax will use the register
                        // (uninitialized) instead of the slot.
                        if let Some(phys) = self.dest_reg(dest) {
                            let reg = phys_reg_name(phys);
                            let load_instr = self.mov_load_for_type(ty);
                            let src_ref = self.slot_ref(alloca_slot);
                            emit!(self.state, "    {} {}, %eax", load_instr, src_ref);
                            emit!(self.state, "    movl %eax, %{}", reg);
                            self.state.reg_cache.invalidate_acc();
                        }
                        return;
                    }
                }
                if let Some(dest_slot) = self.state.get_slot(dest.0) {
                    if is_i128_type(ty) {
                        for i in (0..16).step_by(4) {
                            let src_ref = self.slot_ref_offset(alloca_slot, i as i64);
                            let dst_ref = self.slot_ref_offset(dest_slot, i as i64);
                            emit!(self.state, "    movl {}, %eax", src_ref);
                            emit!(self.state, "    movl %eax, {}", dst_ref);
                        }
                    } else if ty == IrType::F128 {
                        let src_ref = self.slot_ref(alloca_slot);
                        let dst_ref = self.slot_ref(dest_slot);
                        emit!(self.state, "    fldt {}", src_ref);
                        emit!(self.state, "    fstpt {}", dst_ref);
                        self.state.f128_direct_slots.insert(dest.0);
                    } else if ty == IrType::F64 || ty == IrType::I64 || ty == IrType::U64 {
                        let src_ref = self.slot_ref(alloca_slot);
                        let dst_ref = self.slot_ref(dest_slot);
                        emit!(self.state, "    movl {}, %eax", src_ref);
                        emit!(self.state, "    movl %eax, {}", dst_ref);
                        let src_ref_hi = self.slot_ref_offset(alloca_slot, 4);
                        let dst_ref_hi = self.slot_ref_offset(dest_slot, 4);
                        emit!(self.state, "    movl {}, %eax", src_ref_hi);
                        emit!(self.state, "    movl %eax, {}", dst_ref_hi);
                    } else {
                        let load_instr = self.mov_load_for_type(ty);
                        let src_ref = self.slot_ref(alloca_slot);
                        emit!(self.state, "    {} {}, %eax", load_instr, src_ref);
                        self.store_eax_to(dest);
                    }
                    return;
                }
            }
        }

        if self.is_fastcall && param_idx < self.fastcall_reg_param_count {
            if let Some(Some((slot, _slot_ty))) = self.state.param_alloca_slots.get(param_idx) {
                let load_instr = self.mov_load_for_type(ty);
                let slot_ref = self.slot_ref(*slot);
                emit!(self.state, "    {} {}, %eax", load_instr, slot_ref);
                self.store_eax_to(dest);
            }
            return;
        }

        let stack_base: i64 = 8;
        let stack_offset_adjust = if self.is_fastcall {
            self.fastcall_reg_param_count as i64 * 4
        } else {
            0
        };
        let param_offset = if param_idx < self.state.param_classes.len() {
            match self.state.param_classes[param_idx] {
                ParamClass::StackScalar { offset }
                | ParamClass::StructStack { offset, .. }
                | ParamClass::LargeStructStack { offset, .. }
                | ParamClass::F128AlwaysStack { offset }
                | ParamClass::I128Stack { offset }
                | ParamClass::F128Stack { offset }
                | ParamClass::LargeStructByRefStack { offset, .. } => {
                    stack_base + offset - stack_offset_adjust
                }
                ParamClass::IntReg { .. } => {
                    // Regparm: param was stored to its alloca slot in emit_store_params.
                    // This should have been handled by the alloca_slot path above.
                    // If we get here, just use a fallback offset.
                    stack_base + (param_idx as i64) * 4
                }
                _ => stack_base + (param_idx as i64) * 4,
            }
        } else {
            stack_base + (param_idx as i64) * 4
        };

        if is_i128_type(ty) {
            if let Some(slot) = self.state.get_slot(dest.0) {
                for i in (0..16).step_by(4) {
                    let src_ref = self.param_ref(param_offset + i as i64);
                    let dst_ref = self.slot_ref_offset(slot, i as i64);
                    emit!(self.state, "    movl {}, %eax", src_ref);
                    emit!(self.state, "    movl %eax, {}", dst_ref);
                }
            }
        } else if ty == IrType::F128 {
            if let Some(dest_slot) = self.state.get_slot(dest.0) {
                let src_ref = self.param_ref(param_offset);
                let dst_ref = self.slot_ref(dest_slot);
                emit!(self.state, "    fldt {}", src_ref);
                emit!(self.state, "    fstpt {}", dst_ref);
                self.state.f128_direct_slots.insert(dest.0);
            }
        } else if ty == IrType::F64 || ty == IrType::I64 || ty == IrType::U64 {
            if let Some(slot) = self.state.get_slot(dest.0) {
                let src_ref = self.param_ref(param_offset);
                let dst_ref = self.slot_ref(slot);
                emit!(self.state, "    movl {}, %eax", src_ref);
                emit!(self.state, "    movl %eax, {}", dst_ref);
                let src_ref_hi = self.param_ref(param_offset + 4);
                let dst_ref_hi = self.slot_ref_offset(slot, 4);
                emit!(self.state, "    movl {}, %eax", src_ref_hi);
                emit!(self.state, "    movl %eax, {}", dst_ref_hi);
            }
        } else {
            let load_instr = self.mov_load_for_type(ty);
            let src_ref = self.param_ref(param_offset);
            emit!(self.state, "    {} {}, %eax", load_instr, src_ref);
            self.store_eax_to(dest);
        }
    }

    // ---- emit_epilogue_and_ret ----

    pub(super) fn emit_epilogue_and_ret_impl(&mut self, frame_size: i64) {
        self.emit_epilogue(frame_size);
        if self.state.uses_sret {
            self.state.emit("    ret $4");
        } else if self.is_fastcall && self.fastcall_stack_cleanup > 0 {
            emit!(self.state, "    ret ${}", self.fastcall_stack_cleanup);
        } else {
            self.state.emit("    ret");
        }
    }

    // ---- store/load instr for type ----

    pub(super) fn store_instr_for_type_impl(&self, ty: IrType) -> &'static str {
        self.mov_store_for_type(ty)
    }

    pub(super) fn load_instr_for_type_impl(&self, ty: IrType) -> &'static str {
        self.mov_load_for_type(ty)
    }
}
