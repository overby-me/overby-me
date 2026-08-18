//! Grow-only, index-addressed arenas backed by private forever-mmap
//! segments.
//!
//! Shared by the allocation table and the quarantine queue: both need
//! index-addressable metadata that must never touch the global allocator
//! (which `cementite` is about to become). Indices are `u32`; index 0 is
//! reserved as [`NONE`]. A `free_head` is provided for callers that keep a
//! freelist; the link field lives in the payload, so freelist *policy* is
//! the caller's (see `table`'s record/overflow reuse and `alloc`'s
//! quarantine nodes).

use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use crate::sys::alloc_zeroed_forever;

/// Reserved "no index" sentinel. Never handed out by [`Arena::bump`].
pub(crate) const NONE: u32 = 0;

const SEG_BITS: u32 = 16;
const SEG_LEN: usize = 1 << SEG_BITS;
const SEG_COUNT: usize = 1 << 12;

/// Grow-only arena of `T`, addressed by `u32` index. Segments are mapped on
/// demand and never released. Allocation happens under whatever mutex the
/// caller already holds, so the freelist head needs no CAS loop.
pub(crate) struct Arena<T> {
    segs: [AtomicPtr<T>; SEG_COUNT],
    next: AtomicU32,
    /// Freelist head for the caller's reuse policy, or [`NONE`].
    pub(crate) free_head: AtomicU32,
}

impl<T> Arena<T> {
    pub(crate) const fn new() -> Arena<T> {
        Arena {
            segs: [const { AtomicPtr::new(core::ptr::null_mut()) }; SEG_COUNT],
            next: AtomicU32::new(1),
            free_head: AtomicU32::new(NONE),
        }
    }

    pub(crate) fn get(&self, idx: u32) -> &T {
        debug_assert_ne!(idx, NONE);
        let seg = self.segs[(idx >> SEG_BITS) as usize].load(Ordering::Acquire);
        debug_assert!(!seg.is_null());
        // SAFETY: segments are live forever-mappings; idx < next is upheld
        // by construction (indices only come from bump()).
        unsafe { &*seg.add(idx as usize & (SEG_LEN - 1)) }
    }

    /// Bump-allocates a fresh index, mapping its segment on demand.
    pub(crate) fn bump(&self) -> u32 {
        let idx = self.next.fetch_add(1, Ordering::Relaxed);
        assert!(
            (idx as usize) < SEG_LEN * SEG_COUNT,
            "cementite: metadata arena exhausted"
        );
        let seg_slot = &self.segs[(idx >> SEG_BITS) as usize];
        if seg_slot.load(Ordering::Acquire).is_null() {
            let fresh = alloc_zeroed_forever(SEG_LEN * size_of::<T>()).cast::<T>();
            // A lost race would leave the loser's segment unused; cannot
            // happen under the caller's mutex, kept defensive anyway.
            let _ = seg_slot.compare_exchange(
                core::ptr::null_mut(),
                fresh,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
        idx
    }
}
