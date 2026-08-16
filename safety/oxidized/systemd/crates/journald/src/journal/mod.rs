//! Journal core data structures and storage.
//!
//! This module re-exports the shared journal types from `libsystemd`
//! so that journald can use them without duplicating code.

pub use libsystemd::journal::entry;
pub use libsystemd::journal::storage;
