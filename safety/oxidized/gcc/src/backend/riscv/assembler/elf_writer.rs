//! ELF object file writer for RISC-V.
//!
//! Takes parsed assembly statements and produces an ELF .o (relocatable) file
//! with proper sections, symbols, and relocations for RISC-V ELF (32 or 64-bit).
//!
//! Uses `ElfWriterBase` from `elf.rs` for shared section/symbol/relocation
//! management, directive processing, and ELF serialization. This file only
//! contains RISC-V-specific logic: instruction encoding dispatch, pcrel_hi/lo
//! pairing, RV64C compression, GNU numeric labels, and branch resolution.

// ELF writer helpers; some section/relocation utilities defined for completeness.
#![allow(dead_code)]

use super::compress;
use super::encoder::{encode_insn_directive, encode_instruction, EncodeResult, RelocType};
use super::parser::{
    AsmStatement, DataValue, Directive, Operand, SizeExpr, SymbolType, Visibility,
};
use crate::backend::elf::{
    self, ElfWriterBase, ObjReloc, ELFCLASS64, EM_RISCV, SHF_ALLOC, SHF_EXECINSTR, SHF_WRITE,
    SHT_NOBITS, SHT_PROGBITS, STT_FUNC, STT_NOTYPE, STT_OBJECT, STT_TLS, STV_HIDDEN, STV_INTERNAL,
    STV_PROTECTED,
};
use std::collections::{HashMap, HashSet};

// ELF flags for RISC-V
pub(super) const EF_RISCV_RVC: u32 = 0x1;
const EF_RISCV_FLOAT_ABI_SOFT: u32 = 0x0;
pub(super) const EF_RISCV_FLOAT_ABI_SINGLE: u32 = 0x2;
pub(super) const EF_RISCV_FLOAT_ABI_DOUBLE: u32 = 0x4;
pub(super) const EF_RISCV_FLOAT_ABI_QUAD: u32 = 0x6;

/// RISC-V NOP instruction: `addi x0, x0, 0` = 0x00000013 in little-endian
const RISCV_NOP: [u8; 4] = [0x13, 0x00, 0x00, 0x00];

/// The ELF writer for RISC-V.
///
/// Composes with `ElfWriterBase` for shared infrastructure and adds
/// RISC-V-specific pcrel_hi/lo pairing, RV64C compression, GNU numeric
/// label resolution, and RISC-V branch/call relocation patching.
pub struct ElfWriter {
    /// Shared ELF writer state (sections, symbols, labels, directives)
    pub base: ElfWriterBase,
    /// Pending relocations that reference local labels (resolved after all labels are known)
    pending_branch_relocs: Vec<PendingReloc>,
    /// Counter for generating synthetic pcrel_hi labels
    pcrel_hi_counter: u32,
    /// GNU numeric labels: e.g., "1" -> [(section, offset), ...] in definition order.
    numeric_labels: HashMap<String, Vec<(String, u64)>>,
    /// Deferred data expressions that reference forward labels (resolved after all statements).
    deferred_exprs: Vec<DeferredExpr>,
    /// ELF e_flags to use (default: RVC + double-float ABI)
    elf_flags: u32,
    /// ELF class: ELFCLASS64 (default) or ELFCLASS32 for RV32 targets.
    elf_class: u8,
    /// When true, don't emit R_RISCV_RELAX relocations (set by `.option norelax`).
    no_relax: bool,
    /// Stack for `.option push`/`.option pop` to save/restore the `no_relax` state.
    option_stack: Vec<bool>,
}

/// A data expression that couldn't be evaluated immediately (e.g., forward label reference)
/// and must be resolved after all statements are processed.
struct DeferredExpr {
    section: String,
    offset: u64,
    size: usize,
    expr: String,
}

struct PendingReloc {
    section: String,
    offset: u64,
    reloc_type: u32,
    symbol: String,
    addend: i64,
    /// For pcrel_lo12 relocations resolved locally: the offset of the
    /// corresponding auipc (pcrel_hi) instruction.
    pcrel_hi_offset: Option<u64>,
}

/// Check if a label name is a GNU numeric label (e.g., "1", "42").
fn is_numeric_label(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_digit())
}

/// Check if a symbol reference is a GNU numeric label reference (e.g., "1b", "1f", "42b").
/// Returns Some((label_name, is_backward)) if it is, None otherwise.
fn parse_numeric_label_ref(symbol: &str) -> Option<(&str, bool)> {
    if symbol.len() < 2 {
        return None;
    }
    let last_char = symbol.as_bytes()[symbol.len() - 1];
    let is_backward = last_char == b'b' || last_char == b'B';
    let is_forward = last_char == b'f' || last_char == b'F';
    if !is_backward && !is_forward {
        return None;
    }
    let label_part = &symbol[..symbol.len() - 1];
    if label_part.is_empty() || !label_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((label_part, is_backward))
}

/// Pre-process assembly statements to resolve GNU numeric label references.
/// Numeric labels like `1:` can be defined multiple times. References like `1b`
/// (backward) and `1f` (forward) must resolve to the nearest matching definition.
fn resolve_numeric_label_refs(statements: &[AsmStatement]) -> Vec<AsmStatement> {
    // First pass: collect all numeric label definition positions
    let mut label_defs: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    let mut instance_counter: HashMap<String, usize> = HashMap::new();

    for (i, stmt) in statements.iter().enumerate() {
        if let AsmStatement::Label(name) = stmt {
            if is_numeric_label(name) {
                let instance = instance_counter.entry(name.clone()).or_insert(0);
                label_defs
                    .entry(name.clone())
                    .or_default()
                    .push((i, *instance));
                *instance += 1;
            }
        }
    }

    if label_defs.is_empty() {
        return statements.to_vec();
    }

    // Second pass: rewrite labels and references
    let mut result = Vec::with_capacity(statements.len());
    let mut dot_counter: usize = 0;

    for (i, stmt) in statements.iter().enumerate() {
        match stmt {
            AsmStatement::Label(name) if is_numeric_label(name) => {
                if let Some(defs) = label_defs.get(name) {
                    for &(def_idx, inst_id) in defs {
                        if def_idx == i {
                            let new_name = format!(".Lnum_{}_{}", name, inst_id);
                            result.push(AsmStatement::Label(new_name));
                            break;
                        }
                    }
                } else {
                    result.push(stmt.clone());
                }
            }
            AsmStatement::Instruction {
                mnemonic,
                operands,
                raw_operands,
            } => {
                let new_operands: Vec<Operand> = operands
                    .iter()
                    .map(|op| rewrite_numeric_ref_in_operand(op, i, &label_defs))
                    .collect();
                result.push(AsmStatement::Instruction {
                    mnemonic: mnemonic.clone(),
                    operands: new_operands,
                    raw_operands: raw_operands.clone(),
                });
            }
            AsmStatement::Directive(dir) => {
                let new_dir =
                    rewrite_numeric_refs_in_directive(dir, i, &label_defs, &mut dot_counter);
                // If the directive references '.', a synthetic label was inserted
                // before the directive. Check and handle.
                for d in new_dir {
                    result.push(d);
                }
            }
            _ => result.push(stmt.clone()),
        }
    }

    result
}

