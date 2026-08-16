//! RiscvCodegen: prologue/epilogue and stack frame operations.

use super::emit::{
    callee_saved_name, collect_inline_asm_callee_saved_riscv, RiscvCodegen, CALL_TEMP_CALLEE_SAVED,
    RISCV_ARG_REGS, RISCV_CALLEE_SAVED,
};
use crate::backend::call_abi::{classify_params, ParamClass};
use crate::backend::generation::{calculate_stack_space_common, find_param_alloca};
use crate::common::types::IrType;
use crate::ir::reexports::IrFunction;

impl RiscvCodegen {
    // ---- calculate_stack_space ----

    pub(super) fn calculate_stack_space_impl(&mut self, func: &IrFunction) -> i64 {
        // For variadic functions, count the actual GP registers used by named
        // parameters. A struct that occupies 2 GP regs counts as 2, not 1.
        // This is critical for va_start to correctly point to the first variadic arg.
        if func.is_variadic {
            // For variadic callee, call_abi_config() already has variadic_floats_in_gp: true.
            let classification =
                crate::backend::call_abi::classify_params_full(func, &self.call_abi_config_impl());
            // Use the effective GP register index (includes alignment gaps for I128/F128 pairs)
            // rather than summing gp_reg_count(), so we correctly skip over alignment padding.
            self.va_named_gp_count = classification.int_reg_idx.min(8);
            // Track stack bytes consumed by named params that overflowed to the caller's stack.
            self.va_named_stack_bytes =
                crate::backend::call_abi::named_params_stack_bytes(&classification.classes);
            self.is_variadic = true;
        } else {
            self.is_variadic = false;
        }

        // Run register allocator BEFORE stack space computation so we can
        // skip allocating stack slots for values assigned to registers.
        //
        // For functions with inline asm, collect callee-saved registers that are
        // clobbered or used as explicit constraints, then filter them from the
        // allocation pool. This allows register allocation to proceed using the
        // remaining registers, rather than disabling it entirely. Many kernel
        // functions contain inline asm from inlined spin_lock/spin_unlock; without
        // this, they get no register allocation and enormous stack frames (4KB+),
        // causing kernel stack overflows.
        let mut asm_clobbered_regs: Vec<crate::backend::regalloc::PhysReg> = Vec::new();
        collect_inline_asm_callee_saved_riscv(func, &mut asm_clobbered_regs);

        // Build the full set of available callee-saved registers.
        // All of s1-s11 are available for register allocation since call argument
        // staging now uses only caller-saved t3/t4/t5 (no s-register staging).
        let mut all_available: Vec<crate::backend::regalloc::PhysReg> = RISCV_CALLEE_SAVED.to_vec();
        for &reg in CALL_TEMP_CALLEE_SAVED.iter() {
            all_available.push(reg);
        }

        let available_regs =
            crate::backend::generation::filter_available_regs(&all_available, &asm_clobbered_regs);
        let (reg_assigned, cached_liveness) =
            crate::backend::generation::run_regalloc_and_merge_clobbers(
                func,
                available_regs,
                Vec::new(),
                &asm_clobbered_regs,
                &mut self.reg_assignments,
                &mut self.used_callee_saved,
                true, // RISC-V asm emitter checks reg_assignments for inline asm operands
            );

        let space = calculate_stack_space_common(
            &mut self.state,
            func,
            16,
            |space, alloc_size, align| {
                // RISC-V uses negative offsets from s0 (frame pointer)
                // Honor alignment: round up space to alignment boundary before allocating
                let effective_align = if align > 0 { align.max(8) } else { 8 };
                let alloc = ((alloc_size + 7) & !7).max(8);
                let new_space =
                    ((space + alloc + effective_align - 1) / effective_align) * effective_align;
                (-new_space, new_space)
            },
            &reg_assigned,
            &RISCV_CALLEE_SAVED,
            cached_liveness,
            true,
        );

        // Add space for saving callee-saved registers.
        // Each callee-saved register needs 8 bytes on the stack.
        let callee_save_space = (self.used_callee_saved.len() as i64) * 8;
        space + callee_save_space
    }

    // ---- aligned_frame_size ----

    pub(super) fn aligned_frame_size_impl(&self, raw_space: i64) -> i64 {
        (raw_space + 15) & !15
    }

    // ---- emit_prologue ----

