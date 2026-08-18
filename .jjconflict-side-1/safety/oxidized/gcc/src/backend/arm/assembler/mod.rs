//! Native AArch64 assembler.
//!
//! Parses `.s` assembly text (as emitted by the AArch64 codegen) and produces
//! ELF `.o` object files, removing the dependency on `aarch64-linux-gnu-gcc`
//! for assembly.
//!
//! Architecture:
//! - `parser.rs`     – Tokenize + parse assembly text into `AsmStatement` items
//! - `encoder.rs`    – Encode AArch64 instructions into 32-bit machine words
//! - `elf_writer.rs` – Write ELF object files with sections, symbols, and relocations

pub mod elf_writer;
pub mod encoder;
pub mod parser;

use elf_writer::ElfWriter;
use parser::{parse_asm, AsmDirective, AsmStatement, DataValue, Operand};
use std::collections::HashMap;

/// Assemble AArch64 assembly text into an ELF object file.
///
/// This is the default assembler (used when the `gcc_assembler` feature is disabled).
pub fn assemble(asm_text: &str, output_path: &str) -> Result<(), String> {
    let statements = parse_asm(asm_text)?;
    let statements = expand_literal_pools(&statements);
    let statements = resolve_numeric_labels(&statements);
    let mut writer = ElfWriter::new();
    writer.process_statements(&statements)?;
    writer.write_elf(output_path)?;
    Ok(())
}

/// Expand `LdrLiteralPool` pseudo-instructions into literal pool loads.
///
/// `ldr Xn, =symbol` must load the exact linker-resolved address via a literal
/// pool, not via `adrp+add` (which gives a PC-relative address). This matters
/// for code that runs at a different address than its linked address (e.g.,
/// early boot code running at physical addresses before MMU setup).
///
/// This pass replaces each `LdrLiteralPool` with a `ldr Xn, .Llpool_N`
/// instruction and accumulates pool entries. Pool entries are flushed (emitted
/// as `.quad` data with labels) when:
/// - A `.ltorg` / `.pool` directive is encountered
/// - A section-changing directive is encountered (`.section`, `.pushsection`, `.text`, `.data`)
/// - The end of the statement list is reached
///
/// Each pool entry is 8-byte aligned and consists of:
///   .Llpool_N: .quad symbol[+addend]
fn expand_literal_pools(statements: &[AsmStatement]) -> Vec<AsmStatement> {
    struct PoolEntry {
        label: String,
        symbol: String,
        addend: i64,
    }

    let mut result = Vec::with_capacity(statements.len());
    let mut pending_pool: Vec<PoolEntry> = Vec::new();
    let mut pool_counter: usize = 0;

    // TODO: Track W vs X register to emit .long (4-byte) entries for W registers
    // instead of always emitting .quad (8-byte). Currently safe on little-endian
    // (loads low 32 bits) but wrong on big-endian and wastes 4 bytes per W entry.
    let flush_pool = |pool: &mut Vec<PoolEntry>, out: &mut Vec<AsmStatement>| {
        if pool.is_empty() {
            return;
        }
        // Align pool to 8 bytes
        out.push(AsmStatement::Directive(AsmDirective::Balign(8)));
        for entry in pool.drain(..) {
            out.push(AsmStatement::Label(entry.label));
            if entry.addend != 0 {
                out.push(AsmStatement::Directive(AsmDirective::Quad(vec![
                    DataValue::SymbolOffset(entry.symbol, entry.addend),
                ])));
            } else {
                out.push(AsmStatement::Directive(AsmDirective::Quad(vec![
                    DataValue::Symbol(entry.symbol),
                ])));
            }
        }
    };

    for stmt in statements {
        match stmt {
            AsmStatement::LdrLiteralPool {
                reg,
                symbol,
                addend,
            } => {
                let pool_label = format!(".Llpool_{}", pool_counter);
                pool_counter += 1;

                // Emit: ldr reg, .Llpool_N (PC-relative literal load)
                result.push(AsmStatement::Instruction {
                    mnemonic: "ldr".to_string(),
                    operands: vec![
                        Operand::Reg(reg.clone()),
                        Operand::Symbol(pool_label.clone()),
                    ],
                    raw_operands: format!("{}, {}", reg, pool_label),
                });

                pending_pool.push(PoolEntry {
                    label: pool_label,
                    symbol: symbol.clone(),
                    addend: *addend,
                });
            }
            AsmStatement::Directive(AsmDirective::Ltorg) => {
                flush_pool(&mut pending_pool, &mut result);
            }
            AsmStatement::Directive(AsmDirective::Section(_))
            | AsmStatement::Directive(AsmDirective::PushSection(_))
            | AsmStatement::Directive(AsmDirective::PopSection)
            | AsmStatement::Directive(AsmDirective::Previous) => {
                // Flush pool before section change (pool must be in the same section)
                flush_pool(&mut pending_pool, &mut result);
                result.push(stmt.clone());
            }
            _ => {
                result.push(stmt.clone());
            }
        }
    }

    // Flush any remaining pool entries at the end
    flush_pool(&mut pending_pool, &mut result);

    result
}

