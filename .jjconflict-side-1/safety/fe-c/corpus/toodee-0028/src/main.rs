//! RUSTSEC-2021-0028 / CVE-2021-28028 reproducer against real `toodee` 0.2.0 —
//! an out-of-bounds write via `insert_row`.
//!
//! `insert_row` reserves space based on the iterator's `ExactSizeIterator::len()`
//! but writes the *actual* items it yields (the bug). A `Liar` iterator claims a
//! length of 2 but yields 100, so `insert_row` reserves room for 2 and then
//! `ptr::write`s 100 elements, overrunning the backing `Vec` buffer.
//!
//! The buffer is a global-allocator (`FecAlloc`) allocation; the overrunning
//! `ptr::write` is caught by the write-call extent check, resolved from the
//! `as_mut_ptr` derivation root (I10), aborting `OutOfBounds`.

extern crate cementite;

use toodee::TooDee;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

/// Claims `len() == 2` but yields 100 items — the length lie that drives the
/// out-of-bounds write.
struct Liar {
    yielded: usize,
}

impl Iterator for Liar {
    type Item = u64;
    fn next(&mut self) -> Option<u64> {
        if self.yielded < 100 {
            self.yielded += 1;
            Some(0xDEAD_BEEF_DEAD_BEEF)
        } else {
            None
        }
    }
}

impl ExactSizeIterator for Liar {
    fn len(&self) -> usize {
        2 // the lie: reserve space for 2, but 100 are written
    }
}

fn main() {
    let mut grid: TooDee<u64> = TooDee::default();
    // insert_row reserves room for len()==2 but writes 100 -> OOB write.
    grid.insert_row(0, Liar { yielded: 0 });
    println!("NO_ABORT");
    std::hint::black_box(grid);
}
