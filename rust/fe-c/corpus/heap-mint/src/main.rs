//! Heap use-after-free with **mint-site** naming (trace -0130 debuggability).
//!
//! A `Box<Pair>` is heap-allocated; a field reference `&(*p).b` is minted into
//! it (a raw->safe cast — point 1 `ensure`, which records where the reference
//! was born on the allocation); the `Box` is freed (poison keeps it
//! findable-as-dead in quarantine); and the value is read through the now
//! dangling reference. Both modes abort `UseAfterFree`.
//!
//! Because the mint is in *this* instrumented binary (unlike `lru-0130`, whose
//! reborrow is inside the uninstrumented crate), the report names `minted_at`
//! — the source line the dangling reference was born at, resolved on the freed
//! allocation. `case` additionally names `read_at`, since its dealloc-reachable
//! re-check is injected right at the dangling read. Together they are the
//! both-sites debuggability pair the -0130 trace calls for (the precise *free*
//! line still awaits the `nofree` callgraph).

extern crate cementite;

use std::hint::black_box;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

#[repr(C)]
struct Pair {
    a: u64,
    b: u64,
}

fn main() {
    let boxed = Box::new(Pair { a: 0xAAAA, b: 0xBBBB });
    let p = Box::into_raw(boxed);
    // Mint a field reference into the heap allocation. The cast ensure records
    // THIS source line on the allocation as its mint site.
    let r: &u64 = unsafe { &(*p).b };
    black_box(r as *const u64);
    // Free the box: poison keeps it findable-as-dead in quarantine (F2).
    drop(unsafe { Box::from_raw(p) });
    // Read through the dangling reference: aborts UseAfterFree, naming the mint.
    let v = *r;
    println!("NO_ABORT v={v:#x}");
    black_box(v);
}
