//! Hand-audited census fixture (Task A5 spot-check). Every pointer
//! operation the census must see is present and counted in the comment
//! beside it; `tests/census.rs` asserts the census meets these minimums.
//! Minimums, not exact counts: MIR lowering inserts extra temporaries and
//! the standard library adds more, so the census can only ever see *more*
//! than the source shows, never fewer. Under-counting would break I1.

// An FFI edge: an extern "C" declaration plus a call site.  [ffi_calls >= 1]
unsafe extern "C" {
    fn abs(input: i32) -> i32;
}

/// One raw-pointer local, one raw dereference read.
pub fn raw_read(p: *const u8) -> u8 {
    // *p is a raw dereference.  [raw_ptr_locals >= 1, raw_derefs >= 1]
    unsafe { *p }
}

/// One raw-pointer write.
pub fn raw_write(p: *mut u8, v: u8) {
    // *p = v is a raw dereference (write).  [raw_derefs >= 1]
    unsafe { *p = v }
}

/// A raw->safe reborrow: &*p turns a raw pointer into a reference.
pub fn reborrow(p: *const u32) -> u32 {
    // &*p is the raw->safe boundary.  [raw_to_safe_casts >= 1]
    let r: &u32 = unsafe { &*p };
    *r
}

/// A pointer-to-integer cast (provenance-losing).
pub fn to_int(p: *const u8) -> usize {
    // `p as usize` exposes the address.  [ptr_int_casts >= 1]
    p as usize
}

/// Calls across the FFI edge.
pub fn call_ffi(x: i32) -> i32 {
    // abs is extern "C".  [ffi_calls >= 1]
    unsafe { abs(x) }
}

fn main() {
    let x = 7u8;
    println!("{}", raw_read(&x));
    let mut y = 0u8;
    raw_write(&mut y, 3);
    let z = 9u32;
    println!("{}", reborrow(&z));
    println!("{}", to_int(&x));
    println!("{}", call_ffi(-5));
}
