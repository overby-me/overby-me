//! Shared F128 (IEEE 754 binary128) soft-float orchestration for ARM and RISC-V.
//!
//! ARM and RISC-V both lack hardware quad-precision FP, so all F128 operations
//! go through compiler-rt/libgcc soft-float library calls. The orchestration
//! logic (load operand, save to temp, shuffle args, call libcall, convert result)
//! is identical between the two; only the register names, instruction mnemonics,
//! and F128 register representation differ:
//!
//! - **ARM**: F128 lives in a single NEON Q register (q0/q1). Moving between
//!   arg positions is `mov v1.16b, v0.16b`. Sign bit flip uses `mov`+`eor`+`mov`
//!   on the high lane.
//! - **RISC-V**: F128 lives in a GP register pair (a0:a1 / a2:a3). Moving between
//!   arg positions is `mv a2, a0; mv a3, a1`. Sign bit flip uses `li`+`slli`+`xor`
//!   on the high register.
//!
//! The `F128SoftFloat` trait captures arch-specific primitives, and the `f128_*`
//! free functions implement the shared orchestration once. This covers:
//!
//! - **Operand loading** (`f128_operand_to_arg1`): load F128 with full precision
//! - **Store/load dispatch** (`f128_emit_store`, `f128_emit_load`, etc.): the
//!   SlotAddr 4-way dispatch for F128 store/load/store_with_offset/load_with_offset
//! - **Cast dispatch** (`f128_emit_cast`): int<->F128 and float<->F128 casts
//! - **Binop dispatch** (`f128_emit_binop`): F128 arithmetic via libcalls
//! - **Comparison** (`f128_cmp`): F128 comparison via libcalls
//! - **Negation** (`f128_neg`): sign bit flip

use crate::backend::cast::FloatOp;
use crate::backend::state::{SlotAddr, StackSlot};
use crate::common::types::IrType;
use crate::ir::reexports::{IrCmpOp, IrConst, Operand, Value};

/// Arch-specific primitives for F128 soft-float operations.
///
/// Each method emits a small sequence of instructions (1-5 lines) specific to
/// the architecture. The shared orchestration functions call these in the right
/// order to implement full-precision F128 loads, stores, arithmetic, and comparisons.
pub trait F128SoftFloat {
    // --- State access ---

    /// Access the codegen state (for emit, get_slot, get_f128_source, etc.).
    fn state(&mut self) -> &mut crate::backend::state::CodegenState;

    /// Get the stack slot for a value (delegates to state().get_slot).
    fn f128_get_slot(&self, val_id: u32) -> Option<StackSlot>;

    /// Get the f128 load source tracking for a value.
    fn f128_get_source(&self, val_id: u32) -> Option<(u32, i64, bool)>;

    /// Resolve a value's slot address (Direct/Indirect/OverAligned).
    fn f128_resolve_slot_addr(&self, val_id: u32) -> Option<SlotAddr>;

    // --- Loading f128 constants ---

    /// Load an f128 constant (given as lo:hi u64 halves) into the first argument
    /// position (ARM: q0 via x0/x1+fmov; RISC-V: a0:a1 via li).
    fn f128_load_const_to_arg1(&mut self, lo: u64, hi: u64);

    // --- Loading f128 from memory ---

    /// Load f128 (16 bytes) from an indirect source into arg1 position.
    /// The address register (ARM: x17, RISC-V: t5) already points to the data.
    fn f128_load_16b_from_addr_reg_to_arg1(&mut self);

    /// Load f128 from a slot at the given frame-relative offset into arg1 position.
    /// (ARM: `ldr q0, [sp, #offset]`; RISC-V: `ld a0, offset(s0); ld a1, offset+8(s0)`)
    fn f128_load_from_frame_offset_to_arg1(&mut self, offset: i64);

    // --- Address computation ---

    /// Load a pointer from a slot into the address register (ARM: x17, RISC-V: t5).
    /// For allocas, computes the address of the alloca; for non-allocas, loads the pointer value.
    fn f128_load_ptr_to_addr_reg(&mut self, slot: StackSlot, val_id: u32);

    /// Add an offset to the address register (ARM: x17, RISC-V: t5).
    fn f128_add_offset_to_addr_reg(&mut self, offset: i64);

