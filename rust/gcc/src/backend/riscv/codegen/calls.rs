//! RiscvCodegen: call ABI operations.

use super::emit::{callee_saved_name, RiscvCodegen, RISCV_ARG_REGS};
use crate::backend::call_abi::{compute_stack_arg_space, CallAbiConfig, CallArgClass};
use crate::backend::generation::is_i128_type;
use crate::common::types::IrType;
use crate::ir::reexports::{IrConst, Operand, Value};

impl RiscvCodegen {
    pub(super) fn call_abi_config_impl(&self) -> CallAbiConfig {
        CallAbiConfig {
            max_int_regs: 8,
            max_float_regs: 8,
            align_i128_pairs: true,
            f128_in_fp_regs: false,
            f128_in_gp_pairs: true,
            variadic_floats_in_gp: true,
            large_struct_by_ref: true,
            use_sysv_struct_classification: false,
            use_riscv_float_struct_classification: true,
            allow_struct_split_reg_stack: true,
            align_struct_pairs: true,
            sret_uses_dedicated_reg: false,
        }
    }

    pub(super) fn emit_call_compute_stack_space_impl(
        &self,
        arg_classes: &[CallArgClass],
        _arg_types: &[IrType],
    ) -> usize {
        compute_stack_arg_space(arg_classes)
    }

