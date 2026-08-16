extern crate cementite;

use std::hint::black_box;

use partial_sort::PartialSort;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

fn main() {
    let mut v: Vec<u64> = black_box(vec![9, 7, 5, 3, 1, 8, 6, 4, 2, 0]); // 10 elements
    eprintln!("BASE={:p} len={}", v.as_ptr(), v.len());
    // last = 40 >> len = 10. With debug-assertions off, partial_sort's
    // `debug_assert!(last <= v.len())` is elided, so its get_unchecked reads
    // walk past the 10-element buffer (read-only OOB, per the advisory).
    v.partial_sort(40, |a, b| a.cmp(b));
    println!("NO_ABORT v0={}", v[0]);
    black_box(&v);
}