    /// Compute the aligned address for an over-aligned alloca into the address register.
    fn f128_alloca_aligned_addr(&mut self, slot: StackSlot, val_id: u32);

    // --- Fallback: f64 -> f128 conversion ---

    /// Load an operand as an f64 bit pattern into the accumulator register
    /// (ARM: operand_to_x0; RISC-V: operand_to_t0), then convert to f128 via
    /// __extenddftf2. After this call, arg1 holds the f128 value.
    fn f128_load_operand_and_extend(&mut self, op: &Operand);

    // --- Arg shuffling ---

    /// Move the f128 value from arg1 position to arg2 position.
    /// (ARM: `mov v1.16b, v0.16b`; RISC-V: `mv a2, a0; mv a3, a1`)
    fn f128_move_arg1_to_arg2(&mut self);

    /// Save the f128 in arg1 to the stack pointer (16 bytes at sp).
    /// Used as temp storage when we need to load both operands.
    /// (ARM: `str q0, [sp]`; RISC-V: `sd a0, 0(sp); sd a1, 8(sp)`)
    fn f128_save_arg1_to_sp(&mut self);

    /// Reload the f128 from the stack pointer back into arg1.
    /// (ARM: `ldr q0, [sp]`; RISC-V: `ld a0, 0(sp); ld a1, 8(sp)`)
    fn f128_reload_arg1_from_sp(&mut self);

    // --- Stack temp allocation ---

    /// Allocate temp stack space (16 bytes). (ARM: `sub sp, sp, #16`; RISC-V: `addi sp, sp, -16`)
    fn f128_alloc_temp_16(&mut self);

    /// Free temp stack space (16 bytes). (ARM: `add sp, sp, #16`; RISC-V: `addi sp, sp, 16`)
    fn f128_free_temp_16(&mut self);

    // --- Calls ---

    /// Emit a call to a named library function.
    /// (ARM: `bl <name>`; RISC-V: `call <name>`)
    fn f128_call(&mut self, name: &str);

    // --- Result handling ---

    /// Convert f128 result (in arg1) to f64 approximation and move to accumulator.
    /// Calls __trunctfdf2, then moves the f64 from float reg to GP acc.
    /// (ARM: `bl __trunctfdf2; fmov x0, d0`; RISC-V: `call __trunctfdf2; fmv.x.d t0, fa0`)
    fn f128_truncate_result_to_acc(&mut self);

    // --- F128 store to slot ---

    /// Store f128 constant halves (lo, hi) directly to a stack slot.
    /// (ARM: load imm + str; RISC-V: li + sd)
    fn f128_store_const_halves_to_slot(&mut self, lo: u64, hi: u64, slot: StackSlot);

    /// Store f128 from arg1 to a stack slot (16 bytes).
    /// (ARM: `str q0, [sp, #offset]`; RISC-V: `sd a0, offset(s0); sd a1, offset+8(s0)`)
    fn f128_store_arg1_to_slot(&mut self, slot: StackSlot);

    /// Load f128 (16 bytes) from a source slot at offset, store to dest slot.
    /// This is a direct memory-to-memory copy (load then store, 16 bytes).
    fn f128_copy_slot_to_slot(&mut self, src_offset: i64, dest_slot: StackSlot);

    /// Load f128 from addr_reg, store to dest slot.
    fn f128_copy_addr_reg_to_slot(&mut self, dest_slot: StackSlot);

    // --- F128 store to address ---

    /// Store f128 constant halves to address in addr_reg.
    fn f128_store_const_halves_to_addr(&mut self, lo: u64, hi: u64);

    /// Save addr_reg to a scratch register before potentially clobbering it.
    fn f128_save_addr_reg(&mut self);

    /// Load f128 from source slot offset, store to saved addr.
    fn f128_copy_slot_to_saved_addr(&mut self, src_offset: i64);

    /// Load f128 from addr_reg (source), store to saved addr (dest).
    /// The source address is in addr_reg, the dest address was saved by f128_save_addr_reg.
    fn f128_copy_addr_reg_to_saved_addr(&mut self);

    /// Store the f128 in arg1 to the saved address (from f128_save_addr_reg).
    fn f128_store_arg1_to_saved_addr(&mut self);

    // --- F128 negation ---

