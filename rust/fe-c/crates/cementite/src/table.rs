//! The allocation table: page-radix from address to allocation record.
//!
//! Shape (see `docs/cementite-api.md`): a three-level radix over 36-bit page
//! numbers (48-bit VA, 4 KiB pages, 12 bits per level). Each leaf is a
//! [`PageSlot`] describing the allocations touching that page:
//!
//! - up to four inline start entries (allocations whose base lies in the
//!   page), an overflow chain for denser pages, and
//! - a `spill` record for an allocation flowing into the page from an
//!   earlier one, so interior pointers into multi-page allocations resolve.
//!
//! Consulted at derivation roots and fallbacks only (I10), never per
//! access. Readers are lock-free: slots and records are seqlock-guarded
//! atomics (crossbeam-style: odd/even sequence with release/acquire
//! fences), writers serialize on one mutex. Registration order makes a
//! window where a capability is findable but dead impossible: the liveness
//! bit is set before the record is linked, and cleared before it is
//! unlinked (with the caller releasing memory only afterwards, per I7).
//!
//! Async-signal-safety of the read path holds (atomics only); the write
//! path takes a mutex and is not signal-safe. That matches the open
//! question recorded in the API draft and is v0-acceptable because
//! interceptors that allocate from signal handlers are already off the
//! table for the allocator itself.

use core::sync::atomic::{
    AtomicPtr, AtomicU16, AtomicU32, AtomicU64, AtomicUsize, Ordering, fence,
};
use std::sync::Mutex;

use crate::arena::{Arena, NONE};
use crate::cap::{ALLOC_ID_LIMIT, AllocId, Cap, CapFlags, PackedCap};
use crate::liveness::LivenessBitmap;
use crate::sys::alloc_zeroed_forever;

/// Page size the radix indexes by.
pub const PAGE_SHIFT: u32 = 12;
const PAGE_MASK: usize = (1 << PAGE_SHIFT) - 1;
/// Tracked virtual-address bits. Registration above this aborts loudly; the
/// runtime does not support opt-in 57-bit VA layouts yet.
pub const VA_BITS: u32 = 48;
const VA_LIMIT: usize = 1 << VA_BITS;

const LEVEL_BITS: u32 = 12;
const LEVEL_LEN: usize = 1 << LEVEL_BITS;
const LEVEL_MASK: usize = LEVEL_LEN - 1;

const INLINE_ENTRIES: usize = 4;
const OVERFLOW_CAP: usize = 30;

/// Per-page leaf. All fields are atomics read under the slot seqlock.
struct PageSlot {
    seq: AtomicU32,
    /// Occupied inline entries (dense, no holes).
    count: AtomicU32,
    /// Record of the allocation spilling into this page from an earlier
    /// page, or [`NONE`].
    spill: AtomicU32,
    /// Head of the overflow chain, or [`NONE`].
    overflow: AtomicU32,
    /// Base offsets within the page for the inline entries.
    offs: [AtomicU16; INLINE_ENTRIES],
    /// Record indices for the inline entries.
    recs: [AtomicU32; INLINE_ENTRIES],
}

/// Overflow node for pages with more than four allocation starts. Entries
/// are append-only with `rec == NONE` tombstones; the chain is recycled
/// when the page empties.
struct OverflowNode {
    next: AtomicU32,
    len: AtomicU32,
    offs: [AtomicU16; OVERFLOW_CAP],
    recs: [AtomicU32; OVERFLOW_CAP],
}

/// Allocation record. Fields are written before the record is linked into
/// any slot; membership changes bump the owning slot's seq, so readers
/// validating the slot seq never observe a partially updated record.
struct Record {
    /// Reserved for record-level updates that do not change membership
    /// (flag updates such as escape marking); bumped odd/even like slots.
    seq: AtomicU32,
    /// Registration site (SiteId, 0 = unknown). Doubles as the freelist
    /// link while the record is free.
    site: AtomicU32,
    base: AtomicUsize,
    len: AtomicUsize,
    /// `PackedCap` encoding: id in the high 48 bits, flags in the low 16.
    id_flags: AtomicU64,
}

type L3 = [PageSlot; LEVEL_LEN];
type L2 = [AtomicPtr<L3>; LEVEL_LEN];
type L1 = [AtomicPtr<L2>; LEVEL_LEN];

