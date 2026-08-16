//! Write-intrinsic extent overrun (point 0, write path). A
//! `ptr::copy_nonoverlapping` copies four `u64`s (32 bytes) into a one-element
//! (8-byte) buffer. The destination pointer is the buffer's base — in bounds —
//! but the write covers `[base, base+32)`, overrunning the allocation by 24
//! bytes. A single-address destination check sees only that `base` is in
//! bounds and passes; the extent check verifies `[dst, dst + count*size_of)`
//! and aborts OutOfBounds, resolved at the owning buffer (I10). Instrumented in
//! both modes (write checks are not mode-gated).

extern crate cementite;

use std::hint::black_box;
use std::ptr;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

fn main() {
    let mut dst: Vec<u64> = black_box(vec![0u64; 1]); // 8-byte buffer
    let src: [u64; 4] = black_box([0xAAAA_AAAA_AAAA_AAAA; 4]); // 32 bytes
    let p = dst.as_mut_ptr();
    eprintln!("DST={p:p}");
    // Destination base is in bounds, but copying 4 u64s writes [base, base+32),
    // overrunning the 8-byte buffer. The extent check aborts here.
    unsafe { ptr::copy_nonoverlapping(src.as_ptr(), p, 4) };
    println!("NO_ABORT dst0={:#x}", dst[0]);
    black_box(dst);
}