    /// Flip the sign bit of the f128 in arg1.
    /// (ARM: `mov x0, v0.d[1]; eor x0, x0, #0x80...; mov v0.d[1], x0`)
    /// (RISC-V: `li t0, 1; slli t0, t0, 63; xor a1, a1, t0`)
    fn f128_flip_sign_bit(&mut self);

    // --- Comparison result mapping ---

    /// Map a comparison libcall result to a boolean in the accumulator.
    /// ARM uses `cmp w0, #0; cset x0, <cond>`.
    /// RISC-V uses seqz/snez/slti/slt/xori sequences.
    fn f128_cmp_result_to_bool(&mut self, kind: crate::backend::cast::F128CmpKind);

    // --- Result store ---

    /// Store the accumulator to dest (ARM: store_x0_to; RISC-V: store_t0_to).
    fn f128_store_acc_to_dest(&mut self, dest: &Value);

    /// Track that dest has full f128 data in its own slot (for subsequent loads).
    fn f128_track_self(&mut self, dest_id: u32);

    /// Set the accumulator cache to hold the given value (without writing to slot).
    fn f128_set_acc_cache(&mut self, dest_id: u32);

    /// Has dynamic alloca flag (needed for ARM's SP-relative addressing workaround).
    /// Returns the current value and sets it to the given value.
    fn f128_set_dyn_alloca(&mut self, val: bool) -> bool;

    // --- Store/Load dispatch primitives (used by shared emit_store/emit_load) ---

    /// Move a callee-saved register value into the address register (ARM: x17, RISC-V: t5).
    /// Called when a pointer is register-allocated.
    fn f128_move_callee_reg_to_addr_reg(&mut self, val_id: u32) -> bool;

    /// Move the computed aligned address into the address register.
    /// ARM needs `mov x17, x9` because x9 is the alloca addr register and x17 is the
    /// F128 addr register. RISC-V uses t5 for both, so this is a no-op.
    fn f128_move_aligned_to_addr_reg(&mut self) {}

    /// Load a pointer from a (non-alloca) slot into the address register.
    /// This differs from `f128_load_ptr_to_addr_reg` in that it always
    /// loads the pointer value, never computes an alloca address.
    fn f128_load_indirect_ptr_to_addr_reg(&mut self, slot: StackSlot, val_id: u32);

    /// Load f128 from addr_reg, convert to f64 approx, store to dest.
    /// This is the "load through pointer" path: load 16 bytes from the address
    /// register, call __trunctfdf2, move result to accumulator, store to dest.
    fn f128_load_from_addr_reg_to_acc(&mut self, dest: &Value);

    /// Load f128 from a direct alloca slot, convert to f64 approx, store to accumulator.
    fn f128_load_from_direct_slot_to_acc(&mut self, slot: StackSlot);

    /// Store arg1 (f128 result) to dest slot and produce f64 approximation.
    /// This is the common epilogue for cast/binop results: store full f128 to
    /// dest slot, track self, call __trunctfdf2, update cache. Does NOT store
    /// f64 back to the slot (that would overwrite the full-precision f128).
    fn f128_store_result_and_truncate(&mut self, dest: &Value);

    /// Load the accumulator value and move it to the first integer argument register.
    /// (ARM: already in x0; RISC-V: `mv a0, t0`)
    fn f128_move_acc_to_arg0(&mut self);

    /// Move the f128 return value from arg1 to accumulator as f64 result.
    /// (ARM: result already in x0 from __fixtfdi; RISC-V: `mv t0, a0`)
    fn f128_move_arg0_to_acc(&mut self);

    /// Load an operand into the accumulator.
    /// (ARM: operand_to_x0; RISC-V: operand_to_t0)
    fn f128_load_operand_to_acc(&mut self, op: &Operand);

    /// Sign-extend a sub-64-bit signed integer in the accumulator.
    /// (ARM: sxtb/sxth/sxtw; RISC-V: slli+srai/sext.w)
    fn f128_sign_extend_acc(&mut self, from_size: usize);

    /// Zero-extend a sub-64-bit unsigned integer in the accumulator.
    /// (ARM: and/mov w0,w0; RISC-V: andi/slli+srli)
    fn f128_zero_extend_acc(&mut self, from_size: usize);

    /// Narrow the accumulator to a smaller integer type using emit_cast_instrs.
    fn f128_narrow_acc(&mut self, to_ty: IrType);

