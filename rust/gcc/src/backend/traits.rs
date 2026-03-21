//! ArchCodegen trait: the interface each backend implements.
//!
//! This trait defines ~185 methods that each architecture must implement to provide
//! its specific register names, instruction mnemonics, and ABI details. The trait
//! also provides ~64 default method implementations that capture shared codegen
//! patterns (store/load dispatch, cast handling, i128 operations, control flow).
//!
//! The default implementations are built from small "primitive" methods that each
//! backend overrides with 1-4 line arch-specific implementations. This design lets
//! the shared framework express the algorithm once while backends only provide the
//! instruction-level differences.
//!
//! ## `delegate_to_impl!` macro
//!
//! Most backend `impl ArchCodegen for XxxCodegen` blocks consist of one-liner
//! delegations: `fn foo(&mut self, x: T) { self.foo_impl(x) }`. The
//! `delegate_to_impl!` macro eliminates this boilerplate. Each backend lists
//! the method signatures it delegates, and the macro generates the forwarding
//! body by appending `_impl` to the method name.

/// Generates `ArchCodegen` trait method implementations that delegate to a
/// corresponding `_impl` method on `self`. This eliminates the repetitive
/// one-liner delegation pattern in every backend's `impl ArchCodegen` block.
///
/// Each line maps a trait method to its `_impl` counterpart. The macro handles
/// both `&self` and `&mut self` receivers and optional return types.
///
/// # Usage
/// ```ignore
/// delegate_to_impl! {
///     fn calculate_stack_space(&mut self, func: &IrFunction) -> i64 => calculate_stack_space_impl;
///     fn emit_prologue(&mut self, func: &IrFunction, frame_size: i64) => emit_prologue_impl;
///     fn store_instr_for_type(&self, ty: IrType) -> &'static str => store_instr_for_type_impl;
/// }
/// ```
#[macro_export]
macro_rules! delegate_to_impl {
    // Entry point: parse one delegation at a time and recurse
    () => {};

    // &mut self method with return type
    (fn $name:ident(&mut self $(, $pname:ident : $pty:ty)*) -> $ret:ty => $impl_name:ident; $($rest:tt)*) => {
        fn $name(&mut self $(, $pname : $pty)*) -> $ret { self.$impl_name($($pname),*) }
        delegate_to_impl!{ $($rest)* }
    };

    // &mut self method without return type
    (fn $name:ident(&mut self $(, $pname:ident : $pty:ty)*) => $impl_name:ident; $($rest:tt)*) => {
        fn $name(&mut self $(, $pname : $pty)*) { self.$impl_name($($pname),*) }
        delegate_to_impl!{ $($rest)* }
    };

    // &self method with return type
    (fn $name:ident(&self $(, $pname:ident : $pty:ty)*) -> $ret:ty => $impl_name:ident; $($rest:tt)*) => {
        fn $name(&self $(, $pname : $pty)*) -> $ret { self.$impl_name($($pname),*) }
        delegate_to_impl!{ $($rest)* }
    };

    // &self method without return type
    (fn $name:ident(&self $(, $pname:ident : $pty:ty)*) => $impl_name:ident; $($rest:tt)*) => {
        fn $name(&self $(, $pname : $pty)*) { self.$impl_name($($pname),*) }
        delegate_to_impl!{ $($rest)* }
    };
}

use super::cast::{classify_float_binop, FloatOp};
use super::common::PtrDirective;
use super::generation::is_i128_type;
use super::regalloc::PhysReg;
use super::state::{CodegenState, SlotAddr, StackSlot};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{
    AtomicOrdering, AtomicRmwOp, BlockId, IntrinsicOp, IrBinOp, IrCmpOp, IrConst, IrFunction,
    IrUnaryOp, Operand, Value,
};

/// Minimum number of switch cases required to consider a jump table.
/// Fewer cases are better served by a linear compare-and-branch chain.
pub const MIN_JUMP_TABLE_CASES: usize = 4;

/// Maximum number of entries in a generated jump table.
/// Tables larger than this waste too much memory for sparse switches.
pub const MAX_JUMP_TABLE_RANGE: usize = 4096;

/// Minimum density percentage (cases * 100 / range) for jump table eligibility.
/// Below this threshold, the table would be mostly empty and a linear chain
/// of compare-and-branch instructions is more efficient.
pub const MIN_JUMP_TABLE_DENSITY_PERCENT: usize = 40;

/// Trait that each architecture implements to provide its specific code generation.
///
/// The shared framework calls these methods during instruction dispatch.
/// Each method should emit the appropriate assembly instructions.
pub trait ArchCodegen {
    /// Mutable access to the shared codegen state.
    fn state(&mut self) -> &mut CodegenState;
    /// Immutable access to the shared codegen state.
    fn state_ref(&self) -> &CodegenState;

    /// The pointer directive for this architecture's data emission.
    fn ptr_directive(&self) -> PtrDirective;

    /// Calculate stack space and assign locations for all values in the function.
    /// Returns the raw stack space needed (before alignment).
    fn calculate_stack_space(&mut self, func: &IrFunction) -> i64;

    /// Emit function prologue (save frame pointer, allocate stack).
    fn emit_prologue(&mut self, func: &IrFunction, frame_size: i64);

    /// Emit function epilogue (restore frame pointer, deallocate stack).
    fn emit_epilogue(&mut self, frame_size: i64);

    /// Store function parameters from argument registers to their stack slots.
    fn emit_store_params(&mut self, func: &IrFunction);

    /// Emit a ParamRef instruction: load the value of parameter `param_idx` into `dest`.
    /// This is used when mem2reg has promoted a parameter alloca to SSA and the
    /// initial parameter value needs to be materialized as an SSA value.
    fn emit_param_ref(&mut self, dest: &Value, param_idx: usize, ty: IrType);

    /// Load an operand into the primary accumulator register (rax/x0/t0).
    fn emit_load_operand(&mut self, op: &Operand);

    /// Store the primary accumulator to a value's stack slot.
    fn emit_store_result(&mut self, dest: &Value);

    /// Copy a value from src to dest.
    ///
    /// Default implementation is register-allocation-aware: when both src and dest
    /// have physical register assignments, emits a direct reg-to-reg move. When only
    /// dest has a register, loads through the accumulator then moves. Otherwise falls
    /// back to the accumulator load/store path.
    ///
    /// Backends needing special handling (e.g., x86 F128 x87 copies) should override
    /// this, handle their special case, then call the default for the rest.
    fn emit_copy_value(&mut self, dest: &Value, src: &Operand) {
        let dest_phys = self.get_phys_reg_for_value(dest.0);
        let src_phys = match src {
            Operand::Value(v) => self.get_phys_reg_for_value(v.0),
            _ => None,
        };

        match (dest_phys, src_phys) {
            (Some(d), Some(s)) => {
                // Direct register-to-register copy
                if d.0 != s.0 {
                    self.emit_reg_to_reg_move(s, d);
                }
                self.state().reg_cache.invalidate_acc();
            }
            (Some(d), None) => {
                // Dest in register, src on stack/constant: load to acc then move
                self.emit_load_operand(src);
                self.emit_acc_to_phys_reg(d);
                self.state().reg_cache.invalidate_acc();
            }
            _ => {
                // No register assignment: use accumulator path
                self.emit_load_operand(src);
                self.emit_store_result(dest);
            }
        }
    }

    /// Get the physical register assigned to a value, if any.
    /// Returns None when the value is stack-allocated or not register-assigned.
    /// Backends with register allocation must implement this; the default returns
    /// None which makes emit_copy_value fall through to the accumulator path.
    fn get_phys_reg_for_value(&self, _val_id: u32) -> Option<PhysReg> {
        None
    }

    /// Emit a register-to-register move between two physical registers.
    /// Only called when get_phys_reg_for_value returns Some for both src and dest.
    fn emit_reg_to_reg_move(&mut self, _src: PhysReg, _dest: PhysReg) {
        panic!(
            "backend must implement emit_reg_to_reg_move when get_phys_reg_for_value returns Some"
        );
    }

    /// Move the accumulator value into a physical register.
    /// Only called when get_phys_reg_for_value returns Some for the dest.
    fn emit_acc_to_phys_reg(&mut self, _dest: PhysReg) {
        panic!(
            "backend must implement emit_acc_to_phys_reg when get_phys_reg_for_value returns Some"
        );
    }

    /// Compute the runtime-aligned address of an over-aligned alloca into the
    /// pointer register (same register used by emit_load_ptr_from_slot: rcx on x86).
    fn emit_alloca_aligned_addr(&mut self, slot: StackSlot, val_id: u32);

    /// Compute aligned alloca address into the accumulator (rax/x0/a0).
    fn emit_alloca_aligned_addr_to_acc(&mut self, slot: StackSlot, val_id: u32);

    /// Emit a store instruction: store val to the address in ptr.
    /// Default implementation uses `SlotAddr` to dispatch the 3-way
    /// alloca/over-aligned/indirect pattern once for both i128 and typed stores.
    /// Backends needing to intercept specific types (e.g., x86 F128) can override
    /// this, handle their special case, then call `emit_store_default` for the rest.
    fn emit_store(&mut self, val: &Operand, ptr: &Value, ty: IrType) {
        emit_store_default(self, val, ptr, ty);
    }

