//! What LangRef says about the intrinsics, in two generated tables.
//!
//! [`names`] is the set of base names it documents, which is what decides
//! whether an undeclared `llvm.*` call is an intrinsic upstream would build a
//! declaration for or a symbol that does not exist. [`table`] is the subset
//! of those whose signature LangRef states consistently enough to check.
//!
//! Both come from `corpus/intrinsic-names.nu` and
//! `corpus/intrinsic-signatures.nu`, which explain their derivations.

pub mod names;
pub mod table;
