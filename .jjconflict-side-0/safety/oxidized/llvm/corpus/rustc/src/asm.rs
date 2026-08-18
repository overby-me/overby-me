//@ flags: -Cpanic=abort
//! Inline assembly, which is a call to a value rather than to a symbol and
//! carries a constraint string the rest of the corpus never exercises. The
//! forms here are the ones the constraint grammar turns on: an output, an
//! input, a read-write operand tied to its output, a clobber, and a block
//! with `options(nostack)` so the `sideeffect` marker moves.
//!
//! The register names are x86-64's, which is the target the whole corpus is
//! generated for.
#![no_std]

use core::arch::asm;

pub fn identity(x: u64) -> u64 {
    let out: u64;
    unsafe {
        asm!("mov {0}, {1}", out(reg) out, in(reg) x);
    }
    out
}

pub fn increment(mut x: u64) -> u64 {
    unsafe {
        asm!("inc {0}", inout(reg) x);
    }
    x
}

pub fn add(a: u64, b: u64) -> u64 {
    let out: u64;
    unsafe {
        asm!("mov {0}, {1}", "add {0}, {2}", out(reg) out, in(reg) a, in(reg) b);
    }
    out
}

pub fn barrier() {
    unsafe {
        asm!("", options(nostack, preserves_flags));
    }
}

pub fn clobbering(x: u64) -> u64 {
    let out: u64;
    unsafe {
        asm!("mov rax, {0}", "mov {1}, rax", in(reg) x, out(reg) out, out("rax") _);
    }
    out
}
