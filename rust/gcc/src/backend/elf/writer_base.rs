//! Shared ELF writer base for ARM and RISC-V assembler backends.
//!
//! ARM and RISC-V ELF writers share ~400 lines of identical code for section
//! management, symbol tracking, relocation recording, alignment, directive
//! processing, data emission, and ELF serialization.
//!
//! This `ElfWriterBase` struct captures all of that shared state and logic.
//! Each arch-specific ElfWriter composes with this base and adds its own
//! instruction encoding, branch resolution, and other arch-specific features:
//! - ARM adds `pending_sym_diffs` and AArch64-specific branch resolution
//! - RISC-V adds `pcrel_hi_counter`, `numeric_labels`, and RV64C compression

use super::constants::*;
use super::linker_symbols::default_section_flags;
use super::object_writer::write_relocatable_object;
use super::object_writer::{ElfConfig, ObjReloc, ObjSection};
use super::symbol_table::{build_elf_symbol_table, ObjSymbol, SymbolTableInput};
use std::collections::HashMap;

/// Shared ELF writer state used by both ARM and RISC-V assembler backends.
///
/// This struct manages sections, symbols, labels, and relocations using the
/// shared `ObjSection`/`ObjReloc`/`ObjSymbol` types directly, eliminating the
/// per-arch conversion step in `write_elf()`.
///
/// Architecture-specific ElfWriters compose with this base:
/// - ARM adds `pending_sym_diffs` and AArch64-specific branch resolution
/// - RISC-V adds `pcrel_hi_counter`, `numeric_labels`, and RV64C compression
pub struct ElfWriterBase {
    /// Current section we're emitting into
    pub current_section: String,
    /// All sections being built (using shared ObjSection directly)
    pub sections: HashMap<String, ObjSection>,
    /// Section order (for deterministic output)
    pub section_order: Vec<String>,
    /// Extra symbols (e.g., COMMON symbols from .comm directives)
    pub extra_symbols: Vec<ObjSymbol>,
    /// Local labels -> (section, offset) for branch resolution
    pub labels: HashMap<String, (String, u64)>,
    /// Symbols that have been declared .globl
    pub global_symbols: HashMap<String, bool>,
    /// Symbols declared .weak
    pub weak_symbols: HashMap<String, bool>,
    /// Symbol types from .type directives
    pub symbol_types: HashMap<String, u8>,
    /// Symbol sizes from .size directives
    pub symbol_sizes: HashMap<String, u64>,
    /// Symbol visibility from .hidden/.protected/.internal
    pub symbol_visibility: HashMap<String, u8>,
    /// Symbol aliases from .set/.equ directives
    pub aliases: HashMap<String, String>,
    /// Section stack for .pushsection/.popsection (saves both current and previous section)
    section_stack: Vec<(String, String)>,
    /// Previous section for .section/.previous swapping
    previous_section: String,
    /// NOP instruction bytes for code section alignment padding.
    /// ARM: `[0x1f, 0x20, 0x03, 0xd5]` (d503201f), RISC-V: `[0x13, 0x00, 0x00, 0x00]` (00000013)
    nop_bytes: [u8; 4],
    /// Default text section alignment (4 for ARM, 2 for RISC-V with compressed instructions)
    text_align: u64,
}

impl ElfWriterBase {
    pub fn new(nop_bytes: [u8; 4], text_align: u64) -> Self {
        Self {
            current_section: String::new(),
            sections: HashMap::new(),
            section_order: Vec::new(),
            extra_symbols: Vec::new(),
            labels: HashMap::new(),
            global_symbols: HashMap::new(),
            weak_symbols: HashMap::new(),
            symbol_types: HashMap::new(),
            symbol_sizes: HashMap::new(),
            symbol_visibility: HashMap::new(),
            aliases: HashMap::new(),
            section_stack: Vec::new(),
            previous_section: String::new(),
            nop_bytes,
            text_align,
        }
    }

    /// Ensure a section exists. If it doesn't, create it with the given properties.
    pub fn ensure_section(&mut self, name: &str, sh_type: u32, sh_flags: u64, align: u64) {
        if !self.sections.contains_key(name) {
            self.sections.insert(
                name.to_string(),
                ObjSection {
                    name: name.to_string(),
                    sh_type,
                    sh_flags,
                    data: Vec::new(),
                    sh_addralign: align,
                    relocs: Vec::new(),
                    comdat_group: None,
                },
            );
            self.section_order.push(name.to_string());
        }
    }

