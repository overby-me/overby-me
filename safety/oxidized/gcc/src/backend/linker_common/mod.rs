//! Shared linker infrastructure for all backends.
//!
//! Split into focused submodules:
//!
//! - `types`: ELF64 object file types (Elf64Section, Elf64Symbol, Elf64Rela, Elf64Object, DynSymbol)
//! - `parse_object`: ELF64 relocatable object file parser
//! - `parse_shared`: Shared library (.so) symbol parsing and SONAME extraction
//! - `section_map`: Input-to-output section name mapping
//! - `dynstr`: Dynamic string table builder for .dynstr emission
//! - `hash`: GNU and SysV ELF hash functions
//! - `symbols`: InputSection, OutputSection, GlobalSymbolOps trait, linker-defined symbols
//! - `merge`: Section merging and common symbol allocation
//! - `dynamic`: Dynamic symbol matching, library loading, and symbol registration
//! - `archive`: Archive loading and generic file dispatch
//! - `resolve_lib`: Library name resolution helper
//! - `write`: ELF64 binary emission helpers (section/program headers, alignment)
//! - `args`: Shared linker argument parsing
//! - `check`: Post-link undefined symbol checking
//! - `eh_frame`: .eh_frame_hdr builder for stack unwinding
//! - `gc_sections`: Garbage collection (`--gc-sections`) for ELF64 linkers
//!
//! This module extracts the duplicated linker code that was copied across x86,
//! ARM, RISC-V, and (partially) i686 backends. It provides:
//!
//! - **ELF64 object parser**: `parse_elf64_object()` replaces near-identical
//!   `parse_object()` functions in x86, ARM, and RISC-V linkers.
//! - **Shared library parser**: `parse_shared_library_symbols()` and `parse_soname()`
//!   for extracting dynamic symbols from .so files.
//! - **Dynamic symbol matching**: `match_shared_library_dynsyms()`,
//!   `load_shared_library_elf64()`, and `resolve_dynamic_symbols_elf64()` for
//!   matching undefined globals against shared library exports with WEAK alias
//!   detection and as-needed semantics.
//! - **Linker-defined symbols**: `LINKER_DEFINED_SYMBOLS` constant and
//!   `is_linker_defined_symbol()` for the superset of symbols the linker
//!   provides during layout (used by all 4 backends).
//! - **Archive loading**: `load_archive_members()` and `member_resolves_undefined()`
//!   for iterative archive resolution (the --start-group algorithm).
//! - **Section mapping**: `map_section_name()` for input-to-output section mapping.
//! - **DynStrTab**: Dynamic string table builder for dynamic linking.
//! - **GNU hash**: `build_gnu_hash()` for .gnu.hash section generation.
//! - **ELF64 writing helpers**: `write_elf64_shdr()`, `write_elf64_phdr()`,
//!   `write_elf64_phdr_at()`, `align_up_64()`, `pad_to()` for binary emission.
//! - **Argument parsing**: `parse_linker_args()` and `LinkerArgs` for shared
//!   `-Wl,` flag parsing across backends.
//! - **Undefined symbol checking**: `check_undefined_symbols_elf64()` for
//!   post-link validation via the `GlobalSymbolOps` trait.
//!
//! Each backend linker still handles its own:
//! - Architecture-specific relocation application
//! - PLT/GOT layout (different instruction sequences per arch)
//! - ELF header emission (different e_machine, base addresses)
//! - Dynamic linking specifics (version tables, etc.)

// ── Submodule declarations ──────────────────────────────────────────────

mod archive;
mod args;
mod check;
mod dynamic;
mod dynstr;
mod eh_frame;
mod gc_sections;
mod hash;
mod merge;
mod parse_object;
mod parse_shared;
mod resolve_lib;
mod section_map;
mod symbols;
mod types;
mod write;

// ── Re-exports ──────────────────────────────────────────────────────────
//
// Re-export all public items at the linker_common:: level so that external
// callers see no change from the previous flat-file layout.

// types.rs
pub use types::{DynSymbol, Elf64Object, Elf64Rela, Elf64Section, Elf64Symbol};

// parse_object.rs
pub use parse_object::parse_elf64_object;

// parse_shared.rs
pub use parse_shared::{parse_shared_library_symbols, parse_soname};

// dynstr.rs
pub use dynstr::DynStrTab;

// hash.rs
pub use hash::{gnu_hash, sysv_hash};

// symbols.rs
pub use symbols::{
    is_linker_defined_symbol, is_valid_c_identifier_for_section, resolve_start_stop_symbols,
    GlobalSymbolOps, OutputSection,
};

// merge.rs
pub use merge::{allocate_common_symbols_elf64, merge_sections_elf64, merge_sections_elf64_gc};

// dynamic.rs
pub use dynamic::{
    load_shared_library_elf64, register_symbols_elf64, resolve_dynamic_symbols_elf64,
};

// archive.rs
pub use archive::{load_archive_elf64, load_thin_archive_elf64};

// resolve_lib.rs
pub use resolve_lib::resolve_lib;

// write.rs
pub use write::{align_up_64, pad_to, write_elf64_phdr, write_elf64_phdr_at, write_elf64_shdr};

// args.rs
pub use args::parse_linker_args;

// check.rs
pub use check::check_undefined_symbols_elf64;

// eh_frame.rs
pub use eh_frame::{build_eh_frame_hdr, count_eh_frame_fdes};

// gc_sections.rs
pub use gc_sections::gc_collect_sections_elf64;
