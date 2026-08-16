//! Heap-escape stack use-after-scope reproducer (Task B5, I9). A raw pointer
//! to a stack local is captured by-move into a boxed closure stored past the
//! frame, then dereferenced when the closure is invoked later. This is the
//! RUSTSEC-2021-0128 closure shape (a closure capturing a stack borrow, boxed
//! and stored by SQLite, invoked later) — here with a raw-pointer capture so
//! the closure's read is a checked raw dereference (v0's instrumentation
//! point 0).
//!
//! Under the Fe-C instrument driver, the escape analysis sees the raw stack
//! pointer captured into a closure aggregate (which is heap-boxed) and
//! registers the local's stack scope (I8); the frame's return poisons it; the
//! closure's later dereference resolves the dead scope and aborts, naming the
//! capture site as `escaped_at`.

extern crate cementite;

use std::hint::black_box;
use std::sync::atomic::{AtomicPtr, Ordering};

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

// The stored closure, kept past the registering frame: a pointer to a leaked
// box, the "stored somewhere global" role SQLite plays for a registered
// scalar function.
static CB: AtomicPtr<Box<dyn Fn() -> u64>> = AtomicPtr::new(std::ptr::null_mut());

#[inline(never)]
fn register() {
    let local: u64 = black_box(0xDEAD_BEEF_CAFE);
    eprintln!("STACK_LOCAL={:p}", &local);
    let p: *const u64 = &local;
    // Capture the raw stack pointer by move into a boxed closure kept past
    // this frame — the escape into the heap closure.
    let cb: Box<dyn Fn() -> u64> = Box::new(move || unsafe { *p });
    CB.store(Box::into_raw(Box::new(cb)), Ordering::SeqCst);
} // frame teardown -> local's stack scope poisoned

fn main() {
    register();
    // Invoke the stored closure: it dereferences the now-dangling pointer.
    let cb = CB.load(Ordering::SeqCst);
    let v = unsafe { (*cb)() };
    println!("NO_ABORT v={v:#x}");
}