    /// Get the current write offset within the current section.
    pub fn current_offset(&self) -> u64 {
        self.sections
            .get(&self.current_section)
            .map(|s| s.data.len() as u64)
            .unwrap_or(0)
    }

    /// Append raw bytes to the current section.
    pub fn emit_bytes(&mut self, bytes: &[u8]) {
        if let Some(section) = self.sections.get_mut(&self.current_section) {
            section.data.extend_from_slice(bytes);
        }
    }

    /// Append a 16-bit little-endian value to the current section.
    pub fn emit_u16_le(&mut self, val: u16) {
        self.emit_bytes(&val.to_le_bytes());
    }

    /// Append a 32-bit little-endian value to the current section.
    pub fn emit_u32_le(&mut self, val: u32) {
        self.emit_bytes(&val.to_le_bytes());
    }

    /// Record a relocation at the current offset in the current section.
    pub fn add_reloc(&mut self, reloc_type: u32, symbol: String, addend: i64) {
        let offset = self.current_offset();
        let section = self.current_section.clone();
        if let Some(s) = self.sections.get_mut(&section) {
            s.relocs.push(ObjReloc {
                offset,
                reloc_type,
                symbol_name: symbol,
                addend,
            });
        }
    }

    /// Align the current section's data to the specified byte boundary.
    ///
    /// Code sections are NOP-padded using the architecture's NOP instruction;
    /// data sections are zero-padded.
    pub fn align_to(&mut self, align: u64) {
        if align <= 1 {
            return;
        }
        if let Some(section) = self.sections.get_mut(&self.current_section) {
            let current = section.data.len() as u64;
            let aligned = (current + align - 1) & !(align - 1);
            let padding = (aligned - current) as usize;
            if section.sh_flags & SHF_EXECINSTR != 0 && align >= 4 {
                let full_nops = padding / 4;
                let remainder = padding % 4;
                for _ in 0..full_nops {
                    section.data.extend_from_slice(&self.nop_bytes);
                }
                section.data.extend(std::iter::repeat_n(0u8, remainder));
            } else {
                section.data.extend(std::iter::repeat_n(0u8, padding));
            }
            if align > section.sh_addralign {
                section.sh_addralign = align;
            }
        }
    }

    /// Ensure we're in a text section, creating one if needed.
    pub fn ensure_text_section(&mut self) {
        if self.current_section.is_empty() {
            self.ensure_section(
                ".text",
                SHT_PROGBITS,
                SHF_ALLOC | SHF_EXECINSTR,
                self.text_align,
            );
            self.current_section = ".text".to_string();
        }
    }

    /// Process a .section directive with parsed fields.
    ///
    /// `sec_name`: section name, `flags_str`: flag characters ("awx" etc.),
    /// `flags_explicit`: whether flags were explicitly provided (vs default),
    /// `sec_type_str`: optional type string ("@nobits", "@note", etc.)
    pub fn process_section_directive(
        &mut self,
        sec_name: &str,
        flags_str: &str,
        flags_explicit: bool,
        sec_type_str: Option<&str>,
    ) {
        let sh_type = match sec_type_str {
            Some("@nobits") => SHT_NOBITS,
            Some("@note") => SHT_NOTE,
            _ => {
                if sec_name == ".bss"
                    || sec_name.starts_with(".bss.")
                    || sec_name.starts_with(".tbss")
                {
                    SHT_NOBITS
                } else {
                    SHT_PROGBITS
                }
            }
        };

        let mut sh_flags = 0u64;
        if flags_str.contains('a') {
            sh_flags |= SHF_ALLOC;
        }
        if flags_str.contains('w') {
            sh_flags |= SHF_WRITE;
        }
        if flags_str.contains('x') {
            sh_flags |= SHF_EXECINSTR;
        }
        if flags_str.contains('M') {
            sh_flags |= SHF_MERGE;
        }
        if flags_str.contains('S') {
            sh_flags |= SHF_STRINGS;
        }
        if flags_str.contains('T') {
            sh_flags |= SHF_TLS;
        }
        if flags_str.contains('G') {
            sh_flags |= SHF_GROUP;
        }

        if sh_flags == 0 && !flags_explicit {
            sh_flags = default_section_flags(sec_name);
        }

        let align = if sh_flags & SHF_EXECINSTR != 0 {
            self.text_align
        } else {
            1
        };
        self.ensure_section(sec_name, sh_type, sh_flags, align);
        self.previous_section = std::mem::replace(&mut self.current_section, sec_name.to_string());
    }