/// The table singleton plus every piece of state it owns.
struct Table {
    write_lock: Mutex<()>,
    l1: L1,
    records: Arena<Record>,
    overflow: Arena<OverflowNode>,
    liveness: LivenessBitmap,
    next_id: AtomicU64,
}

// SAFETY: all interior state is atomics or the mutex; raw node pointers are
// only ever published after full initialization.
unsafe impl Sync for Table {}

static TABLE: Table = Table {
    write_lock: Mutex::new(()),
    l1: [const { AtomicPtr::new(core::ptr::null_mut()) }; LEVEL_LEN],
    records: Arena::new(),
    overflow: Arena::new(),
    liveness: LivenessBitmap::new(),
    next_id: AtomicU64::new(1),
};

/// Seqlock write guard over one slot: bumps to odd, runs `f`, bumps to
/// even. Callers hold the table write mutex.
fn slot_write<R>(slot: &PageSlot, f: impl FnOnce(&PageSlot) -> R) -> R {
    let s = slot.seq.load(Ordering::Relaxed);
    slot.seq.store(s.wrapping_add(1), Ordering::Relaxed);
    fence(Ordering::Release);
    let r = f(slot);
    slot.seq.store(s.wrapping_add(2), Ordering::Release);
    r
}

/// Walks the radix to the slot for page number `page_num`.
fn slot_for(page_num: usize, create: bool) -> Option<&'static PageSlot> {
    let l1_idx = page_num >> (2 * LEVEL_BITS);
    let l2_idx = (page_num >> LEVEL_BITS) & LEVEL_MASK;
    let l3_idx = page_num & LEVEL_MASK;

    let l2 = descend::<L2>(&TABLE.l1[l1_idx], create)?;
    let l3 = descend::<L3>(&l2[l2_idx], create)?;
    Some(&l3[l3_idx])
}

/// Loads a child node, mapping it on first use when `create` is set.
fn descend<N>(slot: &AtomicPtr<N>, create: bool) -> Option<&'static N> {
    let mut node = slot.load(Ordering::Acquire);
    if node.is_null() {
        if !create {
            return None;
        }
        let fresh = alloc_zeroed_forever(size_of::<N>()).cast::<N>();
        match slot.compare_exchange(
            core::ptr::null_mut(),
            fresh,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => node = fresh,
            Err(existing) => node = existing,
        }
    }
    // SAFETY: node is a live forever-mapping; zeroed memory is a valid
    // empty node for all three levels (null pointers / empty slots).
    Some(unsafe { &*node })
}

/// What [`deregister`] found. The record is intentionally kept allocated
/// until [`release_record`] so violation reports can still name the freed
/// allocation while it sits in quarantine (I7 / trace -0130).
#[derive(Debug, Clone, Copy)]
pub struct FreedAlloc {
    /// Identity the allocation had.
    pub id: AllocId,
    /// Its base address.
    pub base: usize,
    /// Its length in bytes.
    pub len: usize,
    /// Opaque handle for [`release_record`].
    pub record: u32,
}