/// Check if a label name is a GNU assembler numeric label (e.g., "1", "42").
fn is_numeric_label(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())
}

/// Check if a string is a numeric forward/backward reference like "1f" or "2b".
/// Returns Some((number_str, is_forward)) if it is, None otherwise.
fn parse_numeric_ref(name: &str) -> Option<(&str, bool)> {
    if name.len() < 2 {
        return None;
    }
    let last = name.as_bytes()[name.len() - 1];
    let num_part = &name[..name.len() - 1];
    if !num_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match last {
        b'f' | b'F' => Some((num_part, true)),
        b'b' | b'B' => Some((num_part, false)),
        _ => None,
    }
}

/// Resolve a numeric label reference to its unique name.
fn resolve_numeric_name(
    name: &str,
    current_idx: usize,
    defs: &HashMap<String, Vec<(usize, String)>>,
) -> Option<String> {
    let (num, is_forward) = parse_numeric_ref(name)?;
    let def_list = defs.get(num)?;

    if is_forward {
        def_list
            .iter()
            .find(|(idx, _)| *idx > current_idx)
            .map(|(_, name)| name.clone())
    } else {
        def_list
            .iter()
            .rev()
            .find(|(idx, _)| *idx < current_idx)
            .map(|(_, name)| name.clone())
    }
}

/// Resolve numeric local labels (1:, 2:, etc.) and their references (1f, 1b)
/// into unique internal label names.
///
/// GNU assembler numeric labels can be defined multiple times. Each forward
/// reference `Nf` refers to the next definition of `N`, and each backward
/// reference `Nb` refers to the most recent definition of `N`.
fn resolve_numeric_labels(statements: &[AsmStatement]) -> Vec<AsmStatement> {
    // First pass: find all numeric label definitions and assign unique names.
    let mut defs: HashMap<String, Vec<(usize, String)>> = HashMap::new();
    let mut unique_counter: HashMap<String, usize> = HashMap::new();

    for (i, stmt) in statements.iter().enumerate() {
        if let AsmStatement::Label(name) = stmt {
            if is_numeric_label(name) {
                let count = unique_counter.entry(name.clone()).or_insert(0);
                let unique_name = format!(".Lnum_{}_{}", name, *count);
                *count += 1;
                defs.entry(name.clone()).or_default().push((i, unique_name));
            }
        }
    }

    // If no numeric labels found, return original
    if defs.is_empty() {
        return statements.to_vec();
    }

    // Second pass: resolve all references
    let mut result = Vec::with_capacity(statements.len());
    for (i, stmt) in statements.iter().enumerate() {
        match stmt {
            AsmStatement::Label(name) if is_numeric_label(name) => {
                if let Some(def_list) = defs.get(name) {
                    if let Some((_, unique_name)) = def_list.iter().find(|(idx, _)| *idx == i) {
                        result.push(AsmStatement::Label(unique_name.clone()));
                        continue;
                    }
                }
                result.push(stmt.clone());
            }
            AsmStatement::Instruction {
                mnemonic,
                operands,
                raw_operands,
            } => {
                let new_ops: Vec<Operand> = operands
                    .iter()
                    .map(|op| resolve_numeric_operand(op, i, &defs))
                    .collect();
                result.push(AsmStatement::Instruction {
                    mnemonic: mnemonic.clone(),
                    operands: new_ops,
                    raw_operands: raw_operands.clone(),
                });
            }
            AsmStatement::Directive(dir) => {
                result.push(AsmStatement::Directive(resolve_numeric_directive(
                    dir, i, &defs,
                )));
            }
            _ => result.push(stmt.clone()),
        }
    }

    result
}