    /// Switch to a named standard section (.text, .data, .bss, .rodata).
    pub fn switch_to_standard_section(&mut self, name: &str, sh_type: u32, sh_flags: u64) {
        let align = if sh_flags & SHF_EXECINSTR != 0 {
            self.text_align
        } else {
            1
        };
        self.ensure_section(name, sh_type, sh_flags, align);
        self.previous_section = std::mem::replace(&mut self.current_section, name.to_string());
    }

    /// Restore the previous section (for `.previous` directive).
    /// Swaps current and previous sections, so repeated `.previous` toggles between two sections.
    pub fn restore_previous_section(&mut self) {
        if !self.previous_section.is_empty() {
            std::mem::swap(&mut self.current_section, &mut self.previous_section);
        }
    }

    /// Push current section onto the stack and switch to a new section.
    /// Saves both current_section and previous_section so that .popsection
    /// fully restores the section state (matching GNU as behavior).
    pub fn push_section(
        &mut self,
        name: &str,
        flags_str: &str,
        flags_explicit: bool,
        sec_type: Option<&str>,
    ) {
        self.section_stack
            .push((self.current_section.clone(), self.previous_section.clone()));
        self.process_section_directive(name, flags_str, flags_explicit, sec_type);
    }

    /// Pop the section stack and restore both current and previous sections.
    pub fn pop_section(&mut self) {
        if let Some((saved_current, saved_previous)) = self.section_stack.pop() {
            self.current_section = saved_current;
            self.previous_section = saved_previous;
        }
    }

    /// Switch to a numbered subsection within the current section.
    ///
    /// `.subsection N` creates an internal section `PARENT.__subsection.N` that
    /// gets merged back into the parent section after all statements are processed.
    /// Subsections are concatenated in numeric order (0 first, then 1, 2, ...).
    pub fn set_subsection(&mut self, n: u64) {
        // Determine the parent section name (strip any existing subsection suffix)
        let parent = if let Some(pos) = self.current_section.find(".__subsection.") {
            self.current_section[..pos].to_string()
        } else {
            self.current_section.clone()
        };

        if n == 0 {
            // Switch back to parent section (subsection 0 = parent itself)
            self.previous_section = std::mem::replace(&mut self.current_section, parent);
        } else {
            // Switch to subsection N
            let sub_name = format!("{}.__subsection.{}", parent, n);
            // Inherit properties from parent section
            if !self.sections.contains_key(&sub_name) {
                if let Some(parent_sec) = self.sections.get(&parent) {
                    let sh_type = parent_sec.sh_type;
                    let sh_flags = parent_sec.sh_flags;
                    let align = parent_sec.sh_addralign;
                    self.ensure_section(&sub_name, sh_type, sh_flags, align);
                } else {
                    // Parent doesn't exist yet; create subsection with code defaults
                    self.ensure_section(&sub_name, 1, 0x6, self.text_align); // SHT_PROGBITS, AX
                }
            }
            self.previous_section = std::mem::replace(&mut self.current_section, sub_name);
        }
    }

