//! Strength reduction for integer division and modulo by constants.
//!
//! Replaces slow `div`/`idiv` instructions (20-90 cycles on x86) with fast
//! multiply-and-shift sequences (3-5 cycles). This is one of the most impactful
//! single optimizations for integer-heavy code.
//!
//! Supported transformations:
//!
//! **Unsigned division by constant** (32-bit and 64-bit):
//!   `x /u C` => `mulhi(x, M) >> s` (with optional add-and-shift fixup)
//!   32-bit uses 64-bit intermediate; 64-bit uses 128-bit intermediate.
//!   Uses the "magic number" algorithm from Hacker's Delight by Henry S. Warren Jr.
//!
//! **Signed division by power-of-2**:
//!   `x /s 2^k` => `(x + ((x >> (N-1)) >>> (N - k))) >> k`
//!   Adds a bias for negative numbers to ensure correct rounding toward zero.
//!
//! **Signed division by constant** (32-bit and 64-bit):
//!   `x /s C` => multiply by magic number + shift + sign correction
//!   Uses the signed magic number algorithm from Hacker's Delight.
//!
//! **Modulo by constant**:
//!   `x % C` => `x - (x / C) * C`  (using the optimized division above)
//!
//! All transformations produce correct results for the full range of inputs,
//! including edge cases (0, 1, -1, INT_MIN, UINT_MAX, INT64_MIN, UINT64_MAX).
//!
//! ## Implementation
//!
//! 64-bit magic numbers require 128-bit intermediate products. We use the
//! compiler's existing I128/U128 types: cast both operands to 128-bit,
//! multiply, shift right by 64, and truncate back to 64-bit. All four backends
//! (x86, i686, ARM64, RISC-V) have I128 multiply support that maps to efficient
//! hardware instructions (mulq/imulq on x86, umulh/smulh on ARM64,
//! mulhu/mulh on RISC-V).
//!
//! - Negative divisors are handled via identity: `x / -C == -(x / C)` and
//!   `x % -C == x % C` (in C, the sign of the remainder follows the dividend).

use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};

