//! Core encoding helpers for i686 instruction encoding.
//!
//! ModR/M, SIB, segment prefixes, memory operand encoding, and relocation helpers.

use super::*;

impl super::InstructionEncoder {
    /// Encode ModR/M byte.
    pub(super) fn modrm(&self, mod_: u8, reg: u8, rm: u8) -> u8 {
        (mod_ << 6) | ((reg & 7) << 3) | (rm & 7)
    }

    /// Encode SIB byte.
    pub(super) fn sib(&self, scale: u8, index: u8, base: u8) -> u8 {
        let scale_bits = match scale {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            _ => 0,
        };
        (scale_bits << 6) | ((index & 7) << 3) | (base & 7)
    }

    /// Encode ModR/M + SIB + displacement for a memory operand.
    ///
    /// For i686 REL relocations, the relocation offset must point to the
    /// displacement field (where the addend is embedded), not to the ModR/M
    /// byte. So we defer add_relocation() until after emitting ModR/M and SIB.
    /// Emit segment override prefix if the memory operand has a segment.
    pub(super) fn emit_segment_prefix(&mut self, mem: &MemoryOperand) {
        if let Some(ref seg) = mem.segment {
            match seg.as_str() {
                "fs" => self.bytes.push(0x64),
                "gs" => self.bytes.push(0x65),
                "es" => self.bytes.push(0x26),
                "cs" => self.bytes.push(0x2E),
                "ss" => self.bytes.push(0x36),
                "ds" => self.bytes.push(0x3E),
                _ => {}
            }
        }
    }

    pub(super) fn encode_modrm_mem(
        &mut self,
        reg_field: u8,
        mem: &MemoryOperand,
    ) -> Result<(), String> {
        let base = mem.base.as_ref();
        let index = mem.index.as_ref();

        // Parse displacement but defer relocation until after ModR/M+SIB bytes
        let (disp_val, has_symbol, pending_reloc) = match &mem.displacement {
            Displacement::None => (0i64, false, None),
            Displacement::Integer(v) => (*v, false, None),
            Displacement::Symbol(sym) => (0i64, true, Some((sym.clone(), R_386_32, 0i64))),
            Displacement::SymbolAddend(sym, addend) => {
                (0i64, true, Some((sym.clone(), R_386_32, *addend)))
            }
            Displacement::SymbolPlusOffset(sym, offset) => {
                (0i64, true, Some((sym.clone(), R_386_32, *offset)))
            }
            Displacement::SymbolMod(sym, modifier) => {
                let reloc_type = self.tls_reloc_type(modifier);
                (0i64, true, Some((sym.clone(), reloc_type, 0i64)))
            }
        };

        // No base register - need SIB with no-base encoding or direct displacement
        if base.is_none() && index.is_none() {
            // Direct memory reference - mod=00, rm=101 (disp32)
            self.bytes.push(self.modrm(0, reg_field, 5));
            // Emit relocation now, pointing at the displacement bytes
            if let Some((sym, reloc_type, addend)) = pending_reloc {
                self.add_relocation(&sym, reloc_type, addend);
            }
            self.bytes
                .extend_from_slice(&(disp_val as i32).to_le_bytes());
            return Ok(());
        }

        let base_reg = base.map(|r| &r.name as &str).unwrap_or("");
        let base_num = if !base_reg.is_empty() {
            reg_num(base_reg).unwrap_or(0)
        } else {
            5
        };

        // Determine if we need SIB
        let need_sib = index.is_some()
            || (base_num & 7) == 4  // ESP always needs SIB
            || base.is_none();

        // Determine displacement size
        let (mod_bits, disp_size) = if has_symbol {
            (2, 4) // always use disp32 for symbols
        } else if disp_val == 0 && (base_num & 7) != 5 {
            // No displacement (EBP always needs at least disp8)
            (0, 0)
        } else if (-128..=127).contains(&disp_val) {
            (1, 1) // disp8
        } else {
            (2, 4) // disp32
        };

        if need_sib {
            let idx = index.as_ref();
            let idx_num = idx.map(|r| reg_num(&r.name).unwrap_or(4)).unwrap_or(4);
            let scale = mem.scale.unwrap_or(1);

            if base.is_none() {
                // No base - disp32 with SIB
                self.bytes.push(self.modrm(0, reg_field, 4));
                self.bytes.push(self.sib(scale, idx_num, 5));
                // Emit relocation after ModR/M+SIB, before displacement
                if let Some((sym, reloc_type, addend)) = pending_reloc {
                    self.add_relocation(&sym, reloc_type, addend);
                }
                self.bytes
                    .extend_from_slice(&(disp_val as i32).to_le_bytes());
            } else {
                self.bytes.push(self.modrm(mod_bits, reg_field, 4));
                self.bytes.push(self.sib(scale, idx_num, base_num));
                // Emit relocation after ModR/M+SIB, before displacement
                if let Some((sym, reloc_type, addend)) = pending_reloc {
                    self.add_relocation(&sym, reloc_type, addend);
                }
                match disp_size {
                    0 => {}
                    1 => self.bytes.push(disp_val as u8),
                    4 => self
                        .bytes
                        .extend_from_slice(&(disp_val as i32).to_le_bytes()),
                    _ => unreachable!(),
                }
            }
        } else {
            self.bytes.push(self.modrm(mod_bits, reg_field, base_num));
            // Emit relocation after ModR/M, before displacement
            if let Some((sym, reloc_type, addend)) = pending_reloc {
                self.add_relocation(&sym, reloc_type, addend);
            }
            match disp_size {
                0 => {}
                1 => self.bytes.push(disp_val as u8),
                4 => self
                    .bytes
                    .extend_from_slice(&(disp_val as i32).to_le_bytes()),
                _ => unreachable!(),
            }
        }

        Ok(())
    }

    /// Add a relocation relative to current position.
    pub(super) fn add_relocation(&mut self, symbol: &str, reloc_type: u32, addend: i64) {
        // Strip @PLT suffix from symbol names - the suffix only affects relocation type,
        // not the symbol name in the ELF symbol table.
        let (sym, rtype) = if let Some(base) = symbol.strip_suffix("@PLT") {
            let plt_type = if reloc_type == R_386_PC32 {
                R_386_PLT32
            } else {
                reloc_type
            };
            (base, plt_type)
        } else {
            (symbol, reloc_type)
        };
        self.relocations.push(Relocation {
            offset: self.bytes.len() as u64,
            symbol: sym.to_string(),
            reloc_type: rtype,
            addend,
            diff_symbol: None,
        });
    }

    /// Add a relocation for a label that may contain `symbol+offset` or `symbol-offset`.
    /// Splits the label string and extracts the addend if present.
    pub(super) fn add_relocation_for_label(&mut self, label: &str, reloc_type: u32) {
        let (sym, addend) = split_label_offset(label);
        self.add_relocation(sym, reloc_type, addend);
    }

    pub(super) fn add_relocation_with_diff(
        &mut self,
        symbol: &str,
        reloc_type: u32,
        addend: i64,
        diff_sym: &str,
    ) {
        self.relocations.push(Relocation {
            offset: self.bytes.len() as u64,
            symbol: symbol.to_string(),
            reloc_type,
            addend,
            diff_symbol: Some(diff_sym.to_string()),
        });
    }
}
