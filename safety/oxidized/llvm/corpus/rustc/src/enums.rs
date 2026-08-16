//@ flags: -Cpanic=abort
//! Discriminants and the switches that read them. A C-like enum matched
//! exhaustively becomes a `switch` with a dense case list; a niche-optimised
//! `Option<&T>` becomes a null test; and a payload enum becomes a switch on
//! the tag followed by a load from the variant's field. The unreachable arm
//! rustc emits for an exhaustive match is where `unreachable` comes from.
#![no_std]

#[derive(Clone, Copy)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Shl,
    Shr,
}

pub fn apply(op: Op, a: i32, b: i32) -> i32 {
    match op {
        Op::Add => a.wrapping_add(b),
        Op::Sub => a.wrapping_sub(b),
        Op::Mul => a.wrapping_mul(b),
        Op::Div => a.wrapping_div(b),
        Op::Rem => a.wrapping_rem(b),
        Op::Shl => a.wrapping_shl(b as u32),
        Op::Shr => a.wrapping_shr(b as u32),
    }
}

pub enum Value {
    Nothing,
    Byte(u8),
    Word(u32),
    Pair(u32, u32),
}

pub fn widen(value: &Value) -> u64 {
    match value {
        Value::Nothing => 0,
        Value::Byte(b) => u64::from(*b),
        Value::Word(w) => u64::from(*w),
        Value::Pair(a, b) => u64::from(*a) << 32 | u64::from(*b),
    }
}

pub fn deref_or(option: Option<&u32>, fallback: u32) -> u32 {
    match option {
        Some(value) => *value,
        None => fallback,
    }
}
