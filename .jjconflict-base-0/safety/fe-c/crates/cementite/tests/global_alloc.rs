//! Installs `FecAlloc` as the real `#[global_allocator]` for this test
//! binary and exercises it under heavy, realistic allocation traffic. This
//! is the end-to-end proof that the quarantining allocator can back a whole
//! program without recursing or deadlocking — every `Vec`/`Box`/`String`
//! below routes through registration, deregistration and quarantine.
//!
//! Not run under Miri: the metadata backend uses real `mmap` off the global
//! allocator path, which Miri cannot execute. Miri coverage of the same
//! unsafe lives in the crate's unit tests via direct `GlobalAlloc` calls.
//!
//! Excluded when the `interpose` feature is on: that build's `#[no_mangle]`
//! malloc override plus a `#[global_allocator]` would register Rust
//! allocations twice. Interposition is exercised by `tests/interpose_c.rs`.
#![cfg(all(not(miri), not(feature = "interpose")))]

use std::collections::HashMap;

use cementite::{FecAlloc, quarantine_bytes, set_quarantine_budget};

#[global_allocator]
static A: FecAlloc = FecAlloc;

#[test]
fn heavy_traffic_stays_bounded_and_correct() {
    set_quarantine_budget(2 << 20);

    // Vectors of varied sizes, grown and dropped.
    let mut keep: Vec<Vec<u64>> = Vec::new();
    for round in 0..2_000 {
        let mut v = Vec::with_capacity(round % 512 + 1);
        for i in 0..(round % 512 + 1) {
            v.push((round * i) as u64);
        }
        // Checksum forces the writes to be observed, so a mis-sized
        // registration that clipped the buffer would corrupt or trap.
        let sum: u64 = v.iter().copied().sum();
        assert_eq!(
            sum,
            (0..(round as u64 % 512 + 1))
                .map(|i| round as u64 * i)
                .sum()
        );
        if round % 3 == 0 {
            keep.push(v);
        }
        if keep.len() > 64 {
            keep.remove(0);
        }
    }

    // Heap-heavy container churn: strings into a map and back out.
    let mut map: HashMap<String, String> = HashMap::new();
    for i in 0..5_000 {
        map.insert(
            format!("key-{i}"),
            format!("value-{i}-{}", "x".repeat(i % 40)),
        );
        if i % 7 == 0 && i > 0 {
            map.remove(&format!("key-{}", i - 1));
        }
    }
    assert!(map.contains_key("key-4999"));
    assert_eq!(map["key-10"], "value-10-xxxxxxxxxx");

    drop(keep);
    drop(map);

    // The budget is a soft cap checked after each free; the steady state
    // must be within a small multiple of it, never unbounded growth.
    assert!(
        quarantine_bytes() <= 4 << 20,
        "quarantine grew unbounded: {} bytes",
        quarantine_bytes()
    );
}
