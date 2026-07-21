//! `FecAlloc`: the quarantining global allocator, plus libc-interposition
//! bookkeeping (Task A3; the `dlsym` interceptors themselves are Task A4).
//!
//! Every successful allocation is registered in the allocation table with
//! its exact bounds. Every free runs the I7 sequence:
//!
//! 1. [`table::deregister`] clears the liveness bit — *before* any memory
//!    is released — so a stale pointer carrying the freed id fails
//!    structurally from this instant on.
//! 2. The address enters quarantine (bytes-budgeted FIFO), withholding it
//!    from reuse so a fresh allocation cannot present a valid capability at
//!    the same address while stale references exist (ABA defence, trace
//!    `rustsec-2021-0130` section F2).
//! 3. Only on eviction is the memory handed back to the system allocator
//!    and the table record released.
//!
//! The budget is an atomic with a compile-time default and a runtime
//! setter. Reading `FEC_*` env vars from inside a global allocator would
//! reenter the allocator (`std::env::var` allocates), so env wiring is
//! deferred to a safe process-init point in `cargo-fe-c`; the setter is
//! what v0 and its tests use.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::arena::{Arena, NONE};
use crate::cap::{AllocId, CapFlags};
use crate::table::{self, FreedAlloc};

/// Default quarantine budget: 16 MiB withheld from reuse. Tunable at
/// runtime via [`set_quarantine_budget`].
pub const DEFAULT_QUARANTINE_BYTES: usize = 16 << 20;

static QUARANTINE_BUDGET: AtomicUsize = AtomicUsize::new(DEFAULT_QUARANTINE_BYTES);

/// Sets the quarantine byte budget. Freed allocations are withheld from
/// reuse until the total quarantined bytes would exceed this, at which
/// point the oldest are released (FIFO). A budget of 0 disables quarantine
/// (every free is released immediately) while keeping the liveness/table
/// bookkeeping intact.
pub fn set_quarantine_budget(bytes: usize) {
    QUARANTINE_BUDGET.store(bytes, Ordering::Relaxed);
}

/// The current quarantine byte budget.
pub fn quarantine_budget() -> usize {
    QUARANTINE_BUDGET.load(Ordering::Relaxed)
}

/// Bytes currently held in quarantine (freed but not yet released).
pub fn quarantine_bytes() -> usize {
    QUARANTINE.state.lock().unwrap().bytes
}

/// One quarantined-but-not-yet-released allocation. Fields are plain (not
/// atomic): every access is under [`Quarantine::state`]'s mutex.
///
/// `ptr` keeps the *original* allocation pointer with its provenance, so
/// the eventual `System.dealloc` uses a real pointer rather than one
/// reconstructed from an integer address — Fe-C holds itself to the strict
/// provenance it exists to enforce.
struct QNode {
    next: u32,
    ptr: *mut u8,
    align: usize,
    freed: FreedAlloc,
}

impl QNode {
    const EMPTY: QNode = QNode {
        next: NONE,
        ptr: core::ptr::null_mut(),
        align: 1,
        freed: FreedAlloc {
            id: AllocId::NULL,
            base: 0,
            len: 0,
            record: NONE,
        },
    };
}

struct QState {
    head: u32,
    tail: u32,
    bytes: usize,
}

/// The quarantine FIFO. Nodes live in a forever-mmap arena so the
/// quarantine never allocates through the global allocator it backs.
struct Quarantine {
    nodes: Arena<QCell>,
    state: Mutex<QState>,
}

/// Interior-mutable cell for a [`QNode`], written only under the state
/// mutex. `Arena::get` hands out `&QCell`; the mutex makes the access
/// exclusive.
struct QCell {
    inner: std::cell::UnsafeCell<QNode>,
}

// SAFETY: every QCell access is serialized by Quarantine::state's mutex.
unsafe impl Sync for QCell {}

static QUARANTINE: Quarantine = Quarantine {
    nodes: Arena::new(),
    state: Mutex::new(QState {
        head: NONE,
        tail: NONE,
        bytes: 0,
    }),
};

/// Counts memory released while its allocation was still marked live —
/// i.e. an I7 ordering violation. Must stay 0. Compiled only in tests so
/// production pays nothing; the release path also `debug_assert`s it.
#[cfg(test)]
static RELEASED_WHILE_LIVE: AtomicUsize = AtomicUsize::new(0);

impl Quarantine {
    /// Reads a node for mutation under the held state lock. Interior
    /// mutability through the mutex is exactly what clippy's `mut_from_ref`
    /// cannot see: the lock, not the type, provides exclusivity.
    ///
    /// # Safety
    ///
    /// The caller holds `self.state`, and no other `&mut` to this node is
    /// live, so the returned borrow is exclusive.
    #[allow(clippy::mut_from_ref)]
    unsafe fn node(&self, idx: u32) -> &mut QNode {
        // SAFETY: idx came from the arena; the state lock is held, so no
        // other reference to this cell exists.
        unsafe { &mut *self.nodes.get(idx).inner.get() }
    }