/// Transform division/modulo by constants in a single function.
pub(crate) fn div_by_const_function(func: &mut IrFunction) -> usize {
    let mut changes = 0;
    let mut next_id = func.next_value_id;
    if next_id == 0 {
        next_id = func.max_value_id() + 1;
    }

    // Build sets of values known to fit in 32 bits.
    // We track two separate properties:
    //   - is_known_u32: value fits in unsigned 32-bit range [0, 2^32-1].
    //     Safe for unsigned division strength reduction (UDiv/URem).
    //   - is_known_i32: value fits in signed 32-bit range [-2^31, 2^31-1].
    //     Safe for signed division strength reduction (SDiv/SRem).
    //
    // The distinction matters for sign-extending casts from signed types to
    // unsigned 64-bit: e.g. (U64)(I8)-128 = 0xFFFFFFFFFFFFFF80, which does NOT
    // fit in u32, but sign-extended values from I32->I64 DO fit in i32 range.
    let max_id = func.max_value_id() as usize;
    let mut is_known_u32: Vec<bool> = vec![false; max_id + 1];
    let mut is_known_i32: Vec<bool> = vec![false; max_id + 1];
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::ParamRef { dest, ty, .. } => {
                    // (d) Function parameter of <=32-bit integer type.
                    // After mem2reg, parameters are used directly without
                    // Load+Cast, but they still fit in 32 bits.
                    //
                    // Note: On some architectures (ARM64, RISC-V), the upper
                    // 32 bits of a register passing a 32-bit argument may not
                    // be properly zero/sign-extended. The expand_*_in_i64
                    // functions handle this by explicitly masking the input.
                    if ty.is_integer() && ty.size() <= 4 {
                        let id = dest.0 as usize;
                        if id <= max_id {
                            if ty.is_unsigned() {
                                is_known_u32[id] = true;
                                is_known_i32[id] = true;
                            } else {
                                is_known_i32[id] = true;
                            }
                        }
                    }
                }
                Instruction::Cast {
                    dest,
                    from_ty,
                    to_ty,
                    ..
                } => {
                    let id = dest.0 as usize;
                    if id <= max_id {
                        // (a) Widening from <=32-bit to 64-bit
                        if from_ty.is_integer()
                            && from_ty.size() <= 4
                            && (*to_ty == IrType::I64 || *to_ty == IrType::U64)
                        {
                            if from_ty.is_unsigned() {
                                // Zero-extension: value fits in both u32 and i32 ranges
                                // (unsigned <=32-bit values are always non-negative and <= 2^32-1)
                                is_known_u32[id] = true;
                                is_known_i32[id] = true;
                            } else {
                                // Sign-extension from signed type: value fits in i32 range
                                // but NOT necessarily in u32 range (e.g. -128 sign-extended
                                // to U64 becomes 0xFFFFFFFFFFFFFF80, which is > 2^32).
                                is_known_i32[id] = true;
                                // Only safe for unsigned if widening to I64 (not U64),
                                // AND the value will only be used in signed context.
                                // Conservative: do NOT mark as u32-safe.
                            }
                        }
                        // (b) Narrowing/truncation to <=32-bit
                        if to_ty.is_integer() && to_ty.size() <= 4 {
                            is_known_u32[id] = true;
                            is_known_i32[id] = true;
                        }
                    }
                }
                Instruction::Load { dest, ty, .. } => {
                    // (c) Load of a <=32-bit integer type
                    if ty.is_integer() && ty.size() <= 4 {
                        let id = dest.0 as usize;
                        if id <= max_id {
                            is_known_u32[id] = true;
                            is_known_i32[id] = true;
                        }
                    }
                }
                Instruction::BinOp { dest, op, ty, .. } => {
                    // (e) BinOp that produces a <=32-bit result.
                    // After the narrow pass, operations may be narrowed to I32/U32.
                    // Also, And with a small mask produces a small result.
                    if ty.is_integer() && ty.size() <= 4 {
                        let id = dest.0 as usize;
                        if id <= max_id {
                            if ty.is_unsigned() || *op == IrBinOp::And || *op == IrBinOp::LShr {
                                is_known_u32[id] = true;
                            }
                            is_known_i32[id] = true;
                        }
                    }
                }
                Instruction::Cmp { dest, .. } => {
                    // Comparison results are 0 or 1, always fit in 32 bits.
                    let id = dest.0 as usize;
                    if id <= max_id {
                        is_known_u32[id] = true;
                        is_known_i32[id] = true;
                    }
                }
                _ => {}
            }
        }
    }

    // Helper: check if an operand is known to fit in unsigned 32 bits.
    let lhs_is_u32 = |op: &Operand| -> bool {
        match op {
            Operand::Value(v) => {
                let id = v.0 as usize;
                id <= max_id && is_known_u32[id]
            }
            Operand::Const(c) => {
                if let Some(v) = c.to_i64() {
                    v >= 0 && v <= u32::MAX as i64
                } else {
                    false
                }
            }
        }
    };

    // Helper: check if an operand is known to fit in signed 32 bits.
    let lhs_is_i32 = |op: &Operand| -> bool {
        match op {
            Operand::Value(v) => {
                let id = v.0 as usize;
                id <= max_id && is_known_i32[id]
            }
            Operand::Const(c) => {
                if let Some(v) = c.to_i64() {
                    v >= i32::MIN as i64 && v <= i32::MAX as i64
                } else {
                    false
                }
            }
        }
    };

    for block in &mut func.blocks {
        let mut new_insts: Vec<Instruction> = Vec::new();
        let has_spans = !block.source_spans.is_empty();
        let mut new_spans: Vec<crate::common::source::Span> = Vec::new();
        let old_spans = std::mem::take(&mut block.source_spans);
        for (inst_idx, inst) in block.instructions.drain(..).enumerate() {
            let span = if has_spans && inst_idx < old_spans.len() {
                Some(old_spans[inst_idx])
            } else {
                None
            };
            match &inst {
                Instruction::BinOp {
                    dest,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => {
                    let const_val = match rhs {
                        Operand::Const(c) => c.to_i64(),
                        _ => None,
                    };
                    if let Some(divisor) = const_val {
                        // We handle:
                        // 1. Native 32-bit types (I32/U32) - always safe.
                        // 2. I64/U64 operations where the LHS is provably a
                        //    widened 32-bit value (from C integer promotion).
                        //    Without this check, genuine 64-bit divisions
                        //    (e.g. long long / 10) would get 32-bit magic
                        //    numbers and produce wrong results.
                        //
                        // For unsigned ops (UDiv/URem), we need the LHS to fit
                        // in unsigned 32-bit range [0, 2^32-1]. For signed ops
                        // (SDiv/SRem), it must fit in signed 32-bit range
                        // [-2^31, 2^31-1]. This matters because sign-extending
                        // a negative signed value to U64 produces a value that
                        // exceeds u32 range (e.g. (U64)(char)-128 = 0xFFFFFFFFFFFFFF80).
                        let expanded = match op {
                            IrBinOp::UDiv => {
                                // For unsigned division, reinterpret i64 bits as u64.
                                // Large u64 divisors (> i64::MAX) appear as negative i64.
                                let udivisor = divisor as u64;
                                if udivisor >= 2 && udivisor <= u32::MAX as u64 {
                                    let d32 = udivisor as u32;
                                    match *ty {
                                        IrType::U32 => {
                                            expand_udiv32(*dest, lhs, d32, *ty, &mut next_id)
                                        }
                                        // Note: widened_op_type() maps U64 to I64, so UDiv on
                                        // unsigned long long uses I64 type. Match both.
                                        IrType::I64 | IrType::U64 if lhs_is_u32(lhs) => {
                                            expand_udiv32_in_i64(*dest, lhs, d32, &mut next_id)
                                        }
                                        IrType::I64 | IrType::U64 => {
                                            expand_udiv64(*dest, lhs, udivisor, *ty, &mut next_id)
                                        }
                                        _ => None,
                                    }
                                } else if udivisor >= 2 {
                                    match *ty {
                                        IrType::I64 | IrType::U64 => {
                                            expand_udiv64(*dest, lhs, udivisor, *ty, &mut next_id)
                                        }
                                        _ => None,
                                    }
                                } else {
                                    None
                                }
                            }
                            IrBinOp::SDiv => {
                                if divisor > 1 && divisor <= i32::MAX as i64 {
                                    match *ty {
                                        IrType::I32 => expand_sdiv32(
                                            *dest,
                                            lhs,
                                            divisor as i32,
                                            *ty,
                                            &mut next_id,
                                        ),
                                        IrType::I64 if lhs_is_i32(lhs) => expand_sdiv32_in_i64(
                                            *dest,
                                            lhs,
                                            divisor as i32,
                                            &mut next_id,
                                        ),
                                        IrType::I64 => {
                                            expand_sdiv64(*dest, lhs, divisor, &mut next_id)
                                        }
                                        _ => None,
                                    }
                                } else if divisor > i32::MAX as i64 {
                                    // Divisor > i32::MAX: only 64-bit
                                    match *ty {
                                        IrType::I64 => {
                                            expand_sdiv64(*dest, lhs, divisor, &mut next_id)
                                        }
                                        _ => None,
                                    }
                                } else if divisor < -1 {
                                    // Negative divisor: x / -C == -(x / C)
                                    let pos_divisor = -divisor;
                                    if pos_divisor <= i32::MAX as i64 {
                                        match *ty {
                                            IrType::I32 => {
                                                let pd = pos_divisor as i32;
                                                expand_sdiv_neg(
                                                    *dest,
                                                    lhs,
                                                    pd,
                                                    *ty,
                                                    &lhs_is_i32,
                                                    &mut next_id,
                                                )
                                            }
                                            IrType::I64 if lhs_is_i32(lhs) => {
                                                let pd = pos_divisor as i32;
                                                expand_sdiv_neg(
                                                    *dest,
                                                    lhs,
                                                    pd,
                                                    *ty,
                                                    &lhs_is_i32,
                                                    &mut next_id,
                                                )
                                            }
                                            IrType::I64 => expand_sdiv64_neg(
                                                *dest,
                                                lhs,
                                                pos_divisor,
                                                &mut next_id,
                                            ),
                                            _ => None,
                                        }
                                    } else {
                                        // pos_divisor > i32::MAX: 64-bit only
                                        // Exclude i64::MIN because -i64::MIN overflows
                                        if divisor > i64::MIN {
                                            match *ty {
                                                IrType::I64 => expand_sdiv64_neg(
                                                    *dest,
                                                    lhs,
                                                    pos_divisor,
                                                    &mut next_id,
                                                ),
                                                _ => None,
                                            }
                                        } else {
                                            None
                                        }
                                    }
                                } else {
                                    None
                                }
                            }
                            IrBinOp::URem => {
                                let udivisor = divisor as u64;
                                if udivisor >= 2 && udivisor <= u32::MAX as u64 {
                                    let d32 = udivisor as u32;
                                    match *ty {
                                        IrType::U32 => {
                                            expand_urem32(*dest, lhs, d32, *ty, &mut next_id)
                                        }
                                        IrType::I64 | IrType::U64 if lhs_is_u32(lhs) => {
                                            expand_urem32_in_i64(*dest, lhs, d32, &mut next_id)
                                        }
                                        IrType::I64 | IrType::U64 => {
                                            expand_urem64(*dest, lhs, udivisor, *ty, &mut next_id)
                                        }
                                        _ => None,
                                    }
                                } else if udivisor >= 2 {
                                    match *ty {
                                        IrType::I64 | IrType::U64 => {
                                            expand_urem64(*dest, lhs, udivisor, *ty, &mut next_id)
                                        }
                                        _ => None,
                                    }
                                } else {
                                    None
                                }
                            }
                            IrBinOp::SRem => {
                                if divisor > 1 && divisor <= i32::MAX as i64 {
                                    match *ty {
                                        IrType::I32 => expand_srem32(
                                            *dest,
                                            lhs,
                                            divisor as i32,
                                            *ty,
                                            &mut next_id,
                                        ),
                                        IrType::I64 if lhs_is_i32(lhs) => expand_srem32_in_i64(
                                            *dest,
                                            lhs,
                                            divisor as i32,
                                            &mut next_id,
                                        ),
                                        IrType::I64 => {
                                            expand_srem64(*dest, lhs, divisor, &mut next_id)
                                        }
                                        _ => None,
                                    }
                                } else if divisor > i32::MAX as i64 {
                                    match *ty {
                                        IrType::I64 => {
                                            expand_srem64(*dest, lhs, divisor, &mut next_id)
                                        }
                                        _ => None,
                                    }
                                } else if divisor < -1 {
                                    // Negative divisor: x % -C == x % C (sign follows dividend in C)
                                    let pos_divisor = -divisor;
                                    if pos_divisor <= i32::MAX as i64 {
                                        let pd = pos_divisor as i32;
                                        match *ty {
                                            IrType::I32 => {
                                                expand_srem32(*dest, lhs, pd, *ty, &mut next_id)
                                            }
                                            IrType::I64 if lhs_is_i32(lhs) => {
                                                expand_srem32_in_i64(*dest, lhs, pd, &mut next_id)
                                            }
                                            IrType::I64 => {
                                                expand_srem64(*dest, lhs, pos_divisor, &mut next_id)
                                            }
                                            _ => None,
                                        }
                                    } else if divisor > i64::MIN {
                                        match *ty {
                                            IrType::I64 => {
                                                expand_srem64(*dest, lhs, pos_divisor, &mut next_id)
                                            }
                                            _ => None,
                                        }
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };
                        if let Some(exp) = expanded {
                            let count = exp.len();
                            new_insts.extend(exp);
                            // Add spans for each expanded instruction
                            if has_spans {
                                let s = span.unwrap_or(crate::common::source::Span::new(0, 0, 0));
                                for _ in 0..count {
                                    new_spans.push(s);
                                }
                            }
                            changes += 1;
                            continue;
                        }
                    }
                    new_insts.push(inst);
                    if has_spans {
                        new_spans.push(span.unwrap_or(crate::common::source::Span::new(0, 0, 0)));
                    }
                }
                _ => {
                    new_insts.push(inst);
                    if has_spans {
                        new_spans.push(span.unwrap_or(crate::common::source::Span::new(0, 0, 0)));
                    }
                }
            }
        }
        block.instructions = new_insts;
        if has_spans {
            block.source_spans = new_spans;
        }
    }

    if changes > 0 {
        func.next_value_id = next_id;
    }
    changes
}

/// Allocate a fresh Value ID.
fn fresh_value(next_id: &mut u32) -> Value {
    let v = Value(*next_id);
    *next_id += 1;
    v
}

// ─── Unsigned division by constant (32-bit) ────────────────────────────────

/// Compute the magic number for unsigned 32-bit division by `d`.
/// Returns (magic_number, post_shift, needs_add) where:
/// - If !needs_add: result = (x * magic) >> 32 >> post_shift
/// - If needs_add: result = ((x - mulhi(x, magic)) >> 1 + mulhi(x, magic)) >> (post_shift - 1)
///
/// Uses a direct computation approach: find the smallest p such that
/// ceil(2^(32+p) / d) * d - 2^(32+p) <= 2^p, then magic = ceil(2^(32+p) / d).
/// If magic >= 2^32 (doesn't fit in 32 bits), use the add-and-shift fixup.
fn compute_unsigned_magic_32(d: u32) -> Option<(u64, u32, bool)> {
    assert!(d >= 2);

    // Try each shift amount starting from 0
    for p in 0u32..32 {
        // magic = ceil(2^(32+p) / d)
        let two_pow = 1u128 << (32 + p);
        let magic = two_pow.div_ceil(d as u128);

        // Check if this magic number works: the error must be <= 2^p
        // error = magic * d - 2^(32+p) which must be <= 2^p
        let error = magic * d as u128 - two_pow;
        if error <= (1u128 << p) {
            if magic <= u32::MAX as u128 {
                // Fits in 32 bits - simple case
                return Some((magic as u64, p, false));
            } else if magic <= u64::MAX as u128 {
                // Doesn't fit in 32 bits - use add-and-shift fixup
                // We need to subtract 2^32 from magic and compensate
                let m = magic - (1u128 << 32);
                return Some((m as u64, p, true));
            }
        }
    }

    // Some large divisors near u32::MAX cannot be handled with 32-bit magic
    // numbers. Fall back to native division for these rare cases.
    None
}

/// Expand `dest = x /u C` for 32-bit unsigned x.
fn expand_udiv32(
    dest: Value,
    x: &Operand,
    d: u32,
    ty: IrType,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    // Power of 2: already handled by simplify pass, but handle here too for completeness
    if d.is_power_of_two() {
        let shift = d.trailing_zeros();
        return Some(vec![Instruction::BinOp {
            dest,
            op: IrBinOp::LShr,
            lhs: *x,
            rhs: Operand::Const(IrConst::from_i64(shift as i64, ty)),
            ty,
        }]);
    }

    let (magic, shift, needs_add) = compute_unsigned_magic_32(d)?;

    let mut insts = Vec::new();

    // Step 1: zero-extend x to 64 bits
    let x64 = fresh_value(next_id);
    insts.push(Instruction::Cast {
        dest: x64,
        src: *x,
        from_ty: IrType::U32,
        to_ty: IrType::U64,
    });

    // Step 2: multiply by magic number (in 64 bits)
    let product = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: product,
        op: IrBinOp::Mul,
        lhs: Operand::Value(x64),
        rhs: Operand::Const(IrConst::I64(magic as i64)),
        ty: IrType::U64,
    });

    // Step 3: get high 32 bits (shift right by 32)
    let hi = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: hi,
        op: IrBinOp::LShr,
        lhs: Operand::Value(product),
        rhs: Operand::Const(IrConst::I64(32)),
        ty: IrType::U64,
    });

    if !needs_add {
        // Simple case: result = hi >> shift
        if shift == 0 {
            // Truncate to 32 bits
            insts.push(Instruction::Cast {
                dest,
                src: Operand::Value(hi),
                from_ty: IrType::U64,
                to_ty: IrType::U32,
            });
        } else {
            let shifted = fresh_value(next_id);
            insts.push(Instruction::BinOp {
                dest: shifted,
                op: IrBinOp::LShr,
                lhs: Operand::Value(hi),
                rhs: Operand::Const(IrConst::I64(shift as i64)),
                ty: IrType::U64,
            });
            insts.push(Instruction::Cast {
                dest,
                src: Operand::Value(shifted),
                from_ty: IrType::U64,
                to_ty: IrType::U32,
            });
        }
    } else {
        // Needs add fixup: result = ((x - hi) >> 1 + hi) >> (shift - 1)
        // Step 4: x - hi (in 64 bits, using zero-extended x)
        let diff = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: diff,
            op: IrBinOp::Sub,
            lhs: Operand::Value(x64),
            rhs: Operand::Value(hi),
            ty: IrType::U64,
        });

        // Step 5: diff >> 1
        let half = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: half,
            op: IrBinOp::LShr,
            lhs: Operand::Value(diff),
            rhs: Operand::Const(IrConst::I64(1)),
            ty: IrType::U64,
        });

        // Step 6: half + hi
        let sum = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: sum,
            op: IrBinOp::Add,
            lhs: Operand::Value(half),
            rhs: Operand::Value(hi),
            ty: IrType::U64,
        });

        // Step 7: sum >> (shift - 1)
        if shift > 1 {
            let result64 = fresh_value(next_id);
            insts.push(Instruction::BinOp {
                dest: result64,
                op: IrBinOp::LShr,
                lhs: Operand::Value(sum),
                rhs: Operand::Const(IrConst::I64((shift - 1) as i64)),
                ty: IrType::U64,
            });
            insts.push(Instruction::Cast {
                dest,
                src: Operand::Value(result64),
                from_ty: IrType::U64,
                to_ty: IrType::U32,
            });
        } else {
            // shift == 1, already shifted by 1 above
            insts.push(Instruction::Cast {
                dest,
                src: Operand::Value(sum),
                from_ty: IrType::U64,
                to_ty: IrType::U32,
            });
        }
    }

    Some(insts)
}

