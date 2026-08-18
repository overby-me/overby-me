//! RISC-V sub-word atomic operations, atomic load/store/RMW/CAS, fence,
//! and software implementations of CLZ/CTZ/BSWAP/POPCOUNT builtins.
//!
//! RISC-V only provides word (32-bit) and doubleword (64-bit) atomic instructions
//! (AMO and LR/SC). For sub-word types (I8/U8/I16/U16), this module implements
//! atomic RMW and CAS operations using word-aligned LR.W/SC.W loops with bit
//! masking. It also provides software loop-based implementations of count leading
//! zeros, count trailing zeros, byte swap, and population count for targets that
//! lack the Zbb extension.

use super::emit::RiscvCodegen;
use crate::backend::state::CodegenState;
use crate::common::types::IrType;
use crate::ir::reexports::{AtomicOrdering, AtomicRmwOp, Operand, Value};

impl RiscvCodegen {
    /// Get the AMO ordering suffix.
    pub(super) fn amo_ordering(ordering: AtomicOrdering) -> &'static str {
        match ordering {
            AtomicOrdering::Relaxed => "",
            AtomicOrdering::Acquire => ".aq",
            AtomicOrdering::Release => ".rl",
            AtomicOrdering::AcqRel => ".aqrl",
            AtomicOrdering::SeqCst => ".aqrl",
        }
    }

    /// Get the AMO width suffix for word/doubleword operations.
    /// Sub-word types (I8/U8/I16/U16) should use the sub-word atomic helpers instead.
    pub(super) fn amo_width_suffix(ty: IrType) -> &'static str {
        match ty {
            IrType::I32 | IrType::U32 => "w",
            _ => "d",
        }
    }

    /// Sign-extend result for sub-word types after atomic ops.
    pub(super) fn sign_extend_riscv(state: &mut CodegenState, ty: IrType) {
        match ty {
            IrType::I8 => {
                state.emit("    slli t0, t0, 56");
                state.emit("    srai t0, t0, 56");
            }
            IrType::U8 => {
                state.emit("    andi t0, t0, 0xff");
            }
            IrType::I16 => {
                state.emit("    slli t0, t0, 48");
                state.emit("    srai t0, t0, 48");
            }
            IrType::U16 => {
                state.emit("    slli t0, t0, 48");
                state.emit("    srli t0, t0, 48");
            }
            IrType::I32 => {
                state.emit("    sext.w t0, t0");
            }
            _ => {}
        }
    }

    /// Check if a type requires sub-word atomic handling on RISC-V.
    /// RISC-V only has word (32-bit) and doubleword (64-bit) atomic instructions.
    pub(super) fn is_subword_type(ty: IrType) -> bool {
        matches!(ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16)
    }

    /// Get the bit width of a sub-word type.
    pub(super) fn subword_bits(ty: IrType) -> u32 {
        match ty {
            IrType::I8 | IrType::U8 => 8,
            IrType::I16 | IrType::U16 => 16,
            _ => unreachable!("subword_bits called with non-subword type: {:?}", ty),
        }
    }

    /// Emit sub-word atomic RMW using word-aligned LR.W/SC.W with bit masking.
    ///
    /// On entry: t1 = original ptr, t2 = value to apply
    /// On exit: t0 = old sub-word value (not sign-extended yet)
    ///
    /// Strategy: Align the address to a word boundary, compute shift and mask,
    /// then use LR.W/SC.W loop operating on the containing word while only
    /// modifying the sub-word field.
    ///
    /// Register usage (all caller-saved):
    ///   a2 = word-aligned address
    ///   a3 = bit shift amount
    ///   a4 = shifted mask for sub-word field
    ///   a5 = inverted mask (~mask)
    ///   t0 = loaded word (old value)
    ///   t2 = shifted value operand
    ///   t3 = temporary, t4 = new word to store, t5 = SC result flag
    pub(super) fn emit_subword_atomic_rmw(&mut self, op: AtomicRmwOp, ty: IrType, aq_rl: &str) {
        let bits = Self::subword_bits(ty);
        let loop_label = self.state.fresh_label("sw_rmw_loop");
        let done_label = self.state.fresh_label("sw_rmw_done");

        // a2 = word-aligned address (ptr & ~3)
        self.state.emit("    andi a2, t1, -4");
        // a3 = byte offset within word: (ptr & 3)
        self.state.emit("    andi a3, t1, 3");
        // a3 = bit shift = byte_offset * 8
        self.state.emit("    slli a3, a3, 3");
        // a4 = mask for the sub-word field (e.g., 0xFF or 0xFFFF shifted into position)
        if bits == 8 {
            self.state.emit("    li a4, 0xff");
        } else {
            // 16-bit: can't use andi with 0xffff, load it
            self.state.emit("    lui a4, 16"); // a4 = 0x10000
            self.state.emit("    addiw a4, a4, -1"); // a4 = 0xFFFF
        }
        self.state.emit("    sllw a4, a4, a3"); // a4 = mask << shift
                                                // a5 = ~mask (inverted mask for clearing the field)
        self.state.emit("    not a5, a4");
        // Shift the value into position: t2 = (val & field_mask) << shift
        if bits == 8 {
            self.state.emit("    andi t2, t2, 0xff");
        } else {
            self.state.emit("    slli t2, t2, 48");
            self.state.emit("    srli t2, t2, 48");
        }
        self.state.emit("    sllw t2, t2, a3"); // t2 = val << shift

        // LR/SC loop
        self.state.emit_fmt(format_args!("{}:", loop_label));
        self.state
            .emit_fmt(format_args!("    lr.w{} t0, (a2)", aq_rl));

        match op {
            AtomicRmwOp::Xchg | AtomicRmwOp::TestAndSet => {
                // new_word = (old_word & ~mask) | (new_val & mask)
                if matches!(op, AtomicRmwOp::TestAndSet) {
                    // Override: set the byte to 1
                    self.state.emit("    li t3, 1");
                    self.state.emit("    sllw t3, t3, a3");
                } else {
                    self.state.emit("    mv t3, t2");
                }
                self.state.emit("    and t4, t0, a5"); // clear old field
                self.state.emit("    or t4, t4, t3"); // insert new value
            }
            AtomicRmwOp::Add => {
                // Extract old sub-word, add val, insert result
                self.state.emit("    and t3, t0, a4"); // t3 = old field (shifted)
                self.state.emit("    add t3, t3, t2"); // t3 = old + val (shifted)
                self.state.emit("    and t3, t3, a4"); // mask to field width
                self.state.emit("    and t4, t0, a5"); // clear old field
                self.state.emit("    or t4, t4, t3"); // insert new value
            }
            AtomicRmwOp::Sub => {
                // Extract old sub-word, subtract val, insert result
                self.state.emit("    and t3, t0, a4"); // t3 = old field (shifted)
                self.state.emit("    sub t3, t3, t2"); // t3 = old - val (shifted)
                self.state.emit("    and t3, t3, a4"); // mask to field width
                self.state.emit("    and t4, t0, a5"); // clear old field
                self.state.emit("    or t4, t4, t3"); // insert new value
            }
            AtomicRmwOp::And => {
                // new_field = old_field & val_field
                // For AND: bits outside the field should remain unchanged.
                // new_word = old_word & (val_shifted | ~mask)
                self.state.emit("    or t3, t2, a5"); // val_shifted | ~mask
                self.state.emit("    and t4, t0, t3"); // old & (val | ~mask)
            }
            AtomicRmwOp::Or => {
                // new_word = old_word | (val_shifted & mask)
                self.state.emit("    and t3, t2, a4"); // val & mask (already masked, but safe)
                self.state.emit("    or t4, t0, t3");
            }
            AtomicRmwOp::Xor => {
                // new_word = old_word ^ (val_shifted & mask)
                self.state.emit("    and t3, t2, a4");
                self.state.emit("    xor t4, t0, t3");
            }
            AtomicRmwOp::Nand => {
                // new_field = ~(old_field & val_field)
                self.state.emit("    and t3, t0, a4"); // old field
                self.state.emit("    and t3, t3, t2"); // old & val (shifted)
                self.state.emit("    not t3, t3"); // ~(old & val) - full word invert
                self.state.emit("    and t3, t3, a4"); // mask to field
                self.state.emit("    and t4, t0, a5"); // clear old field
                self.state.emit("    or t4, t4, t3"); // insert new value
            }
        }

        // SC: rd (t5) must differ from rs2 (t4) per RISC-V spec
        self.state
            .emit_fmt(format_args!("    sc.w{} t5, t4, (a2)", aq_rl));
        self.state
            .emit_fmt(format_args!("    bnez t5, {}", loop_label));
        self.state.emit_fmt(format_args!("{}:", done_label));
        // Extract the old sub-word value: t0 = (old_word >> shift) & field_mask
        self.state.emit("    srlw t0, t0, a3");
        if bits == 8 {
            self.state.emit("    andi t0, t0, 0xff");
        } else {
            self.state.emit("    slli t0, t0, 48");
            self.state.emit("    srli t0, t0, 48");
        }
    }

    /// Emit sub-word atomic CAS using word-aligned LR.W/SC.W with bit masking.
    ///
    /// On entry: t1 = ptr, t2 = expected, t3 = desired
    /// On exit: t0 = old sub-word value (for !returns_bool) or success flag
    ///
    /// Register usage (all caller-saved):
    ///   a2 = word-aligned address
    ///   a3 = bit shift amount
    ///   a4 = shifted mask
    ///   a5 = inverted mask (~mask)
    ///   t0 = loaded word
    ///   t2 = shifted expected, t3 = shifted desired
    ///   t4 = new word to store, t5 = SC result flag
    pub(super) fn emit_subword_atomic_cmpxchg(
        &mut self,
        ty: IrType,
        aq_rl: &str,
        returns_bool: bool,
    ) {
        let bits = Self::subword_bits(ty);
        let loop_label = self.state.fresh_label("sw_cas_loop");
        let fail_label = self.state.fresh_label("sw_cas_fail");
        let done_label = self.state.fresh_label("sw_cas_done");

        // a2 = word-aligned address
        self.state.emit("    andi a2, t1, -4");
        // a3 = bit shift
        self.state.emit("    andi a3, t1, 3");
        self.state.emit("    slli a3, a3, 3");
        // a4 = mask
        if bits == 8 {
            self.state.emit("    li a4, 0xff");
        } else {
            self.state.emit("    lui a4, 16");
            self.state.emit("    addiw a4, a4, -1");
        }
        self.state.emit("    sllw a4, a4, a3");
        // a5 = ~mask
        self.state.emit("    not a5, a4");
        // Mask and shift expected and desired
        if bits == 8 {
            self.state.emit("    andi t2, t2, 0xff");
        } else {
            self.state.emit("    slli t2, t2, 48");
            self.state.emit("    srli t2, t2, 48");
        }
        self.state.emit("    sllw t2, t2, a3"); // t2 = expected << shift
        if bits == 8 {
            self.state.emit("    andi t3, t3, 0xff");
        } else {
            self.state.emit("    slli t3, t3, 48");
            self.state.emit("    srli t3, t3, 48");
        }
        self.state.emit("    sllw t3, t3, a3"); // t3 = desired << shift

        // LR/SC loop
        self.state.emit_fmt(format_args!("{}:", loop_label));
        self.state
            .emit_fmt(format_args!("    lr.w{} t0, (a2)", aq_rl));
        // Compare only the sub-word field
        self.state.emit("    and t4, t0, a4"); // t4 = current field
        self.state
            .emit_fmt(format_args!("    bne t4, t2, {}", fail_label));
        // Build new word: (old & ~mask) | desired_shifted
        self.state.emit("    and t4, t0, a5");
        self.state.emit("    or t4, t4, t3");
        // SC: rd (t5) must differ from rs2 (t4) per RISC-V spec
        self.state
            .emit_fmt(format_args!("    sc.w{} t5, t4, (a2)", aq_rl));
        self.state
            .emit_fmt(format_args!("    bnez t5, {}", loop_label));
        // Success
        if returns_bool {
            self.state.emit("    li t0, 1");
        } else {
            // Extract old sub-word value
            self.state.emit("    srlw t0, t0, a3");
            if bits == 8 {
                self.state.emit("    andi t0, t0, 0xff");
            } else {
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srli t0, t0, 48");
            }
        }
        self.state.emit_fmt(format_args!("    j {}", done_label));
        self.state.emit_fmt(format_args!("{}:", fail_label));
        if returns_bool {
            self.state.emit("    li t0, 0");
        } else {
            // Extract old sub-word value from loaded word
            self.state.emit("    srlw t0, t0, a3");
            if bits == 8 {
                self.state.emit("    andi t0, t0, 0xff");
            } else {
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srli t0, t0, 48");
            }
        }
        self.state.emit_fmt(format_args!("{}:", done_label));
    }

    /// Software CLZ (count leading zeros). Input in t0, result in t0.
    /// For 32-bit types, counts leading zeros in the lower 32 bits.
    pub(super) fn emit_clz(&mut self, ty: IrType) {
        let bits: u64 = match ty {
            IrType::I32 | IrType::U32 => 32,
            _ => 64,
        };
        let loop_label = self.state.fresh_label("clz_loop");
        let done_label = self.state.fresh_label("clz_done");
        let zero_label = self.state.fresh_label("clz_zero");

        if bits == 32 {
            // Mask to 32 bits to avoid counting upper bits
            self.state.emit("    slli t0, t0, 32");
            self.state.emit("    srli t0, t0, 32");
        }

        // Handle zero case: clz(0) = bits
        self.state
            .emit_fmt(format_args!("    beqz t0, {}", zero_label));
        // t1 = count = 0, scan from MSB
        self.state.emit("    li t1, 0");
        // t2 = mask = 1 << (bits-1)
        self.state.emit("    li t2, 1");
        self.state
            .emit_fmt(format_args!("    slli t2, t2, {}", bits - 1));
        self.state.emit_fmt(format_args!("{}:", loop_label));
        self.state.emit("    and t3, t0, t2");
        self.state
            .emit_fmt(format_args!("    bnez t3, {}", done_label));
        self.state.emit("    srli t2, t2, 1"); // shift mask right
        self.state.emit("    addi t1, t1, 1");
        self.state.emit_fmt(format_args!("    j {}", loop_label));
        self.state.emit_fmt(format_args!("{}:", zero_label));
        self.state.emit_fmt(format_args!("    li t1, {}", bits));
        self.state.emit_fmt(format_args!("{}:", done_label));
        self.state.emit("    mv t0, t1");
    }

    /// Software CTZ (count trailing zeros). Input in t0, result in t0.
    pub(super) fn emit_ctz(&mut self, ty: IrType) {
        let bits = match ty {
            IrType::I32 | IrType::U32 => 32u64,
            _ => 64u64,
        };
        let loop_label = self.state.fresh_label("ctz_loop");
        let done_label = self.state.fresh_label("ctz_done");

        // t1 = count, starts at 0
        self.state.emit("    li t1, 0");
        self.state.emit_fmt(format_args!("{}:", loop_label));
        self.state.emit_fmt(format_args!("    li t2, {}", bits));
        self.state
            .emit_fmt(format_args!("    beq t1, t2, {}", done_label)); // if counted all bits, done
        self.state.emit("    andi t3, t0, 1");
        self.state
            .emit_fmt(format_args!("    bnez t3, {}", done_label)); // found a 1 bit
        self.state.emit("    srli t0, t0, 1");
        self.state.emit("    addi t1, t1, 1");
        self.state.emit_fmt(format_args!("    j {}", loop_label));
        self.state.emit_fmt(format_args!("{}:", done_label));
        self.state.emit("    mv t0, t1");
    }

    /// Software BSWAP (byte swap). Input in t0, result in t0.
    pub(super) fn emit_bswap(&mut self, ty: IrType) {
        match ty {
            IrType::I16 | IrType::U16 => {
                // Swap bytes of a 16-bit value
                // t1 = (t0 >> 8) & 0xFF, t2 = (t0 & 0xFF) << 8
                self.state.emit("    andi t1, t0, 0xff");
                self.state.emit("    slli t1, t1, 8");
                self.state.emit("    srli t0, t0, 8");
                self.state.emit("    andi t0, t0, 0xff");
                self.state.emit("    or t0, t0, t1");
            }
            IrType::I32 | IrType::U32 => {
                // Swap 4 bytes: ABCD -> DCBA
                self.state.emit("    mv t1, t0");
                // Byte 0 -> byte 3
                self.state.emit("    andi t2, t1, 0xff");
                self.state.emit("    slli t0, t2, 24");
                // Byte 1 -> byte 2
                self.state.emit("    srli t2, t1, 8");
                self.state.emit("    andi t2, t2, 0xff");
                self.state.emit("    slli t2, t2, 16");
                self.state.emit("    or t0, t0, t2");
                // Byte 2 -> byte 1
                self.state.emit("    srli t2, t1, 16");
                self.state.emit("    andi t2, t2, 0xff");
                self.state.emit("    slli t2, t2, 8");
                self.state.emit("    or t0, t0, t2");
                // Byte 3 -> byte 0
                self.state.emit("    srli t2, t1, 24");
                self.state.emit("    andi t2, t2, 0xff");
                self.state.emit("    or t0, t0, t2");
                // Zero-extend to 32 bits (clear upper 32 bits)
                self.state.emit("    slli t0, t0, 32");
                self.state.emit("    srli t0, t0, 32");
            }
            _ => {
                // 64-bit byte swap: reverse all 8 bytes
                self.state.emit("    mv t1, t0");
                self.state.emit("    li t0, 0");
                // Use a shift-based approach: extract each byte and place it
                for i in 0..8u64 {
                    let src_shift = i * 8;
                    let dst_shift = (7 - i) * 8;
                    self.state
                        .emit_fmt(format_args!("    srli t2, t1, {}", src_shift));
                    self.state.emit("    andi t2, t2, 0xff");
                    if dst_shift > 0 {
                        self.state
                            .emit_fmt(format_args!("    slli t2, t2, {}", dst_shift));
                    }
                    self.state.emit("    or t0, t0, t2");
                }
            }
        }
    }

    /// Software POPCOUNT (population count / count set bits). Input in t0, result in t0.
    /// Uses Brian Kernighan's algorithm: repeatedly clear the lowest set bit.
    pub(super) fn emit_popcount(&mut self, ty: IrType) {
        let loop_label = self.state.fresh_label("popcnt_loop");
        let done_label = self.state.fresh_label("popcnt_done");

        if ty == IrType::I32 || ty == IrType::U32 {
            // Mask to 32 bits
            self.state.emit("    slli t0, t0, 32");
            self.state.emit("    srli t0, t0, 32");
        }

        // t1 = count
        self.state.emit("    li t1, 0");
        self.state.emit_fmt(format_args!("{}:", loop_label));
        self.state
            .emit_fmt(format_args!("    beqz t0, {}", done_label));
        self.state.emit("    addi t2, t0, -1"); // t2 = n - 1
        self.state.emit("    and t0, t0, t2"); // n &= n - 1 (clear lowest set bit)
        self.state.emit("    addi t1, t1, 1");
        self.state.emit_fmt(format_args!("    j {}", loop_label));
        self.state.emit_fmt(format_args!("{}:", done_label));
        self.state.emit("    mv t0, t1");
    }

    // ---- Trait-level atomic operations (delegated from ArchCodegen) ----

    pub(super) fn emit_atomic_rmw_impl(
        &mut self,
        dest: &Value,
        op: AtomicRmwOp,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        // Load ptr into t1, val into t2
        self.operand_to_t0(ptr);
        self.state.emit("    mv t1, t0"); // t1 = ptr
        self.operand_to_t0(val);
        self.state.emit("    mv t2, t0"); // t2 = val

        let aq_rl = Self::amo_ordering(ordering);

        if Self::is_subword_type(ty) {
            self.emit_subword_atomic_rmw(op, ty, aq_rl);
        } else {
            let suffix = Self::amo_width_suffix(ty);
            match op {
                AtomicRmwOp::Add => {
                    self.state
                        .emit_fmt(format_args!("    amoadd.{}{} t0, t2, (t1)", suffix, aq_rl));
                }
                AtomicRmwOp::Sub => {
                    self.state.emit("    neg t2, t2");
                    self.state
                        .emit_fmt(format_args!("    amoadd.{}{} t0, t2, (t1)", suffix, aq_rl));
                }
                AtomicRmwOp::And => {
                    self.state
                        .emit_fmt(format_args!("    amoand.{}{} t0, t2, (t1)", suffix, aq_rl));
                }
                AtomicRmwOp::Or => {
                    self.state
                        .emit_fmt(format_args!("    amoor.{}{} t0, t2, (t1)", suffix, aq_rl));
                }
                AtomicRmwOp::Xor => {
                    self.state
                        .emit_fmt(format_args!("    amoxor.{}{} t0, t2, (t1)", suffix, aq_rl));
                }
                AtomicRmwOp::Xchg => {
                    self.state
                        .emit_fmt(format_args!("    amoswap.{}{} t0, t2, (t1)", suffix, aq_rl));
                }
                AtomicRmwOp::Nand => {
                    let loop_label = self.state.fresh_label("atomic_nand");
                    self.state.emit_fmt(format_args!("{}:", loop_label));
                    self.state
                        .emit_fmt(format_args!("    lr.{}{} t0, (t1)", suffix, aq_rl));
                    self.state.emit("    and t3, t0, t2");
                    self.state.emit("    not t3, t3");
                    self.state
                        .emit_fmt(format_args!("    sc.{}{} t4, t3, (t1)", suffix, aq_rl));
                    self.state
                        .emit_fmt(format_args!("    bnez t4, {}", loop_label));
                }
                AtomicRmwOp::TestAndSet => {
                    self.state.emit("    li t2, 1");
                    self.state
                        .emit_fmt(format_args!("    amoswap.{}{} t0, t2, (t1)", suffix, aq_rl));
                }
            }
        }
        Self::sign_extend_riscv(&mut self.state, ty);
        self.store_t0_to(dest);
    }

    pub(super) fn emit_atomic_cmpxchg_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        expected: &Operand,
        desired: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
        _failure_ordering: AtomicOrdering,
        returns_bool: bool,
    ) {
        self.operand_to_t0(ptr);
        self.state.emit("    mv t1, t0");
        self.operand_to_t0(desired);
        self.state.emit("    mv t3, t0");
        self.operand_to_t0(expected);
        self.state.emit("    mv t2, t0");

        let aq_rl = Self::amo_ordering(ordering);

        if Self::is_subword_type(ty) {
            self.emit_subword_atomic_cmpxchg(ty, aq_rl, returns_bool);
        } else {
            let suffix = Self::amo_width_suffix(ty);

            let loop_label = self.state.fresh_label("cas_loop");
            let fail_label = self.state.fresh_label("cas_fail");
            let done_label = self.state.fresh_label("cas_done");

            self.state.emit_fmt(format_args!("{}:", loop_label));
            self.state
                .emit_fmt(format_args!("    lr.{}{} t0, (t1)", suffix, aq_rl));
            self.state
                .emit_fmt(format_args!("    bne t0, t2, {}", fail_label));
            self.state
                .emit_fmt(format_args!("    sc.{}{} t4, t3, (t1)", suffix, aq_rl));
            self.state
                .emit_fmt(format_args!("    bnez t4, {}", loop_label));
            if returns_bool {
                self.state.emit("    li t0, 1");
            }
            self.state.emit_fmt(format_args!("    j {}", done_label));
            self.state.emit_fmt(format_args!("{}:", fail_label));
            if returns_bool {
                self.state.emit("    li t0, 0");
            }
            self.state.emit_fmt(format_args!("{}:", done_label));
        }
        self.store_t0_to(dest);
    }

    pub(super) fn emit_atomic_load_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        self.operand_to_t0(ptr);
        if Self::is_subword_type(ty) {
            if matches!(ordering, AtomicOrdering::SeqCst) {
                self.state.emit("    fence rw, rw");
            }
            match ty {
                IrType::I8 => self.state.emit("    lb t0, 0(t0)"),
                IrType::U8 => self.state.emit("    lbu t0, 0(t0)"),
                IrType::I16 => self.state.emit("    lh t0, 0(t0)"),
                IrType::U16 => self.state.emit("    lhu t0, 0(t0)"),
                _ => unreachable!("non-subword type in subword atomic load: {:?}", ty),
            }
            if matches!(
                ordering,
                AtomicOrdering::Acquire | AtomicOrdering::AcqRel | AtomicOrdering::SeqCst
            ) {
                self.state.emit("    fence r, rw");
            }
        } else {
            let suffix = Self::amo_width_suffix(ty);
            let lr_suffix = match ordering {
                AtomicOrdering::Relaxed | AtomicOrdering::Release => "",
                AtomicOrdering::Acquire => ".aq",
                AtomicOrdering::AcqRel | AtomicOrdering::SeqCst => ".aqrl",
            };
            self.state
                .emit_fmt(format_args!("    lr.{}{} t0, (t0)", suffix, lr_suffix));
            Self::sign_extend_riscv(&mut self.state, ty);
        }
        self.store_t0_to(dest);
    }

    pub(super) fn emit_atomic_store_impl(
        &mut self,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        self.operand_to_t0(val);
        self.state.emit("    mv t1, t0"); // t1 = val
        self.operand_to_t0(ptr);
        if Self::is_subword_type(ty) {
            if matches!(
                ordering,
                AtomicOrdering::Release | AtomicOrdering::AcqRel | AtomicOrdering::SeqCst
            ) {
                self.state.emit("    fence rw, w");
            }
            match ty {
                IrType::I8 | IrType::U8 => self.state.emit("    sb t1, 0(t0)"),
                IrType::I16 | IrType::U16 => self.state.emit("    sh t1, 0(t0)"),
                _ => unreachable!("non-subword type in subword atomic store: {:?}", ty),
            }
            if matches!(ordering, AtomicOrdering::SeqCst) {
                self.state.emit("    fence rw, rw");
            }
        } else {
            let aq_rl = Self::amo_ordering(ordering);
            let suffix = Self::amo_width_suffix(ty);
            self.state.emit_fmt(format_args!(
                "    amoswap.{}{} zero, t1, (t0)",
                suffix, aq_rl
            ));
        }
    }

    pub(super) fn emit_fence_impl(&mut self, ordering: AtomicOrdering) {
        match ordering {
            AtomicOrdering::Relaxed => {}
            AtomicOrdering::Acquire => self.state.emit("    fence r, rw"),
            AtomicOrdering::Release => self.state.emit("    fence rw, w"),
            AtomicOrdering::AcqRel | AtomicOrdering::SeqCst => self.state.emit("    fence rw, rw"),
        }
    }
}
