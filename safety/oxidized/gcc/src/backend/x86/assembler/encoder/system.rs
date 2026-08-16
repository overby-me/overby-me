use super::*;

impl super::InstructionEncoder {
    /// Encode OUT instruction: outb/outw/outl
    /// AT&T syntax: outb %al, %dx  OR  outb %al, $imm8
    pub(crate) fn encode_out(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        let size: u8 = match mnemonic {
            "outb" => 1,
            "outw" => 2,
            "outl" => 4,
            _ => return Err(format!("unknown out mnemonic: {}", mnemonic)),
        };

        // Handle zero-operand form (implicit operands)
        if ops.is_empty() {
            if size == 2 {
                self.bytes.push(0x66);
            }
            self.bytes.push(if size == 1 { 0xEE } else { 0xEF });
            return Ok(());
        }

        if ops.len() != 2 {
            return Err(format!("{} requires 0 or 2 operands", mnemonic));
        }

        // AT&T: outb %al, %dx  (src=al, dst=dx)
        // AT&T: outb %al, $imm8  (src=al, dst=imm)
        // Also handle parenthesized form: outl %eax, (%dx)
        match (&ops[0], &ops[1]) {
            (Operand::Register(_), Operand::Register(_)) => {
                // outb %al, %dx  =>  0xEE (byte) or 0xEF (word/dword)
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.bytes.push(if size == 1 { 0xEE } else { 0xEF });
                Ok(())
            }
            (Operand::Register(_), Operand::Memory(_)) => {
                // outl %eax, (%dx)  =>  same encoding as register form
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.bytes.push(if size == 1 { 0xEE } else { 0xEF });
                Ok(())
            }
            (Operand::Register(_), Operand::Immediate(ImmediateValue::Integer(val))) => {
                // outb %al, $imm8  =>  0xE6 ib (byte) or 0xE7 ib (word/dword)
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.bytes.push(if size == 1 { 0xE6 } else { 0xE7 });
                self.bytes.push(*val as u8);
                Ok(())
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    /// Encode IN instruction: inb/inw/inl
    /// AT&T syntax: inb %dx, %al  OR  inb $imm8, %al
    pub(crate) fn encode_in(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        let size: u8 = match mnemonic {
            "inb" => 1,
            "inw" => 2,
            "inl" => 4,
            _ => return Err(format!("unknown in mnemonic: {}", mnemonic)),
        };

        // Handle zero-operand form (implicit operands)
        if ops.is_empty() {
            if size == 2 {
                self.bytes.push(0x66);
            }
            self.bytes.push(if size == 1 { 0xEC } else { 0xED });
            return Ok(());
        }

        if ops.len() != 2 {
            return Err(format!("{} requires 0 or 2 operands", mnemonic));
        }

        // AT&T: inb %dx, %al  (src=dx, dst=al)
        // AT&T: inb $imm8, %al  (src=imm, dst=al)
        // Also handle parenthesized form: inl (%dx), %eax
        match (&ops[0], &ops[1]) {
            (Operand::Register(_), Operand::Register(_)) => {
                // inb %dx, %al  =>  0xEC (byte) or 0xED (word/dword)
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.bytes.push(if size == 1 { 0xEC } else { 0xED });
                Ok(())
            }
            (Operand::Memory(_), Operand::Register(_)) => {
                // inl (%dx), %eax  =>  same encoding as register form
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.bytes.push(if size == 1 { 0xEC } else { 0xED });
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(_)) => {
                // inb $imm8, %al  =>  0xE4 ib (byte) or 0xE5 ib (word/dword)
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.bytes.push(if size == 1 { 0xE4 } else { 0xE5 });
                self.bytes.push(*val as u8);
                Ok(())
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    pub(crate) fn encode_clflush(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("clflush requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0xAE]);
                self.encode_modrm_mem(7, mem)
            }
            _ => Err("clflush requires memory operand".to_string()),
        }
    }

    /// Encode VERW instruction: 0F 00 /5
    pub(crate) fn encode_verw(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("verw requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0x00]);
                self.encode_modrm_mem(5, mem)
            }
            Operand::Register(reg) => {
                let rm = reg_num(&reg.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, 0x00]);
                self.bytes.push(self.modrm(3, 5, rm));
                Ok(())
            }
            _ => Err("verw requires memory or register operand".to_string()),
        }
    }

    /// Encode MOV to/from control register: 0F 20 /r (read) or 0F 22 /r (write)
    pub(crate) fn encode_mov_cr(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("mov cr requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Register(cr), Operand::Register(gp)) if is_control_reg(&cr.name) => {
                // movq %crN, %rax  =>  0F 20 ModR/M
                let cr_num = control_reg_num(&cr.name).ok_or("bad control register")?;
                let gp_num = reg_num(&gp.name).ok_or("bad register")?;
                // Need REX prefix for r8-r15 or for cr8
                let mut rex = 0u8;
                if cr_num >= 8 {
                    rex |= 0x44;
                } // REX.R
                if needs_rex_ext(&gp.name) {
                    rex |= 0x41;
                } // REX.B
                if rex != 0 {
                    self.bytes.push(rex);
                }
                self.bytes.extend_from_slice(&[0x0F, 0x20]);
                self.bytes.push(self.modrm(3, cr_num & 7, gp_num));
                Ok(())
            }
            (Operand::Register(gp), Operand::Register(cr)) if is_control_reg(&cr.name) => {
                // movq %rax, %crN  =>  0F 22 ModR/M
                let cr_num = control_reg_num(&cr.name).ok_or("bad control register")?;
                let gp_num = reg_num(&gp.name).ok_or("bad register")?;
                let mut rex = 0u8;
                if cr_num >= 8 {
                    rex |= 0x44;
                } // REX.R
                if needs_rex_ext(&gp.name) {
                    rex |= 0x41;
                } // REX.B
                if rex != 0 {
                    self.bytes.push(rex);
                }
                self.bytes.extend_from_slice(&[0x0F, 0x22]);
                self.bytes.push(self.modrm(3, cr_num & 7, gp_num));
                Ok(())
            }
            _ => Err("unsupported mov cr operands".to_string()),
        }
    }

    /// Encode MOV to/from debug register: 0F 21 /r (read) or 0F 23 /r (write)
    pub(crate) fn encode_mov_dr(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("mov dr requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Register(dr), Operand::Register(gp)) if is_debug_reg(&dr.name) => {
                // movq %drN, %rax  =>  0F 21 ModR/M
                let dr_num = debug_reg_num(&dr.name).ok_or("bad debug register")?;
                let gp_num = reg_num(&gp.name).ok_or("bad register")?;
                let mut rex = 0u8;
                if needs_rex_ext(&gp.name) {
                    rex |= 0x41;
                } // REX.B
                if rex != 0 {
                    self.bytes.push(rex);
                }
                self.bytes.extend_from_slice(&[0x0F, 0x21]);
                self.bytes.push(self.modrm(3, dr_num, gp_num));
                Ok(())
            }
            (Operand::Register(gp), Operand::Register(dr)) if is_debug_reg(&dr.name) => {
                // movq %rax, %drN  =>  0F 23 ModR/M
                let dr_num = debug_reg_num(&dr.name).ok_or("bad debug register")?;
                let gp_num = reg_num(&gp.name).ok_or("bad register")?;
                let mut rex = 0u8;
                if needs_rex_ext(&gp.name) {
                    rex |= 0x41;
                } // REX.B
                if rex != 0 {
                    self.bytes.push(rex);
                }
                self.bytes.extend_from_slice(&[0x0F, 0x23]);
                self.bytes.push(self.modrm(3, dr_num, gp_num));
                Ok(())
            }
            _ => Err("unsupported mov dr operands".to_string()),
        }
    }

    /// Encode SGDT/SIDT/LGDT/LIDT: 0F 01 /N (memory operand)
    /// Handles suffixed forms (sgdtl, lgdtq, etc.) and label operands.
    pub(crate) fn encode_system_table(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err(format!("{} requires 1 operand", mnemonic));
        }
        // Strip size suffix (l/q) to get base mnemonic
        let base = mnemonic.trim_end_matches(['l', 'q']);
        let reg_ext = match base {
            "sgdt" => 0,
            "sidt" => 1,
            "lgdt" => 2,
            "lidt" => 3,
            _ => return Err(format!("unknown system table instruction: {}", mnemonic)),
        };
        match &ops[0] {
            Operand::Memory(mem) => {
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.encode_modrm_mem(reg_ext, mem)
            }
            // Label operand: treat as RIP-relative memory reference
            Operand::Label(name) => {
                let mem = MemoryOperand {
                    segment: None,
                    displacement: Displacement::Symbol(name.clone()),
                    base: None,
                    index: None,
                    scale: None,
                };
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.encode_modrm_mem(reg_ext, &mem)
            }
            _ => Err(format!("{} requires memory operand", mnemonic)),
        }
    }

