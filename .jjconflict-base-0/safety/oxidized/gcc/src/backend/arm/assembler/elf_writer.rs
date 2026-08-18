//! ELF object file writer for AArch64.
//!
//! Takes parsed assembly statements and produces an ELF .o (relocatable) file
//! with proper sections, symbols, and relocations for AArch64/ELF64.
//!
//! Uses `ElfWriterBase` from `elf.rs` for shared section/symbol/relocation
//! management, directive processing, and ELF serialization. This file only
//! contains AArch64-specific logic: instruction encoding dispatch, branch
//! resolution (AArch64 relocation types), and symbol difference resolution.

// ELF writer helpers; some section/relocation utilities defined for completeness.
#![allow(dead_code)]

use super::encoder::{encode_instruction, EncodeResult, RelocType};
use super::parser::{AsmDirective, AsmStatement, DataValue, Operand, SizeExpr, SymbolKind};
use crate::backend::elf::{
    self, ElfWriterBase, ObjReloc, ELFCLASS64, EM_AARCH64, STT_FUNC, STT_NOTYPE, STT_OBJECT,
    STT_TLS, STV_HIDDEN, STV_INTERNAL, STV_PROTECTED,
};

/// AArch64 NOP instruction: `d503201f` in little-endian
const AARCH64_NOP: [u8; 4] = [0x1f, 0x20, 0x03, 0xd5];

/// The ELF writer for AArch64.
///
/// Composes with `ElfWriterBase` for shared infrastructure and adds
/// AArch64-specific branch resolution and symbol difference handling.
pub struct ElfWriter {
    /// Shared ELF writer state (sections, symbols, labels, directives)
    pub base: ElfWriterBase,
    /// Pending branch/local relocations to resolve after all labels are known.
    /// Includes all branch-type relocs (B, BL, B.cond, CBZ, TBZ, etc.) plus
    /// local .L-prefixed label references, so intra-section targets can be
    /// resolved at assembly time without emitting unnecessary relocations.
    pending_branch_relocs: Vec<PendingReloc>,
    /// Pending symbol differences to resolve after all labels are known
    pending_sym_diffs: Vec<PendingSymDiff>,
    /// Pending raw expressions to resolve after all labels are known
    pending_exprs: Vec<PendingExpr>,
    /// Pending instructions with symbolic offset expressions (resolved after labels are known)
    pending_instructions: Vec<PendingInstruction>,
}

struct PendingReloc {
    section: String,
    offset: u64,
    reloc_type: u32,
    symbol: String,
    addend: i64,
}

/// Returns true if the given ELF relocation type is a branch/jump relocation
/// that should be deferred for intra-section resolution at assembly time.
fn is_branch_reloc_type(elf_type: u32) -> bool {
    matches!(
        elf_type,
        279 |  // R_AARCH64_TSTBR14
        280 |  // R_AARCH64_CONDBR19
        282 |  // R_AARCH64_JUMP26
        283 // R_AARCH64_CALL26
    )
}

/// A pending raw expression to be resolved after all labels are known.
struct PendingExpr {
    section: String,
    offset: u64,
    expr: String,
    size: usize,
}

/// A pending symbol difference to be resolved after all labels are known.
struct PendingSymDiff {
    /// Section containing the data directive
    section: String,
    /// Offset within that section where the value should be written
    offset: u64,
    /// The positive symbol (A in A - B)
    sym_a: String,
    /// The negative symbol (B in A - B)
    sym_b: String,
    /// Extra addend
    extra_addend: i64,
    /// Size in bytes (1, 4, or 8)
    size: usize,
}

/// A pending instruction whose operands contain symbolic expressions.
/// A NOP placeholder is emitted at the instruction offset; after all labels
/// are known, the expression is evaluated, operands are updated, and the
/// instruction is re-encoded and patched in place.
struct PendingInstruction {
    section: String,
    offset: u64,
    mnemonic: String,
    operands: Vec<Operand>,
    raw_operands: String,
}