// ─── Signed division by constant (32-bit) ──────────────────────────────────

/// Compute the magic number for signed 32-bit division by `d`.
/// Returns (magic_number, shift_amount).
/// The result is: mulhi(x, magic) + adjust >> shift, then add sign correction.
///
/// Based on Hacker's Delight, 2nd edition, Figure 10-2.
fn compute_signed_magic_32(d: i32) -> (i64, u32) {
    assert!(d >= 2);
    let ad = d as u32; // absolute value (d > 0)
    let t = 0x80000000u32; // 2^31
    let anc = t - 1 - (t % ad); // absolute value of nc
    let mut p: u32 = 31;
    let mut q1 = 0x80000000u64 / anc as u64;
    let mut r1 = 0x80000000u64 - q1 * anc as u64;
    let mut q2 = 0x80000000u64 / ad as u64;
    let mut r2 = 0x80000000u64 - q2 * ad as u64;

    loop {
        p += 1;
        q1 *= 2;
        r1 *= 2;
        if r1 >= anc as u64 {
            q1 += 1;
            r1 -= anc as u64;
        }
        q2 *= 2;
        r2 *= 2;
        if r2 >= ad as u64 {
            q2 += 1;
            r2 -= ad as u64;
        }
        let delta = ad as u64 - r2;
        if q1 < delta || (q1 == delta && r1 == 0) {
            continue;
        }
        break;
    }

    // Magic number (can be negative, stored as i64 to preserve sign)
    let magic = (q2 + 1) as i64;
    let shift = p - 32;
    (magic, shift)
}

