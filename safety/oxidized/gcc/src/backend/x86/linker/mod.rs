//! Native x86-64 ELF linker.
//!
//! Links ELF relocatable object files (.o) and static archives (.a) into
//! a dynamically-linked ELF executable. Resolves undefined symbols against
//! shared libraries (e.g., libc.so.6) and generates PLT/GOT entries for
//! dynamic function calls.
//!
//! ## Module structure
//!
//! - `elf`: ELF64 constants, type aliases, parsing (delegates to shared linker_common)
//! - `types`: `GlobalSymbol` struct, `GlobalSymbolOps` impl, arch constants
//! - `input`: Input file loading (objects, archives, shared libs, linker scripts)
//! - `plt_got`: PLT/GOT entry construction and IFUNC collection
//! - `link`: Orchestration - `link_builtin` and `link_shared` entry points
//! - `emit_exec`: Executable emission (both static and dynamic)
//! - `emit_shared`: Shared library (.so) emission

#[allow(dead_code)]
pub mod elf;
mod emit_exec;
mod emit_shared;
mod input;
mod link;
mod plt_got;
pub mod types;

#[cfg(not(feature = "gcc_linker"))]
pub use link::link_builtin;
#[cfg(not(feature = "gcc_linker"))]
pub use link::link_shared;
