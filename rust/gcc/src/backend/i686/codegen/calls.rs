//! I686Codegen: function call operations (cdecl calling convention).

use super::emit::I686Codegen;
use crate::backend::call_abi;
use crate::backend::generation::is_i128_type;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{Operand, Value};

impl I686Codegen {
    pub(super) fn call_abi_config_impl(&self) -> call_abi::CallAbiConfig {
        call_abi::CallAbiConfig {
            max_int_regs: self.regparm as usize,
            max_float_regs: 0,
            align_i128_pairs: false,
            f128_in_fp_regs: false,
            f128_in_gp_pairs: false,
            variadic_floats_in_gp: false,
            large_struct_by_ref: false,
            use_sysv_struct_classification: false,
            use_riscv_float_struct_classification: false,
            allow_struct_split_reg_stack: false,
            align_struct_pairs: false,
            sret_uses_dedicated_reg: false,
        }
    }

    pub(super) fn emit_call_compute_stack_space_impl(
        &self,
        arg_classes: &[call_abi::CallArgClass],
        arg_types: &[IrType],
    ) -> usize {
        let mut total = 0;
        for (i, ac) in arg_classes.iter().enumerate() {
            let ty = if i < arg_types.len() {
                arg_types[i]
            } else {
                IrType::I32
            };
            match ac {
                call_abi::CallArgClass::Stack => match ty {
                    IrType::F64 | IrType::I64 | IrType::U64 => total += 8,
                    _ => total += 4,
                },
                call_abi::CallArgClass::F128Stack => total += 12,
                call_abi::CallArgClass::I128Stack => total += 16,
                call_abi::CallArgClass::StructByValStack { size } => total += (*size + 3) & !3,
                call_abi::CallArgClass::LargeStructStack { size } => total += (*size + 3) & !3,
                call_abi::CallArgClass::ZeroSizeSkip => {}
                call_abi::CallArgClass::IntReg { .. } => {} // regparm: in register, no stack space
                _ => total += 4,
            }
        }
        (total + 15) & !15
    }

    pub(super) fn emit_call_f128_pre_convert_impl(
        &mut self,
        _args: &[Operand],
        _arg_classes: &[call_abi::CallArgClass],
        _arg_types: &[IrType],
        _stack_arg_space: usize,
    ) -> usize {
        0 // No F128 pre-conversion needed on i686
    }

