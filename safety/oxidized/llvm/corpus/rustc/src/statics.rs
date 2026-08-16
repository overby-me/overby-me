//@ flags: -Cpanic=abort
//! What a module holds outside its functions: constant aggregates, a string
//! literal with an alignment, a static pointing at another static so the
//! initializer is a relocation rather than a number, and a mutable one.
//! Between them they cover the initializer forms and the linkage and
//! alignment clauses a global carries.
//!
//! The mutable one holds an atomic rather than being a `static mut`, which
//! prints the same `global` rather than `constant` and does not put a bare
//! mutable global in the tree for a reader to copy.
#![no_std]

#[repr(C)]
pub struct Entry {
    pub key: u32,
    pub name: &'static str,
}

pub static TABLE: [Entry; 2] = [
    Entry { key: 1, name: "one" },
    Entry { key: 2, name: "two" },
];

pub static FIRST: &Entry = &TABLE[0];

pub static COUNTER: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

#[repr(align(64))]
pub struct Aligned(pub [u8; 8]);

pub static PADDED: Aligned = Aligned([0; 8]);

pub fn key_of(index: usize) -> u32 {
    TABLE[index].key
}

pub fn first_name() -> &'static str {
    FIRST.name
}