/// Registers `[base, base + len)` as a live allocation and returns its
/// fresh, never-recycled id.
///
/// Entry point for the allocator and libc interceptors only; instrumented
/// code never calls this directly.
///
/// # Panics
///
/// Panics on zero length, null/overflowing/out-of-VA ranges, id-space
/// exhaustion, or overlap with an existing registration. Each of those is
/// runtime-integrity corruption, not an input condition (PLAN section 7:
/// louder failures).
pub fn register(base: usize, len: usize, flags: CapFlags, site: u32) -> AllocId {
    assert!(len > 0, "cementite: zero-length registration");
    assert!(base != 0, "cementite: null registration");
    let end = base
        .checked_add(len)
        .expect("cementite: registration wraps the address space");
    assert!(
        end <= VA_LIMIT,
        "cementite: registration above the 48-bit VA limit"
    );

    let _guard = TABLE.write_lock.lock().unwrap();

    let raw_id = TABLE.next_id.fetch_add(1, Ordering::Relaxed);
    assert!(
        raw_id < ALLOC_ID_LIMIT,
        "cementite: allocation ids exhausted"
    );
    let id = AllocId::from_raw(raw_id);

    let rec_idx = rec_alloc();
    let rec = TABLE.records.get(rec_idx);
    let rseq = rec.seq.load(Ordering::Relaxed);
    rec.seq.store(rseq.wrapping_add(1), Ordering::Relaxed);
    fence(Ordering::Release);
    rec.base.store(base, Ordering::Relaxed);
    rec.len.store(len, Ordering::Relaxed);
    rec.site.store(site, Ordering::Relaxed);
    rec.id_flags
        .store(PackedCap::pack(id, flags).id_and_flags, Ordering::Relaxed);
    rec.seq.store(rseq.wrapping_add(2), Ordering::Release);

    // Live before findable: a lookup can never resolve a dead capability.
    TABLE.liveness.set(id);

    let first_page = base >> PAGE_SHIFT;
    let last_page = (end - 1) >> PAGE_SHIFT;

    let first_slot = slot_for(first_page, true).expect("create");
    slot_write(first_slot, |s| {
        insert_start(s, (base & PAGE_MASK) as u16, rec_idx)
    });

    for page_num in first_page + 1..=last_page {
        let slot = slot_for(page_num, true).expect("create");
        slot_write(slot, |s| {
            let prev = s.spill.swap(rec_idx, Ordering::Relaxed);
            assert_eq!(
                prev, NONE,
                "cementite: overlapping registrations (spill collision)"
            );
        });
    }

    id
}

/// Inserts a start entry into a slot (inline first, then overflow chain,
/// reusing tombstones). Runs inside [`slot_write`].
fn insert_start(slot: &PageSlot, off: u16, rec_idx: u32) {
    let count = slot.count.load(Ordering::Relaxed) as usize;
    if count < INLINE_ENTRIES {
        slot.offs[count].store(off, Ordering::Relaxed);
        slot.recs[count].store(rec_idx, Ordering::Relaxed);
        slot.count.store(count as u32 + 1, Ordering::Relaxed);
        return;
    }

    // Reuse a tombstone anywhere in the chain.
    let mut node_idx = slot.overflow.load(Ordering::Relaxed);
    while node_idx != NONE {
        let node = TABLE.overflow.get(node_idx);
        let len = node.len.load(Ordering::Relaxed) as usize;
        for j in 0..len {
            if node.recs[j].load(Ordering::Relaxed) == NONE {
                node.offs[j].store(off, Ordering::Relaxed);
                node.recs[j].store(rec_idx, Ordering::Relaxed);
                return;
            }
        }
        if len < OVERFLOW_CAP {
            node.offs[len].store(off, Ordering::Relaxed);
            node.recs[len].store(rec_idx, Ordering::Relaxed);
            node.len.store(len as u32 + 1, Ordering::Relaxed);
            return;
        }
        node_idx = node.next.load(Ordering::Relaxed);
    }

    // Chain full (or absent): push a fresh node in front.
    let fresh_idx = ovf_alloc();
    let fresh = TABLE.overflow.get(fresh_idx);
    fresh
        .next
        .store(slot.overflow.load(Ordering::Relaxed), Ordering::Relaxed);
    fresh.offs[0].store(off, Ordering::Relaxed);
    fresh.recs[0].store(rec_idx, Ordering::Relaxed);
    fresh.len.store(1, Ordering::Relaxed);
    slot.overflow.store(fresh_idx, Ordering::Relaxed);
}

/// Unregisters the allocation starting at exactly `base`.
///
/// Clears the liveness bit first (I7: the caller releases the memory only
/// after this returns), then unlinks the range. Returns `None` when no
/// allocation starts at `base`, which the interposition tier uses to pass
/// through frees of pointers that predate the interceptors.
pub fn deregister(base: usize) -> Option<FreedAlloc> {
    let _guard = TABLE.write_lock.lock().unwrap();

    let first_page = base >> PAGE_SHIFT;
    let slot = slot_for(first_page, false)?;
    let rec_idx = find_start(slot, base)?;
    let rec = TABLE.records.get(rec_idx);
    let len = rec.len.load(Ordering::Relaxed);
    let packed = PackedCap {
        id_and_flags: rec.id_flags.load(Ordering::Relaxed),
    };
    let id = packed.id();

    // Dead before unlinked: no window where the table serves a capability
    // whose liveness bit is still set for a freed allocation.
    TABLE.liveness.clear(id);

    slot_write(slot, |s| remove_start(s, rec_idx));

    let last_page = (base + len - 1) >> PAGE_SHIFT;
    for page_num in first_page + 1..=last_page {
        let spill_slot = slot_for(page_num, false).expect("spill slot must exist");
        slot_write(spill_slot, |s| {
            let prev = s.spill.swap(NONE, Ordering::Relaxed);
            debug_assert_eq!(prev, rec_idx, "cementite: spill chain corrupted");
        });
    }

    Some(FreedAlloc {
        id,
        base,
        len,
        record: rec_idx,
    })
}