    /// Emit a load instruction: load from the address in ptr to dest.
    /// Default implementation uses `SlotAddr` to dispatch the 3-way pattern.
    /// Backends needing to intercept specific types (e.g., x86 F128) can override
    /// this, handle their special case, then call `emit_load_default` for the rest.
    fn emit_load(&mut self, dest: &Value, ptr: &Value, ty: IrType) {
        emit_load_default(self, dest, ptr, ty);
    }

    /// Emit a load with a segment override prefix (e.g., %gs: or %fs:).
    /// Used for GCC named address space extensions (__seg_gs, __seg_fs) on x86.
    /// Default: panics (only x86 supports segment overrides).
    fn emit_seg_load(&mut self, _dest: &Value, _ptr: &Value, _ty: IrType, _seg: AddressSpace) {
        panic!("segment override loads only supported on x86");
    }

    /// Emit a segment-overridden load using a direct symbol(%rip) reference.
    /// Used when the pointer is a global address, avoiding register-indirect
    /// addressing which would use the absolute address as a segment offset.
    fn emit_seg_load_symbol(&mut self, _dest: &Value, _sym: &str, _ty: IrType, _seg: AddressSpace) {
        panic!("segment override loads only supported on x86");
    }

    /// Emit a store with a segment override prefix (e.g., %gs: or %fs:).
    /// Default: panics (only x86 supports segment overrides).
    fn emit_seg_store(&mut self, _val: &Operand, _ptr: &Value, _ty: IrType, _seg: AddressSpace) {
        panic!("segment override stores only supported on x86");
    }

    /// Emit a segment-overridden store using a direct symbol(%rip) reference.
    /// Used when the pointer is a global address, avoiding register-indirect
    /// addressing which would use the absolute address as a segment offset.
    fn emit_seg_store_symbol(
        &mut self,
        _val: &Operand,
        _sym: &str,
        _ty: IrType,
        _seg: AddressSpace,
    ) {
        panic!("segment override stores only supported on x86");
    }

    /// Whether this backend supports folding GlobalAddr + Load/Store into a
    /// single PC-relative memory access (e.g., `symbol(%rip)` on x86-64).
    /// Returns false by default; x86-64 returns true.
    fn supports_global_addr_fold(&self) -> bool {
        false
    }

    /// Emit a RIP-relative load from a global symbol (folded GlobalAddr + Load).
    /// Used to fold GlobalAddr + Load into a single `movl symbol(%rip), %eax`
    /// (or appropriate variant). Default: panics.
    fn emit_global_load_rip_rel(&mut self, _dest: &Value, _sym: &str, _ty: IrType) {
        panic!("global RIP-relative load only supported on x86");
    }

    /// Emit a RIP-relative store to a global symbol (folded GlobalAddr + Store).
    /// Used to fold GlobalAddr + Store into a single `movl %eax, symbol(%rip)`
    /// (or appropriate variant). Default: panics.
    fn emit_global_store_rip_rel(&mut self, _val: &Operand, _sym: &str, _ty: IrType) {
        panic!("global RIP-relative store only supported on x86");
    }

