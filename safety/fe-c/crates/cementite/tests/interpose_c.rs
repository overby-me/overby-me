//! Exercises libc allocator interposition (Task A4). Built only with the
//! `interpose` feature, which links this crate's `#[no_mangle]` malloc
//! family so it overrides libc's for the whole test binary. Every call
//! below goes through the real C `malloc`/`free`/… symbols — the same ones
//! C code calls — so registration through them is exactly the foreign-code
//! path.
//!
//! Covers the Task A4 acceptance: allocations made through the libc symbols
//! (including libc-internal ones via `strdup`, and C code in
//! `tests/harness.c`) appear in the table with correct bounds.
#![cfg(feature = "interpose")]

use std::ffi::{CStr, c_char, c_int, c_void};

use cementite::{CapFlags, table};

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn calloc(nmemb: usize, size: usize) -> *mut c_void;
    fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn posix_memalign(memptr: *mut *mut c_void, align: usize, size: usize) -> c_int;
    fn strdup(s: *const c_char) -> *mut c_char;
}

#[test]
fn malloc_is_tracked_with_exact_bounds() {
    // SAFETY: standard libc calls; the block is freed at the end.
    unsafe {
        let p = malloc(200);
        assert!(!p.is_null());

        let cap = table::lookup(p as usize).expect("malloc tracked in table");
        assert_eq!(cap.base, p as usize, "base matches");
        assert_eq!(cap.len, 200, "length matches the request");
        assert!(
            cap.flags.contains(CapFlags::ESCAPED),
            "foreign alloc flagged"
        );
        assert!(table::is_live(cap.id));
        assert!(
            table::lookup(p as usize + 199).is_some(),
            "last byte in bounds"
        );
        assert!(
            table::lookup(p as usize + 200).is_none(),
            "one past the end out"
        );

        free(p);
        assert!(!table::is_live(cap.id), "liveness cleared on free (I7)");
        assert!(table::lookup(p as usize).is_none(), "unlinked on free");
    }
}

#[test]
fn calloc_is_tracked_with_total_bounds() {
    // SAFETY: standard libc calls.
    unsafe {
        let p = calloc(16, 8);
        assert!(!p.is_null());
        let cap = table::lookup(p as usize).expect("calloc tracked");
        assert_eq!(cap.len, 128, "nmemb*size registered");
        // calloc returns zeroed memory.
        assert_eq!(*(p as *const u8), 0);
        free(p);
        assert!(!table::is_live(cap.id));
    }
}

#[test]
fn realloc_retracks_the_new_block() {
    // SAFETY: standard libc calls.
    unsafe {
        let p = malloc(32);
        let id0 = table::lookup(p as usize).unwrap().id;

        // Grow well past the bin so libc is likely to move it; either way
        // the result must be tracked at its new size.
        let q = realloc(p, 4096);
        assert!(!q.is_null());
        let cap = table::lookup(q as usize).expect("realloc result tracked");
        assert_eq!(cap.len, 4096, "retracked at the new size");
        if !std::ptr::eq(q, p) {
            assert!(table::lookup(p as usize).is_none(), "moved: old unlinked");
        }
        assert_ne!(cap.id, id0, "a resized block gets a fresh identity");

        free(q);
        assert!(!table::is_live(cap.id));
    }
}

#[test]
fn posix_memalign_is_tracked_and_aligned() {
    // SAFETY: standard libc calls.
    unsafe {
        let mut p: *mut c_void = std::ptr::null_mut();
        let r = posix_memalign(&mut p, 256, 1000);
        assert_eq!(r, 0);
        assert_eq!(p as usize % 256, 0, "honoured the alignment");
        let cap = table::lookup(p as usize).expect("posix_memalign tracked");
        assert_eq!(cap.len, 1000);
        free(p);
        assert!(!table::is_live(cap.id));
    }
}

#[test]
fn strdup_internal_allocation_is_tracked() {
    // strdup is C code inside libc; its internal malloc routes through our
    // override, so the duplicated string must appear in the table — the
    // "libc-internal allocation" half of the acceptance.
    let src = c"fe-c interposes libc";
    // SAFETY: src is a valid NUL-terminated C string; result freed below.
    unsafe {
        let dup = strdup(src.as_ptr());
        assert!(!dup.is_null());
        let cap = table::lookup(dup as usize).expect("strdup allocation tracked");
        assert!(
            cap.len >= src.to_bytes_with_nul().len(),
            "covers the copied string ({} >= {})",
            cap.len,
            src.to_bytes_with_nul().len()
        );
        assert_eq!(
            CStr::from_ptr(dup).to_bytes(),
            src.to_bytes(),
            "content copied correctly"
        );
        free(dup as *mut c_void);
        assert!(!table::is_live(cap.id));
    }
}

// ---- compiled C harness ---------------------------------------------------

unsafe extern "C" {
    // Defined in tests/harness.c, compiled by build.rs under this feature.
    // Allocates `n` bytes with libc malloc, writes a marker, returns it.
    fn fec_harness_alloc(n: usize) -> *mut c_void;
    fn fec_harness_free(p: *mut c_void);
}

#[test]
fn c_harness_allocation_is_tracked() {
    // A genuinely C-compiled translation unit calling malloc: proves the
    // override applies across the FFI/link boundary, not just to Rust-side
    // extern calls. Doubles as the first mixed-language build smoke.
    // SAFETY: harness returns a live malloc'd block of n bytes.
    unsafe {
        let p = fec_harness_alloc(512);
        assert!(!p.is_null());
        let cap = table::lookup(p as usize).expect("C-harness malloc tracked");
        assert_eq!(cap.len, 512, "C caller's request size registered");
        assert!(cap.flags.contains(CapFlags::ESCAPED));
        // The harness wrote a 0xAB marker to the first byte.
        assert_eq!(*(p as *const u8), 0xAB);
        fec_harness_free(p);
        assert!(!table::is_live(cap.id));
    }
}