impl ElfWriter {
    pub fn new() -> Self {
        Self {
            base: ElfWriterBase::new(AARCH64_NOP, 4),
            pending_branch_relocs: Vec::new(),
            pending_sym_diffs: Vec::new(),
            pending_exprs: Vec::new(),
            pending_instructions: Vec::new(),
        }
    }

    /// Resolve pending symbol differences after all labels are known.
    fn resolve_sym_diffs(&mut self) -> Result<(), String> {
        let pending = std::mem::take(&mut self.pending_sym_diffs);
        for diff in &pending {
            let sym_a_info = self.base.labels.get(&diff.sym_a).cloned();
            let sym_b_info = self.base.labels.get(&diff.sym_b).cloned();

            match (sym_a_info, sym_b_info) {
                (Some((sec_a, off_a)), Some((sec_b, off_b))) => {
                    if sec_a == sec_b {
                        // Same section: resolve at assembly time by patching the data
                        let value = (off_a as i64) - (off_b as i64) + diff.extra_addend;
                        if let Some(section) = self.base.sections.get_mut(&diff.section) {
                            let off = diff.offset as usize;
                            if diff.size == 1 && off < section.data.len() {
                                section.data[off] = value as u8;
                            } else if diff.size == 4 && off + 4 <= section.data.len() {
                                section.data[off..off + 4]
                                    .copy_from_slice(&(value as i32).to_le_bytes());
                            } else if diff.size == 8 && off + 8 <= section.data.len() {
                                section.data[off..off + 8].copy_from_slice(&value.to_le_bytes());
                            }
                        }
                    } else {
                        // Cross-section: emit R_AARCH64_PREL32 or PREL64 based on size
                        let addend =
                            off_a as i64 + diff.offset as i64 - off_b as i64 + diff.extra_addend;
                        let reloc_type = if diff.size == 8 {
                            RelocType::Prel64.elf_type()
                        } else {
                            RelocType::Prel32.elf_type()
                        };
                        if let Some(section) = self.base.sections.get_mut(&diff.section) {
                            section.relocs.push(ObjReloc {
                                offset: diff.offset,
                                reloc_type,
                                symbol_name: sec_a.clone(),
                                addend,
                            });
                        }
                    }
                }
                _ => {
                    // Forward-referenced or external symbols: emit PREL32 or PREL64 based on size
                    let reloc_type = if diff.size == 8 {
                        RelocType::Prel64.elf_type()
                    } else {
                        RelocType::Prel32.elf_type()
                    };
                    if let Some(section) = self.base.sections.get_mut(&diff.section) {
                        section.relocs.push(ObjReloc {
                            offset: diff.offset,
                            reloc_type,
                            symbol_name: diff.sym_a.clone(),
                            addend: diff.extra_addend,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Resolve pending raw expressions by substituting label offsets and evaluating.
    fn resolve_pending_exprs(&mut self) -> Result<(), String> {
        let pending = std::mem::take(&mut self.pending_exprs);
        for pexpr in &pending {
            // Substitute label names with their offset values
            let mut expr = pexpr.expr.clone();

            // Collect all label names, sorted longest first to avoid partial replacements
            let mut label_names: Vec<&String> = self.base.labels.keys().collect();
            label_names.sort_by_key(|name| std::cmp::Reverse(name.len()));

            let mut all_resolved = true;
            for label_name in &label_names {
                if expr.contains(label_name.as_str()) {
                    if let Some((_section, offset)) = self.base.labels.get(*label_name) {
                        expr = expr.replace(label_name.as_str(), &offset.to_string());
                    } else {
                        all_resolved = false;
                    }
                }
            }

            if all_resolved {
                // Try to evaluate the expression
                if let Ok(val) = crate::backend::asm_expr::parse_integer_expr(&expr) {
                    if let Some(section) = self.base.sections.get_mut(&pexpr.section) {
                        let off = pexpr.offset as usize;
                        match pexpr.size {
                            1 if off < section.data.len() => {
                                section.data[off] = val as u8;
                            }
                            2 if off + 2 <= section.data.len() => {
                                section.data[off..off + 2]
                                    .copy_from_slice(&(val as i16).to_le_bytes());
                            }
                            4 if off + 4 <= section.data.len() => {
                                section.data[off..off + 4]
                                    .copy_from_slice(&(val as i32).to_le_bytes());
                            }
                            8 if off + 8 <= section.data.len() => {
                                section.data[off..off + 8].copy_from_slice(&val.to_le_bytes());
                            }
                            _ => {}
                        }
                    }
                } else {
                    // Couldn't evaluate - emit as symbol reference
                    if let Some(section) = self.base.sections.get_mut(&pexpr.section) {
                        section.relocs.push(ObjReloc {
                            offset: pexpr.offset,
                            reloc_type: RelocType::Abs32.elf_type(),
                            symbol_name: pexpr.expr.clone(),
                            addend: 0,
                        });
                    }
                }
            } else {
                // Unresolved symbols - emit as relocation
                if let Some(section) = self.base.sections.get_mut(&pexpr.section) {
                    section.relocs.push(ObjReloc {
                        offset: pexpr.offset,
                        reloc_type: RelocType::Abs32.elf_type(),
                        symbol_name: pexpr.expr.clone(),
                        addend: 0,
                    });
                }
            }
        }
        Ok(())
    }

    /// Resolve pending instructions that contain symbolic offset expressions.
    /// After all labels are positioned, substitute label names with offsets,
    /// evaluate the expression, update operands, re-encode, and patch in place.
    fn resolve_pending_instructions(&mut self) -> Result<(), String> {
        let pending = std::mem::take(&mut self.pending_instructions);
        for pinstr in &pending {
            // Resolve each operand that has a deferred expression
            let resolved_operands: Vec<Operand> = pinstr
                .operands
                .iter()
                .map(|op| match op {
                    Operand::MemExpr {
                        base,
                        expr,
                        writeback,
                    } => {
                        if let Some(val) = self.resolve_label_expr(expr) {
                            if *writeback {
                                Operand::MemPreIndex {
                                    base: base.clone(),
                                    offset: val,
                                }
                            } else {
                                Operand::Mem {
                                    base: base.clone(),
                                    offset: val,
                                }
                            }
                        } else {
                            op.clone()
                        }
                    }
                    Operand::Expr(expr) => {
                        if let Some(val) = self.resolve_label_expr(expr) {
                            Operand::Imm(val)
                        } else {
                            op.clone()
                        }
                    }
                    _ => op.clone(),
                })
                .collect();

            // Re-encode the instruction with resolved operands
            match encode_instruction(&pinstr.mnemonic, &resolved_operands, &pinstr.raw_operands) {
                Ok(EncodeResult::Word(word)) => {
                    if let Some(section) = self.base.sections.get_mut(&pinstr.section) {
                        let off = pinstr.offset as usize;
                        if off + 4 <= section.data.len() {
                            section.data[off..off + 4].copy_from_slice(&word.to_le_bytes());
                        }
                    }
                }
                Ok(EncodeResult::WordWithReloc { word, reloc }) => {
                    let elf_type = reloc.reloc_type.elf_type();
                    if let Some(section) = self.base.sections.get_mut(&pinstr.section) {
                        let off = pinstr.offset as usize;
                        if off + 4 <= section.data.len() {
                            section.data[off..off + 4].copy_from_slice(&word.to_le_bytes());
                        }
                        let is_local = reloc.symbol.starts_with(".L")
                            || reloc.symbol.starts_with(".l")
                            || reloc.symbol == ".";
                        if is_local || is_branch_reloc_type(elf_type) {
                            self.pending_branch_relocs.push(PendingReloc {
                                section: pinstr.section.clone(),
                                offset: pinstr.offset,
                                reloc_type: elf_type,
                                symbol: reloc.symbol.clone(),
                                addend: reloc.addend,
                            });
                        } else {
                            section.relocs.push(ObjReloc {
                                offset: pinstr.offset,
                                reloc_type: elf_type,
                                symbol_name: reloc.symbol,
                                addend: reloc.addend,
                            });
                        }
                    }
                }
                Ok(EncodeResult::Words(words)) => {
                    if let Some(section) = self.base.sections.get_mut(&pinstr.section) {
                        let off = pinstr.offset as usize;
                        for (j, word) in words.iter().enumerate() {
                            let wo = off + j * 4;
                            if wo + 4 <= section.data.len() {
                                section.data[wo..wo + 4].copy_from_slice(&word.to_le_bytes());
                            }
                        }
                    }
                }
                Ok(EncodeResult::Skip) => {}
                Err(e) => {
                    // Log error but don't fail the whole assembly
                    eprintln!(
                        "warning: failed to resolve deferred instruction '{}': {}",
                        pinstr.mnemonic, e
                    );
                }
            }
        }
        Ok(())
    }

    /// Substitute all known label names in an expression string with their byte
    /// offsets, then evaluate the resulting numeric expression.
    fn resolve_label_expr(&self, expr: &str) -> Option<i64> {
        let mut resolved = expr.to_string();

        // Collect all label names, sorted longest first to avoid partial replacements
        let mut label_names: Vec<&String> = self.base.labels.keys().collect();
        label_names.sort_by_key(|name| std::cmp::Reverse(name.len()));

        for label_name in &label_names {
            if resolved.contains(label_name.as_str()) {
                if let Some((_section, offset)) = self.base.labels.get(*label_name) {
                    resolved = resolved.replace(label_name.as_str(), &offset.to_string());
                }
            }
        }

        crate::backend::asm_expr::parse_integer_expr(&resolved).ok()
    }

    /// Process all parsed assembly statements.
    pub fn process_statements(&mut self, statements: &[AsmStatement]) -> Result<(), String> {
        for stmt in statements {
            self.process_statement(stmt)?;
        }
        // Merge subsections (e.g., .text.__subsection.1 → .text) before resolving
        let remap = self.base.merge_subsections();
        // Fix up pending references that pointed to now-merged subsection names
        if !remap.is_empty() {
            for reloc in &mut self.pending_branch_relocs {
                if let Some((parent, offset_adj)) = remap.get(&reloc.section) {
                    reloc.offset += offset_adj;
                    reloc.section = parent.clone();
                }
            }
            for diff in &mut self.pending_sym_diffs {
                if let Some((parent, offset_adj)) = remap.get(&diff.section) {
                    diff.offset += offset_adj;
                    diff.section = parent.clone();
                }
            }
            for expr in &mut self.pending_exprs {
                if let Some((parent, offset_adj)) = remap.get(&expr.section) {
                    expr.offset += offset_adj;
                    expr.section = parent.clone();
                }
            }
            for instr in &mut self.pending_instructions {
                if let Some((parent, offset_adj)) = remap.get(&instr.section) {
                    instr.offset += offset_adj;
                    instr.section = parent.clone();
                }
            }
        }
        // Resolve symbol differences first (needs all labels to be known)
        self.resolve_sym_diffs()?;
        self.resolve_pending_exprs()?;
        self.resolve_pending_instructions()?;
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
                self.base.labels.insert(name.clone(), (section, offset));
                Ok(())
            }

            AsmStatement::Directive(dir) => self.process_directive(dir),

            AsmStatement::Instruction {
                mnemonic,
                operands,
                raw_operands,
            } => self.process_instruction(mnemonic, operands, raw_operands),

            AsmStatement::LdrLiteralPool { .. } => {
                // Should have been expanded by expand_literal_pools() before reaching here
                Err("LdrLiteralPool should have been expanded before ELF writing".to_string())
            }
        }
    }

    fn process_directive(&mut self, dir: &AsmDirective) -> Result<(), String> {
        match dir {
            AsmDirective::Section(sec) => {
                self.base.process_section_directive(
                    &sec.name,
                    sec.flags.as_deref().unwrap_or(""),
                    sec.flags.is_some(), // flags are explicit when provided
                    sec.section_type.as_deref(),
                );
                Ok(())
            }

            AsmDirective::PushSection(sec) => {
                self.base.push_section(
                    &sec.name,
                    sec.flags.as_deref().unwrap_or(""),
                    sec.flags.is_some(),
                    sec.section_type.as_deref(),
                );
                Ok(())
            }

            AsmDirective::PopSection => {
                self.base.pop_section();
                Ok(())
            }

            AsmDirective::Previous => {
                self.base.restore_previous_section();
                Ok(())
            }

            AsmDirective::Subsection(n) => {
                self.base.set_subsection(*n);
                Ok(())
            }

            AsmDirective::Global(sym) => {
                for s in sym.split(',') {
                    let s = s.trim();
                    if !s.is_empty() {
                        self.base.set_global(s);
                    }
                }
                Ok(())
            }
            AsmDirective::Weak(sym) => {
                self.base.set_weak(sym);
                Ok(())
            }
            AsmDirective::Hidden(sym) => {
                self.base.set_visibility(sym, STV_HIDDEN);
                Ok(())
            }
            AsmDirective::Protected(sym) => {
                self.base.set_visibility(sym, STV_PROTECTED);
                Ok(())
            }
            AsmDirective::Internal(sym) => {
                self.base.set_visibility(sym, STV_INTERNAL);
                Ok(())
            }

            AsmDirective::SymbolType(sym, kind) => {
                let st = match kind {
                    SymbolKind::Function => STT_FUNC,
                    SymbolKind::Object => STT_OBJECT,
                    SymbolKind::TlsObject => STT_TLS,
                    SymbolKind::NoType => STT_NOTYPE,
                };
                self.base.set_symbol_type(sym, st);
                Ok(())
            }

            AsmDirective::Size(sym, expr) => {
                match expr {
                    SizeExpr::CurrentMinusSymbol(label) => {
                        self.base.set_symbol_size(sym, Some(label), None);
                    }
                    SizeExpr::Constant(size) => {
                        self.base.set_symbol_size(sym, None, Some(*size));
                    }
                }
                Ok(())
            }

            AsmDirective::Align(bytes) | AsmDirective::Balign(bytes) => {
                self.base.align_to(*bytes);
                Ok(())
            }

            AsmDirective::Byte(vals) => self.emit_data_values(vals, 1),

            AsmDirective::Short(vals) => {
                for val in vals {
                    self.base.emit_bytes(&(*val as u16).to_le_bytes());
                }
                Ok(())
            }

            AsmDirective::Long(vals) => self.emit_data_values(vals, 4),
            AsmDirective::Quad(vals) => self.emit_data_values(vals, 8),

            AsmDirective::Zero(size, fill) => {
                self.base.emit_bytes(&vec![*fill; *size]);
                Ok(())
            }
            AsmDirective::Asciz(bytes) => {
                self.base.emit_bytes(bytes);
                Ok(())
            }
            AsmDirective::Ascii(bytes) => {
                self.base.emit_bytes(bytes);
                Ok(())
            }

            AsmDirective::Comm(sym, size, align) => {
                self.base.emit_comm(sym, *size, *align);
                Ok(())
            }

            AsmDirective::Local(_) => Ok(()),

            AsmDirective::Set(alias, target) => {
                self.base.set_alias(alias, target);
                Ok(())
            }

            AsmDirective::Incbin { path, skip, count } => {
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

            AsmDirective::RawBytes(bytes) => {
                self.base.emit_bytes(bytes);
                Ok(())
            }

            AsmDirective::Cfi | AsmDirective::Ignored | AsmDirective::Ltorg => Ok(()),
        }
    }

    /// Emit typed data values (Long or Quad) with proper relocations.
    fn emit_data_values(&mut self, vals: &[DataValue], size: usize) -> Result<(), String> {
        for val in vals {
            match val {
                DataValue::Integer(v) => {
                    self.base.emit_data_integer(*v, size);
                }
                DataValue::Symbol(sym) => {
                    let reloc_type = if size == 4 {
                        RelocType::Abs32.elf_type()
                    } else {
                        RelocType::Abs64.elf_type()
                    };
                    self.base.emit_data_symbol_ref(sym, 0, size, reloc_type);
                }
                DataValue::SymbolOffset(sym, addend) => {
                    let reloc_type = if size == 4 {
                        RelocType::Abs32.elf_type()
                    } else {
                        RelocType::Abs64.elf_type()
                    };
                    self.base
                        .emit_data_symbol_ref(sym, *addend, size, reloc_type);
                }
                DataValue::SymbolDiff(sym_a, sym_b) => {
                    self.record_sym_diff(sym_a, sym_b, 0, size);
                }
                DataValue::SymbolDiffAddend(sym_a, sym_b, addend) => {
                    self.record_sym_diff(sym_a, sym_b, *addend, size);
                }
                DataValue::Expr(expr) => {
                    let section = self.base.current_section.clone();
                    let offset = self.base.current_offset();
                    self.pending_exprs.push(PendingExpr {
                        section,
                        offset,
                        expr: expr.clone(),
                        size,
                    });
                    self.base.emit_placeholder(size);
                }
            }
        }
        Ok(())
    }

    /// Record a pending symbol difference for deferred resolution.
    fn record_sym_diff(&mut self, sym_a: &str, sym_b: &str, extra_addend: i64, size: usize) {
        let section = self.base.current_section.clone();
        let offset = self.base.current_offset();
        self.pending_sym_diffs.push(PendingSymDiff {
            section,
            offset,
            sym_a: sym_a.to_string(),
            sym_b: sym_b.to_string(),
            extra_addend,
            size,
        });
        self.base.emit_placeholder(size);
    }

    /// Check if any operand contains a deferred symbolic expression.
    fn has_deferred_expr(operands: &[Operand]) -> bool {
        operands
            .iter()
            .any(|op| matches!(op, Operand::MemExpr { .. } | Operand::Expr(_)))
    }

    fn process_instruction(
        &mut self,
        mnemonic: &str,
        operands: &[Operand],
        raw_operands: &str,
    ) -> Result<(), String> {
        self.base.ensure_text_section();

        // If any operand has a symbolic expression that needs deferred resolution,
        // emit a NOP placeholder and queue for later resolution.
        if Self::has_deferred_expr(operands) {
            let section = self.base.current_section.clone();
            let offset = self.base.current_offset();
            self.pending_instructions.push(PendingInstruction {
                section,
                offset,
                mnemonic: mnemonic.to_string(),
                operands: operands.to_vec(),
                raw_operands: raw_operands.to_string(),
            });
            // Emit NOP placeholder (will be patched during resolution)
            self.base.emit_u32_le(u32::from_le_bytes(AARCH64_NOP));
            return Ok(());
        }

        match encode_instruction(mnemonic, operands, raw_operands) {
            Ok(EncodeResult::Word(word)) => {
                self.base.emit_u32_le(word);
                Ok(())
            }
            Ok(EncodeResult::WordWithReloc { word, reloc }) => {
                let elf_type = reloc.reloc_type.elf_type();
                let is_local = reloc.symbol.starts_with(".L")
                    || reloc.symbol.starts_with(".l")
                    || reloc.symbol == ".";

                if is_local || is_branch_reloc_type(elf_type) {
                    let offset = self.base.current_offset();
                    self.pending_branch_relocs.push(PendingReloc {
                        section: self.base.current_section.clone(),
                        offset,
                        reloc_type: elf_type,
                        symbol: reloc.symbol.clone(),
                        addend: reloc.addend,
                    });
                    self.base.emit_u32_le(word);
                } else {
                    self.base.add_reloc(elf_type, reloc.symbol, reloc.addend);
                    self.base.emit_u32_le(word);
                }
                Ok(())
            }
            Ok(EncodeResult::Words(words)) => {
                for word in words {
                    self.base.emit_u32_le(word);
                }
                Ok(())
            }
            Ok(EncodeResult::Skip) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Resolve local branch labels to PC-relative offsets using AArch64 relocation types.
    /// For symbols defined in the same section, the PC-relative offset is computed and
    /// patched directly into the instruction (matching GAS behavior). For undefined or
    /// cross-section symbols, an external relocation is emitted.
    fn resolve_local_branches(&mut self) -> Result<(), String> {
        for reloc in &self.pending_branch_relocs {
            // "." means current address (branch to self)
            let (target_section, target_offset) = if reloc.symbol == "." {
                (reloc.section.clone(), reloc.offset)
            } else if let Some(label_info) = self.base.labels.get(&reloc.symbol) {
                label_info.clone()
            } else {
                // Symbol not defined locally - emit as external relocation
                if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                    section.relocs.push(ObjReloc {
                        offset: reloc.offset,
                        reloc_type: reloc.reloc_type,
                        symbol_name: reloc.symbol.clone(),
                        addend: reloc.addend,
                    });
                }
                continue;
            };

            if target_section != reloc.section {
                // Cross-section reference - convert to section symbol + offset
                if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                    section.relocs.push(ObjReloc {
                        offset: reloc.offset,
                        reloc_type: reloc.reloc_type,
                        symbol_name: target_section.clone(),
                        addend: target_offset as i64 + reloc.addend,
                    });
                }
                continue;
            }

            let pc_offset = (target_offset as i64) - (reloc.offset as i64) + reloc.addend;

            if let Some(section) = self.base.sections.get_mut(&reloc.section) {
                let instr_offset = reloc.offset as usize;
                if instr_offset + 4 > section.data.len() {
                    continue;
                }

                let mut word = u32::from_le_bytes([
                    section.data[instr_offset],
                    section.data[instr_offset + 1],
                    section.data[instr_offset + 2],
                    section.data[instr_offset + 3],
                ]);

                match reloc.reloc_type {
                    282 | 283 => {
                        // R_AARCH64_JUMP26 / R_AARCH64_CALL26
                        let imm26 = ((pc_offset >> 2) as u32) & 0x3FFFFFF;
                        word |= imm26;
                    }
                    280 => {
                        // R_AARCH64_CONDBR19
                        let imm19 = ((pc_offset >> 2) as u32) & 0x7FFFF;
                        word |= imm19 << 5;
                    }
                    279 => {
                        // R_AARCH64_TSTBR14
                        let imm14 = ((pc_offset >> 2) as u32) & 0x3FFF;
                        word |= imm14 << 5;
                    }
                    273 => {
                        // R_AARCH64_LD_PREL_LO19 - LDR literal
                        let imm19 = ((pc_offset >> 2) as u32) & 0x7FFFF;
                        word |= imm19 << 5;
                    }
                    274 => {
                        // R_AARCH64_ADR_PREL_LO21 - ADR instruction
                        let imm = pc_offset as i32;
                        let immlo = (imm as u32) & 0x3;
                        let immhi = ((imm as u32) >> 2) & 0x7FFFF;
                        word |= (immlo << 29) | (immhi << 5);
                    }
                    275 => {
                        // R_AARCH64_ADR_PREL_PG_HI21 - ADRP instruction (local resolution)
                        let pc_page = (reloc.offset as i64) & !0xFFF;
                        let target_page = (target_offset as i64) & !0xFFF;
                        let page_off = target_page - pc_page;
                        let imm = (page_off >> 12) as i32;
                        let immlo = (imm as u32) & 0x3;
                        let immhi = ((imm as u32) >> 2) & 0x7FFFF;
                        word |= (immlo << 29) | (immhi << 5);
                    }
                    _ => {
                        // Unknown reloc type for local branch - leave as external
                        section.relocs.push(ObjReloc {
                            offset: reloc.offset,
                            reloc_type: reloc.reloc_type,
                            symbol_name: reloc.symbol.clone(),
                            addend: reloc.addend,
                        });
                        continue;
                    }
                }

                section.data[instr_offset..instr_offset + 4].copy_from_slice(&word.to_le_bytes());
            }
        }
        Ok(())
    }

    /// Write the final ELF object file.
    pub fn write_elf(&mut self, output_path: &str) -> Result<(), String> {
        let config = elf::ElfConfig {
            e_machine: EM_AARCH64,
            e_flags: 0,
            elf_class: ELFCLASS64,
            force_rela: false,
        };
        self.base.write_elf(output_path, &config, false)
    }
}