    /// Encode LMSW (Load Machine Status Word): 0F 01 /6
    /// Accepts a 16-bit register or memory operand.
    pub(crate) fn encode_lmsw(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("lmsw requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let rm = reg_num(&reg.name).ok_or("bad register")?;
                if needs_rex_ext(&reg.name) {
                    self.bytes.push(self.rex(false, false, false, true));
                }
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.bytes.push(self.modrm(3, 6, rm));
                Ok(())
            }
            Operand::Memory(mem) => {
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.encode_modrm_mem(6, mem)
            }
            _ => Err("lmsw requires register or memory operand".to_string()),
        }
    }

    /// Encode SMSW (Store Machine Status Word): 0F 01 /4
    /// Accepts a register or memory operand.
    /// 16-bit register form gets a 66h prefix.
    pub(crate) fn encode_smsw(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("smsw requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let rm = reg_num(&reg.name).ok_or("bad register")?;
                let is_16 = is_reg16(&reg.name);
                if is_16 {
                    self.bytes.push(0x66);
                }
                if needs_rex_ext(&reg.name) {
                    self.bytes.push(self.rex(false, false, false, true));
                }
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.bytes.push(self.modrm(3, 4, rm));
                Ok(())
            }
            Operand::Memory(mem) => {
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.encode_modrm_mem(4, mem)
            }
            _ => Err("smsw requires register or memory operand".to_string()),
        }
    }

    /// Encode system register instructions (LLDT, LTR, STR, SLDT): opcode /ext
    /// These take a 16-bit register or memory operand.
    pub(crate) fn encode_system_reg(
        &mut self,
        ops: &[Operand],
        opcode: &[u8],
        ext: u8,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("system register instruction requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let reg_num = reg_num(&reg.name).ok_or("bad register")?;
                // No REX.W for 16-bit system register instructions
                if needs_rex_ext(&reg.name) {
                    self.bytes.push(self.rex(false, false, false, true));
                }
                self.bytes.extend_from_slice(opcode);
                self.bytes.push(self.modrm(3, ext, reg_num));
                Ok(())
            }
            Operand::Memory(mem) => {
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(opcode);
                self.encode_modrm_mem(ext, mem)
            }
            _ => Err("system register instruction requires register or memory operand".to_string()),
        }
    }

    /// Encode memory-only instructions (INVLPG, FXSAVE, FXRSTOR, etc.): opcode /ext
    pub(crate) fn encode_mem_only(
        &mut self,
        ops: &[Operand],
        opcode: &[u8],
        ext: u8,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("instruction requires 1 memory operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.emit_rex_rm(0, "", mem);
                self.bytes.extend_from_slice(opcode);
                self.encode_modrm_mem(ext, mem)
            }
            _ => Err("instruction requires memory operand".to_string()),
        }
    }

    /// Encode FXSAVEQ: REX.W + 0F AE /0 (64-bit fxsave)
    pub(crate) fn encode_fxsaveq(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("fxsaveq requires 1 memory operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                // Force REX.W prefix
                let rex_b = mem.base.as_ref().is_some_and(|r| needs_rex_ext(&r.name));
                let rex_x = mem.index.as_ref().is_some_and(|r| needs_rex_ext(&r.name));
                self.bytes.push(self.rex(true, false, rex_x, rex_b));
                self.bytes.extend_from_slice(&[0x0F, 0xAE]);
                self.encode_modrm_mem(0, mem)
            }
            _ => Err("fxsaveq requires memory operand".to_string()),
        }
    }

    /// Encode FXRSTORQ: REX.W + 0F AE /1 (64-bit fxrstor)
    pub(crate) fn encode_fxrstorq(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("fxrstorq requires 1 memory operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                // Force REX.W prefix
                let rex_b = mem.base.as_ref().is_some_and(|r| needs_rex_ext(&r.name));
                let rex_x = mem.index.as_ref().is_some_and(|r| needs_rex_ext(&r.name));
                self.bytes.push(self.rex(true, false, rex_x, rex_b));
                self.bytes.extend_from_slice(&[0x0F, 0xAE]);
                self.encode_modrm_mem(1, mem)
            }
            _ => Err("fxrstorq requires memory operand".to_string()),
        }
    }

    /// Encode INVPCID: 66 0F 38 82 /r
    pub(crate) fn encode_invpcid(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("invpcid requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Memory(mem), Operand::Register(reg)) => {
                let reg_num = reg_num(&reg.name).ok_or("bad register")?;
                self.bytes.push(0x66);
                self.emit_rex_rm(0, &reg.name, mem);
                self.bytes.extend_from_slice(&[0x0F, 0x38, 0x82]);
                self.encode_modrm_mem(reg_num, mem)
            }
            _ => Err("invpcid requires memory and register operands".to_string()),
        }
    }

    /// Encode RDFSBASE/RDGSBASE/WRFSBASE/WRGSBASE: F3 0F AE /N
    pub(crate) fn encode_fsgsbase(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err(format!("{} requires 1 register operand", mnemonic));
        }
        let ext = match mnemonic {
            "rdfsbase" => 0,
            "rdgsbase" => 1,
            "wrfsbase" => 2,
            "wrgsbase" => 3,
            _ => return Err(format!("unknown fsgsbase variant: {}", mnemonic)),
        };
        match &ops[0] {
            Operand::Register(reg) => {
                let num = reg_num(&reg.name).ok_or("bad register")?;
                self.bytes.push(0xF3);
                // Need REX.W for 64-bit register, REX.B for extended register
                let is_64 = is_reg64(&reg.name);
                let ext_reg = needs_rex_ext(&reg.name);
                if is_64 || ext_reg {
                    self.bytes.push(self.rex(is_64, false, false, ext_reg));
                }
                self.bytes.extend_from_slice(&[0x0F, 0xAE]);
                self.bytes.push(self.modrm(3, ext, num));
                Ok(())
            }
            _ => Err(format!("{} requires register operand", mnemonic)),
        }
    }

    /// Encode LJMP (far jump): EA cp (direct) or FF /5 (indirect memory)
    pub(crate) fn encode_ljmp(&mut self, ops: &[Operand], _mnemonic: &str) -> Result<(), String> {
        match ops.len() {
            // ljmpl *mem - indirect far jump through memory
            1 => {
                match &ops[0] {
                    Operand::Indirect(inner) => match inner.as_ref() {
                        Operand::Memory(mem) => {
                            self.emit_rex_rm(0, "", mem);
                            self.bytes.push(0xFF);
                            self.encode_modrm_mem(5, mem)
                        }
                        _ => Err("ljmp indirect requires memory operand".to_string()),
                    },
                    Operand::Memory(mem) => {
                        // ljmp *mem (without explicit indirect prefix)
                        self.emit_rex_rm(0, "", mem);
                        self.bytes.push(0xFF);
                        self.encode_modrm_mem(5, mem)
                    }
                    _ => {
                        Err("ljmp requires indirect memory or segment:offset operands".to_string())
                    }
                }
            }
            // ljmpl $segment, $offset - direct far jump (opcode 0xEA, valid in 32-bit/16-bit mode only;
            // used by kernel realmode trampoline code compiled with gcc_m16)
            2 => match (&ops[0], &ops[1]) {
                (
                    Operand::Immediate(ImmediateValue::Integer(seg)),
                    Operand::Immediate(ImmediateValue::Integer(off)),
                ) => {
                    self.bytes.push(0xEA);
                    self.bytes.extend_from_slice(&(*off as u32).to_le_bytes());
                    self.bytes.extend_from_slice(&(*seg as u16).to_le_bytes());
                    Ok(())
                }
                (
                    Operand::Immediate(ImmediateValue::Integer(seg)),
                    Operand::Immediate(ImmediateValue::Symbol(sym)),
                )
                | (
                    Operand::Immediate(ImmediateValue::Integer(seg)),
                    Operand::Immediate(ImmediateValue::SymbolPlusOffset(sym, _)),
                ) => {
                    let addend = match &ops[1] {
                        Operand::Immediate(ImmediateValue::SymbolPlusOffset(_, a)) => *a,
                        _ => 0,
                    };
                    self.bytes.push(0xEA);
                    self.add_relocation(sym, R_X86_64_32, addend);
                    self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                    self.bytes.extend_from_slice(&(*seg as u16).to_le_bytes());
                    Ok(())
                }
                _ => Err("ljmp requires $segment, $offset operands".to_string()),
            },
            _ => Err("ljmp requires 1 or 2 operands".to_string()),
        }
    }

    /// Encode LAR (Load Access Rights): 0F 02 /r
    pub(crate) fn encode_lar(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("lar requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                // Infer size from destination register
                let size = infer_reg_size(&dst.name);
                self.emit_rex_rr(size, &dst.name, &src.name);
                self.bytes.extend_from_slice(&[0x0F, 0x02]);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                let size = infer_reg_size(&dst.name);
                self.emit_rex_rm(size, &dst.name, mem);
                self.bytes.extend_from_slice(&[0x0F, 0x02]);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported lar operands".to_string()),
        }
    }

    /// Encode LSL (Load Segment Limit): 0F 03 /r
    pub(crate) fn encode_lsl(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("lsl requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                // 16-bit operand size prefix
                if src.name.starts_with('w')
                    || src.name.len() == 2 && src.name.ends_with('x')
                    || src.name.ends_with('i')
                        && !src.name.starts_with('e')
                        && !src.name.starts_with('r')
                {
                    // Heuristic: check if it's 16-bit register
                    let is_16 = matches!(
                        src.name.as_str(),
                        "ax" | "bx" | "cx" | "dx" | "si" | "di" | "sp" | "bp"
                    );
                    if is_16 {
                        self.bytes.push(0x66);
                    }
                }
                self.emit_rex_rr(4, &dst.name, &src.name);
                self.bytes.extend_from_slice(&[0x0F, 0x03]);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.emit_rex_rm(4, &dst.name, mem);
                self.bytes.extend_from_slice(&[0x0F, 0x03]);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported lsl operands".to_string()),
        }
    }
}
