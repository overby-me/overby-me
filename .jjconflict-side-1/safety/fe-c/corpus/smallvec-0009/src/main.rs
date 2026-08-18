//! RUSTSEC-2019-0009 reproducer against real `smallvec` 0.6.9 — a heap
//! use-after-free via `SmallVec::grow()`.
//!
//! Calling `grow` on a **spilled** (heap-backed) SmallVec with a value equal to
//! its current capacity frees the existing heap data (the bug), leaving the
//! SmallVec pointing at freed memory. A subsequent read reads the freed buffer.
//!
//! The spilled buffer is a global-allocator (`FecAlloc`) allocation; grow frees
//! it (poison keeps it findable-as-dead in quarantine, F2), and the read of the
//! freed buffer resolves the dead allocation and aborts `UseAfterFree`.

extern crate cementite;

use smallvec::SmallVec;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

fn main() {
    let mut v: SmallVec<[u8; 2]> = SmallVec::new();
    // Spill to the heap (more than the 2-element inline capacity).
    v.push(0x11);
    v.push(0x22);
    v.push(0x33);
    v.push(0x44);
    let cap = v.capacity();
    // grow(current capacity) frees the existing heap data (RUSTSEC-2019-0009).
    v.grow(cap);
    // Read the SmallVec: reads the freed heap buffer -> use-after-free.
    let x = v[0];
    println!("NO_ABORT x={x:#x}");
    std::hint::black_box(x);
}