/// Expand `dest = x /s C` for 32-bit signed x.
fn expand_sdiv32(
    dest: Value,
    x: &Operand,
    d: i32,
    _ty: IrType,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    // Power of 2: x / 2^k => (x + (x >> 31 >>> (32 - k))) >> k
    if d > 0 && (d as u32).is_power_of_two() {
        let k = d.trailing_zeros();
        let mut insts = Vec::new();

        // Arithmetic shift right by 31 to get sign bit (all 1s or all 0s)
        let sign = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: sign,
            op: IrBinOp::AShr,
            lhs: *x,
            rhs: Operand::Const(IrConst::I32(31)),
            ty: IrType::I32,
        });

        // Logical shift right to get bias: (32 - k) bits
        let bias = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: bias,
            op: IrBinOp::LShr,
            lhs: Operand::Value(sign),
            rhs: Operand::Const(IrConst::I32(32 - k as i32)),
            ty: IrType::I32,
        });

        // Add bias to x
        let biased = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: biased,
            op: IrBinOp::Add,
            lhs: *x,
            rhs: Operand::Value(bias),
            ty: IrType::I32,
        });

        // Arithmetic shift right by k
        insts.push(Instruction::BinOp {
            dest,
            op: IrBinOp::AShr,
            lhs: Operand::Value(biased),
            rhs: Operand::Const(IrConst::I32(k as i32)),
            ty: IrType::I32,
        });

        return Some(insts);
    }

    // General case: signed magic number division
    let (magic, shift) = compute_signed_magic_32(d);

    let mut insts = Vec::new();

    // Step 1: sign-extend x to 64 bits
    let x64 = fresh_value(next_id);
    insts.push(Instruction::Cast {
        dest: x64,
        src: *x,
        from_ty: IrType::I32,
        to_ty: IrType::I64,
    });

    // Step 2: multiply by magic number (in 64 bits)
    let product = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: product,
        op: IrBinOp::Mul,
        lhs: Operand::Value(x64),
        rhs: Operand::Const(IrConst::I64(magic)),
        ty: IrType::I64,
    });

    // Step 3: get high 32 bits (arithmetic shift right by 32)
    let hi = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: hi,
        op: IrBinOp::AShr,
        lhs: Operand::Value(product),
        rhs: Operand::Const(IrConst::I64(32)),
        ty: IrType::I64,
    });

    // Step 4: shift right by shift amount
    let shifted = if shift > 0 {
        let s = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: s,
            op: IrBinOp::AShr,
            lhs: Operand::Value(hi),
            rhs: Operand::Const(IrConst::I64(shift as i64)),
            ty: IrType::I64,
        });
        s
    } else {
        hi
    };

    // Step 5: add sign correction: result + (result >> 63)
    // This adds 1 when the result is negative (rounds toward zero)
    let sign_bit = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: sign_bit,
        op: IrBinOp::LShr,
        lhs: Operand::Value(shifted),
        rhs: Operand::Const(IrConst::I64(63)),
        ty: IrType::I64,
    });

    let corrected = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: corrected,
        op: IrBinOp::Add,
        lhs: Operand::Value(shifted),
        rhs: Operand::Value(sign_bit),
        ty: IrType::I64,
    });

    // Step 6: truncate back to 32 bits
    insts.push(Instruction::Cast {
        dest,
        src: Operand::Value(corrected),
        from_ty: IrType::I64,
        to_ty: IrType::I32,
    });

    Some(insts)
}

// ─── Unsigned modulo by constant (32-bit) ──────────────────────────────────

/// Expand `dest = x %u C` for 32-bit unsigned x.
/// Uses: x % C = x - (x / C) * C
fn expand_urem32(
    dest: Value,
    x: &Operand,
    d: u32,
    ty: IrType,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    // Power of 2: already handled by simplify pass (x & (d-1))
    if d.is_power_of_two() {
        return None;
    }

    // Compute x / d first
    let quotient = fresh_value(next_id);
    let mut insts = expand_udiv32(quotient, x, d, ty, next_id)?;

    // quotient * d
    let prod = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: prod,
        op: IrBinOp::Mul,
        lhs: Operand::Value(quotient),
        rhs: Operand::Const(IrConst::from_i64(d as i64, ty)),
        ty,
    });

    // x - quotient * d
    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: *x,
        rhs: Operand::Value(prod),
        ty,
    });

    Some(insts)
}

// ─── Signed modulo by constant (32-bit) ────────────────────────────────────

/// Expand `dest = x %s C` for 32-bit signed x.
/// Uses: x % C = x - (x / C) * C
fn expand_srem32(
    dest: Value,
    x: &Operand,
    d: i32,
    ty: IrType,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    // Compute x / d first
    let quotient = fresh_value(next_id);
    let mut insts = expand_sdiv32(quotient, x, d, ty, next_id)?;

    // quotient * d
    let prod = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: prod,
        op: IrBinOp::Mul,
        lhs: Operand::Value(quotient),
        rhs: Operand::Const(IrConst::I32(d)),
        ty,
    });

    // x - quotient * d
    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: *x,
        rhs: Operand::Value(prod),
        ty,
    });

    Some(insts)
}

// ─── Unsigned division by constant (64-bit) ────────────────────────────────

/// Compute the magic number for unsigned 64-bit division by `d`.
/// Returns (magic_number, post_shift, needs_add) where:
/// - If !needs_add: result = mulhi(x, magic) >> post_shift
/// - If needs_add: result = ((x - mulhi(x, magic)) >> 1 + mulhi(x, magic)) >> (post_shift - 1)
///
/// Uses the same algorithm as the 32-bit version, scaled up to 64-bit
/// with 128-bit intermediate arithmetic (available via Rust's u128).
fn compute_unsigned_magic_64(d: u64) -> Option<(u128, u32, bool)> {
    assert!(d >= 2);

    // Try each shift amount starting from 0
    for p in 0u32..64 {
        // magic = ceil(2^(64+p) / d)
        // Use checked shifts to avoid overflow in the 128-bit computation
        let two_pow: u128 = 1u128 << (64 + p);
        let magic = two_pow.div_ceil(d as u128);

        // Check if this magic number works: the error must be <= 2^p
        let error = magic * d as u128 - two_pow;
        if error <= (1u128 << p) {
            if magic <= u64::MAX as u128 {
                // Fits in 64 bits - simple case
                return Some((magic, p, false));
            } else {
                // Doesn't fit in 64 bits - use add-and-shift fixup
                // Subtract 2^64 from magic and compensate
                let m = magic - (1u128 << 64);
                if m <= u64::MAX as u128 {
                    return Some((m, p, true));
                }
            }
        }
    }

    // Fallback for very large divisors near u64::MAX
    None
}

