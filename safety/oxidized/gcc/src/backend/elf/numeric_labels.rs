//! Numeric local label resolution for x86 and i686 assemblers.
//!
//! GNU assembler numeric labels (e.g., `1:`, `42:`) can be defined multiple
//! times. Forward references (`1f`) resolve to the NEXT definition, backward
//! references (`1b`) resolve to the MOST RECENT definition.
//!
//! This module provides the resolution logic shared by x86 and i686 ELF
//! writers. Both architectures use the same x86 parser AST types
//! (AsmItem, Operand, Displacement, DataValue, etc.), so these functions
//! operate on those types directly.
//!
//! ARM and RISC-V use different parser types and don't have this pattern
//! (ARM has no numeric labels; RISC-V has its own pre-pass).

use crate::backend::x86::assembler::parser::{
    AsmItem, DataValue, Displacement, ImmediateValue, Instruction, MemoryOperand, Operand,
};
use std::collections::HashMap;

/// Check if a string is a numeric local label (just digits, e.g., "1", "42").
pub fn is_numeric_label(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())
}

/// Check if a string is a numeric forward/backward reference like "1f" or "2b".
/// Returns Some((number_str, is_forward)) if it is, None otherwise.
pub fn parse_numeric_ref(name: &str) -> Option<(&str, bool)> {
    if name.len() < 2 {
        return None;
    }
    let last = name.as_bytes()[name.len() - 1];
    let num_part = &name[..name.len() - 1];
    if !num_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match last {
        b'f' => Some((num_part, true)),
        b'b' => Some((num_part, false)),
        _ => None,
    }
}

/// Resolve numeric local labels (1:, 2:, etc.) and their references (1f, 1b)
/// into unique internal label names.
///
/// GNU assembler numeric labels can be defined multiple times. Each forward
/// reference `Nf` refers to the next definition of `N`, and each backward
/// reference `Nb` refers to the most recent definition of `N`.
///
/// This function renames each definition to a unique `.Lnum_N_K` name and
/// updates all references accordingly. Used by both x86 and i686 ELF writers.
pub fn resolve_numeric_labels(items: &[AsmItem]) -> Vec<AsmItem> {
    // First pass: find all numeric label definitions and assign unique names.
    let mut defs: HashMap<String, Vec<(usize, String)>> = HashMap::new();
    let mut unique_counter: HashMap<String, usize> = HashMap::new();

    for (i, item) in items.iter().enumerate() {
        if let AsmItem::Label(name) = item {
            if is_numeric_label(name) {
                let count = unique_counter.entry(name.clone()).or_insert(0);
                let unique_name = format!(".Lnum_{}_{}", name, *count);
                *count += 1;
                defs.entry(name.clone()).or_default().push((i, unique_name));
            }
        }
    }

    if defs.is_empty() {
        return items.to_vec();
    }

    let mut result = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        match item {
            AsmItem::Label(name) if is_numeric_label(name) => {
                if let Some(def_list) = defs.get(name) {
                    if let Some((_, unique_name)) = def_list.iter().find(|(idx, _)| *idx == i) {
                        result.push(AsmItem::Label(unique_name.clone()));
                        continue;
                    }
                }
                result.push(item.clone());
            }
            AsmItem::Instruction(instr) => {
                let new_ops: Vec<Operand> = instr
                    .operands
                    .iter()
                    .map(|op| resolve_numeric_operand(op, i, &defs))
                    .collect();
                result.push(AsmItem::Instruction(Instruction {
                    prefix: instr.prefix.clone(),
                    mnemonic: instr.mnemonic.clone(),
                    operands: new_ops,
                }));
            }
            AsmItem::Short(vals) => {
                result.push(AsmItem::Short(resolve_numeric_data_values(vals, i, &defs)));
            }
            AsmItem::Long(vals) => {
                result.push(AsmItem::Long(resolve_numeric_data_values(vals, i, &defs)));
            }
            AsmItem::Quad(vals) => {
                result.push(AsmItem::Quad(resolve_numeric_data_values(vals, i, &defs)));
            }
            AsmItem::Byte(vals) => {
                result.push(AsmItem::Byte(resolve_numeric_data_values(vals, i, &defs)));
            }
            AsmItem::SkipExpr(expr, fill) => {
                let new_expr = resolve_numeric_refs_in_expr(expr, i, &defs);
                result.push(AsmItem::SkipExpr(new_expr, *fill));
            }
            AsmItem::Org(sym, offset) => {
                if let Some(resolved) = resolve_numeric_name(sym, i, &defs) {
                    result.push(AsmItem::Org(resolved, *offset));
                } else {
                    result.push(item.clone());
                }
            }
            _ => result.push(item.clone()),
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
        Operand::Label(name) => {
            if let Some(resolved) = resolve_numeric_name(name, current_idx, defs) {
                Operand::Label(resolved)
            } else {
                op.clone()
            }
        }
        Operand::Memory(mem) => {
            if let Some(new_disp) =
                resolve_numeric_displacement(&mem.displacement, current_idx, defs)
            {
                Operand::Memory(MemoryOperand {
                    segment: mem.segment.clone(),
                    displacement: new_disp,
                    base: mem.base.clone(),
                    index: mem.index.clone(),
                    scale: mem.scale,
                })
            } else {
                op.clone()
            }
        }
        Operand::Immediate(ImmediateValue::SymbolDiff(lhs, rhs)) => {
            let new_lhs =
                resolve_numeric_name(lhs, current_idx, defs).unwrap_or_else(|| lhs.clone());
            let new_rhs =
                resolve_numeric_name(rhs, current_idx, defs).unwrap_or_else(|| rhs.clone());
            Operand::Immediate(ImmediateValue::SymbolDiff(new_lhs, new_rhs))
        }
        Operand::Immediate(ImmediateValue::Symbol(name)) => {
            if let Some(resolved) = resolve_numeric_name(name, current_idx, defs) {
                Operand::Immediate(ImmediateValue::Symbol(resolved))
            } else {
                op.clone()
            }
        }
        Operand::Indirect(inner) => {
            let resolved_inner = resolve_numeric_operand(inner, current_idx, defs);
            Operand::Indirect(Box::new(resolved_inner))
        }
        _ => op.clone(),
    }
}

