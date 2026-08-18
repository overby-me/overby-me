//! Native RISC-V 64-bit ELF linker.
//!
//! Links ELF relocatable object files (.o) and static archives (.a) into
//! a dynamically-linked ELF64 executable for RISC-V 64-bit. Also supports
//! producing shared libraries (ET_DYN) via `link_shared()`.
//!
//! Shared linker infrastructure (ELF parsing, section merging, symbol registration,
//! common symbol allocation, archive loading) is provided by `linker_common`.
//! This module provides RISC-V-specific logic: PLT/GOT construction, relocation
//! application, address layout, and ELF emission.
//!
//! This is the default linker (used when the `gcc_linker` feature is disabled).
//!
//! ## Module structure
//!
//! - `elf_read`: ELF64 constants, type aliases, parsing (delegates to shared linker_common)
//! - `relocations`: RISC-V relocation constants, instruction patching, shared types
//! - `input`: Phase 1 - input file loading, archive resolution, shared lib discovery
//! - `sections`: Phase 2 - section merging with proper alignment
//! - `symbols`: Phase 3 - global symbol table construction, GOT/PLT identification
//! - `reloc`: Relocation application (shared between exec and .so linking)
//! - `link`: Orchestration - `link_builtin` and `link_shared` entry points
//! - `emit_exec`: Executable emission (static and dynamic)
//! - `emit_shared`: Shared library (.so) emission

mod elf_read;
mod emit_exec;
mod emit_shared;
mod input;
mod link;
mod reloc;
mod relocations;
mod sections;
mod symbols;

#[cfg(not(feature = "gcc_linker"))]
pub use link::link_builtin;
#[cfg(not(feature = "gcc_linker"))]
pub use link::link_shared;
