use super::*;

impl super::InstructionEncoder {
    // ---- Encoding helpers ----

    /// Build a REX prefix byte.
    pub(crate) fn rex(&self, w: bool, r: bool, x: bool, b: bool) -> u8 {
        let mut rex = 0x40u8;
        if w {
            rex |= 0x08;
        }
        if r {
            rex |= 0x04;
        }
        if x {
            rex |= 0x02;
        }
        if b {
            rex |= 0x01;
        }
        rex
    }

    /// Encode ModR/M byte.
    pub(crate) fn modrm(&self, mod_: u8, reg: u8, rm: u8) -> u8 {
        (mod_ << 6) | ((reg & 7) << 3) | (rm & 7)
    }

    /// Encode SIB byte.
    pub(crate) fn sib(&self, scale: u8, index: u8, base: u8) -> u8 {
        let scale_bits = match scale {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            _ => 0,
        };
        (scale_bits << 6) | ((index & 7) << 3) | (base & 7)
    }

    /// Emit REX prefix if needed for reg-reg operation.
    pub(crate) fn emit_rex_rr(&mut self, size: u8, reg: &str, rm: &str) {
        let w = size == 8;
        let r = needs_rex_ext(reg);
        let b = needs_rex_ext(rm);
        let need_rex = w || r || b || is_rex_required_8bit(reg) || is_rex_required_8bit(rm);
        if need_rex {
            self.bytes.push(self.rex(w, r, false, b));
        }
    }

    /// Emit segment override prefix (0x64 for %fs, 0x65 for %gs) if present.
    /// Must be emitted before any operand-size override, REX prefix, or opcode.
    // TODO: emit_segment_prefix is called in mov, ALU ops, push, and pop.
    // Other instruction families that accept memory operands should also call this.
    pub(crate) fn emit_segment_prefix(&mut self, mem: &MemoryOperand) -> Result<(), String> {
        if let Some(ref seg) = mem.segment {
            match seg.as_str() {
                "fs" => self.bytes.push(0x64),
                "gs" => self.bytes.push(0x65),
                _ => return Err(format!("unsupported segment override: %{}", seg)),
            }
        }
        Ok(())
    }

    /// Emit REX prefix for a memory operand where 'reg' is the reg field.
    pub(crate) fn emit_rex_rm(&mut self, size: u8, reg: &str, mem: &MemoryOperand) {
        let w = size == 8;
        let r = needs_rex_ext(reg);
        let b = mem.base.as_ref().is_some_and(|b| needs_rex_ext(&b.name));
        let x = mem.index.as_ref().is_some_and(|i| needs_rex_ext(&i.name));
        let need_rex = w || r || b || x || is_rex_required_8bit(reg);
        if need_rex {
            self.bytes.push(self.rex(w, r, x, b));
        }
    }

    /// Emit REX prefix for unary operation on register.
    pub(crate) fn emit_rex_unary(&mut self, size: u8, rm: &str) {
        let w = size == 8;
        let b = needs_rex_ext(rm);
        let need_rex = w || b || is_rex_required_8bit(rm);
        if need_rex {
            self.bytes.push(self.rex(w, false, false, b));
        }
    }

