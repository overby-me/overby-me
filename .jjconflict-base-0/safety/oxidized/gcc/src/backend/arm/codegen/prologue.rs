//! ArmCodegen: prologue/epilogue and stack frame operations.

use super::emit::{
    callee_saved_name, ArmCodegen, ARM_ARG_REGS, ARM_CALLEE_SAVED, ARM_CALLER_SAVED,
};
use crate::backend::call_abi::{classify_params, ParamClass};
use crate::backend::generation::{calculate_stack_space_common, find_param_alloca};
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrFunction, Value};

impl ArmCodegen {
    // ---- calculate_stack_space ----

    pub(super) fn calculate_stack_space_impl(&mut self, func: &IrFunction) -> i64 {
        use crate::backend::regalloc::PhysReg;
        use crate::ir::reexports::Instruction;

        let mut asm_clobbered_regs: Vec<PhysReg> = Vec::new();
        Self::prescan_inline_asm_callee_saved(func, &mut asm_clobbered_regs);
        let base_regs: &[PhysReg] = if func.is_variadic {
            &[]
        } else {
            &ARM_CALLEE_SAVED
        };
        let available_regs =
            crate::backend::generation::filter_available_regs(base_regs, &asm_clobbered_regs);

        let mut caller_saved_regs: Vec<PhysReg> = if func.is_variadic {
            Vec::new()
        } else {
            ARM_CALLER_SAVED.to_vec()
        };
        let mut has_f128_ops = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::BinOp { ty, .. }
                    | Instruction::UnaryOp { ty, .. }
                    | Instruction::Cmp { ty, .. }
                    | Instruction::Load { ty, .. }
                    | Instruction::Store { ty, .. }
                        if *ty == IrType::F128 =>
                    {
                        has_f128_ops = true;
                    }
                    Instruction::Cast { to_ty, .. } if *to_ty == IrType::F128 => {
                        has_f128_ops = true;
                    }
                    Instruction::Cast { from_ty, .. } if *from_ty == IrType::F128 => {
                        has_f128_ops = true;
                    }
                    _ => {}
                }
            }
        }
        if has_f128_ops {
            caller_saved_regs.clear();
        }

        let (reg_assigned, cached_liveness) =
            crate::backend::generation::run_regalloc_and_merge_clobbers(
                func,
                available_regs,
                caller_saved_regs,
                &asm_clobbered_regs,
                &mut self.reg_assignments,
                &mut self.used_callee_saved,
                false,
            );

        let mut space = calculate_stack_space_common(
            &mut self.state,
            func,
            16,
            |space, alloc_size, align| {
                let effective_align = if align > 0 { align.max(8) } else { 8 };
                let slot = (space + effective_align - 1) & !(effective_align - 1);
                let new_space = slot + ((alloc_size + 7) & !7).max(8);
                (slot, new_space)
            },
            &reg_assigned,
            &ARM_CALLEE_SAVED,
            cached_liveness,
            false,
        );

        if func.is_variadic {
            space = (space + 7) & !7;
            self.va_gp_save_offset = space;
            space += 64;

            if !self.general_regs_only {
                space = (space + 15) & !15;
                self.va_fp_save_offset = space;
                space += 128;
            }

            let config = self.call_abi_config_impl();
            let param_classes = crate::backend::call_abi::classify_params(func, &config);
            let mut named_gp = 0usize;
            let mut named_fp = 0usize;
            for (i, class) in param_classes.iter().enumerate() {
                // On ARM64, the sret pointer goes in x8 (a dedicated register),
                // NOT in x0-x7. Don't count it as consuming a GP argument register,
                // otherwise va_start computes the wrong __gr_offs and skips the
                // first variadic argument.
                if self.state.uses_sret && i == 0 {
                    continue;
                }
                named_gp += class.gp_reg_count();
                if matches!(
                    class,
                    crate::backend::call_abi::ParamClass::FloatReg { .. }
                        | crate::backend::call_abi::ParamClass::F128FpReg { .. }
                ) {
                    named_fp += 1;
                }
            }
            self.va_named_gp_count = named_gp.min(8);
            self.va_named_fp_count = named_fp.min(8);
            self.va_named_stack_bytes =
                crate::backend::call_abi::named_params_stack_bytes(&param_classes);
        }

        let save_count = self.used_callee_saved.len() as i64;
        if save_count > 0 {
            space = (space + 7) & !7;
            self.callee_save_offset = space;
            space += save_count * 8;
        }

        space
    }

    // ---- aligned_frame_size ----

    pub(super) fn aligned_frame_size_impl(&self, raw_space: i64) -> i64 {
        (raw_space + 15) & !15
    }

    // ---- emit_prologue ----

    pub(super) fn emit_prologue_impl(&mut self, func: &IrFunction, frame_size: i64) {
        self.current_return_type = func.return_type;
        self.current_frame_size = frame_size;
        self.frame_base_offset = None;
        self.emit_prologue_arm(frame_size);

        let used_regs = self.used_callee_saved.clone();
        let base = self.callee_save_offset;
        let n = used_regs.len();
        let mut i = 0;
        while i + 1 < n {
            let r1 = callee_saved_name(used_regs[i]);
            let r2 = callee_saved_name(used_regs[i + 1]);
            let offset = base + (i as i64) * 8;
            self.emit_stp_to_sp(r1, r2, offset);
            i += 2;
        }
        if i < n {
            let r = callee_saved_name(used_regs[i]);
            let offset = base + (i as i64) * 8;
            self.emit_store_to_sp(r, offset, "str");
        }
    }

    // ---- emit_epilogue ----

    pub(super) fn emit_epilogue_impl(&mut self, frame_size: i64) {
        self.emit_restore_callee_saved();
        self.emit_epilogue_arm(frame_size);
    }

    // ---- emit_store_params ----

    pub(super) fn emit_store_params_impl(&mut self, func: &IrFunction) {
        if func.is_variadic {
            self.emit_save_variadic_regs();
        }

        let config = self.call_abi_config_impl();
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

        // Pre-store optimization: when a GP param's alloca is dead (promoted by
        // mem2reg) but the ParamRef dest is register-allocated to a callee-saved
        // register, store the ABI arg register directly to that callee-saved
        // register in the prologue.  This is critical because:
        // 1. Dead alloca means no stack slot exists for this param
        // 2. The ABI register (x0-x7) will be clobbered by subsequent codegen
        //    (ARM uses x0 as the universal scratch/result register)
        // 3. We must save the value NOW, before any other code runs
        // 4. emit_param_ref will see param_pre_stored and skip code generation
        let sret_shift = if self.state.uses_sret { 1usize } else { 0 };
        let mut paramref_dests: Vec<Option<Value>> = vec![None; func.params.len()];
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
        // Build a map from physical register -> list of param indices that use it,
        // so we can detect when two params share the same callee-saved register.
        let mut reg_to_params: crate::common::fx_hash::FxHashMap<u8, Vec<usize>> =
            crate::common::fx_hash::FxHashMap::default();
        for (i, _) in func.params.iter().enumerate() {
            if let Some(paramref_dest) = paramref_dests[i] {
                if let Some(&phys_reg) = self.reg_assignments.get(&paramref_dest.0) {
                    reg_to_params.entry(phys_reg.0).or_default().push(i);
                }
            }
        }

        for (i, _) in func.params.iter().enumerate() {
            let class = param_classes[i];
            if !class.uses_gp_reg() {
                continue;
            }
            // Skip params that have an alloca slot (they'll be handled by emit_store_gp_params)
            let has_slot = self
                .state
                .param_alloca_slots
                .get(i)
                .and_then(|opt| opt.as_ref())
                .is_some();
            if has_slot {
                continue;
            }

            if let Some(paramref_dest) = paramref_dests[i] {
                if let Some(&phys_reg) = self.reg_assignments.get(&paramref_dest.0) {
                    // Only pre-store to callee-saved registers (x20-x28).
                    // Caller-saved registers (x13, x14) cannot be used because
                    // they may overlap with scratch registers.
                    let is_callee_saved = phys_reg.0 >= 20 && phys_reg.0 <= 28;
                    if is_callee_saved {
                        // Safety check: if another param's dest is also assigned
                        // to this register, skip pre-store to avoid conflicts.
                        // The register allocator may assign the same register to
                        // two params whose live ranges don't overlap, but pre-store
                        // extends the effective lifetime to function entry.
                        if let Some(users) = reg_to_params.get(&phys_reg.0) {
                            if users.len() > 1 {
                                continue;
                            }
                        }
                        let dest_reg = callee_saved_name(phys_reg);
                        if let ParamClass::IntReg { reg_idx } = class {
                            let actual_idx = if sret_shift > 0 && reg_idx == 0 && i == 0 {
                                // sret: the pointer comes in x8
                                self.state
                                    .emit_fmt(format_args!("    mov {}, x8", dest_reg));
                                self.state.param_pre_stored.insert(i);
                                continue;
                            } else if reg_idx >= sret_shift {
                                reg_idx - sret_shift
                            } else {
                                reg_idx
                            };
                            let src_reg = ARM_ARG_REGS[actual_idx];
                            self.state
                                .emit_fmt(format_args!("    mov {}, {}", dest_reg, src_reg));
                            self.state.param_pre_stored.insert(i);
                        }
                    }
                }
            }
        }

        self.emit_store_gp_params(func, &param_classes);
        self.emit_store_fp_params(func, &param_classes);
        self.emit_store_stack_params(func, &param_classes);
    }

    // ---- emit_param_ref ----

    pub(super) fn emit_param_ref_impl(&mut self, dest: &Value, param_idx: usize, ty: IrType) {
        if param_idx >= self.state.param_classes.len() {
            return;
        }

        // If this param was pre-stored directly to its register-allocated
        // destination during emit_store_params, the value is already in place.
        // No code needs to be emitted — the register already holds the value.
        if self.state.param_pre_stored.contains(&param_idx) {
            return;
        }

        if param_idx < self.state.param_alloca_slots.len() {
            if let Some((slot, alloca_ty)) = self.state.param_alloca_slots[param_idx] {
                let ldr_instr = self.load_instr_for_type_impl(alloca_ty);
                let (actual_instr, reg) = Self::arm_parse_load(ldr_instr);
                self.emit_load_from_sp(reg, slot.0, actual_instr);
                self.store_x0_to(dest);
                return;
            }
        }

        let class = self.state.param_classes[param_idx];
        let frame_size = self.current_frame_size;

        // AArch64 ABI: sret shifts GP register indices
        let sret_shift = if self.state.uses_sret { 1usize } else { 0 };

        match class {
            ParamClass::IntReg { reg_idx } => {
                let actual_reg = if sret_shift > 0 && reg_idx == 0 && param_idx == 0 {
                    Self::reg_for_type("x8", ty)
                } else {
                    let actual_idx = if reg_idx >= sret_shift {
                        reg_idx - sret_shift
                    } else {
                        reg_idx
                    };
                    Self::reg_for_type(ARM_ARG_REGS[actual_idx], ty)
                };
                let dst = Self::reg_for_type("x0", ty);
                if actual_reg != dst {
                    self.state
                        .emit_fmt(format_args!("    mov {}, {}", dst, actual_reg));
                }
                self.store_x0_to(dest);
            }
            ParamClass::FloatReg { reg_idx } => {
                if ty == IrType::F32 {
                    self.state
                        .emit_fmt(format_args!("    fmov w0, s{}", reg_idx));
                } else {
                    self.state
                        .emit_fmt(format_args!("    fmov x0, d{}", reg_idx));
                }
                self.store_x0_to(dest);
            }
            ParamClass::StackScalar { offset } => {
                let src = frame_size + offset;
                let ldr_instr = self.load_instr_for_type_impl(ty);
                let (actual_instr, reg) = Self::arm_parse_load(ldr_instr);
                self.emit_load_from_sp(reg, src, actual_instr);
                self.store_x0_to(dest);
            }
            _ => {}
        }
    }

    // ---- emit_epilogue_and_ret ----

    pub(super) fn emit_epilogue_and_ret_impl(&mut self, frame_size: i64) {
        self.emit_restore_callee_saved();
        self.emit_epilogue_arm(frame_size);
        self.state.emit("    ret");
    }

    // ---- store_instr_for_type / load_instr_for_type ----

    pub(super) fn store_instr_for_type_impl(&self, ty: IrType) -> &'static str {
        Self::str_for_type(ty)
    }

    pub(super) fn load_instr_for_type_impl(&self, ty: IrType) -> &'static str {
        match ty {
            IrType::I8 => "ldrsb",
            IrType::U8 => "ldrb",
            IrType::I16 => "ldrsh",
            IrType::U16 => "ldrh",
            IrType::I32 => "ldrsw",
            IrType::U32 | IrType::F32 => "ldr32",
            _ => "ldr64",
        }
    }
}