/// Expand `dest = x /u C` for native 64-bit unsigned x using 128-bit multiply.
/// `ty` is the IR type of the original operation (I64 or U64 - on 64-bit targets,
/// widened_op_type maps both to I64, but the UDiv opcode indicates unsigned semantics).
fn expand_udiv64(
    dest: Value,
    x: &Operand,
    d: u64,
    ty: IrType,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    // Power of 2: simple shift
    if d.is_power_of_two() {
        let shift = d.trailing_zeros();
        return Some(vec![Instruction::BinOp {
            dest,
            op: IrBinOp::LShr,
            lhs: *x,
            rhs: Operand::Const(IrConst::I64(shift as i64)),
            ty,
        }]);
    }

    let (magic, shift, needs_add) = compute_unsigned_magic_64(d)?;

    let mut insts = Vec::new();

    // Step 1: cast x to U64 if needed (IR may use I64 for unsigned long long),
    // then zero-extend to 128 bits. We must use U64->U128 to ensure zero-extension.
    let x_u64 = if ty == IrType::U64 {
        *x
    } else {
        // ty is I64 but operation is unsigned - reinterpret bits as unsigned
        let v = fresh_value(next_id);
        insts.push(Instruction::Cast {
            dest: v,
            src: *x,
            from_ty: ty,
            to_ty: IrType::U64,
        });
        Operand::Value(v)
    };

    let x128 = fresh_value(next_id);
    insts.push(Instruction::Cast {
        dest: x128,
        src: x_u64,
        from_ty: IrType::U64,
        to_ty: IrType::U128,
    });

    // Step 2: multiply by magic number (in 128 bits, unsigned)
    let product = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: product,
        op: IrBinOp::Mul,
        lhs: Operand::Value(x128),
        rhs: Operand::Const(IrConst::I128(magic as i128)),
        ty: IrType::U128,
    });

    // Step 3: get high 64 bits (logical shift right by 64)
    let hi128 = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: hi128,
        op: IrBinOp::LShr,
        lhs: Operand::Value(product),
        rhs: Operand::Const(IrConst::I128(64)),
        ty: IrType::U128,
    });

    // Step 4: truncate to 64 bits, then cast back to original type
    let hi_u64 = fresh_value(next_id);
    insts.push(Instruction::Cast {
        dest: hi_u64,
        src: Operand::Value(hi128),
        from_ty: IrType::U128,
        to_ty: IrType::U64,
    });

    // Cast back to the original type (I64 if needed)
    let hi = if ty == IrType::U64 {
        hi_u64
    } else {
        let v = fresh_value(next_id);
        insts.push(Instruction::Cast {
            dest: v,
            src: Operand::Value(hi_u64),
            from_ty: IrType::U64,
            to_ty: ty,
        });
        v
    };

    if !needs_add {
        // Simple case: result = hi >> shift
        if shift == 0 {
            insts.push(Instruction::Copy {
                dest,
                src: Operand::Value(hi),
            });
        } else {
            insts.push(Instruction::BinOp {
                dest,
                op: IrBinOp::LShr,
                lhs: Operand::Value(hi),
                rhs: Operand::Const(IrConst::I64(shift as i64)),
                ty,
            });
        }
    } else {
        // Needs add fixup: result = ((x - hi) >> 1 + hi) >> (shift - 1)
        let diff = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: diff,
            op: IrBinOp::Sub,
            lhs: *x,
            rhs: Operand::Value(hi),
            ty,
        });

        let half = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: half,
            op: IrBinOp::LShr,
            lhs: Operand::Value(diff),
            rhs: Operand::Const(IrConst::I64(1)),
            ty,
        });

        let sum = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: sum,
            op: IrBinOp::Add,
            lhs: Operand::Value(half),
            rhs: Operand::Value(hi),
            ty,
        });

        if shift > 1 {
            insts.push(Instruction::BinOp {
                dest,
                op: IrBinOp::LShr,
                lhs: Operand::Value(sum),
                rhs: Operand::Const(IrConst::I64((shift - 1) as i64)),
                ty,
            });
        } else {
            insts.push(Instruction::Copy {
                dest,
                src: Operand::Value(sum),
            });
        }
    }

    Some(insts)
}

// ─── Signed division by constant (64-bit) ──────────────────────────────────

/// Compute the magic number for signed 64-bit division by `d`.
/// Returns (magic_number, shift_amount).
///
/// Based on Hacker's Delight, 2nd edition, adapted for 64-bit using 128-bit
/// intermediate arithmetic.
fn compute_signed_magic_64(d: i64) -> (i128, u32) {
    assert!(d >= 2);
    let ad = d as u64; // absolute value (d > 0)
    let t: u64 = 0x8000000000000000u64; // 2^63
    let anc = t - 1 - (t % ad); // absolute value of nc
    let mut p: u32 = 63;
    let mut q1: u128 = (1u128 << 63) / anc as u128;
    let mut r1: u128 = (1u128 << 63) - q1 * anc as u128;
    let mut q2: u128 = (1u128 << 63) / ad as u128;
    let mut r2: u128 = (1u128 << 63) - q2 * ad as u128;

    loop {
        p += 1;
        q1 *= 2;
        r1 *= 2;
        if r1 >= anc as u128 {
            q1 += 1;
            r1 -= anc as u128;
        }
        q2 *= 2;
        r2 *= 2;
        if r2 >= ad as u128 {
            q2 += 1;
            r2 -= ad as u128;
        }
        let delta = ad as u128 - r2;
        if q1 < delta || (q1 == delta && r1 == 0) {
            continue;
        }
        break;
    }

    // Magic number (can be negative when interpreted as signed)
    let magic = (q2 + 1) as i128;
    let shift = p - 64;
    (magic, shift)
}

/// Expand `dest = x /s C` for native 64-bit signed x using 128-bit multiply.
fn expand_sdiv64(dest: Value, x: &Operand, d: i64, next_id: &mut u32) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    // Power of 2: x / 2^k => (x + (x >> 63 >>> (64 - k))) >> k
    if d > 0 && (d as u64).is_power_of_two() {
        let k = d.trailing_zeros();
        let mut insts = Vec::new();

        let sign = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: sign,
            op: IrBinOp::AShr,
            lhs: *x,
            rhs: Operand::Const(IrConst::I64(63)),
            ty: IrType::I64,
        });

        let bias = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: bias,
            op: IrBinOp::LShr,
            lhs: Operand::Value(sign),
            rhs: Operand::Const(IrConst::I64(64 - k as i64)),
            ty: IrType::I64,
        });

        let biased = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: biased,
            op: IrBinOp::Add,
            lhs: *x,
            rhs: Operand::Value(bias),
            ty: IrType::I64,
        });

        insts.push(Instruction::BinOp {
            dest,
            op: IrBinOp::AShr,
            lhs: Operand::Value(biased),
            rhs: Operand::Const(IrConst::I64(k as i64)),
            ty: IrType::I64,
        });

        return Some(insts);
    }

    // General case: signed magic number division
    let (magic, shift) = compute_signed_magic_64(d);
    let mut insts = Vec::new();

    // Step 1: sign-extend x to 128 bits
    let x128 = fresh_value(next_id);
    insts.push(Instruction::Cast {
        dest: x128,
        src: *x,
        from_ty: IrType::I64,
        to_ty: IrType::I128,
    });

    // Step 2: multiply by magic number (in 128 bits)
    let product = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: product,
        op: IrBinOp::Mul,
        lhs: Operand::Value(x128),
        rhs: Operand::Const(IrConst::I128(magic)),
        ty: IrType::I128,
    });

    // Step 3: get high 64 bits (arithmetic shift right by 64)
    let hi128 = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: hi128,
        op: IrBinOp::AShr,
        lhs: Operand::Value(product),
        rhs: Operand::Const(IrConst::I128(64)),
        ty: IrType::I128,
    });

    // Step 4: truncate to 64 bits
    let hi = fresh_value(next_id);
    insts.push(Instruction::Cast {
        dest: hi,
        src: Operand::Value(hi128),
        from_ty: IrType::I128,
        to_ty: IrType::I64,
    });

    // Step 5: shift right by shift amount
    let shifted = if shift > 0 {
        let s = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: s,
            op: IrBinOp::AShr,
            lhs: Operand::Value(hi),
            rhs: Operand::Const(IrConst::I64(shift as i64)),
            ty: IrType::I64,
        });
        s
    } else {
        hi
    };

    // Step 6: sign correction: result + (result >>> 63)
    let sign_bit = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: sign_bit,
        op: IrBinOp::LShr,
        lhs: Operand::Value(shifted),
        rhs: Operand::Const(IrConst::I64(63)),
        ty: IrType::I64,
    });

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Add,
        lhs: Operand::Value(shifted),
        rhs: Operand::Value(sign_bit),
        ty: IrType::I64,
    });

    Some(insts)
}