    pub(super) fn emit_call_stack_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[call_abi::CallArgClass],
        arg_types: &[IrType],
        stack_arg_space: usize,
        _fptr_spill: usize,
        _f128_temp_space: usize,
    ) -> i64 {
        if stack_arg_space > 0 {
            emit!(self.state, "    subl ${}, %esp", stack_arg_space);
            self.esp_adjust += stack_arg_space as i64;
        }

        let mut stack_offset: usize = 0;
        for (i, ac) in arg_classes.iter().enumerate() {
            match ac {
                call_abi::CallArgClass::I128Stack => {
                    self.emit_call_i128_stack_arg(&args[i], stack_offset);
                    stack_offset += 16;
                }
                call_abi::CallArgClass::F128Stack => {
                    self.emit_call_f128_stack_arg(&args[i], stack_offset);
                    stack_offset += 12;
                }
                call_abi::CallArgClass::StructByValStack { size }
                | call_abi::CallArgClass::LargeStructStack { size } => {
                    let sz = *size;
                    self.emit_call_struct_stack_arg(&args[i], stack_offset, sz);
                    stack_offset += (sz + 3) & !3;
                }
                call_abi::CallArgClass::Stack => {
                    let ty = arg_types[i];
                    if ty == IrType::F64 || ty == IrType::I64 || ty == IrType::U64 {
                        self.emit_call_8byte_stack_arg(&args[i], ty, stack_offset);
                        stack_offset += 8;
                    } else {
                        self.operand_to_eax(&args[i]);
                        emit!(self.state, "    movl %eax, {}(%esp)", stack_offset);
                        stack_offset += 4;
                    }
                }
                call_abi::CallArgClass::ZeroSizeSkip => {}
                call_abi::CallArgClass::IntReg { .. } => {} // regparm: handled in emit_call_reg_args
                _ => {
                    self.operand_to_eax(&args[i]);
                    emit!(self.state, "    movl %eax, {}(%esp)", stack_offset);
                    stack_offset += 4;
                }
            }
        }

        stack_arg_space as i64
    }

    pub(super) fn emit_call_reg_args_impl(
        &mut self,
        args: &[Operand],
        arg_classes: &[call_abi::CallArgClass],
        _arg_types: &[IrType],
        _total_sp_adjust: i64,
        _f128_temp_space: usize,
        _stack_arg_space: usize,
        _struct_arg_riscv_float_classes: &[Option<crate::common::types::RiscvFloatClass>],
    ) {
        if self.regparm == 0 {
            return; // cdecl: no register args
        }
        // regparm register order: EAX (reg_idx 0), EDX (reg_idx 1), ECX (reg_idx 2).
        // We must load args into registers in reverse order to avoid clobbering
        // EAX (the accumulator) before we're done using it to load other values.
        // Collect register args first, then emit in reverse order.
        let regparm_regs: &[&str] = &["%eax", "%edx", "%ecx"];
        let mut reg_args: Vec<(usize, usize)> = Vec::new(); // (arg_idx, reg_idx)
        for (i, ac) in arg_classes.iter().enumerate() {
            if let call_abi::CallArgClass::IntReg { reg_idx } = ac {
                reg_args.push((i, *reg_idx));
            }
        }
        // Emit in reverse order so we load into edx/ecx before eax
        // (since operand_to_eax uses eax as accumulator).
        for &(arg_i, reg_idx) in reg_args.iter().rev() {
            if reg_idx < regparm_regs.len() {
                let dest_reg = regparm_regs[reg_idx];
                if dest_reg == "%eax" {
                    self.operand_to_eax(&args[arg_i]);
                } else {
                    self.operand_to_eax(&args[arg_i]);
                    emit!(self.state, "    movl %eax, {}", dest_reg);
                    self.state.reg_cache.invalidate_acc();
                }
            }
        }
    }

    pub(super) fn emit_call_instruction_impl(
        &mut self,
        direct_name: Option<&str>,
        func_ptr: Option<&Operand>,
        indirect: bool,
        _stack_arg_space: usize,
    ) {
        if let Some(name) = direct_name {
            if self.state.needs_plt(name) {
                emit!(self.state, "    call {}@PLT", name);
            } else {
                emit!(self.state, "    call {}", name);
            }
        } else if indirect {
            if let Some(fptr) = func_ptr {
                self.operand_to_eax(fptr);
            }
            self.state.emit("    call *%eax");
        }
    }

    pub(super) fn emit_call_cleanup_impl(
        &mut self,
        stack_arg_space: usize,
        _f128_temp_space: usize,
        _indirect: bool,
    ) {
        if stack_arg_space > 0 {
            emit!(self.state, "    addl ${}, %esp", stack_arg_space);
            self.esp_adjust -= stack_arg_space as i64;
        }
    }

    pub(super) fn emit_call_store_result_impl(&mut self, dest: &Value, return_type: IrType) {
        if return_type == IrType::I64 || return_type == IrType::U64 {
            if let Some(slot) = self.state.get_slot(dest.0) {
                let sr0 = self.slot_ref(slot);
                let sr4 = self.slot_ref_offset(slot, 4);
                emit!(self.state, "    movl %eax, {}", sr0);
                emit!(self.state, "    movl %edx, {}", sr4);
            }
            self.state.reg_cache.invalidate_acc();
        } else if is_i128_type(return_type) {
            self.emit_call_store_i128_result(dest);
        } else if return_type.is_long_double() {
            self.emit_call_store_f128_result(dest);
        } else if return_type == IrType::F32 {
            self.emit_call_move_f32_to_acc();
            self.emit_store_result(dest);
        } else if return_type == IrType::F64 {
            self.emit_f64_store_from_x87(dest);
            self.state.reg_cache.invalidate_acc();
        } else {
            self.emit_store_result(dest);
        }
    }

    pub(super) fn emit_call_store_i128_result_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr0 = self.slot_ref(slot);
            let sr4 = self.slot_ref_offset(slot, 4);
            emit!(self.state, "    movl %eax, {}", sr0);
            emit!(self.state, "    movl %edx, {}", sr4);
        }
    }

    pub(super) fn emit_call_store_f128_result_impl(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    fstpt {}", sr);
            self.state.f128_direct_slots.insert(dest.0);
        }
    }

    pub(super) fn emit_call_move_f32_to_acc_impl(&mut self) {
        self.state.emit("    subl $4, %esp");
        self.state.emit("    fstps (%esp)");
        self.state.emit("    movl (%esp), %eax");
        self.state.emit("    addl $4, %esp");
    }

    pub(super) fn emit_call_move_f64_to_acc_impl(&mut self) {
        self.state.emit("    subl $8, %esp");
        self.state.emit("    fstpl (%esp)");
        self.state.emit("    movl (%esp), %eax");
        self.state.emit("    addl $8, %esp");
    }
}
