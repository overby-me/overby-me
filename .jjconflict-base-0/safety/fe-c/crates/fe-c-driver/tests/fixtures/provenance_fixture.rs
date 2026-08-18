//! Provenance dataflow fixture (Task B1). Reproduces the shape of
//! `smallvec::insert_many` — the derivation root `as_mut_ptr()`, propagated
//! through `.add(..)` pointer arithmetic, reaching an overflowing write —
//! in both write forms real unsafe code uses: a direct `*p = v` and a
//! `ptr::write(p, v)` call. `tests/provenance.rs` runs the driver with
//! `FEC_PROV_FN=insert_many_like` and asserts the write is rooted at
//! `as_mut_ptr` (the I10 property the checker depends on).

/// Direct-deref write form: `*p = v`.
pub fn insert_many_like_direct(v: &mut Vec<u8>, index: usize, src: &[u8]) {
    let start = v.as_mut_ptr(); // derivation root: as_mut_ptr
    for (i, &b) in src.iter().enumerate() {
        let cur = unsafe { start.add(index + i) }; // propagate through .add
        unsafe { *cur = b }; // rooted write
    }
}

/// Intrinsic-call write form: `ptr::write(p, v)`, exactly as
/// `smallvec::insert_many` writes each element.
pub fn insert_many_like_write(v: &mut Vec<u8>, index: usize, src: &[u8]) {
    let start = v.as_mut_ptr(); // derivation root: as_mut_ptr
    let ptr = unsafe { start.add(index) }; // propagate
    for (i, &b) in src.iter().enumerate() {
        let cur = unsafe { ptr.add(i) }; // propagate again
        unsafe { std::ptr::write(cur, b) }; // rooted write via intrinsic
    }
}

fn main() {
    let mut a = vec![0u8; 4];
    insert_many_like_direct(&mut a, 0, &[1, 2, 3, 4]);
    let mut b = vec![0u8; 4];
    insert_many_like_write(&mut b, 0, &[5, 6, 7, 8]);
    println!("{a:?} {b:?}");
}
