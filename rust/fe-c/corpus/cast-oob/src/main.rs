//! Raw->safe cast out-of-bounds (point 1 / trace §3.1 `ensure`). A raw pointer
//! whose referent is off the end of an allocation is cast to a safe reference.
//! The reference is spatially out of bounds, but the borrow checker cannot see
//! it — the pointer arithmetic (or a too-wide reinterpret) launders provenance.
//!
//! Two scenarios, selected by argv, both aborting `OutOfBounds` resolved at the
//! derivation root (the owning `Vec` buffer) in **both** modes:
//!
//! - default: a **whole-object** `&*bad` reborrow of a pointer six elements past
//!   a four-element buffer.
//! - `field`: a **field reborrow** `&(*p).b` where the buffer is reinterpreted
//!   as a wider `#[repr(C)]` struct, so field `b` lies past the end. This is the
//!   case the point-1 ensure must cover: `case` elides the reference's later
//!   dereferences, so an unvetted field reborrow off the end would be missed.
//!
//! The ensure fires in both modes: `case` would elide the reference's later
//! dereferences, so the cast is the only checkpoint; and `through`'s deref check
//! resolves the *faulting* address, which is off the end (unknown provenance),
//! so it too relies on the cast ensure to catch an out-of-bounds mint.

extern crate cementite;

use std::hint::black_box;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

/// 16 bytes — twice the width of the one-element `u64` buffer it reinterprets.
#[repr(C)]
struct Pair {
    a: u64,
    b: u64,
}

/// Whole-object reborrow `&*bad` of an out-of-bounds raw pointer.
fn whole_object() {
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

/// Field reborrow `&(*p).b` of a field that lies off the end of the buffer.
fn field_reborrow() {
    let v: Vec<u64> = black_box(vec![0x2222_2222_2222_2222; 1]); // 8-byte buffer
    let base = v.as_ptr();
    eprintln!("BASE={base:p}");
    // Reinterpret the 8-byte buffer as a 16-byte Pair: field `b` (offset 8) is
    // off the end. The pointer to the object is in bounds, so the borrow
    // checker and a whole-object cast ensure would both pass — only a
    // field-granular ensure sees that `&(*p).b` reads [base+8, base+16).
    let p = base as *const Pair;
    // Field reborrow of the out-of-bounds field: the cast ensure aborts here.
    let r: &u64 = unsafe { &(*p).b };
    let x = *r;
    println!("NO_ABORT x={x:#x}");
    black_box(x);
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("field") {
        field_reborrow();
    } else {
        whole_object();
    }
}
