//! What LangRef says about the intrinsics, in two generated tables.
//!
//! [`names`] is the set of base names it documents, which is what decides
//! whether an undeclared `llvm.*` call is an intrinsic upstream would build a
//! declaration for or a symbol that does not exist. [`table`] is the subset
//! of those whose signature LangRef states consistently enough to check.
//!
//! [`attributes`] is what upstream gives each of them, which is neither of
//! those things: an intrinsic carries attributes nothing in the text says,
//! and upstream replaces whatever a declaration was written with by them.
//! LangRef documents fourteen of those out of eight hundred `declare` lines,
//! which is why this one is measured against the assembler rather than read.
//!
//! All three come from `corpus/intrinsic-names.nu`,
//! `corpus/intrinsic-signatures.nu` and `corpus/intrinsic-attributes.nu`,
//! which explain their derivations.

pub mod attributes;
pub mod names;
pub mod overloads;
pub mod table;
