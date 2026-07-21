//! `cementite`: the Fe-C runtime. The hard phase (Fe3C) that ships inside a
//! hardened binary.
//!
//! Design record: `docs/cementite-api.md` and `docs/through-mode-coherence.md`
//! in the repository. The load-bearing invariants (PLAN.md §2):
//!
//! - I2: capability-shaped API; allocation ids are never recycled; liveness
//!   is a bitmap indexed by id.
//! - I7: `free` clears the liveness bit before releasing memory, and freed
//!   addresses pass through quarantine.
//! - I10: capabilities are resolved at derivation roots and propagated;
//!   checks compare at the dereference and never re-resolve from the
//!   faulting address.
//!
//! Populated by Task A2 (core data structures), A3 (allocator), and A4
//! (libc interposition).

pub const _TASK_A2_PENDING: () = ();