    /// Pushes a freed allocation into quarantine, then evicts oldest
    /// entries until the budget is satisfied. Called after
    /// [`table::deregister`] has already cleared the liveness bit. `ptr`
    /// carries the original allocation's provenance for the deferred free.
    fn push_and_evict(&self, ptr: *mut u8, align: usize, freed: FreedAlloc) {
        let mut st = self.state.lock().unwrap();

        let idx = self.alloc_node(&mut st);
        // SAFETY: state lock held.
        let node = unsafe { self.node(idx) };
        node.next = NONE;
        node.ptr = ptr;
        node.align = align;
        node.freed = freed;

        if st.tail == NONE {
            st.head = idx;
        } else {
            // SAFETY: state lock held; tail is a valid node index.
            unsafe { self.node(st.tail) }.next = idx;
        }
        st.tail = idx;
        st.bytes += freed.len;

        let budget = quarantine_budget();
        while st.bytes > budget && st.head != NONE {
            self.evict_head(&mut st);
        }
    }

    /// Allocates a node index, reusing the freelist before bumping. The
    /// `_st` witness documents that the state lock is held, which is what
    /// makes the [`Quarantine::node`] accesses sound.
    fn alloc_node(&self, _st: &mut QState) -> u32 {
        let head = self.nodes.free_head.load(Ordering::Relaxed);
        if head != NONE {
            // SAFETY: state lock held (via &mut st); head is a free node.
            let next = unsafe { self.node(head) }.next;
            self.nodes.free_head.store(next, Ordering::Relaxed);
            return head;
        }
        let idx = self.nodes.bump();
        // Fresh segment memory is zeroed; place a defined QNode there.
        // SAFETY: state lock held; idx is freshly ours.
        unsafe { *self.node(idx) = QNode::EMPTY };
        idx
    }

    /// Releases the oldest quarantined allocation: hands its memory back to
    /// the system allocator and frees its table record. Runs under the
    /// state lock.
    fn evict_head(&self, st: &mut QState) {
        let idx = st.head;
        debug_assert_ne!(idx, NONE);
        // SAFETY: state lock held; head is a valid node.
        let (next, ptr, align, freed) = {
            let node = unsafe { self.node(idx) };
            (node.next, node.ptr, node.align, node.freed)
        };

        st.head = next;
        if st.head == NONE {
            st.tail = NONE;
        }
        st.bytes -= freed.len;

        // I7: by the time memory is released the liveness bit is already
        // clear (deregister cleared it on entry). A reorder that released
        // before clearing would trip this.
        debug_assert!(
            !table::is_live(freed.id),
            "I7 violated: releasing memory for a still-live allocation"
        );
        #[cfg(test)]
        if table::is_live(freed.id) {
            RELEASED_WHILE_LIVE.fetch_add(1, Ordering::Relaxed);
        }

        // Return the memory to the system allocator, then free the record
        // so the freed allocation is no longer reportable.
        if let Ok(layout) = Layout::from_size_align(freed.len, align) {
            // SAFETY: `ptr` is the original allocation pointer (provenance
            // intact); `len`/`align` reconstruct the exact layout the
            // system allocator handed out; the address left instrumented
            // circulation when it entered quarantine.
            unsafe { System.dealloc(ptr, layout) };
        }
        table::release_record(freed);

        // Recycle the node.
        // SAFETY: state lock held; idx is no longer linked in the FIFO.
        unsafe { self.node(idx) }.next = self.nodes.free_head.load(Ordering::Relaxed);
        self.nodes.free_head.store(idx, Ordering::Relaxed);
    }
}

/// The Fe-C global allocator. Install in a hardened binary with
/// `#[global_allocator] static A: FecAlloc = FecAlloc;`.
pub struct FecAlloc;