/// Finds the record index of the allocation starting at `base`, if any.
fn find_start(slot: &PageSlot, base: usize) -> Option<u32> {
    let count = slot.count.load(Ordering::Relaxed) as usize;
    for i in 0..count.min(INLINE_ENTRIES) {
        let rec_idx = slot.recs[i].load(Ordering::Relaxed);
        if rec_idx != NONE && TABLE.records.get(rec_idx).base.load(Ordering::Relaxed) == base {
            return Some(rec_idx);
        }
    }
    let mut node_idx = slot.overflow.load(Ordering::Relaxed);
    while node_idx != NONE {
        let node = TABLE.overflow.get(node_idx);
        let len = node.len.load(Ordering::Relaxed) as usize;
        for j in 0..len.min(OVERFLOW_CAP) {
            let rec_idx = node.recs[j].load(Ordering::Relaxed);
            if rec_idx != NONE && TABLE.records.get(rec_idx).base.load(Ordering::Relaxed) == base {
                return Some(rec_idx);
            }
        }
        node_idx = node.next.load(Ordering::Relaxed);
    }
    None
}

/// Removes the start entry pointing at `rec_idx`; recycles the overflow
/// chain when the page has no live entries left. Runs inside
/// [`slot_write`].
fn remove_start(slot: &PageSlot, rec_idx: u32) {
    let count = slot.count.load(Ordering::Relaxed) as usize;
    for i in 0..count.min(INLINE_ENTRIES) {
        if slot.recs[i].load(Ordering::Relaxed) == rec_idx {
            // Swap-with-last keeps the inline entries dense.
            let last = count - 1;
            slot.offs[i].store(slot.offs[last].load(Ordering::Relaxed), Ordering::Relaxed);
            slot.recs[i].store(slot.recs[last].load(Ordering::Relaxed), Ordering::Relaxed);
            slot.recs[last].store(NONE, Ordering::Relaxed);
            slot.count.store(last as u32, Ordering::Relaxed);
            maybe_recycle_overflow(slot);
            return;
        }
    }
    let mut node_idx = slot.overflow.load(Ordering::Relaxed);
    while node_idx != NONE {
        let node = TABLE.overflow.get(node_idx);
        let len = node.len.load(Ordering::Relaxed) as usize;
        for j in 0..len.min(OVERFLOW_CAP) {
            if node.recs[j].load(Ordering::Relaxed) == rec_idx {
                node.recs[j].store(NONE, Ordering::Relaxed);
                maybe_recycle_overflow(slot);
                return;
            }
        }
        node_idx = node.next.load(Ordering::Relaxed);
    }
    panic!("cementite: removing an unlinked start entry");
}

