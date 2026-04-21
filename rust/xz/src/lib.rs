//! `rust-xz` library crate.
//!
//! All real logic lives in the sibling modules; the `xz` binary
//! (`src/main.rs`) is a thin wrapper that re-exports the same
//! modules privately. Exposing them as a `lib` target lets benches,
//! integration tests, and the fuzz harness link directly against
//! the codec without re-compiling everything.

pub mod cli;
pub mod codec;
pub mod list;
pub mod options;
pub mod processor;
pub mod suffix;