    pub(super) fn emit_prologue_impl(&mut self, func: &IrFunction, frame_size: i64) {
        self.current_return_type = func.return_type;
        self.current_frame_size = frame_size;
        self.emit_prologue_riscv(frame_size);

        // Save callee-saved registers used by the register allocator.
        // They are saved at the bottom of the frame (highest negative offsets from s0).
        let used_regs = self.used_callee_saved.clone();
        for (i, &reg) in used_regs.iter().enumerate() {
            let offset = -frame_size + (i as i64 * 8);
            let reg_name = callee_saved_name(reg);
            self.emit_store_to_s0(reg_name, offset, "sd");
        }
    }

    // ---- emit_epilogue ----

    pub(super) fn emit_epilogue_impl(&mut self, frame_size: i64) {
        // Restore callee-saved registers before epilogue.
        let used_regs = self.used_callee_saved.clone();
        for (i, &reg) in used_regs.iter().enumerate() {
            let offset = -frame_size + (i as i64 * 8);
            let reg_name = callee_saved_name(reg);
            self.emit_load_from_s0(reg_name, offset, "ld");
        }
        self.emit_epilogue_riscv(frame_size);
    }

    // ---- emit_store_params ----

    pub(super) fn emit_store_params_impl(&mut self, func: &IrFunction) {
        let float_arg_regs = ["fa0", "fa1", "fa2", "fa3", "fa4", "fa5", "fa6", "fa7"];

        // For variadic functions: save all integer register args (a0-a7) to the
        // register save area at POSITIVE offsets from s0.
        if func.is_variadic {
            for i in 0..8usize {
                self.emit_store_to_s0(RISCV_ARG_REGS[i], (i as i64) * 8, "sd");
            }
        }

        // Use shared parameter classification. On RISC-V, variadic functions
        // route float params through GP regs; non-variadic use FP regs.
        let mut config = self.call_abi_config_impl();
        config.variadic_floats_in_gp = func.is_variadic;
        let param_classes = classify_params(func, &config);
        // Save param classes for ParamRef instructions
        self.state.param_classes = param_classes.clone();
        self.state.num_params = func.params.len();
        self.state.func_is_variadic = func.is_variadic;

        // Pre-compute param alloca slots for emit_param_ref
        self.state.param_alloca_slots = (0..func.params.len())
            .map(|i| {
                find_param_alloca(func, i)
                    .and_then(|(dest, ty)| self.state.get_slot(dest.0).map(|slot| (slot, ty)))
            })
            .collect();

        // Stack-passed params are at positive s0 offsets.
        // For variadic: register save area occupies s0+0..s0+56, stack params at s0+64+.
        let stack_base: i64 = if func.is_variadic { 64 } else { 0 };

        // Check if any F128 params exist (need to save all regs before __trunctfdf2).
        let has_f128_reg_params = param_classes
            .iter()
            .any(|c| matches!(c, ParamClass::F128GpPair { .. }));
        let f128_save_offset: i64 = if has_f128_reg_params && !func.is_variadic {
            self.state.emit("    addi sp, sp, -128");
            for i in 0..8usize {
                self.state
                    .emit_fmt(format_args!("    sd {}, {}(sp)", RISCV_ARG_REGS[i], i * 8));
            }
            for i in 0..8usize {
                self.state
                    .emit_fmt(format_args!("    fsd fa{}, {}(sp)", i, 64 + i * 8));
            }
            0i64
        } else {
            0
        };

        for (i, _param) in func.params.iter().enumerate() {
            let class = param_classes[i];

            let (slot, ty) = match find_param_alloca(func, i) {
                Some((dest, ty)) => match self.state.get_slot(dest.0) {
                    Some(slot) => (slot, ty),
                    None => continue,
                },
                None => continue,
            };

            match class {
                ParamClass::IntReg { reg_idx } => {
                    // GP register param - extend sub-64-bit values to full
                    // 64 bits and store with sd so the entire 8-byte slot is
                    // valid for subsequent ld loads (e.g. from Copy values
                    // sharing the slot via slot coalescing).
                    if has_f128_reg_params && !func.is_variadic {
                        let off = f128_save_offset + (reg_idx as i64) * 8;
                        let load_instr = Self::load_for_type(ty);
                        self.state
                            .emit_fmt(format_args!("    {} t0, {}(sp)", load_instr, off));
                        self.emit_store_to_s0("t0", slot.0, "sd");
                    } else if func.is_variadic {
                        // For variadic, load from save area with extending load.
                        let save_off = (reg_idx as i64) * 8;
                        let load_instr = Self::load_for_type(ty);
                        self.emit_load_from_s0("t0", save_off, load_instr);
                        self.emit_store_to_s0("t0", slot.0, "sd");
                    } else {
                        Self::emit_extend_reg(&mut self.state, RISCV_ARG_REGS[reg_idx], "t0", ty);
                        self.emit_store_to_s0("t0", slot.0, "sd");
                    }
                }
                ParamClass::FloatReg { reg_idx } => {
                    if has_f128_reg_params && !func.is_variadic {
                        // FP regs were saved to stack; load from save area.
                        let fp_off = f128_save_offset + 64 + (reg_idx as i64) * 8;
                        if ty == IrType::F32 {
                            self.state
                                .emit_fmt(format_args!("    flw ft0, {}(sp)", fp_off));
                            self.state.emit("    fmv.x.w t0, ft0");
                        } else {
                            self.state
                                .emit_fmt(format_args!("    fld ft0, {}(sp)", fp_off));
                            self.state.emit("    fmv.x.d t0, ft0");
                        }
                    } else if ty == IrType::F32 {
                        self.state
                            .emit_fmt(format_args!("    fmv.x.w t0, {}", float_arg_regs[reg_idx]));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    fmv.x.d t0, {}", float_arg_regs[reg_idx]));
                    }
                    self.emit_store_to_s0("t0", slot.0, "sd");
                }
                ParamClass::I128RegPair { base_reg_idx } => {
                    if func.is_variadic {
                        let lo_off = (base_reg_idx as i64) * 8;
                        let hi_off = ((base_reg_idx + 1) as i64) * 8;
                        self.emit_load_from_s0("t0", lo_off, "ld");
                        self.emit_store_to_s0("t0", slot.0, "sd");
                        self.emit_load_from_s0("t0", hi_off, "ld");
                        self.emit_store_to_s0("t0", slot.0 + 8, "sd");
                    } else if has_f128_reg_params {
                        let lo_off = f128_save_offset + (base_reg_idx as i64) * 8;
                        let hi_off = f128_save_offset + ((base_reg_idx + 1) as i64) * 8;
                        self.state
                            .emit_fmt(format_args!("    ld t0, {}(sp)", lo_off));
                        self.emit_store_to_s0("t0", slot.0, "sd");
                        self.state
                            .emit_fmt(format_args!("    ld t0, {}(sp)", hi_off));
                        self.emit_store_to_s0("t0", slot.0 + 8, "sd");
                    } else {
                        self.emit_store_to_s0(RISCV_ARG_REGS[base_reg_idx], slot.0, "sd");
                        self.emit_store_to_s0(RISCV_ARG_REGS[base_reg_idx + 1], slot.0 + 8, "sd");
                    }
                }
                ParamClass::StructByValReg { base_reg_idx, size } => {
                    if has_f128_reg_params && !func.is_variadic {
                        let lo_off = f128_save_offset + (base_reg_idx as i64) * 8;
                        self.state
                            .emit_fmt(format_args!("    ld t0, {}(sp)", lo_off));
                        self.emit_store_to_s0("t0", slot.0, "sd");
                        if size > 8 {
                            let hi_off = f128_save_offset + ((base_reg_idx + 1) as i64) * 8;
                            self.state
                                .emit_fmt(format_args!("    ld t0, {}(sp)", hi_off));
                            self.emit_store_to_s0("t0", slot.0 + 8, "sd");
                        }
                    } else if func.is_variadic {
                        let lo_off = (base_reg_idx as i64) * 8;
                        self.emit_load_from_s0("t0", lo_off, "ld");
                        self.emit_store_to_s0("t0", slot.0, "sd");
                        if size > 8 {
                            let hi_off = ((base_reg_idx + 1) as i64) * 8;
                            self.emit_load_from_s0("t0", hi_off, "ld");
                            self.emit_store_to_s0("t0", slot.0 + 8, "sd");
                        }
                    } else {
                        self.emit_store_to_s0(RISCV_ARG_REGS[base_reg_idx], slot.0, "sd");
                        if size > 8 {
                            self.emit_store_to_s0(
                                RISCV_ARG_REGS[base_reg_idx + 1],
                                slot.0 + 8,
                                "sd",
                            );
                        }
                    }
                }
                ParamClass::F128GpPair {
                    lo_reg_idx,
                    hi_reg_idx,
                } => {
                    // F128 in GP register pair: store full 16-byte f128 directly to alloca.
                    // This preserves quad precision (e.g., LDBL_MIN != 0).
                    if func.is_variadic {
                        let lo_off = (lo_reg_idx as i64) * 8;
                        let hi_off = (hi_reg_idx as i64) * 8;
                        self.emit_load_from_s0("t0", lo_off, "ld");
                        self.emit_store_to_s0("t0", slot.0, "sd");
                        self.emit_load_from_s0("t0", hi_off, "ld");
                        self.emit_store_to_s0("t0", slot.0 + 8, "sd");
                    } else {
                        // Load from F128 save area.
                        let lo_off = f128_save_offset + (lo_reg_idx as i64) * 8;
                        let hi_off = f128_save_offset + (hi_reg_idx as i64) * 8;
                        self.state
                            .emit_fmt(format_args!("    ld t0, {}(sp)", lo_off));
                        self.emit_store_to_s0("t0", slot.0, "sd");
                        self.state
                            .emit_fmt(format_args!("    ld t0, {}(sp)", hi_off));
                        self.emit_store_to_s0("t0", slot.0 + 8, "sd");
                    }
                }
                ParamClass::F128Stack { offset } | ParamClass::F128AlwaysStack { offset } => {
                    // F128 from stack: store full 16-byte f128 directly to alloca.
                    let src = stack_base + offset;
                    self.emit_load_from_s0("t0", src, "ld");
                    self.emit_store_to_s0("t0", slot.0, "sd");
                    self.emit_load_from_s0("t0", src + 8, "ld");
                    self.emit_store_to_s0("t0", slot.0 + 8, "sd");
                }
                ParamClass::I128Stack { offset } => {
                    let src = stack_base + offset;
                    self.emit_load_from_s0("t0", src, "ld");
                    self.emit_store_to_s0("t0", slot.0, "sd");
                    self.emit_load_from_s0("t0", src + 8, "ld");
                    self.emit_store_to_s0("t0", slot.0 + 8, "sd");
                }
                ParamClass::StackScalar { offset } => {
                    // Load with extending load so the full 8-byte dest
                    // slot is valid for subsequent ld loads.
                    let src = stack_base + offset;
                    let load_instr = Self::load_for_type(ty);
                    self.emit_load_from_s0("t0", src, load_instr);
                    self.emit_store_to_s0("t0", slot.0, "sd");
                }
                ParamClass::StructStack { offset, size }
                | ParamClass::LargeStructStack { offset, size } => {
                    let src = stack_base + offset;
                    let n_dwords = size.div_ceil(8);
                    for qi in 0..n_dwords {
                        let src_off = src + (qi as i64 * 8);
                        let dst_off = slot.0 + (qi as i64 * 8);
                        self.emit_load_from_s0("t0", src_off, "ld");
                        self.emit_store_to_s0("t0", dst_off, "sd");
                    }
                }
                ParamClass::LargeStructByRefReg { reg_idx, size } => {
                    // RISC-V LP64: register holds a pointer to the struct data.
                    // Copy size bytes from the pointer into the local alloca.
                    let src_reg = if has_f128_reg_params && !func.is_variadic {
                        let off = f128_save_offset + (reg_idx as i64) * 8;
                        self.state.emit_fmt(format_args!("    ld t0, {}(sp)", off));
                        "t0"
                    } else if func.is_variadic {
                        let off = (reg_idx as i64) * 8;
                        self.emit_load_from_s0("t0", off, "ld");
                        "t0"
                    } else {
                        RISCV_ARG_REGS[reg_idx]
                    };
                    // t1 = pointer to struct data
                    if src_reg != "t1" {
                        self.state.emit_fmt(format_args!("    mv t1, {}", src_reg));
                    }
                    // Large structs (>2048 bytes) have dword offsets exceeding
                    // RISC-V's 12-bit signed immediate; emit_load_from_reg handles this.
                    let n_dwords = size.div_ceil(8);
                    for qi in 0..n_dwords {
                        let src_off = (qi * 8) as i64;
                        let dst_off = slot.0 + src_off;
                        Self::emit_load_from_reg(&mut self.state, "t0", "t1", src_off, "ld");
                        self.emit_store_to_s0("t0", dst_off, "sd");
                    }
                }
                ParamClass::LargeStructByRefStack { offset, size } => {
                    // RISC-V LP64: stack slot holds a pointer to the struct data.
                    // Load the pointer, then copy the struct data into the alloca.
                    let caller_offset = stack_base + offset;
                    self.emit_load_from_s0("t1", caller_offset, "ld");
                    let n_dwords = size.div_ceil(8);
                    for qi in 0..n_dwords {
                        let src_off = (qi * 8) as i64;
                        let dst_off = slot.0 + src_off;
                        Self::emit_load_from_reg(&mut self.state, "t0", "t1", src_off, "ld");
                        self.emit_store_to_s0("t0", dst_off, "sd");
                    }
                }
                ParamClass::StructSplitRegStack {
                    reg_idx,
                    stack_offset,
                    size,
                } => {
                    // RISC-V psABI: first 8 bytes in GP register, remaining bytes on the stack.
                    if func.is_variadic {
                        // Variadic: load first half from register save area
                        let off = (reg_idx as i64) * 8;
                        self.emit_load_from_s0("t0", off, "ld");
                        self.emit_store_to_s0("t0", slot.0, "sd");
                    } else if has_f128_reg_params {
                        let off = f128_save_offset + (reg_idx as i64) * 8;
                        self.state.emit_fmt(format_args!("    ld t0, {}(sp)", off));
                        self.emit_store_to_s0("t0", slot.0, "sd");
                    } else {
                        self.emit_store_to_s0(RISCV_ARG_REGS[reg_idx], slot.0, "sd");
                    }
                    // Second half from the caller's stack
                    let src = stack_base + stack_offset;
                    let remaining = size - 8;
                    let n_dwords = remaining.div_ceil(8);
                    for qi in 0..n_dwords {
                        let src_off = src + (qi as i64 * 8);
                        let dst_off = slot.0 + 8 + (qi as i64 * 8);
                        self.emit_load_from_s0("t0", src_off, "ld");
                        self.emit_store_to_s0("t0", dst_off, "sd");
                    }
                }
                // F128 in FP reg doesn't happen on RISC-V.
                ParamClass::F128FpReg { .. } | ParamClass::ZeroSizeSkip => {}

                // RISC-V LP64D: struct with float/double fields passed in FP registers.
                ParamClass::StructSseReg {
                    lo_fp_idx,
                    hi_fp_idx,
                    size,
                } => {
                    let float_arg_regs_inner =
                        ["fa0", "fa1", "fa2", "fa3", "fa4", "fa5", "fa6", "fa7"];
                    // Get the RISC-V float class to determine field sizes
                    let rv_class = func.params[i].riscv_float_class;
                    let (lo_is_double, hi_is_double) = match rv_class {
                        Some(crate::common::types::RiscvFloatClass::OneFloat { is_double }) => {
                            (is_double, false)
                        }
                        Some(crate::common::types::RiscvFloatClass::TwoFloats {
                            lo_is_double,
                            hi_is_double,
                        }) => (lo_is_double, hi_is_double),
                        _ => (size > 4, true), // fallback guess
                    };
                    // Store first float/double field
                    if lo_is_double {
                        self.state.emit_fmt(format_args!(
                            "    fsd {}, {}(s0)",
                            float_arg_regs_inner[lo_fp_idx], slot.0
                        ));
                    } else {
                        self.state.emit_fmt(format_args!(
                            "    fsw {}, {}(s0)",
                            float_arg_regs_inner[lo_fp_idx], slot.0
                        ));
                    }
                    // Store second float/double field (if present)
                    if let Some(hi_idx) = hi_fp_idx {
                        let hi_offset = if lo_is_double { 8 } else { 4 };
                        if hi_is_double {
                            self.state.emit_fmt(format_args!(
                                "    fsd {}, {}(s0)",
                                float_arg_regs_inner[hi_idx],
                                slot.0 + hi_offset
                            ));
                        } else {
                            self.state.emit_fmt(format_args!(
                                "    fsw {}, {}(s0)",
                                float_arg_regs_inner[hi_idx],
                                slot.0 + hi_offset
                            ));
                        }
                    }
                }
                ParamClass::StructMixedIntSseReg {
                    int_reg_idx,
                    fp_reg_idx,
                    size: _,
                } => {
                    // Integer first, float second (in memory layout)
                    let float_arg_regs_inner =
                        ["fa0", "fa1", "fa2", "fa3", "fa4", "fa5", "fa6", "fa7"];
                    let rv_class = func.params[i].riscv_float_class;
                    let (float_is_double, int_size, float_offset) = match rv_class {
                        Some(crate::common::types::RiscvFloatClass::IntAndFloat {
                            float_is_double,
                            int_size,
                            float_offset,
                            ..
                        }) => (float_is_double, int_size, float_offset),
                        _ => (true, 8, 8), // fallback
                    };
                    // Store integer part at beginning of struct
                    let int_store = if int_size <= 4 { "sw" } else { "sd" };
                    self.state.emit_fmt(format_args!(
                        "    {} {}, {}(s0)",
                        int_store, RISCV_ARG_REGS[int_reg_idx], slot.0
                    ));
                    // Store float part at its offset
                    if float_is_double {
                        self.state.emit_fmt(format_args!(
                            "    fsd {}, {}(s0)",
                            float_arg_regs_inner[fp_reg_idx],
                            slot.0 + float_offset as i64
                        ));
                    } else {
                        self.state.emit_fmt(format_args!(
                            "    fsw {}, {}(s0)",
                            float_arg_regs_inner[fp_reg_idx],
                            slot.0 + float_offset as i64
                        ));
                    }
                }
                ParamClass::StructMixedSseIntReg {
                    fp_reg_idx,
                    int_reg_idx,
                    size: _,
                } => {
                    // Float first, integer second (in memory layout)
                    let float_arg_regs_inner =
                        ["fa0", "fa1", "fa2", "fa3", "fa4", "fa5", "fa6", "fa7"];
                    let rv_class = func.params[i].riscv_float_class;
                    let (float_is_double, int_offset, int_size) = match rv_class {
                        Some(crate::common::types::RiscvFloatClass::FloatAndInt {
                            float_is_double,
                            int_offset,
                            int_size,
                            ..
                        }) => (float_is_double, int_offset, int_size),
                        _ => (true, 8, 8), // fallback
                    };
                    // Store float part at beginning of struct
                    if float_is_double {
                        self.state.emit_fmt(format_args!(
                            "    fsd {}, {}(s0)",
                            float_arg_regs_inner[fp_reg_idx], slot.0
                        ));
                    } else {
                        self.state.emit_fmt(format_args!(
                            "    fsw {}, {}(s0)",
                            float_arg_regs_inner[fp_reg_idx], slot.0
                        ));
                    }
                    // Store integer part at its offset
                    let int_store = if int_size <= 4 { "sw" } else { "sd" };
                    self.state.emit_fmt(format_args!(
                        "    {} {}, {}(s0)",
                        int_store,
                        RISCV_ARG_REGS[int_reg_idx],
                        slot.0 + int_offset as i64
                    ));
                }
            }
        }

        // Clean up the F128 save area.
        if has_f128_reg_params && !func.is_variadic {
            self.state.emit("    addi sp, sp, 128");
        }
    }

