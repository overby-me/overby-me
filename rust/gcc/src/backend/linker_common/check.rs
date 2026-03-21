//! Post-link undefined symbol checking.
//!
//! Validates that all required symbols have been resolved after linking,
//! filtering out dynamic, weak, and linker-defined symbols.

use std::collections::HashMap;

use super::symbols::{is_linker_defined_symbol, GlobalSymbolOps};
use crate::backend::elf::STB_WEAK;

/// Check for undefined symbols in the global symbol table and return an error
/// if any truly undefined symbols are found.
///
/// Filters out dynamic symbols, weak symbols, and linker-defined symbols
/// using the `GlobalSymbolOps` trait methods. `max_report` limits how many
/// symbols are shown in the error message (typically 20).
pub fn check_undefined_symbols_elf64<G: GlobalSymbolOps>(
    globals: &HashMap<String, G>,
    max_report: usize,
) -> Result<(), String> {
    let mut truly_undefined: Vec<&String> = globals
        .iter()
        .filter(|(name, sym)| {
            !sym.is_defined()
                && !sym.is_dynamic()
                && (sym.info() >> 4) != STB_WEAK
                && !is_linker_defined_symbol(name)
        })
        .map(|(name, _)| name)
        .collect();
    if truly_undefined.is_empty() {
        return Ok(());
    }
    truly_undefined.sort();
    truly_undefined.truncate(max_report);
    Err(format!(
        "undefined symbols: {}",
        truly_undefined
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}
