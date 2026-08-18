//! RUSTSEC-2021-0003 reproducer (see `docs/traces/rustsec-2021-0003.md`).
//!
//! `SmallVec::insert_many` reserved using the iterator's `size_hint` *lower*
//! bound, then wrote every item yielded. A `filter` iterator whose lower
//! bound is 0 overflows the 1-byte spilled buffer, writing past its end into
//! the neighbouring `String`.
//!
//! Built under the Fe-C instrument driver with `FecAlloc` installed, the
//! overflowing `ptr::write` traps. The report must name the **SmallVec**
//! allocation (the derivation root of the write, `as_mut_ptr()`), *not* the
//! `String` the faulting address lands in — resolving from the faulting
//! address would find the live `String` and wrongly pass (I10 / F10).

extern crate cementite;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

use smallvec::SmallVec;

fn main() {
    let mut v: SmallVec<[u8; 0]> = SmallVec::new();
    v.push(123); // spill to a 1-byte heap buffer

    let s = String::from("neighbouring allocation");

    // The test parses these to check the report names V, not S.
    eprintln!("SMALLVEC_BASE={:p}", v.as_ptr());
    eprintln!("STRING_BASE={:p}", s.as_ptr());

    let iter = (0u8..=255).filter(|n| n % 2 == 0);
    assert_eq!(iter.size_hint().0, 0, "lower bound lies");

    v.insert_many(0, iter); // reserves 0, writes 128 -> overflow -> trap

    // Only reached if the checker failed to trap (a bug).
    println!("NO_ABORT s={s}");
}