    /// Encode ModR/M + SIB + displacement for a memory operand.
    /// Returns the bytes to append. `reg_field` is the /r value (3 bits).
    pub(crate) fn encode_modrm_mem(
        &mut self,
        reg_field: u8,
        mem: &MemoryOperand,
    ) -> Result<(), String> {
        let base = mem.base.as_ref();
        let index = mem.index.as_ref();

        // RIP-relative addressing
        if let Some(base_reg) = base {
            if base_reg.name == "rip" {
                // ModR/M: mod=00, rm=101 (RIP-relative)
                self.bytes.push(self.modrm(0, reg_field, 5));
                // 32-bit displacement (will be filled by relocation)
                match &mem.displacement {
                    Displacement::Symbol(sym) => {
                        self.add_relocation(sym, R_X86_64_PC32, -4);
                        self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                    }
                    Displacement::SymbolAddend(sym, addend) => {
                        self.add_relocation(sym, R_X86_64_PC32, *addend - 4);
                        self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                    }
                    Displacement::SymbolPlusOffset(sym, offset) => {
                        self.add_relocation(sym, R_X86_64_PC32, *offset - 4);
                        self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                    }
                    Displacement::SymbolMod(sym, modifier) => {
                        let reloc_type = match modifier.as_str() {
                            "GOTPCREL" => R_X86_64_GOTPCREL,
                            "GOTTPOFF" => R_X86_64_GOTTPOFF,
                            "PLT" => R_X86_64_PLT32,
                            _ => R_X86_64_PC32,
                        };
                        self.add_relocation(sym, reloc_type, -4);
                        self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                    }
                    Displacement::Integer(val) => {
                        self.bytes.extend_from_slice(&(*val as i32).to_le_bytes());
                    }
                    Displacement::None => {
                        self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                    }
                }
                return Ok(());
            }
        }

        // Handle symbol displacements that need relocations.
        // We defer emitting the relocation until after the ModR/M and SIB bytes
        // so the relocation offset correctly points to the displacement bytes.
        let (disp_val, has_symbol, deferred_reloc) = match &mem.displacement {
            Displacement::None => (0i64, false, None),
            Displacement::Integer(v) => (*v, false, None),
            Displacement::Symbol(sym) => (0i64, true, Some((sym.clone(), R_X86_64_32S, 0i64))),
            Displacement::SymbolAddend(sym, addend) => {
                (0i64, true, Some((sym.clone(), R_X86_64_32S, *addend)))
            }
            Displacement::SymbolPlusOffset(sym, offset) => {
                (0i64, true, Some((sym.clone(), R_X86_64_32S, *offset)))
            }
            Displacement::SymbolMod(sym, modifier) => {
                let reloc_type = match modifier.as_str() {
                    "TPOFF" => R_X86_64_TPOFF32,
                    "GOTPCREL" => R_X86_64_GOTPCREL,
                    _ => R_X86_64_32S,
                };
                (0i64, true, Some((sym.clone(), reloc_type, 0i64)))
            }
        };

        // No base register - need SIB with no-base encoding
        if base.is_none() && index.is_none() {
            // Direct memory reference - mod=00, rm=100 (SIB), SIB: base=101 (no base)
            self.bytes.push(self.modrm(0, reg_field, 4));
            self.bytes.push(self.sib(1, 4, 5)); // index=100 (none), base=101 (disp32)
            if let Some((sym, reloc_type, addend)) = deferred_reloc {
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
            || (base_num & 7) == 4  // RSP/R12 always need SIB
            || base.is_none();

        // Determine displacement size
        let (mod_bits, disp_size) = if has_symbol {
            (2, 4) // always use disp32 for symbols
        } else if disp_val == 0 && (base_num & 7) != 5 {
            // No displacement (RBP/R13 always need at least disp8)
            (0, 0)
        } else if (-128..=127).contains(&disp_val) {
            (1, 1) // disp8
        } else {
            (2, 4) // disp32
        };

        if need_sib {
            let idx = index.as_ref();
            let idx_num = idx.map(|r| reg_num(&r.name).unwrap_or(4)).unwrap_or(4); // 4 = no index
            let scale = mem.scale.unwrap_or(1);

            if base.is_none() {
                // No base - disp32 with SIB
                self.bytes.push(self.modrm(0, reg_field, 4));
                self.bytes.push(self.sib(scale, idx_num, 5));
                if let Some((sym, reloc_type, addend)) = deferred_reloc {
                    self.add_relocation(&sym, reloc_type, addend);
                }
                self.bytes
                    .extend_from_slice(&(disp_val as i32).to_le_bytes());
            } else {
                self.bytes.push(self.modrm(mod_bits, reg_field, 4));
                self.bytes.push(self.sib(scale, idx_num, base_num));
                if let Some((sym, reloc_type, addend)) = deferred_reloc {
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
            if let Some((sym, reloc_type, addend)) = deferred_reloc {
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
    pub(crate) fn add_relocation(&mut self, symbol: &str, reloc_type: u32, addend: i64) {
        // Strip @PLT suffix from symbol names - the suffix only affects relocation type,
        // not the symbol name in the ELF symbol table. Use PLT32 reloc when @PLT is present.
        let (sym, rtype) = if let Some(base) = symbol.strip_suffix("@PLT") {
            let plt_type = if reloc_type == R_X86_64_PC32 {
                R_X86_64_PLT32
            } else {
                reloc_type
            };
            (base, plt_type)
        } else {
            (symbol, reloc_type)
        };
        self.relocations.push(Relocation {
            offset: self.offset + self.bytes.len() as u64 - (self.offset), // adjusted in caller
            symbol: sym.to_string(),
            reloc_type: rtype,
            addend,
        });
    }

    /// Adjust a RIP-relative relocation's addend to account for immediate bytes
    /// that follow the displacement field in the instruction encoding.
    ///
    /// In x86-64, RIP-relative addressing computes the effective address as
    /// RIP + disp32, where RIP points to the byte *after* the current instruction.
    /// The R_X86_64_PC32 relocation computes S + A - P, where P is the address of
    /// the disp32 field. So the addend A must equal -(bytes from disp32 to end of
    /// instruction). `encode_modrm_mem` always uses A = -4 (for the disp32 itself),
    /// but instructions with trailing immediate bytes need A = -(4 + trailing_bytes).
    ///
    /// `reloc_count_before` is the length of `self.relocations` before
    /// `encode_modrm_mem` was called. This ensures we only adjust the relocation
    /// that was emitted by `encode_modrm_mem`, not any subsequent ones.
    pub(crate) fn adjust_rip_reloc_addend(
        &mut self,
        reloc_count_before: usize,
        trailing_bytes: i64,
    ) {
        // Only adjust if encode_modrm_mem added a relocation
        if self.relocations.len() > reloc_count_before {
            let reloc = &mut self.relocations[reloc_count_before];
            match reloc.reloc_type {
                R_X86_64_PC32 | R_X86_64_PLT32 | R_X86_64_GOTPCREL | R_X86_64_GOTTPOFF => {
                    reloc.addend -= trailing_bytes;
                }
                _ => {}
            }
        }
    }
}