// ─── 64-bit modulo by constant ─────────────────────────────────────────────

/// Expand `dest = x %u C` for 64-bit unsigned x.
fn expand_urem64(
    dest: Value,
    x: &Operand,
    d: u64,
    ty: IrType,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 || d.is_power_of_two() {
        return None;
    }

    let quotient = fresh_value(next_id);
    let mut insts = expand_udiv64(quotient, x, d, ty, next_id)?;

    let prod = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: prod,
        op: IrBinOp::Mul,
        lhs: Operand::Value(quotient),
        rhs: Operand::Const(IrConst::I64(d as i64)),
        ty,
    });

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: *x,
        rhs: Operand::Value(prod),
        ty,
    });

    Some(insts)
}

/// Expand `dest = x %s C` for 64-bit signed x.
fn expand_srem64(dest: Value, x: &Operand, d: i64, next_id: &mut u32) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    let quotient = fresh_value(next_id);
    let mut insts = expand_sdiv64(quotient, x, d, next_id)?;

    let prod = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: prod,
        op: IrBinOp::Mul,
        lhs: Operand::Value(quotient),
        rhs: Operand::Const(IrConst::I64(d)),
        ty: IrType::I64,
    });

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: *x,
        rhs: Operand::Value(prod),
        ty: IrType::I64,
    });

    Some(insts)
}

/// Expand `dest = x /s -C` for 64-bit by computing `-(x /s C)`.
fn expand_sdiv64_neg(
    dest: Value,
    x: &Operand,
    pos_d: i64,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    let quotient = fresh_value(next_id);
    let mut insts = expand_sdiv64(quotient, x, pos_d, next_id)?;

    // Negate: dest = 0 - quotient
    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: Operand::Const(IrConst::I64(0)),
        rhs: Operand::Value(quotient),
        ty: IrType::I64,
    });

    Some(insts)
}

// ─── I64-promoted variants ─────────────────────────────────────────────────
// These handle the common case where C integer promotion widened 32-bit
// operations to I64. The magic number trick works in I64 directly since
// multiplying a 32-bit-range value by a 32-bit magic in 64 bits gives
// exactly the same high 32 bits.

/// Expand `dest = x /u C` where both x and C are in I64 but C fits in u32.
/// x was zero-extended from U32 to I64 by the C lowering.
fn expand_udiv32_in_i64(
    dest: Value,
    x: &Operand,
    d: u32,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }
    if d.is_power_of_two() {
        let shift = d.trailing_zeros();
        return Some(vec![Instruction::BinOp {
            dest,
            op: IrBinOp::LShr,
            lhs: *x,
            rhs: Operand::Const(IrConst::I64(shift as i64)),
            ty: IrType::I64,
        }]);
    }

    let (magic, shift, needs_add) = compute_unsigned_magic_32(d)?;
    let mut insts = Vec::new();

    // Ensure the input is zero-extended to 64 bits. The LHS is known to be
    // a 32-bit value (from is_known_u32 tracking), but it may be a function
    // parameter where the upper 32 bits are undefined on some architectures
    // (ARM64, RISC-V). Mask to guarantee correct unsigned 32-bit range.
    let x_masked = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: x_masked,
        op: IrBinOp::And,
        lhs: *x,
        rhs: Operand::Const(IrConst::I64(0xFFFFFFFF)),
        ty: IrType::I64,
    });
    let x_safe = Operand::Value(x_masked);

    // Multiply by magic number (in 64 bits). Since x fits in u32 and magic
    // fits in u32, the product fits in u64 without overflow.
    let product = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: product,
        op: IrBinOp::Mul,
        lhs: x_safe,
        rhs: Operand::Const(IrConst::I64(magic as i64)),
        ty: IrType::I64,
    });

    // Get high 32 bits
    let hi = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: hi,
        op: IrBinOp::LShr,
        lhs: Operand::Value(product),
        rhs: Operand::Const(IrConst::I64(32)),
        ty: IrType::I64,
    });

    if !needs_add {
        if shift == 0 {
            // Result is hi, already in I64
            insts.push(Instruction::Copy {
                dest,
                src: Operand::Value(hi),
            });
        } else {
            insts.push(Instruction::BinOp {
                dest,
                op: IrBinOp::LShr,
                lhs: Operand::Value(hi),
                rhs: Operand::Const(IrConst::I64(shift as i64)),
                ty: IrType::I64,
            });
        }
    } else {
        // Add fixup: ((x - hi) >> 1 + hi) >> (shift - 1)
        let diff = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: diff,
            op: IrBinOp::Sub,
            lhs: x_safe,
            rhs: Operand::Value(hi),
            ty: IrType::I64,
        });

        let half = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: half,
            op: IrBinOp::LShr,
            lhs: Operand::Value(diff),
            rhs: Operand::Const(IrConst::I64(1)),
            ty: IrType::I64,
        });

        let sum = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: sum,
            op: IrBinOp::Add,
            lhs: Operand::Value(half),
            rhs: Operand::Value(hi),
            ty: IrType::I64,
        });

        if shift > 1 {
            insts.push(Instruction::BinOp {
                dest,
                op: IrBinOp::LShr,
                lhs: Operand::Value(sum),
                rhs: Operand::Const(IrConst::I64((shift - 1) as i64)),
                ty: IrType::I64,
            });
        } else {
            insts.push(Instruction::Copy {
                dest,
                src: Operand::Value(sum),
            });
        }
    }

    Some(insts)
}

/// Expand `dest = x /s C` where both x and C are in I64 but C fits in i32.
/// x was sign-extended from I32 to I64 by the C lowering.
fn expand_sdiv32_in_i64(
    dest: Value,
    x: &Operand,
    d: i32,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    // Power of 2: x / 2^k => (x + (x >> 63 >>> (64 - k))) >> k
    // In I64, we use 63-bit shift and 64-k logical shift
    if d > 0 && (d as u32).is_power_of_two() {
        let k = d.trailing_zeros();
        let mut insts = Vec::new();

        let sign = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: sign,
            op: IrBinOp::AShr,
            lhs: *x,
            rhs: Operand::Const(IrConst::I64(63)),
            ty: IrType::I64,
        });

        // Bias correction for signed power-of-2 division in I64:
        // A sign-extended 32-bit value in I64 has all upper bits equal to
        // the sign bit, so the standard 64-bit pattern works correctly:
        // bias = sign >>> (64 - k) isolates the low k bits of the sign mask.
        let bias = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: bias,
            op: IrBinOp::LShr,
            lhs: Operand::Value(sign),
            rhs: Operand::Const(IrConst::I64(64 - k as i64)),
            ty: IrType::I64,
        });

        let biased = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: biased,
            op: IrBinOp::Add,
            lhs: *x,
            rhs: Operand::Value(bias),
            ty: IrType::I64,
        });

        insts.push(Instruction::BinOp {
            dest,
            op: IrBinOp::AShr,
            lhs: Operand::Value(biased),
            rhs: Operand::Const(IrConst::I64(k as i64)),
            ty: IrType::I64,
        });

        return Some(insts);
    }

    // General case: signed magic number division
    let (magic, shift) = compute_signed_magic_32(d);
    let mut insts = Vec::new();

    // x is already in I64 (sign-extended from I32).
    // Multiply by magic number (in 64 bits).
    let product = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: product,
        op: IrBinOp::Mul,
        lhs: *x,
        rhs: Operand::Const(IrConst::I64(magic)),
        ty: IrType::I64,
    });

    // Get high 32 bits (arithmetic shift for signed)
    let hi = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: hi,
        op: IrBinOp::AShr,
        lhs: Operand::Value(product),
        rhs: Operand::Const(IrConst::I64(32)),
        ty: IrType::I64,
    });

    // Shift right by shift amount
    let shifted = if shift > 0 {
        let s = fresh_value(next_id);
        insts.push(Instruction::BinOp {
            dest: s,
            op: IrBinOp::AShr,
            lhs: Operand::Value(hi),
            rhs: Operand::Const(IrConst::I64(shift as i64)),
            ty: IrType::I64,
        });
        s
    } else {
        hi
    };

    // Sign correction: result + (result >>> 63)
    let sign_bit = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: sign_bit,
        op: IrBinOp::LShr,
        lhs: Operand::Value(shifted),
        rhs: Operand::Const(IrConst::I64(63)),
        ty: IrType::I64,
    });

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Add,
        lhs: Operand::Value(shifted),
        rhs: Operand::Value(sign_bit),
        ty: IrType::I64,
    });

    Some(insts)
}

