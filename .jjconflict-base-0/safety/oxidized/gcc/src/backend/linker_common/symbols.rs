//! Shared linker data structures and linker-defined symbol tables.
//!
//! Contains `InputSection`, `OutputSection`, and the `GlobalSymbolOps` trait
//! shared across x86 and ARM 64-bit linkers. Also defines the set of
//! linker-provided symbols and `__start_`/`__stop_` resolution logic.

use super::types::{DynSymbol, Elf64Symbol};

/// Reference to one input section placed within an output section.
pub struct InputSection {
    pub object_idx: usize,
    pub section_idx: usize,
    pub output_offset: u64,
    pub size: u64,
}

/// A merged output section in the final executable or shared library.
pub struct OutputSection {
    pub name: String,
    pub sh_type: u32,
    pub flags: u64,
    pub alignment: u64,
    pub inputs: Vec<InputSection>,
    pub data: Vec<u8>,
    pub addr: u64,
    pub file_offset: u64,
    pub mem_size: u64,
}

/// Trait abstracting over backend-specific GlobalSymbol types.
///
/// Provides the interface needed by shared linker functions: symbol registration,
/// section merging, dynamic symbol matching, and common symbol allocation.
/// Each backend implements this for its own GlobalSymbol struct.
pub trait GlobalSymbolOps: Clone {
    fn is_defined(&self) -> bool;
    fn is_dynamic(&self) -> bool;
    fn info(&self) -> u8;
    fn section_idx(&self) -> u16;
    fn value(&self) -> u64;
    fn size(&self) -> u64;
    fn new_defined(obj_idx: usize, sym: &Elf64Symbol) -> Self;
    fn new_common(obj_idx: usize, sym: &Elf64Symbol) -> Self;
    fn new_undefined(sym: &Elf64Symbol) -> Self;
    fn set_common_bss(&mut self, bss_offset: u64);

    /// Create a GlobalSymbol representing a dynamic symbol resolved from a shared library.
    fn new_dynamic(dsym: &DynSymbol, soname: &str) -> Self;
}

// ── Linker-defined symbols ──────────────────────────────────────────────
//
// These symbols are provided by the linker during layout and should not be
// reported as undefined. The superset covers all architectures (x86, ARM,
// RISC-V, i686). Architecture-specific symbols (e.g., __global_pointer$ for
// RISC-V) are included; having extra entries is harmless.

/// Symbols that the linker defines during layout.
///
/// Used by `is_linker_defined_symbol()` and `resolve_dynamic_symbols_elf64()`
/// to avoid false "undefined symbol" errors and unnecessary shared library lookups.
pub const LINKER_DEFINED_SYMBOLS: &[&str] = &[
    "_GLOBAL_OFFSET_TABLE_",
    "__bss_start",
    "__bss_start__",
    "__BSS_END__",
    "_edata",
    "edata",
    "_end",
    "end",
    "__end",
    "__end__",
    "_etext",
    "etext",
    "__ehdr_start",
    "__executable_start",
    // Note: _start is intentionally excluded -- it comes from crt1.o, not the linker.
    // Suppressing it here would mask missing-CRT errors.
    "__dso_handle",
    "_DYNAMIC",
    "__data_start",
    "data_start",
    "__DATA_BEGIN__",
    "__SDATA_BEGIN__",
    "__init_array_start",
    "__init_array_end",
    "__fini_array_start",
    "__fini_array_end",
    "__preinit_array_start",
    "__preinit_array_end",
    "__rela_iplt_start",
    "__rela_iplt_end",
    "__rel_iplt_start",
    "__rel_iplt_end",
    "__global_pointer$", // RISC-V
    "_IO_stdin_used",
    "_init",
    "_fini",
    "___tls_get_addr", // i686 TLS
    "__tls_get_addr",  // x86-64 TLS
    // Exception handling / unwinding (often weak, but may appear undefined)
    "_ITM_registerTMCloneTable",
    "_ITM_deregisterTMCloneTable",
    "__gcc_personality_v0",
    "_Unwind_Resume",
    "_Unwind_ForcedUnwind",
    "_Unwind_GetCFA",
    "__pthread_initialize_minimal",
    "_dl_rtld_map",
    "__GNU_EH_FRAME_HDR",
    "__getauxval",
    // Dynamic linker debug interface (provided by ld-linux*.so at runtime)
    "_r_debug",
    "_dl_debug_state",
    "_dl_mcount",
];

/// Check whether a symbol name is one that the linker provides during layout.
///
/// This includes the static list of well-known linker symbols, plus the
/// GNU ld `__start_<section>` / `__stop_<section>` pattern: when an undefined
/// symbol matches `__start_X` or `__stop_X` where `X` is a valid C identifier,
/// the linker will auto-generate it to point to the start/end of section `X`.
pub fn is_linker_defined_symbol(name: &str) -> bool {
    if LINKER_DEFINED_SYMBOLS.contains(&name) {
        return true;
    }
    // Recognize __start_<ident> and __stop_<ident> patterns (GNU ld feature).
    // The section name must be a valid C identifier for this to apply.
    // Note: GNU ld only resolves these when section X actually exists, but we
    // accept the pattern here to suppress "undefined symbol" errors early.
    // Actual resolution (in each backend) is guarded by section existence.
    if let Some(suffix) = name
        .strip_prefix("__start_")
        .or_else(|| name.strip_prefix("__stop_"))
    {
        return is_valid_c_identifier_for_section(suffix);
    }
    false
}

/// Check if a string is a valid C identifier (used for __start_/__stop_ section pattern).
/// Also used by RISC-V linker which has different section structures.
pub fn is_valid_c_identifier_for_section(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Resolve `__start_<section>` and `__stop_<section>` symbols against the output sections.
///
/// GNU ld auto-generates these symbols when there are undefined references to
/// `__start_X` or `__stop_X` where `X` is the name of an existing output section
/// that is also a valid C identifier. `__start_X` gets the address of the section's
/// start, `__stop_X` gets the address of the section's end (start + size).
///
/// Returns a vector of (name, address) pairs for all resolved symbols.
pub fn resolve_start_stop_symbols(output_sections: &[OutputSection]) -> Vec<(String, u64)> {
    let mut result = Vec::new();
    for sec in output_sections {
        if is_valid_c_identifier_for_section(&sec.name) {
            result.push((format!("__start_{}", sec.name), sec.addr));
            result.push((format!("__stop_{}", sec.name), sec.addr + sec.mem_size));
        }
    }
    result
}
