//! i686 type cast emission.
//!
//! Handles `emit_cast` and `emit_cast_instrs` for the i686 backend.
//! On i686, casts involving F64/F128 or 64-bit integers require special
//! handling because:
//! - F64 values are 8 bytes but the accumulator (eax) is only 32 bits
//! - F128 (long double) is native x87 80-bit extended precision (12 bytes)
//! - 64-bit integers use the eax:edx register pair
//!
//! All F64/F128 conversions go through the x87 FPU, bypassing the default
//! emit_load_operand path that assumes values fit in a single register.

use super::emit::I686Codegen;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{Operand, Value};

impl I686Codegen {
    /// Override emit_cast to handle F64 source/destination specially on i686.
    /// F64 values are 8 bytes but the accumulator is only 32 bits, so we use
    /// x87 FPU for all F64 conversions, bypassing the default emit_load_operand path.
    pub(super) fn emit_cast_impl(
        &mut self,
        dest: &Value,
        src: &Operand,
        from_ty: IrType,
        to_ty: IrType,
    ) {
        use crate::backend::cast::{classify_cast_with_f128, CastKind};

        // Let the default handle i128 conversions
        if crate::backend::generation::is_i128_type(from_ty)
            || crate::backend::generation::is_i128_type(to_ty)
        {
            crate::backend::traits::emit_cast_default(self, dest, src, from_ty, to_ty);
            return;
        }

        // On i686, F128 (long double) is native x87 80-bit extended precision,
        // stored as 12 bytes. We must use f128_is_native=true so that F128 casts
        // go through the dedicated SignedToF128/UnsignedToF128/F128ToSigned/etc.
        // paths that use fstpt (12-byte store), not through the F64 paths that
        // use fstpl (8-byte store) which would corrupt F128 values.
        match classify_cast_with_f128(from_ty, to_ty, true) {
            // --- Casts where F64 is the destination (result needs 8-byte slot) ---
            CastKind::SignedToFloat {
                to_f64: true,
                from_ty: src_ty,
            } => {
                self.emit_signed_to_f64(src, src_ty, dest);
            }
            CastKind::UnsignedToFloat {
                to_f64: true,
                from_ty,
            } => {
                self.emit_unsigned_to_f64(src, from_ty, dest);
            }
            CastKind::FloatToFloat { widen: true } => {
                // F32 -> F64: load F32 from eax, x87 will auto-extend
                self.operand_to_eax(src);
                self.state.emit("    pushl %eax");
                self.state.emit("    flds (%esp)");
                self.state.emit("    addl $4, %esp");
                // st(0) is now the F64 value, store to 8-byte slot
                self.emit_f64_store_from_x87(dest);
                self.state.reg_cache.invalidate_acc();
            }

            // --- Casts where F64 is the source (need to load 8-byte value) ---
            CastKind::FloatToSigned { from_f64: true } => {
                self.emit_f64_to_signed(src, to_ty, dest);
            }
            CastKind::FloatToUnsigned {
                from_f64: true,
                to_u64,
            } => {
                self.emit_f64_to_unsigned(src, to_u64, to_ty, dest);
            }
            CastKind::FloatToFloat { widen: false } => {
                // F64 -> F32: load full 8-byte F64, convert to F32 on x87
                self.emit_f64_load_to_x87(src);
                self.state.emit("    subl $4, %esp");
                self.state.emit("    fstps (%esp)");
                self.state.emit("    movl (%esp), %eax");
                self.state.emit("    addl $4, %esp");
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
            }

            // --- F128 <-> F64/F32 conversions ---
            CastKind::FloatToF128 { from_f32 } => {
                self.emit_float_to_f128(src, from_f32, dest);
            }
            CastKind::F128ToFloat { to_f32 } => {
                self.emit_f128_to_float(src, to_f32, dest);
            }

            // --- F128 <-> int conversions ---
            CastKind::SignedToF128 { from_ty: src_ty } => {
                self.emit_signed_to_f128(src, src_ty, dest);
            }
            CastKind::UnsignedToF128 { from_ty: src_ty } => {
                self.emit_unsigned_to_f128(src, src_ty, dest);
            }
            CastKind::F128ToSigned { to_ty: dest_ty } => {
                self.emit_f128_to_signed(src, dest_ty, dest);
            }
            CastKind::F128ToUnsigned { to_ty: dest_ty } => {
                self.emit_f128_to_unsigned(src, dest_ty, dest);
            }

            // --- I64 -> F32: use x87 fildq for full 64-bit precision ---
            CastKind::SignedToFloat {
                to_f64: false,
                from_ty: IrType::I64,
            } => {
                self.emit_load_acc_pair(src);
                self.state.emit("    pushl %edx");
                self.state.emit("    pushl %eax");
                self.state.emit("    fildq (%esp)");
                self.state.emit("    fstps (%esp)");
                self.state.emit("    movl (%esp), %eax");
                self.state.emit("    addl $8, %esp");
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
            }
            // --- U64 -> F32: use x87 with unsigned handling ---
            CastKind::UnsignedToFloat {
                to_f64: false,
                from_ty: IrType::U64,
            } => {
                self.emit_u64_to_f32(src, dest);
            }
            // --- F32 -> I64: use x87 fisttpq ---
            CastKind::FloatToSigned { from_f64: false } if to_ty == IrType::I64 => {
                self.emit_f32_to_i64(src, dest);
            }
            // --- F32 -> U64: use x87 fisttpq ---
            CastKind::FloatToUnsigned {
                from_f64: false,
                to_u64: true,
            } => {
                self.emit_f32_to_i64(src, dest); // same implementation as F32->I64
            }

            // --- Same-size cast between I64 and U64: copy all 8 bytes ---
            CastKind::SignedToUnsignedSameSize { to_ty: IrType::U64 }
            | CastKind::UnsignedToSignedSameSize { to_ty: IrType::I64 }
            | CastKind::Noop
                if matches!(
                    (from_ty, to_ty),
                    (IrType::I64, IrType::U64)
                        | (IrType::U64, IrType::I64)
                        | (IrType::I64, IrType::I64)
                        | (IrType::U64, IrType::U64)
                ) =>
            {
                self.emit_load_acc_pair(src);
                self.emit_store_acc_pair(dest);
                self.state.reg_cache.invalidate_all();
            }

            // --- Widening casts to I64/U64 need full 8-byte store ---
            CastKind::IntWiden { .. } if matches!(to_ty, IrType::I64 | IrType::U64) => {
                self.operand_to_eax(src);
                self.emit_cast_instrs_impl(from_ty, to_ty);
                // Set high half: sign-extend for signed sources, zero-extend for unsigned
                if from_ty.is_signed() {
                    self.state.emit("    cltd"); // sign-extend eax into edx:eax
                } else {
                    self.state.emit("    xorl %edx, %edx");
                }
                self.emit_store_acc_pair(dest);
                self.state.reg_cache.invalidate_all();
            }
            // --- I64/U64 narrowing to smaller types ---
            CastKind::IntNarrow { .. } if matches!(from_ty, IrType::I64 | IrType::U64) => {
                // Load only the low 32 bits (truncation)
                self.operand_to_eax(src);
                self.emit_cast_instrs_impl(from_ty, to_ty);
                self.store_eax_to(dest);
            }
            // --- All other casts use the default path (emit_load_operand -> eax -> cast -> store) ---
            _ => {
                self.operand_to_eax(src);
                self.emit_cast_instrs_impl(from_ty, to_ty);
                self.store_eax_to(dest);
            }
        }
    }