/// Expand `dest = x /s -C` by computing `-(x /s C)`.
/// Handles both native I32 and I64-promoted cases.
fn expand_sdiv_neg(
    dest: Value,
    x: &Operand,
    pos_d: i32,
    ty: IrType,
    lhs_is_i32: &dyn Fn(&Operand) -> bool,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    // Compute x / pos_d into a temporary
    let quotient = fresh_value(next_id);
    let mut insts = match ty {
        IrType::I32 => expand_sdiv32(quotient, x, pos_d, ty, next_id)?,
        IrType::I64 if lhs_is_i32(x) => expand_sdiv32_in_i64(quotient, x, pos_d, next_id)?,
        _ => return None,
    };

    // Negate: dest = 0 - quotient
    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: Operand::Const(IrConst::from_i64(0, ty)),
        rhs: Operand::Value(quotient),
        ty,
    });

    Some(insts)
}

/// Expand `dest = x %u C` in I64.
fn expand_urem32_in_i64(
    dest: Value,
    x: &Operand,
    d: u32,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 || d.is_power_of_two() {
        return None;
    }

    let quotient = fresh_value(next_id);
    let mut insts = expand_udiv32_in_i64(quotient, x, d, next_id)?;

    let prod = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: prod,
        op: IrBinOp::Mul,
        lhs: Operand::Value(quotient),
        rhs: Operand::Const(IrConst::I64(d as i64)),
        ty: IrType::I64,
    });

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: *x,
        rhs: Operand::Value(prod),
        ty: IrType::I64,
    });

    Some(insts)
}

