//! RUSTSEC-2020-0039 / CVE-2020-35892 reproducer against real `simple-slab`
//! 0.3.2 — an out-of-bounds read via unchecked `Slab::index()`.
//!
//! `index()` is `&*(self.mem.offset(index))` with no bounds check, so indexing
//! past the end reads out of bounds. `self.mem` is a `libc::malloc` allocation,
//! caught by cementite's interpose tier (A4): the `#[no_mangle] malloc` override
//! registers the buffer, the opaque-origin root fix roots the malloc'd pointer
//! (so the index offset resolves from the base, not the faulting address), and
//! the instrumented reborrow aborts OutOfBounds. No FecAlloc — interposition
//! tracks the allocation; the reborrow ensure fires in both modes.

extern crate cementite;

use simple_slab::Slab;

fn main() {
    let mut slab: Slab<u64> = Slab::with_capacity(2); // libc::malloc of 16 bytes
    slab.insert(0x1111_1111_1111_1111);
    slab.insert(0x2222_2222_2222_2222);
    eprintln!("LEN={}", slab.len());
    // index() does no bounds check: `&*(mem.offset(8))` reads 48 bytes past the
    // two-element (16-byte) buffer.
    let x = slab[8];
    println!("NO_ABORT x={x:#x}");
    std::hint::black_box(x);
}