    // --- Helper methods for cast families ---

    /// Signed integer -> F64 via x87 FPU.
    fn emit_signed_to_f64(&mut self, src: &Operand, src_ty: IrType, dest: &Value) {
        if src_ty == IrType::I64 {
            // I64 -> F64: load full 64-bit value, use fildq
            self.emit_load_acc_pair(src);
            self.state.emit("    pushl %edx");
            self.state.emit("    pushl %eax");
            self.state.emit("    fildq (%esp)");
            self.state.emit("    addl $8, %esp");
        } else {
            self.operand_to_eax(src);
            match src_ty {
                IrType::I8 => self.state.emit("    movsbl %al, %eax"),
                IrType::I16 => self.state.emit("    movswl %ax, %eax"),
                _ => {}
            }
            self.state.emit("    pushl %eax");
            self.state.emit("    fildl (%esp)");
            self.state.emit("    addl $4, %esp");
        }
        // st(0) = F64 result, store to dest's 8-byte slot
        self.emit_f64_store_from_x87(dest);
        self.state.reg_cache.invalidate_acc();
    }

    /// Unsigned integer -> F64 via x87 FPU.
    fn emit_unsigned_to_f64(&mut self, src: &Operand, from_ty: IrType, dest: &Value) {
        if from_ty == IrType::U64 {
            // U64 -> F64: fildq treats the value as signed, so values
            // >= 2^63 need correction by adding float constant 2^64.
            self.emit_load_acc_pair(src);
            self.state.emit("    pushl %edx");
            self.state.emit("    pushl %eax");
            self.state.emit("    fildq (%esp)");
            self.state.emit("    addl $8, %esp");
            self.state.emit("    testl %edx, %edx");
            let done_label = self.state.fresh_label("u64_f64_done");
            self.state.out.emit_jcc_label("    jns", &done_label);
            // High bit set: add 2^64 (float 0x5F800000) to fix sign
            self.state.emit("    pushl $0x5F800000");
            self.state.emit("    fadds (%esp)");
            self.state.emit("    addl $4, %esp");
            self.state.out.emit_named_label(&done_label);
        } else {
            // U8/U16/U32 -> F64: handle high-bit-set U32 values
            self.operand_to_eax(src);
            let big_label = self.state.fresh_label("u2f_big");
            let done_label = self.state.fresh_label("u2f_done");
            self.state.emit("    testl %eax, %eax");
            self.state.out.emit_jcc_label("    js", &big_label);
            // Positive (< 2^31): fildl works directly
            self.state.emit("    pushl %eax");
            self.state.emit("    fildl (%esp)");
            self.state.emit("    addl $4, %esp");
            self.state.out.emit_jmp_label(&done_label);
            self.state.out.emit_named_label(&big_label);
            // Bit 31 set: push as u64 (zero-extend), use fildq
            self.state.emit("    pushl $0");
            self.state.emit("    pushl %eax");
            self.state.emit("    fildq (%esp)");
            self.state.emit("    addl $8, %esp");
            self.state.out.emit_named_label(&done_label);
        }
        self.emit_f64_store_from_x87(dest);
        self.state.reg_cache.invalidate_acc();
    }

