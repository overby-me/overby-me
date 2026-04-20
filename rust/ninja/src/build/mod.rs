//! Build orchestration. Splits responsibilities across:
//!   - `plan`: target resolution and dependency-order scheduling
//!   - `runner`: per-edge subprocess spawning, with optional parallelism
//!   - `expand`: edge-context variable expansion for `command`, `$in`,
//!     `$out`, etc.

mod expand;
mod plan;
mod runner;

pub use runner::run;