    /// Move a float value from the accumulator to the float argument register
    /// and extend it from F32 to F128 or F64 to F128.
    /// (ARM: `fmov s0/d0, w0/x0; bl __extendsftf2/__extenddftf2`)
    /// (RISC-V: `fmv.w.x/fmv.d.x fa0, t0; call __extendsftf2/__extenddftf2`)
    fn f128_extend_float_to_f128(&mut self, from_ty: IrType);

    /// Convert an F128 in arg1 to F32 or F64 and move to accumulator.
    /// (ARM: `bl __trunctfsf2; fmov w0, s0` or `bl __trunctfdf2; fmov x0, d0`)
    /// (RISC-V: `call __trunctfsf2; fmv.x.w t0, fa0` or `call __trunctfdf2; fmv.x.d t0, fa0`)
    fn f128_truncate_to_float_acc(&mut self, to_ty: IrType);

    /// Check if this backend has the `is_alloca` method accessible.
    /// Both ARM and RISC-V do, so this just delegates.
    fn f128_is_alloca(&self, val_id: u32) -> bool;
}

// =============================================================================
// Shared orchestration functions
// =============================================================================

/// Load an F128 operand into the first argument position with full precision.
///
/// Three paths:
/// 1. **Constant**: load f128 bytes directly as lo:hi.
/// 2. **Tracked value**: load full 16-byte f128 from the original memory location.
/// 3. **Fallback**: load f64 approximation and extend via __extenddftf2.
pub fn f128_operand_to_arg1<T: F128SoftFloat + ?Sized>(cg: &mut T, op: &Operand) {
    // Path 1: F128 constant with full-precision f128 bytes.
    if let Operand::Const(IrConst::LongDouble(_, f128_bytes)) = op {
        let lo = u64::from_le_bytes(f128_bytes[0..8].try_into().unwrap());
        let hi = u64::from_le_bytes(f128_bytes[8..16].try_into().unwrap());
        cg.f128_load_const_to_arg1(lo, hi);
        // The accumulator register (ARM: x0) was clobbered with constant data.
        // Invalidate the cache so subsequent loads don't get a stale hit.
        cg.state().reg_cache.invalidate_all();
        return;
    }

    // Path 2: Value with tracked f128 source (preserves full precision).
    if let Operand::Value(v) = op {
        if let Some((src_id, offset, is_indirect)) = cg.f128_get_source(v.0) {
            if is_indirect {
                if let Some(slot) = cg.f128_get_slot(src_id) {
                    cg.f128_load_ptr_to_addr_reg(slot, src_id);
                    if offset != 0 {
                        cg.f128_add_offset_to_addr_reg(offset);
                    }
                    cg.f128_load_16b_from_addr_reg_to_arg1();
                    return;
                }
            } else {
                let addr = cg.f128_resolve_slot_addr(src_id);
                if let Some(addr) = addr {
                    match addr {
                        SlotAddr::Direct(slot) | SlotAddr::Indirect(slot) => {
                            let effective = slot.0 + offset;
                            cg.f128_load_from_frame_offset_to_arg1(effective);
                        }
                        SlotAddr::OverAligned(slot, id) => {
                            cg.f128_alloca_aligned_addr(slot, id);
                            if offset != 0 {
                                cg.f128_add_offset_to_addr_reg(offset);
                            }
                            cg.f128_load_16b_from_addr_reg_to_arg1();
                        }
                    }
                    return;
                }
            }
        }
    }

    // Path 3: Fallback - load f64 approximation and convert to f128.
    cg.f128_load_operand_and_extend(op);
}