    /// F64 -> signed integer via x87 FPU.
    fn emit_f64_to_signed(&mut self, src: &Operand, to_ty: IrType, dest: &Value) {
        self.emit_f64_load_to_x87(src);
        if to_ty == IrType::I64 {
            // F64 -> I64: use fisttpq for full 64-bit conversion
            self.state.emit("    subl $8, %esp");
            self.state.emit("    fisttpq (%esp)");
            self.state.emit("    movl (%esp), %eax");
            self.state.emit("    movl 4(%esp), %edx");
            self.state.emit("    addl $8, %esp");
            self.emit_store_acc_pair(dest);
        } else {
            // F64 -> I32/I16/I8: use fisttpl for 32-bit conversion
            self.state.emit("    subl $4, %esp");
            self.state.emit("    fisttpl (%esp)");
            self.state.emit("    movl (%esp), %eax");
            self.state.emit("    addl $4, %esp");
            // Truncate to target width for sub-32-bit signed types
            match to_ty {
                IrType::I8 => self.state.emit("    movsbl %al, %eax"),
                IrType::I16 => self.state.emit("    movswl %ax, %eax"),
                _ => {}
            }
            self.state.reg_cache.invalidate_acc();
            self.store_eax_to(dest);
        }
    }

    /// F64 -> unsigned integer via x87 FPU.
    fn emit_f64_to_unsigned(&mut self, src: &Operand, to_u64: bool, to_ty: IrType, dest: &Value) {
        self.emit_f64_load_to_x87(src);
        if to_u64 {
            // F64 -> U64: use fisttpq for full 64-bit conversion
            self.state.emit("    subl $8, %esp");
            self.state.emit("    fisttpq (%esp)");
            self.state.emit("    movl (%esp), %eax");
            self.state.emit("    movl 4(%esp), %edx");
            self.state.emit("    addl $8, %esp");
            self.emit_store_acc_pair(dest);
        } else {
            // F64 -> unsigned sub-64-bit: use fisttpq then take low 32 bits
            self.state.emit("    subl $8, %esp");
            self.state.emit("    fisttpq (%esp)");
            self.state.emit("    movl (%esp), %eax");
            self.state.emit("    addl $8, %esp");
            // Truncate to target width for sub-32-bit unsigned types
            match to_ty {
                IrType::U8 => self.state.emit("    movzbl %al, %eax"),
                IrType::U16 => self.state.emit("    movzwl %ax, %eax"),
                _ => {}
            }
            self.state.reg_cache.invalidate_acc();
            self.store_eax_to(dest);
        }
    }

