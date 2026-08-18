//! RUSTSEC-2025-0109: binary_vec_io 0.1.12 out-of-bounds read. The safe
//! `binary_write_from_ref<T>(f: &mut File, p: &T, n: usize)` does
//!
//! ```ignore
//! let raw = p as *const T as *const u8;
//! let sli: &[u8] = std::slice::from_raw_parts(raw, n * std::mem::size_of::<T>());
//! f.write_all(sli)?;
//! ```
//!
//! It accepts a *single* `&T` but builds a slice of `n * size_of::<T>()` bytes.
//! With `n > 1` the slice reaches `(n - 1) * size_of::<T>()` bytes past the
//! one-element allocation, and `write_all` reads out of bounds.
//!
//! The slice-constructor extent check vets `[p, p + n * size_of::<T>())` against
//! the derivation root (the heap-boxed `T`) at the `from_raw_parts` mint — which
//! runs *before* `write_all` — so it aborts `OutOfBounds` in **both** modes,
//! naming the owning allocation (I10), before any out-of-bounds byte is read.
//! The `T` is `Box`-allocated so FecAlloc tracks it.

extern crate cementite;

use std::hint::black_box;

use binary_vec_io::binary_write_from_ref;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

fn main() {
    // A single heap-allocated u64 (8 bytes), tracked by FecAlloc.
    let boxed: Box<u64> = Box::new(0xDEAD_BEEF_0BAD_F00D);
    let p: &u64 = &boxed;
    eprintln!("BASE={p:p}");

    // /dev/null: the write never runs — the check aborts at from_raw_parts.
    let mut sink = std::fs::File::create("/dev/null").expect("open /dev/null");

    // n = 100: binary_write_from_ref builds from_raw_parts(p, 100 * 8) — an
    // 800-byte slice over the 8-byte u64. The slice-ctor extent check aborts.
    let r = binary_write_from_ref(&mut sink, p, 100);
    println!("NO_ABORT r={r:?}");
    black_box(r.is_ok());
}