/// Store an F128 value to a direct stack slot.
///
/// Three paths:
/// 1. **Constant**: store lo:hi halves directly.
/// 2. **Tracked value**: copy 16 bytes from the tracked source.
/// 3. **Fallback**: convert f64 to f128 via __extenddftf2, store result.
pub fn f128_store_to_slot<T: F128SoftFloat + ?Sized>(cg: &mut T, val: &Operand, slot: StackSlot) {
    // Path 1: F128 constant.
    if let Some((lo, hi)) = crate::backend::cast::f128_const_halves(val) {
        cg.f128_store_const_halves_to_slot(lo, hi, slot);
        // The accumulator register was clobbered with constant data.
        cg.state().reg_cache.invalidate_all();
        return;
    }

    // Path 2: Tracked value with full f128 source.
    if let Operand::Value(v) = val {
        if let Some((src_id, offset, is_indirect)) = cg.f128_get_source(v.0) {
            if is_indirect {
                if let Some(src_slot) = cg.f128_get_slot(src_id) {
                    cg.f128_load_ptr_to_addr_reg(src_slot, src_id);
                    if offset != 0 {
                        cg.f128_add_offset_to_addr_reg(offset);
                    }
                    cg.f128_copy_addr_reg_to_slot(slot);
                    return;
                }
            } else if let Some(src_slot) = cg.f128_get_slot(src_id) {
                let src_off = src_slot.0 + offset;
                cg.f128_copy_slot_to_slot(src_off, slot);
                return;
            }
        }
    }

    // Path 3: Fallback - extend f64 to f128, store result.
    cg.f128_load_operand_and_extend(val);
    cg.f128_store_arg1_to_slot(slot);
    cg.state().reg_cache.invalidate_all();
}

/// Store an F128 value to the address in the addr register (ARM: x17, RISC-V: t5).
///
/// Three paths:
/// 1. **Constant**: store lo:hi halves directly to addr.
/// 2. **Tracked value**: copy 16 bytes from tracked source to addr.
/// 3. **Fallback**: convert f64 to f128, store result to saved addr.
pub fn f128_store_to_addr_reg<T: F128SoftFloat + ?Sized>(cg: &mut T, val: &Operand) {
    // Path 1: F128 constant.
    if let Some((lo, hi)) = crate::backend::cast::f128_const_halves(val) {
        cg.f128_store_const_halves_to_addr(lo, hi);
        // The accumulator register was clobbered with constant data.
        cg.state().reg_cache.invalidate_all();
        return;
    }

    // Path 2: Tracked value with full f128 source.
    if let Operand::Value(v) = val {
        if let Some((src_id, offset, is_indirect)) = cg.f128_get_source(v.0) {
            if is_indirect {
                if let Some(src_slot) = cg.f128_get_slot(src_id) {
                    cg.f128_save_addr_reg();
                    cg.f128_load_ptr_to_addr_reg(src_slot, src_id);
                    if offset != 0 {
                        cg.f128_add_offset_to_addr_reg(offset);
                    }
                    cg.f128_copy_addr_reg_to_saved_addr();
                    return;
                }
            } else if let Some(src_slot) = cg.f128_get_slot(src_id) {
                let src_off = src_slot.0 + offset;
                cg.f128_save_addr_reg();
                cg.f128_copy_slot_to_saved_addr(src_off);
                return;
            }
        }
    }

    // Path 3: Fallback - save addr, convert f64 to f128, store to saved addr.
    cg.f128_save_addr_reg();
    cg.f128_load_operand_and_extend(val);
    cg.f128_store_arg1_to_saved_addr();
    cg.state().reg_cache.invalidate_all();
}

/// Negate an F128 value with full precision by flipping the IEEE 754 sign bit.
///
/// 1. Load full f128 into arg1.
/// 2. XOR the sign bit (bit 127).
/// 3. Store full f128 result to dest slot.
/// 4. Convert to f64 approximation for register-based data flow.
pub fn f128_neg<T: F128SoftFloat + ?Sized>(cg: &mut T, dest: &Value, src: &Operand) {
    // Step 1: Load full-precision f128 into arg1.
    f128_operand_to_arg1(cg, src);
    // Step 2: Flip the sign bit.
    cg.f128_flip_sign_bit();
    // Step 3: Store full f128 result to dest slot.
    if let Some(dest_slot) = cg.f128_get_slot(dest.0) {
        cg.f128_store_arg1_to_slot(dest_slot);
        cg.f128_track_self(dest.0);
    }
    // Step 4: Convert to f64 approximation in accumulator.
    cg.f128_truncate_result_to_acc();
    cg.state().reg_cache.invalidate_all();
    cg.f128_set_acc_cache(dest.0);
}

