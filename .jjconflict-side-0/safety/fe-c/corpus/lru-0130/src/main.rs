//! RUSTSEC-2021-0130 reproducer against real `lru` 0.6.6 (Task C2 / B-phase).
//!
//! `LruCache::iter()` yields references borrowed out of the doubly-linked list
//! nodes, with a lifetime too detached from the cache borrow (the bug). So the
//! loop can `pop(key)` — dropping the `Box<LruEntry>` and freeing the node —
//! and then read `value`, a reference into the now-freed node. See
//! `docs/traces/rustsec-2021-0130.md`. The borrow checker misses it because
//! the aliasing launders through a `*mut LruEntry`.
//!
//! Under `FEC_MODE=through` the read `*value` is a checked dereference: it
//! resolves the freed node allocation and aborts `UseAfterFree`. Under
//! case-like mode the safe dereference is elided (case needs the §3.4
//! dealloc-reachable re-check, C2, to catch it — not yet built), so it runs on
//! into UB; that path is not asserted here.

extern crate cementite;

use lru::LruCache;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

fn main() {
    let mut cache = LruCache::new(100);
    cache.put(1u64, 0xAAAA_0001u64);
    cache.put(2u64, 0xAAAA_0002u64);
    cache.put(3u64, 0xAAAA_0003u64);

    for (key, value) in cache.iter() {
        // Frees the node that `value` points into (drops its Box).
        cache.pop(key);
        eprintln!("VALUE_PTR={value:p}");
        // Reads the freed node through the dangling reference.
        let v = *value;
        println!("NO_ABORT v={v:#x}");
        std::hint::black_box(v);
    }
}
