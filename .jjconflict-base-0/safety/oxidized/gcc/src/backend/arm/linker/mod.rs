//! Native AArch64 ELF64 linker.
//!
//! Links ELF relocatable object files (.o) and static archives (.a) into
//! a dynamically-linked ELF64 executable for AArch64 (ARM 64-bit). Also supports
//! producing shared libraries (ET_DYN) via `link_shared()`.
//!
//! Shared linker infrastructure (ELF parsing, section merging, symbol registration,
//! common symbol allocation, archive loading) is provided by `linker_common`.
//! This module provides AArch64-specific logic: PLT/GOT construction, relocation
//! application, address layout, and ELF emission.
//!
//! This is the default linker (used when the `gcc_linker` feature is disabled).
//! CRT object discovery and library path resolution are handled by
//! common.rs's `resolve_builtin_link_setup`.
//!
//! ## Module structure
//!
//! - `elf`: ELF64 constants, type aliases, parsing (delegates to shared linker_common)
//! - `reloc`: AArch64-specific relocation application and encoding helpers
//! - `types`: `GlobalSymbol` struct, `GlobalSymbolOps` impl, arch constants
//! - `input`: Input file loading (objects, archives, shared libs, linker scripts)
//! - `plt_got`: PLT/GOT entry list construction from relocation scanning
//! - `link`: Orchestration - `link_builtin` and `link_shared` entry points
//! - `emit_dynamic`: Dynamic executable emission (PLT/GOT/.dynamic)
//! - `emit_shared`: Shared library (.so) emission
//! - `emit_static`: Static executable emission

#[allow(dead_code)] // Re-exports ELF constants/types; not all constants used by every linker path
pub mod elf;
mod emit_dynamic;
mod emit_shared;
mod emit_static;
mod input;
mod link;
mod plt_got;
pub mod reloc;
pub mod types;

#[cfg(not(feature = "gcc_linker"))]
pub use link::link_builtin;
#[cfg(not(feature = "gcc_linker"))]
pub use link::link_shared;