    pub(super) fn emit_call_f128_pre_convert_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[CallArgClass],
        _arg_types: &[IrType],
        _stack_arg_space: usize,
    ) -> usize {
        let mut f128_temp_space: i64 = 0;
        for (i, arg) in args.iter().enumerate() {
            if matches!(arg_classes[i], CallArgClass::F128Reg { .. }) {
                if let Operand::Value(_) = arg {
                    f128_temp_space += 16;
                }
            }
        }
        if f128_temp_space > 0 {
            self.emit_addi_sp(-f128_temp_space);
            let mut temp_offset: i64 = 0;
            for (i, arg) in args.iter().enumerate() {
                if !matches!(arg_classes[i], CallArgClass::F128Reg { .. }) {
                    continue;
                }
                if let Operand::Value(_) = arg {
                    self.emit_f128_operand_to_a0_a1(arg);
                    self.emit_store_to_sp("a0", temp_offset, "sd");
                    self.emit_store_to_sp("a1", temp_offset + 8, "sd");
                    temp_offset += 16;
                }
            }
        }
        f128_temp_space as usize
    }

    pub(super) fn emit_call_stack_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[CallArgClass],
        _arg_types: &[IrType],
        stack_arg_space: usize,
        _fptr_spill: usize,
        _f128_temp_space: usize,
    ) -> i64 {
        if stack_arg_space > 0 {
            self.emit_addi_sp(-(stack_arg_space as i64));
            let mut offset: usize = 0;
            for (arg_i, arg) in args.iter().enumerate() {
                if !arg_classes[arg_i].is_stack() {
                    continue;
                }
                match arg_classes[arg_i] {
                    CallArgClass::StructByValStack { size }
                    | CallArgClass::LargeStructStack { size } => {
                        let n_dwords = size.div_ceil(8);
                        match arg {
                            Operand::Value(v) => {
                                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                                    let reg_name = callee_saved_name(reg);
                                    self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
                                } else if let Some(slot) = self.state.get_slot(v.0) {
                                    if self.state.is_alloca(v.0) {
                                        self.emit_alloca_addr("t0", v.0, slot.0);
                                    } else {
                                        self.emit_load_from_s0("t0", slot.0, "ld");
                                    }
                                } else {
                                    self.state.emit("    li t0, 0");
                                }
                            }
                            Operand::Const(_) => {
                                self.operand_to_t0(arg);
                            }
                        }
                        for qi in 0..n_dwords {
                            let src_off = (qi * 8) as i64;
                            Self::emit_load_from_reg(&mut self.state, "t1", "t0", src_off, "ld");
                            self.emit_store_to_sp("t1", offset as i64 + src_off, "sd");
                        }
                        offset += n_dwords * 8;
                    }
                    CallArgClass::I128Stack => {
                        offset = (offset + 15) & !15;
                        match arg {
                            Operand::Const(c) => {
                                if let IrConst::I128(v) = c {
                                    self.state
                                        .emit_fmt(format_args!("    li t0, {}", *v as u64 as i64));
                                    self.emit_store_to_sp("t0", offset as i64, "sd");
                                    self.state.emit_fmt(format_args!(
                                        "    li t0, {}",
                                        (*v >> 64) as u64 as i64
                                    ));
                                    self.emit_store_to_sp("t0", (offset + 8) as i64, "sd");
                                } else {
                                    self.operand_to_t0(arg);
                                    self.emit_store_to_sp("t0", offset as i64, "sd");
                                    self.emit_store_to_sp("zero", (offset + 8) as i64, "sd");
                                }
                            }
                            Operand::Value(v) => {
                                if let Some(slot) = self.state.get_slot(v.0) {
                                    if self.state.is_alloca(v.0) {
                                        self.emit_alloca_addr("t0", v.0, slot.0);
                                        self.emit_store_to_sp("t0", offset as i64, "sd");
                                        self.emit_store_to_sp("zero", (offset + 8) as i64, "sd");
                                    } else {
                                        self.emit_load_from_s0("t0", slot.0, "ld");
                                        self.emit_store_to_sp("t0", offset as i64, "sd");
                                        self.emit_load_from_s0("t0", slot.0 + 8, "ld");
                                        self.emit_store_to_sp("t0", (offset + 8) as i64, "sd");
                                    }
                                }
                            }
                        }
                        offset += 16;
                    }
                    CallArgClass::F128Stack => {
                        offset = (offset + 15) & !15;
                        match arg {
                            Operand::Const(ref c) => {
                                let bytes = match c {
                                    IrConst::LongDouble(_, f128_bytes) => *f128_bytes,
                                    _ => {
                                        let f64_val = c.to_f64().unwrap_or(0.0);
                                        crate::ir::reexports::f64_to_f128_bytes(f64_val)
                                    }
                                };
                                let lo = i64::from_le_bytes(bytes[0..8].try_into().unwrap());
                                let hi = i64::from_le_bytes(bytes[8..16].try_into().unwrap());
                                self.state.emit_fmt(format_args!("    li t0, {}", lo));
                                self.emit_store_to_sp("t0", offset as i64, "sd");
                                self.state.emit_fmt(format_args!("    li t0, {}", hi));
                                self.emit_store_to_sp("t0", (offset + 8) as i64, "sd");
                            }
                            Operand::Value(_) => {
                                self.emit_f128_operand_to_a0_a1(arg);
                                self.emit_store_to_sp("a0", offset as i64, "sd");
                                self.emit_store_to_sp("a1", (offset + 8) as i64, "sd");
                            }
                        }
                        offset += 16;
                    }
                    CallArgClass::StructSplitRegStack { size, .. } => {
                        match arg {
                            Operand::Value(v) => {
                                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                                    let reg_name = callee_saved_name(reg);
                                    self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
                                } else if let Some(slot) = self.state.get_slot(v.0) {
                                    if self.state.is_alloca(v.0) {
                                        self.emit_alloca_addr("t0", v.0, slot.0);
                                    } else {
                                        self.emit_load_from_s0("t0", slot.0, "ld");
                                    }
                                } else {
                                    self.state.emit("    li t0, 0");
                                }
                            }
                            Operand::Const(_) => {
                                self.operand_to_t0(arg);
                            }
                        }
                        let stack_dwords = (size - 8).div_ceil(8);
                        for qi in 0..stack_dwords {
                            let src_off = 8 + (qi * 8) as i64;
                            Self::emit_load_from_reg(&mut self.state, "t1", "t0", src_off, "ld");
                            self.emit_store_to_sp("t1", offset as i64 + (qi * 8) as i64, "sd");
                        }
                        offset += stack_dwords * 8;
                    }
                    CallArgClass::Stack => {
                        self.operand_to_t0(arg);
                        self.emit_store_to_sp("t0", offset as i64, "sd");
                        offset += 8;
                    }
                    _ => {}
                }
            }
        }
        0 // RISC-V loads operands from s0-relative slots (not SP-relative), so no SP adjust needed
    }

    pub(super) fn emit_call_reg_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[CallArgClass],
        arg_types: &[IrType],
        _total_sp_adjust: i64,
        _f128_temp_space: usize,
        stack_arg_space: usize,
        struct_arg_riscv_float_classes: &[Option<crate::common::types::RiscvFloatClass>],
    ) {
        let float_arg_regs = ["fa0", "fa1", "fa2", "fa3", "fa4", "fa5", "fa6", "fa7"];

        // GP arg staging strategy: use only caller-saved temp registers (t3, t4, t5)
        // for staging, avoiding callee-saved s2-s6. This allows s2-s6 to be used
        // by the register allocator, significantly reducing stack frame sizes.
        //
        // Phase 1: Stage the first 3 GP args into t3/t4/t5, handle float args.
        // Phase 2: Move staged values to target a-registers.
        // Phase 3: Load remaining GP args (4th+) directly into target a-registers.
        //
        // Phase 3 is safe because operand_to_t0 only reads from stack slots or
        // callee-saved registers (never from a-registers), so writing to a-registers
        // in Phase 2 doesn't conflict with loading sources in Phase 3.
        let temp_regs = ["t3", "t4", "t5"];
        let mut staged_gp: Vec<(usize, &str)> = Vec::new(); // (target_reg_idx, temp_reg)
        let mut deferred_gp: Vec<(usize, usize)> = Vec::new(); // (target_reg_idx, arg_index)
        let mut temp_i = 0usize;
        for (i, arg) in args.iter().enumerate() {
            match arg_classes[i] {
                CallArgClass::IntReg { reg_idx } => {
                    if temp_i < temp_regs.len() {
                        // Stage into t3/t4/t5
                        self.operand_to_t0(arg);
                        self.state
                            .emit_fmt(format_args!("    mv {}, t0", temp_regs[temp_i]));
                        staged_gp.push((reg_idx, temp_regs[temp_i]));
                        temp_i += 1;
                    } else {
                        // Defer: will load directly into target a-register after Phase 2
                        deferred_gp.push((reg_idx, i));
                    }
                }
                CallArgClass::FloatReg { reg_idx } => {
                    self.operand_to_t0(arg);
                    let arg_ty = if i < arg_types.len() {
                        Some(arg_types[i])
                    } else {
                        None
                    };
                    if arg_ty == Some(IrType::F32) {
                        self.state
                            .emit_fmt(format_args!("    fmv.w.x {}, t0", float_arg_regs[reg_idx]));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    fmv.d.x {}, t0", float_arg_regs[reg_idx]));
                    }
                }
                _ => {}
            }
        }

        // Phase 2: Move staged values from t3/t4/t5 to target a-registers
        for (target_idx, temp_reg) in &staged_gp {
            self.state.emit_fmt(format_args!(
                "    mv {}, {}",
                RISCV_ARG_REGS[*target_idx], temp_reg
            ));
        }

        // Phase 3: Load deferred GP args directly into target a-registers.
        // This is safe because operand_to_t0 reads from s0-relative stack slots
        // or callee-saved s-registers, never from a-registers. Invalidate the
        // accumulator cache first since Phase 2 emissions may have left stale
        // cache entries.
        if !deferred_gp.is_empty() {
            self.state.reg_cache.invalidate_acc();
        }
        for &(target_idx, arg_idx) in &deferred_gp {
            self.operand_to_t0(&args[arg_idx]);
            self.state
                .emit_fmt(format_args!("    mv {}, t0", RISCV_ARG_REGS[target_idx]));
        }

        // Load F128 register pair args
        let mut f128_var_temp_offset: i64 = 0;
        for (i, arg) in args.iter().enumerate() {
            if let CallArgClass::F128Reg { reg_idx: base_reg } = arg_classes[i] {
                match arg {
                    Operand::Const(ref c) => {
                        let bytes = match c {
                            IrConst::LongDouble(_, f128_bytes) => *f128_bytes,
                            _ => {
                                let f64_val = c.to_f64().unwrap_or(0.0);
                                crate::ir::reexports::f64_to_f128_bytes(f64_val)
                            }
                        };
                        let lo = i64::from_le_bytes(bytes[0..8].try_into().unwrap());
                        let hi = i64::from_le_bytes(bytes[8..16].try_into().unwrap());
                        self.state
                            .emit_fmt(format_args!("    li {}, {}", RISCV_ARG_REGS[base_reg], lo));
                        self.state.emit_fmt(format_args!(
                            "    li {}, {}",
                            RISCV_ARG_REGS[base_reg + 1],
                            hi
                        ));
                    }
                    Operand::Value(_) => {
                        let offset = f128_var_temp_offset + stack_arg_space as i64;
                        self.emit_load_from_sp(RISCV_ARG_REGS[base_reg], offset, "ld");
                        self.emit_load_from_sp(RISCV_ARG_REGS[base_reg + 1], offset + 8, "ld");
                        f128_var_temp_offset += 16;
                    }
                }
            }
        }

        // Load i128 register pair args
        for (i, arg) in args.iter().enumerate() {
            if let CallArgClass::I128RegPair { base_reg_idx } = arg_classes[i] {
                match arg {
                    Operand::Const(c) => {
                        if let IrConst::I128(v) = c {
                            self.state.emit_fmt(format_args!(
                                "    li {}, {}",
                                RISCV_ARG_REGS[base_reg_idx], *v as u64 as i64
                            ));
                            self.state.emit_fmt(format_args!(
                                "    li {}, {}",
                                RISCV_ARG_REGS[base_reg_idx + 1],
                                (*v >> 64) as u64 as i64
                            ));
                        } else {
                            self.operand_to_t0(arg);
                            self.state.emit_fmt(format_args!(
                                "    mv {}, t0",
                                RISCV_ARG_REGS[base_reg_idx]
                            ));
                            self.state.emit_fmt(format_args!(
                                "    mv {}, zero",
                                RISCV_ARG_REGS[base_reg_idx + 1]
                            ));
                        }
                    }
                    Operand::Value(v) => {
                        if let Some(slot) = self.state.get_slot(v.0) {
                            if self.state.is_alloca(v.0) {
                                self.emit_alloca_addr(RISCV_ARG_REGS[base_reg_idx], v.0, slot.0);
                                self.state.emit_fmt(format_args!(
                                    "    mv {}, zero",
                                    RISCV_ARG_REGS[base_reg_idx + 1]
                                ));
                            } else {
                                self.emit_load_from_s0(RISCV_ARG_REGS[base_reg_idx], slot.0, "ld");
                                self.emit_load_from_s0(
                                    RISCV_ARG_REGS[base_reg_idx + 1],
                                    slot.0 + 8,
                                    "ld",
                                );
                            }
                        }
                    }
                }
            }
        }

        // Load struct-by-value register args
        for (i, arg) in args.iter().enumerate() {
            if let CallArgClass::StructByValReg { base_reg_idx, size } = arg_classes[i] {
                let regs_needed = if size <= 8 { 1 } else { 2 };
                match arg {
                    Operand::Value(v) => {
                        if let Some(&reg) = self.reg_assignments.get(&v.0) {
                            let reg_name = callee_saved_name(reg);
                            self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
                        } else if let Some(slot) = self.state.get_slot(v.0) {
                            if self.state.is_alloca(v.0) {
                                self.emit_alloca_addr("t0", v.0, slot.0);
                            } else {
                                self.emit_load_from_s0("t0", slot.0, "ld");
                            }
                        } else {
                            self.state.emit("    li t0, 0");
                        }
                    }
                    Operand::Const(_) => {
                        self.operand_to_t0(arg);
                    }
                }
                self.state.emit_fmt(format_args!(
                    "    ld {}, 0(t0)",
                    RISCV_ARG_REGS[base_reg_idx]
                ));
                if regs_needed > 1 {
                    self.state.emit_fmt(format_args!(
                        "    ld {}, 8(t0)",
                        RISCV_ARG_REGS[base_reg_idx + 1]
                    ));
                }
            }
        }

        // Load split struct register args
        for (i, arg) in args.iter().enumerate() {
            if let CallArgClass::StructSplitRegStack { reg_idx, .. } = arg_classes[i] {
                match arg {
                    Operand::Value(v) => {
                        if let Some(&reg) = self.reg_assignments.get(&v.0) {
                            let reg_name = callee_saved_name(reg);
                            self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
                        } else if let Some(slot) = self.state.get_slot(v.0) {
                            if self.state.is_alloca(v.0) {
                                self.emit_alloca_addr("t0", v.0, slot.0);
                            } else {
                                self.emit_load_from_s0("t0", slot.0, "ld");
                            }
                        } else {
                            self.state.emit("    li t0, 0");
                        }
                    }
                    Operand::Const(_) => {
                        self.operand_to_t0(arg);
                    }
                }
                self.state
                    .emit_fmt(format_args!("    ld {}, 0(t0)", RISCV_ARG_REGS[reg_idx]));
            }
        }

        // Load RISC-V LP64D float struct args into FP registers
        for (i, arg) in args.iter().enumerate() {
            let needs_ptr = matches!(
                arg_classes[i],
                CallArgClass::StructSseReg { .. }
                    | CallArgClass::StructMixedIntSseReg { .. }
                    | CallArgClass::StructMixedSseIntReg { .. }
            );
            if !needs_ptr {
                continue;
            }

            let rv_class = struct_arg_riscv_float_classes.get(i).copied().flatten();

            match arg {
                Operand::Value(v) => {
                    if let Some(&reg) = self.reg_assignments.get(&v.0) {
                        let reg_name = callee_saved_name(reg);
                        self.state.emit_fmt(format_args!("    mv t0, {}", reg_name));
                    } else if let Some(slot) = self.state.get_slot(v.0) {
                        if self.state.is_alloca(v.0) {
                            self.emit_addi_s0("t0", slot.0);
                        } else {
                            self.emit_load_from_s0("t0", slot.0, "ld");
                        }
                    } else {
                        self.state.emit("    li t0, 0");
                    }
                }
                Operand::Const(_) => {
                    self.operand_to_t0(arg);
                }
            }

            match arg_classes[i] {
                CallArgClass::StructSseReg {
                    lo_fp_idx,
                    hi_fp_idx,
                    size,
                } => {
                    let (lo_is_double, hi_is_double) = match rv_class {
                        Some(crate::common::types::RiscvFloatClass::OneFloat { is_double }) => {
                            (is_double, false)
                        }
                        Some(crate::common::types::RiscvFloatClass::TwoFloats {
                            lo_is_double,
                            hi_is_double,
                        }) => (lo_is_double, hi_is_double),
                        _ => (size > 4, true),
                    };
                    if lo_is_double {
                        self.state
                            .emit_fmt(format_args!("    fld {}, 0(t0)", float_arg_regs[lo_fp_idx]));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    flw {}, 0(t0)", float_arg_regs[lo_fp_idx]));
                    }
                    if let Some(hi_idx) = hi_fp_idx {
                        let hi_offset = if lo_is_double { 8 } else { 4 };
                        if hi_is_double {
                            self.state.emit_fmt(format_args!(
                                "    fld {}, {}(t0)",
                                float_arg_regs[hi_idx], hi_offset
                            ));
                        } else {
                            self.state.emit_fmt(format_args!(
                                "    flw {}, {}(t0)",
                                float_arg_regs[hi_idx], hi_offset
                            ));
                        }
                    }
                }
                CallArgClass::StructMixedIntSseReg {
                    int_reg_idx,
                    fp_reg_idx,
                    size: _,
                } => {
                    let (float_is_double, int_size, float_offset) = match rv_class {
                        Some(crate::common::types::RiscvFloatClass::IntAndFloat {
                            float_is_double,
                            int_size,
                            float_offset,
                            ..
                        }) => (float_is_double, int_size, float_offset),
                        _ => (true, 8, 8),
                    };
                    let int_load = if int_size <= 4 { "lw" } else { "ld" };
                    self.state.emit_fmt(format_args!(
                        "    {} {}, 0(t0)",
                        int_load, RISCV_ARG_REGS[int_reg_idx]
                    ));
                    if float_is_double {
                        self.state.emit_fmt(format_args!(
                            "    fld {}, {}(t0)",
                            float_arg_regs[fp_reg_idx], float_offset
                        ));
                    } else {
                        self.state.emit_fmt(format_args!(
                            "    flw {}, {}(t0)",
                            float_arg_regs[fp_reg_idx], float_offset
                        ));
                    }
                }
                CallArgClass::StructMixedSseIntReg {
                    fp_reg_idx,
                    int_reg_idx,
                    size: _,
                } => {
                    let (float_is_double, int_offset, int_size) = match rv_class {
                        Some(crate::common::types::RiscvFloatClass::FloatAndInt {
                            float_is_double,
                            int_offset,
                            int_size,
                            ..
                        }) => (float_is_double, int_offset, int_size),
                        _ => (true, 8, 8),
                    };
                    if float_is_double {
                        self.state.emit_fmt(format_args!(
                            "    fld {}, 0(t0)",
                            float_arg_regs[fp_reg_idx]
                        ));
                    } else {
                        self.state.emit_fmt(format_args!(
                            "    flw {}, 0(t0)",
                            float_arg_regs[fp_reg_idx]
                        ));
                    }
                    let int_load = if int_size <= 4 { "lw" } else { "ld" };
                    self.state.emit_fmt(format_args!(
                        "    {} {}, {}(t0)",
                        int_load, RISCV_ARG_REGS[int_reg_idx], int_offset
                    ));
                }
                _ => {}
            }
        }
    }

    pub(super) fn emit_call_instruction_impl(
        &mut self,
        direct_name: Option<&str>,
        func_ptr: Option<&Operand>,
        _indirect: bool,
        _stack_arg_space: usize,
    ) {
        if let Some(name) = direct_name {
            self.state.emit_fmt(format_args!("    call {}", name));
        } else if let Some(ptr) = func_ptr {
            self.operand_to_t0(ptr);
            self.state.emit("    mv t2, t0");
            self.state.emit("    jalr ra, t2, 0");
        }
    }

    pub(super) fn emit_call_cleanup_impl(
        &mut self,
        stack_arg_space: usize,
        f128_temp_space: usize,
        _indirect: bool,
    ) {
        if f128_temp_space > 0 && stack_arg_space == 0 {
            self.emit_addi_sp(f128_temp_space as i64);
        }
        if stack_arg_space > 0 {
            let cleanup = stack_arg_space as i64 + f128_temp_space as i64;
            self.emit_addi_sp(cleanup);
        }
    }

    pub(super) fn emit_call_store_result_impl(&mut self, dest: &Value, return_type: IrType) {
        if is_i128_type(return_type) {
            if let Some(slot) = self.state.get_slot(dest.0) {
                self.emit_store_to_s0("a0", slot.0, "sd");
                self.emit_store_to_s0("a1", slot.0 + 8, "sd");
            }
        } else if return_type.is_long_double() {
            if let Some(slot) = self.state.get_slot(dest.0) {
                self.emit_store_to_s0("a0", slot.0, "sd");
                self.emit_store_to_s0("a1", slot.0 + 8, "sd");
            }
            self.state.emit("    call __trunctfdf2");
            self.state.emit("    fmv.x.d t0, fa0");
            self.state.reg_cache.invalidate_all();
            self.state.track_f128_self(dest.0);
            if let Some(&reg) = self.reg_assignments.get(&dest.0) {
                let reg_name = callee_saved_name(reg);
                self.state.emit_fmt(format_args!("    mv {}, t0", reg_name));
            }
        } else if return_type == IrType::F32 {
            if let Some(slot) = self.state.get_slot(dest.0) {
                self.state.emit("    fmv.x.w t0, fa0");
                self.emit_store_to_s0("t0", slot.0, "sd");
            }
        } else if return_type.is_float() {
            if let Some(slot) = self.state.get_slot(dest.0) {
                self.state.emit("    fmv.x.d t0, fa0");
                self.emit_store_to_s0("t0", slot.0, "sd");
            }
        } else if let Some(&reg) = self.reg_assignments.get(&dest.0) {
            let reg_name = callee_saved_name(reg);
            self.state.emit_fmt(format_args!("    mv {}, a0", reg_name));
        } else if let Some(slot) = self.state.get_slot(dest.0) {
            self.emit_store_to_s0("a0", slot.0, "sd");
        }
    }
}
