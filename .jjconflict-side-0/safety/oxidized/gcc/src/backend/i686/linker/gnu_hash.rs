//! GNU hash table building for ELF32.
//!
//! Builds the `.gnu.hash` section for dynamically-linked i686 executables.
//! Uses 32-bit bloom filter words (ELF32 word size) and the GNU hash algorithm.

use crate::backend::linker_common;

/// Build a .gnu.hash section for ELF32.
///
/// `hashed_names`: names of symbols that go into the hash (at indices >= symoffset)
/// `symoffset`: first hashed symbol's index in .dynsym
///
/// Returns `(hash_data, sorted_indices)` where `sorted_indices` maps new position
/// to original index in `hashed_names`, so the caller can reorder dynsym entries.
pub(super) fn build_gnu_hash_32(hashed_names: &[String], symoffset: u32) -> (Vec<u8>, Vec<usize>) {
    let num_hashed = hashed_names.len();
    let nbuckets = if num_hashed == 0 {
        1
    } else {
        num_hashed.next_power_of_two().max(1)
    } as u32;
    let bloom_size: u32 = 1;
    let bloom_shift: u32 = 5;

    // Compute hashes
    let orig_hashes: Vec<u32> = hashed_names
        .iter()
        .map(|name| linker_common::gnu_hash(name.as_bytes()))
        .collect();

    // Sort by bucket for proper chain grouping
    let mut indices: Vec<usize> = (0..num_hashed).collect();
    indices.sort_by_key(|&i| orig_hashes[i] % nbuckets);
    let sym_hashes: Vec<u32> = indices.iter().map(|&i| orig_hashes[i]).collect();

    // Build bloom filter (single 32-bit word for ELF32)
    let mut bloom_word: u32 = 0;
    for &h in &sym_hashes {
        bloom_word |= 1u32 << (h % 32);
        bloom_word |= 1u32 << ((h >> bloom_shift) % 32);
    }

    // Build buckets and chains
    let mut buckets = vec![0u32; nbuckets as usize];
    let mut chains = vec![0u32; num_hashed];
    for (i, &h) in sym_hashes.iter().enumerate() {
        let bucket = (h % nbuckets) as usize;
        if buckets[bucket] == 0 {
            buckets[bucket] = symoffset + i as u32;
        }
        chains[i] = h & !1;
    }

    // Mark the last symbol in each bucket chain with bit 0 set
    for bucket_idx in 0..nbuckets as usize {
        if buckets[bucket_idx] == 0 {
            continue;
        }
        let mut last_in_bucket = 0;
        for (i, &h) in sym_hashes.iter().enumerate() {
            if (h % nbuckets) as usize == bucket_idx {
                last_in_bucket = i;
            }
        }
        chains[last_in_bucket] |= 1;
    }

    // Serialize
    let mut data = Vec::new();
    data.extend_from_slice(&nbuckets.to_le_bytes());
    data.extend_from_slice(&symoffset.to_le_bytes());
    data.extend_from_slice(&bloom_size.to_le_bytes());
    data.extend_from_slice(&bloom_shift.to_le_bytes());
    data.extend_from_slice(&bloom_word.to_le_bytes());
    for &b in &buckets {
        data.extend_from_slice(&b.to_le_bytes());
    }
    for &c in &chains {
        data.extend_from_slice(&c.to_le_bytes());
    }

    (data, indices)
}
