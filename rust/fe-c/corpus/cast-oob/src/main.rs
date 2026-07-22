//! Raw->safe cast out-of-bounds (point 1 / trace §3.1 `ensure`). A raw pointer
//! past the end of an allocation is cast to a safe reference (`&*bad`). The
//! reference is spatially out of bounds, but the borrow checker cannot see it —
//! the pointer arithmetic launders the provenance.
//!
//! The raw->safe cast ensure resolves the pointer's derivation root (the
//! `Vec`'s buffer) and validates the referent's `[bad, bad+8)` extent lies
//! inside it, aborting `OutOfBounds`. This fires in **both** modes: `case`
//! would elide the reference's later dereferences, so the cast is the only
//! checkpoint; and `through`'s deref check resolves the *faulting* address,
//! which is off the end (unknown provenance), so it too relies on the cast
//! ensure to catch an out-of-bounds mint.

extern crate cementite;

use std::hint::black_box;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

fn main() {
    let v: Vec<u64> = black_box(vec![0x1111_1111_1111_1111; 4]); // 32-byte buffer
    let base = v.as_ptr();
    eprintln!("BASE={base:p}");
    // A raw pointer six elements in — past the four-element (32-byte) buffer.
    let bad = unsafe { base.add(6) };
    // Raw->safe cast of the out-of-bounds pointer: the cast ensure aborts here.
    let r: &u64 = unsafe { &*bad };
    let x = *r;
    println!("NO_ABORT x={x:#x}");
    black_box(x);
}
