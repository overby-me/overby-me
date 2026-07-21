//! Id-indexed liveness bitmap.
//!
//! With never-recycled 48-bit ids each id is allocated once and freed at
//! most once, so the "generation" is one bit (`docs/through-mode-coherence.md`,
//! the epoch-collapses section). Temporal safety is `is_live(id)`.
//!
//! I7 gives the ordering contract: the allocator calls [`clear`] *before*
//! releasing the underlying memory. The bitmap itself only promises the
//! atomic orderings that make that call sequence sound across threads
//! (Release on clear, Acquire on load).
//!
//! Layout: ids are dense (sequential counter), so the bitmap is a two-level
//! directory of lazily mapped chunks:
//! id bits 48 = directory 13 | subdirectory 12 | chunk 23 (1 MiB of bits).

use core::sync::atomic::{AtomicPtr, AtomicU64, Ordering};

use crate::cap::{ALLOC_ID_BITS, AllocId};
use crate::sys::alloc_zeroed_forever;

const CHUNK_BITS: u32 = 23;
const SUBDIR_BITS: u32 = 12;
const DIR_BITS: u32 = ALLOC_ID_BITS - CHUNK_BITS - SUBDIR_BITS;

const CHUNK_WORDS: usize = 1 << (CHUNK_BITS - 6);
const SUBDIR_LEN: usize = 1 << SUBDIR_BITS;
const DIR_LEN: usize = 1 << DIR_BITS;

/// One chunk: 2^23 liveness bits as atomic words.
type Chunk = [AtomicU64; CHUNK_WORDS];
/// One subdirectory: pointers to chunks.
type Subdir = [AtomicPtr<Chunk>; SUBDIR_LEN];

/// The bitmap. One static instance lives in [`crate::table`].
pub(crate) struct LivenessBitmap {
    dir: [AtomicPtr<Subdir>; DIR_LEN],
}

impl LivenessBitmap {
    pub(crate) const fn new() -> LivenessBitmap {
        LivenessBitmap {
            dir: [const { AtomicPtr::new(core::ptr::null_mut()) }; DIR_LEN],
        }
    }

    #[inline]
    fn split(id: AllocId) -> (usize, usize, usize, u64) {
        let raw = id.raw();
        let dir = (raw >> (CHUNK_BITS + SUBDIR_BITS)) as usize;
        let sub = ((raw >> CHUNK_BITS) as usize) & (SUBDIR_LEN - 1);
        let word = ((raw as usize) & ((1 << CHUNK_BITS) - 1)) >> 6;
        let bit = 1u64 << (raw & 63);
        (dir, sub, word, bit)
    }

    /// Chunk for `id`, mapping it (and its subdirectory) on first use.
    fn chunk(&self, dir: usize, sub: usize) -> &Chunk {
        let subdir_slot = &self.dir[dir];
        let mut subdir = subdir_slot.load(Ordering::Acquire);
        if subdir.is_null() {
            let fresh = alloc_zeroed_forever(size_of::<Subdir>()).cast::<Subdir>();
            match subdir_slot.compare_exchange(
                core::ptr::null_mut(),
                fresh,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => subdir = fresh,
                // Lost the race; the winner's mapping is used and ours is
                // forever-allocated but unreachable. Registration is not a
                // hot enough path to justify an unmap protocol here.
                Err(existing) => subdir = existing,
            }
        }
        // SAFETY: subdir is a live forever-mapping initialized to zero,
        // which is a valid all-null Subdir.
        let subdir = unsafe { &*subdir };

        let chunk_slot = &subdir[sub];
        let mut chunk = chunk_slot.load(Ordering::Acquire);
        if chunk.is_null() {
            let fresh = alloc_zeroed_forever(size_of::<Chunk>()).cast::<Chunk>();
            match chunk_slot.compare_exchange(
                core::ptr::null_mut(),
                fresh,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => chunk = fresh,
                Err(existing) => chunk = existing,
            }
        }
        // SAFETY: as above; zeroed memory is a valid all-dead Chunk.
        unsafe { &*chunk }
    }

    /// Marks `id` live. Called by registration before the allocation is
    /// published in the table.
    pub(crate) fn set(&self, id: AllocId) {
        let (dir, sub, word, bit) = Self::split(id);
        self.chunk(dir, sub)[word].fetch_or(bit, Ordering::Release);
    }

    /// Clears `id`'s liveness bit. Per I7 the caller does this *before*
    /// releasing the allocation's memory; Release ordering makes the clear
    /// visible before any later reuse of the address can be observed.
    pub(crate) fn clear(&self, id: AllocId) {
        let (dir, sub, word, bit) = Self::split(id);
        self.chunk(dir, sub)[word].fetch_and(!bit, Ordering::Release);
    }

    /// Whether `id` is live. The null id is never live.
    pub(crate) fn is_live(&self, id: AllocId) -> bool {
        if id.is_null() {
            return false;
        }
        let (dir, sub, word, bit) = Self::split(id);
        // Read through the directory without forcing chunks into
        // existence: an unmapped chunk means the id was never set.
        let subdir = self.dir[dir].load(Ordering::Acquire);
        if subdir.is_null() {
            return false;
        }
        // SAFETY: live forever-mapping, see chunk().
        let subdir: &Subdir = unsafe { &*subdir };
        let chunk = subdir[sub].load(Ordering::Acquire);
        if chunk.is_null() {
            return false;
        }
        // SAFETY: live forever-mapping, see chunk().
        let chunk: &Chunk = unsafe { &*chunk };
        chunk[word].load(Ordering::Acquire) & bit != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_query_clear() {
        let bm = LivenessBitmap::new();
        let a = AllocId::from_raw(1);
        let b = AllocId::from_raw(2);
        assert!(!bm.is_live(a));
        bm.set(a);
        assert!(bm.is_live(a));
        assert!(!bm.is_live(b));
        bm.clear(a);
        assert!(!bm.is_live(a));
    }

    #[test]
    fn null_id_never_live() {
        let bm = LivenessBitmap::new();
        assert!(!bm.is_live(AllocId::NULL));
    }

    #[test]
    fn ids_across_chunk_and_subdir_boundaries() {
        let bm = LivenessBitmap::new();
        let boundary_ids = [
            (1u64 << CHUNK_BITS) - 1,
            1u64 << CHUNK_BITS,
            (1u64 << (CHUNK_BITS + SUBDIR_BITS)) - 1,
            1u64 << (CHUNK_BITS + SUBDIR_BITS),
            crate::cap::ALLOC_ID_LIMIT - 1,
        ];
        for &raw in &boundary_ids {
            let id = AllocId::from_raw(raw);
            assert!(!bm.is_live(id), "id {raw:#x} unexpectedly live");
            bm.set(id);
            assert!(bm.is_live(id), "id {raw:#x} did not stick");
        }
        // Neighbours are untouched.
        assert!(!bm.is_live(AllocId::from_raw(boundary_ids[1] + 1)));
        for &raw in &boundary_ids {
            bm.clear(AllocId::from_raw(raw));
            assert!(!bm.is_live(AllocId::from_raw(raw)));
        }
    }
}