/// Frees the whole overflow chain back to the arena freelist once no live
/// entry remains anywhere in the page.
fn maybe_recycle_overflow(slot: &PageSlot) {
    if slot.count.load(Ordering::Relaxed) != 0 {
        return;
    }
    let mut node_idx = slot.overflow.load(Ordering::Relaxed);
    if node_idx == NONE {
        return;
    }
    // Any live tombstoned-around entry keeps the chain.
    let mut probe = node_idx;
    while probe != NONE {
        let node = TABLE.overflow.get(probe);
        let len = node.len.load(Ordering::Relaxed) as usize;
        for j in 0..len.min(OVERFLOW_CAP) {
            if node.recs[j].load(Ordering::Relaxed) != NONE {
                return;
            }
        }
        probe = node.next.load(Ordering::Relaxed);
    }
    slot.overflow.store(NONE, Ordering::Relaxed);
    while node_idx != NONE {
        let node = TABLE.overflow.get(node_idx);
        let next = node.next.load(Ordering::Relaxed);
        node.len.store(0, Ordering::Relaxed);
        node.next.store(
            TABLE.overflow.free_head.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        TABLE.overflow.free_head.store(node_idx, Ordering::Relaxed);
        node_idx = next;
    }
}

/// Returns a freed allocation's record to the arena. Called at quarantine
/// eviction (Task A3); after this the freed allocation's identity is no
/// longer reportable.
pub fn release_record(freed: FreedAlloc) {
    let _guard = TABLE.write_lock.lock().unwrap();
    let rec = TABLE.records.get(freed.record);
    let rseq = rec.seq.load(Ordering::Relaxed);
    rec.seq.store(rseq.wrapping_add(1), Ordering::Relaxed);
    fence(Ordering::Release);
    rec.base.store(0, Ordering::Relaxed);
    rec.len.store(0, Ordering::Relaxed);
    rec.id_flags.store(0, Ordering::Relaxed);
    rec.seq.store(rseq.wrapping_add(2), Ordering::Release);
    // Freelist link lives in the (now meaningless) site field.
    rec.site.store(
        TABLE.records.free_head.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
    TABLE
        .records
        .free_head
        .store(freed.record, Ordering::Relaxed);
}

fn rec_alloc() -> u32 {
    let head = TABLE.records.free_head.load(Ordering::Relaxed);
    if head != NONE {
        let next = TABLE.records.get(head).site.load(Ordering::Relaxed);
        TABLE.records.free_head.store(next, Ordering::Relaxed);
        return head;
    }
    TABLE.records.bump()
}

fn ovf_alloc() -> u32 {
    let head = TABLE.overflow.free_head.load(Ordering::Relaxed);
    if head != NONE {
        let next = TABLE.overflow.get(head).next.load(Ordering::Relaxed);
        TABLE.overflow.free_head.store(next, Ordering::Relaxed);
        return head;
    }
    TABLE.overflow.bump()
}

/// Whether `id` is still live. One atomic bitmap load; the temporal half
/// of every check.
#[inline]
pub fn is_live(id: AllocId) -> bool {
    TABLE.liveness.is_live(id)
}

/// Resolves the allocation containing `addr`, if any.
///
/// This is the derivation-root/fallback path only (I10): checks compare a
/// propagated capability at the dereference and never resolve the faulting
/// address here.
pub fn lookup(addr: usize) -> Option<Cap> {
    if addr >= VA_LIMIT {
        return None;
    }
    let slot = slot_for(addr >> PAGE_SHIFT, false)?;

    loop {
        let s1 = slot.seq.load(Ordering::Acquire);
        if s1 & 1 != 0 {
            core::hint::spin_loop();
            continue;
        }

        let found = scan_slot(slot, addr);

        fence(Ordering::Acquire);
        if slot.seq.load(Ordering::Relaxed) == s1 {
            return found;
        }
    }
}

/// One seqlock-guarded scan attempt over a slot. Results are only
/// meaningful if the caller's seq validation passes afterwards.
fn scan_slot(slot: &PageSlot, addr: usize) -> Option<Cap> {
    let count = slot.count.load(Ordering::Relaxed) as usize;
    for i in 0..count.min(INLINE_ENTRIES) {
        let rec_idx = slot.recs[i].load(Ordering::Relaxed);
        if rec_idx != NONE
            && let Some(cap) = record_cap_covering(rec_idx, addr)
        {
            return Some(cap);
        }
    }

    let mut node_idx = slot.overflow.load(Ordering::Relaxed);
    while node_idx != NONE {
        let node = TABLE.overflow.get(node_idx);
        let len = node.len.load(Ordering::Relaxed) as usize;
        for j in 0..len.min(OVERFLOW_CAP) {
            let rec_idx = node.recs[j].load(Ordering::Relaxed);
            if rec_idx != NONE
                && let Some(cap) = record_cap_covering(rec_idx, addr)
            {
                return Some(cap);
            }
        }
        node_idx = node.next.load(Ordering::Relaxed);
    }

    let spill = slot.spill.load(Ordering::Relaxed);
    if spill != NONE {
        return record_cap_covering(spill, addr);
    }
    None
}

/// Seq-stable read of a record, returned as a capability when it covers
/// `addr`.
fn record_cap_covering(rec_idx: u32, addr: usize) -> Option<Cap> {
    let rec = TABLE.records.get(rec_idx);
    loop {
        let r1 = rec.seq.load(Ordering::Acquire);
        if r1 & 1 != 0 {
            core::hint::spin_loop();
            continue;
        }
        let base = rec.base.load(Ordering::Relaxed);
        let len = rec.len.load(Ordering::Relaxed);
        let packed = PackedCap {
            id_and_flags: rec.id_flags.load(Ordering::Relaxed),
        };
        fence(Ordering::Acquire);
        if rec.seq.load(Ordering::Relaxed) != r1 {
            continue;
        }
        return if base != 0 && addr >= base && addr < base + len {
            Some(Cap {
                base,
                len,
                id: packed.id(),
                flags: packed.flags(),
            })
        } else {
            None
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distinct, non-overlapping fake address ranges per test. The table is
    /// a process-global singleton, so tests place their registrations in
    /// disjoint VA windows instead of resetting it.
    fn window(tag: usize) -> usize {
        0x1000_0000_0000 + tag * 0x1_0000_0000
    }

    #[test]
    fn register_lookup_deregister_roundtrip() {
        let base = window(1) + 0x123;
        let id = register(base, 64, CapFlags::EMPTY, 0);
        assert!(is_live(id));

        let cap = lookup(base).expect("base resolves");
        assert_eq!((cap.base, cap.len, cap.id), (base, 64, id));
        assert!(lookup(base + 63).is_some(), "last byte resolves");
        assert!(lookup(base + 64).is_none(), "one past the end misses");
        assert!(lookup(base - 1).is_none(), "before the base misses");

        let freed = deregister(base).expect("was registered");
        assert_eq!((freed.id, freed.base, freed.len), (id, base, 64));
        assert!(!is_live(id), "liveness cleared by deregister (I7)");
        assert!(lookup(base).is_none(), "unlinked after deregister");
        release_record(freed);
    }

    #[test]
    fn ids_are_never_recycled() {
        let base = window(2);
        let a = register(base, 16, CapFlags::EMPTY, 0);
        let freed = deregister(base).unwrap();
        release_record(freed);
        let b = register(base, 16, CapFlags::EMPTY, 0);
        assert_ne!(a, b, "same address, fresh id: no ABA");
        assert!(a.raw() < b.raw(), "ids are monotone");
        assert!(!is_live(a));
        assert!(is_live(b));
        release_record(deregister(base).unwrap());
    }

    #[test]
    fn multiple_allocations_share_a_page() {
        let page = window(3);
        let a = register(page + 0x10, 0x10, CapFlags::EMPTY, 0);
        let b = register(page + 0x40, 0x08, CapFlags::EMPTY, 0);
        let c = register(page + 0x80, 0x20, CapFlags::EMPTY, 0);

        assert_eq!(lookup(page + 0x15).unwrap().id, a);
        assert_eq!(lookup(page + 0x44).unwrap().id, b);
        assert_eq!(lookup(page + 0x9f).unwrap().id, c);
        assert!(lookup(page + 0x20).is_none(), "gap between a and b");
        assert!(lookup(page + 0x48).is_none(), "gap after b");

        release_record(deregister(page + 0x40).unwrap());
        assert!(lookup(page + 0x44).is_none());
        assert_eq!(lookup(page + 0x15).unwrap().id, a, "a survives b's removal");
        assert_eq!(lookup(page + 0x9f).unwrap().id, c, "c survives b's removal");
        release_record(deregister(page + 0x10).unwrap());
        release_record(deregister(page + 0x80).unwrap());
    }

    #[test]
    fn interior_pointers_into_spanning_allocation() {
        let base = window(4) + 0x800;
        let len = 5 * 4096;
        let id = register(base, len, CapFlags::EMPTY, 0);

        for probe in [base, base + 4096, base + 3 * 4096 + 7, base + len - 1] {
            let cap = lookup(probe).expect("interior resolves through spill");
            assert_eq!(cap.id, id);
            assert_eq!((cap.base, cap.len), (base, len));
        }
        assert!(lookup(base + len).is_none());

        let freed = deregister(base).unwrap();
        assert!(lookup(base + 4096).is_none(), "spill pages unlinked");
        release_record(freed);
    }

    #[test]
    fn small_allocation_beside_spanning_tail() {
        // Allocation A spans into page P; allocation B starts inside P.
        // Both must resolve in P.
        let a_base = window(5) + 0xf00;
        let a_id = register(a_base, 0x400, CapFlags::EMPTY, 0); // ends 0x300 into P
        let p = (a_base + 0x400) & !0xfff;
        let b_base = p + 0x800;
        let b_id = register(b_base, 0x10, CapFlags::EMPTY, 0);

        assert_eq!(lookup(a_base + 0x3ff).unwrap().id, a_id, "tail of A in P");
        assert_eq!(lookup(b_base).unwrap().id, b_id, "B's start entry in P");
        assert!(lookup(p + 0x400).is_none(), "gap between A's end and B");

        release_record(deregister(a_base).unwrap());
        assert_eq!(lookup(b_base).unwrap().id, b_id);
        release_record(deregister(b_base).unwrap());
    }

    #[test]
    fn dense_page_overflows_inline_entries() {
        let page = window(6);
        let n = 40; // 4 inline + 36 overflow
        let ids: Vec<AllocId> = (0..n)
            .map(|i| register(page + i * 0x40, 0x20, CapFlags::EMPTY, 0))
            .collect();
        for (i, &id) in ids.iter().enumerate() {
            let cap = lookup(page + i * 0x40 + 5).expect("dense entry resolves");
            assert_eq!(cap.id, id);
        }
        // Remove the middle, check neighbours survive, then empty the page
        // so the chain recycles.
        release_record(deregister(page + 20 * 0x40).unwrap());
        assert!(lookup(page + 20 * 0x40).is_none());
        assert_eq!(lookup(page + 19 * 0x40).unwrap().id, ids[19]);
        assert_eq!(lookup(page + 21 * 0x40).unwrap().id, ids[21]);
        for i in (0..n).filter(|&i| i != 20) {
            release_record(deregister(page + i * 0x40).unwrap());
        }
        assert!(lookup(page + 5).is_none());

        // The page works again after full recycling.
        let re = register(page + 0x40, 0x20, CapFlags::EMPTY, 0);
        assert_eq!(lookup(page + 0x41).unwrap().id, re);
        release_record(deregister(page + 0x40).unwrap());
    }

    #[test]
    fn flags_travel_through_lookup() {
        let base = window(7);
        let flags = CapFlags::STACK.union(CapFlags::ESCAPED);
        let id = register(base, 32, flags, 0);
        let cap = lookup(base).unwrap();
        assert_eq!(cap.flags, flags);
        assert_eq!(cap.id, id);
        release_record(deregister(base).unwrap());
    }

    #[test]
    fn deregister_unknown_base_is_none() {
        assert!(deregister(window(8) + 0x50).is_none());
        // A page with entries still misses on a non-start address.
        let base = window(8) + 0x100;
        register(base, 8, CapFlags::EMPTY, 0);
        assert!(deregister(base + 1).is_none());
        release_record(deregister(base).unwrap());
    }

    #[test]
    fn concurrent_readers_see_consistent_state() {
        use std::sync::atomic::AtomicBool;

        let base_window = window(9);
        let stop = &*Box::leak(Box::new(AtomicBool::new(false)));

        // Iteration counts trimmed under Miri; the interleaving space is
        // what matters, not the volume.
        let (writer_iters, reader_iters) = if cfg!(miri) {
            (40, 200)
        } else {
            (2000, 200_000)
        };

        let writer = std::thread::spawn(move || {
            for i in 0..writer_iters {
                let base = base_window + (i % 16) * 0x100 + 0x10;
                let id = register(base, 0x80, CapFlags::EMPTY, 0);
                assert!(is_live(id));
                let freed = deregister(base).unwrap();
                release_record(freed);
            }
            stop.store(true, Ordering::Release);
        });

        let reader = std::thread::spawn(move || {
            let mut hits = 0usize;
            for i in 0..reader_iters {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let probe = base_window + (i % 16) * 0x100 + 0x20;
                if let Some(cap) = lookup(probe) {
                    // A capability handed out must be internally coherent.
                    assert!(cap.covers(probe, 1));
                    assert!(!cap.id.is_null());
                    hits += 1;
                }
            }
            hits
        });

        writer.join().unwrap();
        let _ = reader.join().unwrap();
    }
}