/// F128 comparison via soft-float libcalls with full precision.
///
/// 1. Load LHS f128 into arg1, save to stack temp.
/// 2. Load RHS f128 into arg1, move to arg2.
/// 3. Reload LHS from stack temp into arg1.
/// 4. Call comparison libcall.
/// 5. Map result to boolean in accumulator.
pub fn f128_cmp<T: F128SoftFloat + ?Sized>(
    cg: &mut T,
    dest: &Value,
    op: IrCmpOp,
    lhs: &Operand,
    rhs: &Operand,
) {
    // Force frame-pointer-relative addressing during temp allocation
    // (ARM needs this because sub sp breaks sp-relative slot addressing).
    let saved = cg.f128_set_dyn_alloca(true);

    // Step 1: Allocate temp, load LHS, save to sp.
    cg.f128_alloc_temp_16();
    f128_operand_to_arg1(cg, lhs);
    cg.f128_save_arg1_to_sp();

    // Step 2: Load RHS, move to arg2.
    f128_operand_to_arg1(cg, rhs);
    cg.f128_move_arg1_to_arg2();

    // Step 3: Reload LHS from sp into arg1.
    cg.f128_reload_arg1_from_sp();

    // Step 4: Free temp, restore dyn_alloca flag.
    cg.f128_free_temp_16();
    cg.f128_set_dyn_alloca(saved);

    // Step 5: Call comparison libcall and map result.
    let (libcall, kind) = crate::backend::cast::f128_cmp_libcall(op);
    cg.f128_call(libcall);
    cg.f128_cmp_result_to_bool(kind);

    cg.state().reg_cache.invalidate_all();
    cg.f128_store_acc_to_dest(dest);
}

// =============================================================================
// Shared store/load dispatch orchestration
// =============================================================================

/// F128 store dispatch: resolve the pointer's SlotAddr and store 16 bytes.
///
/// Handles four cases: register-allocated pointer, Direct alloca, OverAligned
/// alloca, and Indirect (non-alloca pointer in slot). Each case resolves to
/// either a direct slot store or an address-register store.
pub fn f128_emit_store<T: F128SoftFloat + ?Sized>(cg: &mut T, val: &Operand, ptr: &Value) {
    let is_indirect = !cg.f128_is_alloca(ptr.0);

    // Check if the pointer lives in a callee-saved register.
    if cg.f128_move_callee_reg_to_addr_reg(ptr.0) {
        f128_store_to_addr_reg(cg, val);
        return;
    }

    let addr = cg.f128_resolve_slot_addr(ptr.0);
    if let Some(addr) = addr {
        match addr {
            SlotAddr::Direct(slot) if !is_indirect => {
                f128_store_to_slot(cg, val, slot);
            }
            SlotAddr::Direct(slot) | SlotAddr::Indirect(slot) => {
                cg.f128_load_indirect_ptr_to_addr_reg(slot, ptr.0);
                f128_store_to_addr_reg(cg, val);
            }
            SlotAddr::OverAligned(slot, id) => {
                cg.f128_alloca_aligned_addr(slot, id);
                cg.f128_move_aligned_to_addr_reg();
                f128_store_to_addr_reg(cg, val);
            }
        }
    }
}

/// F128 load dispatch: resolve the pointer's SlotAddr, load 16 bytes,
/// convert to f64 approximation, and store to dest.
///
/// Also tracks the f128 source for full-precision reloads.
pub fn f128_emit_load<T: F128SoftFloat + ?Sized>(cg: &mut T, dest: &Value, ptr: &Value) {
    cg.state().track_f128_load(dest.0, ptr.0, 0);
    let is_indirect = !cg.f128_is_alloca(ptr.0);

    // Check if the pointer lives in a callee-saved register.
    if cg.f128_move_callee_reg_to_addr_reg(ptr.0) {
        cg.f128_load_from_addr_reg_to_acc(dest);
        return;
    }

    let addr = cg.f128_resolve_slot_addr(ptr.0);
    if let Some(addr) = addr {
        match addr {
            SlotAddr::Direct(slot) if !is_indirect => {
                cg.f128_load_from_direct_slot_to_acc(slot);
            }
            SlotAddr::Direct(slot) | SlotAddr::Indirect(slot) => {
                cg.f128_load_indirect_ptr_to_addr_reg(slot, ptr.0);
                cg.f128_load_from_addr_reg_to_acc(dest);
                return;
            }
            SlotAddr::OverAligned(slot, id) => {
                cg.f128_alloca_aligned_addr(slot, id);
                cg.f128_move_aligned_to_addr_reg();
                cg.f128_load_from_addr_reg_to_acc(dest);
                return;
            }
        }
    } else {
        return;
    }
    // Convert f128 to f64, store to dest.
    cg.f128_truncate_result_to_acc();
    cg.state().reg_cache.invalidate_all();
    cg.f128_store_acc_to_dest(dest);
}

