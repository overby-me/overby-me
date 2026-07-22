//! Stack use-after-scope-exit reproducer (Task B5, I8) at **lexical
//! granularity**. A stack local lives in an inner block; its address escapes
//! the block (laundered through a `static`, exactly as the RUSTSEC-2021-0128
//! rusqlite closure launders a stack borrow out across the FFI boundary).
//! The block ends — so the local's storage is dead — but `main`'s frame is
//! still running, and the escaped pointer is dereferenced *later in the same
//! frame*.
//!
//! Frame-granularity scope hooks (poison only at `Return`) would not fire
//! until `main` ends, and so would miss this. The lexical hooks poison the
//! region at the local's drop glue (its lexical death point, which survives
//! MIR optimization where `StorageDead` does not), so the stale dereference
//! resolves it as a dead stack scope and aborts — the report names that
//! scope, not whatever now occupies the address.

extern crate cementite;

use std::sync::atomic::{AtomicUsize, Ordering};

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

static STASH: AtomicUsize = AtomicUsize::new(0);

#[inline(never)]
fn unrelated_work() -> u64 {
    // Keeps main's frame busy after the inner block, so the dereference is
    // unmistakably in a live frame whose inner scope has already ended.
    std::hint::black_box(0)
}

fn main() {
    let dangling: *const usize;
    {
        // A `Drop` type (like the rusqlite `String`): its drop glue forces a
        // `StorageDead(local)` at the inner block's end even though the
        // address escaped, giving the lexical poison point.
        let local = String::from(std::hint::black_box("dead at block end"));
        STASH.store(&local as *const String as usize, Ordering::SeqCst);
        eprintln!("STACK_LOCAL={:p}", &local);
        dangling = STASH.load(Ordering::SeqCst) as *const usize;
    } // inner block ends: local dropped, StorageDead(local) -> scope poisoned

    unrelated_work();

    // Dereference into the dead inner-block scope, still inside main's frame.
    let v = unsafe { *dangling };
    println!("NO_ABORT v={v:#x}");
}
