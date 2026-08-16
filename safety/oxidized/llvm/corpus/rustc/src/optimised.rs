//@ flags: -Cpanic=abort -Copt-level=3 -Ccodegen-units=1
//! The other seeds are built at `-Copt-level=0`, which keeps the IR close to
//! what the front end emitted. This one is built at `-Copt-level=3`, where
//! LLVM's own passes have run: loops are unrolled and vectorised, so the
//! output holds vector types, shuffles, and the `!llvm.loop` metadata a
//! transformed loop carries, none of which an unoptimised build produces.
//!
//! One codegen unit, because `--emit=llvm-ir -o` writes a single module and
//! rustc splits an optimised crate across sixteen by default, which would
//! leave whichever one happened to be written. Every function is
//! `no_mangle`, because at `-Copt-level=2` and above a plain `pub fn` in a
//! library crate is internalised and then deleted, which is how the first
//! version of this seed came out holding nothing at all.
#![no_std]

#[unsafe(no_mangle)]
pub fn sum(values: &[i32]) -> i32 {
    let mut total = 0i32;
    let mut index = 0;
    while index < values.len() {
        total = total.wrapping_add(values[index]);
        index += 1;
    }
    total
}

#[unsafe(no_mangle)]
pub fn scale(values: &mut [f32], factor: f32) {
    let mut index = 0;
    while index < values.len() {
        values[index] *= factor;
        index += 1;
    }
}

#[unsafe(no_mangle)]
pub fn find(values: &[u8], needle: u8) -> usize {
    let mut index = 0;
    while index < values.len() {
        if values[index] == needle {
            return index;
        }
        index += 1;
    }
    values.len()
}

#[unsafe(no_mangle)]
pub fn saturate(values: &mut [i16]) {
    let mut index = 0;
    while index < values.len() {
        values[index] = values[index].saturating_add(1);
        index += 1;
    }
}
