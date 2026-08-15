//! What LangRef says about the intrinsics, in five generated tables.
//!
//! [`names`] is the set of base names it documents, which is what decides
//! whether an undeclared `llvm.*` call is an intrinsic upstream would build a
//! declaration for or a symbol that does not exist. [`table`] is the subset
//! of those whose signature LangRef states consistently enough to check, and
//! [`overloads`] is the other half of that reading: which positions LangRef
//! varies *together*, and so have to be one type at any call.
//!
//! [`attributes`] is what upstream gives each of them, which is none of those
//! things: an intrinsic carries attributes nothing in the text says, and
//! upstream replaces whatever a declaration was written with by them. LangRef
//! documents fourteen of those out of eight hundred `declare` lines, which is
//! why that one is measured against the assembler rather than read.
//!
//! [`mangling`] is about the name rather than the signature: which positions
//! an overloaded intrinsic carries in its own name, so that a module writing
//! the shorter one prints what upstream prints. [`mangle`] is what a type
//! spells there, which is measured separately and is not generated.
//!
//! They come from `corpus/intrinsic-names.nu`,
//! `corpus/intrinsic-signatures.nu`, `corpus/intrinsic-overloads.nu`,
//! `corpus/intrinsic-attributes.nu` and `corpus/intrinsic-mangling.nu`, which
//! explain their derivations. Each looks itself up through [`candidates`],
//! which [`reduce`] owns so that regenerating a table cannot take it away.

pub mod attributes;
pub mod declared;
pub mod mangle;
pub mod mangling;
pub mod names;
pub mod overloads;
pub mod recognised;
pub mod reduce;
pub mod renames;
pub mod rewrites;
pub mod table;

pub use reduce::{base_name, candidates, is_documented, is_known};
