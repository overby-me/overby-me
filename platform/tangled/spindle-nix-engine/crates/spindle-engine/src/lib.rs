//! Engine trait and implementations for `tangled-spindle-nix`.
//!
//! This crate defines the [`Engine`] trait that all execution engines must
//! implement, plus the `nix` engine that replaces Docker+Nixery with native
//! Nix builds and child process execution.
//!
//! # Architecture
//!
//! The engine is responsible for:
//! 1. Transforming incoming pipeline workflows into internal [`Workflow`] representations.
//! 2. Setting up the execution environment (building Nix closures, creating workspace dirs).
//! 3. Executing individual steps as child processes.
//! 4. Tearing down the execution environment after completion.

pub mod nix_deps;
pub mod nix_engine;
pub mod traits;
pub mod workspace;

pub use nix_engine::NixEngine;
pub use traits::Engine;

// Re-export `UnlockedSecret` from `spindle-models` for backward compatibility.
// The canonical definition lives in `spindle_models::UnlockedSecret`.
pub use spindle_models::UnlockedSecret;
