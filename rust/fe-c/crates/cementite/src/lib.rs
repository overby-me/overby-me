//! `cementite`: the Fe-C runtime. The hard phase (Fe3C) that ships inside a
//! hardened binary.
//!
//! Design record: `docs/cementite-api.md` and `docs/through-mode-coherence.md`
//! in the repository. The load-bearing invariants (PLAN.md, section 2):
//!
//! - I2: capability-shaped API; allocation ids are never recycled; liveness
//!   is a bitmap indexed by id.
//! - I7: `free` clears the liveness bit before releasing memory, and freed
//!   addresses pass through quarantine.
//! - I10: capabilities are resolved at derivation roots and propagated;
//!   checks compare at the dereference and never re-resolve from the
//!   faulting address.
//!
//! Current contents: capability types, the allocation table plus liveness
//! bitmap (Task A2), and the quarantining global allocator (Task A3). Libc
//! interposition (A4) and the check surface (phase B) build on these.

#![warn(missing_docs)]

mod alloc;
mod arena;
mod cap;
mod liveness;
mod sys;
pub mod table;

pub use alloc::{
    DEFAULT_QUARANTINE_BYTES, FecAlloc, quarantine_budget, quarantine_bytes, set_quarantine_budget,
};
pub use cap::{ALLOC_ID_BITS, ALLOC_ID_LIMIT, AllocId, Cap, CapFlags, PackedCap};