    /// Sign/zero-extend a GP register value to 64 bits for sub-64-bit types.
    /// For 64-bit or larger types, just moves src to dest.
    fn emit_extend_reg(
        state: &mut crate::backend::state::CodegenState,
        src: &str,
        dest: &str,
        ty: IrType,
    ) {
        match ty {
            IrType::I8 => {
                if src != dest {
                    state.emit_fmt(format_args!("    mv {}, {}", dest, src));
                }
                state.emit_fmt(format_args!("    slli {}, {}, 56", dest, dest));
                state.emit_fmt(format_args!("    srai {}, {}, 56", dest, dest));
            }
            IrType::U8 => {
                state.emit_fmt(format_args!("    andi {}, {}, 0xff", dest, src));
            }
            IrType::I16 => {
                if src != dest {
                    state.emit_fmt(format_args!("    mv {}, {}", dest, src));
                }
                state.emit_fmt(format_args!("    slli {}, {}, 48", dest, dest));
                state.emit_fmt(format_args!("    srai {}, {}, 48", dest, dest));
            }
            IrType::U16 => {
                if src != dest {
                    state.emit_fmt(format_args!("    mv {}, {}", dest, src));
                }
                state.emit_fmt(format_args!("    slli {}, {}, 48", dest, dest));
                state.emit_fmt(format_args!("    srli {}, {}, 48", dest, dest));
            }
            IrType::I32 => {
                state.emit_fmt(format_args!("    sext.w {}, {}", dest, src));
            }
            IrType::U32 | IrType::F32 => {
                if src != dest {
                    state.emit_fmt(format_args!("    mv {}, {}", dest, src));
                }
                state.emit_fmt(format_args!("    slli {}, {}, 32", dest, dest));
                state.emit_fmt(format_args!("    srli {}, {}, 32", dest, dest));
            }
            _ => {
                if src != dest {
                    state.emit_fmt(format_args!("    mv {}, {}", dest, src));
                }
            }
        }
    }

