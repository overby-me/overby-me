//! Stack use-after-scope-exit reproducer (Task B5, I8).
//!
//! `stash_stack_ptr` takes the address of a stack local and launders it out
//! of the frame through a `static` — the same escape the RUSTSEC-2021-0128
//! `rusqlite` closure performs across the FFI boundary (a boxed closure
//! capturing a stack borrow, stored by SQLite and invoked later). After the
//! frame returns, the stack region is torn down; dereferencing the escaped
//! pointer must trap.
//!
//! Under the Fe-C instrument driver with `FecAlloc` installed, the scope
//! hooks register the local's stack region at entry and poison it at frame
//! teardown, so the stale dereference resolves it as a dead stack scope and
//! aborts — the report names that scope, not whatever now occupies the
//! address.

extern crate cementite;

use std::sync::atomic::{AtomicUsize, Ordering};

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

static STASH: AtomicUsize = AtomicUsize::new(0);

#[inline(never)]
fn stash_stack_ptr() {
    // black_box stops const-promotion, keeping `local` a real stack slot.
    let local: u64 = std::hint::black_box(0xDEAD_BEEF_CAFE);
    STASH.store(&local as *const u64 as usize, Ordering::SeqCst);
    eprintln!("STACK_LOCAL={:p}", &local);
} // frame teardown -> scope poisoned

fn main() {
    stash_stack_ptr();
    let dangling = STASH.load(Ordering::SeqCst) as *const u64;
    // Dereference into the dead stack frame.
    let v = unsafe { *dangling };
    println!("NO_ABORT v={v:#x}");
}