    /// Merge all subsections back into their parent sections.
    ///
    /// After all statements are processed, subsections like `.text.__subsection.1`
    /// are appended to their parent `.text` in numeric order. Labels and relocations
    /// are adjusted to account for the new offsets.
    ///
    /// Returns a mapping from subsection name to (parent_name, offset_adjustment)
    /// so callers can fix up any pending references that point to subsection names.
    pub fn merge_subsections(&mut self) -> HashMap<String, (String, u64)> {
        let mut remap = HashMap::new();

        // Collect subsection names grouped by parent
        let mut subsections: std::collections::BTreeMap<
            String,
            std::collections::BTreeMap<u64, String>,
        > = std::collections::BTreeMap::new();

        for name in &self.section_order {
            if let Some(pos) = name.find(".__subsection.") {
                let parent = name[..pos].to_string();
                let num: u64 = name[pos + 14..].parse().unwrap_or(0);
                subsections
                    .entry(parent)
                    .or_default()
                    .insert(num, name.clone());
            }
        }

        if subsections.is_empty() {
            return remap;
        }

        // For each parent, append subsections in order
        for (parent, subs) in &subsections {
            for sub_name in subs.values() {
                let sub_data;
                let sub_relocs;
                {
                    let sub_sec = match self.sections.get(sub_name) {
                        Some(s) => s,
                        None => continue,
                    };
                    sub_data = sub_sec.data.clone();
                    sub_relocs = sub_sec.relocs.clone();
                }

                let parent_len = self
                    .sections
                    .get(parent)
                    .map(|s| s.data.len() as u64)
                    .unwrap_or(0);

                // Record the remapping for callers
                remap.insert(sub_name.clone(), (parent.clone(), parent_len));

                // Append data
                if let Some(parent_sec) = self.sections.get_mut(parent) {
                    parent_sec.data.extend_from_slice(&sub_data);
                    // Append relocations with adjusted offsets
                    for mut reloc in sub_relocs {
                        reloc.offset += parent_len;
                        parent_sec.relocs.push(reloc);
                    }
                }

                // Adjust labels that reference this subsection
                let labels_to_update: Vec<(String, u64)> = self
                    .labels
                    .iter()
                    .filter(|(_, (sec, _))| sec == sub_name)
                    .map(|(name, (_, off))| (name.clone(), *off))
                    .collect();

                for (label_name, old_offset) in labels_to_update {
                    self.labels
                        .insert(label_name, (parent.clone(), old_offset + parent_len));
                }

                // Remove the subsection
                self.sections.remove(sub_name);
            }
        }

        // Remove subsection names from section_order
        self.section_order
            .retain(|name| !name.contains(".__subsection."));

        // Fix current_section if it pointed to a subsection
        if self.current_section.contains(".__subsection.") {
            if let Some(pos) = self.current_section.find(".__subsection.") {
                self.current_section = self.current_section[..pos].to_string();
            }
        }
        if self.previous_section.contains(".__subsection.") {
            if let Some(pos) = self.previous_section.find(".__subsection.") {
                self.previous_section = self.previous_section[..pos].to_string();
            }
        }

        remap
    }

    /// Record .globl for a symbol.
    pub fn set_global(&mut self, sym: &str) {
        self.global_symbols.insert(sym.to_string(), true);
    }

    /// Record .weak for a symbol.
    pub fn set_weak(&mut self, sym: &str) {
        self.weak_symbols.insert(sym.to_string(), true);
    }

    /// Record symbol visibility (.hidden, .protected, .internal).
    pub fn set_visibility(&mut self, sym: &str, vis: u8) {
        self.symbol_visibility.insert(sym.to_string(), vis);
    }

    /// Record .type for a symbol (STT_FUNC, STT_OBJECT, etc.).
    pub fn set_symbol_type(&mut self, sym: &str, st: u8) {
        self.symbol_types.insert(sym.to_string(), st);
    }

    /// Record .size for a symbol. If `current_minus_label` is Some, computes
    /// `current_offset - label_offset` in the same section. Otherwise uses the absolute value.
    pub fn set_symbol_size(
        &mut self,
        sym: &str,
        current_minus_label: Option<&str>,
        absolute: Option<u64>,
    ) {
        if let Some(label) = current_minus_label {
            if let Some((section, label_offset)) = self.labels.get(label) {
                if *section == self.current_section {
                    let current = self.current_offset();
                    let size = current - label_offset;
                    self.symbol_sizes.insert(sym.to_string(), size);
                }
            }
        } else if let Some(size) = absolute {
            self.symbol_sizes.insert(sym.to_string(), size);
        }
    }

    /// Emit a .comm symbol (COMMON block).
    pub fn emit_comm(&mut self, sym: &str, size: u64, align: u64) {
        self.extra_symbols.push(ObjSymbol {
            name: sym.to_string(),
            value: align,
            size,
            binding: STB_GLOBAL,
            sym_type: STT_OBJECT,
            visibility: STV_DEFAULT,
            section_name: "*COM*".to_string(),
        });
    }

    /// Record a .set/.equ alias.
    pub fn set_alias(&mut self, alias: &str, target: &str) {
        self.aliases.insert(alias.to_string(), target.to_string());
    }