// SAFETY: alloc/dealloc forward to the system allocator for the actual
// memory and only add table bookkeeping around it; the returned pointers
// are exactly the system allocator's.
unsafe impl GlobalAlloc for FecAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded verbatim; layout is a valid non-zero layout by
        // the GlobalAlloc contract.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            table::register(ptr as usize, layout.size(), CapFlags::EMPTY, 0);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded verbatim.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            table::register(ptr as usize, layout.size(), CapFlags::EMPTY, 0);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        match table::deregister(ptr as usize) {
            // Registered: liveness bit is now clear; quarantine the address
            // and defer the real free to eviction.
            Some(freed) => QUARANTINE.push_and_evict(ptr, layout.align(), freed),
            // Not registered (allocated before install, or a zero-size
            // request the runtime skipped): free straight through.
            // SAFETY: ptr/layout came from a matching System allocation.
            None => unsafe { System.dealloc(ptr, layout) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::GlobalAlloc;

    /// Serializes tests: they share the process-global quarantine and
    /// budget, so they must not interleave.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn drain_quarantine() {
        // Budget 0 forces every held entry to be released.
        let saved = quarantine_budget();
        set_quarantine_budget(0);
        // A throwaway alloc/free cycle runs the eviction loop to empty.
        let l = Layout::from_size_align(16, 8).unwrap();
        // SAFETY: matched alloc/dealloc.
        unsafe {
            let p = FecAlloc.alloc(l);
            FecAlloc.dealloc(p, l);
        }
        set_quarantine_budget(saved);
    }

    #[test]
    fn alloc_registers_with_exact_bounds() {
        let _g = TEST_LOCK.lock().unwrap();
        let layout = Layout::from_size_align(48, 16).unwrap();
        // SAFETY: valid layout; pointer freed below.
        let p = unsafe { FecAlloc.alloc(layout) };
        assert!(!p.is_null());
        assert_eq!(p as usize % 16, 0);

        let cap = table::lookup(p as usize).expect("registered on alloc");
        assert_eq!(cap.base, p as usize);
        assert_eq!(cap.len, 48);
        assert!(table::is_live(cap.id));
        assert!(
            table::lookup(p as usize + 47).is_some(),
            "last byte in bounds"
        );

        // SAFETY: matched free.
        unsafe { FecAlloc.dealloc(p, layout) };
    }

    #[test]
    fn free_clears_liveness_before_release_and_withholds_address() {
        let _g = TEST_LOCK.lock().unwrap();
        // A generous budget so the freed block stays quarantined (not yet
        // released) after the free returns.
        set_quarantine_budget(DEFAULT_QUARANTINE_BYTES);

        let layout = Layout::from_size_align(64, 8).unwrap();
        // SAFETY: valid layout.
        let p = unsafe { FecAlloc.alloc(layout) };
        let id = table::lookup(p as usize).unwrap().id;
        assert!(table::is_live(id));

        let bytes_before = quarantine_bytes();
        // SAFETY: matched free.
        unsafe { FecAlloc.dealloc(p, layout) };

        // I7: liveness cleared synchronously at free, before the memory is
        // released. The address is still quarantined (withheld), so a stale
        // pointer carrying `id` fails on liveness, and no fresh allocation
        // can reuse this exact address yet (ABA defence).
        assert!(!table::is_live(id), "liveness cleared at free (I7)");
        assert!(
            table::lookup(p as usize).is_none(),
            "record unlinked from the table at free"
        );
        assert_eq!(
            quarantine_bytes(),
            bytes_before + 64,
            "freed block is quarantined, memory not yet released"
        );

        drain_quarantine();
    }

    #[test]
    fn quarantine_respects_byte_budget_under_churn() {
        let _g = TEST_LOCK.lock().unwrap();
        drain_quarantine();
        let budget = 64 * 1024;
        set_quarantine_budget(budget);

        // Real churn on native; a representative slice under Miri, whose
        // interpreter makes 10k allocation cycles impractically slow.
        let iters = if cfg!(miri) { 300 } else { 10_000 };
        let layout = Layout::from_size_align(1024, 8).unwrap();
        for _ in 0..iters {
            // SAFETY: matched alloc/free each iteration.
            unsafe {
                let p = FecAlloc.alloc(layout);
                assert!(!p.is_null());
                FecAlloc.dealloc(p, layout);
            }
            // After each free the quarantine is trimmed to budget (the last
            // freed block is 1 KiB <= budget, so the bound holds exactly).
            assert!(
                quarantine_bytes() <= budget,
                "quarantine {} exceeded budget {}",
                quarantine_bytes(),
                budget
            );
        }

        // No memory was ever released while still marked live (I7).
        assert_eq!(RELEASED_WHILE_LIVE.load(Ordering::Relaxed), 0);
        drain_quarantine();
    }

    #[test]
    fn oversized_free_does_not_blow_the_budget() {
        let _g = TEST_LOCK.lock().unwrap();
        drain_quarantine();
        set_quarantine_budget(4096);

        // A single allocation larger than the whole budget must still be
        // freed: it is released immediately rather than parked.
        let layout = Layout::from_size_align(1 << 16, 8).unwrap();
        // SAFETY: matched alloc/free.
        unsafe {
            let p = FecAlloc.alloc(layout);
            FecAlloc.dealloc(p, layout);
        }
        assert_eq!(quarantine_bytes(), 0, "oversized free not parked");
        drain_quarantine();
    }

    #[test]
    fn ids_are_not_reused_across_quarantined_free() {
        let _g = TEST_LOCK.lock().unwrap();
        set_quarantine_budget(DEFAULT_QUARANTINE_BYTES);
        let layout = Layout::from_size_align(32, 8).unwrap();

        // SAFETY: matched alloc/free.
        let (id1, p1) = unsafe {
            let p = FecAlloc.alloc(layout);
            let id = table::lookup(p as usize).unwrap().id;
            FecAlloc.dealloc(p, layout);
            (id, p)
        };
        assert!(!table::is_live(id1));

        // A subsequent allocation gets a fresh id, and cannot land on the
        // quarantined address: its memory is still held by the system
        // allocator (not released until eviction), so no reuse is possible.
        // SAFETY: matched alloc/free.
        unsafe {
            let p2 = FecAlloc.alloc(layout);
            let id2 = table::lookup(p2 as usize).unwrap().id;
            assert_ne!(id1, id2);
            assert_ne!(p2, p1, "quarantined address withheld from reuse");
            FecAlloc.dealloc(p2, layout);
        }
        drain_quarantine();
    }
}