    // ---- emit_param_ref ----

    pub(super) fn emit_param_ref_impl(
        &mut self,
        dest: &crate::ir::reexports::Value,
        param_idx: usize,
        ty: IrType,
    ) {
        if param_idx >= self.state.param_classes.len() {
            return;
        }

        // Prefer loading from the param alloca slot (where emit_store_params
        // already saved the incoming register value). This avoids issues where
        // ABI registers get clobbered during emit_store_params' processing.
        if param_idx < self.state.param_alloca_slots.len() {
            if let Some((slot, alloca_ty)) = self.state.param_alloca_slots[param_idx] {
                let load_instr = Self::load_for_type(alloca_ty);
                self.emit_load_from_s0("t0", slot.0, load_instr);
                self.store_t0_to(dest);
                return;
            }
        }

        // Fallback: read directly from ABI location
        let class = self.state.param_classes[param_idx];
        let stack_base: i64 = if self.state.func_is_variadic { 64 } else { 0 };

        match class {
            ParamClass::IntReg { reg_idx } => {
                self.state
                    .emit_fmt(format_args!("    mv t0, {}", RISCV_ARG_REGS[reg_idx]));
                self.store_t0_to(dest);
            }
            ParamClass::FloatReg { reg_idx } => {
                let float_arg_regs = ["fa0", "fa1", "fa2", "fa3", "fa4", "fa5", "fa6", "fa7"];
                if ty == IrType::F32 {
                    self.state
                        .emit_fmt(format_args!("    fmv.x.w t0, {}", float_arg_regs[reg_idx]));
                } else {
                    self.state
                        .emit_fmt(format_args!("    fmv.x.d t0, {}", float_arg_regs[reg_idx]));
                }
                self.store_t0_to(dest);
            }
            ParamClass::StackScalar { offset } => {
                let src = stack_base + offset;
                let load_instr = Self::load_for_type(ty);
                self.emit_load_from_s0("t0", src, load_instr);
                self.store_t0_to(dest);
            }
            // Struct/i128/F128 params remain in allocas
            _ => {}
        }
    }