/// F128 store with constant offset dispatch.
///
/// Resolves the base pointer's SlotAddr and stores 16 bytes at base + offset.
pub fn f128_emit_store_with_offset<T: F128SoftFloat + ?Sized>(
    cg: &mut T,
    val: &Operand,
    base: &Value,
    offset: i64,
) {
    let is_indirect = !cg.f128_is_alloca(base.0);

    // Check if the base pointer lives in a callee-saved register.
    if cg.f128_move_callee_reg_to_addr_reg(base.0) {
        if offset != 0 {
            cg.f128_add_offset_to_addr_reg(offset);
        }
        f128_store_to_addr_reg(cg, val);
        return;
    }

    let addr = cg.f128_resolve_slot_addr(base.0);
    if let Some(addr) = addr {
        match addr {
            SlotAddr::Direct(slot) if !is_indirect => {
                let folded_slot = StackSlot(slot.0 + offset);
                f128_store_to_slot(cg, val, folded_slot);
            }
            SlotAddr::Direct(slot) | SlotAddr::Indirect(slot) => {
                cg.f128_load_indirect_ptr_to_addr_reg(slot, base.0);
                if offset != 0 {
                    cg.f128_add_offset_to_addr_reg(offset);
                }
                f128_store_to_addr_reg(cg, val);
            }
            SlotAddr::OverAligned(slot, id) => {
                cg.f128_alloca_aligned_addr(slot, id);
                cg.f128_move_aligned_to_addr_reg();
                if offset != 0 {
                    cg.f128_add_offset_to_addr_reg(offset);
                }
                f128_store_to_addr_reg(cg, val);
            }
        }
    }
}

/// F128 load with constant offset dispatch.
///
/// Resolves the base pointer's SlotAddr, loads 16 bytes at base + offset,
/// converts to f64 approximation, and stores to dest.
pub fn f128_emit_load_with_offset<T: F128SoftFloat + ?Sized>(
    cg: &mut T,
    dest: &Value,
    base: &Value,
    offset: i64,
) {
    cg.state().track_f128_load(dest.0, base.0, offset);
    let is_indirect = !cg.f128_is_alloca(base.0);

    // Check if the base pointer lives in a callee-saved register.
    let loaded = if cg.f128_move_callee_reg_to_addr_reg(base.0) {
        if offset != 0 {
            cg.f128_add_offset_to_addr_reg(offset);
        }
        cg.f128_load_from_addr_reg_to_acc(dest);
        return; // load_from_addr_reg_to_acc handles truncation + store
    } else {
        let addr = cg.f128_resolve_slot_addr(base.0);
        if let Some(addr) = addr {
            match addr {
                SlotAddr::Direct(slot) if !is_indirect => {
                    let folded_slot = StackSlot(slot.0 + offset);
                    cg.f128_load_from_direct_slot_to_acc(folded_slot);
                }
                SlotAddr::Direct(slot) | SlotAddr::Indirect(slot) => {
                    cg.f128_load_indirect_ptr_to_addr_reg(slot, base.0);
                    if offset != 0 {
                        cg.f128_add_offset_to_addr_reg(offset);
                    }
                    cg.f128_load_from_addr_reg_to_acc(dest);
                    return;
                }
                SlotAddr::OverAligned(slot, id) => {
                    cg.f128_alloca_aligned_addr(slot, id);
                    cg.f128_move_aligned_to_addr_reg();
                    if offset != 0 {
                        cg.f128_add_offset_to_addr_reg(offset);
                    }
                    cg.f128_load_from_addr_reg_to_acc(dest);
                    return;
                }
            }
            true
        } else {
            false
        }
    };
    if loaded {
        cg.f128_truncate_result_to_acc();
        cg.state().reg_cache.invalidate_all();
        cg.f128_store_acc_to_dest(dest);
    }
}

// =============================================================================
// Shared cast orchestration
// =============================================================================