/// Rewrite a numeric label reference in an operand to a synthetic label name.
fn rewrite_numeric_ref_in_operand(
    op: &Operand,
    stmt_idx: usize,
    label_defs: &HashMap<String, Vec<(usize, usize)>>,
) -> Operand {
    match op {
        Operand::Symbol(s) => {
            if let Some(new_name) = resolve_numeric_ref_name(s, stmt_idx, label_defs) {
                Operand::Symbol(new_name)
            } else {
                op.clone()
            }
        }
        Operand::Label(s) => {
            if let Some(new_name) = resolve_numeric_ref_name(s, stmt_idx, label_defs) {
                Operand::Label(new_name)
            } else {
                op.clone()
            }
        }
        Operand::SymbolOffset(s, off) => {
            if let Some(new_name) = resolve_numeric_ref_name(s, stmt_idx, label_defs) {
                Operand::SymbolOffset(new_name, *off)
            } else {
                op.clone()
            }
        }
        _ => op.clone(),
    }
}

/// Resolve a numeric label reference like "1b" or "2f" to a synthetic label name.
fn resolve_numeric_ref_name(
    symbol: &str,
    stmt_idx: usize,
    label_defs: &HashMap<String, Vec<(usize, usize)>>,
) -> Option<String> {
    let (label_name, is_backward) = parse_numeric_label_ref(symbol)?;
    let defs = label_defs.get(label_name)?;

    if is_backward {
        let mut best: Option<usize> = None;
        for &(def_idx, inst_id) in defs {
            if def_idx < stmt_idx {
                best = Some(inst_id);
            }
        }
        best.map(|inst_id| format!(".Lnum_{}_{}", label_name, inst_id))
    } else {
        for &(def_idx, inst_id) in defs {
            if def_idx > stmt_idx {
                return Some(format!(".Lnum_{}_{}", label_name, inst_id));
            }
        }
        None
    }
}

/// Rewrite a symbol name that may be a numeric label ref or `.` (current position).
/// If it's `.`, a synthetic label is generated and `needs_dot_label` is set.
fn rewrite_symbol_name(
    name: &str,
    stmt_idx: usize,
    label_defs: &HashMap<String, Vec<(usize, usize)>>,
    dot_counter: &mut usize,
    needs_dot_label: &mut Option<String>,
) -> String {
    if name == "." {
        let label = format!(".Ldot_{}", *dot_counter);
        *dot_counter += 1;
        *needs_dot_label = Some(label.clone());
        label
    } else if let Some(resolved) = resolve_numeric_ref_name(name, stmt_idx, label_defs) {
        resolved
    } else {
        name.to_string()
    }
}

/// Decompose a symbol name that may contain an embedded addend.
///
/// For example, `"cgroup_bpf_enabled_key+144"` -> `("cgroup_bpf_enabled_key", 144)`.
/// If there is no embedded addend, returns `(name, 0)`.
///
/// This is needed because inline asm operand substitution can produce symbol
/// references like `sym+offset` as a single string, but ELF relocations must
/// reference the base symbol with a numeric addend in the RELA entry.
fn decompose_symbol_addend(name: &str) -> (String, i64) {
    // Split on the last `+` or `-` if the suffix is a plain integer.
    // Names without arithmetic (e.g. `.Ldot_2`, `my_func`) pass through as-is.
    if let Some(plus_pos) = name.rfind('+') {
        let base = &name[..plus_pos];
        let offset_str = name[plus_pos + 1..].trim();
        if !base.is_empty() && !offset_str.is_empty() {
            if let Ok(offset) = offset_str.parse::<i64>() {
                return (base.to_string(), offset);
            }
        }
    } else if let Some(minus_pos) = name.rfind('-') {
        // Only if it's not the first character (not a negative number)
        if minus_pos > 0 {
            let base = &name[..minus_pos];
            let offset_str = &name[minus_pos..]; // includes the '-'
            if !base.is_empty() {
                if let Ok(offset) = offset_str.parse::<i64>() {
                    return (base.to_string(), offset);
                }
            }
        }
    }
    (name.to_string(), 0)
}

/// Rewrite numeric label refs and `.` in a DataValue.
fn rewrite_data_value(
    dv: &DataValue,
    stmt_idx: usize,
    label_defs: &HashMap<String, Vec<(usize, usize)>>,
    dot_counter: &mut usize,
    dot_labels: &mut Vec<String>,
) -> DataValue {
    match dv {
        DataValue::SymbolDiff {
            sym_a,
            sym_b,
            addend,
        } => {
            let mut needs_dot = None;
            let new_a =
                rewrite_symbol_name(sym_a, stmt_idx, label_defs, dot_counter, &mut needs_dot);
            if let Some(l) = needs_dot.take() {
                dot_labels.push(l);
            }
            let new_b =
                rewrite_symbol_name(sym_b, stmt_idx, label_defs, dot_counter, &mut needs_dot);
            if let Some(l) = needs_dot.take() {
                dot_labels.push(l);
            }
            DataValue::SymbolDiff {
                sym_a: new_a,
                sym_b: new_b,
                addend: *addend,
            }
        }
        DataValue::Symbol { name, addend } => {
            let mut needs_dot = None;
            let new_name =
                rewrite_symbol_name(name, stmt_idx, label_defs, dot_counter, &mut needs_dot);
            if let Some(l) = needs_dot.take() {
                dot_labels.push(l);
            }
            DataValue::Symbol {
                name: new_name,
                addend: *addend,
            }
        }
        DataValue::Expression(expr) => {
            // Rewrite numeric refs and '.' in expression strings
            let resolved =
                rewrite_expr_numeric_refs(expr, stmt_idx, label_defs, dot_counter, dot_labels);
            DataValue::Expression(resolved)
        }
        other => other.clone(),
    }
}

