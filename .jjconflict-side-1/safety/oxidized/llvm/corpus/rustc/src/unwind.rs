//@ flags: -Cpanic=unwind
//! The unwinding shapes: invoke, landingpad with a cleanup clause, resume,
//! and the personality function on the enclosing function. This is the one
//! seed built with the default panic strategy rather than `panic=abort`.
#![no_std]

pub struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        FLAG.store(1, core::sync::atomic::Ordering::SeqCst);
    }
}

pub static FLAG: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

unsafe extern "Rust" {
    fn may_panic(x: u32) -> u32;
}

#[unsafe(no_mangle)]
pub fn with_guard(x: u32) -> u32 {
    let _guard = Guard;
    // SAFETY: `may_panic` is never linked; the seed only has to compile so
    // that rustc emits an invoke around a call that can unwind.
    unsafe { may_panic(x) }
}

#[unsafe(no_mangle)]
pub fn two_guards(x: u32) -> u32 {
    let _a = Guard;
    let _b = Guard;
    // SAFETY: as above.
    unsafe { may_panic(x) }
}

#[unsafe(no_mangle)]
pub fn indexing_can_panic(xs: &[u32], i: usize) -> u32 {
    let _guard = Guard;
    xs[i]
}