    /// F32/F64 -> F128 (x87 80-bit long double).
    fn emit_float_to_f128(&mut self, src: &Operand, from_f32: bool, dest: &Value) {
        if from_f32 {
            // F32 -> F128: load F32 onto x87, store as F128
            self.operand_to_eax(src);
            self.state.emit("    pushl %eax");
            self.state.emit("    flds (%esp)");
            self.state.emit("    addl $4, %esp");
        } else {
            // F64 -> F128: load F64 onto x87
            self.emit_f64_load_to_x87(src);
        }
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    fstpt {}", sr);
            self.state.f128_direct_slots.insert(dest.0);
        }
        self.state.reg_cache.invalidate_acc();
    }

    /// F128 (x87 80-bit) -> F32/F64.
    fn emit_f128_to_float(&mut self, src: &Operand, to_f32: bool, dest: &Value) {
        self.emit_f128_load_to_x87(src);
        if to_f32 {
            // F128 -> F32
            self.state.emit("    subl $4, %esp");
            self.state.emit("    fstps (%esp)");
            self.state.emit("    movl (%esp), %eax");
            self.state.emit("    addl $4, %esp");
            self.state.reg_cache.invalidate_acc();
            self.store_eax_to(dest);
        } else {
            // F128 -> F64
            self.emit_f64_store_from_x87(dest);
            self.state.reg_cache.invalidate_acc();
        }
    }

    /// Signed integer -> F128 (x87 80-bit long double).
    fn emit_signed_to_f128(&mut self, src: &Operand, src_ty: IrType, dest: &Value) {
        if src_ty == IrType::I64 {
            // I64 -> F128: load full 64-bit value via register pair, use fildq
            self.emit_load_acc_pair(src);
            self.state.emit("    pushl %edx");
            self.state.emit("    pushl %eax");
            self.state.emit("    fildq (%esp)");
            self.state.emit("    addl $8, %esp");
        } else {
            self.operand_to_eax(src);
            match src_ty {
                IrType::I8 => self.state.emit("    movsbl %al, %eax"),
                IrType::I16 => self.state.emit("    movswl %ax, %eax"),
                _ => {}
            }
            self.state.emit("    pushl %eax");
            self.state.emit("    fildl (%esp)");
            self.state.emit("    addl $4, %esp");
        }
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    fstpt {}", sr);
            self.state.f128_direct_slots.insert(dest.0);
        }
        self.state.reg_cache.invalidate_acc();
    }

    /// Unsigned integer -> F128 (x87 80-bit long double).
    fn emit_unsigned_to_f128(&mut self, src: &Operand, src_ty: IrType, dest: &Value) {
        if src_ty == IrType::U64 {
            // U64 -> F128 (x87 80-bit long double):
            // fildq treats the value as signed. For values >= 2^63
            // (high bit set), fildq gives a negative result. We fix
            // this by adding 2^64 (as a float constant 0x5F800000).
            self.emit_load_acc_pair(src);
            self.state.emit("    pushl %edx");
            self.state.emit("    pushl %eax");
            self.state.emit("    fildq (%esp)");
            self.state.emit("    addl $8, %esp");
            self.state.emit("    testl %edx, %edx");
            let done_label = self.state.fresh_label("u64_f128_done");
            self.state.out.emit_jcc_label("    jns", &done_label);
            // High bit set: add 2^64 to compensate for signed interpretation.
            // Float constant 0x5F800000 = 2^64 = 18446744073709551616.0f
            self.state.emit("    pushl $0x5F800000");
            self.state.emit("    fadds (%esp)");
            self.state.emit("    addl $4, %esp");
            self.state.out.emit_named_label(&done_label);
        } else {
            // U8/U16/U32 -> F128: handle high-bit-set U32 values
            self.operand_to_eax(src);
            let big_label = self.state.fresh_label("u2f128_big");
            let done_label = self.state.fresh_label("u2f128_done");
            self.state.emit("    testl %eax, %eax");
            self.state.out.emit_jcc_label("    js", &big_label);
            self.state.emit("    pushl %eax");
            self.state.emit("    fildl (%esp)");
            self.state.emit("    addl $4, %esp");
            self.state.out.emit_jmp_label(&done_label);
            self.state.out.emit_named_label(&big_label);
            // Bit 31 set: zero-extend to 64-bit and use fildq
            self.state.emit("    pushl $0");
            self.state.emit("    pushl %eax");
            self.state.emit("    fildq (%esp)");
            self.state.emit("    addl $8, %esp");
            self.state.out.emit_named_label(&done_label);
        }
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    fstpt {}", sr);
            self.state.f128_direct_slots.insert(dest.0);
        }
        self.state.reg_cache.invalidate_acc();
    }

    /// F128 (x87 80-bit) -> signed integer.
    fn emit_f128_to_signed(&mut self, src: &Operand, dest_ty: IrType, dest: &Value) {
        self.emit_f128_load_to_x87(src);
        if dest_ty == IrType::I64 {
            // F128 -> I64: use fisttpq for full 64-bit conversion
            self.state.emit("    subl $8, %esp");
            self.state.emit("    fisttpq (%esp)");
            self.state.emit("    movl (%esp), %eax");
            self.state.emit("    movl 4(%esp), %edx");
            self.state.emit("    addl $8, %esp");
            self.emit_store_acc_pair(dest);
        } else {
            // F128 -> I32/I16/I8: use fisttpl for 32-bit conversion
            self.state.emit("    subl $4, %esp");
            self.state.emit("    fisttpl (%esp)");
            self.state.emit("    movl (%esp), %eax");
            self.state.emit("    addl $4, %esp");
            self.state.reg_cache.invalidate_acc();
            self.store_eax_to(dest);
        }
    }

    /// F128 (x87 80-bit) -> unsigned integer.
    fn emit_f128_to_unsigned(&mut self, src: &Operand, dest_ty: IrType, dest: &Value) {
        self.emit_f128_load_to_x87(src);
        if dest_ty == IrType::U64 {
            // F128 -> U64: use fisttpq for full 64-bit conversion
            self.state.emit("    subl $8, %esp");
            self.state.emit("    fisttpq (%esp)");
            self.state.emit("    movl (%esp), %eax");
            self.state.emit("    movl 4(%esp), %edx");
            self.state.emit("    addl $8, %esp");
            self.emit_store_acc_pair(dest);
        } else {
            // F128 -> U32/U16/U8: use fisttpq then take low 32 bits
            self.state.emit("    subl $8, %esp");
            self.state.emit("    fisttpq (%esp)");
            self.state.emit("    movl (%esp), %eax");
            self.state.emit("    addl $8, %esp");
            self.state.reg_cache.invalidate_acc();
            self.store_eax_to(dest);
        }
    }

    /// U64 -> F32 via x87 with unsigned correction.
    fn emit_u64_to_f32(&mut self, src: &Operand, dest: &Value) {
        self.emit_load_acc_pair(src);
        self.state.emit("    pushl %edx");
        self.state.emit("    pushl %eax");
        self.state.emit("    fildq (%esp)");
        self.state.emit("    addl $8, %esp");
        // If high bit was set, fildq gave a negative result; add 2^64
        self.state.emit("    testl %edx, %edx");
        let done_label = self.state.fresh_label("u64_f32_done");
        self.state.out.emit_jcc_label("    jns", &done_label);
        // Float constant 0x5F800000 = 2^64
        self.state.emit("    pushl $0x5F800000");
        self.state.emit("    fadds (%esp)");
        self.state.emit("    addl $4, %esp");
        self.state.out.emit_named_label(&done_label);
        self.state.emit("    subl $4, %esp");
        self.state.emit("    fstps (%esp)");
        self.state.emit("    movl (%esp), %eax");
        self.state.emit("    addl $4, %esp");
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    /// F32 -> I64/U64 via x87 fisttpq.
    fn emit_f32_to_i64(&mut self, src: &Operand, dest: &Value) {
        self.operand_to_eax(src);
        self.state.emit("    subl $8, %esp");
        self.state.emit("    movl %eax, (%esp)");
        self.state.emit("    flds (%esp)");
        self.state.emit("    fisttpq (%esp)");
        self.state.emit("    movl (%esp), %eax");
        self.state.emit("    movl 4(%esp), %edx");
        self.state.emit("    addl $8, %esp");
        self.emit_store_acc_pair(dest);
        self.state.reg_cache.invalidate_all();
    }

    /// Emit scalar cast instructions (non-F64/F128, non-64-bit integer).
    /// Operates on value already in eax, result left in eax.
    pub(super) fn emit_cast_instrs_impl(&mut self, from_ty: IrType, to_ty: IrType) {
        use crate::backend::cast::{classify_cast, CastKind};

        match classify_cast(from_ty, to_ty) {
            CastKind::Noop | CastKind::UnsignedToSignedSameSize { .. } => {}

            CastKind::IntNarrow { to_ty } => {
                // Truncation to a narrower type: sign-extend or zero-extend
                // the sub-register to fill all of %eax. Without this, the
                // upper bits of %eax retain stale data from the wider
                // computation, which corrupts truthiness checks (testl %eax)
                // and other 32-bit operations on the narrowed value.
                if to_ty.is_signed() {
                    match to_ty {
                        IrType::I8 => self.state.emit("    movsbl %al, %eax"),
                        IrType::I16 => self.state.emit("    movswl %ax, %eax"),
                        _ => {} // I32: no-op (already 32-bit)
                    }
                } else {
                    match to_ty {
                        IrType::U8 => self.state.emit("    movzbl %al, %eax"),
                        IrType::U16 => self.state.emit("    movzwl %ax, %eax"),
                        _ => {} // U32: no-op
                    }
                }
            }

            CastKind::IntWiden { from_ty, .. } => {
                if from_ty.is_unsigned() {
                    match from_ty {
                        IrType::U8 => self.state.emit("    movzbl %al, %eax"),
                        IrType::U16 => self.state.emit("    movzwl %ax, %eax"),
                        // U32 -> I64/U64: no-op on i686 (eax already has 32 bits)
                        _ => {}
                    }
                } else {
                    match from_ty {
                        IrType::I8 => self.state.emit("    movsbl %al, %eax"),
                        IrType::I16 => self.state.emit("    movswl %ax, %eax"),
                        // I32 -> I64/U64: no-op on i686 (eax already has 32 bits)
                        _ => {}
                    }
                }
            }

            CastKind::SignedToUnsignedSameSize { to_ty } => {
                // On i686, same-size signed->unsigned: mask for sub-32-bit types
                match to_ty {
                    IrType::U8 => self.state.emit("    movzbl %al, %eax"),
                    IrType::U16 => self.state.emit("    movzwl %ax, %eax"),
                    _ => {} // U32, U64: no-op
                }
            }

            CastKind::SignedToFloat { to_f64: false, .. } => {
                // Signed int -> F32 via SSE
                self.state.emit("    cvtsi2ssl %eax, %xmm0");
                self.state.emit("    movd %xmm0, %eax");
            }

            CastKind::UnsignedToFloat { to_f64: false, .. } => {
                // U8/U16/U32 -> F32
                let big_label = self.state.fresh_label("u2f_big");
                let done_label = self.state.fresh_label("u2f_done");
                self.state.emit("    testl %eax, %eax");
                self.state.out.emit_jcc_label("    js", &big_label);
                self.state.emit("    cvtsi2ssl %eax, %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                self.state.out.emit_jmp_label(&done_label);
                self.state.out.emit_named_label(&big_label);
                self.state.emit("    pushl $0");
                self.state.emit("    pushl %eax");
                self.state.emit("    fildq (%esp)");
                self.state.emit("    fstps (%esp)");
                self.state.emit("    popl %eax");
                self.state.emit("    addl $4, %esp");
                self.state.out.emit_named_label(&done_label);
            }

            CastKind::FloatToSigned { from_f64: false } => {
                // F32 -> signed int via SSE
                self.state.emit("    movd %eax, %xmm0");
                self.state.emit("    cvttss2si %xmm0, %eax");
                // Truncate to target width for sub-32-bit signed types
                match to_ty {
                    IrType::I8 => self.state.emit("    movsbl %al, %eax"),
                    IrType::I16 => self.state.emit("    movswl %ax, %eax"),
                    _ => {}
                }
            }

            CastKind::FloatToUnsigned {
                from_f64: false,
                to_u64,
            } => {
                if to_u64 {
                    // F32 -> U64: use x87
                    self.state.emit("    pushl %eax");
                    self.state.emit("    flds (%esp)");
                    self.state.emit("    addl $4, %esp");
                    self.state.emit("    subl $8, %esp");
                    self.state.emit("    fisttpq (%esp)");
                    self.state.emit("    movl (%esp), %eax");
                    self.state.emit("    movl 4(%esp), %edx");
                    self.state.emit("    addl $8, %esp");
                } else {
                    // F32 -> unsigned int: cvttss2si treats result as signed
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    cvttss2si %xmm0, %eax");
                    // Truncate to target width for sub-32-bit unsigned types
                    match to_ty {
                        IrType::U8 => self.state.emit("    movzbl %al, %eax"),
                        IrType::U16 => self.state.emit("    movzwl %ax, %eax"),
                        _ => {}
                    }
                }
            }

            // F64/F128 casts are handled by emit_cast_impl above
            _ => {}
        }
    }
}
