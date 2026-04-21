//! Build orchestration. Splits responsibilities across:
//!   - `plan`: target resolution and dependency-order scheduling
//!   - `runner`: per-edge subprocess spawning, with optional parallelism
//!   - `expand`: edge-context variable expansion for `command`, `$in`,
//!     `$out`, etc.

mod depfile;
mod dyndep;
mod expand;
pub mod log;
mod plan;
mod runner;

pub use expand::expand_in_edge as expand_in_edge_pub;
pub use runner::run;