    /// Emit a load with a folded GEP constant offset: load from (base + const_offset).
    ///
    /// This is an optimization for the common pattern:
    ///   %ptr = GEP %base, const_offset
    ///   %val = Load %ptr
    /// The GEP instruction is skipped and the offset is folded into the load.
    ///
    /// Works for alloca bases (Direct: folded into rbp-relative slot,
    /// OverAligned: compute aligned addr + offset) and non-alloca bases
    /// (Indirect: load base pointer, add offset, load through it).
    fn emit_load_with_const_offset(&mut self, dest: &Value, base: &Value, offset: i64, ty: IrType) {
        let addr = self.state_ref().resolve_slot_addr(base.0);
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
                    // Non-alloca base: load the base pointer from its stack slot
                    // to the addr register, add the constant offset, then load.
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

    /// Emit a store with a folded GEP constant offset: store val to (base + const_offset).
    ///
    /// Works for both alloca bases (Direct: folded into rbp-relative slot) and
    /// non-alloca bases (Indirect: load base pointer, add offset, store through it).
    fn emit_store_with_const_offset(
        &mut self,
        val: &Operand,
        base: &Value,
        offset: i64,
        ty: IrType,
    ) {
        self.emit_load_operand(val);
        let addr = self.state_ref().resolve_slot_addr(base.0);
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
                    // Non-alloca base: save the value, load the base pointer,
                    // add the constant offset, then store through it.
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

    /// Add a constant offset to the address register (rcx on x86, x9 on ARM, t5 on RISC-V).
    fn emit_add_offset_to_addr_reg(&mut self, offset: i64);

    /// Emit a binary operation. Default: dispatches i128/float/integer ops.
    fn emit_binop(&mut self, dest: &Value, op: IrBinOp, lhs: &Operand, rhs: &Operand, ty: IrType) {
        if is_i128_type(ty) {
            self.emit_i128_binop(dest, op, lhs, rhs);
            return;
        }
        if ty.is_float() {
            let float_op = classify_float_binop(op)
                .unwrap_or_else(|| panic!("unsupported float binop: {:?} on type {:?}", op, ty));
            self.emit_float_binop(dest, float_op, lhs, rhs, ty);
            return;
        }
        self.emit_int_binop(dest, op, lhs, rhs, ty);
    }

    /// Emit a float binary operation (add/sub/mul/div).
    fn emit_float_binop(
        &mut self,
        dest: &Value,
        op: FloatOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        let mnemonic = self.emit_float_binop_mnemonic(op);
        self.emit_load_operand(lhs);
        self.emit_acc_to_secondary();
        self.emit_load_operand(rhs);
        self.emit_float_binop_impl(mnemonic, ty);
        self.emit_store_result(dest);
    }

    /// Get the instruction mnemonic for a float binary op.
    /// Default returns "fadd"/"fsub"/"fmul"/"fdiv" (ARM, RISC-V).
    /// x86 overrides to return "add"/"sub"/"mul"/"div" (no `f` prefix).
    fn emit_float_binop_mnemonic(&self, op: FloatOp) -> &'static str {
        match op {
            FloatOp::Add => "fadd",
            FloatOp::Sub => "fsub",
            FloatOp::Mul => "fmul",
            FloatOp::Div => "fdiv",
        }
    }

    /// Emit the arch-specific float binop instructions.
    fn emit_float_binop_impl(&mut self, mnemonic: &str, ty: IrType);

    /// Emit an integer binary operation (all IrBinOp variants).
    fn emit_int_binop(
        &mut self,
        dest: &Value,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    );

    /// Emit a unary operation.
    /// Default dispatches i128 → F128 neg → float → int to arch-specific primitives.
    /// Backends that override this should call `emit_unaryop_default` for unhandled cases.
    fn emit_unaryop(&mut self, dest: &Value, op: IrUnaryOp, src: &Operand, ty: IrType) {
        emit_unaryop_default(self, dest, op, src, ty);
    }

    /// Emit a comparison operation.
    ///
    /// Default dispatches i128 → f128 → float → integer to per-type primitives.
    /// This mirrors the pattern used by emit_binop and emit_unaryop.
    fn emit_cmp(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) {
        if is_i128_type(ty) {
            self.emit_i128_cmp(dest, op, lhs, rhs);
            return;
        }
        if ty == IrType::F128 {
            self.emit_f128_cmp(dest, op, lhs, rhs);
            return;
        }
        if ty.is_float() {
            self.emit_float_cmp(dest, op, lhs, rhs, ty);
            return;
        }
        self.emit_int_cmp(dest, op, lhs, rhs, ty);
    }

    /// Emit a floating-point comparison (F32/F64).
    /// Called by the default emit_cmp for float types (not F128, not i128).
    fn emit_float_cmp(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    );

    /// Emit an F128 (long double / quad precision) comparison.
    /// Called by the default emit_cmp for F128 types.
    /// On x86, uses x87 fucomip; on ARM/RISC-V, uses soft-float libcalls.
    fn emit_f128_cmp(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand);

    /// Emit an integer comparison.
    /// Called by the default emit_cmp for non-float, non-i128 types.
    fn emit_int_cmp(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType);

    /// Emit a function call (direct or indirect).
    ///
    /// The default implementation provides the shared algorithmic skeleton that all four
    /// architectures follow: classify args → emit stack args → load register args → call → cleanup → store result.
    /// Backends override the small `emit_call_*` hook methods instead of reimplementing this entire method.
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
        _is_fastcall: bool,
        ret_eightbyte_classes: &[crate::common::types::EightbyteClass],
    ) {
        use super::call_abi::*;
        let config = self.call_abi_config();
        let mut arg_classes = classify_call_args(
            args,
            arg_types,
            struct_arg_sizes,
            struct_arg_aligns,
            struct_arg_classes,
            struct_arg_riscv_float_classes,
            is_variadic,
            &config,
        );

        // AArch64 ABI: the sret pointer goes in x8 (indirect result register),
        // NOT in x0 as a regular argument.  Reclassify: mark arg[0] as ZeroSizeSkip
        // (it will be handled by emit_call_sret_setup) and shift all other GP
        // register indices down by one so that x0 is available for the first
        // real argument.
        // Additionally, since the sret pointer freed up a GP register slot (the
        // classification used one for sret that actually goes in x8), promote
        // the first GP stack-overflow arg to use the freed register (x7).
        let sret_operand = if is_sret && self.sret_uses_dedicated_reg() && !args.is_empty() {
            arg_classes[0] = CallArgClass::ZeroSizeSkip;
            // Find the max GP register index used after shifting, to know
            // which register slot is now free for promotion.
            let max_int_regs = config.max_int_regs; // 8 for ARM64
            for cls in arg_classes.iter_mut().skip(1) {
                match cls {
                    CallArgClass::IntReg { reg_idx } if *reg_idx > 0 => {
                        *reg_idx -= 1;
                    }
                    CallArgClass::I128RegPair { base_reg_idx } if *base_reg_idx > 0 => {
                        *base_reg_idx -= 1;
                    }
                    CallArgClass::StructByValReg { base_reg_idx, .. } if *base_reg_idx > 0 => {
                        *base_reg_idx -= 1;
                    }
                    _ => {}
                }
            }
            // Promote the first single-register GP stack arg to the freed slot (max_int_regs - 1).
            // The classification originally overflowed this arg because sret consumed a slot.
            // IMPORTANT: Only promote non-float Stack args. Float args that overflowed from
            // FP registers should stay on the stack, not be moved to a GP register.
            let freed_reg = max_int_regs - 1; // x7
            for i in 1..arg_classes.len() {
                match &arg_classes[i] {
                    CallArgClass::Stack => {
                        // Check if this arg is a float type -- floats that overflowed
                        // from FP registers must stay on the stack, not go in GP regs.
                        let is_float = i < arg_types.len() && arg_types[i].is_float();
                        if !is_float {
                            arg_classes[i] = CallArgClass::IntReg { reg_idx: freed_reg };
                            break;
                        }
                    }
                    CallArgClass::StructByValStack { size } if *size <= 8 => {
                        let sz = *size;
                        arg_classes[i] = CallArgClass::StructByValReg {
                            base_reg_idx: freed_reg,
                            size: sz,
                        };
                        break;
                    }
                    _ => {}
                }
            }
            Some(&args[0])
        } else {
            None
        };

        // Phase 0: Spill indirect function pointer before any stack manipulation.
        let indirect = func_ptr.is_some() && direct_name.is_none();
        if indirect {
            self.emit_call_spill_fptr(func_ptr.expect("indirect call requires func_ptr"));
        }

        // Compute stack space needed for overflow args.
        let stack_arg_space = self.emit_call_compute_stack_space(&arg_classes, arg_types);

        // Phase 1: Pre-convert F128 values that need helper calls (before stack args clobber regs).
        let f128_temp_space =
            self.emit_call_f128_pre_convert(args, &arg_classes, arg_types, stack_arg_space);

        // Each phase may clobber the accumulator register (t0 on RISC-V, rax on x86) via
        // helper calls or loading different values, so invalidate the cache at boundaries.
        self.state().reg_cache.invalidate_acc();

        // Phase 2: Emit stack overflow args.
        let total_sp_adjust = self.emit_call_stack_args(
            args,
            &arg_classes,
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

        // Phase 3: Load register args (GP, FP, i128, struct-by-val, F128).
        self.emit_call_reg_args(
            args,
            &arg_classes,
            arg_types,
            total_sp_adjust,
            f128_temp_space,
            stack_arg_space,
            struct_arg_riscv_float_classes,
        );

        // Phase 3.5: Set up sret pointer in dedicated register (x8 on AArch64).
        if let Some(sret_op) = sret_operand {
            self.emit_call_sret_setup(sret_op, total_sp_adjust);
        }

        // Phase 4: Emit the actual call instruction.
        self.emit_call_instruction(direct_name, func_ptr, indirect, stack_arg_space);

        // Phase 5: Clean up stack.
        // On i386 SysV, sret calls have the callee pop the hidden pointer with `ret $4`,
        // so we subtract those bytes from the caller's cleanup.
        let callee_pops = self.callee_pops_bytes_for_sret(is_sret);
        let effective_stack_cleanup = stack_arg_space.saturating_sub(callee_pops);
        self.emit_call_cleanup(effective_stack_cleanup, f128_temp_space, indirect);

        // Phase 6: Store return value.
        if let Some(dest) = dest {
            self.set_call_ret_eightbyte_classes(ret_eightbyte_classes);
            self.emit_call_store_result(&dest, return_type);
        }
    }

    // ---- Call hook methods (overridden by each backend) ----

    /// Return the ABI configuration for this architecture's function calls.
    fn call_abi_config(&self) -> super::call_abi::CallAbiConfig;

    /// Compute how much stack space to allocate for overflow arguments.
    /// x86 returns raw push bytes; ARM/RISC-V return pre-allocated SP space.
    /// `arg_types` is provided so that i686 can account for F64 taking 8 bytes on the stack.
    fn emit_call_compute_stack_space(
        &self,
        arg_classes: &[super::call_abi::CallArgClass],
        arg_types: &[IrType],
    ) -> usize;

    /// Spill an indirect function pointer to a safe location before stack manipulation.
    /// No-op on x86 (uses r10). ARM/RISC-V spill to stack.
    fn emit_call_spill_fptr(&mut self, func_ptr: &Operand) {
        let _ = func_ptr;
    }

    /// Size of the function pointer spill slot (0 for x86, 16 for ARM).
    fn emit_call_fptr_spill_size(&self) -> usize {
        0
    }

    /// Pre-convert F128 variable arguments that need __extenddftf2/__trunctfdf2.
    /// Returns the temp stack space allocated for converted results.
    fn emit_call_f128_pre_convert(
        &mut self,
        _args: &[Operand],
        _arg_classes: &[super::call_abi::CallArgClass],
        _arg_types: &[IrType],
        _stack_arg_space: usize,
    ) -> usize {
        0
    }

    /// Emit stack overflow arguments. Returns total SP adjustment (stack_arg_space + fptr_spill + f128_temp).
    fn emit_call_stack_args(
        &mut self,
        args: &[Operand],
        arg_classes: &[super::call_abi::CallArgClass],
        arg_types: &[IrType],
        stack_arg_space: usize,
        fptr_spill: usize,
        f128_temp_space: usize,
    ) -> i64;

    /// Load arguments into registers (GP, FP, i128, struct-by-val, F128).
    fn emit_call_reg_args(
        &mut self,
        args: &[Operand],
        arg_classes: &[super::call_abi::CallArgClass],
        arg_types: &[IrType],
        total_sp_adjust: i64,
        f128_temp_space: usize,
        stack_arg_space: usize,
        struct_arg_riscv_float_classes: &[Option<crate::common::types::RiscvFloatClass>],
    );

    /// Emit the call/bl/jalr instruction.
    /// `stack_arg_space` is passed so ARM can reload the spilled fptr at the correct offset.
    fn emit_call_instruction(
        &mut self,
        direct_name: Option<&str>,
        func_ptr: Option<&Operand>,
        indirect: bool,
        stack_arg_space: usize,
    );

    /// Clean up stack space after the call returns.
    fn emit_call_cleanup(&mut self, stack_arg_space: usize, f128_temp_space: usize, indirect: bool);

    /// Returns true if this architecture uses a dedicated register (not part of the
    /// normal argument sequence) for the sret pointer.  AArch64 uses x8; x86-64, RISC-V
    /// and i686 pass it as the first normal argument.
    fn sret_uses_dedicated_reg(&self) -> bool {
        false
    }

    /// Emit the sret pointer into the dedicated register (e.g., x8 on AArch64).
    /// Only called when `sret_uses_dedicated_reg()` returns true.
    fn emit_call_sret_setup(&mut self, _sret_operand: &Operand, _total_sp_adjust: i64) {}

    /// Returns the number of bytes the callee pops from the stack on return.
    /// On i386 SysV, functions returning via sret do `ret $4` to pop the hidden
    /// pointer. All other architectures and non-sret calls return 0.
    fn callee_pops_bytes_for_sret(&self, _is_sret: bool) -> usize {
        0
    }

    /// Stash the SysV eightbyte classification for the call's return struct.
    /// x86-64 overrides this to store the classes for use in emit_call_store_result.
    /// Other backends ignore it (default no-op).
    fn set_call_ret_eightbyte_classes(
        &mut self,
        _classes: &[crate::common::types::EightbyteClass],
    ) {
    }

    /// Store the function's return value from ABI registers to the destination slot.
    ///
    /// Default implementation dispatches by return type to small primitives:
    /// - i128: `emit_call_store_i128_result`
    /// - F128: `emit_call_store_f128_result`
    /// - F32: `emit_call_move_f32_to_acc` then store
    /// - F64/float: `emit_call_move_f64_to_acc` then store
    /// - integer: store directly (return value already in accumulator)
    ///
    /// Backends with special return handling (e.g., x86 F128 via x87) should override.
    fn emit_call_store_result(&mut self, dest: &Value, return_type: IrType) {
        if is_i128_type(return_type) {
            self.emit_call_store_i128_result(dest);
        } else if return_type.is_long_double() {
            self.emit_call_store_f128_result(dest);
        } else if return_type == IrType::F32 {
            self.emit_call_move_f32_to_acc();
            self.emit_store_result(dest);
        } else if return_type.is_float() {
            self.emit_call_move_f64_to_acc();
            self.emit_store_result(dest);
        } else {
            self.emit_store_result(dest);
        }
    }

    /// Store i128 return value from ABI register pair to dest.
    fn emit_call_store_i128_result(&mut self, dest: &Value);

    /// Store F128 return value to dest (may involve libcall truncation).
    fn emit_call_store_f128_result(&mut self, dest: &Value);

    /// Move F32 return value from ABI float register to integer accumulator.
    fn emit_call_move_f32_to_acc(&mut self);

    /// Move F64 return value from ABI float register to integer accumulator.
    fn emit_call_move_f64_to_acc(&mut self);

    /// Emit a global address load.
    fn emit_global_addr(&mut self, dest: &Value, name: &str);

    /// Emit a global address using absolute addressing (R_X86_64_32S).
    /// Used in kernel code model for GlobalAddr values that are only used
    /// as integer values (e.g., `(unsigned long)_text`), not as pointers.
    /// Default: falls back to emit_global_addr.
    fn emit_global_addr_absolute(&mut self, dest: &Value, name: &str) {
        self.emit_global_addr(dest, name);
    }

    /// Emit the address of a thread-local variable using TLS access patterns.
    /// Uses Local Exec model for executables:
    /// - x86-64: %fs:0 + x@TPOFF
    /// - AArch64: mrs tpidr_el0 + :tprel_hi12:/:tprel_lo12_nc: offsets
    /// - RISC-V: tp register + %tprel_hi/%tprel_lo offsets
    fn emit_tls_global_addr(&mut self, dest: &Value, name: &str);

    /// Emit a get-element-pointer (base + offset).
    fn emit_gep(&mut self, dest: &Value, base: &Value, offset: &Operand) {
        // Optimized path for constant offsets: avoid loading offset into acc
        // and adding via secondary register. Instead, directly compute
        // base_addr + constant using efficient addressing modes.
        if let Operand::Const(c) = offset {
            let const_offset = match c {
                IrConst::I64(n) => Some(*n),
                IrConst::I32(n) => Some(*n as i64),
                IrConst::I16(n) => Some(*n as i64),
                IrConst::I8(n) => Some(*n as i64),
                _ => None,
            };
            // Backend displacement fields are signed 32-bit (x86 disp32, ARM imm9/12,
            // RISC-V imm12).  If the offset doesn't fit in i32, sign-narrow it:
            // unsigned type constants (e.g. U32 -1 stored as 4294967295) must be
            // re-interpreted as signed 32-bit for pointer arithmetic.  Offsets that
            // truly exceed i32 after narrowing fall through to the general (register)
            // path.
            let const_offset = const_offset.and_then(|off| {
                if off >= i32::MIN as i64 && off <= i32::MAX as i64 {
                    Some(off)
                } else if off > i32::MAX as i64 && off <= u32::MAX as i64 {
                    // U32-range: reinterpret as signed 32-bit
                    Some(off as i32 as i64)
                } else {
                    None
                }
            });
            if let Some(off) = const_offset {
                if let Some(addr) = self.state_ref().resolve_slot_addr(base.0) {
                    match addr {
                        SlotAddr::OverAligned(slot, id) => {
                            // Compute aligned base addr into acc, then add offset.
                            self.emit_alloca_aligned_addr_to_acc(slot, id);
                            self.emit_gep_add_const_to_acc(off);
                        }
                        SlotAddr::Direct(slot) => {
                            // Alloca: lea (slot+offset)(%rbp), %rax — single instruction.
                            self.emit_gep_direct_const(slot, off);
                        }
                        SlotAddr::Indirect(slot) => {
                            // Pointer in slot: load ptr, then lea offset(%rax), %rax.
                            self.emit_gep_indirect_const(slot, off, base.0);
                        }
                    }
                    self.emit_store_result(dest);
                    return;
                }
            }
        }
        // General path for non-constant offsets.
        if let Some(addr) = self.state_ref().resolve_slot_addr(base.0) {
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    self.emit_alloca_aligned_addr_to_acc(slot, id);
                    self.emit_acc_to_secondary();
                }
                SlotAddr::Direct(slot) => self.emit_slot_addr_to_secondary(slot, true, base.0),
                SlotAddr::Indirect(slot) => self.emit_slot_addr_to_secondary(slot, false, base.0),
            }
        }
        self.emit_load_operand(offset);
        self.emit_add_secondary_to_acc();
        self.emit_store_result(dest);
    }

    /// Emit optimized GEP for Direct (alloca) base + constant offset.
    /// Default: lea (slot+offset)(%rbp), %rax — single instruction on x86.
    fn emit_gep_direct_const(&mut self, slot: StackSlot, offset: i64) {
        // Default fallback: lea slot(%rbp) to secondary, load offset to acc, add.
        self.emit_slot_addr_to_secondary(slot, true, 0);
        self.emit_gep_add_const_to_acc_from_secondary(offset);
    }

    /// Emit optimized GEP for Indirect base + constant offset.
    /// Default: load ptr from slot, then lea offset(ptr), %rax — two instructions on x86.
    fn emit_gep_indirect_const(&mut self, slot: StackSlot, offset: i64, val_id: u32) {
        // Default fallback: load ptr to secondary, load offset to acc, add.
        self.emit_slot_addr_to_secondary(slot, false, val_id);
        self.emit_gep_add_const_to_acc_from_secondary(offset);
    }

    /// Add a constant offset to accumulator (used after computing base in acc).
    fn emit_gep_add_const_to_acc(&mut self, offset: i64) {
        // Default: load constant to acc as secondary, add (inefficient fallback).
        // Backends should override with `addq $offset, %rax` or `leaq offset(%rax), %rax`.
        self.emit_acc_to_secondary();
        self.emit_load_operand(&Operand::Const(IrConst::I64(offset)));
        self.emit_add_secondary_to_acc();
    }

    /// Helper for default GEP const implementations: secondary has base, need acc = base + offset.
    fn emit_gep_add_const_to_acc_from_secondary(&mut self, offset: i64) {
        self.emit_load_operand(&Operand::Const(IrConst::I64(offset)));
        self.emit_add_secondary_to_acc();
    }

    /// Move accumulator to secondary register (push on x86).
    fn emit_acc_to_secondary(&mut self);

    /// Emit architecture-specific instructions for a type cast.
    fn emit_cast_instrs(&mut self, from_ty: IrType, to_ty: IrType);

    /// Emit a type cast. Handles i128 widening/narrowing/copy using accumulator
    /// pair primitives, and delegates non-i128 casts to emit_cast_instrs.
    /// Backends that override this should call `emit_cast_default` for unhandled cases.
    fn emit_cast(&mut self, dest: &Value, src: &Operand, from_ty: IrType, to_ty: IrType) {
        emit_cast_default(self, dest, src, from_ty, to_ty);
    }

    /// Emit a memory copy.
    fn emit_memcpy(&mut self, dest: &Value, src: &Value, size: usize) {
        if let Some(addr) = self.state_ref().resolve_slot_addr(dest.0) {
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    self.emit_alloca_aligned_addr(slot, id);
                    self.emit_memcpy_store_dest_from_acc();
                }
                SlotAddr::Direct(slot) => self.emit_memcpy_load_dest_addr(slot, true, dest.0),
                SlotAddr::Indirect(slot) => self.emit_memcpy_load_dest_addr(slot, false, dest.0),
            }
        }
        if let Some(addr) = self.state_ref().resolve_slot_addr(src.0) {
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    self.emit_alloca_aligned_addr(slot, id);
                    self.emit_memcpy_store_src_from_acc();
                }
                SlotAddr::Direct(slot) => self.emit_memcpy_load_src_addr(slot, true, src.0),
                SlotAddr::Indirect(slot) => self.emit_memcpy_load_src_addr(slot, false, src.0),
            }
        }
        self.emit_memcpy_impl(size);
    }

    /// Store accumulator to memcpy dest register.
    fn emit_memcpy_store_dest_from_acc(&mut self);
    /// Store accumulator to memcpy src register.
    fn emit_memcpy_store_src_from_acc(&mut self);

    /// Emit va_arg: extract next variadic argument from va_list and store to dest.
    fn emit_va_arg(&mut self, dest: &Value, va_list_ptr: &Value, result_ty: IrType);

    /// Emit va_start: initialize a va_list for variadic argument access.
    fn emit_va_start(&mut self, va_list_ptr: &Value);

    /// Emit va_end: clean up a va_list. No-op on all current targets.
    fn emit_va_end(&mut self, _va_list_ptr: &Value) {}

    /// Emit va_copy: copy src va_list to dest va_list.
    fn emit_va_copy(&mut self, dest_ptr: &Value, src_ptr: &Value);

    /// Emit va_arg for struct types: read `size` bytes of struct data from the
    /// va_list and store them at `dest_ptr`. The va_list is advanced appropriately.
    fn emit_va_arg_struct(&mut self, dest_ptr: &Value, va_list_ptr: &Value, size: usize);

    /// Emit va_arg for struct types with eightbyte classification info.
    ///
    /// On x86-64, small structs (<=16 bytes) may be passed in registers.
    /// `eightbyte_classes` carries the SysV ABI classification so the backend
    /// can check if all required register slots are available. If not, the
    /// entire struct is read from the overflow area (per ABI rules).
    ///
    /// Default implementation ignores the classification and calls
    /// `emit_va_arg_struct`.
    fn emit_va_arg_struct_ex(
        &mut self,
        dest_ptr: &Value,
        va_list_ptr: &Value,
        size: usize,
        _eightbyte_classes: &[crate::common::types::EightbyteClass],
    ) {
        self.emit_va_arg_struct(dest_ptr, va_list_ptr, size);
    }

    /// Emit an atomic read-modify-write operation.
    fn emit_atomic_rmw(
        &mut self,
        dest: &Value,
        op: AtomicRmwOp,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    );

    /// Emit an atomic compare-and-exchange operation.
    fn emit_atomic_cmpxchg(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        expected: &Operand,
        desired: &Operand,
        ty: IrType,
        success_ordering: AtomicOrdering,
        failure_ordering: AtomicOrdering,
        returns_bool: bool,
    );

    /// Emit an atomic load.
    fn emit_atomic_load(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    );

    /// Emit an atomic store.
    fn emit_atomic_store(
        &mut self,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    );

    /// Emit a memory fence.
    fn emit_fence(&mut self, ordering: AtomicOrdering);

    /// Emit inline assembly.
    fn emit_inline_asm(
        &mut self,
        template: &str,
        outputs: &[(String, Value, Option<String>)],
        inputs: &[(String, Operand, Option<String>)],
        clobbers: &[String],
        operand_types: &[IrType],
        goto_labels: &[(String, BlockId)],
        input_symbols: &[Option<String>],
    );

    /// Emit raw inline assembly template for naked functions (no operand substitution).
    fn emit_raw_inline_asm(&mut self, template: &str) {
        for line in template.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                self.state().emit_fmt(format_args!("    {}", trimmed));
            }
        }
    }

    /// Emit inline assembly with per-operand segment overrides.
    /// Default: delegates to emit_inline_asm (ignoring segment overrides).
    /// x86 backend overrides this to apply %gs:/%fs: prefixes to memory operands.
    fn emit_inline_asm_with_segs(
        &mut self,
        template: &str,
        outputs: &[(String, Value, Option<String>)],
        inputs: &[(String, Operand, Option<String>)],
        clobbers: &[String],
        operand_types: &[IrType],
        goto_labels: &[(String, BlockId)],
        input_symbols: &[Option<String>],
        _seg_overrides: &[AddressSpace],
    ) {
        self.emit_inline_asm(
            template,
            outputs,
            inputs,
            clobbers,
            operand_types,
            goto_labels,
            input_symbols,
        );
    }

    /// Emit a return terminator.
    /// Backends that override this should call `emit_return_default` for unhandled cases.
    fn emit_return(&mut self, val: Option<&Operand>, frame_size: i64) {
        emit_return_default(self, val, frame_size);
    }

    // ---- Architecture-specific instruction primitives ----

    // --- 128-bit (accumulator pair) primitives ---

    /// Sign-extend the accumulator into the high half of the accumulator pair.
    fn emit_sign_extend_acc_high(&mut self);

    /// Zero the high half of the accumulator pair.
    fn emit_zero_acc_high(&mut self);

    /// Load an operand into the accumulator pair.
    fn emit_load_acc_pair(&mut self, op: &Operand);

    /// Store the accumulator pair to a value's 16-byte stack slot.
    fn emit_store_acc_pair(&mut self, dest: &Value);

    /// Store the accumulator pair directly to a stack slot.
    fn emit_store_pair_to_slot(&mut self, slot: StackSlot);

    /// Load the accumulator pair from a stack slot.
    fn emit_load_pair_from_slot(&mut self, slot: StackSlot);

    /// Save the accumulator pair to scratch regs before loading a pointer.
    fn emit_save_acc_pair(&mut self);

    /// Store the saved accumulator pair through the pointer now in the addr reg.
    fn emit_store_pair_indirect(&mut self);

    /// Load the accumulator pair through the pointer now in the addr reg.
    fn emit_load_pair_indirect(&mut self);

    /// Emit 128-bit negate on the accumulator pair.
    fn emit_i128_neg(&mut self);

    /// Emit 128-bit bitwise NOT on the accumulator pair.
    fn emit_i128_not(&mut self);

    /// Emit i128/u128 -> f32/f64 conversion via compiler-rt call.
    /// `src` is the i128 operand, `from_signed` indicates signed vs unsigned,
    /// `to_ty` is F32 or F64. Result should be left in the accumulator (integer reg).
    fn emit_i128_to_float_call(&mut self, src: &Operand, from_signed: bool, to_ty: IrType);

    /// Emit f32/f64 -> i128/u128 conversion via compiler-rt call.
    /// `src` is the float operand, `to_signed` indicates signed vs unsigned,
    /// `from_ty` is F32 or F64. Result should be left in the acc pair.
    fn emit_float_to_i128_call(&mut self, src: &Operand, to_signed: bool, from_ty: IrType);

    // --- Return primitives ---

    /// Get the current function's return type.
    fn current_return_type(&self) -> IrType;

    /// Move the i128 accumulator pair into the ABI return registers.
    fn emit_return_i128_to_regs(&mut self);

    /// Move the accumulator into the ABI f128 return register/format.
    fn emit_return_f128_to_reg(&mut self);

    /// Move the accumulator into the ABI float return register (f32).
    fn emit_return_f32_to_reg(&mut self);

    /// Move the accumulator into the ABI float return register (f64).
    fn emit_return_f64_to_reg(&mut self);

    /// Move the accumulator into the ABI integer return register.
    fn emit_return_int_to_reg(&mut self);

    /// Emit function epilogue and return instruction.
    fn emit_epilogue_and_ret(&mut self, frame_size: i64);

    // --- Typed store/load primitives ---

    /// Return the store instruction mnemonic for a type.
    fn store_instr_for_type(&self, ty: IrType) -> &'static str;

    /// Return the load instruction mnemonic for a type.
    fn load_instr_for_type(&self, ty: IrType) -> &'static str;

    /// Store the accumulator to a slot using a typed instruction.
    fn emit_typed_store_to_slot(&mut self, instr: &'static str, ty: IrType, slot: StackSlot);

    /// Load from a slot into the accumulator using a typed instruction.
    fn emit_typed_load_from_slot(&mut self, instr: &'static str, slot: StackSlot);

    /// Save the accumulator to a scratch register.
    fn emit_save_acc(&mut self);

    /// Load a pointer value from a non-alloca slot into the address register.
    /// `val_id` is the IR value ID of the pointer, allowing register-aware backends
    /// to load from a callee-saved register instead of the stack slot.
    fn emit_load_ptr_from_slot(&mut self, slot: StackSlot, val_id: u32);

    /// Store the saved accumulator through the address register.
    fn emit_typed_store_indirect(&mut self, instr: &'static str, ty: IrType);

    /// Load through the address register into the accumulator.
    fn emit_typed_load_indirect(&mut self, instr: &'static str);

    // --- GEP primitives ---

    /// Load a slot's effective address into a secondary register.
    /// `val_id` is the IR value ID, allowing register-aware backends to use
    /// a callee-saved register instead of loading from the stack slot.
    fn emit_slot_addr_to_secondary(&mut self, slot: StackSlot, is_alloca: bool, val_id: u32);

    /// Add the secondary register to the accumulator.
    fn emit_add_secondary_to_acc(&mut self);

    // --- Dynamic alloca primitives ---

    /// Add an immediate value to the accumulator.
    fn emit_add_imm_to_acc(&mut self, imm: i64);

    /// Round up the accumulator to 16-byte alignment.
    fn emit_round_up_acc_to_16(&mut self);

    /// Subtract the accumulator from the stack pointer.
    fn emit_sub_sp_by_acc(&mut self);

    /// Move the stack pointer value into the accumulator.
    fn emit_mov_sp_to_acc(&mut self);

    /// Align the accumulator to the given alignment.
    fn emit_align_acc(&mut self, align: usize);

    // --- Memcpy primitives ---

    /// Load the dest address for memcpy into the arch-specific dest register.
    /// `val_id` is the IR value ID for register-aware backends.
    fn emit_memcpy_load_dest_addr(&mut self, slot: StackSlot, is_alloca: bool, val_id: u32);

    /// Load the src address for memcpy into the arch-specific src register.
    /// `val_id` is the IR value ID for register-aware backends.
    fn emit_memcpy_load_src_addr(&mut self, slot: StackSlot, is_alloca: bool, val_id: u32);

    /// Emit the actual copy loop/instruction for memcpy.
    fn emit_memcpy_impl(&mut self, size: usize);

    // --- Unary operation primitives ---

    fn emit_float_neg(&mut self, ty: IrType);
    /// Negate an F128 (long double) value with full precision.
    /// This method must handle loading, negation, and storing the result
    /// (including emit_store_result). It is NOT followed by emit_store_result
    /// in the caller.
    /// Default falls back to emit_float_neg + emit_store_result, which only
    /// operates on the f64 approximation. ARM and RISC-V override this to flip
    /// the IEEE 754 sign bit of the full 128-bit representation.
    fn emit_f128_neg(&mut self, dest: &Value, src: &Operand) {
        // Default: use the f64-based emit_float_neg (loses precision on non-x86).
        // ARM and RISC-V override this with full-precision implementations.
        self.emit_load_operand(src);
        self.emit_float_neg(IrType::F128);
        self.emit_store_result(dest);
    }
    fn emit_int_neg(&mut self, ty: IrType);
    fn emit_int_not(&mut self, ty: IrType);
    fn emit_int_clz(&mut self, ty: IrType);
    fn emit_int_ctz(&mut self, ty: IrType);
    fn emit_int_bswap(&mut self, ty: IrType);
    fn emit_int_popcount(&mut self, ty: IrType);

    // --- Control flow primitives ---

    /// The unconditional jump mnemonic.
    fn jump_mnemonic(&self) -> &'static str;

    /// The trap/unreachable instruction.
    fn trap_instruction(&self) -> &'static str;

    /// Emit a branch-if-nonzero instruction.
    fn emit_branch_nonzero(&mut self, label: &str);

    /// Emit an indirect jump through the accumulator register.
    fn emit_jump_indirect(&mut self);

    // --- Default control flow implementations ---

    /// Emit an unconditional branch.
    fn emit_branch(&mut self, label: &str) {
        let mnemonic = self.jump_mnemonic();
        self.state()
            .emit_fmt(format_args!("    {} {}", mnemonic, label));
    }

    /// Emit an unconditional branch to a BlockId, avoiding String allocation.
    fn emit_branch_to_block(&mut self, block: BlockId) {
        // Cache the mnemonic first to avoid borrow conflict with state()
        let mnemonic = self.jump_mnemonic();
        let out = &mut self.state().out;
        out.write_str("    ");
        out.write_str(mnemonic);
        out.write_str(" .LBB");
        out.write_u64(block.0 as u64);
        out.newline();
    }

    /// Emit an unreachable trap instruction.
    fn emit_unreachable(&mut self) {
        let trap = self.trap_instruction();
        self.state().emit_fmt(format_args!("    {}", trap));
    }

    /// Emit a conditional select: dest = cond != 0 ? true_val : false_val.
    ///
    /// Default implementation uses a branch-based sequence. Backends can override
    /// this to emit cmov (x86), csel (ARM), or other conditional move instructions.
    fn emit_select(
        &mut self,
        dest: &Value,
        cond: &Operand,
        true_val: &Operand,
        false_val: &Operand,
        _ty: IrType,
    ) {
        // Default: branch-based select
        let label_id = self.state().next_label_id();
        let true_label = format!(".Lsel_true_{}", label_id);
        let end_label = format!(".Lsel_end_{}", label_id);

        // Load condition and branch
        self.emit_load_operand(cond);
        self.emit_branch_nonzero(&true_label);

        // False path: load false_val -> dest, jump to end
        self.emit_load_operand(false_val);
        self.emit_store_result(dest);
        self.emit_branch(&end_label);

        // True path: load true_val -> dest
        self.state().emit_fmt(format_args!("{}:", true_label));
        self.emit_load_operand(true_val);
        self.emit_store_result(dest);

        // End label
        self.state().emit_fmt(format_args!("{}:", end_label));
    }

    /// Emit a conditional branch to BlockIds, avoiding String allocations.
    fn emit_cond_branch_blocks(
        &mut self,
        cond: &Operand,
        true_block: BlockId,
        false_block: BlockId,
    ) {
        self.emit_load_operand(cond);
        // We need the label strings for emit_branch_nonzero which takes &str.
        // Use as_label() here since emit_branch_nonzero is arch-specific.
        let true_label = true_block.as_label();
        self.emit_branch_nonzero(&true_label);
        self.emit_branch_to_block(false_block);
    }

    /// Emit a fused compare-and-branch: perform the comparison and immediately
    /// branch based on the result, without materializing the boolean value.
    ///
    /// Replaces the sequence:
    ///   cmp a, b → setCC → movzb → store → load → test → jne/je
    /// with:
    ///   cmp a, b → jCC true_label → jmp false_label
    ///
    /// Only called for integer (non-float, non-i128) comparisons whose result
    /// has exactly one use (the CondBranch terminator).
    ///
    /// Default implementation falls back to separate cmp + cond_branch.
    /// Backends should override this for efficient direct conditional jumps.
    fn emit_fused_cmp_branch(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
        true_label: &str,
        false_label: &str,
    ) {
        // Default: emit separate cmp and branch (backends override for efficiency).
        // Use a dummy dest that won't match any real stack slot.
        let dummy_dest = Value(u32::MAX);
        self.emit_cmp(&dummy_dest, op, lhs, rhs, ty);
        // The accumulator holds the boolean result; branch on it.
        self.emit_branch_nonzero(true_label);
        self.emit_branch(false_label);
        // Invalidate cache since dummy_dest may have polluted it.
        self.state().reg_cache.invalidate_all();
    }

    /// Emit a fused compare-and-branch to BlockIds, avoiding String allocations.
    fn emit_fused_cmp_branch_blocks(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
        true_block: BlockId,
        false_block: BlockId,
    ) {
        // Default: delegate to string-based version.
        let true_label = true_block.as_label();
        let false_label = false_block.as_label();
        self.emit_fused_cmp_branch(op, lhs, rhs, ty, &true_label, &false_label);
    }

    /// Emit an indirect branch (computed goto).
    fn emit_indirect_branch(&mut self, target: &Operand) {
        self.emit_load_operand(target);
        self.emit_jump_indirect();
    }

    /// Emit a switch dispatch: jump to the case target matching `val`, or to `default`.
    ///
    /// For dense cases (>= 4 cases, density > 40%), emits a jump table:
    ///   1. Range check: if val < min or val > max, branch to default
    ///   2. Compute index: index = val - min
    ///   3. Load target address from jump table: addr = table[index]
    ///   4. Indirect branch to addr
    ///
    /// For sparse cases, falls back to a linear chain of compare-and-branch.
    fn emit_switch(
        &mut self,
        val: &Operand,
        cases: &[(i64, BlockId)],
        default: &BlockId,
        ty: IrType,
    ) {
        // Check density for jump table eligibility (disabled by -fno-jump-tables)
        let use_jump_table = if self.state_ref().no_jump_tables {
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
            // Sparse: linear compare-and-branch chain
            self.emit_load_operand(val);
            for &(case_val, target) in cases {
                let label = target.as_label();
                self.emit_switch_case_branch(case_val, &label, ty);
            }
            self.emit_branch_to_block(*default);
        }
    }

    /// Emit a jump table for a dense switch statement.
    ///
    /// Backends should use the shared helpers `build_jump_table()` (builds the table
    /// mapping indices to BlockIds) and `emit_jump_table_rodata()` (emits the .rodata
    /// section with pointer-sized absolute entries) to avoid duplicating the table
    /// construction and data emission logic. x86 overrides this entirely to handle
    /// PIC mode (relative .long entries).
    fn emit_switch_jump_table(
        &mut self,
        val: &Operand,
        cases: &[(i64, BlockId)],
        default: &BlockId,
        ty: IrType,
    );

    /// Emit a compare-and-branch for a single switch case:
    /// compare the accumulator against `case_val` and branch to `label` if equal.
    /// The accumulator value must be preserved across the call.
    fn emit_switch_case_branch(&mut self, case_val: i64, label: &str, ty: IrType);

    /// Emit a label address load (GCC &&label extension).
    fn emit_label_addr(&mut self, dest: &Value, label: &str) {
        self.emit_global_addr(dest, label);
    }

    /// Emit code to capture the second F64 return value after a function call.
    fn emit_get_return_f64_second(&mut self, dest: &Value);

    /// Emit code to set the second F64 return value before a return.
    fn emit_set_return_f64_second(&mut self, src: &Operand);

    /// Emit code to capture the second F32 return value.
    fn emit_get_return_f32_second(&mut self, dest: &Value);

    /// Emit code to set the second F32 return value.
    fn emit_set_return_f32_second(&mut self, src: &Operand);

    /// Emit code to capture the second F128 return value after a function call.
    /// On x86-64: reads the second long double from x87 st(0) after the first fstpt.
    fn emit_get_return_f128_second(&mut self, dest: &Value);

    /// Emit code to set the second F128 return value before a return.
    /// On x86-64: loads an additional long double onto the x87 FPU stack.
    fn emit_set_return_f128_second(&mut self, src: &Operand);

    /// Emit the function directive for the function type attribute.
    fn function_type_directive(&self) -> &'static str {
        "@function"
    }

    /// Emit dynamic stack allocation.
    ///
    /// When alignment > 16, we must allocate `size + (align - 1)` bytes
    /// (rounded up to 16) so that after aligning the base pointer upward,
    /// the aligned pointer plus `size` bytes stays within the allocated region.
    /// Without this extra padding, the upward-aligned pointer can overlap with
    /// the callee-saved register save area or return address above the VLA.
    fn emit_dyn_alloca(&mut self, dest: &Value, size: &Operand, align: usize) {
        self.emit_load_operand(size);
        if align > 16 {
            // Add (align - 1) extra bytes for alignment padding, then round
            // to 16 to keep SP properly aligned. This guarantees: after we
            // subtract the padded size from SP and align the pointer UP to
            // `align`, the aligned pointer + original size <= old SP.
            self.emit_add_imm_to_acc((align - 1) as i64);
        }
        self.emit_round_up_acc_to_16();
        self.emit_sub_sp_by_acc();
        self.emit_mov_sp_to_acc();
        if align > 16 {
            self.emit_align_acc(align);
        }
        self.emit_store_result(dest);
    }

    /// Emit stack save: store the current stack pointer to dest.
    /// Used to capture SP before VLA allocations for later restoration.
    fn emit_stack_save(&mut self, dest: &Value) {
        self.emit_mov_sp_to_acc();
        self.emit_store_result(dest);
    }

    /// Emit stack restore: restore the stack pointer from a saved value.
    /// Used to reclaim VLA stack space when jumping backward past VLA declarations.
    fn emit_stack_restore(&mut self, ptr: &Value) {
        self.emit_load_operand(&Operand::Value(*ptr));
        self.emit_mov_acc_to_sp();
    }

    /// Move the accumulator value to the stack pointer.
    /// Used by emit_stack_restore to reset SP.
    fn emit_mov_acc_to_sp(&mut self);

    /// Emit a 128-bit value copy.
    fn emit_copy_i128(&mut self, dest: &Value, src: &Operand) {
        self.emit_load_operand(src);
        self.emit_store_result(dest);
    }

    /// Frame size including alignment and saved registers.
    fn aligned_frame_size(&self, raw_space: i64) -> i64;

    // ---- 128-bit binary operation dispatch ----

    /// Prepare operands for a 128-bit binary operation.
    fn emit_i128_prep_binop(&mut self, lhs: &Operand, rhs: &Operand);

    fn emit_i128_add(&mut self);
    fn emit_i128_sub(&mut self);
    fn emit_i128_mul(&mut self);
    fn emit_i128_and(&mut self);
    fn emit_i128_or(&mut self);
    fn emit_i128_xor(&mut self);
    fn emit_i128_shl(&mut self);
    fn emit_i128_lshr(&mut self);
    fn emit_i128_ashr(&mut self);

    /// Emit an i128 division/remainder via compiler-rt call.
    fn emit_i128_divrem_call(&mut self, func_name: &str, lhs: &Operand, rhs: &Operand);

    /// Store the i128 result pair to a destination value.
    fn emit_i128_store_result(&mut self, dest: &Value);

    /// Emit an optimized 128-bit shift left by a known constant amount.
    /// LHS is already loaded into the arch-specific registers. Result goes to result regs.
    fn emit_i128_shl_const(&mut self, amount: u32);

    /// Emit an optimized 128-bit logical right shift by a known constant amount.
    fn emit_i128_lshr_const(&mut self, amount: u32);

    /// Emit an optimized 128-bit arithmetic right shift by a known constant amount.
    fn emit_i128_ashr_const(&mut self, amount: u32);

    /// Emit an i128 binary operation. Default dispatches to per-op primitives.
    fn emit_i128_binop(&mut self, dest: &Value, op: IrBinOp, lhs: &Operand, rhs: &Operand) {
        match op {
            IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem => {
                let func_name = match op {
                    IrBinOp::SDiv => "__divti3",
                    IrBinOp::UDiv => "__udivti3",
                    IrBinOp::SRem => "__modti3",
                    IrBinOp::URem => "__umodti3",
                    _ => unreachable!("i128 divrem matched non-div/rem op: {:?}", op),
                };
                self.emit_i128_divrem_call(func_name, lhs, rhs);
            }
            // For shifts with constant amounts, emit optimized branchless code
            IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr
                if get_const_i128_shift_amount(rhs).is_some() =>
            {
                let amount = get_const_i128_shift_amount(rhs)
                    .expect("i128 const shift amount (guarded by is_some)");
                // Load only the LHS (the value to shift)
                self.emit_i128_prep_shift_lhs(lhs);
                match op {
                    IrBinOp::Shl => self.emit_i128_shl_const(amount),
                    IrBinOp::LShr => self.emit_i128_lshr_const(amount),
                    IrBinOp::AShr => self.emit_i128_ashr_const(amount),
                    _ => unreachable!("i128 const-shift matched non-shift op: {:?}", op),
                }
            }
            _ => {
                self.emit_i128_prep_binop(lhs, rhs);
                match op {
                    IrBinOp::Add => self.emit_i128_add(),
                    IrBinOp::Sub => self.emit_i128_sub(),
                    IrBinOp::Mul => self.emit_i128_mul(),
                    IrBinOp::And => self.emit_i128_and(),
                    IrBinOp::Or => self.emit_i128_or(),
                    IrBinOp::Xor => self.emit_i128_xor(),
                    IrBinOp::Shl => self.emit_i128_shl(),
                    IrBinOp::LShr => self.emit_i128_lshr(),
                    IrBinOp::AShr => self.emit_i128_ashr(),
                    _ => unreachable!("unhandled i128 binary op: {:?}", op),
                }
            }
        }
        self.emit_i128_store_result(dest);
    }

    /// Load the LHS operand for a shift (without loading RHS).
    /// Default implementation uses the full prep_binop with a dummy RHS.
    fn emit_i128_prep_shift_lhs(&mut self, lhs: &Operand) {
        self.emit_i128_prep_binop(lhs, &Operand::Const(IrConst::I128(0)));
    }

    // ---- 128-bit comparison dispatch ----

    /// Prepare operands for a 128-bit comparison.
    fn emit_i128_prep_cmp(&mut self, lhs: &Operand, rhs: &Operand) {
        self.emit_i128_prep_binop(lhs, rhs);
    }

    /// Emit an i128 equality comparison (Eq or Ne).
    fn emit_i128_cmp_eq(&mut self, is_ne: bool);

    /// Emit an ordered i128 comparison.
    fn emit_i128_cmp_ordered(&mut self, op: IrCmpOp);

    /// Store the i128 comparison result to dest.
    fn emit_i128_cmp_store_result(&mut self, dest: &Value);

    /// Emit an i128 comparison. Default dispatches Eq/Ne vs ordered.
    fn emit_i128_cmp(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand) {
        self.emit_i128_prep_cmp(lhs, rhs);
        match op {
            IrCmpOp::Eq => self.emit_i128_cmp_eq(false),
            IrCmpOp::Ne => self.emit_i128_cmp_eq(true),
            _ => self.emit_i128_cmp_ordered(op),
        }
        self.emit_i128_cmp_store_result(dest);
    }

    /// Emit a target-independent intrinsic operation (fences, SIMD, CRC32, etc.).
    /// Each backend must implement this to emit the appropriate native instructions.
    fn emit_intrinsic(
        &mut self,
        _dest: &Option<Value>,
        _op: &IntrinsicOp,
        _dest_ptr: &Option<Value>,
        _args: &[Operand],
    ) {
    }

    /// Emit runtime helper stubs needed by this architecture.
    /// Called after all functions are generated, before the .note.GNU-stack section.
    /// The i686 backend uses this to emit __divdi3/__udivdi3/__moddi3/__umoddi3
    /// as weak symbols for standalone builds without libgcc.
    fn emit_runtime_stubs(&mut self) {}
}

// ── Shared jump table helpers ─────────────────────────────────────────────────

/// Build a jump table from switch cases: maps each index in [min..max] to a BlockId.
/// Returns (table, min_val, range).
pub fn build_jump_table(cases: &[(i64, BlockId)], default: &BlockId) -> (Vec<BlockId>, i64, usize) {
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

    let mut table = vec![*default; range];
    for &(case_val, target) in cases {
        let idx = (case_val - min_val) as usize;
        table[idx] = target;
    }
    (table, min_val, range)
}

// ── Default store/load implementations as free functions ──────────────────────
//
// These are extracted as free functions so that backends that override
// emit_store/emit_load (e.g., x86 for F128 handling) can delegate to the
// default logic for non-special-cased types without duplicating the 3-way
// SlotAddr dispatch code.

/// Extract a constant shift amount from an operand (for optimizing 128-bit shifts).
/// Returns Some(amount) if the operand is a constant in the range [0, 127].
pub fn get_const_i128_shift_amount(rhs: &Operand) -> Option<u32> {
    match rhs {
        Operand::Const(c) => {
            let val = c.to_i64()?;
            if (0..128).contains(&val) {
                Some(val as u32)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Default store implementation: 3-way SlotAddr dispatch for i128 and typed stores.
/// Backends that override `emit_store` should call this for types they don't handle specially.
pub fn emit_store_default(
    cg: &mut (impl ArchCodegen + ?Sized),
    val: &Operand,
    ptr: &Value,
    ty: IrType,
) {
    let addr = cg.state_ref().resolve_slot_addr(ptr.0);
    if is_i128_type(ty) {
        cg.emit_load_acc_pair(val);
        if let Some(addr) = addr {
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    cg.emit_save_acc_pair();
                    cg.emit_alloca_aligned_addr(slot, id);
                    cg.emit_store_pair_indirect();
                }
                SlotAddr::Direct(slot) => cg.emit_store_pair_to_slot(slot),
                SlotAddr::Indirect(slot) => {
                    // Load the pointer into the ptr register (ecx/x9/t5) BEFORE
                    // saving the accumulator pair to callee-saved regs (esi:edi on i686).
                    // On i686, the pointer may be register-allocated to esi or edi,
                    // so emit_save_acc_pair() would clobber it if done first.
                    cg.emit_load_ptr_from_slot(slot, ptr.0);
                    cg.emit_save_acc_pair();
                    cg.emit_store_pair_indirect();
                }
            }
        }
        return;
    }
    cg.emit_load_operand(val);
    if let Some(addr) = addr {
        let store_instr = cg.store_instr_for_type(ty);
        match addr {
            SlotAddr::OverAligned(slot, id) => {
                cg.emit_save_acc();
                cg.emit_alloca_aligned_addr(slot, id);
                cg.emit_typed_store_indirect(store_instr, ty);
            }
            SlotAddr::Direct(slot) => cg.emit_typed_store_to_slot(store_instr, ty, slot),
            SlotAddr::Indirect(slot) => {
                cg.emit_save_acc();
                cg.emit_load_ptr_from_slot(slot, ptr.0);
                cg.emit_typed_store_indirect(store_instr, ty);
            }
        }
    }
}

/// Default load implementation: 3-way SlotAddr dispatch for i128 and typed loads.
/// Backends that override `emit_load` should call this for types they don't handle specially.
pub fn emit_load_default(
    cg: &mut (impl ArchCodegen + ?Sized),
    dest: &Value,
    ptr: &Value,
    ty: IrType,
) {
    let addr = cg.state_ref().resolve_slot_addr(ptr.0);
    if is_i128_type(ty) {
        if let Some(addr) = addr {
            match addr {
                SlotAddr::OverAligned(slot, id) => {
                    cg.emit_alloca_aligned_addr(slot, id);
                    cg.emit_load_pair_indirect();
                }
                SlotAddr::Direct(slot) => cg.emit_load_pair_from_slot(slot),
                SlotAddr::Indirect(slot) => {
                    cg.emit_load_ptr_from_slot(slot, ptr.0);
                    cg.emit_load_pair_indirect();
                }
            }
            cg.emit_store_acc_pair(dest);
        }
        return;
    }
    if let Some(addr) = addr {
        let load_instr = cg.load_instr_for_type(ty);
        match addr {
            SlotAddr::OverAligned(slot, id) => {
                cg.emit_alloca_aligned_addr(slot, id);
                cg.emit_typed_load_indirect(load_instr);
            }
            SlotAddr::Direct(slot) => cg.emit_typed_load_from_slot(load_instr, slot),
            SlotAddr::Indirect(slot) => {
                cg.emit_load_ptr_from_slot(slot, ptr.0);
                cg.emit_typed_load_indirect(load_instr);
            }
        }
        cg.emit_store_result(dest);
    }
}

/// Default cast implementation: handles i128 widening/narrowing/copy,
/// and delegates non-i128 casts to emit_cast_instrs.
/// Backends that override `emit_cast` should call this for types they don't handle specially.
pub fn emit_cast_default(
    cg: &mut (impl ArchCodegen + ?Sized),
    dest: &Value,
    src: &Operand,
    from_ty: IrType,
    to_ty: IrType,
) {
    // float/double -> i128/u128: call compiler-rt __fixdfti/__fixsfti/__fixunsdfti/__fixunssfti
    if is_i128_type(to_ty) && from_ty.is_float() {
        let to_signed = to_ty.is_signed();
        cg.emit_float_to_i128_call(src, to_signed, from_ty);
        cg.emit_store_acc_pair(dest);
        return;
    }
    // i128/u128 -> float/double: call compiler-rt __floattidf/__floattisf/__floatuntidf/__floatuntisf
    if is_i128_type(from_ty) && to_ty.is_float() {
        let from_signed = from_ty.is_signed();
        cg.emit_i128_to_float_call(src, from_signed, to_ty);
        cg.emit_store_result(dest);
        return;
    }
    // integer -> i128/u128: widen to 64-bit then sign/zero extend high half
    if is_i128_type(to_ty) && !is_i128_type(from_ty) {
        cg.emit_load_operand(src);
        if from_ty.size() < 8 {
            let widen_to = if from_ty.is_signed() {
                IrType::I64
            } else {
                IrType::U64
            };
            cg.emit_cast_instrs(from_ty, widen_to);
        }
        if from_ty.is_signed() {
            cg.emit_sign_extend_acc_high();
        } else {
            cg.emit_zero_acc_high();
        }
        cg.emit_store_acc_pair(dest);
        return;
    }
    // i128/u128 -> integer: truncate (just use low 64 bits)
    if is_i128_type(from_ty) && !is_i128_type(to_ty) {
        cg.emit_load_acc_pair(src);
        if to_ty.size() < 8 {
            cg.emit_cast_instrs(IrType::I64, to_ty);
        }
        cg.emit_store_result(dest);
        return;
    }
    if is_i128_type(from_ty) && is_i128_type(to_ty) {
        cg.emit_load_acc_pair(src);
        cg.emit_store_acc_pair(dest);
        return;
    }
    cg.emit_load_operand(src);
    cg.emit_cast_instrs(from_ty, to_ty);
    cg.emit_store_result(dest);
}

/// Default unary operation implementation: dispatches i128 → F128 neg → float → int
/// to arch-specific primitives.
/// Backends that override `emit_unaryop` should call this for types they don't handle specially.
pub fn emit_unaryop_default(
    cg: &mut (impl ArchCodegen + ?Sized),
    dest: &Value,
    op: IrUnaryOp,
    src: &Operand,
    ty: IrType,
) {
    // IsConstant should have been resolved by constant folding.
    // If it survived to codegen, the operand was not constant, so emit 0.
    if op == IrUnaryOp::IsConstant {
        cg.emit_load_operand(&Operand::Const(IrConst::I32(0)));
        cg.emit_store_result(dest);
        return;
    }
    if is_i128_type(ty) {
        cg.emit_load_acc_pair(src);
        match op {
            IrUnaryOp::Neg => cg.emit_i128_neg(),
            IrUnaryOp::Not => cg.emit_i128_not(),
            _ => {}
        }
        cg.emit_store_acc_pair(dest);
        return;
    }
    if ty == IrType::F128 && op == IrUnaryOp::Neg {
        cg.emit_f128_neg(dest, src);
        return;
    }
    cg.emit_load_operand(src);
    if ty.is_float() {
        match op {
            IrUnaryOp::Neg => cg.emit_float_neg(ty),
            IrUnaryOp::Not => cg.emit_int_not(ty),
            _ => {}
        }
    } else {
        match op {
            IrUnaryOp::Neg => cg.emit_int_neg(ty),
            IrUnaryOp::Not => cg.emit_int_not(ty),
            IrUnaryOp::Clz => cg.emit_int_clz(ty),
            IrUnaryOp::Ctz => cg.emit_int_ctz(ty),
            IrUnaryOp::Bswap => cg.emit_int_bswap(ty),
            IrUnaryOp::Popcount => cg.emit_int_popcount(ty),
            IrUnaryOp::IsConstant => unreachable!("IsConstant handled above"),
        }
    }
    cg.emit_store_result(dest);
}

/// Default return implementation: loads value, moves to appropriate return register,
/// and emits epilogue. Backends that override `emit_return` should call this for
/// cases they don't handle specially.
pub fn emit_return_default(
    cg: &mut (impl ArchCodegen + ?Sized),
    val: Option<&Operand>,
    frame_size: i64,
) {
    if let Some(val) = val {
        let ret_ty = cg.current_return_type();
        if is_i128_type(ret_ty) {
            cg.emit_load_acc_pair(val);
            cg.emit_return_i128_to_regs();
            cg.emit_epilogue_and_ret(frame_size);
            return;
        }
        cg.emit_load_operand(val);
        if ret_ty.is_long_double() {
            cg.emit_return_f128_to_reg();
        } else if ret_ty == IrType::F32 {
            cg.emit_return_f32_to_reg();
        } else if ret_ty.is_float() {
            cg.emit_return_f64_to_reg();
        } else {
            cg.emit_return_int_to_reg();
        }
    }
    cg.emit_epilogue_and_ret(frame_size);
}