    /// Resolve .set/.equ aliases in an expression string.
    /// Replaces symbol names (like `.L__gpr_num_t0`) with their numeric values.
    pub fn resolve_expr_aliases(&self, expr: &str) -> String {
        let mut result = String::with_capacity(expr.len());
        let bytes = expr.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            // Symbol names start with a letter, underscore, or dot
            if c == b'.' || c == b'_' || c.is_ascii_alphabetic() {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && (bytes[i] == b'.' || bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric())
                {
                    i += 1;
                }
                let sym = &expr[start..i];
                // Chase alias chain
                let mut resolved = sym;
                let mut seen = 0;
                while let Some(target) = self.aliases.get(resolved) {
                    resolved = target.as_str();
                    seen += 1;
                    if seen > 20 {
                        break;
                    }
                }
                result.push_str(resolved);
            } else {
                result.push(c as char);
                i += 1;
            }
        }
        result
    }

    /// Resolve label names in an expression to their numeric offsets.
    /// This handles `.Ldot_N` synthetic labels (current position) and any
    /// section-local labels that can be resolved to constant offsets.
    pub fn resolve_expr_labels(&self, expr: &str) -> String {
        let cur_section = &self.current_section;
        let cur_offset = self.current_offset();
        let mut result = String::with_capacity(expr.len());
        let bytes = expr.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'.' || c == b'_' || c.is_ascii_alphabetic() {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && (bytes[i] == b'.' || bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric())
                {
                    i += 1;
                }
                let sym = &expr[start..i];
                // Standalone '.' means current position
                if sym == "." {
                    result.push_str(&cur_offset.to_string());
                // Check if this is a .Ldot_N label (current position)
                } else if sym.starts_with(".Ldot_") {
                    result.push_str(&cur_offset.to_string());
                } else if let Some((sec, off)) = self.labels.get(sym) {
                    if sec == cur_section {
                        result.push_str(&off.to_string());
                    } else {
                        result.push_str(sym);
                    }
                } else {
                    result.push_str(sym);
                }
            } else {
                result.push(c as char);
                i += 1;
            }
        }
        result
    }

    /// Resolve ALL label names in an expression to their numeric offsets,
    /// using the specified section as context. Unlike `resolve_expr_labels`,
    /// this also resolves `.Ldot_N` labels from their stored definitions
    /// (not the current offset), making it suitable for deferred resolution.
    pub fn resolve_expr_all_labels(&self, expr: &str, section: &str) -> String {
        let mut result = String::with_capacity(expr.len());
        let bytes = expr.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'.' || c == b'_' || c.is_ascii_alphabetic() {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && (bytes[i] == b'.' || bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric())
                {
                    i += 1;
                }
                let sym = &expr[start..i];
                if let Some((sec, off)) = self.labels.get(sym) {
                    if sec == section {
                        result.push_str(&off.to_string());
                    } else {
                        result.push_str(sym);
                    }
                } else {
                    result.push_str(sym);
                }
            } else {
                result.push(c as char);
                i += 1;
            }
        }
        result
    }

    /// Resolve label names in an expression to their offsets regardless of section.
    ///
    /// Unlike `resolve_expr_all_labels` which only resolves labels in the same
    /// section, this resolves ALL labels to their offsets. This is safe for
    /// expressions that compute differences between labels in the same section
    /// (the section offsets cancel out), which is common in kernel ALTERNATIVE
    /// macros (e.g., `889f - 888f` computing the size of alternative code).
    pub fn resolve_expr_cross_section(&self, expr: &str) -> String {
        let resolved = self.resolve_expr_aliases(expr);
        let mut result = String::with_capacity(resolved.len());
        let bytes = resolved.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'.' || c == b'_' || c.is_ascii_alphabetic() {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && (bytes[i] == b'.' || bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric())
                {
                    i += 1;
                }
                let sym = &resolved[start..i];
                if let Some((_sec, off)) = self.labels.get(sym) {
                    result.push_str(&off.to_string());
                } else {
                    result.push_str(sym);
                }
            } else {
                result.push(c as char);
                i += 1;
            }
        }
        result
    }

    /// Emit a plain integer value for .byte (size=1), .short (size=2), .long (size=4) or .quad (size=8).
    pub fn emit_data_integer(&mut self, val: i64, size: usize) {
        match size {
            1 => self.emit_bytes(&[val as u8]),
            2 => self.emit_bytes(&(val as u16).to_le_bytes()),
            4 => self.emit_bytes(&(val as u32).to_le_bytes()),
            _ => self.emit_bytes(&(val as u64).to_le_bytes()),
        }
    }

    /// Emit a symbol reference with a relocation.
    pub fn emit_data_symbol_ref(&mut self, sym: &str, addend: i64, size: usize, reloc_type: u32) {
        self.add_reloc(reloc_type, sym.to_string(), addend);
        match size {
            1 => self.emit_bytes(&[0u8]),
            2 => self.emit_bytes(&0u16.to_le_bytes()),
            4 => self.emit_bytes(&0u32.to_le_bytes()),
            _ => self.emit_bytes(&0u64.to_le_bytes()),
        }
    }

    /// Emit placeholder bytes for a deferred value (symbol diff, etc.).
    pub fn emit_placeholder(&mut self, size: usize) {
        match size {
            1 => self.emit_bytes(&[0u8]),
            2 => self.emit_bytes(&0u16.to_le_bytes()),
            4 => self.emit_bytes(&0u32.to_le_bytes()),
            _ => self.emit_bytes(&0u64.to_le_bytes()),
        }
    }

    /// Resolve local label references in data relocations.
    ///
    /// When a data directive like `.xword .Lstr0` references a local label
    /// in a different section, the local label won't be in the symbol table.
    /// Convert these to section_symbol + offset_of_label_in_section.
    pub fn resolve_local_data_relocs(&mut self) {
        let labels = self.labels.clone();
        for sec_name in &self.section_order.clone() {
            if let Some(section) = self.sections.get_mut(sec_name) {
                for reloc in &mut section.relocs {
                    // Skip pcrel_lo12 relocations — they must keep their
                    // .Lpcrel_hi label reference (not section+offset)
                    let is_pcrel_lo = reloc.reloc_type == 24 || reloc.reloc_type == 25;
                    if is_pcrel_lo {
                        continue;
                    }
                    if (reloc.symbol_name.starts_with(".L") || reloc.symbol_name.starts_with(".l"))
                        && !reloc.symbol_name.is_empty()
                    {
                        if let Some((label_section, label_offset)) = labels.get(&reloc.symbol_name)
                        {
                            reloc.addend += *label_offset as i64;
                            reloc.symbol_name = label_section.clone();
                        }
                    }
                }
            }
        }
    }

    /// Build the symbol table and serialize the ELF object file.
    ///
    /// `config`: ELF configuration (machine type, flags, class)
    /// `include_referenced_locals`: whether to include .L* labels referenced
    /// by relocations (needed by RISC-V for pcrel_hi/pcrel_lo pairs)
    pub fn write_elf(
        &mut self,
        output_path: &str,
        config: &ElfConfig,
        include_referenced_locals: bool,
    ) -> Result<(), String> {
        if !include_referenced_locals {
            self.resolve_local_data_relocs();
        }

        let symtab_input = SymbolTableInput {
            labels: &self.labels,
            global_symbols: &self.global_symbols,
            weak_symbols: &self.weak_symbols,
            symbol_types: &self.symbol_types,
            symbol_sizes: &self.symbol_sizes,
            symbol_visibility: &self.symbol_visibility,
            aliases: &self.aliases,
            sections: &self.sections,
            include_referenced_locals,
        };

        let mut symbols = build_elf_symbol_table(&symtab_input);
        // Remove UND entries for any symbols that are also in extra_symbols (e.g. COMMON).
        // A symbol that is both referenced in relocations and declared as COMMON should
        // only appear once (as COMMON), not as both UND and COMMON.
        for extra in &self.extra_symbols {
            if extra.section_name == "*COM*" {
                symbols.retain(|s| !(s.name == extra.name && s.section_name == "*UND*"));
            }
        }
        symbols.append(&mut self.extra_symbols);

        let elf_bytes =
            write_relocatable_object(config, &self.section_order, &self.sections, &symbols)?;

        std::fs::write(output_path, &elf_bytes)
            .map_err(|e| format!("failed to write ELF file: {}", e))?;

        Ok(())
    }
}