/// Resolve numeric label references in data values (.long, .quad, .byte directives).
fn resolve_numeric_data_values(
    vals: &[DataValue],
    current_idx: usize,
    defs: &HashMap<String, Vec<(usize, String)>>,
) -> Vec<DataValue> {
    vals.iter()
        .map(|val| match val {
            DataValue::Symbol(name) => {
                if let Some(resolved) = resolve_numeric_name(name, current_idx, defs) {
                    DataValue::Symbol(resolved)
                } else {
                    val.clone()
                }
            }
            DataValue::SymbolDiff(lhs, rhs) => {
                let new_lhs =
                    resolve_numeric_name(lhs, current_idx, defs).unwrap_or_else(|| lhs.clone());
                let new_rhs =
                    resolve_numeric_name(rhs, current_idx, defs).unwrap_or_else(|| rhs.clone());
                DataValue::SymbolDiff(new_lhs, new_rhs)
            }
            DataValue::SymbolOffset(name, offset) => {
                if let Some(resolved) = resolve_numeric_name(name, current_idx, defs) {
                    DataValue::SymbolOffset(resolved, *offset)
                } else {
                    val.clone()
                }
            }
            _ => val.clone(),
        })
        .collect()
}

/// Resolve a numeric label reference name (e.g., "1f" -> ".Lnum_1_0").
pub fn resolve_numeric_name(
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

/// Resolve numeric label references in a displacement.
fn resolve_numeric_displacement(
    disp: &Displacement,
    current_idx: usize,
    defs: &HashMap<String, Vec<(usize, String)>>,
) -> Option<Displacement> {
    match disp {
        Displacement::Symbol(name) => {
            resolve_numeric_name(name, current_idx, defs).map(Displacement::Symbol)
        }
        Displacement::SymbolAddend(name, addend) => resolve_numeric_name(name, current_idx, defs)
            .map(|n| Displacement::SymbolAddend(n, *addend)),
        Displacement::SymbolMod(name, modifier) => resolve_numeric_name(name, current_idx, defs)
            .map(|n| Displacement::SymbolMod(n, modifier.clone())),
        Displacement::SymbolPlusOffset(name, offset) => {
            resolve_numeric_name(name, current_idx, defs)
                .map(|n| Displacement::SymbolPlusOffset(n, *offset))
        }
        _ => None,
    }
}

/// Resolve numeric label references (e.g., "6651f", "661b") within an expression string.
/// Scans for patterns like digits followed by 'f' or 'b' and replaces them with unique names.
pub fn resolve_numeric_refs_in_expr(
    expr: &str,
    current_idx: usize,
    defs: &HashMap<String, Vec<(usize, String)>>,
) -> String {
    let mut result = String::with_capacity(expr.len());
    let bytes = expr.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'f' || bytes[i] == b'b') {
                let next = i + 1;
                if next >= bytes.len() || !bytes[next].is_ascii_alphanumeric() {
                    let ref_name = &expr[start..=i];
                    if let Some(resolved) = resolve_numeric_name(ref_name, current_idx, defs) {
                        result.push_str(&resolved);
                        i += 1;
                        continue;
                    }
                }
            }
            result.push_str(&expr[start..i]);
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}