/// Resolve numeric label references in a single operand.
fn resolve_numeric_operand(
    op: &Operand,
    current_idx: usize,
    defs: &HashMap<String, Vec<(usize, String)>>,
) -> Operand {
    match op {
        Operand::Symbol(name) => {
            if let Some(resolved) = resolve_numeric_name(name, current_idx, defs) {
                Operand::Symbol(resolved)
            } else {
                op.clone()
            }
        }
        Operand::Label(name) => {
            if let Some(resolved) = resolve_numeric_name(name, current_idx, defs) {
                Operand::Label(resolved)
            } else {
                op.clone()
            }
        }
        Operand::SymbolOffset(name, off) => {
            if let Some(resolved) = resolve_numeric_name(name, current_idx, defs) {
                Operand::SymbolOffset(resolved, *off)
            } else {
                op.clone()
            }
        }
        Operand::MemExpr {
            base,
            expr,
            writeback,
        } => {
            let resolved_expr = resolve_numeric_refs_in_expr(expr, current_idx, defs);
            Operand::MemExpr {
                base: base.clone(),
                expr: resolved_expr,
                writeback: *writeback,
            }
        }
        Operand::Expr(expr) => {
            let resolved_expr = resolve_numeric_refs_in_expr(expr, current_idx, defs);
            Operand::Expr(resolved_expr)
        }
        _ => op.clone(),
    }
}

/// Resolve numeric label references (e.g., `1b`, `2f`) within an expression string.
/// Replaces each occurrence with the resolved unique label name (e.g., `.Lnum_1_3`).
fn resolve_numeric_refs_in_expr(
    expr: &str,
    current_idx: usize,
    defs: &HashMap<String, Vec<(usize, String)>>,
) -> String {
    let bytes = expr.as_bytes();
    let mut result = String::with_capacity(expr.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            // Skip hex literals (0x..., 0X...) and binary literals (0b..., 0B...)
            // to avoid misinterpreting hex digits as label refs (e.g., 0x1b)
            if bytes[i] == b'0'
                && i + 1 < bytes.len()
                && (bytes[i + 1] == b'x'
                    || bytes[i + 1] == b'X'
                    || bytes[i + 1] == b'b'
                    || bytes[i + 1] == b'B')
            {
                // Consume the entire hex/binary literal
                i += 2; // skip 0x or 0b
                while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                result.push_str(&expr[start..i]);
                continue;
            }
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            // Check if followed by 'b' or 'f' (not part of a longer identifier)
            if i < bytes.len()
                && (bytes[i] == b'b' || bytes[i] == b'f' || bytes[i] == b'B' || bytes[i] == b'F')
                && (i + 1 >= bytes.len() || !bytes[i + 1].is_ascii_alphanumeric())
            {
                let label_ref = &expr[start..=i];
                i += 1;
                if let Some(resolved) = resolve_numeric_name(label_ref, current_idx, defs) {
                    result.push_str(&resolved);
                } else {
                    result.push_str(label_ref);
                }
            } else {
                // Regular number
                result.push_str(&expr[start..i]);
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

/// Resolve numeric label references in data directives.
fn resolve_numeric_directive(
    dir: &AsmDirective,
    current_idx: usize,
    defs: &HashMap<String, Vec<(usize, String)>>,
) -> AsmDirective {
    match dir {
        AsmDirective::Byte(vals) => {
            AsmDirective::Byte(resolve_numeric_data_values(vals, current_idx, defs))
        }
        AsmDirective::Long(vals) => {
            AsmDirective::Long(resolve_numeric_data_values(vals, current_idx, defs))
        }
        AsmDirective::Quad(vals) => {
            AsmDirective::Quad(resolve_numeric_data_values(vals, current_idx, defs))
        }
        _ => dir.clone(),
    }
}

/// Resolve numeric label references in data values.
fn resolve_numeric_data_values(
    vals: &[DataValue],
    current_idx: usize,
    defs: &HashMap<String, Vec<(usize, String)>>,
) -> Vec<DataValue> {
    vals.iter()
        .map(|v| match v {
            DataValue::Symbol(name) => {
                if let Some(resolved) = resolve_numeric_name(name, current_idx, defs) {
                    DataValue::Symbol(resolved)
                } else {
                    v.clone()
                }
            }
            DataValue::SymbolOffset(name, off) => {
                if let Some(resolved) = resolve_numeric_name(name, current_idx, defs) {
                    DataValue::SymbolOffset(resolved, *off)
                } else {
                    v.clone()
                }
            }
            DataValue::SymbolDiff(a, b) => {
                let new_a = resolve_numeric_name(a, current_idx, defs).unwrap_or_else(|| a.clone());
                let new_b = resolve_numeric_name(b, current_idx, defs).unwrap_or_else(|| b.clone());
                DataValue::SymbolDiff(new_a, new_b)
            }
            DataValue::SymbolDiffAddend(a, b, add) => {
                let new_a = resolve_numeric_name(a, current_idx, defs).unwrap_or_else(|| a.clone());
                let new_b = resolve_numeric_name(b, current_idx, defs).unwrap_or_else(|| b.clone());
                DataValue::SymbolDiffAddend(new_a, new_b, *add)
            }
            _ => v.clone(),
        })
        .collect()
}
