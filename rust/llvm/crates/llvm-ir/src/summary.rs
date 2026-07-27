//! The ThinLTO module summary index, `^0 = module: (path: "a.o", ...)`.
//!
//! A combined index describes what every module in a link contains, so that
//! the linker can decide what to import without reading the modules. It is
//! written after the module body and refers to itself by `^N`.
//!
//! It is modelled syntactically, the way specialized debug-info nodes are:
//! the grammar is uniform (a keyword, a value, and tuples of keyed or
//! positional values nested to any depth) and nothing here reads what the
//! keywords mean. That is deliberate rather than lazy, and it has a
//! consequence worth knowing: `llvm-dis` fills in defaults it knows about
//! (`visibility: default`, `importType: definition`) and appends a
//! `blockcount` entry, so a summary printed by us matches what was written
//! rather than what upstream would print. Doing better means modelling
//! ThinLTO, which is a tier of its own.

/// One `^N = ...` line.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SummaryEntry {
    pub id: u32,
    /// The keyword before the colon: `module`, `gv`, `flags`, `blockcount`,
    /// `typeid`, `typeidCompatibleVTable`.
    pub kind: String,
    pub value: SummaryValue,
}

/// A value inside a summary entry.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SummaryValue {
    /// Counts, GUIDs and hashes, all of which fit 64 bits.
    Number(u64),
    String(String),
    /// `^3`, a reference to another entry.
    Ref(u32),
    /// A bare word: a linkage, a visibility, `null`, `none`, `notcold`.
    Word(String),
    /// `(a: 1, b: 2)` or `(1, 2, 3)`, and every mixture of the two.
    Tuple(Vec<SummaryField>),
}

/// One item of a tuple, named or positional.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SummaryField {
    pub key: Option<String>,
    pub value: SummaryValue,
}