    // ---- emit_epilogue_and_ret ----

    pub(super) fn emit_epilogue_and_ret_impl(&mut self, frame_size: i64) {
        // Restore callee-saved registers before frame teardown.
        let used_regs = self.used_callee_saved.clone();
        for (i, &reg) in used_regs.iter().enumerate() {
            let offset = -frame_size + (i as i64 * 8);
            let reg_name = callee_saved_name(reg);
            self.emit_load_from_s0(reg_name, offset, "ld");
        }
        self.emit_epilogue_riscv(frame_size);
        self.state.emit("    ret");
    }

    // ---- store_instr_for_type / load_instr_for_type ----

    pub(super) fn store_instr_for_type_impl(&self, ty: IrType) -> &'static str {
        Self::store_for_type(ty)
    }

    pub(super) fn load_instr_for_type_impl(&self, ty: IrType) -> &'static str {
        Self::load_for_type(ty)
    }

    // ---- Internal prologue/epilogue helpers ----

    /// Emit prologue: allocate stack and save ra/s0.
    ///
    /// Stack layout (s0 points to top of frame = old sp):
    ///   s0 - 8:  saved ra
    ///   s0 - 16: saved s0
    ///   s0 - 16 - ...: local data (allocas and value slots)
    ///   sp: bottom of frame
    pub(super) fn emit_prologue_riscv(&mut self, frame_size: i64) {
        // For variadic functions, the register save area (64 bytes for a0-a7) is
        // placed ABOVE s0, contiguous with the caller's stack-passed arguments.
        // Layout: s0+0..s0+56 = a0..a7, s0+64+ = caller stack args.
        // This means total_alloc = frame_size + 64 for variadic, but s0 = sp + frame_size.
        let total_alloc = if self.is_variadic {
            frame_size + 64
        } else {
            frame_size
        };

        const PAGE_SIZE: i64 = 4096;

        // Small-frame path requires ALL immediates to fit in 12 bits:
        // -total_alloc (sp adjust), and frame_size (s0 setup).
        if Self::fits_imm12(-total_alloc) && Self::fits_imm12(total_alloc) {
            // Small frame: all offsets fit in 12-bit immediates
            self.state
                .emit_fmt(format_args!("    addi sp, sp, -{}", total_alloc));
            // ra and s0 are saved relative to s0, which is sp + frame_size
            // (NOT sp + total_alloc for variadic functions!)
            self.state
                .emit_fmt(format_args!("    sd ra, {}(sp)", frame_size - 8));
            self.state
                .emit_fmt(format_args!("    sd s0, {}(sp)", frame_size - 16));
            self.state
                .emit_fmt(format_args!("    addi s0, sp, {}", frame_size));
            if self.state.emit_cfi {
                self.state.emit_fmt(format_args!("    .cfi_def_cfa s0, 0"));
                self.state.emit("    .cfi_offset ra, -8");
                self.state.emit("    .cfi_offset s0, -16");
            }
        } else if total_alloc > PAGE_SIZE {
            // Stack probing: for large frames, touch each page so the kernel
            // can grow the stack mapping. Without this, a single large sub
            // can skip guard pages and cause a segfault.
            let probe_label = self.state.fresh_label("stack_probe");
            self.state
                .emit_fmt(format_args!("    li t1, {}", total_alloc));
            self.state
                .emit_fmt(format_args!("    li t2, {}", PAGE_SIZE));
            self.state.emit_fmt(format_args!("{}:", probe_label));
            self.state.emit("    sub sp, sp, t2");
            self.state.emit("    sd zero, 0(sp)");
            self.state.emit("    sub t1, t1, t2");
            self.state
                .emit_fmt(format_args!("    bgt t1, t2, {}", probe_label));
            self.state.emit("    sub sp, sp, t1");
            self.state.emit("    sd zero, 0(sp)");
            // Compute s0 = sp + frame_size (NOT total_alloc)
            self.state
                .emit_fmt(format_args!("    li t0, {}", frame_size));
            self.state.emit("    add t0, sp, t0");
            // Save ra and old s0 at s0-8, s0-16
            self.state.emit("    sd ra, -8(t0)");
            self.state.emit("    sd s0, -16(t0)");
            self.state.emit("    mv s0, t0");
            if self.state.emit_cfi {
                self.state.emit("    .cfi_def_cfa s0, 0");
                self.state.emit("    .cfi_offset ra, -8");
                self.state.emit("    .cfi_offset s0, -16");
            }
        } else {
            // Large frame: use t0 for offsets
            self.state
                .emit_fmt(format_args!("    li t0, {}", total_alloc));
            self.state.emit("    sub sp, sp, t0");
            // Compute s0 = sp + frame_size (NOT total_alloc)
            self.state
                .emit_fmt(format_args!("    li t0, {}", frame_size));
            self.state.emit("    add t0, sp, t0");
            // Save ra and old s0 at s0-8, s0-16
            self.state.emit("    sd ra, -8(t0)");
            self.state.emit("    sd s0, -16(t0)");
            self.state.emit("    mv s0, t0");
            if self.state.emit_cfi {
                self.state.emit("    .cfi_def_cfa s0, 0");
                self.state.emit("    .cfi_offset ra, -8");
                self.state.emit("    .cfi_offset s0, -16");
            }
        }
    }

    /// Emit epilogue: restore ra/s0 and deallocate stack.
    pub(super) fn emit_epilogue_riscv(&mut self, frame_size: i64) {
        let total_alloc = if self.is_variadic {
            frame_size + 64
        } else {
            frame_size
        };
        // When DynAlloca is used, SP was modified at runtime, so we must restore
        // from s0 (frame pointer) rather than using SP-relative offsets.
        if !self.state.has_dyn_alloca
            && Self::fits_imm12(-total_alloc)
            && Self::fits_imm12(total_alloc)
        {
            // Small frame: restore from known sp offsets
            // ra/s0 saved at sp + frame_size - 8/16 (relative to current sp)
            self.state
                .emit_fmt(format_args!("    ld ra, {}(sp)", frame_size - 8));
            self.state
                .emit_fmt(format_args!("    ld s0, {}(sp)", frame_size - 16));
            self.state
                .emit_fmt(format_args!("    addi sp, sp, {}", total_alloc));
        } else {
            // Large frame or DynAlloca: restore from s0-relative offsets (always fit in imm12).
            self.state.emit("    ld ra, -8(s0)");
            self.state.emit("    ld t0, -16(s0)");
            // For variadic functions, s0 + 64 = old_sp, so sp = s0 + 64
            if self.is_variadic {
                self.state.emit("    addi sp, s0, 64");
            } else {
                self.state.emit("    mv sp, s0");
            }
            self.state.emit("    mv s0, t0");
        }
    }
}
