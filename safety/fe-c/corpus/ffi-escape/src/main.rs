//! Cross-FFI stack use-after-scope reproducer (Task B5, I9 / trace F6).
//!
//! A stack borrow escapes **out** to C through an `extern "C"` registration
//! call (`fec_register`), the registering frame returns, then C re-enters
//! Rust through a trampoline (`fec_invoke` -> `trampoline`) and dereferences
//! the now-dead stack local. This is the RUSTSEC-2021-0128 shape: a closure
//! capturing a stack borrow, handed to SQLite via `create_scalar_function`,
//! stored, and invoked later.
//!
//! Under the Fe-C instrument driver, the escape analysis sees `&local` passed
//! to a foreign call and registers its stack scope (I8); the frame's return
//! poisons it; the trampoline's dereference of the escaped pointer resolves
//! the dead scope and aborts. The C harness itself is never instrumented —
//! only the Rust boundary is checked (trace F8).

extern crate cementite;

use std::ffi::c_void;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

unsafe extern "C" {
    fn fec_register(cb: extern "C" fn(*mut c_void) -> i64, data: *mut c_void);
    fn fec_invoke() -> i64;
}

/// The Rust callback C re-enters through. Reads the escaped stack local.
extern "C" fn trampoline(data: *mut c_void) -> i64 {
    let p = data as *const u64;
    // Dereference into the (dead) stack scope the pointer escaped from.
    unsafe { *p as i64 }
}

#[inline(never)]
fn register_with_stack_borrow() {
    // black_box stops const-promotion, keeping `local` a real stack slot.
    let local: u64 = std::hint::black_box(0xDEAD_BEEF_CAFE);
    eprintln!("STACK_LOCAL={:p}", &local);
    // Outbound FFI: hand &local straight out to C. This is the escape (F6).
    unsafe {
        fec_register(trampoline, &local as *const u64 as *mut c_void);
    }
} // frame teardown -> local's stack scope poisoned

fn main() {
    register_with_stack_borrow();
    // C re-enters Rust with the now-dangling pointer.
    let r = unsafe { fec_invoke() };
    println!("NO_ABORT r={r:#x}");
}