/// F128 cast dispatch: handles all F128-related casts (int<->F128, float<->F128).
///
/// Returns `true` if the cast was handled, `false` if the caller should use the
/// default cast path.
pub fn f128_emit_cast<T: F128SoftFloat + ?Sized>(
    cg: &mut T,
    dest: &Value,
    src: &Operand,
    from_ty: IrType,
    to_ty: IrType,
) -> bool {
    let is_i128 = |ty: IrType| ty == IrType::I128 || ty == IrType::U128;

    // int -> F128
    if to_ty == IrType::F128 && !from_ty.is_float() && !is_i128(from_ty) {
        cg.f128_load_operand_to_acc(src);
        if from_ty.is_signed() {
            cg.f128_sign_extend_acc(from_ty.size());
            cg.f128_move_acc_to_arg0();
            cg.f128_call("__floatditf");
        } else {
            cg.f128_zero_extend_acc(from_ty.size());
            cg.f128_move_acc_to_arg0();
            cg.f128_call("__floatunditf");
        }
        cg.state().reg_cache.invalidate_all();
        cg.f128_store_result_and_truncate(dest);
        return true;
    }

    // F128 -> int
    if from_ty == IrType::F128 && !to_ty.is_float() && !is_i128(to_ty) {
        f128_operand_to_arg1(cg, src);
        if to_ty.is_unsigned() || to_ty == IrType::Ptr {
            cg.f128_call("__fixunstfdi");
        } else {
            cg.f128_call("__fixtfdi");
        }
        cg.f128_move_arg0_to_acc();
        cg.state().reg_cache.invalidate_all();
        if to_ty.size() < 8 {
            cg.f128_narrow_acc(to_ty);
        }
        cg.f128_store_acc_to_dest(dest);
        return true;
    }

    // float -> F128
    if to_ty == IrType::F128 && from_ty.is_float() {
        cg.f128_load_operand_to_acc(src);
        cg.f128_extend_float_to_f128(from_ty);
        cg.state().reg_cache.invalidate_all();
        cg.f128_store_result_and_truncate(dest);
        return true;
    }

    // F128 -> float
    if from_ty == IrType::F128 && to_ty.is_float() {
        f128_operand_to_arg1(cg, src);
        cg.f128_truncate_to_float_acc(to_ty);
        cg.state().reg_cache.invalidate_all();
        cg.f128_store_acc_to_dest(dest);
        return true;
    }

    false
}

// =============================================================================
// Shared binop orchestration
// =============================================================================

/// F128 binary operation via soft-float libcalls with full precision.
///
/// 1. Allocate stack temp.
/// 2. Load LHS f128 into arg1, save to temp.
/// 3. Load RHS f128 into arg1, move to arg2.
/// 4. Reload LHS from temp into arg1.
/// 5. Call arithmetic libcall.
/// 6. Free temp.
/// 7. Store full f128 result to dest slot, produce f64 approximation.
pub fn f128_emit_binop<T: F128SoftFloat + ?Sized>(
    cg: &mut T,
    dest: &Value,
    op: FloatOp,
    lhs: &Operand,
    rhs: &Operand,
) {
    let libcall = match op {
        FloatOp::Add => "__addtf3",
        FloatOp::Sub => "__subtf3",
        FloatOp::Mul => "__multf3",
        FloatOp::Div => "__divtf3",
    };

    // Force frame-pointer-relative addressing during temp allocation.
    let saved = cg.f128_set_dyn_alloca(true);

    // Step 1: Allocate temp stack space for saving LHS.
    cg.f128_alloc_temp_16();

    // Step 2: Load LHS f128, save to temp.
    f128_operand_to_arg1(cg, lhs);
    cg.f128_save_arg1_to_sp();

    // Step 3: Load RHS f128, move to arg2.
    f128_operand_to_arg1(cg, rhs);
    cg.f128_move_arg1_to_arg2();

    // Step 4: Reload LHS from temp.
    cg.f128_reload_arg1_from_sp();

    // Step 5: Free temp, restore flag.
    cg.f128_free_temp_16();
    cg.f128_set_dyn_alloca(saved);

    // Step 6: Call the arithmetic libcall.
    cg.f128_call(libcall);

    // Step 7: Store full f128 result and produce f64 approximation.
    cg.f128_store_result_and_truncate(dest);
}
