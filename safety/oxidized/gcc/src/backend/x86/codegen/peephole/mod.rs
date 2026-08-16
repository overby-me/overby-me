//! x86-64 peephole optimizer for assembly text.
//!
//! Operates on generated assembly text to eliminate redundant patterns from the
//! stack-based codegen. Lines are pre-parsed into `LineInfo` structs so hot-path
//! pattern matching uses integer/enum comparisons instead of string parsing.
//!
//! ## Pass structure
//!
//! 1. **Local passes** (iterative, up to 8 rounds): `combined_local_pass` merges
//!    7 single-scan patterns (self-moves, reverse-moves, redundant jumps,
//!    conditional branch inversion, adjacent store/load, redundant zero/sign
//!    extensions, redundant xorl %eax,%eax) plus push/pop elimination and
//!    binary-op push/pop rewriting.
//!
//! 2. **Global passes** (once): global store forwarding (across fallthrough
//!    labels), register copy propagation, dead register move elimination,
//!    dead store elimination, compare-and-branch fusion, and memory operand
//!    folding.
//!
//! 3. **Post-global cleanup + loop trampolines + tail calls + never-read stores
//!    + callee-save elimination + frame compaction**: see `passes/mod.rs`.

mod passes;
mod types;

pub(crate) use passes::peephole_optimize;
