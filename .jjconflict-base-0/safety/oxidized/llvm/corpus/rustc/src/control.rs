//@ flags: -Cpanic=abort
//! Control flow: conditional branches, loops, dense and sparse matches, and
//! the unreachable terminator. Produces switch instructions with several
//! cases, phi-free block structure at O0, and predecessor lists worth
//! reproducing exactly.
#![no_std]

#[unsafe(no_mangle)]
pub fn branch(c: bool, x: i32, y: i32) -> i32 {
    if c { x } else { y }
}

#[unsafe(no_mangle)]
pub fn nested(a: i32, b: i32) -> i32 {
    if a > 0 {
        if b > 0 { a + b } else { a - b }
    } else if b > 0 {
        b - a
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub fn counted_loop(n: u32) -> u32 {
    let mut total = 0u32;
    let mut i = 0u32;
    while i < n {
        total = total.wrapping_add(i);
        i += 1;
    }
    total
}

#[unsafe(no_mangle)]
pub fn dense_match(x: u8) -> u32 {
    match x {
        0 => 10,
        1 => 20,
        2 => 30,
        3 => 40,
        4 => 50,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub fn sparse_match(x: u32) -> u32 {
    match x {
        1 => 1,
        1000 => 2,
        1000000 => 3,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub fn early_exit(xs: &[u8], needle: u8) -> bool {
    let mut i = 0;
    while i < xs.len() {
        if xs[i] == needle {
            return true;
        }
        i += 1;
    }
    false
}

#[unsafe(no_mangle)]
pub fn never_returns() -> ! {
    loop {}
}