/// Expand `dest = x %s C` in I64.
fn expand_srem32_in_i64(
    dest: Value,
    x: &Operand,
    d: i32,
    next_id: &mut u32,
) -> Option<Vec<Instruction>> {
    if d < 2 {
        return None;
    }

    let quotient = fresh_value(next_id);
    let mut insts = expand_sdiv32_in_i64(quotient, x, d, next_id)?;

    let prod = fresh_value(next_id);
    insts.push(Instruction::BinOp {
        dest: prod,
        op: IrBinOp::Mul,
        lhs: Operand::Value(quotient),
        rhs: Operand::Const(IrConst::I64(d as i64)),
        ty: IrType::I64,
    });

    insts.push(Instruction::BinOp {
        dest,
        op: IrBinOp::Sub,
        lhs: *x,
        rhs: Operand::Value(prod),
        ty: IrType::I64,
    });

    Some(insts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unsigned_magic_7() {
        let (magic, shift, needs_add) = compute_unsigned_magic_32(7).unwrap();
        // Verify: for all u32, (x * magic) >> 32 >> shift should == x / 7
        for &x in &[
            0u32,
            1,
            6,
            7,
            8,
            13,
            14,
            100,
            255,
            1000,
            u32::MAX,
            u32::MAX - 1,
        ] {
            let result = if !needs_add {
                ((x as u64 * magic) >> 32 >> shift) as u32
            } else {
                let hi = (x as u64 * magic) >> 32;
                let diff = x as u64 - hi;
                ((diff >> 1).wrapping_add(hi) >> (shift - 1)) as u32
            };
            assert_eq!(result, x / 7, "Failed for x={}", x);
        }
    }

    #[test]
    fn test_unsigned_magic_10() {
        let (magic, shift, needs_add) = compute_unsigned_magic_32(10).unwrap();
        for &x in &[
            0u32,
            1,
            9,
            10,
            11,
            99,
            100,
            255,
            1000,
            u32::MAX,
            u32::MAX - 1,
        ] {
            let result = if !needs_add {
                ((x as u64 * magic) >> 32 >> shift) as u32
            } else {
                let hi = (x as u64 * magic) >> 32;
                let diff = x as u64 - hi;
                ((diff >> 1).wrapping_add(hi) >> (shift - 1)) as u32
            };
            assert_eq!(result, x / 10, "Failed for x={}", x);
        }
    }

    #[test]
    fn test_unsigned_magic_3() {
        let (magic, shift, needs_add) = compute_unsigned_magic_32(3).unwrap();
        for &x in &[
            0u32,
            1,
            2,
            3,
            4,
            5,
            6,
            100,
            u32::MAX,
            u32::MAX - 1,
            u32::MAX - 2,
        ] {
            let result = if !needs_add {
                ((x as u64 * magic) >> 32 >> shift) as u32
            } else {
                let hi = (x as u64 * magic) >> 32;
                let diff = x as u64 - hi;
                ((diff >> 1).wrapping_add(hi) >> (shift - 1)) as u32
            };
            assert_eq!(result, x / 3, "Failed for x={}", x);
        }
    }

    #[test]
    fn test_signed_magic_7() {
        let (magic, shift) = compute_signed_magic_32(7);
        for &x in &[
            0i32,
            1,
            -1,
            6,
            7,
            -7,
            8,
            -8,
            13,
            14,
            100,
            -100,
            i32::MAX,
            i32::MIN + 1,
        ] {
            let x64 = x as i64;
            let product = x64 * magic;
            let hi = product >> 32;
            let shifted = hi >> shift;
            let sign_bit = (shifted as u64) >> 63;
            let result = (shifted + sign_bit as i64) as i32;
            assert_eq!(result, x / 7, "Failed for x={}", x);
        }
    }

    #[test]
    fn test_signed_magic_10() {
        let (magic, shift) = compute_signed_magic_32(10);
        for &x in &[
            0i32,
            1,
            -1,
            9,
            10,
            -10,
            11,
            -11,
            99,
            100,
            -100,
            i32::MAX,
            i32::MIN + 1,
        ] {
            let x64 = x as i64;
            let product = x64 * magic;
            let hi = product >> 32;
            let shifted = hi >> shift;
            let sign_bit = (shifted as u64) >> 63;
            let result = (shifted + sign_bit as i64) as i32;
            assert_eq!(result, x / 10, "Failed for x={}", x);
        }
    }

    #[test]
    fn test_unsigned_magic_exhaustive_small() {
        // Exhaustively test small divisors with a representative range
        for d in 2u32..=20 {
            let (magic, shift, needs_add) = compute_unsigned_magic_32(d).unwrap();
            for x in (0..1000u32).chain(u32::MAX - 1000..=u32::MAX) {
                let result = if !needs_add {
                    ((x as u64 * magic) >> 32 >> shift) as u32
                } else {
                    let hi = (x as u64 * magic) >> 32;
                    let diff = x as u64 - hi;
                    ((diff >> 1).wrapping_add(hi) >> (shift - 1)) as u32
                };
                assert_eq!(result, x / d, "Failed for x={} d={}", x, d);
            }
        }
    }

    #[test]
    fn test_signed_magic_exhaustive_small() {
        // Exhaustively test small divisors with a representative range
        for d in 2i32..=20 {
            let (magic, shift) = compute_signed_magic_32(d);
            let test_vals: Vec<i32> = (-1000..1000)
                .chain(std::iter::once(i32::MAX))
                .chain(std::iter::once(i32::MIN + 1))
                .collect();
            for &x in &test_vals {
                let x64 = x as i64;
                let product = x64 * magic;
                let hi = product >> 32;
                let shifted = hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let result = (shifted + sign_bit as i64) as i32;
                assert_eq!(result, x / d, "Failed for x={} d={}", x, d);
            }
        }
    }

    #[test]
    fn test_negative_divisor_sdiv() {
        // Negative divisor: x / -C == -(x / C)
        // We compute x / C using the magic number, then negate.
        for pos_d in 2i32..=20 {
            let neg_d = -pos_d;
            let (magic, shift) = compute_signed_magic_32(pos_d);
            let test_vals: Vec<i32> = (-1000..1000)
                .chain(std::iter::once(i32::MAX))
                .chain(std::iter::once(i32::MIN + 1))
                .collect();
            for &x in &test_vals {
                // Compute x / pos_d using magic numbers
                let x64 = x as i64;
                let product = x64 * magic;
                let hi = product >> 32;
                let shifted = hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let pos_result = (shifted + sign_bit as i64) as i32;
                // Negate for negative divisor
                let result = -pos_result;
                assert_eq!(result, x / neg_d, "Failed for x={} d={}", x, neg_d);
            }
        }
    }

    #[test]
    fn test_negative_divisor_srem() {
        // Negative divisor remainder: x % -C == x % C (sign follows dividend in C)
        for pos_d in 2i32..=20 {
            let neg_d = -pos_d;
            let test_vals: Vec<i32> = (-1000..1000)
                .chain(std::iter::once(i32::MAX))
                .chain(std::iter::once(i32::MIN + 1))
                .collect();
            for &x in &test_vals {
                assert_eq!(
                    x % pos_d,
                    x % neg_d,
                    "x % {} != x % {} for x={}",
                    pos_d,
                    neg_d,
                    x
                );
            }
        }
    }

    // ─── 64-bit unsigned magic number tests ────────────────────────────────

    #[test]
    fn test_unsigned_magic_64_div10() {
        let (magic, shift, needs_add) = compute_unsigned_magic_64(10).unwrap();
        let test_vals: Vec<u64> = vec![
            0,
            1,
            9,
            10,
            11,
            99,
            100,
            255,
            1000,
            u32::MAX as u64,
            u32::MAX as u64 + 1,
            u64::MAX,
            u64::MAX - 1,
            u64::MAX / 2,
            12345678901234567,
            0xCCCCCCCCCCCCCCCC,
        ];
        for x in test_vals {
            let result = if !needs_add {
                let product = (x as u128 * magic) >> 64 >> shift;
                product as u64
            } else {
                let hi = ((x as u128 * magic) >> 64) as u64;
                let diff = x - hi;
                (diff >> 1).wrapping_add(hi) >> (shift - 1)
            };
            assert_eq!(result, x / 10, "Failed for x={}", x);
        }
    }

    #[test]
    fn test_unsigned_magic_64_exhaustive_small_divisors() {
        for d in 2u64..=20 {
            let (magic, shift, needs_add) = compute_unsigned_magic_64(d).unwrap();
            let test_vals: Vec<u64> = (0..1000u64)
                .chain(u64::MAX - 1000..=u64::MAX)
                .chain(std::iter::once(u32::MAX as u64))
                .chain(std::iter::once(u32::MAX as u64 + 1))
                .collect();
            for x in test_vals {
                let result = if !needs_add {
                    ((x as u128 * magic) >> 64 >> shift) as u64
                } else {
                    let hi = ((x as u128 * magic) >> 64) as u64;
                    let diff = x - hi;
                    (diff >> 1).wrapping_add(hi) >> (shift - 1)
                };
                assert_eq!(result, x / d, "Failed for x={} d={}", x, d);
            }
        }
    }

    #[test]
    fn test_unsigned_magic_64_large_divisors() {
        // Test with divisors that don't fit in 32 bits
        let large_divisors: Vec<u64> = vec![
            u32::MAX as u64 + 1, // 2^32
            0x100000000,         // 2^32
            0x123456789ABCDEF0,
            u64::MAX / 2,
            u64::MAX / 3,
            1_000_000_000_000, // 10^12
        ];
        for d in large_divisors {
            if let Some((magic, shift, needs_add)) = compute_unsigned_magic_64(d) {
                for &x in &[0u64, 1, d - 1, d, d + 1, d * 2, u64::MAX, u64::MAX - 1] {
                    let result = if !needs_add {
                        ((x as u128 * magic) >> 64 >> shift) as u64
                    } else {
                        let hi = ((x as u128 * magic) >> 64) as u64;
                        let diff = x - hi;
                        (diff >> 1).wrapping_add(hi) >> (shift - 1)
                    };
                    assert_eq!(result, x / d, "Failed for x={} d={}", x, d);
                }
            }
        }
    }

    // ─── 64-bit signed magic number tests ──────────────────────────────────

    #[test]
    fn test_signed_magic_64_div10() {
        let (magic, shift) = compute_signed_magic_64(10);
        let test_vals: Vec<i64> = vec![
            0,
            1,
            -1,
            9,
            10,
            -10,
            11,
            -11,
            99,
            100,
            -100,
            i64::MAX,
            i64::MIN + 1,
            1234567890123456789,
            -1234567890123456789,
        ];
        for x in test_vals {
            let x128 = x as i128;
            let product = x128 * magic;
            let hi = product >> 64;
            let shifted = hi >> shift;
            let sign_bit = (shifted as u64) >> 63;
            let result = (shifted + sign_bit as i128) as i64;
            assert_eq!(result, x / 10, "Failed for x={}", x);
        }
    }

    #[test]
    fn test_signed_magic_64_exhaustive_small_divisors() {
        for d in 2i64..=20 {
            let (magic, shift) = compute_signed_magic_64(d);
            let test_vals: Vec<i64> = (-1000..1000i64)
                .chain(std::iter::once(i64::MAX))
                .chain(std::iter::once(i64::MIN + 1))
                .chain(std::iter::once(i64::MAX / d * d))
                .chain(std::iter::once(i64::MIN / d * d))
                .collect();
            for x in test_vals {
                let x128 = x as i128;
                let product = x128 * magic;
                let hi = product >> 64;
                let shifted = hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let result = (shifted + sign_bit as i128) as i64;
                assert_eq!(result, x / d, "Failed for x={} d={}", x, d);
            }
        }
    }

    #[test]
    fn test_signed_magic_64_large_divisors() {
        // Test with divisors that don't fit in 32 bits
        let large_divisors: Vec<i64> = vec![
            i32::MAX as i64 + 1,
            1_000_000_000_000, // 10^12
            i64::MAX / 3,
        ];
        for d in large_divisors {
            let (magic, shift) = compute_signed_magic_64(d);
            let test_vals: Vec<i64> = vec![
                0,
                1,
                -1,
                d - 1,
                d,
                d + 1,
                -d + 1,
                -d,
                -d - 1,
                i64::MAX,
                i64::MIN + 1,
            ];
            for x in test_vals {
                let x128 = x as i128;
                let product = x128 * magic;
                let hi = product >> 64;
                let shifted = hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let result = (shifted + sign_bit as i128) as i64;
                assert_eq!(result, x / d, "Failed for x={} d={}", x, d);
            }
        }
    }

    #[test]
    fn test_negative_divisor_sdiv64() {
        // Negative 64-bit divisor: x / -C == -(x / C)
        for pos_d in [2i64, 3, 7, 10, 100, 1_000_000_000_000] {
            let (magic, shift) = compute_signed_magic_64(pos_d);
            let test_vals: Vec<i64> = vec![0, 1, -1, 100, -100, i64::MAX, i64::MIN + 1];
            for x in test_vals {
                let x128 = x as i128;
                let product = x128 * magic;
                let hi = product >> 64;
                let shifted = hi >> shift;
                let sign_bit = (shifted as u64) >> 63;
                let pos_result = (shifted + sign_bit as i128) as i64;
                let result = -pos_result;
                assert_eq!(result, x / (-pos_d), "Failed for x={} d={}", x, -pos_d);
            }
        }
    }
}
