//@ flags: -Cpanic=abort
//! Memory: allocas, loads and stores with alignment, getelementptr on
//! structs, arrays and slices, aggregate returns, and the noalias and
//! dereferenceable parameter attributes rustc puts on references.
#![no_std]

#[repr(C)]
pub struct Header {
    pub tag: u8,
    pub length: u32,
    pub data: *const u8,
}

#[repr(C)]
pub struct Nested {
    pub head: Header,
    pub tail: [u16; 3],
}

#[unsafe(no_mangle)]
pub fn read_field(h: &Header) -> u32 {
    h.length
}

#[unsafe(no_mangle)]
pub fn write_field(h: &mut Header, value: u32) {
    h.length = value;
}

#[unsafe(no_mangle)]
pub fn nested_field(n: &Nested) -> u16 {
    n.tail[1]
}

#[unsafe(no_mangle)]
pub fn build(tag: u8, length: u32) -> Header {
    Header {
        tag,
        length,
        data: core::ptr::null(),
    }
}

#[unsafe(no_mangle)]
pub fn index_slice(xs: &[u32], i: usize) -> u32 {
    xs[i]
}

#[unsafe(no_mangle)]
pub fn slice_length(xs: &[u32]) -> usize {
    xs.len()
}

#[unsafe(no_mangle)]
pub fn sum_array(xs: &[u64; 8]) -> u64 {
    let mut total = 0u64;
    let mut i = 0;
    while i < 8 {
        total = total.wrapping_add(xs[i]);
        i += 1;
    }
    total
}

#[unsafe(no_mangle)]
pub unsafe fn raw_copy(dst: *mut u8, src: *const u8, n: usize) {
    // SAFETY: the caller promises the two ranges are valid and disjoint. This
    // seed exists to make rustc emit llvm.memcpy, not to be called.
    unsafe { core::ptr::copy_nonoverlapping(src, dst, n) }
}

#[unsafe(no_mangle)]
pub unsafe fn volatile_touch(p: *mut u32) -> u32 {
    // SAFETY: the caller promises `p` is a valid, aligned u32. This seed
    // exists to make rustc emit volatile loads and stores.
    unsafe {
        let v = core::ptr::read_volatile(p);
        core::ptr::write_volatile(p, v.wrapping_add(1));
        v
    }
}

#[unsafe(no_mangle)]
pub fn tuple_return(a: u32, b: u64) -> (u32, u64) {
    (a, b)
}
