//@ flags: -Cpanic=abort
//! Atomics and thread-local state: every read-modify-write operation rustc
//! lowers directly, compare-exchange in both strengths, fences, and all six
//! memory orderings.
#![no_std]

use core::sync::atomic::{
    AtomicI32, AtomicIsize, AtomicPtr, AtomicU8, AtomicU64, Ordering, compiler_fence, fence,
};

pub static COUNTER: AtomicU64 = AtomicU64::new(0);
pub static FLAG: AtomicU8 = AtomicU8::new(0);
pub static SIGNED: AtomicI32 = AtomicI32::new(-1);
pub static SIZE: AtomicIsize = AtomicIsize::new(0);
pub static POINTER: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());

#[unsafe(no_mangle)]
pub fn add_relaxed(n: u64) -> u64 {
    COUNTER.fetch_add(n, Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub fn sub_release(n: u64) -> u64 {
    COUNTER.fetch_sub(n, Ordering::Release)
}

#[unsafe(no_mangle)]
pub fn and_acquire(n: u8) -> u8 {
    FLAG.fetch_and(n, Ordering::Acquire)
}

#[unsafe(no_mangle)]
pub fn or_acqrel(n: u8) -> u8 {
    FLAG.fetch_or(n, Ordering::AcqRel)
}

#[unsafe(no_mangle)]
pub fn xor_seqcst(n: u8) -> u8 {
    FLAG.fetch_xor(n, Ordering::SeqCst)
}

#[unsafe(no_mangle)]
pub fn nand(n: u8) -> u8 {
    FLAG.fetch_nand(n, Ordering::SeqCst)
}

#[unsafe(no_mangle)]
pub fn signed_max(n: i32) -> i32 {
    SIGNED.fetch_max(n, Ordering::SeqCst)
}

#[unsafe(no_mangle)]
pub fn signed_min(n: i32) -> i32 {
    SIGNED.fetch_min(n, Ordering::SeqCst)
}

#[unsafe(no_mangle)]
pub fn unsigned_max(n: u64) -> u64 {
    COUNTER.fetch_max(n, Ordering::SeqCst)
}

#[unsafe(no_mangle)]
pub fn swap(n: u64) -> u64 {
    COUNTER.swap(n, Ordering::SeqCst)
}

#[unsafe(no_mangle)]
pub fn strong_exchange(old: u64, new: u64) -> Result<u64, u64> {
    COUNTER.compare_exchange(old, new, Ordering::SeqCst, Ordering::Acquire)
}

#[unsafe(no_mangle)]
pub fn weak_exchange(old: u64, new: u64) -> Result<u64, u64> {
    COUNTER.compare_exchange_weak(old, new, Ordering::Release, Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub fn load_store(p: *mut u8) {
    POINTER.store(p, Ordering::Release);
    SIZE.store(POINTER.load(Ordering::Acquire) as isize, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub fn fences() {
    fence(Ordering::SeqCst);
    fence(Ordering::Acquire);
    compiler_fence(Ordering::Release);
}
