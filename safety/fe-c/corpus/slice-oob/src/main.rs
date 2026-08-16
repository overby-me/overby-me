//! Slice-index out-of-bounds through a `from_raw_parts` slice whose length lies
//! about the buffer (the case-mode slice-reborrow gap). A `&[u64]` is built over
//! a four-element (32-byte) buffer but claims 1000 elements. `s[500]` passes the
//! slice's own bounds check (500 < 1000) yet reads element 500 — far past the
//! real allocation.
//!
//! The index reborrow `let r: &u64 = &s[i]` mints a safe reference off the end.
//! In MIR (debug, un-inlined) this is `_r = &(*_s)[_i]` — a `Deref` of the slice
//! reference followed by an `Index`. `through` mode checks the later safe deref
//! `*r` and aborts; `case` elides that deref, so the *reborrow* is the only
//! checkpoint. The point-1 ensure must resolve the slice's derivation root (the
//! `as_ptr()` buffer, per I10 — never the faulting element address) and bounds-
//! check the indexed element's extent, so `case` aborts too.
//!
//! Both modes abort `OutOfBounds` resolved at the derivation root.

extern crate cementite;

use std::hint::black_box;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

/// Held reborrow `let r = &s[i]; *r` — the reference outlives the index
/// expression, so `case`'s elision of `*r` leaves the reborrow as the only
/// spatial checkpoint.
fn slice_reborrow() {
    let buf: Vec<u64> = black_box(vec![0x5555_5555_5555_5555; 4]); // 32-byte buffer
    let base = buf.as_ptr();
    eprintln!("BASE={base:p}");
    // A slice that lies: 1000 elements over a 4-element buffer.
    let s: &[u64] = unsafe { std::slice::from_raw_parts(base, 1000) };
    // Index 500 is in bounds of the *claimed* length but past the real buffer.
    let r: &u64 = &s[500];
    let x = *r;
    println!("NO_ABORT x={x:#x}");
    black_box(x);
}

fn main() {
    slice_reborrow();
}
