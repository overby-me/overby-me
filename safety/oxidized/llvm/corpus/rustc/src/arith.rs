//@ flags: -Cpanic=abort
//! Integer and floating-point arithmetic, comparisons and conversions.
//! Exercises the binary operators, their no-wrap flags, every cast opcode
//! rustc reaches for, and both comparison instructions.
#![no_std]

#[unsafe(no_mangle)]
pub fn wrapping(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
        .wrapping_sub(b)
        .wrapping_mul(a)
        .wrapping_shl(3)
}

#[unsafe(no_mangle)]
pub fn unsigned_ops(a: u64, b: u64) -> u64 {
    (a / b) ^ (a % b) | (a & b) | (a >> 2) | (a << 1)
}

#[unsafe(no_mangle)]
pub fn signed_shift(a: i64, b: u32) -> i64 {
    a >> b
}

#[unsafe(no_mangle)]
pub fn checked(a: i32, b: i32) -> Option<i32> {
    a.checked_add(b)
}

#[unsafe(no_mangle)]
pub fn floats(x: f64, y: f64) -> f64 {
    (x + y) * (x - y) / (y + 1.0) - (x % y)
}

#[unsafe(no_mangle)]
pub fn single(x: f32) -> f32 {
    -x * 0.5
}

#[unsafe(no_mangle)]
pub fn compare_ints(a: i32, b: i32) -> bool {
    a < b && a != b
}

#[unsafe(no_mangle)]
pub fn compare_unsigned(a: u32, b: u32) -> bool {
    a >= b || a == b
}

#[unsafe(no_mangle)]
pub fn compare_floats(x: f64, y: f64) -> bool {
    x <= y
}

#[unsafe(no_mangle)]
pub fn conversions(a: i8, b: u16, x: f32) -> (i64, u8, f64, i32, u64) {
    (
        a as i64,
        b as u8,
        x as f64,
        x as i32,
        (x as f64).to_bits(),
    )
}

#[unsafe(no_mangle)]
pub fn bool_to_int(a: bool) -> u8 {
    a as u8
}

#[unsafe(no_mangle)]
pub fn wide_math(a: i128, b: u128) -> i128 {
    a.wrapping_mul(b as i128).wrapping_add(1)
}
