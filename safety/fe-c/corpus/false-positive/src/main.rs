//! False-positive workload (Task B4): exercises hashbrown's raw-pointer
//! heavy SwissTable across grow, probe, tombstone and iteration, with
//! `FecAlloc` installed so the instrumented checks resolve real
//! capabilities from the allocation table. Legitimate in-bounds unsafe must
//! never trap — the whole run must complete and print `FP_OK`.

extern crate cementite;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

use hashbrown::HashMap;

fn main() {
    // Repeated grow/insert forces reallocation and rehashing (raw-pointer
    // bucket moves) many times over.
    let mut m: HashMap<u64, u64> = HashMap::new();
    for round in 0..4 {
        for i in 0..20_000u64 {
            m.insert(i, i.wrapping_mul(2654435761).wrapping_add(round));
        }
        // Probe every key (raw-pointer bucket reads).
        let mut acc = 0u64;
        for i in 0..20_000u64 {
            acc = acc.wrapping_add(*m.get(&i).unwrap());
        }
        std::hint::black_box(acc);
        // Tombstone half, then reinsert (probe-sequence stress).
        for i in (0..20_000u64).step_by(2) {
            m.remove(&i);
        }
        assert_eq!(m.len(), 10_000);
    }

    // Iterate (raw-pointer scan over the control bytes + buckets).
    let sum: u64 = m.values().copied().sum();
    let count = m.iter().count();

    // A second map of a differently-sized value to vary layouts.
    let mut s: HashMap<String, Vec<u8>> = HashMap::new();
    for i in 0..5_000 {
        s.insert(format!("key-{i}"), vec![i as u8; i % 37]);
    }
    s.retain(|_, v| v.len() % 2 == 0);

    println!("FP_OK sum={sum} count={count} smap={}", s.len());
}