/// Rewrite numeric label refs and '.' inside a raw expression string.
fn rewrite_expr_numeric_refs(
    expr: &str,
    stmt_idx: usize,
    label_defs: &HashMap<String, Vec<(usize, usize)>>,
    dot_counter: &mut usize,
    dot_labels: &mut Vec<String>,
) -> String {
    // Replace standalone '.' that represents current position
    // We need to be careful: '.' could appear in symbol names like '.Lfoo'
    // The pattern we're looking for is '.' surrounded by operators or parens
    let mut result = String::with_capacity(expr.len());
    let bytes = expr.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'.' {
            // Check if this is a standalone '.' (current position)
            let prev_is_sep = i == 0
                || matches!(
                    bytes[i - 1],
                    b' ' | b'('
                        | b')'
                        | b'+'
                        | b'-'
                        | b'*'
                        | b'/'
                        | b'|'
                        | b'&'
                        | b'^'
                        | b','
                        | b'~'
                );
            let next_is_sep = i + 1 >= len
                || matches!(
                    bytes[i + 1],
                    b' ' | b'('
                        | b')'
                        | b'+'
                        | b'-'
                        | b'*'
                        | b'/'
                        | b'|'
                        | b'&'
                        | b'^'
                        | b','
                        | b'~'
                );
            if prev_is_sep && next_is_sep {
                let label = format!(".Ldot_{}", *dot_counter);
                *dot_counter += 1;
                dot_labels.push(label.clone());
                result.push_str(&label);
                i += 1;
                continue;
            }
        }
        // Try to match a numeric label reference (digits followed by 'f' or 'b')
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < len && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < len
                && (bytes[i] == b'f' || bytes[i] == b'F' || bytes[i] == b'b' || bytes[i] == b'B')
            {
                let next_after = if i + 1 < len { bytes[i + 1] } else { b' ' };
                // Must not be followed by an alphanumeric (to avoid matching hex or identifiers)
                if !next_after.is_ascii_alphanumeric() && next_after != b'_' {
                    let ref_str = &expr[start..=i];
                    if let Some(resolved) = resolve_numeric_ref_name(ref_str, stmt_idx, label_defs)
                    {
                        result.push_str(&resolved);
                        i += 1;
                        continue;
                    }
                }
            }
            // Not a numeric ref, push digits as-is
            result.push_str(&expr[start..i]);
            continue;
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

/// Rewrite a list of DataValues, collecting any dot labels needed.
fn rewrite_data_values(
    values: &[DataValue],
    stmt_idx: usize,
    label_defs: &HashMap<String, Vec<(usize, usize)>>,
    dot_counter: &mut usize,
    dot_labels: &mut Vec<String>,
) -> Vec<DataValue> {
    values
        .iter()
        .map(|dv| rewrite_data_value(dv, stmt_idx, label_defs, dot_counter, dot_labels))
        .collect()
}

/// Rewrite numeric label references and '.' in a directive.
/// Returns a list of statements: possibly a synthetic label before the directive.
fn rewrite_numeric_refs_in_directive(
    dir: &Directive,
    stmt_idx: usize,
    label_defs: &HashMap<String, Vec<(usize, usize)>>,
    dot_counter: &mut usize,
) -> Vec<AsmStatement> {
    let mut dot_labels: Vec<String> = Vec::new();
    let new_dir = match dir {
        Directive::Byte(vals) => {
            let new_vals =
                rewrite_data_values(vals, stmt_idx, label_defs, dot_counter, &mut dot_labels);
            Directive::Byte(new_vals)
        }
        Directive::Short(vals) => {
            let new_vals =
                rewrite_data_values(vals, stmt_idx, label_defs, dot_counter, &mut dot_labels);
            Directive::Short(new_vals)
        }
        Directive::Long(vals) => {
            let new_vals =
                rewrite_data_values(vals, stmt_idx, label_defs, dot_counter, &mut dot_labels);
            Directive::Long(new_vals)
        }
        Directive::Quad(vals) => {
            let new_vals =
                rewrite_data_values(vals, stmt_idx, label_defs, dot_counter, &mut dot_labels);
            Directive::Quad(new_vals)
        }
        _ => dir.clone(),
    };
    let mut stmts = Vec::new();
    // Insert synthetic dot labels before the directive
    for label in dot_labels {
        stmts.push(AsmStatement::Label(label));
    }
    stmts.push(AsmStatement::Directive(new_dir));
    stmts
}

impl ElfWriter {
    pub fn new() -> Self {
        Self {
            base: ElfWriterBase::new(RISCV_NOP, 2),
            pending_branch_relocs: Vec::new(),
            pcrel_hi_counter: 0,
            numeric_labels: HashMap::new(),
            deferred_exprs: Vec::new(),
            elf_flags: EF_RISCV_FLOAT_ABI_DOUBLE | EF_RISCV_RVC,
            elf_class: ELFCLASS64,
            no_relax: false,
            option_stack: Vec::new(),
        }
    }

    /// Set the ELF e_flags (e.g., to change float ABI from the default double-float).
    pub fn set_elf_flags(&mut self, flags: u32) {
        self.elf_flags = flags;
    }

    /// Set the ELF class (ELFCLASS32 or ELFCLASS64).
    pub fn set_elf_class(&mut self, class: u8) {
        self.elf_class = class;
    }

    // R_RISCV_RELAX relocations are emitted alongside CALL_PLT, BRANCH, and
    // JAL relocations so the linker can perform relaxation (shortening
    // auipc+jalr to jal, etc.). This is required because linker relaxation
    // changes code layout, which would invalidate any locally-resolved offsets.

    /// R_RISCV_RELAX ELF relocation type
    const R_RISCV_RELAX: u32 = 51;

    /// R_RISCV_ALIGN ELF relocation type - marks alignment padding that the
    /// linker may need to adjust during relaxation.
    const R_RISCV_ALIGN: u32 = 43;

    /// Emit alignment padding with an R_RISCV_ALIGN relocation in executable
    /// sections (when relaxation is enabled). The linker needs these to know
    /// where alignment padding exists so it can re-align after relaxation
    /// changes code sizes.
    fn emit_align_with_reloc(&mut self, align_bytes: u64) {
        if align_bytes <= 1 {
            return;
        }
        let offset_before = self.base.current_offset();
        self.base.align_to(align_bytes);
        let offset_after = self.base.current_offset();
        let padding = offset_after - offset_before;
        if padding > 0 && !self.no_relax {
            // Only emit R_RISCV_ALIGN in executable sections where linker
            // relaxation may change code sizes and require re-alignment.
            if let Some(s) = self.base.sections.get_mut(&self.base.current_section) {
                if (s.sh_flags & SHF_EXECINSTR) != 0 {
                    s.relocs.push(ObjReloc {
                        offset: offset_before,
                        reloc_type: Self::R_RISCV_ALIGN,
                        symbol_name: String::new(),
                        addend: padding as i64,
                    });
                }
            }
        }
    }

    /// Process all parsed assembly statements.
    pub fn process_statements(&mut self, statements: &[AsmStatement]) -> Result<(), String> {
        let statements = resolve_numeric_label_refs(statements);
        for stmt in &statements {
            self.process_statement(stmt)?;
        }
        // Merge subsections (e.g., .text.__subsection.1 → .text) before resolving
        // relocations. This is critical for kernel ALTERNATIVE macros which use
        // .subsection 1 to place alternative code within the same section.
        let remap = self.base.merge_subsections();
        // Fix up pending references that pointed to now-merged subsection names.
        // Deferred expressions and branch relocs created inside a subsection store the
        // subsection name as their section; after merging, that section no longer exists.
        // Remap them to the parent section with the correct offset adjustment.
        if !remap.is_empty() {
            for reloc in &mut self.pending_branch_relocs {
                if let Some((parent, offset_adj)) = remap.get(&reloc.section) {
                    reloc.offset += offset_adj;
                    reloc.section = parent.clone();
                }
            }
            for expr in &mut self.deferred_exprs {
                if let Some((parent, offset_adj)) = remap.get(&expr.section) {
                    expr.offset += offset_adj;
                    expr.section = parent.clone();
                }
            }
        }
        self.resolve_deferred_exprs()?;
        // Compression is disabled: the linker handles relaxation via
        // R_RISCV_RELAX. Running our own compression would change code
        // layout in ways the linker's relaxation pass doesn't expect.
        // self.compress_executable_sections();
        self.resolve_local_branches()?;
        Ok(())
    }

    fn process_statement(&mut self, stmt: &AsmStatement) -> Result<(), String> {
        match stmt {
            AsmStatement::Empty => Ok(()),

            AsmStatement::Label(name) => {
                self.base.ensure_text_section();
                let section = self.base.current_section.clone();
                let offset = self.base.current_offset();
                self.base
                    .labels
                    .insert(name.clone(), (section.clone(), offset));
                if is_numeric_label(name) {
                    self.numeric_labels
                        .entry(name.clone())
                        .or_default()
                        .push((section, offset));
                }
                Ok(())
            }

            AsmStatement::Directive(directive) => self.process_directive(directive),

            AsmStatement::Instruction {
                mnemonic,
                operands,
                raw_operands,
            } => self.process_instruction(mnemonic, operands, raw_operands),
        }
    }

    fn process_directive(&mut self, directive: &Directive) -> Result<(), String> {
        match directive {
            Directive::PushSection(info) => {
                self.base.push_section(
                    &info.name,
                    &info.flags,
                    info.flags_explicit,
                    Some(info.sec_type.as_str()),
                );
                Ok(())
            }
            Directive::PopSection => {
                self.base.pop_section();
                Ok(())
            }
            Directive::Previous => {
                self.base.restore_previous_section();
                Ok(())
            }
            Directive::Section(info) => {
                self.base.process_section_directive(
                    &info.name,
                    &info.flags,
                    info.flags_explicit,
                    Some(info.sec_type.as_str()),
                );
                Ok(())
            }

            Directive::Text => {
                self.base.switch_to_standard_section(
                    ".text",
                    SHT_PROGBITS,
                    SHF_ALLOC | SHF_EXECINSTR,
                );
                Ok(())
            }
            Directive::Data => {
                self.base
                    .switch_to_standard_section(".data", SHT_PROGBITS, SHF_ALLOC | SHF_WRITE);
                Ok(())
            }
            Directive::Bss => {
                self.base
                    .switch_to_standard_section(".bss", SHT_NOBITS, SHF_ALLOC | SHF_WRITE);
                Ok(())
            }
            Directive::Rodata => {
                self.base
                    .switch_to_standard_section(".rodata", SHT_PROGBITS, SHF_ALLOC);
                Ok(())
            }

            Directive::Globl(sym) => {
                for s in sym.split(',') {
                    let s = s.trim();
                    if !s.is_empty() {
                        self.base.set_global(s);
                    }
                }
                Ok(())
            }
            Directive::Weak(sym) => {
                for s in sym.split(',') {
                    let s = s.trim();
                    if !s.is_empty() {
                        self.base.set_weak(s);
                    }
                }
                Ok(())
            }

            Directive::SymVisibility(sym, vis) => {
                let v = match vis {
                    Visibility::Hidden => STV_HIDDEN,
                    Visibility::Protected => STV_PROTECTED,
                    Visibility::Internal => STV_INTERNAL,
                };
                self.base.set_visibility(sym, v);
                Ok(())
            }

            Directive::Type(sym, st) => {
                let elf_type = match st {
                    SymbolType::Function => STT_FUNC,
                    SymbolType::Object => STT_OBJECT,
                    SymbolType::TlsObject => STT_TLS,
                    SymbolType::NoType => STT_NOTYPE,
                };
                self.base.set_symbol_type(sym, elf_type);
                Ok(())
            }

            Directive::Size(sym, size_expr) => {
                match size_expr {
                    SizeExpr::CurrentMinus(label) => {
                        self.base.set_symbol_size(sym, Some(label), None);
                    }
                    SizeExpr::Absolute(size) => {
                        self.base.set_symbol_size(sym, None, Some(*size));
                    }
                }
                Ok(())
            }

            Directive::Align(val) => {
                // RISC-V .align N means 2^N bytes (same as .p2align)
                let bytes = 1u64 << val;
                self.emit_align_with_reloc(bytes);
                Ok(())
            }

            Directive::Balign(val) => {
                self.emit_align_with_reloc(*val);
                Ok(())
            }

            Directive::Byte(values) => {
                for dv in values {
                    match dv {
                        DataValue::Integer(v) => self.base.emit_bytes(&[*v as u8]),
                        DataValue::Symbol { name, addend } => {
                            // Try resolving as alias (.set/.equ) — may be a forward ref
                            let resolved = self.base.resolve_expr_aliases(name);
                            let deferred_expr = if *addend != 0 {
                                format!("({}) + ({})", name, addend)
                            } else {
                                name.clone()
                            };
                            let eval_expr = if *addend != 0 {
                                format!("({}) + ({})", resolved, addend)
                            } else {
                                resolved.clone()
                            };
                            if let Ok(v) = crate::backend::asm_expr::parse_integer_expr(&eval_expr)
                            {
                                self.base.emit_bytes(&[v as u8]);
                            } else {
                                // Defer: alias not yet defined (forward reference)
                                let section = self.base.current_section.clone();
                                let offset = self.base.current_offset();
                                self.deferred_exprs.push(DeferredExpr {
                                    section,
                                    offset,
                                    size: 1,
                                    expr: deferred_expr,
                                });
                                self.base.emit_placeholder(1);
                            }
                        }
                        _ => self.base.emit_bytes(&[0u8]),
                    }
                }
                Ok(())
            }

            Directive::Short(values) => {
                for dv in values {
                    match dv {
                        DataValue::Integer(v) => self.base.emit_bytes(&(*v as u16).to_le_bytes()),
                        DataValue::Expression(expr) => {
                            let resolved = self.base.resolve_expr_aliases(expr);
                            let resolved = self.base.resolve_expr_labels(&resolved);
                            match crate::backend::asm_expr::parse_integer_expr(&resolved) {
                                Ok(v) => self.base.emit_bytes(&(v as u16).to_le_bytes()),
                                Err(_) => {
                                    // Defer: expression contains forward references
                                    let section = self.base.current_section.clone();
                                    let offset = self.base.current_offset();
                                    self.deferred_exprs.push(DeferredExpr {
                                        section,
                                        offset,
                                        size: 2,
                                        expr: expr.clone(),
                                    });
                                    self.base.emit_placeholder(2);
                                }
                            }
                        }
                        DataValue::Symbol { name, addend } => {
                            // Try resolving as alias (.set/.equ) — may be a forward ref
                            let resolved = self.base.resolve_expr_aliases(name);
                            let deferred_expr = if *addend != 0 {
                                format!("({}) + ({})", name, addend)
                            } else {
                                name.clone()
                            };
                            let eval_expr = if *addend != 0 {
                                format!("({}) + ({})", resolved, addend)
                            } else {
                                resolved.clone()
                            };
                            if let Ok(v) = crate::backend::asm_expr::parse_integer_expr(&eval_expr)
                            {
                                self.base.emit_bytes(&(v as u16).to_le_bytes());
                            } else {
                                // Defer: alias not yet defined (forward reference)
                                let section = self.base.current_section.clone();
                                let offset = self.base.current_offset();
                                self.deferred_exprs.push(DeferredExpr {
                                    section,
                                    offset,
                                    size: 2,
                                    expr: deferred_expr,
                                });
                                self.base.emit_placeholder(2);
                            }
                        }
                        DataValue::SymbolDiff {
                            sym_a,
                            sym_b,
                            addend,
                        } => {
                            let add_type = RelocType::Add16.elf_type();
                            let sub_type = RelocType::Sub16.elf_type();
                            let (base_a, extra_a) = decompose_symbol_addend(sym_a);
                            let (base_b, extra_b) = decompose_symbol_addend(sym_b);
                            self.base.add_reloc(add_type, base_a, *addend + extra_a);
                            self.base.add_reloc(sub_type, base_b, extra_b);
                            self.base.emit_placeholder(2);
                        }
                    }
                }
                Ok(())
            }

            Directive::Long(values) => {
                for dv in values {
                    self.emit_data_value(dv, 4)?;
                }
                Ok(())
            }

            Directive::Quad(values) => {
                for dv in values {
                    self.emit_data_value(dv, 8)?;
                }
                Ok(())
            }

            Directive::Zero { size, fill } => {
                self.base.emit_bytes(&vec![*fill; *size]);
                Ok(())
            }

            Directive::Asciz(s) => {
                self.base.emit_bytes(s);
                self.base.emit_bytes(&[0]);
                Ok(())
            }

            Directive::Ascii(s) => {
                self.base.emit_bytes(s);
                Ok(())
            }

            Directive::Comm { sym, size, align } => {
                self.base.emit_comm(sym, *size, *align);
                Ok(())
            }

            Directive::Local(_) => Ok(()),

            Directive::Set(alias, target) => {
                // If the target expression contains '.', resolve it as
                // current offset, then try to evaluate to a constant.
                let mut resolved = self.base.resolve_expr_aliases(target);
                if resolved.contains('.') {
                    resolved = self.base.resolve_expr_labels(&resolved);
                }
                // Try to evaluate to a constant; if so, store the constant
                if let Ok(v) = crate::backend::asm_expr::parse_integer_expr(&resolved) {
                    self.base.set_alias(alias, &v.to_string());
                } else {
                    self.base.set_alias(alias, &resolved);
                }
                Ok(())
            }

            Directive::ArchOption(opt) => {
                let opt = opt.trim();
                if opt == "norelax" {
                    self.no_relax = true;
                } else if opt == "relax" {
                    self.no_relax = false;
                } else if opt == "push" {
                    self.option_stack.push(self.no_relax);
                } else if opt == "pop" {
                    if let Some(saved) = self.option_stack.pop() {
                        self.no_relax = saved;
                    }
                }
                // rvc/norvc are silently accepted (compression not yet supported)
                Ok(())
            }

            Directive::Attribute(_) => Ok(()),

            Directive::Cfi | Directive::Ignored => Ok(()),

            Directive::Insn(args) => {
                self.base.ensure_text_section();
                match encode_insn_directive(args) {
                    Ok(EncodeResult::Word(word)) => {
                        self.base.emit_u32_le(word);
                        Ok(())
                    }
                    Ok(EncodeResult::Half(half)) => {
                        self.base.emit_u16_le(half);
                        Ok(())
                    }
                    Ok(_) => Ok(()),
                    Err(e) => Err(e),
                }
            }

            Directive::Incbin { path, skip, count } => {
                let data = std::fs::read(path)
                    .map_err(|e| format!(".incbin: failed to read '{}': {}", path, e))?;
                let skip = *skip as usize;
                let data = if skip < data.len() {
                    &data[skip..]
                } else {
                    &[]
                };
                let data = match count {
                    Some(c) => {
                        let c = *c as usize;
                        if c < data.len() {
                            &data[..c]
                        } else {
                            data
                        }
                    }
                    None => data,
                };
                self.base.emit_bytes(data);
                Ok(())
            }

            Directive::Subsection(n) => {
                self.base.set_subsection(*n);
                Ok(())
            }

            Directive::Unknown { name, args } => Err(format!(
                "unsupported RISC-V assembler directive: {} {}",
                name, args
            )),
        }
    }

    /// Emit a typed data value for .long (size=4) or .quad (size=8).
    ///
    /// Note: `SymbolDiff` sym_a/sym_b may contain embedded addends (e.g.
    /// `"cgroup_bpf_enabled_key+144"`) from inline asm operand substitution.
    /// These are decomposed via [`decompose_symbol_addend`] so the ELF
    /// relocation references the base symbol with a proper addend.
    fn emit_data_value(&mut self, dv: &DataValue, size: usize) -> Result<(), String> {
        match dv {
            DataValue::SymbolDiff {
                sym_a,
                sym_b,
                addend,
            } => {
                let (add_type, sub_type) = if size == 4 {
                    (RelocType::Add32.elf_type(), RelocType::Sub32.elf_type())
                } else {
                    (RelocType::Add64.elf_type(), RelocType::Sub64.elf_type())
                };
                // Decompose sym_a if it contains an embedded addend (e.g.
                // "cgroup_bpf_enabled_key+144") so the relocation references
                // the base symbol with a numeric addend rather than creating a
                // bogus symbol named "symbol+offset".
                let (base_a, extra_a) = decompose_symbol_addend(sym_a);
                let (base_b, extra_b) = decompose_symbol_addend(sym_b);
                self.base.add_reloc(add_type, base_a, *addend + extra_a);
                self.base.add_reloc(sub_type, base_b, extra_b);
                self.base.emit_placeholder(size);
            }
            DataValue::Symbol { name, addend } => {
                // Try resolving as alias (.set/.equ) first — the "symbol"
                // may actually be a compile-time constant defined via .set.
                let resolved = self.base.resolve_expr_aliases(name);
                if let Ok(v) = crate::backend::asm_expr::parse_integer_expr(&resolved) {
                    self.base.emit_data_integer(v + addend, size);
                } else {
                    let reloc_type = if size == 4 {
                        RelocType::Abs32.elf_type()
                    } else {
                        RelocType::Abs64.elf_type()
                    };
                    self.base
                        .emit_data_symbol_ref(name, *addend, size, reloc_type);
                }
            }
            DataValue::Integer(v) => {
                self.base.emit_data_integer(*v, size);
            }
            DataValue::Expression(expr) => {
                let mut resolved = self.base.resolve_expr_aliases(expr);
                // Resolve .Ldot_N synthetic labels to current offset
                resolved = self.base.resolve_expr_labels(&resolved);
                match crate::backend::asm_expr::parse_integer_expr(&resolved) {
                    Ok(v) => self.base.emit_data_integer(v, size),
                    Err(_) => {
                        // Expression contains unresolved symbols (e.g., forward references).
                        // Defer resolution until all labels are known.
                        let section = self.base.current_section.clone();
                        let offset = self.base.current_offset();
                        self.deferred_exprs.push(DeferredExpr {
                            section,
                            offset,
                            size,
                            expr: expr.clone(),
                        });
                        // Emit placeholder bytes that will be patched later
                        self.base.emit_placeholder(size);
                    }
                }
            }
        }
        Ok(())
    }

    /// Resolve deferred data expressions now that all labels are known.
    fn resolve_deferred_exprs(&mut self) -> Result<(), String> {
        let deferred = std::mem::take(&mut self.deferred_exprs);
        for def in &deferred {
            // Re-resolve with all labels now available (using stored label offsets)
            let resolved = self.base.resolve_expr_aliases(&def.expr);
            let resolved = self.base.resolve_expr_all_labels(&resolved, &def.section);
            let value = match crate::backend::asm_expr::parse_integer_expr(&resolved) {
                Ok(v) => v,
                Err(_) => {
                    // Cross-section label reference: try resolving labels from
                    // ANY section. This handles kernel ALTERNATIVE macros where
                    // .2byte expressions in .alternative reference labels placed
                    // in .text (or a subsection merged into .text).
                    let cross_resolved = self.base.resolve_expr_cross_section(&def.expr);
                    // TODO: emit a warning on Err instead of silently producing 0.
                    // This fallback handles cases where macro argument splitting
                    // produces single-symbol expressions (e.g., ".Lnum_889_0"
                    // from "889f - 888f" being split by whitespace in macro args).
                    // Proper fix: make split_macro_args respect parameter count.
                    crate::backend::asm_expr::parse_integer_expr(&cross_resolved)
                        .unwrap_or_default()
                }
            };
            if let Some(section) = self.base.sections.get_mut(&def.section) {
                let off = def.offset as usize;
                if def.size == 4 && off + 4 <= section.data.len() {
                    section.data[off..off + 4].copy_from_slice(&(value as u32).to_le_bytes());
                } else if def.size == 8 && off + 8 <= section.data.len() {
                    section.data[off..off + 8].copy_from_slice(&value.to_le_bytes());
                } else if def.size == 2 && off + 2 <= section.data.len() {
                    section.data[off..off + 2].copy_from_slice(&(value as u16).to_le_bytes());
                } else if def.size == 1 && off < section.data.len() {
                    section.data[off] = value as u8;
                }
            }
        }
        Ok(())
    }

    fn process_instruction(
        &mut self,
        mnemonic: &str,
        operands: &[Operand],
        raw_operands: &str,
    ) -> Result<(), String> {
        self.base.ensure_text_section();

        match encode_instruction(mnemonic, operands, raw_operands) {
            Ok(EncodeResult::Word(word)) => {
                self.base.emit_u32_le(word);
                Ok(())
            }
            Ok(EncodeResult::Half(half)) => {
                self.base.emit_u16_le(half);
                Ok(())
            }
            Ok(EncodeResult::WordWithReloc { word, reloc }) => {
                let elf_type = reloc.reloc_type.elf_type();
                let is_pcrel_hi =
                    elf_type == 23 || elf_type == 20 || elf_type == 22 || elf_type == 21;

                if is_pcrel_hi {
                    let label = format!(".Lpcrel_hi{}", self.pcrel_hi_counter);
                    self.pcrel_hi_counter += 1;
                    let section = self.base.current_section.clone();
                    let offset = self.base.current_offset();
                    self.base.labels.insert(label, (section, offset));
                }

                // For BRANCH (16), JAL (17): always emit as external relocations
                // so the linker resolves offsets correctly after relaxation.
                // For CALL_PLT (19): emit with paired R_RISCV_RELAX.
                let is_branch_or_jal = elf_type == 16 || elf_type == 17;
                let is_call_plt = elf_type == 19;

                if is_call_plt {
                    self.base
                        .add_reloc(elf_type, reloc.symbol.clone(), reloc.addend);
                    if !self.no_relax {
                        self.base.add_reloc(Self::R_RISCV_RELAX, String::new(), 0);
                    }
                    self.base.emit_u32_le(word);
                } else if is_branch_or_jal {
                    self.base
                        .add_reloc(elf_type, reloc.symbol.clone(), reloc.addend);
                    self.base.emit_u32_le(word);
                } else {
                    let is_local = reloc.symbol.starts_with(".L")
                        || reloc.symbol.starts_with(".l")
                        || parse_numeric_label_ref(&reloc.symbol).is_some();

                    if is_local {
                        let offset = self.base.current_offset();
                        self.pending_branch_relocs.push(PendingReloc {
                            section: self.base.current_section.clone(),
                            offset,
                            reloc_type: elf_type,
                            symbol: reloc.symbol.clone(),
                            addend: reloc.addend,
                            pcrel_hi_offset: None,
                        });
                        self.base.emit_u32_le(word);
                    } else {
                        self.base
                            .add_reloc(elf_type, reloc.symbol.clone(), reloc.addend);
                        self.base.emit_u32_le(word);
                    }
                }
                Ok(())
            }
            Ok(EncodeResult::Words(words)) => {
                for word in words {
                    self.base.emit_u32_le(word);
                }
                Ok(())
            }
            Ok(EncodeResult::WordsWithRelocs(items)) => {
                let mut pcrel_hi_label: Option<String> = None;

                for (word, reloc_opt) in &items {
                    if let Some(reloc) = reloc_opt {
                        let elf_type = reloc.reloc_type.elf_type();
                        let is_pcrel_hi = elf_type == 23;
                        let is_got_hi = elf_type == 20;
                        let is_tls_gd_hi = elf_type == 22;
                        let is_tls_got_hi = elf_type == 21;

                        if is_pcrel_hi || is_got_hi || is_tls_gd_hi || is_tls_got_hi {
                            let label = format!(".Lpcrel_hi{}", self.pcrel_hi_counter);
                            self.pcrel_hi_counter += 1;
                            let section = self.base.current_section.clone();
                            let offset = self.base.current_offset();
                            self.base.labels.insert(label.clone(), (section, offset));
                            pcrel_hi_label = Some(label);

                            self.base
                                .add_reloc(elf_type, reloc.symbol.clone(), reloc.addend);
                            if !self.no_relax {
                                self.base.add_reloc(Self::R_RISCV_RELAX, String::new(), 0);
                            }
                            self.base.emit_u32_le(*word);
                            continue;
                        }

                        let is_pcrel_lo12_i = elf_type == 24;
                        let is_pcrel_lo12_s = elf_type == 25;

                        if let Some(hi_label) = pcrel_hi_label
                            .as_ref()
                            .filter(|_| is_pcrel_lo12_i || is_pcrel_lo12_s)
                        {
                            let hi_label = hi_label.clone();
                            self.base.add_reloc(elf_type, hi_label, 0);
                            if !self.no_relax {
                                self.base.add_reloc(Self::R_RISCV_RELAX, String::new(), 0);
                            }
                            self.base.emit_u32_le(*word);
                            continue;
                        }

                        // For BRANCH (16), JAL (17): always emit as external
                        // relocations. For CALL_PLT (19): emit with R_RISCV_RELAX.
                        let is_branch_or_jal = elf_type == 16 || elf_type == 17;
                        let is_call_plt = elf_type == 19;
                        if is_call_plt {
                            self.base
                                .add_reloc(elf_type, reloc.symbol.clone(), reloc.addend);
                            if !self.no_relax {
                                self.base.add_reloc(Self::R_RISCV_RELAX, String::new(), 0);
                            }
                        } else if is_branch_or_jal {
                            self.base
                                .add_reloc(elf_type, reloc.symbol.clone(), reloc.addend);
                        } else {
                            let is_local = reloc.symbol.starts_with(".L")
                                || reloc.symbol.starts_with(".l")
                                || parse_numeric_label_ref(&reloc.symbol).is_some();
                            if is_local {
                                let offset = self.base.current_offset();
                                self.pending_branch_relocs.push(PendingReloc {
                                    section: self.base.current_section.clone(),
                                    offset,
                                    reloc_type: elf_type,
                                    symbol: reloc.symbol.clone(),
                                    addend: reloc.addend,
                                    pcrel_hi_offset: None,
                                });
                            } else {
                                self.base
                                    .add_reloc(elf_type, reloc.symbol.clone(), reloc.addend);
                            }
                        }
                    }
                    self.base.emit_u32_le(*word);
                }
                Ok(())
            }
            Ok(EncodeResult::Skip) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Compress eligible 32-bit instructions in executable sections to 16-bit
    /// RV64C equivalents.
    fn compress_executable_sections(&mut self) {
        let exec_sections: Vec<String> = self
            .base
            .sections
            .iter()
            .filter(|(_, s)| (s.sh_flags & SHF_EXECINSTR) != 0)
            .map(|(name, _)| name.clone())
            .collect();

        for sec_name in &exec_sections {
            let mut reloc_offsets = HashSet::new();

            for pr in &self.pending_branch_relocs {
                if pr.section == *sec_name {
                    reloc_offsets.insert(pr.offset);
                    if pr.reloc_type == 19 {
                        reloc_offsets.insert(pr.offset + 4);
                    }
                }
            }

            if let Some(section) = self.base.sections.get(sec_name) {
                for r in &section.relocs {
                    reloc_offsets.insert(r.offset);
                    if r.reloc_type == 19 {
                        reloc_offsets.insert(r.offset + 4);
                    }
                }
            }

            let section_data = match self.base.sections.get(sec_name) {
                Some(s) => s.data.clone(),
                None => continue,
            };

            let (new_data, offset_map) = compress::compress_section(&section_data, &reloc_offsets);

            if new_data.len() == section_data.len() {
                continue;
            }

            if let Some(section) = self.base.sections.get_mut(sec_name) {
                section.data = new_data;
                for r in &mut section.relocs {
                    r.offset = compress::remap_offset(r.offset, &offset_map);
                }
            }

            for pr in &mut self.pending_branch_relocs {
                if pr.section == *sec_name {
                    pr.offset = compress::remap_offset(pr.offset, &offset_map);
                }
            }

            for (_, (label_sec, label_offset)) in self.base.labels.iter_mut() {
                if label_sec == sec_name {
                    *label_offset = compress::remap_offset(*label_offset, &offset_map);
                }
            }

            for (_, defs) in self.numeric_labels.iter_mut() {
                for (def_sec, def_offset) in defs.iter_mut() {
                    if def_sec == sec_name {
                        *def_offset = compress::remap_offset(*def_offset, &offset_map);
                    }
                }
            }

            for sym in &mut self.base.extra_symbols {
                if sym.section_name == *sec_name {
                    sym.value = compress::remap_offset(sym.value, &offset_map);
                }
            }
        }
    }

    /// Resolve a numeric label reference like "1b" or "1f" to a (section, offset).
    fn resolve_numeric_label_ref(
        &self,
        label_name: &str,
        is_backward: bool,
        ref_section: &str,
        ref_offset: u64,
    ) -> Option<(String, u64)> {
        let defs = self.numeric_labels.get(label_name)?;
        if is_backward {
            let mut best: Option<&(String, u64)> = None;
            for def in defs {
                if def.0 == ref_section && def.1 <= ref_offset {
                    best = Some(def);
                }
            }
            best.cloned()
        } else {
            for def in defs {
                if def.0 == ref_section && def.1 > ref_offset {
                    return Some(def.clone());
                }
            }
            None
        }
    }

    /// Resolve local branch labels to PC-relative offsets using RISC-V relocation types.
    fn resolve_local_branches(&mut self) -> Result<(), String> {
        for reloc in &self.pending_branch_relocs {
            // pcrel_lo12 relocations must always be emitted as external relocations
            // so the linker can pair them with their corresponding pcrel_hi20.
            let is_pcrel_lo = reloc.reloc_type == 24 || reloc.reloc_type == 25;
            if is_pcrel_lo {
                if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                    section.relocs.push(ObjReloc {
                        offset: reloc.offset,
                        reloc_type: reloc.reloc_type,
                        symbol_name: reloc.symbol.clone(),
                        addend: reloc.addend,
                    });
                }
                continue;
            }

            let resolved =
                if let Some((label_name, is_backward)) = parse_numeric_label_ref(&reloc.symbol) {
                    self.resolve_numeric_label_ref(
                        label_name,
                        is_backward,
                        &reloc.section,
                        reloc.offset,
                    )
                } else {
                    self.base.labels.get(&reloc.symbol).cloned()
                };

            let (target_section, target_offset) = match resolved {
                Some(v) => v,
                None => {
                    if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                        section.relocs.push(ObjReloc {
                            offset: reloc.offset,
                            reloc_type: reloc.reloc_type,
                            symbol_name: reloc.symbol.clone(),
                            addend: reloc.addend,
                        });
                    }
                    continue;
                }
            };

            if target_section != reloc.section {
                if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                    section.relocs.push(ObjReloc {
                        offset: reloc.offset,
                        reloc_type: reloc.reloc_type,
                        symbol_name: reloc.symbol.clone(),
                        addend: reloc.addend,
                    });
                }
                continue;
            }

            let ref_offset = reloc.pcrel_hi_offset.unwrap_or(reloc.offset);
            let pc_offset = (target_offset as i64) - (ref_offset as i64) + reloc.addend;

            if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                let instr_offset = reloc.offset as usize;

                match reloc.reloc_type {
                    16 => {
                        // R_RISCV_BRANCH (B-type, 12-bit)
                        if instr_offset + 4 > section.data.len() {
                            continue;
                        }
                        let mut word = u32::from_le_bytes([
                            section.data[instr_offset],
                            section.data[instr_offset + 1],
                            section.data[instr_offset + 2],
                            section.data[instr_offset + 3],
                        ]);
                        let imm = pc_offset as u32;
                        let bit12 = (imm >> 12) & 1;
                        let bit11 = (imm >> 11) & 1;
                        let bits10_5 = (imm >> 5) & 0x3F;
                        let bits4_1 = (imm >> 1) & 0xF;
                        word &= 0x01FFF07F;
                        word |= (bit12 << 31) | (bits10_5 << 25) | (bits4_1 << 8) | (bit11 << 7);
                        section.data[instr_offset..instr_offset + 4]
                            .copy_from_slice(&word.to_le_bytes());
                    }
                    17 => {
                        // R_RISCV_JAL (J-type, 20-bit)
                        if instr_offset + 4 > section.data.len() {
                            continue;
                        }
                        let mut word = u32::from_le_bytes([
                            section.data[instr_offset],
                            section.data[instr_offset + 1],
                            section.data[instr_offset + 2],
                            section.data[instr_offset + 3],
                        ]);
                        let imm = pc_offset as u32;
                        let bit20 = (imm >> 20) & 1;
                        let bits10_1 = (imm >> 1) & 0x3FF;
                        let bit11 = (imm >> 11) & 1;
                        let bits19_12 = (imm >> 12) & 0xFF;
                        word &= 0x00000FFF;
                        word |=
                            (bit20 << 31) | (bits10_1 << 21) | (bit11 << 20) | (bits19_12 << 12);
                        section.data[instr_offset..instr_offset + 4]
                            .copy_from_slice(&word.to_le_bytes());
                    }
                    19 => {
                        // R_RISCV_CALL_PLT (AUIPC + JALR pair, 8 bytes)
                        if instr_offset + 8 > section.data.len() {
                            continue;
                        }

                        let hi = ((pc_offset as i32 + 0x800) >> 12) as u32;
                        let lo = ((pc_offset as i32) << 20 >> 20) as u32;

                        let mut auipc = u32::from_le_bytes([
                            section.data[instr_offset],
                            section.data[instr_offset + 1],
                            section.data[instr_offset + 2],
                            section.data[instr_offset + 3],
                        ]);
                        auipc = (auipc & 0xFFF) | (hi << 12);
                        section.data[instr_offset..instr_offset + 4]
                            .copy_from_slice(&auipc.to_le_bytes());

                        let mut jalr = u32::from_le_bytes([
                            section.data[instr_offset + 4],
                            section.data[instr_offset + 5],
                            section.data[instr_offset + 6],
                            section.data[instr_offset + 7],
                        ]);
                        jalr = (jalr & 0xFFFFF) | ((lo & 0xFFF) << 20);
                        section.data[instr_offset + 4..instr_offset + 8]
                            .copy_from_slice(&jalr.to_le_bytes());
                    }
                    23 => {
                        // R_RISCV_PCREL_HI20 (AUIPC hi20)
                        if instr_offset + 4 > section.data.len() {
                            continue;
                        }
                        let hi = ((pc_offset as i32 + 0x800) >> 12) as u32;
                        let mut word = u32::from_le_bytes([
                            section.data[instr_offset],
                            section.data[instr_offset + 1],
                            section.data[instr_offset + 2],
                            section.data[instr_offset + 3],
                        ]);
                        word = (word & 0xFFF) | (hi << 12);
                        section.data[instr_offset..instr_offset + 4]
                            .copy_from_slice(&word.to_le_bytes());
                    }
                    24 => {
                        // R_RISCV_PCREL_LO12_I (ADDI/LD lo12 I-type)
                        if instr_offset + 4 > section.data.len() {
                            continue;
                        }
                        let lo = (pc_offset as i32) & 0xFFF;
                        let mut word = u32::from_le_bytes([
                            section.data[instr_offset],
                            section.data[instr_offset + 1],
                            section.data[instr_offset + 2],
                            section.data[instr_offset + 3],
                        ]);
                        word = (word & 0xFFFFF) | (((lo as u32) & 0xFFF) << 20);
                        section.data[instr_offset..instr_offset + 4]
                            .copy_from_slice(&word.to_le_bytes());
                    }
                    25 => {
                        // R_RISCV_PCREL_LO12_S (SW/SD lo12 S-type)
                        if instr_offset + 4 > section.data.len() {
                            continue;
                        }
                        let lo = (pc_offset as i32) & 0xFFF;
                        let mut word = u32::from_le_bytes([
                            section.data[instr_offset],
                            section.data[instr_offset + 1],
                            section.data[instr_offset + 2],
                            section.data[instr_offset + 3],
                        ]);
                        let imm_lo = (lo as u32) & 0x1F;
                        let imm_hi = ((lo as u32) >> 5) & 0x7F;
                        word &= 0x01FFF07F;
                        word |= (imm_hi << 25) | (imm_lo << 7);
                        section.data[instr_offset..instr_offset + 4]
                            .copy_from_slice(&word.to_le_bytes());
                    }
                    _ => {
                        section.relocs.push(ObjReloc {
                            offset: reloc.offset,
                            reloc_type: reloc.reloc_type,
                            symbol_name: reloc.symbol.clone(),
                            addend: reloc.addend,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Write the final ELF object file.
    pub fn write_elf(&mut self, output_path: &str) -> Result<(), String> {
        let config = elf::ElfConfig {
            e_machine: EM_RISCV,
            e_flags: self.elf_flags,
            elf_class: self.elf_class,
            // RISC-V always uses RELA relocations, even in 32-bit mode
            force_rela: true,
        };
        // RISC-V needs include_referenced_locals=true for pcrel_hi synthetic labels
        self.base.write_elf(output_path, &config, true)
    }
}
