//! Shared ELF types, constants, and utilities used by all assembler and linker backends.
//!
//! Split into focused submodules:
//!
//! - `constants`: ELF format constants (section types, flags, symbol bindings, etc.)
//! - `string_table`: `StringTable` for building ELF string tables
//! - `io`: Binary read/write helpers and ELF structure writers
//! - `archive`: Archive (.a) and linker script parsing
//! - `linker_symbols`: Linker-defined symbols and section helpers
//! - `section_flags`: Section flags parsing for `.section` directives
//! - `parse_string`: String literal parser for assembler directives
//! - `object_writer`: Shared relocatable ELF object (.o) writer
//! - `numeric_labels`: Numeric local label resolution (x86/i686)
//! - `symbol_table`: Shared symbol table builder for assembler backends
//! - `writer_base`: `ElfWriterBase` for ARM/RISC-V assembler backends

mod archive;
mod constants;
mod io;
mod linker_symbols;
mod numeric_labels;
mod object_writer;
pub(crate) mod parse_string;
mod section_flags;
mod string_table;
mod symbol_table;
mod writer_base;

// Re-export everything at the elf:: level so existing `use crate::backend::elf::{...}`
// imports continue to work without any changes.

// constants
#[allow(unused_imports)]
pub use constants::*;

// string_table
#[allow(unused_imports)]
pub use string_table::StringTable;

// io
#[allow(unused_imports)]
pub use io::*;

// archive
#[allow(unused_imports)]
pub use archive::{
    is_thin_archive, parse_archive_members, parse_linker_script, parse_linker_script_entries,
    parse_thin_archive_members, LinkerScriptEntry,
};

// linker_symbols
#[allow(unused_imports)]
pub use linker_symbols::{
    default_section_flags, get_standard_linker_symbols, section_index, LinkerDefinedSym,
    LinkerSymbolAddresses,
};

// section_flags
pub use section_flags::parse_section_flags;

// parse_string
pub use parse_string::parse_string_literal;

// object_writer
pub use object_writer::{write_relocatable_object, ElfConfig, ObjReloc, ObjSection};

// numeric_labels
#[allow(unused_imports)]
pub use numeric_labels::{
    is_numeric_label, parse_numeric_ref, resolve_numeric_labels, resolve_numeric_name,
    resolve_numeric_refs_in_expr,
};

// symbol_table
pub use symbol_table::{build_elf_symbol_table, ObjSymbol, SymbolTableInput};

// writer_base
pub use writer_base::ElfWriterBase;
