//! Root-resolve cost: the price of one table lookup at a derivation root.
//!
//! Reference point from the design phase: ~2 ns for a dense-table lookup,
//! warm cache, single thread (a floor, not a promise; see
//! `docs/cementite-api.md`). Per I10 this is the *cold* path, paid at
//! derivation roots and fallbacks; the per-access hot path is a register
//! compare against an already-resolved capability.

use std::hint::black_box;

use cementite::CapFlags;
use cementite::table::{lookup, register};
use criterion::{Criterion, criterion_group, criterion_main};

fn bench_root_resolve(c: &mut Criterion) {
    // A 1 MiB allocation resolved through a spill slot (interior pointer).
    let spanning = 0x2000_0000_0100usize;
    register(spanning, 1 << 20, CapFlags::EMPTY, 0);

    // A page holding four small allocations (inline-entry scan).
    let inline_page = 0x2100_0000_0000usize;
    for i in 0..4 {
        register(inline_page + i * 0x100, 0x80, CapFlags::EMPTY, 0);
    }

    // A dense page: 40 allocation starts, so the scan walks the overflow
    // chain. This is the documented worst case, not the common one.
    let dense_page = 0x2200_0000_0000usize;
    for i in 0..40 {
        register(dense_page + i * 0x40, 0x20, CapFlags::EMPTY, 0);
    }

    let mut group = c.benchmark_group("root_resolve");
    group.bench_function("interior_of_spanning_alloc", |b| {
        b.iter(|| lookup(black_box(spanning + 0x8_0000)))
    });
    group.bench_function("inline_page_hit", |b| {
        b.iter(|| lookup(black_box(inline_page + 3 * 0x100 + 5)))
    });
    group.bench_function("overflow_page_hit", |b| {
        b.iter(|| lookup(black_box(dense_page + 39 * 0x40 + 5)))
    });
    group.bench_function("miss_mapped_page", |b| {
        b.iter(|| lookup(black_box(inline_page + 0xf00)))
    });
    group.bench_function("miss_unmapped_page", |b| {
        b.iter(|| lookup(black_box(0x2300_0000_0000usize)))
    });
    group.finish();
}

criterion_group!(benches, bench_root_resolve);
criterion_main!(benches);
