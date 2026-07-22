//! RUSTSEC-2021-0003 control: the *patched* smallvec 1.6.1. The identical
//! program must run clean under Fe-C instrumentation — every injected check
//! passes because 1.6.1 reserves correctly and never overflows.

extern crate cementite;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

use smallvec::SmallVec;

fn main() {
    let mut v: SmallVec<[u8; 0]> = SmallVec::new();
    v.push(123);
    let s = String::from("neighbouring allocation");
    let iter = (0u8..=255).filter(|n| n % 2 == 0);
    v.insert_many(0, iter);
    // Reached: no overflow, so no trap.
    println!("CONTROL_OK len={} s_len={}", v.len(), s.len());
}
