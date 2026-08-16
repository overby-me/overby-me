//! `through`-mode safe-reference use-after-scope (Task C1). A closure captures
//! a **safe** reference (`&u64`) to a stack local, is boxed and kept past the
//! frame, then reads the local through that reference when invoked.
//!
//! The read (`*r`) is a *safe* dereference, so `case` mode **elides** it
//! (vetted once at the borrow) and would not catch the use-after-scope — only
//! `through` mode **checks** it. This is the RUSTSEC-2021-0128 §3.2 shape: the
//! real rusqlite closure reads its captured borrow through a safe reference,
//! which `through`'s safe-deref checking is what catches.
//!
//! Storing a `&local` in a `'static` closure would normally be rejected by the
//! borrow checker; a lifetime-laundering `transmute` stands in for the
//! too-relaxed lifetime bound the rusqlite API had (the actual bug).

extern crate cementite;

use std::hint::black_box;
use std::sync::atomic::{AtomicPtr, Ordering};

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

static CB: AtomicPtr<Box<dyn Fn() -> u64>> = AtomicPtr::new(std::ptr::null_mut());

#[inline(never)]
fn register() {
    let local: u64 = black_box(0xDEAD_BEEF_CAFE);
    eprintln!("STACK_LOCAL={:p}", &local);
    // The too-relaxed lifetime bound: launder `&local` to `&'static` so it can
    // be stored past the frame, exactly what the rusqlite API wrongly allowed.
    let r: &'static u64 = unsafe { std::mem::transmute::<&u64, &'static u64>(&local) };
    // Capture the SAFE reference by move into a boxed closure kept past frame.
    let cb: Box<dyn Fn() -> u64> = Box::new(move || *r);
    CB.store(Box::into_raw(Box::new(cb)), Ordering::SeqCst);
} // frame teardown -> local's stack scope poisoned

fn main() {
    register();
    // Invoke the stored closure: it reads the now-dead local through `*r`,
    // a safe dereference that only `through` mode checks.
    let cb = CB.load(Ordering::SeqCst);
    let v = unsafe { (*cb)() };
    println!("NO_ABORT v={v:#x}");
}
